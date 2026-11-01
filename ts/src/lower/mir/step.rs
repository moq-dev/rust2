//! Iterators stepped in place: a slice's or a `Vec`'s iterator, or a range of numbers,
//! that only `next` reads, as a `for` loop's is, is two variables, of the array and an
//! index or of the bounds, rather than an `rt.Iter`, which a bundler can't shake its
//! adaptors from. Variables, not an object of both: V8 took up to 1.25x as long over a
//! `BTreeMap` stepping an object's fields.

use rustc_data_structures::fx::{FxHashMap, FxHashSet};
use rustc_middle::mir::{self, Local, Operand, Rvalue, StatementKind, TerminatorKind};
use rustc_middle::ty::{self, Ty, TyCtxt};

use super::Fx;
use crate::lower::library::{self, Std};
use crate::ts::{Expr, Key, Stmt, StmtKind, Type};

/// The item type of a slice's or a `Vec`'s iterator, if `ty` is one.
fn array_iter<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
	let ty::Adt(adt, args) = ty.kind() else {
		return None;
	};
	let path = library::std_path(tcx, adt.did());
	let path: Vec<&str> = path.iter().map(String::as_str).collect();
	match path.as_slice() {
		// A reference to an array of bytes is a slice of it made as the item is read.
		["slice", "iter", "Iter" | "IterMut"] if args.types().next().is_some_and(crate::lower::byte_array) => None,
		["slice", "iter", "Iter" | "IterMut"] | ["vec", "into_iter", "IntoIter"] => args.types().next(),
		_ => None,
	}
}

/// Whether `ty` is what such an iterator is made from: a slice, an array, or a `Vec`.
fn array<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
	match ty.peel_refs().kind() {
		ty::Slice(_) | ty::Array(..) => true,
		ty::Adt(adt, _) => Std::of(tcx, adt.did()) == Some(Std::Vec),
		_ => false,
	}
}

