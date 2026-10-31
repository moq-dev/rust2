//! Copies of `Copy` aggregates, by their type.
//!
//! JS shares an object where Rust copies its value, so a `Copy` value that is a JS object
//! the crate's code writes into in place is copied where Rust copies it. A function made
//! for its type copies what it holds by value and shares what it holds by reference, as
//! Rust does. A value of a type nothing writes into never changes once made, so sharing
//! it is a copy no one can tell apart.

use std::cell::RefCell;

use rustc_data_structures::fx::{FxHashMap, FxHashSet, FxIndexSet};
use rustc_hir::def::DefKind;
use rustc_hir::def_id::DefId;
use rustc_index::IndexVec;
use rustc_middle::mir::visit::{MutatingUseContext, PlaceContext, Visitor};
use rustc_middle::mir::{self, Local, Location, Place, ProjectionElem, Rvalue};
use rustc_middle::ty::fast_reject::DeepRejectCtxt;
use rustc_middle::ty::{self, EarlyBinder, Ty, TyCtxt, TypeVisitableExt};
use rustc_mir_dataflow::ResultsCursor;
use rustc_mir_dataflow::impls::MaybeLiveLocals;
use rustc_span::{DUMMY_SP, Span};

use super::library::{Flow, Std};
use super::mir::holders::{Filled, Trace};
use super::{AdtRepr, Translator, field_name, names};
use crate::program::{SymKind, Symbol};
use crate::ts::{self, Expr, Key, Param, Stmt, StmtKind};

impl<'a, 'tcx> Translator<'a, 'tcx> {
	/// Whether a `ty` is copied where Rust copies it: a `Copy` value that is, or holds by
	/// value, a JS object the crate's code writes into in place.
	pub fn needs_copy(&self, ty: Ty<'tcx>) -> bool {
		self.is_copy(ty) && self.changes(ty)
	}

	/// Whether a value of `ty` is, or holds by value, a JS object the crate writes into.
	fn changes(&self, ty: Ty<'tcx>) -> bool {
		let (object, parts): (bool, Vec<Ty<'tcx>>) = match ty.kind() {
			ty::Tuple(tys) if !tys.is_empty() => (true, tys.to_vec()),
			ty::Array(elem, _) => (true, vec![*elem]),
			ty::Adt(adt, args) => match Std::of(self.tcx, adt.did()) {
				// Their payload, or no object at all; a write into an `rt.Result`'s payload
				// replaces the result.
				Some(
					Std::Option
					| Std::Poll
					| Std::Result
					| Std::ControlFlow
					| Std::Reverse
					| Std::Wrapping
					| Std::Saturating,
				) => (false, args.types().collect()),
				Some(Std::Bound) => (true, args.types().collect()),
				Some(Std::MaybeUninit) => return self.changes(args.type_at(0)),
				Some(_) => return false,
				None if self.own(adt.did()) && adt.all_fields().next().is_some() => {
					(true, adt.all_fields().map(|f| self.field_ty(f.did, args)).collect())
				}
				None => return false,
			},
			_ => return false,
		};
		object
			&& self
				.writes
				.contains(&self.writes.into, false, self.tcx, ty, self.polymorphic())
			|| parts.into_iter().any(|t| self.changes(t))
	}

	/// Whether the crate may give a part of a `ty` kept as a JS object another object: by
	/// a write of the part, or of a whole value that is or holds a `ty`.
	pub fn replaced(&self, ty: Ty<'tcx>) -> bool {
		let (writes, polymorphic) = (&self.writes, self.polymorphic());
		writes.contains(&writes.replaced, true, self.tcx, ty, polymorphic)
			|| writes.contains(&writes.holders, false, self.tcx, ty, polymorphic)
	}

