//! The `pub` API of the facade, as rustc checked it, to the [IR](crate::ir).
//!
//! Every `pub` item reachable from the facade's root, through its re-exports into other
//! crates too, is exported by fixed rules, or refused with its span. A type it does not
//! reach is refused. Nothing is opted in by attribute and nothing is skipped silently: the
//! config's `exclude` is the only way to leave an item out, and `#[doc(hidden)]` items,
//! which are no part of the documented API, what C can never hold, and traits C cannot
//! implement, which C never passes, are listed in the census. A generic type
//! exports at the instances it infers, before any type is mapped: its defaults, each
//! instance an inherent impl is of, its exports' implementations of the traits bounding it,
//! and each instance a signature names at any choice its bounds allow; and only there.

use std::collections::VecDeque;

use rustc_attr_ir::LangItem;
use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::{CrateNum, DefId, LOCAL_CRATE};
use rustc_infer::infer::TyCtxtInferExt;
use rustc_infer::traits::util::elaborate;
use rustc_infer::traits::{Obligation, ObligationCause};
use rustc_middle::ty::fast_reject::{TreatParams, simplify_type};
use rustc_middle::ty::{
	self, CallerBoundsIterator as _, GenericArgKind, Ty, TyCtxt, TypeSuperVisitable, TypeVisitable, TypeVisitableExt,
	TypeVisitor, TypingEnv,
};
use rustc_span::{DUMMY_SP, Span, sym};
use rustc_trait_selection::infer::InferCtxtExt;
use rustc_trait_selection::traits::ObligationCtxt;
use rustc_trait_selection::traits::query::evaluate_obligation::InferCtxtExt as _;

use crate::config;
use crate::ir::{
	Access, Api, Callback, Closure, Collection, Constant, Dyn, Enum, Family, Field, Function, Held, Home, Int, Literal,
	Mutability, Object, On, Param, Path, Receiver, Record, Task, TaskKind, Traits, Type, Variant,
};

/// Why something is refused: a construct, ranked by the census, and what it names.
#[derive(Clone, Debug)]
pub struct Why {
	pub construct: &'static str,
	pub detail: String,
	/// Where else the reason lies, and what is there.
	pub note: Option<(Span, String)>,
}

/// Types of crates that are no export, by crate and name, each exported as an object where
/// some crate builds with it, and named at the facade's root by its role: `url::Url` as
/// `Url` and `serde_json::Value`, which a `Serialize` or `Deserialize` parameter takes, as
/// `Json`, each made by parsing its text; and `std::time::Instant` as `Instant`, a value
/// only Rust makes, which C holds as its bytes and compares.
const FOREIGN: [(&str, &str, &str); 3] = [
	("url", "Url", "Url"),
	("serde_json", "Value", "Json"),
	("std", "Instant", "Instant"),
];

/// The construct of another crate's type named both by value and in an `Arc`, whose handle
/// holds the `Arc`.
const BOTH: &str = "another crate's type both by value and in an Arc";

/// The construct of an instance inference missed: a bug in the exporter, not the crate.
pub const NOT_INFERRED: &str = "instance not inferred";

/// What C can never hold: left out and listed in the census rather than refused, since
/// exporting it would need constructors or returns the crate has no use for.
const UNHELD: [&str; 4] = [
	"object no export returns",
	"value no export returns",
	"generic type no export instantiates",
	"type C never holds",
];

fn why(construct: &'static str, detail: impl Into<String>) -> Why {
	Why {
		construct,
		detail: detail.into(),
		note: None,
	}
}

/// Why a function cannot be called from safe foreign code, if it cannot: a C-variadic
/// one, or one whose ABI Rust cannot call (custom, interrupt, or GPU kernel), under its
/// own construct, else any `unsafe fn`, whose preconditions no binding can check.
fn unsafety(tcx: TyCtxt<'_>, def_id: DefId) -> Option<Why> {
	let sig = tcx.fn_sig(def_id).skip_binder().skip_binder();
	if sig.c_variadic() {
		Some(why("C-variadic function", ""))
	} else if uncallable(sig.abi()) {
		Some(why("uncallable ABI", sig.abi().to_string()))
	} else if sig.safety().is_unsafe() {
		Some(why("unsafe fn", ""))
	} else {
		None
	}
}

/// Whether Rust refuses to call a function of this ABI directly, as rustc's own call check
/// does for `CanonAbi::{Custom, Interrupt(_), GpuKernel}`.
fn uncallable(abi: rustc_abi::ExternAbi) -> bool {
	use rustc_abi::ExternAbi::*;
	matches!(
		abi,
		Custom
			| X86Interrupt
			| RiscvInterruptM
			| RiscvInterruptS
			| Msp430Interrupt
			| AvrInterrupt
			| AvrNonBlockingInterrupt
			| GpuKernel
			| PtxKernel
	)
}

/// Why foreign code cannot implement a trait soundly, if it cannot: an `unsafe trait`, or
/// a required method foreign code would implement without the caller's preconditions.
fn trait_unsafety(tcx: TyCtxt<'_>, def_id: DefId) -> Option<Why> {
	if tcx.trait_def(def_id).safety.is_unsafe() {
		return Some(why("unsafe trait", ""));
	}
	(tcx.associated_items(def_id).in_definition_order())
		.filter(|item| item.is_fn() && !item.defaultness(tcx).has_value())
		.find_map(|item| {
			unsafety(tcx, item.def_id).map(|why| Why {
				detail: item.name().to_string(),
				..why
			})
		})
}

/// An item the export refuses, with every reason found.
pub struct Refusal {
	/// Its path from its crate's root.
	pub path: String,
	/// What kind of item it is: `fn`, `method`, `struct`, ...
	pub kind: &'static str,
	pub span: Span,
	pub why: Vec<Why>,
}

/// What collection found.
pub struct Collected {
	pub api: Api,
	pub report: Report,
}

/// The facade, the local crate.
pub struct Facade<'a> {
	/// The first segment of its items' paths in the IR.
	pub root: String,
	pub package: &'a str,
	/// Paths from its root the config leaves out.
	pub exclude: &'a [String],
	/// The dependency the glue names each crate it builds with by, by its crate name.
	pub crates: &'a std::collections::BTreeMap<String, String>,
	/// Paths from its root a census checks, without leaving out what they name: each
	/// path one names is recorded in [`Report::audited`], as the exclude would have checked it.
	pub audit: &'a [String],
}

/// What collection found in the facade, by paths from its root.
pub struct Report {
	pub package: String,
	/// The facade's root, the first segment of its items' paths in the IR.
	pub root: String,
	pub refusals: Vec<Refusal>,
	/// Records and enums exported as objects, since a field does not map: the first.
	pub opaque: Vec<(String, Why)>,
	/// Traits no foreign implementation can implement, left out: the first reason.
	pub traits: Vec<(String, Why)>,
	/// `poll_x` methods that do not map, left out since `x` exports as a future.
	pub twins: Vec<String>,
	/// `#[doc(hidden)]` items, by path.
	pub hidden: Vec<String>,
	/// Items C can never hold or call, by path, and why.
	pub unheld: Vec<(String, Why)>,
	/// Excluded paths, and how many items each left out.
	pub excluded: Vec<(String, usize)>,
	/// A census's audited paths, and each path one names, whether or not that item exports.
	pub audited: Vec<(String, Vec<String>)>,
	/// Type aliases, which export nothing of their own.
	pub aliases: Vec<String>,
}

/// How an exported crate's type is exported, once decided.
#[derive(Clone)]
enum Decl {
	Record(usize),
	/// A `Copy` type C cannot see inside: a record of its bytes, numbered among records.
	Value(usize),
	Enum(usize),
	Object(usize),
	Callback(usize),
	Refused(Why),
}

/// A type or trait the export declares: an item, or a type with type parameters at one
/// instance, by its arguments with lifetimes erased.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key<'tcx> {
	def_id: DefId,
	args: Option<ty::GenericArgs<'tcx>>,
}

/// A type the export declares, and how it names it.
#[derive(Clone)]
struct Site<'tcx> {
	key: Key<'tcx>,
	/// Its generic arguments as exported, each lifetime `'static`.
	args: ty::GenericArgs<'tcx>,
	/// How many of its type and const arguments it is named by: each up to the last that is not its
	/// default, as Rust spells it.
	explicit: usize,
	/// Its item's path, which names its members in the IR.
	path: Path,
	/// Its path with an instance's arguments, which reports name it and its members by.
	shown: Path,
}

/// The arguments a function exports at: an instance, if a type parameter exports at
/// several types.
struct Instantiation<'tcx> {
	args: ty::GenericArgs<'tcx>,
	/// The type parameters that stay parameters, each taking a lent closure or a foreign
	/// implementation, which typing maps.
	kept: Vec<Ty<'tcx>>,
	/// The arguments of the type parameters that export at several types, and of the const
	/// parameters, which name it.
	named: Vec<ty::GenericArg<'tcx>>,
}

struct Collector<'tcx> {
	tcx: TyCtxt<'tcx>,
	/// Each `pub` item and its shortest path, in the order the walk met them.
	items: Vec<(DefId, Path)>,
	/// How many leading segments of each item's path lie outside its crate, as
	/// [`Home::outside`] counts them.
	outside: rustc_data_structures::fx::FxHashMap<DefId, usize>,
	/// Each crate's [`Home::krate`].
	keys: rustc_data_structures::fx::FxHashMap<CrateNum, String>,
	decls: rustc_data_structures::fx::FxHashMap<Key<'tcx>, Decl>,
	/// The instances generic types export at, each type's by how reports name them.
	instances: Vec<Site<'tcx>>,
	/// Instances that do not export, and why.
	rejected: rustc_data_structures::fx::FxHashMap<Key<'tcx>, Why>,
	/// Whether instances are still being inferred, before any type is declared: each type
	/// then counts as mapping, so inference takes every choice typing could.
	inferring: bool,
	/// The crates of the items the walk reached, the facade's first.
	crates: Vec<CrateNum>,
	/// How many of `crates` the walk from the root reached, before any reached type's.
	walked: usize,
	report: Report,
	/// Items the walk left out, and why: `excluded` or `doc(hidden)`.
	left_out: rustc_data_structures::fx::FxHashMap<DefId, &'static str>,
	/// The type parameters of the function being mapped that take a foreign
	/// implementation or a lent closure, with the type each takes.
	impls: std::cell::RefCell<Vec<(Ty<'tcx>, Type)>>,
	/// Whether the return being mapped may be a borrow of an object that is not `Clone`,
	/// lent from the receiver's handle.
	lending: std::cell::Cell<bool>,
	bytes: Option<DefId>,
	/// `serde_json::Map`, a map of `serde_json::Value`s.
	json_map: Option<DefId>,
	/// The standard library's crates, `core`, `alloc`, and `std`, known by what each
	/// defines, since the facade or another crate may share their names.
	sysroot: Vec<CrateNum>,
	/// Each type in [`FOREIGN`] that some crate builds with and no export is, and each other
	/// crate's type a signature names behind an `Arc` or by value, and the object it exports as.
	foreign: Vec<(DefId, usize)>,
	/// The types of [`Self::foreign`] signatures name by value, which their handles hold
	/// rather than an `Arc` of them.
	owned: Vec<DefId>,
	/// Types signatures name both by value and behind an `Arc`: their handles hold the `Arc`,
	/// so the value is refused.
	both: Vec<DefId>,
	/// `tracing::Level`, where some crate builds with it and no export is it.
	level: Option<Level>,
	/// `std::ops::Bound`, and `std::ops::RangeBounds`, which names it.
	bound: Option<DefId>,
	range_bounds: Option<DefId>,
	/// Objects no export returns, left out.
	orphans: rustc_data_structures::fx::FxHashSet<Key<'tcx>>,
	/// Types a signature names that the root does not reach, each exported by its public
	/// path in its own crate.
	reach: Vec<DefId>,
	/// The glue's dependency on each crate, as [`Facade::crates`].
	dependencies: std::collections::BTreeMap<String, String>,
	/// Such types this pass met that `reach` lacks.
	unreached: std::cell::RefCell<Vec<DefId>>,
	/// Types an opaque `Arc` holds that a signature also names bare, which stay refused
	/// rather than export twice.
	barred: Vec<DefId>,
	/// Whether the type being mapped is a `Result`'s error, which never reaches a type.
	erring: std::cell::Cell<bool>,
	/// Whether the type being mapped is only tried, as a candidate or before another rule,
	/// whose failure reaches no type.
	probing: std::cell::Cell<bool>,
	api: Api,
}

/// `tracing::Level`, exported as a plain enum of its five levels, and `LevelFilter`, as an
/// `Option<Level>` whose `None` is `OFF`.
#[derive(Clone, Copy)]
struct Level {
	level: DefId,
	filter: DefId,
	/// Its enum, once declared.
	index: usize,
}

/// Every crate, each after those it depends on, the facade's last.
fn order(tcx: TyCtxt<'_>) -> Vec<CrateNum> {
	let mut order: Vec<CrateNum> = (tcx.postorder_cnums(()).iter())
		.copied()
		.filter(|&k| k != LOCAL_CRATE)
		.collect();
	order.push(LOCAL_CRATE);
	order
}

/// Collect the facade's API, leaving out what its `exclude` names.
pub fn collect<'tcx>(tcx: TyCtxt<'tcx>, facade: &Facade<'_>) -> Collected {
	// A type at a crate's root, by the crate's name and its own.
	let root = |krate: &str, name: &str| {
		let krate = *tcx.crates(()).iter().find(|&&k| tcx.crate_name(k).as_str() == krate)?;
		(tcx.module_children(krate.as_def_id()).iter())
			.find(|child| child.ident.as_str() == name)
			.and_then(|child| child.res.opt_def_id())
	};
	let bytes = root("bytes", "Bytes");
	let json_map = root("serde_json", "Map");
	let sysroot: Vec<CrateNum> = [
		tcx.lang_items().sized_trait(),
		tcx.lang_items().owned_box(),
		tcx.get_diagnostic_item(rustc_span::Symbol::intern("HashMap")),
	]
	.into_iter()
	.flatten()
	.map(|def_id| def_id.krate)
	.collect();
	// std's is a diagnostic item, as `std::time` holds it below the crate's root.
	let foreign: Vec<DefId> = (FOREIGN.iter())
		.filter_map(|&(krate, name, _)| match krate {
			"std" => tcx.get_diagnostic_item(rustc_span::Symbol::intern(name)),
			_ => root(krate, name),
		})
		.collect();
	let level = root("tracing_core", "Level").zip(root("tracing_core", "LevelFilter"));
	let range_bounds = tcx.get_diagnostic_item(rustc_span::Symbol::intern("RangeBounds"));
	// `RangeBounds::start_bound` returns a `Bound`, which has no diagnostic item of its own.
	let bound = range_bounds.and_then(|tr| {
		let start = tcx
			.associated_items(tr)
			.in_definition_order()
			.find(|item| item.name().as_str() == "start_bound")?;
		match tcx.fn_sig(start.def_id).skip_binder().skip_binder().output().kind() {
			ty::Adt(adt, _) => Some(adt.did()),
			_ => None,
		}
	});
	// A later crate of an earlier one's name takes `#` and how many have it so far.
	let mut keys = rustc_data_structures::fx::FxHashMap::default();
	let mut seen: std::collections::HashMap<rustc_span::Symbol, usize> = Default::default();
	for krate in order(tcx) {
		let name = tcx.crate_name(krate);
		let count = seen.entry(name).or_default();
		*count += 1;
		let key = match *count {
			1 => name.to_string(),
			n => format!("{name}#{n}"),
		};
		keys.insert(krate, key);
	}
	let pass = |orphans: &rustc_data_structures::fx::FxHashSet<Key<'tcx>>, reach: &[DefId], barred: &[DefId]| {
		let mut c = Collector {
			tcx,
			items: Vec::new(),
			outside: Default::default(),
			keys: keys.clone(),
			decls: Default::default(),
			crates: vec![LOCAL_CRATE],
			walked: 1,
			report: Report {
				package: facade.package.to_string(),
				root: facade.root.clone(),
				refusals: Vec::new(),
				opaque: Vec::new(),
				traits: Vec::new(),
				twins: Vec::new(),
				hidden: Vec::new(),
				unheld: Vec::new(),
				excluded: facade.exclude.iter().map(|path| (path.clone(), 0)).collect(),
				audited: facade.audit.iter().map(|path| (path.clone(), Vec::new())).collect(),
				aliases: Vec::new(),
			},
			instances: Vec::new(),
			rejected: Default::default(),
			inferring: true,
			left_out: Default::default(),
			impls: Default::default(),
			lending: Default::default(),
			bytes,
			json_map,
			sysroot: sysroot.clone(),
			foreign: Vec::new(),
			owned: Vec::new(),
			both: Vec::new(),
			level: None,
			bound,
			range_bounds,
			orphans: orphans.clone(),
			reach: reach.to_vec(),
			dependencies: facade.crates.clone(),
			unreached: Default::default(),
			barred: barred.to_vec(),
			erring: Default::default(),
			probing: Default::default(),
			api: Api {
				records: Vec::new(),
				enums: Vec::new(),
				objects: Vec::new(),
				callbacks: Vec::new(),
				functions: Vec::new(),
				constants: Vec::new(),
				crates: Vec::new(),
				reached: Vec::new(),
				targets: vec![target(tcx)],
			},
		};
		c.walk();
		// A foreign type the facade re-exports is its own export, whatever else of its
		// crate the facade reaches.
		c.foreign = (foreign.iter())
			.filter(|&&ty| !c.items.iter().any(|(item, _)| *item == ty))
			.map(|&ty| (ty, 0))
			.collect();
		c.level = level
			.filter(|&(level, filter)| {
				!c.walked(level.krate) && !c.items.iter().any(|(item, _)| *item == level || *item == filter)
			})
			.map(|(level, filter)| Level {
				level,
				filter,
				index: 0,
			});
		c.infer();
		c.declare();
		c.define();
		c.prune();
		c.rehome();
		unshare(&mut c.api);
		c
	};
	// Leaving out an object no export returns leaves out what takes it, which may leave another
	// object no export returns, so collection repeats until it refuses no more. A type the
	// root does not reach is walked from its defining path in the next pass, whose
	// signatures may name more.
	let mut orphans = rustc_data_structures::fx::FxHashSet::default();
	let mut reach = Vec::new();
	let mut barred = Vec::new();
	loop {
		let c = pass(&orphans, &reach, &barred);
		// A reached type an opaque `Arc` also holds would export twice, so it stays refused,
		// as it was before its crate was reached.
		let held: Vec<DefId> = (reach.iter().copied())
			.filter(|&did| c.foreign.iter().any(|&(other, _)| other == did))
			.collect();
		if !held.is_empty() {
			reach.retain(|did| !held.contains(did));
			barred.extend(held);
			orphans.clear();
			continue;
		}
		let more = c.orphans();
		let unreached: Vec<DefId> = (c.unreached.take().into_iter())
			.filter(|&did| !c.foreign.iter().any(|&(other, _)| other == did))
			.collect();
		if more.is_empty() && unreached.is_empty() {
			return Collected {
				api: c.api,
				report: c.report,
			};
		}
		// A newly reached type may return an object marked an orphan, so those start over.
		if unreached.is_empty() {
			orphans.extend(more);
		} else {
			reach.extend(unreached);
			orphans.clear();
		}
	}
}

impl<'tcx> Collector<'tcx> {
	/// What collection found, and `path` without the facade's root.
	fn report(&mut self, path: &[String]) -> (&mut Report, String) {
		(&mut self.report, path[1..].join("::"))
	}

	/// Where the item `def_id` the walk reached belongs, before any instance moves on.
	fn home(&self, def_id: DefId) -> Home {
		let tcx = self.tcx;
		// An associated item, whose path goes through its type's, is at home with it.
		let item = match self.outside.contains_key(&def_id) {
			true => def_id,
			false => match tcx
				.type_of(tcx.parent(def_id))
				.instantiate_identity()
				.skip_norm_wip()
				.kind()
			{
				ty::Adt(adt, _) => adt.did(),
				other => unreachable!("an associated item of {other:?}"),
			},
		};
		Home {
			krate: self.keys[&item.krate].clone(),
			outside: self.outside[&item],
		}
	}

	/// Move each instance, and each function naming one, to the last crate among its own
	/// and those its arguments belong to, then list the crates in that order. An instance
	/// that leaves its crate shows its whole path there.
	fn rehome(&mut self) {
		let tcx = self.tcx;
		let order: Vec<String> = order(tcx).into_iter().map(|k| self.keys[&k].clone()).collect();
		let rank = |krate: &str| {
			order
				.iter()
				.position(|k| k == krate)
				.expect("each crate is in the order")
		};
		let api = &mut self.api;
		// An argument's home may move in turn, so this repeats until none does.
		loop {
			let later = |home: &Home, types: &mut dyn Iterator<Item = &Type>| -> Option<String> {
				let mut last = home.krate.clone();
				for ty in types {
					crate::ir::each(ty, &mut |named| {
						let krate = match named {
							Type::Record(i) => &api.records[*i].home.krate,
							Type::Enum(i) => &api.enums[*i].home.krate,
							Type::Object(i) => &api.objects[*i].home.krate,
							Type::Callback(i, _) => &api.callbacks[*i].home.krate,
							_ => return,
						};
						if rank(krate) > rank(&last) {
							last = krate.clone();
						}
					});
				}
				(last != home.krate).then_some(last)
			};
			let mut moves = Vec::new();
			for (i, r) in api.records.iter().enumerate() {
				moves.extend(later(&r.home, &mut r.args.iter()).map(|k| (0, i, k)));
			}
			for (i, e) in api.enums.iter().enumerate() {
				moves.extend(later(&e.home, &mut e.args.iter()).map(|k| (1, i, k)));
			}
			for (i, o) in api.objects.iter().enumerate() {
				moves.extend(later(&o.home, &mut o.args.iter()).map(|k| (2, i, k)));
			}
			for (i, f) in api.functions.iter().enumerate() {
				let mut types = (f.params.iter().map(|p| &p.ty)).chain([&f.ok]).chain(&f.err);
				moves.extend(later(&f.home, &mut types).map(|k| (3, i, k)));
			}
			if moves.is_empty() {
				break;
			}
			for (kind, i, krate) in moves {
				let home = match kind {
					0 => &mut api.records[i].home,
					1 => &mut api.enums[i].home,
					2 => &mut api.objects[i].home,
					_ => &mut api.functions[i].home,
				};
				*home = Home { krate, outside: 1 };
			}
		}
		// A method whose instance names a later crate's type belongs there too; any other
		// with its type.
		let method_homes = |owner: &Home, methods: &[Function]| -> Vec<Home> {
			(methods.iter())
				.map(|f| {
					let mut last = owner.krate.clone();
					let types = (f.params.iter().map(|p| &p.ty)).chain([&f.ok]).chain(&f.err);
					for ty in types {
						crate::ir::each(ty, &mut |named| {
							let krate = match named {
								Type::Record(i) => &api.records[*i].home.krate,
								Type::Enum(i) => &api.enums[*i].home.krate,
								Type::Object(i) => &api.objects[*i].home.krate,
								Type::Callback(i, _) => &api.callbacks[*i].home.krate,
								_ => return,
							};
							if rank(krate) > rank(&last) {
								last = krate.clone();
							}
						});
					}
					match last == owner.krate {
						true => owner.clone(),
						false => Home {
							krate: last,
							outside: 1,
						},
					}
				})
				.collect()
		};
		let records: Vec<Vec<Home>> = api.records.iter().map(|r| method_homes(&r.home, &r.methods)).collect();
		let enums: Vec<Vec<Home>> = api.enums.iter().map(|e| method_homes(&e.home, &e.methods)).collect();
		let objects: Vec<Vec<Home>> = api.objects.iter().map(|o| method_homes(&o.home, &o.methods)).collect();
		let callbacks: Vec<Vec<Home>> = api
			.callbacks
			.iter()
			.map(|c| method_homes(&c.home, &c.methods))
			.collect();
		let set = |methods: &mut [Function], homes: Vec<Home>| {
			for (method, home) in methods.iter_mut().zip(homes) {
				method.home = home;
			}
		};
		for (r, homes) in api.records.iter_mut().zip(records) {
			set(&mut r.methods, homes);
		}
		for (e, homes) in api.enums.iter_mut().zip(enums) {
			set(&mut e.methods, homes);
		}
		for (o, homes) in api.objects.iter_mut().zip(objects) {
			set(&mut o.methods, homes);
		}
		for (c, homes) in api.callbacks.iter_mut().zip(callbacks) {
			set(&mut c.methods, homes);
		}
		let homes = (api.records.iter().map(|r| &r.home.krate))
			.chain(api.enums.iter().map(|e| &e.home.krate))
			.chain(api.objects.iter().map(|o| &o.home.krate))
			.chain(api.callbacks.iter().map(|c| &c.home.krate))
			.chain(api.functions.iter().map(|f| &f.home.krate))
			.chain(api.constants.iter().map(|c| &c.home.krate));
		let homes: std::collections::BTreeSet<&String> = homes.collect();
		api.crates = order.iter().filter(|k| homes.contains(k)).cloned().collect();
	}

	/// Whether the walk reached an item of `krate`.
	fn exported(&self, krate: CrateNum) -> bool {
		self.crates.contains(&krate)
	}

	/// Whether the walk from the root reached an item of `krate`. A crate only signatures
	/// reach keeps the roles of one no export is: its errors are never reached, one in a
	/// `Result` crossing as its message, and an `Arc` of its type crosses as an opaque handle.
	fn walked(&self, krate: CrateNum) -> bool {
		self.crates[..self.walked].contains(&krate)
	}

	/// Whether the config excludes `path`, counting the exclusion, or else recording it under
	/// each audited path that names it. A member of an instance is left out by its instance's
	/// path or its type's, which leaves it out of each.
	fn exclude(&mut self, path: &[String]) -> bool {
		let report = &mut self.report;
		for (_, key) in config::keys(&path[1..]) {
			if let Some((_, count)) = report.excluded.iter_mut().find(|(excluded, _)| *excluded == key) {
				*count += 1;
				return true;
			}
		}
		// Under every key that names it, so one census lists a broad exclude beside a narrow one.
		for (_, key) in config::keys(&path[1..]) {
			if let Some((_, named)) = report.audited.iter_mut().find(|(audited, _)| *audited == key) {
				named.push(path[1..].join("::"));
			}
		}
		false
	}

	/// Whether a type or trait takes type or const parameters: a type that does exports
	/// only at the instances it infers.
	fn typed(&self, def_id: DefId) -> bool {
		(self.tcx.generics_of(def_id).own_params.iter())
			.any(|p| p.name != rustc_span::kw::SelfUpper && !matches!(p.kind, ty::GenericParamDefKind::Lifetime))
	}

	/// The key of `def_id` at `args`: an instance's, if it takes type parameters.
	fn key(&self, def_id: DefId, args: ty::GenericArgs<'tcx>) -> Key<'tcx> {
		let args = self.typed(def_id).then(|| self.tcx.erase_and_anonymize_regions(args));
		Key { def_id, args }
	}

	/// How the type `def_id` is exported at `args`.
	fn decl(&self, def_id: DefId, args: ty::GenericArgs<'tcx>) -> Option<&Decl> {
		self.decls.get(&self.key(def_id, args))
	}

	/// How a trait is exported.
	fn trait_decl(&self, def_id: DefId) -> Option<&Decl> {
		self.decls.get(&Key { def_id, args: None })
	}

	/// The types an item declares: itself, or each instance it exports at.
	fn sites(&self, def_id: DefId, path: &Path) -> Vec<Site<'tcx>> {
		let instances: Vec<Site<'tcx>> = (self.instances.iter())
			.filter(|site| site.key.def_id == def_id)
			.cloned()
			.collect();
		match instances.is_empty() {
			true => vec![Site {
				key: Key { def_id, args: None },
				args: statics(self.tcx, def_id),
				explicit: 0,
				path: path.clone(),
				shown: path.clone(),
			}],
			false => instances,
		}
	}

