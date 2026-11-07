//! The standard library's ownership and async types, as `rs/task.ts` gives them: `Arc`,
//! `Rc`, `Weak`, `Mutex`, atomics, `Waker`, `Context`, `Poll`, `Pin`, and std's futures.

use rustc_abi::VariantIdx;
use rustc_middle::mir::{BinOp, UnOp};
use rustc_middle::ty::{self, Instance, Ty};
use rustc_span::{Span, Symbol};

use super::Translator;
use super::body::Body;
use super::library::{self, Call, Std};
use super::num::{self, Int, Operand};
use crate::ts::{Expr, Key, Param, Stmt};

impl<'tcx> Translator<'_, 'tcx> {
	/// `Future::Output` of `ty`, if it is a `dyn Future`, whose JS value is an `rs.Future`.
	pub fn dyn_future(&self, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
		let ty::Dynamic(preds, ..) = ty.kind() else {
			return None;
		};
		if preds.principal_def_id()? != self.tcx.lang_items().future_trait()? {
			return None;
		}
		let output = preds.projection_bounds().next()?;
		self.tcx.instantiate_bound_regions_with_erased(output).term.as_type()
	}

	/// `Future::Output` of `ty`, a future.
	pub fn future_output(&mut self, ty: Ty<'tcx>, span: Span) -> Ty<'tcx> {
		let tcx = self.tcx;
		let output = tcx.lang_items().future_output().and_then(|item| {
			let projection = Ty::new_projection(tcx, ty::IsRigid::No, item, [ty]);
			tcx.try_normalize_erasing_regions(self.typing_env_pub(), ty::Unnormalized::new_wip(projection))
				.ok()
		});
		output.unwrap_or_else(|| {
			self.err(span, format!("`{ty}` is not a future"));
			tcx.types.unit
		})
	}
}

impl<'tcx> Body<'_, '_, 'tcx> {
	/// `value`, a `from` coerced to `to`, which points at a `dyn Future` through
	/// references, boxes, and pins, whose JS value is what they point at, or an `rs.Place`
	/// of it for a `&mut` to a reference.
	pub fn dyn_future(&mut self, from: Ty<'tcx>, to: Ty<'tcx>, value: Expr, span: Span) -> Expr {
		let tcx = self.t.tcx;
		let (mut pointee, mut target, mut borrowed, mut value) = (from, to, false, value);
		while !matches!(target.kind(), ty::Dynamic(..)) {
			(pointee, target) = match (pointee.kind(), target.kind()) {
				(ty::Ref(_, a, _), ty::Ref(_, b, _)) => {
					borrowed = true;
					value = self.t.pointee_value(pointee, value);
					(*a, *b)
				}
				(ty::Adt(a, a_args), ty::Adt(b, b_args))
					if a.did() == b.did() && matches!(Std::of(tcx, a.did()), Some(Std::Box | Std::Pin)) =>
				{
					(a_args.type_at(0), b_args.type_at(0))
				}
				// Another pointer, such as an `Arc`, is a value of its own around the future.
				_ => return self.err(span, format!("a coercion to `{to}`")),
			};
		}
		// What drops `to` drops the future, unless it was borrowed.
		let dropped = self.t.drops(to);
		self.future_object(pointee, value, borrowed, dropped, span)
	}

	/// `value`, a `from` that is a future, as an `rs.Future`: itself if it is one already,
	/// and else an `rs.DynFuture` over its `Future` impl, which drops it with itself unless
	/// `borrowed`. A borrowed one that is `dropped` must not drop what it borrows.
	pub fn future_object(&mut self, from: Ty<'tcx>, value: Expr, borrowed: bool, dropped: bool, span: Span) -> Expr {
		let tcx = self.t.tcx;
		let object = match from.kind() {
			ty::Dynamic(..) | ty::Coroutine(..) => true,
			// What a reference or a box points at, which its JS value is, or an `rs.Place` of.
			ty::Ref(_, inner, _) => {
				let value = self.t.pointee_value(from, value);
				return self.future_object(*inner, value, true, dropped, span);
			}
			ty::Adt(adt, args) => match Std::of(tcx, adt.did()) {
				Some(Std::PollFn | Std::Pending | Std::Ready) => true,
				Some(Std::Box | Std::Pin) => {
					return self.future_object(args.type_at(0), value, borrowed, dropped, span);
				}
				_ => false,
			},
			_ => false,
		};
		if object {
			return match borrowed && dropped {
				true => Expr::New(
					Box::new(Expr::Rs("DynFuture".into())),
					vec![value, Expr::Rs("futurePoll".into())],
				),
				false => value,
			};
		}
		// What `Future::poll` takes would be an `rs.Place`, which the box doesn't have.
		if self.t.needs_place(from) {
			return self.err(span, format!("a `dyn Future` of `{from}`, which JS copies"));
		}
		let future = tcx.lang_items().future_trait().expect("a dyn Future names the trait");
		let poll = tcx
			.associated_items(future)
			.filter_by_name_unhygienic(Symbol::intern("poll"))
			.next()
			.expect("Future::poll")
			.def_id;
		let mut args = vec![value, self.fn_value(poll, tcx.mk_args(&[from.into()]), span)];
		if !borrowed {
			args.extend(self.t.drop_fn(from, span));
		}
		Expr::New(Box::new(Expr::Rs("DynFuture".into())), args)
	}
}

