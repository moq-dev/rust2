//! Function bodies, from MIR.
//!
//! MIR is each body's control-flow graph after borrow checking. Every drop is a
//! terminator where drop elaboration put it, with a flag wherever a value is moved on
//! some paths only, and an async body is already a state machine, its resume function
//! and its drop. Blocks become structured JS (`cfg`). Statements become JS statements,
//! with each temporary that is assigned once and read once folded back into the
//! expression that reads it, unless that would reorder an effect.
//!
//! Library calls go through the mappings in `library`, which take their arguments
//! lowered: a `Body` holds the names, and its translator everything else.

mod call;
mod cfg;
pub mod coroutine;
mod decided;
pub mod glue;
pub(super) mod holders;
mod inline;
mod memory;
mod place;
mod rvalue;
mod scalar;
mod step;
mod vec;

use rustc_data_structures::fx::{FxHashMap, FxHashSet};
use rustc_hir::def::DefKind;
use rustc_index::IndexVec;
use rustc_middle::mir::{self, BasicBlock, Local, Operand, Place, Rvalue, StatementKind, TerminatorKind};
use rustc_middle::ty::{self, EarlyBinder, Instance, Ty, TyCtxt};
use rustc_span::{Span, Spanned};

use self::cfg::Exit;
pub use self::place::{slice_member, writes_captures};
use super::Translator;
use super::body::Body;

use crate::ts::{self, Expr, Param, Stmt, StmtKind, Type};

/// Translate one function instance's body from its MIR.
pub fn function<'a, 'tcx>(t: &mut Translator<'a, 'tcx>, instance: Instance<'tcx>) -> Option<ts::Function> {
	let tcx = t.tcx;
	let def_id = instance.def_id();
	let span = tcx.def_span(def_id);
	let mir = instance_mir(t, instance, span)?;
	let next = super::iter::is_next(tcx, def_id);
	if next {
		super::iter::check_impl(t, def_id);
	}
	let ret_ty = mir.return_ty();
	let mut b = Body::new(t);
	let mut fx = Fx::new(&mut b, &mir, Env::Plain, span);
	let params = fx.params();
	let body = fx.lower();
	let ret_span = fx.local_span(mir::RETURN_PLACE);
	let ret = match (next, ret_ty.kind()) {
		(true, ty::Adt(_, args)) => {
			Type::Union(vec![fx.b.t.ty(args.type_at(0), ret_span), Type::Rt("Done", Vec::new())])
		}
		_ => fx.b.t.ret_ty(ret_ty, ret_span),
	};
	Some(ts::Function {
		sym: 0,
		params,
		ret,
		body,
		loc: None,
		doc: None,
	})
}

/// The MIR of `instance`, with its generic arguments in place.
fn instance_mir<'tcx>(t: &mut Translator<'_, 'tcx>, instance: Instance<'tcx>, span: Span) -> Option<mir::Body<'tcx>> {
	let tcx = t.tcx;
	let def_id = instance.def_id();
	t.lowered.insert(instance);
	// A `const`'s, as a function of no arguments that makes its value.
	let body = match tcx.def_kind(def_id) {
		DefKind::Const => tcx.mir_for_ctfe(def_id).clone(),
		_ if !tcx.is_mir_available(def_id) => {
			t.err(span, format!("no MIR for `{}`", tcx.def_path_str(def_id)));
			return None;
		}
		_ => tcx.optimized_mir(def_id).clone(),
	};
	let mut body = match instance.try_instantiate_mir_and_normalize_erasing_regions(
		tcx,
		t.typing_env_pub(),
		EarlyBinder::bind(tcx, body),
	) {
		Ok(body) => body,
		Err(err) => {
			t.err(
				span,
				format!(
					"`{instance}` names `{}`, which does not normalize",
					err.get_type_for_failure()
				),
			);
			return None;
		}
	};
	vec::literals(tcx, &mut body);
	if !t.crates.translates(def_id)
		&& let Some(why) = std_refused(tcx, &body)
	{
		t.err(span, why);
		return None;
	}
	decided::fold(tcx, &t.crates, &mut body);
	same_switches(&mut body);
	Some(body)
}

/// Each switch whose targets all go on to one block, past blocks that only mark where
/// locals' storage begins or ends, made a jump there, as rustc's own simplification makes
/// it where no markers stand between.
fn same_switches(body: &mut mir::Body<'_>) {
	let blocks = &body.basic_blocks;
	let past = |mut bb: BasicBlock| {
		for _ in 0..blocks.len() {
			let data = &blocks[bb];
			match data.terminator().kind {
				TerminatorKind::Goto { target, .. }
					if !data.is_cleanup
						&& data.statements.iter().all(|s| {
							matches!(
								s.kind,
								StatementKind::StorageLive(_) | StatementKind::StorageDead(_) | StatementKind::Nop
							)
						}) =>
				{
					bb = target
				}
				_ => break,
			}
		}
		bb
	};
	let same: Vec<(BasicBlock, BasicBlock)> = blocks
		.iter_enumerated()
		.filter_map(|(bb, data)| {
			let TerminatorKind::SwitchInt { targets, .. } = &data.terminator().kind else {
				return None;
			};
			let first = targets.all_targets()[0];
			targets
				.all_targets()
				.iter()
				.all(|&t| past(t) == past(first))
				.then_some((bb, first))
		})
		.collect();
	for (bb, target) in same {
		body.basic_blocks_mut()[bb].terminator_mut().kind = TerminatorKind::Goto {
			target,
			loop_hint_attrs: Default::default(),
		};
	}
}

/// Why a body of std's can't lower as the crate's do, if it can't. The crate's analysis of
/// what it writes into reads only its own bodies, so one must change nothing in place: no
/// assignment through a projection or a reference, and no mutable borrow or pointer but of
/// a function to call it, as `FnMut` is, whose writes are its own body's. And core's plain
/// integer arithmetic wraps, as `wrapping_add` lowers, where the crate's is checked.
fn std_refused<'tcx>(tcx: TyCtxt<'tcx>, body: &mir::Body<'tcx>) -> Option<&'static str> {
	const WRITE: &str = "a change in place, which a body of std's is not supported to make";
	const WRAP: &str = "arithmetic that wraps, which a body of std's is not supported to do";
	for bb in body.basic_blocks.iter() {
		for stmt in &bb.statements {
			match &stmt.kind {
				StatementKind::Assign(assign) if !assign.0.projection.is_empty() => return Some(WRITE),
				StatementKind::Assign(assign) => match &assign.1 {
					Rvalue::Ref(_, mir::BorrowKind::Mut { .. }, place)
						if place.projection.is_empty()
							&& matches!(
								body.local_decls[place.local].ty.kind(),
								ty::Closure(..) | ty::FnDef(..) | ty::FnPtr(..)
							) => {}
					Rvalue::Ref(_, mir::BorrowKind::Mut { .. }, _) | Rvalue::RawPtr(mir::RawPtrKind::Mut, _) => {
						return Some(WRITE);
					}
					Rvalue::BinaryOp(mir::BinOp::Add | mir::BinOp::Sub | mir::BinOp::Mul, operands)
						if operands.0.ty(body, tcx).is_integral() =>
					{
						return Some(WRAP);
					}
					_ => {}
				},
				StatementKind::SetDiscriminant { .. } => return Some(WRITE),
				_ => {}
			}
		}
		if let TerminatorKind::Call { destination, .. } = &bb.terminator().kind
			&& !destination.projection.is_empty()
		{
			return Some(WRITE);
		}
	}
	None
}

/// What `_1` is.
#[derive(Clone)]
pub enum Env {
	/// A parameter like any other.
	Plain,
	/// A closure's environment, whose fields are what it captured.
	Closure(Vec<Capture>),
	/// A coroutine's state, behind `Pin<&mut Self>` or `&mut Self`: `this` of its class.
	Coroutine,
	/// Drop glue, whose `_1` is a pointer to the value: the value in JS.
	Shim,
}

/// What a closure captured.
#[derive(Clone)]
pub struct Capture {
	pub value: Expr,
	/// Whether `value` is the place a `&mut` captured by reference borrows, which the
	/// closure reads and writes as JS's do, rather than an `rt.Place` of it or the object
	/// there.
	pub pointee: bool,
}

