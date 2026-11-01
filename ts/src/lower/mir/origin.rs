//! Where a raw pointer a body makes points, as far as the body shows. JS keeps a value in
//! no array, and a shared subslice of values it copies, apart from what holds them in
//! Rust, so a pointer leaving one, or measured against another, fails at run time; where
//! the body proves that it does, translation refuses it with a span instead. A pointer the
//! body takes from elsewhere, such as a field or a parameter, is left to the runtime.

use rustc_middle::mir::{self, BasicBlock, Local, Location, Operand, ProjectionElem, Rvalue, TerminatorKind};
use rustc_middle::ty::{self, GenericArgsRef};
use rustc_span::Spanned;

use super::Fx;
use crate::lower::library::{self, Std};
use crate::lower::num::Int;
use crate::lower::pointer::Pointee;

/// The array JS keeps what a pointer points at in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Array {
	/// An array of one, around the place a `&raw` here took, which each `&raw` makes anew.
	Lone(Location),
	/// A shared subslice of values JS copies, which is a copy, made by the call ending this
	/// block, with its length if the body fixes it.
	Copy(BasicBlock, Option<i128>),
	/// The array of a variable of this body's own, or of a field of one.
	Owned,
}

impl Array {
	/// How many elements the array holds, if the body shows it.
	fn len(self) -> Option<i128> {
		match self {
			Array::Lone(_) => Some(1),
			Array::Copy(_, len) => len,
			Array::Owned => None,
		}
	}

	/// What the array is, as a refusal names it.
	fn what(self) -> &'static str {
		match self {
			Array::Lone(_) => "a value in no array",
			Array::Copy(..) => "a shared subslice of values JS copies",
			Array::Owned => "an array of this body's own",
		}
	}
}

/// Where a pointer points: into which array, at which index if the body fixes it.
#[derive(Clone, Copy)]
struct Origin {
	array: Array,
	index: Option<i128>,
}