/// A call of a standard library function on these types, or its arguments back if it is
/// none of theirs. `Ok(None)` refuses it.
pub fn call<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	call: &Call<'tcx>,
	owner: &str,
	name: &str,
	args: Vec<Expr>,
	out: &mut Vec<Stmt>,
) -> Result<Option<Expr>, Vec<Expr>> {
	let tcx = b.t.tcx;
	let std_of = |ty: Ty<'tcx>| match ty.peel_refs().kind() {
		ty::Adt(adt, _) => Std::of(tcx, adt.did()),
		_ => None,
	};
	let self_ty = call.args.types().next();
	let self_std = self_ty.and_then(std_of);
	// A clone of a reference is that reference, not a clone of what it points at.
	let owned_std = self_std.filter(|_| self_ty.is_some_and(|t| !t.is_ref()));
	let e = match (owner, name) {
		("Arc" | "Rc", "new") => Expr::New(Box::new(Expr::Rs("Arc".into())), args),
		("Arc" | "Rc", "new_cyclic") => Expr::rs("arcNewCyclic", args),
		// JS erases the pin.
		("Arc" | "Rc", "pin") => Expr::New(Box::new(Expr::Rs("Arc".into())), args),
		// The object of one of a trait object, which every pointer to it shares. A `dyn Any`'s
		// may hold a copy of a value JS copies, which `Arc::downcast` would not see written.
		("Arc" | "Rc", "get_mut")
			if call.args.type_at(0).is_trait() && !super::dynamic::is_any_object(tcx, call.args.type_at(0)) =>
		{
			Expr::rs("arcGetMut", args)
		}
		// A `&mut` to the value, which is where it is for one JS copies. `make_mut` takes the
		// `Arc`'s place, which it may point at a new `Arc` of a clone, cloned as a box of a
		// slice or text is.
		("Arc" | "Rc", "get_mut" | "make_mut")
			if call.args.type_at(0).is_sized(tcx, b.t.typing_env_pub())
				|| shared_box(tcx, call.args.type_at(0)).is_some() =>
		{
			let value = call.args.type_at(0);
			let place = if b.t.needs_place(value) { "Place" } else { "" };
			match name {
				"get_mut" => Expr::rs(format!("arcGetMut{place}"), args),
				_ => {
					let cloned = shared_box(tcx, value).unwrap_or(value);
					let clone = super::collections::clone_fn(b, cloned, call.span);
					let arc = b.t.sig(call.def_id, call.args).inputs()[0].peel_refs();
					let Some(droppers) = b.t.droppers([arc], call.span) else {
						return Ok(Some(Expr::undefined()));
					};
					// What moves the value to a new allocation, as std's does when only `Weak`s
					// share it: a new object where pointers to it compare.
					let moved = b.t.held_addressed(value).then(|| b.t.shim(value, call.span)).flatten();
					let values = args.into_iter().chain([clone]).chain(droppers).chain(moved).collect();
					Expr::rs(format!("arcMakeMut{place}"), values)
				}
			}
		}
		// An `Arc<[u8]>` of an `Arc<str>`'s bytes, which shares its count, as std's shares its
		// allocation.
		("From", "from")
			if let [to, from] = call.args.as_slice()
				&& let (ty::Adt(_, to), ty::Adt(_, from)) = (to.expect_ty().kind(), from.expect_ty().kind())
				&& self_std == Some(Std::Arc)
				&& std_of(call.args.type_at(1)) == Some(Std::Arc)
				&& matches!(to.type_at(0).kind(), ty::Slice(e) if matches!(e.kind(), ty::Uint(ty::UintTy::U8)))
				&& from.type_at(0).is_str() =>
		{
			Expr::rs("arcStrBytes", args)
		}
		// An `Arc` of a slice or text, of the box `From` makes of the same value, but for the
		// `Arc` itself.
		("From", "from")
			if let [arc, from] = call.args.as_slice()
				&& self_std == Some(Std::Arc)
				&& std_of(from.expect_ty()) != Some(Std::Arc)
				&& let ty::Adt(_, arc) = arc.expect_ty().kind()
				&& let Some(boxed) = shared_box(tcx, arc.type_at(0)) =>
		{
			let value = b.convert(first(args), from.expect_ty(), boxed, call.span, out);
			Expr::New(Box::new(Expr::Rs("Arc".into())), vec![value])
		}
		// A waker, or its raw parts, over the crate's `Wake` impl.
		("From", "from")
			if let [_, from] = call.args.as_slice()
				&& let Some(to @ (Std::Waker | Std::RawWaker)) = self_std
				&& std_of(from.expect_ty()) == Some(Std::Arc) =>
		{
			arc_waker(b, from.expect_ty(), first(args), to == Std::RawWaker, call.span)
		}
		("Waker", "noop") => Expr::Rs("NOOP_WAKER".into()),
		("Waker", "new") => Expr::New(Box::new(Expr::Rs("VTableWaker".into())), args),
		("Waker", "from_raw") => Expr::rs("wakerFromRaw", args),
		("RawWaker", "new") => {
			let [data, vtable] = two(args);
			raw_waker(data, vtable)
		}
		("RawWakerVTable", "new") => Expr::rs("wakerVTable", args),
		("AsRef", "as_ref") if self_std == Some(Std::Arc) => first(args).member("value"),
		("Arc" | "Rc", "downgrade") | ("Weak", "new") => Expr::New(Box::new(Expr::Rs("Weak".into())), args),
		// The value of the last strong one, which its count says, and else its `None`.
		("Arc" | "Rc", "into_inner") => {
			let none = b.t.none_arg(call.args.type_at(0));
			Expr::rs("arcIntoInner", args.into_iter().chain(none).collect())
		}
		("Arc" | "Rc", "strong_count") => first(args).member("strong"),
		("Arc" | "Rc", "weak_count") => first(args).member("weak"),
		// Two `Arc`s of trait objects, of structs of them, or of slices are views of the
		// `Arc`s they were made from, of the array where it is a slice of one.
		("Arc" | "Rc", "ptr_eq") if self_ty.is_some_and(|t| t.is_slice() || super::dynamic::unsized_object(tcx, t)) => {
			Expr::rs("arcPtrEq", args)
		}
		("Arc" | "Rc", "ptr_eq") => {
			let [l, r] = two(args);
			l.binary("===", r)
		}
		("Weak", "upgrade") => Expr::rs("weakUpgrade", args),
		("Weak", "strong_count") => first(args).member("arc").member("strong"),
		("Weak", "weak_count") => Expr::rs("weakWeakCount", args),
		("Weak", "ptr_eq") if self_ty.is_some_and(|t| t.is_slice() || super::dynamic::unsized_object(tcx, t)) => {
			let [l, r] = two(args);
			Expr::rs("arcPtrEq", vec![l.member("arc"), r.member("arc")])
		}
		("Weak", "ptr_eq") => {
			let [l, r] = two(args);
			l.member("arc").binary("===", r.member("arc"))
		}
		("Clone", "clone") if owned_std == Some(Std::Arc) => Expr::rs("arcClone", args),
		("Clone", "clone") if owned_std == Some(Std::Weak) => Expr::rs("weakClone", args),
		// A `RefCell`'s clone borrows it, and a one-time cell's clones what it holds, if anything.
		("Clone", "clone") if matches!(owned_std, Some(Std::RefCell | Std::OnceCell | Std::OnceLock)) => {
			let inner = held(self_ty);
			let clone = super::collections::clone_fn(b, inner, call.span);
			match owned_std {
				Some(Std::RefCell) => Expr::rs("refCellClone", vec![first(args), clone]),
				_ => {
					let none = b.t.none_arg(inner);
					Expr::rs("onceClone", [first(args), clone].into_iter().chain(none).collect())
				}
			}
		}
		// A `Ready`'s clones its value, if it still holds it, and every `Pending` is one.
		("Clone", "clone") if owned_std == Some(Std::Ready) => {
			let clone = super::collections::clone_fn(b, held(self_ty), call.span);
			let clone = (clone != Expr::undefined()).then_some(clone);
			first(args).member("clone").call(clone.into_iter().collect())
		}
		("Clone", "clone") if owned_std == Some(Std::Pending) => Expr::Rs("NEVER".into()),
		// A waker is the executor's, whose `clone` is its own.
		("Clone", "clone") if owned_std == Some(Std::Waker) => first(args).member("clone").call(vec![]),
		// A reference is the object it points at, which its own `Deref` gives: read out of a
		// `&mut` that is an `rs.Place`, whose place `DerefMut` lends on.
		("Deref", "deref") if let Some(ty) = self_ty.filter(|t| t.is_ref()) => b.t.pointee_value(ty, first(args)),
		("DerefMut", "deref_mut") if self_ty.is_some_and(|t| t.is_ref()) => first(args),
		("Deref", "deref") | ("DerefMut", "deref_mut") => match self_std {
			Some(Std::Arc | Std::Mutex | Std::MutexGuard | Std::Ref | Std::RefMut) => first(args).member("value"),
			Some(Std::LazyCell | Std::LazyLock) if name == "deref" => first(args).member("force").call(vec![]),
			// A pin is its pointer, which derefs as it does: an `Arc` to the value it holds, and
			// a reference to what it points at.
			Some(Std::Pin) => match self_ty.map(|t| t.kind()) {
				Some(ty::Adt(_, pin)) => match pin.type_at(0).kind() {
					ty::Adt(arc, _) if Std::of(tcx, arc.did()) == Some(Std::Arc) => first(args).member("value"),
					ty::Ref(..) if name == "deref" => b.t.pointee_value(pin.type_at(0), first(args)),
					_ => first(args),
				},
				_ => first(args),
			},
			_ => return Err(args),
		},
		("Mutex", "new") => Expr::New(Box::new(Expr::Rs("Mutex".into())), args),
		// The mutex is its own guard, and a panic aborts, so nothing poisons it.
		("Mutex", "lock") => Expr::result(true, Expr::rs("mutexLock", args)),
		("Mutex", "try_lock") => Expr::rs("mutexTryLock", args),
		// What it holds, lent as where it is where JS copies it.
		("Mutex" | "RwLock", "get_mut") => Expr::result(true, lend_value(b, call, first(args))),
		("RefCell", "get_mut") => lend_value(b, call, first(args)),
		("Mutex" | "RwLock", "into_inner") => Expr::result(true, first(args).member("value")),
		("RefCell", "into_inner") => first(args).member("value"),
		// A `PoisonError` is what it holds.
		("PoisonError", "into_inner" | "get_ref" | "get_mut") => first(args),
		// Mapped rather than lowered from core's MIR, so one of a lock unwraps its guard in place.
		("Result", "unwrap_or_else") if call.args.types().nth(1).and_then(std_of) == Some(Std::PoisonError) => {
			Expr::rs("resultUnwrapOrElse", args)
		}
		// A cell, or a lock, is its own guard.
		("RefCell" | "RwLock", "new") => Expr::New(Box::new(Expr::Rs("RefCell".into())), args),
		("RefCell", "borrow") => Expr::rs("refCellBorrow", args),
		("RefCell", "borrow_mut") => Expr::rs("refCellBorrowMut", args),
		("RefCell", "try_borrow") => Expr::rs("refCellTryBorrow", args),
		("RefCell", "try_borrow_mut") => Expr::rs("refCellTryBorrowMut", args),
		// A leaked guard lends what it holds and keeps its borrow until `undo_leak`.
		("Ref", "leak") => first(args).member("value"),
		("RefMut", "leak") => lend_value(b, call, first(args)),
		("RefCell", "undo_leak") => lend_value(b, call, Expr::rs("refCellUndoLeak", args)),
		("Ref" | "RwLockReadGuard" | "MappedRwLockReadGuard", "map" | "filter_map" | "try_map" | "map_split") => {
			guard_map(b, call, name, "ref", args, out)
		}
		("RefMut" | "RwLockWriteGuard" | "MappedRwLockWriteGuard", "map" | "filter_map" | "try_map" | "map_split") => {
			guard_map(b, call, name, "refMut", args, out)
		}
		("MutexGuard" | "MappedMutexGuard", "map" | "filter_map") => guard_map(b, call, name, "mutex", args, out),
		("Ref", "clone") => Expr::rs("refClone", args),
		("RefCell", "replace") => Expr::rs("refCellReplace", args),
		// With the value's place where JS would copy it.
		("RefCell", "replace_with") => {
			let copied = b.t.needs_place(call.args.type_at(0));
			Expr::rs(
				"refCellReplaceWith",
				super::iter::tail(args, vec![Expr::lit(copied.to_string())]),
			)
		}
		("RefCell", "take") => {
			let ty = call.args.type_at(0);
			let f = super::collections::trait_fn(tcx, "Default", "default");
			let value = b.call_def(f, tcx.mk_args(&[ty.into()]), Vec::new(), call.span, out);
			Expr::rs("refCellReplace", vec![first(args), value])
		}
		// A panic aborts, so nothing poisons it.
		("RwLock", "read") => Expr::result(true, Expr::rs("rwLockRead", args)),
		("RwLock", "write") => Expr::result(true, Expr::rs("rwLockWrite", args)),
		("RwLock", "try_read") => Expr::rs("rwLockTryRead", args),
		("RwLock", "try_write") => Expr::rs("rwLockTryWrite", args),
		("RwLockWriteGuard", "downgrade") => Expr::rs("rwLockDowngrade", args),
		("Cell", _) => return cell(b, call, name, args, out),
		// A thread-local's value, which is never destroyed.
		("LocalKey", "with") => {
			let [key, f] = two(args);
			f.call(vec![key.member("get").call(Vec::new())])
		}
		("LocalKey", "try_with") => {
			let [key, f] = two(args);
			Expr::result(true, f.call(vec![key.member("get").call(Vec::new())]))
		}
		// With what drops the value it replaces.
		("LocalKey", "set") => {
			let Some(droppers) = b.t.droppers([call.args.type_at(0)], call.span) else {
				return Ok(Some(Expr::undefined()));
			};
			Expr::rs("localKeyRefCellSet", args.into_iter().chain(droppers).collect())
		}
		// With the value's place where JS would copy it.
		("LocalKey", "with_borrow_mut") => match b.t.needs_place(call.args.type_at(0)) {
			true => Expr::rs("localKeyWithBorrowMutPlace", args),
			false => Expr::rs("localKeyWithBorrowMut", args),
		},
		// With the `Option`'s `None` where it is `rs.None`.
		("LocalKey", "initialize_with") => {
			let none = b.t.none_arg(call.args.type_at(0));
			Expr::rs("localKeyInitializeWith", args.into_iter().chain(none).collect())
		}
		("Atomic", _) => return atomic(b, call, name, args),
		("LazyCell" | "LazyLock", "force") => first(args).member("force").call(vec![]),
		("LazyCell" | "LazyLock", "new") => Expr::New(Box::new(Expr::Rs(owner.into())), args),
		("OnceCell" | "OnceLock", _) => return once(b, call, name, args),
		// A `OnceLock` of `true`, which nothing poisons, as a panic aborts.
		("Once", "new") => once_unrun(),
		("Once", "is_completed") => first(args).member("v").binary("===", Expr::lit("true")),
		// With what drops `f`, as `discards` says, uncalled if the `Once` has run.
		("Once", "call_once" | "call_once_force") => {
			let Some(droppers) = b.t.droppers([call.args.type_at(0)], call.span) else {
				return Ok(Some(Expr::undefined()));
			};
			Expr::rs("onceCall", args.into_iter().chain(droppers).collect())
		}
		("OnceState", "is_poisoned") => Expr::rs("onceIsPoisoned", args),
		("From", "from") if matches!(self_std, Some(Std::OnceCell | Std::OnceLock)) => {
			let cell = self_ty.expect("a `From` impl's type");
			Expr::New(Box::new(Expr::Rs(once_class(b, cell).into())), args)
		}
		("atomic", "fence" | "compiler_fence") => Expr::rs("fence", args),
		// Scheduler hints, which promise nothing observable. A spin loop waiting on another
		// thread hangs on one thread natively too.
		("hint", "spin_loop") => Expr::undefined(),
		(_, "yield_now")
			if library::std_path(tcx, call.def_id)
				.first()
				.is_some_and(|m| m == "thread") =>
		{
			Expr::undefined()
		}
		// A panic aborts, so a thread is panicking only while its hook runs.
		(_, "panicking")
			if library::std_path(tcx, call.def_id)
				.first()
				.is_some_and(|m| m == "thread") =>
		{
			match b.t.hooked {
				true => Expr::rs("panicking", Vec::new()),
				false => Expr::lit("false"),
			}
		}
		("Waker", "wake_by_ref") => first(args).member("wakeByRef").call(vec![]),
		("Waker", "wake") => first(args).member("wake").call(vec![]),
		("Waker", "will_wake") => Expr::rs("willWake", args),
		// Its identity: what a pointer to `()` stands for, whatever the waker's JS type.
		("Waker", "data") => {
			let ty = b.t.sig(call.def_id, call.args).output();
			Expr::As(Box::new(first(args).member("data")), b.t.ty(ty, call.span))
		}
		("Waker", "vtable") => first(args).member("vtable"),
		// With what drops its function, unless that is borrowed.
		("poll_fn", "poll_fn") => {
			let dropper = b.t.drop_fn(call.args.type_at(1), call.span);
			Expr::New(
				Box::new(Expr::Rs("PollFn".into())),
				args.into_iter().chain(dropper).collect(),
			)
		}
		("pending", "pending") => Expr::Rs("NEVER".into()),
		// With what drops its value, unless a poll takes it first.
		("ready", "ready") => {
			let dropper = b.t.drop_fn(call.args.type_at(0), call.span);
			Expr::New(
				Box::new(Expr::Rs("Ready".into())),
				args.into_iter().chain(dropper).collect(),
			)
		}
		("Ready", "into_inner") => first(args).member("intoInner").call(vec![]),
		// A box's JS value is what it holds.
		("Box", "pin") => first(args),
		("Future", "poll") => match self_ty.map(Ty::kind) {
			// A `&mut` polls what it points at, whose pinned `&mut` is an `rs.Place` of it.
			Some(ty::Ref(_, inner, _)) => {
				let [future, cx] = two(args);
				let args = vec![future.member("v"), cx];
				b.call_def(call.def_id, tcx.mk_args(&[(*inner).into()]), args, call.span, out)
			}
			// std's futures and a `dyn Future` are `rs.Future`s, which poll themselves.
			_ if matches!(self_std, Some(Std::PollFn | Std::Pending | Std::Ready))
				|| self_ty.is_some_and(|ty| b.t.dyn_future(ty).is_some()) =>
			{
				let [future, cx] = two(args);
				future.member("poll").call(vec![cx])
			}
			// A pinned pointer, a box, or an `AssertUnwindSafe` polls what it points at or holds,
			// which its JS value is.
			Some(ty::Adt(adt, adt_args))
				if matches!(
					Std::of(tcx, adt.did()),
					Some(Std::Pin | Std::Box | Std::AssertUnwindSafe)
				) =>
			{
				let inner = match Std::of(tcx, adt.did()) {
					Some(Std::Pin) => adt_args.type_at(0).builtin_deref(true),
					_ => Some(adt_args.type_at(0)),
				};
				let Some(inner) = inner else { return Err(args) };
				b.call_def(call.def_id, tcx.mk_args(&[inner.into()]), args, call.span, out)
			}
			_ => return Err(args),
		},
		("Context", "waker" | "from_waker")
		| ("Pin", "new" | "new_unchecked" | "as_mut" | "get_mut" | "get_unchecked_mut")
		| ("pin", "pin_new_unchecked_in_helper")
		| ("future", "get_context")
		| ("IntoFuture", "into_future") => first(args),
		("mem", "drop") => {
			let ty = call.args.type_at(0);
			let value = first(args);
			match b.t.drop_value(value.clone(), ty, call.span) {
				Some(dropped) => dropped,
				None if value.is_path() => Expr::undefined(),
				None => value,
			}
		}
		_ => return Err(args),
	};
	Ok(Some(e))
}