	/// The type a site declares.
	fn site_ty(&self, site: &Site<'tcx>) -> Ty<'tcx> {
		self.tcx
			.type_of(site.key.def_id)
			.instantiate(self.tcx, site.args)
			.skip_norm_wip()
	}

	/// How many of `def_id`'s type and const arguments `args` names it by, as Rust spells
	/// it: each up to the last that is not its default.
	fn explicit(&self, def_id: DefId, args: ty::GenericArgs<'tcx>) -> usize {
		let tcx = self.tcx;
		let params: Vec<&ty::GenericParamDef> = (tcx.generics_of(def_id).own_params.iter())
			.filter(|p| !matches!(p.kind, ty::GenericParamDefKind::Lifetime))
			.collect();
		let mut explicit = params.len();
		while let Some(param) = explicit.checked_sub(1).map(|i| params[i]) {
			let arg = args[param.index as usize];
			match param.default_value(tcx) {
				Some(default)
					if tcx.erase_and_anonymize_regions(default.instantiate(tcx, args).skip_norm_wip())
						== tcx.erase_and_anonymize_regions(arg) =>
				{
					explicit -= 1
				}
				_ => break,
			}
		}
		explicit
	}

	/// The first `explicit` type and const arguments of `args`, which name an instance.
	fn shown_args(args: ty::GenericArgs<'tcx>, explicit: usize) -> Vec<ty::GenericArg<'tcx>> {
		let named = args
			.iter()
			.filter(|arg| !matches!(arg.kind(), GenericArgKind::Lifetime(_)));
		named.take(explicit).collect()
	}

	/// A type or const argument as reports spell it.
	fn spell_arg(&self, arg: ty::GenericArg<'tcx>) -> String {
		match arg.kind() {
			GenericArgKind::Type(ty) => self.spell(ty),
			GenericArgKind::Const(ct) => self.const_arg(ct).map_or_else(|_| ct.to_string(), |c| c.to_string()),
			GenericArgKind::Lifetime(region) => region.to_string(),
		}
	}

	/// A const argument's value, which must be a `bool` or an integer that is not 128 bits.
	fn const_arg(&self, ct: ty::Const<'tcx>) -> Result<Literal, Why> {
		let tcx = self.tcx;
		let refused = || why("const argument", ct.to_string());
		let ty::ConstKind::Value(value) = ct.kind() else {
			return Err(refused());
		};
		let scalar = value.try_to_leaf().ok_or_else(refused)?;
		let bits = scalar.to_bits(scalar.size());
		match value.ty.kind() {
			ty::Bool => Ok(Literal::Bool(bits != 0)),
			_ => int(value.ty)
				.map(|int| Literal::Int(int, signed(tcx, value.ty, bits)))
				.ok_or_else(refused),
		}
	}

	/// A type as reports spell it, each export's by its path in the IR, and an instance by
	/// the arguments that name it.
	fn spell(&self, ty: Ty<'tcx>) -> String {
		let tcx = self.tcx;
		match ty.kind() {
			ty::Tuple(list) if list.is_empty() => "()".to_string(),
			ty::Adt(adt, _) if tcx.lang_items().string() == Some(adt.did()) => "String".to_string(),
			ty::Adt(adt, args) => {
				let path = match self.items.iter().find(|(def_id, _)| *def_id == adt.did()) {
					Some((_, path)) => path.join("::"),
					None => tcx.def_path_str(adt.did()),
				};
				let explicit = self.explicit(adt.did(), *args);
				let args: Vec<String> = (Self::shown_args(*args, explicit).into_iter())
					.map(|arg| self.spell_arg(arg))
					.collect();
				match args.is_empty() {
					true => path,
					false => format!("{path}<{}>", args.join(", ")),
				}
			}
			_ => ty.to_string(),
		}
	}

	/// Infer the instances each generic type exports at, with the instances inside each: its
	/// defaults, the instance each of its inherent impls is of, each choice among its default
	/// and the implementations of the traits bounding each parameter, and each instance a
	/// signature the export maps names, at every choice its bounds allow. An instance adds
	/// its own fields and methods, so this repeats until it infers no more, which it does:
	/// no instance holds one of its own type, so each is no deeper than the generic types.
	fn infer(&mut self) {
		let tcx = self.tcx;
		let mut candidates: Vec<Ty<'tcx>> = Vec::new();
		for (def_id, _) in self.items.clone() {
			if !matches!(tcx.def_kind(def_id), DefKind::Struct | DefKind::Enum) || !self.typed(def_id) {
				continue;
			}
			for &imp in tcx.inherent_impls(def_id) {
				let ty = tcx.type_of(imp).instantiate_identity().skip_norm_wip();
				if !ty.has_non_region_param() {
					candidates.push(ty);
				}
			}
			let mut choices: Vec<Vec<ty::GenericArg<'tcx>>> = vec![Vec::new()];
			for param in &tcx.generics_of(def_id).own_params {
				let implementors = match param.kind {
					ty::GenericParamDefKind::Type { .. } => {
						self.implementors(def_id, tcx.mk_param_from_def(param).expect_ty())
					}
					_ => Vec::new(),
				};
				choices = (choices.into_iter())
					.flat_map(|choice| {
						let options: Vec<ty::GenericArg<'tcx>> = match param.kind {
							ty::GenericParamDefKind::Lifetime => vec![tcx.lifetimes.re_static.into()],
							ty::GenericParamDefKind::Const { .. } => (param.default_value(tcx))
								.map(|default| default.instantiate(tcx, &choice[..]).skip_norm_wip())
								.into_iter()
								.collect(),
							ty::GenericParamDefKind::Type { .. } => (param.default_value(tcx))
								.map(|default| default.instantiate(tcx, &choice[..]).skip_norm_wip())
								.into_iter()
								.chain(implementors.iter().map(|&ty| ty.into()))
								.collect(),
						};
						options.into_iter().map(move |option| [&choice[..], &[option]].concat())
					})
					.collect();
			}
			let adt = tcx.adt_def(def_id);
			candidates.extend(
				choices
					.into_iter()
					.map(|args| Ty::new_adt(tcx, adt, tcx.mk_args(&args))),
			);
		}
		let mut seen = rustc_data_structures::fx::FxHashSet::default();
		loop {
			let inferred = self.instances.len();
			let named = self.named();
			// Each with the instances inside it, which its export needs.
			let types: Vec<Ty<'tcx>> = (candidates.iter().chain(&named))
				.flat_map(|ty| ty.walk())
				.filter_map(|arg| arg.as_type())
				.filter(|&ty| seen.insert(ty))
				.collect();
			for ty in candidates.iter().chain(&named) {
				self.held_forms(*ty);
			}
			for ty in types {
				match self.admit(ty) {
					Some((key, Ok(site))) if !self.instances.iter().any(|other| other.key == key) => {
						self.instances.push(site)
					}
					Some((_, Ok(_))) | None => {}
					Some((key, Err(why))) => {
						self.rejected.insert(key, why);
					}
				}
			}
			if self.instances.len() == inferred {
				break;
			}
		}
		self.instances.sort_by(|a, b| a.shown.cmp(&b.shown));
		self.inferring = false;
	}

	/// The type an `Arc` holds, if it exports as an opaque handle: a type of a crate no
	/// export is, the standard library and the types mapped by role aside, taking no type
	/// parameters, and `Send` and `Sync`, as a handle is shared across threads.
	fn opaque_arc(&self, ty: Ty<'tcx>) -> Option<DefId> {
		let tcx = self.tcx;
		let ty::Adt(arc, args) = ty.kind() else { return None };
		if !tcx.is_diagnostic_item(sym::Arc, arc.did()) {
			return None;
		}
		let inner = args.type_at(0);
		let ty::Adt(adt, inner_args) = inner.kind() else {
			return None;
		};
		let did = adt.did();
		let mapped = Some(did) == self.bytes || Some(did) == self.json_map || !self.opaque(did);
		let held = !self.walked(did.krate) && !self.sysroot.contains(&did.krate) && !mapped;
		(held && inner_args.is_empty() && self.is_send(inner, TypingEnv::fully_monomorphized()) && self.is_sync(inner))
			.then_some(did)
	}

	/// The type a handle holds by value, if it exports as an opaque handle: as
	/// [`Self::opaque_arc`]'s, a type of a crate the walk from the root does not reach,
	/// taking no type parameters, and `Send` and `Sync`, but not an iterator or a future,
	/// nor one that exports by its own crate's path. Its lifetimes are `'static`, as an object's are.
	fn opaque_value(&self, ty: Ty<'tcx>) -> Option<DefId> {
		let ty::Adt(adt, args) = ty.kind() else { return None };
		let did = adt.did();
		let mapped = Some(did) == self.bytes || Some(did) == self.json_map || !self.opaque(did);
		let held = !self.walked(did.krate) && !self.sysroot.contains(&did.krate) && !mapped;
		// Each lifetime is `'static` in the export, as an object's is.
		let erasable = args.iter().all(|arg| matches!(arg.kind(), GenericArgKind::Lifetime(_)));
		// Asked of a signature's type, whose lifetimes may be bound by it.
		let ty = ty::fold_regions(self.tcx, ty, |_, _| self.tcx.lifetimes.re_erased);
		// An iterator crosses as the array of its items instead, and a future as a future.
		let iterator = implements(self.tcx, ty, did, sym::Iterator);
		let future = (self.tcx.lang_items().future_trait()).is_some_and(|tr| {
			let (infcx, env) = (self.tcx.infer_ctxt()).build_with_typing_env(TypingEnv::fully_monomorphized());
			infcx.type_implements_trait(tr, [ty], env).must_apply_modulo_regions()
		});
		(held
			&& erasable
			&& !iterator
			&& !future
			&& self.is_send(ty, TypingEnv::fully_monomorphized())
			&& self.is_sync(ty)
			&& !self.reaches(ty, did))
		.then_some(did)
	}

	/// Whether `did`, the type of `ty`, exports with its API by its public path in its own
	/// crate once a signature names it: an error never does, crossing as its message.
	fn reaches(&self, ty: Ty<'tcx>, did: DefId) -> bool {
		self.reachable(did) && !implements(self.tcx, ty, did, rustc_span::Symbol::intern("Error"))
	}

	/// Whether `ty` is another crate's type that an opaque handle holds, by value or behind
	/// an `Arc`.
	fn is_opaque(&self, ty: Ty<'tcx>) -> bool {
		self.opaque_arc(ty).is_some() || self.opaque_value(ty).is_some()
	}

	/// Record each other crate's type `ty` names behind an `Arc` or by value, which an
	/// opaque handle holds: an `Arc`'s type is not also by value.
	fn held_forms(&mut self, ty: Ty<'tcx>) {
		let mut walker = ty.walk();
		while let Some(arg) = walker.next() {
			let Some(ty) = arg.as_type() else { continue };
			let (did, owned) = match (self.opaque_arc(ty), self.opaque_value(ty)) {
				(Some(did), _) => (did, false),
				(None, Some(did)) => (did, true),
				(None, None) => continue,
			};
			walker.skip_current_subtree();
			match self.foreign.iter().any(|&(other, _)| other == did) {
				false => {
					self.foreign.push((did, 0));
					if owned {
						self.owned.push(did);
					}
				}
				true if self.owned.contains(&did) != owned && !self.both.contains(&did) => {
					self.owned.retain(|&other| other != did);
					self.both.push(did);
				}
				true => {}
			}
		}
	}

	/// Whether a foreign type exports as an opaque handle rather than by its role in
	/// [`FOREIGN`].
	fn opaque(&self, did: DefId) -> bool {
		let tcx = self.tcx;
		let (krate, name) = (tcx.crate_name(did.krate), tcx.item_name(did));
		!FOREIGN
			.iter()
			.any(|&(k, n, _)| k == krate.as_str() && n == name.as_str())
	}

	/// Whether the foreign type `did` is a value C holds as its bytes, as a `Copy` one is.
	fn foreign_value(&self, did: DefId) -> bool {
		let tcx = self.tcx;
		let ty = tcx.type_of(did).instantiate_identity().skip_norm_wip();
		!self.opaque(did) && implements(tcx, ty, did, sym::Copy) && padding(tcx, ty).is_some()
	}

	/// Each type a signature the export maps names, bounds included, at every choice
	/// [`Self::instantiate`] takes while inferring: each function's, constant's, and trait
	/// method's, and each type's `pub` fields and the inherent methods and constants that
	/// apply to it, an instance's at its arguments.
	fn named(&mut self) -> Vec<Ty<'tcx>> {
		let tcx = self.tcx;
		let mut named = Vec::new();
		let mut sites = Vec::new();
		for (def_id, path) in self.items.clone() {
			match tcx.def_kind(def_id) {
				DefKind::Fn => named.extend(self.instantiated(def_id, &path, None)),
				DefKind::Const => named.push(tcx.type_of(def_id).instantiate_identity().skip_norm_wip()),
				DefKind::Trait => {
					for item in tcx.associated_items(def_id).in_definition_order() {
						if let ty::AssocKind::Fn { name, .. } = item.kind {
							let method = [&path[..], &[name.to_string()]].concat();
							named.extend(self.instantiated(item.def_id, &method, None));
						}
					}
				}
				DefKind::Struct | DefKind::Enum if !self.typed(def_id) => sites.extend(self.sites(def_id, &path)),
				_ => {}
			}
		}
		// `declare` refuses an instance whose arguments cannot name one, before mapping any
		// of its members, which could name ever larger instances, as `Grow<(T,)>` does.
		sites.extend(
			(self.instances.iter())
				.filter(|site| self.unarguable(site).is_none())
				.cloned(),
		);
		for site in sites {
			let adt = tcx.adt_def(site.key.def_id);
			// An object's private fields are no part of its API, nor are those the config
			// leaves out.
			for field in adt.all_fields().filter(|field| field.vis.is_public()) {
				let shown = [&site.shown[..], &[field.name.to_string()]].concat();
				if self.field_key(&site, field.name, &shown).is_some() {
					named.push(field.ty(tcx, site.args).skip_norm_wip());
				}
			}
			for (item, parent, shown, hidden) in self.members(&site) {
				match item.kind {
					_ if hidden => {}
					ty::AssocKind::Const { .. } => {
						named.push(tcx.type_of(item.def_id).instantiate(tcx, parent).skip_norm_wip())
					}
					_ => named.extend(self.instantiated(item.def_id, &shown, Some(parent))),
				}
			}
		}
		named
	}

	/// Why the first argument an instance is named by cannot map as [`Self::arg`] requires:
	/// a type that is not `()`, a primitive, `String`, `Bytes`, `Duration`, a `NonZero` of
	/// an exported integer, a foreign object, or an export's type, or a const that is not
	/// a `bool` or integer.
	fn unarguable(&self, site: &Site<'tcx>) -> Option<Why> {
		let tcx = self.tcx;
		let mut args = Self::shown_args(site.args, site.explicit).into_iter();
		args.find_map(|arg| {
			let ty = match arg.kind() {
				GenericArgKind::Const(ct) => return self.const_arg(ct).err(),
				GenericArgKind::Type(ty) => ty,
				GenericArgKind::Lifetime(_) => return None,
			};
			let unarguable = match ty.kind() {
				ty::Tuple(list) => !list.is_empty(),
				ty::Bool | ty::Char | ty::Float(_) => false,
				ty::Int(_) | ty::Uint(_) => int(ty).is_none(),
				ty::Adt(adt, args) if tcx.is_diagnostic_item(rustc_span::Symbol::intern("NonZero"), adt.did()) => {
					int(args.type_at(0)).is_none()
				}
				ty::Adt(adt, _) => {
					let mapped = tcx.lang_items().string() == Some(adt.did())
						|| self.bytes == Some(adt.did())
						|| tcx.is_diagnostic_item(rustc_span::Symbol::intern("Duration"), adt.did())
						|| self.exported(adt.did().krate)
						|| self.foreign.iter().any(|&(ty, _)| ty == adt.did());
					// A handle holds its value erased, so no instance names its type.
					!mapped || self.owned.contains(&adt.did())
				}
				_ => true,
			};
			unarguable.then(|| why("type argument", self.spell(ty)))
		})
	}