	/// Whether a `ty` is, or holds by value at any depth, a part that `object` says is a JS
	/// object, into which a raw pointer may be made through a reference that the crate's
	/// code can't follow back to where it was borrowed, by any body or by `body` alone.
	pub fn taken(&self, body: DefId, ty: Ty<'tcx>, object: &dyn Fn(Ty<'tcx>) -> bool) -> bool {
		let (writes, polymorphic) = (&self.writes, self.polymorphic());
		let local = writes.taken_in.get(&body);
		by_value(self.tcx, self.typing_env(), ty).any(|part| {
			object(part)
				&& (writes.contains(&writes.taken, false, self.tcx, part, polymorphic)
					|| local.is_some_and(|local| writes.contains(local, false, self.tcx, part, polymorphic)))
		})
	}

	/// `value`, a `ty` read out of where it is kept, as a value of its own, as Rust's read
	/// copies its bytes: the JS objects it holds by value copied where the crate's code
	/// writes into them in place.
	pub fn snapshot(&mut self, value: Expr, ty: Ty<'tcx>, span: Span) -> Expr {
		match self.changes(ty).then(|| self.shim(ty, span)).flatten() {
			Some(copy) => copy.call(vec![value]),
			None => value,
		}
	}

	/// `value`, copied if a `ty` is.
	pub fn copied(&mut self, value: Expr, ty: Ty<'tcx>, span: Span) -> Expr {
		match self.copy_fn(ty, span) {
			Some(copy) => copy.call(vec![value]),
			None => value,
		}
	}

	/// The function copying a `ty`, made the first time, or `None` if a `ty` is shared.
	pub fn copy_fn(&mut self, ty: Ty<'tcx>, span: Span) -> Option<Expr> {
		if !self.needs_copy(ty) {
			return None;
		}
		self.shim(ty, span)
	}

	/// The function copying a `ty`, made the first time.
	fn shim(&mut self, ty: Ty<'tcx>, span: Span) -> Option<Expr> {
		// An array's copy is the same at every length, so its name has none: `array_u8`.
		let tcx = self.tcx;
		let (ty, any_length) = match ty.kind() {
			ty::Array(elem, _) => (Ty::new_array(tcx, *elem, 0), true),
			ty::Adt(adt, args) if Std::of(tcx, adt.did()) == Some(Std::MaybeUninit) => match args.type_at(0).kind() {
				ty::Array(elem, _) => (Ty::new_maybe_uninit(tcx, Ty::new_array(tcx, *elem, 0)), true),
				_ => (ty, false),
			},
			_ => (ty, false),
		};
		if let Some(&sym) = self.copy_shims.get(&ty) {
			return Some(Expr::Sym(sym));
		}
		// Only an instance can say what a parameter holds; a census writes nothing.
		if self.polymorphic() && ty.has_param() {
			return None;
		}
		let sym = self.program.sym(Symbol {
			module: Vec::new(),
			base: "copy".into(),
			suffix: match any_length {
				true => names::ty(tcx, ty).replacen("array0_", "array_", 1),
				false => names::ty(tcx, ty),
			},
			owner: None,
			kind: SymKind::Function,
			order: 0,
			span,
			instance: ty::print::with_no_trimmed_paths!(ty.to_string()),
			name: String::new(),
			alias: None,
		});
		self.copy_shims.insert(ty, sym);
		let body = self.copy_body(ty, span);
		let ts_ty = self.ty(ty, span);
		self.program.items.push(ts::Item::Function(ts::Function {
			sym,
			params: vec![Param {
				name: "v".into(),
				ty: Some(ts_ty.clone()),
			}],
			ret: ts_ty,
			body,
			loc: None,
			doc: None,
		}));
		Some(Expr::Sym(sym))
	}

	/// The body of the function copying `v`, a `ty`.
	fn copy_body(&mut self, ty: Ty<'tcx>, span: Span) -> Vec<Stmt> {
		self.copy_parts(ty, false, span)
	}

	/// The body of the function copying `v`, a `ty`, or a `MaybeUninit` of one if `uninit`:
	/// then any of its parts kept as objects may hold nothing yet.
	fn copy_parts(&mut self, ty: Ty<'tcx>, uninit: bool, span: Span) -> Vec<Stmt> {
		let v = || Expr::ident("v");
		let ret = |e: Expr| vec![StmtKind::Return(Some(e.typed())).at(None)];
		let tcx = self.tcx;
		let part = |t: Ty<'tcx>| match uninit {
			true => Ty::new_maybe_uninit(tcx, t),
			false => t,
		};
		match ty.kind() {
			ty::Tuple(tys) => {
				let parts = tys
					.iter()
					.enumerate()
					.map(|(i, t)| self.snapshot(v().index(Expr::lit(i.to_string())), part(t), span))
					.collect();
				ret(Expr::Array(parts))
			}
			ty::Array(elem, _) => ret(
				match self.changes(*elem).then(|| self.shim(part(*elem), span)).flatten() {
					Some(copy) => v().member("map").call(vec![copy]),
					None => v().member("slice").call(Vec::new()),
				},
			),
			ty::Adt(adt, args) => match Std::of(tcx, adt.did()) {
				Some(Std::MaybeUninit) => {
					let none = StmtKind::If {
						cond: v().binary("===", Expr::undefined()),
						then: ret(v()),
						els: Vec::new(),
					};
					let mut body = vec![none.at(None)];
					body.extend(self.copy_parts(args.type_at(0), true, span));
					body
				}
				Some(Std::Option) => ret(self.copy_unless(self.none(args.type_at(0)), args.type_at(0), span)),
				Some(Std::Poll) => ret(self.copy_unless(self.pending(args.type_at(0)), args.type_at(0), span)),
				Some(Std::Result) => ret(self.copy_result(args.type_at(0), args.type_at(1), span)),
				Some(Std::Reverse | Std::Wrapping | Std::Saturating) => ret(self.snapshot(v(), args.type_at(0), span)),
				Some(Std::Bound) => {
					let ts_ty = self.ty(ty, span);
					let held = self.snapshot(v().member("_0"), args.type_at(0), span);
					let copy = Expr::Object(vec![
						(Key::Name("tag".into()), v().member("tag")),
						(Key::Name("_0".into()), held),
					]);
					ret(Expr::Cond(
						Box::new(v().member("tag").binary("===", Expr::Str("Unbounded".into()))),
						Box::new(v()),
						Box::new(Expr::As(Box::new(copy), ts_ty)),
					))
				}
				Some(Std::ControlFlow) => {
					let (residual, value) = (args.type_at(0), args.type_at(1));
					ret(match (Flow::of(tcx, args), residual.kind()) {
						(Flow::Result, ty::Adt(_, residual)) => self.copy_result(value, residual.type_at(1), span),
						(Flow::Option, _) => self.copy_unless(self.none(value), value, span),
						_ => self.copy_result(value, residual, span),
					})
				}
				_ => {
					let info = self.adt(*adt, span);
					match info.repr {
						AdtRepr::Struct { ctor, fields } => {
							let variant = adt.non_enum_variant();
							let values = (variant.fields.iter().zip(&fields))
								.map(|(field, &id)| {
									let field_ty = self.field_ty(field.did, args);
									self.snapshot(v().field(id), part(field_ty), span)
								})
								.collect();
							ret(self.construct(info.sym, &ctor, &fields, values))
						}
						AdtRepr::Union => {
							let ts_ty = self.ty(ty, span);
							let mut body = Vec::new();
							for variant in adt.variants().iter().filter(|v| !v.fields.is_empty()) {
								let tag = variant.name.to_string();
								let mut props = vec![(Key::Name("tag".into()), Expr::Str(tag.clone()))];
								for (i, field) in variant.fields.iter().enumerate() {
									let name = field_name(field.name.as_str(), i);
									let field_ty = self.field_ty(field.did, args);
									props.push((
										Key::Name(name.clone()),
										self.snapshot(v().member(name), field_ty, span),
									));
								}
								body.push(
									StmtKind::If {
										cond: v().member("tag").binary("===", Expr::Str(tag)),
										then: ret(Expr::As(Box::new(Expr::Object(props)), ts_ty.clone())),
										els: Vec::new(),
									}
									.at(None),
								);
							}
							// A variant without fields has nothing to change in place.
							body.extend(ret(v()));
							body
						}
						// A slot no field was written into yet holds nothing.
						AdtRepr::Slots(_) => {
							let (_, slots) = self.overlay(*adt, span).expect("a union is laid out where it is made");
							let ts_ty = self.ty(ty, span);
							let slots = slots
								.into_iter()
								.map(|(name, slot_ty)| {
									let slot_ty = self.instantiate(slot_ty, args);
									let value = self.snapshot(
										v().member(name.clone()),
										Ty::new_maybe_uninit(tcx, slot_ty),
										span,
									);
									(name, value)
								})
								.collect();
							ret(super::unions::made(slots, ts_ty))
						}
						AdtRepr::Tags => unreachable!("an enum without fields is shared"),
					}
				}
			},
			_ => unreachable!("`{ty}` is shared"),
		}
	}

	/// A copy of `v`, an `rt.Result` of `ok` or `err`.
	fn copy_result(&mut self, ok: Ty<'tcx>, err: Ty<'tcx>, span: Span) -> Expr {
		let v = || Expr::ident("v");
		Expr::Cond(
			Box::new(v().member("ok")),
			Box::new(Expr::rt("Ok", vec![self.snapshot(v().member("value"), ok, span)])),
			Box::new(Expr::rt("Err", vec![self.snapshot(v().member("error"), err, span)])),
		)
	}

	/// A copy of `v`, which is `none` or a `some`.
	fn copy_unless(&mut self, none: Expr, some: Ty<'tcx>, span: Span) -> Expr {
		let v = || Expr::ident("v");
		Expr::Cond(
			Box::new(v().binary("===", none)),
			Box::new(v()),
			Box::new(self.snapshot(v(), some, span)),
		)
	}

	/// The type of field `did` of an ADT with `args`.
	pub fn field_ty(&self, did: rustc_hir::def_id::DefId, args: ty::GenericArgsRef<'tcx>) -> Ty<'tcx> {
		self.instantiate(self.tcx.type_of(did).instantiate_identity().skip_norm_wip(), args)
	}

	/// `ty`, over an item's own type parameters, with `args` for them.
	pub fn instantiate(&self, ty: Ty<'tcx>, args: ty::GenericArgsRef<'tcx>) -> Ty<'tcx> {
		let ty = EarlyBinder::bind(self.tcx, ty);
		self.tcx
			.try_instantiate_and_normalize_erasing_regions(args, self.typing_env_pub(), ty)
			.unwrap_or_else(|_| ty.instantiate(self.tcx, args).skip_norm_wip())
	}
}