/// An `Atomic<T>` method: an `rs.Atomic`'s, with its orderings, which the runtime checks.
/// `Ok(None)` refuses it.
fn atomic<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	call: &Call<'tcx>,
	name: &str,
	args: Vec<Expr>,
) -> Result<Option<Expr>, Vec<Expr>> {
	let tcx = b.t.tcx;
	let imp = tcx.inherent_impl_of_assoc(call.def_id).expect("an inherent method");
	let ty::Adt(_, impl_args) = tcx.type_of(imp).instantiate_identity().skip_norm_wip().kind() else {
		unreachable!("an atomic is a struct")
	};
	let ty = impl_args.type_at(0);
	let int = Int::of(ty);
	let fetch = |f: Expr, mut args: Vec<Expr>| {
		args.insert(1, f);
		Expr::rs("atomicFetch", args)
	};
	// A pointer compares as `ptr::eq` does only where it is the object it points at, or
	// a slice of the array it points at, which `rs.sliceSame` compares.
	let generics = tcx.generics_of(imp).count();
	let atomic = tcx
		.type_of(imp)
		.instantiate(tcx, &call.args[..generics])
		.skip_norm_wip();
	let pointee = match atomic.kind() {
		ty::Adt(_, atomic_args) => match atomic_args.type_at(0).kind() {
			ty::RawPtr(pointee, _) => Some(*pointee),
			_ => None,
		},
		_ => None,
	};
	if let Some(pointee) = pointee
		&& matches!(
			name,
			"compare_exchange" | "compare_exchange_weak" | "try_update" | "fetch_update" | "update"
		)
		&& b.t.incomparable(pointee).is_some()
	{
		return Ok(Some(b.err(
			call.span,
			format!("comparing pointers to `{pointee}` in an atomic, which JS has no address of"),
		)));
	}
	let same = pointee.and_then(|p| b.t.same_fn(p));
	// A compare-exchange, given what compares a wide integer, whose limbs `===` can't, or
	// pointers `===` can't.
	let exchange = |f: &str, mut args: Vec<Expr>| {
		args.extend(int.filter(|int| int.big()).map(|int| int.func("eq")));
		args.extend(same.clone());
		Expr::rs(f, args)
	};
	let e = match name {
		"new" => Expr::New(Box::new(Expr::Rs("Atomic".into())), args),
		"load" => Expr::rs("atomicLoad", args),
		"store" => Expr::rs("atomicStore", args),
		"swap" => Expr::rs("atomicSwap", args),
		"into_inner" => first(args).member("v"),
		// The atomic is the place that holds its value.
		"get_mut" => first(args),
		"compare_exchange" | "compare_exchange_weak" => exchange("atomicCompareExchange", args),
		"fetch_add" if let Some(int) = int => fetch(int.func("wrappingAdd"), args),
		"fetch_sub" if let Some(int) = int => fetch(int.func("wrappingSub"), args),
		"fetch_max" | "fetch_min" if let Some(int) = int => {
			let f = match int.big() {
				true => int.func(&name[6..]),
				false => Expr::Rs(name[6..].into()),
			};
			fetch(f, args)
		}
		"fetch_and" | "fetch_or" | "fetch_xor" | "fetch_nand" => {
			let op = match name {
				"fetch_and" | "fetch_nand" => BinOp::BitAnd,
				"fetch_or" => BinOp::BitOr,
				_ => BinOp::BitXor,
			};
			fetch(binary(b, op, name == "fetch_nand", ty, call.span), args)
		}
		// `fetch_xor(true)`, as std's is.
		"fetch_not" if ty.is_bool() => {
			let f = binary(b, BinOp::BitXor, false, ty, call.span);
			let mut args = args;
			args.insert(1, Expr::lit("true"));
			fetch(f, args)
		}
		// std's loop of compare-exchanges, which retries only if `f` changed the atomic.
		"try_update" | "fetch_update" => exchange("atomicTryUpdate", args),
		"update" => exchange("atomicUpdate", args),
		_ => return Ok(None),
	};
	Ok(Some(e))
}

