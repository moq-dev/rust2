//! Rvalues: arithmetic, casts, aggregates, and closures.

use rustc_abi::{FieldIdx, VariantIdx};
use rustc_data_structures::fx::FxHashSet;
use rustc_index::IndexVec;
use rustc_middle::mir::visit::{MutatingUseContext, PlaceContext, Visitor};
use rustc_middle::mir::{self, AggregateKind, BinOp, CastKind, Operand, Place, Rvalue, UnOp};
use rustc_middle::ty::adjustment::PointerCoercion;
use rustc_middle::ty::{self, Instance, Ty, TyCtxt};
use rustc_span::Span;

use super::{Capture, Env, Fx, Slot};
use crate::lower::library::Std;
use crate::lower::num::{self, Int};
use crate::lower::pointer::Pointee;
use crate::lower::{AdtRepr, byte_array, field_name};
use crate::ts::{Expr, Key, Param, Stmt, StmtKind, Type};

impl<'m, 'b, 'a, 'tcx> Fx<'m, 'b, 'a, 'tcx> {
	/// The value of `rvalue`, assigned to `dest`, or `None` if it has none to assign.
	pub(super) fn rvalue(
		&mut self,
		rvalue: &Rvalue<'tcx>,
		dest: Place<'tcx>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Option<Expr> {
		let tcx = self.tcx;
		let ty = rvalue.ty(&self.mir.local_decls, tcx);
		Some(match rvalue {
			Rvalue::Use(op, _) => self.operand(op, span, out),
			Rvalue::CopyForDeref(place) => self.place_expr(*place, span, out),
			Rvalue::Repeat(op, count) => self.repeat(op, *count, span, out),
			Rvalue::Ref(_, kind, place) => {
				if self.union_whole(*place, span).is_some() {
					return Some(self.err(span, "a reference to a union's field kept in more than one slot"));
				}
				if let Some(e) = self.mut_subslice(*kind, *place, span, out) {
					return Some(e);
				}
				if let mir::BorrowKind::Mut { .. } = kind {
					let pointee = place.ty(&self.mir.local_decls, tcx).ty;
					if self.b.t.needs_place(pointee) {
						if let Some(reborrowed) = self.reborrowed(*kind, *place, span, out) {
							return Some(reborrowed);
						}
						let target = self.place_expr(*place, span, out);
						return Some(self.make_place(*place, target, span, out));
					}
					if matches!(pointee.kind(), ty::Ref(_, _, rustc_ast::Mutability::Not)) {
						return Some(self.err(
							span,
							format!("a mutable reference to `{pointee}`, which is no object it can change in place"),
						));
					}
				}
				let value = self.place_expr(*place, span, out);
				// A borrow of an array of bytes is a slice of all of it, sharing its bytes. One of
				// a `MaybeUninit` of them through a raw pointer is a view of the pointer's bytes.
				let pointee = place.ty(&self.mir.local_decls, tcx).ty;
				match (byte_array(pointee), self.in_bytes(*place)) {
					(true, false) => Expr::rt("byteSliceFrom", vec![value]),
					(false, true) => Expr::rt("byteSliceView", vec![value]),
					_ => value,
				}
			}
			// A bounds check of a `&mut` slice reads its length through a raw pointer that
			// only `PtrMetadata` reads, which reads the slice's.
			Rvalue::RawPtr(mir::RawPtrKind::FakeForPtrMetadata, place) => self.place_expr(*place, span, out),
			Rvalue::RawPtr(_, place) => {
				if self.union_whole(*place, span).is_some() {
					return Some(self.err(span, "a pointer to a union's field kept in more than one slot"));
				}
				if let Some(why) = self.out_of_step(*place, false) {
					return Some(self.err(span, why));
				}
				let pointee = place.ty(&self.mir.local_decls, tcx).ty;
				match self.b.t.pointee(pointee) {
					Pointee::Object => self.place_expr(*place, span, out),
					// A slice of all of an array's bytes, as a borrow of it is.
					Pointee::Bytes => {
						let value = self.place_expr(*place, span, out);
						match self.in_bytes(*place) {
							true => value,
							false => Expr::rt("byteSliceFrom", vec![value]),
						}
					}
					// `&raw mut *p` is `p`. A pointer to `()` is an identity, which only another
					// pointer is: not the `rt.Place` of a place or of a `&mut`, each borrow's own.
					Pointee::Place => match place.projection.as_slice() {
						[rest @ .., mir::ProjectionElem::Deref]
							if !self.box_pointer(place.local) && {
								let base = Place::ty_from(place.local, rest, &self.mir.local_decls, tcx).ty;
								match self.b.t.identity_pointee(pointee) {
									true => base.is_raw_ptr(),
									false => self.b.t.place_pointee(base).is_some(),
								}
							} =>
						{
							let base = Place {
								local: place.local,
								projection: tcx.mk_place_elems(rest),
							};
							self.place_expr(base, span, out)
						}
						_ if self.b.t.identity_pointee(pointee) => self.err(
							span,
							"a raw pointer to a place of `()`, which has no address of its own",
						),
						// A reference to an object is the object, not the place holding it.
						[.., mir::ProjectionElem::Deref] if !self.box_pointer(place.local) => self.err(
							span,
							format!(
								"a raw pointer to `{pointee}` behind a reference, which is the object and not where it is kept"
							),
						),
						_ => {
							let target = self.place_expr(*place, span, out);
							self.make_place(*place, target, span, out)
						}
					},
				}
			}
			Rvalue::Reborrow(..) => self.err(span, "reborrowing a type by its `Reborrow` impl is not supported"),
			Rvalue::ThreadLocalRef(_) => self.err(span, "thread locals are not supported"),
			Rvalue::WrapUnsafeBinder(..) => self.err(span, "unsafe binders are not supported"),
			Rvalue::Cast(kind, op, to) => self.cast(*kind, op, *to, span, out),
			Rvalue::BinaryOp(BinOp::BitOr, ops)
				if let Operand::Move(p) = &ops.0
					&& let Some(&(hi, lo)) = p.as_local().and_then(|l| self.halves.get(&l)) =>
			{
				let hi = self.half(hi, span, out);
				let lo = self.half(lo, span, out);
				Expr::rt("u64FromHalves", vec![hi, lo])
			}
			Rvalue::BinaryOp(op, ops) => {
				let (l, r) = &**ops;
				let lty = l.ty(&self.mir.local_decls, tcx);
				let rty = r.ty(&self.mir.local_decls, tcx);
				let op = op.overflowing_to_wrapping().unwrap_or(*op);
				let op = match op {
					BinOp::AddUnchecked => BinOp::Add,
					BinOp::SubUnchecked => BinOp::Sub,
					BinOp::MulUnchecked => BinOp::Mul,
					BinOp::ShlUnchecked => BinOp::Shl,
					BinOp::ShrUnchecked => BinOp::Shr,
					op => op,
				};
				if let ty::RawPtr(pointee, _) = lty.kind()
					&& matches!(op, BinOp::Eq | BinOp::Ne)
				{
					let x = self.operand(l, span, out);
					let y = self.operand(r, span, out);
					let same = crate::lower::pointer::compare(self.b, x, y, *pointee, span);
					return Some(match op {
						BinOp::Eq => same,
						_ => same.not(),
					});
				}
				if matches!(op, BinOp::Cmp | BinOp::Offset) {
					return Some(self.err(span, format!("`{op:?}` is not supported")));
				}
				let lv = self.operand(l, span, out);
				let rv = self.operand(r, span, out);
				let l = num::Operand {
					expr: lv,
					value: self.const_int(l),
				};
				let r = num::Operand {
					expr: rv,
					value: self.const_int(r),
				};
				num::binary(self.b, op, l, r, lty, rty, span)
			}
			Rvalue::UnaryOp(op, operand) => {
				let aty = operand.ty(&self.mir.local_decls, tcx);
				let value = self.operand(operand, span, out);
				match op {
					UnOp::Not | UnOp::Neg => num::unary(self.b, *op, value, aty, span),
					UnOp::PtrMetadata => match aty.builtin_deref(true).map(|t| t.kind()) {
						Some(ty::Str) => Expr::rt("stringLen", vec![value]),
						Some(ty::Slice(_)) => super::place::slice_member(value, "length"),
						_ => self.err(span, format!("the metadata of `{aty}`")),
					},
				}
			}
			// A local held as its parts has its flag.
			Rvalue::Discriminant(place) if self.scalar_enum(*place) => {
				let flag = self.scalar_flag(*place, span);
				let pty = place.ty(&self.mir.local_decls, tcx).ty;
				let int = Int::of(pty.discriminant_ty(tcx)).expect("an enum's discriminant is an integer");
				Expr::Cond(Box::new(flag), Box::new(int.lit(1)), Box::new(int.lit(0)))
			}
			Rvalue::Discriminant(place) => {
				let pty = place.ty(&self.mir.local_decls, tcx).ty;
				let scrut = self.place_expr(*place, span, out);
				self.discriminant(scrut, pty, span, out)
			}
			Rvalue::Aggregate(kind, ops) => self.aggregate(kind, ops, ty, dest, span, out)?,
		})
	}

