//! Switches an enclosing switch already decided: `match x { Dot => match x { .. } }`
//! tests `x` again where only `Dot` reaches. TypeScript narrows `x` in the arm and
//! refuses a test for another variant, so each such switch goes straight to the arms
//! that can still match.
//!
//! A forward dataflow over the body finds, at each switch on a place's variant, the
//! variants the place can hold: a switch's edge narrows them, and anything that can
//! change the place forgets them. A `bool` that a variant test or a derived `==` set, as
//! `if matches!(x, Dot)` tests, carries what each of its values says of the place to the
//! switch on it.
//!
//! The same dataflow follows what TypeScript narrowed each place to, which only an
//! assignment it sees undoes. Where a call changed the place since, or a library call's
//! test narrowed it, [`narrowing`] finds the tests and payload reads that must see the
//! place's whole type.

use rustc_abi::VariantIdx;
use rustc_data_structures::fx::FxHashSet;
use rustc_index::IndexVec;
use rustc_middle::mir::interpret::GlobalId;
use rustc_middle::mir::visit::{NonMutatingUseContext, PlaceContext, Visitor};
use rustc_middle::mir::{
	self, AggregateKind, BasicBlock, BorrowKind, ConstOperand, Local, Location, MutBorrowKind, Operand, Place,
	ProjectionElem, Rvalue, StatementKind, SwitchTargets, TerminatorKind,
};
use rustc_middle::ty::{self, Instance, Ty, TyCtxt};
use rustc_span::Spanned;

use crate::lower::Crates;
use crate::lower::collections::trait_fn;

/// Fold each switch in `body` that an enclosing switch or a `bool` decided.
pub fn fold<'tcx>(tcx: TyCtxt<'tcx>, crates: &Crates<'tcx>, body: &mut mir::Body<'tcx>) {
	let flow = Flow::new(tcx, crates, body);
	let entry = flow.solve(body);
	let mut folded = false;
	for bb in body.basic_blocks.indices() {
		let Some(facts) = entry[bb].clone() else {
			continue;
		};
		let (Some((def, kept, otherwise)), _) = flow.block(body, bb, facts, None) else {
			continue;
		};
		let data = &mut body.basic_blocks_mut()[bb];
		let TerminatorKind::SwitchInt { discr, targets } = &mut data.terminator_mut().kind else {
			unreachable!("decided only for a switch")
		};
		if kept.len() == targets.iter().count() {
			continue;
		}
		folded = true;
		if kept.iter().any(|&(_, t)| t != otherwise) {
			*targets = SwitchTargets::new(kept.into_iter(), otherwise);
			continue;
		}
		// A discriminant the switch moved has no other reader.
		let moved = matches!(discr, Operand::Move(_));
		data.terminator_mut().kind = TerminatorKind::Goto {
			target: otherwise,
			loop_hint_attrs: Default::default(),
		};
		if moved && let Some(def) = def {
			data.statements[def].kind = StatementKind::Nop;
		}
	}
	// What only a folded edge reached is gone, so nothing it does counts as a use.
	if folded {
		let reachable = mir::traversal::reachable_as_bitset(body);
		for (bb, data) in body.basic_blocks_mut().iter_enumerated_mut() {
			if !reachable.contains(bb) {
				data.statements.clear();
				data.terminator_mut().kind = TerminatorKind::Unreachable;
			}
		}
	}
}