/// The types the translated crates' code writes into in place: through a projection, as `p.x = 1`
/// writes into `p`; through a reference or a box, as `*r = v` does into what `r` points
/// at; or by anything code it doesn't read can do with a mutable reference or pointer to
/// one, which counts as a write: one a parameter of a std function the crate names takes,
/// or one kept anywhere but in a local. A kept one writes nothing whole, since what reads
/// it back is the crate's code, whose writes count where they are, or std's, given it
/// where a call shows. Bodies are read as written, over their own type parameters, so a
/// write into a type parameter stands for the type each instance of the body that
/// translation reaches gives it, or in a census lowering a function over its own type
/// parameters, for every type its bounds let it be. A write that fills memory nothing
/// points into yet gives no pointer's object another.
#[derive(Default)]
pub struct Writes<'tcx> {
	into: Types<'tcx>,
	/// The types the code may write whole, each standing for every type it holds by value
	/// too: a write of a whole value, as `*r = v`, a call of a std function given a
	/// mutable reference to it, or one that replaces a `RefCell`'s value through a shared
	/// reference, gives each part of `*r` another object, and each part those hold, at
	/// any depth.
	replaced: Types<'tcx>,
	/// The types a part of which, kept as a JS object, a write of that part alone gives
	/// another object, as `p.x = Inner {..}` does to `p`'s.
	holders: Types<'tcx>,
	/// The types a raw pointer is made into through a reference to one, where the pointer
	/// may outlive the call that made it: whoever borrowed the place it points at can't
	/// give that place another object while the pointer may be read.
	taken: Types<'tcx>,
	/// As `taken`, by the body making each, of pointers that die with the call but through
	/// a reference that may point at that body's own locals, where it can't tell which.
	taken_in: FxHashMap<DefId, Types<'tcx>>,
	/// The bodies read.
	read: FxHashSet<DefId>,
	/// The arguments of each body's instances that translation reached, without type
	/// parameters.
	reached: FxHashMap<DefId, Vec<ty::GenericArgsRef<'tcx>>>,
}

/// Types, each without type parameters or over those of the body naming it.
#[derive(Default)]
struct Types<'tcx> {
	/// Each type without type parameters, an array as a slice of its element, since the
	/// two share storage in JS.
	concrete: FxHashSet<Ty<'tcx>>,
	/// Each type over the type parameters of the body writing it, and that body.
	generic: FxIndexSet<(DefId, Ty<'tcx>)>,
	/// What [`Writes::contains`] found of each type it was asked about, and whether it was
	/// asked over type parameters.
	found: RefCell<FxHashMap<(Ty<'tcx>, bool), bool>>,
}

impl<'tcx> Writes<'tcx> {
	/// The writes of every function, closure, and coroutine body of `bodies` and of
	/// `reached`, the instances translation reached, that `own` says translates, and of the
	/// closures and coroutines they hold: a binding stands in for any other, which counts
	/// as code it doesn't read.
	pub fn of_bodies(
		t: &Translator<'_, 'tcx>,
		bodies: impl IntoIterator<Item = DefId>,
		reached: &FxHashSet<ty::Instance<'tcx>>,
		own: &dyn Fn(DefId) -> bool,
	) -> Self {
		let tcx = t.tcx;
		let memo = RefCell::default();
		let mut writes = Writes::default();
		for instance in reached.iter().filter(|instance| !instance.args.has_param()) {
			writes.reached.entry(instance.def_id()).or_default().push(instance.args);
		}
		let mut queue: Vec<DefId> = bodies.into_iter().chain(reached.iter().map(|i| i.def_id())).collect();
		while let Some(def_id) = queue.pop() {
			if !writes.read.insert(def_id) || !own(def_id) {
				continue;
			}
			let body = match tcx.def_kind(def_id) {
				// A thread-local's `const`, which lowers from this MIR as a function.
				DefKind::Const if super::library::local_init(tcx, def_id).is_some() => tcx.mir_for_ctfe(def_id),
				kind if kind.is_fn_like() && tcx.is_mir_available(def_id) => tcx.optimized_mir(def_id),
				// One without MIR, such as an import, is refused where it is reached.
				_ => continue,
			};
			// Another crate lists none of its closures' bodies.
			for local in body.local_decls.iter() {
				queue.extend(local.ty.walk().filter_map(|arg| match *arg.as_type()?.kind() {
					ty::Closure(nested, _) | ty::Coroutine(nested, _) | ty::CoroutineClosure(nested, _) => Some(nested),
					_ => None,
				}));
			}
			BodyWrites {
				tcx,
				body,
				def_id,
				writes: &mut writes,
				own,
				borrows: borrows(body),
				trace: Trace::new(t, body, ty::TypingEnv::post_analysis(tcx, def_id), &memo),
				live: None,
			}
			.visit_body(body);
		}
		writes
	}

	/// Whether translation must run again to judge the writes `instance` reaches: its body
	/// is unread, or writes a type over its type parameters and these arguments are new.
	pub fn unread(&self, instance: ty::Instance<'tcx>) -> bool {
		let def_id = instance.def_id();
		if !self.read.contains(&def_id) {
			return true;
		}
		let writes = |types: &Types<'tcx>| types.generic.iter().any(|&(body, _)| body == def_id);
		!instance.args.has_param()
			&& (writes(&self.into)
				|| writes(&self.replaced)
				|| writes(&self.holders)
				|| writes(&self.taken)
				|| self.taken_in.get(&def_id).is_some_and(writes))
			&& !self
				.reached
				.get(&def_id)
				.is_some_and(|args| args.contains(&instance.args))
	}

	/// Whether `ty` is one of `types`, or with `held`, a type one of them holds by value.
	/// Over type parameters, as a census lowers some functions, it may be, and a write into
	/// a type parameter counts for every type its bounds allow; otherwise for the type each
	/// reached instance gives it.
	fn contains(&self, types: &Types<'tcx>, held: bool, tcx: TyCtxt<'tcx>, ty: Ty<'tcx>, polymorphic: bool) -> bool {
		let ty = tcx.erase_and_anonymize_regions(ty);
		let stored = storage(tcx, ty);
		if ty.has_param() || types.concrete.contains(&stored) {
			return true;
		}
		if let Some(&found) = types.found.borrow().get(&(ty, polymorphic)) {
			return found;
		}
		let found = types.generic.iter().any(|&(def_id, written)| match polymorphic {
			// A parameter can be the array itself, as in `*dst = value` with `dst: &mut T`, and
			// its slice pattern can be any array of that element.
			true => {
				instantiates(tcx, def_id, written, ty) || stored != ty && instantiates(tcx, def_id, written, stored)
			}
			false => self.reached.get(&def_id).is_some_and(|reached| {
				reached.iter().any(|&args| {
					let written = EarlyBinder::bind(tcx, written);
					// One that doesn't normalize counts.
					let env = ty::TypingEnv::fully_monomorphized();
					tcx.try_instantiate_and_normalize_erasing_regions(args, env, written)
						.ok()
						.is_none_or(|written| match held {
							true => by_value(tcx, env, written).any(|part| storage(tcx, part) == stored),
							false => storage(tcx, written) == stored,
						})
				})
			}),
		});
		types.found.borrow_mut().insert((ty, polymorphic), found);
		found
	}
}

impl<'tcx> Types<'tcx> {
	fn insert(&mut self, tcx: TyCtxt<'tcx>, def_id: DefId, ty: Ty<'tcx>) {
		let ty = storage(tcx, ty);
		match ty.has_param() {
			true => _ = self.generic.insert((def_id, ty)),
			false => _ = self.concrete.insert(ty),
		}
	}
}

/// `ty` as [`Types`] keys it: without regions, a `MaybeUninit` as what it holds, and an
/// array as a slice of its element.
fn storage<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Ty<'tcx> {
	let mut ty = tcx.erase_and_anonymize_regions(ty);
	while let ty::Adt(adt, args) = ty.kind()
		&& Std::of(tcx, adt.did()) == Some(Std::MaybeUninit)
	{
		ty = args.type_at(0);
	}
	match ty.kind() {
		ty::Array(elem, _) => Ty::new_slice(tcx, *elem),
		_ => ty,
	}
}