/// How a MIR local lowers.
#[derive(Clone, Debug)]
enum Slot<'tcx> {
	/// No JS value: `()`, a function item, or another zero-sized value.
	Unit,
	/// A JS variable.
	Var(String),
	/// A reference assigned once, read as the place it borrows, which is set once its
	/// assignment is lowered.
	Alias(Option<Expr>),
	/// A discriminant only a `switchInt` reads: tested as the place's variant.
	Discr(Option<(Expr, Ty<'tcx>)>),
	/// A discriminant only a `switchInt` reads, of a local held as its parts: its flag,
	/// whether it is its second variant.
	Flag(Option<Expr>),
}

/// A temporary's value, held back to fold into the one expression that reads it.
struct Pending {
	name: String,
	value: Expr,
	/// In the order assignments ran.
	order: u32,
	/// Whether evaluating it can do or throw anything.
	effect: bool,
	/// Whether it reads memory something else could change.
	memory: bool,
	/// The aggregate's operands, for a call that takes them apart, each with whether it is
	/// a reference that reads the same wherever it is read.
	parts: Option<Vec<(Expr, bool)>>,
	/// The names it reads, which an assignment to must not pass it.
	reads: Vec<String>,
	/// Whether it is a `Copy` aggregate read from where it could change, which only an
	/// argument may share: emitted on its own, it is copied.
	copy: bool,
	/// Whether it is an aggregate's part, with no variable of its own until emitted.
	decl: bool,
	/// Whether reading it again, wherever and however often, reads the same.
	rereads: bool,
}

pub struct Fx<'m, 'b, 'a, 'tcx> {
	pub b: &'m mut Body<'b, 'a, 'tcx>,
	pub mir: &'m mir::Body<'tcx>,
	pub tcx: TyCtxt<'tcx>,
	env: Env,
	slots: IndexVec<Local, Slot<'tcx>>,
	/// Where each local assigned once is assigned, by a statement.
	def_at: IndexVec<Local, Option<(BasicBlock, usize)>>,
	/// How many times each local is read, and assigned whole.
	uses: IndexVec<Local, u32>,
	defs: IndexVec<Local, u32>,
	/// Locals something borrows, which a call or closure can change behind a read.
	borrowed: IndexVec<Local, bool>,
	/// Locals changed in place: assigned through a projection, borrowed mutably, or moved,
	/// holding a `Copy` aggregate, into what can change it later: a field, a local changed
	/// in place, or a call that `keeps` it. The return place is one when its caller has no
	/// copy to make. Each holds its own JS objects, which a move hands over uncopied.
	mutated: IndexVec<Local, bool>,
	/// The names of borrowed locals.
	shared: FxHashSet<String>,
	/// The names of locals whose own value nothing changes: a tuple, array, or enum or
	/// struct of their own, never changed in place nor borrowed, so a call can't change
	/// what their fields hold.
	frozen_names: FxHashSet<String>,
	/// `(value, overflowed)` pairs of checked arithmetic, whose `.1` is false once the value
	/// is made, which panics where the assert would.
	checked: FxHashSet<Local>,
	/// The temporaries held back, in order.
	pending: Vec<Pending>,
	order: u32,
	/// Locals some statement assigns, which need a declaration.
	assigned: IndexVec<Local, bool>,
	span: Span,
	pub labels: u32,
	/// Whether this is an `Iterator::next`, which returns an item or `rt.DONE`.
	next: bool,
	/// Blocks whose only statement is `unreachable`.
	unreachable: FxHashSet<BasicBlock>,
	/// For the aggregate lowered last, whether each operand is a reference that reads the
	/// same wherever it is read.
	rereadable_parts: Option<Vec<bool>>,
	/// Blocks that start an ignored macro's code, and the block after it.
	redirect: FxHashMap<BasicBlock, BasicBlock>,
	/// Whether the operands lowered now go to a local nothing changes in place.
	into_frozen: bool,
	/// Whether the value held back next is copied unless it folds into an argument.
	copy_on_flush: bool,
	/// Whether a copy lowered now shares what it copies, whatever it reads from.
	force_share: bool,
	/// Locals whose one read is as a call's argument, which may share what it copies.
	arg_use: IndexVec<Local, bool>,
	/// Locals whose one read is as an argument to a call given nothing that changes memory.
	arg_alone: IndexVec<Local, bool>,
	/// Locals whose one read is as an argument to a call whose result could hold it.
	arg_kept: IndexVec<Local, bool>,
	/// Where each left-out macro invocation in the body is written.
	ignored_sites: Vec<Span>,
	/// The operands of aggregates assigned once to locals nothing changes.
	var_parts: FxHashMap<Local, Vec<Expr>>,
	/// Locals holding what `Iterator::next` returns by the protocol: an item or `rt.DONE`.
	next_results: FxHashSet<Local>,
	/// Their names, which a switch on their variant tests against `rt.DONE`.
	next_names: FxHashSet<String>,
	/// Next results of a range of numbers stepped in place: each holds only the item, so
	/// JS keeps it a number, and a flag holds whether there was one.
	range_nexts: FxHashSet<Local>,
	/// Their names, and their flags' names, which a switch on their variant tests.
	range_flags: FxHashMap<String, String>,
	/// Iterators over an array stepped in place, by `step`.
	stepped: FxHashSet<Local>,
	/// The borrows `next` steps them by, and what each borrows.
	stepped_borrows: FxHashMap<Local, Local>,
	/// References a call returns to what its `&mut` argument borrows, as `Cow::to_mut`'s:
	/// the place each reads and writes, which a store through one rebuilds as a `Ref`'s.
	reborrows: FxHashMap<Local, Place<'tcx>>,
	/// The two variables each holds its fields in, by `step`.
	stepped_vars: FxHashMap<Local, [String; 2]>,
	/// Those moved into another, which shares its variables.
	stepped_into: FxHashMap<Local, Local>,
	/// Statements that change nothing: a swap through a temporary nothing else names, as
	/// an async body moves its context at each `await`.
	noops: FxHashSet<(BasicBlock, usize)>,
	/// The lengths a bounds check compares with, of the slices whose `rt.idx` it is.
	idx_of: FxHashMap<Local, Place<'tcx>>,
	/// The halves a `|` joins into a `u64`, by the local of the shift that is its left side.
	halves: FxHashMap<Local, (&'m Operand<'tcx>, &'m Operand<'tcx>)>,
	/// Temporaries of those joins, whose assignments go unlowered.
	joined: FxHashSet<Local>,
	/// The paths a switch lowered so far tests the variant of, with the enum it tests,
	/// which `tsc` narrows in its arms. An `Option`'s `Some` is the path itself, so it
	/// may be tested as both.
	switched: FxHashSet<(String, Ty<'tcx>)>,
	/// Locals whose variant some switch tests, or that std takes, directly or through a
	/// reference to them.
	tested: IndexVec<Local, bool>,
	/// The names of those that hold a value TypeScript would narrow by what it is assigned.
	widened: FxHashSet<String>,
	/// Locals held as their parts, by `scalar`.
	scalars: FxHashMap<Local, scalar::Scalar<'tcx>>,
	/// Names nothing assigns again that `scalar` reads in place: variables of parts assigned
	/// once, and the slices it took apart.
	steady: FxHashSet<String>,
	/// Those slices, with their types, which need declarations.
	scalar_binds: Vec<(String, Type)>,
	/// The tests and payload reads TypeScript may have narrowed stale.
	narrowing: decided::Narrowing<'tcx>,
	/// The statement or terminator being lowered.
	at: mir::Location,
}

impl<'m, 'b, 'a, 'tcx> Fx<'m, 'b, 'a, 'tcx> {
	pub fn new(b: &'m mut Body<'b, 'a, 'tcx>, mir: &'m mir::Body<'tcx>, env: Env, span: Span) -> Self {
		let tcx = b.t.tcx;
		let n = mir.local_decls.len();
		let narrowing = decided::narrowing(tcx, &b.t.crates, mir);
		let mut fx = Fx {
			b,
			mir,
			tcx,
			env,
			slots: IndexVec::from_elem_n(Slot::Unit, n),
			def_at: IndexVec::from_elem_n(None, n),
			uses: IndexVec::from_elem_n(0, n),
			defs: IndexVec::from_elem_n(0, n),
			borrowed: IndexVec::from_elem_n(false, n),
			mutated: IndexVec::from_elem_n(false, n),
			shared: FxHashSet::default(),
			frozen_names: FxHashSet::default(),
			checked: FxHashSet::default(),
			pending: Vec::new(),
			order: 0,
			assigned: IndexVec::from_elem_n(false, n),
			span,
			labels: 0,
			next: crate::lower::iter::is_next(tcx, mir.source.def_id()),
			unreachable: FxHashSet::default(),
			rereadable_parts: None,
			redirect: FxHashMap::default(),
			into_frozen: false,
			copy_on_flush: false,
			force_share: false,
			arg_use: IndexVec::from_elem_n(false, n),
			arg_alone: IndexVec::from_elem_n(false, n),
			arg_kept: IndexVec::from_elem_n(false, n),
			ignored_sites: Vec::new(),
			var_parts: FxHashMap::default(),
			next_results: FxHashSet::default(),
			next_names: FxHashSet::default(),
			range_nexts: FxHashSet::default(),
			range_flags: FxHashMap::default(),
			stepped: FxHashSet::default(),
			stepped_borrows: FxHashMap::default(),
			reborrows: FxHashMap::default(),
			stepped_vars: FxHashMap::default(),
			stepped_into: FxHashMap::default(),
			noops: FxHashSet::default(),
			idx_of: FxHashMap::default(),
			halves: FxHashMap::default(),
			joined: FxHashSet::default(),
			switched: FxHashSet::default(),
			tested: IndexVec::from_elem_n(false, n),
			widened: FxHashSet::default(),
			scalars: FxHashMap::default(),
			steady: FxHashSet::default(),
			scalar_binds: Vec::new(),
			narrowing,
			at: mir::START_BLOCK.start_location(),
		};
		fx.analyze();
		fx
	}

	pub fn err(&mut self, span: Span, msg: impl Into<String>) -> Expr {
		self.b.t.err(span, msg);
		Expr::undefined()
	}

	/// Count each local's reads and assignments, and decide how each lowers.
	fn analyze(&mut self) {
		let mir = self.mir;
		self.find_ignored_sites();
		self.find_noops();
		let mut bounds = Vec::new();
		// Locals whose value moves whole into another's, `(to, from)`.
		let mut moves = Vec::new();
		for (bb, data) in mir.basic_blocks.iter_enumerated() {
			if data.statements.iter().all(|s| {
				matches!(
					s.kind,
					StatementKind::StorageDead(_) | StatementKind::StorageLive(_) | StatementKind::Nop
				)
			}) && matches!(data.terminator().kind, TerminatorKind::Unreachable)
			{
				self.unreachable.insert(bb);
			}
			// A left-out macro's code reads nothing, so a parameter only it reads is unused.
			for (i, stmt) in data.statements.iter().enumerate() {
				if self.noops.contains(&(bb, i)) || self.in_ignored(stmt.source_info.span) {
					continue;
				}
				match &stmt.kind {
					StatementKind::Assign(assign) => {
						let (place, rvalue) = &**assign;
						if let Some(local) = place.as_local() {
							self.def_at[local] = Some((bb, i));
						}
						self.def(*place);
						self.rvalue_uses(rvalue);
						let ops = match rvalue {
							Rvalue::Use(op, _) => std::slice::from_ref(op),
							Rvalue::Aggregate(_, ops) => &ops.raw[..],
							_ => &[],
						};
						let moved: Vec<Local> = ops.iter().filter_map(|op| self.moved_from(op)).collect();
						for from in moved {
							match place.as_local() {
								Some(to) => moves.push((to, from)),
								// Into a field or through a reference, which changes in place.
								None => self.mutated[from] = true,
							}
						}
						if let Rvalue::BinaryOp(op, _) = rvalue
							&& op.overflowing_to_wrapping().is_some()
							&& let Some(local) = place.as_local()
						{
							self.checked.insert(local);
						}
					}
					StatementKind::SetDiscriminant { place, .. } => {
						self.place_use(**place);
						self.mutated[place.local] = true;
					}
					_ => {}
				}
			}
			let term = data.terminator();
			if !data.is_cleanup && self.in_ignored(term.source_info.span) {
				continue;
			}
			match &term.kind {
				TerminatorKind::SwitchInt { discr, .. } => self.operand_use(discr),
				TerminatorKind::Return => {
					self.uses[mir::RETURN_PLACE] += 1;
					// The caller can change what it holds, and has no copy to make.
					if self.moves_copied(mir.local_decls[mir::RETURN_PLACE].ty) {
						self.mutated[mir::RETURN_PLACE] = true;
					}
				}
				// A drop JS can't see reads nothing.
				TerminatorKind::Drop { place, .. } => {
					if self.b.t.drops(place.ty(&mir.local_decls, self.tcx).ty) {
						self.place_use(*place);
					}
				}
				TerminatorKind::Call {
					func,
					args,
					destination,
					..
				} => {
					self.operand_use(func);
					let result = destination.ty(&mir.local_decls, self.tcx).ty;
					let translated = self.translated(func);
					for arg in args.iter() {
						self.operand_use(&arg.node);
						let (Operand::Copy(p) | Operand::Move(p)) = &arg.node else {
							continue;
						};
						let part = p.ty(&mir.local_decls, self.tcx).ty;
						let kept = self.keeps(translated, args, part, result);
						if kept && let Some(from) = self.moved_from(&arg.node) {
							self.mutated[from] = true;
						}
						if let Some(local) = p.as_local() {
							// A callee given nothing that can change what the argument is can't
							// change its source while it runs.
							let alone = !std::iter::once(func)
								.chain(args.iter().map(|a| &a.node))
								.any(|op| self.changes(op.ty(&mir.local_decls, self.tcx), part));
							self.arg_use[local] = true;
							self.arg_alone[local] = alone;
							self.arg_kept[local] = kept || self.b.t.holds(result, part);
						}
					}
					self.def(*destination);
				}
				TerminatorKind::Assert { cond, msg, .. } => {
					// The assert after checked arithmetic reads `.1`, which lowering folds away.
					let folded = matches!(cond, Operand::Move(p) | Operand::Copy(p)
						if p.projection.len() == 1 && self.checked.contains(&p.local));
					if let mir::AssertKind::BoundsCheck { len, index } = &**msg {
						// Counted once every length's definition is known.
						bounds.push((cond, len, index));
					} else if !folded && !redundant(msg) {
						self.operand_use(cond);
					}
				}
				_ => {}
			}
		}
		// What moves into a local changed in place must be its source's own too.
		let mut changed = true;
		while std::mem::take(&mut changed) {
			for &(to, from) in &moves {
				if self.mutated[to] && !self.mutated[from] {
					self.mutated[from] = true;
					changed = true;
				}
			}
		}
		// A bounds check of a slice's length is `rt.idx` of the slice; others test their
		// condition, and show their operands.
		for (cond, len, index) in bounds {
			if let Operand::Copy(l) | Operand::Move(l) = len
				&& let Some(l) = l.as_local()
				&& let Some(slice) = self.measured(l)
				&& self.stable(slice)
			{
				self.idx_of.insert(l, slice);
				self.place_use(slice);
				self.operand_use(index);
				continue;
			}
			self.operand_use(cond);
			self.operand_use(len);
			self.operand_use(index);
		}
		self.drop_unread();
		self.find_next_results();
		self.find_stepped();
		self.unborrow_passed();
		self.find_halves();
		self.find_scalars();
		for local in mir.local_decls.indices() {
			let ty = mir.local_decls[local].ty;
			let slot = if self.zero_sized(ty) {
				Slot::Unit
			} else if self.scalars.contains_key(&local) {
				Slot::Var(String::new())
			} else if let Some(value) = self.unassigned(local, ty) {
				Slot::Alias(Some(value))
			} else if self.alias(local) && !self.reads_scalar_object(local) {
				Slot::Alias(None)
			} else if self.discr_only(local) && self.scalar_discr(local) {
				Slot::Flag(None)
			} else if self.discr_only(local) {
				Slot::Discr(None)
			} else {
				Slot::Var(String::new())
			};
			self.slots[local] = slot;
			// A temporary folded into what reads it is declared nowhere, so a type that can't
			// tell its values apart is refused here. The signature types the parameters and
			// the result, and a left-out macro's code holds nothing.
			if !self.is_arg(local) && local != mir::RETURN_PLACE && !self.in_ignored(self.local_span(local)) {
				let held = match self.next_results.contains(&local) {
					true => next_item(self.tcx, ty),
					false => ty,
				};
				self.b.t.check_erased(held, self.local_span(local));
			}
		}
		self.find_tested();
		// Names: what debug info calls a local, else its number.
		let mut named: FxHashMap<Local, rustc_span::Symbol> = FxHashMap::default();
		for info in &mir.var_debug_info {
			if let mir::VarDebugInfoContents::Place(place) = &info.value
				&& let Some(local) = place.as_local()
				&& info.composite.is_none()
			{
				named.entry(local).or_insert(info.name);
			}
		}
		for local in mir.local_decls.indices() {
			if let Slot::Var(_) = self.slots[local] {
				let base = match named.get(&local) {
					Some(name) => super::camel(name.as_str()),
					None => format!("_{}", local.as_u32()),
				};
				let name = self.b.fresh(&base);
				if self.borrowed[local] {
					self.shared.insert(name.clone());
				}
				if self.stepped.contains(&local) {
					self.name_stepped(local, &base);
				}
				self.name_scalar(local, &base);
				let owned = match mir.local_decls[local].ty.kind() {
					ty::Tuple(_) | ty::Array(..) => true,
					ty::Adt(adt, _) => {
						self.b.t.own(adt.did())
							|| matches!(
								crate::lower::library::Std::of(self.tcx, adt.did()),
								Some(crate::lower::library::Std::Option | crate::lower::library::Std::Result)
							)
					}
					_ => false,
				};
				// A byte slice's fields are readonly, so its parts are frozen even in a parameter.
				let bytes = matches!(mir.local_decls[local].ty.kind(), ty::Ref(_, inner, ty::Mutability::Not) if place::byte_slice(*inner));
				if (bytes || owned && !self.is_arg(local)) && !self.borrowed[local] && !self.mutated[local] {
					self.frozen_names.insert(name.clone());
				}
				self.slots[local] = Slot::Var(name);
			}
		}
		self.share_stepped();
	}

	/// Uncount the reads of what computes a local nothing reads without doing anything
	/// else, as the test of an assert a runtime function checks for itself is.
	fn drop_unread(&mut self) {
		let mir = self.mir;
		let mut computed: IndexVec<Local, Vec<&Rvalue<'tcx>>> =
			IndexVec::from_elem_n(Vec::new(), mir.local_decls.len());
		let mut pure = IndexVec::from_elem_n(true, mir.local_decls.len());
		for (bb, data) in mir.basic_blocks.iter_enumerated() {
			for (i, stmt) in data.statements.iter().enumerate() {
				if self.noops.contains(&(bb, i)) || self.in_ignored(stmt.source_info.span) {
					continue;
				}
				if let StatementKind::Assign(assign) = &stmt.kind
					&& let Some(local) = assign.0.as_local()
				{
					match &assign.1 {
						// One that can throw is lowered even unread, so it keeps its reads.
						r @ (Rvalue::BinaryOp(..)
						| Rvalue::UnaryOp(..)
						| Rvalue::Use(..)
						| Rvalue::RawPtr(mir::RawPtrKind::FakeForPtrMetadata, _))
							if !rvalue::effect(r, self) =>
						{
							computed[local].push(r)
						}
						_ => pure[local] = false,
					}
				}
			}
			if let TerminatorKind::Call { destination, .. } = &data.terminator().kind {
				pure[destination.local] = false;
			}
		}
		let mut work: Vec<Local> = mir.local_decls.indices().collect();
		while let Some(local) = work.pop() {
			if self.uses[local] != 0 || !pure[local] || self.is_arg(local) || local == mir::RETURN_PLACE {
				continue;
			}
			for rvalue in std::mem::take(&mut computed[local]) {
				let read: Vec<Place<'tcx>> = match rvalue {
					Rvalue::BinaryOp(_, ops) => [&ops.0, &ops.1].into_iter().filter_map(Operand::place).collect(),
					Rvalue::UnaryOp(_, op) | Rvalue::Use(op, _) => op.place().into_iter().collect(),
					Rvalue::RawPtr(_, p) => vec![*p],
					_ => unreachable!("collected above"),
				};
				for p in read {
					self.uses[p.local] -= 1;
					for elem in p.projection {
						if let mir::ProjectionElem::Index(index) = elem {
							self.uses[index] -= 1;
						}
					}
					work.push(p.local);
				}
			}
		}
	}

	/// A local nothing changes whose one read is a shared borrow, itself read once, is
	/// read once where the borrow is: it folds like a local nothing borrows, as an array
	/// made to pass as a slice does.
	fn unborrow_passed(&mut self) {
		let mir = self.mir;
		for data in mir.basic_blocks.iter() {
			for stmt in &data.statements {
				if let StatementKind::Assign(assign) = &stmt.kind
					&& let (to, Rvalue::Ref(_, mir::BorrowKind::Shared, from)) = &**assign
					&& let Some(to) = to.as_local()
					&& let Some(from) = from.as_local()
					&& self.borrowed[from]
					&& !self.mutated[from]
					&& self.uses[from] == 1
					&& self.defs[to] == 1
					&& self.uses[to] == 1
					&& !self.borrowed[to]
				{
					self.borrowed[from] = false;
				}
			}
		}
	}

	/// Find the swaps `t = move x; x = move t;` through a `t` nothing else names.
	fn find_noops(&mut self) {
		use rustc_middle::mir::visit::{PlaceContext, Visitor};
		struct Mentions(IndexVec<Local, u32>);
		impl<'tcx> Visitor<'tcx> for Mentions {
			fn visit_local(&mut self, local: Local, cx: PlaceContext, _: mir::Location) {
				if !matches!(cx, PlaceContext::NonUse(_)) {
					self.0[local] += 1;
				}
			}
		}
		let mir = self.mir;
		let mut mentions = Mentions(IndexVec::from_elem_n(0, mir.local_decls.len()));
		mentions.visit_body(mir);
		let moved = |stmt: &mir::Statement<'tcx>| match &stmt.kind {
			StatementKind::Assign(assign) => match &assign.1 {
				Rvalue::Use(Operand::Move(from), _) => Some((assign.0.as_local()?, from.as_local()?)),
				_ => None,
			},
			_ => None,
		};
		for (bb, data) in mir.basic_blocks.iter_enumerated() {
			for (i, pair) in data.statements.windows(2).enumerate() {
				if let (Some((t, x)), Some((x2, t2))) = (moved(&pair[0]), moved(&pair[1]))
					&& x == x2
					&& t == t2
					&& t != x
					&& mentions.0[t] == 2
				{
					self.noops.insert((bb, i));
					self.noops.insert((bb, i + 1));
				}
			}
		}
	}

	fn def(&mut self, place: Place<'tcx>) {
		match place.as_local() {
			Some(local) => self.defs[local] += 1,
			// Assigning a projection reads the local it starts from, and changes it in place
			// unless it goes through a reference.
			None => {
				self.place_use(place);
				if own(place) {
					self.mutated[place.local] = true;
				}
			}
		}
	}

	fn place_use(&mut self, place: Place<'tcx>) {
		self.uses[place.local] += 1;
		for elem in place.projection {
			if let mir::ProjectionElem::Index(index) = elem {
				self.uses[index] += 1;
			}
		}
	}

	fn operand_use(&mut self, op: &Operand<'tcx>) {
		if let Operand::Copy(place) | Operand::Move(place) = op {
			self.place_use(*place);
		}
	}

	fn rvalue_uses(&mut self, rvalue: &Rvalue<'tcx>) {
		match rvalue {
			Rvalue::Use(op, _) | Rvalue::Repeat(op, _) | Rvalue::Cast(_, op, _) | Rvalue::UnaryOp(_, op) => {
				self.operand_use(op)
			}
			Rvalue::BinaryOp(_, ops) => {
				self.operand_use(&ops.0);
				self.operand_use(&ops.1);
			}
			// A borrow through a reference borrows what it points at, not the local.
			Rvalue::Ref(_, kind, place) => {
				self.place_use(*place);
				if own(*place) {
					self.borrowed[place.local] = true;
					if let mir::BorrowKind::Mut { .. } = kind {
						self.mutated[place.local] = true;
					}
				}
			}
			// Only `PtrMetadata` reads a fake borrow, so it borrows nothing.
			Rvalue::RawPtr(mir::RawPtrKind::FakeForPtrMetadata, place) => self.place_use(*place),
			Rvalue::RawPtr(_, place) => {
				self.place_use(*place);
				if own(*place) {
					self.borrowed[place.local] = true;
					self.mutated[place.local] = true;
				}
			}
			Rvalue::Discriminant(place) | Rvalue::CopyForDeref(place) => self.place_use(*place),
			Rvalue::Reborrow(_, _, place) => {
				self.place_use(*place);
				self.borrowed[place.local] = true;
			}
			Rvalue::Aggregate(_, ops) => ops.iter().for_each(|op| self.operand_use(op)),
			Rvalue::ThreadLocalRef(_) | Rvalue::WrapUnsafeBinder(..) => {}
		}
	}

	/// Whether a `ty` has no JS value: `()`, a function item, or `..`.
	fn zero_sized(&self, ty: Ty<'tcx>) -> bool {
		match ty.kind() {
			ty::Tuple(tys) => tys.is_empty(),
			ty::FnDef(..) | ty::Never => true,
			ty::Adt(adt, _) => {
				crate::lower::library::Std::of(self.tcx, adt.did()) == Some(crate::lower::library::Std::RangeFull)
			}
			_ => false,
		}
	}

	/// A zero-sized value that nothing assigns, the result included, which MIR leaves
	/// unassigned since it holds nothing: JS needs it made, as a constant of it is. A
	/// closure's is declared once instead, in [`Self::lower`].
	fn unassigned(&mut self, local: Local, ty: Ty<'tcx>) -> Option<Expr> {
		if self.defs[local] != 0 || self.is_arg(local) || self.uses[local] == 0 || ty.is_closure() {
			return None;
		}
		let env = self.b.t.typing_env_pub();
		let layout = self.tcx.layout_of(env.as_query_input(ty)).ok()?;
		if !layout.is_zst() || layout.is_uninhabited() {
			return None;
		}
		let span = self.local_span(local);
		Some(self.const_value(mir::ConstValue::ZeroSized, ty, span))
	}

	/// Whether `local` is a box's pointer, as a box's dereference reads it.
	fn box_pointer(&self, local: Local) -> bool {
		matches!(
			self.def_rvalue(local),
			Some(Rvalue::Cast(mir::CastKind::BoxDerefTransmute, ..))
		)
	}

	/// Whether `local` is a parameter.
	fn is_arg(&self, local: Local) -> bool {
		(1..=self.mir.arg_count).contains(&local.as_usize())
	}

	/// Whether `local` is a reference assigned once from a place that stays put, which
	/// every read can read in its place, or a box's pointer, which is the box in JS.
	fn alias(&self, local: Local) -> bool {
		let mir = self.mir;
		if self.is_arg(local) || self.defs[local] != 1 || self.borrowed[local] {
			return false;
		}
		let Some(rvalue) = self.def_rvalue(local) else {
			return false;
		};
		if self.context_of_arg(local) {
			return true;
		}
		// A number or a flag assigned once from a constant: the constant.
		if let Rvalue::Use(Operand::Constant(_), _) = rvalue
			&& !self.mutated[local]
		{
			let ty = mir.local_decls[local].ty;
			return ty.is_bool() || ty.is_char() || crate::lower::num::Int::of(ty).is_some_and(|i| !i.big());
		}
		match (mir.local_decls[local].ty.kind(), rvalue) {
			// A borrow: borrow checking keeps the place as it is but through the borrow,
			// unless the path to it goes through a reference held in a field or element,
			// which can be replaced while the borrow lives.
			// A borrow of bytes not yet in a slice makes one, and one of a `MaybeUninit` of
			// bytes in a slice a view of them.
			(ty::Ref(..), Rvalue::Ref(_, _, place))
				if crate::lower::byte_array(place.ty(&mir.local_decls, self.tcx).ty) != self.in_bytes(*place) =>
			{
				false
			}
			(ty::Ref(..), Rvalue::Ref(_, _, place)) => {
				self.stable(*place)
					&& place
						.projection
						.iter()
						.skip(1)
						.all(|e| !matches!(e, mir::ProjectionElem::Deref))
			}
			// A copy of a reference, or a box's pointer: the place it is read from must not
			// change at all.
			(
				ty::Ref(..),
				Rvalue::Use(Operand::Copy(place) | Operand::Move(place), _) | Rvalue::CopyForDeref(place),
			)
			| (
				ty::RawPtr(..),
				Rvalue::Cast(mir::CastKind::BoxDerefTransmute, Operand::Copy(place) | Operand::Move(place), _),
			) if self.env_field(*place) => true,
			(
				ty::Ref(..),
				Rvalue::Use(Operand::Copy(place) | Operand::Move(place), _) | Rvalue::CopyForDeref(place),
			)
			| (
				ty::RawPtr(..),
				Rvalue::Cast(mir::CastKind::BoxDerefTransmute, Operand::Copy(place) | Operand::Move(place), _),
			) => {
				self.stable(*place)
					&& !self.mutated[place.local]
					&& place
						.projection
						.iter()
						.all(|e| !matches!(e, mir::ProjectionElem::Deref))
			}
			_ => false,
		}
	}

	/// Whether a reference in `local` is read whole, rather than only through, or a mutable
	/// reborrow of what it points at is: passed on, kept, or captured. A `&mut` parameter of
	/// std's reads through it, as its mapping takes what it points at.
	fn read_whole(&self, local: Local) -> bool {
		use rustc_middle::mir::visit::{PlaceContext, Visitor};
		struct Reads<'a, 'tcx> {
			local: Local,
			whole: bool,
			reborrows: Vec<Local>,
			pointees: &'a dyn Fn(&Operand<'tcx>, usize) -> bool,
		}
		impl<'tcx> Visitor<'tcx> for Reads<'_, 'tcx> {
			fn visit_terminator(&mut self, term: &mir::Terminator<'tcx>, at: mir::Location) {
				if let TerminatorKind::Call { func, args, .. } = &term.kind
					&& args.iter().enumerate().any(|(i, a)| {
						a.node.place().is_some_and(|p| p.as_local() == Some(self.local)) && !(self.pointees)(func, i)
					}) {
					self.whole = true;
				}
				if !matches!(term.kind, TerminatorKind::Call { .. }) {
					self.super_terminator(term, at);
				}
			}
			fn visit_assign(&mut self, to: &Place<'tcx>, rvalue: &Rvalue<'tcx>, at: mir::Location) {
				if let Rvalue::Ref(_, mir::BorrowKind::Mut { .. }, p) = rvalue
					&& p.local == self.local
					&& let [mir::ProjectionElem::Deref] = p.projection.as_slice()
					&& let Some(to) = to.as_local()
				{
					self.reborrows.push(to);
				}
				self.super_assign(to, rvalue, at);
			}
			fn visit_place(&mut self, place: &Place<'tcx>, cx: PlaceContext, _: mir::Location) {
				let used = matches!(cx, PlaceContext::NonMutatingUse(_))
					|| matches!(cx, PlaceContext::MutatingUse(mir::visit::MutatingUseContext::Borrow));
				if place.local == self.local && place.projection.is_empty() && used {
					self.whole = true;
				}
			}
		}
		let tcx = self.tcx;
		let pointees = |func: &Operand<'tcx>, i: usize| match *func.ty(&self.mir.local_decls, tcx).kind() {
			ty::FnDef(def_id, gargs) if self.b.t.mapped(def_id, gargs.skip_binder()) => {
				let sig = tcx.fn_sig(def_id).instantiate_identity().skip_norm_wip();
				let input = tcx.instantiate_bound_regions_with_erased(sig).inputs().get(i).copied();
				input.is_some_and(|t| matches!(t.kind(), ty::Ref(_, _, rustc_ast::Mutability::Mut)))
			}
			_ => false,
		};
		let mut reads = Reads {
			local,
			whole: false,
			reborrows: Vec::new(),
			pointees: &pointees,
		};
		reads.visit_body(self.mir);
		reads.whole || reads.reborrows.into_iter().any(|r| r != local && self.read_whole(r))
	}

	/// Whether `local` is the task's context an async body is resumed with, as it passes it
	/// on: a parameter nothing assigns, or a local assigned it once, in another type that
	/// is the context in JS too.
	fn context_of_arg(&self, local: Local) -> bool {
		if self.borrowed[local] || self.mutated[local] || !rvalue::context_like(self, self.mir.local_decls[local].ty) {
			return false;
		}
		if self.is_arg(local) {
			return self.defs[local] == 0;
		}
		if self.defs[local] != 1 {
			return false;
		}
		let from = match self.def_rvalue(local) {
			Some(
				Rvalue::Use(Operand::Copy(p) | Operand::Move(p), _)
				| Rvalue::Cast(mir::CastKind::Transmute, Operand::Copy(p) | Operand::Move(p), _),
			) => p,
			Some(Rvalue::Aggregate(_, ops)) if ops.len() == 1 => match &ops[rustc_abi::FieldIdx::ZERO] {
				Operand::Copy(p) | Operand::Move(p) => p,
				Operand::Constant(_) | Operand::RuntimeChecks(_) => return false,
			},
			_ => return false,
		};
		from.as_local().is_some_and(|from| self.context_of_arg(from))
	}

	/// Whether `place` is a field of a closure's environment, which stays put for the call.
	fn env_field(&self, place: Place<'tcx>) -> bool {
		matches!(self.env, Env::Closure(_))
			&& place.local.as_u32() == 1
			&& matches!(
				place.projection.as_slice(),
				[mir::ProjectionElem::Field(..)] | [mir::ProjectionElem::Deref, mir::ProjectionElem::Field(..)]
			)
	}

	/// Locals an `Iterator::next` assigns that only a switch on their variant and a read
	/// of `Some`'s value read: each holds the item or `rt.DONE`, as the protocol returns
	/// it, so an iterator of `Option` keeps each `None` apart from its end.
	fn find_next_results(&mut self) {
		use rustc_middle::mir::visit::{MutatingUseContext, NonMutatingUseContext, PlaceContext, Visitor};
		struct Uses<'tcx> {
			local: Local,
			/// The variants whose value may be read.
			variants: &'static [u32],
			/// Where a move of the whole into `?`'s branch may be.
			branch: Option<mir::Location>,
			ok: bool,
			_tcx: std::marker::PhantomData<Ty<'tcx>>,
		}
		impl<'tcx> Visitor<'tcx> for Uses<'tcx> {
			fn visit_place(&mut self, place: &Place<'tcx>, cx: PlaceContext, at: mir::Location) {
				if place.local != self.local {
					return;
				}
				self.ok &= match place.projection.as_slice() {
					[] => {
						matches!(
							cx,
							PlaceContext::NonMutatingUse(NonMutatingUseContext::Inspect)
								| PlaceContext::MutatingUse(MutatingUseContext::Call)
						) || (matches!(cx, PlaceContext::NonMutatingUse(NonMutatingUseContext::Move))
							&& Some(at) == self.branch)
					}
					[
						mir::ProjectionElem::Downcast(_, v),
						mir::ProjectionElem::Field(f, _),
						..,
					] => self.variants.contains(&v.as_u32()) && f.as_u32() == 0,
					_ => false,
				};
			}
		}
		let mir = self.mir;
		let tcx = self.tcx;
		// `?` on a local: where, and what it branches to.
		let mut branches: FxHashMap<Local, (mir::Location, Local)> = FxHashMap::default();
		let mut nexts = Vec::new();
		for (bb, data) in mir.basic_blocks.iter_enumerated() {
			let TerminatorKind::Call {
				func,
				args,
				destination,
				..
			} = &data.terminator().kind
			else {
				continue;
			};
			let ty::FnDef(def_id, gargs) = *func.ty(&mir.local_decls, tcx).kind() else {
				continue;
			};
			let Some(local) = destination.as_local() else {
				continue;
			};
			// The return place leaves as the `Option` the signature declares, unless this is a
			// `next` itself, which returns the protocol's item or `rt.DONE` below.
			if crate::lower::iter::is_next(tcx, def_id) && self.defs[local] == 1 && local != mir::RETURN_PLACE {
				nexts.push((local, gargs.skip_binder().type_at(0)));
			}
			if crate::lower::library::key(tcx, def_id).is_some_and(|(o, n)| o == "Try" && n == "branch")
				&& let [arg] = &**args
				&& let Operand::Move(p) = &arg.node
				&& let Some(from) = p.as_local()
			{
				let at = mir::Location {
					block: bb,
					statement_index: data.statements.len(),
				};
				branches.insert(from, (at, local));
			}
		}
		for (local, iter_ty) in nexts {
			let branch = branches.get(&local).copied();
			let mut uses = Uses {
				local,
				variants: &[1],
				branch: branch.map(|(at, _)| at),
				ok: true,
				_tcx: std::marker::PhantomData,
			};
			uses.visit_body(mir);
			if !uses.ok {
				continue;
			}
			// What `?` branches to holds the same: the item, or `rt.DONE` to return early.
			if let Some((_, to)) = branch {
				let mut uses = Uses {
					local: to,
					variants: &[0, 1],
					branch: None,
					ok: self.defs[to] == 1,
					_tcx: std::marker::PhantomData,
				};
				uses.visit_body(mir);
				if !uses.ok {
					continue;
				}
				self.next_results.insert(to);
			} else if small_range(tcx, iter_ty) {
				self.range_nexts.insert(local);
			}
			self.next_results.insert(local);
		}
		// An `Iterator::next` returns the item or `rt.DONE` too.
		if self.next {
			self.next_results.insert(mir::RETURN_PLACE);
		}
	}

	/// Each `(hi as u64) << 32 | lo as u64` of halves of at most 32 bits, as moq's
	/// `from_halves` writes it, which lowers to `rt.u64FromHalves(hi, lo)`: no shift that
	/// boxes limbs past 2^53, and no `|` that unboxes them. Its shift and casts go
	/// unlowered, so it reads the halves where the `|` is, which takes temporaries read
	/// once, of halves nothing assigns again.
	fn find_halves(&mut self) {
		let mir: &'m mir::Body<'tcx> = self.mir;
		let tcx = self.tcx;
		let temp = |op: &Operand<'tcx>| match op {
			Operand::Move(p) => p.as_local().filter(|&l| self.defs[l] == 1 && self.uses[l] == 1),
			_ => None,
		};
		// A temporary's cast to `u64` from at most 32 unsigned bits (`usize` is 32), and
		// what it casts.
		let half = |op: &Operand<'tcx>| {
			let local = temp(op)?;
			let Some(Rvalue::Cast(mir::CastKind::IntToInt, from, _)) = self.def_rvalue(local) else {
				return None;
			};
			let narrow =
				matches!(from.ty(&mir.local_decls, tcx).kind(), ty::Uint(u) if u.bit_width().is_none_or(|b| b <= 32));
			let fixed = match from {
				Operand::Constant(_) => true,
				Operand::Copy(p) | Operand::Move(p) => {
					p.as_local().is_some_and(|l| !self.mutated[l]) && self.stable(*p)
				}
				_ => false,
			};
			(narrow && fixed).then_some((local, from))
		};
		let mut found = Vec::new();
		for data in mir.basic_blocks.iter() {
			for stmt in &data.statements {
				let StatementKind::Assign(assign) = &stmt.kind else {
					continue;
				};
				let Rvalue::BinaryOp(mir::BinOp::BitOr, ops) = &assign.1 else {
					continue;
				};
				let Some(shift) = temp(&ops.0).filter(|_| ops.0.ty(&mir.local_decls, tcx) == tcx.types.u64) else {
					continue;
				};
				let Some(Rvalue::BinaryOp(mir::BinOp::Shl, shifted)) = self.def_rvalue(shift) else {
					continue;
				};
				if self.const_int(&shifted.1) == Some(32)
					&& let (Some(hi), Some(lo)) = (half(&shifted.0), half(&ops.1))
				{
					found.push((shift, hi, lo));
				}
			}
		}
		for (shift, (cast_hi, hi), (cast_lo, lo)) in found {
			self.halves.insert(shift, (hi, lo));
			self.joined.extend([shift, cast_hi, cast_lo]);
		}
	}

	/// Mark the locals whose variant a switch or a library call's test tests, that a
	/// closure or async block captures and could test, or that std takes, whose runtime
	/// function infers its type parameters from them, and what references to them borrow.
	fn find_tested(&mut self) {
		let tcx = self.tcx;
		let mut work: Vec<Local> = self.narrowing.tested().collect();
		for data in self.mir.basic_blocks.iter() {
			if let TerminatorKind::Call { func, args, .. } = &data.terminator().kind
				&& let ty::FnDef(def_id, _) = *func.ty(&self.mir.local_decls, tcx).kind()
				&& crate::lower::library::is_std(tcx, def_id)
			{
				work.extend(args.iter().filter_map(|arg| arg.node.place().map(|p| p.local)));
			}
			for stmt in &data.statements {
				let StatementKind::Assign(assign) = &stmt.kind else {
					continue;
				};
				match &assign.1 {
					Rvalue::Discriminant(place) => work.push(place.local),
					Rvalue::Aggregate(kind, ops)
						if matches!(
							**kind,
							mir::AggregateKind::Closure(..) | mir::AggregateKind::Coroutine(..)
						) =>
					{
						work.extend(ops.iter().filter_map(|op| op.place().map(|p| p.local)));
					}
					_ => {}
				}
			}
		}
		while let Some(local) = work.pop() {
			if std::mem::replace(&mut self.tested[local], true) {
				continue;
			}
			if let Slot::Alias(_) = self.slots[local]
				&& let Some(
					Rvalue::Ref(_, _, p)
					| Rvalue::Use(Operand::Copy(p) | Operand::Move(p), _)
					| Rvalue::CopyForDeref(p)
					| Rvalue::Cast(_, Operand::Copy(p) | Operand::Move(p), _),
				) = self.def_rvalue(local)
			{
				work.push(p.local);
			}
		}
	}

	/// An `Option` this `Iterator::next` returns, as it returns it: the item, or
	/// `rt.DONE` for `None`.
	pub(super) fn done_of(&self, option: Expr) -> Expr {
		let ty::Adt(_, args) = self.mir.return_ty().kind() else {
			unreachable!("`next` returns an `Option`")
		};
		let none = self.b.t.none(args.type_at(0));
		match option {
			option if option == none => Expr::Rt("DONE".into()),
			option => {
				let none = self.b.t.none_arg(args.type_at(0));
				Expr::rt("iterFromOption", [option].into_iter().chain(none).collect())
			}
		}
	}

	/// `value`, the `rvalue` assigned to `place`, as a value of its whole type where
	/// something tests `place`'s variant or std takes it: TypeScript narrows a variable to
	/// the type of what it was assigned, as a `Poll` made `Ready` to its payload's, and
	/// would refuse a test for another variant, or infer a `None`'s `Option<T>` as of
	/// `T = undefined`.
	fn widen(&mut self, place: Place<'tcx>, rvalue: &Rvalue<'tcx>, value: Expr, span: Span) -> Expr {
		let ty = place.ty(&self.mir.local_decls, self.tcx).ty;
		// What is made here, rather than read from a place declared with its type.
		let made = matches!(rvalue, Rvalue::Aggregate(..) | Rvalue::Use(Operand::Constant(_), _));
		if !made || !self.tested[place.local] || !self.b.t.narrows(ty) {
			return value;
		}
		// `Some` narrows to its payload's type, which only a test for `None` refutes, and
		// that reads nothing of it.
		if let Rvalue::Aggregate(kind, _) = rvalue
			&& let mir::AggregateKind::Adt(def, variant, args, ..) = **kind
			&& crate::lower::library::Std::of(self.tcx, def) == Some(crate::lower::library::Std::Option)
			&& variant.index() == 1
			&& !self.b.t.narrows(args.type_at(0))
		{
			return value;
		}
		let local = place.as_local();
		let ts_ty = match local {
			Some(local) if self.stepped_vars.contains_key(&local) => return value,
			Some(local) => self.decl_ty(local),
			None => self.b.t.ty(ty, span),
		};
		let value = match whole(value, ts_ty) {
			Ok(value) => value,
			Err(value) => return value,
		};
		// Its declaration keeps the `as`.
		if let Some(local) = local
			&& let Slot::Var(name) = &self.slots[local]
		{
			self.widened.insert(name.clone());
		}
		value
	}

	/// The whole type of the place whose discriminant `discr` holds.
	fn scrutinee_ty(&mut self, discr: Local, span: Span) -> Type {
		let Some(Rvalue::Discriminant(place)) = self.def_rvalue(discr) else {
			unreachable!("widened only where a discriminant is read")
		};
		match place.as_local() {
			Some(local) => self.decl_ty(local),
			None => self.b.t.ty(place.ty(&self.mir.local_decls, self.tcx).ty, span),
		}
	}

	/// The rvalue a local's one assignment by a statement assigns.
	fn def_rvalue(&self, local: Local) -> Option<&'m Rvalue<'tcx>> {
		let (bb, i) = self.def_at[local]?;
		let mir: &'m mir::Body<'tcx> = self.mir;
		match &mir.basic_blocks[bb].statements[i].kind {
			StatementKind::Assign(assign) => Some(&assign.1),
			_ => None,
		}
	}

	/// The slice whose length `len` is assigned: by `PtrMetadata` of a reference to it, or
	/// of the fake raw pointer to it a bounds check of a `&mut` slice reads.
	fn measured(&self, len: Local) -> Option<Place<'tcx>> {
		let Some(Rvalue::UnaryOp(mir::UnOp::PtrMetadata, Operand::Copy(slice) | Operand::Move(slice))) =
			self.def_rvalue(len)
		else {
			return None;
		};
		match slice.as_local().and_then(|raw| self.def_rvalue(raw)) {
			Some(Rvalue::RawPtr(mir::RawPtrKind::FakeForPtrMetadata, place)) => self.stable(*slice).then_some(*place),
			_ => Some(*slice),
		}
	}

	/// Whether `place` names the same memory wherever it is read: every local in it is
	/// assigned at most once.
	fn stable(&self, place: Place<'tcx>) -> bool {
		let fixed = |local: Local| self.defs[local] <= 1 && (!self.is_arg(local) || self.defs[local] == 0);
		fixed(place.local)
			&& place.projection.iter().all(|elem| match elem {
				mir::ProjectionElem::Index(index) => fixed(index),
				_ => true,
			})
	}

	/// Whether every read of `local` is a `switchInt` on it, after its one assignment of a
	/// discriminant.
	fn discr_only(&self, local: Local) -> bool {
		if self.defs[local] != 1 || self.uses[local] != 1 {
			return false;
		}
		let assigned = matches!(self.def_rvalue(local), Some(Rvalue::Discriminant(_)));
		let mut switched = false;
		for data in self.mir.basic_blocks.iter() {
			if let TerminatorKind::SwitchInt {
				discr: Operand::Move(p) | Operand::Copy(p),
				..
			} = &data.terminator().kind
				&& p.as_local() == Some(local)
			{
				switched = true;
			}
		}
		assigned && switched
	}

	/// Whether the switch on the discriminant `local` tests it, rather than going the one
	/// way its type leaves.
	fn discr_tested(&self, local: Local) -> bool {
		self.mir.basic_blocks.iter().any(|data| {
			matches!(
				&data.terminator().kind,
				TerminatorKind::SwitchInt { discr: Operand::Move(p) | Operand::Copy(p), targets }
					if p.as_local() == Some(local) && !self.switch_targets(targets).0.is_empty()
			)
		})
	}

	/// The parameters: `_1` through the argument count, but a closure's environment or a
	/// coroutine's `this`.
	pub fn params(&mut self) -> Vec<Param> {
		let mut params = Vec::new();
		let skip = match self.env {
			Env::Plain | Env::Shim => 0,
			_ => 1,
		};
		for local in self.mir.args_iter().skip(skip) {
			let ty = match (&self.env, self.mir.local_decls[local].ty.kind()) {
				(Env::Shim, ty::RawPtr(pointee, _) | ty::Ref(_, pointee, _)) => *pointee,
				(_, _) => self.mir.local_decls[local].ty,
			};
			let name = match &self.slots[local] {
				Slot::Var(name) => name.clone(),
				_ => self.b.fresh("_"),
			};
			let ty = self.b.t.ty(ty, self.local_span(local));
			params.push(Param { name, ty: Some(ty) });
		}
		params
	}

	/// The body: its locals' declarations, then its blocks as structured statements.
	pub fn lower(&mut self) -> Vec<Stmt> {
		// A parameter changed in place gets its own copy, since the caller's may be shared.
		let mut entry = Vec::new();
		for local in self.mir.args_iter() {
			let ty = self.mir.local_decls[local].ty;
			if self.mutated[local]
				&& let Slot::Var(name) = &self.slots[local]
				&& let Some(copy) = self.b.t.copy_fn(ty, self.local_span(local))
			{
				let name = name.clone();
				entry.push(StmtKind::Expr(Expr::ident(&name).assign(copy.call(vec![Expr::ident(&name)]))).at(None));
			}
		}
		// A closure that captures nothing holds nothing, so MIR never assigns it: JS needs
		// its function.
		for local in self.mir.local_decls.indices() {
			if let ty::Closure(def_id, args) = *self.mir.local_decls[local].ty.kind()
				&& self.defs[local] == 0
				&& !self.is_arg(local)
				&& self.uses[local] > 0
				&& let Slot::Var(name) = self.slots[local].clone()
			{
				let f = self.bare_closure(def_id, args, self.span);
				self.assigned[local] = true;
				entry.push(StmtKind::Expr(Expr::ident(&name).assign(f)).at(None));
			}
		}
		let n = self.mir.basic_blocks.len();
		self.skip_ignored_macros();
		let mut succs = vec![Vec::new(); n];
		for (bb, data) in self.mir.basic_blocks.iter_enumerated() {
			if data.is_cleanup {
				continue;
			}
			// Past blocks that do nothing but go on, such as one that only ends locals'
			// storage, as each node's exit goes.
			succs[bb.as_usize()] = match self.redirect.get(&bb) {
				Some(&to) => vec![self.forward(to).as_usize()],
				None => {
					let mut succs: Vec<usize> = Vec::new();
					for to in self.successors(data.terminator()) {
						let to = self.forward(to).as_usize();
						if !succs.contains(&to) {
							succs.push(to);
						}
					}
					succs
				}
			};
		}
		let graph = cfg::Graph { succs };
		let mut labels = self.labels;
		let body = cfg::structure(&graph, self, &mut labels);
		self.labels = labels;
		let mut out = Vec::new();
		for local in self.mir.local_decls.indices() {
			// Parameters are declared by the signature.
			if self.is_arg(local) || !self.assigned[local] {
				continue;
			}
			if let Some(vars) = self.stepped_vars.get(&local).cloned() {
				// Declared once, by the local that shares them.
				if self.stepped_root(local) != local {
					continue;
				}
				for (name, ty) in vars.into_iter().zip(self.stepped_tys(local)) {
					out.push(StmtKind::Declare { name, ty }.at(None));
				}
			} else if let Slot::Var(name) = self.slots[local].clone() {
				let ty = self.decl_ty(local);
				out.push(StmtKind::Declare { name, ty }.at(None));
			}
		}
		self.scalar_decls(&mut out);
		let mut flags: Vec<String> = self.range_flags.values().cloned().collect();
		flags.sort();
		for name in flags {
			out.push(
				StmtKind::Declare {
					name,
					ty: Type::Builtin("boolean"),
				}
				.at(None),
			);
		}
		out.extend(entry);
		out.extend(body);
		drop_self_assignments(&mut out);
		// A return folds in the value assigned only to a local: a closure's captures are
		// read after it returns.
		let returnable = (self.slots.iter())
			.filter_map(|slot| match slot {
				Slot::Var(name) => Some(name.clone()),
				_ => None,
			})
			.collect();
		return_in_place(&mut out, &returnable);
		prune_unread(&mut out);
		if self.zero_sized(self.mir.local_decls[mir::RETURN_PLACE].ty) {
			cfg::strip_return(&mut out);
		}
		cfg::hoist_else(&mut out);
		declare_in_place(&mut out, &self.widened);
		cfg::tidy_loops(&mut out);
		// A loop's own labels first, which its header's `break` names.
		step::for_of(&mut out);
		inline::inline_temps(&mut out, &self.shared);
		// A loop can start by leaving now that what it tests is folded in.
		cfg::tidy_loops(&mut out);
		out
	}

	/// Every left-out macro invocation's span, which its arguments' code is within too.
	fn find_ignored_sites(&mut self) {
		let mut sites = Vec::new();
		for data in self.mir.basic_blocks.iter() {
			let spans = data.statements.iter().map(|s| s.source_info.span);
			for span in spans.chain([data.terminator().source_info.span]) {
				if let Some(site) = self.b.t.ignored_site(span)
					&& !sites.contains(&site)
				{
					sites.push(site);
				}
			}
		}
		self.ignored_sites = sites;
	}

	/// Send each block whose terminator an ignored macro made to the one block its code
	/// leaves to. Such a macro evaluates to `()`, so its code has one way out.
	fn skip_ignored_macros(&mut self) {
		let mir = self.mir;
		let flags: IndexVec<BasicBlock, bool> = mir
			.basic_blocks
			.iter()
			.map(|data| !data.is_cleanup && self.in_ignored(data.terminator().source_info.span))
			.collect();
		let ignored = |bb: BasicBlock| flags[bb];
		for bb in mir.basic_blocks.indices() {
			if !ignored(bb) {
				continue;
			}
			let mut seen = FxHashSet::default();
			let mut stack = vec![bb];
			let mut exits = FxHashSet::default();
			while let Some(next) = stack.pop() {
				if !seen.insert(next) {
					continue;
				}
				for succ in self.successors(mir.basic_blocks[next].terminator()) {
					// Past blocks that only end storage, which may go on into another macro.
					let succ = if ignored(succ) { succ } else { self.forward(succ) };
					match ignored(succ) {
						true => stack.push(succ),
						false => {
							exits.insert(succ);
						}
					}
				}
			}
			match exits.len() {
				1 => {
					self.redirect.insert(bb, *exits.iter().next().unwrap());
				}
				_ => {
					let span = mir.basic_blocks[bb].terminator().source_info.span;
					self.err(span, "an ignored macro whose code has other than one way out");
				}
			}
		}
	}

	/// Whether `span` is within a left-out macro's invocation.
	fn in_ignored(&self, span: Span) -> bool {
		let span = span.source_callsite();
		self.ignored_sites.iter().any(|site| site.contains(span))
	}

	/// A local's declared type: the value's, for a checked pair.
	fn decl_ty(&mut self, local: Local) -> Type {
		let ty = self.mir.local_decls[local].ty;
		if self.next_results.contains(&local) {
			let item = self.b.t.held_ty(next_item(self.tcx, ty), self.local_span(local));
			if self.range_nexts.contains(&local) {
				return item;
			}
			return Type::Union(vec![item, Type::Rt("Done", Vec::new())]);
		}
		let ty = match (self.checked.contains(&local), ty.kind()) {
			(true, ty::Tuple(tys)) => tys[0],
			// A box's pointer is the box's contents in JS.
			(_, ty::RawPtr(pointee, _)) if self.box_pointer(local) => *pointee,
			_ => ty,
		};
		self.b.t.held_ty(ty, self.local_span(local))
	}

	/// Where a local is declared, which a refusal of its type points at: for drop glue,
	/// whose locals are std's, where the value drops.
	pub(super) fn local_span(&self, local: Local) -> Span {
		match self.env {
			Env::Shim => self.span,
			_ => self.mir.local_decls[local].source_info.span,
		}
	}

	/// Where a terminator can go, without its unwinding.
	fn successors(&self, term: &mir::Terminator<'tcx>) -> Vec<BasicBlock> {
		match &term.kind {
			TerminatorKind::Goto { target, .. } => vec![*target],
			// An unreachable default is no edge: the last value takes its place.
			TerminatorKind::SwitchInt { targets, .. } => {
				let (arms, default) = self.switch_targets(targets);
				let mut all: Vec<BasicBlock> = Vec::new();
				for bb in arms.into_iter().map(|(_, bb)| bb).chain([default]) {
					if !all.contains(&bb) {
						all.push(bb);
					}
				}
				all
			}
			TerminatorKind::Assert { .. } if fails(term) => Vec::new(),
			TerminatorKind::Drop { target, .. } | TerminatorKind::Assert { target, .. } => vec![*target],
			TerminatorKind::Call { target, .. } => target.iter().copied().collect(),
			TerminatorKind::FalseEdge { real_target, .. } | TerminatorKind::FalseUnwind { real_target, .. } => {
				vec![*real_target]
			}
			TerminatorKind::Yield { resume, .. } => vec![*resume],
			TerminatorKind::InlineAsm { targets, .. } => targets.to_vec(),
			_ => Vec::new(),
		}
	}

	// Locals

	/// A switch's arms and default, each past blocks that do nothing but go on, without
	/// the arms that go where the default does. An unreachable default leaves the last
	/// arm as the default.
	fn switch_targets(&self, targets: &mir::SwitchTargets) -> (Vec<(u128, BasicBlock)>, BasicBlock) {
		let mut arms: Vec<(u128, BasicBlock)> = targets.iter().collect();
		let mut default = targets.otherwise();
		if self.unreachable.contains(&default)
			&& let Some((_, last)) = arms.pop()
		{
			default = last;
		}
		let default = self.forward(default);
		let returns = |bb: BasicBlock| matches!(self.mir.basic_blocks[bb].terminator().kind, TerminatorKind::Return);
		arms.retain_mut(|(_, target)| {
			*target = self.forward(*target);
			*target != default && !(returns(*target) && returns(default) && self.empty(*target) && self.empty(default))
		});
		(arms, default)
	}

	/// Whether `bb` has no statement that does anything.
	fn empty(&self, bb: BasicBlock) -> bool {
		self.mir.basic_blocks[bb].statements.iter().enumerate().all(|(i, s)| {
			matches!(
				s.kind,
				StatementKind::StorageDead(_) | StatementKind::StorageLive(_) | StatementKind::Nop
			) || self.noops.contains(&(bb, i))
		})
	}

	/// Where `bb` goes, past blocks that do nothing but go on.
	fn forward(&self, mut bb: BasicBlock) -> BasicBlock {
		for _ in 0..self.mir.basic_blocks.len() {
			let data = &self.mir.basic_blocks[bb];
			let TerminatorKind::Goto { target, .. } = data.terminator().kind else {
				return bb;
			};
			if !self.empty(bb) || data.is_cleanup || self.redirect.contains_key(&bb) {
				return bb;
			}
			bb = target;
		}
		bb
	}

	/// The JS name of a local held in a variable.
	fn var(&self, local: Local) -> Option<&str> {
		match &self.slots[local] {
			Slot::Var(name) => Some(name),
			_ => None,
		}
	}

	/// The value held back for `local`, if it is.
	fn held(&self, local: Local) -> Option<&Pending> {
		let name = self.var(local)?;
		self.pending.iter().find(|p| p.name == name)
	}

	/// Whether `local` can be held back as a pending temporary: one assignment, one read.
	fn foldable(&self, local: Local) -> bool {
		matches!(self.slots[local], Slot::Var(_))
			&& self.defs[local] == 1
			&& self.uses[local] == 1
			&& !self.borrowed[local]
			&& !self.is_arg(local)
	}

	/// Whether the aggregate assigned to `local` is read by its fields as its operands: it
	/// is assigned once, never changed, and not held back.
	fn parts_read(&self, local: Local) -> bool {
		matches!(self.slots[local], Slot::Var(_))
			&& !self.stepped_vars.contains_key(&local)
			&& self.uses[local] != 0
			&& !self.foldable(local)
			&& self.defs[local] == 1
			&& !self.mutated[local]
	}

	/// Whether reading `place` again, wherever and however often, reads what it reads now:
	/// a place nothing changes, a value held back that rereads the same, or a reference
	/// read as what it is assigned.
	fn rereads(&self, place: Place<'tcx>) -> bool {
		match (self.held(place.local), &self.slots[place.local]) {
			(Some(p), _) => place.projection.is_empty() && p.rereads,
			(None, Slot::Alias(_)) => {
				place.projection.is_empty()
					&& self
						.def_rvalue(place.local)
						.is_some_and(|rvalue| self.rvalue_rereads(rvalue))
			}
			_ => {
				self.stable(place)
					&& !self.mutated[place.local]
					&& place
						.projection
						.iter()
						.all(|e| !matches!(e, mir::ProjectionElem::Deref))
			}
		}
	}

	/// Whether `rvalue`, read again, reads what it reads now: a constant, a copy of a place
	/// that rereads the same, or a borrow, which borrow checking keeps as it is, unless the
	/// path to it goes through a reference that can be replaced first. A copy of a
	/// reference is read from where it is copied, which can be assigned another.
	fn rvalue_rereads(&self, rvalue: &Rvalue<'tcx>) -> bool {
		match rvalue {
			Rvalue::Use(Operand::Constant(_), _) => true,
			Rvalue::Use(Operand::Copy(from) | Operand::Move(from), _) | Rvalue::CopyForDeref(from) => {
				self.rereads(*from)
			}
			Rvalue::Ref(_, _, from) => {
				let fixed = |l: Local| self.defs[l] <= 1 && (!self.is_arg(l) || self.defs[l] == 0);
				let root = Place::from(from.local);
				from.projection.iter().all(|e| match e {
					mir::ProjectionElem::Index(index) => fixed(index),
					_ => true,
				}) && match from
					.projection
					.iter()
					.rposition(|e| matches!(e, mir::ProjectionElem::Deref))
				{
					// The borrow keeps a variable as it is, but not a value held back.
					None => match self.slots[from.local] {
						Slot::Var(_) if self.held(from.local).is_none() => true,
						_ => self.rereads(root),
					},
					Some(0) => self.rereads(root),
					Some(_) => false,
				}
			}
			_ => false,
		}
	}

	// Emission

	/// Emit the statements `stmts`, which together are one MIR statement, folding in the
	/// pending temporaries they read once where that keeps every effect in order. If
	/// `hold`, nothing runs now: the statements are a value being held back itself.
	fn emit(&mut self, stmts: Vec<Stmt>, hold: bool, out: &mut Vec<Stmt>) {
		if self.pending.is_empty() {
			out.extend(stmts);
			return;
		}
		// Where each pending temporary is read, in evaluation order, and what else is read
		// before it.
		let mut reads = Vec::new();
		for stmt in &stmts {
			scan_stmt(stmt, &mut reads, false);
		}
		let names: FxHashMap<&str, usize> = self
			.pending
			.iter()
			.enumerate()
			.map(|(i, p)| (p.name.as_str(), i))
			.collect();
		let mut count = vec![0u32; self.pending.len()];
		let mut deferred = vec![false; self.pending.len()];
		for read in &reads {
			if let Read::Ident(name, lazy) = read
				&& let Some(&i) = names.get(name.as_str())
			{
				count[i] += 1;
				deferred[i] |= *lazy;
			}
		}
		let fold: Vec<bool> = (0..self.pending.len()).map(|i| count[i] == 1 && !deferred[i]).collect();
		// Check the folded ones run in their order, after nothing they could change or be
		// changed by.
		let mut ok = true;
		let mut last: Option<u32> = None;
		let mut seen_memory = false;
		let mut seen_effect = false;
		for read in &reads {
			match read {
				Read::Ident(name, _) => match names.get(name.as_str()) {
					Some(&i) if fold[i] => {
						let p = &self.pending[i];
						if last.is_some_and(|l| l >= p.order)
							|| (p.effect && (seen_memory || seen_effect))
							|| (p.memory && seen_effect)
						{
							ok = false;
						}
						last = Some(p.order);
					}
					Some(_) => {}
					None => seen_memory |= self.shared.contains(name.as_str()),
				},
				Read::Memory => seen_memory = true,
				Read::Field(name) => seen_memory |= !self.frozen_names.contains(name.as_str()),
				Read::Effect | Read::Panic => seen_effect = true,
			}
		}
		// What stays pending past this statement, and what must run before it.
		let first_folded = (0..self.pending.len())
			.filter(|&i| fold[i])
			.map(|i| self.pending[i].order)
			.min();
		// A temporary held past a write of a name it reads would read the new value.
		let written = assigned(&stmts);
		let mut flush = Vec::new();
		for (i, p) in self.pending.iter().enumerate() {
			if fold[i] {
				continue;
			}
			let read = count[i] > 0;
			let overwritten = written.contains(&p.name) || p.reads.iter().any(|r| written.contains(r));
			let keep = match hold {
				true => !read && !overwritten,
				false => !read && !p.effect && !p.memory && !overwritten,
			};
			let after = first_folded.is_some_and(|f| p.order > f);
			if !keep {
				ok &= !after;
				flush.push(i);
			} else if hold && after && (p.effect || p.memory) {
				// It ran after something folded into a value held back past it.
				ok = false;
			}
		}
		if !ok {
			self.flush(out);
			out.extend(stmts);
			return;
		}
		let pending = std::mem::take(&mut self.pending);
		let mut stmts = stmts;
		let mut kept = Vec::new();
		for (i, p) in pending.into_iter().enumerate() {
			if fold[i] {
				for stmt in &mut stmts {
					if subst_stmt(stmt, &p.name, &p.value) {
						break;
					}
				}
			} else if flush.contains(&i) {
				self.flush_one(p, out);
			} else {
				kept.push(p);
			}
		}
		self.pending = kept;
		out.extend(stmts);
	}

	fn flush_one(&mut self, p: Pending, out: &mut Vec<Stmt>) {
		let local = self.local_named(&p.name);
		if let Some(local) = local {
			self.assigned[local] = true;
		}
		let value = match (p.copy, local) {
			(true, Some(local)) => self
				.b
				.t
				.copied(p.value, self.mir.local_decls[local].ty, self.local_span(local)),
			_ => p.value,
		};
		out.push(match p.decl {
			true => StmtKind::decl(false, p.name, None, Some(value)).at(None),
			false => StmtKind::Expr(Expr::ident(&p.name).assign(value)).at(None),
		});
	}

	/// Emit the pending temporaries `e` reads, and those held back before them, so that
	/// `e` can be read more than once, or later.
	pub(super) fn materialize(&mut self, e: &Expr, out: &mut Vec<Stmt>) {
		let mut names = Vec::new();
		reads_of(e, &mut names);
		let Some(last) = self
			.pending
			.iter()
			.filter(|p| names.contains(&p.name))
			.map(|p| p.order)
			.max()
		else {
			return;
		};
		let pending = std::mem::take(&mut self.pending);
		let (flush, keep): (Vec<Pending>, Vec<Pending>) = pending.into_iter().partition(|p| p.order <= last);
		self.pending = keep;
		for p in flush {
			self.flush_one(p, out);
		}
	}

	fn local_named(&self, name: &str) -> Option<Local> {
		self.slots
			.iter_enumerated()
			.find(|(_, slot)| matches!(slot, Slot::Var(n) if n == name))
			.map(|(local, _)| local)
	}

	/// `exprs`, after the statements `pre`, with what they read that is held back folded
	/// in where that keeps every effect in order; what can't fold is emitted to `out`.
	fn fold_exprs(&mut self, pre: Vec<Stmt>, exprs: Vec<Expr>, hold: bool, out: &mut Vec<Stmt>) -> Vec<Expr> {
		let n = exprs.len();
		let mut stmts = pre;
		stmts.extend(exprs.into_iter().map(|e| StmtKind::Expr(e).at(None)));
		let mut emitted = Vec::new();
		self.emit(stmts, hold, &mut emitted);
		let folded = emitted
			.drain(emitted.len() - n..)
			.map(|s| match s.kind {
				StmtKind::Expr(e) => e,
				_ => unreachable!("an expression statement"),
			})
			.collect();
		out.extend(emitted);
		folded
	}

	/// Hold back `value`, the assignment of `local`, to fold into what reads it.
	fn hold(
		&mut self,
		local: Local,
		value: Expr,
		effect: bool,
		memory: bool,
		parts: Option<Vec<(Expr, bool)>>,
		rereads: bool,
	) {
		let name = self.var(local).expect("a pending local is a variable").to_string();
		self.order += 1;
		let mut reads = Vec::new();
		reads_of(&value, &mut reads);
		let copy = std::mem::take(&mut self.copy_on_flush);
		self.pending.push(Pending {
			name,
			value,
			order: self.order,
			effect,
			memory,
			parts,
			reads,
			copy,
			decl: false,
			rereads,
		});
	}

	/// Hold back `value`, the assignment of the flag `flag`, to fold into the switch that
	/// reads it.
	fn hold_flag(&mut self, flag: String, value: Expr) {
		self.order += 1;
		let mut reads = Vec::new();
		reads_of(&value, &mut reads);
		self.pending.push(Pending {
			name: flag,
			value,
			order: self.order,
			effect: true,
			memory: true,
			parts: None,
			reads,
			copy: false,
			decl: false,
			rereads: false,
		});
	}

	/// The parts of the pending aggregate `local`, each held back in its place, so the
	/// expression that reads them keeps what they do and read in order.
	fn take_parts(&mut self, op: &Operand<'tcx>) -> Option<Vec<Expr>> {
		let Operand::Move(place) = op else {
			return None;
		};
		let local = place.as_local()?;
		let name = self.var(local)?;
		let i = self.pending.iter().position(|p| p.name == name && p.parts.is_some())?;
		let p = self.pending.remove(i);
		let parts: Vec<Expr> = p
			.parts
			.expect("checked above")
			.into_iter()
			.map(|(part, _)| part)
			.collect();
		if !p.effect && !p.memory {
			return Some(parts);
		}
		let n = parts.len() as u32;
		for q in &mut self.pending {
			if q.order > p.order {
				q.order += n;
			}
		}
		self.order += n;
		let mut names = Vec::new();
		let mut held = 0;
		for value in parts {
			let (effect, memory) = self.part_reads(&value);
			if !effect && !memory {
				names.push(value);
				continue;
			}
			let mut reads = Vec::new();
			reads_of(&value, &mut reads);
			let name = self.b.fresh("part");
			names.push(Expr::ident(&name));
			self.pending.insert(
				i + held,
				Pending {
					name,
					value,
					order: p.order + 1 + held as u32,
					effect,
					memory,
					parts: None,
					reads,
					copy: false,
					decl: true,
					rereads: false,
				},
			);
			held += 1;
		}
		Some(names)
	}

	/// The operand `field` of the aggregate `local` held back for its one read, held back
	/// in the aggregate's place, which is never made. Its other operands are dropped, so
	/// only when they do nothing. A reference that reads the same later is read in place.
	pub(super) fn held_part(&mut self, local: Local, field: usize) -> Option<Expr> {
		let name = self.var(local)?;
		let i = self.pending.iter().position(|p| p.name == name && p.parts.is_some())?;
		let parts = self.pending[i].parts.as_ref().expect("checked above");
		let (part, rereadable) = parts.get(field)?.clone();
		let others = parts.iter().enumerate().filter(|&(j, _)| j != field);
		if others.clone().any(|(_, (v, _))| self.part_reads(v).0) {
			return None;
		}
		let (effect, memory) = self.part_reads(&part);
		if !effect && !memory || rereadable {
			self.pending.remove(i);
			return Some(part);
		}
		let name = self.b.fresh("part");
		let p = &mut self.pending[i];
		p.reads.clear();
		reads_of(&part, &mut p.reads);
		p.name = name.clone();
		p.value = part;
		p.effect = effect;
		p.memory = memory;
		p.parts = None;
		p.decl = true;
		Some(Expr::ident(name))
	}

	/// Whether evaluating the aggregate part `value` can do or throw anything, and whether
	/// it reads memory something else could change.
	fn part_reads(&self, value: &Expr) -> (bool, bool) {
		let mut events = Vec::new();
		scan(value, &mut events, false);
		let effect = events.iter().any(|r| matches!(r, Read::Effect | Read::Panic));
		let memory = events.iter().any(|r| match r {
			Read::Memory => true,
			Read::Field(name) => !self.frozen_names.contains(name.as_str()),
			Read::Ident(name, _) => self.shared.contains(name.as_str()),
			Read::Effect | Read::Panic => false,
		});
		(effect, memory)
	}

	/// Assign `value` to the local `local` now, or hold it back.
	fn set_local(
		&mut self,
		local: Local,
		value: Expr,
		effect: bool,
		memory: bool,
		parts: Option<Vec<bool>>,
		out: &mut Vec<Stmt>,
	) {
		if self.stepped_vars.contains_key(&local) {
			self.set_stepped(local, value, out);
			return;
		}
		match self.slots[local].clone() {
			Slot::Var(name) => {
				if self.uses[local] == 0 {
					// Nothing reads it: only its effects run.
					if effect {
						self.emit(vec![StmtKind::Expr(value).at(None)], false, out);
					}
					return;
				}
				// An empty array takes its type from its declaration, as nothing in it says.
				if self.foldable(local) && !matches!(step::peel(&value), Expr::Array(items) if items.is_empty()) {
					// A borrow rereads the same by what it borrows, judged before what that
					// reads that is held back folds into it; anything else only as a constant
					// or a name that stays put.
					let borrow = self
						.def_rvalue(local)
						.is_some_and(|rvalue| matches!(rvalue, Rvalue::Ref(..)) && self.rvalue_rereads(rvalue));
					// What it reads that is held back folds into it first, so nothing held
					// back reads another, and its effects and reads go with it.
					let mut reads = Vec::new();
					reads_of(&value, &mut reads);
					let (effect, memory) = self
						.pending
						.iter()
						.filter(|p| reads.contains(&p.name))
						.fold((effect, memory), |(e, m), p| (e || p.effect, m || p.memory));
					let mut folded = self.fold_exprs(Vec::new(), vec![value], true, out);
					let value = folded.pop().unwrap();
					let parts =
						parts.and_then(|rereadable| Some(parts_of(&value)?.into_iter().zip(rereadable).collect()));
					let mut bare = &value;
					while let Expr::Located(_, e) = bare {
						bare = e;
					}
					let rereads = borrow || value.is_constant() || self.stable_name(bare);
					self.hold(local, value, effect, memory, parts, rereads);
					return;
				}
				self.assigned[local] = true;
				let stmt = StmtKind::Expr(Expr::ident(&name).assign(value)).at(None);
				self.emit(vec![stmt], false, out);
				// An aggregate assigned once and never changed: its operands, as emitted,
				// read in place of its fields.
				if parts.is_some()
					&& self.parts_read(local)
					&& let Some(Stmt {
						kind: StmtKind::Expr(Expr::Assign(_, value)),
						..
					}) = out.last()
					&& let Some(parts) = parts_of(value)
				{
					self.var_parts.insert(local, parts);
				}
			}
			Slot::Unit => {
				if effect {
					self.emit(vec![StmtKind::Expr(value).at(None)], false, out);
				}
			}
			Slot::Alias(_) | Slot::Discr(_) | Slot::Flag(_) => unreachable!("set by their own assignment"),
		}
	}

	// Blocks

	fn block(&mut self, bb: BasicBlock, out: &mut Vec<Stmt>) -> Exit {
		let mir = self.mir;
		let data = &mir.basic_blocks[bb];
		for (i, stmt) in data.statements.iter().enumerate() {
			if !self.in_ignored(stmt.source_info.span) && !self.noops.contains(&(bb, i)) {
				self.at = mir::Location {
					block: bb,
					statement_index: i,
				};
				self.statement(stmt, out);
			}
		}
		if let Some(&to) = self.redirect.get(&bb) {
			return Exit::Goto(to.as_usize());
		}
		self.at = mir::Location {
			block: bb,
			statement_index: data.statements.len(),
		};
		self.terminator(data.terminator(), out)
	}

	fn statement(&mut self, stmt: &mir::Statement<'tcx>, out: &mut Vec<Stmt>) {
		let span = stmt.source_info.span;
		match &stmt.kind {
			StatementKind::Assign(assign) => {
				let (place, rvalue) = &**assign;
				// Whichever way the reference lowers, as an alias of the place or a value.
				if let Rvalue::Ref(_, _, borrowed) = rvalue
					&& let Some(why) = self.lent_out_of_step(*borrowed)
				{
					self.err(span, why);
				}
				self.assign(*place, rvalue, span, out);
			}
			StatementKind::SetDiscriminant { place, variant_index } => {
				let target = self.set_discriminant(**place, *variant_index, span);
				if let Some(stmt) = target {
					self.emit(vec![stmt], false, out);
				}
			}
			StatementKind::Intrinsic(intrinsic) => match &**intrinsic {
				mir::NonDivergingIntrinsic::Assume(_) => {}
				mir::NonDivergingIntrinsic::CopyNonOverlapping(_) => {
					self.err(span, "`copy_nonoverlapping` is not supported");
				}
			},
			StatementKind::FakeRead(..)
			| StatementKind::StorageLive(_)
			| StatementKind::StorageDead(_)
			| StatementKind::PlaceMention(_)
			| StatementKind::AscribeUserType(..)
			| StatementKind::Coverage(_)
			| StatementKind::ConstEvalCounter
			| StatementKind::Nop
			| StatementKind::BackwardIncompatibleDropHint { .. } => {}
		}
	}

	fn assign(&mut self, place: Place<'tcx>, rvalue: &Rvalue<'tcx>, span: Span, out: &mut Vec<Stmt>) {
		if let Some(local) = place.as_local() {
			if self.joined.contains(&local) {
				return;
			}
			if self.scalars.contains_key(&local) {
				self.scalar_assign(local, rvalue, span, out);
				return;
			}
			match self.slots[local] {
				Slot::Alias(_) => {
					let target = match rvalue {
						Rvalue::Ref(_, kind, p) => match self.mut_subslice(*kind, *p, span, out) {
							// One view, however many times it is read.
							Some(e) => self.path(e, "view", out),
							None => match self.reborrowed(*kind, *p, span, out) {
								Some(e) => e,
								// Read whole, the place is held as it is now for its `rt.Place`.
								None if self.points_at_place(local) && self.read_whole(local) => {
									let target = self.place_expr(*p, span, out);
									self.stable_target(*p, target, span, out)
								}
								None => self.place_expr(*p, span, out),
							},
						},
						// A copy of a `&mut` captured by reference borrows the same place.
						Rvalue::CopyForDeref(p) | Rvalue::Use(Operand::Copy(p) | Operand::Move(p), _)
							if let Some(target) = self.pointee_capture(*p) =>
						{
							target
						}
						Rvalue::CopyForDeref(p) => self.place_expr(*p, span, out),
						Rvalue::Use(Operand::Copy(p) | Operand::Move(p), _) => self.place_expr(*p, span, out),
						Rvalue::Cast(_, Operand::Copy(p) | Operand::Move(p), _) => self.place_expr(*p, span, out),
						Rvalue::Use(op @ Operand::Constant(_), _) => self.operand(op, span, out),
						// The task's context, wrapped.
						Rvalue::Aggregate(_, ops) => match &ops[rustc_abi::FieldIdx::ZERO] {
							Operand::Copy(p) | Operand::Move(p) => self.place_expr(*p, span, out),
							_ => unreachable!("checked by context_of_arg"),
						},
						_ => unreachable!("an alias is a borrow or a copy of one"),
					};
					// Read once, it folds in what it reads that is held back, like any other read.
					if self.uses[local] > 1 {
						self.materialize(&target, out);
					}
					self.slots[local] = Slot::Alias(Some(target));
					return;
				}
				Slot::Discr(_) => {
					let Rvalue::Discriminant(p) = rvalue else {
						unreachable!("checked by discr_only")
					};
					let ty = p.ty(&self.mir.local_decls, self.tcx).ty;
					let target = self.place_expr(*p, span, out);
					// A switch whose other variants can't exist tests nothing, so the read
					// runs only for its effects.
					let target = match self.discr_tested(local) {
						true => self.path(target, "scrutinee", out),
						false => {
							let target = self.fold_exprs(Vec::new(), vec![target], false, out).remove(0);
							if self.part_reads(&target).0 {
								self.emit(vec![StmtKind::Expr(target).at(None)], false, out);
							}
							Expr::undefined()
						}
					};
					self.slots[local] = Slot::Discr(Some((target, ty)));
					return;
				}
				Slot::Flag(_) => {
					let Rvalue::Discriminant(p) = rvalue else {
						unreachable!("checked by scalar_discr")
					};
					let flag = self.scalar_flag(*p, span);
					self.slots[local] = Slot::Flag(Some(flag));
					return;
				}
				_ => {}
			}
		}
		let mut pre = Vec::new();
		// A copy into a local nothing changes in place can share what it copies, if that
		// doesn't change either. One into a temporary only an argument reads shares it as
		// long as it folds into the argument, before anything could change it, if the callee
		// can't change it either (a local's own value nothing borrows, or a callee given
		// nothing that changes memory) nor return it.
		self.into_frozen = place.as_local().is_some_and(|l| !self.mutated[l]);
		let shares = place.as_local().is_some_and(|l| {
			self.arg_use[l]
				&& !self.arg_kept[l]
				&& self.foldable(l)
				&& matches!(rvalue, Rvalue::Use(Operand::Copy(p), _)
					if !self.frozen(*p)
						&& (own(*p) && !self.borrowed[p.local] || self.arg_alone[l])
						&& self.b.t.needs_copy(p.ty(&self.mir.local_decls, self.tcx).ty))
		});
		if shares {
			self.into_frozen = true;
			self.force_share = true;
		}
		let value = self.rvalue(rvalue, place, span, &mut pre);
		let value = match (value, place.as_local()) {
			(Some(value), Some(local)) if self.stepped.contains(&local) => Some(self.stepped_make(value, span)),
			(value, _) => value,
		};
		self.into_frozen = false;
		self.force_share = false;
		self.copy_on_flush = shares;
		let Some(value) = value else {
			out.extend(pre);
			return;
		};
		let value = self.widen(place, rvalue, value, span);
		// What an `Iterator::next` returns, by the protocol.
		let value = match (
			self.next && place.local == mir::RETURN_PLACE && place.projection.is_empty(),
			rvalue,
		) {
			(false, _) => value,
			(true, Rvalue::Aggregate(kind, _)) if matches!(**kind, mir::AggregateKind::Adt(_, v, ..) if v.as_u32() == 0) => {
				Expr::Rt("DONE".into())
			}
			(true, Rvalue::Aggregate(..)) => value,
			(true, Rvalue::Use(Operand::Move(p) | Operand::Copy(p), _))
				if p.as_local().is_some_and(|l| self.next_results.contains(&l)) =>
			{
				value
			}
			(true, _) => self.done_of(value),
		};
		let effect = rvalue::effect(rvalue, self);
		// A copy shared until it folds into its argument reads its source there.
		let memory = rvalue::memory(rvalue, self) || shares;
		let parts = self.rereadable_parts.take();
		if let Some(local) = place.as_local() {
			let parts = match rvalue {
				Rvalue::Aggregate(..) => parts,
				_ => None,
			};
			if !pre.is_empty() {
				self.emit(pre, false, out);
			}
			self.set_local(local, value, effect, memory, parts, out);
			return;
		}
		if !pre.is_empty() {
			self.emit(pre, false, out);
		}
		let stmts = self.store(place, value, span);
		self.emit(stmts, false, out);
	}

	/// Hold `value` in a path that reads it again for free.
	pub fn path(&mut self, value: Expr, base: &str, out: &mut Vec<Stmt>) -> Expr {
		if value.is_path() {
			self.materialize(&value, out);
			return value;
		}
		let name = self.b.fresh(base);
		let stmt = StmtKind::decl(false, name.clone(), None, Some(value)).at(None);
		self.emit(vec![stmt], false, out);
		Expr::ident(name)
	}

	/// Hold `slice`, an `rt.ByteSlice` to read more than once, in a path, as a tuple's part
	/// already is.
	pub(super) fn slice_path(&mut self, slice: Expr, out: &mut Vec<Stmt>) -> Expr {
		match &slice {
			// A local's parts, which do nothing.
			_ if place::made_slice(&slice).is_some() => slice,
			Expr::Index(base, i) if base.is_path() && i.is_constant() => {
				self.materialize(&slice, out);
				slice
			}
			_ => self.path(slice, "slice", out),
		}
	}

	/// The first `n` bytes of `slice`, each read where it is: from the slice it is a range
	/// of, if it is one at a constant offset.
	pub(super) fn bytes_at(&mut self, slice: Expr, n: u64, out: &mut Vec<Stmt>) -> Vec<Expr> {
		let range = match &slice {
			Expr::Call(f, args) if matches!(&**f, Expr::Rt(name) if name == "byteSliceRange") => {
				match args.as_slice() {
					[base, Expr::Lit(from), ..] => from.parse::<u64>().ok().map(|from| (base.clone(), from)),
					_ => None,
				}
			}
			_ => None,
		};
		let (base, from) = range.unwrap_or((slice, 0));
		let base = self.slice_path(base, out);
		(from..from + n)
			.map(|i| place::byte_at(base.clone(), Expr::lit(i.to_string())))
			.collect()
	}

	fn terminator(&mut self, term: &mir::Terminator<'tcx>, out: &mut Vec<Stmt>) -> Exit {
		let span = term.source_info.span;
		match &term.kind {
			TerminatorKind::Goto { target, .. } => Exit::Goto(target.as_usize()),
			TerminatorKind::FalseEdge { real_target, .. } | TerminatorKind::FalseUnwind { real_target, .. } => {
				Exit::Goto(real_target.as_usize())
			}
			TerminatorKind::Return if !self.mir.local_decls[mir::RETURN_PLACE].ty.is_never() => {
				let ret = self.mir.local_decls[mir::RETURN_PLACE].ty;
				let value = match self.zero_sized(ret) {
					true => None,
					false => {
						let mut pre = Vec::new();
						// Uncopied: a caller that changes the result in place copies it.
						let value = self.place_expr(Place::return_place(), span, &mut pre);
						out.extend(pre);
						Some(value)
					}
				};
				let loc = self.b.t.loc(span);
				self.emit(vec![StmtKind::Return(value).at(loc)], false, out);
				self.flush(out);
				Exit::End
			}
			// A `Return` from a function of type `!`, as a generic instantiated at `!` is, is
			// unreachable too: it has no value to return, and `tsc` must see that.
			TerminatorKind::Unreachable | TerminatorKind::Return => {
				self.emit(
					vec![StmtKind::Expr(Expr::rt("unreachable", vec![Expr::undefined()])).at(None)],
					false,
					out,
				);
				Exit::End
			}
			TerminatorKind::SwitchInt { discr, targets } => self.switch(discr, targets, span, out),
			TerminatorKind::Drop { place, target, .. } => {
				let ty = place.ty(&self.mir.local_decls, self.tcx).ty;
				// An `FnOnce` body drops its closure whole, which is what it captured.
				if let Env::Closure(captures) = &self.env
					&& place.local.as_u32() == 1
					&& place.projection.is_empty()
					&& matches!(ty.kind(), ty::Closure(..))
				{
					let mut glue = self.b.t.closure_glue(ty, &captures.clone(), span);
					// In a block of its own, whose variable no other drop's can clash with.
					if glue.iter().any(|s| matches!(s.kind, StmtKind::Decl { .. })) {
						glue = vec![
							StmtKind::Block {
								label: None,
								body: glue,
							}
							.at(None),
						];
					}
					self.emit(glue, false, out);
				} else if self.b.t.drops(ty) {
					let mut pre = Vec::new();
					let value = self.place_expr(*place, span, &mut pre);
					let dropped = match place.as_local().is_some_and(|l| self.stepped.contains(&l)) {
						true => self.stepped_drop(&value, ty, span),
						false => self.b.t.drop_value(value, ty, span),
					};
					if let Some(dropped) = dropped {
						pre.push(StmtKind::Expr(dropped).at(None));
					}
					self.emit(pre, false, out);
				}
				Exit::Goto(target.as_usize())
			}
			TerminatorKind::Call {
				func,
				args,
				destination,
				target,
				fn_span,
				..
			} => {
				self.call(func, args, *destination, *fn_span, out);
				match target {
					Some(target) => Exit::Goto(target.as_usize()),
					None => {
						self.flush(out);
						Exit::End
					}
				}
			}
			TerminatorKind::Assert {
				cond,
				expected,
				msg,
				target,
				..
			} => {
				self.assert(cond, *expected, msg, span, out);
				match fails(term) {
					true => {
						self.flush(out);
						Exit::End
					}
					false => Exit::Goto(target.as_usize()),
				}
			}
			TerminatorKind::TailCall { .. } => {
				self.err(span, "`become` is not supported");
				Exit::End
			}
			TerminatorKind::InlineAsm { .. } => {
				self.err(span, "inline assembly is not supported");
				Exit::End
			}
			TerminatorKind::Yield { .. } | TerminatorKind::CoroutineDrop => {
				self.err(span, "a coroutine before its state transform");
				Exit::End
			}
			TerminatorKind::UnwindResume | TerminatorKind::UnwindTerminate(_) => {
				self.err(span, "unwinding, though a panic aborts");
				Exit::End
			}
		}
	}

	fn switch(&mut self, discr: &Operand<'tcx>, targets: &mir::SwitchTargets, span: Span, out: &mut Vec<Stmt>) -> Exit {
		let (arms, default) = self.switch_targets(targets);
		let discr_local = match discr {
			Operand::Copy(p) | Operand::Move(p) => p.as_local(),
			_ => None,
		};
		// A variant TypeScript may have narrowed the place away from is tested on the
		// place's whole type.
		let whole = discr_local
			.filter(|&l| {
				self.def_at[l].is_some_and(|(bb, i)| {
					self.narrowing.widened(mir::Location {
						block: bb,
						statement_index: i,
					})
				})
			})
			.map(|l| self.scrutinee_ty(l, span));
		let widen = |test: Expr, read: &Expr| match (&whole, read.path_key()) {
			(Some(whole), Some(key)) => widen_reads(test, &key, whole),
			_ => test,
		};
		let tests: Vec<(Expr, BasicBlock)> = match discr_local.map(|l| self.slots[l].clone()) {
			Some(Slot::Discr(Some((scrut, ty)))) => {
				if !arms.is_empty() && whole.is_none() {
					self.switched.extend(scrut.path_key().map(|key| (key, ty)));
				}
				let tests: Vec<Expr> = arms
					.iter()
					.map(|&(value, _)| {
						let test = self.variant_test(&scrut, ty, value, span);
						widen(test, &scrut)
					})
					.collect();
				// What one test reads that is held back folds into it; of several, all but
				// the first may not run.
				let conds = match tests.len() {
					1 => self.fold_exprs(Vec::new(), tests, false, out),
					_ => tests,
				};
				conds.into_iter().zip(arms.iter().map(|&(_, t)| t)).collect()
			}
			// The discriminants of `Option`, `Result`, and `ControlFlow` are their variants'.
			Some(Slot::Flag(Some(flag))) => {
				let tests: Vec<Expr> = arms
					.iter()
					.map(|&(value, _)| match value {
						0 => flag.clone().not(),
						_ => flag.clone(),
					})
					.collect();
				let conds = match tests.len() {
					1 => self.fold_exprs(Vec::new(), tests, false, out),
					_ => tests,
				};
				conds.into_iter().zip(arms.iter().map(|&(_, t)| t)).collect()
			}
			_ => {
				let ty = discr.ty(&self.mir.local_decls, self.tcx);
				let mut pre = Vec::new();
				let value = self.operand(discr, span, &mut pre);
				let value = match arms.len() {
					0 | 1 => value,
					_ => match value.is_path() {
						true => value,
						false => {
							let name = self.b.fresh("value");
							pre.push(StmtKind::decl(false, name.clone(), None, Some(value)).at(None));
							Expr::ident(name)
						}
					},
				};
				let mut tests = Vec::new();
				for &(v, target) in &arms {
					let test = match ty.kind() {
						ty::Bool => match v {
							0 => value.clone().not(),
							_ => value.clone(),
						},
						ty::Char => value.clone().binary("===", Expr::lit(v.to_string())),
						_ => match super::num::Int::of(ty) {
							Some(int) => {
								let lit = int.lit(int.fold(v as i128));
								int.compare("===", value.clone(), lit)
							}
							None => self.err(span, format!("a switch on `{ty}`")),
						},
					};
					tests.push((widen(test, &value), target));
				}
				// What the tests read that is held back folds into them.
				let targets: Vec<BasicBlock> = tests.iter().map(|(_, t)| *t).collect();
				let conds = self.fold_exprs(pre, tests.into_iter().map(|(c, _)| c).collect(), false, out);
				conds.into_iter().zip(targets).collect()
			}
		};
		self.flush(out);
		// Arms to one target are one test.
		let mut grouped: Vec<(Expr, BasicBlock)> = Vec::new();
		for (cond, target) in tests {
			match grouped.iter_mut().find(|(_, t)| *t == target) {
				Some((c, _)) => *c = std::mem::replace(c, Expr::undefined()).binary("||", cond),
				None => grouped.push((cond, target)),
			}
		}
		let tests: Vec<(Expr, usize)> = grouped.into_iter().map(|(c, t)| (fold(c), t.as_usize())).collect();
		match tests.is_empty() {
			true => Exit::Goto(default.as_usize()),
			false => Exit::Branch(tests, default.as_usize()),
		}
	}

	fn assert(
		&mut self,
		cond: &Operand<'tcx>,
		expected: bool,
		msg: &mir::AssertMessage<'tcx>,
		span: Span,
		out: &mut Vec<Stmt>,
	) {
		// Checked arithmetic already panicked with this message as it made its value.
		if let Operand::Move(p) | Operand::Copy(p) = cond
			&& p.projection.len() == 1
			&& self.checked.contains(&p.local)
		{
			return;
		}
		if redundant(msg) {
			return;
		}
		if let mir::AssertKind::BoundsCheck { len, index } = msg
			&& let Operand::Copy(l) | Operand::Move(l) = len
			&& let Some(&slice) = l.as_local().and_then(|l| self.idx_of.get(&l))
		{
			let mut pre = Vec::new();
			let slice = self.place_expr(slice, span, &mut pre);
			let index = self.operand(index, span, &mut pre);
			// A slice held as its parts is tested by its length, so none is made to test.
			if let Some([_, _, length]) = place::made_slice(&slice) {
				let length = length.clone();
				let index = self.path(index, "index", &mut pre);
				let failed = fold(index.clone().binary(">=", length.clone()));
				if !matches!(&failed, Expr::Lit(t) if t == "false")
					&& let Some(panic) = self.assert_panic(msg, vec![length, index], span)
				{
					pre.push(
						StmtKind::If {
							cond: failed,
							then: vec![StmtKind::Expr(panic).at(None)],
							els: Vec::new(),
						}
						.at(self.b.t.loc(span)),
					);
				}
				self.emit(pre, false, out);
				return;
			}
			pre.push(StmtKind::Expr(Expr::rt("idx", vec![slice, index])).at(self.b.t.loc(span)));
			self.emit(pre, false, out);
			return;
		}
		let mut pre = Vec::new();
		let value = self.operand(cond, span, &mut pre);
		let mut exprs = vec![value];
		if let mir::AssertKind::BoundsCheck { len, index } = msg {
			exprs.push(self.operand(len, span, &mut pre));
			exprs.push(self.operand(index, span, &mut pre));
		}
		let mut folded = self.fold_exprs(pre, exprs, false, out).into_iter();
		let value = fold(folded.next().unwrap());
		let operands: Vec<Expr> = folded.collect();
		let mut pre = Vec::new();
		let failed = fold(match expected {
			true => value.not(),
			false => value,
		});
		// An assert of a constant that holds, such as of a shift by a literal.
		if matches!(&failed, Expr::Lit(t) if t == "false") {
			return;
		}
		let panic = match self.assert_panic(msg, operands, span) {
			Some(panic) => panic,
			None => return,
		};
		// An assert that always fails, such as a coroutine's after it returned.
		if matches!(&failed, Expr::Lit(t) if t == "true") {
			pre.push(StmtKind::Expr(panic).at(self.b.t.loc(span)));
			self.emit(pre, false, out);
			return;
		}
		pre.push(
			StmtKind::If {
				cond: failed,
				then: vec![StmtKind::Expr(panic).at(None)],
				els: Vec::new(),
			}
			.at(self.b.t.loc(span)),
		);
		self.emit(pre, false, out);
	}

	/// The panic of an assert that fails: the runtime's own function for a coroutine
	/// resumed out of turn, as rustc calls a lang item, else `rt.panic`.
	fn assert_panic(&mut self, msg: &mir::AssertMessage<'tcx>, operands: Vec<Expr>, span: Span) -> Option<Expr> {
		use mir::AssertKind;
		use rustc_hir::{CoroutineDesugaring, CoroutineKind};
		let async_fn = |kind: &CoroutineKind| matches!(kind, CoroutineKind::Desugared(CoroutineDesugaring::Async, _));
		match msg {
			AssertKind::ResumedAfterReturn(kind) if async_fn(kind) => Some(Expr::rt("asyncFnResumed", Vec::new())),
			AssertKind::ResumedAfterPanic(kind) if async_fn(kind) => Some(Expr::rt("asyncFnResumedPanic", Vec::new())),
			_ => {
				let message = self.assert_message(msg, operands, span)?;
				Some(Expr::rt("panic", vec![message]))
			}
		}
	}

	/// The message an assert panics with, as the runtime spells it.
	fn assert_message(&mut self, msg: &mir::AssertMessage<'tcx>, operands: Vec<Expr>, span: Span) -> Option<Expr> {
		use mir::AssertKind;
		use rustc_middle::mir::BinOp;
		let text = |s: &str| Some(Expr::Str(s.to_string()));
		match msg {
			AssertKind::BoundsCheck { .. } => {
				let [len, index] = <[Expr; 2]>::try_from(operands).ok()?;
				Some(
					Expr::Str("index out of bounds: the len is ".into())
						.binary("+", len)
						.binary("+", Expr::Str(" but the index is ".into()))
						.binary("+", index),
				)
			}
			AssertKind::Overflow(op, ..) => match op {
				BinOp::Add => text("attempt to add with overflow"),
				BinOp::Sub => text("attempt to subtract with overflow"),
				BinOp::Mul => text("attempt to multiply with overflow"),
				BinOp::Div => text("attempt to divide with overflow"),
				BinOp::Rem => text("attempt to calculate the remainder with overflow"),
				BinOp::Shl => text("attempt to shift left with overflow"),
				BinOp::Shr => text("attempt to shift right with overflow"),
				_ => {
					self.err(span, format!("an overflow check of `{op:?}`"));
					None
				}
			},
			AssertKind::OverflowNeg(_) => text("attempt to negate with overflow"),
			AssertKind::DivisionByZero(_) => text("attempt to divide by zero"),
			AssertKind::RemainderByZero(_) => text("attempt to calculate the remainder with a divisor of zero"),
			_ => {
				self.err(span, format!("an assert of `{msg:?}`"));
				None
			}
		}
	}

	/// Emit every pending temporary, in order.
	fn flush_pending(&mut self, out: &mut Vec<Stmt>) {
		for p in std::mem::take(&mut self.pending) {
			self.flush_one(p, out);
		}
	}
}

