//! The crate's own unions.
//!
//! A union is an object of slots, one per place in its layout that a field's parts are
//! kept at: `at0`, `at4`. Each field is its parts, through tuples and structs down to the
//! values JS keeps whole, each in the slot at its offset. Fields that overlap must keep
//! the same values there, so a read of one field after a write of another reads what
//! Rust would: the same type, or a non-null pointer and an `Option` of it. A field read
//! whole is made from its slots, and one written whole is written into them.

use rustc_abi::FieldIdx;
use rustc_middle::ty::{self, AdtDef, Ty, TypingEnv};
use rustc_span::Span;

use super::library::Std;
use super::{AdtRepr, Translator};
use crate::ts::{Expr, Key, Type};

/// Each field's parts.
pub type Fields = Vec<Vec<Part>>;

/// A part of a union's field, kept whole in a slot.
#[derive(Clone)]
pub struct Part {
	/// The field indices to it from the field.
	pub path: Vec<usize>,
	pub slot: String,
	/// Whether the slot holds an `Option` of it, which a read narrows.
	pub narrow: bool,
}

/// A part of a field kept whole, at `offset` for `size` bytes.
struct Leaf<'tcx> {
	field: usize,
	path: Vec<usize>,
	offset: u64,
	size: u64,
	ty: Ty<'tcx>,
}