impl<'tcx> Fx<'_, '_, '_, 'tcx> {
	/// Find the iterators to step in place: each made by a call from an array, or moved
	/// from another, and otherwise only borrowed for `next` into a protocol local and
	/// dropped. Each borrow is read only as `next`'s receiver.
	pub(super) fn find_stepped(&mut self) {
		let mir = self.mir;
		let tcx = self.tcx;
		let mut iters: Vec<Local> = mir
			.local_decls
			.indices()
			.filter(|&l| !self.is_arg(l) && l != mir::RETURN_PLACE && self.defs[l] == 1)
			.filter(|&l| {
				let ty = mir.local_decls[l].ty;
				// An element JS copies is lent as its place, which `rt.iterSliceMut` makes.
				let places = || self.b.t.iter_item(ty).is_some_and(|item| self.b.t.has_place(item));
				array_iter(tcx, ty).is_some() && !places() || super::small_range(tcx, ty)
			})
			.collect();
		if iters.is_empty() {
			return;
		}
		// What each reads that is allowed: borrows for `next`, drops, moves into another.
		let mut reads: FxHashMap<Local, u32> = FxHashMap::default();
		let mut made = FxHashSet::default();
		let mut moves: Vec<(Local, Local)> = Vec::new();
		let mut borrows: FxHashMap<Local, Local> = FxHashMap::default();
		let mut next_reads: FxHashMap<Local, u32> = FxHashMap::default();
		for data in mir.basic_blocks.iter() {
			for stmt in &data.statements {
				let StatementKind::Assign(assign) = &stmt.kind else {
					continue;
				};
				let (place, rvalue) = &**assign;
				match rvalue {
					Rvalue::Ref(_, mir::BorrowKind::Mut { .. }, from) if from.as_local().is_some() => {
						let from = from.as_local().unwrap();
						*reads.entry(from).or_default() += 1;
						if let Some(to) = place.as_local() {
							borrows.insert(to, from);
						}
					}
					Rvalue::Use(Operand::Move(from), _) if from.as_local().is_some() => {
						let from = from.as_local().unwrap();
						*reads.entry(from).or_default() += 1;
						if let Some(to) = place.as_local() {
							moves.push((from, to));
						}
					}
					Rvalue::Aggregate(..) if let Some(to) = place.as_local() => {
						made.insert(to);
					}
					_ => {}
				}
			}
			match &data.terminator().kind {
				// Counted as `uses` counts it: only a drop JS sees.
				TerminatorKind::Drop { place, .. }
					if let Some(local) = place.as_local()
						&& self.b.t.drops(mir.local_decls[local].ty) =>
				{
					*reads.entry(local).or_default() += 1;
				}
				TerminatorKind::Call {
					func,
					args,
					destination,
					..
				} => {
					let ty::FnDef(def_id, _) = *func.ty(&mir.local_decls, tcx).kind() else {
						continue;
					};
					let maker = library::key(tcx, def_id).is_some_and(|(o, n)| {
						matches!(
							(o.as_str(), n.as_str()),
							("IntoIterator", "into_iter") | ("slice", "iter" | "iter_mut")
						)
					});
					if maker
						&& let Some(to) = destination.as_local()
						&& let [arg] = &**args
					{
						match &arg.node {
							// A range is its own iterator.
							Operand::Move(from)
								if let Some(from) = from.as_local()
									&& super::small_range(tcx, mir.local_decls[from].ty) =>
							{
								*reads.entry(from).or_default() += 1;
								moves.push((from, to));
							}
							op if array(tcx, op.ty(&mir.local_decls, tcx)) => {
								made.insert(to);
							}
							_ => {}
						}
					}
					if crate::lower::iter::is_next(tcx, def_id)
						&& destination.as_local().is_some_and(|l| self.next_results.contains(&l))
						&& let [arg] = &**args
						&& let Operand::Copy(p) | Operand::Move(p) = &arg.node
						&& let Some(recv) = p.as_local()
					{
						*next_reads.entry(recv).or_default() += 1;
					}
				}
				_ => {}
			}
		}
		// A borrow only `next` reads, assigned once and borrowed by nothing.
		let stepping = |fx: &Self, borrow: Local| {
			fx.defs[borrow] == 1
				&& !fx.borrowed[borrow]
				&& next_reads.get(&borrow).copied().unwrap_or(0) == fx.uses[borrow]
		};
		let mut ok: FxHashSet<Local> = iters.drain(..).collect();
		ok.retain(|&l| reads.get(&l).copied().unwrap_or(0) == self.uses[l]);
		ok.retain(|&l| made.contains(&l) || moves.iter().any(|&(_, to)| to == l));
		ok.retain(|&l| borrows.iter().all(|(&b, &from)| from != l || stepping(self, b)));
		// A move joins two iterators that must both step in place, or neither.
		loop {
			let before = ok.len();
			for &(from, to) in &moves {
				if ok.contains(&from) != ok.contains(&to) {
					ok.remove(&from);
					ok.remove(&to);
				}
			}
			if ok.len() == before {
				break;
			}
		}
		for (b, from) in borrows {
			if ok.contains(&from) {
				self.stepped_borrows.insert(b, from);
			}
		}
		// One moved into another and read nowhere else shares its variables: the other is
		// assigned only by the move, so neither is live while the other is.
		for &(from, to) in &moves {
			if ok.contains(&from) && self.uses[from] == 1 {
				self.stepped_into.insert(from, to);
			}
		}
		self.stepped = ok;
	}

	/// The local whose variables `local` holds its fields in: the last it is moved into.
	pub(super) fn stepped_root(&self, mut local: Local) -> Local {
		while let Some(&to) = self.stepped_into.get(&local) {
			local = to;
		}
		local
	}

	/// The array iterator a `next` receiver `op` borrows, if it steps in place. A range's
	/// `next` steps one in place anyway.
	pub(super) fn stepped_array(&self, op: &Operand<'tcx>) -> bool {
		let (Operand::Copy(p) | Operand::Move(p)) = op else {
			return false;
		};
		p.as_local()
			.and_then(|b| self.stepped_borrows.get(&b))
			.is_some_and(|&l| array_iter(self.tcx, self.mir.local_decls[l].ty).is_some())
	}

	/// The keys of `local`'s fields: its array and index, or its bounds.
	fn stepped_keys(&self, local: Local) -> [&'static str; 2] {
		match array_iter(self.tcx, self.mir.local_decls[local].ty) {
			Some(_) => ["s", "i"],
			None => ["start", "end"],
		}
	}

	/// Name the two variables `local` holds its fields in, unless it shares another's. A
	/// read of either counts as one of memory, since `next` changes them in place.
	pub(super) fn name_stepped(&mut self, local: Local, base: &str) {
		if self.stepped_root(local) != local {
			return;
		}
		let vars = self.stepped_keys(local).map(|key| {
			let mut suffix = key.to_string();
			suffix[..1].make_ascii_uppercase();
			let name = self.b.fresh(&format!("{base}{suffix}"));
			self.shared.insert(name.clone());
			name
		});
		self.stepped_vars.insert(local, vars);
	}

	/// Give each stepped local that shares its variables those of the local it shares.
	pub(super) fn share_stepped(&mut self) {
		for &local in &self.stepped {
			let root = self.stepped_root(local);
			if root != local {
				let vars = self.stepped_vars[&root].clone();
				self.stepped_vars.insert(local, vars);
			}
		}
	}

	/// `local` read whole, if it is stepped in place: an object of its variables, which
	/// [`Self::set_stepped`] and [`stepped_field`] take apart again.
	pub(super) fn stepped_value(&self, local: Local) -> Option<Expr> {
		let vars = self.stepped_vars.get(&local)?;
		let keys = self.stepped_keys(local);
		let fields = keys.into_iter().zip(vars);
		Some(Expr::Object(
			fields.map(|(k, v)| (Key::Name(k.into()), Expr::ident(v))).collect(),
		))
	}

	/// Assign `local`'s variables the fields of `value`, as [`Self::stepped_make`] made it.
	pub(super) fn set_stepped(&mut self, local: Local, value: Expr, out: &mut Vec<Stmt>) {
		let value = match value {
			Expr::Located(_, value) => *value,
			value => value,
		};
		// Anything else is refused already.
		let Expr::Object(fields) = value else {
			return;
		};
		self.assigned[local] = true;
		let vars = self.stepped_vars[&local].clone();
		for ((_, value), var) in fields.into_iter().zip(vars) {
			// A move from a local that shares the variables.
			if matches!(&value, Expr::Ident(name) if *name == var) {
				continue;
			}
			let stmt = StmtKind::Expr(Expr::ident(&var).assign(value)).at(None);
			self.emit(vec![stmt], false, out);
		}
	}

	/// An iterator stepped in place, from the `rt.Iter` its mapping makes: `{ s, i: 0 }`
	/// from `rt.iterSlice(s)`, or `rt.iterSlice(s, drop)` of items it owns, which its drop
	/// drops again, and `{ start, end }` from `new rt.Range(start, end)`. A moved one is
	/// already made.
	pub(super) fn stepped_make(&mut self, value: Expr, span: rustc_span::Span) -> Expr {
		let rt = |f: &Expr, name: &str| matches!(f, Expr::Rt(n) if n == name);
		let object = |fields: [(&str, Expr); 2]| {
			Expr::Object(fields.into_iter().map(|(k, v)| (Key::Name(k.into()), v)).collect())
		};
		match value {
			Expr::Call(f, mut args) if rt(&f, "iterSlice") && matches!(args.len(), 1 | 2) => {
				args.truncate(1);
				object([("s", args.pop().unwrap()), ("i", Expr::lit("0"))])
			}
			Expr::New(f, args) if rt(&f, "Range") && args.len() == 2 => {
				let [start, end] = <[Expr; 2]>::try_from(args).expect("checked above");
				object([("start", start), ("end", end)])
			}
			Expr::Object(_) => value,
			// Refused already.
			Expr::Lit(lit) if lit == "undefined" => Expr::undefined(),
			_ => self.err(span, "an iterator stepped in place that its mapping didn't make"),
		}
	}

	/// The next item of `it`, stepped in place, or `rt.DONE`.
	pub(super) fn stepped_next(&self, it: &Expr) -> Expr {
		let s = stepped_field(it, "s");
		let i = stepped_field(it, "i");
		Expr::Cond(
			Box::new(i.clone().binary("<", s.clone().member("length"))),
			Box::new(s.index(Expr::Postfix(Box::new(i), "++"))),
			Box::new(Expr::Rt("DONE".into())),
		)
	}

	/// What drops a `Vec`'s iterator stepped in place, of type `ty`, as `it`: the items from
	/// its index on.
	pub(super) fn stepped_drop(&mut self, it: &Expr, ty: Ty<'tcx>, span: rustc_span::Span) -> Option<Expr> {
		let item = array_iter(self.tcx, ty)?;
		let dropper = self.b.t.drop_fn(item, span)?;
		Some(Expr::rt(
			"dropAll",
			vec![stepped_field(it, "s"), dropper, stepped_field(it, "i")],
		))
	}

	/// The types of a stepped local's variables: its array and index, or its bounds.
	pub(super) fn stepped_tys(&mut self, local: Local) -> [Type; 2] {
		let ty = self.mir.local_decls[local].ty;
		let span = self.local_span(local);
		match array_iter(self.tcx, ty) {
			// What `library::elements` makes of the slice: a view of bytes.
			Some(item) => [
				match item.kind() {
					ty::Uint(ty::UintTy::U8) => Type::Builtin("Uint8Array"),
					_ => self.b.t.held_ty(Ty::new_slice(self.tcx, item), span),
				},
				Type::Builtin("number"),
			],
			None => {
				let ty::Adt(_, args) = ty.kind() else {
					unreachable!("a range is a struct")
				};
				let bound = self.b.t.held_ty(args.type_at(0), span);
				[bound.clone(), bound]
			}
		}
	}
}