	/// An unsigned integer of at most 32 bits, as a `u32`.
	fn half(&mut self, op: &Operand<'tcx>, span: Span, out: &mut Vec<Stmt>) -> Expr {
		let from = op.ty(&self.mir.local_decls, self.tcx);
		let expr = self.operand(op, span, out);
		let value = num::Operand {
			expr,
			value: self.const_int(op),
		};
		num::cast(self.b, value, from, self.tcx.types.u32, span)
	}

	/// The integer an operand has, if it is a constant one, a named `const` included.
	pub(super) fn const_int(&self, op: &Operand<'tcx>) -> Option<i128> {
		let Operand::Constant(c) = op else {
			return None;
		};
		let i = Int::of(c.const_.ty())?;
		let int = c.const_.try_eval_scalar_int(self.tcx, self.b.t.typing_env_pub())?;
		let size = rustc_abi::Size::from_bits(i.bits);
		Some(match i.signed {
			true => int.to_int(size),
			false => int.to_uint(size) as i128,
		})
	}

	fn repeat(&mut self, op: &Operand<'tcx>, count: ty::Const<'tcx>, span: Span, out: &mut Vec<Stmt>) -> Expr {
		let tcx = self.tcx;
		let Some(n) = count.try_to_target_usize(tcx) else {
			return self.err(span, "an array length that is not a constant");
		};
		let elem_ty = op.ty(&self.mir.local_decls, tcx);
		if self.b.t.drops(elem_ty) {
			return self.err(
				span,
				"an array repeat of a value that drops, which is not supported yet",
			);
		}
		let item_ty = self.b.t.ty(elem_ty, span);
		let each = |body: Vec<Stmt>| {
			Expr::ident("Array").member("from").call(vec![
				Expr::Object(vec![(Key::Name("length".into()), Expr::lit(n.to_string()))]),
				Expr::Arrow(Vec::new(), item_ty.clone(), body),
			])
		};
		// What isn't `Copy` repeats only as a constant, which Rust makes anew for each element.
		if !self.b.t.is_copy(elem_ty) {
			let mut body = Vec::new();
			let item = self.operand(op, span, &mut body);
			body.push(StmtKind::Return(Some(item.typed())).at(None));
			return each(body);
		}
		let v = self.operand(op, span, out);
		match elem_ty.kind() {
			ty::Uint(ty::UintTy::U8) => {
				let array = Expr::rt("bytes", vec![Expr::lit(n.to_string())]);
				match &v {
					Expr::Lit(text) if text == "0" => array,
					_ => array.member("fill").call(vec![v]),
				}
			}
			_ => {
				let v = self.path(v, "item", out);
				let item = self.b.t.copied(v, elem_ty, span);
				each(vec![StmtKind::Return(Some(item.typed())).at(None)])
			}
		}
	}

