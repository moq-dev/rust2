//! Rust, as rustc checked it, to the TypeScript AST.
//!
//! Translation starts from the configured roots and follows what they reach. Functions
//! are monomorphized: each instance becomes its own TypeScript function, so a trait
//! call is always a direct call. Types are translated once, generically, and a function of
//! a type is a module-level one taking `self` first, so a bundler drops what goes unused.

mod bind;
pub use bind::typecheck;
mod body;
mod collections;
mod consts;
mod copy;
mod drop;
mod dynamic;
mod float;
mod fmt;
mod iter;
mod lend;
mod library;
mod mir;
pub(crate) mod names;
mod num;
pub mod pointer;
mod stock;
mod task;
mod time;
pub mod unions;

use std::collections::VecDeque;

use rustc_hir::def::DefKind;
use rustc_hir::def_id::{CrateNum, DefId, LocalDefId};
use rustc_middle::ty::{
	self, AdtDef, EarlyBinder, GenericArgsRef, Instance, InstanceKind, Ty, TyCtxt, TypeVisitableExt, TypingEnv,
};
use rustc_span::Span;

use crate::census::{Census, Refusal};
use crate::config::Config;
use crate::program::{Program, Source, SymKind, Symbol};
use crate::ts::{self, Ctor, FieldId, Item, Loc, SymId, Type};

/// Translate what the config's roots reach, or `None` after reporting why not.
pub fn translate<'tcx>(tcx: TyCtxt<'tcx>, config: &Config) -> Option<Program> {
	let (mut t, ()) = Translator::settle(tcx, config, false, |t| {
		for root in &config.roots {
			t.root(root);
		}
		for path in &config.printable {
			t.printable(path);
		}
		t.drain();
	})?;
	for (span, msg) in t.refusals.take().unwrap_or_default() {
		tcx.dcx().span_err(span, format!("rust2-ts: {msg}"));
	}
	t.crate_dirs();
	if t.failed {
		return None;
	}
	t.program.names = t.names();
	t.program.mark_unread();
	t.program.merge();
	if let Err(msg) = t.program.names.check() {
		tcx.dcx().err(format!("rust2-ts: {msg}"));
		return None;
	}
	if let Err((span, msg)) = t.program.name() {
		tcx.dcx().span_err(span, format!("rust2-ts: {msg}"));
		return None;
	}
	Some(t.program)
}

/// Every construct translation refuses in the crate: in what the config's roots reach, as
/// translation does, or where it names none, in what each item reaches as if each were a
/// root.
pub fn census<'tcx>(tcx: TyCtxt<'tcx>, config: &Config) -> Option<Census> {
	let (mut t, polymorphic) = Translator::settle(tcx, config, true, |t| match config.roots.is_empty() {
		true => t.every_item(),
		false => {
			for root in &config.roots {
				t.root(root);
			}
			for path in &config.printable {
				t.printable(path);
			}
			t.drain();
			0
		}
	})?;
	let sm = tcx.sess.source_map();
	let mut seen = rustc_data_structures::fx::FxHashSet::default();
	let mut refusals = Vec::new();
	for (span, message) in t.refusals.take().unwrap_or_default() {
		let (file, line, col, ..) = sm.span_to_location_info(span.source_callsite());
		let file = file.map_or_else(String::new, |file| {
			let file = file
				.name
				.display(rustc_span::RemapPathScopeComponents::DIAGNOSTICS)
				.to_string();
			// A crate from a registry by its version, wherever cargo keeps it.
			match file
				.split_once("/registry/src/")
				.and_then(|(_, rest)| rest.split_once('/'))
			{
				Some((_index, path)) => path.to_string(),
				None => file,
			}
		});
		let refusal = Refusal {
			file,
			line,
			col,
			message,
		};
		// Each instance of a generic function refuses the same spans.
		if seen.insert(refusal.clone()) {
			refusals.push(refusal);
		}
	}
	Some(Census {
		// Those over type parameters, other than the roots, were named but never lowered.
		functions: t.instances.keys().filter(|instance| !instance.args.has_param()).count() + polymorphic,
		polymorphic,
		refusals,
	})
}

impl<'tcx> Translator<'_, 'tcx> {
	/// Lower what each of the crate's items reaches as if each were a root, returning how
	/// many generic functions no instance reached, lowered over their type parameters.
	fn every_item(&mut self) -> usize {
		let tcx = self.tcx;
		let t = self;
		for def_id in tcx.hir_crate_items(()).definitions() {
			if matches!(tcx.def_kind(def_id), DefKind::Struct | DefKind::Enum | DefKind::Union)
				&& !t.bound(def_id.to_def_id())
			{
				t.adt(tcx.adt_def(def_id), tcx.def_span(def_id));
			}
		}
		let functions = |tcx: TyCtxt<'tcx>| {
			tcx.hir_body_owners()
				.filter(move |&def_id| matches!(tcx.def_kind(def_id), DefKind::Fn | DefKind::AssocFn))
		};
		// A derived impl comes along only when called or printable, as in translation.
		for path in &t.config.printable {
			t.printable(path);
		}
		let roots: Vec<DefId> = functions(tcx)
			.map(LocalDefId::to_def_id)
			.filter(|&def_id| {
				!tcx.impl_of_assoc(def_id)
					.is_some_and(|imp| tcx.is_automatically_derived(imp))
					&& !t.bound(def_id)
			})
			.collect();
		let generic = |def_id: DefId| tcx.generics_of(def_id).requires_monomorphization(tcx);
		for &def_id in roots.iter().filter(|&&def_id| !generic(def_id)) {
			t.instance(Instance::mono(tcx, def_id));
		}
		t.drain();
		// A generic function no instance reached is lowered over its own type parameters, so
		// what it refuses whatever they are is counted too.
		let mut polymorphic = 0;
		for &def_id in roots.iter().filter(|&&def_id| generic(def_id)) {
			let lowered = |instance: &Instance<'tcx>| instance.def_id() == def_id && !instance.args.has_param();
			if t.instances.keys().any(lowered) {
				continue;
			}
			polymorphic += 1;
			t.env = TypingEnv::post_analysis(tcx, def_id);
			let instance = Instance::new_raw(def_id, ty::GenericArgs::identity_for_item(tcx, def_id));
			let sym = t.instance(instance);
			t.queue.retain(|&(queued, _)| queued != instance);
			t.function(instance, sym);
			t.drain();
		}
		polymorphic
	}
}

pub struct Translator<'a, 'tcx> {
	pub tcx: TyCtxt<'tcx>,
	config: &'a Config,
	/// The crates whose bodies translate.
	pub crates: Crates<'tcx>,
	pub program: Program,
	instances: rustc_data_structures::fx::FxHashMap<Instance<'tcx>, SymId>,
	queue: VecDeque<(Instance<'tcx>, SymId)>,
	adts: rustc_data_structures::fx::FxHashMap<DefId, Adt>,
	/// Source files by name, as indexes into `program.sources`.
	sources: rustc_data_structures::fx::FxHashMap<String, Option<u32>>,
	failed: bool,
	/// Where trait bounds come from: none outside a census, which lowers some functions
	/// over their own type parameters.
	env: TypingEnv<'tcx>,
	/// Whether this is a census, which lowers some functions over their type parameters and
	/// writes nothing.
	census: bool,
	/// Refusals collected until translation settles, then reported or counted.
	refusals: Option<Vec<(Span, String)>>,
	/// Whether each type's drop does something JS can see, as [`Self::drops`] found.
	drops: rustc_data_structures::fx::FxHashMap<Ty<'tcx>, bool>,
	/// The types [`Self::drops`] is still deciding, and whether the query open now read one.
	drops_open: rustc_data_structures::fx::FxHashSet<Ty<'tcx>>,
	drops_cycle: bool,
	/// The classes given drop glue.
	glued: rustc_data_structures::fx::FxHashSet<DefId>,
	/// The drop glue made from each type's drop shim.
	pub drop_shims: rustc_data_structures::fx::FxHashMap<Ty<'tcx>, SymId>,
	/// The function copying each `Copy` aggregate type.
	copy_shims: rustc_data_structures::fx::FxHashMap<Ty<'tcx>, SymId>,
	/// Each module value, by the item it stands for: a thread-local's key or a static.
	pub values: rustc_data_structures::fx::FxHashMap<DefId, SymId>,
	/// The statics whose values are being read, which a pointer within one can't name yet.
	pub reading: rustc_data_structures::fx::FxHashSet<DefId>,
	/// The statics made on first use, as an `rt.LazyCell` of their value.
	pub deferred: rustc_data_structures::fx::FxHashSet<SymId>,
	/// The types the crate writes into in place, which alone are copied.
	writes: copy::Writes<'tcx>,
	/// The instances whose writes count: those the runs before reached.
	read: rustc_data_structures::fx::FxHashSet<Instance<'tcx>>,
	/// The instances whose bodies lowered from MIR, a closure's included.
	lowered: rustc_data_structures::fx::FxHashSet<Instance<'tcx>>,
	/// The closures whose clones can't share them that the runs before reached, each made
	/// by a function of its captures, which makes its clone too.
	cloned: rustc_data_structures::fx::FxHashSet<DefId>,
	/// Those this run reached.
	cloning: rustc_data_structures::fx::FxHashSet<DefId>,
	/// The trait of each trait object, by its principal trait.
	objects: rustc_data_structures::fx::FxHashMap<DefId, dynamic::Object>,
	/// The class wrapping each type as each trait object.
	object_classes: rustc_data_structures::fx::FxHashMap<(Ty<'tcx>, SymId, dynamic::Held), dynamic::Class>,
	/// The class of each coroutine instance.
	pub coroutines: rustc_data_structures::fx::FxHashMap<Instance<'tcx>, mir::coroutine::Info<'tcx>>,
	/// Whether [`Self::ty`] is typing a variable: see [`Self::held_ty`].
	held: bool,
	/// Where the crate first reached each instance of std's that lowers from its MIR, which
	/// refuses there: its own spans are std's.
	sites: rustc_data_structures::fx::FxHashMap<Instance<'tcx>, Span>,
	/// While such an instance lowers, that span and its function.
	site: Option<(Span, DefId)>,
	/// The hand-written TypeScript the config binds items to.
	bindings: bind::Bindings<'tcx>,
}

/// The crates that translate as one program: the package, and every crate it depends on
/// but std's and those [`stock`] maps whole. What the roots reach of them translates, but
/// for what [`stock`] maps in place of it; any function of the others is its mapping, a
/// binding, or refused.
pub struct Crates<'tcx> {
	tcx: TyCtxt<'tcx>,
	crates: rustc_data_structures::fx::FxHashSet<CrateNum>,
}

impl<'tcx> Crates<'tcx> {
	fn new(tcx: TyCtxt<'tcx>) -> Self {
		let crates = tcx
			.crates(())
			.iter()
			.copied()
			.filter(|&krate| {
				!tcx.crate_dep_kind(krate).macros_only() && !of_std(tcx, krate) && !stock::is_opaque_crate(tcx, krate)
			})
			.collect();
		Self { tcx, crates }
	}