/// What the dataflow knows at a point.
#[derive(Clone, Default, PartialEq)]
struct Facts<'tcx> {
	/// The variants, by discriminant, that each place known so far can hold.
	known: Vec<(Place<'tcx>, Vec<u128>)>,
	/// The `bool`s known to hold a value.
	bools: Vec<(Local, bool)>,
	/// What a `bool`'s value says of a place.
	implied: Vec<Implication<'tcx>>,
	/// What TypeScript narrowed each place to since it last saw the place assigned: the
	/// variants its tests left, or `None` where a library call's test narrowed it, and
	/// what it holds, in a way the dataflow doesn't follow.
	narrowed: Vec<(Place<'tcx>, Option<Vec<u128>>)>,
}

impl<'tcx> Facts<'tcx> {
	/// What both `self` and `other` know, and what either may have narrowed.
	fn join(&self, other: &Self) -> Self {
		let known = self
			.known
			.iter()
			.filter_map(|(p, vs)| {
				let (_, theirs) = other.known.iter().find(|(q, _)| q == p)?;
				Some((*p, union(vs, theirs)))
			})
			.collect();
		// What one way in left unnarrowed has its whole type.
		let narrowed = self
			.narrowed
			.iter()
			.filter_map(|(p, vs)| {
				let (_, theirs) = other.narrowed.iter().find(|(q, _)| q == p)?;
				Some((*p, vs.as_ref().zip(theirs.as_ref()).map(|(a, b)| union(a, b))))
			})
			.collect();
		let bools = self.bools.iter().filter(|b| other.bools.contains(b)).copied().collect();
		// What a `bool` says on either way in, where one way knows only its value.
		let mut implied: Vec<Implication<'tcx>> = Vec::new();
		let keys = |facts: &Self, other: &Self| -> Vec<(Local, Place<'tcx>)> {
			let held = facts
				.bools
				.iter()
				.filter(|b| !other.bools.contains(b))
				.flat_map(|&(b, _)| facts.known.iter().map(move |(p, _)| (b, *p)));
			facts.implied.iter().map(|(b, p, _)| (*b, *p)).chain(held).collect()
		};
		for key in [keys(self, other), keys(other, self)].concat() {
			if implied.iter().any(|(b, p, _)| (*b, *p) == key) {
				continue;
			}
			if let (Some([f, t]), Some([g, u])) = (self.implies(key), other.implies(key)) {
				let either = |a: Option<Vec<u128>>, b: Option<Vec<u128>>| a.zip(b).map(|(a, b)| union(&a, &b));
				implied.push((key.0, key.1, [either(f, g), either(t, u)]));
			}
		}
		Self {
			known,
			bools,
			implied,
			narrowed,
		}
	}

	/// What `b`'s values say of `place`, if anything is known of `b`.
	fn implies(&self, (b, place): (Local, Place<'tcx>)) -> Option<Implied> {
		if let Some(&(_, v)) = self.bools.iter().find(|(c, _)| *c == b) {
			let mut implied = [Some(Vec::new()), Some(Vec::new())];
			implied[v as usize] = self.known.iter().find(|(p, _)| *p == place).map(|(_, vs)| vs.clone());
			return Some(implied);
		}
		self.implied
			.iter()
			.find(|(c, p, _)| (*c, *p) == (b, place))
			.map(|(_, _, implied)| implied.clone())
	}

	/// Narrow each place by what `b` holding `v` says of it, or `false` where `b` can't.
	fn assume(&mut self, b: Local, v: bool) -> bool {
		if self.bools.contains(&(b, !v)) {
			return false;
		}
		for (_, place, implied) in self.implied.iter().filter(|(c, _, _)| *c == b) {
			let Some(vs) = &implied[v as usize] else {
				continue;
			};
			let known = match self.known.iter_mut().find(|(p, _)| p == place) {
				Some((_, known)) => {
					known.retain(|x| vs.contains(x));
					known
				}
				None => {
					self.known.push((*place, vs.clone()));
					&self.known.last().expect("pushed").1
				}
			};
			if known.is_empty() {
				return false;
			}
		}
		self.bools.retain(|(c, _)| *c != b);
		self.bools.push((b, v));
		true
	}

	/// Forget what `gone` says of a place, and of the discriminants read in this block.
	fn forget(&mut self, discrs: &mut Vec<(Local, usize, Place<'tcx>)>, gone: impl Fn(Place<'tcx>) -> bool) {
		self.known.retain(|(p, _)| !gone(*p));
		self.bools.retain(|(b, _)| !gone(Place::from(*b)));
		self.implied.retain(|(b, p, _)| !gone(*p) && !gone(Place::from(*b)));
		discrs.retain(|(_, _, p)| !gone(*p));
	}

	/// What TypeScript narrowed `place` to: `Some(None)` where the dataflow can't name it,
	/// and `None` where it didn't narrow it.
	fn narrowing(&self, place: Place<'tcx>) -> Option<Option<&[u128]>> {
		if self.narrowed.iter().any(|(p, vs)| vs.is_none() && prefix(*p, place)) {
			return Some(None);
		}
		self.narrowed
			.iter()
			.find(|(p, _)| *p == place)
			.map(|(_, vs)| vs.as_deref())
	}

	/// Record that TypeScript narrowed `place` to `to`.
	fn narrow(&mut self, place: Place<'tcx>, to: Option<Vec<u128>>) {
		self.narrowed.retain(|(p, _)| *p != place);
		self.narrowed.push((place, to));
	}
}

/// The variants a place can hold where a `bool` is `false` and where it is `true`, `None`
/// for any.
type Implied = [Option<Vec<u128>>; 2];

/// A `bool`, a place, and what the `bool`'s values say of it.
type Implication<'tcx> = (Local, Place<'tcx>, Implied);

/// The tests and payload reads in a body that must see a place's whole type, since
/// TypeScript may have narrowed the place before a change it doesn't see, as a call's,
/// or by a library call's test.
#[derive(Default)]
pub struct Narrowing<'tcx> {
	/// The statements reading the discriminant that a widened switch tests.
	widened: FxHashSet<Location>,
	/// Places read as a variant TypeScript may not have narrowed them to, with where.
	stale: FxHashSet<(Location, Place<'tcx>)>,
	/// The locals whose variant a library call's test reads.
	tested: FxHashSet<Local>,
}

impl<'tcx> Narrowing<'tcx> {
	/// Whether the switch on the discriminant read `at` tests its place widened.
	pub fn widened(&self, at: Location) -> bool {
		self.widened.contains(&at)
	}

	/// Whether `place`, read `at` as one of its variants, is cast to it.
	pub fn stale(&self, at: Location, place: Place<'tcx>) -> bool {
		self.stale.contains(&(at, place))
	}

	/// The locals whose variant a library call's test reads, which TypeScript narrows as
	/// it does a switch's.
	pub fn tested(&self) -> impl Iterator<Item = Local> + '_ {
		self.tested.iter().copied()
	}
}

/// Where `body`'s tests and payload reads must see a place's whole type.
pub fn narrowing<'tcx>(tcx: TyCtxt<'tcx>, crates: &Crates<'tcx>, body: &mir::Body<'tcx>) -> Narrowing<'tcx> {
	let mut flow = Flow::new(tcx, crates, body);
	// A widened test narrows nothing, which can leave a later test to widen too.
	loop {
		let mut seen = Narrowing::default();
		for (bb, facts) in flow.solve(body).into_iter_enumerated() {
			if let Some(facts) = facts {
				flow.block(body, bb, facts, Some(&mut seen));
			}
		}
		let widened: FxHashSet<BasicBlock> = seen.widened.iter().map(|at| at.block).collect();
		if widened.is_subset(&flow.widened) {
			return seen;
		}
		flow.widened.extend(widened);
	}
}