	fn cast(&mut self, kind: CastKind, op: &Operand<'tcx>, to: Ty<'tcx>, span: Span, out: &mut Vec<Stmt>) -> Expr {
		let tcx = self.tcx;
		let from = op.ty(&self.mir.local_decls, tcx);
		match kind {
			CastKind::IntToInt | CastKind::IntToFloat | CastKind::FloatToInt | CastKind::FloatToFloat => {
				let value = self.operand(op, span, out);
				let operand = num::Operand {
					expr: value,
					value: self.const_int(op),
				};
				num::cast(self.b, operand, from, to, span)
			}
			CastKind::PointerCoercion(coercion, _) => match coercion {
				PointerCoercion::Unsize if let Some(tr) = self.b.fmt_object(to) => {
					let value = self.operand(op, span, out);
					let value = self.path(value, "arg", out);
					self.b.fmt_value(tr, from, value, span, out)
				}
				PointerCoercion::Unsize if let Some((pointee, object)) = arc_object(tcx, from, to) => {
					let value = self.operand(op, span, out);
					self.arc_object(value, pointee, object, span, out)
				}
				// A future, as the `rt.Future` a `dyn Future` is.
				PointerCoercion::Unsize
					if unsized_to(from, to).is_some_and(|part| self.b.t.dyn_future(part).is_some()) =>
				{
					let value = self.operand(op, span, out);
					self.b.dyn_future(from, to, value, span)
				}
				PointerCoercion::Unsize if unsized_to(from, to).is_some_and(|t| t.is_trait()) => {
					let value = self.operand(op, span, out);
					self.b.object_of(value, from, to, span)
				}
				// An array's bytes, as a slice of all of it: a reference to them is one already.
				PointerCoercion::Unsize if unsized_to(from, to).is_some_and(super::place::byte_slice) => {
					match to.builtin_deref(true) {
						Some(pointee) if super::place::byte_slice(pointee) => {
							let value = self.operand(op, span, out);
							match crate::lower::bytes_ref(from) {
								true => value,
								// A raw pointer to a slice is an `rt.Place` of one, which one to an array is not.
								false if from.is_raw_ptr() => {
									self.err(span, format!("a coercion of `{from}` to `{to}`, a pointer to a slice"))
								}
								false => Expr::rt("byteSliceFrom", vec![value]),
							}
						}
						_ => self.err(span, format!("a coercion to `{to}`")),
					}
				}
				PointerCoercion::Unsize
				| PointerCoercion::ReifyFnPointer(_)
				| PointerCoercion::ClosureFnPointer(_)
				| PointerCoercion::MutToConstPointer => self.operand(op, span, out),
				_ => self.err(span, format!("unsupported pointer coercion {coercion:?}")),
			},
			// The same pointer, as one to the same pointee: `*mut T` as `*const T`, or a
			// `MaybeUninit<T>` as the `T` it holds.
			CastKind::PtrToPtr
				if let (Some(a), Some(b)) = (from.builtin_deref(true), to.builtin_deref(true))
					&& self.b.t.uninit_inner(a) == self.b.t.uninit_inner(b) =>
			{
				self.operand(op, span, out)
			}
			CastKind::PtrToPtr => self.err(
				span,
				format!("a cast from `{from}` to `{to}`, which reads the bytes of one type as another"),
			),
			CastKind::PointerExposeProvenance | CastKind::PointerWithExposedProvenance => self.err(
				span,
				format!("a cast from `{from}` to `{to}`, between a pointer and an integer"),
			),
			// A box's pointer is its contents in JS, which a read shares: the pointer's
			// type is `Copy`, but what it points to is not copied.
			CastKind::BoxDerefTransmute => match op {
				Operand::Copy(place) | Operand::Move(place) => self.place_expr(*place, span, out),
				Operand::Constant(_) | Operand::RuntimeChecks(_) => self.operand(op, span, out),
			},
			// An async body's context passes through `ResumeTy` and `NonNull`.
			CastKind::Transmute if context_like(self, from) && context_like(self, to) => self.operand(op, span, out),
			// A pattern type, as `NonZero` holds, is its base type's value.
			CastKind::Transmute if peel_pat(from) == peel_pat(to) => self.operand(op, span, out),
			_ => self.err(span, format!("a `{kind:?}` cast from `{from}` to `{to}`")),
		}
	}

