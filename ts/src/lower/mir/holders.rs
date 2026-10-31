//! What may hold a raw pointer, so that rustc's liveness of those locals bounds where it
//! may still be read: the pointer made at the statement being lowered, or every pointer
//! into memory a write fills, which nothing points into yet if none may be read after.
//! Std's code given one, as its iterators are, keeps it only in what it returns if it can
//! run none of the crate's code: each of the crate's types it is given has no drop glue
//! and meets only bounds that call nothing or that std's own impls, inert in turn, meet.

use std::cell::RefCell;

use rustc_data_structures::either::Either;
use rustc_data_structures::fx::{FxHashMap, FxHashSet, FxIndexSet};
use rustc_hir::def_id::DefId;
use rustc_index::IndexVec;
use rustc_index::bit_set::DenseBitSet;
use rustc_middle::mir::visit::{PlaceContext, Visitor};
use rustc_middle::mir::{
	self, AggregateKind, BorrowKind, Local, Location, Operand, Place, ProjectionElem, RawPtrKind, Rvalue,
	StatementKind, TerminatorKind,
};
use rustc_middle::traits::ImplSource;
use rustc_middle::ty::{self, GenericArgsRef, Instance, Interner as _, Ty, TypingEnv};
use rustc_mir_dataflow::impls::MaybeLiveLocals;
use rustc_mir_dataflow::{Analysis, ResultsCursor};
use rustc_span::Spanned;

use super::{Fx, own};
use crate::lower::Translator;
use crate::lower::library::Std;
use crate::lower::pointer::{self, Derives};

impl<'tcx> Fx<'_, '_, '_, 'tcx> {
	/// The locals that may hold the raw pointer the statement being lowered makes, or what
	/// is made of it, as [`Trace::holders`] finds them. `None` if one may escape where
	/// liveness can't follow it.
	pub(super) fn holders(&self) -> Option<DenseBitSet<Local>> {
		let made = match self.mir.stmt_at(self.at) {
			Either::Left(stmt) => match &stmt.kind {
				StatementKind::Assign(assign) => assign.0,
				_ => return None,
			},
			Either::Right(term) => match &term.kind {
				TerminatorKind::Call { destination, .. } => *destination,
				_ => return None,
			},
		};
		if !own(made) {
			return None;
		}
		let memo = RefCell::default();
		let held = Trace::new(self.b.t, self.mir, self.b.t.typing_env_pub(), &memo).holders(made.local, false);
		held.escapes.is_empty().then_some(held.all)
	}
}

/// A body's raw pointers, followed from where they are made.
pub(in crate::lower) struct Trace<'x, 'a, 'tcx> {
	t: &'x Translator<'a, 'tcx>,
	mir: &'x mir::Body<'tcx>,
	/// Where the body's trait bounds come from.
	env: TypingEnv<'tcx>,
	/// What each of the crate's functions does with the pointers it is given, as found.
	memo: &'x RefCell<Memo<'tcx>>,
	/// Each local's whole assignments, and whether anything else may change it.
	defs: RefCell<Option<IndexVec<Local, Defs<'x, 'tcx>>>>,
}

/// What each of the crate's functions does with the pointers it is given, and which of
/// std's run none of the crate's code.
#[derive(Default)]
pub(in crate::lower) struct Memo<'tcx> {
	/// Whether a function may keep any of the given parameters anywhere but in its result.
	kept: FxHashMap<(DefId, Vec<Local>), bool>,
	/// Where each function's result may point: where its parameters do, or at a constant
	/// or a thread-local; or `None` if anywhere else.
	returned: FxHashMap<DefId, Option<Vec<Source>>>,
	/// Whether std's function or impl, given these arguments, is [`Trace::inert`].
	inert: FxHashMap<(DefId, GenericArgsRef<'tcx>), bool>,
}

/// The locals that may hold a pointer, and where one may escape them.
pub(in crate::lower) struct Held {
	pub all: DenseBitSet<Local>,
	/// Those that may hold a pointer to a part of what one points at, or to anything else
	/// made of it, rather than only to its whole.
	pub parts: DenseBitSet<Local>,
	/// Those, not among `parts`, that may hold pointers std's code or indexing made of one
	/// instead, as an iterator over its elements does: each of a type that tells what it
	/// points at, a whole value of that type or of one holding it.
	elems: DenseBitSet<Local>,
	/// The local the pointers point to, if they are made of it rather than held in it.
	value: Option<Local>,
	/// Where one may escape where liveness can't follow it: into memory, a closure, an
	/// async body's state (which is memory), or a call that may keep it.
	pub escapes: Vec<Location>,
}

/// A local's whole assignments, the locals its mutable borrows are, and whether anything
/// else may change it.
#[derive(Default)]
struct Defs<'x, 'tcx> {
	whole: Vec<Def<'x, 'tcx>>,
	lent: Vec<Local>,
	other: bool,
}

#[derive(Clone, Copy)]
enum Def<'x, 'tcx> {
	Assign(&'x Rvalue<'tcx>),
	Call(&'x Operand<'tcx>, &'x [Spanned<Operand<'tcx>>]),
}