	/// The types a function's signature names, bounds included, at each instantiation it
	/// is not excluded at.
	fn instantiated(&mut self, def_id: DefId, shown: &Path, parent: Option<ty::GenericArgs<'tcx>>) -> Vec<Ty<'tcx>> {
		let tcx = self.tcx;
		let mut named = Vec::new();
		for instance in self.instantiate(def_id, parent).unwrap_or_default() {
			if self.shown(shown, &instance).is_none() {
				continue;
			}
			named.extend(self.spelled(def_id, instance.args));
			let clauses = tcx.clauses_of(def_id).instantiate(tcx, instance.args).clauses;
			let mut types = Types(Vec::new());
			for clause in clauses {
				clause.skip_norm_wip().visit_with(&mut types);
			}
			named.extend(types.0);
		}
		named
	}

	/// The path reports show an instance of a function by, from its function's `shown`,
	/// unless the config excludes it.
	fn shown(&mut self, shown: &Path, instance: &Instantiation<'tcx>) -> Option<Path> {
		let mut shown = shown.clone();
		if !instance.named.is_empty() {
			let spelled: Vec<String> = (instance.named.iter())
				.map(|&arg| match arg.kind() {
					GenericArgKind::Type(ty) => self.spell(ty.peel_refs()),
					_ => self.spell_arg(arg),
				})
				.collect();
			*shown.last_mut().expect("a path has a segment") += &format!("<{}>", spelled.join(", "));
			if self.exclude(&shown) {
				return None;
			}
		}
		Some(shown)
	}

	/// Each type a function's signature names at `args`, normalized, a returned future's
	/// being what it resolves to. A type holding a foreign future names no instance: the future
	/// that holds it, as `Timeout<F::IntoFuture>` does, is polled, never a handle.
	fn spelled(&self, def_id: DefId, args: ty::GenericArgs<'tcx>) -> Vec<Ty<'tcx>> {
		let tcx = self.tcx;
		let env = TypingEnv::post_analysis(tcx, def_id);
		let normalize = |ty: Ty<'tcx>| {
			tcx.try_normalize_erasing_regions(env, ty::Unnormalized::new_wip(ty))
				.unwrap_or(ty)
		};
		let sig = tcx.fn_sig(def_id).instantiate(tcx, args).skip_norm_wip();
		let sig = tcx.instantiate_bound_regions_with_erased(sig);
		// A returned future exports as what it resolves to, never as a handle, so the type
		// of the future itself, such as a closure's, names nothing.
		let output = self.future_output(normalize(sig.output()), env).unwrap_or(sig.output());
		(sig.inputs().iter().copied().chain([output]))
			.map(normalize)
			.flat_map(|ty| ty.walk())
			.filter_map(|arg| arg.as_type())
			.filter(|ty| {
				!ty.walk()
					.any(|arg| arg.as_type().is_some_and(|ty| self.is_foreign_future(ty)))
			})
			.collect()
	}

	/// The types the exports implement a trait bounding `param` of `def_id` for, one of
	/// the exports': each impl's in an export, whose type takes no type parameters and is
	/// `()`, a primitive, `String`, or an exported type. A blanket impl, such as
	/// `impl<S: Session> Boxable for S`, stands for the types implementing what it bounds
	/// its type by, which the caller checks the bounds of.
	fn implementors(&self, def_id: DefId, param: Ty<'tcx>) -> Vec<Ty<'tcx>> {
		let tcx = self.tcx;
		let bounding = |def_id: DefId, param: Ty<'tcx>| {
			(tcx.clauses_of(def_id).instantiate_identity(tcx).clauses.iter())
				.filter_map(|clause| clause.skip_norm_wip().as_trait_clause())
				.filter(move |bound| bound.self_ty().skip_binder() == param && self.exported(bound.def_id().krate))
				.map(|bound| bound.def_id())
				.collect::<Vec<_>>()
		};
		let mut traits = bounding(def_id, param);
		let mut found = Vec::new();
		let mut i = 0;
		while let Some(&tr) = traits.get(i) {
			i += 1;
			for imp in tcx.all_impls(tr) {
				let ty = tcx.type_of(imp).instantiate_identity().skip_norm_wip();
				if let ty::Param(_) = ty.kind() {
					for tr in bounding(imp, ty) {
						if !traits.contains(&tr) {
							traits.push(tr);
						}
					}
					continue;
				}
				let inner = ty.peel_refs();
				let named = inner.is_primitive()
					|| inner.is_unit()
					|| match inner.kind() {
						ty::Adt(adt, _) => {
							tcx.lang_items().string() == Some(adt.did())
								|| self.items.iter().any(|(item, _)| *item == adt.did())
						}
						_ => false,
					};
				if !self.exported(imp.krate) || ty.has_non_region_param() || !named {
					continue;
				}
				let ty = ty::fold_regions(tcx, ty, |_, _| tcx.lifetimes.re_static);
				if !found.contains(&ty) {
					found.push(ty);
				}
			}
		}
		found
	}

	/// The instance `ty` is, if it is an exported generic type at arguments without type
	/// parameters, or why it does not export: an argument holds an instance of a type
	/// inside it, which could otherwise name ever deeper instances, a bound is unmet, or
	/// the config excludes it.
	fn admit(&mut self, ty: Ty<'tcx>) -> Option<(Key<'tcx>, Result<Site<'tcx>, Why>)> {
		let tcx = self.tcx;
		let ty::Adt(adt, args) = *ty.kind() else {
			return None;
		};
		let def_id = adt.did();
		let (_, path) = self.items.iter().find(|(item, _)| *item == def_id)?.clone();
		if !matches!(tcx.def_kind(def_id), DefKind::Struct | DefKind::Enum)
			|| !self.typed(def_id)
			|| args.has_non_region_param()
		{
			return None;
		}
		let args = ty::fold_regions(tcx, args, |_, _| tcx.lifetimes.re_static);
		let key = self.key(def_id, args);
		let spelled = self.spell(ty);
		let nests = ty
			.walk()
			.filter_map(|arg| arg.as_type())
			.any(|outer| match outer.kind() {
				ty::Adt(outer, inner) => (inner.iter().flat_map(|arg| arg.walk())).any(
					|arg| matches!(arg.as_type().map(Ty::kind), Some(ty::Adt(inner, _)) if inner.did() == outer.did()),
				),
				_ => false,
			});
		if nests {
			return Some((
				key,
				Err(why("instance holding an instance of a type inside it", spelled)),
			));
		}
		let (infcx, env) = tcx.infer_ctxt().build_with_typing_env(TypingEnv::fully_monomorphized());
		for clause in tcx.clauses_of(def_id).instantiate(tcx, args).clauses {
			let clause = clause.skip_norm_wip();
			let obligation = Obligation::new(tcx, ObligationCause::dummy(), env, clause);
			if !infcx.predicate_must_hold_modulo_regions(&obligation) {
				return Some((key, Err(why("unmet bound of the instance", clause.to_string()))));
			}
		}
		let explicit = self.explicit(def_id, args);
		let mut shown = path.clone();
		if explicit > 0 {
			let spelled: Vec<String> = (Self::shown_args(args, explicit).into_iter())
				.map(|arg| self.spell_arg(arg))
				.collect();
			let last = shown.last_mut().expect("a path has a segment");
			*last = format!("{last}<{}>", spelled.join(", "));
		}
		if self.exclude(&shown) {
			return Some((key, Err(why("excluded type", spelled))));
		}
		let site = Site {
			key,
			args,
			explicit,
			path,
			shown,
		};
		Some((key, Ok(site)))
	}

	fn refuse(&mut self, path: &[String], kind: &'static str, span: Span, why: Vec<Why>) {
		let (report, path) = self.report(path);
		match why.iter().find(|why| UNHELD.contains(&why.construct)) {
			Some(unheld) => report.unheld.push((path, unheld.clone())),
			None => report.refusals.push(Refusal { path, kind, span, why }),
		}
	}

	/// Every `pub` item reachable from the facade's root, through re-exports of other
	/// crates' items and modules too, breadth first so each keeps its shortest path, which
	/// starts with the facade's root.
	fn walk(&mut self) {
		let tcx = self.tcx;
		let mut seen = rustc_data_structures::fx::FxHashSet::default();
		// Each module with its path, and the module each segment of it names.
		let root = LOCAL_CRATE.as_def_id();
		let mut queue = VecDeque::from([(root, vec![self.report.root.clone()], vec![root])]);
		while let Some((module, path, modules)) = queue.pop_front() {
			let children = match module.as_local() {
				Some(local) => tcx.module_children_local(local),
				None => tcx.module_children(module),
			};
			for child in children {
				if !child.vis.is_public() || child.ident.name == rustc_span::kw::Underscore {
					continue;
				}
				let Res::Def(kind, def_id) = child.res else {
					continue;
				};
				// Variants and constructors come along with their type.
				if matches!(kind, DefKind::Ctor(..) | DefKind::Variant) {
					continue;
				}
				let mut path = path.clone();
				path.push(child.ident.to_string());
				// A crate re-exported whole makes its every release part of the facade's
				// semver, so each such `pub use` is refused once, and the crate not walked.
				// A glob of its root re-exports its items instead, which export. The span is
				// the `pub use` naming the crate, past any glob of the module holding it.
				if kind == DefKind::Mod && def_id.is_crate_root() && !def_id.is_local() {
					if self.exclude(&path) {
						continue;
					}
					let span = child.reexport_chain.last().and_then(|reexport| reexport.id());
					let span = span.map_or_else(|| tcx.def_span(def_id), |id| tcx.def_span(id));
					let krate = tcx.crate_name(def_id.krate).to_string();
					self.refuse(&path, "use", span, vec![why("re-export of a whole crate", krate)]);
					continue;
				}
				if !seen.insert(def_id) {
					continue;
				}
				let mut modules = modules.clone();
				modules.push(def_id);
				if self.exclude(&path) {
					self.left_out.insert(def_id, "excluded type");
					continue;
				}
				if tcx.is_doc_hidden(def_id) {
					let (report, rest) = self.report(&path);
					report.hidden.push(rest);
					self.left_out.insert(def_id, "doc(hidden) type");
					continue;
				}
				let span = child
					.reexport_chain
					.first()
					.and_then(|reexport| reexport.id())
					.map_or_else(|| tcx.def_span(def_id), |id| tcx.def_span(id));
				// Its types map by role, not as items of the facade's.
				if self.sysroot.contains(&def_id.krate) {
					let krate = tcx.crate_name(def_id.krate);
					let why = why("re-export of the standard library's item", krate.to_string());
					self.refuse(&path, "use", span, vec![why]);
					continue;
				}
				match kind {
					DefKind::Mod => queue.push_back((def_id, path, modules)),
					DefKind::TyAlias => {
						let (report, rest) = self.report(&path);
						report.aliases.push(rest);
					}
					DefKind::Macro(_) => self.refuse(&path, "macro", span, vec![why("macro", "")]),
					DefKind::Static { .. } => self.refuse(&path, "static", span, vec![why("static", "")]),
					DefKind::Struct
					| DefKind::Enum
					| DefKind::Union
					| DefKind::Trait
					| DefKind::Fn
					| DefKind::Const => {
						if !self.crates.contains(&def_id.krate) {
							self.crates.push(def_id.krate);
						}
						// Its crate's own segments end the path, after any naming the
						// facade's modules or another crate's.
						let own = (modules.iter().rev())
							.take_while(|m| m.krate == def_id.krate && !m.is_crate_root())
							.count();
						self.outside.insert(def_id, path.len() - own);
						self.items.push((def_id, path))
					}
					_ => self.refuse(&path, "item", span, vec![why("item kind", format!("{kind:?}"))]),
				}
			}
		}
		self.walk_reach(&seen);
	}

	/// Walk each type in `reach` the root did not, by its public path in its own crate, as if
	/// a module of the facade re-exported that crate's root.
	fn walk_reach(&mut self, seen: &rustc_data_structures::fx::FxHashSet<DefId>) {
		let tcx = self.tcx;
		self.walked = self.crates.len();
		for def_id in self.reach.clone() {
			if seen.contains(&def_id) {
				continue;
			}
			let mut path = vec![self.report.root.clone()];
			if !def_id.is_local() {
				path.push(tcx.crate_name(def_id.krate).to_string());
			}
			let own = self.crate_path(def_id).expect("a reached type has a path in its crate");
			let outside = path.len();
			path.extend(own);
			if self.exclude(&path) {
				self.left_out.insert(def_id, "excluded type");
				continue;
			}
			if tcx.is_doc_hidden(def_id) {
				let (report, rest) = self.report(&path);
				report.hidden.push(rest);
				self.left_out.insert(def_id, "doc(hidden) type");
				continue;
			}
			// The glue depends on its crate only once a type of it is walked.
			if !def_id.is_local() && !self.api.reached.iter().any(|r| r.krate == path[1]) {
				self.api.reached.push(crate::ir::Reached {
					krate: path[1].clone(),
					targets: self.api.targets.clone(),
				});
			}
			if !self.crates.contains(&def_id.krate) {
				self.crates.push(def_id.krate);
			}
			self.outside.insert(def_id, outside);
			self.items.push((def_id, path));
		}
	}

	/// Whether `def_id` exports by its public path in its own crate. Another crate's must be
	/// the only crate of its name, named by none of the facade's root items, and one the glue
	/// can depend on, so its path names it there.
	fn reachable(&self, def_id: DefId) -> bool {
		let tcx = self.tcx;
		if self.crate_path(def_id).is_none() {
			return false;
		}
		if def_id.is_local() {
			return true;
		}
		if self.barred.contains(&def_id) || self.sysroot.contains(&def_id.krate) {
			return false;
		}
		let name = tcx.crate_name(def_id.krate);
		let root = tcx.module_children_local(LOCAL_CRATE.as_def_id().expect_local());
		self.keys[&def_id.krate] == name.as_str()
			&& !root.iter().any(|child| child.ident.name == name)
			&& self.dependencies.contains_key(name.as_str())
	}

	/// Walk `def_id` by its public path in its own crate in the next pass, unless the type
	/// being mapped is only tried.
	fn reached(&self, def_id: DefId) {
		if self.probing.get() {
			return;
		}
		let mut unreached = self.unreached.borrow_mut();
		if !unreached.contains(&def_id) {
			unreached.push(def_id);
		}
	}

	/// The shortest public path to `def_id` from its own crate's root, through that crate's
	/// modules and re-exports, as `kio::Pending` names one defined in a private module.
	fn crate_path(&self, def_id: DefId) -> Option<Vec<String>> {
		let tcx = self.tcx;
		let root = def_id.krate.as_def_id();
		let mut seen = rustc_data_structures::fx::FxHashSet::default();
		let mut queue = VecDeque::from([(root, Vec::new())]);
		while let Some((module, path)) = queue.pop_front() {
			let children = match module.as_local() {
				Some(local) => tcx.module_children_local(local),
				None => tcx.module_children(module),
			};
			for child in children {
				let Res::Def(kind, id) = child.res else {
					continue;
				};
				if !child.vis.is_public()
					|| child.ident.name == rustc_span::kw::Underscore
					|| id.krate != def_id.krate
					|| (kind == DefKind::Mod && tcx.is_doc_hidden(id))
				{
					continue;
				}
				let mut path = path.clone();
				path.push(child.ident.to_string());
				if id == def_id {
					return Some(path);
				}
				if kind == DefKind::Mod && seen.insert(id) {
					queue.push_back((id, path));
				}
			}
		}
		None
	}

	/// Decide how each type and trait is exported, before any signature names one.
	fn declare(&mut self) {
		let tcx = self.tcx;
		for (def_id, path) in self.items.clone() {
			if !matches!(
				tcx.def_kind(def_id),
				DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::Trait
			) {
				continue;
			}
			for site in self.sites(def_id, &path) {
				let decl = match self.unarguable(&site) {
					Some(why) => Decl::Refused(why),
					None => self.declare_site(&site),
				};
				self.decls.insert(site.key, decl);
			}
		}
		// An instance holds its arguments, so it exports only if each does.
		loop {
			self.settle();
			let mut changed = false;
			for site in self.instances.clone() {
				if !matches!(self.decls.get(&site.key), Some(Decl::Refused(_)) | None)
					&& let Err(why) = self.args(&site)
				{
					self.decls.insert(site.key, Decl::Refused(why));
					changed = true;
				}
			}
			if !changed {
				break;
			}
		}
		// A trait whose methods do not map is refused before any signature takes it.
		for (def_id, path) in self.items.clone() {
			if matches!(self.trait_decl(def_id), Some(Decl::Callback(_)))
				&& let Err(why) = self.callback(def_id, &path)
			{
				let first = why.into_iter().next().expect("a refusal has a reason");
				self.decls.insert(Key { def_id, args: None }, Decl::Refused(first));
			}
		}
		// Number each kind in item order, with placeholders filled in by `define`.
		for (def_id, path) in self.items.clone() {
			let span = tcx.def_span(def_id);
			let docs = docs(tcx, def_id);
			let home = self.home(def_id);
			for site in self.sites(def_id, &path) {
				let Some(decl) = self.decls.get_mut(&site.key) else {
					continue;
				};
				let api = &mut self.api;
				let (path, shown, docs) = (site.path, site.shown, docs.clone());
				match decl {
					Decl::Record(i) => {
						*i = api.records.len();
						api.records.push(Record {
							path,
							home: home.clone(),
							shown,
							args: Vec::new(),
							span,
							on: On::ALL,
							docs,
							fields: Vec::new(),
							tuple: false,
							methods: Vec::new(),
							traits: Traits::default(),
							opaque: None,
							padding: Default::default(),
						});
					}
					Decl::Enum(i) => {
						*i = api.enums.len();
						api.enums.push(Enum {
							path,
							home: home.clone(),
							shown,
							args: Vec::new(),
							span,
							on: On::ALL,
							docs,
							variants: Vec::new(),
							exhaustive: true,
							methods: Vec::new(),
							traits: Traits::default(),
							foreign: false,
						});
					}
					Decl::Value(i) => {
						*i = api.records.len();
						let ty = tcx.type_of(def_id).instantiate(tcx, site.args).skip_norm_wip();
						let lifetimes = tcx.generics_of(def_id).own_counts().lifetimes;
						let opaque = opaque(tcx, &self.sysroot, ty, lifetimes, false);
						let padding = padding(tcx, ty).expect("a value's bytes do not depend on it");
						api.records.push(Record {
							path,
							home: home.clone(),
							shown,
							args: Vec::new(),
							span,
							on: On::ALL,
							docs,
							fields: Vec::new(),
							tuple: false,
							methods: Vec::new(),
							traits: Traits::default(),
							opaque: Some(opaque),
							padding: [(target(tcx), padding)].into(),
						});
					}
					Decl::Object(i) => {
						*i = api.objects.len();
						let ty = tcx.type_of(def_id).instantiate(tcx, site.args).skip_norm_wip();
						let sync = implements(tcx, ty, def_id, sym::Sync);
						let copy = implements(tcx, ty, def_id, sym::Copy);
						let lifetimes = tcx.generics_of(def_id).own_counts().lifetimes;
						api.objects.push(Object {
							path,
							home: home.clone(),
							shown,
							args: Vec::new(),
							span,
							on: On::ALL,
							docs,
							sync,
							copy,
							shared: false,
							lifetimes,
							methods: Vec::new(),
							traits: Traits::default(),
							foreign: false,
							opaque: None,
						});
					}
					Decl::Callback(i) => {
						*i = api.callbacks.len();
						api.callbacks.push(Callback {
							path,
							home: home.clone(),
							span,
							on: On::ALL,
							docs,
							methods: Vec::new(),
						});
					}
					Decl::Refused(_) => {}
				}
			}
		}
		// After every declared object, so `prune` can drop each that no export names. Until
		// here, each maps only to decide what does, never to an index kept.
		// A foreign value goes after every declared record, for the same reason.
		for n in 0..self.foreign.len() {
			let def_id = self.foreign[n].0;
			let object = self.foreign_object(def_id, self.api.objects.len());
			match self.foreign_value(def_id) {
				true => {
					self.foreign[n].1 = self.api.records.len();
					let ty = tcx.type_of(def_id).instantiate_identity().skip_norm_wip();
					let opaque = opaque(tcx, &self.sysroot, ty, 0, true);
					let padding = padding(tcx, ty).expect("a value's bytes do not depend on it");
					self.api.records.push(Record {
						path: object.path,
						home: object.home,
						shown: object.shown,
						args: object.args,
						span: object.span,
						on: object.on,
						docs: object.docs,
						fields: Vec::new(),
						tuple: false,
						methods: object.methods,
						traits: object.traits,
						opaque: Some(opaque),
						padding: [(target(tcx), padding)].into(),
					});
				}
				false => {
					self.foreign[n].1 = self.api.objects.len();
					self.api.objects.push(object);
				}
			}
		}
		// After every declared enum, so `prune` can drop it if no export names it.
		if let Some(level) = &mut self.level {
			level.index = self.api.enums.len();
			let def_id = level.level;
			let variants = ["Trace", "Debug", "Info", "Warn", "Error"];
			self.api.enums.push(Enum {
				path: vec![tcx.crate_name(def_id.krate).to_string(), "Level".to_string()],
				home: Home {
					krate: self.keys[&def_id.krate].clone(),
					outside: 1,
				},
				shown: vec![self.report.root.clone(), "Level".to_string()],
				args: Vec::new(),
				span: tcx.def_span(def_id),
				on: On::ALL,
				docs: "`tracing::Level`: how verbose a span or event is, from the most to the least.".to_string(),
				variants: (variants.iter().zip(0..))
					.map(|(name, discriminant)| Variant {
						name: name.to_string(),
						discriminant,
						fields: Vec::new(),
						tuple: false,
						on: On::ALL,
					})
					.collect(),
				// A match sees through its constants to the five levels they are.
				exhaustive: true,
				methods: Vec::new(),
				// Its `Debug` is left out: Dart would name it as it names the `Debug` level,
				// and it prints only what `Display` and the variant's name already say.
				traits: Traits {
					clone: true,
					eq: true,
					display: true,
					debug: false,
					default: false,
				},
				foreign: true,
			});
		}
	}

	/// The object a foreign type exports as, at `index`: one in [`FOREIGN`] made by parsing
	/// its text, and any other an opaque handle to an `Arc` of it.
	fn foreign_object(&self, def_id: DefId, index: usize) -> Object {
		let tcx = self.tcx;
		let span = tcx.def_span(def_id);
		let ty = Type::Object(index);
		let krate = tcx.crate_name(def_id.krate);
		let name = tcx.item_name(def_id);
		if self.opaque(def_id) {
			let owned = self.owned.contains(&def_id);
			let docs = match owned {
				true => format!(
					"`{}`: a handle to another crate's value, which Rust takes back or borrows, with none of its API.",
					tcx.def_path_str(def_id)
				),
				false => format!(
					"`Arc<{}>`: a handle to another crate's value, which Rust takes back, with none of its API.",
					tcx.def_path_str(def_id)
				),
			};
			return Object {
				path: vec![krate.to_string(), name.to_string()],
				home: Home {
					krate: self.keys[&def_id.krate].clone(),
					outside: 2,
				},
				shown: vec![self.report.root.clone(), krate.to_string(), name.to_string()],
				args: Vec::new(),
				span,
				on: On::ALL,
				docs,
				sync: true,
				copy: false,
				// A held value is moved out of its handle, as an object of the crate's is, and
				// its clone is no part of the API.
				shared: !owned,
				lifetimes: 0,
				methods: Vec::new(),
				traits: Traits {
					clone: !owned,
					..Traits::default()
				},
				foreign: true,
				opaque: Some(match owned {
					true => Held::Value,
					false => Held::Arc,
				}),
			};
		}
		let role = (FOREIGN.iter())
			.find(|&&(k, n, _)| k == krate.as_str() && n == name.as_str())
			.map(|&(_, _, role)| role)
			.expect("a foreign type is in FOREIGN");
		let segments = |first: &str, second: &str, rest: &str| -> Path {
			[first, second, rest]
				.iter()
				.filter(|s| !s.is_empty())
				.map(|s| s.to_string())
				.collect()
		};
		// Its path is its own crate's, which the glue names it by; it shows at the facade's
		// root, which names it in every language.
		let path = |rest: &str| segments(krate.as_str(), name.as_str(), rest);
		let shown = |rest: &str| segments(&self.report.root, role, rest);
		let parse = |docs: &str| Function {
			path: path("parse"),
			shown: shown("parse"),
			span,
			on: On::ALL,
			docs: docs.to_string(),
			receiver: None,
			params: vec![Param {
				name: "input".to_string(),
				ty: Type::Ref(Mutability::Not, Box::new(Type::Str)),
				interned: false,
			}],
			units: Vec::new(),
			ok: ty.clone(),
			err: Some(Type::String),
			future: false,
			eager: false,
			holds: false,
			task: None,
			collect: false,
			lends: false,
			access: Access::Parse,
			instance: None,
			home: Home::default(),
		};
		let (docs, methods, traits) = match krate.as_str() {
			// Made only by Rust, as its clock is the process's own.
			"std" => (
				"`std::time::Instant`: a moment on the process's monotonic clock, which only Rust makes.",
				Vec::new(),
				Traits {
					clone: true,
					eq: true,
					debug: true,
					..Traits::default()
				},
			),
			"url" => (
				"`url::Url`: a URL, made by parsing its text.",
				vec![parse("Parse an absolute URL, failing with why it is not one.")],
				Traits {
					clone: true,
					eq: true,
					display: true,
					debug: true,
					default: false,
				},
			),
			// Its contents are the caller's JSON library's to read, from its text.
			_ => (
				"`serde_json::Value`: a JSON document, made by parsing its text and read back as text.",
				vec![
					parse("Parse a JSON document, failing with why it is not one."),
					Function {
						path: path("to_text"),
						shown: shown("to_text"),
						span,
						on: On::ALL,
						docs: "The document as JSON text.".to_string(),
						receiver: Some(Receiver::Ref),
						params: Vec::new(),
						units: Vec::new(),
						ok: Type::String,
						err: None,
						future: false,
						eager: false,
						holds: false,
						task: None,
						collect: false,
						lends: false,
						access: Access::Text,
						instance: None,
						home: Home::default(),
					},
				],
				Traits::default(),
			),
		};
		let self_ty = tcx.type_of(def_id).instantiate_identity().skip_norm_wip();
		// std's is named through its module, which the glue names it by.
		let path = match krate.as_str() {
			"std" => tcx.def_path_str(def_id).split("::").map(str::to_string).collect(),
			_ => path(""),
		};
		Object {
			path,
			home: Home {
				krate: self.keys[&def_id.krate].clone(),
				outside: 1,
			},
			shown: shown(""),
			args: Vec::new(),
			span,
			on: On::ALL,
			docs: docs.to_string(),
			sync: true,
			copy: implements(tcx, self_ty, def_id, sym::Copy),
			shared: traits.clone && self.clone_shares(self_ty, def_id),
			lifetimes: 0,
			methods,
			traits,
			foreign: true,
			opaque: None,
		}
	}

	/// How a type or trait is exported: a struct whose fields are all `pub`, or an enum,
	/// is a value if its fields map, and any other type is an object.
	fn declare_site(&self, site: &Site<'tcx>) -> Decl {
		let tcx = self.tcx;
		let def_id = site.key.def_id;
		let instance = site.key.args.is_some();
		let params = &tcx.generics_of(def_id).own_params;
		let generic = params.iter().any(|p| p.name != rustc_span::kw::SelfUpper);
		let lifetimes = params
			.iter()
			.any(|p| matches!(p.kind, ty::GenericParamDefKind::Lifetime));
		match tcx.def_kind(def_id) {
			DefKind::Union => Decl::Refused(why("union", "")),
			// Held by a handle that outlives any call, it is the form that borrows nothing.
			DefKind::Struct | DefKind::Enum if lifetimes && (instance || !self.typed(def_id)) => self.object(site),
			DefKind::Struct | DefKind::Enum if generic && !instance => {
				Decl::Refused(why("generic type no export instantiates", ""))
			}
			DefKind::Trait if generic && !instance => Decl::Refused(why("generic type", "")),
			// A value is copied across and lowered into memory C frees, so its own `Drop`
			// would run when Rust lets go of it, not when C does.
			DefKind::Struct | DefKind::Enum if tcx.adt_destructor(def_id).is_some() => self.object(site),
			DefKind::Struct => {
				let variant = tcx.adt_def(def_id).non_enum_variant();
				let record = !variant.fields.is_empty()
					&& variant.fields.iter().all(|f| f.vis.is_public())
					&& !variant.is_field_list_non_exhaustive();
				match record {
					true => Decl::Record(0),
					false => self.object(site),
				}
			}
			// No value is one, so it is a handle C never holds, which no C enum is.
			DefKind::Enum if tcx.adt_def(def_id).variants().is_empty() => self.object(site),
			DefKind::Enum => match wide(tcx, def_id) {
				Some(why) => Decl::Refused(why),
				None => Decl::Enum(0),
			},
			_ => Decl::Callback(0),
		}
	}

	/// The arguments an instance is named by, each type of which must export as a value or
	/// a handle: Rust fills in the defaults after them, which may be any type.
	fn args(&self, site: &Site<'tcx>) -> Result<Vec<Type>, Why> {
		(Self::shown_args(site.args, site.explicit).into_iter())
			.map(|arg| match arg.kind() {
				GenericArgKind::Const(ct) => self.const_arg(ct).map(Type::Const),
				_ => self.arg(arg.expect_ty()),
			})
			.collect()
	}

	/// A type argument that names an instance: a value or a handle.
	fn arg(&self, ty: Ty<'tcx>) -> Result<Type, Why> {
		let arg = match ty.kind() {
			ty::Tuple(list) if list.is_empty() => Type::Unit,
			_ => self.ty(ty, Pos::Behind)?,
		};
		match arg {
			Type::Unit
			| Type::Bool
			| Type::Int(_)
			| Type::NonZero(_)
			| Type::F32
			| Type::F64
			| Type::Char
			| Type::String
			| Type::Bytes
			| Type::Duration
			| Type::Record(_)
			| Type::Enum(_)
			| Type::Object(_) => Ok(arg),
			_ => Err(why("type argument", self.spell(ty))),
		}
	}

	/// An object, if the type can move across threads, or if it is also `Copy`, a value C
	/// holds as its bytes, unless which of its bytes it initializes depends on its value, so
	/// its padding could not be zeroed. Shared borrows of an object that is not `Sync` are
	/// exclusive.
	fn object(&self, site: &Site<'tcx>) -> Decl {
		let (tcx, ty, def_id) = (self.tcx, self.site_ty(site), site.key.def_id);
		let copy = implements(tcx, ty, def_id, sym::Copy) && padding(tcx, ty).is_some();
		if self.orphans.contains(&site.key) {
			let construct = match copy {
				true => "value no export returns",
				false => "object no export returns",
			};
			return Decl::Refused(why(construct, ""));
		}
		match (implements(tcx, ty, def_id, sym::Send), copy) {
			(true, true) => Decl::Value(0),
			(true, false) => Decl::Object(0),
			(false, _) => Decl::Refused(why("object not Send", "")),
		}
	}

	/// The objects C can never hold a handle to: no export it can call returns one.
	fn orphans(&self) -> Vec<Key<'tcx>> {
		let api = &self.api;
		let types = (api.objects.iter().enumerate()).map(|(i, o)| (Type::Object(i), &o.methods, o.traits));
		let types = types.chain((api.records.iter().enumerate()).map(|(i, r)| (Type::Record(i), &r.methods, r.traits)));
		let types = types.chain((api.enums.iter().enumerate()).map(|(i, e)| (Type::Enum(i), &e.methods, e.traits)));
		// Each function and the type its receiver is, and each type `Default` makes.
		let mut functions: Vec<(&Function, Option<Type>)> = api.functions.iter().map(|f| (f, None)).collect();
		let mut defaults = Vec::new();
		for (ty, methods, traits) in types {
			functions.extend(methods.iter().map(|f| (f, f.receiver.map(|_| ty.clone()))));
			if traits.default {
				defaults.push(ty);
			}
		}
		// Each object and opaque value C can hold, which only Rust makes.
		let mut held: std::collections::HashSet<Type> = std::collections::HashSet::new();
		loop {
			let mut reached = Vec::new();
			for ty in &defaults {
				returns(api, ty, &mut reached);
			}
			for (function, receiver) in &functions {
				let callable = (receiver.iter())
					.chain(function.params.iter().map(|p| &p.ty))
					.all(|ty| buildable(api, ty, &held));
				if callable {
					returns(api, &function.ok, &mut reached);
					if let Some(err) = &function.err {
						returns(api, err, &mut reached);
					}
					// A closure's objects are lent to C for each call of it.
					for param in &function.params {
						if let Type::Closure(closure_) = &param.ty {
							let params = &closure_.params;
							params.iter().for_each(|ty| returns(api, ty, &mut reached));
						}
					}
				}
			}
			reached.retain(|ty| !held.contains(ty));
			if reached.is_empty() {
				break;
			}
			held.extend(reached);
		}
		// Another crate's type an opaque handle holds, as each object of the crate's; a
		// foreign value's index is a record's.
		let opaque = (self.foreign.iter())
			.filter(|&&(did, i)| {
				!self.foreign_value(did) && api.objects[i].opaque.is_some() && !held.contains(&Type::Object(i))
			})
			.map(|&(def_id, _)| Key { def_id, args: None });
		(self.decls.iter())
			.filter_map(|(&key, decl)| match decl {
				Decl::Object(i) if !held.contains(&Type::Object(*i)) => Some(key),
				Decl::Value(i) if !held.contains(&Type::Record(*i)) => Some(key),
				_ => None,
			})
			.chain(opaque)
			.collect()
	}

	/// Make each record or enum whose fields do not map an object, until none changes.
	fn settle(&mut self) {
		loop {
			let mut changed = false;
			for site in self.all_sites() {
				if !matches!(self.decls.get(&site.key), Some(Decl::Record(_) | Decl::Enum(_))) {
					continue;
				}
				let fields = match self.recursive(&site) {
					true => Err(vec![why("recursive value", "")]),
					false => self.fields(&site).map(|_| ()),
				};
				if let Err(why) = fields {
					let first = why.into_iter().next().expect("a refusal has a reason");
					let (report, rest) = self.report(&site.shown);
					report.opaque.push((rest, first));
					let decl = self.object(&site);
					self.decls.insert(site.key, decl);
					changed = true;
				}
			}
			if !changed {
				break;
			}
		}
	}