impl<'tcx> Fx<'_, '_, '_, 'tcx> {
	/// Why the call of std's `def_id` would fail at run time where Rust's is defined, if the
	/// body proves it would: `offset_from` between pointers JS keeps in different arrays, a
	/// raw or mutable slice of a byte in no array of bytes, or a pointer, slice, or copy
	/// reaching out of an array of one or a copied subslice.
	pub(super) fn stray(
		&self,
		def_id: rustc_hir::def_id::DefId,
		gargs: GenericArgsRef<'tcx>,
		args: &[Spanned<Operand<'tcx>>],
	) -> Option<String> {
		let (owner, name) = library::key(self.tcx, def_id)?;
		let arg = |i: usize| args.get(i).map(|a| &a.node);
		let pointee = gargs.types().next()?;
		if self.b.t.zero_sized(pointee) || self.b.t.pointee(pointee) != Pointee::Place {
			return None;
		}
		let out = |o: Origin, n: Option<i128>, what: &str| {
			let index = o.index?;
			let past = o.array.len().is_some_and(|len| index + n.unwrap_or(0) > len);
			(index < 0 || past).then(|| {
				format!(
					"{what} reaching past {}, which JS keeps apart from what holds it in Rust",
					o.array.what()
				)
			})
		};
		match (owner.as_str(), name.as_str()) {
			("ptr" | "NonNull", "offset_from" | "offset_from_unsigned") => {
				let (a, b) = (self.origin(arg(0)?)?, self.origin(arg(1)?)?);
				let apart = match (a.array, b.array) {
					(Array::Owned, Array::Owned) => false,
					(a, b) => a != b,
				};
				apart.then(|| {
					format!(
						"`offset_from` between a pointer into {} and one into {}, which JS keeps in different arrays",
						a.array.what(),
						b.array.what()
					)
				})
			}
			("ptr" | "NonNull", "add" | "sub" | "offset") => {
				let o = self.moved(self.origin(arg(0)?)?, name.as_str(), arg(1)?);
				out(o, None, "a pointer")
			}
			("raw", "from_raw_parts" | "from_raw_parts_mut")
			| ("ptr", "slice_from_raw_parts" | "slice_from_raw_parts_mut")
			| ("NonNull", "slice_from_raw_parts") => {
				let o = self.origin(arg(0)?)?;
				let byte = matches!(pointee.kind(), ty::Uint(ty::UintTy::U8));
				if byte && name != "from_raw_parts" && matches!(o.array, Array::Lone(_)) {
					return Some("a raw or mutable slice of a byte in no array of bytes".into());
				}
				out(o, Some(self.const_int(arg(1)?)?), "a slice")
			}
			(
				"ptr",
				"copy"
				| "copy_nonoverlapping"
				| "copy_to"
				| "copy_to_nonoverlapping"
				| "copy_from"
				| "copy_from_nonoverlapping",
			)
			| ("NonNull", "copy_to" | "copy_to_nonoverlapping" | "copy_from" | "copy_from_nonoverlapping") => {
				let n = self.const_int(arg(2)?);
				[arg(0)?, arg(1)?]
					.into_iter()
					.find_map(|p| out(self.origin(p)?, n, "a copy"))
			}
			_ => None,
		}
	}

	/// Where the pointer `op` points, if the body shows it.
	fn origin(&self, op: &Operand<'tcx>) -> Option<Origin> {
		let local = self.traced(op)?;
		if let Some((bb, i)) = self.def_at[local] {
			let rvalue = self.def_rvalue(local)?;
			return match rvalue {
				Rvalue::Use(op, _)
				| Rvalue::Cast(
					mir::CastKind::PtrToPtr
					| mir::CastKind::PointerCoercion(ty::adjustment::PointerCoercion::MutToConstPointer, _),
					op,
					_,
				) if self.same_pointee(op, local) => self.origin(op),
				Rvalue::RawPtr(mir::RawPtrKind::Const | mir::RawPtrKind::Mut, place) => {
					match place.projection.as_slice() {
						// `&raw mut *p` is `p`.
						[ProjectionElem::Deref] => self.origin(&Operand::Copy(mir::Place::from(place.local))),
						[.., ProjectionElem::Deref] => None,
						[.., ProjectionElem::Index(_) | ProjectionElem::ConstantIndex { .. }] => {
							(!place.projection.contains(&ProjectionElem::Deref)).then_some(Origin {
								array: Array::Owned,
								index: None,
							})
						}
						_ => Some(Origin {
							array: Array::Lone(Location {
								block: bb,
								statement_index: i,
							}),
							index: Some(0),
						}),
					}
				}
				_ => None,
			};
		}
		let (def_id, _, args) = self.def_call(local)?;
		let (owner, name) = library::key(self.tcx, def_id)?;
		match (owner.as_str(), name.as_str()) {
			("ptr" | "NonNull", "add" | "sub" | "offset") => {
				Some(self.moved(self.origin(&args[0].node)?, name.as_str(), &args[1].node))
			}
			("NonNull", "as_ptr" | "new_unchecked") => self.origin(&args[0].node),
			("slice" | "Vec", "as_ptr" | "as_mut_ptr") => Some(Origin {
				array: self.array(&args[0].node)?,
				index: Some(0),
			}),
			_ => None,
		}
	}

	/// `o` moved by the offset `op` as `add`, `sub`, or `offset` moves it.
	fn moved(&self, o: Origin, name: &str, op: &Operand<'tcx>) -> Origin {
		let n = self.const_int(op).map(|n| match name {
			"sub" => -n,
			_ => n,
		});
		Origin {
			index: o.index.zip(n).map(|(i, n)| i + n),
			..o
		}
	}

	/// The array the slice or `Vec`, or reference to one, `op` is.
	fn array(&self, op: &Operand<'tcx>) -> Option<Array> {
		let local = self.traced(op)?;
		if self.def_at[local].is_some() {
			return match self.def_rvalue(local)? {
				Rvalue::Use(op, _) | Rvalue::Cast(mir::CastKind::PointerCoercion(..), op, _) => self.array(op),
				Rvalue::Ref(_, _, place) | Rvalue::CopyForDeref(place) => match place.projection.as_slice() {
					[ProjectionElem::Deref] => self.array(&Operand::Copy(mir::Place::from(place.local))),
					projection
						if !projection.contains(&ProjectionElem::Deref)
							&& match place.ty(&self.mir.local_decls, self.tcx).ty.kind() {
								ty::Array(..) => true,
								ty::Adt(adt, _) => Std::of(self.tcx, adt.did()) == Some(Std::Vec),
								_ => false,
							} =>
					{
						Some(Array::Owned)
					}
					_ => None,
				},
				_ => None,
			};
		}
		let (def_id, gargs, args) = self.def_call(local)?;
		let (owner, name) = library::key(self.tcx, def_id)?;
		let container = gargs.type_at(0).peel_refs();
		// `get_unchecked` is generic over the element, not the slice.
		let elem = match (owner.as_str(), name.as_str(), container.kind()) {
			("slice", "get_unchecked", _) => container,
			("Index", "index", ty::Slice(e) | ty::Array(e, _)) => *e,
			("Index", "index", ty::Adt(adt, a)) if Std::of(self.tcx, adt.did()) == Some(Std::Vec) => a.type_at(0),
			_ => return None,
		};
		let range = gargs.type_at(1);
		// A shared range of bytes is a view of them.
		if matches!(elem.kind(), ty::Uint(ty::UintTy::U8)) || Int::of(range).is_some() {
			return None;
		}
		let call = self.def_block(local)?;
		Some(Array::Copy(call, self.range_len(&args[1].node, container)))
	}

	/// How many elements the range `op` takes of `container`, if the body fixes it.
	fn range_len(&self, op: &Operand<'tcx>, container: ty::Ty<'tcx>) -> Option<i128> {
		let local = self.traced(op)?;
		let Rvalue::Aggregate(kind, ops) = self.def_rvalue(local)? else {
			return None;
		};
		let mir::AggregateKind::Adt(did, ..) = **kind else {
			return None;
		};
		let all = match container.kind() {
			ty::Array(_, n) => n.try_to_target_usize(self.tcx).map(i128::from),
			_ => None,
		};
		let ops: Vec<_> = ops.iter().map(|op| self.const_int(op)).collect();
		match (self.tcx.item_name(did).as_str(), ops.as_slice()) {
			("Range", [Some(start), Some(end)]) => Some(end - start),
			("RangeTo", [Some(end)]) => Some(*end),
			("RangeFrom", [Some(start)]) => Some(all? - start),
			("RangeFull", []) => all,
			_ => None,
		}
	}

	/// The local `op` reads whole, if the body assigns it once.
	fn traced(&self, op: &Operand<'tcx>) -> Option<Local> {
		let local = op.place()?.as_local()?;
		(self.defs[local] == 1 && !self.is_arg(local)).then_some(local)
	}

	/// Whether `op` is a pointer to what `local` points at.
	fn same_pointee(&self, op: &Operand<'tcx>, local: Local) -> bool {
		let pointee = |ty: ty::Ty<'tcx>| ty.builtin_deref(true);
		let from = op.ty(&self.mir.local_decls, self.tcx);
		pointee(from).is_some() && pointee(from) == pointee(self.mir.local_decls[local].ty)
	}

	/// The function, its generic arguments, and its arguments, of the call that assigns `local`.
	fn def_call(
		&self,
		local: Local,
	) -> Option<(
		rustc_hir::def_id::DefId,
		GenericArgsRef<'tcx>,
		&[Spanned<Operand<'tcx>>],
	)> {
		let bb = self.def_block(local)?;
		let TerminatorKind::Call { func, args, .. } = &self.mir.basic_blocks[bb].terminator().kind else {
			return None;
		};
		let ty::FnDef(def_id, gargs) = *func.ty(&self.mir.local_decls, self.tcx).kind() else {
			return None;
		};
		Some((def_id, gargs.skip_binder(), args))
	}

	/// The block whose call assigns `local`.
	fn def_block(&self, local: Local) -> Option<BasicBlock> {
		self.mir
			.basic_blocks
			.iter_enumerated()
			.find_map(|(bb, data)| match &data.terminator().kind {
				TerminatorKind::Call { destination, .. } if destination.as_local() == Some(local) => Some(bb),
				_ => None,
			})
	}
}