/// `ty` and each type it holds by value, at any depth: in a field, an element, or a
/// payload. An array of no elements holds none, and a type parameter, or a type `env`
/// can't normalize, stands for what it holds.
fn by_value<'tcx>(tcx: TyCtxt<'tcx>, env: ty::TypingEnv<'tcx>, ty: Ty<'tcx>) -> impl Iterator<Item = Ty<'tcx>> {
	let mut held = FxIndexSet::default();
	let mut stack = vec![ty];
	while let Some(ty) = stack.pop() {
		if !held.insert(ty) {
			continue;
		}
		match *ty.kind() {
			ty::Array(_, len) if len.try_to_target_usize(tcx) == Some(0) => {}
			ty::Array(elem, _) | ty::Pat(elem, _) => stack.push(elem),
			ty::Tuple(tys) => stack.extend(tys),
			ty::Adt(adt, args) => stack.extend(adt.all_fields().map(|field| {
				let ty = field.ty(tcx, args).skip_norm_wip();
				tcx.try_normalize_erasing_regions(env, ty::Unnormalized::new_wip(ty))
					.unwrap_or(ty)
			})),
			_ => {}
		}
	}
	held.into_iter()
}

/// Whether `written`, a type over the type parameters of `def_id`, is `ty` for some
/// arguments that satisfy its bounds. A projection on them, which only those arguments
/// decide, is any type that satisfies its associated type's bounds.
fn instantiates<'tcx>(tcx: TyCtxt<'tcx>, def_id: DefId, written: Ty<'tcx>, ty: Ty<'tcx>) -> bool {
	use rustc_infer::infer::TyCtxtInferExt;
	use rustc_infer::traits::{Obligation, ObligationCause};
	use rustc_middle::ty::{BottomUpFolder, TypeFoldable, Upcast};
	use rustc_trait_selection::traits::ObligationCtxt;
	if !DeepRejectCtxt::relate_rigid_infer(tcx).types_may_unify(ty, written) {
		return false;
	}
	let (infcx, env) = tcx
		.infer_ctxt()
		.build_with_typing_env(ty::TypingEnv::fully_monomorphized());
	let ocx = ObligationCtxt::new(&infcx);
	let cause = ObligationCause::dummy();
	let register = |clause: ty::Clause<'tcx>| ocx.register_obligation(Obligation::new(tcx, cause.clone(), env, clause));
	let args = infcx.fresh_args_for_item(DUMMY_SP, def_id);
	let written = EarlyBinder::bind(tcx, written)
		.instantiate(tcx, args)
		.skip_normalization();
	let mut projections = Vec::new();
	let written = written.fold_with(&mut BottomUpFolder {
		tcx,
		ty_op: |t: Ty<'tcx>| match *t.kind() {
			ty::Alias(
				_,
				alias @ ty::AliasTy {
					kind: ty::Projection { .. },
					..
				},
			) => {
				let var = infcx.next_ty_var(DUMMY_SP);
				projections.push((alias, var));
				var
			}
			_ => t,
		},
		lt_op: |r| r,
		ct_op: |c| c,
	});
	for (alias, var) in projections {
		let ty::Projection { def_id: item } = alias.kind else {
			unreachable!("only projections are replaced")
		};
		for clause in tcx.item_bounds(item).iter_instantiated(tcx, alias.args) {
			let clause = clause.skip_normalization().fold_with(&mut BottomUpFolder {
				tcx,
				ty_op: |t: Ty<'tcx>| match *t.kind() {
					ty::Alias(_, a) if a == alias => var,
					_ => t,
				},
				lt_op: |r| r,
				ct_op: |c| c,
			});
			register(ocx.normalize(&cause, env, ty::Unnormalized::new(clause)));
		}
		let projection = ty::ProjectionClause {
			projection_term: alias.into(),
			term: var.into(),
		};
		register(projection.upcast(tcx));
	}
	let written = ocx.normalize(&cause, env, ty::Unnormalized::new(written));
	if ocx.eq(&cause, env, written, ty).is_err() {
		return false;
	}
	for (clause, _) in tcx.clauses_of(def_id).instantiate(tcx, args) {
		register(ocx.normalize(&cause, env, clause));
	}
	!ocx.try_evaluate_obligations().has_errors()
}