	/// Whether `def_id` translates: it is of a crate that does, and no mapping stands in for it.
	pub fn translates(&self, def_id: DefId) -> bool {
		def_id.is_local() || (self.crates.contains(&def_id.krate) && !stock::mapped(self.tcx, def_id))
	}
}

/// Whether `krate` is std or a crate std is built from: `-Zbuild-std` compiles those, and
/// only those, with stability on every item.
fn of_std(tcx: TyCtxt<'_>, krate: CrateNum) -> bool {
	tcx.lookup_stability(krate.as_def_id()).is_some()
}

/// A translated struct or enum, and how it is represented.
#[derive(Clone)]
pub struct Adt {
	pub sym: SymId,
	pub repr: AdtRepr,
}

#[derive(Clone)]
pub enum AdtRepr {
	/// A struct: a plain object, or a class if it may drop, whose fields are named at print time.
	Struct { ctor: Ctor, fields: Vec<FieldId> },
	/// An enum whose variants carry no data: a string per variant.
	Tags,
	/// An enum: an object per variant, tagged by its name.
	Union,
	/// A union: an object of slots, and each field's parts in them, as `unions` has it.
	Slots(unions::Fields),
}

/// A JS object that a `&mut` can change in place.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
	Class,
	Bytes,
	Array,
	Vec,
	Closure,
}

/// The JS values a type's own can be that `Option` and `Poll` erase to: `None` is
/// `undefined`, or `rt.None` where `Some` can already be `undefined`, and `Pending` is
/// `rt.Pending`, or `rt.OuterPending` where `Ready` can already be `rt.Pending`.
#[derive(Clone, Copy, Default)]
pub struct Erased {
	pub undefined: bool,
	pub none: bool,
	pub pending: bool,
	pub outer_pending: bool,
	/// A type parameter's value, which each instance decides.
	pub param: bool,
}

impl Erased {
	/// What an `Option` of a type erased so erases to: its `None` is `undefined`, or
	/// `rt.None` where `Some` can already be `undefined`.
	fn option(self) -> Self {
		match self.undefined {
			true => Self { none: true, ..self },
			false => Self {
				undefined: true,
				..self
			},
		}
	}

	/// What a `Poll` of a type erased so erases to: its `Pending` is `rt.Pending`, or
	/// `rt.OuterPending` where `Ready` can already be `rt.Pending`.
	fn poll(self) -> Self {
		match self.pending {
			true => Self {
				outer_pending: true,
				..self
			},
			false => Self { pending: true, ..self },
		}
	}
}

impl<'a, 'tcx> Translator<'a, 'tcx> {
	/// A translator that has run `reach`, with what it copies and refuses settled. Whether
	/// code writes into a type in place decides that, and of another crate's code, only what
	/// translation reaches runs, and of a generic body, only the instances it reaches, which
	/// it knows once it has reached them. So it runs again while it reaches a body whose
	/// writes it didn't read, or an instance of one writing a type parameter that it didn't
	/// judge by. A closure's clone may come before or after the closure is made, which
	/// makes it as its clones need, so it runs again too when it reaches a new one.
	fn settle<R>(
		tcx: TyCtxt<'tcx>,
		config: &'a Config,
		census: bool,
		mut reach: impl FnMut(&mut Self) -> R,
	) -> Option<(Self, R)> {
		let mut read = rustc_data_structures::fx::FxHashSet::default();
		let mut cloned = rustc_data_structures::fx::FxHashSet::default();
		loop {
			let mut t = Self::new(tcx, config, census, read, cloned)?;
			t.bind();
			let out = reach(&mut t);
			let reached: Vec<Instance<'tcx>> = t
				.instances
				.keys()
				.chain(&t.lowered)
				.copied()
				.filter(|&instance| t.crates.translates(instance.def_id()) && t.writes.unread(instance))
				.collect();
			if reached.is_empty() && t.cloning.is_subset(&t.cloned) {
				return Some((t, out));
			}
			read = std::mem::take(&mut t.read);
			read.extend(reached);
			cloned = std::mem::take(&mut t.cloned);
			cloned.extend(t.cloning.drain());
		}
	}

	/// Note a clone of the closure `def_id` that can't share it.
	pub fn clone_closure(&mut self, def_id: DefId) {
		self.cloning.insert(def_id);
	}

	/// Whether a clone of the closure `def_id` that can't share it was reached, so it is
	/// made by a function of its captures.
	pub fn clones_closure(&self, def_id: DefId) -> bool {
		self.cloned.contains(&def_id)
	}

	fn new(
		tcx: TyCtxt<'tcx>,
		config: &'a Config,
		census: bool,
		read: rustc_data_structures::fx::FxHashSet<Instance<'tcx>>,
		cloned: rustc_data_structures::fx::FxHashSet<DefId>,
	) -> Option<Self> {
		if tcx.data_layout.pointer_size().bits() != 32 {
			tcx.dcx().err(
				"rust2-ts: translation needs a 32-bit target, such as wasm32-unknown-unknown, so usize is a number",
			);
			return None;
		}
		Some(Self {
			tcx,
			config,
			crates: Crates::new(tcx),
			program: Program::default(),
			instances: Default::default(),
			queue: VecDeque::new(),
			adts: Default::default(),
			sources: Default::default(),
			failed: false,
			env: TypingEnv::fully_monomorphized(),
			census,
			refusals: Some(Vec::new()),
			drops: Default::default(),
			drops_open: Default::default(),
			drops_cycle: false,
			glued: Default::default(),
			coroutines: Default::default(),
			objects: Default::default(),
			object_classes: Default::default(),
			drop_shims: Default::default(),
			copy_shims: Default::default(),
			values: Default::default(),
			reading: Default::default(),
			deferred: Default::default(),
			// Once the bindings are known.
			writes: Default::default(),
			read,
			lowered: Default::default(),
			cloned,
			cloning: Default::default(),
			held: false,
			sites: Default::default(),
			site: None,
			bindings: Default::default(),
		})
	}

	/// Whether a struct's values are plain objects rather than its class's.
	pub fn plain(&self, sym: SymId) -> bool {
		self.program.plain.contains(&sym)
	}

	/// A struct's value, from its fields' values in declaration order.
	pub fn construct(&self, sym: SymId, ctor: &Ctor, fields: &[FieldId], values: Vec<ts::Expr>) -> ts::Expr {
		let class = Box::new(ts::Expr::Sym(sym));
		match ctor {
			_ if self.plain(sym) => ts::Expr::Object(
				fields
					.iter()
					.zip(values)
					.map(|(f, v)| (ts::Key::Field(*f), v))
					.collect(),
			),
			Ctor::Unit => ts::Expr::New(class, Vec::new()),
			Ctor::Positional => ts::Expr::New(class, values),
		}
	}

	/// Translate every queued function instance, and those they reach.
	///
	/// A census leaves an instance over type parameters to its own function's turn, since
	/// its bounds are that function's, not the caller's. Translation never reaches one. Each
	/// other lowers as translation lowers it, even one a function lowered over its type
	/// parameters reached.
	fn drain(&mut self) {
		let env = std::mem::replace(&mut self.env, TypingEnv::fully_monomorphized());
		while let Some((instance, sym)) = self.queue.pop_front() {
			if !self.census || !instance.args.has_param() {
				self.function(instance, sym);
			}
		}
		self.env = env;
	}

	/// Refuse a reached crate whose files another crate's would be: another crate's modules
	/// go under its name, which a module at the package's root or another version of the
	/// crate could share.
	fn crate_dirs(&mut self) {
		let tcx = self.tcx;
		let reached: std::collections::BTreeSet<CrateNum> = self
			.instances
			.keys()
			.map(|instance| instance.def_id().krate)
			.chain(self.adts.keys().map(|def_id| def_id.krate))
			.filter(|&krate| krate != rustc_hir::def_id::LOCAL_CRATE)
			.collect();
		let mut names = rustc_data_structures::fx::FxHashMap::default();
		for &krate in &reached {
			let name = tcx.crate_name(krate);
			if names.insert(name, krate).is_some() {
				self.err(
					tcx.def_span(krate.as_def_id()),
					format!("more than one version of `{name}` is reached, whose modules would share their files"),
				);
			}
		}
		for child in tcx.module_children_local(rustc_hir::def_id::CRATE_DEF_ID) {
			if let rustc_hir::def::Res::Def(DefKind::Mod, def_id) = child.res
				&& def_id.is_local()
				&& names.contains_key(&child.ident.name)
			{
				self.err(
					tcx.def_span(def_id),
					format!(
						"module `{}` would share its files with the crate of its name",
						child.ident
					),
				);
			}
		}
	}