/// How much of what a pointer was made for one may point at, finest last.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
	Whole,
	/// As [`Held::elems`].
	Elems,
	Part,
}

/// Where a pointer may have come from.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Source {
	/// Memory the call assigning this local, or changing it through a borrow, may have
	/// allocated, reached only through what the local holds.
	Fresh(Local),
	/// A borrow of this local or a part of it.
	Local(Local),
	/// This parameter.
	Param(Local),
	/// A constant or a thread-local, which no local is.
	Static,
}

/// The memory a write fills.
#[derive(Clone, Copy)]
pub(in crate::lower) enum Filled {
	/// Where this local points.
	Through(Local),
	/// This local or a part of it.
	Local(Local),
}

impl<'x, 'a, 'tcx> Trace<'x, 'a, 'tcx> {
	pub fn new(
		t: &'x Translator<'a, 'tcx>,
		mir: &'x mir::Body<'tcx>,
		env: TypingEnv<'tcx>,
		memo: &'x RefCell<Memo<'tcx>>,
	) -> Self {
		Self {
			t,
			mir,
			env,
			memo,
			defs: RefCell::new(None),
		}
	}

	/// The locals that may hold the pointer `root` holds, or with `value`, one to `root`
	/// itself, or what is made of it: a copy, a cast, a reference or pointer made through
	/// it, a value read through it that could hold one, or what std or the crate returns
	/// given one.
	pub fn holders(&self, root: Local, value: bool) -> Held {
		let mir = self.mir;
		let mut held = Held {
			all: DenseBitSet::new_empty(mir.local_decls.len()),
			parts: DenseBitSet::new_empty(mir.local_decls.len()),
			elems: DenseBitSet::new_empty(mir.local_decls.len()),
			value: value.then_some(root),
			escapes: Vec::new(),
		};
		held.all.insert(root);
		let mut grew = true;
		while grew {
			grew = false;
			held.escapes.clear();
			for (block, data) in mir.basic_blocks.iter_enumerated() {
				for (statement_index, stmt) in data.statements.iter().enumerate() {
					let at = Location { block, statement_index };
					match &stmt.kind {
						StatementKind::Assign(assign) => {
							let (to, rvalue) = &**assign;
							if !reads(&held.all, |v| v.visit_rvalue(rvalue, at)) {
								continue;
							}
							if let Rvalue::Aggregate(kind, _) = rvalue
								&& matches!(
									**kind,
									AggregateKind::Closure(..)
										| AggregateKind::Coroutine(..)
										| AggregateKind::CoroutineClosure(..)
								) {
								held.escapes.push(at);
								continue;
							}
							let level = match rvalue {
								Rvalue::Cast(_, op, ty) => {
									self.retyped(level(rvalue, &held), op.ty(&mir.local_decls, self.t.tcx), *ty)
								}
								_ => level(rvalue, &held),
							};
							match self.held_in(*to, level, &mut held) {
								Some(new) => grew |= new,
								None => held.escapes.push(at),
							}
						}
						StatementKind::Intrinsic(_) if reads(&held.all, |v| v.visit_statement(stmt, at)) => {
							held.escapes.push(at);
						}
						_ => {}
					}
				}
				let at = mir.terminator_loc(block);
				let term = data.terminator();
				match &term.kind {
					TerminatorKind::Call {
						func,
						args,
						destination,
						..
					} => {
						let given: Vec<usize> = (0..args.len())
							.filter(|&i| reads_operand(&args[i].node, &held.all))
							.collect();
						if given.is_empty() && !reads_operand(func, &held.all) {
							continue;
						}
						let returned = destination.ty(&mir.local_decls, self.t.tcx).ty;
						match self.call(func, args, returned, &given, &held) {
							Some(level) => match self.held_in(*destination, level, &mut held) {
								Some(new) => grew |= new,
								None => held.escapes.push(at),
							},
							None => held.escapes.push(at),
						}
					}
					TerminatorKind::Drop { place, .. }
						if held.all.contains(place.local)
							&& self.runs_own_code(place.ty(&mir.local_decls, self.t.tcx).ty) =>
					{
						held.escapes.push(at);
					}
					TerminatorKind::TailCall { .. }
					| TerminatorKind::InlineAsm { .. }
					| TerminatorKind::Yield { .. }
						if reads(&held.all, |v| v.visit_terminator(term, at)) =>
					{
						held.escapes.push(at);
					}
					_ => {}
				}
			}
		}
		held
	}

	/// Whether `to`, given a pointer at `level` of what it points at, newly holds it; `None`
	/// if that is memory, reached through a pointer.
	fn held_in(&self, to: Place<'tcx>, level: Level, held: &mut Held) -> Option<bool> {
		if !self.carries(to.ty(&self.mir.local_decls, self.t.tcx).ty) {
			return Some(false);
		}
		if !own(to) {
			return None;
		}
		let mut new = held.all.insert(to.local);
		// A local assigned in part holds whatever else it held, too.
		match level {
			_ if !to.projection.is_empty() => new |= held.parts.insert(to.local),
			Level::Part => new |= held.parts.insert(to.local),
			Level::Elems => new |= held.elems.insert(to.local),
			Level::Whole => {}
		}
		Some(new)
	}

