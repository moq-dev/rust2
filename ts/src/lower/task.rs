//! The standard library's ownership and async types, as `rt/task.ts` gives them: `Arc`,
//! `Rc`, `Weak`, `Mutex`, atomics, `Waker`, `Context`, `Poll`, `Pin`, and std's futures.

use rustc_abi::VariantIdx;
use rustc_middle::mir::{BinOp, UnOp};
use rustc_middle::ty::{self, Instance, Ty};
use rustc_span::{Span, Symbol};

use super::Translator;
use super::body::Body;
use super::library::{self, Call, Std};
use super::num::{self, Int, Operand};
use crate::ts::{Expr, Param, Stmt};

impl<'tcx> Translator<'_, 'tcx> {
	/// `Future::Output` of `ty`, if it is a `dyn Future`, whose JS value is an `rt.Future`.
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
	/// references, boxes, and pins, whose JS value is what they point at, or an `rt.Place`
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

	/// `value`, a `from` that is a future, as an `rt.Future`: itself if it is one already,
	/// and else an `rt.DynFuture` over its `Future` impl, which drops it with itself unless
	/// `borrowed`. A borrowed one that is `dropped` must not drop what it borrows.
	fn future_object(&mut self, from: Ty<'tcx>, value: Expr, borrowed: bool, dropped: bool, span: Span) -> Expr {
		let tcx = self.t.tcx;
		let object = match from.kind() {
			ty::Dynamic(..) | ty::Coroutine(..) => true,
			// What a reference or a box points at, which its JS value is, or an `rt.Place` of.
			ty::Ref(_, inner, _) => {
				let value = self.t.pointee_value(from, value);
				return self.future_object(*inner, value, true, dropped, span);
			}
			ty::Adt(adt, args) => match Std::of(tcx, adt.did()) {
				Some(Std::PollFn | Std::Pending) => true,
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
					Box::new(Expr::Rt("DynFuture".into())),
					vec![value, Expr::Rt("futurePoll".into())],
				),
				false => value,
			};
		}
		// What `Future::poll` takes would be an `rt.Place`, which the box doesn't have.
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
		Expr::New(Box::new(Expr::Rt("DynFuture".into())), args)
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
	let e = match (owner, name) {
		("Arc" | "Rc", "new") => Expr::New(Box::new(Expr::Rt("Arc".into())), args),
		("Arc" | "Rc", "new_cyclic") => Expr::rt("arcNewCyclic", args),
		// JS erases the pin.
		("Arc" | "Rc", "pin") => Expr::New(Box::new(Expr::Rt("Arc".into())), args),
		// A `&mut` to the value, which is where it is for one JS copies. `make_mut` takes the
		// `Arc`'s place, which it may point at a new `Arc`.
		("Arc" | "Rc", "get_mut" | "make_mut") if call.args.type_at(0).is_sized(tcx, b.t.typing_env_pub()) => {
			let value = call.args.type_at(0);
			let place = if b.t.needs_place(value) { "Place" } else { "" };
			match name {
				"get_mut" => Expr::rt(format!("arcGetMut{place}"), args),
				_ => {
					let clone = super::collections::clone_fn(b, value, call.span);
					let arc = b.t.sig(call.def_id, call.args).inputs()[0].peel_refs();
					let Some(droppers) = b.t.droppers([arc], call.span) else {
						return Ok(Some(Expr::undefined()));
					};
					let values = args.into_iter().chain([clone]).chain(droppers).collect();
					Expr::rt(format!("arcMakeMut{place}"), values)
				}
			}
		}
		// An `Arc<str>` of a string, which JS shares as it is.
		("From", "from")
			if let [arc, from] = call.args.as_slice()
				&& let ty::Adt(_, arc) = arc.expect_ty().kind()
				&& self_std == Some(Std::Arc)
				&& arc.type_at(0).is_str()
				&& library::string(tcx, from.expect_ty().peel_refs()) =>
		{
			Expr::New(Box::new(Expr::Rt("Arc".into())), args)
		}
		// A waker over the crate's `Wake` impl.
		("From", "from")
			if let [_, from] = call.args.as_slice()
				&& self_std == Some(Std::Waker)
				&& std_of(from.expect_ty()) == Some(Std::Arc) =>
		{
			arc_waker(b, from.expect_ty(), first(args), call.span)
		}
		("Waker", "noop") => Expr::Rt("NOOP_WAKER".into()),
		("AsRef", "as_ref") if self_std == Some(Std::Arc) => first(args).member("value"),
		("Arc" | "Rc", "downgrade") | ("Weak", "new") => Expr::New(Box::new(Expr::Rt("Weak".into())), args),
		// The value of the last strong one, which its count says, and else its `None`.
		("Arc" | "Rc", "into_inner") => {
			let none = b.t.none_arg(call.args.type_at(0));
			Expr::rt("arcIntoInner", args.into_iter().chain(none).collect())
		}
		("Arc" | "Rc", "strong_count") => first(args).member("strong"),
		("Arc" | "Rc", "weak_count") => first(args).member("weak"),
		// Two `Arc`s of trait objects are views of the `Arc`s they were made from.
		("Arc" | "Rc", "ptr_eq") if self_ty.is_some_and(Ty::is_trait) => Expr::rt("arcPtrEq", args),
		("Arc" | "Rc", "ptr_eq") => {
			let [l, r] = two(args);
			l.binary("===", r)
		}
		("Weak", "upgrade") => Expr::rt("weakUpgrade", args),
		("Weak", "strong_count") => first(args).member("arc").member("strong"),
		("Weak", "weak_count") => Expr::rt("weakWeakCount", args),
		("Weak", "ptr_eq") if self_ty.is_some_and(Ty::is_trait) => {
			let [l, r] = two(args);
			Expr::rt("arcPtrEq", vec![l.member("arc"), r.member("arc")])
		}
		("Weak", "ptr_eq") => {
			let [l, r] = two(args);
			l.member("arc").binary("===", r.member("arc"))
		}
		("Clone", "clone") if self_std == Some(Std::Arc) => Expr::rt("arcClone", args),
		("Clone", "clone") if self_std == Some(Std::Weak) => Expr::rt("weakClone", args),
		// A waker is the executor's, whose `clone` is its own.
		("Clone", "clone") if self_std == Some(Std::Waker) => first(args).member("clone").call(vec![]),
		// A reference is the object it points at, which its own `Deref` gives: read out of a
		// `&mut` that is an `rt.Place`, whose place `DerefMut` lends on.
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
		("Mutex", "new") => Expr::New(Box::new(Expr::Rt("Mutex".into())), args),
		// The mutex is its own guard, and a panic aborts, so nothing poisons it.
		("Mutex", "lock") => Expr::result(true, Expr::rt("mutexLock", args)),
		("Mutex", "try_lock") => Expr::rt("mutexTryLock", args),
		("Mutex", "get_mut") => Expr::result(true, first(args).member("value")),
		// A `PoisonError` is what it holds.
		("PoisonError", "into_inner" | "get_ref" | "get_mut") => first(args),
		// Mapped rather than lowered from core's MIR, so one of a lock unwraps its guard in place.
		("Result", "unwrap_or_else") if call.args.types().nth(1).and_then(std_of) == Some(Std::PoisonError) => {
			Expr::rt("resultUnwrapOrElse", args)
		}
		// A cell, or a lock, is its own guard.
		("RefCell" | "RwLock", "new") => Expr::New(Box::new(Expr::Rt("RefCell".into())), args),
		("RefCell", "borrow") => Expr::rt("refCellBorrow", args),
		("RefCell", "borrow_mut") => Expr::rt("refCellBorrowMut", args),
		("RefCell", "replace") => Expr::rt("refCellReplace", args),
		("RefCell", "take") => {
			let ty = call.args.type_at(0);
			let f = super::collections::trait_fn(tcx, "Default", "default");
			let value = b.call_def(f, tcx.mk_args(&[ty.into()]), Vec::new(), call.span, out);
			Expr::rt("refCellReplace", vec![first(args), value])
		}
		// A panic aborts, so nothing poisons it.
		("RwLock", "read") => Expr::result(true, Expr::rt("rwLockRead", args)),
		("RwLock", "write") => Expr::result(true, Expr::rt("rwLockWrite", args)),
		("RwLock", "try_read") => Expr::rt("rwLockTryRead", args),
		("RwLock", "try_write") => Expr::rt("rwLockTryWrite", args),
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
			Expr::rt("localKeyRefCellSet", args.into_iter().chain(droppers).collect())
		}
		// With the value's place where JS would copy it.
		("LocalKey", "with_borrow_mut") => match b.t.needs_place(call.args.type_at(0)) {
			true => Expr::rt("localKeyWithBorrowMutPlace", args),
			false => Expr::rt("localKeyWithBorrowMut", args),
		},
		// With the `Option`'s `None` where it is `rt.None`.
		("LocalKey", "initialize_with") => {
			let none = b.t.none_arg(call.args.type_at(0));
			Expr::rt("localKeyInitializeWith", args.into_iter().chain(none).collect())
		}
		("Atomic", _) => return atomic(b, call, name, args),
		("LazyCell" | "LazyLock", "force") => first(args).member("force").call(vec![]),
		("OnceCell" | "OnceLock", _) => return once(b, call, name, args),
		// A `OnceLock` of `true`, which nothing poisons, as a panic aborts.
		("Once", "new") => once_unrun(),
		("Once", "is_completed") => first(args).member("v").binary("===", Expr::lit("true")),
		// With what drops `f`, as `discards` says, uncalled if the `Once` has run.
		("Once", "call_once" | "call_once_force") => {
			let Some(droppers) = b.t.droppers([call.args.type_at(0)], call.span) else {
				return Ok(Some(Expr::undefined()));
			};
			Expr::rt("onceCall", args.into_iter().chain(droppers).collect())
		}
		("OnceState", "is_poisoned") => Expr::rt("onceIsPoisoned", args),
		("From", "from") if matches!(self_std, Some(Std::OnceCell | Std::OnceLock)) => {
			let cell = self_ty.expect("a `From` impl's type");
			Expr::New(Box::new(Expr::Rt(once_class(b, cell).into())), args)
		}
		("atomic", "fence" | "compiler_fence") => Expr::rt("fence", args),
		// A panic aborts, so no thread is ever unwinding from one.
		(_, "panicking")
			if library::std_path(tcx, call.def_id)
				.first()
				.is_some_and(|m| m == "thread") =>
		{
			Expr::lit("false")
		}
		("Waker", "wake_by_ref") => first(args).member("wakeByRef").call(vec![]),
		("Waker", "wake") => first(args).member("wake").call(vec![]),
		("Waker", "will_wake") => Expr::rt("willWake", args),
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
				Box::new(Expr::Rt("PollFn".into())),
				args.into_iter().chain(dropper).collect(),
			)
		}
		("pending", "pending") => Expr::Rt("NEVER".into()),
		// A box's JS value is what it holds.
		("Box", "pin") => first(args),
		("Future", "poll") => match self_ty.map(Ty::kind) {
			// A `&mut` polls what it points at, whose pinned `&mut` is an `rt.Place` of it.
			Some(ty::Ref(_, inner, _)) => {
				let [future, cx] = two(args);
				let args = vec![future.member("v"), cx];
				b.call_def(call.def_id, tcx.mk_args(&[(*inner).into()]), args, call.span, out)
			}
			// std's futures and a `dyn Future` are `rt.Future`s, which poll themselves.
			_ if matches!(self_std, Some(Std::PollFn | Std::Pending))
				|| self_ty.is_some_and(|ty| b.t.dyn_future(ty).is_some()) =>
			{
				let [future, cx] = two(args);
				future.member("poll").call(vec![cx])
			}
			// A pinned pointer or a box polls what it points at, which its JS value is.
			Some(ty::Adt(adt, adt_args)) if matches!(Std::of(tcx, adt.did()), Some(Std::Pin | Std::Box)) => {
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

/// An `Atomic<T>` method: an `rt.Atomic`'s, with its orderings, which the runtime checks.
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
		Expr::rt("atomicFetch", args)
	};
	// A pointer compares as `ptr::eq` does only where it is the object it points at, or
	// a slice of the bytes it points at, which `rt.byteSliceSame` compares.
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
	let bytes = pointee.is_some_and(|p| b.t.pointee(p) == crate::lower::pointer::Pointee::Bytes);
	// A compare-exchange, given what compares a wide integer, whose limbs `===` can't, or
	// pointers to bytes.
	let exchange = |f: &str, mut args: Vec<Expr>| {
		args.extend(int.filter(|int| int.big()).map(|int| int.func("eq")));
		args.extend(bytes.then(|| Expr::Rt("byteSliceSame".into())));
		Expr::rt(f, args)
	};
	let e = match name {
		"new" => Expr::New(Box::new(Expr::Rt("Atomic".into())), args),
		"load" => Expr::rt("atomicLoad", args),
		"store" => Expr::rt("atomicStore", args),
		"swap" => Expr::rt("atomicSwap", args),
		"into_inner" => first(args).member("v"),
		// The atomic is the place that holds its value.
		"get_mut" => first(args),
		"compare_exchange" | "compare_exchange_weak" => exchange("atomicCompareExchange", args),
		"fetch_add" if let Some(int) = int => fetch(int.func("wrappingAdd"), args),
		"fetch_sub" if let Some(int) = int => fetch(int.func("wrappingSub"), args),
		"fetch_max" | "fetch_min" if let Some(int) = int => {
			let f = match int.big() {
				true => int.func(&name[6..]),
				false => Expr::Rt(name[6..].into()),
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
	let tcx = b.t.tcx;
	let ty = call.args.type_at(0);
	let e = match name {
		"new" => Expr::New(Box::new(Expr::Rt("Cell".into())), args),
		"get" => b.t.copied(first(args).member("v"), ty, call.span),
		"into_inner" => first(args).member("v"),
		"replace" => Expr::rt("cellReplace", args),
		// With what drops the value it replaces.
		"set" => {
			let Some(droppers) = b.t.droppers([ty], call.span) else {
				return Ok(Some(Expr::undefined()));
			};
			Expr::rt("cellSet", args.into_iter().chain(droppers).collect())
		}
		"take" => {
			let f = super::collections::trait_fn(tcx, "Default", "default");
			let value = b.call_def(f, tcx.mk_args(&[ty.into()]), Vec::new(), call.span, out);
			Expr::rt("cellReplace", vec![first(args), value])
		}
		_ => return Ok(None),
	};
	Ok(Some(e))
}

/// A `OnceCell<T>` or `OnceLock<T>` method: its `Option<T>`, `v`, read or written as std
/// does, with that `Option`'s `None` where it is `rt.None`. `Ok(None)` refuses it.
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
		"take" => Expr::rt("onceTake", args.into_iter().chain(none).collect()),
		"set" => Expr::rt("onceSet", args.into_iter().chain(none).collect()),
		// With what drops `f`, as `discards` says, uncalled if the cell is set.
		"get_or_init" => {
			let Some(mut droppers) = b.t.droppers([call.args.type_at(1)], call.span) else {
				return Ok(Some(Expr::undefined()));
			};
			if none.is_some() {
				droppers.resize(1, Expr::undefined());
			}
			Expr::rt("onceGetOrInit", args.into_iter().chain(droppers).chain(none).collect())
		}
		_ => return Ok(None),
	};
	Ok(Some(e))
}

/// `OnceCell::new()` or `OnceLock::new()` of `cell`: empty.
pub fn once_new<'tcx>(b: &Body<'_, '_, 'tcx>, cell: Ty<'tcx>) -> Expr {
	let ty::Adt(_, args) = cell.kind() else {
		unreachable!("a one-time cell is a struct")
	};
	Expr::New(
		Box::new(Expr::Rt(once_class(b, cell).into())),
		vec![b.t.none(args.type_at(0))],
	)
}

/// `Once::new()`: an `rt.OnceLock` holding nothing, as no function has run.
pub fn once_unrun() -> Expr {
	Expr::New(Box::new(Expr::Rt("OnceLock".into())), vec![Expr::undefined()])
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

/// `Waker::from(arc)`, an `Arc<W>`: an `rt.ArcWaker` over `W`'s `Wake::wake`, what drops
/// the `Arc`, and `W`'s own `wake_by_ref` if it overrides the default.
fn arc_waker<'tcx>(b: &mut Body<'_, '_, 'tcx>, arc: Ty<'tcx>, value: Expr, span: Span) -> Expr {
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
	Expr::New(Box::new(Expr::Rt("ArcWaker".into())), args)
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