impl cfg::Lower for Fx<'_, '_, '_, '_> {
	fn node(&mut self, node: usize, out: &mut Vec<Stmt>) -> Exit {
		let exit = self.block(BasicBlock::from_usize(node), out);
		let forward = |to: usize| self.forward(BasicBlock::from_usize(to)).as_usize();
		match exit {
			Exit::Goto(to) => Exit::Goto(forward(to)),
			Exit::Branch(arms, default) => Exit::Branch(
				arms.into_iter().map(|(cond, to)| (cond, forward(to))).collect(),
				forward(default),
			),
			Exit::End => Exit::End,
		}
	}

	fn flush(&mut self, out: &mut Vec<Stmt>) {
		self.flush_pending(out);
	}
}

impl Fx<'_, '_, '_, '_> {
	fn flush(&mut self, out: &mut Vec<Stmt>) {
		cfg::Lower::flush(self, out);
	}
}

impl<'tcx> Fx<'_, '_, '_, 'tcx> {
	/// Whether a value of `ty` can change a `part` in place: it holds a raw pointer, or a
	/// mutable reference to or interior mutability around something that could hold a
	/// `part` or be part of one.
	fn changes(&self, ty: Ty<'tcx>, part: Ty<'tcx>) -> bool {
		use rustc_middle::ty::TypeVisitableExt;
		let env = self.b.t.typing_env_pub();
		let overlaps = |t: Ty<'tcx>| self.b.t.holds(t, part) || self.b.t.holds(part, t);
		self.b.t.reaches(ty, |t| match t.kind() {
			ty::RawPtr(..) => true,
			ty::Ref(_, inner, rustc_ast::Mutability::Mut) => overlaps(*inner),
			// Inside a binder, as a function pointer's signature, only its kind tells.
			_ => !t.has_escaping_bound_vars() && !t.is_freeze(self.tcx, env) && overlaps(t),
		})
	}

	/// Whether a call given `args`, of std's code unless `translated`, may change what it
	/// takes by value of type `part` in place after its source gave it: a value with no
	/// copy holding a `Copy` aggregate, whatever the callee, or a `Copy` aggregate std
	/// keeps, or keeps a part of, in its result or where another argument reaches, as
	/// `Vec::push` keeps it in the vector. The crate's own code copies a `Copy` aggregate
	/// it keeps itself.
	pub(super) fn keeps(
		&self,
		translated: bool,
		args: &[Spanned<Operand<'tcx>>],
		part: Ty<'tcx>,
		result: Ty<'tcx>,
	) -> bool {
		if self.moves_copied(part) {
			return true;
		}
		!translated
			&& self.copied_parts(part).into_iter().any(|t| {
				self.holds_behind(result, t)
					|| args
						.iter()
						.any(|a| self.b.t.stores(a.node.ty(&self.mir.local_decls, self.tcx), t))
			})
	}

	/// The local whose own value `op` moves out, which nothing reads after, if it holds a
	/// `Copy` aggregate: by `move`, or by `copy` of a type that isn't `Copy`, as optimized
	/// MIR moves too.
	pub(super) fn moved_from(&self, op: &Operand<'tcx>) -> Option<Local> {
		let (Operand::Copy(p) | Operand::Move(p)) = op else {
			return None;
		};
		let ty = p.ty(&self.mir.local_decls, self.tcx).ty;
		let moves = matches!(op, Operand::Move(_)) || !self.b.t.is_copy(ty);
		(moves && own(*p) && self.holds_copied(ty)).then_some(p.local)
	}

	/// Whether a value of `ty` has no copy to make but holds a `Copy` aggregate by value,
	/// so wherever it moves can change what it holds, which must then be its own.
	fn moves_copied(&self, ty: Ty<'tcx>) -> bool {
		!self.b.t.is_copy(ty) && self.holds_copied(ty)
	}

	/// Whether a value of `ty` is or holds by value a `Copy` aggregate.
	fn holds_copied(&self, ty: Ty<'tcx>) -> bool {
		!self.copied_parts(ty).is_empty()
	}

	/// The `Copy` aggregates, JS objects Rust code copies, that a value of `ty` is or holds
	/// by value, not behind a reference, in any of which std could take it apart.
	fn copied_parts(&self, ty: Ty<'tcx>) -> Vec<Ty<'tcx>> {
		let mut parts = Vec::new();
		let mut seen = FxHashSet::default();
		let mut stack = vec![ty];
		while let Some(ty) = stack.pop() {
			if !seen.insert(ty) {
				continue;
			}
			if self.b.t.needs_copy(ty) {
				parts.push(ty);
			}
			match ty.kind() {
				ty::Tuple(tys) => stack.extend(tys.iter()),
				ty::Array(elem, _) | ty::Slice(elem) => stack.push(*elem),
				ty::Adt(adt, args) if self.b.t.own(adt.did()) => {
					stack.extend(adt.all_fields().map(|field| self.b.t.field_ty(field.did, args)))
				}
				ty::Adt(_, args) => stack.extend(args.types()),
				ty::Closure(_, args) => stack.extend(args.as_closure().upvar_tys()),
				_ => {}
			}
		}
		parts
	}

	/// Whether `func` is a function the crate translates, rather than std's.
	pub(super) fn translated(&self, func: &Operand<'tcx>) -> bool {
		match *func.ty(&self.mir.local_decls, self.tcx).kind() {
			ty::FnDef(def_id, args) => self.b.t.translated(def_id, args.skip_binder()),
			_ => false,
		}
	}

	/// Whether a value of `ty` could hold a `part` where changing it in place changes no
	/// local holding the value: behind a box, a reference, or a collection. One it holds
	/// in a field or a variant changes in place only as its local does, which copies it.
	fn holds_behind(&self, ty: Ty<'tcx>, part: Ty<'tcx>) -> bool {
		if ty == part {
			return false;
		}
		use crate::lower::library::Std;
		match ty.kind() {
			ty::Tuple(tys) => tys.iter().any(|t| self.holds_behind(t, part)),
			ty::Array(elem, _) => self.holds_behind(*elem, part),
			ty::Adt(adt, args)
				if self.b.t.own(adt.did())
					|| matches!(
						self.b.t.std_of(ty),
						Some(Std::Option | Std::Result | Std::ControlFlow | Std::Poll)
					) =>
			{
				adt.all_fields()
					.any(|field| self.holds_behind(self.b.t.field_ty(field.did, args), part))
			}
			_ => self.b.t.holds(ty, part),
		}
	}
}

/// `value` as a value of `ty`, its JS union type: `Ok` of it, or `Err` of an `Ok` or `Err`,
/// which TypeScript types as the whole union already.
pub(super) fn whole(value: Expr, ty: Type) -> Result<Expr, Expr> {
	let mut inner = &value;
	while let Expr::Located(_, e) = inner {
		inner = e;
	}
	match inner {
		Expr::As(_, t) if *t == ty => Ok(value),
		Expr::Call(f, _) if matches!(&**f, Expr::Rt(n) if n == "Ok" || n == "Err") => Err(value),
		_ => Ok(Expr::As(Box::new(value.typed()), ty)),
	}
}

impl<'tcx> Translator<'_, 'tcx> {
	/// Whether a value of `ty` reaches somewhere that could take a `part` in: a raw
	/// pointer, or a mutable reference to or interior mutability around what could hold one.
	pub(crate) fn stores(&self, ty: Ty<'tcx>, part: Ty<'tcx>) -> bool {
		use rustc_middle::ty::TypeVisitableExt;
		let env = self.typing_env_pub();
		self.reaches(ty, |t| match t.kind() {
			ty::RawPtr(..) => true,
			ty::Ref(_, inner, rustc_ast::Mutability::Mut) => self.holds(*inner, part),
			_ => !t.has_escaping_bound_vars() && !t.is_freeze(self.tcx, env) && self.holds(t, part),
		})
	}

	/// Whether a value of `ty` could hold a `part`: holds its type, or hides what it holds.
	pub(crate) fn holds(&self, ty: Ty<'tcx>, part: Ty<'tcx>) -> bool {
		// An array and a slice of its element share storage through an unsizing coercion.
		let storage = |t: Ty<'tcx>| match t.kind() {
			ty::Array(elem, _) => Ty::new_slice(self.tcx, *elem),
			_ => t,
		};
		let part = storage(part);
		self.reaches(ty, |t| {
			storage(t) == part
				|| matches!(
					t.kind(),
					ty::Dynamic(..) | ty::Alias(..) | ty::Param(_) | ty::Closure(..) | ty::Coroutine(..)
				)
		})
	}

	/// Whether `f` holds for any type a value of `ty` holds: its type arguments, which are
	/// what std's types hold, and the fields of the crate's own types.
	pub(crate) fn reaches(&self, ty: Ty<'tcx>, mut f: impl FnMut(Ty<'tcx>) -> bool) -> bool {
		let mut seen = FxHashSet::default();
		let mut stack = vec![ty];
		while let Some(ty) = stack.pop() {
			for t in ty.walk().filter_map(|arg| arg.as_type()) {
				if f(t) {
					return true;
				}
				if let ty::Adt(adt, args) = t.kind()
					&& self.own(adt.did())
					&& seen.insert(t)
				{
					stack.extend(adt.all_fields().map(|field| self.field_ty(field.did, args)));
				}
			}
		}
		false
	}
}