	/// What a call of `func` given `args`, of which those at `given` hold pointers, returns
	/// as a `returned`: a pointer at what level of what one points at; `None` if it may keep
	/// one anywhere else.
	fn call(
		&self,
		func: &Operand<'tcx>,
		args: &[Spanned<Operand<'tcx>>],
		returned: Ty<'tcx>,
		given: &[usize],
		held: &Held,
	) -> Option<Level> {
		if reads_operand(func, &held.all) {
			return None;
		}
		let ty::FnDef(def_id, gargs) = *func.ty(&self.mir.local_decls, self.t.tcx).kind() else {
			return None;
		};
		// What a given argument holds, or a part of it if it is no local's own value.
		let arg = |i: usize| match args[i].node.place() {
			Some(p) if !p.is_indirect() => held.of(p.local),
			_ => Level::Part,
		};
		if let Some(derives) = pointer::derives(self.t.tcx, def_id) {
			if pointer::stores(self.t.tcx, def_id) && given.iter().any(|&i| i > 0) {
				return None;
			}
			return Some(match derives {
				Derives::Same if given == [0] => {
					self.retyped(arg(0), args[0].node.ty(&self.mir.local_decls, self.t.tcx), returned)
				}
				_ => Level::Part,
			});
		}
		if let Some(did) = self.read_fn(def_id, gargs.skip_binder()) {
			let params = given.iter().map(|&i| Local::from_usize(i + 1)).collect();
			return (!self.keeps(did, params)).then_some(Level::Part);
		}
		let tys: Vec<Ty<'tcx>> = given
			.iter()
			.map(|&i| args[i].node.ty(&self.mir.local_decls, self.t.tcx))
			.collect();
		self.leaves(def_id, gargs.skip_binder(), args, &tys)
			.then(|| given.iter().map(|&i| arg(i)).fold(Level::Elems, Level::max))
	}

	/// `level`, of a pointer of type `from` made one of type `to`: a part where it pointed at
	/// [`Level::Elems`] and now points at another type, which its type no longer tells.
	fn retyped(&self, level: Level, from: Ty<'tcx>, to: Ty<'tcx>) -> Level {
		let tcx = self.t.tcx;
		let pointee = |ty: Ty<'tcx>| {
			let pointee = match *ty.kind() {
				ty::Ref(_, pointee, _) | ty::RawPtr(pointee, _) => pointee,
				ty::Adt(adt, args) if ty.is_box() || Std::of(tcx, adt.did()) == Some(Std::NonNull) => args.type_at(0),
				_ => return None,
			};
			Some(storage(tcx, self.peel(pointee)))
		};
		match (level, pointee(from), pointee(to)) {
			(Level::Elems, Some(from), Some(to)) if from != to => Level::Part,
			_ => level,
		}
	}

	/// The crate's function a call of `def_id` with `args` runs, if its body translates and
	/// takes its arguments as it declares them, which a closure's doesn't.
	fn read_fn(&self, def_id: DefId, args: ty::GenericArgsRef<'tcx>) -> Option<DefId> {
		let tcx = self.t.tcx;
		let instance = Instance::try_resolve(tcx, self.env, def_id, args).ok()??;
		let ty::InstanceKind::Item(did) = instance.def else {
			return None;
		};
		(self.t.lowers(instance)
			&& !crate::lower::library::is_std(tcx, did)
			&& !tcx.is_closure_like(did)
			&& tcx.def_kind(did).is_fn_like()
			&& tcx.is_mir_available(did))
		.then_some(did)
	}

	/// Whether the crate's function `did` may keep what its `params` hold anywhere but in
	/// its result.
	fn keeps(&self, did: DefId, params: Vec<Local>) -> bool {
		let key = (did, params);
		if let Some(&kept) = self.memo.borrow().kept.get(&key) {
			return kept;
		}
		// Until found, as for a function that calls itself, it may.
		self.memo.borrow_mut().kept.insert(key.clone(), true);
		let callee = self.callee(did);
		let kept = key
			.1
			.iter()
			.any(|&param| !callee.holders(param, false).escapes.is_empty());
		self.memo.borrow_mut().kept.insert(key, kept);
		kept
	}

	/// Where the crate's function `did`'s result may point: where each [`Source::Param`]
	/// does, or at a [`Source::Static`]; or `None` if anywhere else.
	fn returned(&self, did: DefId) -> Option<Vec<Source>> {
		if let Some(returned) = self.memo.borrow().returned.get(&did) {
			return returned.clone();
		}
		self.memo.borrow_mut().returned.insert(did, None);
		let returned = self.callee(did).origins(mir::RETURN_PLACE, None).and_then(|sources| {
			sources
				.into_iter()
				.map(|source| match source {
					Source::Param(_) | Source::Static => Some(source),
					Source::Fresh(_) | Source::Local(_) => None,
				})
				.collect()
		});
		self.memo.borrow_mut().returned.insert(did, returned.clone());
		returned
	}