	/// Refuse a construct outside the subset: inside a body of std's, at the crate's call.
	pub fn err(&mut self, span: Span, msg: impl Into<String>) {
		self.failed = true;
		let (span, msg) = match self.site {
			Some((site, def_id)) => (site, format!("{}, in `{}`", msg.into(), self.tcx.def_path_str(def_id))),
			None => (span, msg.into()),
		};
		match &mut self.refusals {
			Some(refusals) => refusals.push((span, msg)),
			None => _ = self.tcx.dcx().span_err(span, format!("rust2-ts: {msg}")),
		}
	}

	fn typing_env(&self) -> TypingEnv<'tcx> {
		self.env
	}

	/// Whether a census is lowering a function over its own type parameters. Only an
	/// instance decides what a parameter is, so nothing that hinges on one is refused
	/// then: each instance translation reaches is lowered, and refused, on its own.
	pub fn polymorphic(&self) -> bool {
		self.env != TypingEnv::fully_monomorphized()
	}

	/// Whether `ty` is a type parameter, or a projection from one, that only an instance
	/// decides.
	pub fn deferred(&self, ty: Ty<'tcx>) -> bool {
		self.polymorphic() && matches!(ty.kind(), ty::Param(_) | ty::Alias(..))
	}

	pub fn typing_env_pub(&self) -> TypingEnv<'tcx> {
		self.typing_env()
	}

	fn root(&mut self, path: &str) {
		let Some(def_id) = self.config_item(path) else {
			return;
		};
		match self.tcx.def_kind(def_id) {
			DefKind::Fn => {
				if self.tcx.generics_of(def_id).requires_monomorphization(self.tcx) {
					self.err(self.tcx.def_span(def_id), "a root must not be generic");
					return;
				}
				let instance = Instance::mono(self.tcx, def_id);
				self.instance(instance);
			}
			DefKind::Struct | DefKind::Enum => {
				if !self.config_type(def_id) {
					return;
				}
				let self_ty = self.tcx.type_of(def_id).instantiate_identity().skip_norm_wip();
				// Its inherent methods, and its impls of the crate's own traits: another
				// crate's trait, such as a derived `Debug`, comes along only when called or
				// printable.
				let mut impls: Vec<DefId> = self.tcx.inherent_impls(def_id).to_vec();
				for (trait_id, locals) in self.tcx.all_local_trait_impls(()) {
					if !trait_id.is_local() {
						continue;
					}
					for local in locals {
						let impl_ty = self
							.tcx
							.type_of(local.to_def_id())
							.instantiate_identity()
							.skip_norm_wip();
						if impl_ty == self_ty {
							impls.push(local.to_def_id());
						}
					}
				}
				for imp in impls {
					if self.tcx.generics_of(imp).requires_monomorphization(self.tcx) {
						continue;
					}
					for item in self.tcx.associated_items(imp).in_definition_order() {
						if item.is_fn() && !self.tcx.generics_of(item.def_id).requires_monomorphization(self.tcx) {
							self.instance(Instance::mono(self.tcx, item.def_id));
						}
					}
				}
				self.blanket_impls(self_ty);
			}
			kind => self.err(
				self.tcx.def_span(def_id),
				format!("a root must be a function or a type, not {kind:?}"),
			),
		}
	}

	/// A type whose `Display` and `Debug` impls come along, whichever it has, so callers
	/// can print it.
	fn printable(&mut self, path: &str) {
		let tcx = self.tcx;
		let Some(def_id) = self.config_item(path) else {
			return;
		};
		let kind = tcx.def_kind(def_id);
		if !matches!(kind, DefKind::Struct | DefKind::Enum) {
			self.err(
				tcx.def_span(def_id),
				format!("a printable item must be a type, not {kind:?}"),
			);
			return;
		}
		if !self.config_type(def_id) {
			return;
		}
		let args = ty::GenericArgs::for_item(tcx, def_id, |_, _| tcx.lifetimes.re_erased.into());
		let self_ty = Ty::new_adt(tcx, tcx.adt_def(def_id), args);
		let mut found = false;
		for name in [rustc_span::sym::Display, rustc_span::sym::Debug] {
			let Some(fmt) = tcx.get_diagnostic_item(name).and_then(|trait_id| {
				tcx.associated_items(trait_id)
					.in_definition_order()
					.find(|item| item.is_fn())
			}) else {
				continue;
			};
			if let Ok(Some(instance)) =
				Instance::try_resolve(tcx, self.typing_env(), fmt.def_id, tcx.mk_args(&[self_ty.into()]))
			{
				self.instance(instance);
				found = true;
			}
		}
		if !found {
			self.err(
				tcx.def_span(def_id),
				"a printable type must implement `Display` or `Debug`",
			);
		}
	}

	/// The item a config path names, or `None` after reporting that it names none.
	fn config_item(&mut self, path: &str) -> Option<DefId> {
		let segments: Vec<&str> = path.split("::").collect();
		let def_id = bind::resolve_in(self.tcx, rustc_hir::def_id::CRATE_DEF_ID.to_def_id(), &segments)
			.filter(|def_id| def_id.is_local());
		let Some(def_id) = def_id else {
			self.tcx.dcx().err(format!("rust2-ts: no item `{path}` in the crate"));
			self.failed = true;
			return None;
		};
		if self.bound(def_id) {
			self.err(self.tcx.def_span(def_id), "a root or printable item can't be bound");
			return None;
		}
		Some(def_id)
	}

	/// Translate a type the config names, which must not be generic.
	fn config_type(&mut self, def_id: DefId) -> bool {
		if self.tcx.generics_of(def_id).requires_monomorphization(self.tcx) {
			self.err(
				self.tcx.def_span(def_id),
				"a root or printable type must not be generic",
			);
			return false;
		}
		self.adt(self.tcx.adt_def(def_id), rustc_span::DUMMY_SP);
		true
	}

	/// The methods of blanket impls of the crate's traits that hold for `self_ty`, such
	/// as `impl<T: Message> Encode<Version> for T`.
	fn blanket_impls(&mut self, self_ty: Ty<'tcx>) {
		let tcx = self.tcx;
		for (trait_id, locals) in tcx.all_local_trait_impls(()) {
			if !trait_id.is_local() {
				continue;
			}
			for local in locals {
				let imp = local.to_def_id();
				let impl_ty = tcx.type_of(imp).instantiate_identity().skip_norm_wip();
				if !matches!(impl_ty.kind(), ty::Param(_)) {
					continue;
				}
				// Only the self parameter: any other would need choosing.
				let generics = tcx.generics_of(imp);
				let counts = generics.own_counts();
				if counts.types != 1 || counts.consts != 0 || generics.parent.is_some() {
					continue;
				}
				let args = tcx.mk_args_from_iter(generics.own_params.iter().map(|param| -> ty::GenericArg<'tcx> {
					match param.kind {
						ty::GenericParamDefKind::Lifetime => tcx.lifetimes.re_erased.into(),
						_ => self_ty.into(),
					}
				}));
				let trait_ref = tcx.impl_trait_ref(imp).instantiate(tcx, args).skip_norm_wip();
				for item in tcx.associated_items(imp).in_definition_order() {
					if !item.is_fn() || tcx.generics_of(item.def_id).own_requires_monomorphization() {
						continue;
					}
					let Some(trait_item) = item.trait_item_def_id() else {
						continue;
					};
					if let Ok(Some(instance)) =
						Instance::try_resolve(tcx, self.typing_env(), trait_item, trait_ref.args)
					{
						self.instance(instance);
					}
				}
			}
		}
	}

	/// The symbol of a function instance, queueing its translation the first time, or the
	/// export of a binding that stands in for it.
	pub fn instance(&mut self, instance: Instance<'tcx>) -> SymId {
		if let Some(&sym) = self.instances.get(&instance) {
			return sym;
		}
		let tcx = self.tcx;
		let def_id = instance.def_id();
		if let Some(sym) = self.bound_fn(instance, tcx.def_span(def_id)) {
			return sym;
		}
		let base = names::fn_base(tcx, def_id);
		// A function of a translated type sits in the type's own module, escaped only where it is
		// declared, so callers call it by its own name: `Decoder.new`.
		let owner = self.owner(instance);
		let (module, base) = match owner {
			Some(ty) => (self.program.syms[ty].module.clone(), base),
			None => (self.module_of(def_id), escape(&base)),
		};
		let sym = self.program.sym(Symbol {
			module,
			base,
			suffix: self.suffix(instance, owner.is_some()),
			owner: None,
			kind: SymKind::Function,
			order: self.order(tcx.def_span(def_id)),
			span: tcx.def_span(def_id),
			// Without trimmed paths, which rustc computes only for a diagnostic.
			instance: ty::print::with_no_trimmed_paths!(instance.to_string()),
			name: String::new(),
			alias: None,
		});
		self.instances.insert(instance, sym);
		self.queue.push_back((instance, sym));
		sym
	}

	/// The symbol of an instance a call at `span` reaches, noting where the crate first
	/// reached one of std's.
	pub fn callee(&mut self, instance: Instance<'tcx>, span: Span) -> SymId {
		if !self.crates.translates(instance.def_id()) {
			let site = self.site.map_or(span, |(site, _)| site);
			self.sites.entry(instance).or_insert(site);
		}
		self.instance(instance)
	}

	/// The type an associated function's impl or trait is for.
	pub fn self_ty(&self, instance: Instance<'tcx>) -> Option<Ty<'tcx>> {
		let def_id = instance.def_id();
		if let Some(imp) = self.tcx.impl_of_assoc(def_id) {
			return Some(self.subst(instance.args, self.tcx.type_of(imp).skip_binder()));
		}
		self.tcx.trait_of_assoc(def_id).map(|_| instance.args.type_at(0))
	}

	/// The translated type an associated function's impl or trait is for, if any.
	fn owner(&mut self, instance: Instance<'tcx>) -> Option<SymId> {
		let self_ty = self.self_ty(instance)?.peel_refs();
		let ty::Adt(adt, _) = self_ty.kind() else {
			return None;
		};
		if !self.crates.translates(adt.did()) {
			return None;
		}
		let span = self.tcx.def_span(instance.def_id());
		Some(self.adt(*adt, span).sym)
	}

	/// The Rust module `def_id` is in, from the crate root; another crate's is under a
	/// directory of its name, as `core/option` for `Option::map`.
	pub fn module_of(&self, def_id: DefId) -> Vec<String> {
		let tcx = self.tcx;
		let mut module = tcx.parent(def_id);
		while tcx.def_kind(module) != DefKind::Mod {
			module = tcx.parent(module);
		}
		let krate = (!def_id.is_local()).then(|| tcx.crate_name(def_id.krate).to_string());
		let path = tcx.def_path(module).data;
		krate
			.into_iter()
			.chain(
				path.iter()
					.filter_map(|part| part.data.get_opt_name().map(|name| name.to_string())),
			)
			.collect()
	}

	/// Where `span` is written, for sorting: where its macro is called, for an item a
	/// macro makes.
	pub fn order(&self, span: Span) -> u32 {
		span.source_callsite().lo().0
	}

	fn source(&mut self, file: &rustc_span::SourceFile) -> Option<u32> {
		let key = format!("{:?}", file.name);
		if let Some(&index) = self.sources.get(&key) {
			return index;
		}
		let index = match (file.name.clone().into_local_path(), file.src.as_deref()) {
			(Some(path), Some(text)) => {
				let path = std::path::absolute(&path).unwrap_or(path);
				self.program.sources.push(Source {
					path,
					text: text.to_string(),
				});
				Some(self.program.sources.len() as u32 - 1)
			}
			_ => None,
		};
		self.sources.insert(key, index);
		index
	}

	/// Where `span` starts, for the source map: the macro call for macro-made code.
	pub fn loc(&mut self, span: Span) -> Option<Loc> {
		if span.is_dummy() {
			return None;
		}
		let span = span.source_callsite();
		let sm = self.tcx.sess.source_map();
		let pos = sm.lookup_char_pos(span.lo());
		let file = self.source(&pos.file)?;
		let text = &self.program.sources[file as usize].text;
		let line_start = pos.file.lines()[pos.line - 1];
		let start = (line_start - rustc_span::RelativeBytePos(0)).0 as usize;
		let offset = (span.lo() - pos.file.start_pos).0 as usize;
		let col = text.get(start..offset).map_or(0, |s| s.encode_utf16().count()) as u32;
		Some(Loc {
			file,
			line: pos.line as u32 - 1,
			col,
		})
	}

	pub fn subst<T: ty::TypeFoldable<TyCtxt<'tcx>>>(&self, args: GenericArgsRef<'tcx>, value: T) -> T {
		self.tcx
			.instantiate_and_normalize_erasing_regions(args, self.typing_env(), EarlyBinder::bind(self.tcx, value))
	}

	/// `def_id`'s signature for `args`, with what its projections name, as `Deref::deref`'s
	/// `&Self::Target`, where they resolve: a census's call over type parameters may leave one.
	pub fn sig(&self, def_id: DefId, args: GenericArgsRef<'tcx>) -> ty::FnSig<'tcx> {
		let sig = self.tcx.fn_sig(def_id).instantiate(self.tcx, args).skip_norm_wip();
		let sig = self.tcx.instantiate_bound_regions_with_erased(sig);
		self.tcx
			.try_normalize_erasing_regions(self.typing_env(), ty::Unnormalized::new_wip(sig))
			.unwrap_or(sig)
	}

	/// The standard library type `ty` is, if it is one with its own representation.
	pub fn std_of(&self, ty: Ty<'tcx>) -> Option<library::Std> {
		match ty.kind() {
			ty::Adt(adt, _) => library::Std::of(self.tcx, adt.did()),
			_ => None,
		}
	}

	pub fn is_copy(&self, ty: Ty<'tcx>) -> bool {
		self.tcx.type_is_copy_modulo_regions(self.typing_env(), ty)
	}

	/// Whether a call of `def_id` with `args` runs a function whose body translates, the
	/// crate's own or std's from its MIR, rather than a mapping.
	pub fn translated(&self, def_id: DefId, args: GenericArgsRef<'tcx>) -> bool {
		match Instance::try_resolve(self.tcx, self.typing_env(), def_id, args) {
			Ok(Some(instance)) => self.lowers(instance) || self.dispatches(instance),
			_ => false,
		}
	}

	/// Whether `instance`'s body lowers from its MIR: the crate's own function, unless a
	/// binding stands in for it, or one of std's that `library` leaves to its MIR.
	pub fn lowers(&self, instance: Instance<'tcx>) -> bool {
		match instance.def {
			InstanceKind::Item(did) => {
				self.bound_module(instance).is_none()
					&& ((self.crates.translates(did) && !stock::maps(self.tcx, instance))
						|| library::from_mir(self, instance))
			}
			_ => false,
		}
	}

	/// Whether a call of `def_id` with `args` runs std's mapping, which takes a `&mut`
	/// parameter as what it points at, rather than a body that translates or a binding's export.
	pub fn mapped(&self, def_id: DefId, args: GenericArgsRef<'tcx>) -> bool {
		match Instance::try_resolve(self.tcx, self.typing_env(), def_id, args) {
			Ok(Some(instance)) => {
				!self.lowers(instance) && !self.dispatches(instance) && self.bound_module(instance).is_none()
			}
			_ => true,
		}
	}

	/// Whether a call of `def_id` with `args` calls a binding's export.
	pub fn binds(&self, def_id: DefId, args: GenericArgsRef<'tcx>) -> bool {
		match Instance::try_resolve(self.tcx, self.typing_env(), def_id, args) {
			Ok(Some(instance)) => self.bound_module(instance).is_some(),
			_ => false,
		}
	}

	/// A translated struct or enum, translating its definition the first time.
	pub fn adt(&mut self, adt: AdtDef<'tcx>, span: Span) -> Adt {
		if let Some(info) = self.adts.get(&adt.did()) {
			return info.clone();
		}
		// Its fields' types are refused where they are written, whatever reached it.
		let held = std::mem::replace(&mut self.held, false);
		let info = self.define(adt, span);
		self.held = held;
		info
	}

	fn define(&mut self, adt: AdtDef<'tcx>, span: Span) -> Adt {
		let tcx = self.tcx;
		let did = adt.did();
		let mut overlay = Vec::new();
		let repr = match () {
			_ if adt.is_struct() => {
				let variant = adt.non_enum_variant();
				// `struct E {}` and `struct E()` construct as `struct E;` does.
				let ctor = match variant.fields.is_empty() {
					true => Ctor::Unit,
					false => Ctor::Positional,
				};
				AdtRepr::Struct {
					ctor,
					fields: Vec::new(),
				}
			}
			_ if adt.is_enum() && adt.variants().iter().all(|v| v.fields.is_empty()) => AdtRepr::Tags,
			_ if adt.is_enum() => AdtRepr::Union,
			_ => match self.overlay(adt, span) {
				Ok((fields, slots)) => {
					overlay = slots;
					AdtRepr::Slots(fields)
				}
				Err(msg) => {
					self.err(span, msg);
					AdtRepr::Slots(vec![Vec::new(); adt.non_enum_variant().fields.len()])
				}
			},
		};
		let kind = match repr {
			AdtRepr::Struct { .. } => SymKind::Struct,
			_ => SymKind::Union,
		};
		let name = tcx.item_name(did).to_string();
		let sym = self.program.sym(Symbol {
			module: self.type_module(did),
			base: name,
			suffix: String::new(),
			owner: None,
			kind,
			order: self.order(tcx.def_span(did)),
			span: tcx.def_span(did),
			instance: String::new(),
			name: String::new(),
			alias: None,
		});
		// A struct is a plain object, unless it may drop: then it is a class, whose
		// `[Symbol.dispose]` whatever holds it calls without knowing its type. One holding a
		// type parameter may, through an instance.
		if matches!(repr, AdtRepr::Struct { .. })
			&& !self.drops(tcx.type_of(did).instantiate_identity().skip_norm_wip())
		{
			self.program.plain.insert(sym);
		}
		let mut info = Adt { sym, repr };
		if let AdtRepr::Struct { fields, .. } = &mut info.repr {
			for (i, field) in adt.non_enum_variant().fields.iter().enumerate() {
				fields.push(self.program.field(sym, field_name(field.name.as_str(), i)));
			}
		}
		self.adts.insert(did, info.clone());

		let params: Vec<String> = tcx
			.generics_of(did)
			.own_params
			.iter()
			.filter_map(|param| match param.kind {
				ty::GenericParamDefKind::Type { .. } => Some(param.name.to_string()),
				ty::GenericParamDefKind::Lifetime => None,
				ty::GenericParamDefKind::Const { .. } => {
					self.err(
						span,
						format!("const generic types are not supported: `{}`", tcx.def_path_str(did)),
					);
					None
				}
			})
			.collect();
		let identity = ty::GenericArgs::identity_for_item(tcx, did);
		let item = match &info.repr {
			AdtRepr::Struct { ctor, fields } => {
				let variant = adt.non_enum_variant();
				let fields = (variant.fields.iter().zip(fields))
					.map(|(field, &id)| {
						let span = tcx.def_span(field.did);
						ts::Field {
							id,
							ty: self.ty(field.ty(tcx, identity).skip_norm_wip(), span),
							doc: self.doc(field.did),
							init: ts::FieldInit::Ctor,
						}
					})
					.collect();
				Item::Struct {
					sym,
					params,
					fields,
					ctor: ctor.clone(),
					doc: self.doc(did),
				}
			}
			AdtRepr::Tags => Item::Union {
				sym,
				params,
				// An enum without variants has no values, and a union of no strings would
				// print as no type at all.
				ty: match adt.variants().is_empty() {
					true => Type::Builtin("never"),
					false => Type::Union(
						adt.variants()
							.iter()
							.map(|v| Type::StrLit(v.name.to_string()))
							.collect(),
					),
				},
				doc: self.doc(did),
			},
			AdtRepr::Slots(_) => Item::Union {
				sym,
				params,
				ty: self.overlay_ty(overlay, span),
				doc: self.doc(did),
			},
			AdtRepr::Union => {
				let variants = adt
					.variants()
					.iter()
					.map(|v| {
						let mut fields = vec![("tag".to_string(), Type::StrLit(v.name.to_string()))];
						for (i, field) in v.fields.iter().enumerate() {
							let span = tcx.def_span(field.did);
							let ty = self.ty(field.ty(tcx, identity).skip_norm_wip(), span);
							fields.push((field_name(field.name.as_str(), i), ty));
						}
						Type::Object(fields)
					})
					.collect();
				Item::Union {
					sym,
					params,
					ty: Type::Union(variants),
					doc: self.doc(did),
				}
			}
		};
		self.program.items.push(unnamed_params(item));
		self.drop_glue(adt, &info, span);
		info
	}

	/// Whether a `ty`'s JS type is a union of its variants', which TypeScript narrows a
	/// variable to the member it was assigned.
	pub fn narrows(&self, ty: Ty<'tcx>) -> bool {
		match ty.kind() {
			ty::Adt(adt, _) => match library::Std::of(self.tcx, adt.did()) {
				Some(
					library::Std::Option
					| library::Std::Poll
					| library::Std::Result
					| library::Std::ControlFlow
					| library::Std::Ordering
					| library::Std::ErrorKind,
				) => true,
				Some(_) => false,
				None => adt.is_enum() && adt.variants().len() > 1,
			},
			_ => false,
		}
	}

	/// What kind of JS object a `&mut` to `ty` can change in place, if any.
	pub fn object_kind(&self, ty: Ty<'tcx>) -> Option<ObjectKind> {
		match ty.kind() {
			ty::Array(elem, _) | ty::Slice(elem) => match elem.kind() {
				ty::Uint(ty::UintTy::U8) => Some(ObjectKind::Bytes),
				_ => Some(ObjectKind::Array),
			},
			// A JS array, whose elements change in place as an array's do.
			ty::Tuple(tys) if !tys.is_empty() => Some(ObjectKind::Array),
			// A binding's value: an object, unless Rust copies it, which JS can't.
			ty::Adt(adt, _) if self.bound_drops(adt.did()).is_some() => {
				(!self.is_copy(ty)).then_some(ObjectKind::Class)
			}
			ty::Adt(adt, args) => match library::Std::of(self.tcx, adt.did()) {
				Some(library::Std::Vec) => Some(ObjectKind::Vec),
				Some(
					library::Std::Formatter
					| library::Std::DebugStruct
					| library::Std::DebugTuple
					| library::Std::DebugList
					| library::Std::Arc
					| library::Std::Weak
					| library::Std::Mutex
					| library::Std::MutexGuard
					| library::Std::RefCell
					| library::Std::Ref
					| library::Std::RefMut
					| library::Std::Cell
					| library::Std::LocalKey
					| library::Std::Atomic
					| library::Std::OnceCell
					| library::Std::OnceLock
					| library::Std::Once
					| library::Std::OnceState
					| library::Std::LazyCell
					| library::Std::LazyLock
					| library::Std::Waker
					| library::Std::WakerVTable
					| library::Std::Context
					| library::Std::Pin
					| library::Std::PollFn
					| library::Std::Pending
					| library::Std::Range
					| library::Std::RangeInclusive
					| library::Std::RangeFrom
					| library::Std::Peekable
					| library::Std::HashMap
					| library::Std::HashSet
					| library::Std::BTreeMap
					| library::Std::BTreeSet
					| library::Std::VecDeque
					| library::Std::BinaryHeap
					| library::Std::PeekMut
					| library::Std::Entry
					| library::Std::Hasher
					| library::Std::BytesMut
					| library::Std::ThreadRng,
				) => Some(ObjectKind::Class),
				// A place, unless a pointer to what it holds is that object.
				Some(library::Std::MaybeUninit) => match self.pointee(args.type_at(0)) {
					pointer::Pointee::Object | pointer::Pointee::Bytes => self.object_kind(args.type_at(0)),
					pointer::Pointee::Place => None,
				},
				Some(_) => None,
				None if adt.is_struct() || adt.is_union() => Some(ObjectKind::Class),
				None => None,
			},
			// Its captures change through it, as a JS function's do.
			ty::Closure(..) => Some(ObjectKind::Closure),
			ty::Coroutine(..) | ty::Dynamic(..) => Some(ObjectKind::Class),
			_ if self.deferred(ty) => Some(ObjectKind::Class),
			_ => None,
		}
	}

	/// Whether a JS value of `ty` is a copy that no JS value can point at, such as a number
	/// or a reference, or an object its clones may share, as an `Arc`'s and a `Waker`'s are,
	/// pinned or boxed too, which JS erases, so a `&mut` to it is an `rt.Place`: where the
	/// value is, read and written through `v`.
	pub fn needs_place(&self, ty: Ty<'tcx>) -> bool {
		use library::Std;
		if self.object_kind(ty).is_none() {
			return true;
		}
		let mut erased = ty;
		while let ty::Adt(adt, args) = erased.kind()
			&& matches!(Std::of(self.tcx, adt.did()), Some(Std::Pin | Std::Box))
		{
			erased = args.type_at(0);
		}
		matches!(self.std_of(erased), Some(Std::Arc | Std::Waker))
	}

	/// What `ty` points at, if it is a `&mut` that is an `rt.Place`.
	pub fn place_pointee(&self, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
		match ty.kind() {
			ty::Ref(_, inner, rustc_ast::Mutability::Mut) if self.needs_place(*inner) => Some(*inner),
			ty::RawPtr(..) => self.raw_place(ty),
			_ => None,
		}
	}

	/// What `value`, a `ty` that is a reference, points at: an `rt.Place`'s `v`, and
	/// otherwise the reference's own JS value.
	pub fn pointee_value(&self, ty: Ty<'tcx>, value: ts::Expr) -> ts::Expr {
		match (self.place_pointee(ty), value.place_read()) {
			(Some(_), Some(read)) => read.clone(),
			(Some(_), None) => value.member("v"),
			(None, _) => value,
		}
	}

	/// Whether `ty` holds a `&mut` that is an `rt.Place`, other than in what a closure or
	/// coroutine captured.
	pub fn has_place(&self, ty: Ty<'tcx>) -> bool {
		let mut walker = ty.walk();
		while let Some(arg) = walker.next() {
			let Some(t) = arg.as_type() else { continue };
			if matches!(t.kind(), ty::Closure(..) | ty::Coroutine(..) | ty::CoroutineClosure(..)) {
				walker.skip_current_subtree();
			} else if self.place_pointee(t).is_some() {
				return true;
			}
		}
		false
	}

	/// The TypeScript type of a Rust type.
	pub fn ty(&mut self, ty: Ty<'tcx>, span: Span) -> Type {
		let tcx = self.tcx;
		match ty.kind() {
			ty::Bool => Type::Builtin("boolean"),
			ty::Int(_) | ty::Uint(_) => num::Int::of(ty).unwrap().ty(),
			_ if let Some(float) = float::Float::of(ty) => float.ty(),
			ty::Char => Type::Rt("char", Vec::new()),
			ty::Str => Type::Builtin("string"),
			ty::Never => Type::Builtin("never"),
			ty::Tuple(tys) if tys.is_empty() => Type::Builtin("undefined"),
			ty::Tuple(tys) => Type::Tuple(tys.iter().map(|t| self.ty(t, span)).collect()),
			ty::Ref(_, inner, rustc_ast::Mutability::Mut) if self.needs_place(*inner) => {
				Type::Rt("Place", vec![self.ty(*inner, span)])
			}
			ty::RawPtr(inner, _) => self.pointer_ty(*inner, span),
			ty::Ref(..) if bytes_ref(ty) => Type::Rt("ByteSlice", vec![]),
			ty::Ref(_, inner, _) => self.ty(*inner, span),
			ty::Array(elem, _) => match elem.kind() {
				ty::Uint(ty::UintTy::U8) => Type::Builtin("Uint8Array"),
				_ => Type::Array(Box::new(self.ty(*elem, span))),
			},
			ty::Slice(elem) => match elem.kind() {
				ty::Uint(ty::UintTy::U8) => Type::Rt("ByteSlice", vec![]),
				_ => Type::Array(Box::new(self.ty(*elem, span))),
			},
			ty::Param(param) => Type::Param(param.name.to_string()),
			ty::FnDef(def_id, args) => {
				let sig = self.sig(*def_id, args.skip_binder());
				self.fn_ty(sig.inputs(), sig.output(), span)
			}
			ty::FnPtr(..) => {
				let sig = tcx.instantiate_bound_regions_with_erased(ty.fn_sig(tcx));
				self.fn_ty(sig.inputs(), sig.output(), span)
			}
			ty::Closure(_, args) => {
				let sig = tcx.instantiate_bound_regions_with_erased(args.as_closure().sig());
				let inputs: Vec<Ty<'tcx>> = match sig.inputs() {
					[tupled] => tupled.tuple_fields().iter().collect(),
					other => other.to_vec(),
				};
				self.fn_ty(&inputs, sig.output(), span)
			}
			ty::Adt(adt, args) => self.adt_ty(*adt, args, span),
			ty::Pat(base, _) => self.ty(*base, span),
			ty::Coroutine(_, args) => self.future_ty(args.as_coroutine().return_ty(), span),
			ty::Dynamic(..) if let Some(output) = self.dyn_future(ty) => self.future_ty(output, span),
			// `&dyn Debug` and `&dyn Display` are values ready to write.
			ty::Dynamic(preds, ..)
				if preds
					.principal_def_id()
					.and_then(|did| fmt::Trait::of(tcx, did))
					.is_some() =>
			{
				Type::Rt("Fmt", vec![])
			}
			ty::Dynamic(..) if let Some(f) = self.dyn_fn(ty) => self.fn_ty(&f.inputs, f.output, span),
			ty::Dynamic(..) => match self.object(ty, span) {
				Some(object) => Type::Sym(object.sym, Vec::new()),
				None => Type::Builtin("unknown"),
			},
			_ if self.deferred(ty) => Type::Builtin("unknown"),
			_ => {
				self.unrepresented(span, format!("unsupported type `{ty}`"));
				Type::Builtin("unknown")
			}
		}
	}

	/// The TypeScript type of a variable that holds a `ty`. It refuses no type without a JS
	/// value, which is refused where a value of it is made or received: a parameter, a
	/// field, a call, or a constant.
	pub fn held_ty(&mut self, ty: Ty<'tcx>, span: Span) -> Type {
		let held = std::mem::replace(&mut self.held, true);
		let ts = self.ty(ty, span);
		self.held = held;
		ts
	}

	/// Refuse a type without a JS value, unless typing a variable.
	fn unrepresented(&mut self, span: Span, msg: String) {
		if !self.held {
			self.err(span, msg);
		}
	}

	/// Refuse each type in `ty` whose JS value can't tell two of its values apart, as an
	/// `Option` of what can already be `undefined`.
	pub fn check_erased(&mut self, ty: Ty<'tcx>, span: Span) {
		let mut msgs: Vec<String> = Vec::new();
		for part in ty.walk().filter_map(|arg| arg.as_type()) {
			if let Some(msg) = self.erasure_conflict(part)
				&& !msgs.contains(&msg)
			{
				msgs.push(msg);
			}
		}
		for msg in msgs {
			self.err(span, msg);
		}
	}

	/// Why a `ty`'s JS value, not looking inside its arguments, can't tell two of its
	/// values apart, if it can't.
	fn erasure_conflict(&self, ty: Ty<'tcx>) -> Option<String> {
		let ty::Adt(adt, args) = ty.kind() else {
			return None;
		};
		match library::Std::of(self.tcx, adt.did())? {
			library::Std::Option => {
				let inner = args.type_at(0);
				let erased = self.erases_to(inner);
				(erased.undefined && erased.none).then(|| {
					format!("`Option<{inner}>` is not supported: its `None` and `Some` would both be `rt.None`")
				})
			}
			library::Std::Poll => {
				let inner = args.type_at(0);
				let erased = self.erases_to(inner);
				(erased.pending && erased.outer_pending).then(|| {
					format!(
						"`Poll<{inner}>` is not supported: its `Pending` and `Ready` would both be `rt.OuterPending`"
					)
				})
			}
			library::Std::ControlFlow if library::Flow::of(self.tcx, args) == library::Flow::Option => {
				let erased = self.erases_to(args.type_at(1));
				(erased.undefined && erased.none)
					.then(|| format!("`{ty}` is not supported: its break and its continue would both be `rt.None`"))
			}
			_ => None,
		}
	}

	/// A function value's type, whose `()` result is `undefined`: TypeScript infers a
	/// `Vec<()>`'s items from it, and `void` is no `undefined`.
	fn fn_ty(&mut self, inputs: &[Ty<'tcx>], output: Ty<'tcx>, span: Span) -> Type {
		let params = inputs.iter().map(|t| self.ty(*t, span)).collect();
		Type::Func(params, Box::new(self.ty(output, span)))
	}

	/// The return type of a declared function returning `ty`, whose `()` is `void`.
	pub fn ret_ty(&mut self, ty: Ty<'tcx>, span: Span) -> Type {
		match ty.is_unit() {
			true => Type::Builtin("void"),
			false => self.ty(ty, span),
		}
	}

	fn adt_ty(&mut self, adt: AdtDef<'tcx>, args: GenericArgsRef<'tcx>, span: Span) -> Type {
		let tcx = self.tcx;
		if let Some(sym) = self.bound_type(adt, args, span) {
			let args = args.types().map(|t| self.ty(t, span)).collect();
			return Type::Sym(sym, args);
		}
		if let Some(std) = library::Std::of(tcx, adt.did()) {
			return match std {
				library::Std::Option => {
					if let Some(msg) = self.erasure_conflict(Ty::new_adt(tcx, adt, args)) {
						self.err(span, msg);
					}
					self.option_ty(args.type_at(0), span)
				}
				library::Std::Result => Type::Rt(
					"Result",
					vec![self.ty(args.type_at(0), span), self.ty(args.type_at(1), span)],
				),
				library::Std::Vec => match args.type_at(0).kind() {
					ty::Uint(ty::UintTy::U8) => Type::Rt("ByteVec", vec![]),
					_ => Type::Array(Box::new(self.ty(args.type_at(0), span))),
				},
				library::Std::String => Type::Builtin("string"),
				library::Std::Cow if library::string(tcx, Ty::new_adt(tcx, adt, args)) => Type::Builtin("string"),
				library::Std::Cow if let Some(elem) = library::cow_slice(tcx, Ty::new_adt(tcx, adt, args)) => {
					match elem.kind() {
						ty::Uint(ty::UintTy::U8) => Type::Rt("ByteCow", vec![]),
						_ => Type::Rt("Cow", vec![self.ty(elem, span)]),
					}
				}
				library::Std::Cow => {
					self.unrepresented(span, format!("`{}` is not supported yet", Ty::new_adt(tcx, adt, args)));
					Type::Builtin("unknown")
				}
				library::Std::Box
				| library::Std::Reverse
				| library::Std::NonZero
				| library::Std::Wrapping
				| library::Std::Saturating
				| library::Std::PoisonError => self.ty(args.type_at(0), span),
				library::Std::Bound => Type::Rt("RangeBound", vec![self.ty(args.type_at(0), span)]),
				library::Std::TryLockError => Type::Rt("TryLockError", vec![self.ty(args.type_at(0), span)]),
				library::Std::FromUtf8Error => Type::Rt("FromUtf8Error", vec![]),
				library::Std::Utf8Error => Type::Rt("Utf8Error", vec![]),
				library::Std::TryFromIntError => Type::Rt("TryFromIntError", vec![]),
				library::Std::ParseIntError => Type::Rt("ParseIntError", vec![]),
				library::Std::ErrorKind => match self.tcx.item_name(adt.did()).as_str() {
					"IntErrorKind" => Type::Rt("IntErrorKind", vec![]),
					"CharErrorKind" => Type::Rt("CharErrorKind", vec![]),
					"FloatErrorKind" => Type::Rt("FloatErrorKind", vec![]),
					_ => Type::Rt("TryFromFloatSecsErrorKind", vec![]),
				},
				library::Std::ParseBoolError => Type::Rt("ParseBoolError", vec![]),
				library::Std::ParseCharError => Type::Rt("ParseCharError", vec![]),
				library::Std::ParseFloatError => Type::Rt("ParseFloatError", vec![]),
				library::Std::TryFromFloatSecsError => Type::Rt("TryFromFloatSecsError", vec![]),
				library::Std::Formatter => Type::Rt("Formatter", vec![]),
				library::Std::Arguments => Type::Rt("Arguments", vec![]),
				library::Std::FmtError => Type::Rt("FmtError", vec![]),
				library::Std::DebugStruct => Type::Rt("DebugStruct", vec![]),
				library::Std::DebugTuple => Type::Rt("DebugTuple", vec![]),
				library::Std::DebugList => Type::Rt("DebugList", vec![]),
				library::Std::AssertKind => Type::Builtin("string"),
				library::Std::Arc => Type::Rt("Arc", vec![self.ty(args.type_at(0), span)]),
				library::Std::Weak => Type::Rt("Weak", vec![self.ty(args.type_at(0), span)]),
				// A guard's first argument is its lifetime.
				library::Std::Mutex | library::Std::MutexGuard => {
					let inner = args.types().next().expect("a guarded type");
					Type::Rt("Mutex", vec![self.ty(inner, span)])
				}
				library::Std::RefCell | library::Std::Ref | library::Std::RefMut => {
					let inner = args.types().next().expect("a borrowed type");
					Type::Rt("RefCell", vec![self.ty(inner, span)])
				}
				library::Std::Cell => Type::Rt("Cell", vec![self.ty(args.type_at(0), span)]),
				library::Std::LocalKey => Type::Rt("LocalKey", vec![self.ty(args.type_at(0), span)]),
				library::Std::Atomic => Type::Rt("Atomic", vec![self.ty(args.type_at(0), span)]),
				library::Std::AtomicOrdering => Type::Rt("AtomicOrdering", vec![]),
				// With the `Option`'s `None` where it is `rt.None`.
				std @ (library::Std::OnceCell | library::Std::OnceLock) => {
					let inner = args.type_at(0);
					if let Some(msg) = self.erasure_conflict(Ty::new_option(tcx, inner)) {
						self.err(span, msg);
					}
					let mut args = vec![self.ty(inner, span)];
					args.extend(self.erases_to(inner).undefined.then(|| Type::Rt("None", vec![])));
					let name = match std {
						library::Std::OnceCell => "OnceCell",
						_ => "OnceLock",
					};
					Type::Rt(name, args)
				}
				library::Std::Once => Type::Rt("Once", vec![]),
				library::Std::OnceState => Type::Rt("OnceState", vec![]),
				library::Std::LazyCell => Type::Rt("LazyCell", vec![self.ty(args.type_at(0), span)]),
				library::Std::LazyLock => Type::Rt("LazyLock", vec![self.ty(args.type_at(0), span)]),
				library::Std::Waker => Type::Rt("Waker", vec![]),
				library::Std::WakerVTable => Type::Rt("WakerVTable", vec![]),
				library::Std::Context => Type::Rt("Context", vec![]),
				library::Std::Poll => {
					if let Some(msg) = self.erasure_conflict(Ty::new_adt(tcx, adt, args)) {
						self.err(span, msg);
					}
					self.poll_ty(args.type_at(0), span)
				}
				library::Std::Pin => self.ty(args.type_at(0), span),
				library::Std::PollFn | library::Std::Pending => {
					let output = self.future_output(Ty::new_adt(tcx, adt, args), span);
					self.future_ty(output, span)
				}
				library::Std::Range => Type::Rt("Range", vec![self.ty(args.type_at(0), span)]),
				library::Std::RangeInclusive => Type::Rt("RangeInclusive", vec![self.ty(args.type_at(0), span)]),
				library::Std::RangeTo => Type::Object(vec![("end".into(), self.ty(args.type_at(0), span))]),
				library::Std::RangeToInclusive => Type::Object(vec![
					("end".into(), self.ty(args.type_at(0), span)),
					("inclusive".into(), Type::Builtin("true")),
				]),
				library::Std::RangeFrom => Type::Rt("RangeFrom", vec![self.ty(args.type_at(0), span)]),
				library::Std::RangeFull => Type::Builtin("undefined"),
				library::Std::Argument => Type::Rt("Fmt", vec![]),
				library::Std::NonNull => self.pointer_ty(args.type_at(0), span),
				library::Std::MaybeUninit => self.ty(args.type_at(0), span),
				library::Std::Peekable => {
					let item = self.iter_item(Ty::new_adt(tcx, adt, args));
					Type::Rt("Peekable", vec![self.item_ty(item, span)])
				}
				library::Std::Infallible => Type::Builtin("never"),
				// The key and value types, or the item type, without the hasher or allocator.
				library::Std::HashMap
				| library::Std::BTreeMap
				| library::Std::Entry
				| library::Std::HashSet
				| library::Std::BTreeSet
				| library::Std::VecDeque
				| library::Std::BinaryHeap
				| library::Std::PeekMut => {
					let (name, n) = match std {
						library::Std::HashMap => ("HashMap", 2),
						library::Std::BTreeMap => ("BTreeMap", 2),
						library::Std::Entry => ("Entry", 2),
						library::Std::HashSet => ("HashSet", 1),
						library::Std::BTreeSet => ("BTreeSet", 1),
						library::Std::BinaryHeap => ("BinaryHeap", 1),
						library::Std::PeekMut => ("PeekMut", 1),
						_ => ("VecDeque", 1),
					};
					let tys: Vec<Ty<'tcx>> = args.types().take(n).collect();
					Type::Rt(name, tys.into_iter().map(|t| self.ty(t, span)).collect())
				}
				library::Std::Ordering => Type::Rt("Ordering", vec![]),
				library::Std::Hasher => Type::Rt("Hasher", vec![]),
				library::Std::RandomState => Type::Rt("RandomState", vec![]),
				library::Std::Duration => Type::Rt("Duration", vec![]),
				library::Std::Bytes => Type::Rt("ByteSlice", vec![]),
				library::Std::BytesMut => Type::Rt("BytesMut", vec![]),
				library::Std::ThreadRng => Type::Rt("ThreadRng", vec![]),
				library::Std::ControlFlow => {
					if let Some(msg) = self.erasure_conflict(Ty::new_adt(tcx, adt, args)) {
						self.err(span, msg);
					}
					let (residual, value) = (args.type_at(0), args.type_at(1));
					match library::Flow::of(tcx, args) {
						library::Flow::Result => {
							let ty::Adt(_, residual) = residual.kind() else {
								unreachable!("a `Result` residual")
							};
							Type::Rt("Result", vec![self.ty(value, span), self.ty(residual.type_at(1), span)])
						}
						library::Flow::Option => self.option_ty(value, span),
						library::Flow::Other => Type::Rt("Result", vec![self.ty(value, span), self.ty(residual, span)]),
					}
				}
			};
		}
		if let Some(inner) = library::wrapped(tcx, Ty::new_adt(tcx, adt, args)) {
			return self.ty(inner, span);
		}
		// Nothing, as its value is.
		if library::nothing(tcx, Ty::new_adt(tcx, adt, args)) {
			return Type::Builtin("undefined");
		}
		// Every other iterator std has is an `rt.Iter` of its items.
		if !self.crates.translates(adt.did())
			&& let Some(item) = self.iter_item(Ty::new_adt(tcx, adt, args))
		{
			return Type::Rt("Iter", vec![self.ty(item, span)]);
		}
		if !self.crates.translates(adt.did()) {
			self.unrepresented(
				span,
				format!("unsupported type `{}` from another crate", tcx.def_path_str(adt.did())),
			);
			return Type::Builtin("unknown");
		}
		let info = self.adt(adt, span);
		if let Some(msg) = self.generic_erasure(adt, args) {
			self.err(span, msg);
		}
		let args = args.types().map(|t| self.ty(t, span)).collect();
		Type::Sym(info.sym, args)
	}

	/// Why an instance of the crate's generic `adt` with `args` can't hold its fields' values,
	/// if it can't: an `Option` or a `Poll` over a type parameter that its definition types
	/// `T | undefined` or `rt.Poll<T>`, whose `None` or `Pending` this instance's `T` makes
	/// `rt.None` or `rt.OuterPending`.
	fn generic_erasure(&self, adt: AdtDef<'tcx>, args: GenericArgsRef<'tcx>) -> Option<String> {
		let tcx = self.tcx;
		if !args.types().any(|t| {
			let erased = self.erases_to(t);
			erased.undefined || erased.pending
		}) {
			return None;
		}
		adt.all_fields().find_map(|field| {
			let generic = tcx.type_of(field.did).instantiate_identity().skip_norm_wip();
			generic.walk().filter_map(|arg| arg.as_type()).find_map(|part| {
				let ty::Adt(wrapper, wrapper_args) = part.kind() else {
					return None;
				};
				let std = library::Std::of(tcx, wrapper.did());
				if !matches!(std, Some(library::Std::Option | library::Std::Poll)) {
					return None;
				}
				let inner = wrapper_args.type_at(0);
				let erased = self.erases_to(inner);
				let instance = ty::EarlyBinder::bind(tcx, inner).instantiate(tcx, args).skip_norm_wip();
				let ty = Ty::new_adt(tcx, adt, args);
				match std {
					Some(library::Std::Option) if erased.param && !erased.undefined => {
						self.erases_to(instance).undefined.then(|| {
							format!(
								"`{ty}` is not supported: its field `{}` is `Option<{inner}>`, typed `{inner} | undefined`, whose `None` would be `rt.None` as `Option<{instance}>`",
								field.name
							)
						})
					}
					Some(library::Std::Poll) if erased.param && !erased.pending => {
						self.erases_to(instance).pending.then(|| {
							format!(
								"`{ty}` is not supported: its field `{}` is `Poll<{inner}>`, typed `rt.Poll<{inner}>`, whose `Pending` would be `rt.OuterPending` as `Poll<{instance}>`",
								field.name
							)
						})
					}
					_ => None,
				}
			})
		})
	}

	/// The type of an iterator's items, which every iterator has.
	fn item_ty(&mut self, item: Option<Ty<'tcx>>, span: Span) -> Type {
		match item {
			Some(item) => self.ty(item, span),
			None => {
				self.err(span, "an iterator without an item type");
				Type::Builtin("unknown")
			}
		}
	}

	/// The TypeScript type of an `Option<inner>`: `T | undefined`, or `T | rt.None` where
	/// `T` can already be `undefined`.
	fn option_ty(&mut self, inner: Ty<'tcx>, span: Span) -> Type {
		let none = match self.erases_to(inner).undefined {
			true => Type::Rt("None", vec![]),
			false => Type::Builtin("undefined"),
		};
		Type::Union(vec![self.ty(inner, span), none])
	}

	/// The TypeScript type of a future of `output`, an `rt.Future`, whose `poll` returns
	/// `rt.Pending` as its own `Pending`, so an `output` that can already be one is refused.
	fn future_ty(&mut self, output: Ty<'tcx>, span: Span) -> Type {
		if self.erases_to(output).pending {
			self.unrepresented(
				span,
				format!("a future of `{output}`, which can be `rt.Pending`, as the future's own `Pending` is"),
			);
		}
		Type::Rt("Future", vec![self.ty(output, span)])
	}

	/// The TypeScript type of a `Poll<inner>`: `rt.Poll<T>`, or `T | rt.OuterPending` where
	/// `T` can already be `rt.Pending`.
	fn poll_ty(&mut self, inner: Ty<'tcx>, span: Span) -> Type {
		let ready = self.ty(inner, span);
		match self.erases_to(inner).pending {
			true => Type::Union(vec![ready, Type::Rt("OuterPending", vec![])]),
			false => Type::Rt("Poll", vec![ready]),
		}
	}

	/// The JS value of `Pending` in a `Poll<inner>`: `rt.Pending`, or `rt.OuterPending`
	/// where `inner` can already be `rt.Pending`.
	pub fn pending(&self, inner: Ty<'tcx>) -> ts::Expr {
		match self.erases_to(inner).pending {
			true => ts::Expr::Rt("OuterPending".into()),
			false => ts::Expr::Rt("Pending".into()),
		}
	}

	/// The JS value of `None` in an `Option<inner>`, and of `Break` in a `ControlFlow`
	/// that `?` on one makes: `undefined`, or `rt.None` where `inner` can already be
	/// `undefined`.
	pub fn none(&self, inner: Ty<'tcx>) -> ts::Expr {
		self.none_arg(inner).unwrap_or_else(ts::Expr::undefined)
	}

	/// `rt.None`, if it is an `Option<inner>`'s `None`, which a runtime function taking
	/// one takes after its own arguments.
	pub fn none_arg(&self, inner: Ty<'tcx>) -> Option<ts::Expr> {
		self.erases_to(inner).undefined.then(|| ts::Expr::Rt("None".into()))
	}

	/// Which of the values `Option` and `Poll` erase to a `ty`'s JS value can already be,
	/// seen through the wrappers that erase to their contents.
	pub fn erases_to(&self, ty: Ty<'tcx>) -> Erased {
		match ty.kind() {
			_ if library::nothing(self.tcx, ty) => Erased {
				undefined: true,
				..Erased::default()
			},
			ty::Param(_) => Erased {
				param: true,
				..Erased::default()
			},
			// An `rt.Place`, which is an object.
			ty::Ref(..) if self.place_pointee(ty).is_some() => Erased::default(),
			ty::Ref(_, inner, _) => self.erases_to(*inner),
			ty::Adt(adt, args) => match library::Std::of(self.tcx, adt.did()) {
				Some(
					library::Std::Box
					| library::Std::Pin
					| library::Std::Reverse
					| library::Std::NonZero
					| library::Std::Wrapping
					| library::Std::Saturating
					| library::Std::PoisonError,
				) => self.erases_to(args.type_at(0)),
				// `undefined` while it holds nothing.
				Some(library::Std::MaybeUninit) => Erased {
					undefined: true,
					..self.erases_to(args.type_at(0))
				},
				Some(library::Std::Option) => self.erases_to(args.type_at(0)).option(),
				Some(library::Std::Poll) => self.erases_to(args.type_at(0)).poll(),
				Some(library::Std::ControlFlow) if library::Flow::of(self.tcx, args) == library::Flow::Option => {
					self.erases_to(args.type_at(1)).option()
				}
				_ => Erased::default(),
			},
			_ => Erased::default(),
		}
	}

	/// Translate a queued function instance.
	fn function(&mut self, instance: Instance<'tcx>, sym: SymId) {
		let tcx = self.tcx;
		let def_id = instance.def_id();
		let span = tcx.def_span(def_id);
		let InstanceKind::Item(_) = instance.def else {
			self.err(span, format!("unsupported function `{}`", tcx.def_path_str(def_id)));
			return;
		};
		if !self.lowers(instance) {
			self.err(span, format!("`{}` is from another crate", tcx.def_path_str(def_id)));
			return;
		}
		let std = !self.crates.translates(def_id);
		let site = std.then(|| (self.sites.get(&instance).copied().unwrap_or(span), def_id));
		let outer = std::mem::replace(&mut self.site, site);
		let function = self.lower_function(instance, sym, span);
		self.site = outer;
		if let Some(function) = function {
			if std {
				self.program.std.insert(sym);
			}
			self.program.items.push(Item::Function(function));
		}
	}

	fn lower_function(&mut self, instance: Instance<'tcx>, sym: SymId, span: Span) -> Option<ts::Function> {
		let tcx = self.tcx;
		let def_id = instance.def_id();
		let mut function = mir::function(self, instance)?;
		function.sym = sym;
		function.loc = self.loc(span);
		// A trait method's impl often leaves its docs to the trait.
		function.doc = self
			.doc(def_id)
			.or_else(|| tcx.trait_item_of(def_id).and_then(|item| self.doc(item)));
		Some(function)
	}

	/// The doc comment on an item, as written.
	fn doc(&self, def_id: DefId) -> Option<String> {
		if !self.crates.translates(def_id) {
			return None;
		}
		#[expect(deprecated, reason = "doc comments are no parsed attribute")]
		let lines: Vec<String> = self
			.tcx
			.get_all_attrs(def_id)
			.iter()
			.filter_map(|attr| attr.doc_str())
			.map(|doc| doc.to_string())
			.collect();
		let doc = lines.join("\n");
		(!doc.trim().is_empty()).then_some(doc)
	}

	/// Where the invocation of a left-out macro that made `span` is written, if one did:
	/// the outermost of `tracing`'s events, which expand to one another. Translated code
	/// installs no subscriber, and without one an event evaluates none of its fields and
	/// does nothing, so leaving it out is exact.
	pub fn ignored_site(&self, mut span: Span) -> Option<Span> {
		let mut site = None;
		while span.from_expansion() {
			let data = span.ctxt().outer_expn_data();
			if let Some(def_id) = data.macro_def_id
				&& self.tcx.crate_name(def_id.krate).as_str() == "tracing"
				&& matches!(
					self.tcx.item_name(def_id).as_str(),
					"event" | "trace" | "debug" | "info" | "warn" | "error"
				) {
				site = Some(data.call_site);
			}
			span = data.call_site;
		}
		site
	}
}

