//! Names: the suffixes that tell instances apart, each a function of its instance alone,
//! and the census of names import bindings are chosen by: every name translation could
//! declare at a module's top level, in every crate it could translate from.

use rustc_hir::def::DefKind;
use rustc_hir::def_id::{DefId, DefIndex, LOCAL_CRATE};
use rustc_hir::definitions::{DefPathData, DefPathDataName};
use rustc_middle::ty::{self, GenericArg, Instance, Ty, TyCtxt};

use super::{Translator, camel, escape, library};
use crate::program::{ModPath, Names};

impl<'tcx> Translator<'_, 'tcx> {
	/// What tells this instance apart from others of its item, after a `$`: the items it is
	/// nested in below its module, the trait it implements with the trait's type arguments,
	/// then its self type and its own type arguments, each as [`ty`] spells it. In its
	/// type's module, `owned`, the type goes without saying, but not its arguments nor the
	/// references to it: `fmt_Display_copy_Pair` for `Wrap<copy::Pair>`'s `Display::fmt`.
	/// It depends on the instance alone, so reaching another renames nothing; empty for a
	/// function of its module that is neither generic, of a type, nor of a trait.
	pub(super) fn suffix(&self, instance: Instance<'tcx>, owned: bool) -> String {
		let tcx = self.tcx;
		let def_id = instance.def_id();
		// A thread-local key's initializer, which its base names by the key, by where the key is.
		let mut parts = enclosing(tcx, library::local_init(tcx, def_id).unwrap_or(def_id));
		let trait_ref = match tcx.trait_impl_of_assoc(def_id) {
			Some(imp) => Some(self.subst(instance.args, tcx.impl_trait_ref(imp).skip_binder())),
			// A trait's own method, which an impl did not override.
			None => tcx
				.trait_of_assoc(def_id)
				.map(|trait_id| ty::TraitRef::from_assoc(tcx, trait_id, instance.args)),
		};
		if let Some(trait_ref) = trait_ref {
			parts.push(trait_name(tcx, trait_ref));
		}
		if let Some(self_ty) = self.self_ty(instance) {
			let mut refs = Vec::new();
			let mut inner = self_ty;
			while let ty::Ref(_, pointee, mutability) = inner.kind() {
				refs.push(reference(*mutability).to_string());
				inner = *pointee;
			}
			match inner.kind() {
				ty::Adt(_, args) if owned => {
					parts.extend(refs);
					parts.extend(args.types().map(|t| ty(tcx, t)));
				}
				_ => parts.push(ty(tcx, self_ty)),
			}
		}
		let own = tcx.generics_of(def_id).parent_count;
		parts.extend(args(tcx, &instance.args[own..]));
		parts.join("_")
	}

	/// The module of the type or trait `def_id`, beside its Rust module, named for it and
	/// for any items it is nested in below that module: `copy.Pair` for `copy::Pair`, and
	/// `copy.Pair$run` for a `Pair` declared in `copy::run`.
	pub fn type_module(&self, def_id: DefId) -> ModPath {
		let mut name = self.tcx.item_name(def_id).to_string();
		let enclosing = enclosing(self.tcx, def_id);
		if !enclosing.is_empty() {
			name = format!("{name}${}", enclosing.join("_"));
		}
		crate::program::type_module(&self.module_of(def_id), &name)
	}

	/// Walk every definition of the crates that translate, private and nested ones too, and
	/// those of std's that lower from their MIR, as rustc walks the crate graph for its
	/// trimmed paths, naming each as translation would.
	pub(super) fn names(&self) -> Names {
		let tcx = self.tcx;
		let mut names = Names::default();
		// The copies and drops `lib.ts` makes.
		for shim in ["copy", "drop"] {
			importable(&mut names, shim, Vec::new());
		}
		for (module, namespace, prefix) in self.bound_files() {
			let mut path = module.clone();
			*path.last_mut().expect("a file name") = prefix.to_string();
			names.declare(namespace, module.clone(), path);
		}

		let cstore = rustc_metadata::creader::CStore::from_tcx(tcx);
		let mut crates: Vec<_> = tcx
			.crates(())
			.iter()
			.copied()
			.filter(|&krate| self.crates.crates.contains(&krate) || library::is_std(tcx, krate.as_def_id()))
			.collect();
		crates.push(LOCAL_CRATE);
		for krate in crates {
			let count = match krate {
				LOCAL_CRATE => tcx.iter_local_def_id().count(),
				_ => cstore.num_def_ids_untracked(krate),
			};
			for index in 0..count {
				let def_id = DefId {
					krate,
					index: DefIndex::from_usize(index),
				};
				if library::is_std(tcx, def_id) {
					self.std_names(def_id, &mut names);
				} else if self.own(def_id) && !self.bound(def_id) {
					self.own_names(def_id, &mut names);
				}
			}
		}
		names
	}

	/// What std's `def_id` declares: a function that lowers from its MIR, or a trait an
	/// object of which is a type's module.
	fn std_names(&self, def_id: DefId, names: &mut Names) {
		let tcx = self.tcx;
		match tcx.def_kind(def_id) {
			DefKind::Fn | DefKind::AssocFn if library::may_from_mir(tcx, def_id) => {
				importable(names, &escape(&fn_base(tcx, def_id)), self.module_of(def_id));
			}
			DefKind::Trait => self.namespace(def_id, names),
			_ => {}
		}
	}

	/// What the crate's `def_id` declares, and where.
	fn own_names(&self, def_id: DefId, names: &mut Names) {
		let tcx = self.tcx;
		let module = || self.module_of(def_id);
		match tcx.def_kind(def_id) {
			DefKind::Fn => {
				importable(names, &escape(&fn_base(tcx, def_id)), module());
				self.slot_names(def_id, names);
			}
			DefKind::Const | DefKind::Static { .. } | DefKind::AssocConst => {
				importable(names, &escape(tcx.item_name(def_id).as_str()), module())
			}
			DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::Trait => self.namespace(def_id, names),
			// A function of an impl or a trait, but for one of a type that translates, which
			// is in the type's module, and a trait's required one, which only an impl's is.
			DefKind::AssocFn => {
				let parent = tcx.parent(def_id);
				let named = match tcx.def_kind(parent) {
					DefKind::Impl { .. } => {
						let ty = tcx.type_of(parent).instantiate_identity().skip_norm_wip().peel_refs();
						!matches!(ty.kind(), ty::Adt(adt, _) if self.own(adt.did()))
					}
					_ => tcx.defaultness(def_id).has_value(),
				};
				if named {
					importable(names, &escape(&camel(tcx.item_name(def_id).as_str())), module());
					self.slot_names(def_id, names);
				}
			}
			// A coroutine's class, beside the function it is of.
			DefKind::Closure if tcx.coroutine_kind(def_id).is_some() => {
				let parent = tcx.typeck_root_def_id(def_id);
				if let Some(name) = tcx.opt_item_name(parent) {
					importable(names, &future_base(name.as_str()), self.module_of(parent));
				}
			}
			_ => {}
		}
	}

	/// The slot of `Ok` parts beside the function `def_id`, if it returns a `Result`.
	fn slot_names(&self, def_id: DefId, names: &mut Names) {
		let output = self.tcx.fn_sig(def_id).skip_binder().output().skip_binder();
		if matches!(output.kind(), ty::Adt(adt, _) if library::Std::of(self.tcx, adt.did()) == Some(library::Std::Result))
		{
			importable(names, &slot_base(self.tcx, def_id), self.module_of(def_id));
		}
	}

	/// A type's module, which importers bind whole by the type's name.
	fn namespace(&self, def_id: DefId, names: &mut Names) {
		let name = self.tcx.item_name(def_id);
		// One nested in a function goes by it too, as its module does.
		let mut path = self.module_of(def_id);
		path.extend(enclosing(self.tcx, def_id));
		let module = self.type_module(def_id);
		if path.is_empty() {
			path.push("lib".into());
		}
		names.declare(name.as_str(), module, path);
	}
}