/// A switch's statement that reads its discriminant, if it tests a variant, and the arms
/// and default that can still match.
type Decided = (Option<usize>, Vec<(u128, BasicBlock)>, BasicBlock);

/// What a body's places are, for the dataflow.
struct Flow<'c, 'tcx> {
	tcx: TyCtxt<'tcx>,
	crates: &'c Crates<'tcx>,
	/// Each reference local whose one write is an assignment, as the place it borrows or
	/// the place behind the reference it copies.
	borrows: IndexVec<Local, Option<Place<'tcx>>>,
	/// Parameters written nowhere, which keep their value.
	kept: IndexVec<Local, bool>,
	/// Locals something can change behind the dataflow's back, by a raw pointer or a
	/// two-phase borrow.
	escaped: IndexVec<Local, bool>,
	/// The `bool`s a switch branches on, or on the negation of.
	conditions: IndexVec<Local, bool>,
	/// Each place a local's one write gives a variant, with its discriminant: an enum
	/// built, a constant one, or a reference to a constant one, as `x == Dot` compares to.
	made: Vec<(Place<'tcx>, u128)>,
	/// The switches on a variant that test their place widened, which narrows nothing.
	widened: FxHashSet<BasicBlock>,
}

impl<'c, 'tcx> Flow<'c, 'tcx> {
	fn new(tcx: TyCtxt<'tcx>, crates: &'c Crates<'tcx>, body: &mir::Body<'tcx>) -> Self {
		let n = body.local_decls.len();
		let mut writes = Writes(IndexVec::from_elem_n(0, n));
		writes.visit_body(body);
		let writes = writes.0;
		let mut rvalues: IndexVec<Local, Option<&Rvalue<'tcx>>> = IndexVec::from_elem_n(None, n);
		for data in body.basic_blocks.iter() {
			for stmt in &data.statements {
				if let StatementKind::Assign(assign) = &stmt.kind
					&& let Some(local) = assign.0.as_local()
				{
					rvalues[local] = Some(&assign.1);
				}
			}
		}
		let arg = |local: Local| (1..=body.arg_count).contains(&local.as_usize());
		let mut borrows = IndexVec::from_elem_n(None, n);
		for (local, rvalue) in rvalues.iter_enumerated() {
			// Its one write is this assignment, so nothing points it elsewhere.
			if writes[local] != 1 || arg(local) || !body.local_decls[local].ty.is_ref() {
				continue;
			}
			borrows[local] = match rvalue {
				Some(Rvalue::Ref(_, _, place)) => Some(*place),
				Some(Rvalue::Use(Operand::Copy(p) | Operand::Move(p), _) | Rvalue::CopyForDeref(p)) => {
					Some(tcx.mk_place_deref(*p))
				}
				_ => None,
			};
		}
		let made = rvalues
			.iter_enumerated()
			.filter(|&(local, _)| writes[local] == 1 && !arg(local))
			.filter_map(|(local, rvalue)| match (*rvalue)? {
				Rvalue::Aggregate(kind, _) => match **kind {
					AggregateKind::Adt(def, variant, ..) if tcx.adt_def(def).is_enum() => {
						let v = tcx.adt_def(def).discriminant_for_variant(tcx, variant).val;
						Some((Place::from(local), v))
					}
					_ => None,
				},
				Rvalue::Use(Operand::Constant(c), _) => {
					let place = match c.ty().is_ref() {
						true => tcx.mk_place_deref(Place::from(local)),
						false => Place::from(local),
					};
					Some((place, constant(tcx, body, c)?))
				}
				_ => None,
			})
			.collect();
		let kept = writes
			.iter_enumerated()
			.map(|(local, &w)| w == 0 && arg(local))
			.collect();
		let mut conditions: IndexVec<Local, bool> = IndexVec::from_elem_n(false, n);
		let mut work: Vec<Local> = body
			.basic_blocks
			.iter()
			.filter_map(|data| match &data.terminator().kind {
				TerminatorKind::SwitchInt { discr, .. } => discr.place()?.as_local(),
				_ => None,
			})
			.collect();
		while let Some(local) = work.pop() {
			if !body.local_decls[local].ty.is_bool() || std::mem::replace(&mut conditions[local], true) {
				continue;
			}
			if let Some(Rvalue::UnaryOp(mir::UnOp::Not, op) | Rvalue::Use(op, _)) = rvalues[local]
				&& let Some(from) = op.place().and_then(|p| p.as_local())
			{
				work.push(from);
			}
		}
		let mut flow = Self {
			tcx,
			crates,
			borrows,
			kept,
			escaped: IndexVec::from_elem_n(false, n),
			conditions,
			made,
			widened: FxHashSet::default(),
		};
		for data in body.basic_blocks.iter() {
			for stmt in &data.statements {
				let StatementKind::Assign(assign) = &stmt.kind else {
					continue;
				};
				let escapes = match &assign.1 {
					Rvalue::RawPtr(_, p) => Some(*p),
					// Reads go past a two-phase borrow before it writes, which then forgets
					// only what the reference is known to borrow.
					Rvalue::Ref(
						_,
						BorrowKind::Mut {
							kind: MutBorrowKind::TwoPhaseBorrow,
						},
						p,
					) if flow.canon(tcx.mk_place_deref(assign.0)).local == assign.0.local => Some(*p),
					_ => None,
				};
				if let Some(p) = escapes {
					let local = flow.canon(p).local;
					flow.escaped[local] = true;
				}
			}
		}
		flow
	}