/// The field `key` of an iterator stepped in place: its variable, from the object
/// [`Fx::stepped_value`] reads it as.
pub(super) fn stepped_field(it: &Expr, key: &str) -> Expr {
	match it {
		Expr::Object(fields) => fields
			.iter()
			.find(|(k, _)| matches!(k, Key::Name(n) if n == key))
			.map(|(_, v)| v.clone())
			.expect("a stepped iterator's field"),
		it => it.clone().member(key),
	}
}

/// Each loop over an array iterator stepped in place that only the loop steps, as
/// `for (const item of array)`: a `while (true)` whose first statements take the next
/// item and leave on `rt.DONE`. Nothing else names the index, so nothing sees where it
/// stopped, but for the `rt.dropAll` of a `Vec`'s items it owns right after the loop,
/// which drops nothing when the loop can only leave at the end.
pub(super) fn for_of(stmts: &mut Vec<Stmt>) {
	let mut names: FxHashMap<String, usize> = FxHashMap::default();
	for stmt in stmts.iter() {
		super::visit_stmt(stmt, &mut |e| {
			super::visit_expr(e, &mut |e| {
				if let Expr::Ident(name) = e {
					*names.entry(name.clone()).or_default() += 1;
				}
			})
		});
	}
	// The variables declared `= 0`, as an index starts.
	let mut zeros = FxHashSet::default();
	decls(stmts, &mut |name, init| {
		if matches!(init, Expr::Lit(l) if l == "0") {
			zeros.insert(name.to_string());
		}
	});
	let mut indexes = FxHashSet::default();
	loops(stmts, &names, &zeros, &mut indexes);
	if !indexes.is_empty() {
		drop_decls(stmts, &indexes);
	}
}