/// The value std's `def_id`, given `this` as its `self`, writes whole through that shared
/// reference, putting another object in its place: the value of the `RefCell` it is, or
/// that a thread-local's `RefCell` holds. A `Cell`'s are left out, as none of its methods
/// that translate lends a reference into its value.
fn replaces<'tcx>(tcx: TyCtxt<'tcx>, def_id: DefId, this: Ty<'tcx>) -> Option<Ty<'tcx>> {
	let (owner, name) = super::library::key(tcx, def_id)?;
	let replaces = matches!(
		(owner.as_str(), name.as_str()),
		("RefCell", "replace" | "take") | ("LocalKey", "set" | "replace" | "take")
	);
	if !replaces || !super::library::is_std(tcx, def_id) {
		return None;
	}
	let mut ty = this.builtin_deref(true)?;
	loop {
		let ty::Adt(adt, args) = ty.kind() else {
			return None;
		};
		match Std::of(tcx, adt.did()) {
			Some(Std::LocalKey) => ty = args.type_at(0),
			Some(Std::RefCell) => return Some(args.type_at(0)),
			_ => return None,
		}
	}
}

/// The place each local of `body` borrows, where its one assignment is a borrow and
/// nothing borrows the local itself, through which it could be pointed elsewhere.
fn borrows<'tcx>(body: &mir::Body<'tcx>) -> IndexVec<mir::Local, Option<Place<'tcx>>> {
	let mut defs = IndexVec::from_elem(0u32, &body.local_decls);
	let mut borrows = IndexVec::from_elem(None, &body.local_decls);
	let mut lent = FxHashSet::default();
	for data in body.basic_blocks.iter() {
		for stmt in &data.statements {
			let mir::StatementKind::Assign(assign) = &stmt.kind else {
				continue;
			};
			if let Rvalue::Ref(_, _, place) | Rvalue::RawPtr(_, place) = assign.1
				&& !place.is_indirect()
			{
				lent.insert(place.local);
			}
			if let Some(local) = assign.0.as_local() {
				defs[local] += 1;
				if let Rvalue::Ref(_, _, place) = assign.1 {
					borrows[local] = Some(place);
				}
			}
		}
		if let mir::TerminatorKind::Call { destination, .. } = &data.terminator().kind
			&& let Some(local) = destination.as_local()
		{
			defs[local] += 1;
		}
	}
	for (local, borrow) in borrows.iter_enumerated_mut() {
		if defs[local] != 1 || lent.contains(&local) {
			*borrow = None;
		}
	}
	borrows
}

/// `place`, read through each reference that `borrow` says borrows a place, as that place:
/// `(*r).0` of `r = &mut pair` is `pair.0`.
pub fn through_borrows<'tcx>(
	tcx: TyCtxt<'tcx>,
	mut place: Place<'tcx>,
	borrow: impl Fn(mir::Local) -> Option<Place<'tcx>>,
) -> Place<'tcx> {
	while let [ProjectionElem::Deref, rest @ ..] = place.projection.as_slice()
		&& let Some(borrowed) = borrow(place.local)
	{
		place = borrowed.project_deeper(rest, tcx);
	}
	place
}

/// The places a pointer made of `place` at `at` may point into: `place` read through each
/// reference that `borrow` says borrows a place, as [`through_borrows`] reads it, and
/// through one that [`retargeted`] follows, as each place it may borrow there.
pub fn borrowed_places<'tcx>(
	tcx: TyCtxt<'tcx>,
	body: &mir::Body<'tcx>,
	place: Place<'tcx>,
	at: Location,
	borrow: &dyn Fn(Local) -> Option<Place<'tcx>>,
) -> Vec<Place<'tcx>> {
	let place = through_borrows(tcx, place, borrow);
	let Some((ProjectionElem::Deref, rest)) = place.projection.split_first() else {
		return vec![place];
	};
	match retargeted(tcx, body, place.local, at) {
		Some(borrowed) => borrowed
			.into_iter()
			.map(|b| through_borrows(tcx, b.project_deeper(rest, tcx), borrow))
			.collect(),
		None => vec![place],
	}
}

/// The places the reference `local` may borrow at `at`, where nothing borrows it but to
/// read it, or to point it elsewhere by `mem::replace` or `mem::swap`, as
/// `mem::replace(&mut r, &mut b)` points `r` at `b`: those of each borrow that reaches
/// `at`, assigned to it or given it by such a call. `None` if something else assigns it,
/// or the parameter's value may reach `at`.
pub fn retargeted<'tcx>(
	tcx: TyCtxt<'tcx>,
	body: &mir::Body<'tcx>,
	local: Local,
	at: Location,
) -> Option<Vec<Place<'tcx>>> {
	retargeted_in(tcx, body, local, at, &mut FxHashSet::default())
}