	/// What is known as each block is entered, `None` where nothing reaches it.
	fn solve(&self, body: &mir::Body<'tcx>) -> IndexVec<BasicBlock, Option<Facts<'tcx>>> {
		let mut entry: IndexVec<BasicBlock, Option<Facts<'tcx>>> = IndexVec::from_elem(None, &body.basic_blocks);
		entry[mir::START_BLOCK] = Some(Facts::default());
		let mut work = vec![mir::START_BLOCK];
		while let Some(bb) = work.pop() {
			let Some(facts) = entry[bb].clone() else {
				continue;
			};
			for (to, out) in self.block(body, bb, facts, None).1 {
				let joined = match &entry[to] {
					None => out,
					Some(old) => old.join(&out),
				};
				if entry[to].as_ref() != Some(&joined) {
					entry[to] = Some(joined);
					work.push(to);
				}
			}
		}
		entry
	}

	/// `place`, through each reference it reads that borrows a place that stays put.
	fn canon(&self, mut place: Place<'tcx>) -> Place<'tcx> {
		while let [ProjectionElem::Deref, rest @ ..] = place.projection.as_slice()
			&& let Some(borrowed) = self.borrows[place.local]
		{
			let borrowed = self.canon(borrowed);
			let fixed = borrowed.projection.iter().enumerate().all(|(i, elem)| match elem {
				ProjectionElem::Deref => i == 0 && self.kept[borrowed.local],
				_ => !indexed(elem),
			});
			if !fixed {
				break;
			}
			place = borrowed.project_deeper(rest, self.tcx);
		}
		place
	}

	/// Whether the dataflow can follow the variant `place` holds.
	fn tracks(&self, body: &mir::Body<'tcx>, place: Place<'tcx>) -> bool {
		!self.escaped[place.local] && !place.projection.iter().any(indexed) && variants(self.tcx, body, place).is_some()
	}

	/// What a write to `place` makes unknown: `place` and what holds it, but not its own
	/// parts, which a variant's payload is.
	fn written(&self, place: Place<'tcx>) -> impl Fn(Place<'tcx>) -> bool + use<'tcx> {
		let place = self.canon(place);
		move |p| prefix(place, p)
	}

	/// What a copy of a pointer `place` holds can change: what it points at, and whatever
	/// is reached through a pointer it holds.
	fn lent(&self, body: &mir::Body<'tcx>, place: Place<'tcx>) -> Option<impl Fn(Place<'tcx>) -> bool + use<'tcx>> {
		let ty = place.ty(&body.local_decls, self.tcx).ty;
		if !lends(self.tcx, self.crates, ty) {
			return None;
		}
		let base = self.canon(place);
		let pointee = (ty.is_ref() || ty.is_raw_ptr()).then(|| self.canon(self.tcx.mk_place_deref(place)));
		Some(move |p: Place<'tcx>| {
			pointee.is_some_and(|t| prefix(t, p))
				|| p.projection.iter().enumerate().any(|(i, e)| {
					matches!(e, ProjectionElem::Deref)
						&& base.local == p.local
						&& p.projection[..i].starts_with(base.projection)
				})
		})
	}

	/// Whether TypeScript sees a write to `place` as an assignment to its path.
	fn visible(&self, place: Place<'tcx>) -> bool {
		self.canon(place) == place && !matches!(place.projection.last(), Some(ProjectionElem::Deref))
	}

	/// Whether a call to `func` may be written out as a test TypeScript narrows what it
	/// reads by: a call into the library, as it writes `PartialEq::eq` of an `Ordering`
	/// as `===` and `Result::is_ok` as `.ok`, whose result a switch branches on.
	fn tests(&self, body: &mir::Body<'tcx>, func: &Operand<'tcx>, destination: Place<'tcx>) -> bool {
		let Some((def, args)) = func.const_fn_def() else {
			return false;
		};
		destination.as_local().is_some_and(|local| self.conditions[local])
			&& !matches!(
				Instance::try_resolve(self.tcx, body.typing_env(self.tcx), def, args),
				Ok(Some(instance)) if self.crates.translates(instance.def_id())
			)
	}

	/// What assigning `rvalue` to `place` makes known of a `bool` that a switch tests: the
	/// value a constant gives it, or what is known of the `bool` it copies or negates.
	fn held(
		&self,
		body: &mir::Body<'tcx>,
		facts: &Facts<'tcx>,
		place: Place<'tcx>,
		rvalue: &Rvalue<'tcx>,
	) -> Option<Facts<'tcx>> {
		let b = place.as_local().filter(|&b| self.conditions[b] && !self.escaped[b])?;
		let (from, not) = match rvalue {
			Rvalue::Use(Operand::Constant(c), _) => {
				return Some(Facts {
					bools: vec![(b, c.const_.try_to_bool()?)],
					..Facts::default()
				});
			}
			Rvalue::Use(op, _) => (op, false),
			Rvalue::UnaryOp(mir::UnOp::Not, op) => (op, true),
			_ => return None,
		};
		let from = from.place()?.as_local()?;
		if !body.local_decls[from].ty.is_bool() {
			return None;
		}
		let bools = facts
			.bools
			.iter()
			.filter(|(c, _)| *c == from)
			.map(|&(_, v)| (b, v != not))
			.collect();
		let implied = facts
			.implied
			.iter()
			.filter(|(c, _, _)| *c == from)
			.map(|(_, p, [f, t])| match not {
				false => (b, *p, [f.clone(), t.clone()]),
				true => (b, *p, [t.clone(), f.clone()]),
			})
			.collect();
		Some(Facts {
			bools,
			implied,
			..Facts::default()
		})
	}