/// A function's name before escaping and its suffix: a thread-local key's initializer by
/// its key, as the macro names it alike for every key.
pub fn fn_base(tcx: TyCtxt<'_>, def_id: DefId) -> String {
	match library::local_init(tcx, def_id) {
		Some(key) => format!("{}$init", tcx.item_name(key)),
		None => camel(tcx.item_name(def_id).as_str()),
	}
}

/// The name of the slot the function `def_id` hands back its `Ok`'s parts in.
pub fn slot_base(tcx: TyCtxt<'_>, def_id: DefId) -> String {
	format!("{}Out", camel(tcx.item_name(def_id).as_str()))
}

/// The name of the class of a coroutine of the function `name`.
pub fn future_base(name: &str) -> String {
	format!("{}Future", camel(name))
}

/// Note that `module` declares `name`, which importers qualify by its path: the root's is
/// `lib`, as its file is.
fn importable(names: &mut Names, name: &str, module: ModPath) {
	let path = match module.is_empty() {
		true => vec!["lib".into()],
		false => module.clone(),
	};
	names.declare(name, module, path);
}

/// A type as a suffix spells it: each type, trait, and closure by its [`path`], then its
/// type arguments; a reference as `ref` or `mut`, a slice as `slice`, an array as `array`
/// and its length, and a tuple as `tuple` and its length, before what they hold; all
/// joined by `_`: `ref_slice_u8` for `&[u8]`, and `Option_tuple2_u8_array4_u8` for
/// `Option<(u8, [u8; 4])>`.
pub fn ty<'tcx>(tcx: TyCtxt<'tcx>, t: Ty<'tcx>) -> String {
	let parts: Vec<String> = match t.kind() {
		// Its arguments, but not defaults such as a `Vec`'s allocator.
		ty::Adt(adt, adt_args) => {
			let adt_args = tcx.generics_of(adt.did()).own_args_no_defaults(tcx, adt_args);
			defined(tcx, adt.did(), adt_args)
		}
		ty::Ref(_, inner, mutability) => vec![reference(*mutability).into(), ty(tcx, *inner)],
		ty::RawPtr(inner, mutability) => {
			let ptr = match mutability {
				ty::Mutability::Not => "ptr",
				ty::Mutability::Mut => "mutPtr",
			};
			vec![ptr.into(), ty(tcx, *inner)]
		}
		// Named, since a suffix without its part would read as no instance.
		ty::Tuple(tys) if tys.is_empty() => vec!["unit".into()],
		ty::Never => vec!["never".into()],
		ty::Tuple(tys) => std::iter::once(format!("tuple{}", tys.len()))
			.chain(tys.iter().map(|t| ty(tcx, t)))
			.collect(),
		ty::Slice(elem) => vec!["slice".into(), ty(tcx, *elem)],
		ty::Array(elem, len) => vec![format!("array{}", konst(tcx, *len)), ty(tcx, *elem)],
		// By its trait, the types it binds, and its auto traits, as `dyn_Future_u64`.
		ty::Dynamic(preds, ..) => {
			let principal = preds.principal().map(|p| p.skip_binder());
			let bound = preds.projection_bounds().filter_map(|p| p.skip_binder().term.as_type());
			std::iter::once("dyn".to_string())
				.chain(principal.map(|p| path(tcx, p.def_id)))
				.chain(
					principal
						.iter()
						.flat_map(|p| p.args.types())
						.chain(bound)
						.map(|t| ty(tcx, t)),
				)
				.chain(preds.auto_traits().map(|def_id| path(tcx, def_id)))
				.collect()
		}
		// By where it is defined, not by its span, which code above it moves, with the type
		// arguments of the function it is in.
		ty::Closure(def_id, closure) => defined(tcx, *def_id, closure.as_closure().parent_args()),
		ty::Coroutine(def_id, coroutine) => defined(tcx, *def_id, coroutine.as_coroutine().parent_args()),
		ty::CoroutineClosure(def_id, closure) => defined(tcx, *def_id, closure.as_coroutine_closure().parent_args()),
		ty::FnDef(def_id, fn_args) => defined(tcx, *def_id, fn_args.skip_binder()),
		_ => vec![t.to_string().replace(|c: char| !c.is_alphanumeric(), "")],
	};
	parts.join("_")
}