/// [`retargeted`], past the references and locations `visiting` already follows, which
/// a `mem::swap` in a loop leads back to: each adds no borrow the first doesn't find.
fn retargeted_in<'tcx>(
	tcx: TyCtxt<'tcx>,
	body: &mir::Body<'tcx>,
	local: Local,
	at: Location,
	visiting: &mut FxHashSet<(Local, Location)>,
) -> Option<Vec<Place<'tcx>>> {
	if !body.local_decls[local].ty.is_ref() {
		return None;
	}
	if !visiting.insert((local, at)) {
		return Some(Vec::new());
	}
	// Each `&mut local`, which only such a call may take.
	let mut lent = FxHashSet::default();
	for data in body.basic_blocks.iter() {
		for stmt in &data.statements {
			if let mir::StatementKind::Assign(assign) = &stmt.kind
				&& let Rvalue::Ref(_, _, p) | Rvalue::RawPtr(_, p) = &assign.1
				&& p.local == local
				&& !p.is_indirect()
			{
				match (&assign.1, assign.0.as_local()) {
					(Rvalue::Ref(_, mir::BorrowKind::Shared | mir::BorrowKind::Fake(_), _), _) => {}
					(Rvalue::Ref(_, mir::BorrowKind::Mut { .. }, p), Some(by)) if p.projection.is_empty() => {
						lent.insert(by);
					}
					_ => return None,
				}
			}
		}
	}
	let rebinding = |args: &[rustc_span::Spanned<mir::Operand<'tcx>>], func: &mir::Operand<'tcx>| {
		let i = args.iter().position(|a| {
			a.node
				.place()
				.and_then(|p| p.as_local())
				.is_some_and(|l| lent.contains(&l))
		})?;
		let (def_id, _) = func.const_fn_def()?;
		Some((
			i,
			super::library::key(tcx, def_id).map(|(_, name)| name),
			super::pointer::rebinds(tcx, def_id),
		))
	};
	// Each `&mut local` is used once, by such a call.
	let mut uses = FxHashMap::default();
	UsesOf(&lent, &mut uses).visit_body(body);
	for (data, bb) in body.basic_blocks.iter().zip(body.basic_blocks.indices()) {
		if let mir::TerminatorKind::Call { func, args, .. } = &data.terminator().kind
			&& let Some((i, _, rebinds)) = rebinding(args, func)
			&& i < rebinds
		{
			let by = args[i].node.place().and_then(|p| p.as_local());
			if by.is_some_and(|by| uses.get(&by) == Some(&vec![body.terminator_loc(bb)])) {
				uses.remove(&by.unwrap());
			}
		}
	}
	if !uses.is_empty() {
		return None;
	}
	// What a local assigned once, from a borrow, borrows.
	let borrowed = |op: &mir::Operand<'tcx>| {
		let by = op.place()?.as_local()?;
		let called = body.basic_blocks.iter().any(
			|data| matches!(&data.terminator().kind, mir::TerminatorKind::Call { destination, .. } if destination.local == by),
		);
		let mut defs = body
			.basic_blocks
			.iter()
			.flat_map(|data| &data.statements)
			.filter_map(|stmt| match &stmt.kind {
				mir::StatementKind::Assign(assign) if assign.0.local == by => Some(&assign.1),
				_ => None,
			});
		match (defs.next(), defs.next(), called) {
			(Some(Rvalue::Ref(_, _, p)), None, false) => Some(*p),
			_ => None,
		}
	};
	let blocks = &body.basic_blocks;
	let mut found = Vec::new();
	let mut seen = FxHashSet::default();
	let mut stack = vec![(at.block, Some(at.statement_index))];
	while let Some((bb, before)) = stack.pop() {
		let data = &blocks[bb];
		let end = match before {
			Some(i) => i,
			None => {
				match &data.terminator().kind {
					mir::TerminatorKind::Call { destination, .. } if destination.as_local() == Some(local) => {
						return None;
					}
					mir::TerminatorKind::Yield { resume_arg, .. } if resume_arg.as_local() == Some(local) => {
						return None;
					}
					mir::TerminatorKind::Call { func, args, .. } => match rebinding(args, func) {
						Some((_, Some(name), _)) if name == "replace" => {
							found.push(borrowed(&args[1].node)?);
							continue;
						}
						Some((i, Some(name), _)) if name == "swap" => {
							let other = borrowed(&args[1 - i].node)?.as_local()?;
							found.extend(retargeted_in(tcx, body, other, body.terminator_loc(bb), visiting)?);
							continue;
						}
						Some(_) => return None,
						None => {}
					},
					_ => {}
				}
				data.statements.len()
			}
		};
		let def = data.statements[..end].iter().rev().find_map(|stmt| match &stmt.kind {
			mir::StatementKind::Assign(assign) if assign.0.as_local() == Some(local) => Some(&assign.1),
			_ => None,
		});
		match def {
			Some(Rvalue::Ref(_, _, p)) => found.push(*p),
			Some(_) => return None,
			None if bb == mir::START_BLOCK => return None,
			None => {
				for &pred in &blocks.predecessors()[bb] {
					if seen.insert(pred) {
						stack.push((pred, None));
					}
				}
			}
		}
	}
	found.dedup();
	Some(found)
}

/// Where each of some locals is used, other than assigned or marked.
struct UsesOf<'x>(&'x FxHashSet<Local>, &'x mut FxHashMap<Local, Vec<Location>>);

impl<'tcx> Visitor<'tcx> for UsesOf<'_> {
	fn visit_local(&mut self, local: Local, context: PlaceContext, at: Location) {
		if self.0.contains(&local)
			&& !matches!(
				context,
				PlaceContext::NonUse(_) | PlaceContext::MutatingUse(MutatingUseContext::Store)
			) {
			self.1.entry(local).or_default().push(at);
		}
	}
}

/// The writes of one body.
struct BodyWrites<'w, 'm, 'a, 'tcx> {
	tcx: TyCtxt<'tcx>,
	body: &'m mir::Body<'tcx>,
	def_id: DefId,
	writes: &'w mut Writes<'tcx>,
	own: &'w dyn Fn(DefId) -> bool,
	/// The place each local borrows, where its one assignment is a borrow.
	borrows: IndexVec<mir::Local, Option<Place<'tcx>>>,
	/// The body's pointers, which tell whether a write fills memory nothing points into.
	trace: Trace<'m, 'a, 'tcx>,
	/// Rustc's liveness of the body's locals, found the first time it is needed.
	live: Option<ResultsCursor<'m, 'tcx, MaybeLiveLocals>>,
}