impl<'a, 'tcx> Translator<'a, 'tcx> {
	/// The slots of union `adt`, and each one's type: an error if its fields overlap in
	/// values JS keeps apart.
	pub fn overlay(&mut self, adt: AdtDef<'tcx>, span: Span) -> Result<(Fields, Vec<(String, Ty<'tcx>)>), String> {
		let tcx = self.tcx;
		let env = TypingEnv::post_analysis(tcx, adt.did());
		let identity = ty::GenericArgs::identity_for_item(tcx, adt.did());
		let mut leaves = Vec::new();
		for (i, field) in adt.non_enum_variant().fields.iter().enumerate() {
			let ty = self.field_ty(field.did, identity);
			self.leaves(env, ty, i, Vec::new(), 0, &mut leaves)?;
		}
		let mut slots: Vec<(u64, u64, Ty<'tcx>)> = Vec::new();
		for leaf in &leaves {
			match slots
				.iter_mut()
				.find(|(o, s, _)| *o < leaf.offset + leaf.size && leaf.offset < o + s)
			{
				None => slots.push((leaf.offset, leaf.size, leaf.ty)),
				Some((o, s, ty)) => {
					if (*o, *s) != (leaf.offset, leaf.size) || !self.same_bits(*ty, leaf.ty) {
						let path = tcx.def_path_str(adt.did());
						return Err(format!(
							"union `{path}`, whose fields overlap in `{ty}` and `{}`, which JS keeps apart",
							leaf.ty
						));
					}
					// The slot holds the wider of the two: an `Option` holds the pointer.
					if self.is_option(leaf.ty) {
						*ty = leaf.ty;
					}
				}
			}
		}
		let _ = span;
		let mut fields: Fields = vec![Vec::new(); adt.non_enum_variant().fields.len()];
		for leaf in leaves {
			let held = slots.iter().find(|(o, ..)| *o == leaf.offset).map(|(.., ty)| *ty);
			let narrow = held.is_some_and(|t| self.is_option(t)) && !self.is_option(leaf.ty);
			fields[leaf.field].push(Part {
				path: leaf.path,
				slot: slot_name(leaf.offset),
				narrow,
			});
		}
		slots.sort_by_key(|(o, ..)| *o);
		Ok((fields, slots.into_iter().map(|(o, _, ty)| (slot_name(o), ty)).collect()))
	}

	/// Push the parts of a `ty` at `offset` in field `field`, by the `path` to it.
	fn leaves(
		&self,
		env: TypingEnv<'tcx>,
		ty: Ty<'tcx>,
		field: usize,
		path: Vec<usize>,
		offset: u64,
		out: &mut Vec<Leaf<'tcx>>,
	) -> Result<(), String> {
		let tcx = self.tcx;
		let layout = tcx
			.layout_of(env.as_query_input(ty))
			.map_err(|e| format!("a union field of `{ty}`, whose layout is not known: {e}"))?;
		if layout.is_zst() {
			return Ok(());
		}
		let parts: Option<Vec<Ty<'tcx>>> = match ty.kind() {
			ty::Tuple(tys) => Some(tys.to_vec()),
			ty::Adt(adt, args) if adt.is_struct() && Std::of(tcx, adt.did()).is_none() => Some(
				adt.non_enum_variant()
					.fields
					.iter()
					.map(|f| self.field_ty(f.did, args))
					.collect(),
			),
			_ => None,
		};
		match parts {
			Some(parts) => {
				for (i, part) in parts.into_iter().enumerate() {
					let mut path = path.clone();
					path.push(i);
					let at = offset + layout.fields.offset(i).bytes();
					self.leaves(env, part, field, path, at, out)?;
				}
			}
			None => out.push(Leaf {
				field,
				path,
				offset,
				size: layout.size.bytes(),
				ty,
			}),
		}
		Ok(())
	}

	/// Whether every value of `a` and of `b` that both can hold has the same JS value.
	fn same_bits(&self, a: Ty<'tcx>, b: Ty<'tcx>) -> bool {
		let tcx = self.tcx;
		let base = |t: Ty<'tcx>| -> Ty<'tcx> {
			let t = match t.kind() {
				ty::Adt(adt, args) if Std::of(tcx, adt.did()) == Some(Std::Option) => {
					let inner = args.type_at(0);
					match self.non_null(inner) {
						true => inner,
						false => t,
					}
				}
				_ => t,
			};
			tcx.erase_and_anonymize_regions(t)
		};
		base(a) == base(b)
	}

	/// Whether a `ty` is never null, so an `Option` of it is the same pointer.
	fn non_null(&self, ty: Ty<'tcx>) -> bool {
		match ty.kind() {
			ty::Ref(..) => true,
			ty::Adt(adt, _) => matches!(Std::of(self.tcx, adt.did()), Some(Std::NonNull | Std::Box)),
			_ => false,
		}
	}

	fn is_option(&self, ty: Ty<'tcx>) -> bool {
		matches!(ty.kind(), ty::Adt(adt, _) if Std::of(self.tcx, adt.did()) == Some(Std::Option))
	}

	/// The TypeScript type of union `adt`'s slots.
	pub fn overlay_ty(&mut self, slots: Vec<(String, Ty<'tcx>)>, span: Span) -> Type {
		Type::Object(slots.into_iter().map(|(name, ty)| (name, self.ty(ty, span))).collect())
	}

	/// The slots `value`, a field of a union of type `ty`, puts in it: each part's
	/// expression, by its slot.
	pub fn slots_of(&mut self, value: Expr, ty: Ty<'tcx>, parts: &[Part], span: Span) -> Vec<(String, Expr)> {
		parts
			.iter()
			.map(|part| (part.slot.clone(), self.part_of(value.clone(), ty, &part.path, span)))
			.collect()
	}

	/// The part of `value`, a `ty`, at `path`.
	fn part_of(&mut self, value: Expr, ty: Ty<'tcx>, path: &[usize], span: Span) -> Expr {
		let Some((&first, rest)) = path.split_first() else {
			return value;
		};
		match ty.kind() {
			ty::Tuple(tys) => self.part_of(value.index(Expr::lit(first.to_string())), tys[first], rest, span),
			ty::Adt(adt, args) => {
				let info = self.adt(*adt, span);
				let AdtRepr::Struct { fields, .. } = info.repr else {
					unreachable!("a union's part is in a struct")
				};
				let field = &adt.non_enum_variant().fields[FieldIdx::from_usize(first)];
				let part_ty = self.field_ty(field.did, args);
				let read = self.held_field(Translator::read_field(value, fields[first]), field.did, args, span);
				self.part_of(read, part_ty, rest, span)
			}
			_ => unreachable!("a union's part is in a tuple or a struct"),
		}
	}

	/// A `ty`, made of the parts in `parts` of the union `base`.
	pub fn of_slots(&mut self, base: Expr, ty: Ty<'tcx>, parts: &[Part], span: Span) -> Expr {
		if let [part] = parts
			&& part.path.is_empty()
		{
			let value = base.member(part.slot.clone());
			return match part.narrow {
				true => Expr::As(Box::new(value), self.ty(ty, span)),
				false => value,
			};
		}
		let within = |i: usize| -> Vec<Part> { within(parts, i) };
		match ty.kind() {
			ty::Tuple(tys) if tys.is_empty() => Expr::undefined(),
			ty::Tuple(tys) => {
				let values = (0..tys.len())
					.map(|i| {
						let part = within(i);
						self.of_slots(base.clone(), tys[i], &part, span)
					})
					.collect();
				let ts_ty = self.ty(ty, span);
				Expr::As(Box::new(Expr::Array(values)), ts_ty)
			}
			ty::Adt(adt, args) if adt.is_struct() => {
				let info = self.adt(*adt, span);
				let AdtRepr::Struct { ctor, fields } = info.repr else {
					unreachable!("a struct")
				};
				let tys: Vec<Ty<'tcx>> = adt
					.non_enum_variant()
					.fields
					.iter()
					.map(|f| self.field_ty(f.did, args))
					.collect();
				let defs = &adt.non_enum_variant().fields;
				let values = (tys.into_iter().zip(defs.iter()).enumerate())
					.map(|(i, (t, field))| {
						let part = within(i);
						let value = self.of_slots(base.clone(), t, &part, span);
						self.field_value(value, adt.did(), field.did, args, span)
					})
					.collect();
				self.construct(info.sym, &ctor, &fields, values)
			}
			// Nothing JS holds, as `PhantomData`.
			_ => Expr::undefined(),
		}
	}

	/// The parts of field `field` of union `adt`.
	pub fn union_field(&mut self, adt: AdtDef<'tcx>, field: usize, span: Span) -> Vec<Part> {
		match self.adt(adt, span).repr {
			AdtRepr::Slots(fields) => fields[field].clone(),
			_ => unreachable!("a union"),
		}
	}
}

/// The parts of `parts` within part `i` of what they make, by their paths from it.
pub fn within(parts: &[Part], i: usize) -> Vec<Part> {
	parts
		.iter()
		.filter(|part| part.path.first() == Some(&i))
		.map(|part| Part {
			path: part.path[1..].to_vec(),
			..part.clone()
		})
		.collect()
}

/// Whether `parts` are one slot's, whole.
pub fn whole(parts: &[Part]) -> bool {
	matches!(parts, [part] if part.path.is_empty())
}

/// The name of the slot at `offset`.
fn slot_name(offset: u64) -> String {
	format!("at{offset}")
}

/// A union of slots, `slots` set and every other `undefined`, as a `ty`.
pub fn made(slots: Vec<(String, Expr)>, ty: Type) -> Expr {
	let object = Expr::Object(slots.into_iter().map(|(name, e)| (Key::Name(name), e)).collect());
	Expr::As(Box::new(Expr::As(Box::new(object), Type::Builtin("unknown"))), ty)
}