/// Whether `place` is part of its local's own value, rather than reached through a
/// reference or pointer.
pub(super) fn own(place: Place<'_>) -> bool {
	place
		.projection
		.iter()
		.all(|e| !matches!(e, mir::ProjectionElem::Deref))
}

/// What an expression reads, in evaluation order.
enum Read {
	/// A name, and whether it is read later or maybe not at all: in a function, or in a
	/// branch that may not run.
	Ident(String, bool),
	/// Memory: a member or an element.
	Memory,
	/// A member or an element of a local's own value, by its name.
	Field(String),
	/// A call, which may do anything.
	Effect,
	/// A call of the runtime's that reads its arguments only, and may panic.
	Panic,
}

fn scan_stmt(stmt: &Stmt, reads: &mut Vec<Read>, lazy: bool) {
	match &stmt.kind {
		StmtKind::Expr(e) | StmtKind::Return(Some(e)) => scan(e, reads, lazy),
		StmtKind::Decl { init: Some(e), .. } => scan(e, reads, lazy),
		StmtKind::If { cond, then, els } => {
			scan(cond, reads, lazy);
			then.iter().chain(els).for_each(|s| scan_stmt(s, reads, true));
		}
		StmtKind::While { cond, body, .. } => {
			scan(cond, reads, true);
			body.iter().for_each(|s| scan_stmt(s, reads, true));
		}
		StmtKind::ForOf { iter, body, .. } => {
			scan(iter, reads, lazy);
			body.iter().for_each(|s| scan_stmt(s, reads, true));
		}
		StmtKind::Block { body, .. } => body.iter().for_each(|s| scan_stmt(s, reads, lazy)),
		_ => {}
	}
}