	/// Every type and trait the export declares, in item order.
	fn all_sites(&self) -> Vec<Site<'tcx>> {
		(self.items.iter())
			.filter(|(def_id, _)| {
				matches!(
					self.tcx.def_kind(*def_id),
					DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::Trait
				)
			})
			.flat_map(|(def_id, path)| self.sites(*def_id, path))
			.collect()
	}

	/// Whether a value holds itself, through other values, an `Option`, or a `Box`: C
	/// holds each value in place, so it cannot.
	fn recursive(&self, start: &Site<'tcx>) -> bool {
		let tcx = self.tcx;
		let mut seen = rustc_data_structures::fx::FxHashSet::default();
		let mut stack = vec![(start.key.def_id, start.args)];
		while let Some((def_id, args)) = stack.pop() {
			for field in tcx.adt_def(def_id).all_fields() {
				for arg in field.ty(tcx, args).skip_norm_wip().walk() {
					let Some(ty) = arg.as_type() else {
						continue;
					};
					let ty::Adt(inner, inner_args) = ty.kind() else {
						continue;
					};
					let key = self.key(inner.did(), *inner_args);
					if !matches!(self.decls.get(&key), Some(Decl::Record(_) | Decl::Enum(_))) {
						continue;
					}
					if key == start.key {
						return true;
					}
					if seen.insert(key) {
						stack.push((inner.did(), *inner_args));
					}
				}
			}
		}
		false
	}

	/// A struct's or enum's variants, with their fields mapped.
	fn fields(&self, site: &Site<'tcx>) -> Result<Vec<Variant>, Vec<Why>> {
		let tcx = self.tcx;
		let adt = tcx.adt_def(site.key.def_id);
		let identity = site.args;
		let mut errs = Vec::new();
		let mut variants = Vec::new();
		let discriminants: Vec<i128> = match adt.is_enum() {
			true => adt.discriminants(tcx).map(|(_, d)| signed(tcx, d.ty, d.val)).collect(),
			false => vec![0],
		};
		for (variant, discriminant) in adt.variants().iter().zip(discriminants) {
			if adt.is_enum() && variant.is_field_list_non_exhaustive() {
				errs.push(why("non-exhaustive variant", variant.name.to_string()));
			}
			let mut fields = Vec::new();
			for field in &variant.fields {
				let ty = field.ty(tcx, identity).skip_norm_wip();
				match self.ty(ty, Pos::Field) {
					Ok(ty) => fields.push(Field {
						name: field.name.to_string(),
						docs: docs(tcx, field.did),
						ty,
					}),
					Err(why) => errs.push(why),
				}
			}
			let tuple = variant.ctor_kind() == Some(rustc_hir::def::CtorKind::Fn);
			variants.push(Variant {
				name: variant.name.to_string(),
				discriminant,
				fields,
				tuple,
				on: On::ALL,
			});
		}
		match errs.is_empty() {
			true => Ok(variants),
			false => Err(errs),
		}
	}

	/// Fill in every declaration, then the functions and constants.
	fn define(&mut self) {
		let tcx = self.tcx;
		for (def_id, path) in self.items.clone() {
			let span = tcx.def_span(def_id);
			let kind = tcx.def_kind(def_id);
			if !matches!(kind, DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::Trait) {
				match kind {
					DefKind::Fn => {
						for (function, shown, _) in self.functions(def_id, &path, &path, None, None) {
							match function {
								Ok(function) => self.api.functions.push(Function {
									home: self.home(def_id),
									..function
								}),
								Err(why) => self.refuse(&shown, "fn", span, why),
							}
						}
					}
					DefKind::Const => {
						if let Some(getter) = self.constant_item(def_id, path, None) {
							let home = self.home(def_id);
							self.api.functions.push(Function { home, ..getter });
						}
					}
					_ => {}
				}
				continue;
			}
			for site in self.sites(def_id, &path) {
				let decl = self.decls.get(&site.key).cloned();
				// A trait exports only as what C implements: one it cannot implement is no
				// part of C's API, while a signature that takes one from C is refused.
				// An unsafe contract is not something C lacks but something it cannot keep.
				if kind == DefKind::Trait
					&& let Some(why) = trait_unsafety(tcx, def_id)
				{
					self.refuse(&site.shown, "trait", span, vec![why]);
					continue;
				}
				if let (Some(Decl::Refused(why)), DefKind::Trait) = (&decl, kind) {
					let (report, rest) = self.report(&site.shown);
					report.traits.push((rest, why.clone()));
					continue;
				}
				if let Some(Decl::Refused(why)) = decl {
					let kind = match kind {
						_ if site.key.args.is_some() => "instance",
						DefKind::Enum => "enum",
						DefKind::Union => "union",
						_ => "struct",
					};
					self.refuse(&site.shown, kind, span, vec![why]);
					continue;
				}
				if let Some(Decl::Callback(index)) = decl {
					self.api.callbacks[index].methods = self.callback(def_id, &path).expect("settled");
					continue;
				}
				let args = self.args(&site).expect("settled");
				match decl {
					Some(Decl::Record(index)) => {
						let variants = self.fields(&site).expect("settled");
						let variant = variants.into_iter().next().expect("a struct has a variant");
						let (methods, traits) = self.methods(&site);
						let methods = self.unmutated(methods);
						let record = &mut self.api.records[index];
						record.args = args;
						record.fields = variant.fields;
						record.tuple = variant.tuple;
						record.methods = methods;
						record.traits = traits;
					}
					Some(Decl::Enum(index)) => {
						let variants = self.fields(&site).expect("settled");
						let exhaustive = !tcx.adt_def(def_id).is_variant_list_non_exhaustive();
						let (methods, traits) = self.methods(&site);
						let methods = self.unmutated(methods);
						let e = &mut self.api.enums[index];
						e.args = args;
						e.variants = variants;
						e.exhaustive = exhaustive;
						e.methods = methods;
						e.traits = traits;
					}
					Some(Decl::Value(index)) => {
						let (mut methods, traits) = self.methods(&site);
						if kind == DefKind::Struct {
							methods.splice(0..0, self.accessors(&site));
						}
						let methods = self.written_back(methods);
						let record = &mut self.api.records[index];
						record.args = args;
						record.methods = methods;
						record.traits = traits;
					}
					Some(Decl::Object(index)) => {
						let (mut methods, traits) = self.methods(&site);
						if kind == DefKind::Struct {
							methods.splice(0..0, self.accessors(&site));
						}
						let shared = traits.clone && self.clone_shares(self.site_ty(&site), def_id);
						let object = &mut self.api.objects[index];
						object.args = args;
						object.methods = methods;
						object.traits = traits;
						object.shared = shared && object.sync;
					}
					_ => {}
				}
			}
		}
	}

	/// Drop each foreign type's object that no export names, so a crate that only builds
	/// with `url` exports none of it. They are the last objects, so dropping one renumbers
	/// only those after it.
	fn prune(&mut self) {
		// The last enum, so dropping it renumbers nothing.
		if let Some(level) = self.level {
			let gone = Type::Enum(level.index);
			let api = &self.api;
			let signature = |f: &Function| {
				(f.params.iter().map(|p| &p.ty))
					.chain([&f.ok])
					.chain(&f.err)
					.chain(f.instance.iter().flat_map(|i| i.args.iter().flatten().chain(&i.named)))
					.any(|ty| names(ty, &gone))
			};
			let fields = |fields: &[Field]| fields.iter().any(|f| names(&f.ty, &gone));
			let args = |args: &[Type]| args.iter().any(|ty| names(ty, &gone));
			let used = api.functions.iter().any(signature)
				|| (api.records.iter()).any(|r| args(&r.args) || fields(&r.fields) || r.methods.iter().any(signature))
				|| (api.enums.iter()).any(|e| {
					args(&e.args) || e.variants.iter().any(|v| fields(&v.fields)) || e.methods.iter().any(signature)
				})
				|| (api.objects.iter()).any(|o| args(&o.args) || o.methods.iter().any(signature))
				|| (api.callbacks.iter()).any(|c| c.methods.iter().any(signature));
			if !used {
				self.api.enums.pop();
				self.level = None;
			}
		}
		for n in (0..self.foreign.len()).rev() {
			let gone = self.foreign[n].1;
			let value = self.foreign_value(self.foreign[n].0);
			let api = &self.api;
			let object = match value {
				true => Type::Record(gone),
				false => Type::Object(gone),
			};
			let signature = |f: &Function| {
				(f.params.iter().map(|p| &p.ty))
					.chain([&f.ok])
					.chain(&f.err)
					.chain(f.instance.iter().flat_map(|i| i.args.iter().flatten().chain(&i.named)))
					.any(|ty| names(ty, &object))
			};
			let fields = |fields: &[Field]| fields.iter().any(|f| names(&f.ty, &object));
			let args = |args: &[Type]| args.iter().any(|ty| names(ty, &object));
			let used = api.functions.iter().any(signature)
				|| (api.records.iter().enumerate()).any(|(i, r)| {
					(!value || i != gone) && (args(&r.args) || fields(&r.fields) || r.methods.iter().any(signature))
				})
				|| (api.enums.iter()).any(|e| {
					args(&e.args) || e.variants.iter().any(|v| fields(&v.fields)) || e.methods.iter().any(signature)
				})
				|| (api.objects.iter().enumerate())
					.any(|(i, o)| (value || i != gone) && (args(&o.args) || o.methods.iter().any(signature)))
				|| (api.callbacks.iter()).any(|c| c.methods.iter().any(signature));
			if used {
				continue;
			}
			// A foreign value is among the last records, as an object among the last objects.
			match value {
				true => drop(self.api.records.remove(gone)),
				false => drop(self.api.objects.remove(gone)),
			}
			self.foreign.remove(n);
			let values: Vec<bool> = (self.foreign[n..].iter())
				.map(|&(did, _)| self.foreign_value(did))
				.collect();
			for (later, later_value) in self.foreign[n..].iter_mut().zip(values) {
				if later_value == value {
					later.1 -= 1;
				}
			}
			let api = &mut self.api;
			let functions = (api.functions.iter_mut())
				.chain(api.records.iter_mut().flat_map(|r| &mut r.methods))
				.chain(api.enums.iter_mut().flat_map(|e| &mut e.methods))
				.chain(api.objects.iter_mut().flat_map(|o| &mut o.methods))
				.chain(api.callbacks.iter_mut().flat_map(|c| &mut c.methods));
			for f in functions {
				let instance = (f.instance.iter_mut()).flat_map(|i| i.args.iter_mut().flatten().chain(&mut i.named));
				(f.params.iter_mut().map(|p| &mut p.ty))
					.chain([&mut f.ok])
					.chain(&mut f.err)
					.chain(instance)
					.for_each(|ty| renumber(ty, gone, value));
			}
			for r in &mut api.records {
				(r.args.iter_mut())
					.chain(r.fields.iter_mut().map(|f| &mut f.ty))
					.for_each(|ty| renumber(ty, gone, value));
			}
			for e in &mut api.enums {
				(e.args.iter_mut())
					.chain(
						e.variants
							.iter_mut()
							.flat_map(|v| v.fields.iter_mut().map(|f| &mut f.ty)),
					)
					.for_each(|ty| renumber(ty, gone, value));
			}
			for o in &mut api.objects {
				o.args.iter_mut().for_each(|ty| renumber(ty, gone, value));
			}
		}
	}

	/// The `field@` key of the field `name` of `site`, shown as `shown`, and whether a method
	/// or constant shares its name, or `None` if the config excludes it. As in rustdoc,
	/// `field@` and the path name the field, and the path alone does unless a method or
	/// constant of this instance takes the name, which it then names.
	fn field_key(
		&mut self,
		site: &Site<'tcx>,
		name: rustc_span::Symbol,
		shown: &[String],
	) -> Option<(Vec<String>, bool)> {
		let tcx = self.tcx;
		let self_ty = self.site_ty(site);
		let shared = (tcx.inherent_impls(site.key.def_id).iter())
			.filter_map(|&imp| Some((imp, self.impl_args(imp, self_ty)?)))
			.flat_map(|(imp, parent)| {
				(tcx.associated_items(imp).filter_by_name_unhygienic(name)).map(move |item| (item, parent))
			})
			.any(|(item, parent)| {
				matches!(item.kind, ty::AssocKind::Fn { .. } | ty::AssocKind::Const { .. })
					&& tcx.visibility(item.def_id).is_public()
					&& self.applies(item.def_id, parent)
			});
		let mut key = shown.to_vec();
		key[1].insert_str(0, "field@");
		(!self.exclude(&key) && (shared || !self.exclude(shown))).then_some((key, shared))
	}

	/// A getter and a setter for each `pub` field of a struct exported as an object or an
	/// opaque value: the getter returns a copy, and the setter assigns in place, as
	/// `x.field = value` does.
	fn accessors(&mut self, site: &Site<'tcx>) -> Vec<Function> {
		let tcx = self.tcx;
		let mut accessors = Vec::new();
		for field in &tcx.adt_def(site.key.def_id).non_enum_variant().fields {
			if !field.vis.is_public() {
				continue;
			}
			let name = field.name.to_string();
			let shown = [&site.shown[..], std::slice::from_ref(&name)].concat();
			let Some((key, shared)) = self.field_key(site, field.name, &shown) else {
				continue;
			};
			let span = tcx.def_span(field.did);
			let ty = field.ty(tcx, site.args).skip_norm_wip();
			// An instance's field of `()`, such as a default extension, holds nothing to read.
			if ty.is_unit() {
				continue;
			}
			// Read through `&self`, so returned as a copy, as a borrowed return is, and
			// written with a value C gives up, as an owned parameter is.
			let read = Ty::new_imm_ref(tcx, tcx.lifetimes.re_erased, ty);
			// A field that is an object but not `Clone` is lent from the receiver's handle,
			// as a method's borrowed return of one is.
			let lends = !self.is_clone(ty)
				&& matches!(ty.kind(), ty::Adt(adt, args) if matches!(self.decl(adt.did(), *args), Some(Decl::Object(_))));
			let mapped = {
				self.lending.set(lends);
				let get = self.ty(read, Pos::Return);
				self.lending.set(false);
				// A handle outlives every call, so a reference it holds is `'static`, which
				// C's borrow for the call is not.
				let set = match self.ty(ty, Pos::Param) {
					Ok(set) if matches!(set, Type::Ref(..)) || lends_inside(&set) => {
						Err(why("'static borrow", ty.to_string()))
					}
					set => set,
				};
				match (get, set) {
					(Ok(get), Ok(set)) => Ok((get, set)),
					(get, set) => {
						let mut whys: Vec<Why> = get.err().into_iter().collect();
						// A type neither way maps is one reason, not two.
						let new = set.err().filter(|set| {
							!whys
								.iter()
								.any(|get| get.construct == set.construct && get.detail == set.detail)
						});
						whys.extend(new);
						Err(whys)
					}
				}
			};
			match mapped {
				Ok((get, set)) => {
					let accessor = |receiver, params, ok, set| Function {
						path: [&site.path[..], std::slice::from_ref(&name)].concat(),
						shown: shown.clone(),
						span,
						on: On::ALL,
						docs: docs(tcx, field.did),
						receiver: Some(receiver),
						params,
						units: Vec::new(),
						ok,
						err: None,
						future: false,
						eager: false,
						holds: false,
						task: None,
						collect: false,
						lends: false,
						access: Access::Field { shared, set },
						instance: None,
						home: Home::default(),
					};
					let lent = lends
						&& matches!(&get, Type::Ref(Mutability::Not, inner) if matches!(**inner, Type::Object(_)));
					let mut getter = accessor(Receiver::Ref, Vec::new(), get, false);
					getter.lends = lent;
					accessors.push(getter);
					let value = Param {
						name: "value".to_string(),
						ty: set,
						interned: false,
					};
					accessors.push(accessor(Receiver::Mut, vec![value], Type::Unit, true));
				}
				Err(why) => self.refuse(if shared { &key } else { &shown }, "field", span, why),
			}
		}
		accessors
	}

	/// A type's inherent `pub` methods, from each impl that applies to it, and the std
	/// traits it implements.
	/// A value's methods, refusing each through `&mut self`: C passes the value as a copy,
	/// so what the method changes would be lost.
	fn unmutated(&mut self, methods: Vec<Function>) -> Vec<Function> {
		let (mutated, kept): (Vec<Function>, Vec<Function>) =
			(methods.into_iter()).partition(|m| m.receiver == Some(Receiver::Mut));
		for m in mutated {
			self.refuse(&m.shown, "method", m.span, vec![why("`&mut self` of a value", "")]);
		}
		kept
	}

	/// An opaque value's methods, refusing each through `&mut self` whose change Rust could
	/// not hand back with what it returns: a future's, which outlives the call, and a
	/// `Result`'s, whose error a language raising it would lose the change with.
	fn written_back(&mut self, methods: Vec<Function>) -> Vec<Function> {
		let (refused, kept): (Vec<Function>, Vec<Function>) =
			(methods.into_iter()).partition(|m| m.receiver == Some(Receiver::Mut) && (m.future || m.err.is_some()));
		for m in refused {
			let construct = match m.future {
				true => "`&mut self` of a value in a future",
				false => "`&mut self` of a value returning a Result",
			};
			self.refuse(&m.shown, "method", m.span, vec![why(construct, "")]);
		}
		kept
	}

	fn methods(&mut self, site: &Site<'tcx>) -> (Vec<Function>, Traits) {
		let tcx = self.tcx;
		let def_id = site.key.def_id;
		let mut methods = Vec::new();
		let mut refused = Vec::new();
		let self_ty = self.site_ty(site);
		for (item, parent, shown, hidden) in self.members(site) {
			if hidden {
				let (report, rest) = self.report(&shown);
				report.hidden.push(rest);
				continue;
			}
			let method = [&site.path[..], &[item.name().to_string()]].concat();
			let span = tcx.def_span(item.def_id);
			if let ty::AssocKind::Const { .. } = item.kind {
				// An instance's constant is read in Rust, named through the instance.
				let parent = site.key.args.map(|_| parent);
				methods.extend(self.constant_item(item.def_id, shown, parent).map(|mut getter| {
					getter.path = method;
					getter
				}));
				continue;
			}
			for (function, shown, instance) in self.functions(item.def_id, &method, &shown, Some(self_ty), Some(parent))
			{
				match function {
					Ok(function) => methods.push(function),
					Err(why) => refused.push((shown, span, why, instance)),
				}
			}
		}
		// `poll_x` is left out when `x` exports as a future: the future is its poll.
		// One made of a `Poll` is no twin's.
		let futures: Vec<String> = (methods.iter())
			.filter(|m| m.future && m.task.is_none())
			.map(|m| m.name().to_string())
			.collect();
		let twin = |name: &str| {
			name.strip_prefix("poll_")
				.is_some_and(|x| futures.iter().any(|f| f == x))
		};
		let (twins, kept): (Vec<Function>, Vec<Function>) =
			(methods.into_iter()).partition(|m| m.future && m.task.is_some() && m.instance.is_none() && twin(m.name()));
		methods = kept;
		for function in twins {
			let (report, rest) = self.report(&function.shown);
			report.twins.push(rest);
		}
		for (method, span, why, instance) in refused {
			let name = method.last().expect("a method has a name");
			let twin = !instance && twin(name);
			match twin {
				true => {
					let (report, rest) = self.report(&method);
					report.twins.push(rest);
				}
				false => self.refuse(&method, "method", span, why),
			}
		}
		let env = TypingEnv::post_analysis(tcx, def_id);
		let implements = |item| {
			tcx.get_diagnostic_item(item).is_some_and(|tr| {
				let (infcx, env) = tcx.infer_ctxt().build_with_typing_env(env);
				infcx
					.type_implements_trait(tr, [self_ty], env)
					.must_apply_modulo_regions()
			})
		};
		let traits = Traits {
			clone: implements(sym::Clone),
			eq: tcx.lang_items().eq_trait().is_some_and(|tr| {
				let (infcx, env) = tcx.infer_ctxt().build_with_typing_env(env);
				infcx
					.type_implements_trait(tr, [self_ty, self_ty], env)
					.must_apply_modulo_regions()
			}),
			display: implements(sym::Display),
			debug: implements(sym::Debug),
			default: implements(sym::Default),
		};
		(methods, traits)
	}

	/// A type's inherent `pub` methods and constants the config does not exclude, each with
	/// the arguments of its impl, the path reports show it by, and whether it is
	/// `doc(hidden)`. One whose own bounds fail at the instance is none of its methods, as
	/// is each of an impl whose bounds fail.
	fn members(&mut self, site: &Site<'tcx>) -> Vec<(ty::AssocItem, ty::GenericArgs<'tcx>, Path, bool)> {
		let tcx = self.tcx;
		let self_ty = self.site_ty(site);
		let mut members = Vec::new();
		for &imp in tcx.inherent_impls(site.key.def_id) {
			let Some(parent) = self.impl_args(imp, self_ty) else {
				continue;
			};
			for &item in tcx.associated_items(imp).in_definition_order() {
				if !matches!(item.kind, ty::AssocKind::Fn { .. } | ty::AssocKind::Const { .. })
					|| !tcx.visibility(item.def_id).is_public()
				{
					continue;
				}
				let shown = [&site.shown[..], &[item.name().to_string()]].concat();
				if self.exclude(&shown) {
					continue;
				}
				let hidden = tcx.is_doc_hidden(item.def_id);
				if hidden || self.applies(item.def_id, parent) {
					members.push((item, parent, shown, hidden));
				}
			}
		}
		members
	}

	/// The arguments an inherent impl applies to `self_ty` at, if it does: its type
	/// parameters as `self_ty` fixes them, its lifetimes as they are, and its bounds met.
	fn impl_args(&self, imp: DefId, self_ty: Ty<'tcx>) -> Option<ty::GenericArgs<'tcx>> {
		let tcx = self.tcx;
		let mut args = ty::GenericArgs::identity_for_item(tcx, imp).as_slice().to_vec();
		let pattern = tcx.type_of(imp).instantiate_identity().skip_norm_wip();
		if !matches(tcx, pattern, self_ty, &mut args) {
			return None;
		}
		let args = tcx.mk_args(&args);
		let env = TypingEnv::post_analysis(tcx, imp);
		let (infcx, param_env) = tcx.infer_ctxt().build_with_typing_env(env);
		let holds = tcx.clauses_of(imp).instantiate(tcx, args).clauses.iter().all(|clause| {
			let obligation = Obligation::new(tcx, ObligationCause::dummy(), param_env, clause.skip_norm_wip());
			infcx.predicate_must_hold_modulo_regions(&obligation)
		});
		holds.then_some(args)
	}

	/// Whether an impl item's bounds hold with the impl's parameters at `parent`. A bound on
	/// the item's own type parameters is the choice of a type for each.
	fn applies(&self, item: DefId, parent: ty::GenericArgs<'tcx>) -> bool {
		let tcx = self.tcx;
		let mut args = ty::GenericArgs::identity_for_item(tcx, item).as_slice().to_vec();
		args[..parent.len()].copy_from_slice(parent.as_slice());
		self.holds(item, tcx.mk_args(&args), &[])
	}

	/// A trait's required methods, as a callback interface.
	fn callback(&mut self, def_id: DefId, path: &[String]) -> Result<Vec<Function>, Vec<Why>> {
		let tcx = self.tcx;
		let mut errs = Vec::new();
		// Foreign code would implement it without upholding the contract it documents.
		if tcx.trait_def(def_id).safety.is_unsafe() {
			errs.push(why("unsafe trait", ""));
		}
		// It may already cover the foreign implementation, which could not then implement it.
		if !tcx.trait_impls_of(def_id).blanket_impls().is_empty() {
			errs.push(why("trait with a blanket impl", ""));
		}
		for clause in tcx.explicit_super_clauses_of(def_id).iter_identity_copied() {
			let (clause, _) = clause.skip_norm_wip();
			if let Some(trait_pred) = clause.as_trait_clause() {
				let tr = trait_pred.def_id();
				if !marker(tcx, tr) {
					errs.push(why("supertrait", tcx.def_path_str(tr)));
				}
			}
		}
		let mut methods = Vec::new();
		let self_ty = Ty::new_param(tcx, 0, rustc_span::kw::SelfUpper);
		for item in tcx.associated_items(def_id).in_definition_order() {
			match item.kind {
				ty::AssocKind::Fn { name, .. } => {
					// A provided method keeps its body: only required ones are foreign.
					if item.defaultness(tcx).has_value() {
						continue;
					}
					let mut method = path.to_vec();
					method.push(name.to_string());
					let function = self
						.instantiate(item.def_id, None)
						.and_then(|instances| match &instances[..] {
							[one] if one.named.is_empty() => {
								self.function(item.def_id, method.clone(), method, Some(self_ty), one)
							}
							_ => Err(vec![why("generic parameter", name.to_string())]),
						});
					match function {
						Ok(function) if function.future => errs.push(why("async trait method", name.to_string())),
						Ok(function)
							if !function.params.iter().all(|p| foreign_arg(&p.ty))
								|| !foreign_return(&function.ok)
								|| function.err.is_some() =>
						{
							errs.push(why("callback signature", name.to_string()))
						}
						Ok(function) if function.receiver == Some(Receiver::Owned) || function.receiver.is_none() => {
							errs.push(why("trait method without &self", name.to_string()))
						}
						Ok(function) => methods.push(function),
						// A per-method reason names its method, as `trait_unsafety` does.
						Err(why) => errs.extend(why.into_iter().map(|why| match why.construct {
							"unsafe fn" | "C-variadic function" | "uncallable ABI" => Why {
								detail: name.to_string(),
								..why
							},
							_ => why,
						})),
					}
				}
				// An `impl Trait` return's own type: the method refuses it.
				ty::AssocKind::Type {
					data: ty::AssocTypeData::Rpitit(_),
				} => {}
				ty::AssocKind::Type { .. } => errs.push(why("associated type", item.name().to_string())),
				ty::AssocKind::Const { .. } => errs.push(why("associated const", item.name().to_string())),
			}
		}
		match errs.is_empty() {
			true => Ok(methods),
			false => Err(errs),
		}
	}

	/// A function or method as exported, at each instance it exports at, each with the path
	/// reports show it by and whether it is an instance.
	fn functions(
		&mut self,
		def_id: DefId,
		path: &Path,
		shown: &Path,
		self_ty: Option<Ty<'tcx>>,
		parent: Option<ty::GenericArgs<'tcx>>,
	) -> Vec<(Result<Function, Vec<Why>>, Path, bool)> {
		let instances = match self.instantiate(def_id, parent) {
			Ok(instances) => instances,
			Err(why) => return vec![(Err(why), shown.clone(), false)],
		};
		let mut functions = Vec::new();
		for instance in &instances {
			let Some(shown) = self.shown(shown, instance) else {
				continue;
			};
			let function = self.function(def_id, path.clone(), shown.clone(), self_ty, instance);
			functions.push((function, shown, !instance.named.is_empty()));
		}
		functions
	}

	/// A function or method's signature at one of its instances, which the config names by
	/// `shown`: a method's receiver is `self_ty`.
	fn function(
		&self,
		def_id: DefId,
		path: Path,
		shown: Path,
		self_ty: Option<Ty<'tcx>>,
		instance: &Instantiation<'tcx>,
	) -> Result<Function, Vec<Why>> {
		let tcx = self.tcx;
		if let Some(why) = unsafety(tcx, def_id) {
			return Err(vec![why]);
		}
		let mut errs = Vec::new();
		let args = instance.args;
		// Each kept parameter takes a closure, if an `Fn` trait bounds it, a reader, if
		// `io::Read` and `io::Seek` do, or else the foreign implementation `instantiate` kept
		// it for.
		let mut impls = Vec::new();
		for &param in &instance.kept {
			let taken = (self.closure(def_id, param, args.as_slice()))
				.or_else(|| self.reader(def_id, param, args.as_slice()))
				.unwrap_or_else(|| {
					let callback = self.taken(def_id, param).expect("a kept parameter takes a callback");
					Ok(Type::Callback(callback, Dyn::Impl))
				});
			match taken {
				Ok(taken) => impls.push((param, taken)),
				Err(why) => errs.push(why),
			}
		}
		if !errs.is_empty() {
			return Err(errs);
		}
		*self.impls.borrow_mut() = impls;
		// What the glue names a declared type parameter as, which a call cannot always
		// infer; an `impl Trait` one cannot be named, and is inferred from its argument.
		let mut arg = |arg: ty::GenericArg<'tcx>| {
			let mapped = match arg.kind() {
				GenericArgKind::Const(ct) => self.const_arg(ct).map(Type::Const),
				GenericArgKind::Type(ty) => match *ty.kind() {
					ty::Ref(_, inner, ty::Mutability::Not) => {
						self.arg(inner).map(|inner| Type::Ref(Mutability::Not, Box::new(inner)))
					}
					_ => self.arg(ty),
				},
				GenericArgKind::Lifetime(_) => unreachable!("no lifetime names an instance"),
			};
			mapped.map_err(|why| errs.push(why)).ok()
		};
		let instance = match instance.named.is_empty() {
			true => None,
			false => {
				let declared = (tcx.generics_of(def_id).own_params.iter()).filter(|p| {
					matches!(
						p.kind,
						ty::GenericParamDefKind::Type { synthetic: false, .. } | ty::GenericParamDefKind::Const { .. }
					)
				});
				// A kept parameter is left for Rust to infer from its argument.
				let args = declared
					.map(|p| args[p.index as usize])
					.filter_map(|a| match a.as_type().is_some_and(|ty| instance.kept.contains(&ty)) {
						true => Some(None),
						false => arg(a).map(Some),
					})
					.collect();
				let named = (instance.named.iter())
					.filter_map(|&a| match a.kind() {
						GenericArgKind::Type(ty) => arg(ty.peel_refs().into()),
						_ => arg(a),
					})
					.collect();
				Some(crate::ir::Instance { args, named })
			}
		};
		let env = TypingEnv::post_analysis(tcx, def_id);
		let sig = tcx.fn_sig(def_id).instantiate(tcx, args).skip_norm_wip();
		let (statics, lifetimes, interned) = self.borrows(def_id, sig.skip_binder());
		errs.extend(lifetimes);
		// Whether what it returns borrows: any lifetime in it that is not `'static`.
		let borrows = sig.skip_binder().output().walk().any(|arg| match arg.kind() {
			GenericArgKind::Lifetime(region) => !statics.contains(&region),
			_ => false,
		});
		let names = tcx.fn_arg_idents(def_id);
		let has_self = matches!(tcx.def_kind(def_id), DefKind::AssocFn)
			&& matches!(
				tcx.associated_item(def_id).kind,
				ty::AssocKind::Fn { has_self: true, .. }
			);
		// What it returns may borrow an object that is not `Clone` from a `&self` of an
		// object's alone, whose handle then lends it: Rust lends it only while no exclusive
		// borrow of the receiver begins, as the handle checks.
		let unbound = sig.skip_binder();
		let lendable = match unbound.inputs().first().map(|ty| ty.kind()) {
			Some(&ty::Ref(region, inner, ty::Mutability::Not))
				if has_self
					&& matches!(inner.kind(), ty::Adt(adt, args) if matches!(self.decl(adt.did(), *args), Some(Decl::Object(_)))) =>
			{
				let regions: Vec<_> = (unbound.output().walk()).filter_map(|arg| arg.as_region()).collect();
				!regions.is_empty()
					&& regions.iter().all(|&r| r == region)
					&& (unbound.inputs()[1..].iter())
						.all(|input| !input.walk().any(|arg| arg.as_region() == Some(region)))
			}
			_ => false,
		};
		// Rust writes back a value C passes for `&mut`, which nothing it returns may still
		// borrow.
		let written: Vec<_> = (unbound.inputs().iter())
			.filter_map(|input| match input.kind() {
				ty::Ref(region, inner, ty::Mutability::Mut)
					if !matches!(inner.kind(), ty::Adt(adt, args) if matches!(self.decl(adt.did(), *args), Some(Decl::Object(_)))) =>
				{
					Some(*region)
				}
				_ => None,
			})
			.collect();
		if (unbound.output().walk()).any(|arg| arg.as_region().is_some_and(|region| written.contains(&region))) {
			errs.push(why(
				"returned borrow of a value written back",
				unbound.output().to_string(),
			));
		}
		let sig = tcx.instantiate_bound_regions_with_erased(sig);
		let normalize = |ty: Ty<'tcx>| {
			tcx.try_normalize_erasing_regions(env, ty::Unnormalized::new_wip(ty))
				.unwrap_or(ty)
		};
		let callback = tcx.def_kind(tcx.parent(def_id)) == DefKind::Trait;
		let mut output = normalize(sig.output());
		// A returned `Poll` is a future's: each time C polls the future, it calls the function
		// with the poll's context, so the future owns every other argument and passes it again.
		let polled = match output.kind() {
			ty::Adt(adt, args) if !callback && tcx.is_lang_item(adt.did(), LangItem::Poll) => Some(args.type_at(0)),
			_ => None,
		};
		let mut tasks = Vec::new();
		let mut inputs = sig.inputs().iter().copied().map(normalize).enumerate();
		let receiver = match has_self {
			false => None,
			true => {
				let (_, ty) = inputs.next().expect("a method has self");
				let self_ty = self_ty.map(normalize);
				match ty.kind() {
					_ if Some(ty) == self_ty => Some(Receiver::Owned),
					// A poll of the object its task is made into, as kio's `Waiter::poll_future`
					// is, takes it from the future, as it would a parameter.
					ty::Ref(_, inner, _)
						if Some(*inner) == self_ty
							&& polled.is_some()
							&& let Some(kind) = self.task(ty, true) =>
					{
						tasks.push(Task { at: 0, kind });
						None
					}
					ty::Ref(_, inner, m) if Some(*inner) == self_ty => Some(match m {
						ty::Mutability::Not => Receiver::Ref,
						ty::Mutability::Mut => Receiver::Mut,
					}),
					_ => {
						errs.push(why("receiver", ty.to_string()));
						None
					}
				}
			}
		};
		// Where Rust's parameters after `self` start: at `self`, when the task takes its place.
		let first = usize::from(receiver.is_some());
		let again = |ty: Ty<'tcx>| ty.is_ref() || tcx.type_is_copy_modulo_regions(env, ty);
		if polled.is_some()
			&& receiver == Some(Receiver::Owned)
			&& let Some(self_ty) = self_ty
			&& !again(normalize(self_ty))
		{
			errs.push(why("poll taking self by value", self_ty.to_string()));
		}
		let mut params = Vec::new();
		let mut units = Vec::new();
		for (i, ty) in inputs {
			// C passes nothing for `()`, the one value of its type. A callback's stays
			// refused: the glue's implementation of its trait names every parameter.
			if ty.is_unit() && !callback {
				units.push(i - first);
				continue;
			}
			let name = names
				.get(i)
				.copied()
				.flatten()
				.map_or_else(|| format!("arg{i}"), |ident| ident.to_string());
			if !callback && let Some(kind) = self.task(ty, polled.is_some()) {
				// C passes a waker for a context the future does not supply.
				if polled.is_none() {
					params.push(Param {
						name,
						ty: Type::Waker,
						interned: false,
					});
				}
				tasks.push(Task { at: i - first, kind });
				continue;
			}
			// The future owns a future C implements, which it lends pinned to each poll.
			if polled.is_some() && self.pinned_future(ty) {
				params.push(Param {
					name,
					ty: Type::Future,
					interned: false,
				});
				continue;
			}
			if polled.is_some() && !again(ty) {
				errs.push(why("poll's parameter taken by value that is not Copy", ty.to_string()));
				continue;
			}
			match self.ty(ty, Pos::Param) {
				// C's value is lifted into an owned Rust one, lent only as the parameter
				// itself, or as `Some` of it. A callback's parameters are lent the other way,
				// to C.
				Ok(lifted)
					if !callback
						&& lends_inside(match &lifted {
							Type::Option(inner) if matches!(**inner, Type::Ref(..)) => inner,
							lifted => lifted,
						}) =>
				{
					errs.push(why("reference inside a parameter", ty.to_string()));
				}
				Ok(mapped) => {
					// Rust writes back what the call left in a value C passes, which a
					// future, a callback, or a null pointer for `None` would complicate
					// with no use for it.
					match &mapped {
						Type::Option(inner) if inner.written() => {
							errs.push(why("&mut of a value inside an Option", ty.to_string()))
						}
						_ if mapped.written() && callback => {
							errs.push(why("&mut of a value in a callback", ty.to_string()))
						}
						_ => {}
					}
					params.push(Param {
						name,
						ty: mapped,
						interned: !callback && interned.contains(&i),
					})
				}
				Err(why) => errs.push(why),
			}
		}
		let task = tasks.pop();
		if !tasks.is_empty() {
			errs.push(why("several task parameters", ""));
		}
		let mut future = false;
		let mut eager = !tcx.asyncness(def_id).is_async();
		if let Some(inner) = polled {
			if task.is_none() {
				errs.push(why("Poll with no Context or waiter parameter", output.to_string()));
			}
			if borrows {
				errs.push(why("borrowed Poll", inner.to_string()));
			}
			output = normalize(inner);
			future = true;
			eager = false;
		} else if let Some(inner) = self.future_output(output, env) {
			// The context C's waker makes lives only for the call.
			if task.is_some() {
				errs.push(why("Context of a function returning a future", ""));
			}
			// What the call passes for a kept parameter, a closure, reader, or foreign
			// implementation, is `Send` and `Sync`, as the future holding it may be too.
			let kept: Vec<Ty<'tcx>> = self.impls.borrow().iter().map(|&(param, _)| param).collect();
			let marked = (kept.into_iter())
				.flat_map(|param| [sym::Send, sym::Sync].map(|item| (param, item)))
				.filter_map(|(param, item)| {
					let tr = tcx.get_diagnostic_item(item)?;
					Some(ty::Upcast::upcast(ty::TraitRef::new(tcx, tr, [param]), tcx))
				});
			let kept = TypingEnv::new(
				ty::ParamEnv::new(tcx, env.param_env.caller_bounds().all_clauses(tcx).iter().chain(marked)),
				env.typing_mode(),
			);
			if !self.is_send(output, kept) {
				errs.push(why("future not Send", ""));
			}
			// An eager future holds what it borrows only until it completes, so its output
			// may borrow none of it.
			if eager && borrows && self.output_borrows(unbound.output(), inner, &statics) {
				errs.push(why(
					"borrowing output of a future from a function that is not async",
					inner.to_string(),
				));
			}
			output = normalize(inner);
			future = true;
		}
		// A future, polled or async, lifts each value into what it owns, so has nothing to
		// write back.
		if future && params.iter().any(|p: &Param| p.ty.written()) {
			errs.push(why("&mut of a value in a future", ""));
		}
		// A reader is lent for the call, which a future outlives.
		if future && params.iter().any(|p: &Param| *p.ty.base() == Type::Reader) {
			errs.push(why("reader lent to a future", ""));
		}
		// An iterator no other rule maps crosses as the array of its items, all taken at
		// the call, as `collect` takes them, when no caller could tell; a future's stays
		// refused.
		let mut collect = false;
		if !future
			&& self.probe(|| self.ret(output)).is_err()
			&& let Some(items) = self
				.collected(output, env)
				.and_then(|item| self.vec_of(normalize(item)))
		{
			output = items;
			collect = true;
		}
		// Only the returned value itself, a bare `&Object`, is lent: not one inside it, nor
		// the error.
		let ret = |ty: Ty<'tcx>| {
			self.lending
				.set(lendable && !future && matches!(ty.kind(), ty::Ref(_, _, ty::Mutability::Not)));
			let mapped = self.ret(ty);
			self.lending.set(false);
			mapped
		};
		let (ok, err) = match output.kind() {
			ty::Adt(adt, args) if tcx.is_diagnostic_item(sym::Result, adt.did()) => {
				let error = args.type_at(1);
				// Another crate's error, such as `anyhow::Error` or `io::Error`, crosses as
				// its message, as an exported error's `Display` already reads, even where a
				// signature reaches its crate. One a handle holds by value is its message too,
				// held or not.
				self.erring.set(true);
				let err = self.ty(error, Pos::Return);
				self.erring.set(false);
				let foreign = |mapped: &Result<Type, Why>| {
					self.opaque_value(error).is_some()
						|| matches!(mapped, Err(why) if why.construct == "foreign type" || why.construct == BOTH)
				};
				let err = match err {
					err if foreign(&err) && implements(tcx, error, def_id, rustc_span::sym::Display) => {
						Ok(Type::String)
					}
					err => err,
				};
				(ret(args.type_at(0)), Some(err))
			}
			_ => (ret(output), None),
		};
		let ok_ty = match output.kind() {
			ty::Adt(adt, args) if tcx.is_diagnostic_item(sym::Result, adt.did()) => args.type_at(0),
			_ => output,
		};
		// The glue cannot name an `impl Trait`'s hidden type, so one in a return that maps
		// crosses only as what the glue passes on whole: an object's handle, or a value's
		// bytes. A future and a collected iterator are taken apart instead, so it is what
		// their output or items name that crosses.
		let unrevealed = sig.output();
		let mut named = if ok.is_ok() { vec![unrevealed] } else { Vec::new() };
		while let Some(ty) = named.pop() {
			for ty in ty.walk().filter_map(|arg| arg.as_type()) {
				let ty::Alias(
					_,
					ty::AliasTy {
						kind: ty::AliasTyKind::Opaque { def_id },
						args,
						..
					},
				) = ty.kind()
				else {
					continue;
				};
				if ty == unrevealed && (future || collect) {
					let bounds = tcx.item_bounds(*def_id).skip_binder();
					named.extend(bounds.iter().filter_map(|clause| {
						let term = clause.as_projection_clause()?.skip_binder().term.as_type()?;
						Some(ty::EarlyBinder::bind(tcx, term).instantiate(tcx, args).skip_norm_wip())
					}));
					continue;
				}
				let hidden = normalize(ty);
				if !matches!(hidden.kind(), ty::Adt(adt, args)
					if matches!(self.decl(adt.did(), *args), Some(Decl::Object(_) | Decl::Value(_))))
				{
					errs.push(why("opaque type", ty.to_string()));
				}
			}
		}
		let lends = matches!(ok, Ok(Type::Ref(Mutability::Not, ref inner)) if matches!(**inner, Type::Object(_)))
			&& !self.is_clone(ok_ty.peel_refs());
		let ok = ok.map_err(|why| errs.push(why)).ok();
		let err = match err {
			Some(Ok(err)) => Some(err),
			Some(Err(why)) => {
				errs.push(why);
				None
			}
			None => None,
		};
		match (errs.is_empty(), ok) {
			(true, Some(ok)) => Ok(Function {
				path,
				shown,
				span: tcx.def_span(def_id),
				on: On::ALL,
				docs: docs(tcx, def_id),
				receiver,
				params,
				units,
				ok,
				err,
				future,
				eager: future && eager,
				holds: future && eager && borrows,
				task,
				collect,
				lends,
				access: Access::Call,
				instance,
				home: Home::default(),
			}),
			_ => Err(errs),
		}
	}

	/// The generic arguments a function is exported with: its parent's (an impl's at the
	/// instance, or a trait's `Self`, which stays a parameter, as the trait is the
	/// callback), lifetimes as they are, which [`Self::borrows`] decides, a type parameter
	/// an `Fn` trait bounds kept for a lent closure, and each other by the rules in
	/// [`Self::choose`], or else kept for a foreign implementation, or else at each type
	/// [`Self::each`] finds, an instance for each choice where its bounds hold. While
	/// inferring, a parameter takes every type `choose` ranks and every type `each` finds,
	/// as typing may take any of them.
	fn instantiate(
		&self,
		def_id: DefId,
		parent: Option<ty::GenericArgs<'tcx>>,
	) -> Result<Vec<Instantiation<'tcx>>, Vec<Why>> {
		let tcx = self.tcx;
		let generics = tcx.generics_of(def_id);
		let identity = ty::GenericArgs::identity_for_item(tcx, def_id);
		let mut args: Vec<ty::GenericArg<'tcx>> = identity.as_slice().to_vec();
		let count = generics.parent_count;
		if let Some(parent) = parent {
			args[..count].copy_from_slice(parent.as_slice());
		}
		let base = args.clone();
		let mut kept = Vec::new();
		// Each choice of the parameters so far, with the arguments that name it.
		let mut choices: Vec<(Vec<ty::GenericArg<'tcx>>, Vec<ty::GenericArg<'tcx>>)> = vec![(args, Vec::new())];
		for (i, arg) in identity.iter().enumerate() {
			let param = match arg.kind() {
				_ if i < count => continue,
				GenericArgKind::Lifetime(_) => continue,
				GenericArgKind::Const(param) => {
					let next: Vec<_> = (choices.into_iter())
						.flat_map(|(args, named)| {
							(self.each_const(def_id, param, &args).into_iter()).map(move |value| {
								let mut args = args.clone();
								args[i] = value.into();
								(args, [&named[..], &[value.into()]].concat())
							})
						})
						.collect();
					// The first refusal ends it, before any later parameter's candidates are mapped.
					if next.is_empty() {
						return Err(vec![why("const generic", param.to_string())]);
					}
					choices = next;
					continue;
				}
				GenericArgKind::Type(param)
					if self.lent(def_id, param) || self.reader(def_id, param, &base).is_some() =>
				{
					kept.push(param);
					continue;
				}
				GenericArgKind::Type(param) => param,
			};
			let mut next = Vec::new();
			let mut refused = None;
			for (args, named) in choices {
				// Its bounds see each earlier parameter as this choice has it, as
				// `impl Into<Timed<&'a T>>` names `T`, and `impl Into<Option<R>>` the `R` chosen.
				let each = match self.choose(def_id, param, &args) {
					// Typing takes a later type only where an earlier one's export is refused.
					Ok(ranked) if self.inferring => {
						let last = ranked.iter().position(|&ty| !self.names_export(ty));
						ranked[..last.map_or(ranked.len(), |last| last + 1)].to_vec()
					}
					Ok(ranked) => {
						let mut args = args;
						args[i] = ranked[0].into();
						next.push((args, named));
						continue;
					}
					Err(Why {
						construct: "callback bound",
						..
					}) if self.taken(def_id, param).is_some() => {
						if !kept.contains(&param) {
							kept.push(param);
						}
						next.push((args, named));
						continue;
					}
					// A conversion refused for its chains takes no other type in its place.
					Err(
						why @ Why {
							construct: "ambiguous conversion" | "growing conversion",
							..
						},
					) => return Err(vec![why]),
					Err(why) => {
						let mut each = self.each(def_id, param, &base);
						// Before any trait is declared, it may stay for a foreign implementation.
						if self.inferring && self.foreign(def_id, param).is_some() {
							each.push(param);
						}
						if each.is_empty() {
							refused.get_or_insert(why);
						}
						each
					}
				};
				for ty in each {
					let mut args = args.clone();
					args[i] = ty.into();
					next.push((args, [&named[..], &[ty.into()]].concat()));
				}
			}
			// A choice no type of this parameter meets is dropped; with none left, the first
			// refusal ends it.
			if let (true, Some(why)) = (next.is_empty(), refused) {
				return Err(vec![why]);
			}
			choices = next;
		}
		let choices: Vec<_> = (choices.into_iter())
			.map(|(args, named)| (tcx.mk_args(&args), named))
			.collect();
		let unmet = self.unmet(def_id, choices[0].0, &kept);
		let instances: Vec<Instantiation<'tcx>> = (choices.into_iter())
			.filter(|(args, _)| self.holds(def_id, *args, &kept))
			.map(|(args, named)| Instantiation {
				args,
				kept: kept.clone(),
				named,
			})
			.collect();
		match (instances.is_empty(), unmet) {
			(true, Some(unmet)) => Err(vec![why("unmet bound", unmet.to_string())]),
			_ => Ok(instances),
		}
	}

	/// Whether `def_id`'s bounds hold at `args`, with `kept` the parameters kept for what
	/// the call passes.
	fn holds(&self, def_id: DefId, args: ty::GenericArgs<'tcx>, kept: &[Ty<'tcx>]) -> bool {
		self.unmet(def_id, args, kept).is_none()
	}

	/// The first of `def_id`'s bounds that fails at `args`. A bound on a kept parameter is
	/// met by what the call passes, and one naming a parameter not yet chosen is stated by
	/// the item's own environment only at its parameters, so both are skipped. Any other
	/// bound naming a kept parameter must hold for each type passed for it, as the item's
	/// own environment proves: `B: From<U>` holds for every `U: Callback` only through an
	/// impl for each.
	fn unmet(&self, def_id: DefId, args: ty::GenericArgs<'tcx>, kept: &[Ty<'tcx>]) -> Option<ty::Clause<'tcx>> {
		let tcx = self.tcx;
		let env = TypingEnv::post_analysis(tcx, def_id);
		let (infcx, param_env) = tcx.infer_ctxt().build_with_typing_env(env);
		let checked = |clause: ty::Clause<'tcx>| {
			if !clause.has_non_region_param() {
				return true;
			}
			let (args, term) = match clause.kind().skip_binder() {
				ty::ClauseKind::Trait(pred) => (pred.trait_ref.args, None),
				ty::ClauseKind::Projection(pred) => (pred.projection_term.args, pred.term.as_type()),
				_ => return false,
			};
			let mut params = (args.iter().chain(term.map(Into::into)))
				.flat_map(|arg| arg.walk())
				.filter_map(|arg| arg.as_type())
				.filter(|ty| matches!(ty.kind(), ty::Param(_)));
			let subject = args.first().and_then(|arg| arg.as_type());
			params.all(|ty| kept.contains(&ty)) && !subject.is_some_and(|ty| kept.contains(&ty))
		};
		(tcx.clauses_of(def_id).instantiate(tcx, args).clauses.iter())
			.map(|clause| clause.skip_norm_wip())
			.find(|&clause| {
				checked(clause) && {
					let obligation = Obligation::new(tcx, ObligationCause::dummy(), param_env, clause);
					!infcx.predicate_must_hold_modulo_regions(&obligation)
				}
			})
	}

	/// `def_id`'s bounds at `base`, with each they imply, such as a supertrait's.
	fn implied(&self, def_id: DefId, base: &[ty::GenericArg<'tcx>]) -> Vec<ty::Clause<'tcx>> {
		let tcx = self.tcx;
		let clauses = tcx.clauses_of(def_id).instantiate(tcx, tcx.mk_args(base)).clauses;
		elaborate(tcx, clauses.iter().map(|clause| clause.skip_norm_wip())).collect()
	}

	/// The types a type parameter may be exported as, best first, which typing takes the
	/// first of: `X` for each `Into<X>` or `AsRef<X>` its bounds imply, or `&X` and then
	/// `String` or `Vec<T>` where `X` is `str` or `[T]`, `Vec<X>` for
	/// `impl IntoIterator<Item = X>`, or `&[Y]` where `X` is `&Y`, `&[u8]` for
	/// `impl io::Read`, `serde_json::Value` for a serde bound, then `String`,
	/// `Bytes`, and `Vec<u8>`, each that meets its bounds and maps.
	fn choose(&self, def_id: DefId, param: Ty<'tcx>, base: &[ty::GenericArg<'tcx>]) -> Result<Vec<Ty<'tcx>>, Why> {
		let tcx = self.tcx;
		let clauses = tcx.clauses_of(def_id);
		let identity = clauses.instantiate(tcx, tcx.mk_args(base));
		let bounds: Vec<_> = identity
			.clauses
			.iter()
			.map(|clause| clause.skip_norm_wip())
			.filter_map(|clause| clause.as_trait_clause())
			.filter(|pred| pred.self_ty().skip_binder() == param)
			.collect();
		// With the bounds they imply: `IntoBytes: AsRef<[u8]>` makes a parameter it bounds bytes.
		let implied = self.implied(def_id, base);
		let outlives = must_outlive(&implied, param);
		let string = tcx
			.lang_items()
			.string()
			.map(|string| Ty::new_adt(tcx, tcx.adt_def(string), ty::GenericArgs::empty()));
		let mut candidates = Vec::new();
		let implied = (implied.iter())
			.filter_map(|clause| clause.as_trait_clause())
			.filter(|pred| pred.self_ty().skip_binder() == param);
		for bound in implied {
			let tr = bound.def_id();
			let target = || bound.skip_binder().trait_ref.args.type_at(1);
			if tcx.is_diagnostic_item(sym::Into, tr) || tcx.is_diagnostic_item(sym::AsRef, tr) {
				// An `Into` target that does not map takes what converts into it instead. The
				// signature names the target, so unlike a candidate only tried, mapping it reaches
				// it by its public path in its own crate.
				let maps = (self.inferring && self.names_export(target())) || self.ty(target(), Pos::Param).is_ok();
				let sources = (tcx.is_diagnostic_item(sym::Into, tr) && !maps)
					.then(|| self.sources(def_id, target(), outlives))
					.transpose()?;
				let sources = (sources.into_iter().flatten()).map(|(x, outlives, imp)| (x, outlives, Some(imp)));
				for (target, outlives, imp) in std::iter::once((target(), outlives, None)).chain(sources) {
					let owned = match target.kind() {
						ty::Str => string,
						ty::Slice(elem) => self.vec_of(*elem),
						_ => {
							candidates.push((target, imp));
							continue;
						}
					};
					// A borrow of what C lends, unless the parameter outlives the call, then the
					// owned form, which meets bounds that only owned types implement.
					if !outlives {
						candidates.push((Ty::new_imm_ref(tcx, tcx.lifetimes.re_erased, target), imp));
					}
					candidates.extend(owned.map(|owned| (owned, imp)));
				}
			} else if Some(tr) == self.range_bounds {
				candidates.extend(self.bounds(target()).map(|ty| (ty, None)));
			} else if tcx.is_diagnostic_item(sym::IntoIterator, tr) {
				// Borrowed items iterate a slice C lends, borrowed for the items' own lifetime,
				// so the signature's borrows decide whether C's slice lives long enough.
				let items = self.items(&identity.clauses, param).and_then(|item| match item.kind() {
					ty::Ref(region, elem, ty::Mutability::Not)
						if !outlives && !matches!(region.kind(), ty::ReBound(..)) =>
					{
						Some(Ty::new_imm_ref(tcx, *region, Ty::new_slice(tcx, *elem)))
					}
					_ => self.vec_of(item),
				});
				candidates.extend(items.map(|ty| (ty, None)));
			} else if tcx.is_diagnostic_item(rustc_span::Symbol::intern("IoRead"), tr) && !outlives {
				// What it reads, which C lends for the call.
				let bytes = Ty::new_slice(tcx, tcx.types.u8);
				candidates.push((Ty::new_imm_ref(tcx, tcx.lifetimes.re_erased, bytes), None));
			} else if serde(tcx, tr) {
				candidates.extend(self.json().map(|ty| (ty, None)));
			} else if Some(tr) == tcx.lang_items().future_trait() || tcx.is_diagnostic_item(sym::IntoFuture, tr) {
				// A future the foreign side implements, which Rust polls until done.
				candidates.extend(self.foreign_future().map(|ty| (ty, None)));
			}
		}
		candidates.extend(string.map(|ty| (ty, None)));
		if let Some(bytes) = self.bytes {
			candidates.push((Ty::new_adt(tcx, tcx.adt_def(bytes), ty::GenericArgs::empty()), None));
		}
		candidates.extend(self.vec_of(tcx.types.u8).map(|ty| (ty, None)));
		// `impl Into<Arc<str>>` takes a `String` too, which maps where `Arc<str>` does not.
		// Not another crate's type a handle holds: the glue never names it, so nothing would
		// fix the call's type argument.
		candidates.retain(|&(candidate, _)| {
			self.at(def_id, param, base, candidate).is_some()
				&& self.maps(candidate, Pos::Param)
				&& !(candidate.walk().filter_map(|arg| arg.as_type())).any(|ty| self.is_opaque(ty))
		});
		// Two sources that C passes as one buffer but that hold different values, as a
		// `String` and a `Vec<u8>` do, leave the caller no say in which conversion runs. Forms
		// of one value, as `&str` and `String` are, convert alike.
		let buffers: Vec<_> = (candidates.iter())
			.filter_map(|&(ty, imp)| Some((ty, self.ty(ty, Pos::Param).ok()?.buffer()?, imp?)))
			.collect();
		for (i, (x, buffer, _)) in buffers.iter().enumerate() {
			if let Some((y, _, imp)) = (buffers[..i].iter()).find(|(_, other, _)| other != buffer) {
				return Err(Why {
					construct: "ambiguous conversion",
					detail: x.to_string(),
					note: Some((tcx.def_span(*imp), format!("`{y}` converts too, which C passes alike"))),
				});
			}
		}
		if !candidates.is_empty() {
			return Ok(candidates.into_iter().map(|(candidate, _)| candidate).collect());
		}
		// A bound on one of the crate's traits takes a foreign implementation.
		let local: Vec<_> = bounds
			.iter()
			.map(|b| b.def_id())
			.filter(|tr| matches!(self.trait_decl(*tr), Some(Decl::Callback(_))))
			.collect();
		let sized = [tcx.lang_items().sized_trait(), tcx.lang_items().meta_sized_trait()];
		let names: Vec<String> = bounds
			.iter()
			.filter(|b| !sized.contains(&Some(b.def_id())))
			.map(|b| tcx.def_path_str(b.def_id()))
			.collect();
		match local[..] {
			[_] => Err(why("callback bound", names.join(" + "))),
			_ => Err(why("generic parameter", names.join(" + "))),
		}
	}

	/// `serde_json::Value`, if it exports as a foreign object.
	fn json(&self) -> Option<Ty<'tcx>> {
		let tcx = self.tcx;
		(self.foreign.iter())
			.find(|&&(ty, _)| tcx.crate_name(ty.krate).as_str() == "serde_json")
			.map(|&(ty, _)| Ty::new_adt(tcx, tcx.adt_def(ty), ty::GenericArgs::empty()))
	}

	/// The `Item` that `clauses` bind `param`'s `IntoIterator` to, if it names no type
	/// parameter, which a `Vec` of it then iterates as.
	fn items(&self, clauses: &[ty::Unnormalized<'tcx, ty::Clause<'tcx>>], param: Ty<'tcx>) -> Option<Ty<'tcx>> {
		let tcx = self.tcx;
		(clauses.iter())
			.filter_map(|clause| clause.skip_norm_wip().as_projection_clause())
			.map(|pred| pred.skip_binder())
			.find(|pred| {
				let term = pred.projection_term;
				let item = term.expect_projection_def_id();
				term.self_ty() == param
					&& tcx.is_diagnostic_item(sym::IntoIterator, tcx.parent(item))
					&& tcx.item_name(item) == sym::Item
			})
			.and_then(|pred| pred.term.as_type())
			.filter(|item| !item.has_non_region_param())
	}

	/// What `impl Into<target>` takes through chains of `From` implementations into
	/// `target`, closest first, each with whether it must outlive the call, as `outlives`
	/// says the parameter must, and the implementation it converts through. A source that
	/// is a type parameter takes each `X` of its `Into<X>` or `AsRef<X>` bounds, so
	/// `impl<B: Into<Bytes>> From<B> for Timed<Bytes>` makes `impl Into<Timed<Bytes>>`
	/// bytes, and an `Into<X>` whose `X` does not map takes what converts into `X` in turn,
	/// each target once, which ends a cycle. Any other source is taken as it is. A chain
	/// longer than the crate's recursion limit, past which rustc proves no conversion
	/// either, grows without end and is refused.
	fn sources(&self, def_id: DefId, target: Ty<'tcx>, outlives: bool) -> Result<Vec<(Ty<'tcx>, bool, DefId)>, Why> {
		let tcx = self.tcx;
		let sig = tcx.fn_sig(def_id).instantiate_identity().skip_norm_wip();
		let (statics, ..) = self.borrows(def_id, sig.skip_binder());
		// Each target, with whether what converts into it must outlive the call, and how
		// many conversions it is from the parameter.
		let root = target;
		let mut targets = vec![(target, outlives, 0)];
		let mut found = Vec::new();
		let mut i = 0;
		while let Some(&(target, outlives, depth)) = targets.get(i) {
			for (x, lasting, imp, into) in self.hop(def_id, target, &statics) {
				let outlives = outlives || lasting;
				if into && !self.maps(x, Pos::Param) {
					if !tcx.recursion_limit().value_within_limit(depth + 1) {
						return Err(Why {
							construct: "growing conversion",
							detail: root.to_string(),
							note: Some((tcx.def_span(imp), "applies past the recursion limit".to_string())),
						});
					}
					if !targets.iter().any(|&(y, o, _)| (y, o) == (x, outlives)) {
						targets.push((x, outlives, depth + 1));
					}
					continue;
				}
				// A source that borrows for `'static`, or must outlive the call, or converts into a
				// parameter that must, owns it, so one that borrows, as `&'a u32` into
				// `impl Into<Held<'a>> + 'static`, is not taken. A target it converts through may
				// name a lifetime, as `Scoped<'static>` does.
				let regions: Vec<_> = x.walk().filter_map(|arg| arg.as_region()).collect();
				let lent = regions.iter().any(|r| r.is_static()) || (outlives && !regions.is_empty());
				if !lent && !found.iter().any(|&(y, o, _)| (y, o) == (x, outlives)) {
					found.push((x, outlives, imp));
				}
			}
			i += 1;
		}
		Ok(found)
	}

	/// The sources of the `From` implementations into `target`, each with whether it must
	/// outlive the call, its implementation, and whether a type parameter's `Into` bound
	/// names it, so what converts into it converts into `target` too. Each names `'static`
	/// where `target` ties a lifetime to `'static` or to one of `statics`, which the export
	/// makes `'static`, as `Held<'static>` ties the `'a` of `From<&'a u32> for Held<'a>`.
	fn hop(&self, def_id: DefId, target: Ty<'tcx>, statics: &[ty::Region<'tcx>]) -> Vec<(Ty<'tcx>, bool, DefId, bool)> {
		let tcx = self.tcx;
		let Some(from) = tcx.get_diagnostic_item(sym::From) else {
			return Vec::new();
		};
		let Some(simple) = simplify_type(tcx, target, TreatParams::AsRigid) else {
			return Vec::new();
		};
		let mut found = Vec::new();
		for &imp in tcx
			.trait_impls_of(from)
			.non_blanket_impls()
			.get(&simple)
			.into_iter()
			.flatten()
		{
			let (infcx, env) = tcx
				.infer_ctxt()
				.build_with_typing_env(TypingEnv::post_analysis(tcx, def_id));
			let args = infcx.fresh_args_for_item(DUMMY_SP, imp);
			let trait_ref = tcx.impl_trait_ref(imp).instantiate(tcx, args).skip_norm_wip();
			let source = trait_ref.args.type_at(1);
			let ocx = ObligationCtxt::new(&infcx);
			if ocx
				.eq(&ObligationCause::dummy(), env, trait_ref.self_ty(), target)
				.is_err()
			{
				continue;
			}
			let clauses = tcx.clauses_of(imp).instantiate(tcx, args).clauses;
			let implied: Vec<_> = elaborate(tcx, clauses.iter().map(|clause| clause.skip_norm_wip()))
				.map(|clause| infcx.deeply_resolve_ignoring_regions(clause))
				.collect();
			// Each of the implementation's lifetimes that matching `target` relates to one that is
			// `'static`, and each that outlives one of those, as `'b: 'a` makes `'b`.
			let forever = |r: ty::Region<'tcx>| r.is_static() || statics.contains(&r);
			let mut pinned: Vec<_> = infcx.with_region_constraints(|data| {
				(data.constraints.iter())
					.filter_map(|(c, _)| match (c.sub.is_var(), c.sup.is_var()) {
						(true, false) if forever(c.sup) => Some(c.sub),
						(false, true) if forever(c.sub) => Some(c.sup),
						_ => None,
					})
					.collect()
			});
			while let Some(long) = (implied.iter())
				.filter_map(|clause| clause.as_region_outlives_clause())
				.map(|pred| pred.skip_binder())
				.find(|pred| (forever(pred.1) || pinned.contains(&pred.1)) && !pinned.contains(&pred.0))
				.map(|pred| pred.0)
			{
				pinned.push(long);
			}
			let pin = |ty: Ty<'tcx>| {
				ty::fold_regions(tcx, infcx.deeply_resolve_ignoring_regions(ty), |r, _| {
					if r.is_bound() {
						r
					} else if r.is_static() || pinned.contains(&r) {
						tcx.lifetimes.re_static
					} else {
						tcx.lifetimes.re_erased
					}
				})
			};
			// Any other source is taken as it is: `impl<'a, V> From<&'a V> for Timed<&'a V>`
			// makes `impl Into<Timed<&'a T>>` take `&T`. One that may have to outlive the call,
			// by naming `'static` or by a lifetime bound, takes nothing C lends.
			if !source.is_ty_var() {
				let x = pin(source);
				let lasting = x.walk().any(|arg| arg.as_region().is_some_and(|r| r.is_static()))
					|| (clauses.iter()).any(|clause| {
						let clause = clause.skip_norm_wip();
						clause.as_type_outlives_clause().is_some() || clause.as_region_outlives_clause().is_some()
					});
				if !lasting && !x.has_non_region_infer() {
					found.push((x, false, imp, false));
				}
				continue;
			}
			let source = infcx.deeply_resolve_ignoring_regions(source);
			let outlives = must_outlive(&implied, source);
			for bound in (implied.iter())
				.filter_map(|clause| clause.as_trait_clause())
				.filter(|bound| {
					tcx.is_diagnostic_item(sym::Into, bound.def_id())
						|| tcx.is_diagnostic_item(sym::AsRef, bound.def_id())
				}) {
				let into = tcx.is_diagnostic_item(sym::Into, bound.def_id());
				let bound = bound.skip_binder();
				let x = pin(bound.trait_ref.args.type_at(1));
				if bound.self_ty() == source && !x.has_non_region_infer() {
					found.push((x, outlives, imp, into));
				}
			}
		}
		found
	}

	/// The arguments with `param` of `def_id` as `candidate`, if its bounds hold there.
	fn at(
		&self,
		def_id: DefId,
		param: Ty<'tcx>,
		base: &[ty::GenericArg<'tcx>],
		candidate: Ty<'tcx>,
	) -> Option<ty::GenericArgs<'tcx>> {
		let tcx = self.tcx;
		let ty::Param(p) = param.kind() else {
			unreachable!("a generic type argument is a parameter");
		};
		let mut args = base.to_vec();
		args[p.index as usize] = candidate.into();
		let args = tcx.mk_args(&args);
		self.holds(def_id, args, &[]).then_some(args)
	}

	/// The types a type parameter exports at each of, where no one type is the rules':
	/// each type the exports implement a trait bounding it for, or a reference to it where
	/// that meets the bounds too, which leaves C its handle; and, as an argument of a
	/// generic type the signature names, each argument of an instance that meets its
	/// bounds where each generic type the signature names is an instance.
	fn each(&self, def_id: DefId, param: Ty<'tcx>, base: &[ty::GenericArg<'tcx>]) -> Vec<Ty<'tcx>> {
		let tcx = self.tcx;
		let at = |candidate: Ty<'tcx>| self.at(def_id, param, base, candidate);
		// A lifetime bound, such as `'static`, which a reference to a value C passes for the
		// call cannot meet, though `at` holds modulo regions.
		let outlives = must_outlive(&self.implied(def_id, base), param);
		let mut found: Vec<Ty<'tcx>> = Vec::new();
		for ty in self.implementors(def_id, param) {
			if outlives && ty.is_ref() {
				continue;
			}
			// An implementation for `&X` is taken as it is: C passes no reference to one.
			let borrowed = (!ty.is_ref() && !outlives).then(|| Ty::new_imm_ref(tcx, tcx.lifetimes.re_erased, ty));
			let ty = match borrowed.filter(|&borrowed| at(borrowed).is_some()) {
				Some(borrowed) => borrowed,
				None if at(ty).is_some() => ty,
				None => continue,
			};
			// `X` and an implementation for `&X` both choose `&X`.
			let ty = tcx.erase_and_anonymize_regions(ty);
			if !found.contains(&ty) && self.maps(ty, Pos::Param) {
				found.push(ty);
			}
		}
		let exported = |ty: Ty<'tcx>| match ty.kind() {
			ty::Adt(adt, args) if self.exported(adt.did().krate) && self.typed(adt.did()) => Some((adt.did(), args)),
			_ => None,
		};
		let spelled = |args| self.spelled(def_id, args);
		let argument = spelled(tcx.mk_args(base))
			.into_iter()
			.filter_map(exported)
			.any(|(_, args)| args.iter().any(|arg| arg.walk().any(|a| a == param.into())));
		for site in self.instances.iter().filter(|_| argument) {
			for candidate in site.args.types() {
				let erased = tcx.erase_and_anonymize_regions(candidate);
				if found
					.iter()
					.any(|&f| tcx.erase_and_anonymize_regions(f.peel_refs()) == erased)
				{
					continue;
				}
				let Some(args) = at(candidate) else {
					continue;
				};
				// A type naming a parameter not yet chosen is checked once it is.
				let named = (spelled(args).into_iter().filter_map(exported))
					.filter(|(_, args)| !args.has_non_region_param())
					.all(|(adt, args)| {
						let key = self.key(adt, *args);
						match self.inferring {
							true => self.instances.iter().any(|site| site.key == key),
							false => matches!(
								self.decls.get(&key),
								Some(Decl::Record(_) | Decl::Value(_) | Decl::Enum(_) | Decl::Object(_))
							),
						}
					});
				if named {
					found.push(candidate);
				}
			}
		}
		found
	}

	/// The values a const parameter exports at: as an argument of a generic type the
	/// signature names, each value an instance of it has there, where each generic type the
	/// signature then names at chosen arguments only is an instance.
	fn each_const(&self, def_id: DefId, param: ty::Const<'tcx>, args: &[ty::GenericArg<'tcx>]) -> Vec<ty::Const<'tcx>> {
		let tcx = self.tcx;
		let exported = |ty: Ty<'tcx>| match ty.kind() {
			ty::Adt(adt, args) if self.exported(adt.did().krate) && self.typed(adt.did()) => Some((adt.did(), args)),
			_ => None,
		};
		let spelled = |args| self.spelled(def_id, args);
		let ty::ConstKind::Param(p) = param.kind() else {
			unreachable!("a generic const argument is a parameter");
		};
		let mut found: Vec<ty::Const<'tcx>> = Vec::new();
		for (adt, at) in spelled(tcx.mk_args(args)).into_iter().filter_map(exported) {
			let Some(j) = at.iter().position(|arg| arg == param.into()) else {
				continue;
			};
			for site in self.instances.iter().filter(|site| site.key.def_id == adt) {
				let Some(value) = site.args[j].as_const() else {
					continue;
				};
				if found.contains(&value) {
					continue;
				}
				let mut args = args.to_vec();
				args[p.index as usize] = value.into();
				// A type naming a parameter not yet chosen is checked once it is.
				let named = spelled(tcx.mk_args(&args))
					.into_iter()
					.filter_map(exported)
					.filter(|(_, args)| !args.has_non_region_param())
					.all(|(adt, args)| {
						let key = self.key(adt, *args);
						match self.inferring {
							true => self.instances.iter().any(|site| site.key == key),
							false => matches!(
								self.decls.get(&key),
								Some(Decl::Record(_) | Decl::Value(_) | Decl::Enum(_) | Decl::Object(_))
							),
						}
					});
				if named {
					found.push(value);
				}
			}
		}
		found
	}

	/// Whether an `Fn` trait bounds a type parameter, which then takes a lent closure.
	fn lent(&self, def_id: DefId, param: Ty<'tcx>) -> bool {
		let tcx = self.tcx;
		(tcx.clauses_of(def_id).instantiate_identity(tcx).clauses.iter())
			.filter_map(|clause| clause.skip_norm_wip().as_trait_clause())
			.any(|pred| pred.self_ty().skip_binder() == param && tcx.fn_trait_kind_from_def_id(pred.def_id()).is_some())
	}

	/// Whether `ty` maps where `pos` puts it. While inferring, before any type is declared,
	/// one naming an export's type does.
	fn maps(&self, ty: Ty<'tcx>, pos: Pos) -> bool {
		(self.inferring && self.names_export(ty)) || self.probe(|| self.ty(ty, pos)).is_ok()
	}

	/// `map`, reaching no type a refusal names.
	fn probe<T>(&self, map: impl FnOnce() -> T) -> T {
		let was = self.probing.replace(true);
		let mapped = map();
		self.probing.set(was);
		mapped
	}

	/// Whether `ty` names an export's type, which maps only once declared.
	fn names_export(&self, ty: Ty<'tcx>) -> bool {
		ty.walk()
			.filter_map(|arg| arg.as_type())
			.any(|ty| matches!(ty.kind(), ty::Adt(adt, _) if self.exported(adt.did().krate)))
	}

	/// The closure a type parameter takes, if an `Fn` trait bounds it, which C implements as
	/// a function: lent for the call, or kept past it, by a lifetime bound or by what the
	/// function returns, until Rust frees it.
	fn closure(&self, def_id: DefId, param: Ty<'tcx>, base: &[ty::GenericArg<'tcx>]) -> Option<Result<Type, Why>> {
		let tcx = self.tcx;
		let clauses = tcx.clauses_of(def_id).instantiate(tcx, tcx.mk_args(base)).clauses;
		let clauses: Vec<_> = clauses.iter().map(|clause| clause.skip_norm_wip()).collect();
		let output = tcx.lang_items().fn_once_output();
		let mut inputs = None;
		let mut ret = None;
		let mut kept = false;
		let mut other = Vec::new();
		for &clause in &clauses {
			if let Some(pred) = clause.as_trait_clause()
				&& pred.self_ty().skip_binder() == param
			{
				let tr = pred.def_id();
				if tcx.fn_trait_kind_from_def_id(tr).is_some() {
					inputs = Some(pred.map_bound(|pred| pred.trait_ref.args.type_at(1)));
				} else if !marker(tcx, tr) {
					other.push(tcx.def_path_str(tr));
				}
			} else if let Some(pred) = clause.as_projection_clause()
				&& pred.skip_binder().projection_term.self_ty() == param
				&& Some(pred.skip_binder().projection_term.expect_projection_def_id()) == output
			{
				ret = tcx.instantiate_bound_regions_with_erased(pred).term.as_type();
			} else if let Some(pred) = clause.as_type_outlives_clause()
				&& pred.skip_binder().0 == param
			{
				kept = true;
			} else if mentions(clause, param) {
				other.push(clause.to_string());
			}
		}
		let inputs = inputs?;
		// The lifetimes the closure is generic over, which erasing loses.
		let bound = inputs.skip_binder();
		let inputs = tcx.instantiate_bound_regions_with_erased(inputs);
		let sig = tcx.fn_sig(def_id).instantiate(tcx, tcx.mk_args(base)).skip_norm_wip();
		// An `async fn`'s future, or a returned `impl Trait`, captures it.
		kept |= sig.skip_binder().output().walk().any(|arg| arg == param.into());
		// An opaque value, or a reference to one, which C copies. Another crate's value an
		// opaque handle holds is an object, never a value, so it never matches.
		let copied = |ty: Ty<'tcx>, mapped: &Type| {
			let base = match mapped {
				Type::Ref(Mutability::Not, inner) => &**inner,
				mapped => mapped,
			};
			matches!(base, Type::Record(_))
				&& matches!(ty.peel_refs().kind(), ty::Adt(adt, args)
					if matches!(self.decl(adt.did(), *args), Some(Decl::Value(_))) || self.foreign_value(adt.did()))
		};
		// Each parameter makes its own closure, which one type parameter cannot name twice.
		if sig
			.skip_binder()
			.inputs()
			.iter()
			.filter(|&&input| input == param)
			.count()
			> 1
		{
			return Some(Err(why("closure type taken twice", param.to_string())));
		}
		if !other.is_empty() {
			return Some(Err(why("closure bound", other.join(" + "))));
		}
		let (ty::Tuple(inputs), ty::Tuple(bound)) = (inputs.kind(), bound.kind()) else {
			return Some(Err(why("closure signature", inputs.to_string())));
		};
		let mut params = Vec::new();
		for (input, bound) in inputs.iter().zip(bound.iter()) {
			// A handle holds its object at `'static`, which one borrowing for the call isn't.
			if let ty::Ref(_, inner, _) = bound.kind()
				&& inner.has_escaping_bound_vars()
			{
				return Some(Err(why("lent object that borrows", bound.to_string())));
			}
			match self.ty(input, Pos::Param) {
				// An object is lent as a handle for the call, but an opaque one, whose
				// handle holds its value erased, has none to lend; an opaque value is copied, which C keeps.
				Ok(ty)
					if foreign_arg(&ty)
						|| matches!(&ty, Type::Ref(_, inner) if matches!(**inner, Type::Object(_)))
							&& !matches!(input.kind(), ty::Ref(_, inner, _) if self.is_opaque(*inner))
						|| copied(input, &ty) =>
				{
					params.push(ty)
				}
				Ok(_) => return Some(Err(why("closure signature", input.to_string()))),
				Err(why) => return Some(Err(why)),
			}
		}
		let Some(ret) = ret else {
			return Some(Err(why("closure signature", param.to_string())));
		};
		// A `Result` is its value and its error, as a function's is.
		let (ok, err) = match ret.kind() {
			ty::Adt(adt, args) if tcx.is_diagnostic_item(sym::Result, adt.did()) => {
				(args.type_at(0), Some(args.type_at(1)))
			}
			_ => (ret, None),
		};
		// What C hands Rust: a scalar, a `String`, or an opaque value or an `Option` of one,
		// owned, as C has no reference to hand back.
		let owned = |ty: Ty<'tcx>, mapped: &Type| !matches!(mapped, Type::Ref(..)) && copied(ty, mapped);
		let ok = match self.ty(ok, Pos::Return) {
			Ok(mapped)
				if foreign_return(&mapped)
					|| mapped == Type::String
					|| match (&mapped, ok.kind()) {
						(Type::Option(inner), ty::Adt(_, args)) => owned(args.type_at(0), inner),
						(mapped, _) => owned(ok, mapped),
					} =>
			{
				mapped
			}
			Ok(_) => return Some(Err(why("closure signature", ret.to_string()))),
			Err(why) => return Some(Err(why)),
		};
		let err = match err.map(|err| self.ty(err, Pos::Return)) {
			None => None,
			Some(Ok(Type::String)) => Some(Type::String),
			Some(Ok(_)) => return Some(Err(why("closure signature", ret.to_string()))),
			Some(Err(why)) => return Some(Err(why)),
		};
		Some(Ok(Type::Closure(Box::new(Closure { params, ok, err, kept }))))
	}

	/// The reader a type parameter takes, if `io::Read` and `io::Seek` bound it: one C
	/// implements, which Rust reads and seeks only during the call. One bounded otherwise,
	/// but by what that implementation meets, or kept past the call, is refused.
	fn reader(&self, def_id: DefId, param: Ty<'tcx>, base: &[ty::GenericArg<'tcx>]) -> Option<Result<Type, Why>> {
		let tcx = self.tcx;
		let item = |name: &str| tcx.get_diagnostic_item(rustc_span::Symbol::intern(name));
		let (read, seek) = (item("IoRead")?, item("IoSeek")?);
		let clauses = tcx.clauses_of(def_id).instantiate(tcx, tcx.mk_args(base)).clauses;
		let mut bounds = Vec::new();
		let mut other = Vec::new();
		let mut kept = false;
		for clause in clauses.iter().map(|clause| clause.skip_norm_wip()) {
			if let Some(pred) = clause.as_trait_clause()
				&& pred.self_ty().skip_binder() == param
			{
				bounds.push(pred.def_id());
			} else if let Some(pred) = clause.as_type_outlives_clause()
				&& pred.skip_binder().0 == param
			{
				kept = true;
			} else if mentions(clause, param) {
				other.push(clause.to_string());
			}
		}
		if !bounds.contains(&read) || !bounds.contains(&seek) {
			return None;
		}
		other.extend(
			(bounds.iter())
				.filter(|&&tr| tr != read && tr != seek && !marker(tcx, tr))
				.map(|&tr| tcx.def_path_str(tr)),
		);
		if !other.is_empty() {
			return Some(Err(why("reader bound", other.join(" + "))));
		}
		let sig = tcx.fn_sig(def_id).instantiate(tcx, tcx.mk_args(base)).skip_norm_wip();
		kept |= sig.skip_binder().output().walk().any(|arg| arg == param.into());
		if kept {
			return Some(Err(why("reader kept past the call", param.to_string())));
		}
		Some(Ok(Type::Reader))
	}

	/// The trait a type parameter may take a foreign implementation of: one bounded by a
	/// single exported trait without type parameters, and otherwise only by what that
	/// implementation meets. It does once the trait is declared a callback.
	fn foreign(&self, def_id: DefId, param: Ty<'tcx>) -> Option<DefId> {
		let tcx = self.tcx;
		let mut callback = None;
		for clause in tcx.clauses_of(def_id).instantiate_identity(tcx).clauses {
			let Some(bound) = clause.skip_norm_wip().as_trait_clause() else {
				continue;
			};
			if bound.self_ty().skip_binder() != param {
				continue;
			}
			let tr = bound.def_id();
			let exported = self.items.iter().any(|(item, _)| *item == tr) && !self.typed(tr);
			match callback {
				None if tcx.def_kind(tr) == DefKind::Trait && exported => callback = Some(tr),
				_ if marker(tcx, tr) => {}
				_ => return None,
			}
		}
		callback
	}

	/// The callback a type parameter takes a foreign implementation of, as declared.
	fn taken(&self, def_id: DefId, param: Ty<'tcx>) -> Option<usize> {
		match self.trait_decl(self.foreign(def_id, param)?) {
			Some(Decl::Callback(i)) => Some(*i),
			_ => None,
		}
	}

	/// `Pin<Box<dyn Future<Output = ()> + Send>>`: what Rust takes a foreign future as.
	fn foreign_future(&self) -> Option<Ty<'tcx>> {
		let tcx = self.tcx;
		let lang = tcx.lang_items();
		let future = ty::ExistentialTraitRef::new(tcx, lang.future_trait()?, ty::List::<ty::GenericArg<'tcx>>::empty());
		let output = ty::ExistentialProjection::new(
			tcx,
			lang.future_output()?,
			ty::List::<ty::GenericArg<'tcx>>::empty(),
			tcx.types.unit.into(),
		);
		let preds = tcx.mk_poly_existential_predicates(&[
			ty::Binder::dummy(ty::ExistentialPredicate::Trait(future)),
			ty::Binder::dummy(ty::ExistentialPredicate::Projection(output)),
			ty::Binder::dummy(ty::ExistentialPredicate::AutoTrait(tcx.get_diagnostic_item(sym::Send)?)),
		]);
		let boxed = Ty::new_box(tcx, Ty::new_dynamic(tcx, preds, tcx.lifetimes.re_static));
		Some(Ty::new_adt(
			tcx,
			tcx.adt_def(lang.pin_type()?),
			tcx.mk_args(&[boxed.into()]),
		))
	}

	/// Whether `ty` is `Pin<&mut F>` of [`Self::foreign_future`], as a poll of a future
	/// takes it.
	fn pinned_future(&self, ty: Ty<'tcx>) -> bool {
		let tcx = self.tcx;
		matches!(ty.kind(), ty::Adt(adt, args)
			if tcx.lang_items().pin_type() == Some(adt.did())
				&& matches!(args.type_at(0).kind(), ty::Ref(_, inner, ty::Mutability::Mut) if self.is_foreign_future(*inner)))
	}

	/// Whether `ty` is [`Self::foreign_future`], whatever its lifetime.
	fn is_foreign_future(&self, ty: Ty<'tcx>) -> bool {
		let tcx = self.tcx;
		self.foreign_future()
			.is_some_and(|future| tcx.erase_and_anonymize_regions(future) == tcx.erase_and_anonymize_regions(ty))
	}

	/// `Vec<item>`.
	fn vec_of(&self, item: Ty<'tcx>) -> Option<Ty<'tcx>> {
		let tcx = self.tcx;
		let vec = tcx.get_diagnostic_item(sym::Vec)?;
		let alloc = tcx.lang_items().global_alloc_ty()?;
		let alloc = Ty::new_adt(tcx, tcx.adt_def(alloc), ty::GenericArgs::empty());
		Some(Ty::new_adt(
			tcx,
			tcx.adt_def(vec),
			tcx.mk_args(&[item.into(), alloc.into()]),
		))
	}

	/// What `ty` yields, if it is an iterator whose items a return can take at the call with
	/// no caller able to tell: one of core's and alloc's iterators over a slice, string, or
	/// collection, or serde_json's over its map, through adapters that neither make nor
	/// compare items, and no trait object. Collecting it ends and runs no other crate's code
	/// but `Clone`, which a borrowed item crosses through anyway, and the closures and
	/// functions inside, such as `Filter`'s predicate or `Split`'s pattern, which run on
	/// every item at the call rather than only on those a caller stopping early reaches:
	/// each must drop nothing, and a `Filter` only items whose drop runs nothing, so what
	/// it discards at the call is no different from what it would discard later.
	/// A range is a value, which crosses as itself or not at all.
	fn collected(&self, ty: Ty<'tcx>, env: TypingEnv<'tcx>) -> Option<Ty<'tcx>> {
		/// Their paths below the crate, which are the same in core and alloc as in std: the
		/// iterators, and the collections an adapter may turn into one, which hand over every
		/// element, unlike a `Result` that drops its error.
		const DRAINED: &[&str] = &[
			"array::IntoIter",
			"collections::BTreeMap",
			"collections::BTreeSet",
			"collections::BinaryHeap",
			"collections::LinkedList",
			"collections::VecDeque",
			"collections::binary_heap::Drain",
			"collections::binary_heap::IntoIter",
			"collections::binary_heap::Iter",
			"collections::btree_map::IntoIter",
			"collections::btree_map::Iter",
			"collections::btree_map::IterMut",
			"collections::btree_map::Keys",
			"collections::btree_map::Values",
			"collections::btree_map::ValuesMut",
			"collections::btree_set::IntoIter",
			"collections::btree_set::Iter",
			"collections::linked_list::IntoIter",
			"collections::linked_list::Iter",
			"collections::linked_list::IterMut",
			"collections::vec_deque::Drain",
			"collections::vec_deque::IntoIter",
			"collections::vec_deque::Iter",
			"collections::vec_deque::IterMut",
			"iter::Chain",
			"iter::Cloned",
			"iter::Copied",
			"iter::Enumerate",
			"iter::Filter",
			"iter::Flatten",
			"iter::Fuse",
			"iter::Peekable",
			"iter::Rev",
			"option::IntoIter",
			"option::Option",
			"option::Iter",
			"option::IterMut",
			"result::IntoIter",
			"result::Iter",
			"result::IterMut",
			"slice::Iter",
			"slice::IterMut",
			"str::Bytes",
			"str::CharIndices",
			"str::Chars",
			"str::Lines",
			"str::Split",
			"str::SplitTerminator",
			"str::SplitWhitespace",
			"string::Drain",
			"vec::Drain",
			"vec::IntoIter",
			"vec::Vec",
		];
		let tcx = self.tcx;
		let iterator = tcx.get_diagnostic_item(sym::Iterator)?;
		let into_iterator = tcx.get_diagnostic_item(sym::IntoIterator)?;
		let (infcx, param_env) = tcx.infer_ctxt().build_with_typing_env(env);
		let implements = |tr, ty| {
			infcx
				.type_implements_trait(tr, [ty], param_env)
				.must_apply_modulo_regions()
		};
		if !matches!(ty.kind(), ty::Adt(..)) || !implements(iterator, ty) {
			return None;
		}
		// core and alloc, known by what each defines.
		let lang = tcx.lang_items();
		let std = |krate| {
			[lang.sized_trait(), lang.owned_box()]
				.into_iter()
				.flatten()
				.any(|d| d.krate == krate)
		};
		// serde_json's map iterators, which hand over every entry as alloc's do.
		const JSON: &[&str] = &["map::IntoIter", "map::Iter", "map::Keys", "map::Values"];
		let json = |krate| self.json_map.is_some_and(|map| map.krate == krate);
		let drained = |did: DefId| {
			let path = tcx.def_path_str(did);
			let listed = match () {
				_ if std(did.krate) => DRAINED,
				_ if json(did.krate) => JSON,
				_ => return false,
			};
			path.split_once("::").is_some_and(|(_, path)| listed.contains(&path))
		};
		let assoc = |tr, name: &str| {
			let item = tcx
				.associated_items(tr)
				.in_definition_order()
				.find(|item| item.name().as_str() == name);
			item.map(|item| item.def_id)
		};
		let (next, into_iter) = (assoc(iterator, "next")?, assoc(into_iterator, "into_iter")?);
		let item = tcx.get_diagnostic_item(sym::IteratorItem)?;
		let into_iter_ty = assoc(into_iterator, "IntoIter")?;
		let project = |assoc, ty| {
			let projection = Ty::new_projection(tcx, ty::IsRigid::No, assoc, [ty]);
			tcx.try_normalize_erasing_regions(env, ty::Unnormalized::new_wip(projection))
				.ok()
		};
		// Whether core's or alloc's impl of `method` is the one `ty` runs, or for one of
		// serde_json's iterators, serde_json's.
		let by_std = |method, ty: Ty<'tcx>| {
			let ours = |krate| {
				std(krate) || matches!(ty.kind(), ty::Adt(adt, _) if json(adt.did().krate) && krate == adt.did().krate)
			};
			ty::Instance::try_resolve(tcx, env, method, tcx.mk_args(&[ty.into()]))
				.is_ok_and(|instance| instance.is_some_and(|instance| ours(instance.def_id().krate)))
		};
		let filter = |ty: Ty<'tcx>| matches!(ty.kind(), ty::Adt(adt, _) if std(adt.did().krate) && tcx.def_path_str(adt.did()).ends_with("::iter::Filter"));
		// Every type the iterator holds, and every item and iterator its adapters drive,
		// which `ty`'s own arguments need not name: `Flatten` turns each item into its own.
		let mut seen = rustc_data_structures::fx::FxHashSet::default();
		let mut queue = vec![ty];
		while let Some(next_ty) = queue.pop() {
			let mut walk = next_ty.walk();
			while let Some(arg) = walk.next() {
				let Some(ty) = arg.as_type() else { continue };
				if !seen.insert(ty) {
					continue;
				}
				// A closure or function is called, not driven: what it captures is no item.
				if matches!(ty.kind(), ty::Closure(..) | ty::FnDef(..)) {
					if ty.needs_drop(tcx, env) {
						return None;
					}
					walk.skip_current_subtree();
					continue;
				}
				let plain = matches!(
					ty.kind(),
					ty::Adt(..)
						| ty::Bool
						| ty::Char
						| ty::Int(_)
						| ty::Uint(_)
						| ty::Float(_)
						| ty::Str
						| ty::Array(..)
						| ty::Slice(_)
						| ty::Ref(..)
						| ty::Tuple(_)
				);
				let (method, driven) = match (implements(iterator, ty), implements(into_iterator, ty)) {
					(true, _) => (next, item),
					(false, true) => (into_iter, into_iter_ty),
					(false, false) if plain => continue,
					(false, false) => return None,
				};
				let listed = match ty.kind() {
					ty::Adt(adt, _) => drained(adt.did()),
					_ => plain,
				};
				if !listed || !by_std(method, ty) {
					return None;
				}
				let driven = project(driven, ty)?;
				if filter(ty) && driven.needs_drop(tcx, env) {
					return None;
				}
				queue.push(driven);
			}
		}
		// Besides items, the listed iterators call into only their allocator and `Split`'s
		// pattern, whose impls must be core's or alloc's too, on a type or a borrow of one.
		let krate = tcx.lang_items().sized_trait()?.krate;
		let callee = |path: &str, method: &str| {
			let tr = (tcx.traits(krate).iter().copied())
				.find(|&tr| tcx.def_path_str(tr).split_once("::").is_some_and(|(_, p)| p == path))?;
			Some((tr, assoc(tr, method)?))
		};
		let callees = [
			callee("alloc::Allocator", "deallocate")?,
			callee("str::pattern::Pattern", "into_searcher")?,
		];
		let calls = |ty: Ty<'tcx>| (callees.iter()).any(|&(tr, method)| implements(tr, ty) && !by_std(method, ty));
		if seen.iter().any(|&ty| calls(ty)) {
			return None;
		}
		project(item, ty)
	}

	/// What `ty` resolves to when awaited, if it is a future.
	fn future_output(&self, ty: Ty<'tcx>, env: TypingEnv<'tcx>) -> Option<Ty<'tcx>> {
		let tcx = self.tcx;
		let future = tcx.lang_items().future_trait()?;
		let output = tcx.lang_items().future_output()?;
		let (infcx, param_env) = tcx.infer_ctxt().build_with_typing_env(env);
		if !infcx
			.type_implements_trait(future, [ty], param_env)
			.must_apply_modulo_regions()
		{
			return None;
		}
		let projection = Ty::new_projection(tcx, ty::IsRigid::No, output, [ty]);
		tcx.try_normalize_erasing_regions(env, ty::Unnormalized::new_wip(projection))
			.ok()
	}

	/// The lifetimes the export makes `'static`, and why a signature's borrows cannot
	/// cross. A handle outlives every call, so it holds the `'static` form of its type:
	/// each lifetime an object takes, in the impl's type, an argument, or the return, is
	/// `'static`, as is each lifetime that must outlive one. A borrow C lends lives only
	/// for the call, so it may take none of them, but a `&str` parameter, which the glue
	/// interns; every other lifetime is the call's. Also returns the inputs it interns.
	fn borrows(&self, def_id: DefId, sig: ty::FnSig<'tcx>) -> (Vec<ty::Region<'tcx>>, Vec<Why>, Vec<usize>) {
		let tcx = self.tcx;
		// Each `'long: 'short` the signature's bounds name.
		let mut outlives = Vec::new();
		for clause in tcx.clauses_of(def_id).instantiate_identity(tcx).clauses {
			let clause = clause.skip_norm_wip();
			if let Some(pred) = clause.as_region_outlives_clause() {
				let pred = pred.skip_binder();
				outlives.push((pred.0, pred.1));
			} else if let Some(pred) = clause.as_type_outlives_clause() {
				let pred = pred.skip_binder();
				outlives.extend(
					pred.0
						.walk()
						.filter_map(|arg| arg.as_region())
						.map(|long| (long, pred.1)),
				);
			}
		}
		let close = |statics: &mut Vec<ty::Region<'tcx>>| {
			while let Some(&(long, _)) = outlives
				.iter()
				.find(|(long, short)| statics.contains(short) && !statics.contains(long))
			{
				statics.push(long);
			}
		};
		let mut statics = vec![tcx.lifetimes.re_static];
		let parent = tcx.parent(def_id);
		if matches!(tcx.def_kind(parent), DefKind::Impl { .. }) {
			let self_ty = tcx.type_of(parent).instantiate_identity().skip_norm_wip();
			statics.extend(self_ty.walk().filter_map(|arg| arg.as_region()));
		}
		let mut lent = Vec::new();
		for (n, &input) in sig.inputs().iter().enumerate() {
			let (held, borrows) = self.lifetimes(input);
			statics.extend(held);
			lent.extend(borrows.into_iter().map(|region| (region, n, input)));
		}
		close(&mut statics);
		let mut errs = Vec::new();
		let mut refused = Vec::new();
		let mut interned = Vec::new();
		for &(region, n, input) in &lent {
			// A string lives as long as the program once interned, which a binding passes
			// as it passes any other.
			if statics.contains(&region)
				&& let ty::Ref(r, inner, ty::Mutability::Not) = input.kind()
				&& *r == region
				&& inner.is_str()
			{
				interned.push(n);
				continue;
			}
			if statics.contains(&region) && !refused.contains(&input) {
				refused.push(input);
				let mut why = why("'static borrow", input.to_string());
				why.note = self.region_span(def_id, region).map(|span| {
					let note = format!("`{region}` is `'static`, as a handle outlives every call");
					(span, note)
				});
				errs.push(why);
			}
		}
		// A returned object's lifetimes are `'static` too, which no lent borrow may be; not a
		// `Result`'s error of another crate, which crosses as its message, even a future's,
		// which the glue makes its message inside the future, while what it borrows lives.
		let output = self.awaited(sig.output()).unwrap_or(sig.output());
		let output = match output.kind() {
			ty::Adt(adt, args)
				if tcx.is_diagnostic_item(sym::Result, adt.did())
					&& self.opaque_value(args.type_at(1)).is_some()
					&& implements(
						tcx,
						ty::fold_regions(tcx, args.type_at(1), |_, _| tcx.lifetimes.re_erased),
						def_id,
						rustc_span::sym::Display,
					) =>
			{
				args.type_at(0)
			}
			_ => sig.output(),
		};
		let returned = statics.len();
		for region in self.lifetimes(output).0 {
			if !statics.contains(&region) {
				statics.push(region);
			}
		}
		close(&mut statics);
		if (lent.iter()).any(|(region, n, _)| !interned.contains(n) && statics[returned..].contains(region)) {
			errs.push(why("returned object that borrows", sig.output().to_string()));
		}
		(statics, errs, interned)
	}

	/// What an `impl Future` resolves to, its lifetimes kept as the signature binds them.
	fn awaited(&self, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
		let tcx = self.tcx;
		let ty::Alias(
			_,
			ty::AliasTy {
				kind: ty::AliasTyKind::Opaque { def_id },
				args,
				..
			},
		) = ty.kind()
		else {
			return None;
		};
		let output = tcx.lang_items().future_output()?;
		let bounds = tcx.item_bounds(*def_id).skip_binder();
		let term = bounds.iter().find_map(|clause| {
			let p = clause.as_projection_clause()?.skip_binder();
			(p.projection_term.expect_projection_def_id() == output).then_some(p.term.as_type()?)
		})?;
		Some(ty::EarlyBinder::bind(tcx, term).instantiate(tcx, args).skip_norm_wip())
	}

	/// Where a named lifetime of `def_id`'s signature is declared.
	fn region_span(&self, def_id: DefId, region: ty::Region<'tcx>) -> Option<Span> {
		let tcx = self.tcx;
		match region.kind() {
			ty::ReEarlyParam(param) => Some(tcx.def_span(tcx.generics_of(def_id).region_param(param, tcx).def_id)),
			ty::ReBound(_, bound) => match bound.kind {
				ty::BoundRegionKind::Named(id) => Some(tcx.def_span(id)),
				_ => None,
			},
			_ => None,
		}
	}

	/// Whether the output of the future `future` may name a lifetime outside `statics`,
	/// whatever type holds it: an `impl Future`'s by what its bounds name; another future's,
	/// whose lifetimes `output` has erased, by naming any at all.
	fn output_borrows(&self, future: Ty<'tcx>, output: Ty<'tcx>, statics: &[ty::Region<'tcx>]) -> bool {
		let tcx = self.tcx;
		let ty::Alias(
			_,
			ty::AliasTy {
				kind: ty::AliasTyKind::Opaque { def_id },
				args,
				..
			},
		) = future.kind()
		else {
			return output.walk().any(|arg| arg.as_region().is_some());
		};
		// Elaborated, as a supertrait may name the output.
		(tcx.item_bounds(*def_id).skip_binder().iter())
			.filter_map(|clause| clause.as_projection_clause()?.skip_binder().term.as_type())
			.map(|term| ty::EarlyBinder::bind(tcx, term).instantiate(tcx, args).skip_norm_wip())
			.any(|term| {
				term.walk()
					.any(|arg| arg.as_region().is_some_and(|r| !statics.contains(&r)))
			})
	}

	/// The lifetimes in `ty`: those its objects take, then those it borrows with.
	fn lifetimes(&self, ty: Ty<'tcx>) -> (Vec<ty::Region<'tcx>>, Vec<ty::Region<'tcx>>) {
		let mut held = Vec::new();
		let mut lent = Vec::new();
		for ty in ty.walk().filter_map(|arg| arg.as_type()) {
			match ty.kind() {
				ty::Ref(region, ..) => lent.push(*region),
				// An object, an opaque value, or another crate's value an opaque handle holds, holds
				// what its lifetime parameters name.
				ty::Adt(adt, args)
					if matches!(self.decl(adt.did(), *args), Some(Decl::Object(_) | Decl::Value(_)))
						|| self.opaque_value(ty).is_some() =>
				{
					held.extend(args.regions())
				}
				// An `impl Future` holds what its bounds name, as an `async fn` its output.
				ty::Alias(
					_,
					ty::AliasTy {
						kind: ty::AliasTyKind::Opaque { def_id },
						args,
						..
					},
				) => {
					let tcx = self.tcx;
					// Elaborated, as a supertrait may be the future or name its output.
					let bounds = tcx.item_bounds(*def_id).skip_binder();
					// Any other `impl Trait` holds what its hidden type holds, as `impl Iterator + '_`
					// over an object borrowing the receiver does; one collected at the call, or a
					// future, whose borrows are checked apart, holds nothing past it.
					let future = (bounds.iter()).any(|clause| {
						clause
							.as_trait_clause()
							.is_some_and(|tr| tcx.is_lang_item(tr.def_id(), LangItem::Future))
					});
					if !future {
						// Revealed through each `impl Trait` it returns in turn, as `function` reveals it.
						let mut hidden = tcx.type_of(*def_id).instantiate(tcx, args).skip_norm_wip();
						while let ty::Alias(
							_,
							ty::AliasTy {
								kind: ty::AliasTyKind::Opaque { def_id },
								args,
								..
							},
						) = hidden.kind()
						{
							hidden = tcx.type_of(*def_id).instantiate(tcx, args).skip_norm_wip();
						}
						// One whose hidden type is an object crosses as its handle, which the glue
						// keeps as the opaque type itself, and one whose hidden type is a value as
						// its bytes, so each holds every lifetime it captures, even one its hidden
						// type does not borrow; one collected at the call holds nothing past it,
						// and any other is refused. Looked up with regions erased, since they may
						// be the signature's bound ones, which a key never names.
						let erased = ty::fold_regions(tcx, hidden, |_, _| tcx.lifetimes.re_erased);
						let object = matches!(erased.kind(), ty::Adt(adt, args)
							if matches!(self.decl(adt.did(), *args), Some(Decl::Object(_) | Decl::Value(_))));
						if object {
							held.extend(args.iter().flat_map(|arg| arg.walk()).filter_map(|arg| arg.as_region()));
						}
					}
					for clause in bounds {
						let term = clause
							.as_projection_clause()
							.and_then(|p| p.skip_binder().term.as_type());
						// Instantiated alone, outside the clause's binder, so the call's own
						// lifetimes stay as the signature binds them.
						if let Some(term) = term {
							let term = ty::EarlyBinder::bind(tcx, term).instantiate(tcx, args).skip_norm_wip();
							let (h, l) = self.lifetimes(term);
							held.extend(h);
							lent.extend(l);
						}
					}
				}
				_ => {}
			}
		}
		(held, lent)
	}

	/// What a function returns: a tuple returned whole, even a pair of bounds, is written
	/// through an out pointer per element.
	fn ret(&self, ty: Ty<'tcx>) -> Result<Type, Why> {
		match ty.kind() {
			ty::Tuple(list) if !list.is_empty() => self.tuple(ty, Pos::Return),
			_ => self.ty(ty, Pos::Return),
		}
	}

	/// The tuple `ty`, each element mapped as inside a value at `pos`.
	fn tuple(&self, ty: Ty<'tcx>, pos: Pos) -> Result<Type, Why> {
		let elements = (ty.tuple_fields().iter())
			.map(|element| self.ty(element, pos.inner()))
			.collect::<Result<Vec<_>, _>>()?;
		// C lends nothing inside a value, and an element lowered back into one is a clone
		// of what Rust lent.
		if elements.iter().any(borrows) && !matches!(pos, Pos::Return | Pos::Returned) {
			return Err(why("tuple of references", ty.to_string()));
		}
		Ok(Type::Tuple(elements))
	}

	/// `(Bound<T>, Bound<T>)`, which a `RangeBounds<T>` parameter takes.
	fn bounds(&self, int: Ty<'tcx>) -> Option<Ty<'tcx>> {
		let tcx = self.tcx;
		let bound = Ty::new_adt(tcx, tcx.adt_def(self.bound?), tcx.mk_args(&[int.into()]));
		Some(Ty::new_tup(tcx, &[bound, bound]))
	}

	/// The integer a `Bound` holds.
	fn bound_of(&self, ty: Ty<'tcx>) -> Option<Int> {
		match ty.kind() {
			ty::Adt(adt, args) if Some(adt.did()) == self.bound => int(args.type_at(0)),
			_ => None,
		}
	}

	/// Whether `ty` holds a handle a copy would duplicate: an object that is not `Copy`, or any opaque one.
	fn holds_handle(&self, ty: Ty<'tcx>) -> bool {
		let tcx = self.tcx;
		ty.walk().filter_map(|arg| arg.as_type()).any(|ty| match ty.kind() {
			ty::Adt(adt, args) => {
				// An opaque handle is never copied, even of a `Copy` value: lifting takes it. A
				// type [`FOREIGN`] maps by its role is an object.
				let foreign = self.foreign.iter().any(|&(ty, _)| ty == adt.did());
				let opaque = foreign && self.opaque(adt.did());
				let object = foreign || matches!(self.decl(adt.did(), *args), Some(Decl::Object(_)));
				opaque || object && !implements(tcx, ty, adt.did(), sym::Copy)
			}
			_ => false,
		})
	}

	fn is_clone(&self, ty: Ty<'tcx>) -> bool {
		let tcx = self.tcx;
		let Some(clone) = tcx.lang_items().clone_trait() else {
			return false;
		};
		let (infcx, env) = tcx.infer_ctxt().build_with_typing_env(TypingEnv::fully_monomorphized());
		infcx
			.type_implements_trait(clone, [ty], env)
			.must_apply_modulo_regions()
	}

	/// Whether a clone of the type `ty`, which `def_id` declares, is a value no shared
	/// borrow can tell apart from the original but by an `Arc`'s or `Rc`'s count, so copies
	/// may share one: each part of it is reached through an `Arc`, an `Rc`, or a shared
	/// reference, which a clone shares, or never changes behind `&`. A `Cell` or `Mutex` it owns, even inside a `Box`,
	/// changes apart in a clone, and a `Drop` or a `Clone` written by hand may count
	/// clones, so any of them, or a type it cannot see through, keeps clones apart.
	fn clone_shares(&self, ty: Ty<'tcx>, def_id: DefId) -> bool {
		let env = TypingEnv::post_analysis(self.tcx, def_id);
		self.shares(ty, env, &mut Default::default())
	}

	fn shares(
		&self,
		ty: Ty<'tcx>,
		env: TypingEnv<'tcx>,
		seen: &mut rustc_data_structures::fx::FxHashSet<Ty<'tcx>>,
	) -> bool {
		let tcx = self.tcx;
		let ty = tcx
			.try_normalize_erasing_regions(env, ty::Unnormalized::new_wip(ty))
			.unwrap_or(ty);
		match ty.kind() {
			ty::Bool | ty::Char | ty::Int(_) | ty::Uint(_) | ty::Float(_) | ty::Str | ty::Never | ty::FnPtr(..) => true,
			ty::Ref(_, _, ty::Mutability::Not) => true,
			ty::Array(inner, _) | ty::Slice(inner) => self.shares(*inner, env, seen),
			ty::Tuple(elements) => elements.iter().all(|e| self.shares(e, env, seen)),
			// Met again through a type that holds itself: what else it holds decides.
			ty::Adt(..) if !seen.insert(ty) => true,
			ty::Adt(adt, args) => {
				let did = adt.did();
				let std = |names: &[&str]| {
					(names.iter()).any(|name| tcx.is_diagnostic_item(rustc_span::Symbol::intern(name), did))
				};
				if adt.is_unsafe_cell() || adt.is_union() {
					false
				} else if adt.is_phantom_data() || Some(did) == self.bytes || std(&["Arc", "Rc", "ArcWeak", "RcWeak"]) {
					true
				} else if adt.is_box()
					|| tcx.lang_items().string() == Some(did)
					|| std(&[
						"Option", "Result", "Vec", "VecDeque", "HashMap", "HashSet", "BTreeMap", "BTreeSet", "Cow",
					]) {
					// std's own `Clone`, written by hand, clones each element.
					args.types().all(|arg| self.shares(arg, env, seen))
				} else if self.sysroot.contains(&did.krate) && tcx.type_is_copy_modulo_regions(env, ty) {
					// std's `Clone` of a `Copy` type, such as `NonZero`'s, copies it, as std's own
					// specializations of `Clone` for `Copy` types assume.
					true
				} else {
					let derived = tcx.lang_items().clone_fn().is_some_and(|clone| {
						ty::Instance::try_resolve(tcx, env, clone, tcx.mk_args(&[ty.into()]))
							.ok()
							.flatten()
							.and_then(|instance| tcx.impl_of_assoc(instance.def_id()))
							.is_some_and(|imp| tcx.is_automatically_derived(imp))
					});
					derived
						&& tcx.adt_destructor(did).is_none()
						&& adt
							.all_fields()
							.all(|field| self.shares(field.ty(tcx, *args).skip_norm_wip(), env, seen))
				}
			}
			_ => false,
		}
	}

	/// What a parameter of type `ty` polls its task through, if it does: a `&Context` or
	/// `&mut Context`, or where the function returns a `Poll`, a reference to an object
	/// exactly one safe, non-generic associated function makes from a `Waker` alone, of a
	/// type with no generic parameters, such as kio's `Waiter`.
	fn task(&self, ty: Ty<'tcx>, polled: bool) -> Option<TaskKind> {
		let tcx = self.tcx;
		let ty::Ref(_, inner, m) = *ty.kind() else {
			return None;
		};
		let ty::Adt(adt, args) = *inner.kind() else {
			return None;
		};
		if tcx.is_lang_item(adt.did(), LangItem::Context) {
			return Some(TaskKind::Context(match m {
				ty::Mutability::Not => Mutability::Not,
				ty::Mutability::Mut => Mutability::Mut,
			}));
		}
		// A generic type's constructor may belong to another of its instances, and the glue
		// cannot name an opaque handle's type to call its constructor.
		if !polled || m == ty::Mutability::Mut || !args.is_empty() || self.opaque_value(inner).is_some() {
			return None;
		}
		let Ok(Type::Object(object)) = self.ty(inner, Pos::Behind) else {
			return None;
		};
		let waker = |ty: Ty<'tcx>| matches!(ty.kind(), ty::Adt(w, _) if tcx.is_diagnostic_item(rustc_span::Symbol::intern("Waker"), w.did()));
		let makes: Vec<String> = (tcx.inherent_impls(adt.did()).iter())
			.flat_map(|&imp| tcx.associated_items(imp).in_definition_order())
			.filter(|item| {
				matches!(item.kind, ty::AssocKind::Fn { has_self: false, .. })
					&& tcx.visibility(item.def_id).is_public()
			})
			.filter(|item| {
				let sig = tcx.fn_sig(item.def_id).skip_binder().skip_binder();
				// The call names no generic arguments and opens no `unsafe` block.
				let generics = tcx.generics_of(item.def_id).own_counts();
				sig.safety().is_safe()
					&& generics.types + generics.consts == 0
					&& matches!(sig.inputs(), [input] if waker(*input))
					&& matches!(sig.output().kind(), ty::Adt(out, _) if out.did() == adt.did())
			})
			.map(|item| item.name().to_string())
			.collect();
		match <[String; 1]>::try_from(makes) {
			Ok([make]) => Some(TaskKind::Made { object, make }),
			Err(_) => None,
		}
	}

	/// Whether `ty` is `Sync`, as a handle shared across threads must be.
	fn is_sync(&self, ty: Ty<'tcx>) -> bool {
		let tcx = self.tcx;
		tcx.lang_items().sync_trait().is_some_and(|sync| {
			let (infcx, env) = tcx.infer_ctxt().build_with_typing_env(TypingEnv::fully_monomorphized());
			infcx.type_implements_trait(sync, [ty], env).must_apply_modulo_regions()
		})
	}

	fn is_send(&self, ty: Ty<'tcx>, env: TypingEnv<'tcx>) -> bool {
		let tcx = self.tcx;
		let Some(send) = tcx.get_diagnostic_item(sym::Send) else {
			return true;
		};
		let (infcx, param_env) = tcx.infer_ctxt().build_with_typing_env(env);
		infcx
			.type_implements_trait(send, [ty], param_env)
			.must_apply_modulo_regions()
	}

	/// A constant: a C constant if a `bool` or integer, or else a function returning it,
	/// which the caller adds. One of an instance's impl, at its `parent` arguments, is
	/// always a function, as its value may differ by instance.
	fn constant_item(&mut self, def_id: DefId, path: Path, parent: Option<ty::GenericArgs<'tcx>>) -> Option<Function> {
		let tcx = self.tcx;
		let span = tcx.def_span(def_id);
		let ty = match parent {
			Some(parent) => tcx.type_of(def_id).instantiate(tcx, parent).skip_norm_wip(),
			None => {
				let why = match self.constant(def_id) {
					Ok(value) => {
						let home = self.home(def_id);
						self.api.constants.push(Constant {
							path,
							home,
							span,
							on: On::ALL,
							docs: docs(tcx, def_id),
							value,
						});
						return None;
					}
					Err(why) => why,
				};
				if why.construct != "constant type" {
					self.refuse(&path, "const", span, vec![why]);
					return None;
				}
				tcx.type_of(def_id).instantiate_identity().skip_norm_wip()
			}
		};
		match self.ty(ty, Pos::Return) {
			Ok(ok) => Some(Function {
				path: path.clone(),
				shown: path,
				span,
				on: On::ALL,
				docs: docs(tcx, def_id),
				receiver: None,
				params: Vec::new(),
				units: Vec::new(),
				ok,
				err: None,
				future: false,
				eager: false,
				holds: false,
				task: None,
				collect: false,
				lends: false,
				access: Access::Const,
				instance: None,
				home: Home::default(),
			}),
			Err(why) => {
				self.refuse(&path, "const", span, vec![why]);
				None
			}
		}
	}

	/// A constant's value, if it is a `bool` or integer.
	fn constant(&self, def_id: DefId) -> Result<Literal, Why> {
		let tcx = self.tcx;
		let ty = tcx.type_of(def_id).instantiate_identity().skip_norm_wip();
		let int = match ty.kind() {
			ty::Bool => None,
			ty::Int(_) | ty::Uint(_) => match int(ty) {
				Some(int) => Some(int),
				None => return Err(why("constant type", ty.to_string())),
			},
			_ => return Err(why("constant type", ty.to_string())),
		};
		let Ok(value) = tcx.const_eval_poly(def_id) else {
			return Err(why("constant value", ""));
		};
		let Some(scalar) = value.try_to_scalar_int() else {
			return Err(why("constant value", ""));
		};
		let bits = scalar.to_bits(scalar.size());
		Ok(match int {
			None => Literal::Bool(bits != 0),
			Some(int) => Literal::Int(int, signed(tcx, ty, bits)),
		})
	}

	/// The exported form of `ty` where it appears.
	fn ty(&self, ty: Ty<'tcx>, pos: Pos) -> Result<Type, Why> {
		let tcx = self.tcx;
		Ok(match ty.kind() {
			ty::Tuple(list) if list.is_empty() => match pos {
				Pos::Return => Type::Unit,
				_ => return Err(why("unit outside a return", "")),
			},
			ty::Bool => Type::Bool,
			ty::Char => Type::Char,
			ty::Float(ty::FloatTy::F32) => Type::F32,
			ty::Float(ty::FloatTy::F64) => Type::F64,
			ty::Uint(ty::UintTy::U128) => Type::U128,
			ty::Int(_) | ty::Uint(_) => Type::Int(int(ty).ok_or_else(|| why("128-bit integer", ty.to_string()))?),
			ty::Ref(_, inner, m) => {
				let inner_ty = match inner.kind() {
					ty::Str => Type::Str,
					ty::Adt(adt, _) if tcx.is_diagnostic_item(rustc_span::Symbol::intern("Path"), adt.did()) => {
						Type::Path
					}
					ty::Slice(elem) if *elem == tcx.types.u8 => Type::Slice,
					// An array, as a `Vec`'s, which C passes and Rust returns as a clone.
					ty::Slice(elem) => {
						let item = self.ty(
							*elem,
							match pos {
								Pos::Return | Pos::Returned => Pos::Returned,
								_ => Pos::Behind,
							},
						)?;
						if borrows(&item) && !matches!(pos, Pos::Return | Pos::Returned) {
							return Err(why("slice of references", elem.to_string()));
						}
						Type::List(Collection::Slice, Box::new(item))
					}
					ty::Dynamic(..) => return Err(why("trait object", inner.to_string())),
					// A returned `&Waker` crosses as its clone, as any borrowed return does.
					ty::Adt(adt, _)
						if pos == Pos::Return
							&& tcx.is_diagnostic_item(rustc_span::Symbol::intern("Waker"), adt.did()) =>
					{
						Type::Waker
					}
					// A reader C implements, lent for the call.
					ty::Param(_)
						if pos == Pos::Param
							&& *m == ty::Mutability::Mut
							&& (self.impls.borrow().iter())
								.any(|(param, taken)| param == inner && *taken == Type::Reader) =>
					{
						Type::Reader
					}
					_ => self.ty(
						*inner,
						match pos {
							Pos::Return | Pos::Returned => Pos::Returned,
							_ => Pos::Behind,
						},
					)?,
				};
				let mutability = match m {
					ty::Mutability::Not => Mutability::Not,
					ty::Mutability::Mut => Mutability::Mut,
				};
				match (&inner_ty, pos, mutability) {
					// An opaque handle holds its `Arc` erased, which Rust cannot replace in place.
					(Type::Object(_), _, Mutability::Mut) if self.opaque_arc(*inner).is_some() => {
						return Err(why("&mut of another crate's Arc", inner.to_string()));
					}
					// Returned as a clone: a handle in it is a new handle to its clone.
					(Type::Str | Type::Slice | Type::Path, Pos::Return | Pos::Returned, _) => {}
					// Lent from the receiver's handle, as [`Self::function`] decides, but for
					// another crate's value, which a handle holds only erased and by value.
					(Type::Object(_), Pos::Return, Mutability::Not)
						if self.lending.take() && self.opaque_value(*inner).is_none() => {}
					// A slice's clone is a `Vec` of its elements' clones.
					(_, Pos::Return | Pos::Returned, _)
						if !self.is_clone(match inner.kind() {
							ty::Slice(elem) => *elem,
							_ => *inner,
						}) =>
					{
						return Err(why("borrowed return that is not Clone", inner.to_string()));
					}
					(_, Pos::Field, _) => return Err(why("reference field", inner.to_string())),
					(Type::Object(_), Pos::Param, _) => {}
					// Lent only through a handle: one inside a `Box` or `Option` crosses by
					// value, which would move it out of the caller's handle.
					(_, Pos::Param, _) if self.holds_handle(*inner) => {
						return Err(why("reference to a handle inside a value", inner.to_string()));
					}
					// C passes the value in, and Rust writes back what the call left, but C
					// lends no buffer mutably.
					(_, Pos::Param, Mutability::Mut) if matches!(inner.kind(), ty::Str | ty::Slice(_)) => {
						return Err(why("&mut of a slice", inner.to_string()));
					}
					_ => {}
				}
				Type::Ref(mutability, Box::new(inner_ty))
			}
			ty::Adt(adt, args) => {
				let did = adt.did();
				if tcx.lang_items().string() == Some(did) {
					Type::String
				} else if tcx.is_diagnostic_item(sym::Vec, did) {
					let elem = args.type_at(0);
					if elem == tcx.types.u8 {
						Type::Vec
					} else {
						let inner = self.ty(elem, pos.inner())?;
						// C lends nothing inside an array, and an element lowered back into
						// one is a clone of what Rust lent.
						if borrows(&inner) && !matches!(pos, Pos::Return | Pos::Returned) {
							return Err(why("Vec of references", elem.to_string()));
						}
						Type::List(Collection::Vec, Box::new(inner))
					}
				} else if let Some(c) = (tcx.is_diagnostic_item(rustc_span::Symbol::intern("BTreeSet"), did))
					.then_some(Collection::BTreeSet)
					.or_else(|| {
						(tcx.is_diagnostic_item(rustc_span::Symbol::intern("BTreeMap"), did))
							.then_some(Collection::BTreeMap)
					})
					// serde_json's own, unless serde_json is an export.
					.or_else(|| (Some(did) == self.json_map && !self.walked(did.krate)).then_some(Collection::JsonMap))
				{
					// A set crosses as its items, and a map as its pairs, in order.
					let inner = match c {
						Collection::BTreeSet => self.ty(args.type_at(0), pos.inner())?,
						_ => Type::Tuple(vec![
							self.ty(args.type_at(0), pos.inner())?,
							self.ty(args.type_at(1), pos.inner())?,
						]),
					};
					if !matches!(pos, Pos::Return | Pos::Returned) {
						if borrows(&inner) {
							return Err(why("collection of references", ty.to_string()));
						}
						// Rust builds what C passes from its items, which a set of floats,
						// say, cannot be built from.
						let item = match c {
							Collection::BTreeSet => args.type_at(0),
							_ => Ty::new_tup(tcx, &[args.type_at(0), args.type_at(1)]),
						};
						let builds = tcx.get_diagnostic_item(sym::FromIterator).is_some_and(|tr| {
							let (infcx, env) = tcx.infer_ctxt().build_with_typing_env(TypingEnv::fully_monomorphized());
							infcx
								.type_implements_trait(tr, [ty, item], env)
								.must_apply_modulo_regions()
						});
						if !builds {
							return Err(why("collection not built from its items", ty.to_string()));
						}
					}
					Type::List(c, Box::new(inner))
				} else if Some(did) == self.bytes {
					Type::Bytes
				} else if tcx.is_diagnostic_item(rustc_span::Symbol::intern("Duration"), did) {
					Type::Duration
				} else if tcx.is_diagnostic_item(rustc_span::Symbol::intern("NonZero"), did) {
					Type::NonZero(int(args.type_at(0)).ok_or_else(|| why("NonZero", args.type_at(0).to_string()))?)
				} else if tcx.is_diagnostic_item(rustc_span::Symbol::intern("PathBuf"), did) {
					Type::PathBuf
				} else if let Some(family) = (tcx.lang_items().sized_trait().map(|d| d.krate) == Some(did.krate))
					.then(|| match tcx.item_name(did).as_str() {
						"SocketAddr" => Some(Family::Any),
						"SocketAddrV4" => Some(Family::V4),
						"SocketAddrV6" => Some(Family::V6),
						_ => None,
					})
					.flatten()
				{
					Type::SocketAddr(family)
				} else if let Some(&(_, i)) = (self.foreign.iter()).find(|&&(ty, _)| ty == did && !self.opaque(did)) {
					// A value holds only values.
					match pos {
						_ if self.foreign_value(did) => Type::Record(i),
						Pos::Field => return Err(why("object in a value", tcx.def_path_str(did))),
						_ => Type::Object(i),
					}
				} else if tcx.lang_items().pin_type() == Some(did) {
					match pos {
						Pos::Param if self.is_foreign_future(ty) => Type::Future,
						_ => return Err(why("Pin", ty.to_string())),
					}
				} else if let Some(level) = self.level.filter(|l| l.level == did || l.filter == did) {
					let ty = Type::Enum(level.index);
					match did == level.level {
						true => ty,
						// `OFF` is the filter that lets no level through.
						false => Type::Option(Box::new(ty)),
					}
				} else if tcx.is_diagnostic_item(rustc_span::Symbol::intern("Waker"), did) {
					match pos {
						Pos::Param | Pos::Return => Type::Waker,
						_ => return Err(why("Waker outside a parameter or a return", "")),
					}
				} else if tcx.is_lang_item(did, LangItem::Context) {
					// A context is only a waker lent for a call, so a returned one crosses as
					// its waker's clone.
					match pos {
						Pos::Return => Type::Waker,
						_ => return Err(why("Context outside a return or a polled parameter", "")),
					}
				} else if tcx.lang_items().range_struct() == Some(did) {
					Type::Range(int(args.type_at(0)).ok_or_else(|| why("Range", args.type_at(0).to_string()))?)
				} else if Some(did) == self.bound {
					Type::Bound(int(args.type_at(0)).ok_or_else(|| why("Bound", args.type_at(0).to_string()))?)
				} else if tcx.is_diagnostic_item(sym::Option, did) {
					// A parameter's optional borrow is lent as the parameter itself is: a null
					// handle or buffer for `None`.
					let inner_pos = match (pos, args.type_at(0).kind()) {
						(Pos::Param, ty::Ref(..)) => Pos::Param,
						_ => pos.inner(),
					};
					let inner = self.ty(args.type_at(0), inner_pos)?;
					// C has one `None` per option, so it cannot hold one inside another, even
					// through a `Box`.
					if matches!(inner.base(), Type::Option(_)) {
						return Err(why("nested Option", ty.to_string()));
					}
					Type::Option(Box::new(inner))
				} else if tcx.is_diagnostic_item(sym::Result, did) {
					return Err(why("Result outside a return", ""));
				} else if adt.is_box() {
					let inner = args.type_at(0);
					match inner.kind() {
						ty::Dynamic(preds, ..) => {
							return self.dynamic(preds.principal_def_id(), Dyn::Box, inner, pos);
						}
						// A boxed slice crosses as a `Vec`'s array does, but of bytes, which
						// no buffer form takes.
						ty::Slice(elem) if *elem == tcx.types.u8 => {
							return Err(why("boxed slice of bytes", ty.to_string()));
						}
						ty::Slice(elem) => {
							let item = self.ty(*elem, pos.inner())?;
							if borrows(&item) && !matches!(pos, Pos::Return | Pos::Returned) {
								return Err(why("boxed slice of references", elem.to_string()));
							}
							Type::List(Collection::Box, Box::new(item))
						}
						_ => Type::Box(Box::new(self.ty(inner, pos.inner())?)),
					}
				} else if tcx.is_diagnostic_item(sym::Arc, did) {
					let inner = args.type_at(0);
					match inner.kind() {
						ty::Dynamic(preds, ..) => {
							return self.dynamic(preds.principal_def_id(), Dyn::Arc, inner, pos);
						}
						// Held erased, as a handle with none of its API, which Rust takes back.
						_ => match (self.opaque_arc(ty))
							.and_then(|inner| self.foreign.iter().find(|&&(other, _)| other == inner))
						{
							Some(&(did, _))
								if self.orphans.contains(&Key {
									def_id: did,
									args: None,
								}) =>
							{
								return Err(why("type C never holds", inner.to_string()));
							}
							Some(_) if pos == Pos::Field => {
								return Err(why("object in a value", inner.to_string()));
							}
							Some(&(_, i)) => Type::Object(i),
							None => return Err(why("Arc", strip_args(tcx, did))),
						},
					}
				} else if let Some(did) = self.opaque_value(ty)
					&& let Some(&(_, i)) = self.foreign.iter().find(|&&(other, _)| other == did)
				{
					// Held by value, as a handle with none of its API, which Rust takes back
					// or borrows.
					if self.both.contains(&did) {
						return Err(why(BOTH, strip_args(tcx, did)));
					}
					if self.orphans.contains(&Key {
						def_id: did,
						args: None,
					}) {
						return Err(why("type C never holds", strip_args(tcx, did)));
					}
					match pos {
						Pos::Field => return Err(why("object in a value", strip_args(tcx, did))),
						_ => Type::Object(i),
					}
				} else if !self.walked(did.krate) && (self.erring.get() || !self.reach.contains(&did)) {
					// One public in its own crate exports by that path, in the next pass, unless
					// it is an error: one in a `Result` crosses as its message, and one elsewhere
					// is held by value, as above, or refused.
					if !self.erring.get() && !self.reach.contains(&did) && self.reaches(ty, did) {
						self.reached(did);
					}
					return Err(why("foreign type", strip_args(tcx, did)));
				} else {
					let key = self.key(did, *args);
					// An instance is named as its key would spell it.
					let name = match key.args {
						Some(_) => self.spell(ty),
						None => tcx.def_path_str(did),
					};
					match self.decls.get(&key) {
						Some(Decl::Record(i) | Decl::Value(i)) => Type::Record(*i),
						Some(Decl::Enum(i)) => Type::Enum(*i),
						// A value holds only values, and handles it can copy.
						Some(Decl::Object(_)) if pos == Pos::Field && !implements(tcx, ty, did, sym::Copy) => {
							return Err(why("object in a value", name));
						}
						Some(Decl::Object(i)) => Type::Object(*i),
						Some(Decl::Refused(refused)) if UNHELD.contains(&refused.construct) => {
							return Err(why("type C never holds", name));
						}
						Some(Decl::Refused(_)) => return Err(why("refused type", name)),
						Some(Decl::Callback(_)) | None => {
							if let Some(construct) = self.left_out.get(&did) {
								return Err(why(construct, name));
							}
							if let Some(why) = self.rejected.get(&key) {
								return Err(why.clone());
							}
							if args.has_non_region_param() {
								return Err(why("type parameter", name));
							}
							// Inference admits or rejects each instance a signature names, so one
							// it did not is a gap in inference, which even a census fails on.
							if key.args.is_some() && self.items.iter().any(|(item, _)| *item == did) {
								let mut why = why(NOT_INFERRED, name);
								let note =
									format!("`{}` exports only at the instances it infers", strip_args(tcx, did));
								why.note = Some((tcx.def_span(did), note));
								return Err(why);
							}
							// One public in its own crate exports by that path, in the next pass.
							if !self.reach.contains(&did) && self.reachable(did) {
								self.reached(did);
							}
							return Err(why("type not reachable from the crate root", name));
						}
					}
				}
			}
			ty::Tuple(list) => match list.as_slice() {
				[start, end]
					if start == end
						&& let Some(int) = self.bound_of(*start) =>
				{
					Type::Bounds(int)
				}
				_ => self.tuple(ty, pos)?,
			},
			// An element type beyond `u8` takes the same shape once an export needs one.
			ty::Array(elem, len) if *elem == tcx.types.u8 => {
				let n = len
					.try_to_target_usize(tcx)
					.ok_or_else(|| why("array of unknown length", ty.to_string()))?;
				// C has no struct of no bytes.
				match usize::try_from(n) {
					Ok(0) => return Err(why("empty array", ty.to_string())),
					Ok(n) => Type::Array(n),
					Err(_) => return Err(why("array too long", ty.to_string())),
				}
			}
			ty::Array(..) => return Err(why("array of an element other than u8", ty.to_string())),
			ty::Slice(_) | ty::Str => return Err(why("unsized type", ty.to_string())),
			ty::Dynamic(..) => return Err(why("trait object", ty.to_string())),
			ty::Param(_) => match self.impls.borrow().iter().find(|(param, _)| *param == ty) {
				Some((_, Type::Reader)) => return Err(why("reader not lent as &mut", ty.to_string())),
				Some((_, taken)) if pos == Pos::Param => taken.clone(),
				_ => return Err(why("type parameter", ty.to_string())),
			},
			ty::Closure(..) | ty::FnDef(..) | ty::FnPtr(..) => return Err(why("function value", "")),
			ty::RawPtr(..) => return Err(why("raw pointer", "")),
			ty::Never => return Err(why("never type", "")),
			ty::Alias(..) => return Err(why("opaque type", ty.to_string())),
			_ => return Err(why("type", ty.to_string())),
		})
	}

	/// A trait object of one of the crate's traits, as a foreign implementation.
	/// A foreign implementation crosses only into Rust, as a parameter.
	fn dynamic(&self, principal: Option<DefId>, form: Dyn, ty: Ty<'tcx>, pos: Pos) -> Result<Type, Why> {
		let mut refusal = why("trait object", ty.to_string());
		match principal.map(|tr| (tr, self.trait_decl(tr))) {
			Some((_, Some(Decl::Callback(i)))) if pos == Pos::Param => return Ok(Type::Callback(*i, form)),
			// Left out of the export, so the reason it is shows where a signature needs it.
			Some((tr, Some(Decl::Refused(left)))) => {
				let reason = match left.detail.is_empty() {
					true => left.construct.to_string(),
					false => format!("{} `{}`", left.construct, left.detail),
				};
				let note = format!("C cannot implement `{}`: {reason}", self.tcx.def_path_str(tr));
				refusal.note = Some((self.tcx.def_span(tr), note));
			}
			_ => {}
		}
		Err(refusal)
	}
}

/// Each type a visited value names, at any depth.
struct Types<'tcx>(Vec<Ty<'tcx>>);

