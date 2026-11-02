//! Constants: rustc evaluates each, and its value is written at each use, as Rust
//! inlines it.

use rustc_middle::mir::interpret::{GlobalAlloc, Scalar};
use rustc_middle::mir::{self, ConstValue};
use rustc_middle::ty::{self, Ty};
use rustc_span::Span;

use super::body::Fx;

impl<'a, 'tcx> Fx<'a, 'tcx> {
	/// A constant operand's value.
	pub fn constant(&mut self, c: &mir::ConstOperand<'tcx>, span: Span) -> String {
		let ty = c.ty();
		match c.const_.eval(self.tcx, self.t_env(), span) {
			Ok(val) => self.const_value(val, ty, span),
			Err(_) => self.err(span, "a constant that fails to evaluate"),
		}
	}

	/// The value of a constant `val` of type `ty`.
	fn const_value(&mut self, val: ConstValue, ty: Ty<'tcx>, span: Span) -> String {
		let tcx = self.tcx;
		if ty.is_unit() {
			return "struct{}{}".into();
		}
		match ty.kind() {
			ty::FnDef(..) => return "struct{}{}".into(),
			ty::Closure(..) => return format!("{}{{}}", self.ty(ty, span)),
			_ => {}
		}
		// An aggregate, field by field.
		if matches!(ty.kind(), ty::Adt(..) | ty::Tuple(_) | ty::Array(..)) {
			if let ty::Adt(adt, _) = ty.kind()
				&& let Some(mapped) = super::library::mapped(tcx, adt.did())
				&& !mapped.is_runtime_layout()
			{
				return match mapped {
					super::library::Mapped::Runtime("struct{}") => "struct{}{}".into(),
					super::library::Mapped::Vec => format!("{}(nil)", self.ty(ty, span)),
					_ => self.err(span, format!("a constant of type `{ty}`")),
				};
			}
			let Some(d) = tcx.try_destructure_mir_constant_for_user_output(val, ty) else {
				return self.err(span, format!("a constant of type `{ty}` that can't be taken apart"));
			};
			let values: Vec<String> = d.fields.iter().map(|(v, t)| self.const_value(*v, *t, span)).collect();
			let go = self.ty(ty, span);
			return match ty.kind() {
				ty::Adt(adt, args) => {
					let variant = d.variant.unwrap_or(rustc_abi::FIRST_VARIANT);
					self.adt_lit(*adt, variant, args, &values, &go, span)
				}
				_ => format!("{go}{{{}}}", values.join(", ")),
			};
		}
		match val {
			ConstValue::Scalar(Scalar::Int(int)) => match ty.kind() {
				ty::Bool => (int.to_bits_unchecked() != 0).to_string(),
				ty::Char => format!("rune({})", int.to_u32()),
				ty::Int(_) | ty::Uint(_) => self.int_lit(ty, int.to_bits_unchecked(), span),
				ty::RawPtr(..) if int.is_null() => format!("{}(nil)", self.ty(ty, span)),
				_ => self.err(span, format!("a scalar constant of type `{ty}`")),
			},
			ConstValue::Scalar(Scalar::Ptr(ptr, _)) => {
				let (prov, offset) = ptr.prov_and_relative_offset();
				let ty::Ref(_, inner, _) = ty.kind() else {
					return self.err(span, format!("a pointer constant of type `{ty}`"));
				};
				match tcx.global_alloc(prov.alloc_id()) {
					GlobalAlloc::Memory(alloc) => {
						if let ty::Array(elem, len) = inner.kind()
							&& let (Some(len), ty::Uint(ty::UintTy::U8)) = (len.try_to_target_usize(tcx), elem.kind())
						{
							let start = offset.bytes() as usize;
							let bytes = alloc
								.inner()
								.inspect_with_uninit_and_ptr_outside_interpreter(start..start + len as usize);
							return format!(
								"&[{len}]uint8{{{}}}",
								bytes.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", ")
							);
						}
						let value = self.const_value(
							ConstValue::Indirect {
								alloc_id: prov.alloc_id(),
								offset,
							},
							*inner,
							span,
						);
						match inner.kind() {
							ty::Str | ty::Slice(_) | ty::Dynamic(..) => value,
							_ => format!("ptrTo({value})"),
						}
					}
					_ => self.err(
						span,
						format!("a constant of type `{ty}` pointing into a static or function"),
					),
				}
			}
			ConstValue::Slice { .. } => match val.try_get_slice_bytes_for_diagnostics(tcx) {
				Some(bytes) if matches!(ty.builtin_deref(true).map(|t| t.kind()), Some(ty::Str | ty::Slice(_))) => {
					bytes_lit(bytes)
				}
				_ => self.err(span, format!("a slice constant of type `{ty}`")),
			},
			ConstValue::ZeroSized => format!("{}{{}}", self.ty(ty, span)),
			ConstValue::Indirect { .. } => self.err(span, format!("a constant of type `{ty}` in memory")),
		}
	}
}

/// Bytes as a Go `[]uint8` literal, through a string literal.
pub fn bytes_lit(bytes: &[u8]) -> String {
	let mut s = String::from("[]uint8(\"");
	for &b in bytes {
		match b {
			b'"' => s.push_str("\\\""),
			b'\\' => s.push_str("\\\\"),
			0x20..=0x7e => s.push(b as char),
			_ => s.push_str(&format!("\\x{b:02x}")),
		}
	}
	s.push_str("\")");
	s
}