/// A read of a member or element of `base`.
fn field(base: &Expr) -> Read {
	match base {
		Expr::Ident(name) => Read::Field(name.clone()),
		_ => Read::Memory,
	}
}

fn scan(e: &Expr, reads: &mut Vec<Read>, lazy: bool) {
	match e {
		Expr::Ident(name) => reads.push(Read::Ident(name.clone(), lazy)),
		Expr::This | Expr::Sym(_) | Expr::Rt(_) | Expr::Lit(_) | Expr::Str(_) => {}
		Expr::Array(items) => items.iter().for_each(|e| scan(e, reads, lazy)),
		Expr::Object(props) => props.iter().for_each(|(_, e)| scan(e, reads, lazy)),
		// The runtime's and the program's own items never change.
		Expr::Member(base, _) if matches!(&**base, Expr::Rt(_) | Expr::Sym(_)) => {}
		Expr::Member(base, _) => {
			scan(base, reads, lazy);
			reads.push(field(base));
		}
		Expr::Index(base, index) => {
			scan(base, reads, lazy);
			scan(index, reads, lazy);
			reads.push(field(base));
		}
		// A byte slice made of its parts only holds them.
		Expr::New(f, args) if matches!(&**f, Expr::Rt(name) if name == "ByteSlice") => {
			args.iter().for_each(|e| scan(e, reads, lazy))
		}
		Expr::Call(f, args) | Expr::New(f, args) => {
			// A method's lookup reads no data the arguments could change: only its receiver.
			match &**f {
				Expr::Member(recv, _) => scan(recv, reads, lazy),
				f => scan(f, reads, lazy),
			}
			args.iter().for_each(|e| scan(e, reads, lazy));
			reads.push(match &**f {
				Expr::Rt(name) if reads_only(name) => Read::Panic,
				_ => Read::Effect,
			});
		}
		Expr::Unary(_, e) | Expr::Located(_, e) | Expr::As(e, _) | Expr::Instantiate(e, _) => scan(e, reads, lazy),
		Expr::Postfix(e, _) => {
			scan(e, reads, lazy);
			reads.push(Read::Effect);
		}
		Expr::Binary(l, op, r) => {
			scan(l, reads, lazy);
			let lazy_r = lazy || matches!(*op, "&&" | "||" | "??");
			scan(r, reads, lazy_r);
		}
		Expr::Cond(c, a, b) => {
			scan(c, reads, lazy);
			scan(a, reads, true);
			scan(b, reads, true);
		}
		Expr::Assign(target, value) => {
			// A variable assigned is not read; a place's base and index are, before the value,
			// but not the place itself.
			match &**target {
				Expr::Ident(_) => {}
				Expr::Member(base, _) => scan(base, reads, lazy),
				Expr::Index(base, index) => {
					scan(base, reads, lazy);
					scan(index, reads, lazy);
				}
				target => scan(target, reads, lazy),
			}
			scan(value, reads, lazy);
			reads.push(Read::Effect);
		}
		Expr::Arrow(_, _, body) => body.iter().for_each(|s| scan_stmt(s, reads, true)),
		Expr::Lambda(_, body) => scan(body, reads, true),
	}
}