/// A `Cell<T>` method: its value, `v`, read, written, or replaced as std does. `Ok(None)`
/// refuses it.
fn cell<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	call: &Call<'tcx>,
	name: &str,
	args: Vec<Expr>,
	out: &mut Vec<Stmt>,
) -> Result<Option<Expr>, Vec<Expr>> {
	use super::pointer::{self, Pointee};
	let tcx = b.t.tcx;
	let span = call.span;
	let ty = call.args.type_at(0);
	// A pointer `as_ptr` gave to an object or an array is it, so a write lands in it.
	let in_place = matches!(b.t.pointee(ty), Pointee::Object | Pointee::Array) && !b.t.zero_sized(ty);
	let default = |b: &mut Body<'_, '_, 'tcx>, out: &mut Vec<Stmt>| {
		let f = super::collections::trait_fn(tcx, "Default", "default");
		b.call_def(f, tcx.mk_args(&[ty.into()]), Vec::new(), span, out)
	};
	let e = match name {
		"new" => Expr::New(Box::new(Expr::Rs("Cell".into())), args),
		"get" => b.t.copied(first(args).member("v"), ty, span),
		"into_inner" => first(args).member("v"),
		// As its `UnsafeCell`'s `get` and `get_mut`, the same `rs.Cell`.
		"as_ptr" | "get_mut" => return Err(args),
		// With what drops the value it replaces.
		"set" if in_place => {
			let [cell, value] = two(args);
			let p = pointer::cell_pointer(b, cell, ty);
			match b.t.drops(ty) {
				true => {
					let old = pointer::replace(b, p, value, ty, span, out);
					b.t.drop_value(old, ty, span).unwrap_or_else(Expr::undefined)
				}
				false => pointer::write(b, p, value, ty, span),
			}
		}
		"replace" | "take" if in_place => {
			let [cell, value] = two(args);
			let value = match name {
				"take" => default(b, out),
				_ => value,
			};
			let p = pointer::cell_pointer(b, cell, ty);
			pointer::replace(b, p, value, ty, span, out)
		}
		"update" if in_place => {
			let [cell, f] = two(args);
			let p = b.spill(pointer::cell_pointer(b, cell, ty), "pointer", out);
			let new = f.call(vec![pointer::read(b, p.clone(), ty, span)]);
			pointer::write(b, p, new, ty, span)
		}
		"swap" => {
			let [x, y] = two(args);
			let (x, y) = (pointer::cell_pointer(b, x, ty), pointer::cell_pointer(b, y, ty));
			pointer::swap(b, x, y, ty, span, out)
		}
		"replace" => Expr::rs("cellReplace", args),
		"update" => Expr::rs("cellUpdate", args),
		// With what drops the value it replaces.
		"set" => {
			let Some(droppers) = b.t.droppers([ty], span) else {
				return Ok(Some(Expr::undefined()));
			};
			Expr::rs("cellSet", args.into_iter().chain(droppers).collect())
		}
		"take" => {
			let value = default(b, out);
			Expr::rs("cellReplace", vec![first(args), value])
		}
		_ => return Ok(None),
	};
	Ok(Some(e))
}