	/// `arc`, an `Arc` of `pointee` unsized to one of the trait object `object`: an
	/// `rt.ArcDyn` sharing its count, over what a box of its value unsizes to, or the same
	/// `Arc` upcast.
	fn arc_object(&mut self, arc: Expr, pointee: Ty<'tcx>, object: Ty<'tcx>, span: Span, out: &mut Vec<Stmt>) -> Expr {
		let tcx = self.tcx;
		// An upcast of a function is the same `rt.ArcDyn`: nothing calls an `Arc` through `FnOnce`.
		if pointee.is_trait() && self.b.t.dyn_fn(object).is_some() {
			return arc;
		}
		// An upcast is of an `rt.ArcDyn` already, whose object has the supertraits' methods.
		let (arc, value) = match pointee.is_trait() {
			true => (None, arc),
			false => {
				let arc = self.path(arc, "arc", out);
				(Some(arc.clone()), arc.member("value"))
			}
		};
		let (from, to) = (Ty::new_box(tcx, pointee), Ty::new_box(tcx, object));
		let value = match self.b.t.dyn_future(object) {
			Some(_) => self.b.dyn_future(from, to, value, span),
			None => self.b.object_of(value, from, to, span),
		};
		match arc {
			Some(arc) => Expr::New(Box::new(Expr::Rt("ArcDyn".into())), vec![arc, value]),
			None => value,
		}
	}

	/// A discriminant as its number.
	fn discriminant(&mut self, scrut: Expr, ty: Ty<'tcx>, span: Span, out: &mut Vec<Stmt>) -> Expr {
		match ty.kind() {
			ty::Coroutine(def_id, args) => {
				let info = super::coroutine::info(self.b.t, *def_id, args, span);
				scrut.field(info.state)
			}
			ty::Adt(adt, _) if adt.is_enum() => {
				let tcx = self.tcx;
				let discrs: Vec<(VariantIdx, u128)> = adt.discriminants(tcx).map(|(i, d)| (i, d.val)).collect();
				// Each discriminant in the enum's own discriminant type, as `as` reads it.
				let int = Int::of(ty.discriminant_ty(tcx)).expect("an enum's discriminant is an integer");
				let discrs: Vec<(VariantIdx, u128)> = discrs
					.into_iter()
					.map(|(i, d)| (i, int.fold(d as i128) as u128))
					.collect();
				match Std::of(tcx, adt.did()) {
					Some(Std::Option | Std::Poll | Std::Result | Std::Entry) => {
						let scrut = self.path(scrut, "scrutinee", out);
						let first = self.variant_test(&scrut, ty, discrs[0].1, span);
						Expr::Cond(
							Box::new(first),
							Box::new(int.lit(discrs[0].1 as i128)),
							Box::new(int.lit(discrs[1].1 as i128)),
						)
					}
					// The value is its discriminant.
					Some(Std::Ordering | Std::AtomicOrdering) => scrut,
					Some(Std::Cow) if let Some(elem) = crate::lower::library::cow_slice(tcx, ty) => {
						let borrows = crate::lower::library::cow_borrows(scrut, elem);
						Expr::Cond(
							Box::new(borrows),
							Box::new(int.lit(discrs[0].1 as i128)),
							Box::new(int.lit(discrs[1].1 as i128)),
						)
					}
					Some(Std::Cow) => self.err(span, crate::lower::library::cow_variant(ty)),
					Some(std @ (Std::Bound | Std::TryLockError | Std::ErrorKind)) => {
						let table = discrs
							.iter()
							.map(|(i, d)| (Key::Name(adt.variant(*i).name.to_string()), int.lit(*d as i128)))
							.collect();
						let key = match std {
							Std::ErrorKind => scrut,
							_ => scrut.member("tag"),
						};
						Expr::Index(Box::new(Expr::Object(table)), Box::new(key))
					}
					None if self.b.t.own(adt.did()) => {
						let info = self.b.t.adt(*adt, span);
						let table = discrs
							.iter()
							.map(|(i, d)| (Key::Name(adt.variant(*i).name.to_string()), int.lit(*d as i128)))
							.collect();
						let key = match info.repr {
							AdtRepr::Tags => scrut,
							_ => scrut.member("tag"),
						};
						Expr::Index(Box::new(Expr::Object(table)), Box::new(key))
					}
					_ => self.err(span, format!("the discriminant of `{ty}`")),
				}
			}
			_ => self.err(span, format!("the discriminant of `{ty}`")),
		}
	}