	/// The trace of the crate's function `did`'s body.
	fn callee(&self, did: DefId) -> Trace<'x, 'a, 'tcx> {
		let tcx = self.t.tcx;
		Trace::new(
			self.t,
			tcx.optimized_mir(did),
			TypingEnv::post_analysis(tcx, did),
			self.memo,
		)
	}

	/// Whether a call of std's `def_id` with `args`, of which the holders are `given`, keeps
	/// none of them anywhere but its result: std's code, calling none of the crate's, with
	/// no argument reaching somewhere that could take one in, as `Vec::push` does.
	fn leaves(
		&self,
		def_id: DefId,
		gargs: ty::GenericArgsRef<'tcx>,
		args: &[Spanned<Operand<'tcx>>],
		given: &[Ty<'tcx>],
	) -> bool {
		self.runs_std(def_id, gargs, args)
			&& !args.iter().any(|a| {
				let t = a.node.ty(&self.mir.local_decls, self.t.tcx);
				given.iter().any(|&h| self.t.stores(t, h))
			})
	}

	/// Whether a call of `def_id` with `args` runs std's code, calling none of the crate's.
	fn runs_std(&self, def_id: DefId, gargs: ty::GenericArgsRef<'tcx>, args: &[Spanned<Operand<'tcx>>]) -> bool {
		if self.translated(def_id, gargs) {
			return false;
		}
		match Instance::try_resolve(self.t.tcx, self.env, def_id, gargs) {
			Ok(Some(instance)) if matches!(instance.def, ty::InstanceKind::Item(_)) => {
				let did = instance.def_id();
				crate::lower::library::is_std(self.t.tcx, did)
					&& self.inert(did, instance.args)
					&& !self.opaque_sig(did)
			}
			_ => !gargs
				.types()
				.chain(args.iter().map(|a| a.node.ty(&self.mir.local_decls, self.t.tcx)))
				.any(|t| self.runs_own_code(t)),
		}
	}

	/// Whether std's function or impl `did`, given `args`, runs none of the crate's code: each
	/// argument that could reach it has no drop glue, and each bound of `did` that one meets
	/// calls nothing, as `Sized` or `Send`, or is met by std's own impl, inert in turn. Code
	/// generic over a type calls its methods only through its bounds, so an iterator over
	/// slots of the crate's type, bound by none, runs none of it.
	fn inert(&self, did: DefId, args: GenericArgsRef<'tcx>) -> bool {
		let tcx = self.t.tcx;
		let key = (did, args);
		if let Some(&inert) = self.memo.borrow().inert.get(&key) {
			return inert;
		}
		// Until found, as for an impl whose bounds come back to it, it may not be.
		self.memo.borrow_mut().inert.insert(key, false);
		let inert = args
			.types()
			.all(|t| !self.runs_own_code(t) || !t.needs_drop(tcx, self.env))
			&& tcx
				.clauses_of(did)
				.instantiate(tcx, args)
				.clauses
				.into_iter()
				.all(|clause| {
					let Some(clause) = clause.skip_norm_wip().as_trait_clause() else {
						return true;
					};
					let trait_ref = tcx.instantiate_bound_regions_with_erased(clause).trait_ref;
					if !trait_ref.args.types().any(|t| self.runs_own_code(t)) || calls_nothing(tcx, trait_ref.def_id) {
						return true;
					}
					let Ok(trait_ref) =
						tcx.try_normalize_erasing_regions(self.env, ty::Unnormalized::new_wip(trait_ref))
					else {
						return false;
					};
					match tcx.codegen_select_candidate(self.env.as_query_input(trait_ref)) {
						Ok(ImplSource::UserDefined(imp)) => {
							crate::lower::library::is_std(tcx, imp.impl_def_id) && self.inert(imp.impl_def_id, imp.args)
						}
						_ => false,
					}
				});
		self.memo.borrow_mut().inert.insert(key, inert);
		inert
	}

	/// Whether std's function `did` takes or returns what could run code that its generic
	/// arguments don't tell: a trait object or a function pointer.
	fn opaque_sig(&self, did: DefId) -> bool {
		let sig = self
			.t
			.tcx
			.fn_sig(did)
			.instantiate_identity()
			.skip_norm_wip()
			.skip_binder();
		sig.inputs_and_output.iter().any(|ty| {
			ty.walk().filter_map(|arg| arg.as_type()).any(|t| {
				matches!(
					t.kind(),
					ty::Dynamic(..) | ty::FnPtr(..) | ty::Closure(..) | ty::CoroutineClosure(..) | ty::Coroutine(..)
				)
			})
		})
	}

	/// Whether a call of `def_id` with `args` runs a function whose body translates.
	fn translated(&self, def_id: DefId, args: ty::GenericArgsRef<'tcx>) -> bool {
		match Instance::try_resolve(self.t.tcx, self.env, def_id, args) {
			Ok(Some(instance)) => self.t.lowers(instance) || self.t.dispatches(instance),
			_ => false,
		}
	}

	/// Whether std's code handed a value of `ty` may call the crate's on it, which could
	/// keep what it is given: through the crate's own types, a trait object, a closure, or
	/// a function pointer.
	fn runs_own_code(&self, ty: Ty<'tcx>) -> bool {
		self.t.reaches(ty, |t| match t.kind() {
			ty::Adt(adt, _) => self.t.own(adt.did()),
			ty::FnDef(def_id, args) => self.translated(*def_id, args.skip_binder()),
			ty::Dynamic(..)
			| ty::Closure(..)
			| ty::CoroutineClosure(..)
			| ty::Coroutine(..)
			| ty::FnPtr(..)
			| ty::Param(_)
			| ty::Alias(..) => true,
			_ => false,
		})
	}

	/// Whether a value of `ty` could hold a pointer or a reference, in any field of any type.
	fn carries(&self, ty: Ty<'tcx>) -> bool {
		let mut seen = FxHashSet::default();
		let mut stack = vec![ty];
		while let Some(ty) = stack.pop() {
			if !seen.insert(ty) {
				continue;
			}
			match ty.kind() {
				ty::Bool
				| ty::Char
				| ty::Int(_)
				| ty::Uint(_)
				| ty::Float(_)
				| ty::Str
				| ty::Never
				| ty::FnDef(..)
				| ty::FnPtr(..) => {}
				ty::Array(elem, _) | ty::Slice(elem) | ty::Pat(elem, _) => stack.push(*elem),
				ty::Tuple(tys) => stack.extend(tys.iter()),
				ty::Adt(adt, args) => stack.extend(adt.all_fields().map(|field| self.t.field_ty(field.did, args))),
				_ => return true,
			}
		}
		false
	}

	/// Whether a write at `at` of a `filled` value fills memory nothing points into yet:
	/// where it may run, no pointer into the memory has escaped, and every local that may
	/// hold one and be read after, but the call's own result `made`, holds only pointers to
	/// the whole of what it was made for, which the write fills in place and gives only its
	/// parts other objects, or, as one made by std's code or indexing, to no part of a
	/// `filled` value.
	pub fn fresh(
		&self,
		filled: Filled,
		ty: Ty<'tcx>,
		at: Location,
		made: Option<Local>,
		live: &mut ResultsCursor<'x, 'tcx, MaybeLiveLocals>,
	) -> bool {
		let sources = match filled {
			Filled::Through(pointer) => match self.origins(pointer, Some(ty)) {
				Some(sources) => sources,
				None => return false,
			},
			Filled::Local(local) => FxIndexSet::from_iter([Source::Local(local)]),
		};
		sources.into_iter().all(|source| {
			let held = match source {
				Source::Fresh(root) => self.holders(root, false),
				Source::Local(root) => self.holders(root, true),
				Source::Param(_) | Source::Static => return false,
			};
			if held.escapes.iter().any(|&escape| self.reaches(escape, at)) {
				return false;
			}
			live.seek_before_primary_effect(at);
			let live = live.get();
			let parts = held
				.parts
				.iter()
				.any(|local| Some(local) != made && live.contains(local));
			!parts
				&& held.elems.iter().all(|local| {
					Some(local) == made
						|| !live.contains(local)
						|| !self.points_inside(self.mir.local_decls[local].ty, ty)
				})
		})
	}

	/// Whether a value of `ty` may hold a pointer into a part of a `filled` value, as its
	/// type tells: to a type a `filled` value holds in place.
	fn points_inside(&self, ty: Ty<'tcx>, filled: Ty<'tcx>) -> bool {
		let tcx = self.t.tcx;
		let filled = self.peel(filled);
		let parts = self.inline_parts(filled);
		self.points_to(ty, |pointee| {
			pointee != filled
				&& parts
					.as_ref()
					.is_none_or(|parts| parts.contains(&storage(tcx, pointee)))
		})
	}

	/// Whether a value of `ty` may hold a pointer, or one behind it, to a value `to` holds
	/// for, as what it points at is peeled; or one its type doesn't tell.
	fn points_to(&self, ty: Ty<'tcx>, to: impl Fn(Ty<'tcx>) -> bool) -> bool {
		let mut seen = FxHashSet::default();
		let mut stack = vec![ty];
		while let Some(ty) = stack.pop() {
			if !seen.insert(ty) {
				continue;
			}
			match ty.kind() {
				ty::Ref(_, pointee, _) | ty::RawPtr(pointee, _) => {
					let pointee = self.peel(*pointee);
					if to(pointee) {
						return true;
					}
					stack.push(pointee);
				}
				_ => match self.fields(ty) {
					Some(fields) => stack.extend(fields),
					None => return true,
				},
			}
		}
		false
	}

	/// Whether a value of `ty` may be or hold a `filled` value in place.
	fn holds_in_place(&self, ty: Ty<'tcx>, filled: Ty<'tcx>) -> bool {
		let tcx = self.t.tcx;
		let (ty, filled) = (self.peel(ty), storage(tcx, self.peel(filled)));
		storage(tcx, ty) == filled || self.inline_parts(ty).is_none_or(|parts| parts.contains(&filled))
	}

	/// The types a value of `ty` holds in place, as storage, but `ty` itself; `None` if it
	/// may hold one its type doesn't tell.
	fn inline_parts(&self, ty: Ty<'tcx>) -> Option<FxHashSet<Ty<'tcx>>> {
		let tcx = self.t.tcx;
		let mut parts = FxHashSet::default();
		let mut stack = self.fields(ty)?;
		while let Some(part) = stack.pop() {
			if matches!(part.kind(), ty::Ref(..) | ty::RawPtr(..)) {
				continue;
			}
			let part = self.peel(part);
			if parts.insert(storage(tcx, part)) {
				stack.extend(self.fields(part)?);
			}
		}
		Some(parts)
	}

	/// The types a value of `ty` holds in place, through std's types too; `None` if its type
	/// doesn't tell, as a type parameter's or a trait object's.
	fn fields(&self, ty: Ty<'tcx>) -> Option<Vec<Ty<'tcx>>> {
		Some(match ty.kind() {
			ty::Bool
			| ty::Char
			| ty::Int(_)
			| ty::Uint(_)
			| ty::Float(_)
			| ty::Str
			| ty::Never
			| ty::FnDef(..)
			| ty::FnPtr(..)
			| ty::Ref(..)
			| ty::RawPtr(..) => Vec::new(),
			ty::Array(elem, _) | ty::Slice(elem) | ty::Pat(elem, _) => vec![*elem],
			ty::Tuple(tys) => tys.to_vec(),
			ty::Adt(adt, args) => adt.all_fields().map(|field| self.t.field_ty(field.did, args)).collect(),
			_ => return None,
		})
	}

	/// `ty`, or what it holds if JS keeps that as it: a `MaybeUninit`'s payload.
	fn peel(&self, mut ty: Ty<'tcx>) -> Ty<'tcx> {
		while let ty::Adt(adt, args) = ty.kind()
			&& Std::of(self.t.tcx, adt.did()) == Some(Std::MaybeUninit)
		{
			ty = args.type_at(0);
		}
		ty
	}

	/// Rustc's liveness of the body's locals.
	pub fn liveness(&self) -> ResultsCursor<'x, 'tcx, MaybeLiveLocals> {
		MaybeLiveLocals
			.iterate_to_fixpoint(self.t.tcx, self.mir, None)
			.into_results_cursor(self.mir)
	}

	/// Whether code at `to` may run after code at `from`, or is it.
	fn reaches(&self, from: Location, to: Location) -> bool {
		if from.block == to.block && from.statement_index <= to.statement_index {
			return true;
		}
		let blocks = &self.mir.basic_blocks;
		let mut seen = DenseBitSet::new_empty(blocks.len());
		let mut stack: Vec<_> = blocks[from.block].terminator().successors().collect();
		while let Some(block) = stack.pop() {
			if block == to.block {
				return true;
			}
			if seen.insert(block) {
				stack.extend(blocks[block].terminator().successors());
			}
		}
		false
	}

	/// Whether a pointer `local` holds may point into one of the body's own locals, as
	/// [`Self::origins`] finds where it came from, or can't tell.
	pub fn may_point_into_locals(&self, local: Local) -> bool {
		self.origins(local, None)
			.is_none_or(|sources| sources.iter().any(|source| matches!(source, Source::Local(_))))
	}

	/// Where the pointers `local` may hold, to a `filled` value if known, may have come
	/// from, following copies, casts, pointers made through them, and calls of std or the
	/// crate that return where they point, or of std's that run none of the crate's code,
	/// which may return what their arguments reach, or memory they allocate, and change what
	/// their mutable borrows of a local hold; `None` if from anywhere else, such as memory.
	fn origins(&self, local: Local, filled: Option<Ty<'tcx>>) -> Option<FxIndexSet<Source>> {
		let defs = self.defs();
		let defs = defs.as_ref().expect("defs are found");
		let tcx = self.t.tcx;
		let mut sources = FxIndexSet::default();
		let mut seen = FxHashSet::default();
		// Each local, and whether what it points to may hold the pointers too.
		let mut stack = vec![(local, false)];
		// The arguments of a call of std's, which may reach the pointers it returns.
		let reached = |args: &[Spanned<Operand<'tcx>>], stack: &mut Vec<(Local, bool)>| {
			for arg in args {
				if self.carries(arg.node.ty(&self.mir.local_decls, tcx)) {
					stack.push((direct(arg.node.place()?)?, true));
				}
			}
			Some(())
		};
		while let Some((local, deep)) = stack.pop() {
			if !seen.insert((local, deep)) || deep && !self.carries(self.mir.local_decls[local].ty) {
				continue;
			}
			if defs[local].other {
				return None;
			}
			if local != mir::RETURN_PLACE && local.as_usize() <= self.mir.arg_count {
				sources.insert(Source::Param(local));
			}
			for &lent in &defs[local].lent {
				for args in self.lent_to(lent)? {
					// What std's code puts there may be memory it allocated, as `Vec::push` does.
					sources.insert(Source::Fresh(local));
					reached(args, &mut stack)?;
				}
			}
			for def in &defs[local].whole {
				match *def {
					Def::Assign(
						Rvalue::Use(Operand::Constant(_), _)
						| Rvalue::Cast(_, Operand::Constant(_), _)
						| Rvalue::ThreadLocalRef(_),
					) => _ = sources.insert(Source::Static),
					Def::Assign(Rvalue::Use(op, _) | Rvalue::Cast(_, op, _)) => {
						stack.push((direct(op.place()?)?, deep))
					}
					Def::Assign(Rvalue::CopyForDeref(place)) => stack.push((direct(*place)?, deep)),
					Def::Assign(Rvalue::Ref(_, _, place) | Rvalue::RawPtr(_, place)) => {
						match place.projection.split_first() {
							Some((ProjectionElem::Deref, rest)) if !rest.contains(&ProjectionElem::Deref) => {
								stack.push((place.local, deep))
							}
							_ if !place.is_indirect() => {
								// What a local reaches may point into it only if it can hold one.
								if !deep
									|| filled
										.is_none_or(|f| self.holds_in_place(self.mir.local_decls[place.local].ty, f))
								{
									sources.insert(Source::Local(place.local));
								}
								if deep {
									stack.push((place.local, true));
								}
							}
							_ => return None,
						}
					}
					Def::Assign(_) => return None,
					Def::Call(func, args) => {
						let ty::FnDef(def_id, gargs) = *func.ty(&self.mir.local_decls, tcx).kind() else {
							return None;
						};
						if pointer::allocates(tcx, def_id) {
							sources.insert(Source::Fresh(local));
						} else if let Some(derives) = pointer::derives(tcx, def_id) {
							if derives == Derives::Loaded {
								return None;
							}
							for arg in args {
								if self.carries(arg.node.ty(&self.mir.local_decls, tcx)) {
									stack.push((direct(arg.node.place()?)?, deep));
								}
							}
						} else if let Some(did) = self.read_fn(def_id, gargs.skip_binder()) {
							for source in self.returned(did)? {
								match source {
									Source::Param(param) => {
										stack.push((direct(args[param.as_usize() - 1].node.place()?)?, deep))
									}
									_ => _ = sources.insert(source),
								}
							}
						} else if self.runs_std(def_id, gargs.skip_binder(), args) {
							sources.insert(Source::Fresh(local));
							reached(args, &mut stack)?;
						} else {
							return None;
						}
					}
				}
			}
		}
		Some(sources)
	}

	/// The arguments of each call `lent`, a mutable borrow of a local, is given to; `None` if
	/// it is used anywhere else, or given to a call that may run the crate's code or keep it
	/// where code could change the local through it after: in another argument or in what
	/// the call returns.
	fn lent_to(&self, lent: Local) -> Option<Vec<&'x [Spanned<Operand<'tcx>>]>> {
		let (mir, tcx) = (self.mir, self.t.tcx);
		let ty = mir.local_decls[lent].ty;
		let borrowed = ty.builtin_deref(true)?;
		let mut calls = Vec::new();
		let mut given = 0;
		for data in mir.basic_blocks.iter() {
			if let TerminatorKind::Call {
				func,
				args,
				destination,
				..
			} = &data.terminator().kind
			{
				let n = args
					.iter()
					.filter(|a| a.node.place().and_then(|p| p.as_local()) == Some(lent))
					.count();
				if n > 0 {
					let ty::FnDef(def_id, gargs) = *func.ty(&mir.local_decls, tcx).kind() else {
						return None;
					};
					let returned = destination.ty(&mir.local_decls, tcx).ty;
					if pointer::derives(tcx, def_id).is_some()
						|| !self.leaves(def_id, gargs.skip_binder(), args, &[ty])
						|| self.points_to(returned, |pointee| self.holds_in_place(pointee, borrowed))
					{
						return None;
					}
					given += n;
					calls.push(&args[..]);
				}
			}
		}
		let mut uses = Uses { local: lent, count: 0 };
		uses.visit_body(mir);
		(uses.count == given).then_some(calls)
	}

	/// The body's [`Defs`], found the first time.
	fn defs(&self) -> std::cell::Ref<'_, Option<IndexVec<Local, Defs<'x, 'tcx>>>> {
		if self.defs.borrow().is_none() {
			let mir = self.mir;
			let mut defs: IndexVec<Local, Defs<'x, 'tcx>> =
				IndexVec::from_fn_n(|_| Defs::default(), mir.local_decls.len());
			let assigned = |to: &Place<'tcx>, def: Def<'x, 'tcx>, defs: &mut IndexVec<Local, Defs<'x, 'tcx>>| match to
				.as_local()
			{
				Some(local) => defs[local].whole.push(def),
				None if !to.is_indirect() => defs[to.local].other = true,
				None => {}
			};
			for data in mir.basic_blocks.iter() {
				for stmt in &data.statements {
					match &stmt.kind {
						StatementKind::Assign(assign) => {
							let (to, rvalue) = &**assign;
							assigned(to, Def::Assign(rvalue), &mut defs);
							// A local something borrows may change through the borrow, which
							// `origins` follows where it is a local's mutable borrow of all of it.
							match rvalue {
								Rvalue::Ref(_, BorrowKind::Mut { .. }, place)
								| Rvalue::RawPtr(RawPtrKind::Mut, place)
									if let Some(borrowed) = place.as_local()
										&& let Some(to) = to.as_local() =>
								{
									defs[borrowed].lent.push(to)
								}
								Rvalue::Ref(_, _, place) | Rvalue::RawPtr(_, place) if !place.is_indirect() => {
									defs[place.local].other = true
								}
								_ => {}
							}
						}
						StatementKind::SetDiscriminant { place, .. } if !place.is_indirect() => {
							defs[place.local].other = true
						}
						_ => {}
					}
				}
				match &data.terminator().kind {
					TerminatorKind::Call {
						func,
						args,
						destination,
						..
					} => assigned(destination, Def::Call(func, args), &mut defs),
					TerminatorKind::InlineAsm { .. }
					| TerminatorKind::Yield { .. }
					| TerminatorKind::TailCall { .. } => defs.iter_mut().for_each(|d| d.other = true),
					_ => {}
				}
			}
			*self.defs.borrow_mut() = Some(defs);
		}
		self.defs.borrow()
	}
}

