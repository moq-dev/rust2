//! Constants: rustc evaluates each, and its value is written at each use, as Rust
//! inlines it.

use rustc_middle::ty::{self, Ty, ValTree};
use rustc_span::Span;

use super::AdtRepr;
use super::body::Body;
use super::library::Std;
use super::num::Int;
use crate::ts::{Expr, Key};

impl<'b, 'a, 'tcx> Body<'b, 'a, 'tcx> {
	/// A constant's value tree as a JS value of type `ty`.
	pub fn value(&mut self, ty: Ty<'tcx>, valtree: ValTree<'tcx>, span: Span) -> Expr {
		let tcx = self.t.tcx;
		let leaf = |v: ValTree<'tcx>| v.try_to_leaf();
		let children = |v: ValTree<'tcx>| -> Vec<(Ty<'tcx>, ValTree<'tcx>)> {
			match &**v {
				ty::ValTreeKind::Branch(consts) => consts
					.iter()
					.map(|c| match c.kind() {
						ty::ConstKind::Value(value) => (value.ty, value.valtree),
						_ => unreachable!("a value tree holds values"),
					})
					.collect(),
				ty::ValTreeKind::Leaf(_) => Vec::new(),
			}
		};
		match ty.kind() {
			ty::RawPtr(..) => match leaf(valtree) {
				Some(scalar) if scalar.is_null() => Expr::Rt("NULL".into()),
				_ => self.err(span, format!("a constant of type `{ty}` made from an integer")),
			},
			ty::Bool => Expr::lit((leaf(valtree).is_some_and(|l| l.to_bits_unchecked() != 0)).to_string()),
			ty::Int(_) | ty::Uint(_) => {
				let int = Int::of(ty).unwrap();
				let Some(scalar) = leaf(valtree) else {
					return self.err(span, "an integer constant without a value");
				};
				let size = rustc_abi::Size::from_bits(int.bits);
				let value = match int.signed {
					true => scalar.to_int(size),
					false => scalar.to_uint(size) as i128,
				};
				int.lit(value)
			}
			_ if let Some(float) = super::float::Float::of(ty) => match leaf(valtree) {
				Some(scalar) => float.constant(scalar),
				None => self.err(span, "a float constant without a value"),
			},
			ty::Char => match leaf(valtree) {
				Some(scalar) => Expr::lit(scalar.to_u32().to_string()),
				None => self.err(span, "a char constant without a value"),
			},
			ty::Ref(_, inner, _) => match inner.kind() {
				ty::Str => {
					let bytes: Vec<u8> = children(valtree)
						.into_iter()
						.map(|(_, v)| v.try_to_leaf().map_or(0, |l| l.to_u8()))
						.collect();
					match String::from_utf8(bytes) {
						Ok(s) => Expr::Str(s),
						Err(_) => self.err(span, "a string constant that is not UTF-8"),
					}
				}
				ty::Slice(elem) => {
					let items: Vec<Expr> = children(valtree)
						.into_iter()
						.map(|(t, v)| self.value(t, v, span))
						.collect();
					match elem.kind() {
						ty::Uint(ty::UintTy::U8) => Expr::rt(
							"byteSliceFrom",
							vec![Expr::ident("Uint8Array").member("of").call(items)],
						),
						_ => Expr::Array(items),
					}
				}
				_ if super::byte_array(*inner) => Expr::rt("byteSliceFrom", vec![self.value(*inner, valtree, span)]),
				_ => self.value(*inner, valtree, span),
			},
			ty::Array(elem, _) => {
				let items: Vec<Expr> = children(valtree)
					.into_iter()
					.map(|(t, v)| self.value(t, v, span))
					.collect();
				match elem.kind() {
					ty::Uint(ty::UintTy::U8) => Expr::ident("Uint8Array").member("of").call(items),
					_ => Expr::Array(items),
				}
			}
			ty::Tuple(tys) if tys.is_empty() => Expr::undefined(),
			ty::Tuple(_) => Expr::Array(
				children(valtree)
					.into_iter()
					.map(|(t, v)| self.value(t, v, span))
					.collect(),
			),
			// What core's wrappers of one value hold is that value, as is a pattern type's.
			ty::Pat(base, _) => self.value(*base, valtree, span),
			ty::Adt(..) if super::library::wrapped(tcx, ty).is_some() => match &children(valtree)[..] {
				[(field_ty, field)] => self.value(*field_ty, *field, span),
				_ => self.err(span, format!("a constant of type `{ty}` without one field")),
			},
			ty::Adt(adt, args) => {
				let fields = children(valtree);
				// Its alignment is an enum, whose variant `i` is `1 << i`.
				if Std::of(tcx, adt.did()) == Some(Std::Layout) {
					let size = fields.first().and_then(|(_, size)| leaf(*size));
					let align = (fields.get(1).and_then(|(_, align)| children(*align).first().copied()))
						.and_then(|(_, variant)| children(variant).first().copied())
						.and_then(|(_, index)| leaf(index));
					return match (size, align) {
						(Some(size), Some(index)) => super::alloc::object(
							Expr::lit(size.to_u32().to_string()),
							Expr::lit((1u64 << index.to_u32()).to_string()),
						),
						_ => self.err(span, format!("a constant of type `{ty}` without its fields")),
					};
				}
				// Its nanoseconds are a pattern type, which only it reads.
				if Std::of(tcx, adt.did()) == Some(Std::Duration) {
					return match super::time::constant(&fields) {
						Some(value) => value,
						None => self.err(span, format!("a constant of type `{ty}` without its fields")),
					};
				}
				let (variant, fields) = match adt.is_enum() {
					true => {
						let Some(((_, discr), rest)) = fields.split_first() else {
							return self.err(span, "an enum constant without a variant");
						};
						let index = discr.try_to_leaf().map_or(0, |l| l.to_u32());
						(rustc_abi::VariantIdx::from_u32(index), rest.to_vec())
					}
					false => (rustc_abi::FIRST_VARIANT, fields),
				};
				let mut values: Vec<Expr> = fields.into_iter().map(|(t, v)| self.value(t, v, span)).collect();
				if let Some(std) = Std::of(tcx, adt.did()) {
					return match (std, variant.index()) {
						(Std::Option, 0) => self.t.none(args.type_at(0)),
						(Std::Option, _) => values.pop().unwrap(),
						(Std::Poll, 1) => self.t.pending(args.type_at(0)),
						(Std::Poll, _) => values.pop().unwrap(),
						(Std::Cow, _) if crate::lower::library::string(tcx, ty) => values.pop().unwrap(),
						(Std::Cow, _) if let Some(elem) = super::library::cow_slice(tcx, ty) => {
							super::library::cow_made(elem, variant, values.pop().unwrap())
						}
						(Std::Result, i) => Expr::result(i == 0, values.pop().unwrap()),
						// `start..=end`, not yet stepped past its end.
						(Std::RangeInclusive, _)
							if values.len() == 3 && matches!(&values[2], Expr::Lit(l) if l == "false") =>
						{
							values.pop();
							values.extend(super::library::range_step(tcx, args.type_at(0), true));
							Expr::New(Box::new(Expr::Rt("RangeInclusive".into())), values)
						}
						(Std::Ordering, _) => super::collections::ordering_variant(variant),
						(Std::AtomicOrdering, _) => super::task::atomic_ordering(variant),
						(Std::Bound | Std::TryLockError, _) => {
							let ts_ty = self.t.ty(ty, span);
							super::library::tagged(adt.variant(variant).name.as_str(), values.pop(), ts_ty)
						}
						(Std::RangeTo | Std::RangeToInclusive, _) => {
							super::library::range_to(values.pop().unwrap(), std == Std::RangeToInclusive)
						}
						(Std::RangeFrom, _) => {
							values.extend(super::library::range_step(tcx, args.type_at(0), true));
							Expr::New(Box::new(Expr::Rt("RangeFrom".into())), values)
						}
						(Std::Range, _) => {
							values.extend(super::library::range_step(tcx, args.type_at(0), false));
							Expr::New(Box::new(Expr::Rt("Range".into())), values)
						}
						(Std::RangeFull, _) => Expr::undefined(),
						(Std::ParseIntError, _) => {
							Expr::Object(vec![(Key::Name("kind".into()), values.pop().unwrap())])
						}
						(Std::ErrorKind, _) => Expr::As(
							Box::new(Expr::Str(adt.variant(variant).name.to_string())),
							self.t.ty(ty, span),
						),
						_ => self.err(span, format!("a constant of type `{ty}`")),
					};
				}
				let values = match self.zero_sized_std(ty, values, span) {
					Ok(made) => return made,
					Err(values) => values,
				};
				if !self.t.own(adt.did()) {
					let foreign = self.t.foreign(adt.did());
					return self.err(span, format!("a constant of type `{ty}` {foreign}"));
				}
				let info = self.t.adt(*adt, span);
				let def = adt.variant(variant);
				match info.repr {
					AdtRepr::Struct { ctor, fields } => {
						let values = (def.fields.iter().zip(values))
							.map(|(field, v)| self.t.field_value(v, adt.did(), field.did, args, span))
							.collect();
						self.t.construct(info.sym, &ctor, &fields, values)
					}
					AdtRepr::Tags => {
						let ts_ty = self.t.ty(ty, span);
						Expr::As(Box::new(Expr::Str(def.name.to_string())), ts_ty)
					}
					AdtRepr::Slots(_) => self.err(span, format!("a constant of type `{ty}`")),
					AdtRepr::Union => {
						let mut props = vec![(Key::Name("tag".into()), Expr::Str(def.name.to_string()))];
						for (i, (field, value)) in def.fields.iter().zip(values).enumerate() {
							props.push((Key::Name(super::field_name(field.name.as_str(), i)), value));
						}
						let ts_ty = self.t.ty(ty, span);
						Expr::As(Box::new(Expr::Object(props)), ts_ty)
					}
				}
			}
			_ => self.err(span, format!("a constant of type `{ty}`")),
		}
	}
}