/// Call `f` on the name and value of each declaration in `stmts` that has one.
fn decls(stmts: &[Stmt], f: &mut dyn FnMut(&str, &Expr)) {
	for stmt in stmts {
		match &stmt.kind {
			StmtKind::Decl {
				name, init: Some(init), ..
			} => f(name, init),
			StmtKind::If { then, els, .. } => {
				decls(then, f);
				decls(els, f);
			}
			StmtKind::Block { body, .. } | StmtKind::While { body, .. } | StmtKind::ForOf { body, .. } => {
				decls(body, f)
			}
			StmtKind::Switch { cases, default, .. } => {
				cases.iter().for_each(|(_, body)| decls(body, f));
				decls(default, f);
			}
			_ => {}
		}
	}
}

/// Rewrite the loops in `stmts`, collecting the indexes they no longer read.
fn loops(
	stmts: &mut Vec<Stmt>,
	names: &FxHashMap<String, usize>,
	zeros: &FxHashSet<String>,
	indexes: &mut FxHashSet<String>,
) {
	let mut at = 0;
	while at < stmts.len() {
		let rest = stmts.get(at + 1).and_then(drop_rest);
		let stmt = &mut stmts[at];
		// The array and the index are declared, and named only by this loop's header, and
		// by a drop of what it leaves if it can only leave at the end.
		if let StmtKind::While { label, cond, body } = &mut stmt.kind
			&& matches!(cond, Expr::Lit(l) if l == "true")
			&& let Some((item, array, index)) = header(body, label.as_deref())
			&& let owned = (rest.as_ref().is_some_and(|(a, i)| *a == array && *i == index)
				&& !leaves(&body[2..], label.as_deref(), &mut Vec::new(), false))
			&& let uses = if owned { 3 } else { 2 }
			&& names.get(&array) == Some(&uses)
			&& names.get(&index) == Some(&uses)
			&& zeros.contains(&index)
		{
			let mut body: Vec<Stmt> = std::mem::take(body).into_iter().skip(2).collect();
			let mut name = item;
			// `const entry = item;` names the item, when nothing else reads it.
			if let Some(StmtKind::Decl {
				mutable: false,
				name: to,
				init: Some(Expr::Ident(from)),
				..
			}) = body.first().map(|s| &s.kind)
				&& *from == name
				&& super::list_names(&body, &name) == 1
			{
				name = to.clone();
				body.remove(0);
			}
			indexes.insert(index);
			stmt.kind = StmtKind::ForOf {
				label: label.take(),
				name,
				iter: Expr::ident(array),
				body,
			};
			if owned {
				stmts.remove(at + 1);
			}
		}
		let stmt = &mut stmts[at];
		at += 1;
		match &mut stmt.kind {
			StmtKind::If { then, els, .. } => {
				loops(then, names, zeros, indexes);
				loops(els, names, zeros, indexes);
			}
			StmtKind::Block { body, .. } | StmtKind::While { body, .. } | StmtKind::ForOf { body, .. } => {
				loops(body, names, zeros, indexes)
			}
			StmtKind::Switch { cases, default, .. } => {
				cases
					.iter_mut()
					.for_each(|(_, body)| loops(body, names, zeros, indexes));
				loops(default, names, zeros, indexes);
			}
			_ => {}
		}
	}
}