/// What a local of `rvalue`, which reads one of the holders, holds pointers to: what a
/// holder it copies or casts does, or a field of it such as an `Option`'s payload; what a
/// reference or pointer made of one or through it does; or an element of that, by index.
fn level(rvalue: &Rvalue<'_>, held: &Held) -> Level {
	match rvalue {
		Rvalue::Use(op, _) | Rvalue::Cast(_, op, _) => match op.place() {
			Some(p) if !p.is_indirect() => held.of(p.local),
			_ => Level::Part,
		},
		Rvalue::CopyForDeref(place) if !place.is_indirect() => held.of(place.local),
		Rvalue::Ref(_, _, place) | Rvalue::RawPtr(_, place) => {
			let (base, rest) = match place.projection.split_first() {
				Some((ProjectionElem::Deref, rest)) => (held.of(place.local), rest),
				_ => (held.at(place.local), place.projection.as_slice()),
			};
			let index = |elem: &ProjectionElem<_, _>| {
				matches!(
					elem,
					ProjectionElem::Index(_) | ProjectionElem::ConstantIndex { .. } | ProjectionElem::Subslice { .. }
				)
			};
			match rest {
				[] => base,
				_ if rest.iter().all(index) => base.max(Level::Elems),
				_ => Level::Part,
			}
		}
		_ => Level::Part,
	}
}