/// A `OnceCell<T>` or `OnceLock<T>` method: its `Option<T>`, `v`, read or written as std
/// does, with that `Option`'s `None` where it is `rs.None`. `Ok(None)` refuses it.
fn once<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	call: &Call<'tcx>,
	name: &str,
	args: Vec<Expr>,
) -> Result<Option<Expr>, Vec<Expr>> {
	let tcx = b.t.tcx;
	let imp = tcx.inherent_impl_of_assoc(call.def_id).expect("an inherent method");
	let generics = tcx.generics_of(imp).count();
	let cell = tcx
		.type_of(imp)
		.instantiate(tcx, &call.args[..generics])
		.skip_norm_wip();
	let inner = call.args.type_at(0);
	let none = b.t.none_arg(inner);
	let e = match name {
		"new" => once_new(b, cell),
		"get" | "into_inner" => first(args).member("v"),
		"take" => Expr::rs("onceTake", args.into_iter().chain(none).collect()),
		"set" => Expr::rs("onceSet", args.into_iter().chain(none).collect()),
		// The place of what it holds where JS copies that, which is the cell's own.
		"get_mut" if b.t.needs_place(inner) => Expr::rs("optionAsMut", args.into_iter().chain(none).collect()),
		"get_mut" => first(args).member("v"),
		// With what drops `f`, as `discards` says, uncalled if the cell is set.
		"get_or_init" | "get_or_try_init" => {
			let Some(mut droppers) = b.t.droppers([call.args.type_at(1)], call.span) else {
				return Ok(Some(Expr::undefined()));
			};
			if none.is_some() {
				droppers.resize(1, Expr::undefined());
			}
			let f = match name {
				"get_or_init" => "onceGetOrInit",
				_ => "onceGetOrTryInit",
			};
			Expr::rs(f, args.into_iter().chain(droppers).chain(none).collect())
		}
		_ => return Ok(None),
	};
	Ok(Some(e))
}