/// The array and index of `rt.dropAll(array, f, index)`, which drops what a loop over a
/// `Vec` it owns left.
fn drop_rest(stmt: &Stmt) -> Option<(String, String)> {
	let StmtKind::Expr(call) = &stmt.kind else {
		return None;
	};
	let Expr::Call(f, args) = peel(call) else {
		return None;
	};
	match (peel(f), args.as_slice()) {
		(Expr::Rt(name), [array, _, index]) if name == "dropAll" => match (peel(array), peel(index)) {
			(Expr::Ident(array), Expr::Ident(index)) => Some((array.clone(), index.clone())),
			_ => None,
		},
		_ => None,
	}
}

/// Whether `stmts`, in the loop labeled `label`, leave it other than at its end: a
/// `return`, or a `break` or `continue` to a statement around it. `inner` holds the labels
/// declared in between, and `nested` whether a loop or `switch` is, which an unlabeled
/// `break` leaves instead. A panic aborts, so it leaves nothing to drop.
fn leaves(stmts: &[Stmt], label: Option<&str>, inner: &mut Vec<String>, nested: bool) -> bool {
	stmts.iter().any(|stmt| match &stmt.kind {
		StmtKind::Return(_) => true,
		StmtKind::Break(None) => !nested,
		StmtKind::Break(Some(to)) => !inner.contains(to),
		StmtKind::Continue(Some(to)) => Some(to.as_str()) != label && !inner.contains(to),
		StmtKind::Continue(None) => false,
		StmtKind::If { then, els, .. } => leaves(then, label, inner, nested) || leaves(els, label, inner, nested),
		StmtKind::Block { label: own, body } => within(own, inner, |inner| leaves(body, label, inner, nested)),
		StmtKind::While { label: own, body, .. } | StmtKind::ForOf { label: own, body, .. } => {
			within(own, inner, |inner| leaves(body, label, inner, true))
		}
		StmtKind::Switch {
			label: own,
			cases,
			default,
			..
		} => within(own, inner, |inner| {
			cases.iter().any(|(_, body)| leaves(body, label, inner, true)) || leaves(default, label, inner, true)
		}),
		StmtKind::Expr(_) | StmtKind::Decl { .. } | StmtKind::Declare { .. } => false,
	})
}