	/// What a call to an enum's derived `PartialEq::eq` or `ne` that sets a `bool` a switch
	/// tests says of the places it compares: where they are equal, they hold the same
	/// variant, and a variant without fields equals only itself.
	fn compared(
		&self,
		body: &mir::Body<'tcx>,
		facts: &Facts<'tcx>,
		func: &Operand<'tcx>,
		args: &[Spanned<Operand<'tcx>>],
		destination: Place<'tcx>,
	) -> Vec<Implication<'tcx>> {
		let tcx = self.tcx;
		let Some(b) = destination
			.as_local()
			.filter(|&b| self.conditions[b] && !self.escaped[b])
		else {
			return Vec::new();
		};
		let Some((def, generics)) = func.const_fn_def() else {
			return Vec::new();
		};
		let (eq, ne) = (trait_fn(tcx, "PartialEq", "eq"), trait_fn(tcx, "PartialEq", "ne"));
		if ![eq, ne].contains(&def) {
			return Vec::new();
		}
		let ty = generics.type_at(0);
		if generics.type_at(1) != ty || !ty.is_enum() {
			return Vec::new();
		}
		let resolve = |f| {
			Instance::try_resolve(tcx, body.typing_env(tcx), f, generics)
				.ok()
				.flatten()
				.map(|i| i.def_id())
		};
		let derived = resolve(eq)
			.and_then(|f| tcx.impl_of_assoc(f))
			.is_some_and(|imp| tcx.is_automatically_derived(imp));
		// A derived impl leaves `ne` to the trait's, which negates `eq`.
		if !derived || (def == ne && resolve(ne) != Some(ne)) {
			return Vec::new();
		}
		let [a, c] = args else {
			return Vec::new();
		};
		// Each side's place, if the dataflow follows it, and the variants it can hold.
		let side = |op: &Operand<'tcx>| -> (Option<Place<'tcx>>, Option<Vec<u128>>) {
			if let Operand::Constant(k) = op {
				return (None, constant(tcx, body, k).map(|v| vec![v]));
			}
			let Some(read) = op
				.place()
				.map(|p| self.canon(tcx.mk_place_deref(p)))
				.filter(|p| self.tracks(body, *p))
			else {
				return (None, None);
			};
			let known = facts.known.iter().find(|(p, _)| *p == read).map(|(_, vs)| vs.clone());
			let made = self.made.iter().find(|(p, _)| *p == read).map(|&(_, v)| vec![v]);
			(Some(read), known.or(made).or_else(|| variants(tcx, body, read)))
		};
		let sides = [side(&a.node), side(&c.node)];
		let ty::Adt(adt, _) = ty.kind() else {
			unreachable!("checked an enum")
		};
		let fieldless = |v: u128| {
			adt.discriminants(tcx)
				.any(|(i, d)| d.val == v && adt.variant(i).fields.is_empty())
		};
		let mut implied = Vec::new();
		for (i, (place, held)) in sides.iter().enumerate() {
			let (Some(place), Some(held)) = (place, held) else {
				continue;
			};
			let other = sides[1 - i].1.as_deref();
			let equal = held
				.iter()
				.copied()
				.filter(|v| other.is_none_or(|o| o.contains(v)))
				.collect();
			let unequal = match other {
				Some(&[v]) if fieldless(v) => held.iter().copied().filter(|&w| w != v).collect(),
				_ => held.clone(),
			};
			let [f, t] = match def == ne {
				false => [unequal, equal],
				true => [equal, unequal],
			};
			implied.push((b, *place, [Some(f), Some(t)]));
		}
		implied
	}

