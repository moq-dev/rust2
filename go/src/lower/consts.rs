//! Constants: rustc evaluates each, and its value is written at each use, as Rust
//! inlines it.

use rustc_abi::Size;
use rustc_hir::def_id::LOCAL_CRATE;
use rustc_middle::mir::interpret::{AllocId, GlobalAlloc, Scalar, alloc_range};
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
							let value = format!(
								"[{len}]uint8{{{}}}",
								bytes.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", ")
							);
							return format!("&{}", self.shared((prov.alloc_id(), offset.bytes(), len, ty), value));
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
							_ => format!("&{}", self.shared((prov.alloc_id(), offset.bytes(), 0, ty), value)),
						}
					}
					_ => self.err(
						span,
						format!("a constant of type `{ty}` pointing into a static or function"),
					),
				}
			}
			ConstValue::Slice { alloc_id, .. } => match val.try_get_slice_bytes_for_diagnostics(tcx) {
				Some(bytes) if matches!(ty.builtin_deref(true).map(|t| t.kind()), Some(ty::Str | ty::Slice(_))) => {
					self.shared((alloc_id, 0, bytes.len() as u64, ty), bytes_lit(bytes))
				}
				_ => self.err(span, format!("a slice constant of type `{ty}`")),
			},
			ConstValue::ZeroSized => format!("{}{{}}", self.ty(ty, span)),
			ConstValue::Indirect { alloc_id, offset } => self.in_memory(alloc_id, offset, ty, span),
		}
	}
}

impl<'a, 'tcx> Fx<'a, 'tcx> {
	/// The package-level variable holding `value`, the constant memory `key` names by its
	/// allocation, offset, length, and type: each use of one allocation is the same
	/// memory, as Rust's references to it are.
	fn shared(&mut self, key: (AllocId, u64, u64, Ty<'tcx>), value: String) -> String {
		if let Some(name) = self.t.allocs.get(&key) {
			return name.clone();
		}
		let name = self.t.fresh("constant");
		self.t.allocs.insert(key, name.clone());
		self.t.declare(LOCAL_CRATE, &name, format!("var {name} = {value}\n"));
		name
	}

	/// The value of a scalar or a reference of type `ty` at `offset` in a constant's
	/// memory, as a promoted `&1` or `&"text"` holds it.
	fn in_memory(&mut self, alloc_id: AllocId, offset: Size, ty: Ty<'tcx>, span: Span) -> String {
		let tcx = self.tcx;
		let GlobalAlloc::Memory(alloc) = tcx.global_alloc(alloc_id) else {
			return self.err(span, format!("a constant of type `{ty}` outside memory"));
		};
		let ptr = tcx.data_layout.pointer_size();
		// A pointer is read with what it points to, and an integer without.
		let read =
			|at: Size, size: Size, pointer: bool| alloc.inner().read_scalar(&tcx, alloc_range(at, size), pointer).ok();
		match ty.kind() {
			ty::Ref(_, inner, _) if matches!(inner.kind(), ty::Str | ty::Slice(_)) => {
				// A pointer to the bytes, then their length.
				let (Some(Scalar::Ptr(to, _)), Some(Scalar::Int(len))) =
					(read(offset, ptr, true), read(offset + ptr, ptr, false))
				else {
					return self.err(span, format!("a constant of type `{ty}` in memory"));
				};
				let (prov, start) = to.prov_and_relative_offset();
				let GlobalAlloc::Memory(bytes) = tcx.global_alloc(prov.alloc_id()) else {
					return self.err(span, format!("a constant of type `{ty}` pointing outside memory"));
				};
				let start = start.bytes() as usize;
				let len = len.to_target_usize(tcx) as usize;
				match inner.kind() {
					ty::Str => {
						let value = bytes_lit(
							bytes
								.inner()
								.inspect_with_uninit_and_ptr_outside_interpreter(start..start + len),
						);
						self.shared((prov.alloc_id(), start as u64, len as u64, ty), value)
					}
					_ => self.err(span, format!("a slice constant of type `{ty}` in memory")),
				}
			}
			ty::Bool | ty::Char | ty::Int(_) | ty::Uint(_) | ty::Ref(..) => {
				let Ok(layout) = tcx.layout_of(self.t_env().as_query_input(ty)) else {
					return self.err(span, format!("a constant of type `{ty}` without a layout"));
				};
				match read(offset, layout.size, ty.is_ref()) {
					Some(scalar) => self.const_value(ConstValue::Scalar(scalar), ty, span),
					None => self.err(span, format!("a constant of type `{ty}` in memory")),
				}
			}
			_ => self.err(span, format!("a constant of type `{ty}` in memory")),
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