/// `f` with `own`, a label, among the `inner` ones.
fn within(own: &Option<String>, inner: &mut Vec<String>, f: impl FnOnce(&mut Vec<String>) -> bool) -> bool {
	inner.extend(own.clone());
	let leaves = f(inner);
	if own.is_some() {
		inner.pop();
	}
	leaves
}

/// The item, array, and index of a loop that starts `const item = i < s.length ?
/// s[i++] : rt.DONE; if (item === rt.DONE) break;`, the `break` leaving it.
fn header(body: &[Stmt], label: Option<&str>) -> Option<(String, String, String)> {
	let [next, test, ..] = body else {
		return None;
	};
	let StmtKind::Decl {
		mutable: false,
		name: item,
		init: Some(init),
		..
	} = &next.kind
	else {
		return None;
	};
	let Expr::Cond(c, a, b) = peel(init) else {
		return None;
	};
	let (Expr::Binary(i, "<", len), Expr::Index(s, at), Expr::Rt(done)) = (peel(c), peel(a), peel(b)) else {
		return None;
	};
	let (Expr::Ident(index), Expr::Member(s2, Key::Name(length)), Expr::Ident(array), Expr::Postfix(i2, "++")) =
		(peel(i), peel(len), peel(s), peel(at))
	else {
		return None;
	};
	let same = |e: &Expr, name: &str| matches!(peel(e), Expr::Ident(n) if n == name);
	if done != "DONE" || length != "length" || !same(s2, array) || !same(i2, index) {
		return None;
	}
	let StmtKind::If { cond, then, els } = &test.kind else {
		return None;
	};
	let leaves = matches!(then.as_slice(), [Stmt { kind: StmtKind::Break(target), .. }]
		if target.is_none() || target.as_deref() == label);
	let tests = matches!(peel(cond), Expr::Binary(l, "===", r)
		if same(l, item) && matches!(peel(r), Expr::Rt(d) if d == "DONE"));
	(leaves && tests && els.is_empty()).then(|| (item.clone(), array.clone(), index.clone()))
}

/// `e` without its source location.
pub(super) fn peel(e: &Expr) -> &Expr {
	match e {
		Expr::Located(_, e) => peel(e),
		e => e,
	}
}

/// Drop the declarations of `names`, which nothing reads any more.
fn drop_decls(stmts: &mut Vec<Stmt>, names: &FxHashSet<String>) {
	stmts.retain(|s| !matches!(&s.kind, StmtKind::Decl { name, .. } if names.contains(name)));
	for stmt in stmts.iter_mut() {
		match &mut stmt.kind {
			StmtKind::If { then, els, .. } => {
				drop_decls(then, names);
				drop_decls(els, names);
			}
			StmtKind::Block { body, .. } | StmtKind::While { body, .. } | StmtKind::ForOf { body, .. } => {
				drop_decls(body, names)
			}
			StmtKind::Switch { cases, default, .. } => {
				cases.iter_mut().for_each(|(_, body)| drop_decls(body, names));
				drop_decls(default, names);
			}
			_ => {}
		}
	}
}