/// Whether `ty` is `[u8; N]`, which is a `Uint8Array`.
pub fn byte_array(ty: Ty<'_>) -> bool {
	matches!(ty.kind(), ty::Array(elem, _) if matches!(elem.kind(), ty::Uint(ty::UintTy::U8)))
}

/// Whether `ty` is a reference to a `[u8; N]`, which is an `rt.ByteSlice` of its bytes, as a
/// `&[u8]` is: a split's chunk is then no view.
pub fn bytes_ref(ty: Ty<'_>) -> bool {
	matches!(ty.kind(), ty::Ref(..)) && byte_array(ty.peel_refs())
}

/// `snake_case` to `camelCase`, keeping leading underscores.
pub fn camel(name: &str) -> String {
	let name = name.strip_prefix("r#").unwrap_or(name);
	let trimmed = name.trim_start_matches('_');
	let mut out: String = name[..name.len() - trimmed.len()].to_string();
	let mut upper = false;
	for ch in trimmed.chars() {
		match ch {
			'_' => upper = true,
			c if upper => {
				out.extend(c.to_uppercase());
				upper = false;
			}
			c => out.push(c),
		}
	}
	if trimmed.ends_with('_') {
		out.push('_');
	}
	out
}

/// `item` with each type parameter its type never names marked unused, as TS wants of
/// one only a `PhantomData` held.
fn unnamed_params(mut item: Item) -> Item {
	let (params, types): (&mut Vec<String>, Vec<&Type>) = match &mut item {
		Item::Struct { params, fields, .. } => (params, fields.iter().map(|f| &f.ty).collect()),
		Item::Union { params, ty, .. } => (params, vec![&*ty]),
		_ => return item,
	};
	for param in params.iter_mut() {
		if !types.iter().any(|t| t.names(param)) {
			*param = format!("_{param}");
		}
	}
	item
}