/// A `Box<pointee>`, if `pointee` is a slice or text, whose value an `rs.Arc` of it holds.
pub fn shared_box<'tcx>(tcx: ty::TyCtxt<'tcx>, pointee: Ty<'tcx>) -> Option<Ty<'tcx>> {
	matches!(pointee.kind(), ty::Slice(_) | ty::Str).then(|| Ty::new_box(tcx, pointee))
}

/// The type a cell or lock of type `ty` holds.
fn held(ty: Option<Ty<'_>>) -> Ty<'_> {
	match ty.map(|ty| ty.kind()) {
		Some(ty::Adt(_, args)) => args.type_at(0),
		_ => unreachable!("a cell or lock is a struct"),
	}
}

/// `map`, `filter_map`, `try_map`, or `map_split` of a guard, by the runtime's functions
/// named for `guard_of`, `ref` for a `Ref` or a read guard: its function called on what
/// the guard lends, and a guard of each part that lends, which for an exclusive one is where the
/// part is, so a write through it reaches.
fn guard_map<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	call: &Call<'tcx>,
	name: &str,
	guard_of: &str,
	args: Vec<Expr>,
	out: &mut Vec<Stmt>,
) -> Expr {
	let tcx = b.t.tcx;
	let exclusive = guard_of != "ref";
	let [guard, f] = two(args);
	let guard = match guard.is_path() {
		true => guard,
		false => b.spill(guard, "guard", out),
	};
	// The guarded value, then each part's type.
	let mut tys = call.args.types();
	let held = tys.next().expect("a guarded type");
	let lent = match exclusive && b.t.needs_place(held) {
		true => Expr::rs("valuePlace", vec![guard.clone()]),
		false => guard.clone().member("value"),
	};
	let lent = f.call(vec![lent]);
	// What the guard holds of a part: a `[u8; N]` as the array its `rs.ByteSlice` views, and
	// for an exclusive one, where the part is: the place of one JS copies, and an object
	// itself, which a whole value is written into in place.
	let converts =
		|b: &Body<'_, '_, 'tcx>, part: Ty<'tcx>| super::byte_array(part) || exclusive && !b.t.needs_place(part);
	let at = |b: &mut Body<'_, '_, 'tcx>, part: Ty<'tcx>, value: Expr| {
		let value = match super::byte_array(part) {
			true => Expr::rs("byteSliceView", vec![value]),
			false => value,
		};
		if !converts(b, part) || !exclusive {
			value
		} else if b.t.write_in(part).is_none() && part.is_sized(tcx, b.t.typing_env_pub()) {
			b.err(
				call.span,
				format!("a mapped guard of `{part}`, which JS can't write in place"),
			)
		} else {
			Expr::rs("partPlace", vec![value])
		}
	};
	let part = tys.next().expect("a part's type");
	let f = |name: &str| format!("{guard_of}{name}");
	match name {
		"map" => {
			let at = at(b, part, lent);
			Expr::rs(f("Map"), vec![guard, at])
		}
		"filter_map" if !converts(b, part) => Expr::rs(f("FilterMap"), vec![guard, lent]),
		"filter_map" => {
			let lent = b.spill(lent, "part", out);
			let some = at(b, part, lent.clone());
			let at = Expr::Cond(
				Box::new(lent.binary("===", Expr::undefined())),
				Box::new(Expr::undefined()),
				Box::new(some),
			);
			Expr::rs(f("FilterMap"), vec![guard, at])
		}
		"try_map" if !converts(b, part) => Expr::rs(f("TryMap"), vec![guard, lent]),
		"try_map" => {
			let lent = b.spill(lent, "part", out);
			let some = at(b, part, lent.clone().member("value"));
			let at = Expr::Cond(
				Box::new(lent.clone().member("ok")),
				Box::new(Expr::result(true, some)),
				Box::new(Expr::result(false, lent.member("error"))),
			);
			Expr::rs(f("TryMap"), vec![guard, at])
		}
		_ => {
			let other = tys.next().expect("the other part's type");
			if !converts(b, part) && !converts(b, other) {
				return Expr::rs(f("MapSplit"), vec![guard, lent]);
			}
			let pair = b.spill(lent, "parts", out);
			let a = at(b, part, pair.clone().index(Expr::lit("0")));
			let other = at(b, other, pair.index(Expr::lit("1")));
			Expr::rs(f("MapSplit"), vec![guard, Expr::Array(vec![a, other])])
		}
	}
}