impl<'tcx> TypeVisitor<TyCtxt<'tcx>> for Types<'tcx> {
	fn visit_ty(&mut self, ty: Ty<'tcx>) {
		self.0.push(ty);
		ty.super_visit_with(self);
	}
}

/// Where a type appears, which decides the forms it may take.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pos {
	Param,
	Return,
	Field,
	/// Inside what a function returns: an `Option`, a `Box`, or behind a reference.
	Returned,
	/// Inside another type.
	Behind,
}

/// Whether a callback can take `ty`: lent to the foreign side for the call, and
/// copied by it if kept.
fn foreign_arg(ty: &Type) -> bool {
	match ty {
		Type::Bool
		| Type::Int(_)
		| Type::NonZero(_)
		| Type::F32
		| Type::F64
		| Type::Char
		| Type::Duration
		| Type::SocketAddr(_) => true,
		Type::String | Type::Vec | Type::Bytes | Type::PathBuf => true,
		Type::Ref(Mutability::Not, inner) => matches!(**inner, Type::Str | Type::Slice | Type::Path),
		Type::Option(inner) => foreign_arg(inner),
		_ => false,
	}
}

/// Whether C can make a `ty` to pass, holding handles only to the objects in `held`.
fn buildable(api: &Api, ty: &Type, held: &std::collections::HashSet<Type>) -> bool {
	match ty {
		Type::Object(_) => held.contains(ty),
		Type::Record(i) if api.records[*i].opaque.is_some() => held.contains(ty),
		Type::Ref(_, inner) | Type::Box(inner) => buildable(api, inner, held),
		Type::Record(i) => api.records[*i].fields.iter().all(|f| buildable(api, &f.ty, held)),
		Type::Tuple(elements) => elements.iter().all(|ty| buildable(api, ty, held)),
		Type::Enum(i) => api.enums[*i]
			.variants
			.iter()
			.any(|v| v.fields.iter().all(|f| buildable(api, &f.ty, held))),
		_ => true,
	}
}