	fn aggregate(
		&mut self,
		kind: &AggregateKind<'tcx>,
		ops: &IndexVec<FieldIdx, Operand<'tcx>>,
		ty: Ty<'tcx>,
		dest: Place<'tcx>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Option<Expr> {
		match kind {
			AggregateKind::Closure(def_id, args) => return Some(self.closure(*def_id, args, ops, span, out)),
			AggregateKind::Coroutine(def_id, args) => {
				return Some(super::coroutine::construct(self, *def_id, args, ops, span, out));
			}
			AggregateKind::CoroutineClosure(..) => return Some(self.err(span, "async closures are not supported")),
			AggregateKind::RawPtr(..) => return Some(self.err(span, "a raw pointer made from parts")),
			_ => {}
		}
		// A reference field read as its operand may be read more than once, or later: what
		// it borrows is evaluated now, once, as Rust does, unless reading it again is the same.
		let parts_read = dest.as_local().is_some_and(|local| self.parts_read(local));
		let mut values = Vec::with_capacity(ops.len());
		let mut rereadable = Vec::with_capacity(ops.len());
		for op in ops.iter() {
			let v = self.operand(op, span, out);
			let is_ref = op.ty(&self.mir.local_decls, self.tcx).is_ref();
			let reread = is_ref && v.is_path() && op.place().is_none_or(|place| self.rereads(place));
			let v = match parts_read && is_ref && !reread {
				true => self.path(v, "part", out),
				false => v,
			};
			values.push(v);
			rereadable.push(reread);
		}
		self.rereadable_parts = Some(rereadable);
		Some(match kind {
			AggregateKind::Tuple if values.is_empty() => Expr::undefined(),
			AggregateKind::Tuple => {
				let ts_ty = self.b.t.ty(ty, span);
				Expr::As(Box::new(Expr::Array(values)), ts_ty)
			}
			AggregateKind::Array(elem) => match elem.kind() {
				ty::Uint(ty::UintTy::U8) => Expr::ident("Uint8Array").member("of").call(values),
				_ => Expr::Array(values),
			},
			AggregateKind::Adt(def_id, variant, args, _, active) => {
				if let Some(active) = active {
					let ty::Adt(adt, _) = ty.kind() else {
						unreachable!("a union")
					};
					let parts = self.b.t.union_field(*adt, active.as_usize(), span);
					let field_ty = ops[FieldIdx::ZERO].ty(&self.mir.local_decls, self.tcx);
					let value = values.pop().expect("a union's field");
					let value = match parts.len() {
						0 | 1 => value,
						_ => self.path(value, "field", out),
					};
					let slots = self.b.t.slots_of(value, field_ty, &parts, span);
					let ts_ty = self.b.t.ty(ty, span);
					return Some(crate::lower::unions::made(slots, ts_ty));
				}
				if let Some(same) = self.rebuilt_result(kind, ops, &values, ty, span, out) {
					return Some(same);
				}
				self.adt(*def_id, *variant, args, values, ty, span)
			}
			_ => unreachable!(),
		})
	}

	/// The `Result` a variant of one is rebuilt from, if it takes what that same variant of
	/// another held, as `map_err` returns its `Ok`: that one's object, since a write into
	/// an `rt.Result`'s payload replaces the result, so none changes in place. The payload,
	/// held back for this one read, goes unread.
	fn rebuilt_result(
		&mut self,
		kind: &AggregateKind<'tcx>,
		ops: &IndexVec<FieldIdx, Operand<'tcx>>,
		values: &[Expr],
		ty: Ty<'tcx>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Option<Expr> {
		let &AggregateKind::Adt(def_id, variant, ..) = kind else {
			return None;
		};
		if Std::of(self.tcx, def_id) != Some(Std::Result) {
			return None;
		}
		let ([Operand::Copy(place) | Operand::Move(place)], [value]) = (&ops.raw[..], values) else {
			return None;
		};
		let (place, temp) = match self.def_rvalue(place.local) {
			Some(Rvalue::Use(Operand::Copy(from) | Operand::Move(from), _)) if place.projection.is_empty() => {
				(*from, Some(place.local))
			}
			_ => (*place, None),
		};
		let [
			ref outer @ ..,
			mir::ProjectionElem::Downcast(_, from),
			mir::ProjectionElem::Field(..),
		] = place.projection[..]
		else {
			return None;
		};
		let whole = Place {
			local: place.local,
			projection: self.tcx.mk_place_elems(outer),
		};
		// What the place holds now is what the payload was read from.
		let stable = self.frozen(whole) && self.defs[whole.local] + u32::from(self.is_arg(whole.local)) <= 1;
		let held = match variant.index() {
			0 => "value",
			_ => "error",
		};
		let payload = |e: &Expr| matches!(e, Expr::Member(_, Key::Name(field)) if field == held);
		// Only the payload read as it is: a copy the operand made must still be made.
		let pending = match (temp.and_then(|temp| self.var(temp)), value) {
			(_, value) if payload(value) => None,
			(Some(name), Expr::Ident(read)) if name == *read => Some(
				self.pending
					.iter()
					.position(|p| p.name == name && !p.effect && payload(&p.value))?,
			),
			_ => return None,
		};
		let whole_ty = whole.ty(&self.mir.local_decls, self.tcx).ty;
		if from != variant || !stable || self.b.t.std_of(whole_ty) != Some(Std::Result) {
			return None;
		}
		if let Some(i) = pending {
			self.pending.remove(i);
		}
		let same = self.place_expr(whole, span, out);
		let ts_ty = self.b.t.ty(ty, span);
		Some(match self.b.t.ty(whole_ty, span) == ts_ty {
			true => same,
			// Of the variant both share.
			false => Expr::As(Box::new(same), ts_ty),
		})
	}

