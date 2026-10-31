//! Calls: to the crate's own functions, to closures, and to std through the library
//! mappings. Some std calls lower here instead, since MIR passes what THIR wrote as an
//! expression through a temporary: a place to replace (`mem::take`), a range to slice
//! by, an array of bytes to read, `format_args!`'s parts, and `?`'s `ControlFlow`.

use rustc_abi::{FieldIdx, VariantIdx};
use rustc_middle::mir::{Operand, Place, ProjectionElem};
use rustc_middle::ty::{self, GenericArgsRef, Instance, Ty, TyCtxt};
use rustc_span::{Span, Spanned};

use super::{Fx, Slot};
use crate::lower::fmt::{self, Piece};
use crate::lower::lend::{self, Plan};
use crate::lower::library::{self, Flow, Std};
use crate::lower::num::Int;
use crate::ts::{Expr, Stmt, StmtKind};

/// What a `&mut` borrows: its path, and the place a store through the reference writes,
/// unless the reference is refused.
type Borrowed<'tcx> = (Expr, Option<Place<'tcx>>);

impl<'m, 'b, 'a, 'tcx> Fx<'m, 'b, 'a, 'tcx> {
	pub(super) fn call(
		&mut self,
		func: &Operand<'tcx>,
		args: &[Spanned<Operand<'tcx>>],
		dest: Place<'tcx>,
		span: Span,
		out: &mut Vec<Stmt>,
	) {
		let tcx = self.tcx;
		if let Some(local) = dest.as_local()
			&& self.scalars.contains_key(&local)
		{
			self.scalar_call(local, func, args, span, out);
			return;
		}
		let fn_ty = func.ty(&self.mir.local_decls, tcx);
		let result = dest.ty(&self.mir.local_decls, tcx).ty;
		let mut pre = Vec::new();
		let (value, effect) = match *fn_ty.kind() {
			// A binding's export, which takes what JS passes, as std's mappings do not.
			ty::FnDef(def_id, gargs) if self.b.t.binds(def_id, gargs.skip_binder()) => (
				self.call_def(def_id, gargs.skip_binder(), args, result, span, &mut pre),
				true,
			),
			ty::FnDef(def_id, gargs) if matches!(lend::plan(self.b.t, def_id, gargs.skip_binder()), Plan::Refused) => {
				let path = tcx.def_path_str(def_id);
				let msg = format!(
					"`{path}` of a reference to an array of bytes it is generic over, which its mapping would take as the array, not as a slice of it"
				);
				(self.err(span, msg), true)
			}
			// `next` into a local that holds the protocol's item or `rt.DONE`, unconverted.
			ty::FnDef(def_id, gargs)
				if crate::lower::iter::is_next(tcx, def_id)
					&& dest.as_local().is_some_and(|l| self.next_results.contains(&l)) =>
			{
				let self_ty = gargs.skip_binder().type_at(0);
				let (iter_ty, recv) = match args[0].node.place() {
					// Through std's `Iterator` for `&mut I` and `Box<I>`: what the last `&mut`
					// points at, read through the `rt.Place` of each `&mut` to a `&mut` before it.
					Some(p) if self_ty.is_ref() || crate::lower::iter::through_mut(self_ty) => {
						let value = self.place_expr(tcx.mk_place_deref(p), span, &mut pre);
						let (last, value) = crate::lower::iter::peel_mut(self_ty, value);
						(last.builtin_deref(true).unwrap_or(last), value)
					}
					_ => (self_ty, self.operand_frozen(&args[0].node, span, &mut pre)),
				};
				if self.range_nexts.contains(&dest.local)
					&& let Some(name) = self.var(dest.local)
				{
					let name = name.to_string();
					self.assigned[dest.local] = true;
					let stepped = match &args[0].node {
						Operand::Copy(p) | Operand::Move(p) => {
							p.as_local().is_some_and(|b| self.stepped_borrows.contains_key(&b))
						}
						Operand::Constant(_) | Operand::RuntimeChecks(_) => false,
					};
					let read = self.uses[dest.local] > 1;
					let held = self.range_next(name, recv, stepped, read, &mut pre);
					self.emit(pre, false, out);
					if let Some((flag, more)) = held {
						self.hold_flag(flag, more);
					}
					return;
				}
				if let Some(name) = self.var(dest.local) {
					let name = name.to_string();
					self.next_names.insert(name);
				}
				if self.stepped_array(&args[0].node) {
					(self.stepped_next(&recv), true)
				} else {
					(crate::lower::iter::next(self.b, iter_ty, recv), true)
				}
			}
			// What std lends of the `[u8; N]`s it holds, its mapping makes slices of, and what
			// it is given, views.
			ty::FnDef(def_id, gargs) if matches!(lend::plan(self.b.t, def_id, gargs.skip_binder()), Plan::Lend(_)) => (
				self.call_def(def_id, gargs.skip_binder(), args, result, span, &mut pre),
				true,
			),
			ty::FnDef(def_id, gargs) => match self.intercept(def_id, gargs.skip_binder(), args, dest, span, &mut pre) {
				Some(Some(done)) => done,
				// Done: the destination is set.
				Some(None) => {
					self.emit(pre, false, out);
					return;
				}
				None => (
					self.call_def(def_id, gargs.skip_binder(), args, result, span, &mut pre),
					true,
				),
			},
			ty::FnPtr(..) => {
				let f = self.operand(func, span, &mut pre);
				let values = self.operands(false, &[], args, result, span, &mut pre);
				(f.call(values), true)
			}
			_ => (self.err(span, format!("a call through `{fn_ty}`")), true),
		};
		let value = match dest.as_local() {
			Some(local) if self.stepped.contains(&local) => self.stepped_make(value, span),
			_ => value,
		};
		let value = match self.b.t.loc(span) {
			Some(loc) if effect => Expr::Located(loc, Box::new(value)),
			_ => value,
		};
		if !pre.is_empty() {
			self.emit(pre, false, out);
		}
		// What an `Iterator::next` returns, by the protocol, unless a `next` made it already.
		let value = match dest.as_local() {
			Some(rustc_middle::mir::RETURN_PLACE)
				if self.next && !matches!(*fn_ty.kind(), ty::FnDef(d, _) if crate::lower::iter::is_next(tcx, d)) =>
			{
				self.done_of(value)
			}
			_ => value,
		};
		// A result into a local changed in place is its own copy: it may share the callee's.
		let value = match dest.as_local() {
			Some(local) if self.mutated[local] => {
				let ty = self.mir.local_decls[local].ty;
				self.b.t.copied(value, ty, span)
			}
			_ => value,
		};
		match dest.as_local() {
			Some(local) => {
				let memory = effect || args.iter().any(|a| super::rvalue::operand_memory(&a.node, self));
				self.set_local(local, value, effect, memory, None, out)
			}
			None => {
				let stmts = self.store(dest, value, span);
				self.emit(stmts, false, out);
			}
		}
	}

	/// Arguments, uncopied, since a callee that changes a parameter in place copies it
	/// first, but for a `Copy` aggregate std `keeps`, copied unless moved out of a local's
	/// own value, which the analysis then made the local's own. Each argument `pointees`
	/// marks is what an `rt.Place` it is reads, as std's mappings take a `&mut` parameter.
	fn operands(
		&mut self,
		translated: bool,
		pointees: &[bool],
		args: &[Spanned<Operand<'tcx>>],
		result: Ty<'tcx>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Vec<Expr> {
		let saved = std::mem::replace(&mut self.into_frozen, true);
		let mut values = Vec::with_capacity(args.len());
		for (i, arg) in args.iter().enumerate() {
			if pointees.get(i) == Some(&true)
				&& let Some(p) = arg.node.place()
				&& self
					.b
					.t
					.place_pointee(p.ty(&self.mir.local_decls, self.tcx).ty)
					.is_some()
			{
				let deref = self.tcx.mk_place_deref(p);
				values.push(self.place_expr(deref, span, out));
				continue;
			}
			let value = self.operand_frozen(&arg.node, span, out);
			values.push(match &arg.node {
				// Its type, rather than its variant's, for std's runtime function to infer
				// its type parameters from, as `widen` gives a local's.
				Operand::Constant(c) if !translated && self.b.t.narrows(c.ty()) => {
					let ty = self.b.t.ty(c.ty(), span);
					super::whole(value, ty).unwrap_or_else(|value| value)
				}
				Operand::Copy(p) | Operand::Move(p)
					if self.moved_from(&arg.node).is_none()
						&& self.keeps(translated, args, p.ty(&self.mir.local_decls, self.tcx).ty, result) =>
				{
					self.b.t.copied(value, p.ty(&self.mir.local_decls, self.tcx).ty, span)
				}
				_ => value,
			});
		}
		self.into_frozen = saved;
		values
	}

	/// An argument, which the callee copies if it changes it in place.
	fn operand_frozen(&mut self, op: &Operand<'tcx>, span: Span, out: &mut Vec<Stmt>) -> Expr {
		match op {
			Operand::Copy(place) | Operand::Move(place) if !self.in_bytes(*place) => self.place_expr(*place, span, out),
			_ => self.operand(op, span, out),
		}
	}

	/// A call of `def_id` with `gargs`: the crate's own function, a closure, or std's.
	fn call_def(
		&mut self,
		def_id: rustc_hir::def_id::DefId,
		gargs: GenericArgsRef<'tcx>,
		args: &[Spanned<Operand<'tcx>>],
		result: Ty<'tcx>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Expr {
		let tcx = self.tcx;
		// Calling a closure or function value: `FnOnce::call_once(f, (args,))`.
		if let Some(trait_id) = tcx.trait_of_assoc(def_id)
			&& tcx.is_fn_trait(trait_id)
			&& let [callee, tupled] = args
		{
			let callee_ty = gargs.type_at(0);
			let values = match self.take_parts(&tupled.node) {
				Some(parts) => parts,
				None => {
					let tuple = self.operand_frozen(&tupled.node, span, out);
					let tuple = self.path(tuple, "args", out);
					let ty::Tuple(tys) = gargs.type_at(1).kind() else {
						return self.err(span, "a closure call whose arguments are not a tuple");
					};
					(0..tys.len())
						.map(|i| tuple.clone().index(Expr::lit(i.to_string())))
						.collect()
				}
			};
			if let ty::FnDef(inner, inner_args) = *callee_ty.peel_refs().kind() {
				return self.b.call_def(inner, inner_args.skip_binder(), values, span, out);
			}
			// A `&mut` to a function JS copies, such as a box of one, or to a reference to one, as
			// std's impls for a `&mut` and a box forward through, is its place: call what is
			// there. A box is what it holds.
			let mut through = callee.node.ty(&self.mir.local_decls, tcx);
			let mut f = match callee.node.place() {
				Some(p) if let Some(pointee) = self.b.t.place_pointee(through) => {
					through = pointee;
					self.place_expr(tcx.mk_place_deref(p), span, out)
				}
				_ => self.operand(&callee.node, span, out),
			};
			loop {
				through = match (through.boxed_ty(), self.b.t.place_pointee(through)) {
					(Some(inner), _) => inner,
					(None, Some(inner)) => {
						f = f.member("v");
						inner
					}
					(None, None) => break,
				};
			}
			// `FnOnce::call_once` of a closure that `Fn` or `FnMut` call, or of a box of one,
			// drops it after, as rustc's shim does, where an `FnOnce` closure's body drops
			// what it owns.
			if tcx.fn_trait_kind_from_def_id(trait_id) == Some(ty::ClosureKind::FnOnce)
				&& self
					.b
					.t
					.owned_fn(callee_ty)
					.is_some_and(|kind| kind != ty::ClosureKind::FnOnce)
				&& self.b.t.drops(callee_ty)
			{
				return Expr::rt("callOnce", [vec![f], values].concat());
			}
			return f.call(values);
		}
		let local = self.b.t.translated(def_id, gargs);
		let pointees = match self.b.t.mapped(def_id, gargs) {
			false => Vec::new(),
			true => {
				if let Some(err) = self.std_places(def_id, gargs, result, span) {
					return err;
				}
				if crate::lower::pointer::from_reference(tcx, def_id, gargs)
					&& let Some(why) = self.made_out_of_step(def_id, gargs, &args[0].node)
				{
					return self.err(span, why);
				}
				crate::lower::pointer::pointees(tcx, def_id)
			}
		};
		let mut values = self.operands(local, &pointees, args, result, span, out);
		// The slices of `[u8; N]`s std is given as a `&T`, as views of their bytes.
		if let Plan::Lend(lent) = lend::plan(self.b.t, def_id, gargs) {
			for &(i, (generic, ty)) in &lent.given {
				let value = std::mem::replace(&mut values[i], Expr::undefined());
				values[i] = lend::give(self.b, generic, ty, value, span);
			}
		}
		self.b.call_def(def_id, gargs, values, span, out)
	}

	/// A refusal of std's `def_id` if it would make a `&mut` to a value JS copies, which
	/// its mapping has as the value, not an `rt.Place`: one it returns or its iterator
	/// yields, other than a map's mapping, or one it hands a closure. A `&mut` it returns
	/// as a type parameter, as `Option::unwrap` does, is what it was given.
	fn std_places(
		&mut self,
		def_id: rustc_hir::def_id::DefId,
		gargs: GenericArgsRef<'tcx>,
		result: Ty<'tcx>,
		span: Span,
	) -> Option<Expr> {
		let tcx = self.tcx;
		let call = library::Call {
			def_id,
			args: gargs,
			span,
		};
		if crate::lower::collections::place_call(self.b, &call) || crate::lower::pointer::lends(tcx, def_id) {
			return None;
		}
		let t = &*self.b.t;
		// Mappings that lend an element JS copies as its place, as a map's `get_mut` does, the
		// item a `Peekable` peeked, the place a `&mut dyn Any` wraps, a `RefCell` key's value,
		// or an `Arc`'s.
		let lends = match library::key(tcx, def_id) {
			Some((owner, name)) => match (owner.as_str(), name.as_str()) {
				("slice", "get_mut" | "first_mut" | "last_mut" | "iter_mut" | "split_first_mut" | "split_last_mut")
				| ("array", "each_mut")
				| ("dyn Any", "downcast_mut")
				| ("Peekable", "peek_mut")
				| ("LocalKey", "with_borrow_mut")
				| ("Arc" | "Rc", "get_mut" | "make_mut") => true,
				("IntoIterator", "into_iter") => gargs.types().next().is_some_and(|ty| {
					let ty = crate::lower::iter::unbox_ref(tcx, ty);
					ty.ref_mutability() == Some(ty::Mutability::Mut)
						&& match ty.peel_refs().kind() {
							ty::Slice(_) | ty::Array(..) => true,
							_ => matches!(
								t.std_of(ty.peel_refs()),
								Some(Std::Vec | Std::Option | Std::HashMap | Std::BTreeMap | Std::VecDeque)
							),
						}
				}),
				_ => false,
			},
			None => false,
		};
		// Places held in what it is given, not one it is given, or that the iterator it adapts
		// yields, were made where they came from, as are what it hands a closure of them.
		let adapts = t.sig(def_id, gargs).inputs().iter().any(|&i| {
			t.place_pointee(i).is_none() && t.has_place(i) || t.iter_item(i).is_some_and(|item| t.has_place(item))
		});
		if lends || adapts {
			return None;
		}
		let handed = gargs.types().any(|ty| {
			let sig = match ty.kind() {
				ty::Closure(_, args) => args.as_closure().sig(),
				ty::FnDef(..) | ty::FnPtr(..) => ty.fn_sig(tcx),
				_ => return false,
			};
			tcx.instantiate_bound_regions_with_erased(sig)
				.inputs()
				.iter()
				.any(|&i| t.has_place(i))
		});
		let yielded = t.iter_item(result).is_some_and(|item| t.has_place(item));
		let made = t.has_place(result) && {
			let resolved = Instance::try_resolve(tcx, t.typing_env_pub(), def_id, gargs)
				.ok()
				.flatten()
				.map_or(def_id, |instance| instance.def_id());
			let sig = tcx.fn_sig(resolved).instantiate_identity().skip_norm_wip();
			let output = tcx.instantiate_bound_regions_with_erased(sig).output();
			output
				.walk()
				.filter_map(|a| a.as_type())
				.any(|ty| matches!(ty.kind(), ty::Ref(_, _, rustc_ast::Mutability::Mut)))
		};
		if !(handed || yielded || made) {
			return None;
		}
		let path = tcx.def_path_str(def_id);
		Some(self.err(
			span,
			format!("`{path}` with a mutable reference to a value JS copies, which its mapping would copy"),
		))
	}

	/// What `Default::default` makes of `ty`: a std type's value, or the call of the crate's
	/// impl, made here if `held`.
	fn default_of(&mut self, ty: Ty<'tcx>, held: bool, span: Span, out: &mut Vec<Stmt>) -> Expr {
		if let Some(default) = library::default_value(self.b, ty, span, out) {
			return default;
		}
		let tcx = self.tcx;
		let f = crate::lower::collections::trait_fn(tcx, "Default", "default");
		let default = self.b.call_def(f, tcx.mk_args(&[ty.into()]), Vec::new(), span, out);
		match held {
			true => self.path(default, "default", out),
			false => default,
		}
	}

	/// A std call MIR needs lowered here: `Some(Some((value, effect)))`, or `Some(None)`
	/// once it set the destination itself, or `None` for the library mappings.
	fn intercept(
		&mut self,
		def_id: rustc_hir::def_id::DefId,
		gargs: GenericArgsRef<'tcx>,
		args: &[Spanned<Operand<'tcx>>],
		dest: Place<'tcx>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Option<Option<(Expr, bool)>> {
		let tcx = self.tcx;
		let bytes = library::inherent_std(tcx, def_id) == Some(Std::Bytes);
		if library::std_path(tcx, def_id).is_empty() && !bytes {
			return None;
		}
		// A box's own `Drop` frees its memory, once drop elaboration dropped or moved out
		// what it holds: nothing JS sees.
		let drop_trait = tcx
			.trait_of_assoc(def_id)
			.or_else(|| tcx.trait_impl_of_assoc(def_id).map(|imp| tcx.impl_trait_id(imp)));
		if drop_trait.is_some_and(|tr| tcx.is_lang_item(tr, rustc_attr_ir::LangItem::Drop))
			&& self
				.b
				.t
				.self_ty(Instance::new_raw(def_id, gargs))
				.is_some_and(|ty| self.b.t.std_of(ty) == Some(Std::Box))
		{
			return Some(Some((Expr::undefined(), false)));
		}
		let (owner, name) = library::key(tcx, def_id)?;
		let targ = |n: usize| gargs.types().nth(n);
		let done = |e: Expr, effect: bool| Some(Some((e, effect)));
		match (owner.as_str(), name.as_str()) {
			// A place of no bytes has nothing to read or write, and every value of its type
			// is the one it holds. What made the `&mut` still runs, as its unread temporary.
			("mem", "take" | "replace" | "swap") if self.b.t.zero_sized(targ(0)?) => match name.as_str() {
				"take" => done(self.default_of(targ(0)?, false, span, out), true),
				"replace" => done(self.operand(&args[1].node, span, out), false),
				_ => done(Expr::undefined(), false),
			},
			// Replacing what a `&mut` borrows.
			// What `Default::default` makes before the swap.
			("mem", "take") => {
				let ty = targ(0)?;
				let place = self.borrowed_place(&args[0].node, span, out)?;
				let default = self.default_of(ty, true, span, out);
				let base = self.read_as(dest, "taken");
				done(self.swap_out(place, ty, default, base, span, out), base.is_some())
			}
			("mem", "replace") => {
				let place = self.borrowed_place(&args[0].node, span, out)?;
				let value = self.operand(&args[1].node, span, out);
				let base = self.read_as(dest, "old");
				done(self.swap_out(place, targ(0)?, value, base, span, out), base.is_some())
			}
			// `mem::replace(self, Some(value))`, and `Some(value)` is `value`.
			("Option", "replace") => {
				let ty = args[0].node.ty(&self.mir.local_decls, tcx).builtin_deref(true)?;
				let place = self.borrowed_place(&args[0].node, span, out)?;
				let value = self.operand(&args[1].node, span, out);
				let base = self.read_as(dest, "old");
				done(self.swap_out(place, ty, value, base, span, out), base.is_some())
			}
			("mem", "swap") => {
				let a = self.borrowed_place(&args[0].node, span, out)?;
				let c = self.borrowed_place(&args[1].node, span, out)?;
				// An object written in place gives a copy of what it held, so it goes first:
				// a place given the object itself would share what then changes in it.
				let (a, c) = match c.1.is_some_and(|p| self.in_object(p)) {
					true => (c, a),
					false => (a, c),
				};
				let old = self.swap_out(a, targ(0)?, c.0.clone(), Some("swapped"), span, out);
				self.set_borrowed(&c, old, span, out);
				done(Expr::undefined(), false)
			}
			// A range of numbers steps in place, with nothing between it and the loop.
			("Iterator", "next") if super::small_range(tcx, targ(0)?) => {
				let range = self.alias_place(&args[0].node)?;
				let start = range.clone().member("start");
				let next = Expr::Cond(
					Box::new(start.clone().binary("<", range.member("end"))),
					Box::new(Expr::Postfix(Box::new(start), "++")),
					Box::new(Expr::undefined()),
				);
				done(next, true)
			}
			// A JS string can't change in place, so what the `&mut` borrows gets a new one.
			("String", "push" | "push_str") | ("AddAssign", "add_assign")
				if owner == "String" || targ(0).is_some_and(|t| self.b.t.std_of(t) == Some(Std::String)) =>
			{
				let place = self.borrowed_place(&args[0].node, span, out)?;
				let text = self.operand(&args[1].node, span, out);
				let text = match name.as_str() {
					"push" => library::char_text(text),
					_ => text,
				};
				let pushed = place.0.clone().binary("+", text);
				self.set_borrowed(&place, pushed, span, out);
				done(Expr::undefined(), false)
			}
			// What the `&mut` borrows gets the value of the other case, as a byte, a char, and a
			// JS string can't change in place.
			("u8" | "char" | "str", "make_ascii_uppercase" | "make_ascii_lowercase") => {
				let place = self.borrowed_place(&args[0].node, span, out)?;
				let to = crate::lower::camel(&name.replacen("make", "to", 1));
				let to = match owner.as_str() {
					"str" => Expr::rt_of("str", &to),
					_ => Expr::Rt(to),
				};
				let changed = to.call(vec![place.0.clone()]);
				self.set_borrowed(&place, changed, span, out);
				done(Expr::undefined(), false)
			}
			// The extended string, from the iterator read as std's `Extend` reads it.
			("Extend", "extend") if targ(0).is_some_and(|t| self.b.t.std_of(t) == Some(Std::String)) => {
				let (item, source) = (targ(1)?, targ(2)?);
				let place = self.borrowed_place(&args[0].node, span, out)?;
				let items = self.operand(&args[1].node, span, out);
				let iter = crate::lower::iter::iter_of(self.b.t, source)?;
				let items = crate::lower::iter::into_iter(self.b, source, items, span)?;
				let extended = crate::lower::iter::string_extend(self.b, place.0.clone(), items, item, iter, span)?;
				self.set_borrowed(&place, extended, span, out);
				done(Expr::undefined(), false)
			}
			// No capacity is kept, and reading one is refused, but reserving past `isize::MAX`
			// panics as std's does.
			("String", "reserve" | "reserve_exact") => {
				let values = self.operands(false, &[true, true], args, tcx.types.unit, span, out);
				done(Expr::rt("stringReserve", values), true)
			}
			("String", "shrink_to_fit" | "shrink_to") => done(Expr::undefined(), false),
			("String", "retain") if targ(0).is_some_and(|f| self.b.t.drops(f)) => {
				let f = targ(0)?;
				done(
					self.err(
						span,
						format!("passing `{f}`, which drops, to `String::{name}`, which is not supported yet"),
					),
					false,
				)
			}
			// The edited string, from the runtime with what it takes after the string.
			(
				"String",
				"clear" | "truncate" | "insert" | "insert_str" | "retain" | "replace_range" | "extend_from_within",
			) => {
				let place = self.borrowed_place(&args[0].node, span, out)?;
				let result = dest.ty(&self.mir.local_decls, tcx).ty;
				let mut values = vec![place.0.clone()];
				values.extend(self.operands(false, &[], &args[1..], result, span, out));
				if name == "insert" {
					let c = values.pop()?;
					values.push(library::char_text(c));
				}
				if name == "retain" {
					let f = std::mem::replace(&mut values[1], Expr::undefined());
					values[1] = library::called_through(self.b.t, targ(0)?, Some(f)).2?;
				}
				// A range std reads the bounds of, as `bounds` makes any `RangeBounds` one.
				if matches!(name.as_str(), "replace_range" | "extend_from_within") {
					let call = library::Call {
						def_id,
						args: gargs,
						span,
					};
					let range = std::mem::replace(&mut values[1], Expr::undefined());
					let len = Expr::rt("stringLen", vec![values[0].clone()]);
					values[1] = crate::lower::collections::bounds(self.b, &call, range).within(len);
				}
				let edited = match name.as_str() {
					"clear" => Expr::Str(String::new()),
					"truncate" => Expr::rt("stringTruncate", values),
					"insert" | "insert_str" => Expr::rt("stringInsertStr", values),
					"retain" => Expr::rt("stringRetain", values),
					"replace_range" => Expr::rt("stringReplaceRange", values),
					_ => Expr::rt("stringExtendFromWithin", values),
				};
				self.set_borrowed(&place, edited, span, out);
				done(Expr::undefined(), false)
			}
			// The runtime gives the edited string and what the edit returns.
			("String", "pop" | "remove" | "split_off") => {
				let place = self.borrowed_place(&args[0].node, span, out)?;
				let result = dest.ty(&self.mir.local_decls, tcx).ty;
				let mut values = vec![place.0.clone()];
				values.extend(self.operands(false, &[], &args[1..], result, span, out));
				let f = match name.as_str() {
					"pop" => "stringPop",
					"remove" => "stringRemove",
					_ => "stringSplitOff",
				};
				let edit = self.path(Expr::rt(f, values), "edit", out);
				self.set_borrowed(&place, edit.clone().index(Expr::lit("0")), span, out);
				done(edit.index(Expr::lit("1")), false)
			}
			("Write", "write_str" | "write_char" | "write_fmt")
				if tcx
					.trait_of_assoc(def_id)
					.is_some_and(|tr| tcx.is_diagnostic_item(rustc_span::Symbol::intern("FmtWrite"), tr))
					&& !self.b.t.translated(def_id, gargs) =>
			{
				self.fmt_write(def_id, gargs, args, dest, span, out)
			}
			// A `Duration` never changes once made, so what the `&mut` borrows gets a new one.
			// std's impls take a `Duration` or a `u32`: another is the crate's.
			("AddAssign" | "SubAssign" | "MulAssign" | "DivAssign", _)
				if targ(0).is_some_and(|t| self.b.t.std_of(t) == Some(Std::Duration))
					&& targ(1).is_some_and(|t| match owner.as_str() {
						"AddAssign" | "SubAssign" => self.b.t.std_of(t) == Some(Std::Duration),
						_ => t == tcx.types.u32,
					}) =>
			{
				let f = match owner.as_str() {
					"AddAssign" => "durationAdd",
					"SubAssign" => "durationSub",
					"MulAssign" => "durationMul",
					_ => "durationDiv",
				};
				let place = self.borrowed_place(&args[0].node, span, out)?;
				let by = self.operand(&args[1].node, span, out);
				let value = Expr::rt(f, vec![place.0.clone(), by]);
				self.set_borrowed(&place, value, span, out);
				done(Expr::undefined(), false)
			}
			// A `Cow<str>` is the `String` it would hold, so the `&mut` borrows the same place.
			("Cow", "to_mut") if targ(0).is_some_and(|t| t.is_str()) => {
				let (place, borrowed) = self.borrowed_place(&args[0].node, span, out)?;
				if let Some(local) = dest.as_local()
					&& self.defs[local] == 1
					&& matches!(self.slots[local], Slot::Var(_))
				{
					self.slots[local] = Slot::Alias(Some(place));
					self.reborrows.extend(borrowed.map(|b| (local, b)));
					return Some(None);
				}
				match borrowed {
					Some(borrowed) => {
						let target = self.stable_target(borrowed, place, span, out);
						done(self.place_of(target, None), true)
					}
					None => done(place, true),
				}
			}
			// A `Wrapping` or a `Saturating` is its integer, so what the `&mut` borrows gets the
			// wrapped or saturated result.
			(_, _)
				if let Some(op) = owner.strip_suffix("Assign").and_then(library::operator)
					&& let Some((overflow, int)) = targ(0).and_then(|t| library::overflowing(tcx, t)) =>
			{
				let place = self.borrowed_place(&args[0].node, span, out)?;
				let by = self.operand(&args[1].node, span, out);
				let l = crate::lower::num::Operand {
					expr: place.0.clone(),
					value: None,
				};
				let r = crate::lower::num::Operand {
					expr: by,
					value: self.const_int(&args[1].node),
				};
				let value = crate::lower::num::overflowing(self.b, overflow, op, l, r, int, span);
				self.set_borrowed(&place, value, span, out);
				done(Expr::undefined(), false)
			}
			// A `Cow<[T]>` that borrows takes a clone of its slice, whose `Vec` the `&mut` lends.
			("Cow", "to_mut") if let ty::Slice(elem) = targ(0)?.kind() => {
				let place = self.borrowed_place(&args[0].node, span, out)?;
				// Read twice, by the clone and by the store.
				self.materialize(&place.0, out);
				let owned = library::cow_cloned(self.b, "IntoOwned", place.0.clone(), *elem, span);
				let owned = self.path(owned, "owned", out);
				self.set_borrowed(&place, owned.clone(), span, out);
				done(owned, true)
			}
			("Option", "take") => {
				let place = self.borrowed_place(&args[0].node, span, out)?;
				let none = self.b.t.none(targ(0)?);
				let option = dest.ty(&self.mir.local_decls, tcx).ty;
				let base = self.read_as(dest, "taken");
				done(self.swap_out(place, option, none, base, span, out), base.is_some())
			}
			// The place of the payload, if any: `as_mut`'s, or what `iter_mut` yields.
			("Option", "as_mut" | "iter_mut") if self.b.t.needs_place(targ(0)?) => {
				let (target, payload) = self.payload_place(&args[0].node, targ(0)?, span, out)?;
				let Some(payload) = payload else {
					return done(target, true);
				};
				let none = target.clone().binary("===", self.b.t.none(targ(0)?));
				let place = self.make_place(payload, target, span, out);
				let lent = Expr::Cond(Box::new(none), Box::new(Expr::undefined()), Box::new(place));
				match name.as_str() {
					"as_mut" => done(lent, true),
					_ => done(Expr::rt("iterOption", vec![lent]), true),
				}
			}
			("Option", "get_or_insert" | "get_or_insert_with" | "get_or_insert_default" | "insert") => {
				self.option_insert(&name, targ(0)?, args, dest, span, out)
			}
			// Indexing, whose result a `&mut` reads in place.
			("Index" | "IndexMut", "index" | "index_mut") => self.index(&name, gargs, args, dest, span, out),
			// Out of bounds is undefined, so the checked index's panic is one outcome of it; a
			// part by a range is checked as indexing by it is, giving `None` where that panics.
			("slice", "get_unchecked" | "get_unchecked_mut" | "get" | "get_mut")
				if name.starts_with("get_unchecked") || Int::of(targ(1)?).is_none() =>
			{
				let slice = ty::Ty::new_slice(tcx, targ(0)?);
				let gargs = tcx.mk_args(&[slice.into(), targ(1)?.into()]);
				let name = match name.as_str() {
					"get_unchecked" => "index",
					"get_unchecked_mut" => "index_mut",
					name => name,
				};
				self.index(name, gargs, args, dest, span, out)
			}
			// A `&mut` to a value JS copies, such as through a `MutexGuard`, reads and writes
			// the place it borrows, as an index does, and so does one to any value a guard or
			// a `RefMut` holds, which is where its mutex or cell keeps it, or that an
			// `rt.Place` lends, as one to a pinned box of an `Arc` does.
			("DerefMut", "deref_mut")
				if let ty::Ref(_, pointee, _) = dest.ty(&self.mir.local_decls, tcx).ty.kind()
					&& (self.b.t.object_kind(*pointee).is_none()
						|| targ(0).is_some_and(|t| {
							matches!(self.b.t.std_of(t), Some(Std::MutexGuard | Std::RefMut))
								|| self.b.t.needs_place(t) && self.b.t.needs_place(*pointee)
						}))
					&& let Some(local) = dest.as_local()
					&& self.defs[local] == 1
					&& matches!(self.slots[local], Slot::Var(_)) =>
			{
				let result = dest.ty(&self.mir.local_decls, tcx).ty;
				// The receiver passes as what it points at, as std's `&mut self` does.
				let values = self.operands(false, &[true], args, result, span, out);
				let place = self.b.call_def(def_id, gargs, values, span, out);
				// A `&mut`, or a pin of one, lends the `rt.Place` it is, whose value is at `v`.
				let lent = targ(0).map(|t| match t.kind() {
					ty::Adt(adt, args) if Std::of(tcx, adt.did()) == Some(Std::Pin) => args.type_at(0),
					_ => t,
				});
				let place = match lent {
					Some(lent) if self.b.t.place_pointee(lent).is_some() => place.member("v"),
					_ => place,
				};
				if !place.is_path() {
					if self.b.t.object_kind(*pointee).is_some() && !self.b.t.needs_place(*pointee) {
						return done(place, true);
					}
					let msg = format!("`{}` of a value JS copies, which is no path", tcx.def_path_str(def_id));
					return done(self.err(span, msg), true);
				}
				self.materialize(&place, out);
				self.slots[local] = Slot::Alias(Some(place));
				Some(None)
			}
			// A `Bytes`'s slice by a range's bounds, read where the range is made. Its bounds
			// check panics, so it runs even if the slice goes unused.
			("Bytes", "slice")
				if bytes && let Some((start, end)) = self.range_bounds(&args[1].node, targ(0)?, span, out) =>
			{
				let b = self.operand(&args[0].node, span, out);
				done(
					Expr::rt("bytesSlice", std::iter::once(b).chain([start]).chain(end).collect()),
					true,
				)
			}
			("Bytes", "slice")
				if bytes
					&& let ty::Adt(adt, _) = targ(0)?.kind()
					&& Std::of(tcx, adt.did()) == Some(Std::RangeToInclusive) =>
			{
				let end = self.range_fields(&args[1].node, *adt, span, out).pop()?;
				let b = self.operand(&args[0].node, span, out);
				done(Expr::rt("bytesSliceToInclusive", vec![b, end]), true)
			}
			// A range's bounds taken apart where it is made, or any other's checked as
			// `slice::range` checks them. A `Copy` aggregate is copied, which JS's
			// `copyWithin` would share.
			("slice", "copy_within") => {
				let elem = targ(0)?;
				let s = self.operand(&args[0].node, span, out);
				let s = self.path(s, "slice", out);
				let (start, end) = match self.range_bounds(&args[1].node, targ(1)?, span, out) {
					Some((start, end)) => (start, end.unwrap_or_else(|| s.clone().member("length"))),
					None => {
						let range = self.operand(&args[1].node, span, out);
						let call = library::Call {
							def_id,
							args: gargs,
							span,
						};
						let ranged = crate::lower::collections::bounds(self.b, &call, range);
						let checked = Expr::rt("boundsRange", vec![ranged.bounds.clone(), s.clone().member("length")]);
						let checked = ranged.after(checked);
						let pair = self.path(checked, "bounds", out);
						(pair.clone().index(Expr::lit("0")), pair.index(Expr::lit("1")))
					}
				};
				let dest = self.operand(&args[2].node, span, out);
				let mut values = vec![s, start, end, dest];
				let f = match elem.kind() {
					ty::Uint(ty::UintTy::U8) => "byteSliceCopyWithin",
					_ => {
						values.extend(self.b.t.copy_fn(elem, span));
						"sliceCopyWithin"
					}
				};
				done(Expr::rt(f, values), true)
			}
			// An integer from an array of bytes made in place: its bytes shifted, no array made.
			(int, "from_be_bytes") if let Some(int) = int_named(int) => {
				let parts = match int.big() {
					true => None,
					// Bytes through a reference are read where they are.
					false if let Some(slice) = self.bytes_source(&args[0].node, span, out) => {
						let n = u64::from(int.bits / 8);
						Some(self.bytes_at(slice, n, out))
					}
					false => self.take_parts(&args[0].node),
				};
				let value = match parts {
					Some(parts) => library::be_word(int, parts),
					None => {
						let bytes = self.operand_frozen(&args[0].node, span, out);
						int.method("fromBeBytes", vec![bytes])
					}
				};
				done(value, false)
			}
			// `format_args!`, as MIR builds it: each argument a function writing its value,
			// and a template of literal pieces and placeholders.
			("Argument", name) if name.starts_with("new_") || name == "from_usize" => {
				let tr = match fmt::argument(name) {
					Ok(Some(tr)) => tr,
					// A width or precision, which the template's spec reads.
					Ok(None) => {
						let count = self.operand(&args[0].node, span, out);
						return done(Expr::rt("fmtCount", vec![count]), true);
					}
					Err(construct) => {
						self.err(span, format!("{construct} is not supported"));
						return done(Expr::undefined(), false);
					}
				};
				// An array of bytes comes by reference, which is a slice of them.
				let ty = match targ(0)? {
					ty if crate::lower::byte_array(ty) => Ty::new_slice(tcx, tcx.types.u8),
					ty => ty,
				};
				let value = self.operand(&args[0].node, span, out);
				// The function reads it later, in a closure, where TypeScript no longer
				// narrows a member of a union: a member is read now, into a name.
				let value = match value {
					Expr::Member(..) | Expr::Index(..) => {
						let name = self.b.fresh("arg");
						let stmt = StmtKind::decl(false, name.clone(), None, Some(value)).at(None);
						self.emit(vec![stmt], false, out);
						Expr::ident(name)
					}
					value => self.path(value, "arg", out),
				};
				done(self.b.fmt_value(tr, ty, value, span, out), false)
			}
			("Arguments", "from_str" | "from_str_nonconst") => {
				let s = self.operand(&args[0].node, span, out);
				done(Expr::Array(vec![s]), false)
			}
			("Arguments", "new") => {
				let Some(template) = self.const_bytes(&args[0].node) else {
					self.err(
						span,
						format!("an unexpected `format_args!` lowering: {:?}", args[0].node),
					);
					return done(Expr::undefined(), false);
				};
				let values = self.operand(&args[1].node, span, out);
				// The arguments array made once in place: each piece its function.
				let made: Option<Vec<Expr>> = match &values {
					Expr::Ident(name) => self.local_named(name).and_then(|l| self.var_parts.get(&l).cloned()),
					_ => None,
				};
				let values = self.path(values, "args", out);
				let pieces = match fmt::decode(&template) {
					Ok(pieces) => pieces,
					Err(msg) => {
						self.err(span, format!("{msg} is not supported"));
						return done(Expr::undefined(), false);
					}
				};
				let mut parts = Vec::new();
				for piece in pieces {
					match piece {
						Piece::Lit(s) => parts.push(Expr::Str(s)),
						Piece::Arg { index, spec } => {
							let arg = |index: usize| match made.as_ref().and_then(|p| p.get(index)) {
								Some(part) => part.clone(),
								None => values.clone().index(Expr::lit(index.to_string())),
							};
							let value = arg(index);
							parts.push(match spec.object(arg) {
								Some(spec) => Expr::rt("fmtSpec", vec![spec, value]),
								None => value,
							});
						}
					}
				}
				done(Expr::Array(parts), false)
			}
			// `?`: the value it was applied to is the `ControlFlow` MIR branches on.
			// The crate's own iterator is its own `IntoIterator`; std's adaptors convert it
			// where they take it.
			("IntoIterator", "into_iter")
				if matches!(targ(0)?.kind(), ty::Adt(adt, _) if self.b.t.own(adt.did()))
					&& self.b.t.iter_item(targ(0)?).is_some() =>
			{
				let value = self.operand(&args[0].node, span, out);
				done(value, false)
			}
			("Try", "branch") => {
				let value = self.operand(&args[0].node, span, out);
				// A `Poll`'s `Pending` and `Ready(None)` are no `rt.Result`: they continue.
				if let Some(poll) = targ(0)
					&& let (Some(Std::Poll), ty::Adt(_, poll_args)) = (self.b.t.std_of(poll), poll.kind())
				{
					let inner = poll_args.type_at(0);
					let ty::Adt(_, inner_args) = inner.kind() else {
						unreachable!("Try for a Poll of a Result or an Option of one")
					};
					let value = match self.b.t.std_of(inner) {
						Some(Std::Option) => {
							let ty::Adt(_, result_args) = inner_args.type_at(0).kind() else {
								unreachable!("Try for a Poll of an Option of a Result")
							};
							let ok = result_args.type_at(0);
							let pending = self.b.t.pending(Ty::new_option(self.tcx, ok));
							Expr::rt("pollOptionBranch", vec![value, pending, self.b.t.none(ok)])
						}
						_ => Expr::rt("pollBranch", vec![value, self.b.t.pending(inner_args.type_at(0))]),
					};
					return done(value, false);
				}
				// A `next`'s item or `rt.DONE` branches as itself.
				if let Some(local) = dest.as_local()
					&& self.next_results.contains(&local)
					&& let Some(name) = self.var(local)
				{
					let name = name.to_string();
					self.next_names.insert(name);
				}
				done(value, false)
			}
			("FromResidual", "from_residual") => {
				let to = targ(0)?;
				let residual = targ(1)?;
				let value = self.operand(&args[0].node, span, out);
				// What a `Result` residual goes into: a `Result`, or one that is ready, and some,
				// which is the same value.
				let result = match (self.b.t.std_of(to), to.kind()) {
					(Some(Std::Result), _) => Some(to),
					(Some(Std::Poll), ty::Adt(_, poll_args)) => {
						let mut inner = poll_args.type_at(0);
						if let (Some(Std::Option), ty::Adt(_, option_args)) = (self.b.t.std_of(inner), inner.kind()) {
							inner = option_args.type_at(0);
						}
						(self.b.t.std_of(inner) == Some(Std::Result)).then_some(inner)
					}
					_ => None,
				};
				match (self.b.t.std_of(to), result.map(|r| r.kind()), residual.kind()) {
					(Some(Std::Option), ..) => {
						let ty::Adt(_, to_args) = to.kind() else {
							unreachable!("an `Option` is an ADT")
						};
						done(self.b.t.none(to_args.type_at(0)), false)
					}
					(_, Some(ty::Adt(_, to_args)), ty::Adt(_, from_args))
						if self.b.t.std_of(residual) == Some(Std::Result) =>
					{
						let (from, to) = (from_args.type_at(1), to_args.type_at(1));
						if from == to {
							return done(value, false);
						}
						let error = match value {
							// Made in place: its error.
							Expr::Call(f, mut items)
								if matches!(&*f, Expr::Rt(n) if n == "Err") && items.len() == 1 =>
							{
								items.pop().expect("checked above")
							}
							value => {
								let value = self.path(value, "residual", out);
								// The residual is an `Err`.
								let err = crate::ts::Type::Object(vec![
									("ok".into(), crate::ts::Type::Builtin("false")),
									("error".into(), self.b.t.ty(from, span)),
								]);
								Expr::As(Box::new(value), err).member("error")
							}
						};
						let converted = self.b.convert(error, from, to, span, out);
						done(Expr::result(false, converted), true)
					}
					_ => {
						self.err(span, format!("`?` from `{residual}` to `{to}`"));
						done(Expr::undefined(), false)
					}
				}
			}
			("DebugList", "entries") => {
				let list = self.operand(&args[0].node, span, out);
				let (item_ty, iter_ty) = (targ(0)?, targ(1)?);
				let items = self.operand(&args[1].node, span, out);
				let items = self.items(items, iter_ty, span)?;
				let debug = self.b.fmt_fn(fmt::Trait::Debug, item_ty, span);
				let mut values = vec![list, items, debug];
				values.extend(self.b.t.drop_fn(item_ty, span));
				done(Expr::rt("debugListEntries", values), true)
			}
			_ => None,
		}
	}

	/// The place a `&mut` operand borrows, as an assignable path, and through the
	/// reference, which a store resolves: an alias of a borrowed place, not a copy of a
	/// reference, which holds the object it points at, or an `rt.Place`, whose `v` it is,
	/// or else the object it points at, written in place. A `[u8; N]`'s is a copy of its
	/// bytes instead, which the store writes in place.
	fn borrowed_place(&mut self, op: &Operand<'tcx>, span: Span, out: &mut Vec<Stmt>) -> Option<Borrowed<'tcx>> {
		let (Operand::Copy(place) | Operand::Move(place)) = op else {
			return None;
		};
		if !place.projection.is_empty() {
			return None;
		}
		let deref = self.tcx.mk_place_deref(*place);
		if self.in_bytes(deref) {
			// Named before the copy, as the store names it.
			let slice = self.place_expr(deref, span, out);
			let slice = self.slice_path(slice, out);
			return Some((Expr::rt("byteSliceToArray", vec![slice]), Some(deref)));
		}
		match &self.slots[place.local] {
			Slot::Alias(Some(target)) if assignable(target) && self.alias_of_place(place.local) => {
				Some((target.clone(), Some(deref)))
			}
			_ if self.b.t.place_pointee(self.mir.local_decls[place.local].ty).is_some() => {
				Some((self.place_expr(deref, span, out), Some(deref)))
			}
			_ if self.in_object(deref) => {
				let target = self.place_expr(deref, span, out);
				Some((self.path(target, "object", out), Some(deref)))
			}
			_ => {
				let pointee = self.mir.local_decls[place.local].ty.peel_refs();
				let err = self.err(span, format!("replacing the value behind a reference to `{pointee}`"));
				Some((err, None))
			}
		}
	}

	/// A `fmt::Write` method of a `String`, a `Formatter`, or the crate's own writer, or of
	/// std's impl for a `&mut W`, which calls `W`'s. A provided method of the crate's impl
	/// writes through its `write_str`. `None` for any other writer.
	fn fmt_write(
		&mut self,
		def_id: rustc_hir::def_id::DefId,
		gargs: GenericArgsRef<'tcx>,
		args: &[Spanned<Operand<'tcx>>],
		dest: Place<'tcx>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Option<Option<(Expr, bool)>> {
		let tcx = self.tcx;
		let name = tcx.item_name(def_id);
		let name = name.as_str();
		let self_ty = gargs.type_at(0);
		// The writer, behind any `&mut`s.
		let mut w = self_ty;
		while let ty::Ref(_, inner, ty::Mutability::Mut) = *w.kind() {
			w = inner;
		}
		let wargs = tcx.mk_args(&[w.into()]);
		let write_str = crate::lower::collections::trait_fn(tcx, "FmtWrite", "write_str");
		let std = self.b.t.std_of(w);
		let own = self.b.t.translated(write_str, wargs);
		if !matches!(std, Some(Std::String | Std::Formatter)) && !own {
			return None;
		}
		let done = |e: Expr, effect: bool| Some(Some((e, effect)));
		let result = dest.ty(&self.mir.local_decls, tcx).ty;
		let literal = match name {
			"write_fmt" => self.literal_arguments(&args[1].node),
			_ => None,
		};
		// The writer's `&mut`, read through each `&mut` std's impl forwards through.
		let through = match args[0].node.place() {
			Some(p) if self_ty.is_ref() => {
				let value = self.place_expr(tcx.mk_place_deref(p), span, out);
				Some(crate::lower::iter::peel_mut(self_ty, value).1)
			}
			_ => None,
		};
		// A JS string can't change in place, so what the `&mut` borrows gets a longer one.
		if std == Some(Std::String) {
			let place = match through {
				Some(value) => (self.path(value, "w", out).member("v"), None),
				None => self.borrowed_place(&args[0].node, span, out)?,
			};
			let [arg] = <[Expr; 1]>::try_from(self.operands(true, &[], &args[1..], result, span, out)).ok()?;
			let ok = Expr::Rt("OK".into());
			let read = place.0.clone();
			let (value, result) = match (name, literal) {
				("write_char", _) => (read.binary("+", library::char_text(arg)), ok),
				("write_fmt", Some(s)) => (read.binary("+", Expr::Str(s)), ok),
				// What a placeholder wrote before an error stays written.
				("write_fmt", None) => {
					let written = self.path(Expr::rt("fmtWriteString", vec![read, arg]), "written", out);
					(written.clone().member("s"), written.member("result"))
				}
				_ => (read.binary("+", arg), ok),
			};
			self.set_borrowed(&place, value, span, out);
			return done(result, false);
		}
		let receiver = match through {
			Some(value) => value,
			None => self.operands(true, &[], &args[..1], result, span, out).pop()?,
		};
		let rest = self.operands(true, &[], &args[1..], result, span, out);
		// The crate's impl writes this method itself.
		if self.b.t.translated(def_id, wargs) {
			let values = [vec![receiver], rest].concat();
			return done(self.b.call_def(def_id, wargs, values, span, out), true);
		}
		let [arg] = <[Expr; 1]>::try_from(rest).ok()?;
		if std == Some(Std::Formatter) {
			let e = match (name, literal) {
				("write_char", _) => Expr::rt("fmtWriteStr", vec![receiver, library::char_text(arg)]),
				("write_fmt", Some(s)) => Expr::rt("fmtWriteStr", vec![receiver, Expr::Str(s)]),
				("write_fmt", None) => Expr::rt("fmtWriteFmt", vec![receiver, arg]),
				_ => Expr::rt("fmtWriteStr", vec![receiver, arg]),
			};
			return done(e, true);
		}
		// The provided `write_fmt` writes a literal with one `write_str`, as std's does.
		let text = match (name, literal) {
			("write_char", _) => library::char_text(arg),
			("write_fmt", Some(s)) => Expr::Str(s),
			_ => {
				let msg = format!(
					"`fmt::Arguments` with a placeholder, or not made in place, written to `{w}`'s own `fmt::Write`, which is not supported yet: std splits what a placeholder writes into `write_str` calls the runtime does not make"
				);
				return done(self.err(span, msg), true);
			}
		};
		done(self.b.call_def(write_str, wargs, vec![receiver, text], span, out), true)
	}

	/// The one literal piece of a `fmt::Arguments` operand, held back where `format_args!`
	/// made it.
	fn literal_arguments(&self, op: &Operand<'tcx>) -> Option<String> {
		let held = self.held(op.place()?.as_local()?)?;
		match super::step::peel(&held.value) {
			Expr::Array(parts) if let [Expr::Str(s)] = parts.as_slice() => Some(s.clone()),
			_ => None,
		}
	}

	/// The path of the `Option` a `&mut` operand borrows, and the place of its `Some`'s
	/// payload, of type `payload`, unless the reference is refused.
	fn payload_place(
		&mut self,
		op: &Operand<'tcx>,
		payload: Ty<'tcx>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Option<Borrowed<'tcx>> {
		let (target, place) = self.borrowed_place(op, span, out)?;
		let elems = [
			ProjectionElem::Downcast(None, VariantIdx::from_u32(1)),
			ProjectionElem::Field(FieldIdx::ZERO, payload),
		];
		Some((target, place.map(|p| p.project_deeper(&elems, self.tcx))))
	}

	/// `get_or_insert`, `get_or_insert_with`, `get_or_insert_default`, or `insert` of the
	/// `Option` a `&mut` borrows: the store, then the payload it lends, an alias of it where
	/// the destination is read only as one.
	fn option_insert(
		&mut self,
		name: &str,
		payload_ty: Ty<'tcx>,
		args: &[Spanned<Operand<'tcx>>],
		dest: Place<'tcx>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Option<Option<(Expr, bool)>> {
		let tcx = self.tcx;
		let (target, payload) = self.payload_place(&args[0].node, payload_ty, span, out)?;
		let Some(payload) = payload else {
			return Some(Some((target, true)));
		};
		// The test reads it before either branch, so what it reads is set before both.
		self.materialize(&target, out);
		let whole = tcx.mk_place_deref(args[0].node.place()?);
		let option_ty = whole.ty(&self.mir.local_decls, tcx).ty;
		// What it inserts, made before the `Option` is read, as Rust makes the argument.
		let value = match name {
			"get_or_insert" | "insert" => {
				let value = self.operand(&args[1].node, span, out);
				Some(self.path(value, "value", out))
			}
			_ => None,
		};
		if name == "insert" {
			// What it replaced drops, as `*self = Some(value)` drops it.
			let old = self.b.fresh("old");
			let drop = self.b.t.drop_value(Expr::ident(&old), option_ty, span);
			if drop.is_some() {
				out.push(StmtKind::decl(false, old, None, Some(target.clone())).at(None));
			}
			out.extend(self.store(whole, value?, span));
			out.extend(drop.map(|drop| StmtKind::Expr(drop).at(None)));
		} else {
			let mut then = Vec::new();
			let mut els = Vec::new();
			let value = match name {
				"get_or_insert" => {
					let value = value?;
					if let Some(drop) = self.b.t.drop_value(value.clone(), payload_ty, span) {
						els.push(StmtKind::Expr(drop).at(None));
					}
					value
				}
				"get_or_insert_default" => self.default_of(payload_ty, true, span, &mut then),
				_ => {
					let f_ty = args[1].node.ty(&self.mir.local_decls, tcx);
					if let ty::FnDef(f, f_args) = *f_ty.kind() {
						self.b.call_def(f, f_args.skip_binder(), Vec::new(), span, &mut then)
					} else {
						// A closure it doesn't call drops; one it calls drops as `FnOnce` does.
						let f = self.operand(&args[1].node, span, out);
						let f = self.path(f, "f", out);
						if let Some(drop) = self.b.t.drop_value(f.clone(), f_ty, span) {
							els.push(StmtKind::Expr(drop).at(None));
						}
						match self.b.t.owned_fn(f_ty) {
							Some(kind) if kind != ty::ClosureKind::FnOnce && self.b.t.drops(f_ty) => {
								Expr::rt("callOnce", vec![f])
							}
							_ => f.call(Vec::new()),
						}
					}
				}
			};
			then.extend(self.store(whole, value, span));
			let cond = target.clone().binary("===", self.b.t.none(payload_ty));
			out.push(StmtKind::If { cond, then, els }.at(None));
		}
		let read = Expr::As(Box::new(target.clone()), self.b.t.ty(payload_ty, span));
		if let Some(local) = dest.as_local()
			&& self.defs[local] == 1
			&& matches!(self.slots[local], Slot::Var(_))
		{
			self.slots[local] = Slot::Alias(Some(read));
			self.reborrows.insert(local, payload);
			return Some(None);
		}
		match self.b.t.needs_place(payload_ty) {
			true => Some(Some((self.make_place(payload, target, span, out), true))),
			false => Some(Some((read, true))),
		}
	}

	/// The place a reference operand reads in place, if it is one.
	fn alias_place(&self, op: &Operand<'tcx>) -> Option<Expr> {
		let (Operand::Copy(place) | Operand::Move(place)) = op else {
			return None;
		};
		match (&self.slots[place.as_local()?], place.projection.is_empty()) {
			(Slot::Alias(Some(target)), true) if target.is_path() => Some(target.clone()),
			_ => None,
		}
	}

	/// `name = recv.next()` of a range of numbers, stepped in place rather than through
	/// `rt.Range`'s generic steps, into the item alone and a flag for whether there was
	/// one: a number that is sometimes `rt.DONE` is slower in V8. Past the end, `name` is
	/// left as it was, since only `Some` reads it. A range only `next` reads (`stepped`)
	/// steps past its end too, so the flag is one comparison, to hold back for the switch
	/// that reads it; it assigns `name` only if something reads it (`read`).
	fn range_next(
		&mut self,
		name: String,
		recv: Expr,
		stepped: bool,
		read: bool,
		out: &mut Vec<Stmt>,
	) -> Option<(String, Expr)> {
		let range = match recv {
			Expr::Object(_) => recv,
			recv => self.path(recv, "range", out),
		};
		let start = super::step::stepped_field(&range, "start");
		let end = super::step::stepped_field(&range, "end");
		let flag = self.b.fresh(&format!("{name}Some"));
		self.range_flags.insert(name.clone(), flag.clone());
		if stepped {
			let step = Expr::Postfix(Box::new(start), "++");
			let item = match read {
				true => Expr::ident(&name).assign(step),
				false => step,
			};
			return Some((flag, item.binary("<", end)));
		}
		let more = start.clone().binary("<", end);
		out.push(StmtKind::Expr(Expr::ident(&flag).assign(more)).at(None));
		let step = Expr::ident(&name).assign(Expr::Postfix(Box::new(start), "++"));
		out.push(
			StmtKind::If {
				cond: Expr::ident(&flag),
				then: vec![StmtKind::Expr(step).at(None)],
				els: Vec::new(),
			}
			.at(None),
		);
		None
	}

	/// The value at `place`, named for `base`, after setting `place` to `value`; or nothing
	/// without a `base`, when nothing reads it.
	fn swap_out(
		&mut self,
		place: Borrowed<'tcx>,
		ty: Ty<'tcx>,
		value: Expr,
		base: Option<&str>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Expr {
		// The object written in place gives what it held as an object of its own.
		if place.1.is_some_and(|p| self.in_object(p)) {
			let old = Expr::rt("replaceIn", vec![place.0, value]);
			let Some(base) = base else {
				out.push(StmtKind::Expr(old).at(None));
				return Expr::undefined();
			};
			let name = self.b.fresh(base);
			out.push(StmtKind::decl(false, name.clone(), None, Some(old)).at(None));
			return Expr::ident(name);
		}
		let Some(base) = base else {
			self.set_borrowed(&place, value, span, out);
			return Expr::undefined();
		};
		let name = self.b.fresh(base);
		// TypeScript widens `rt.None` read from a place narrowed to it into a `symbol`.
		let declared = self.b.t.erases_to(ty).none.then(|| self.b.t.ty(ty, span));
		out.push(StmtKind::decl(false, name.clone(), declared, Some(place.0.clone())).at(None));
		self.set_borrowed(&place, value, span, out);
		Expr::ident(name)
	}

	/// `base` if anything reads what a call leaves in `dest`.
	fn read_as(&self, dest: Place<'tcx>, base: &'static str) -> Option<&'static str> {
		dest.as_local()
			.is_none_or(|local| self.uses[local] != 0)
			.then_some(base)
	}

	/// Set what a `&mut` borrows to `value`.
	fn set_borrowed(&mut self, (read, place): &Borrowed<'tcx>, value: Expr, span: Span, out: &mut Vec<Stmt>) {
		match place {
			Some(place) => out.extend(self.store(*place, value, span)),
			None => out.push(StmtKind::Expr(read.clone().assign(value)).at(None)),
		}
	}

	/// `container[index]`, or a slice of it by a range, which a reference reads in place; or
	/// `get` or `get_mut` of a slice by a range.
	fn index(
		&mut self,
		name: &str,
		gargs: GenericArgsRef<'tcx>,
		args: &[Spanned<Operand<'tcx>>],
		dest: Place<'tcx>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Option<Option<(Expr, bool)>> {
		let container = gargs.type_at(0);
		let index_ty = gargs.type_at(1);
		// A string slices by UTF-8 byte offsets, which a JS string doesn't index by.
		if container.is_str() || self.b.t.std_of(container) == Some(Std::String) {
			if name == "index_mut" {
				return None;
			}
			let base = self.operand(&args[0].node, span, out);
			let (start, end) = self.range_bounds(&args[1].node, index_ty, span, out)?;
			let mut values = vec![self.path(base, "s", out), start];
			values.extend(end);
			return Some(Some((Expr::rt("strSlice", values), true)));
		}
		let elem_ty = elem(self.tcx, container)?;
		let bytes = matches!(elem_ty.kind(), ty::Uint(ty::UintTy::U8));
		let base = self.operand(&args[0].node, span, out);
		match Int::of(index_ty) {
			Some(_) => {
				// A `Vec`'s element is its storage's, a byte vector's in `data`.
				let base = self.path(base, "slice", out);
				let i = self.operand(&args[1].node, span, out);
				let i = self.path(i, "index", out);
				let checked = Expr::rt("idx", vec![base.clone(), i]);
				let checked = self.path(checked, "at", out);
				let place = match container.kind() {
					ty::Adt(..) if bytes => base.member("data").index(checked),
					// An array's bytes come through a reference, which is a slice of them.
					_ if super::place::byte_slice(container) || crate::lower::byte_array(container) => {
						super::place::byte_at(base, checked)
					}
					_ => base.index(checked),
				};
				// A reference to an array of bytes is a slice of them.
				if crate::lower::byte_array(elem_ty) {
					return Some(Some((Expr::rt("byteSliceFrom", vec![place]), true)));
				}
				// The reference reads and writes the element where it is.
				if let Some(local) = dest.as_local()
					&& self.defs[local] == 1
					&& matches!(self.slots[local], Slot::Var(_))
				{
					self.slots[local] = Slot::Alias(Some(place));
					return Some(None);
				}
				if name == "index_mut" && self.b.t.needs_place(elem_ty) {
					let target = self.stable_path(place, false, span, out);
					return Some(Some((self.place_of(target, None), true)));
				}
				Some(Some((place, true)))
			}
			None => {
				let get = matches!(name, "get" | "get_mut");
				let lent = matches!(name, "index_mut" | "get_mut");
				// The range's bounds are taken apart before the slice is read, while they are
				// still held back. Any other range, as an `rt.Bounds`, is checked by the
				// runtime, as std checks its own fields, into a start and end.
				let bounds = match self.range_bounds(&args[1].node, index_ty, span, out) {
					Some(bounds) => Ok(bounds),
					None if crate::lower::collections::plain_bounds(self.b.t, index_ty) => {
						Err(self.operand(&args[1].node, span, out))
					}
					None => return None,
				};
				let base = match container.kind() {
					ty::Adt(..) => library::vec_view(base, elem_ty),
					_ => base,
				};
				// All of a mutable slice of other values is the slice itself.
				let full =
					matches!(index_ty.kind(), ty::Adt(adt, _) if self.tcx.item_name(adt.did()).as_str() == "RangeFull");
				if full && !bytes && lent {
					return Some(Some((base, true)));
				}
				let base = self.path(base, "slice", out);
				let (start, end) = match bounds {
					Ok(bounds) => bounds,
					Err(range) => {
						let length = super::slice_member(base.clone(), "length");
						let checked = match get {
							true => "sliceGetBounds",
							false => "sliceIndexBounds",
						};
						let pair = self.path(Expr::rt(checked, vec![range, length]), "bounds", out);
						(pair.clone().index(Expr::lit("0")), Some(pair.index(Expr::lit("1"))))
					}
				};
				let mut values = vec![base, start];
				values.extend(end);
				// A range of bytes is a view already, and one of other values lent mutably the
				// runtime's view, as a JS array has none of its own.
				let range = match (bytes, lent, get) {
					(true, _, false) => "byteSliceRange",
					(true, _, true) => "byteSliceGetRange",
					(false, true, false) => "sliceRangeMut",
					(false, true, true) => "sliceGetRangeMut",
					(false, false, false) => "sliceRange",
					(false, false, true) => "sliceGetRange",
				};
				Some(Some((Expr::rt(range, values), true)))
			}
		}
	}

	/// The start and optional end of a range: `a..b`, `..b`, `a..`, or `..`.
	fn range_bounds(
		&mut self,
		op: &Operand<'tcx>,
		ty: Ty<'tcx>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Option<(Expr, Option<Expr>)> {
		let tcx = self.tcx;
		let ty::Adt(adt, _) = ty.kind() else {
			return None;
		};
		let name = tcx.item_name(adt.did()).to_string();
		match name.as_str() {
			"RangeFull" => return Some((Expr::lit("0"), None)),
			"Range" | "RangeTo" | "RangeFrom" => {}
			_ => return None,
		}
		let mut fields = self.range_fields(op, *adt, span, out).into_iter();
		match name.as_str() {
			"Range" => Some((fields.next()?, Some(fields.next()?))),
			"RangeTo" => Some((Expr::lit("0"), fields.next())),
			_ => Some((fields.next()?, None)),
		}
	}

	/// A std range's fields, in declaration order, read where the range is made if it is.
	fn range_fields(
		&mut self,
		op: &Operand<'tcx>,
		adt: ty::AdtDef<'tcx>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Vec<Expr> {
		if let Some(parts) = self.take_parts(op) {
			return parts;
		}
		let range = self.operand(op, span, out);
		let range = self.path(range, "range", out);
		adt.non_enum_variant()
			.fields
			.iter()
			.map(|f| range.clone().member(f.name.as_str()))
			.collect()
	}

	/// What a `DebugList` iterates, as an iterable JS value: an array or `Vec`'s elements,
	/// whose items the runtime drops if it owns them, or else an `rt.Iter`, which it drops
	/// once done.
	fn items(&mut self, value: Expr, ty: Ty<'tcx>, span: Span) -> Option<Expr> {
		let tcx = self.tcx;
		match ty.peel_refs().kind() {
			ty::Slice(_) | ty::Array(..) => Some(library::elements(value, ty)),
			ty::Adt(adt, _) if Std::of(tcx, adt.did()) == Some(Std::Vec) => Some(library::elements(value, ty)),
			_ => crate::lower::iter::into_iter(self.b, ty, value, span),
		}
	}
}

/// The element type of a slice, array, or `Vec`.
fn elem<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
	match ty.peel_refs().kind() {
		ty::Slice(e) | ty::Array(e, _) => Some(*e),
		ty::Adt(adt, args) if Std::of(tcx, adt.did()) == Some(Std::Vec) => Some(args.type_at(0)),
		_ => None,
	}
}

fn int_named(name: &str) -> Option<Int> {
	let (signed, bits, pointer) = match name {
		"u8" => (false, 8, false),
		"u16" => (false, 16, false),
		"u32" => (false, 32, false),
		"u64" => (false, 64, false),
		"u128" => (false, 128, false),
		"usize" => (false, 32, true),
		"i8" => (true, 8, false),
		"i16" => (true, 16, false),
		"i32" => (true, 32, false),
		"i64" => (true, 64, false),
		"i128" => (true, 128, false),
		"isize" => (true, 32, true),
		_ => return None,
	};
	Some(Int { bits, signed, pointer })
}

/// How a `ControlFlow` type is represented.
fn flow<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Flow {
	match ty.kind() {
		ty::Adt(_, args) => Flow::of(tcx, args),
		_ => unreachable!("checked by the caller"),
	}
}

/// The test that `scrut`, a `ControlFlow`, is `Continue` (variant 0) or `Break`.
pub fn control_flow_test<'tcx>(fx: &mut Fx<'_, '_, '_, 'tcx>, scrut: &Expr, ty: Ty<'tcx>, variant: VariantIdx) -> Expr {
	let continues = variant.index() == 0;
	if matches!(scrut, Expr::Ident(n) if fx.next_names.contains(n)) {
		return match continues {
			true => scrut.clone().binary("!==", Expr::Rt("DONE".into())),
			false => scrut.clone().binary("===", Expr::Rt("DONE".into())),
		};
	}
	let ty::Adt(_, args) = ty.kind() else {
		unreachable!("checked by the caller")
	};
	Flow::test(fx.b.t, args, scrut.clone(), continues)
}

/// A field of a `ControlFlow`: `Continue`'s value, or `Break`'s, which for `?` is the
/// value it was applied to.
pub fn control_flow_field<'tcx>(
	fx: &mut Fx<'_, '_, '_, 'tcx>,
	base: Expr,
	ty: Ty<'tcx>,
	variant: Option<VariantIdx>,
) -> Expr {
	let continues = variant.is_none_or(|v| v.index() == 0);
	match (flow(fx.tcx, ty), continues) {
		(Flow::Result | Flow::Other, true) => base.member("value"),
		(Flow::Result, false) => base,
		(Flow::Other, false) => base.member("error"),
		(Flow::Option, true) => base,
		(Flow::Option, false) => Expr::undefined(),
	}
}

/// A `ControlFlow` made in place.
pub fn control_flow<'tcx>(
	fx: &mut Fx<'_, '_, '_, 'tcx>,
	ty: Ty<'tcx>,
	variant: VariantIdx,
	mut values: Vec<Expr>,
) -> Expr {
	let value = values.pop().unwrap_or_else(Expr::undefined);
	match (flow(fx.tcx, ty), variant.index()) {
		(Flow::Result | Flow::Other, 0) => Expr::result(true, value),
		(Flow::Result, _) => value,
		(Flow::Other, _) => Expr::result(false, value),
		(Flow::Option, 0) => value,
		(Flow::Option, _) => fx.b.t.none(continued(ty)),
	}
}

/// What a `ControlFlow` continues with.
fn continued(ty: Ty<'_>) -> Ty<'_> {
	match ty.kind() {
		ty::Adt(_, args) => args.type_at(1),
		_ => unreachable!("checked by the caller"),
	}
}

/// Whether an assignment can write `e`: a path, an element, or a member of one.
fn assignable(e: &Expr) -> bool {
	match e {
		Expr::Index(..) => true,
		Expr::Member(base, _) | Expr::Located(_, base) => assignable(base),
		e => e.is_path(),
	}
}