/// `def_id`'s [`path`], then the type and const arguments of `def_args`.
fn defined<'tcx>(tcx: TyCtxt<'tcx>, def_id: DefId, def_args: &[GenericArg<'tcx>]) -> Vec<String> {
	std::iter::once(path(tcx, def_id)).chain(args(tcx, def_args)).collect()
}

/// The type and const arguments of `args`, as a suffix spells them.
fn args<'tcx>(tcx: TyCtxt<'tcx>, args: &[GenericArg<'tcx>]) -> impl Iterator<Item = String> {
	args.iter().filter_map(move |arg| match arg.kind() {
		ty::GenericArgKind::Type(t) => Some(ty(tcx, t)),
		ty::GenericArgKind::Const(c) => Some(konst(tcx, c)),
		ty::GenericArgKind::Lifetime(_) => None,
	})
}

/// A const argument, as an array's length: `4`.
fn konst<'tcx>(tcx: TyCtxt<'tcx>, c: ty::Const<'tcx>) -> String {
	match c.try_to_target_usize(tcx) {
		Some(n) => n.to_string(),
		None => c.to_string().replace(|c: char| !c.is_alphanumeric(), ""),
	}
}

/// What a suffix spells a reference as.
fn reference(mutability: ty::Mutability) -> &'static str {
	match mutability {
		ty::Mutability::Not => "ref",
		ty::Mutability::Mut => "mut",
	}
}

/// A trait with its type arguments, but not defaults such as `PartialEq`'s `Self`:
/// `cmp_PartialEq_ref_str`.
pub fn trait_name<'tcx>(tcx: TyCtxt<'tcx>, trait_ref: ty::TraitRef<'tcx>) -> String {
	let trait_args = tcx
		.generics_of(trait_ref.def_id)
		.own_args_no_defaults(tcx, trait_ref.args);
	std::iter::once(path(tcx, trait_ref.def_id))
		.chain(args(tcx, trait_args))
		.collect::<Vec<_>>()
		.join("_")
}