impl<'tcx> BodyWrites<'_, '_, '_, 'tcx> {
	fn insert(&mut self, ty: Ty<'tcx>) {
		self.writes.into.insert(self.tcx, self.def_id, ty);
	}

	/// `ty`, and each type it holds by value, as replaced: a write of a whole `ty` gives
	/// each part it holds another object, and so each part those hold, at any depth.
	fn replace(&mut self, ty: Ty<'tcx>) {
		for ty in self.parts(ty) {
			self.writes.replaced.insert(self.tcx, self.def_id, ty);
		}
	}

	/// `ty` and each type it holds by value, at any depth, a part of which may be kept as a
	/// JS object.
	fn parts(&self, ty: Ty<'tcx>) -> Vec<Ty<'tcx>> {
		let env = ty::TypingEnv::post_analysis(self.tcx, self.def_id);
		by_value(self.tcx, env, ty).filter(|&ty| self.inline(ty)).collect()
	}

	/// Whether a `ty` may be kept as a JS object that a pointer to it is: a tuple, an
	/// array, a struct or union of a crate that translates, or a `MaybeUninit` of one; or
	/// an `UnsafeCell`'s `rt.Cell`, which the pointer `get` gives is or holds.
	fn inline(&self, ty: Ty<'tcx>) -> bool {
		match ty.kind() {
			ty::Tuple(tys) => !tys.is_empty(),
			ty::Array(..) | ty::Param(_) | ty::Alias(..) => true,
			ty::Adt(adt, args) => match Std::of(self.tcx, adt.did()) {
				Some(Std::MaybeUninit) => self.inline(args.type_at(0)),
				Some(Std::Cell) => self.tcx.is_lang_item(adt.did(), rustc_attr_ir::LangItem::UnsafeCell),
				Some(_) => false,
				None => !adt.is_enum() && (self.own)(adt.did()),
			},
			_ => false,
		}
	}

	/// A write of a whole `place` through `(*r)` at `at`, which writes the place `r`
	/// borrows if `r` is an alias of it, as a store of that place.
	fn store_through(&mut self, place: Place<'tcx>, at: Location) {
		if let [ProjectionElem::Deref] = place.projection.as_slice()
			&& let Some(borrowed) = self.borrows[place.local]
		{
			self.stored(borrowed, at);
		}
	}

	/// A whole `place` written at `at`, unless the memory is fresh: each part of what it
	/// held, at any depth, gets another object, and a whole field kept as an object gives
	/// what holds it another object there. A variable assigned whole is judged where a
	/// pointer into it is taken.
	fn stored(&mut self, place: Place<'tcx>, at: Location) {
		let Some((base, elem)) = place.iter_projections().last() else {
			return;
		};
		let ty = place.ty(self.body, self.tcx).ty;
		let replaced = self.parts(ty);
		let holder = match elem {
			ProjectionElem::Field(..) if self.inline(ty) => {
				Some(base.ty(self.body, self.tcx).ty).filter(|&base| !base.is_union() && self.inline(base))
			}
			_ => None,
		};
		if (!replaced.is_empty() || holder.is_some()) && !self.fresh(place, at, None) {
			for ty in replaced {
				self.writes.replaced.insert(self.tcx, self.def_id, ty);
			}
			if let Some(holder) = holder {
				self.writes.holders.insert(self.tcx, self.def_id, holder);
			}
		}
	}

	/// Whether a write of `place` at `at`, or by a call whose result is `made`, fills memory
	/// nothing points into yet, as [`Trace::fresh`] judges it.
	fn fresh(&mut self, place: Place<'tcx>, at: Location, made: Option<Local>) -> bool {
		let filled = match place.projection.split_first() {
			Some((ProjectionElem::Deref, rest)) if !rest.contains(&ProjectionElem::Deref) => {
				Filled::Through(place.local)
			}
			_ if !place.is_indirect() => Filled::Local(place.local),
			_ => return false,
		};
		let live = self.live.get_or_insert_with(|| self.trace.liveness());
		self.trace
			.fresh(filled, place.ty(self.body, self.tcx).ty, at, made, live)
	}

	/// Whether the call at `at` given `arg` as a `ty`, a mutable reference or pointer to a
	/// type that may be kept as an object, fills memory nothing points into yet there.
	fn fills_fresh(&mut self, ty: Ty<'tcx>, arg: &mir::Operand<'tcx>, at: Location, made: Option<Local>) -> bool {
		let (ty::Ref(_, inner, ty::Mutability::Mut) | ty::RawPtr(inner, ty::Mutability::Mut)) = *ty.kind() else {
			return false;
		};
		self.inline(inner)
			&& arg
				.place()
				.and_then(|p| p.as_local())
				.is_some_and(|p| self.fresh(self.tcx.mk_place_deref(p.into()), at, made))
	}

	/// What each mutable reference or pointer a value of `ty` holds points at, where code
	/// given the value can reach it, written into, and if `replace`, replaced. What a
	/// closure or a coroutine holds only its own body reads, a function holds nothing,
	/// and what a trait object holds counted where it became one.
	fn pointees(&mut self, ty: Ty<'tcx>, replace: bool) {
		let mut stack = vec![ty];
		while let Some(ty) = stack.pop() {
			match *ty.kind() {
				ty::Ref(_, inner, mutability) | ty::RawPtr(inner, mutability) => {
					if mutability.is_mut() {
						self.insert(inner);
						if replace {
							self.replace(inner);
						}
					}
					stack.push(inner);
				}
				ty::Adt(_, args) => stack.extend(args.types()),
				ty::Tuple(tys) => stack.extend(tys),
				ty::Array(elem, _) | ty::Slice(elem) => stack.push(elem),
				_ => {}
			}
		}
	}

	/// A raw pointer made of `place` at `at` into `made`: what each reference it is made
	/// through points at is taken, for every body if the pointer may outlive the call, or
	/// else for this body alone if the reference may point at its own locals. A pointer
	/// made of the body's own local is judged where it is made.
	fn take(&mut self, place: Place<'tcx>, at: Location, made: Place<'tcx>) {
		let borrows = &self.borrows;
		let places = borrowed_places(self.tcx, self.body, place, at, &|local| borrows[local]);
		let outlives = self.outlives(made);
		for place in places {
			let Some(last) = place.projection.iter().rposition(|e| e == ProjectionElem::Deref) else {
				continue;
			};
			let reference = &place.projection[..last];
			if !Place::ty_from(place.local, reference, self.body, self.tcx).ty.is_ref() {
				continue;
			}
			let referent = Place::ty_from(place.local, &place.projection[..=last], self.body, self.tcx).ty;
			let taken = match (outlives, self.outside(place.local, reference)) {
				(true, _) => &mut self.writes.taken,
				(false, false) => self.writes.taken_in.entry(self.def_id).or_default(),
				(false, true) => continue,
			};
			taken.insert(self.tcx, self.def_id, referent);
		}
	}

	/// Whether the reference at `local` and `projection` points at none of this body's
	/// locals: one made of no borrow of one, as a parameter's, or a closure's capture.
	fn outside(&self, local: Local, projection: &[ProjectionElem<Local, Ty<'tcx>>]) -> bool {
		match projection {
			_ if !projection.contains(&ProjectionElem::Deref) => !self.trace.may_point_into_locals(local),
			[ProjectionElem::Deref, ProjectionElem::Field(..)] => {
				local == Local::from_u32(1)
					&& self.tcx.is_closure_like(self.def_id)
					&& self.tcx.coroutine_kind(self.def_id).is_none()
			}
			_ => false,
		}
	}

	/// Whether the pointer `made` holds may be read after the body returns: returned as a
	/// raw pointer, or kept where liveness can't follow it, such as in memory or by a call
	/// that keeps it. One returned only as a reference is a borrow the caller's borrow
	/// checker bounds, which no assignment of what it points at outlives.
	fn outlives(&self, made: Place<'tcx>) -> bool {
		let Some(made) = made.as_local() else {
			return true;
		};
		let held = self.trace.holders(made, false, false);
		let env = ty::TypingEnv::post_analysis(self.tcx, self.def_id);
		let raw = |ty: Ty<'tcx>| {
			ty.is_raw_ptr()
				|| matches!(
					ty.kind(),
					ty::Param(_) | ty::Alias(..) | ty::Closure(..) | ty::Coroutine(..) | ty::CoroutineClosure(..)
				)
		};
		!held.escapes.is_empty()
			|| held.all.contains(mir::RETURN_PLACE) && by_value(self.tcx, env, self.body.return_ty()).any(raw)
	}

	/// Whether the function item `def_id` with `args` translates, whose body is
	/// read, rather than std's or a trait method only an instance resolves.
	fn crate_fn(&self, def_id: DefId, args: ty::GenericArgsRef<'tcx>) -> bool {
		let env = ty::TypingEnv::post_analysis(self.tcx, self.def_id);
		matches!(
			ty::Instance::try_resolve(self.tcx, env, def_id, args),
			Ok(Some(instance)) if matches!(instance.def, ty::InstanceKind::Item(did) if (self.own)(did))
		)
	}
}