/// The objects a returned `ty` may hand C a handle to, and the opaque values it may hand C.
fn returns(api: &Api, ty: &Type, objects: &mut Vec<Type>) {
	match ty {
		Type::Object(_) => objects.push(ty.clone()),
		Type::Record(i) if api.records[*i].opaque.is_some() => objects.push(ty.clone()),
		Type::Ref(_, inner) | Type::Box(inner) | Type::Option(inner) | Type::List(_, inner) => {
			returns(api, inner, objects)
		}
		Type::Tuple(elements) => elements.iter().for_each(|ty| returns(api, ty, objects)),
		Type::Record(i) => api.records[*i].fields.iter().for_each(|f| returns(api, &f.ty, objects)),
		Type::Enum(i) => api.enums[*i]
			.variants
			.iter()
			.flat_map(|v| &v.fields)
			.for_each(|f| returns(api, &f.ty, objects)),
		_ => {}
	}
}

/// Whether `tr` is serde's `Serialize`, `Deserialize`, or `DeserializeOwned`, which
/// `serde_json::Value` meets as a document of any shape.
fn serde(tcx: TyCtxt<'_>, tr: DefId) -> bool {
	matches!(tcx.crate_name(tr.krate).as_str(), "serde" | "serde_core")
		&& matches!(
			tcx.item_name(tr).as_str(),
			"Serialize" | "Deserialize" | "DeserializeOwned"
		)
}