/// Where `def_id` is defined, as a suffix names it: the crate's own item by its module path
/// and the items it is nested in, as `copy_Pair` or `hints_typed_closure1` for
/// `hints::typed::{closure#1}`, with an impl as [`impl_name`] names it; another crate's
/// type, trait, or function by its public path under its crate, as `bytes_Bytes`, but std's
/// without its crate or a module its item repeats: `Option`, `fmt_Display`, `io_Error`.
pub fn path(tcx: TyCtxt<'_>, def_id: DefId) -> String {
	let mut parts = Vec::new();
	path_parts(tcx, def_id, &mut parts);
	parts.join("_")
}

fn path_parts(tcx: TyCtxt<'_>, def_id: DefId, parts: &mut Vec<String>) {
	let public = matches!(
		tcx.def_kind(def_id),
		DefKind::Struct
			| DefKind::Enum
			| DefKind::Union
			| DefKind::Trait
			| DefKind::TraitAlias
			| DefKind::TyAlias
			| DefKind::ForeignTy
			| DefKind::Fn
	);
	if public && !def_id.is_local() {
		parts.extend(public_path(tcx, def_id));
		return;
	}
	let key = tcx.def_key(def_id);
	match key.disambiguated_data.data {
		DefPathData::CrateRoot => {
			if !def_id.is_local() && !library::is_std(tcx, def_id) {
				parts.push(tcx.crate_name(def_id.krate).to_string());
			}
		}
		DefPathData::Impl => parts.push(impl_name(tcx, def_id)),
		// A tuple struct's constructor goes by the struct's name.
		DefPathData::Ctor => path_parts(tcx, tcx.parent(def_id), parts),
		_ => {
			path_parts(tcx, tcx.parent(def_id), parts);
			parts.push(segment(tcx, def_id));
		}
	}
}

/// A definition's name in what defines it, or a closure's as Rust numbers it there:
/// `closure1` for `{closure#1}`.
pub fn segment(tcx: TyCtxt<'_>, def_id: DefId) -> String {
	let data = tcx.def_key(def_id).disambiguated_data;
	match data.data.name() {
		DefPathDataName::Named(name) => name.to_string(),
		DefPathDataName::Anon { namespace } => format!("{namespace}{}", data.disambiguator),
	}
}

/// Another crate's item by the path rustc prints it by, through the modules it is public
/// in: std's without its crate, or a module its item repeats, as `option::Option`.
fn public_path(tcx: TyCtxt<'_>, def_id: DefId) -> Vec<String> {
	let printed = ty::print::with_no_trimmed_paths!(tcx.def_path_str(def_id));
	let mut segments: Vec<String> = printed
		.split("::")
		.map(|s| s.replace(|c: char| !c.is_alphanumeric() && c != '_', ""))
		.collect();
	if library::is_std(tcx, def_id) {
		segments.remove(0);
		let squashed = |s: &str| s.replace('_', "").to_lowercase();
		let mut i = 0;
		while i + 1 < segments.len() {
			match squashed(&segments[i]) == squashed(&segments[i + 1]) {
				true => _ = segments.remove(i),
				false => i += 1,
			}
		}
	}
	segments
}

/// An impl, as what is nested in it is named: its trait with the trait's type arguments,
/// then its self type, as the impl writes them: `fmt_Display_fmt_Wrap_T`.
fn impl_name(tcx: TyCtxt<'_>, imp: DefId) -> String {
	let self_ty = ty(tcx, tcx.type_of(imp).instantiate_identity().skip_norm_wip());
	match tcx.impl_opt_trait_ref(imp) {
		Some(trait_ref) => format!(
			"{}_{self_ty}",
			trait_name(tcx, trait_ref.instantiate_identity().skip_norm_wip())
		),
		None => self_ty,
	}
}

/// The items `def_id` is nested in below its module, as [`path`] names them: `u16s` for
/// `captures::u16s::through`. None for an item of a module, nor of an impl or a trait of
/// one, which a suffix names by its trait and self type.
pub fn enclosing(tcx: TyCtxt<'_>, def_id: DefId) -> Vec<String> {
	let mut parent = tcx.parent(def_id);
	if matches!(tcx.def_kind(parent), DefKind::Impl { .. } | DefKind::Trait) {
		parent = tcx.parent(parent);
	}
	let mut parts = Vec::new();
	below_module(tcx, parent, &mut parts);
	parts
}

fn below_module(tcx: TyCtxt<'_>, def_id: DefId, parts: &mut Vec<String>) {
	match tcx.def_kind(def_id) {
		DefKind::Mod => {}
		DefKind::Impl { .. } => parts.push(impl_name(tcx, def_id)),
		_ => {
			below_module(tcx, tcx.parent(def_id), parts);
			parts.push(segment(tcx, def_id));
		}
	}
}
