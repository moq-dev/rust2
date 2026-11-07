//! Calls: to the crate's own functions, to closures, and to std through the library
//! mappings. Some std calls lower here instead, since MIR passes what THIR wrote as an
//! expression through a temporary: a place to replace (`mem::take`), a range to slice
//! by, an array of bytes to read, `format_args!`'s parts, and `?`'s `ControlFlow`.

use rustc_abi::{FieldIdx, VariantIdx};
use rustc_middle::mir::{Operand, Place, ProjectionElem};
use rustc_middle::ty::{self, GenericArgsRef, Instance, Ty, TyCtxt};
use rustc_span::{Span, Spanned};

use super::{Fx, Slot};
use crate::lower::ObjectKind;
use crate::lower::fmt::{self, Piece};
use crate::lower::lend::{self, Plan};
use crate::lower::library::{self, Flow, Std};
use crate::lower::num::Int;
use crate::ts::{self, Expr, Stmt, StmtKind};

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
		self.arguments.clear();
		if self.forwards(dest) {
			self.forward_call(args, span, out);
			return;
		}
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
			// `next` into a local that holds the protocol's item or `rs.DONE`, unconverted.
			ty::FnDef(def_id, gargs)
				if crate::lower::iter::is_next(tcx, def_id)
					&& dest.as_local().is_some_and(|l| self.next_results.contains(&l)) =>
			{
				let self_ty = gargs.skip_binder().type_at(0);
				let (iter_ty, recv) = match args[0].node.place() {
					// Through std's `Iterator` for `&mut I` and `Box<I>`: what the last `&mut`
					// points at, read through each reference and box before it.
					Some(p) if self_ty.is_ref() || self_ty.boxed_ty().is_some() => {
						let value = self.place_expr(tcx.mk_place_deref(p), span, &mut pre);
						let (last, value) = crate::lower::iter::peel_iter(self.b, self_ty, value);
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
			ty::FnDef(def_id, gargs) if let Some(why) = self.stray(def_id, gargs.skip_binder(), args) => {
				(self.err(span, why), true)
			}
			ty::FnDef(def_id, gargs)
				if let Some(cast) = self.alloc_cast_call(def_id, gargs.skip_binder(), args, span, &mut pre) =>
			{
				(cast, true)
			}
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
		let value = self.locate_arguments(value);
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
	/// marks is what an `rs.Place` it is reads, as std's mappings take a `&mut` parameter.
	pub(super) fn operands(
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
			if arg.node.place().is_some()
				&& let Some(value) = values.last().filter(|v| !matches!(v, Expr::Located(..)))
				&& let Some(loc) = self.b.t.loc(arg.span)
			{
				self.arguments.push((value.clone(), loc));
			}
		}
		self.into_frozen = saved;
		values
	}

	/// `call` with each argument [`Self::operands`] read from a place at that argument's
	/// own Rust. Std's mappings match the arguments' shapes, so they are given them bare,
	/// and each is located where it appears as an argument of what they made.
	pub(super) fn locate_arguments(&mut self, mut call: Expr) -> Expr {
		let mut arguments = std::mem::take(&mut self.arguments);
		locate(&mut call, &mut arguments);
		call
	}

	/// An argument, which the callee copies if it changes it in place.
	fn operand_frozen(&mut self, op: &Operand<'tcx>, span: Span, out: &mut Vec<Stmt>) -> Expr {
		match op {
			Operand::Copy(place) | Operand::Move(place) if !self.in_array(*place) => self.place_expr(*place, span, out),
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
		// Calling a closure or function value: `FnOnce::call_once(f, (args,))`, or
		// `AsyncFnOnce::async_call_once`.
		if let Some(trait_id) = tcx.trait_of_assoc(def_id)
			&& let Some(kind) = tcx
				.fn_trait_kind_from_def_id(trait_id)
				.or_else(|| tcx.async_fn_trait_kind_from_def_id(trait_id))
			&& let [callee, tupled] = args
		{
			// An `AssertUnwindSafe` is the function it holds.
			let mut callee_ty = gargs.type_at(0);
			while let Some(inner) = library::unwind_safe(tcx, callee_ty) {
				callee_ty = inner;
			}
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
				let inner_args = inner_args.skip_binder();
				// std's that takes a `&mut` is called as its value, which lowers the call as a
				// written one does, given the `&mut`s as they are passed.
				if self.b.t.mapped(inner, inner_args)
					&& !matches!(tcx.def_kind(inner), rustc_hir::def::DefKind::Ctor(..))
					&& (self.b.t.sig(inner, inner_args).inputs().iter())
						.any(|i| matches!(i.kind(), ty::Ref(_, _, rustc_ast::Mutability::Mut)))
				{
					return self.b.fn_value(inner, inner_args, span).call(values);
				}
				return self.b.call_def(inner, inner_args, values, span, out);
			}
			// A `&mut` to a function JS copies, such as a box of one, or to a reference to one, as
			// std's impls for a `&mut` and a box forward through, is its place: call what is
			// there. A box, or an `AssertUnwindSafe`, is what it holds.
			let mut through = callee.node.ty(&self.mir.local_decls, tcx);
			let mut f = match callee.node.place() {
				Some(p) if let Some(pointee) = self.b.t.place_pointee(through) => {
					through = pointee;
					self.place_expr(tcx.mk_place_deref(p), span, out)
				}
				_ => self.operand(&callee.node, span, out),
			};
			loop {
				let held = through.boxed_ty().or_else(|| library::unwind_safe(tcx, through));
				through = match (held, self.b.t.place_pointee(through)) {
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
			// what it owns; of an async closure, it makes the future of rustc's by-move body,
			// which owns what the closure captured.
			if kind == ty::ClosureKind::FnOnce
				&& self
					.b
					.t
					.owned_fn(callee_ty)
					.is_some_and(|kind| kind != ty::ClosureKind::FnOnce)
				&& self.b.t.drops(callee_ty)
			{
				return Expr::rs("callOnce", [vec![f], values].concat());
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
					&& let Some(why) = args[0]
						.node
						.place()
						.and_then(|p| self.made_out_of_step(def_id, gargs, tcx.mk_place_deref(p)))
				{
					return self.err(span, why);
				}
				crate::lower::pointer::pointees(self.b.t, def_id, gargs)
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
	/// its mapping has as the value, not an `rs.Place`: one it returns or its iterator
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
		// What `IntoIterator::into_iter` lends of a `&mut` to a collection, each element as
		// its place.
		let into_places = |ty: Ty<'tcx>| {
			let ty = crate::lower::iter::unbox_ref(tcx, ty);
			ty.ref_mutability() == Some(ty::Mutability::Mut)
				&& match ty.peel_refs().kind() {
					ty::Slice(_) | ty::Array(..) => true,
					_ => matches!(
						t.std_of(ty.peel_refs()),
						Some(
							Std::Vec
								| Std::Option
								| Std::Result
								| Std::HashMap
								| Std::HashTable
								| Std::BTreeMap
								| Std::VecDeque
								| Std::LinkedList
						)
					),
				}
		};
		// Mappings that lend an element JS copies as its place, as a map's `get_mut` does, the
		// item a `Peekable` peeked, the place a `&mut dyn Any` wraps, a `RefCell` key's value,
		// a `Result`'s payload, the value of an `Arc`, a lock, or a cell, what a write guard
		// lends its map or its leak, what `borrow_mut` lends, the `&mut` an iterator's `by_ref`
		// is given, what `zip` makes of the collection it is given by `into_iter`, as the free
		// `zip` does of either, or an empty iterator, made or defaulted, which lends nothing.
		let lends = match library::key(tcx, def_id) {
			Some((owner, name)) => match (owner.as_str(), name.as_str()) {
				("slice", "get_mut" | "first_mut" | "last_mut" | "iter_mut" | "split_first_mut" | "split_last_mut")
				| ("array", "each_mut")
				| ("dyn Any", "downcast_mut")
				| ("Peekable", "peek_mut")
				| ("LocalKey", "with_borrow_mut")
				| ("Result", "iter_mut")
				| ("Arc" | "Rc", "get_mut" | "make_mut")
				| ("Mutex" | "RwLock" | "RefCell" | "OnceCell" | "OnceLock", "get_mut")
				| ("RefMut", "leak")
				| ("RefCell", "undo_leak" | "replace_with")
				| ("BorrowMut", "borrow_mut")
				| ("Iterator", "by_ref" | "scan")
				| ("empty", "empty")
				| (
					"RefMut" | "MutexGuard" | "MappedMutexGuard" | "RwLockWriteGuard" | "MappedRwLockWriteGuard",
					"map" | "filter_map" | "try_map" | "map_split",
				) => true,
				("IntoIterator", "into_iter") => gargs.types().next().is_some_and(into_places),
				("Iterator", "zip") => gargs.types().nth(1).is_some_and(into_places),
				("zip", "zip") => gargs.types().take(2).any(into_places),
				("Default", "default") => gargs.types().next().is_some_and(|ty| library::empty_default(tcx, ty)),
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
			// A pointer from a reference to an `Arc`'s value, which is that value's one place.
			("ptr", "from_ref") | ("From", "from")
				if (owner == "ptr"
					|| crate::lower::pointer::raw_pointee(tcx, dest.ty(&self.mir.local_decls, tcx).ty).is_some())
					&& let Some(p) = self.arc_raw(&args[0].node, span, out) =>
			{
				done(p, false)
			}
			// `get` of the cell a reference points at, which the pointer it was made into
			// would only be where that cell is kept.
			("UnsafeCell", "raw_get")
				if let Some(cell) = args[0]
					.node
					.place()
					.and_then(|p| self.raw_cells.get(&p.as_local()?).copied()) =>
			{
				if let Some(why) = self.made_out_of_step(def_id, gargs, cell) {
					return Some(Some((self.err(span, why), false)));
				}
				let cell = self.place_expr(cell, span, out);
				let get = tcx
					.associated_items(tcx.parent(def_id))
					.filter_by_name_unhygienic(rustc_span::Symbol::intern("get"))
					.next()?
					.def_id;
				done(self.b.call_def(get, gargs, vec![cell], span, out), false)
			}
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
				let ty = targ(0)?;
				let a = self.borrowed_place(&args[0].node, span, out)?;
				let c = self.borrowed_place(&args[1].node, span, out)?;
				let in_object = |b: &Borrowed<'tcx>| b.1.is_some_and(|p| self.in_object(p));
				// Which of two elements is written in place is known only as it runs.
				if !in_object(&a)
					&& !in_object(&c)
					&& let (Some((s, i)), Some((t, j))) = (self.element_in(&a, ty), self.element_in(&c, ty))
				{
					let mut made = vec![s, i, t, j];
					made.extend(self.b.t.copy_depth(ty));
					out.push(StmtKind::Expr(Expr::rs("sliceSwapAcross", made)).at(None));
					return done(Expr::undefined(), false);
				}
				// An object written in place gives a copy of what it held, so it goes first:
				// a place given the object itself would share what then changes in it.
				let first = in_object(&c) || !in_object(&a) && self.element_in(&c, ty).is_some();
				let (a, c) = match first {
					true => (c, a),
					false => (a, c),
				};
				let old = self.swap_out(a, ty, c.0.clone(), Some("swapped"), span, out);
				self.set_borrowed(&c, old, span, out);
				done(Expr::undefined(), false)
			}
			// What `clone_from` leaves where the `&mut` borrows a boxed slice or text, through
			// any boxes of it, as `clone_into` leaves it in its target.
			("Clone", "clone_from") | ("ToOwned", "clone_into")
				if targ(0).is_some_and(|t| {
					let own = library::unboxed(self.b.t, t);
					library::box_slice(tcx, own).is_some() || own.boxed_ty().is_some_and(|t| t.is_str())
				}) =>
			{
				let (target, source) = match name.as_str() {
					"clone_from" => (&args[0].node, &args[1].node),
					_ => (&args[1].node, &args[0].node),
				};
				let place = self.borrowed_place(target, span, out)?;
				let source = self.operand(source, span, out);
				let value = match library::box_slice(tcx, library::unboxed(self.b.t, targ(0)?)) {
					Some(elem) => library::box_slice_clone_from(self.b, place.0.clone(), source, elem, span),
					None => source,
				};
				self.set_borrowed(&place, value, span, out);
				done(Expr::undefined(), false)
			}
			// A box of the crate's own value clones it by its own `clone_from`. A box is what
			// it holds, so a `&` to one is a `&` to that, and a `&mut` the place holding it,
			// where a `&mut` to it is not one.
			("Clone", "clone_from") | ("ToOwned", "clone_into")
				if let Some(ty) = targ(0)
					&& let own = library::unboxed(self.b.t, ty)
					&& own != ty
					&& let clone_from = crate::lower::collections::trait_fn(tcx, "Clone", "clone_from")
					&& self.b.t.translated(clone_from, tcx.mk_args(&[own.into()])) =>
			{
				let (target, source) = match name.as_str() {
					"clone_from" => (&args[0].node, &args[1].node),
					_ => (&args[1].node, &args[0].node),
				};
				let mut to = self.operand(target, span, out);
				if !self.b.t.needs_place(own) {
					to = self.b.t.pointee_value(target.ty(&self.mir.local_decls, tcx), to);
				}
				let from = self.operand(source, span, out);
				let call = self
					.b
					.call_def(clone_from, tcx.mk_args(&[own.into()]), vec![to, from], span, out);
				done(call, true)
			}
			// An array that clones over each element where that does more than copy, and a
			// `Vec`, which keeps its buffer, each written in place: a slice's `clone_into` a
			// `Vec` too.
			("Clone", "clone_from") | ("ToOwned", "clone_into")
				if let Some(ty) = self.cloned_over(&name, args)
					&& let own = library::unboxed(self.b.t, ty)
					&& match own.kind() {
						ty::Array(..) => !library::clone_copies(self.b.t, self.b.t.typing_env_pub(), own),
						_ => self.b.t.std_of(own) == Some(Std::Vec),
					} =>
			{
				let (target, source) = match name.as_str() {
					"clone_from" => (&args[0].node, &args[1].node),
					_ => (&args[1].node, &args[0].node),
				};
				let to = self.operand(target, span, out);
				let to = self.b.t.pointee_value(target.ty(&self.mir.local_decls, tcx), to);
				let source = self.operand(source, span, out);
				// A reference to an array is a slice of it, cloned over element by element.
				if let ty::Array(elem, _) = ty.kind() {
					let fns = library::clone_from_fns(self.b, *elem, span);
					return done(Expr::rs("sliceCloneFromSlice", [vec![to, source], fns].concat()), true);
				}
				done(library::clone_from(self.b, to, source, ty, span), true)
			}
			// `*self = source.clone()`, as std's `Clone` for a `T: Clone` has `clone_from` and
			// its `ToOwned` has `clone_into`, cloning before it drops what it writes over, and
			// else std's own over what it holds. A list's maps, as it clones each element into
			// one of its own nodes, and a deque's, as it clears itself first and keeps its
			// capacity.
			("Clone", "clone_from") | ("ToOwned", "clone_into")
				if !self.b.t.translated(def_id, gargs)
					&& !matches!(self.b.t.std_of(targ(0)?), Some(Std::LinkedList | Std::VecDeque)) =>
			{
				let ty = self.cloned_over(&name, args)?;
				if !ty.is_sized(tcx, self.b.t.typing_env_pub()) {
					return None;
				}
				let (target, source) = match name.as_str() {
					"clone_from" => (&args[0].node, &args[1].node),
					_ => (&args[1].node, &args[0].node),
				};
				let place = self.borrowed_place(target, span, out)?;
				let source = self.operand(source, span, out);
				if !library::clone_from_clones(self.b.t, ty) {
					let value = library::clone_from(self.b, place.0.clone(), source, ty, span);
					self.set_borrowed(&place, value, span, out);
					return done(Expr::undefined(), false);
				}
				let clone = crate::lower::collections::trait_fn(tcx, "Clone", "clone");
				let clone = self
					.b
					.call_def(clone, tcx.mk_args(&[ty.into()]), vec![source], span, out);
				let drops = self.b.t.drops(ty);
				let old = self.swap_out(place, ty, clone, drops.then_some("old"), span, out);
				if drops && let Some(drop) = self.b.t.drop_value(old, ty, span) {
					out.push(StmtKind::Expr(drop).at(None));
				}
				done(Expr::undefined(), false)
			}
			// `&raw mut *r`, of the place `r` names where it is an alias of one.
			("ptr", "from_mut") => {
				let place = args[0].node.place()?;
				let target = self.through_aliases(tcx.mk_place_deref(place));
				let raw = rustc_middle::mir::Rvalue::RawPtr(rustc_middle::mir::RawPtrKind::Mut, target);
				done(self.rvalue(&raw, dest, span, out)?, false)
			}
			// A slice or an array of the one value, or a cell of it, read and written where the
			// `&mut` points.
			("raw" | "array" | "Cell" | "UnsafeCell", "from_mut")
				if owner != "raw" || library::std_path(tcx, def_id) == ["slice", "raw", "from_mut"] =>
			{
				let made = self.one_of(&owner, targ(0)?, &args[0].node, span, out)?;
				done(made, true)
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
			// A `String` is an object, which each of its own steps edits in place.
			("String", "push" | "push_str") | ("AddAssign", "add_assign")
				if owner == "String"
					|| (targ(0).is_some_and(|t| library::owned_string(tcx, t))
						&& !self.b.t.translated(def_id, gargs)) =>
			{
				let values = self.operands(false, &[], args, tcx.types.unit, span, out);
				let f = if name == "push" { "stringPush" } else { "stringPushStr" };
				done(Expr::rs(f, values), true)
			}
			("AddAssign", "add_assign")
				if targ(0).is_some_and(|t| library::cow_str(tcx, t)) && !self.b.t.translated(def_id, gargs) =>
			{
				let place = self.borrowed_place(&args[0].node, span, out)?;
				let rhs = self.operand(&args[1].node, span, out);
				let rhs = library::str_cow(tcx, targ(1)?, rhs);
				let joined = Expr::rs("strCowAdd", vec![place.0.clone(), rhs]);
				self.set_borrowed(&place, joined, span, out);
				done(Expr::undefined(), false)
			}
			// What the `&mut` borrows gets the value of the other case, as a byte, a char, and a
			// JS string can't change in place, and the case of ASCII keeps its length.
			("u8" | "char" | "str", "make_ascii_uppercase" | "make_ascii_lowercase") => {
				let place = self.borrowed_place(&args[0].node, span, out)?;
				let to = crate::lower::camel(&name.replacen("make", "to", 1));
				let to = match owner.as_str() {
					"str" => Expr::rs_of("str", &to),
					_ => Expr::Rs(to),
				};
				let changed = to.call(vec![place.0.clone()]);
				self.set_borrowed(&place, changed, span, out);
				done(Expr::undefined(), false)
			}
			// The string extended, by the iterator read as std's `Extend` reads it.
			("Extend", "extend") if targ(0).is_some_and(|t| self.b.t.std_of(t) == Some(Std::String)) => {
				let (item, source) = (targ(1)?, targ(2)?);
				let s = self.operand(&args[0].node, span, out);
				let items = self.operand(&args[1].node, span, out);
				let iter = crate::lower::iter::iter_of(self.b.t, source)?;
				let items = crate::lower::iter::into_iter(self.b, source, items, span)?;
				if crate::lower::iter::str_array(source) {
					return done(Expr::rs("stringExtendArray", vec![s, items]), true);
				}
				done(
					crate::lower::iter::string_extend(self.b, s, items, item, iter, span)?,
					true,
				)
			}
			("String", "reserve" | "reserve_exact" | "try_reserve" | "try_reserve_exact") => {
				let values = self.operands(false, &[], args, tcx.types.unit, span, out);
				done(Expr::rs_of("string", &crate::lower::camel(&name)).call(values), true)
			}
			("String", "shrink_to_fit" | "shrink_to") => {
				let mut values = self.operands(false, &[], args, tcx.types.unit, span, out);
				if name == "shrink_to_fit" {
					values.push(Expr::lit("0"));
				}
				done(Expr::rs("stringShrinkTo", values), true)
			}
			// The string edited in place, by the runtime with what it takes after the string.
			(
				"String",
				"clear" | "truncate" | "insert" | "insert_str" | "retain" | "replace_range" | "extend_from_within",
			) => {
				let result = dest.ty(&self.mir.local_decls, tcx).ty;
				let mut values = vec![self.operand(&args[0].node, span, out)];
				values.extend(self.operands(false, &[], &args[1..], result, span, out));
				if name == "insert" {
					let c = values.pop()?;
					values.push(library::char_text(c));
				}
				// `retain` drops a closure it owns as it returns, after the edit.
				let mut spent = None;
				if name == "retain" {
					let f = std::mem::replace(&mut values[1], Expr::undefined());
					let (owned, lent, f) = library::called_through(self.b.t, targ(0)?, Some(f));
					let mut f = f?;
					if !lent && self.b.t.drops(owned) {
						f = self.path(f, "f", out);
						spent = self.b.t.drop_value(f.clone(), owned, span);
					}
					values[1] = f;
				}
				// A range std reads the bounds of, as `bounds` makes any `RangeBounds` one.
				if matches!(name.as_str(), "replace_range" | "extend_from_within") {
					let call = library::Call {
						def_id,
						args: gargs,
						span,
					};
					let range = std::mem::replace(&mut values[1], Expr::undefined());
					let len = values[0].clone().member("len");
					values[1] = crate::lower::collections::bounds(self.b, &call, range).within(len);
				}
				let f = match name.as_str() {
					"clear" => "stringClear",
					"truncate" => "stringTruncate",
					"insert" | "insert_str" => "stringInsertStr",
					"retain" => "stringRetain",
					"replace_range" => "stringReplaceRange",
					_ => "stringExtendFromWithin",
				};
				out.push(StmtKind::Expr(Expr::rs(f, values)).at(None));
				out.extend(spent.map(|drop| StmtKind::Expr(drop).at(None)));
				done(Expr::undefined(), false)
			}
			("String", "pop" | "remove" | "split_off") => {
				let result = dest.ty(&self.mir.local_decls, tcx).ty;
				let mut values = vec![self.operand(&args[0].node, span, out)];
				values.extend(self.operands(false, &[], &args[1..], result, span, out));
				let f = match name.as_str() {
					"pop" => "stringPop",
					"remove" => "stringRemove",
					_ => "stringSplitOff",
				};
				done(Expr::rs(f, values), true)
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
				let value = Expr::rs(f, vec![place.0.clone(), by]);
				self.set_borrowed(&place, value, span, out);
				done(Expr::undefined(), false)
			}
			// A borrowed `Cow<str>` owns its text first, and the `&mut` is the `String` it then is.
			("Cow", "to_mut") if targ(0).is_some_and(|t| t.is_str()) => {
				let place = self.borrowed_place(&args[0].node, span, out)?;
				self.set_borrowed(&place, Expr::rs("strCowIntoOwned", vec![place.0.clone()]), span, out);
				done(
					Expr::As(Box::new(place.0), crate::ts::Type::Rs("String", Vec::new())),
					false,
				)
			}
			// A `String` is the place of its `str`.
			("String", "as_mut_str" | "leak") => done(self.operand(&args[0].node, span, out), false),
			("DerefMut", "deref_mut") | ("AsMut", "as_mut") | ("BorrowMut", "borrow_mut")
				if targ(0).is_some_and(|t| self.b.t.std_of(t) == Some(Std::String))
					&& dest.ty(&self.mir.local_decls, tcx).ty.peel_refs().is_str() =>
			{
				done(self.operand(&args[0].node, span, out), false)
			}
			("DerefMut", "deref_mut") | ("AsMut", "as_mut") | ("BorrowMut", "borrow_mut")
				if targ(0).is_some_and(|t| t.is_str())
					&& dest.ty(&self.mir.local_decls, tcx).ty.peel_refs().is_str() =>
			{
				self.same_place(args, dest, span, out)
			}
			// A box is what it holds, so the `&mut` borrows the place holding the box, and so is an
			// `AssertUnwindSafe`.
			("DerefMut", "deref_mut") | ("AsMut", "as_mut") | ("BorrowMut", "borrow_mut")
				if targ(0)
					.and_then(|t| t.boxed_ty().or_else(|| library::unwind_safe(tcx, t)))
					.is_some_and(|t| self.b.t.needs_place(t)) =>
			{
				self.same_place(args, dest, span, out)
			}
			// A place nothing else holds: `Default`'s empty `str`.
			("Default", "default")
				if targ(0)
					.and_then(|t| self.b.t.place_pointee(t))
					.is_some_and(|t| t.is_str()) =>
			{
				done(
					Expr::Object(vec![(crate::ts::Key::Name("v".into()), Expr::Str(String::new()))]),
					false,
				)
			}
			// The `&mut str` is the bytes it wrote, which a write through it changes in place.
			("char", "encode_utf8") => {
				let values = self.operands(false, &[], args, dest.ty(&self.mir.local_decls, tcx).ty, span, out);
				done(Expr::rs("charEncodeUtf8", values), true)
			}
			("str", "split_at_mut" | "split_at_mut_checked") => {
				let values = self.operands(false, &[], args, dest.ty(&self.mir.local_decls, tcx).ty, span, out);
				let f = match name.as_str() {
					"split_at_mut" => "strSplitAtMut",
					_ => "strSplitAtMutChecked",
				};
				done(Expr::rs(f, values), true)
			}
			// What the `&mut` borrows gets the result: a primitive's operator's, as `+=` on one
			// computes, or a `Wrapping`'s or a `Saturating`'s, which is its integer, wrapped or
			// saturated.
			(_, _)
				if let Some(op) = owner.strip_suffix("Assign").and_then(library::operator)
					&& let Some(ty) = targ(0)
					&& (library::primitive(ty) || library::overflowing(tcx, ty).is_some()) =>
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
				let value = match library::overflowing(tcx, ty) {
					Some((overflow, int)) => crate::lower::num::overflowing(self.b, overflow, op, l, r, int, span),
					None => crate::lower::num::binary(self.b, op, l, r, ty, targ(1)?, span),
				};
				self.set_borrowed(&place, value, span, out);
				done(Expr::undefined(), false)
			}
			// A `Cow<[T]>` that borrows takes a clone of its slice, whose `Vec` the `&mut` lends.
			("Cow", "to_mut") if let ty::Slice(elem) = targ(0)?.kind() => {
				let place = self.borrowed_place(&args[0].node, span, out)?;
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
			// The place of the payload, if any: `as_mut`'s, or what `iter_mut` yields. A box's is
			// what it holds, which `as_deref_mut` lends.
			("Option", "as_mut" | "iter_mut" | "as_deref_mut")
				if match name.as_str() {
					"as_deref_mut" => targ(0)?.boxed_ty().is_some_and(|t| self.b.t.needs_place(t)),
					_ => self.b.t.needs_place(targ(0)?),
				} =>
			{
				let (target, payload) = self.payload_place(&args[0].node, targ(0)?, span, out)?;
				let Some(payload) = payload else {
					return done(target, true);
				};
				let none = target.clone().binary("===", self.b.t.none(targ(0)?));
				let place = self.make_place(payload, target, span, out);
				let lent = Expr::Cond(Box::new(none), Box::new(Expr::undefined()), Box::new(place));
				match name.as_str() {
					"iter_mut" => done(Expr::rs("iterOption", vec![lent]), true),
					_ => done(lent, true),
				}
			}
			// A slice of the payload, if any, read and written where the `Option` holds it, as
			// `slice::from_mut` makes one of a place or of the object a `&mut` is, and else an
			// empty one.
			("Option", "as_mut_slice") => {
				let inner = targ(0)?;
				if crate::lower::byte(inner) {
					let refused = self.err(
						span,
						"a slice of one byte's place, which a byte slice can view only in an array of bytes",
					);
					return done(refused, true);
				}
				let (target, payload) = self.payload_place(&args[0].node, inner, span, out)?;
				let Some(payload) = payload else {
					return done(target, true);
				};
				let none = target.clone().binary("===", self.b.t.none(inner));
				let some = if self.b.t.needs_place(inner) {
					Expr::rs("sliceOfPlace", vec![self.make_place(payload, target, span, out)])
				} else if self.b.t.write_in(inner).is_some() {
					Expr::rs("sliceIn", vec![target])
				} else {
					self.err(
						span,
						format!("a slice of the payload of `Option<{inner}>`, which JS can't write in place"),
					)
				};
				done(
					Expr::Cond(
						Box::new(none),
						Box::new(Expr::rs("sliceFrom", vec![Expr::Array(vec![])])),
						Box::new(some),
					),
					true,
				)
			}
			("Option", "get_or_insert" | "get_or_insert_with" | "get_or_insert_default" | "insert") => {
				self.option_insert(&name, targ(0)?, args, dest, span, out)
			}
			// Indexing, whose result a `&mut` reads in place.
			("Index" | "IndexMut", "index" | "index_mut") => self.index(&name, gargs, args, dest, span, out),
			("str", "get" | "get_mut") => {
				let gargs = tcx.mk_args(&[tcx.types.str_.into(), targ(0)?.into()]);
				self.index(&name, gargs, args, dest, span, out)
			}
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
			// `rs.Place` lends, as one to a pinned box of an `Arc` does. An impl that
			// translates, such as a crate's own guard's, returns the `rs.Place` a `&mut` is.
			("DerefMut", "deref_mut")
				if let ty::Ref(_, pointee, _) = dest.ty(&self.mir.local_decls, tcx).ty.kind()
					&& (self.b.t.object_kind(*pointee).is_none()
						|| targ(0).is_some_and(|t| {
							matches!(self.b.t.std_of(t), Some(Std::MutexGuard | Std::RefMut))
								|| self.b.t.needs_place(t) && self.b.t.needs_place(*pointee)
						}))
					&& self.b.t.mapped(def_id, gargs)
					&& let Some(local) = dest.as_local()
					&& self.defs[local] == 1
					&& matches!(self.slots[local], Slot::Var(_)) =>
			{
				let result = dest.ty(&self.mir.local_decls, tcx).ty;
				// The receiver passes as what it points at, as std's `&mut self` does.
				let values = self.operands(false, &[true], args, result, span, out);
				let place = self.b.call_def(def_id, gargs, values, span, out);
				// A `&mut`, or a pin of one, lends the `rs.Place` it is, whose value is at `v`.
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
					Expr::rs("bytesSlice", std::iter::once(b).chain([start]).chain(end).collect()),
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
				done(Expr::rs("bytesSliceToInclusive", vec![b, end]), true)
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
						let checked = Expr::rs("boundsRange", vec![ranged.bounds.clone(), s.clone().member("length")]);
						let checked = ranged.after(checked);
						let pair = self.path(checked, "bounds", out);
						(pair.clone().index(Expr::lit("0")), pair.index(Expr::lit("1")))
					}
				};
				let dest = self.operand(&args[2].node, span, out);
				let mut values = vec![s, start, end, dest];
				let f = match elem.kind() {
					_ if crate::lower::byte(elem) => "byteSliceCopyWithin",
					_ => {
						values.extend(self.b.t.copy_fn(elem, span));
						"sliceCopyWithin"
					}
				};
				done(Expr::rs(f, values), true)
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
						return done(Expr::rs("fmtCount", vec![count]), true);
					}
					Err(construct) => {
						self.err(span, format!("{construct} is not supported"));
						return done(Expr::undefined(), false);
					}
				};
				// An array comes by reference, which is a slice of it.
				let ty = match targ(0)?.kind() {
					ty::Array(elem, _) => Ty::new_slice(tcx, *elem),
					_ => targ(0)?,
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
				// A reference to the arguments array, which is a slice of all of it.
				let values = library::slice_items(self.operand(&args[1].node, span, out));
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
								Some(spec) => Expr::rs("fmtSpec", vec![spec, value]),
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
				// A `Poll`'s `Pending` and `Ready(None)` are no `rs.Result`: they continue.
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
							Expr::rs("pollOptionBranch", vec![value, pending, self.b.t.none(ok)])
						}
						_ => Expr::rs("pollBranch", vec![value, self.b.t.pending(inner_args.type_at(0))]),
					};
					return done(value, false);
				}
				// A `next`'s item or `rs.DONE` branches as itself.
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
								if matches!(&*f, Expr::Rs(n) if n == "Err") && items.len() == 1 =>
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
				done(Expr::rs("debugListEntries", values), true)
			}
			_ => None,
		}
	}

	/// The place a `&mut` operand borrows, as an assignable path, and through the
	/// reference, which a store resolves: an alias of a borrowed place, not a copy of a
	/// reference, which holds the object it points at, or an `rs.Place`, whose `v` it is,
	/// or else the object it points at, written in place. A `[u8; N]`'s is a copy of its
	/// bytes instead, which the store writes in place. What it reads that is held back is
	/// emitted, as an edit reads it and then writes it, where MIR reads it once.
	fn borrowed_place(&mut self, op: &Operand<'tcx>, span: Span, out: &mut Vec<Stmt>) -> Option<Borrowed<'tcx>> {
		let (Operand::Copy(place) | Operand::Move(place)) = op else {
			return None;
		};
		if !place.projection.is_empty() {
			return None;
		}
		let deref = self.tcx.mk_place_deref(*place);
		if self.in_array(deref) {
			// Named before the copy, as the store names it.
			let slice = self.place_expr(deref, span, out);
			let slice = self.slice_path(slice, out);
			let ty = deref.ty(&self.mir.local_decls, self.tcx).ty;
			return Some((self.b.t.array_copy(slice, ty, span), Some(deref)));
		}
		let borrowed = match &self.slots[place.local] {
			Slot::Alias(Some(target)) if assignable(target) && self.alias_of_place(place.local) => {
				(target.clone(), Some(deref))
			}
			_ if self.b.t.place_pointee(self.mir.local_decls[place.local].ty).is_some() => {
				(self.place_expr(deref, span, out), Some(deref))
			}
			_ if self.in_object(deref) => {
				let target = self.place_expr(deref, span, out);
				(self.path(target, "object", out), Some(deref))
			}
			_ => {
				let pointee = self.mir.local_decls[place.local].ty.peel_refs();
				let err = self.err(span, format!("replacing the value behind a reference to `{pointee}`"));
				(err, None)
			}
		};
		self.materialize(&borrowed.0, out);
		Some(borrowed)
	}

	/// `slice::from_mut`, `array::from_mut`, or `from_mut` of `owner`, `Cell` or
	/// `UnsafeCell`, of `op`, a `&mut` to a `ty`: of the `rs.Place` it is or of the place it
	/// names, or else in place of the bytes or the object it is, which a cell lends as itself,
	/// as its `get` or `as_ptr` points at it, but a `Cell` of an object a pointer is the
	/// place of reads a copy of, as its writes replace the value it read, and a slice writes
	/// its element into. A slice of one byte no array holds is refused, as is any of an
	/// object JS can't write a whole value into.
	fn one_of(
		&mut self,
		owner: &str,
		ty: Ty<'tcx>,
		op: &Operand<'tcx>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Option<Expr> {
		let tcx = self.tcx;
		let place = op.place().filter(|p| p.projection.is_empty())?;
		let deref = tcx.mk_place_deref(place);
		// `array::from_mut` makes a `&mut [T; 1]`, which is a slice as a `&mut [T]` is.
		let of_place = match owner {
			"Cell" | "UnsafeCell" => "cellOfPlace",
			_ => "sliceOfPlace",
		};
		let of = |made: Expr| Expr::rs(of_place, vec![made]);
		let place_of = self.b.t.pointee(ty) == crate::lower::pointer::Pointee::Place;
		let of_object = |object: Expr| match owner {
			"Cell" if place_of => Expr::rs("cellIn", vec![object]),
			"Cell" | "UnsafeCell" => of(Expr::rs("partPlace", vec![object])),
			_ => Expr::rs("sliceIn", vec![object]),
		};
		let cell = owner != "raw" && owner != "array";
		if !cell && crate::lower::byte(ty) {
			return Some(self.err(
				span,
				"a slice of one byte's place, which a byte slice can view only in an array of bytes",
			));
		}
		if self.in_array(deref) {
			let slice = self.place_expr(deref, span, out);
			return Some(of_object(self.b.t.array_rs(ty, "View").call(vec![slice])));
		}
		// A cell of an object is of it wherever it is, as its place may be one a write lands
		// in, such as an element of a slice of one.
		if cell && self.b.t.write_in(ty).is_some() {
			let object = self.operand(op, span, out);
			return Some(of_object(object));
		}
		if let Slot::Alias(Some(target)) = &self.slots[place.local]
			&& assignable(target)
			&& self.alias_of_place(place.local)
		{
			let target = target.clone();
			let target = self.stable_target(deref, target, span, out);
			return Some(of(self.place_of(target, None)));
		}
		if self.b.t.needs_place(ty) {
			let made = self.operand(op, span, out);
			return Some(of(made));
		}
		if self.b.t.write_in(ty).is_some() {
			let object = self.operand(op, span, out);
			return Some(of_object(object));
		}
		Some(self.err(
			span,
			format!(
				"a slice or a cell of a mutable reference to `{ty}` that names no place, which JS can't write in place"
			),
		))
	}

	/// The place the `&mut` receiver borrows, as the `&mut` the call returns: an alias of it
	/// where the destination is read only as one.
	fn same_place(
		&mut self,
		args: &[Spanned<Operand<'tcx>>],
		dest: Place<'tcx>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Option<Option<(Expr, bool)>> {
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
				Some(Some((self.place_of(target, None), true)))
			}
			None => Some(Some((place, true))),
		}
	}

	/// A `fmt::Write` method of a `String`, a `Formatter`, or the crate's own writer, or of
	/// std's impl for a `&mut W`, which calls `W`'s, as [`Body::fmt_write`] writes it. A
	/// `String` a local holds gets its longer value where it is. `None` for any other writer.
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
		if std == Some(Std::String) {
			let s = match through {
				Some(value) => value,
				None => self.operand(&args[0].node, span, out),
			};
			let [arg] = <[Expr; 1]>::try_from(self.operands(true, &[], &args[1..], result, span, out)).ok()?;
			let mut written = Vec::new();
			let result = self.b.string_write(name, s, arg, literal, &mut written);
			self.emit(written, false, out);
			return done(result, true);
		}
		let receiver = match through {
			Some(value) => value,
			None => self.operands(true, &[], &args[..1], result, span, out).pop()?,
		};
		let [arg] = <[Expr; 1]>::try_from(self.operands(true, &[], &args[1..], result, span, out)).ok()?;
		// The provided `write_fmt` reads it in each function it writes through.
		let receiver = match self.b.t.translated(def_id, wargs) || std.is_some() {
			true => receiver,
			false => self.path(receiver, "w", out),
		};
		done(self.b.fmt_write(def_id, w, [receiver, arg], literal, span, out)?, true)
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

	/// The type `clone_from` or `clone_into`, `name`, of `args` writes over: what its `&mut`
	/// borrows, a `Vec` where a slice's `clone_into` writes one.
	fn cloned_over(&self, name: &str, args: &[Spanned<Operand<'tcx>>]) -> Option<Ty<'tcx>> {
		let target = match name {
			"clone_from" => &args[0].node,
			_ => &args[1].node,
		};
		target.ty(&self.mir.local_decls, self.tcx).builtin_deref(true)
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
								Expr::rs("callOnce", vec![f])
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
		// A reference to an array is a slice of it.
		if payload_ty.is_array() {
			return Some(Some((self.b.t.array_rs(payload_ty, "From").call(vec![read]), true)));
		}
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
	/// `rs.Range`'s generic steps, into the item alone and a flag for whether there was
	/// one: a number that is sometimes `rs.DONE` is slower in V8. Past the end, `name` is
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
		// The object written in place gives what it held as an object of its own, as does
		// the array a slice of other values than bytes holds.
		let array = place.1.filter(|&p| self.in_array(p) && !self.in_bytes(p));
		if place.1.is_some_and(|p| self.in_object(p)) || array.is_some() {
			let object = match array {
				Some(p) => {
					let slice = self.place_expr(p, span, out);
					Expr::rs("sliceView", vec![slice])
				}
				None => place.0,
			};
			let mut made = vec![object, value];
			made.extend(self.b.t.copy_depth(ty));
			let old = Expr::rs("replaceIn", made);
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
		if let Some((slice, index)) = self.element_in(&place, ty) {
			let mut made = vec![slice, index, value];
			made.extend(self.b.t.copy_depth(ty));
			let old = Expr::rs("sliceReplace", made);
			out.push(StmtKind::decl(false, name.clone(), None, Some(old)).at(None));
			return Expr::ident(name);
		}
		// TypeScript widens `rs.None` read from a place narrowed to it into a `symbol`.
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

	/// The slice and the index of `place`, an element of objects of what a reference
	/// points at, which may be a slice of one object that `slice::from_mut` makes, whose
	/// whole write lands in the object, not its slot.
	fn element_in(&self, (read, place): &Borrowed<'tcx>, ty: Ty<'tcx>) -> Option<(Expr, Expr)> {
		if self.b.t.write_in(ty).is_none_or(|kind| kind == ObjectKind::Bytes) {
			return None;
		}
		let target = self.through_aliases((*place)?);
		let [
			base @ ..,
			ProjectionElem::Deref,
			ProjectionElem::Index(_) | ProjectionElem::ConstantIndex { .. },
		] = target.projection.as_slice()
		else {
			return None;
		};
		if !Place::ty_from(target.local, base, &self.mir.local_decls, self.tcx)
			.ty
			.is_ref()
		{
			return None;
		}
		let mut read = read;
		while let Expr::Located(_, inner) = read {
			read = inner;
		}
		let Expr::Index(slice, index) = read else {
			unreachable!("an element's path is an index: {read:?}")
		};
		Some(((**slice).clone(), (**index).clone()))
	}

	/// Set what a `&mut` borrows to `value`.
	fn set_borrowed(&mut self, (read, place): &Borrowed<'tcx>, value: Expr, span: Span, out: &mut Vec<Stmt>) {
		match place {
			Some(place) => out.extend(self.store(*place, value, span)),
			None => self.assign_to(read.clone(), value, None, out),
		}
	}

	/// `container[index]`, or a slice of it by a range, which a reference reads in place; or
	/// `get` or `get_mut` of a slice or a `str` by a range.
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
		// A string slices by UTF-8 byte offsets, which a JS string doesn't index by, and a
		// part a `&mut` borrows is a place of its own, which splices the whole anew.
		if container.is_str() || self.b.t.std_of(container) == Some(Std::String) {
			let get = matches!(name, "get" | "get_mut");
			let base = self.operand(&args[0].node, span, out);
			// A `String` is the place of its text.
			let base = match self.b.t.std_of(container) == Some(Std::String) && !matches!(name, "index_mut" | "get_mut")
			{
				true => base.member("v"),
				false => base,
			};
			let (base, (start, end)) = match self.range_bounds(&args[1].node, index_ty, span, out) {
				Some(bounds) if get => (base, bounds),
				Some(bounds) => (self.path(base, "s", out), bounds),
				// Any other range gives its byte offsets, checked as std checks its own kind.
				None if crate::lower::collections::plain_bounds(self.b.t, index_ty) => {
					let base = self.path(base, "s", out);
					let range = self.operand(&args[1].node, span, out);
					let text = match name {
						"index_mut" | "get_mut" => base.clone().member("v"),
						_ => base.clone(),
					};
					let len = || Expr::rs("stringLen", vec![text.clone()]);
					let checked = match (get, self.b.t.std_of(index_ty)) {
						(true, _) => Expr::rs("sliceGetBounds", vec![range, len()]),
						(_, Some(Std::RangeInclusive | Std::RangeToInclusive)) => {
							Expr::rs("strInclusive", vec![text.clone(), range])
						}
						_ => Expr::rs("boundsRange", vec![range, len()]),
					};
					let pair = self.path(checked, "bounds", out);
					(
						base,
						(pair.clone().index(Expr::lit("0")), Some(pair.index(Expr::lit("1")))),
					)
				}
				None => return None,
			};
			let f = match name {
				"index_mut" => "strPart",
				"get" => "strGet",
				"get_mut" => "strGetMut",
				_ => "strSlice",
			};
			let mut values = vec![base, start];
			values.extend(end);
			return Some(Some((Expr::rs(f, values), true)));
		}
		let elem_ty = elem(self.tcx, container)?;
		let bytes = crate::lower::byte(elem_ty);
		let base = self.operand(&args[0].node, span, out);
		match Int::of(index_ty) {
			Some(_) => {
				// A `Vec`'s element is its storage's: its `items`, or a byte vector's `data`.
				let base = match container.kind() {
					ty::Adt(..) if !bytes => base.member("items"),
					_ => base,
				};
				let base = self.path(base, "slice", out);
				let i = self.operand(&args[1].node, span, out);
				let i = self.path(i, "index", out);
				let checked = Expr::rs("idx", vec![base.clone(), i]);
				let checked = self.path(checked, "at", out);
				let place = match container.kind() {
					ty::Adt(..) if bytes => base.member("data").index(checked),
					// An array comes through a reference, which is a slice of it.
					_ if container.is_slice() || container.is_array() => super::place::slice_at(base, checked),
					_ => base.index(checked),
				};
				// A reference to an array is a slice of it.
				if elem_ty.is_array() {
					return Some(Some((self.b.t.array_rs(elem_ty, "From").call(vec![place]), true)));
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
				// The range's bounds are taken apart before the slice is read, while they are
				// still held back. Any other range, as an `rs.Bounds`, is checked by the
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
				// All of a slice of other values is the slice itself.
				let full =
					matches!(index_ty.kind(), ty::Adt(adt, _) if self.tcx.item_name(adt.did()).as_str() == "RangeFull");
				if full && !bytes {
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
						let pair = self.path(Expr::rs(checked, vec![range, length]), "bounds", out);
						(pair.clone().index(Expr::lit("0")), Some(pair.index(Expr::lit("1"))))
					}
				};
				let mut values = vec![base, start];
				values.extend(end);
				// A range is a slice of the same storage, shared or lent mutably alike.
				let range = match (bytes, get) {
					(true, false) => "byteSliceRange",
					(true, true) => "byteSliceGetRange",
					(false, false) => "sliceRange",
					(false, true) => "sliceGetRange",
				};
				Some(Some((Expr::rs(range, values), true)))
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
	/// whose items the runtime drops if it owns them, or else an `rs.Iter`, which it drops
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

/// The test that `scrut`, a `ControlFlow`, is `Continue` (variant 0) or `Break`, where
/// `next` says it is what `?` on a `next` result branches to, which holds the same.
pub fn control_flow_test<'tcx>(
	fx: &mut Fx<'_, '_, '_, 'tcx>,
	scrut: &Expr,
	ty: Ty<'tcx>,
	variant: VariantIdx,
	next: bool,
) -> Expr {
	let continues = variant.index() == 0;
	if next {
		return match continues {
			true => scrut.clone().binary("!==", Expr::Rs("DONE".into())),
			false => scrut.clone().binary("===", Expr::Rs("DONE".into())),
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

/// Whether an assignment can write `e`: a path, an element, or a member of one, through a
/// cast, as a generic type's field is read as its instance holds it.
fn assignable(e: &Expr) -> bool {
	match e {
		_ if super::place::split_target(e).is_some() => true,
		Expr::Index(..) => true,
		Expr::Member(base, _) | Expr::Located(_, base) | Expr::As(base, _) => assignable(base),
		e => e.is_path(),
	}
}

/// Locate each of `arguments` at the first argument of a call or `new` in `e` it is, and
/// drop it from the list.
fn locate(e: &mut Expr, arguments: &mut Vec<(Expr, ts::Loc)>) {
	if arguments.is_empty() {
		return;
	}
	match e {
		Expr::Call(callee, args) | Expr::New(callee, args) => {
			locate(callee, arguments);
			for arg in args {
				match arguments.iter().position(|(value, _)| value == arg) {
					Some(i) => {
						let (_, loc) = arguments.remove(i);
						*arg = Expr::Located(loc, Box::new(std::mem::replace(arg, Expr::undefined())));
					}
					None => locate(arg, arguments),
				}
			}
		}
		Expr::Member(e, _)
		| Expr::Unary(_, e)
		| Expr::Postfix(e, _)
		| Expr::As(e, _)
		| Expr::Instantiate(e, _)
		| Expr::Lambda(_, e)
		| Expr::Located(_, e) => locate(e, arguments),
		Expr::Index(a, b) | Expr::Binary(a, _, b) | Expr::Assign(a, b) => {
			locate(a, arguments);
			locate(b, arguments);
		}
		Expr::Cond(a, b, c) => {
			locate(a, arguments);
			locate(b, arguments);
			locate(c, arguments);
		}
		Expr::Array(es) => es.iter_mut().for_each(|e| locate(e, arguments)),
		Expr::Object(props) => props.iter_mut().for_each(|(_, e)| locate(e, arguments)),
		_ => {}
	}
}
