//! Places, operands, and constants.
//!
//! A reference is its referent in JS, so `Deref` reads nothing and a field of a borrowed
//! struct is a member of the same object. A reference assigned once from a place that
//! stays put reads as that place, which is how a `&mut` to a number changes the number.

use rustc_abi::{BackendRepr, FieldIdx, Size, VariantIdx};
use rustc_data_structures::fx::FxHashSet;
use rustc_hir::def::DefKind;
use rustc_hir::def_id::{DefId, DefIndex};
use rustc_middle::mir::interpret::{AllocId, GlobalAlloc, GlobalId, Scalar, alloc_range};
use rustc_middle::mir::visit::{MutatingUseContext, PlaceContext, Visitor};
use rustc_middle::mir::{self, ConstValue, Local, Operand, Place, PlaceTy, ProjectionElem};
use rustc_middle::ty::{self, Instance, Ty, TyCtxt, TypeVisitableExt};
use rustc_span::Span;

use super::{Env, Fx, Pending, Slot};
use crate::lower::library::{self, Flow, Std};
use crate::lower::num::Int;
use crate::lower::{AdtRepr, ObjectKind, byte_array, bytes_ref};
use crate::program::{SymKind, Symbol};
use crate::ts::{Expr, Item, Stmt, StmtKind, SymId};

impl<'m, 'b, 'a, 'tcx> Fx<'m, 'b, 'a, 'tcx> {
	/// A local read whole.
	pub(super) fn local_expr(&mut self, local: Local, span: Span) -> Expr {
		if local.as_u32() == 1 {
			match &self.env {
				Env::Coroutine => return Expr::This,
				Env::Closure(_) => return self.err(span, "a closure's environment read whole"),
				Env::Plain | Env::Shim => {}
			}
		}
		if let Some(value) = self.stepped_value(local) {
			return value;
		}
		match &self.slots[local] {
			Slot::Var(name) => Expr::ident(name),
			// A value of type `!`, as a generic instantiated at `!` reads: native code never
			// gets here, and `tsc` sees that through `rt.unreachable`.
			Slot::Unit if self.mir.local_decls[local].ty.is_never() => Expr::rt("unreachable", vec![Expr::undefined()]),
			// A function item holds no value, so each read names the function.
			Slot::Unit if let ty::FnDef(def_id, args) = *self.mir.local_decls[local].ty.kind() => {
				self.b.fn_value(def_id, args.skip_binder(), span)
			}
			Slot::Unit => Expr::undefined(),
			// Passed on or kept, a `&mut` to a borrowed place JS copies is its `rt.Place`.
			Slot::Alias(Some(target)) if self.points_at_place(local) => {
				let target = target.clone();
				let payload = match self.def_rvalue(local) {
					Some(mir::Rvalue::Ref(_, _, p)) => self.payload_ty(*p, span),
					_ => None,
				};
				self.place_of(target, payload)
			}
			Slot::Alias(Some(target)) => target.clone(),
			Slot::Alias(None) => self.err(span, "a reference read before its borrow"),
			Slot::Discr(_) | Slot::Flag(_) => self.err(span, "a discriminant read other than by a switch"),
		}
	}