	/// The enums that `ops` pass by shared reference or by copy.
	fn read_places<'o>(&self, body: &mir::Body<'tcx>, ops: impl Iterator<Item = &'o Operand<'tcx>>) -> Vec<Place<'tcx>>
	where
		'tcx: 'o,
	{
		ops.filter_map(|op| {
			let p = op.place()?;
			let read = match p.ty(&body.local_decls, self.tcx).ty.kind() {
				ty::Ref(_, _, ty::Mutability::Not) => self.tcx.mk_place_deref(p),
				ty::Ref(..) | ty::RawPtr(..) => return None,
				_ => p,
			};
			Some(self.canon(read))
		})
		.filter(|p| self.tracks(body, *p))
		.collect()
	}

	/// Mark what `visit` reads as a variant where TypeScript may not have narrowed it to
	/// that variant: narrowed to others, or in a way the dataflow can't name.
	fn read_variants(
		&self,
		body: &mir::Body<'tcx>,
		facts: &Facts<'tcx>,
		location: Location,
		seen: &mut Narrowing<'tcx>,
		visit: impl FnOnce(&mut Downcasts<'tcx>),
	) {
		let mut reads = Downcasts(self.tcx, Vec::new());
		visit(&mut reads);
		for (base, variant) in reads.1 {
			let ty::Adt(adt, _) = base.ty(&body.local_decls, self.tcx).ty.kind() else {
				continue;
			};
			let discr = adt.discriminant_for_variant(self.tcx, variant).val;
			let stale = match facts.narrowing(self.canon(base)) {
				None => false,
				Some(None) => true,
				Some(Some(vs)) => vs != [discr],
			};
			if stale {
				seen.stale.insert((location, base));
			}
		}
	}

	/// Run `bb` from `facts`: the switch it decided, if any, and the facts on each edge
	/// that can be taken. With `seen`, record where TypeScript's narrowing is stale.
	fn block(
		&self,
		body: &mir::Body<'tcx>,
		bb: BasicBlock,
		mut facts: Facts<'tcx>,
		mut seen: Option<&mut Narrowing<'tcx>>,
	) -> (Option<Decided>, Vec<(BasicBlock, Facts<'tcx>)>) {
		let data = &body.basic_blocks[bb];
		// The discriminants read in this block so far: their locals, statements, and places.
		let mut discrs: Vec<(Local, usize, Place<'tcx>)> = Vec::new();
		for (i, stmt) in data.statements.iter().enumerate() {
			let location = Location {
				block: bb,
				statement_index: i,
			};
			if let Some(seen) = seen.as_deref_mut() {
				self.read_variants(body, &facts, location, seen, |v| v.visit_statement(stmt, location));
			}
			for lent in lent_places(|v| v.visit_statement(stmt, location)) {
				if let Some(gone) = self.lent(body, lent) {
					facts.forget(&mut discrs, gone);
				}
			}
			match &stmt.kind {
				StatementKind::Assign(assign) => {
					let (place, rvalue) = &**assign;
					// A copy holds the variant its original does, and TypeScript narrows
					// what it is assigned to the copy's type.
					let (copied, narrowed) = match rvalue {
						Rvalue::Use(Operand::Copy(from) | Operand::Move(from), _) | Rvalue::CopyForDeref(from) => {
							let (from, to) = (self.canon(*from), self.canon(*place));
							let rebase = |p: &Place<'tcx>| {
								let rest = &p.projection[from.projection.len()..];
								to.project_deeper(rest, self.tcx)
							};
							let copied = facts
								.known
								.iter()
								.filter(|(p, _)| prefix(from, *p))
								.map(|(p, vs)| (rebase(p), vs.clone()))
								.filter(|(p, _)| self.tracks(body, *p))
								.collect();
							let mut narrowed: Vec<(Place<'tcx>, Option<Vec<u128>>)> = facts
								.narrowed
								.iter()
								.filter(|(p, _)| prefix(from, *p))
								.map(|(p, vs)| (rebase(p), vs.clone()))
								.collect();
							if facts.narrowing(from) == Some(None) {
								narrowed.retain(|(p, _)| *p != to);
								narrowed.push((to, None));
							}
							(copied, narrowed)
						}
						_ => (Vec::new(), Vec::new()),
					};
					let held = self.held(body, &facts, *place, rvalue);
					let gone = self.written(*place);
					if self.visible(*place) {
						facts.narrowed.retain(|(p, _)| !gone(*p));
						facts.narrowed.extend(narrowed);
					}
					facts.forget(&mut discrs, gone);
					discrs.retain(|(d, _, _)| place.local != *d);
					// A `&mut` can change what it borrows from then on.
					if let Rvalue::Ref(_, BorrowKind::Mut { .. }, borrowed) = rvalue {
						facts.forget(&mut discrs, self.written(*borrowed));
					}
					facts.known.extend(copied);
					if let Some(held) = held {
						facts.bools.extend(held.bools);
						facts.implied.extend(held.implied);
					}
					if let Rvalue::Discriminant(of) = rvalue
						&& let Some(local) = place.as_local()
						&& self.tracks(body, self.canon(*of))
					{
						discrs.push((local, i, self.canon(*of)));
					}
				}
				StatementKind::SetDiscriminant { place, .. } => facts.forget(&mut discrs, self.written(**place)),
				StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
					facts.forget(&mut discrs, |p| p.local == *local);
				}
				_ => {}
			}
		}
		let term = data.terminator();
		let location = Location {
			block: bb,
			statement_index: data.statements.len(),
		};
		if let Some(seen) = seen.as_deref_mut() {
			self.read_variants(body, &facts, location, seen, |v| v.visit_terminator(term, location));
		}
		for lent in lent_places(|v| v.visit_terminator(term, location)) {
			if let Some(gone) = self.lent(body, lent) {
				facts.forget(&mut discrs, gone);
			}
		}
		let mut out: Vec<(BasicBlock, Facts<'tcx>)> = Vec::new();
		let mut decided = None;
		match &term.kind {
			TerminatorKind::SwitchInt {
				discr: Operand::Copy(d) | Operand::Move(d),
				targets,
			} if let Some(&(_, def, of)) = discrs.iter().find(|(l, _, _)| d.as_local() == Some(*l)) => {
				let all = variants(self.tcx, body, of).expect("checked where read");
				let known = facts
					.known
					.iter()
					.find(|(p, _)| *p == of)
					.map_or(all.clone(), |(_, vs)| vs.clone());
				let target = |v: u128| {
					targets
						.iter()
						.find(|&(w, _)| w == v)
						.map_or(targets.otherwise(), |(_, t)| t)
				};
				// A test for a variant TypeScript narrowed the place away from reads it
				// widened.
				let narrowed = facts.narrowing(of).map(|vs| vs.map(<[u128]>::to_vec));
				let widened = self.widened.contains(&bb);
				let stale = match &narrowed {
					None => false,
					Some(None) => targets.iter().next().is_some(),
					Some(Some(vs)) => targets.iter().any(|(v, _)| !vs.contains(&v)),
				};
				if (widened || stale)
					&& let Some(seen) = seen
				{
					seen.widened.insert(Location {
						block: bb,
						statement_index: def,
					});
				}
				// Each edge that can be taken, with the variants that take it.
				for &v in &known {
					match out.iter_mut().find(|(t, _)| *t == target(v)) {
						Some((_, facts)) => facts.known.last_mut().expect("pushed below").1.push(v),
						None => {
							let mut facts = facts.clone();
							facts.known.retain(|(p, _)| *p != of);
							facts.known.push((of, vec![v]));
							out.push((target(v), facts));
						}
					}
				}
				for (to, facts) in &mut out {
					facts.known.last_mut().expect("pushed above").1.sort_unstable();
					// What TypeScript narrowed it to, less what the tests on the way rule out.
					if !widened {
						let left = narrowed.clone().flatten().unwrap_or_else(|| all.clone());
						facts.narrow(of, Some(left.into_iter().filter(|&v| target(v) == *to).collect()));
					}
				}
				let mut kept: Vec<(u128, BasicBlock)> = targets.iter().filter(|(v, _)| known.contains(v)).collect();
				let listed = |v: &u128| targets.iter().any(|(w, _)| w == *v);
				let mut otherwise = targets.otherwise();
				// Where nothing reaches the default now, the last arm left takes its place. So
				// it does of an unreachable default, which the arms dropped would otherwise
				// leave the last arm to stand for untested, wider than what it matches.
				let dropped = kept.len() < targets.iter().count();
				if (all.iter().any(|v| !listed(v)) || dropped)
					&& known.iter().all(listed)
					&& let Some((_, last)) = kept.pop()
				{
					otherwise = last;
				}
				decided = Some((Some(def), kept, otherwise));
			}
			// A test of a `bool` narrows what its value says.
			TerminatorKind::SwitchInt {
				discr: Operand::Copy(d) | Operand::Move(d),
				targets,
			} if let Some(b) = d.as_local()
				&& body.local_decls[b].ty.is_bool()
				&& !self.escaped[b] =>
			{
				for &to in targets.all_targets() {
					if out.iter().any(|(t, _)| *t == to) {
						continue;
					}
					let mut facts = facts.clone();
					let mut values = [false, true]
						.into_iter()
						.filter(|&v| targets.target_for_value(v as u128) == to);
					let taken = match (values.next(), values.next()) {
						(Some(v), None) => facts.assume(b, v),
						(v, _) => v.is_some(),
					};
					if taken {
						out.push((to, facts));
					}
				}
				// An edge it can't take goes, as a switch on a variant's does.
				let reached = |t: BasicBlock| out.iter().any(|(to, _)| *to == t);
				let mut kept: Vec<(u128, BasicBlock)> = targets.iter().filter(|&(_, t)| reached(t)).collect();
				let mut otherwise = targets.otherwise();
				if !reached(otherwise)
					&& let Some((_, last)) = kept.pop()
				{
					otherwise = last;
				}
				if reached(otherwise) {
					decided = Some((None, kept, otherwise));
				}
			}
			// A test of anything else, as an integer's, changes nothing.
			TerminatorKind::SwitchInt { targets, .. } => {
				for &to in targets.all_targets() {
					if !out.iter().any(|(t, _)| *t == to) {
						out.push((to, facts.clone()));
					}
				}
			}
			TerminatorKind::Goto { target, .. }
			| TerminatorKind::Assert { target, .. }
			| TerminatorKind::FalseEdge {
				real_target: target, ..
			}
			| TerminatorKind::FalseUnwind {
				real_target: target, ..
			} => out.push((*target, facts.clone())),
			TerminatorKind::Call {
				func,
				args,
				destination,
				target: Some(target),
				..
			} => {
				let mut facts = facts.clone();
				let gone = self.written(*destination);
				if self.visible(*destination) {
					facts.narrowed.retain(|(p, _)| !gone(*p));
				}
				facts.forget(&mut discrs, gone);
				facts
					.implied
					.extend(self.compared(body, &facts, func, args, *destination));
				if self.tests(body, func, *destination) {
					for read in self.read_places(body, args.iter().map(|arg| &arg.node)) {
						facts.narrow(read, None);
						if let Some(seen) = seen.as_deref_mut() {
							seen.tested.insert(read.local);
						}
					}
				}
				out.push((*target, facts));
			}
			TerminatorKind::Drop { place, target, .. } => {
				let mut facts = facts.clone();
				facts.forget(&mut discrs, self.written(*place));
				// Drop glue gets a `&mut` to the value.
				if let Some(gone) = self.lent(body, *place) {
					facts.forget(&mut discrs, gone);
				}
				out.push((*target, facts));
			}
			_ => {}
		}
		// Unwinding, and whatever else leaves, knows nothing. A switch's edges are those it
		// can take.
		if !matches!(term.kind, TerminatorKind::SwitchInt { .. }) {
			for to in term.successors() {
				if !out.iter().any(|(t, _)| *t == to) {
					out.push((to, Facts::default()));
				}
			}
		}
		(decided, out)
	}
}

/// How often each local is written itself, not through a pointer it holds: assigned, a
/// call's destination, dropped, or borrowed `&mut` or raw, which can write it later.
struct Writes(IndexVec<Local, usize>);

impl<'tcx> Visitor<'tcx> for Writes {
	fn visit_place(&mut self, place: &Place<'tcx>, context: PlaceContext, _: Location) {
		let writes =
			context.is_mutating_use() || context == PlaceContext::NonMutatingUse(NonMutatingUseContext::RawBorrow);
		if writes && !place.is_indirect() {
			self.0[place.local] += 1;
		}
	}
}

/// The places a statement or terminator copies or moves, and that a `CopyForDeref`
/// reads, as `visit` walks it.
fn lent_places<'tcx>(visit: impl FnOnce(&mut Operands<'tcx>)) -> Vec<Place<'tcx>> {
	let mut ops = Operands(Vec::new());
	visit(&mut ops);
	ops.0
}