impl Held {
	/// What the pointers `local` holds point at; any read of the value the pointers are made
	/// of is a part of it.
	fn of(&self, local: Local) -> Level {
		match self.value == Some(local) {
			true => Level::Part,
			false => self.at(local),
		}
	}

	/// What a pointer to `local` itself points at, of the value the pointers are made of:
	/// its whole, if it is that value.
	fn at(&self, local: Local) -> Level {
		if self.parts.contains(local) {
			Level::Part
		} else if self.elems.contains(local) {
			Level::Elems
		} else {
			Level::Whole
		}
	}
}

/// Whether a bound of `trait_id` lets code call nothing: it and its supertraits have no
/// methods, as `Sized` and `Send`.
fn calls_nothing(tcx: ty::TyCtxt<'_>, trait_id: DefId) -> bool {
	tcx.supertrait_def_ids(trait_id).all(|t| {
		tcx.associated_items(t)
			.in_definition_order()
			.all(|item| !matches!(item.kind, ty::AssocKind::Fn { .. }))
	})
}

/// `ty` as storage, which an array shares with a slice of its element.
fn storage<'tcx>(tcx: ty::TyCtxt<'tcx>, ty: Ty<'tcx>) -> Ty<'tcx> {
	match ty.kind() {
		ty::Array(elem, _) => Ty::new_slice(tcx, *elem),
		_ => ty,
	}
}