/// What `get_mut` of `holder`, a `Mutex`, a `RefCell`, or an `RwLock`, or a leak of a
/// `RefMut`, lends: its value, or where that is where JS copies it.
fn lend_value<'tcx>(b: &Body<'_, '_, 'tcx>, call: &Call<'tcx>, holder: Expr) -> Expr {
	match b.t.needs_place(call.args.types().next().expect("a held type")) {
		true => Expr::rs("valuePlace", vec![holder]),
		false => holder.member("value"),
	}
}

/// `OnceCell::new()` or `OnceLock::new()` of `cell`: empty.
pub fn once_new<'tcx>(b: &Body<'_, '_, 'tcx>, cell: Ty<'tcx>) -> Expr {
	let ty::Adt(_, args) = cell.kind() else {
		unreachable!("a one-time cell is a struct")
	};
	Expr::New(
		Box::new(Expr::Rs(once_class(b, cell).into())),
		vec![b.t.none(args.type_at(0))],
	)
}

/// `Once::new()`: an `rs.OnceLock` holding nothing, as no function has run.
pub fn once_unrun() -> Expr {
	Expr::New(Box::new(Expr::Rs("OnceLock".into())), vec![Expr::undefined()])
}

/// The runtime class of `cell`, a `OnceCell` or `OnceLock`.
fn once_class<'tcx>(b: &Body<'_, '_, 'tcx>, cell: Ty<'tcx>) -> &'static str {
	match b.t.std_of(cell) {
		Some(Std::OnceLock) => "OnceLock",
		_ => "OnceCell",
	}
}