/// Renumber each object after `gone`, which is dropped, in `ty`, or with `value`, each record.
fn renumber(ty: &mut Type, gone: usize, value: bool) {
	match ty {
		Type::Object(o) if !value && *o > gone => *o -= 1,
		Type::Record(r) if value && *r > gone => *r -= 1,
		Type::Option(inner) | Type::Box(inner) | Type::Ref(_, inner) | Type::List(_, inner) => {
			renumber(inner, gone, value)
		}
		Type::Tuple(elements) => elements.iter_mut().for_each(|ty| renumber(ty, gone, value)),
		Type::Closure(closure) => (closure
			.params
			.iter_mut()
			.chain([&mut closure.ok])
			.chain(&mut closure.err))
		.for_each(|ty| renumber(ty, gone, value)),
		_ => {}
	}
}

/// Keep apart the copies of each object an export borrows mutably, as `&mut self` or a
/// `&mut` parameter, or lends to a closure as one: a write through one copy would show in
/// the others, as it never does in a clone.
fn unshare(api: &mut Api) {
	fn borrowed(ty: &Type, out: &mut Vec<usize>) {
		match ty {
			Type::Ref(Mutability::Mut, inner) => {
				if let Type::Object(i) = inner.base() {
					out.push(*i);
				}
				borrowed(inner, out);
			}
			Type::Option(inner) | Type::Box(inner) | Type::Ref(_, inner) | Type::List(_, inner) => borrowed(inner, out),
			Type::Tuple(elements) => elements.iter().for_each(|ty| borrowed(ty, out)),
			Type::Closure(closure) => {
				(closure.params.iter().chain([&closure.ok]).chain(&closure.err)).for_each(|ty| borrowed(ty, out))
			}
			_ => {}
		}
	}
	let mut mutated = Vec::new();
	let methods = (api.records.iter().flat_map(|r| &r.methods))
		.chain(api.enums.iter().flat_map(|e| &e.methods))
		.chain(api.callbacks.iter().flat_map(|c| &c.methods))
		.chain(&api.functions);
	for f in methods {
		f.params.iter().for_each(|p| borrowed(&p.ty, &mut mutated));
	}
	for (i, object) in api.objects.iter().enumerate() {
		for f in &object.methods {
			if f.receiver == Some(Receiver::Mut) {
				mutated.push(i);
			}
			f.params.iter().for_each(|p| borrowed(&p.ty, &mut mutated));
		}
	}
	for i in mutated {
		api.objects[i].shared = false;
	}
}