	/// The JS expression of `place`, read without a copy.
	pub fn place_expr(&mut self, place: Place<'tcx>, span: Span, out: &mut Vec<Stmt>) -> Expr {
		let tcx = self.tcx;
		// A local held as its parts reads them, as far as they go.
		if self.scalars.contains_key(&place.local) {
			let (e, n) = self.scalar_place(place, span);
			let mut pty = PlaceTy::from_ty(self.mir.local_decls[place.local].ty);
			for elem in &place.projection[..n] {
				pty = pty.projection_ty(tcx, *elem);
			}
			return self.project(e, pty, place, n, span, out);
		}
		let decls = &self.mir.local_decls;
		let mut pty = PlaceTy::from_ty(decls[place.local].ty);
		let mut projection = place.projection.iter().peekable();
		let held = match projection.peek() {
			Some(&ProjectionElem::Field(field, _)) if !self.mutated[place.local] => {
				self.held_part(place.local, field.as_usize())
			}
			_ => None,
		};
		// A closure's captures, and a coroutine's state, are `_1`'s fields.
		let e = match (&self.env, place.local.as_u32()) {
			(Env::Closure(captures), 1) => {
				let captures = captures.clone();
				if matches!(projection.peek(), Some(ProjectionElem::Deref)) {
					pty = pty.projection_ty(tcx, ProjectionElem::<Local, Ty<'tcx>>::Deref);
					projection.next();
				}
				match projection.next() {
					Some(elem @ ProjectionElem::Field(field, _)) => {
						pty = pty.projection_ty(tcx, elem);
						let Some(capture) = captures.get(field.as_usize()) else {
							return self.err(span, "a capture the closure doesn't have");
						};
						let (value, pointee) = (capture.value.clone(), capture.pointee);
						match projection.peek() {
							Some(ProjectionElem::Deref) if pointee => {
								pty = pty.projection_ty(tcx, ProjectionElem::<Local, Ty<'tcx>>::Deref);
								projection.next();
								value
							}
							// Passed on or kept whole, it is the place's `rt.Place`, or the
							// object there.
							None if pointee && self.b.t.place_pointee(pty.ty).is_some() => self.place_of(value, None),
							_ => value,
						}
					}
					_ => return self.err(span, "a closure's environment read whole"),
				}
			}
			// A field of an aggregate held back for its one read.
			_ if let Some(part) = held => {
				let elem = *projection.peek().unwrap();
				pty = pty.projection_ty(tcx, elem);
				projection.next();
				part
			}
			// A field of an aggregate made once and never changed is its operand: a
			// reference, held in a path when the aggregate is made unless it reads the same
			// later, or a name that stays put.
			(_, _)
				if let Some(parts) = self.var_parts.get(&place.local)
					&& let Some(ProjectionElem::Field(field, field_ty)) = projection.peek()
					&& let Some(part) = parts.get(field.as_usize())
					&& (matches!(field_ty.kind(), ty::Ref(..)) || part.is_constant() || self.stable_name(part)) =>
			{
				let part = part.clone();
				let elem = *projection.peek().unwrap();
				pty = pty.projection_ty(tcx, elem);
				projection.next();
				part
			}
			// An alias of a borrowed place is that place, which the dereference reads.
			_ if matches!(projection.peek(), Some(ProjectionElem::Deref)) && self.points_at_place(place.local) => {
				let Slot::Alias(Some(target)) = &self.slots[place.local] else {
					return self.err(span, "a reference read before its borrow");
				};
				let target = target.clone();
				pty = pty.projection_ty(tcx, ProjectionElem::<Local, Ty<'tcx>>::Deref);
				projection.next();
				target
			}
			_ => self.local_expr(place.local, span),
		};
		// Checked arithmetic's pair is its value, and `.1` is whether it overflowed, which
		// it didn't: it would have panicked.
		if self.checked.contains(&place.local)
			&& let Some(ProjectionElem::Field(field, _)) = projection.peek()
		{
			let field = *field;
			projection.next();
			if field.as_u32() == 1 {
				return Expr::lit("false");
			}
			pty = PlaceTy::from_ty(match decls[place.local].ty.kind() {
				ty::Tuple(tys) => tys[0],
				_ => unreachable!("a checked pair is a tuple"),
			});
		}
		let from = place.projection.len() - projection.len();
		self.project(e, pty, place, from, span, out)
	}

	/// `e`, the value of `place` projected as far as `from`, a `pty`, projected the rest of
	/// the way.
	fn project(
		&mut self,
		mut e: Expr,
		mut pty: PlaceTy<'tcx>,
		place: Place<'tcx>,
		from: usize,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Expr {
		let tcx = self.tcx;
		let decls = &self.mir.local_decls;
		// Whether `e` is an `rt.ByteSlice` of the place's bytes: a `[u8]`, or a `[u8; N]`
		// behind a reference.
		let mut in_bytes = false;
		// The projections a union's field consumed with it, down to a slot.
		let mut consumed = 0;
		for (read, elem) in place.projection.iter().enumerate().skip(from) {
			let ty = pty.ty;
			if consumed > 0 {
				consumed -= 1;
				pty = pty.projection_ty(tcx, elem);
				continue;
			}
			in_bytes = in_bytes || byte_slice(ty);
			match elem {
				ProjectionElem::Field(field, field_ty)
					if let ty::Adt(adt, _) = ty.kind()
						&& adt.is_union() =>
				{
					let (parts, n, part_ty) =
						self.union_part(*adt, field.as_usize(), field_ty, &place.projection[read + 1..], span);
					consumed = n;
					e = self.b.t.of_slots(e, part_ty, &parts, span);
				}
				ProjectionElem::Deref => {
					// Drop glue's pointer is the value it drops.
					let glue =
						matches!(self.env, Env::Shim) && place.local.as_u32() == 1 && pty.ty == decls[place.local].ty;
					// A box's pointer is the box, which is its contents.
					if self.b.t.place_pointee(ty).is_some() && !glue && !self.box_pointer(place.local) {
						e = e.member("v");
					}
				}
				ProjectionElem::Field(field, _) => {
					e = self.field(e, ty, pty.variant_index, field, span);
				}
				ProjectionElem::Downcast(_, variant) => {
					let base = Place {
						local: place.local,
						projection: tcx.mk_place_elems(&place.projection[..read]),
					};
					if self.narrowing.stale(self.at, base) {
						e = self.as_stale_variant(e, base, ty, variant, span);
					} else if !e.path_key().is_some_and(|key| self.switched.contains(&(key, ty))) {
						e = self.as_variant(e, ty, variant, span);
					}
				}
				ProjectionElem::Index(index) => {
					let i = self.local_expr(index, span);
					e = match in_bytes {
						true => {
							let base = self.slice_path(e, out);
							byte_at(base, i)
						}
						false => e.index(i),
					};
				}
				ProjectionElem::ConstantIndex {
					offset,
					min_length: _,
					from_end,
				} => {
					let base = match from_end || in_bytes {
						true => self.slice_path(e, out),
						false => e,
					};
					let i = match from_end {
						false => Expr::lit(offset.to_string()),
						true => super::scalar::sub(slice_member(base.clone(), "length"), Expr::lit(offset.to_string())),
					};
					e = match in_bytes {
						true => byte_at(base, i),
						false => base.index(i),
					};
				}
				// A slice pattern's rest: `[a, rest @ ..]`.
				ProjectionElem::Subslice {
					from,
					to,
					from_end: true,
				} if byte_slice(ty) => {
					let base = self.slice_path(e, out);
					// A range of parts, which the pattern that took it checked the length for.
					if let Some([data, start, length]) = made_slice(&base) {
						let len = super::scalar::sub(length.clone(), Expr::lit((from + to).to_string()));
						let start = super::scalar::add(start.clone(), Expr::lit(from.to_string()));
						e = Expr::New(Box::new(Expr::Rt("ByteSlice".into())), vec![data.clone(), start, len]);
					} else {
						let mut values = vec![base.clone(), Expr::lit(from.to_string())];
						if to != 0 {
							values.push(base.member("length").binary("-", Expr::lit(to.to_string())));
						}
						e = Expr::rt("byteSliceRange", values);
					}
				}
				// Part of a `[u8; N]` behind a reference, as `[a, rest @ ..]` of a `&[u8; N]`.
				ProjectionElem::Subslice { from, to, from_end } if in_bytes => {
					let end = match (from_end, ty.kind()) {
						(false, _) => to,
						(true, ty::Array(_, len)) if let Some(len) = len.try_to_target_usize(tcx) => len - to,
						_ => return self.err(span, format!("a subslice of `{ty}`")),
					};
					let base = self.slice_path(e, out);
					e = match made_slice(&base) {
						// In bounds of the array's.
						Some([data, start, _]) => Expr::New(
							Box::new(Expr::Rt("ByteSlice".into())),
							vec![
								data.clone(),
								super::scalar::add(start.clone(), Expr::lit(from.to_string())),
								Expr::lit((end - from).to_string()),
							],
						),
						None => {
							let values = vec![base, Expr::lit(from.to_string()), Expr::lit(end.to_string())];
							Expr::rt("byteSliceRange", values)
						}
					};
				}
				ProjectionElem::Subslice { from, to, from_end } => {
					let elem_ty = match ty.kind() {
						ty::Array(elem, _) | ty::Slice(elem) => *elem,
						_ => return self.err(span, format!("a subslice of `{ty}`")),
					};
					let method = match elem_ty.kind() {
						ty::Uint(ty::UintTy::U8) => "subarray",
						_ => "slice",
					};
					let base = self.path(e, "slice", out);
					let end = self.subslice_end(&base, ty, to, from_end);
					e = base.member(method).call(vec![Expr::lit(from.to_string()), end]);
				}
				ProjectionElem::OpaqueCast(_)
				| ProjectionElem::UnwrapUnsafeBinder(_)
				| ProjectionElem::PhantomDeref => {}
			}
			in_bytes = match elem {
				ProjectionElem::Deref => bytes_ref(ty),
				ProjectionElem::Subslice { .. } => in_bytes,
				_ => false,
			};
			pty = pty.projection_ty(tcx, elem);
		}
		e
	}

	/// The parts of field `field` of union `adt` that `rest`, the projections after it,
	/// go down to, how many of them that takes, and the type they make: one slot, or the
	/// parts of an aggregate kept in several.
	pub(super) fn union_part(
		&mut self,
		adt: ty::AdtDef<'tcx>,
		field: usize,
		field_ty: Ty<'tcx>,
		rest: &[ProjectionElem<Local, Ty<'tcx>>],
		span: Span,
	) -> (Vec<crate::lower::unions::Part>, usize, Ty<'tcx>) {
		let mut parts = self.b.t.union_field(adt, field, span);
		let (mut n, mut ty) = (0, field_ty);
		while !crate::lower::unions::whole(&parts)
			&& let Some(ProjectionElem::Field(inner, inner_ty)) = rest.get(n)
		{
			parts = crate::lower::unions::within(&parts, inner.as_usize());
			(n, ty) = (n + 1, *inner_ty);
		}
		(parts, n, ty)
	}

	/// The union field part `place` ends in, if it ends in one kept in more than one slot:
	/// the union's place and the parts.
	pub(super) fn union_whole(
		&mut self,
		place: Place<'tcx>,
		span: Span,
	) -> Option<(Place<'tcx>, Vec<crate::lower::unions::Part>)> {
		let tcx = self.tcx;
		for (i, (base, elem)) in place.iter_projections().enumerate() {
			let ProjectionElem::Field(field, field_ty) = elem else {
				continue;
			};
			let ty::Adt(adt, _) = base.ty(&self.mir.local_decls, tcx).ty.kind() else {
				continue;
			};
			if !adt.is_union() {
				continue;
			}
			let rest = &place.projection[i + 1..];
			let (parts, n, _) = self.union_part(*adt, field.as_usize(), field_ty, rest, span);
			let whole = crate::lower::unions::whole(&parts);
			return (n == rest.len() && !whole).then(|| (base.to_place(tcx), parts));
		}
		None
	}

	/// Whether `place` is a `[u8; N]` read through a reference, whose JS value is an
	/// `rt.ByteSlice` of its bytes rather than a `Uint8Array`, as [`Self::place_expr`]
	/// reads it.
	pub(super) fn in_bytes(&self, place: Place<'tcx>) -> bool {
		let mut in_bytes = false;
		for (base, elem) in place.iter_projections() {
			let ty = base.ty(&self.mir.local_decls, self.tcx).ty;
			in_bytes = match elem {
				ProjectionElem::Deref => bytes_ref(ty),
				ProjectionElem::Subslice { .. } => in_bytes || byte_slice(ty),
				_ => false,
			};
		}
		in_bytes && byte_array(place.ty(&self.mir.local_decls, self.tcx).ty)
	}

	/// Where a slice pattern's rest of `base`, a `ty`, ends: at `to`, or `to` before its end.
	fn subslice_end(&self, base: &Expr, ty: Ty<'tcx>, to: u64, from_end: bool) -> Expr {
		match (from_end, ty.kind()) {
			(false, _) => Expr::lit(to.to_string()),
			(true, ty::Array(_, len)) if let Some(len) = len.try_to_target_usize(self.tcx) => {
				Expr::lit((len - to).to_string())
			}
			(true, _) => base.clone().member("length").binary("-", Expr::lit(to.to_string())),
		}
	}

	/// A mutable borrow of a slice pattern's rest of other than bytes, which a JS array's
	/// `slice` would copy: the runtime's view of it.
	pub(super) fn mut_subslice(
		&mut self,
		kind: mir::BorrowKind,
		place: Place<'tcx>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Option<Expr> {
		if !matches!(kind, mir::BorrowKind::Mut { .. }) {
			return None;
		}
		let (base, ty, from, to, from_end) = place.iter_projections().find_map(|(base, elem)| match elem {
			ProjectionElem::Subslice { from, to, from_end } => {
				let ty = base.ty(&self.mir.local_decls, self.tcx).ty;
				match ty.kind() {
					ty::Array(elem, _) | ty::Slice(elem) if !matches!(elem.kind(), ty::Uint(ty::UintTy::U8)) => {
						Some((base, ty, from, to, from_end))
					}
					_ => None,
				}
			}
			_ => None,
		})?;
		if base.projection.len() + 1 != place.projection.len() {
			return Some(self.err(span, format!("a mutable borrow within a subslice of `{ty}`")));
		}
		let base = self.place_expr(base.to_place(self.tcx), span, out);
		let base = self.path(base, "slice", out);
		let end = self.subslice_end(&base, ty, to, from_end);
		Some(Expr::rt("sliceView", vec![base, Expr::lit(from.to_string()), end]))
	}

	/// `base`, a `ty`, cast to its `variant`, which MIR reads without a switch where the
	/// others can't be, as a derived `==` reads the other operand after comparing
	/// discriminants. `tsc` narrows only what a switch tests.
	fn as_variant(&mut self, base: Expr, ty: Ty<'tcx>, variant: VariantIdx, span: Span) -> Expr {
		match ty.kind() {
			ty::Adt(adt, _)
				if self.b.t.crates.translates(adt.did()) || Std::of(self.tcx, adt.did()) == Some(Std::Result) =>
			{
				match self.variant_ty(ty, variant, span) {
					Some(variant) => Expr::As(Box::new(base), variant),
					None => base,
				}
			}
			_ => base,
		}
	}

	/// `base`, the `place` of a `ty`, cast to its `variant` through its whole type:
	/// TypeScript may have narrowed it to another before a change it doesn't see.
	fn as_stale_variant(
		&mut self,
		base: Expr,
		place: Place<'tcx>,
		ty: Ty<'tcx>,
		variant: VariantIdx,
		span: Span,
	) -> Expr {
		// `rt.Entry` is one class for either variant, which TypeScript narrows nothing by.
		if matches!(ty.kind(), ty::Adt(adt, _) if matches!(Std::of(self.tcx, adt.did()), Some(Std::Entry))) {
			return base;
		}
		let Some(variant) = self.variant_ty(ty, variant, span) else {
			return self.err(
				span,
				format!("a payload of `{ty}` read where TypeScript narrowed it to another variant"),
			);
		};
		let whole = match place.as_local() {
			Some(local) => self.decl_ty(local),
			None => self.b.t.ty(ty, span),
		};
		Expr::As(Box::new(Expr::As(Box::new(base), whole)), variant)
	}

	/// The JS type of `ty`'s `variant`, for a type whose variants hold their payloads:
	/// a union's member, `Result`'s and `ControlFlow`'s, or `Option`'s and `Poll`'s value.
	fn variant_ty(&mut self, ty: Ty<'tcx>, variant: VariantIdx, span: Span) -> Option<crate::ts::Type> {
		let tcx = self.tcx;
		let ty::Adt(adt, args) = ty.kind() else {
			return None;
		};
		// `rt.Result`'s variants.
		let result = |ok: bool, payload| {
			let (flag, name) = match ok {
				true => ("true", "value"),
				false => ("false", "error"),
			};
			crate::ts::Type::Object(vec![
				("ok".into(), crate::ts::Type::Builtin(flag)),
				(name.into(), payload),
			])
		};
		match Std::of(tcx, adt.did()) {
			Some(Std::Option | Std::Poll) => {
				let payload = adt.variant(variant).fields.iter().next()?;
				Some(self.b.t.ty(payload.ty(tcx, args).skip_norm_wip(), span))
			}
			Some(Std::Result) => {
				let payload = self.b.t.ty(args.type_at(variant.index()), span);
				Some(result(variant.index() == 0, payload))
			}
			// What `control_flow_field` reads of each variant.
			Some(Std::ControlFlow) => {
				let (breaks, continues) = (args.type_at(0), args.type_at(1));
				Some(match (Flow::of(tcx, args), variant.index()) {
					(Flow::Option, 0) => self.b.t.ty(continues, span),
					(Flow::Option, _) => match self.b.t.erases_to(continues).undefined {
						true => crate::ts::Type::Rt("None", vec![]),
						false => crate::ts::Type::Builtin("undefined"),
					},
					(Flow::Result | Flow::Other, 0) => result(true, self.b.t.ty(continues, span)),
					(Flow::Result, _) => self.b.t.ty(breaks, span),
					(Flow::Other, _) => result(false, self.b.t.ty(breaks, span)),
				})
			}
			None if self.b.t.own(adt.did()) && matches!(self.b.t.adt(*adt, span).repr, AdtRepr::Union) => {
				let def = adt.variant(variant);
				let mut fields = vec![("tag".to_string(), crate::ts::Type::StrLit(def.name.to_string()))];
				for (i, field) in def.fields.iter().enumerate() {
					let ty = self.b.t.ty(field.ty(tcx, args).skip_norm_wip(), span);
					fields.push((crate::lower::field_name(field.name.as_str(), i), ty));
				}
				Some(crate::ts::Type::Object(fields))
			}
			_ => None,
		}
	}

	/// A field of `base`, a `ty`, in `variant` if it was downcast to one.
	fn field(&mut self, base: Expr, ty: Ty<'tcx>, variant: Option<VariantIdx>, field: FieldIdx, span: Span) -> Expr {
		let tcx = self.tcx;
		match ty.kind() {
			ty::Tuple(_) => base.index(Expr::lit(field.index().to_string())),
			ty::Coroutine(def_id, args) => {
				let info = super::coroutine::info(self.b.t, *def_id, args, span);
				match variant {
					None => base.field(info.upvars[field.index()]),
					Some(variant) => {
						let saved = info.layout_variant_field(variant, field);
						base.field(info.saved[saved])
					}
				}
			}
			ty::Adt(adt, _) => match Std::of(tcx, adt.did()) {
				Some(
					Std::Option
					| Std::Poll
					| Std::Box
					| Std::Pin
					| Std::Reverse
					| Std::NonZero
					| Std::Wrapping
					| Std::Saturating
					| Std::PoisonError,
				) => base,
				// Cast to its variant, which a test narrows only where it reads the field.
				Some(Std::Bound | Std::TryLockError) => {
					let ty::Adt(_, args) = ty.kind() else { unreachable!() };
					let variant = adt.variant(variant.unwrap_or(rustc_abi::FIRST_VARIANT));
					let held = variant.fields[field].ty(tcx, args).skip_norm_wip();
					let fields = vec![
						("tag".to_string(), crate::ts::Type::StrLit(variant.name.to_string())),
						("_0".to_string(), self.b.t.ty(held, span)),
					];
					Expr::As(Box::new(base), crate::ts::Type::Object(fields)).member("_0")
				}
				Some(Std::Result) => match variant.map_or(0, VariantIdx::index) {
					0 => base.member("value"),
					_ => base.member("error"),
				},
				Some(Std::Range | Std::RangeInclusive | Std::RangeTo | Std::RangeFrom | Std::RangeToInclusive) => {
					base.member(adt.non_enum_variant().fields[field].name.as_str())
				}
				Some(Std::Context | Std::NonNull | Std::Entry) => base,
				// `TryFromIntError`'s `IntErrorKind`.
				Some(Std::TryFromIntError) => base.member("kind"),
				// The runtime's field of each of std's errors has its name.
				Some(std) if std.error() => {
					base.member(crate::lower::camel(adt.non_enum_variant().fields[field].name.as_str()))
				}
				// Cast to its variant's class: the slice an `rt.Borrowed` wraps, or the `Vec`.
				Some(Std::Cow) if let Some(elem) = crate::lower::library::cow_slice(tcx, ty) => {
					let bytes = matches!(elem.kind(), ty::Uint(ty::UintTy::U8));
					let owned = variant.is_some_and(|v| v.index() == 1);
					let class = match (bytes, owned) {
						(true, false) => crate::ts::Type::Rt("ByteSlice", Vec::new()),
						(true, true) => crate::ts::Type::Rt("ByteVec", Vec::new()),
						(false, false) => crate::ts::Type::Rt("Borrowed", vec![self.b.t.ty(elem, span)]),
						(false, true) => crate::ts::Type::Array(Box::new(self.b.t.ty(elem, span))),
					};
					let cast = Expr::As(Box::new(base), class);
					match bytes || owned {
						true => cast,
						false => cast.member("slice"),
					}
				}
				Some(Std::Cow) => self.err(span, crate::lower::library::cow_variant(ty)),
				Some(Std::ControlFlow) => super::call::control_flow_field(self, base, ty, variant),
				_ if transparent(self.b.t, ty) || crate::lower::library::wrapped(tcx, ty).is_some() => base,
				None if self.b.t.own(adt.did()) => {
					let info = self.b.t.adt(*adt, span);
					match info.repr {
						AdtRepr::Struct { fields, .. } => base.field(fields[field.index()]),
						AdtRepr::Slots(_) => {
							self.err(span, format!("a field of union `{ty}` read apart from its place"))
						}
						AdtRepr::Union | AdtRepr::Tags => {
							let variant = variant.unwrap_or(rustc_abi::FIRST_VARIANT);
							let def = &adt.variant(variant).fields[field];
							let name = crate::lower::field_name(def.name.as_str(), field.index());
							base.member(name)
						}
					}
				}
				_ => self.err(span, format!("a field of `{ty}`")),
			},
			_ => self.err(span, format!("a field of `{ty}`")),
		}
	}

	/// An operand's value, copied if it is a `Copy` aggregate read from a place that keeps it.
	pub fn operand(&mut self, op: &Operand<'tcx>, span: Span, out: &mut Vec<Stmt>) -> Expr {
		match op {
			// An array of bytes read through a reference, copied out of its slice.
			Operand::Copy(place) | Operand::Move(place) if self.in_bytes(*place) => {
				let value = self.place_expr(*place, span, out);
				Expr::rt("byteSliceToArray", vec![value])
			}
			// JS shares objects where Rust copies, so a `Copy` aggregate is copied where
			// either side could change in place: into a local changed in place, or out of a
			// place that could change.
			Operand::Copy(place) => {
				let ty = place.ty(&self.mir.local_decls, self.tcx).ty;
				// Optimized MIR also copies what isn't `Copy`, where it is a move: copy
				// propagation does in a body without cleanups.
				if changes_captures(self.tcx, ty) && self.b.t.is_copy(ty) {
					return self.err(span, "a copy of a closure that changes what it captured");
				}
				let value = self.place_expr(*place, span, out);
				match self.force_share || self.into_frozen && self.frozen(*place) {
					true => value,
					false => self.b.t.copied(value, ty, span),
				}
			}
			// Moved out of a local's own value changed in place, which is its own.
			Operand::Move(place) => {
				let ty = place.ty(&self.mir.local_decls, self.tcx).ty;
				let value = self.place_expr(*place, span, out);
				match self.into_frozen || super::own(*place) && self.mutated[place.local] {
					true => value,
					false => self.b.t.copied(value, ty, span),
				}
			}
			Operand::Constant(c) => self.constant(c, span),
			Operand::RuntimeChecks(_) => Expr::lit("false"),
		}
	}

	/// Whether `e` names a local nothing assigns again or changes in place, so reading it
	/// later reads what it held earlier.
	pub(super) fn stable_name(&self, e: &Expr) -> bool {
		let Expr::Ident(name) = e else {
			return false;
		};
		self.local_named(name).is_some_and(|local| {
			let assigned = self.defs[local] + u32::from(self.is_arg(local));
			assigned <= 1 && !self.mutated[local]
		})
	}

	/// Whether `place` is part of a local's own value that nothing changes in place.
	pub(super) fn frozen(&self, place: Place<'tcx>) -> bool {
		!self.mutated[place.local] && place.projection.iter().all(|e| !matches!(e, ProjectionElem::Deref))
	}

	/// The statements that assign `value` to `place`, which has projections.
	pub(super) fn store(&mut self, place: Place<'tcx>, value: Expr, span: Span) -> Vec<Stmt> {
		let (place, value) = self.rebuilt(place, value, span);
		let mut out = Vec::new();
		// A raw pointer to a value of no bytes may dangle, and a write through it writes nothing.
		if let [.., ProjectionElem::Deref] = place.projection.as_slice()
			&& matches!(self.mir.local_decls[place.local].ty.kind(), ty::RawPtr(..))
			&& self.b.t.zero_sized(place.ty(&self.mir.local_decls, self.tcx).ty)
		{
			return out;
		}
		// A union's field written whole is written into each slot it is kept in.
		if let Some((union, parts)) = self.union_whole(place, span) {
			let ty = place.ty(&self.mir.local_decls, self.tcx).ty;
			let value = self.path(value, "field", &mut out);
			let target = self.place_expr(union, span, &mut out);
			let target = self.path(target, "union", &mut out);
			for (slot, part) in self.b.t.slots_of(value, ty, &parts, span) {
				out.push(StmtKind::Expr(target.clone().member(slot).assign(part)).at(self.b.t.loc(span)));
			}
			return out;
		}
		// Bytes through a reference are copied into the slice of them it holds.
		if self.in_bytes(place) {
			let target = self.place_expr(place, span, &mut out);
			let target = self.slice_path(target, &mut out);
			let set = target.clone().member("data").member("set");
			out.push(StmtKind::Expr(set.call(vec![value, target.member("start")])).at(self.b.t.loc(span)));
			return out;
		}
		// `*r = v` replaces the object `r` points at in place, which other references and
		// captures share. An alias of a borrowed place assigns the place, and an `rt.Place`
		// its `v`.
		if let Some(pointee) = self.pointee_object(place) {
			let target = self.local_expr(place.local, span);
			let raw =
				matches!(self.mir.local_decls[place.local].ty.kind(), ty::RawPtr(..)) && !self.box_pointer(place.local);
			let e = match raw {
				true => Expr::rt("ptrWrite", vec![target, value]),
				false => match self.write_in(pointee) {
					Some(ObjectKind::Bytes) => target.member("set").call(vec![value]),
					// A `Vec<u8>` is an `rt.ByteVec`, and any other a JS array, as a boxed slice
					// is, whose length a write changes, unlike a tuple's or an array's.
					Some(ObjectKind::Vec) if !byte_vec(self.erased(pointee)) => {
						Expr::rt("vecAssign", vec![target, value])
					}
					Some(ObjectKind::Array) if self.erased(pointee).is_slice() => {
						Expr::rt("vecAssign", vec![target, value])
					}
					Some(_) => Expr::ident("Object").member("assign").call(vec![target, value]),
					None => self.err(span, format!("an assignment through a reference to `{pointee}`")),
				},
			};
			out.push(StmtKind::Expr(e).at(None));
			return out;
		}
		let target = self.place_expr(place, span, &mut out);
		out.push(StmtKind::Expr(target.assign(value)).at(self.b.t.loc(span)));
		out
	}

	/// What `place` is, where it is `*r` for an `r` that is the object it points at, with
	/// no place known to hold it: neither an alias of a borrowed place, nor an `rt.Place`,
	/// nor a closure's environment, nor an alias of a reference to no object, such as a
	/// `&mut &str`, which only an alias of the place it borrows can be.
	pub(super) fn pointee_object(&self, place: Place<'tcx>) -> Option<Ty<'tcx>> {
		let [ProjectionElem::Deref] = place.projection.as_slice() else {
			return None;
		};
		let pointee = place.ty(&self.mir.local_decls, self.tcx).ty;
		let ty = self.mir.local_decls[place.local].ty;
		let lent = matches!(self.slots[place.local], Slot::Alias(_))
			&& !matches!(ty.kind(), ty::RawPtr(..))
			&& self.b.t.object_kind(pointee).is_none();
		let ok = !self.alias_of_place(place.local)
			&& !lent
			&& self.b.t.place_pointee(ty).is_none()
			&& !(place.local.as_u32() == 1 && matches!(self.env, Env::Closure(_)));
		ok.then_some(pointee)
	}

	/// Whether `place` is `*r` for an `r` that is the object it points at, which a whole
	/// value of it, not of bytes, is written into in place, with no place known to hold it.
	pub(super) fn in_object(&self, place: Place<'tcx>) -> bool {
		let raw = matches!(self.mir.local_decls[place.local].ty.kind(), ty::RawPtr(..));
		self.pointee_object(place)
			.and_then(|pointee| self.write_in(pointee))
			.is_some_and(|kind| kind != ObjectKind::Bytes && !raw)
	}

	/// The kind of a `pointee`'s object, through the `Pin`s and `Box`es JS erases, where a
	/// whole value of it can be written into the object in place: one only its owner
	/// holds, unlike an `Arc`'s, which its clones share, and whose parts `Object.assign`
	/// copies, unlike a binding's, an `rt.PollFn`'s or a future's private fields, or a
	/// trait object's, whatever class it is.
	pub(super) fn write_in(&self, pointee: Ty<'tcx>) -> Option<ObjectKind> {
		let erased = self.erased(pointee);
		let hidden = match erased.kind() {
			ty::Adt(adt, _) => {
				self.b.t.bound_drops(adt.did()).is_some() || Std::of(self.tcx, adt.did()) == Some(Std::PollFn)
			}
			ty::Dynamic(..) | ty::Closure(..) => true,
			// A `Uint8Array`'s length never changes, as a boxed slice's may.
			ty::Slice(elem) => matches!(elem.kind(), ty::Uint(ty::UintTy::U8)),
			_ => false,
		};
		self.b.t.object_kind(pointee)?;
		match self.b.t.object_kind(erased)? {
			_ if hidden || shared(self, pointee) => None,
			ObjectKind::Closure => None,
			kind => Some(kind),
		}
	}

	/// What JS keeps of a `ty`: what it holds through any number of `Pin`s and `Box`es.
	fn erased(&self, ty: Ty<'tcx>) -> Ty<'tcx> {
		let mut erased = ty;
		while let ty::Adt(adt, args) = erased.kind()
			&& matches!(Std::of(self.tcx, adt.did()), Some(Std::Pin | Std::Box))
		{
			erased = args.type_at(0);
		}
		erased
	}

	/// A store of `value` to `place` as a store of the whole value to what holds it, where
	/// `place`, or what a reference to it reads, is the payload of an `Option`, `Poll`,
	/// `Result`, or `ControlFlow`: JS shares their values as immutable, so a payload
	/// changes by making its enum again.
	fn rebuilt(&mut self, place: Place<'tcx>, value: Expr, span: Span) -> (Place<'tcx>, Expr) {
		let tcx = self.tcx;
		let (mut at, mut value, mut rebuilt) = (place, value, false);
		loop {
			if let [ProjectionElem::Deref, rest @ ..] = at.projection.as_slice()
				&& self.alias_of_place(at.local)
				&& let Some(mir::Rvalue::Ref(_, _, borrowed)) = self.def_rvalue(at.local)
			{
				at = borrowed.project_deeper(rest, tcx);
				continue;
			}
			if let [ProjectionElem::Deref, rest @ ..] = at.projection.as_slice()
				&& let Some(&borrowed) = self.reborrows.get(&at.local)
			{
				at = borrowed.project_deeper(rest, tcx);
				continue;
			}
			let [.., ProjectionElem::Downcast(_, variant), ProjectionElem::Field(..)] = at.projection.as_slice() else {
				break;
			};
			let base = Place {
				local: at.local,
				projection: tcx.mk_place_elems(&at.projection[..at.projection.len() - 2]),
			};
			let ty = base.ty(&self.mir.local_decls, tcx).ty;
			let ty::Adt(adt, args) = ty.kind() else {
				break;
			};
			if !matches!(
				Std::of(tcx, adt.did()),
				Some(Std::Option | Std::Poll | Std::Result | Std::ControlFlow)
			) {
				break;
			}
			value = self.adt(adt.did(), *variant, args, vec![value], ty, span);
			(at, rebuilt) = (base, true);
		}
		match rebuilt {
			true => (at, value),
			false => (place, value),
		}
	}

	/// Whether `local` is an alias of a borrowed place, as `&mut x` or `&mut v[i]`, rather
	/// than a copy of a reference or a reborrow of what one points at.
	pub(super) fn alias_of_place(&self, local: Local) -> bool {
		matches!(self.slots[local], Slot::Alias(_))
			&& match self.def_rvalue(local) {
				// A reborrow of what an alias of a borrowed place points at borrows that place.
				Some(mir::Rvalue::Ref(_, _, p)) => match p.projection.as_slice() {
					[ProjectionElem::Deref] => self.points_at_place(p.local),
					[.., ProjectionElem::Deref] => false,
					_ => true,
				},
				// A copy of a `&mut` the closure captured by reference.
				Some(mir::Rvalue::CopyForDeref(p) | mir::Rvalue::Use(Operand::Copy(p) | Operand::Move(p), _)) => {
					self.pointee_capture(*p).is_some()
				}
				Some(_) => false,
				// An element `index_mut` borrows.
				None => true,
			}
	}

	/// The place `place` borrows, if it is a `&mut` the closure captured by reference.
	pub(super) fn pointee_capture(&self, place: Place<'tcx>) -> Option<Expr> {
		let Env::Closure(captures) = &self.env else {
			return None;
		};
		let ([ProjectionElem::Field(field, _)] | [ProjectionElem::Deref, ProjectionElem::Field(field, _)]) =
			place.projection.as_slice()
		else {
			return None;
		};
		let capture = captures
			.get(field.as_usize())
			.filter(|c| c.pointee && place.local.as_u32() == 1)?;
		Some(capture.value.clone())
	}

	/// Whether `local` is an `rt.Place` reference that aliases the place it borrows, so its
	/// target is that place rather than the `rt.Place`.
	pub(super) fn points_at_place(&self, local: Local) -> bool {
		self.b.t.place_pointee(self.mir.local_decls[local].ty).is_some() && self.alias_of_place(local)
	}

	/// The `rt.Place` of `target`, the JS path of the borrowed `place`, fixed where it is now.
	pub(super) fn make_place(&mut self, place: Place<'tcx>, target: Expr, span: Span, out: &mut Vec<Stmt>) -> Expr {
		let target = self.stable_target(place, target, span, out);
		let payload = self.payload_ty(place, span);
		self.place_of(target, payload)
	}

	/// The type of `place` if it is a variant's payload, whose JS path is its whole enum,
	/// as `Some`'s is the `Option`.
	pub(super) fn payload_ty(&mut self, place: Place<'tcx>, span: Span) -> Option<crate::ts::Type> {
		let downcast = place.iter_projections().any(|(base, e)| {
			matches!(e, ProjectionElem::Downcast(..)) && base.ty(&self.mir.local_decls, self.tcx).ty.is_enum()
		});
		downcast.then(|| self.b.t.ty(place.ty(&self.mir.local_decls, self.tcx).ty, span))
	}

	/// `target`, the JS path of the borrowed `place`, with the objects and index it goes
	/// through held where they are now, as the borrow fixes them, for an `rt.Place` that
	/// reads and writes it later.
	pub(super) fn stable_target(&mut self, place: Place<'tcx>, target: Expr, span: Span, out: &mut Vec<Stmt>) -> Expr {
		// A store into a `Result`'s value makes the `Result` again, which a path can't.
		for (base, elem) in place.iter_projections() {
			if let ProjectionElem::Downcast(..) = elem
				&& let Some(std @ (Std::Result | Std::ControlFlow)) =
					self.b.t.std_of(base.ty(&self.mir.local_decls, self.tcx).ty)
			{
				return self.err(
					span,
					format!("a mutable reference into a `{std:?}`'s payload, passed on or kept"),
				);
			}
		}
		if super::own(place) {
			return self.stable_path(target, true, span, out);
		}
		// A field or element of what one reference points at: only the reference can change
		// while the borrow lives.
		let through_one = match place.projection.as_slice() {
			[ProjectionElem::Deref, inner @ .., last] => {
				!matches!(last, ProjectionElem::Deref)
					&& inner
						.iter()
						.all(|e| matches!(e, ProjectionElem::Field(..) | ProjectionElem::Downcast(..)))
			}
			_ => false,
		};
		let reference = match &self.slots[place.local] {
			Slot::Var(name) if through_one && starts_from(&target, name) => name.clone(),
			_ => return self.stable_path(target, false, span, out),
		};
		let held = self.held_now(Expr::ident(&reference), out);
		let target = from_held(target, &held);
		self.stable_path(target, true, span, out)
	}

	/// `target`, a path, with each index it goes through held where it is now, and the
	/// object too unless it is `fixed`, which can't change while borrowed.
	pub(super) fn stable_path(&mut self, target: Expr, fixed: bool, span: Span, out: &mut Vec<Stmt>) -> Expr {
		self.materialize(&target, out);
		match target {
			Expr::Located(_, e) => self.stable_path(*e, fixed, span, out),
			// A payload `Option::insert` lends, read as its type.
			Expr::As(e, ty) => Expr::As(Box::new(self.stable_path(*e, fixed, span, out)), ty),
			Expr::Ident(_) | Expr::This => target,
			Expr::Member(object, key) => Expr::Member(Box::new(self.stable_object(*object, fixed, out)), key),
			Expr::Index(object, i) => {
				let object = self.stable_object(*object, fixed, out);
				let i = self.held_now(*i, out);
				Expr::Index(Box::new(object), Box::new(i))
			}
			_ => self.err(span, "a mutable reference passed on or kept, to a value with no place"),
		}
	}

	/// `object`, a path's object, held where it is now unless it is `fixed`, in which case
	/// only the indices it goes through are, as the borrow fixes the objects but not the
	/// variables they are indexed by.
	fn stable_object(&mut self, object: Expr, fixed: bool, out: &mut Vec<Stmt>) -> Expr {
		if !fixed {
			return self.held_now(object, out);
		}
		match object {
			Expr::Located(loc, e) => Expr::Located(loc, Box::new(self.stable_object(*e, true, out))),
			Expr::As(e, ty) => Expr::As(Box::new(self.stable_object(*e, true, out)), ty),
			Expr::Member(e, key) => Expr::Member(Box::new(self.stable_object(*e, true, out)), key),
			Expr::Index(e, i) => {
				let e = self.stable_object(*e, true, out);
				let i = self.held_now(*i, out);
				Expr::Index(Box::new(e), Box::new(i))
			}
			e => e,
		}
	}

	/// `e`, evaluated now into a constant unless reading it later reads the same.
	fn held_now(&mut self, e: Expr, out: &mut Vec<Stmt>) -> Expr {
		if e.is_constant() || matches!(e, Expr::This) || self.stable_name(&e) {
			return e;
		}
		let name = self.b.fresh("borrowed");
		let stmt = StmtKind::decl(false, name.clone(), None, Some(e)).at(None);
		self.emit(vec![stmt], false, out);
		Expr::ident(name)
	}

	/// The `rt.Place` that reads and writes `target`, a path that stays put, read as
	/// `payload` if it is a variant's.
	pub(super) fn place_of(&mut self, target: Expr, payload: Option<crate::ts::Type>) -> Expr {
		let value = self.b.fresh("value");
		let param = crate::ts::Param {
			name: value.clone(),
			ty: None,
		};
		let read = match payload {
			Some(ty) => Expr::As(Box::new(target.clone()), ty),
			None => target.clone(),
		};
		let get = Expr::Lambda(Vec::new(), Box::new(read));
		let set = Expr::Lambda(vec![param], Box::new(target.assign(Expr::ident(value))));
		Expr::rt("place", vec![get, set])
	}

	/// The `rt.Place` of `place`, a reborrow `&mut *r` of a reference `r` that is one, which is `r`'s.
	/// A shared reborrow `&*r` is what `r` points at instead.
	pub(super) fn reborrowed(
		&mut self,
		kind: mir::BorrowKind,
		place: Place<'tcx>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Option<Expr> {
		if !matches!(kind, mir::BorrowKind::Mut { .. }) {
			return None;
		}
		let [rest @ .., ProjectionElem::Deref] = place.projection.as_slice() else {
			return None;
		};
		let base = Place {
			local: place.local,
			projection: self.tcx.mk_place_elems(rest),
		};
		let base_ty = base.ty(&self.mir.local_decls, self.tcx).ty;
		if self.b.t.place_pointee(base_ty).is_none() || base.as_local().is_some_and(|l| self.alias_of_place(l)) {
			return None;
		}
		Some(self.place_expr(base, span, out))
	}

	/// `SetDiscriminant`: a coroutine's state.
	pub(super) fn set_discriminant(&mut self, place: Place<'tcx>, variant: VariantIdx, span: Span) -> Option<Stmt> {
		let ty = place.ty(&self.mir.local_decls, self.tcx).ty;
		let ty::Coroutine(def_id, args) = ty.kind() else {
			self.err(span, format!("setting the discriminant of `{ty}`"));
			return None;
		};
		let info = super::coroutine::info(self.b.t, *def_id, args, span);
		let mut out = Vec::new();
		let target = self.place_expr(place, span, &mut out);
		Some(StmtKind::Expr(target.field(info.state).assign(Expr::lit(variant.as_u32().to_string()))).at(None))
	}

	/// The test that `scrut`, a `ty`, has the variant whose discriminant is `value`.
	pub(super) fn variant_test(&mut self, scrut: &Expr, ty: Ty<'tcx>, value: u128, span: Span) -> Expr {
		let tcx = self.tcx;
		match ty.kind() {
			ty::Coroutine(def_id, args) => {
				let info = super::coroutine::info(self.b.t, *def_id, args, span);
				scrut
					.clone()
					.field(info.state)
					.binary("===", Expr::lit(value.to_string()))
			}
			ty::Adt(adt, args) if adt.is_enum() => {
				let Some((index, _)) = adt.discriminants(tcx).find(|(_, d)| d.val == value) else {
					return self.err(span, format!("no variant of `{ty}` has discriminant {value}"));
				};
				match Std::of(tcx, adt.did()) {
					Some(Std::Option)
						if let Expr::Ident(n) = scrut
							&& let Some(flag) = self.range_flags.get(n) =>
					{
						let flag = Expr::ident(flag);
						match index.index() {
							0 => flag.not(),
							_ => flag,
						}
					}
					Some(Std::Option) if matches!(scrut, Expr::Ident(n) if self.next_names.contains(n)) => {
						match index.index() {
							0 => scrut.clone().binary("===", Expr::Rt("DONE".into())),
							_ => scrut.clone().binary("!==", Expr::Rt("DONE".into())),
						}
					}
					Some(Std::Option) => match index.index() {
						0 => scrut.clone().binary("===", self.b.t.none(args.type_at(0))),
						_ => scrut.clone().binary("!==", self.b.t.none(args.type_at(0))),
					},
					Some(Std::Poll) => match index.index() {
						0 => scrut.clone().binary("!==", self.b.t.pending(args.type_at(0))),
						_ => scrut.clone().binary("===", self.b.t.pending(args.type_at(0))),
					},
					Some(Std::Result) => match index.index() {
						0 => scrut.clone().member("ok"),
						_ => scrut.clone().member("ok").not(),
					},
					None if self.b.t.own(adt.did()) => {
						let info = self.b.t.adt(*adt, span);
						let name = Expr::Str(adt.variant(index).name.to_string());
						match info.repr {
							AdtRepr::Tags => scrut.clone().binary("===", name),
							AdtRepr::Union => scrut.clone().member("tag").binary("===", name),
							AdtRepr::Struct { .. } | AdtRepr::Slots(_) => self.err(span, format!("a switch on `{ty}`")),
						}
					}
					Some(Std::ControlFlow) => super::call::control_flow_test(self, scrut, ty, index),
					Some(Std::Entry) => crate::lower::collections::entry_test(*adt, index, scrut.clone()),
					Some(Std::Cow) if let Some(elem) = crate::lower::library::cow_slice(tcx, ty) => {
						let borrows = crate::lower::library::cow_borrows(scrut.clone(), elem);
						match index.index() {
							0 => borrows,
							_ => borrows.not(),
						}
					}
					Some(Std::Cow) => self.err(span, crate::lower::library::cow_variant(ty)),
					Some(Std::Ordering) => scrut
						.clone()
						.binary("===", crate::lower::collections::ordering_variant(index)),
					Some(Std::AtomicOrdering) => {
						scrut.clone().binary("===", crate::lower::task::atomic_ordering(index))
					}
					Some(Std::Bound | Std::TryLockError) => scrut
						.clone()
						.member("tag")
						.binary("===", Expr::Str(adt.variant(index).name.to_string())),
					Some(Std::ErrorKind) => scrut
						.clone()
						.binary("===", Expr::Str(adt.variant(index).name.to_string())),
					_ => self.err(span, format!("a switch on `{ty}`")),
				}
			}
			_ => self.err(span, format!("a switch on the discriminant of `{ty}`")),
		}
	}

	/// A constant operand's value.
	pub fn constant(&mut self, c: &mir::ConstOperand<'tcx>, span: Span) -> Expr {
		let tcx = self.tcx;
		let env = self.b.t.typing_env_pub();
		if let Some(key) = self.local_key_of(c) {
			return self.local_key(key, span);
		}
		// A reference to a function or a closure that captures nothing, as `&f` promotes to,
		// is what it points at, which has no value tree.
		if let ty::Ref(_, inner, _) = c.ty().kind()
			&& matches!(inner.kind(), ty::FnDef(..) | ty::Closure(..))
		{
			return self.const_value(ConstValue::ZeroSized, *inner, span);
		}
		match c.const_ {
			mir::Const::Ty(ty, ct) => match ct.kind() {
				ty::ConstKind::Value(v) => self.b.value(v.ty, v.valtree, span),
				_ => self.err(span, format!("a constant of type `{ty}` that is no value")),
			},
			mir::Const::Unevaluated(uv, ty) => {
				let instance = match Instance::try_resolve(tcx, env, uv.def, uv.args) {
					Ok(Some(instance)) => instance,
					_ => return self.err(span, "a constant that does not resolve"),
				};
				let cid = GlobalId {
					instance,
					promoted: uv.promoted,
				};
				// A cell's value tree is only its value, so one holding a cell, and one with no
				// value tree, is read from the memory it evaluated to.
				let freeze = ty.has_param() || ty.is_freeze(tcx, env);
				match freeze.then(|| tcx.const_eval_global_id_for_typeck(env, cid, span)) {
					Some(Ok(Ok(valtree))) => self.b.value(ty, valtree, span),
					Some(Err(_)) => self.err(span, "a constant that failed to evaluate"),
					None | Some(Ok(Err(_))) => match tcx.const_eval_global_id(env, cid, span) {
						Ok(val) => self.const_value(val, ty, span),
						Err(_) => self.err(span, "a constant that failed to evaluate"),
					},
				}
			}
			mir::Const::Val(val, ty) => self.const_value(val, ty, span),
		}
	}

	/// The `const` item of the thread-local `c` borrows, as `with`'s `&'static self` is: a
	/// promoted borrow of the key.
	fn local_key_of(&self, c: &mir::ConstOperand<'tcx>) -> Option<DefId> {
		let tcx = self.tcx;
		let ty::Ref(_, key, _) = c.ty().kind() else {
			return None;
		};
		let ty::Adt(adt, _) = key.kind() else {
			return None;
		};
		if Std::of(tcx, adt.did()) != Some(Std::LocalKey) {
			return None;
		}
		let mir::Const::Unevaluated(uv, _) = c.const_ else {
			return None;
		};
		let promoted = &tcx.promoted_mir(uv.def)[uv.promoted?];
		promoted
			.basic_blocks
			.iter()
			.flat_map(|bb| &bb.statements)
			.find_map(|s| {
				let mir::StatementKind::Assign(assign) = &s.kind else {
					return None;
				};
				match &assign.1 {
					mir::Rvalue::Use(mir::Operand::Constant(k), _) => match k.const_ {
						mir::Const::Unevaluated(k, _) if k.promoted.is_none() => Some(k.def),
						_ => None,
					},
					_ => None,
				}
			})
	}

	/// The symbol of the module value that stands for `item`, named for it, and whether
	/// this made it, so that the caller adds its item.
	fn module_value(&mut self, item: DefId) -> (SymId, bool) {
		if let Some(&sym) = self.b.t.values.get(&item) {
			return (sym, false);
		}
		let tcx = self.tcx;
		let sym = self.b.t.program.sym(Symbol {
			module: self.b.t.module_of(item),
			base: tcx.item_name(item).to_string(),
			// A static of a function, by the items it is nested in.
			suffix: crate::lower::names::enclosing(tcx, item).join("_"),
			owner: None,
			kind: SymKind::Value,
			order: self.b.t.order(tcx.def_span(item)),
			span: tcx.def_span(item),
			instance: String::new(),
			name: String::new(),
			alias: None,
		});
		self.b.t.values.insert(item, sym);
		(sym, true)
	}

	/// The module value of the thread-local `key`, made the first time: an `rt.LocalKey`
	/// over what initializes it.
	fn local_key(&mut self, key: DefId, span: Span) -> Expr {
		let tcx = self.tcx;
		// Before its initializer, which may read it.
		let (sym, new) = self.module_value(key);
		if !new {
			return Expr::Sym(sym);
		}
		let key_ty = tcx.type_of(key).instantiate_identity().skip_norm_wip();
		let ty::Adt(_, args) = key_ty.kind() else {
			unreachable!("a thread-local's key is a LocalKey")
		};
		let value_ty = args.type_at(0);
		let (init, lazy) = self.local_init(key, span);
		let mut ctor = vec![init, Expr::lit(if lazy { "true" } else { "false" })];
		// What drops the value an initializer made from within itself, which only an
		// expression's can.
		if lazy {
			ctor.extend(self.b.t.drop_fn(value_ty, span));
		}
		let ty = self.b.t.ty(key_ty, span);
		self.b.t.program.items.push(Item::Value {
			sym,
			ty,
			value: Expr::New(Box::new(Expr::Rt("LocalKey".into())), ctor),
			doc: None,
		});
		Expr::Sym(sym)
	}

	/// What makes the value of the thread-local `key`, and whether it is lazy: the closure
	/// its expression is in, or else, for a `const` one, its `const`, as a function.
	fn local_init(&mut self, key: DefId, span: Span) -> (Expr, bool) {
		let tcx = self.tcx;
		let accessor = tcx
			.mir_for_ctfe(key)
			.local_decls
			.iter()
			.find_map(|l| match l.ty.kind() {
				ty::Closure(def_id, _) => Some(*def_id),
				_ => None,
			});
		let Some(accessor) = accessor else {
			return (self.err(span, "a thread-local without its accessor"), false);
		};
		// `LazyStorage::get`'s function, which calls the one the expression is in.
		let lazy = tcx.optimized_mir(accessor).basic_blocks.iter().find_map(|bb| {
			let mir::TerminatorKind::Call { args, .. } = &bb.terminator().kind else {
				return None;
			};
			args.iter().find_map(|a| match a.node.constant()?.ty().kind() {
				ty::Closure(def_id, args) => Some((*def_id, *args)),
				_ => None,
			})
		});
		if let Some((def_id, args)) = lazy {
			return (self.bare_closure(def_id, args, span), true);
		}
		// A `const` one's `EagerStorage` holds the value of the `const` the macro declares in
		// the key's own initializer, so defined after the key and before its accessor. The
		// storage's own MIR isn't there to read when the key is another crate's.
		let init = (key.index.as_u32() + 1..accessor.index.as_u32())
			.map(|i| DefId {
				krate: key.krate,
				index: DefIndex::from_u32(i),
			})
			.find(|&d| tcx.opt_parent(d) == Some(key) && matches!(tcx.def_kind(d), DefKind::Const));
		match init {
			Some(init) => (Expr::Sym(self.b.t.instance(Instance::mono(tcx, init))), false),
			None => (
				self.err(span, "a thread-local whose initializer translation can't find"),
				false,
			),
		}
	}

	/// The module value of the static `item`, which `ty` borrows whole, made the first time
	/// from the memory it starts with, which a static has even when it is another crate's:
	/// one value every use shares, which only its cells change.
	fn static_value(&mut self, item: DefId, ty: Ty<'tcx>, span: Span) -> Expr {
		let tcx = self.tcx;
		let static_ty = tcx.type_of(item).instantiate_identity().skip_norm_wip();
		if tcx.is_mutable_static(item) {
			return self.err(span, "a mutable static, which any code may write");
		}
		let static_ty = tcx.erase_and_anonymize_regions(static_ty);
		if ty.builtin_deref(true).map(|t| tcx.erase_and_anonymize_regions(t)) != Some(static_ty) {
			return self.err(span, format!("a constant of type `{ty}` into a static"));
		}
		// A raw pointer to a value JS copies is a place, which the value is not.
		if self.b.t.raw_place(ty).is_some() {
			return self.err(
				span,
				format!("a constant of type `{ty}` into a static of a value JS copies"),
			);
		}
		if let Some(&sym) = self.b.t.values.get(&item) {
			return self.static_use(sym);
		}
		let Ok(alloc) = tcx.eval_static_initializer(item) else {
			return self.err(span, "a static that failed to evaluate");
		};
		if !self.b.t.reading.insert(item) {
			return self.err(span, "a static whose value points into itself, before it is made");
		}
		// rustc reads a constant's memory only if it is immutable, so read an immutable copy.
		let mut memory = alloc.inner().clone();
		memory.mutability = rustc_ast::Mutability::Not;
		let alloc_id = tcx.reserve_and_set_memory_alloc(tcx.mk_const_alloc(memory));
		let val = ConstValue::Indirect {
			alloc_id,
			offset: Size::ZERO,
		};
		let value = match byte_array(static_ty) {
			true => Expr::rt("byteSliceFrom", vec![self.const_value(val, static_ty, span)]),
			false => self.const_value(val, static_ty, span),
		};
		self.b.t.reading.remove(&item);
		let (sym, new) = self.module_value(item);
		if new {
			let ty = self.b.t.ty(ty, span);
			// Another static or a class it reads may not be made yet when its module is: one of
			// a later line, or of a module that imports this one, as Rust's modules may. It is
			// then made on first use, which nothing can tell from being made first.
			let syms = &self.b.t.program.syms;
			let mut reads = false;
			value.eager_syms(&mut |id| reads |= matches!(syms[id].kind, SymKind::Value | SymKind::Struct));
			let (value, ty) = match reads {
				true => {
					self.b.t.deferred.insert(sym);
					let make = Expr::Arrow(Vec::new(), ty.clone(), vec![StmtKind::Return(Some(value)).at(None)]);
					let lazy = Expr::New(Box::new(Expr::Rt("LazyCell".into())), vec![make]);
					(lazy, crate::ts::Type::Rt("LazyCell", vec![ty]))
				}
				false => (value, ty),
			};
			self.b.t.program.items.push(Item::Value {
				sym,
				ty,
				value,
				doc: None,
			});
		}
		self.static_use(sym)
	}

	/// A use of the static `sym`: its module value, or what that makes on first use.
	fn static_use(&self, sym: SymId) -> Expr {
		match self.b.t.deferred.contains(&sym) {
			true => Expr::Sym(sym).member("force").call(Vec::new()),
			false => Expr::Sym(sym),
		}
	}

	/// The value of a constant `val` of type `ty`: a fresh one at each use, as Rust copies a
	/// constant there.
	pub(super) fn const_value(&mut self, val: ConstValue, ty: Ty<'tcx>, span: Span) -> Expr {
		let tcx = self.tcx;
		if let Some(value) = self.cell_value(val, ty, span) {
			return value;
		}
		// An aggregate, field by field.
		if let ty::Adt(..) | ty::Tuple(_) | ty::Array(..) = ty.kind()
			&& !matches!(ty.kind(), ty::Tuple(tys) if tys.is_empty())
			&& !matches!(ty.kind(), ty::Adt(adt, _) if adt.is_struct() && self.b.t.own(adt.did()) && adt.all_fields().next().is_none())
		{
			let Some(d) = tcx.try_destructure_mir_constant_for_user_output(val, ty) else {
				return self.err(span, format!("a constant of type `{ty}` that can't be taken apart"));
			};
			let values: Vec<Expr> = d.fields.iter().map(|(v, t)| self.const_value(*v, *t, span)).collect();
			return match ty.kind() {
				ty::Tuple(_) => {
					let ts_ty = self.b.t.ty(ty, span);
					Expr::As(Box::new(Expr::Array(values)), ts_ty)
				}
				ty::Array(elem, _) => match elem.kind() {
					ty::Uint(ty::UintTy::U8) => Expr::ident("Uint8Array").member("of").call(values),
					_ => Expr::Array(values),
				},
				ty::Adt(adt, args) => self.adt(
					adt.did(),
					d.variant.unwrap_or(rustc_abi::FIRST_VARIANT),
					args,
					values,
					ty,
					span,
				),
				_ => unreachable!(),
			};
		}
		match val {
			ConstValue::ZeroSized => match *ty.kind() {
				ty::FnDef(def_id, args) => self.b.fn_value(def_id, args.skip_binder(), span),
				ty::Tuple(tys) if tys.is_empty() => Expr::undefined(),
				ty::Adt(adt, _) if adt.is_struct() && self.b.t.own(adt.did()) => {
					let info = self.b.t.adt(adt, span);
					self.b.t.construct(info.sym, &crate::ts::Ctor::Unit, &[], Vec::new())
				}
				ty::Array(..) => Expr::Array(Vec::new()),
				ty::Closure(def_id, args) => self.bare_closure(def_id, args, span),
				ty::Adt(adt, _) if Std::of(tcx, adt.did()) == Some(Std::RangeFull) => Expr::undefined(),
				_ => self.err(span, format!("a zero-sized constant of type `{ty}`")),
			},
			ConstValue::Scalar(Scalar::Int(int)) => match ty.kind() {
				ty::RawPtr(..) if int.is_null() => Expr::Rt("NULL".into()),
				ty::RawPtr(..) => self.err(span, format!("a constant of type `{ty}` made from an integer")),
				ty::Bool => Expr::lit((int.to_bits_unchecked() != 0).to_string()),
				ty::Char => Expr::lit(int.to_u32().to_string()),
				_ if let Some(float) = crate::lower::float::Float::of(ty) => float.constant(int),
				_ => match Int::of(ty) {
					Some(i) => {
						let size = rustc_abi::Size::from_bits(i.bits);
						let value = match i.signed {
							true => int.to_int(size),
							false => int.to_uint(size) as i128,
						};
						i.lit(value)
					}
					None => self.err(span, format!("a scalar constant of type `{ty}`")),
				},
			},
			ConstValue::Scalar(Scalar::Ptr(ptr, _)) => {
				let (prov, offset) = ptr.prov_and_relative_offset();
				let alloc = match tcx.global_alloc(prov.alloc_id()) {
					GlobalAlloc::Static(item) => {
						return match offset.bytes() {
							0 => self.static_value(item, ty, span),
							_ => self.err(span, format!("a constant of type `{ty}` into a static")),
						};
					}
					GlobalAlloc::Memory(alloc) => alloc,
					GlobalAlloc::Function { instance } if offset.bytes() == 0 => {
						return self.fn_pointer(instance, span);
					}
					_ => return self.err(span, format!("a constant of type `{ty}`")),
				};
				// `&[u8; N]`, such as a byte string.
				let ty::Ref(_, inner, _) = ty.kind() else {
					return self.err(
						span,
						format!("a pointer constant of type `{ty}` into memory translation can't name"),
					);
				};
				let unnamed = format!("a constant of type `{ty}`, a pointer into memory translation can't name");
				let ty::Array(elem, len) = inner.kind() else {
					return self.err(span, unnamed);
				};
				let (Some(len), ty::Uint(ty::UintTy::U8)) = (len.try_to_target_usize(tcx), elem.kind()) else {
					return self.err(span, unnamed);
				};
				let start = offset.bytes() as usize;
				bytes_lit(
					alloc
						.inner()
						.inspect_with_uninit_and_ptr_outside_interpreter(start..start + len as usize),
				)
			}
			ConstValue::Slice { .. } => {
				let Some(bytes) = val.try_get_slice_bytes_for_diagnostics(tcx) else {
					return self.err(span, "a slice constant out of bounds");
				};
				match ty.peel_refs().kind() {
					ty::Str => match std::str::from_utf8(bytes) {
						Ok(s) => Expr::Str(s.to_string()),
						Err(_) => self.err(span, "a string constant that is not UTF-8"),
					},
					ty::Slice(elem) if matches!(elem.kind(), ty::Uint(ty::UintTy::U8)) => bytes_lit(bytes),
					_ => self.err(span, format!("a slice constant of type `{ty}`")),
				}
			}
			ConstValue::Indirect { alloc_id, offset } => match self.loaded(alloc_id, offset, ty) {
				Some(val) => self.const_value(val, ty, span),
				None => self.err(span, format!("a constant of type `{ty}` held in memory")),
			},
		}
	}

	/// The value a `ty` that is no aggregate holds at `offset` in `alloc_id`, as rustc would
	/// give it outside of memory: nothing, a scalar, or a slice from the start of another.
	fn loaded(&self, alloc_id: AllocId, offset: Size, ty: Ty<'tcx>) -> Option<ConstValue> {
		let tcx = self.tcx;
		let env = self.b.t.typing_env_pub();
		let layout = tcx.layout_of(env.as_query_input(ty)).ok()?;
		if layout.is_zst() {
			return Some(ConstValue::ZeroSized);
		}
		let GlobalAlloc::Memory(alloc) = tcx.global_alloc(alloc_id) else {
			return None;
		};
		match layout.backend_repr {
			BackendRepr::Scalar(_) => self
				.scalar(ConstValue::Indirect { alloc_id, offset }, ty)
				.map(ConstValue::Scalar),
			// A `&str` or `&[T]`: its pointer, to the start of another allocation, and length.
			BackendRepr::ScalarPair { .. } if matches!(ty.builtin_deref(false)?.kind(), ty::Str | ty::Slice(_)) => {
				let size = tcx.data_layout.pointer_size();
				let ptr = alloc.inner().read_scalar(&tcx, alloc_range(offset, size), true).ok()?;
				let len = alloc
					.inner()
					.read_scalar(&tcx, alloc_range(offset + size, size), false)
					.ok()?;
				let Scalar::Ptr(ptr, _) = ptr else {
					return None;
				};
				let (prov, start) = ptr.prov_and_relative_offset();
				(start == Size::ZERO).then(|| ConstValue::Slice {
					alloc_id: prov.alloc_id(),
					meta: len.to_target_usize(&tcx).discard_err().unwrap_or(0),
				})
			}
			_ => None,
		}
	}

	/// A function pointer's value: the function or closure `instance` calls.
	fn fn_pointer(&mut self, instance: Instance<'tcx>, span: Span) -> Expr {
		let tcx = self.tcx;
		match instance.def {
			ty::InstanceKind::Item(def_id) if tcx.is_closure_like(def_id) => {
				self.bare_closure(def_id, instance.args, span)
			}
			ty::InstanceKind::Item(def_id) | ty::InstanceKind::Shim(ty::ShimKind::Reify(def_id, _)) => {
				self.b.fn_value(def_id, instance.args, span)
			}
			// A closure coerced to a function pointer calls it once.
			ty::InstanceKind::Shim(ty::ShimKind::ClosureOnce { .. }) => match instance.args.type_at(0).kind() {
				ty::Closure(def_id, args) => self.bare_closure(*def_id, args, span),
				_ => self.err(span, format!("a pointer to `{instance}`")),
			},
			_ => self.err(span, format!("a pointer to `{instance}`")),
		}
	}

	/// The constant an operand is, through a temporary that holds only it, which is
	/// then no longer pending.
	pub(super) fn const_operand(&mut self, op: &Operand<'tcx>) -> Option<Box<mir::ConstOperand<'tcx>>> {
		match op {
			Operand::Constant(c) => Some(c.clone()),
			Operand::Copy(place) | Operand::Move(place) => {
				let local = place.as_local()?;
				if self.defs[local] != 1 {
					return None;
				}
				let Some(mir::Rvalue::Use(Operand::Constant(c), _)) = self.def_rvalue(local) else {
					return None;
				};
				let c = c.clone();
				if let Some(name) = self.var(local) {
					let name = name.to_string();
					self.pending.retain(|p| p.name != name);
				}
				Some(c)
			}
			_ => None,
		}
	}

	/// The slice of the bytes an operand copies from behind a reference, directly or through
	/// a temporary still held back to fold into its one read, which is then no longer
	/// pending: read where they are, they need no copy.
	pub(super) fn bytes_source(&mut self, op: &Operand<'tcx>, span: Span, out: &mut Vec<Stmt>) -> Option<Expr> {
		let (Operand::Copy(place) | Operand::Move(place)) = op else {
			return None;
		};
		if self.in_bytes(*place) {
			return Some(self.place_expr(*place, span, out));
		}
		let local = place.as_local()?;
		if self.defs[local] != 1 || self.uses[local] != 1 {
			return None;
		}
		let name = self.var(local)?.to_string();
		let i = self.pending.iter().position(|p| p.name == name)?;
		let Expr::Call(f, args) = &self.pending[i].value else {
			return None;
		};
		let (Expr::Rt(rt), [slice]) = (&**f, args.as_slice()) else {
			return None;
		};
		if rt != "byteSliceToArray" {
			return None;
		}
		let slice = slice.clone();
		let p = self.pending.remove(i);
		// A path, a subslice of one in bounds, or a local's parts, reads only bytes nothing
		// changed since.
		let path = |e: &Expr| {
			e.is_path()
				|| made_slice(e).is_some()
				|| matches!(e, Expr::Index(base, i) if base.is_path() && i.is_constant())
		};
		let stable = match &slice {
			Expr::Call(f, args) if matches!(&**f, Expr::Rt(name) if name == "byteSliceRange") => {
				matches!(args.as_slice(), [base, Expr::Lit(_), ..] if path(base))
			}
			e => path(e),
		};
		if stable {
			return Some(slice);
		}
		// What the copy read, held where the copy was.
		let name = self.b.fresh("slice");
		self.pending.push(Pending {
			name: name.clone(),
			value: slice,
			parts: None,
			copy: false,
			decl: true,
			..p
		});
		Some(Expr::ident(name))
	}

	/// The bytes of a byte string constant, `&[u8; N]`.
	pub(super) fn const_bytes(&mut self, op: &Operand<'tcx>) -> Option<Vec<u8>> {
		let tcx = self.tcx;
		let c = self.const_operand(op)?;
		let mir::Const::Val(ConstValue::Scalar(Scalar::Ptr(ptr, _)), ty) = c.const_ else {
			return None;
		};
		let ty::Ref(_, inner, _) = ty.kind() else {
			return None;
		};
		let ty::Array(_, len) = inner.kind() else {
			return None;
		};
		let len = len.try_to_target_usize(tcx)? as usize;
		let (prov, offset) = ptr.prov_and_relative_offset();
		let alloc = tcx.global_alloc(prov.alloc_id()).unwrap_memory();
		let start = offset.bytes() as usize;
		Some(
			alloc
				.inner()
				.inspect_with_uninit_and_ptr_outside_interpreter(start..start + len)
				.to_vec(),
		)
	}
}

/// Whether `ty` is `[u8]`, which is an `rt.ByteSlice`.
pub fn byte_slice(ty: Ty<'_>) -> bool {
	matches!(ty.kind(), ty::Slice(elem) if matches!(elem.kind(), ty::Uint(ty::UintTy::U8)))
}

/// Byte `i` of `slice`, an `rt.ByteSlice` that reading twice reads the same.
pub fn byte_at(slice: Expr, i: Expr) -> Expr {
	let at = super::scalar::add(slice_member(slice.clone(), "start"), i);
	slice_member(slice, "data").index(at)
}

/// The field `key` of `slice`, an `rt.ByteSlice`: its part, if it is made in place of parts
/// that do nothing, as a local held as its parts reads.
pub fn slice_member(slice: Expr, key: &str) -> Expr {
	if let Some([data, start, length]) = made_slice(&slice) {
		return match key {
			"data" => data.clone(),
			"start" => start.clone(),
			"length" => length.clone(),
			_ => unreachable!("a field of a byte slice"),
		};
	}
	slice.member(key)
}

/// The parts of `slice`, if it is an `rt.ByteSlice` made in place.
pub fn made_slice(slice: &Expr) -> Option<&[Expr; 3]> {
	match slice {
		Expr::New(f, args) if matches!(&**f, Expr::Rt(name) if name == "ByteSlice") => args.as_slice().try_into().ok(),
		_ => None,
	}
}

/// Whether `ty` is a struct of one field that JS represents as that field: a wrapper
/// around a pointer, such as the `ResumeTy` and `NonNull` an async body passes its
/// context in.
pub fn transparent<'tcx>(t: &crate::lower::Translator<'_, 'tcx>, ty: Ty<'tcx>) -> bool {
	let ty::Adt(adt, _) = ty.kind() else {
		return false;
	};
	let path = crate::lower::library::std_path(t.tcx, adt.did());
	let path: Vec<&str> = path.iter().map(String::as_str).collect();
	matches!(
		path.as_slice(),
		["future", "ResumeTy"] | ["ptr", "non_null", "NonNull"] | ["future", _, "ResumeTy"]
	)
}

/// Whether `ty` holds an `FnMut` closure by value whose body writes a capture: a copy of
/// its arrow function shares the variables it captured, which in Rust each copy has its
/// own of. A closure is `FnMut` where its bound asks for one, even if it writes nothing,
/// and a capture that holds no state, a fn item or a closure capturing nothing, changes
/// nothing when written.
fn changes_captures<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
	match ty.kind() {
		ty::Closure(_, args) => {
			let closure = args.as_closure();
			let writes = closure.kind() == ty::ClosureKind::FnMut && writes_captures(tcx, ty);
			writes || closure.upvar_tys().iter().any(|t| changes_captures(tcx, t))
		}
		ty::Tuple(tys) => tys.iter().any(|t| changes_captures(tcx, t)),
		ty::Array(t, _) => changes_captures(tcx, *t),
		ty::Adt(_, args) => args.types().any(|t| changes_captures(tcx, t)),
		_ => false,
	}
}

/// Whether the body of `ty`, a closure, writes a capture that holds state, which a JS
/// closure sharing its variables would write for each copy of it.
pub fn writes_captures<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
	let ty::Closure(def_id, args) = ty.kind() else {
		return false;
	};
	let upvars = args.as_closure().upvar_tys();
	!tcx.is_mir_available(*def_id)
		|| written_captures(tcx.optimized_mir(*def_id))
			.into_iter()
			.any(|field| upvars.get(field.index()).is_none_or(|t| !stateless(*t)))
}

/// Whether every value of `ty` is the same, so writing one changes nothing.
fn stateless(ty: Ty<'_>) -> bool {
	match ty.kind() {
		ty::FnDef(..) => true,
		ty::Closure(_, args) => args.as_closure().upvar_tys().is_empty(),
		ty::Tuple(tys) => tys.is_empty(),
		_ => false,
	}
}

/// The captures a closure's body `mir` writes into, assigns, or borrows mutably, other than
/// to drop them.
fn written_captures(mir: &mir::Body<'_>) -> FxHashSet<FieldIdx> {
	struct Written(FxHashSet<FieldIdx>);
	impl<'tcx> Visitor<'tcx> for Written {
		fn visit_place(&mut self, place: &Place<'tcx>, cx: PlaceContext, _: mir::Location) {
			if place.local.as_u32() != 1
				|| !cx.is_mutating_use()
				|| cx == PlaceContext::MutatingUse(MutatingUseContext::Drop)
			{
				return;
			}
			let projection = match place.projection.as_slice() {
				[ProjectionElem::Deref, rest @ ..] => rest,
				all => all,
			};
			if let [ProjectionElem::Field(field, _), ..] = projection {
				self.0.insert(*field);
			}
		}
	}
	let mut written = Written(FxHashSet::default());
	written.visit_body(mir);
	written.0
}

/// Whether the JS object of a `ty` is shared or private, so that copying fields into it
/// would change other handles, or nothing.
fn shared<'tcx>(fx: &Fx<'_, '_, '_, 'tcx>, ty: Ty<'tcx>) -> bool {
	let tcx = fx.tcx;
	let mut erased = ty;
	while let ty::Adt(adt, args) = erased.kind()
		&& matches!(Std::of(tcx, adt.did()), Some(Std::Pin | Std::Box))
	{
		erased = args.type_at(0);
	}
	match erased.kind() {
		ty::Adt(adt, _) => matches!(
			Std::of(tcx, adt.did()),
			Some(Std::Arc | Std::Waker | Std::Context | Std::MutexGuard | Std::Ref | Std::RefMut)
		),
		ty::Ref(..) | ty::Coroutine(..) => true,
		_ => false,
	}
}

/// Whether `ty` is a `Vec<u8>`, an `rt.ByteVec`.
fn byte_vec(ty: Ty<'_>) -> bool {
	matches!(ty.kind(), ty::Adt(_, args) if args.types().next().is_some_and(|t| matches!(t.kind(), ty::Uint(ty::UintTy::U8))))
}

/// Whether `path` reads through the variable `name` first.
fn starts_from(path: &Expr, name: &str) -> bool {
	match path {
		Expr::Located(_, e) | Expr::Member(e, _) | Expr::Index(e, _) | Expr::As(e, _) => starts_from(e, name),
		Expr::Ident(n) => n == name,
		_ => false,
	}
}

/// `path`, reading through `held` where it reads through the variable it starts from.
fn from_held(path: Expr, held: &Expr) -> Expr {
	let from = |e: Box<Expr>| Box::new(from_held(*e, held));
	match path {
		Expr::Located(loc, e) => Expr::Located(loc, from(e)),
		Expr::Member(e, key) => Expr::Member(from(e), key),
		Expr::Index(e, i) => Expr::Index(from(e), i),
		Expr::As(e, ty) => Expr::As(from(e), ty),
		_ => held.clone(),
	}
}

impl<'tcx> Fx<'_, '_, '_, 'tcx> {
	/// Why a raw pointer made of `place` here would not keep pointing where Rust's does, if
	/// it would not: JS keeps each part on its path that is an object, from what the last
	/// pointer on it points at or from a local, in an object the crate may give another
	/// object in its place, which the pointer would not see. With `cells`, an `rt.Cell` on
	/// the path is such an object, as one is for the pointer `UnsafeCell::get` gives. A
	/// place read through a reference to a place this body borrows is that place.
	pub(super) fn out_of_step(&self, place: Place<'tcx>, cells: bool) -> Option<String> {
		self.borrowed_places(place).into_iter().find_map(|place| {
			self.parts_out_of_step(place, cells)
				.or_else(|| self.reassigned(place, cells))
		})
	}

	/// Why a reference made of `place` here would not keep pointing where Rust's does, if
	/// a raw pointer the crate's code can't follow back here may be made through it, as
	/// one a callee takes and returns, into a part JS keeps as an object: the variable it
	/// is in, or a part on its path, assigned again while the reference, or what is made of
	/// it, may still be read, as [`Self::out_of_step`] judges the pointer.
	pub(super) fn lent_out_of_step(&self, place: Place<'tcx>) -> Option<String> {
		let ty = place.ty(&self.mir.local_decls, self.tcx).ty;
		if !self
			.b
			.t
			.taken(self.mir.source.def_id(), ty, &|part| self.pointed_object(part, true))
		{
			return None;
		}
		let assigned = self
			.borrowed_places(place)
			.into_iter()
			.any(|place| self.assigned_after(place, past_pointers(place)));
		assigned.then(|| {
			format!("a reference to `{ty}`, through which a raw pointer may be made, in a variable assigned again while that pointer may point there")
		})
	}

	/// The places a pointer made of `place` here may point into, as
	/// [`copy::borrowed_places`] reads them through this body's borrows.
	fn borrowed_places(&self, place: Place<'tcx>) -> Vec<Place<'tcx>> {
		crate::lower::copy::borrowed_places(self.tcx, self.mir, place, self.at, &|local| {
			self.borrow_of(local, false)
		})
	}

	/// Why a raw pointer to `place`, read through no borrow of this body's, would not keep
	/// pointing where Rust's does by a part on its path past its last pointer, if it would
	/// not: one in an element, a payload, a union field, or an object the crate may give
	/// another in its place.
	fn parts_out_of_step(&self, place: Place<'tcx>, cells: bool) -> Option<String> {
		let (tcx, decls, t) = (self.tcx, &self.mir.local_decls, &self.b.t);
		let elems = place.projection.as_slice();
		let start = past_pointers(place);
		for i in start..elems.len() {
			let base = Place::ty_from(place.local, &elems[..i], decls, tcx).ty;
			let ty = Place::ty_from(place.local, &elems[..=i], decls, tcx).ty;
			if !self.pointed_object(ty, cells)
				|| matches!(
					elems[i],
					ProjectionElem::OpaqueCast(_) | ProjectionElem::UnwrapUnsafeBinder(_)
				) {
				continue;
			}
			let field = matches!(elems[i], ProjectionElem::Field(..))
				&& matches!(base.kind(), ty::Tuple(_) | ty::Adt(..))
				&& !base.is_union()
				&& !base.is_enum()
				&& elems[..i]
					.last()
					.is_none_or(|e| !matches!(e, ProjectionElem::Downcast(..)));
			if !field {
				return Some(format!(
					"a raw pointer to `{ty}` in an element, a payload, or a union field, which is not supported yet"
				));
			}
			if t.replaced(base) {
				return Some(format!(
					"a raw pointer to `{ty}` in `{base}`, which the crate may put another object in place of"
				));
			}
		}
		None
	}

	/// Why a raw pointer made of `place` here, read through no borrow of this body's, would
	/// not see the object it points at given another, if it would not: the variable it is
	/// in, or a part on its path, assigned again while it, or what is made of it, may still
	/// be read.
	fn reassigned(&self, place: Place<'tcx>, cells: bool) -> Option<String> {
		let ty = place.ty(&self.mir.local_decls, self.tcx).ty;
		(self.pointed_object(ty, cells) && self.assigned_after(place, past_pointers(place)))
			.then(|| format!("a raw pointer to `{ty}` in a variable assigned again while it may point there"))
	}

	/// Whether a raw pointer to a `ty` is the object JS keeps it in, or with `cells`, is or
	/// holds an `rt.Cell`. Only an instance decides what a parameter is.
	fn pointed_object(&self, ty: Ty<'tcx>, cells: bool) -> bool {
		let t = &self.b.t;
		!(t.polymorphic() && ty.has_param())
			&& (t.pointee(ty) == crate::lower::pointer::Pointee::Object || cells && t.std_of(ty) == Some(Std::Cell))
	}

	/// Why the pointer std's `def_id` makes of what `reference` points at would not keep
	/// pointing where Rust's does, if it would not, judged as [`Self::out_of_step`] judges a
	/// raw pointer: `UnsafeCell::get`'s, holding a `T`, is the `rt.Cell` itself or the
	/// object it holds.
	pub(super) fn made_out_of_step(
		&self,
		def_id: rustc_hir::def_id::DefId,
		gargs: ty::GenericArgsRef<'tcx>,
		reference: &Operand<'tcx>,
	) -> Option<String> {
		let tcx = self.tcx;
		let pointee = tcx.mk_place_deref(reference.place()?);
		let cell = library::key(tcx, def_id).is_some_and(|(owner, _)| owner == "UnsafeCell");
		let held = gargs.type_at(0);
		let place = match cell && self.b.t.pointee(held) == crate::lower::pointer::Pointee::Object {
			true => tcx.mk_place_field(pointee, FieldIdx::ZERO, held),
			false => pointee,
		};
		self.out_of_step(place, cell)
	}

	/// `place`, read through each reference that the body borrows a place into, as that
	/// place, or with `aliases`, only through each alias of one, which a whole write through
	/// assigns. A reference something borrows may be pointed elsewhere, so it is no such one.
	fn through_borrows(&self, place: Place<'tcx>, aliases: bool) -> Place<'tcx> {
		crate::lower::copy::through_borrows(self.tcx, place, |local| self.borrow_of(local, aliases))
	}

	/// The place the reference `local` borrows, as [`Self::through_borrows`] reads it.
	fn borrow_of(&self, local: Local, aliases: bool) -> Option<Place<'tcx>> {
		match self.def_rvalue(local) {
			Some(mir::Rvalue::Ref(_, _, borrowed))
				if self.defs[local] == 1 && !self.borrowed[local] && (!aliases || self.alias_of_place(local)) =>
			{
				Some(*borrowed)
			}
			_ => None,
		}
	}

	/// Whether something on the path of `place` from its `start`, a part with no pointer
	/// on its path after, may be assigned whole after the statement being lowered, before
	/// its local's storage ends, while what holds the pointer made there may still be read:
	/// by an assignment, directly or through an alias of it, or by a std call whose
	/// lowering gives what its `&mut` argument borrows another value. A whole value written
	/// through a reference that is no alias is written into the object there, which gives
	/// only its parts other objects.
	fn assigned_after(&self, place: Place<'tcx>, start: usize) -> bool {
		use rustc_mir_dataflow::Analysis;
		let tcx = self.tcx;
		// Rustc drops the storage markers of locals it merges, so in a loop the search may
		// reach the next iteration's assignments, where the pointer made in this one may be
		// dead.
		let mut liveness = None;
		let mut read_after = |at: mir::Location| {
			let (holders, live) = liveness.get_or_insert_with(|| {
				let live = rustc_mir_dataflow::impls::MaybeLiveLocals
					.iterate_to_fixpoint(tcx, self.mir, None)
					.into_results_cursor(self.mir);
				(self.holders(), live)
			});
			let Some(holders) = holders else {
				return true;
			};
			live.seek_before_primary_effect(at);
			holders.iter().any(|h| live.get().contains(h))
		};
		let assigns = |written: Place<'tcx>| {
			let written = self.through_borrows(written, true);
			let len = written.projection.len();
			let through = matches!(written.projection.last(), Some(ProjectionElem::Deref));
			written.local == place.local
				&& len >= start
				&& place.projection.starts_with(written.projection)
				&& !(through && len == place.projection.len())
		};
		let blocks = &self.mir.basic_blocks;
		let mut seen = FxHashSet::default();
		let mut stack = vec![(self.at.block, self.at.statement_index + 1)];
		while let Some((bb, from)) = stack.pop() {
			let data = &blocks[bb];
			let mut ended = false;
			for (i, stmt) in data.statements.iter().enumerate().skip(from) {
				match &stmt.kind {
					mir::StatementKind::StorageDead(l) | mir::StatementKind::StorageLive(l) if *l == place.local => {
						ended = true;
						break;
					}
					mir::StatementKind::Assign(assign)
						if assigns(assign.0)
							&& read_after(mir::Location {
								block: bb,
								statement_index: i,
							}) =>
					{
						return true;
					}
					_ => {}
				}
			}
			if ended {
				continue;
			}
			let term = data.terminator();
			if let mir::TerminatorKind::Call {
				func,
				args,
				destination,
				..
			} = &term.kind
			{
				let rebound = func
					.const_fn_def()
					.map_or(0, |(def_id, _)| crate::lower::pointer::rebinds(tcx, def_id));
				if (assigns(*destination)
					|| args[..rebound]
						.iter()
						.filter_map(|a| a.node.place())
						.any(|r| assigns(tcx.mk_place_deref(r))))
					&& read_after(self.mir.terminator_loc(bb))
				{
					return true;
				}
			}
			for next in term.successors() {
				if seen.insert(next) {
					stack.push((next, 0));
				}
			}
		}
		false
	}
}

/// Where the path of `place` starts past the last pointer on it.
fn past_pointers(place: Place<'_>) -> usize {
	place
		.projection
		.iter()
		.rposition(|e| matches!(e, ProjectionElem::Deref))
		.map_or(0, |i| i + 1)
}

/// An `rt.ByteSlice` of `bytes`.
fn bytes_lit(bytes: &[u8]) -> Expr {
	let items = bytes.iter().map(|b| Expr::lit(b.to_string())).collect();
	Expr::rt(
		"byteSliceFrom",
		vec![Expr::ident("Uint8Array").member("of").call(items)],
	)
}