impl<'tcx> Visitor<'tcx> for BodyWrites<'_, '_, '_, 'tcx> {
	fn visit_assign(&mut self, place: &Place<'tcx>, rvalue: &Rvalue<'tcx>, location: Location) {
		if let Rvalue::RawPtr(kind, pointee) = rvalue
			&& *kind != mir::RawPtrKind::FakeForPtrMetadata
		{
			self.take(*pointee, location, *place);
		}
		if place.as_local().is_none() {
			self.pointees(place.ty(self.body, self.tcx).ty, false);
		}
		self.store_through(*place, location);
		// A `dyn Any` has no method that writes: a write through its downcast counts where it is.
		let object = |t: Ty<'tcx>| t.is_trait() && !super::dynamic::is_any_object(self.tcx, t);
		if let Rvalue::Cast(mir::CastKind::PointerCoercion(ty::adjustment::PointerCoercion::Unsize, _), op, to) = rvalue
			&& to.walk().any(|arg| arg.as_type().is_some_and(object))
		{
			self.pointees(op.ty(self.body, self.tcx), true);
		}
		self.super_assign(place, rvalue, location);
	}

	fn visit_terminator(&mut self, terminator: &mir::Terminator<'tcx>, location: Location) {
		if let mir::TerminatorKind::Call {
			func,
			args,
			destination,
			..
		} = &terminator.kind
		{
			self.store_through(*destination, location);
			if let Some((def_id, gargs)) = func.const_fn_def()
				&& super::pointer::from_reference(self.tcx, def_id, gargs)
				&& let Some(reference) = args.first().and_then(|a| a.node.place())
			{
				self.take(self.tcx.mk_place_deref(reference), location, *destination);
			}
			let rebound = func
				.const_fn_def()
				.map_or(0, |(def_id, _)| super::pointer::rebinds(self.tcx, def_id));
			for arg in args[..rebound].iter().filter_map(|a| a.node.place()) {
				self.store_through(self.tcx.mk_place_deref(arg), location);
			}
		}
		match &terminator.kind {
			mir::TerminatorKind::Call { destination, .. } if destination.as_local().is_none() => {
				self.pointees(destination.ty(self.body, self.tcx).ty, false)
			}
			mir::TerminatorKind::Yield { value, .. } => self.pointees(value.ty(self.body, self.tcx), true),
			_ => {}
		}
		self.super_terminator(terminator, location);
	}

	/// A function not of the crate, called here or taken as a value that a pointer or a
	/// type parameter calls where no call shows: what its parameters take escapes to it,
	/// and what it [`replaces`] behind its `&self` is replaced. This holds while the
	/// runtime writes in place only through such a parameter, as `copy_from_slice` does,
	/// or through a cell's `&self`, never in a std trait impl that a type parameter reaches.
	fn visit_operand(&mut self, operand: &mir::Operand<'tcx>, location: Location) {
		if let mir::Operand::Constant(constant) = operand
			&& let ty::FnDef(def_id, args) = *constant.ty().kind()
			&& !self.crate_fn(def_id, args.skip_binder())
		{
			let sig = self
				.tcx
				.fn_sig(def_id)
				.instantiate(self.tcx, args.skip_binder())
				.skip_norm_wip();
			if let Some(&this) = sig.skip_binder().inputs().first()
				&& let Some(value) = replaces(self.tcx, def_id, this)
			{
				self.replace(value);
			}
			// Those that point a pointer elsewhere or read through it replace nothing.
			let replace = !super::pointer::keeps(self.tcx, def_id, args.skip_binder());
			// A call's argument may point into memory nothing else points into yet.
			let body = self.body;
			let call = match &body[location.block].terminator().kind {
				mir::TerminatorKind::Call {
					func,
					args,
					destination,
					..
				} if location == body.terminator_loc(location.block) && std::ptr::eq(func, operand) => {
					Some((args, destination.as_local()))
				}
				_ => None,
			};
			for (i, &input) in sig.skip_binder().inputs().iter().enumerate() {
				let fresh = replace
					&& call.is_some_and(|(args, made)| {
						args.get(i)
							.is_some_and(|arg| self.fills_fresh(input, &arg.node, location, made))
					});
				match (fresh, input.kind()) {
					(true, ty::Ref(_, inner, _) | ty::RawPtr(inner, _)) => {
						self.insert(*inner);
						self.pointees(*inner, true);
					}
					_ => self.pointees(input, replace),
				}
			}
		}
		self.super_operand(operand, location);
	}

	fn visit_place(&mut self, place: &Place<'tcx>, context: PlaceContext, location: Location) {
		let PlaceContext::MutatingUse(kind) = context else {
			return;
		};
		// A write to a part of a value writes into the value, unless the part is behind a
		// reference, whose referent it writes into instead.
		for (base, elem) in place.iter_projections() {
			if !matches!(elem, ProjectionElem::Deref) {
				self.insert(base.ty(self.body, self.tcx).ty);
			}
		}
		// A write to a whole local gives it another value, but one through a reference or a
		// box writes into the value there.
		if matches!(place.projection.last(), Some(ProjectionElem::Deref))
			|| matches!(kind, MutatingUseContext::SetDiscriminant)
		{
			self.insert(place.ty(self.body, self.tcx).ty);
		}
		if matches!(
			kind,
			MutatingUseContext::Store
				| MutatingUseContext::Call
				| MutatingUseContext::AsmOutput
				| MutatingUseContext::Yield
		) {
			self.stored(*place, location);
		}
	}
}