/// Whether `ty` is `named` or holds it.
fn names(ty: &Type, named: &Type) -> bool {
	ty == named
		|| match ty {
			Type::Option(inner) | Type::Box(inner) | Type::Ref(_, inner) | Type::List(_, inner) => names(inner, named),
			Type::Tuple(elements) => elements.iter().any(|ty| names(ty, named)),
			Type::Closure(closure) => {
				(closure.params.iter().chain([&closure.ok]).chain(&closure.err)).any(|ty| names(ty, named))
			}
			_ => false,
		}
}

/// Whether `ty` is or holds a reference.
fn borrows(ty: &Type) -> bool {
	match ty {
		Type::Ref(..) => true,
		Type::Option(inner) | Type::Box(inner) | Type::List(_, inner) => borrows(inner),
		Type::Tuple(elements) => elements.iter().any(borrows),
		_ => false,
	}
}

/// Whether `ty` holds a reference inside it, such as `Option<&T>`.
fn lends_inside(ty: &Type) -> bool {
	match ty {
		Type::Ref(_, inner) | Type::Option(inner) | Type::Box(inner) => {
			matches!(**inner, Type::Ref(..)) || lends_inside(inner)
		}
		_ => false,
	}
}

/// Whether a callback can return `ty`: a value with no memory to own.
fn foreign_return(ty: &Type) -> bool {
	matches!(
		ty,
		Type::Unit | Type::Bool | Type::Int(_) | Type::NonZero(_) | Type::F32 | Type::F64 | Type::Char
	)
}

impl Pos {
	/// The position of a type inside an `Option` or `Box` here: a field's stays a field's,
	/// and a return's is still returned.
	fn inner(self) -> Pos {
		match self {
			Pos::Field => Pos::Field,
			Pos::Return | Pos::Returned => Pos::Returned,
			Pos::Param | Pos::Behind => Pos::Behind,
		}
	}
}

fn int(ty: Ty<'_>) -> Option<Int> {
	Some(match ty.kind() {
		ty::Uint(ty::UintTy::U8) => Int::U8,
		ty::Uint(ty::UintTy::U16) => Int::U16,
		ty::Uint(ty::UintTy::U32) => Int::U32,
		ty::Uint(ty::UintTy::U64) => Int::U64,
		ty::Uint(ty::UintTy::Usize) => Int::Usize,
		ty::Int(ty::IntTy::I8) => Int::I8,
		ty::Int(ty::IntTy::I16) => Int::I16,
		ty::Int(ty::IntTy::I32) => Int::I32,
		ty::Int(ty::IntTy::I64) => Int::I64,
		ty::Int(ty::IntTy::Isize) => Int::Isize,
		_ => return None,
	})
}

/// Why a plain enum cannot cross as the C `int32_t` every plain enum crosses as, whatever
/// its `repr`: its first discriminant outside `i32`. An unsigned one past `i128::MAX` wraps
/// through [`signed`], so its bits decide.
fn wide(tcx: TyCtxt<'_>, def_id: DefId) -> Option<Why> {
	let adt = tcx.adt_def(def_id);
	if !adt.variants().iter().all(|v| v.fields.is_empty()) {
		return None;
	}
	adt.discriminants(tcx)
		.zip(adt.variants())
		.find_map(|((_, d), variant)| {
			let (value, fits) = match d.ty.is_signed() {
				true => {
					let value = signed(tcx, d.ty, d.val);
					(value.to_string(), i32::try_from(value).is_ok())
				}
				false => (d.val.to_string(), i32::try_from(d.val).is_ok()),
			};
			(!fits).then(|| Why {
				construct: "discriminant outside `i32`",
				detail: format!("{} = {value}", variant.name),
				note: Some((tcx.def_span(variant.def_id), "has this discriminant".to_string())),
			})
		})
}

/// An integer of type `ty` from its bits: sign-extended if `ty` is signed.
fn signed<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>, bits: u128) -> i128 {
	let width = match ty.kind() {
		ty::Int(int) => int.bit_width(),
		ty::Uint(uint) => uint.bit_width(),
		_ => None,
	};
	let width = width.unwrap_or(tcx.data_layout.pointer_size().bits()) as u32;
	match ty.is_signed() {
		true => {
			let shift = 128 - width;
			((bits << shift) as i128) >> shift
		}
		false => bits as i128,
	}
}

/// An item's doc comment, each line as written after `///`.
fn docs(tcx: TyCtxt<'_>, def_id: DefId) -> String {
	#[expect(deprecated, reason = "doc comments are no parsed attribute to find")]
	let attrs = tcx.get_all_attrs(def_id);
	let lines: Vec<String> = attrs
		.iter()
		.filter_map(|attr| rustc_ast::attr::AttributeExt::doc_str_and_fragment_kind(attr).map(|(doc, ..)| doc))
		.flat_map(|doc| {
			doc.as_str()
				.lines()
				.map(|line| line.strip_prefix(' ').unwrap_or(line).to_string())
				.collect::<Vec<_>>()
		})
		.collect();
	lines.join("\n").trim().to_string()
}

/// Whether `tr` is `Send`, `Sync`, or `Sized`, which a foreign implementation or a lent
/// closure meets.
/// Whether `implied`, bounds with each they imply, makes `param` outlive the call, so it
/// cannot borrow what C lends for the call, as `impl Kept` cannot where `Kept: 'static`.
fn must_outlive<'tcx>(implied: &[ty::Clause<'tcx>], param: Ty<'tcx>) -> bool {
	(implied.iter())
		.filter_map(|clause| clause.as_type_outlives_clause())
		.any(|pred| pred.skip_binder().0 == param)
}

/// Whether a bound on another type names `param`, such as `T: From<F>`.
fn mentions<'tcx>(clause: ty::Clause<'tcx>, param: Ty<'tcx>) -> bool {
	let args: Vec<ty::GenericArg<'tcx>> = match (clause.as_trait_clause(), clause.as_projection_clause()) {
		(Some(pred), _) => pred.skip_binder().trait_ref.args.as_slice().to_vec(),
		(_, Some(pred)) => (pred.skip_binder().projection_term.args.iter())
			.chain(pred.skip_binder().term.as_type().map(Into::into))
			.collect(),
		_ => Vec::new(),
	};
	args.iter().any(|arg| arg.walk().any(|arg| arg == param.into()))
}

fn marker(tcx: TyCtxt<'_>, tr: DefId) -> bool {
	[sym::Send, sym::Sync]
		.iter()
		.any(|&item| tcx.is_diagnostic_item(item, tr))
		|| [
			tcx.lang_items().sized_trait(),
			tcx.lang_items().meta_sized_trait(),
			tcx.lang_items().unpin_trait(),
		]
		.contains(&Some(tr))
}

/// How C holds the `Copy` type `ty`, with `lifetimes` lifetime parameters, as its bytes.
fn opaque<'tcx>(
	tcx: TyCtxt<'tcx>,
	sysroot: &[CrateNum],
	ty: Ty<'tcx>,
	lifetimes: usize,
	foreign: bool,
) -> crate::ir::Opaque {
	let env = TypingEnv::fully_monomorphized();
	let ty = tcx.erase_and_anonymize_regions(ty);
	// A type that has a layout on its target, as every exported `Copy` type does.
	let layout = |ty: Ty<'tcx>| tcx.layout_of(env.as_query_input(ty)).ok().map(|l| l.layout);
	let own = layout(ty).expect("an exported type has a layout");
	let eq = (tcx.get_diagnostic_item(sym::PartialEq))
		.and_then(|tr| (tcx.associated_item_def_ids(tr).iter().copied()).find(|&f| tcx.item_name(f) == sym::eq));
	// Whether every byte of `ty` belongs to an integer, `bool`, or `char` in it, and its
	// `PartialEq`, if any, is derived or std's, so equal values have equal bytes.
	fn dense<'tcx>(
		tcx: TyCtxt<'tcx>,
		sysroot: &[CrateNum],
		env: TypingEnv<'tcx>,
		eq: Option<DefId>,
		ty: Ty<'tcx>,
	) -> bool {
		match ty.kind() {
			ty::Int(_) | ty::Uint(_) | ty::Bool | ty::Char => true,
			ty::Pat(base, _) => dense(tcx, sysroot, env, eq, *base),
			ty::Array(element, _) => dense(tcx, sysroot, env, eq, *element),
			ty::Adt(adt, args) if adt.is_struct() => {
				let std = sysroot.contains(&adt.did().krate);
				let derived = || {
					let Some(eq) = eq else { return true };
					match ty::Instance::try_resolve(tcx, env, eq, tcx.mk_args(&[ty.into(), ty.into()])) {
						Ok(Some(instance)) => {
							(tcx.impl_of_assoc(instance.def_id())).is_none_or(|imp| tcx.is_automatically_derived(imp))
						}
						_ => true,
					}
				};
				if !std && !derived() {
					return false;
				}
				let Ok(own) = tcx.layout_of(env.as_query_input(ty)) else {
					return false;
				};
				let mut covered = 0;
				for field in adt.all_fields() {
					let Ok(field) = tcx.try_normalize_erasing_regions(
						env,
						ty::Unnormalized::new_wip(field.ty(tcx, *args).skip_norm_wip()),
					) else {
						return false;
					};
					let Ok(layout) = tcx.layout_of(env.as_query_input(field)) else {
						return false;
					};
					if !dense(tcx, sysroot, env, eq, field) {
						return false;
					}
					covered += layout.size.bytes();
				}
				covered == own.size.bytes()
			}
			_ => false,
		}
	}
	crate::ir::Opaque {
		size: own.size().bytes(),
		align: own.align().bytes(),
		dense: dense(tcx, sysroot, env, eq, ty),
		lifetimes,
		foreign,
	}
}

/// The target the export is collected for.
fn target(tcx: TyCtxt<'_>) -> String {
	tcx.sess.opts.target_triple.tuple().to_string()
}

/// The padding of `ty` on this target, as `(offset, length)` ranges, which the glue zeroes
/// so no byte Rust leaves uninitialized reaches a language; or none when which bytes it
/// initializes depends on its value: a union, such as `MaybeUninit`, or an enum whose
/// variants initialize different bytes, such as `Option<u32>`, whose `None` leaves its
/// payload uninitialized.
fn padding<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Vec<(u64, u64)>> {
	use rustc_abi::{FieldsShape, Variants};
	use rustc_middle::ty::layout::{LayoutCx, TyAndLayout};
	let env = TypingEnv::fully_monomorphized();
	let cx = LayoutCx::new(tcx, env);
	let ty = tcx.erase_and_anonymize_regions(ty);
	let layout = tcx.layout_of(env.as_query_input(ty)).ok()?;
	// The spans `(offset, length)` of each byte `layout` initializes, sorted and merged, so
	// what they hold grows with the type's shape, never with its size.
	fn init<'tcx>(cx: &LayoutCx<'tcx>, layout: TyAndLayout<'tcx>) -> Option<Vec<(u64, u64)>> {
		match layout.ty.kind() {
			ty::Adt(adt, _) if adt.is_union() => return None,
			ty::Int(_)
			| ty::Uint(_)
			| ty::Float(_)
			| ty::Bool
			| ty::Char
			| ty::RawPtr(..)
			| ty::Ref(..)
			| ty::FnPtr(..) => return Some(merged(vec![(0, layout.size.bytes())])),
			_ => {}
		}
		match &layout.variants {
			Variants::Empty => Some(Vec::new()),
			Variants::Single { .. } => fields(cx, layout),
			Variants::Multiple {
				tag,
				tag_field,
				variants,
				..
			} => {
				let tag = (layout.fields.offset(tag_field.as_usize()).bytes(), tag.size(cx).bytes());
				let mut each: Option<Vec<(u64, u64)>> = None;
				for variant in variants.indices() {
					let mut mine = fields(cx, layout.for_variant(cx, variant))?;
					mine.push(tag);
					let mine = merged(mine);
					match &each {
						Some(first) if *first != mine => return None,
						Some(_) => {}
						None => each = Some(mine),
					}
				}
				Some(each.unwrap_or_default())
			}
		}
	}
	fn fields<'tcx>(cx: &LayoutCx<'tcx>, layout: TyAndLayout<'tcx>) -> Option<Vec<(u64, u64)>> {
		match &layout.fields {
			FieldsShape::Primitive => Some(merged(vec![(0, layout.size.bytes())])),
			FieldsShape::Union(_) => None,
			FieldsShape::Array { stride, count } => {
				// Every element initializes the same bytes, so one is read and stamped at
				// each stride: walking each would never end for `[(); usize::MAX]`.
				let element = layout.field(cx, 0);
				let (size, stride) = (element.size.bytes(), stride.bytes());
				if *count == 0 || size == 0 {
					return Some(Vec::new());
				}
				let mine = init(cx, element)?;
				// A whole element tiling the array, as each of `[u8; 1 << 40]`'s does, is one span.
				if mine == [(0, size)] && stride == size {
					return Some(vec![(0, size * count)]);
				}
				Some(merged(
					(0..*count)
						.flat_map(|n| mine.iter().map(move |&(at, len)| (n * stride + at, len)))
						.collect(),
				))
			}
			FieldsShape::Arbitrary { .. } => {
				let mut spans = Vec::new();
				for n in 0..layout.fields.count() {
					let at = layout.fields.offset(n).bytes();
					spans.extend(
						init(cx, layout.field(cx, n))?
							.into_iter()
							.map(|(start, len)| (at + start, len)),
					);
				}
				Some(merged(spans))
			}
		}
	}
	// `spans` sorted, without empty ones, and each overlapping or adjacent pair joined.
	fn merged(mut spans: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
		spans.retain(|&(_, len)| len > 0);
		spans.sort_unstable();
		let mut out: Vec<(u64, u64)> = Vec::with_capacity(spans.len());
		for (at, len) in spans {
			match out.last_mut() {
				Some((start, n)) if at <= *start + *n => *n = (*n).max(at + len - *start),
				_ => out.push((at, len)),
			}
		}
		out
	}
	// The padding is what the initialized spans leave between them and after the last.
	let mut ranges = Vec::new();
	let mut end = 0;
	for (at, len) in init(&cx, layout)? {
		if at > end {
			ranges.push((end, at - end));
		}
		end = at + len;
	}
	if layout.size.bytes() > end {
		ranges.push((end, layout.size.bytes() - end));
	}
	Some(ranges)
}

/// Whether `ty`, the type `def_id` at some arguments, implements the std trait `item`.
fn implements<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>, def_id: DefId, item: rustc_span::Symbol) -> bool {
	let Some(tr) = tcx.get_diagnostic_item(item) else {
		return false;
	};
	let (infcx, env) = tcx
		.infer_ctxt()
		.build_with_typing_env(TypingEnv::post_analysis(tcx, def_id));
	infcx.type_implements_trait(tr, [ty], env).must_apply_modulo_regions()
}

/// Whether `ty` is an instance of an impl's self type `pattern`, binding each of the
/// impl's type parameters in `args`, which starts as its identity. An instance's
/// arguments are never references or tuples but `()`, so structure decides it; lifetimes
/// stay as the impl names them.
fn matches<'tcx>(tcx: TyCtxt<'tcx>, pattern: Ty<'tcx>, ty: Ty<'tcx>, args: &mut [ty::GenericArg<'tcx>]) -> bool {
	match (pattern.kind(), ty.kind()) {
		(ty::Param(param), _) => {
			let slot = &mut args[param.index as usize];
			match slot.as_type() == Some(pattern) {
				true => {
					*slot = ty.into();
					true
				}
				false => {
					slot.as_type().map(|bound| tcx.erase_and_anonymize_regions(bound))
						== Some(tcx.erase_and_anonymize_regions(ty))
				}
			}
		}
		(ty::Adt(a, pattern_args), ty::Adt(b, ty_args)) if a.did() == b.did() => {
			(pattern_args.iter().zip(ty_args.iter())).all(|(p, t)| match (p.kind(), t.kind()) {
				(GenericArgKind::Type(p), GenericArgKind::Type(t)) => matches(tcx, p, t, args),
				(GenericArgKind::Const(p), GenericArgKind::Const(t)) => match p.kind() {
					ty::ConstKind::Param(param) => {
						let slot = &mut args[param.index as usize];
						match slot.as_const() == Some(p) {
							true => {
								*slot = t.into();
								true
							}
							false => slot.as_const() == Some(t),
						}
					}
					_ => p == t,
				},
				_ => true,
			})
		}
		_ => tcx.erase_and_anonymize_regions(pattern) == tcx.erase_and_anonymize_regions(ty),
	}
}

/// A type's own generic arguments as it is exported: its lifetimes `'static`.
fn statics<'tcx>(tcx: TyCtxt<'tcx>, def_id: DefId) -> ty::GenericArgs<'tcx> {
	ty::GenericArgs::for_item(tcx, def_id, |param, _| match param.kind {
		ty::GenericParamDefKind::Lifetime => tcx.lifetimes.re_static.into(),
		_ => tcx.mk_param_from_def(param),
	})
}

/// A type's path without generic arguments.
fn strip_args(tcx: TyCtxt<'_>, did: DefId) -> String {
	tcx.def_path_str(did)
}