	pub(super) fn adt(
		&mut self,
		def_id: rustc_hir::def_id::DefId,
		variant: VariantIdx,
		args: ty::GenericArgsRef<'tcx>,
		mut values: Vec<Expr>,
		ty: Ty<'tcx>,
		span: Span,
	) -> Expr {
		let tcx = self.tcx;
		let adt = tcx.adt_def(def_id);
		let def = adt.variant(variant);
		if let Some(std) = Std::of(tcx, def_id) {
			return match (std, variant.index()) {
				(Std::Option, 0) => self.b.t.none(args.type_at(0)),
				(Std::Poll, 1) => self.b.t.pending(args.type_at(0)),
				(Std::Option | Std::Poll | Std::Pin, _) => values.pop().unwrap(),
				// Either variant is its text.
				(Std::Cow, _) if crate::lower::library::string(tcx, ty) => values.pop().unwrap(),
				(Std::Cow, _) if let Some(elem) = crate::lower::library::cow_slice(tcx, ty) => {
					crate::lower::library::cow_made(elem, variant, values.pop().unwrap())
				}
				(Std::Result, i) => Expr::result(i == 0, values.pop().unwrap()),
				(Std::FmtError, _) => Expr::New(Box::new(Expr::Rt("FmtError".into())), Vec::new()),
				(Std::ParseBoolError, _) => Expr::New(Box::new(Expr::Rt("ParseBoolError".into())), Vec::new()),
				// `ResumeTy`, an async body's context.
				(Std::Context, _) => values.pop().unwrap_or_else(Expr::undefined),
				(Std::RangeTo | Std::RangeToInclusive, _) => {
					crate::lower::library::range_to(values.pop().unwrap(), std == Std::RangeToInclusive)
				}
				(Std::Reverse | Std::Wrapping | Std::Saturating, _) => values.pop().unwrap(),
				(Std::Bound | Std::TryLockError, _) => {
					let ts_ty = self.b.t.ty(ty, span);
					crate::lower::library::tagged(def.name.as_str(), values.pop(), ts_ty)
				}
				(Std::RangeFrom, _) => {
					values.extend(crate::lower::library::range_step(tcx, args.type_at(0), true));
					Expr::New(Box::new(Expr::Rt("RangeFrom".into())), values)
				}
				(Std::RangeFull, _) => Expr::undefined(),
				(Std::ParseIntError, _) => Expr::Object(vec![(Key::Name("kind".into()), values.pop().unwrap())]),
				(Std::ErrorKind, _) => Expr::As(Box::new(Expr::Str(def.name.to_string())), self.b.t.ty(ty, span)),
				(Std::Ordering, _) => crate::lower::collections::ordering_variant(variant),
				(Std::AtomicOrdering, _) => crate::lower::task::atomic_ordering(variant),
				(Std::ControlFlow, _) => super::call::control_flow(self, ty, variant, values),
				(Std::Range, _) => {
					values.extend(crate::lower::library::range_step(tcx, args.type_at(0), false));
					Expr::New(Box::new(Expr::Rt("Range".into())), values)
				}
				(Std::AssertKind, _) => match def.name.as_str() {
					"Eq" => Expr::Str("==".into()),
					"Ne" => Expr::Str("!=".into()),
					_ => self.err(span, format!("constructing `{ty}`")),
				},
				_ => self.err(span, format!("constructing `{ty}`")),
			};
		}
		if super::place::transparent(self.b.t, ty) || crate::lower::library::wrapped(tcx, ty).is_some() {
			return values.pop().unwrap_or_else(Expr::undefined);
		}
		let values = match self.b.zero_sized_std(ty, values, span) {
			Ok(made) => return made,
			Err(values) => values,
		};
		if !self.b.t.own(def_id) {
			return self.err(span, format!("constructing `{ty}` {}", self.b.t.foreign(def_id)));
		}
		let info = self.b.t.adt(adt, span);
		match info.repr {
			AdtRepr::Struct { ctor, fields } => {
				let field_arg = |i: usize, value: Expr| {
					let generic = tcx
						.type_of(def.fields[FieldIdx::from_usize(i)].did)
						.instantiate_identity()
						.skip_norm_wip();
					match ty::TypeVisitableExt::has_param(&generic) {
						true => value,
						false => value.typed(),
					}
				};
				let values = values.into_iter().enumerate().map(|(i, v)| field_arg(i, v)).collect();
				self.b.t.construct(info.sym, &ctor, &fields, values)
			}
			AdtRepr::Tags => {
				let ts_ty = self.b.t.ty(ty, span);
				Expr::As(Box::new(Expr::Str(def.name.to_string())), ts_ty)
			}
			AdtRepr::Slots(_) => unreachable!("a union is made by its field"),
			AdtRepr::Union => {
				let mut props = vec![(Key::Name("tag".into()), Expr::Str(def.name.to_string()))];
				for (i, value) in values.into_iter().enumerate() {
					let name = field_name(def.fields[FieldIdx::from_usize(i)].name.as_str(), i);
					props.push((Key::Name(name), value));
				}
				let _ = args;
				let ts_ty = self.b.t.ty(ty, span);
				Expr::As(Box::new(Expr::Object(props)), ts_ty)
			}
		}
	}

	/// A closure that captures nothing, as an arrow function.
	pub(super) fn bare_closure(
		&mut self,
		def_id: rustc_hir::def_id::DefId,
		args: ty::GenericArgsRef<'tcx>,
		span: Span,
	) -> Expr {
		self.closure(def_id, args, &IndexVec::new(), span, &mut Vec::new())
	}