/// Whether the runtime's function `name` only reads its arguments, if it may panic: a
/// bounds check, or integer arithmetic or a comparison.
fn reads_only(name: &str) -> bool {
	if matches!(name, "idx" | "at" | "min" | "max" | "lt") {
		return true;
	}
	let Some(op) = [
		"usize", "isize", "u128", "i128", "u16", "i16", "u32", "i32", "u64", "i64", "u8", "i8",
	]
	.iter()
	.find_map(|ty| name.strip_prefix(ty)) else {
		return false;
	};
	matches!(
		op,
		"Add"
			| "Sub"
			| "Mul"
			| "Div"
			| "Rem"
			| "Neg"
			| "Shl"
			| "Shr"
			| "And"
			| "Or"
			| "Xor"
			| "Not"
			| "Eq"
			| "Ne"
			| "Lt"
			| "Le"
			| "Gt"
			| "Ge"
			| "Max"
			| "Min"
			| "Cast"
			| "Low"
	)
}

/// Replace the one read of `name` in `stmt` with `value`; whether it was there.
fn subst_stmt(stmt: &mut Stmt, name: &str, value: &Expr) -> bool {
	match &mut stmt.kind {
		StmtKind::Expr(e) | StmtKind::Return(Some(e)) => subst(e, name, value),
		StmtKind::Decl { init: Some(e), .. } => subst(e, name, value),
		StmtKind::If { cond, .. } => subst(cond, name, value),
		_ => false,
	}
}