struct Operands<'tcx>(Vec<Place<'tcx>>);

impl<'tcx> Visitor<'tcx> for Operands<'tcx> {
	fn visit_operand(&mut self, op: &Operand<'tcx>, _: Location) {
		if let Operand::Copy(p) | Operand::Move(p) = op {
			self.0.push(*p);
		}
	}

	fn visit_rvalue(&mut self, rvalue: &Rvalue<'tcx>, location: Location) {
		if let Rvalue::CopyForDeref(p) = rvalue {
			self.0.push(*p);
		}
		self.super_rvalue(rvalue, location);
	}
}

/// The places a statement or terminator reads or writes as one of their variants, with
/// that variant.
struct Downcasts<'tcx>(TyCtxt<'tcx>, Vec<(Place<'tcx>, VariantIdx)>);

impl<'tcx> Visitor<'tcx> for Downcasts<'tcx> {
	fn visit_place(&mut self, place: &Place<'tcx>, _: PlaceContext, _: Location) {
		for (base, elem) in place.iter_projections() {
			if let ProjectionElem::Downcast(_, variant) = elem {
				self.1.push((base.to_place(self.0), variant));
			}
		}
	}
}

/// The variants in either of `a` and `b`, in order.
fn union(a: &[u128], b: &[u128]) -> Vec<u128> {
	let mut all: Vec<u128> = a.iter().chain(b).copied().collect();
	all.sort_unstable();
	all.dedup();
	all
}