	/// A closure, as an arrow function over what it captured.
	fn closure(
		&mut self,
		def_id: rustc_hir::def_id::DefId,
		args: ty::GenericArgsRef<'tcx>,
		ops: &IndexVec<FieldIdx, Operand<'tcx>>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Expr {
		let tcx = self.tcx;
		let instance = Instance::new_raw(def_id, args);
		let Some(mir) = super::instance_mir(self.b.t, instance, span) else {
			return Expr::undefined();
		};
		let rebound = rebound_captures(&mir);
		// One whose clone can't share it is made by a function of its captures, whose
		// parameters they are, with what they are now as its arguments.
		let made = self.b.t.clones_closure(def_id);
		let mut params = Vec::new();
		let mut values = Vec::new();
		// A capture by reference reads the place it borrows; one by value is the value as
		// it was when the closure was made, a reference held in a variable included, which
		// a closure that assigns it whole reassigns.
		let mut captures = Vec::with_capacity(ops.len());
		for (field, op) in ops.iter_enumerated() {
			let ty = op.ty(&self.mir.local_decls, tcx);
			if made {
				let name = self.b.fresh("captured");
				values.push(self.operand(op, span, out));
				params.push(Param {
					name: name.clone(),
					ty: Some(self.b.t.held_ty(ty, span)),
				});
				captures.push(Capture {
					value: Expr::ident(name),
					pointee: false,
				});
				continue;
			}
			// A `&mut` to a borrowed place the closure reads and writes as a JS closure does,
			// so it can replace an object there whole, as `mem::take` does.
			if let Some(local) = op.place().and_then(|p| p.as_local())
				&& matches!(ty.kind(), ty::Ref(_, _, rustc_ast::Mutability::Mut))
				&& self.alias_of_place(local)
				&& let Slot::Alias(Some(target)) = &self.slots[local]
			{
				captures.push(Capture {
					value: target.clone(),
					pointee: true,
				});
				continue;
			}
			let value = self.operand(op, span, out);
			let in_var = op
				.place()
				.and_then(|p| p.as_local())
				.is_some_and(|l| matches!(self.slots[l], Slot::Var(_)));
			let snapshot = match ty.kind() {
				ty::Ref(..) => in_var && !self.stable_name(&value),
				_ => true,
			};
			// A constant is its own snapshot, unless the closure reassigns it.
			let rebinds = rebound.contains(&field);
			let constant = value.is_constant();
			let value = match snapshot && (rebinds || !constant) {
				false => value,
				true => {
					let name = self.b.fresh("captured");
					// Typed, so a reassigned literal keeps its integer's type.
					let ts_ty = constant.then(|| self.b.t.held_ty(ty, span));
					let stmt = StmtKind::decl(rebinds, name.clone(), ts_ty, Some(value)).at(None);
					self.emit(vec![stmt], false, out);
					Expr::ident(name)
				}
			};
			captures.push(Capture { value, pointee: false });
		}
		let sig = tcx.instantiate_bound_regions_with_erased(args.as_closure().sig());
		// A value, whose `()` result is `undefined` as its type says.
		let ret = self
			.b
			.t
			.ty(sig.output(), mir.local_decls[mir::RETURN_PLACE].source_info.span);
		let ty = Ty::new_closure(tcx, def_id, args);
		let glue = match self.b.t.drops(ty) {
			true => Some(self.b.t.closure_glue(ty, &captures, span)),
			false => None,
		};
		let mut inner = Fx::new(&mut *self.b, &mir, Env::Closure(captures), span);
		let inputs = inner.params();
		let body = inner.lower();
		let mut f = Expr::Arrow(inputs, ret, body);
		let make = made.then(|| self.b.fresh("make"));
		if let Some(make) = &make {
			// Its clone, made again of each capture's, in order, as rustc's clones them.
			let upvars = args.as_closure().upvar_tys();
			let clones = std::iter::zip(&params, upvars)
				.map(|(param, ty)| {
					let value = Expr::ident(&param.name);
					match crate::lower::collections::clone_fn(self.b, ty, span) {
						Expr::Lit(l) if l == "undefined" => value,
						clone => clone.call(vec![value]),
					}
				})
				.collect();
			let again = Expr::Lambda(Vec::new(), Box::new(Expr::ident(make).call(clones)));
			f = Expr::rt("cloning", vec![f, again]);
		}
		// One that owns what drops carries the glue that drops it.
		if let Some(glue) = glue {
			f = Expr::rt("closure", vec![f, Expr::Arrow(Vec::new(), Type::Builtin("void"), glue)]);
		}
		let Some(make) = make else {
			return f;
		};
		let fn_ty = self.b.t.ty(ty, span);
		let made = Expr::Arrow(params, fn_ty, vec![StmtKind::Return(Some(f)).at(None)]);
		self.emit(
			vec![StmtKind::decl(false, make.clone(), None, Some(made)).at(None)],
			false,
			out,
		);
		Expr::ident(make).call(values)
	}
}

/// The captures a closure's body `mir` assigns whole or borrows mutably, other than to
/// drop them: what JS rebinds where Rust writes the closure's own copy.
fn rebound_captures(mir: &mir::Body<'_>) -> FxHashSet<FieldIdx> {
	struct Rebound(FxHashSet<FieldIdx>);
	impl<'tcx> Visitor<'tcx> for Rebound {
		fn visit_place(&mut self, place: &Place<'tcx>, cx: PlaceContext, _: mir::Location) {
			if place.local.as_u32() != 1
				|| !cx.is_mutating_use()
				|| cx == PlaceContext::MutatingUse(MutatingUseContext::Drop)
			{
				return;
			}
			let projection = match place.projection.as_slice() {
				[mir::ProjectionElem::Deref, rest @ ..] => rest,
				all => all,
			};
			if let [mir::ProjectionElem::Field(field, _)] = projection {
				self.0.insert(*field);
			}
		}
	}
	let mut rebound = Rebound(FxHashSet::default());
	rebound.visit_body(mir);
	rebound.0
}