fn subst(e: &mut Expr, name: &str, value: &Expr) -> bool {
	match e {
		Expr::Ident(n) if n == name => {
			*e = value.clone();
			true
		}
		Expr::Ident(_) | Expr::This | Expr::Sym(_) | Expr::Rt(_) | Expr::Lit(_) | Expr::Str(_) => false,
		Expr::Array(items) => items.iter_mut().any(|e| subst(e, name, value)),
		Expr::Object(props) => props.iter_mut().any(|(_, e)| subst(e, name, value)),
		Expr::Member(base, key) => {
			if !subst(base, name, value) {
				return false;
			}
			// The payload of a `Result` made here is what it was made of, which `tsc` reads
			// where it can't narrow the `Result`, cast to its variant or not, and so is what an
			// `rt.Place` made here holds.
			let made = match step::peel(base) {
				Expr::As(made, _) => step::peel(made),
				base => base,
			};
			let folded = match (made, &*key) {
				(Expr::Call(f, args), ts::Key::Name(key))
					if let Expr::Rt(ctor) = step::peel(f)
						&& matches!((ctor.as_str(), key.as_str()), ("Ok", "value") | ("Err", "error"))
						&& let [payload] = args.as_slice() =>
				{
					Some(payload.clone())
				}
				(_, ts::Key::Name(key)) if key == "v" => step::peel(base).place_read().cloned(),
				_ => None,
			};
			if let Some(folded) = folded {
				*e = folded;
			}
			true
		}
		Expr::Index(base, index) => subst(base, name, value) || subst(index, name, value),
		Expr::Call(f, args) | Expr::New(f, args) => {
			if !(subst(f, name, value) || args.iter_mut().any(|e| subst(e, name, value))) {
				return false;
			}
			// A result of what folded in as `undefined` is the shared one.
			if let Expr::Call(f, args) = &*e
				&& let (Expr::Rt(ctor), [arg]) = (&**f, args.as_slice())
				&& matches!(ctor.as_str(), "Ok" | "Err")
			{
				*e = Expr::result(ctor == "Ok", arg.clone());
			}
			// An `Ok` made in place unwraps to its value, as a lock's does, unless what would
			// show or replace an error has an effect.
			if let Expr::Call(f, args) = &*e
				&& let (Expr::Rt(unwrap), [result, rest @ ..]) = (&**f, args.as_slice())
				&& matches!(unwrap.as_str(), "resultUnwrap" | "resultExpect" | "resultUnwrapOrElse")
				&& let Expr::Call(ok, value) = result.unlocated()
				&& matches!(&**ok, Expr::Rt(ok) if ok == "Ok")
				&& let [value] = value.as_slice()
				&& rest
					.iter()
					.all(|e| e.is_path() || matches!(e.unlocated(), Expr::Arrow(..) | Expr::Lambda(..)))
			{
				*e = value.clone();
			}
			true
		}
		Expr::Unary(_, e) | Expr::Located(_, e) | Expr::As(e, _) | Expr::Instantiate(e, _) | Expr::Postfix(e, _) => {
			subst(e, name, value)
		}
		Expr::Binary(l, _, r) | Expr::Assign(l, r) => subst(l, name, value) || subst(r, name, value),
		Expr::Cond(c, a, b) => subst(c, name, value) || subst(a, name, value) || subst(b, name, value),
		Expr::Arrow(..) | Expr::Lambda(..) => false,
	}
}

/// `test` with each read of the path `key` cast to its whole type `ty`, which TypeScript
/// narrows nothing by.
fn widen_reads(test: Expr, key: &str, ty: &Type) -> Expr {
	let widen = |e: Box<Expr>| Box::new(widen_reads(*e, key, ty));
	match test {
		e if e.path_key().as_deref() == Some(key) => Expr::As(Box::new(e), ty.clone()),
		Expr::Member(base, k) => Expr::Member(widen(base), k),
		Expr::Index(base, index) => Expr::Index(widen(base), widen(index)),
		Expr::Call(f, args) => Expr::Call(widen(f), args.into_iter().map(|e| widen_reads(e, key, ty)).collect()),
		Expr::Unary(op, e) => Expr::Unary(op, widen(e)),
		Expr::Located(loc, e) => Expr::Located(loc, widen(e)),
		Expr::Binary(l, op, r) => Expr::Binary(widen(l), op, widen(r)),
		Expr::Cond(c, a, b) => Expr::Cond(widen(c), widen(a), widen(b)),
		e => e,
	}
}

/// Drop the declarations of variables nothing reads, keeping the effects of what was
/// assigned to them, until what those dropped read is read elsewhere.
fn prune_unread(stmts: &mut Vec<Stmt>) {
	loop {
		let mut declared = Vec::new();
		for stmt in stmts.iter() {
			if let StmtKind::Declare { name, .. } = &stmt.kind {
				declared.push(name.clone());
			}
		}
		let mut reads = Vec::new();
		for stmt in stmts.iter() {
			scan_reads(stmt, &mut reads);
		}
		let read: FxHashSet<&str> = reads.iter().map(String::as_str).collect();
		let unread: FxHashSet<String> = declared.into_iter().filter(|n| !read.contains(n.as_str())).collect();
		if unread.is_empty() {
			return;
		}
		strip_assignments(stmts, &unread);
	}
}

/// Every name `stmt` reads, but not as the target of an assignment to it.
fn scan_reads(stmt: &Stmt, out: &mut Vec<String>) {
	let mut visit = |e: &Expr| reads_of(e, out);
	match &stmt.kind {
		StmtKind::Expr(e) | StmtKind::Return(Some(e)) => visit(e),
		StmtKind::Decl { init: Some(e), .. } => visit(e),
		StmtKind::If { cond, then, els } => {
			visit(cond);
			then.iter().chain(els).for_each(|s| scan_reads(s, out));
		}
		StmtKind::While { cond: e, body, .. } | StmtKind::ForOf { iter: e, body, .. } => {
			reads_of(e, out);
			body.iter().for_each(|s| scan_reads(s, out));
		}
		StmtKind::Block { body, .. } => body.iter().for_each(|s| scan_reads(s, out)),
		StmtKind::Switch {
			disc, cases, default, ..
		} => {
			reads_of(disc, out);
			for (v, body) in cases {
				reads_of(v, out);
				body.iter().for_each(|s| scan_reads(s, out));
			}
			default.iter().for_each(|s| scan_reads(s, out));
		}
		_ => {}
	}
}

/// The names `stmts` assign, anywhere in them, as themselves or cast to a variant's type.
fn assigned(stmts: &[Stmt]) -> FxHashSet<String> {
	let mut names = FxHashSet::default();
	for stmt in stmts {
		visit_stmt(stmt, &mut |e| {
			visit_expr(e, &mut |e| {
				if let Expr::Assign(target, _) | Expr::Postfix(target, _) = e
					&& let Some(name) = assigned_name(target)
				{
					names.insert(name.to_string());
				}
			})
		});
	}
	names
}

fn reads_of(e: &Expr, out: &mut Vec<String>) {
	match e {
		Expr::Ident(name) => out.push(name.clone()),
		Expr::This | Expr::Sym(_) | Expr::Rt(_) | Expr::Lit(_) | Expr::Str(_) => {}
		Expr::Array(items) => items.iter().for_each(|e| reads_of(e, out)),
		Expr::Object(props) => props.iter().for_each(|(_, e)| reads_of(e, out)),
		Expr::Member(base, _) => reads_of(base, out),
		Expr::Index(base, index) => {
			reads_of(base, out);
			reads_of(index, out);
		}
		Expr::Call(f, args) | Expr::New(f, args) => {
			reads_of(f, out);
			args.iter().for_each(|e| reads_of(e, out));
		}
		Expr::Unary(_, e)
		| Expr::Located(_, e)
		| Expr::As(e, _)
		| Expr::Instantiate(e, _)
		| Expr::Lambda(_, e)
		| Expr::Postfix(e, _) => reads_of(e, out),
		Expr::Binary(l, _, r) => {
			reads_of(l, out);
			reads_of(r, out);
		}
		Expr::Cond(c, a, b) => {
			reads_of(c, out);
			reads_of(a, out);
			reads_of(b, out);
		}
		// Assigning a name whole doesn't read it.
		Expr::Assign(target, value) => {
			if !matches!(&**target, Expr::Ident(_)) {
				reads_of(target, out);
			}
			reads_of(value, out);
		}
		Expr::Arrow(_, _, body) => body.iter().for_each(|s| scan_reads(s, out)),
	}
}

fn strip_assignments(stmts: &mut Vec<Stmt>, unread: &FxHashSet<String>) {
	stmts.retain(|s| !matches!(&s.kind, StmtKind::Declare { name, .. } if unread.contains(name)));
	for stmt in stmts.iter_mut() {
		match &mut stmt.kind {
			StmtKind::Expr(e) => {
				if let Expr::Assign(target, value) = e
					&& let Expr::Ident(name) = &**target
					&& unread.contains(name)
				{
					*e = std::mem::replace(&mut **value, Expr::undefined());
				}
			}
			StmtKind::If { then, els, .. } => {
				strip_assignments(then, unread);
				strip_assignments(els, unread);
			}
			StmtKind::While { body, .. } | StmtKind::ForOf { body, .. } | StmtKind::Block { body, .. } => {
				strip_assignments(body, unread)
			}
			StmtKind::Switch { cases, default, .. } => {
				cases.iter_mut().for_each(|(_, body)| strip_assignments(body, unread));
				strip_assignments(default, unread);
			}
			_ => {}
		}
	}
	// What is left of an assignment and does nothing.
	stmts.retain(|s| !matches!(&s.kind, StmtKind::Expr(e) if inert(e)));
}

/// Declare each variable declared up front at the assignment that first names it, where
/// everything else that names it follows in the same block: no path reaches those without
/// passing the assignment. `const` if nothing assigns it again. Those `widened` keep
/// the type their value was given.
fn declare_in_place(stmts: &mut Vec<Stmt>, widened: &FxHashSet<String>) {
	let declared: Vec<(String, Type)> = stmts
		.iter()
		.filter_map(|s| match &s.kind {
			StmtKind::Declare { name, ty } => Some((name.clone(), ty.clone())),
			_ => None,
		})
		.collect();
	for (name, ty) in declared {
		let total = list_names(stmts, &name);
		let mut writes = 0;
		stmts
			.iter()
			.for_each(|s| visit_stmt(s, &mut |e| writes += writes_of(e, &name)));
		let widened = widened.contains(&name);
		if total > 0 && place_declaration(stmts, &name, &ty, total, writes > 1, widened) {
			stmts.retain(|s| !matches!(&s.kind, StmtKind::Declare { name: n, .. } if *n == name));
		}
	}
}