/// Whether a value of `ty` can change what it points at: it is or holds a `&mut` or a
/// `*mut`, or hides what it holds, as a type whose fields hold ever deeper instances, past
/// the crate's recursion limit, may.
fn lends<'tcx>(tcx: TyCtxt<'tcx>, crates: &Crates<'tcx>, ty: Ty<'tcx>) -> bool {
	let mut seen = FxHashSet::default();
	let mut stack = vec![(ty, 0)];
	while let Some((ty, depth)) = stack.pop() {
		if depth > tcx.recursion_limit().0 {
			return true;
		}
		for t in ty.walk().filter_map(|arg| arg.as_type()) {
			match t.kind() {
				ty::Ref(_, _, ty::Mutability::Mut)
				| ty::RawPtr(_, ty::Mutability::Mut)
				| ty::Dynamic(..)
				| ty::Alias(..)
				| ty::Param(_) => return true,
				ty::Adt(adt, args) if crates.translates(adt.did()) && seen.insert(t) => {
					stack.extend(
						adt.all_fields()
							.map(|field| (field.ty(tcx, args).skip_norm_wip(), depth + 1)),
					);
				}
				_ => {}
			}
		}
	}
	false
}

/// Whether `elem` picks an element, which a write to another element can be.
fn indexed(elem: ProjectionElem<Local, Ty<'_>>) -> bool {
	matches!(
		elem,
		ProjectionElem::Index(_) | ProjectionElem::ConstantIndex { .. } | ProjectionElem::Subslice { .. }
	)
}

/// Whether `a` is `b` or holds it.
fn prefix<'tcx>(a: Place<'tcx>, b: Place<'tcx>) -> bool {
	a.local == b.local && b.projection.starts_with(a.projection)
}

/// The discriminants of the enum at `place`, if it is one.
fn variants<'tcx>(tcx: TyCtxt<'tcx>, body: &mir::Body<'tcx>, place: Place<'tcx>) -> Option<Vec<u128>> {
	match place.ty(&body.local_decls, tcx).ty.kind() {
		ty::Adt(adt, _) if adt.is_enum() => Some(adt.discriminants(tcx).map(|(_, d)| d.val).collect()),
		_ => None,
	}
}

/// The discriminant of the enum constant `c` is or points at.
fn constant<'tcx>(tcx: TyCtxt<'tcx>, body: &mir::Body<'tcx>, c: &ConstOperand<'tcx>) -> Option<u128> {
	let ty = c.ty().builtin_deref(true).unwrap_or(c.ty());
	let ty::Adt(adt, _) = ty.kind() else {
		return None;
	};
	let mir::Const::Unevaluated(uv, _) = c.const_ else {
		return None;
	};
	if !adt.is_enum() {
		return None;
	}
	let env = body.typing_env(tcx);
	let instance = Instance::try_resolve(tcx, env, uv.def, uv.args).ok()??;
	let cid = GlobalId {
		instance,
		promoted: uv.promoted,
	};
	// A reference's value tree is what it points at's, and an enum's starts with its variant.
	let valtree = tcx.const_eval_global_id_for_typeck(env, cid, c.span).ok()?.ok()?;
	let ty::ValTreeKind::Branch(fields) = &*valtree else {
		return None;
	};
	let ty::ConstKind::Value(variant) = fields.first()?.kind() else {
		return None;
	};
	let index = VariantIdx::from_u32(variant.valtree.try_to_leaf()?.to_u32());
	Some(adt.discriminant_for_variant(tcx, index).val)
}