/// Whether a `ty`'s JS value is the task's context: `&mut Context`, or the `ResumeTy` and
/// `NonNull` an async body passes it in.
pub(super) fn context_like<'tcx>(fx: &Fx<'_, '_, '_, 'tcx>, ty: Ty<'tcx>) -> bool {
	let tcx = fx.tcx;
	match ty.kind() {
		ty::Ref(_, inner, _) => context_like(fx, *inner),
		ty::Adt(adt, args) => match Std::of(tcx, adt.did()) {
			Some(Std::Context) => true,
			_ if super::place::transparent(fx.b.t, ty) => args.types().next().is_none_or(|t| context_like(fx, t)),
			_ => false,
		},
		_ => false,
	}
}

/// The part of `to` that unsizing `from` makes unsized, such as `[T]` from `[T; N]`.
fn unsized_to<'tcx>(from: Ty<'tcx>, to: Ty<'tcx>) -> Option<Ty<'tcx>> {
	match (from.kind(), to.kind()) {
		_ if from == to => None,
		(ty::Array(..), ty::Slice(_)) | (_, ty::Dynamic(..)) => Some(to),
		(ty::Ref(_, a, _) | ty::RawPtr(a, _), ty::Ref(_, b, _) | ty::RawPtr(b, _)) => unsized_to(*a, *b),
		(ty::Adt(a, a_args), ty::Adt(b, b_args)) if a.did() == b.did() => {
			a_args.types().zip(b_args.types()).find_map(|(a, b)| unsized_to(a, b))
		}
		_ => None,
	}
}

/// What an `Arc` unsized to an `Arc` of a trait object points at, and that object.
fn arc_object<'tcx>(tcx: TyCtxt<'tcx>, from: Ty<'tcx>, to: Ty<'tcx>) -> Option<(Ty<'tcx>, Ty<'tcx>)> {
	let (ty::Adt(adt, from_args), ty::Adt(_, to_args)) = (from.kind(), to.kind()) else {
		return None;
	};
	let object = to_args.type_at(0);
	(Std::of(tcx, adt.did()) == Some(Std::Arc) && object.is_trait()).then(|| (from_args.type_at(0), object))
}

/// Whether evaluating `rvalue` can do or throw anything.
pub fn effect<'tcx>(rvalue: &Rvalue<'tcx>, fx: &Fx<'_, '_, '_, 'tcx>) -> bool {
	match rvalue {
		Rvalue::BinaryOp(op, ops) => {
			let ty = ops.0.ty(&fx.mir.local_decls, fx.tcx);
			Int::of(ty.peel_refs()).is_some()
				&& !matches!(
					op,
					BinOp::Eq
						| BinOp::Ne
						| BinOp::Lt
						| BinOp::Le
						| BinOp::Gt
						| BinOp::Ge
						| BinOp::BitAnd
						| BinOp::BitOr
						| BinOp::BitXor
				)
		}
		Rvalue::UnaryOp(UnOp::Neg, _) => true,
		_ => false,
	}
}

/// Whether reading `p` reads memory something else could change.
fn place_memory<'tcx>(p: &Place<'tcx>, fx: &Fx<'_, '_, '_, 'tcx>) -> bool {
	!p.projection.is_empty()
		|| fx.borrowed[p.local]
		|| matches!(fx.slots[p.local], Slot::Alias(_))
		|| p.local.as_u32() == 1 && !matches!(fx.env, Env::Plain | Env::Shim)
}

/// Whether reading `op` reads memory something could change, a local changed in place
/// included: a copy of one held past a write to its field would see the write.
pub fn operand_memory<'tcx>(op: &Operand<'tcx>, fx: &Fx<'_, '_, '_, 'tcx>) -> bool {
	match op {
		Operand::Copy(p) | Operand::Move(p) => place_memory(p, fx) || fx.mutated[p.local],
		_ => false,
	}
}

/// Whether evaluating `rvalue` reads memory that something else could change.
pub fn memory<'tcx>(rvalue: &Rvalue<'tcx>, fx: &Fx<'_, '_, '_, 'tcx>) -> bool {
	let place = |p: &Place<'tcx>| place_memory(p, fx);
	let operand = |op: &Operand<'tcx>| operand_memory(op, fx);
	match rvalue {
		Rvalue::Use(op, _) | Rvalue::Repeat(op, _) | Rvalue::Cast(_, op, _) | Rvalue::UnaryOp(_, op) => operand(op),
		Rvalue::BinaryOp(_, ops) => operand(&ops.0) || operand(&ops.1),
		Rvalue::Ref(_, _, p) | Rvalue::Discriminant(p) | Rvalue::CopyForDeref(p) | Rvalue::RawPtr(_, p) => place(p),
		Rvalue::Aggregate(_, ops) => ops.iter().any(operand),
		Rvalue::ThreadLocalRef(_) | Rvalue::WrapUnsafeBinder(..) | Rvalue::Reborrow(..) => true,
	}
}

/// A pattern type's base, or `ty` itself.
fn peel_pat(ty: Ty<'_>) -> Ty<'_> {
	match ty.kind() {
		ty::Pat(base, _) => *base,
		_ => ty,
	}
}