/// `(old, value) => old op value` over `ty`, negated if `not`.
fn binary<'tcx>(b: &mut Body<'_, '_, 'tcx>, op: BinOp, not: bool, ty: Ty<'tcx>, span: Span) -> Expr {
	let operand = |name: &str| Operand {
		expr: Expr::ident(name),
		value: None,
	};
	let mut value = num::binary(b, op, operand("old"), operand("value"), ty, ty, span);
	if not {
		value = num::unary(b, UnOp::Not, value, ty, span);
	}
	let params = ["old", "value"].map(|name| Param {
		name: name.into(),
		ty: None,
	});
	Expr::Lambda(params.into(), Box::new(value))
}

/// The `atomic::Ordering` variant `index`: its discriminant.
pub fn atomic_ordering(index: VariantIdx) -> Expr {
	Expr::lit(index.as_u32().to_string())
}

/// `Waker::from(arc)`, or `RawWaker::from` if `raw`, of an `Arc<W>`: over `W`'s
/// `Wake::wake`, what drops the `Arc`, and `W`'s own `wake_by_ref` if it overrides the
/// default.
fn arc_waker<'tcx>(b: &mut Body<'_, '_, 'tcx>, arc: Ty<'tcx>, value: Expr, raw: bool, span: Span) -> Expr {
	let tcx = b.t.tcx;
	let ty::Adt(_, arc_args) = arc.kind() else {
		unreachable!("an Arc is a struct")
	};
	let wake_trait = tcx
		.get_diagnostic_item(Symbol::intern("Wake"))
		.expect("Waker::from bounds its Arc by Wake");
	let items = tcx.associated_items(wake_trait);
	let item = |name: &str| {
		items
			.filter_by_name_unhygienic(Symbol::intern(name))
			.next()
			.expect("a method of Wake")
			.def_id
	};
	let (wake, by_ref) = (item("wake"), item("wake_by_ref"));
	let w = tcx.mk_args(&[arc_args.type_at(0).into()]);
	let arc_param = b.fresh("arc");
	let dropped = b.t.drop_value(Expr::ident(arc_param.clone()), arc, span);
	let param = Param {
		name: arc_param,
		ty: Some(b.t.ty(arc, span)),
	};
	let drop = Expr::Lambda(vec![param], Box::new(dropped.unwrap_or_else(Expr::undefined)));
	let mut args = vec![value, b.fn_value(wake, w, span), drop];
	if let Ok(Some(instance)) = Instance::try_resolve(tcx, b.t.typing_env_pub(), by_ref, w)
		&& instance.def_id() != by_ref
	{
		args.push(b.fn_value(by_ref, w, span));
	}
	Expr::rs(if raw { "arcRawWaker" } else { "arcWaker" }, args)
}

/// A `RawWaker` of `data` and `vtable`: an `rs.RawWaker`, under std's names.
pub fn raw_waker(data: Expr, vtable: Expr) -> Expr {
	Expr::Object(vec![
		(Key::Name("data".into()), data),
		(Key::Name("vtable".into()), vtable),
	])
}

fn first(args: Vec<Expr>) -> Expr {
	args.into_iter().next().unwrap_or_else(Expr::undefined)
}

fn two(args: Vec<Expr>) -> [Expr; 2] {
	let mut values = args.into_iter();
	[
		values.next().unwrap_or_else(Expr::undefined),
		values.next().unwrap_or_else(Expr::undefined),
	]
}