/// The local `place` is, read as a whole or through fields of it, with no pointer on its
/// path.
fn direct(place: Place<'_>) -> Option<Local> {
	(!place.is_indirect()).then_some(place.local)
}

/// Whether `op` reads one of `holders`.
fn reads_operand(op: &Operand<'_>, holders: &DenseBitSet<Local>) -> bool {
	op.place().is_some_and(|p| holders.contains(p.local))
}

/// Whether what `visit` visits reads one of `holders`.
fn reads(holders: &DenseBitSet<Local>, visit: impl FnOnce(&mut Reads<'_>)) -> bool {
	let mut reads = Reads { holders, found: false };
	visit(&mut reads);
	reads.found
}

/// How many times what it visits uses `local`, but to assign all of it.
struct Uses {
	local: Local,
	count: usize,
}

impl<'tcx> Visitor<'tcx> for Uses {
	fn visit_local(&mut self, local: Local, context: PlaceContext, _: Location) {
		use rustc_middle::mir::visit::MutatingUseContext;
		if local == self.local
			&& !matches!(
				context,
				PlaceContext::NonUse(_)
					| PlaceContext::MutatingUse(MutatingUseContext::Store | MutatingUseContext::Call)
			) {
			self.count += 1;
		}
	}
}

/// Whether what it visits reads one of the holders.
struct Reads<'a> {
	holders: &'a DenseBitSet<Local>,
	found: bool,
}

impl<'tcx> Visitor<'tcx> for Reads<'_> {
	fn visit_local(&mut self, local: Local, _: PlaceContext, _: Location) {
		self.found |= self.holders.contains(local);
	}
}