/// Turn the statement that first names `name` into its declaration, if it assigns it and
/// all `total` statements naming it follow in its block. A `widened` value keeps its
/// `as`, which then types the variable.
fn place_declaration(stmts: &mut [Stmt], name: &str, ty: &Type, total: usize, mutable: bool, widened: bool) -> bool {
	let Some(i) = stmts.iter().position(|s| stmt_names(s, name) > 0) else {
		return false;
	};
	let assigns = matches!(&stmts[i].kind, StmtKind::Expr(Expr::Assign(target, value))
		if matches!(&**target, Expr::Ident(n) if n == name) && expr_names(value, name) == 0);
	if assigns {
		if list_names(&stmts[i..], name) != total {
			return false;
		}
		let StmtKind::Expr(Expr::Assign(_, value)) = std::mem::replace(&mut stmts[i].kind, StmtKind::Break(None))
		else {
			unreachable!("checked above")
		};
		stmts[i].kind = match *value {
			Expr::As(_, ref t) if widened && t == ty => StmtKind::decl(mutable, name, None, Some(*value)),
			value if widened => StmtKind::Decl {
				mutable,
				name: name.into(),
				ty: Some(ty.clone()),
				init: Some(value),
			},
			value => StmtKind::decl(mutable, name, Some(ty.clone()), Some(value)),
		};
		return true;
	}
	// Else everything naming it must be in one block of this statement.
	if stmt_names(&stmts[i], name) != total {
		return false;
	}
	let place = |body: &mut Vec<Stmt>| {
		list_names(body, name) == total && place_declaration(body, name, ty, total, mutable, widened)
	};
	match &mut stmts[i].kind {
		StmtKind::If { then, els, .. } => place(then) || place(els),
		StmtKind::While { body, .. } | StmtKind::ForOf { body, .. } | StmtKind::Block { body, .. } => place(body),
		StmtKind::Switch { cases, default, .. } => cases.iter_mut().any(|(_, body)| place(body)) || place(default),
		_ => false,
	}
}

/// How many times the statements name `name`, read or assigned.
fn list_names(stmts: &[Stmt], name: &str) -> usize {
	stmts.iter().map(|s| stmt_names(s, name)).sum()
}

fn stmt_names(stmt: &Stmt, name: &str) -> usize {
	let mut n = 0;
	visit_stmt(stmt, &mut |e| n += expr_names(e, name));
	n
}

fn expr_names(e: &Expr, name: &str) -> usize {
	let mut n = 0;
	visit_expr(e, &mut |e| n += matches!(e, Expr::Ident(i) if i == name) as usize);
	n
}

/// How many times `e` itself assigns `name`.
fn writes_of(e: &Expr, name: &str) -> usize {
	let mut n = 0;
	visit_expr(e, &mut |e| {
		if let Expr::Assign(target, _) | Expr::Postfix(target, _) = e
			&& assigned_name(target) == Some(name)
		{
			n += 1;
		}
	});
	n
}

/// The name an assignment to `target` writes whole: itself, or cast to the type of a
/// variant it holds, as a `Cow`'s payload is.
fn assigned_name(mut target: &Expr) -> Option<&str> {
	while let Expr::As(inner, _) = target {
		target = inner;
	}
	match target {
		Expr::Ident(name) => Some(name),
		_ => None,
	}
}

/// Call `f` on each expression the statement holds directly, and in its blocks.
fn visit_stmt(stmt: &Stmt, f: &mut dyn FnMut(&Expr)) {
	match &stmt.kind {
		StmtKind::Expr(e) | StmtKind::Return(Some(e)) | StmtKind::Decl { init: Some(e), .. } => f(e),
		StmtKind::If { cond, then, els } => {
			f(cond);
			then.iter().chain(els).for_each(|s| visit_stmt(s, f));
		}
		StmtKind::While { cond: e, body, .. } | StmtKind::ForOf { iter: e, body, .. } => {
			f(e);
			body.iter().for_each(|s| visit_stmt(s, f));
		}
		StmtKind::Block { body, .. } => body.iter().for_each(|s| visit_stmt(s, f)),
		StmtKind::Switch {
			disc, cases, default, ..
		} => {
			f(disc);
			for (value, body) in cases {
				f(value);
				body.iter().for_each(|s| visit_stmt(s, f));
			}
			default.iter().for_each(|s| visit_stmt(s, f));
		}
		StmtKind::Return(None)
		| StmtKind::Decl { init: None, .. }
		| StmtKind::Declare { .. }
		| StmtKind::Break(_)
		| StmtKind::Continue(_) => {}
	}
}

/// Call `f` on `e` and every expression within it, arrow bodies included.
fn visit_expr(e: &Expr, f: &mut dyn FnMut(&Expr)) {
	f(e);
	match e {
		Expr::Ident(_) | Expr::This | Expr::Sym(_) | Expr::Rt(_) | Expr::Lit(_) | Expr::Str(_) => {}
		Expr::Array(items) => items.iter().for_each(|e| visit_expr(e, f)),
		Expr::Object(props) => props.iter().for_each(|(_, e)| visit_expr(e, f)),
		Expr::Member(e, _)
		| Expr::Unary(_, e)
		| Expr::Located(_, e)
		| Expr::As(e, _)
		| Expr::Instantiate(e, _)
		| Expr::Lambda(_, e)
		| Expr::Postfix(e, _) => visit_expr(e, f),
		Expr::Index(a, b) | Expr::Binary(a, _, b) | Expr::Assign(a, b) => {
			visit_expr(a, f);
			visit_expr(b, f);
		}
		Expr::Call(callee, args) | Expr::New(callee, args) => {
			visit_expr(callee, f);
			args.iter().for_each(|e| visit_expr(e, f));
		}
		Expr::Cond(c, a, b) => {
			visit_expr(c, f);
			visit_expr(a, f);
			visit_expr(b, f);
		}
		Expr::Arrow(_, _, body) => body.iter().for_each(|s| visit_stmt(s, &mut |e| visit_expr(e, f))),
	}
}

/// The operands of an aggregate, from its expression.
fn parts_of(value: &Expr) -> Option<Vec<Expr>> {
	match value {
		Expr::Array(items) => Some(items.clone()),
		Expr::As(inner, _) => parts_of(inner),
		Expr::Call(f, items) if matches!(&**f, Expr::Member(base, crate::ts::Key::Name(of)) if of == "of" && matches!(&**base, Expr::Ident(u) if u == "Uint8Array")) => {
			Some(items.clone())
		}
		Expr::Object(props) => Some(props.iter().map(|(_, v)| v.clone()).collect()),
		Expr::New(f, items) if matches!(&**f, Expr::Rt(name) if name == "Range" || name == "RangeFrom") => {
			Some(items.clone())
		}
		_ => None,
	}
}

/// `e` with comparisons of two number literals, and their negations, worked out.
pub fn fold(e: Expr) -> Expr {
	match e {
		Expr::Binary(l, op, r) => {
			let (l, r) = (fold(*l), fold(*r));
			let num = |e: &Expr| match e {
				Expr::Lit(t) => t.parse::<f64>().ok(),
				_ => None,
			};
			if let (Some(a), Some(b)) = (num(&l), num(&r)) {
				// Integer arithmetic JS does exactly, in 32 bits for the bitwise operators. An
				// `f64` literal is spelled with a point or an exponent, and its -0.0 is no integer.
				let int = |e: &Expr| matches!(e, Expr::Lit(t) if !t.contains(['.', 'e']));
				let small = |v: f64| v.fract() == 0.0 && v.abs() < 2f64.powi(31);
				if int(&l) && int(&r) && small(a) && small(b) {
					let (x, y) = (a as i64, b as i64);
					let int = match op {
						"<<" if (0..31).contains(&y) => Some((x as i32).wrapping_shl(y as u32) as i64),
						">>" if (0..32).contains(&y) => Some(((x as i32) >> y) as i64),
						"|" => Some((x as i32 | y as i32) as i64),
						"&" => Some((x as i32 & y as i32) as i64),
						"^" => Some((x as i32 ^ y as i32) as i64),
						"+" => Some(x + y),
						"-" => Some(x - y),
						_ => None,
					};
					if let Some(v) = int
						&& (x >= 0 && y >= 0 || !matches!(op, "<<" | ">>" | "|" | "&" | "^"))
					{
						return Expr::lit(v.to_string());
					}
				}
				let value = match op {
					"<" => Some(a < b),
					"<=" => Some(a <= b),
					">" => Some(a > b),
					">=" => Some(a >= b),
					"===" => Some(a == b),
					"!==" => Some(a != b),
					_ => None,
				};
				if let Some(value) = value {
					return Expr::lit(value.to_string());
				}
			}
			Expr::Binary(Box::new(l), op, Box::new(r))
		}
		// An element of an array made in place, as a format argument's tuple is read.
		Expr::Index(base, index) => {
			let (base, index) = (fold(*base), fold(*index));
			let items = match &base {
				Expr::Array(items) => Some(items),
				Expr::As(inner, _) => match &**inner {
					Expr::Array(items) => Some(items),
					_ => None,
				},
				_ => None,
			};
			if let (Some(items), Expr::Lit(i)) = (items, &index)
				&& let Ok(i) = i.parse::<usize>()
				&& i < items.len()
				&& items.iter().all(Expr::is_path)
			{
				return items[i].clone();
			}
			Expr::Index(Box::new(base), Box::new(index))
		}
		Expr::Unary("!", inner) => match fold(*inner) {
			Expr::Lit(t) if t == "true" => Expr::lit("false"),
			Expr::Lit(t) if t == "false" => Expr::lit("true"),
			inner => Expr::Unary("!", Box::new(inner)),
		},
		e => e,
	}
}

/// Whether an assert checks what the runtime's division, remainder, negation, or shift
/// checks as the operation it guards runs next, which panics with the same message.
fn redundant(msg: &mir::AssertMessage<'_>) -> bool {
	use mir::{AssertKind, BinOp};
	matches!(
		msg,
		AssertKind::OverflowNeg(_)
			| AssertKind::DivisionByZero(_)
			| AssertKind::RemainderByZero(_)
			| AssertKind::Overflow(BinOp::Div | BinOp::Rem | BinOp::Shl | BinOp::Shr, ..)
	)
}

/// Whether `term` is an assert of a constant that fails.
fn fails(term: &mir::Terminator<'_>) -> bool {
	let TerminatorKind::Assert {
		cond: Operand::Constant(c),
		expected,
		..
	} = &term.kind
	else {
		return false;
	};
	c.const_.try_to_bool().is_some_and(|b| b != *expected)
}

/// Drop the assignments of a variable or a field to itself, which ferrying a value
/// through temporaries leaves, as does a coroutine moving its argument into the
/// variable that keeps its field.
fn drop_self_assignments(stmts: &mut Vec<Stmt>) {
	stmts.retain(|s| !matches!(&s.kind, StmtKind::Expr(Expr::Assign(t, v)) if same_path(t, v)));
	for stmt in stmts.iter_mut() {
		match &mut stmt.kind {
			StmtKind::If { then, els, .. } => {
				drop_self_assignments(then);
				drop_self_assignments(els);
			}
			StmtKind::While { body, .. } | StmtKind::ForOf { body, .. } | StmtKind::Block { body, .. } => {
				drop_self_assignments(body)
			}
			StmtKind::Switch { cases, default, .. } => {
				cases.iter_mut().for_each(|(_, body)| drop_self_assignments(body));
				drop_self_assignments(default);
			}
			_ => {}
		}
	}
}

/// Whether `a` and `b` are one path: a variable, `this`, or a property of one.
fn same_path(a: &Expr, b: &Expr) -> bool {
	match (a, b) {
		(Expr::Ident(a), Expr::Ident(b)) => a == b,
		(Expr::This, Expr::This) => true,
		(Expr::Member(a, ka), Expr::Member(b, kb)) => {
			let key = match (ka, kb) {
				(ts::Key::Name(x), ts::Key::Name(y)) => x == y,
				(ts::Key::Field(x), ts::Key::Field(y)) => x == y,
				_ => false,
			};
			key && same_path(a, b)
		}
		_ => false,
	}
}

/// Return values in place throughout `stmts`, given the names of the body's own locals.
fn return_in_place(stmts: &mut Vec<Stmt>, returnable: &FxHashSet<String>) {
	sink_return(stmts);
	for stmt in stmts.iter_mut() {
		match &mut stmt.kind {
			StmtKind::If { then, els, .. } => {
				return_in_place(then, returnable);
				return_in_place(els, returnable);
			}
			StmtKind::While { body, .. } | StmtKind::ForOf { body, .. } | StmtKind::Block { body, .. } => {
				return_in_place(body, returnable)
			}
			StmtKind::Switch { cases, default, .. } => {
				cases.iter_mut().for_each(|(_, body)| return_in_place(body, returnable));
				return_in_place(default, returnable);
			}
			_ => {}
		}
	}
	while return_assigned(stmts, returnable) {}
}

/// `x = v; ...; return x;` at the end of `stmts` as `...; return v;`, where moving `v`
/// past the statements between changes nothing (there are none, or `v` is a constant or a
/// name none of them assigns) and `x` is `returnable`; whether it was.
fn return_assigned(stmts: &mut Vec<Stmt>, returnable: &FxHashSet<String>) -> bool {
	let Some(Stmt {
		kind: StmtKind::Return(Some(Expr::Ident(name))),
		..
	}) = stmts.last()
	else {
		return false;
	};
	if !returnable.contains(name) {
		return false;
	}
	let name = name.clone();
	let assigns = |e: &Expr, to: &str| matches!(e, Expr::Assign(t, _) if matches!(&**t, Expr::Ident(n) if n == to));
	let mut at = None;
	for i in (0..stmts.len() - 1).rev() {
		let StmtKind::Expr(e) = &stmts[i].kind else {
			return false;
		};
		if assigns(e, &name) {
			at = Some(i);
			break;
		}
		let mut reads = Vec::new();
		reads_of(e, &mut reads);
		if reads.contains(&name) {
			return false;
		}
	}
	let Some(at) = at else {
		return false;
	};
	let StmtKind::Expr(Expr::Assign(_, value)) = &stmts[at].kind else {
		unreachable!("found above")
	};
	let between = &stmts[at + 1..stmts.len() - 1];
	let moves = between.is_empty()
		|| value.is_constant()
		|| matches!(&**value, Expr::Ident(n) if between.iter().all(|s| !matches!(&s.kind, StmtKind::Expr(e) if assigns(e, n))));
	if !moves {
		return false;
	}
	let StmtKind::Expr(Expr::Assign(_, value)) = stmts.remove(at).kind else {
		unreachable!("found above")
	};
	let last = stmts.last_mut().expect("the return");
	last.kind = StmtKind::Return(Some(*value));
	true
}

/// Move a `return x;` that follows an `if` or a labeled block into each way out of it, so
/// each can return what it assigns `x` in place.
fn sink_return(stmts: &mut Vec<Stmt>) {
	while stmts.len() >= 2
		&& matches!(
			stmts.last().map(|s| &s.kind),
			Some(StmtKind::Return(Some(Expr::Ident(_))))
		) {
		let ret = stmts.pop().expect("checked above");
		let last = stmts.last_mut().expect("checked above");
		match &mut last.kind {
			StmtKind::If { then, els, .. } => {
				for branch in [then, els] {
					if !crate::ts::diverges(branch) {
						branch.push(ret.clone());
					}
				}
				return;
			}
			StmtKind::Block {
				label: Some(label),
				body,
			} => {
				let label = label.clone();
				breaks_to_return(body, &label, &ret);
				if !crate::ts::diverges(body) {
					body.push(ret);
				}
				let StmtKind::Block { body, .. } = stmts.pop().expect("checked above").kind else {
					unreachable!("matched above")
				};
				stmts.extend(body);
			}
			_ => {
				stmts.push(ret);
				return;
			}
		}
	}
}

/// Replace each `break label;` in `stmts` with `ret`.
fn breaks_to_return(stmts: &mut [Stmt], label: &str, ret: &Stmt) {
	for stmt in stmts.iter_mut() {
		match &mut stmt.kind {
			StmtKind::Break(Some(l)) if l == label => *stmt = ret.clone(),
			StmtKind::If { then, els, .. } => {
				breaks_to_return(then, label, ret);
				breaks_to_return(els, label, ret);
			}
			StmtKind::While { body, .. } | StmtKind::ForOf { body, .. } | StmtKind::Block { body, .. } => {
				breaks_to_return(body, label, ret)
			}
			StmtKind::Switch { cases, default, .. } => {
				cases
					.iter_mut()
					.for_each(|(_, body)| breaks_to_return(body, label, ret));
				breaks_to_return(default, label, ret);
			}
			_ => {}
		}
	}
}

/// Whether evaluating `e` does nothing at all: a path, or a literal made of them.
fn inert(e: &Expr) -> bool {
	match e {
		Expr::Array(items) => items.iter().all(inert),
		Expr::Object(props) => props.iter().all(|(_, v)| inert(v)),
		Expr::As(e, _) | Expr::Instantiate(e, _) | Expr::Located(_, e) => inert(e),
		Expr::Lambda(..) | Expr::Arrow(..) => true,
		e => e.is_path(),
	}
}

/// The item a next result holds, as `ty`: `Option<Item>`, or `?`'s `ControlFlow<_, Item>`.
fn next_item<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Ty<'tcx> {
	let ty::Adt(adt, args) = ty.kind() else {
		unreachable!("checked by find_next_results")
	};
	match crate::lower::library::Std::of(tcx, adt.did()) {
		Some(crate::lower::library::Std::ControlFlow) => args.type_at(1),
		_ => args.type_at(0),
	}
}

/// Whether `ty` is a `Range` of numbers that fit a JS number, which steps in place.
pub(super) fn small_range<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
	matches!(ty.kind(), ty::Adt(adt, args)
		if crate::lower::library::Std::of(tcx, adt.did()) == Some(crate::lower::library::Std::Range)
			&& crate::lower::num::Int::of(args.type_at(0)).is_some_and(|i| !i.big()))
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A variable an arrow reads below the top of one of its statements stays declared
	/// where the arrow can see it, not at an assignment in a block the arrow is outside.
	#[test]
	fn declare_counts_arrow_reads() {
		let found = || Expr::ident("found");
		let assign = StmtKind::Expr(found().assign(Expr::lit("1"))).at(None);
		let read = StmtKind::Return(Some(Expr::ident("f").call(vec![found()]))).at(None);
		let arrow = Expr::Arrow(Vec::new(), Type::Builtin("number"), vec![read]);
		let then = vec![assign, StmtKind::Break(None).at(None)];
		let cond = Expr::ident("c");
		let body = vec![
			StmtKind::If {
				cond,
				then,
				els: Vec::new(),
			}
			.at(None),
		];
		let ty = Type::Builtin("number");
		let mut stmts = vec![
			StmtKind::Declare {
				name: "found".into(),
				ty,
			}
			.at(None),
			StmtKind::While {
				label: None,
				cond: Expr::lit("true"),
				body,
			}
			.at(None),
			StmtKind::Expr(arrow.call(Vec::new())).at(None),
		];
		declare_in_place(&mut stmts, &FxHashSet::default());
		assert!(matches!(&stmts[0].kind, StmtKind::Declare { name, .. } if name == "found"));
	}
}