/// A field's property name: `camelCase`, or `_0` for a tuple field.
pub fn field_name(name: &str, index: usize) -> String {
	match name.parse::<usize>() {
		Ok(_) => format!("_{index}"),
		Err(_) => camel(name),
	}
}

/// A binding name that is not a JS reserved word.
pub fn escape(name: &str) -> String {
	const RESERVED: &[&str] = &[
		"arguments",
		"await",
		"break",
		"case",
		"catch",
		"class",
		"const",
		"continue",
		"debugger",
		"default",
		"delete",
		"do",
		"else",
		"enum",
		"eval",
		"export",
		"extends",
		"false",
		"finally",
		"for",
		"function",
		"if",
		"implements",
		"import",
		"in",
		"instanceof",
		"interface",
		"let",
		"new",
		"null",
		"package",
		"private",
		"protected",
		"public",
		"return",
		"static",
		"super",
		"switch",
		"this",
		"throw",
		"true",
		"try",
		"typeof",
		"undefined",
		"var",
		"void",
		"while",
		"with",
		"yield",
		"rt",
		// Globals the output calls.
		"Array",
		"Infinity",
		"Math",
		"NaN",
		"Number",
		"Object",
		"String",
		"Uint8Array",
	];
	match RESERVED.contains(&name) {
		true => format!("{name}_"),
		false => name.to_string(),
	}
}
