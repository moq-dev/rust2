//! Casts of pointers to memory from `alloc`, as [`crate::lower::alloc`] models it: one from
//! `*mut u8` to `*mut T` the body shows came from `alloc` or `realloc` makes the memory a JS
//! array of `T`s, and one back to `*mut u8` is the block, only as what `dealloc` or
//! `realloc` takes. Any other cast between them reads the bytes of one type as another,
//! which stays refused.

use rustc_middle::mir::{self, Local, Operand, ProjectionElem, Rvalue, TerminatorKind};
use rustc_middle::ty::{self, Ty};
use rustc_span::{Span, Spanned};

use super::Fx;
use crate::lower::library;
use crate::lower::pointer::Pointee;
use crate::ts::{Expr, Stmt};

/// Why the bytes of memory from `alloc` are refused.
pub(super) const BYTES: &str = "a byte of memory from `alloc`, which JS keeps as the values a cast of its pointer types it as";

impl<'tcx> Fx<'_, '_, '_, 'tcx> {
	/// The cast of the pointer `op` from one to `from` to one to `to`, if it is a cast of
	/// memory from `alloc` between bytes and values JS copies.
	pub(super) fn alloc_cast(
		&mut self,
		op: &Operand<'tcx>,
		from: Ty<'tcx>,
		to: Ty<'tcx>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Option<Expr> {
		let byte = |ty: Ty<'tcx>| matches!(ty.kind(), ty::Uint(ty::UintTy::U8));
		let typed = |fx: &Self, ty: Ty<'tcx>| {
			!byte(ty)
				&& !fx.b.t.zero_sized(ty)
				&& fx.b.t.pointee(ty) == Pointee::Place
				&& ty.is_sized(fx.tcx, fx.b.t.typing_env_pub())
		};
		let (f, elem) = match (byte(from), byte(to)) {
			(true, false) if typed(self, to) && self.allocated(op) => ("allocAs", to),
			(false, true) if typed(self, from) && self.freed(op) => ("allocBytes", from),
			_ => return None,
		};
		let p = self.operand(op, span, out);
		let size = crate::lower::iter::size_of(self.b, elem, span);
		Some(Expr::rt(f, vec![p, size]))
	}

	/// A call of `cast` on a pointer, if it is a cast of memory from `alloc`, as [`Self::alloc_cast`] has it.
	pub(super) fn alloc_cast_call(
		&mut self,
		def_id: rustc_hir::def_id::DefId,
		gargs: ty::GenericArgsRef<'tcx>,
		args: &[Spanned<Operand<'tcx>>],
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Option<Expr> {
		let (owner, name) = library::key(self.tcx, def_id)?;
		if !matches!(owner.as_str(), "ptr" | "NonNull") || name != "cast" {
			return None;
		}
		let mut types = gargs.types();
		let (from, to) = (types.next()?, types.next()?);
		self.alloc_cast(&args.first()?.node, from, to, span, out)
	}

	/// Why `place` can't be read or written, if it is a byte of memory from `alloc`, as the
	/// body shows, which JS keeps as values once a cast types it, and never as bytes.
	pub(super) fn alloc_bytes(&self, place: mir::Place<'tcx>) -> Option<&'static str> {
		let [ProjectionElem::Deref, ..] = place.projection.as_slice() else {
			return None;
		};
		let ty::RawPtr(pointee, _) = self.mir.local_decls[place.local].ty.kind() else {
			return None;
		};
		(matches!(pointee.kind(), ty::Uint(ty::UintTy::U8)) && self.allocated(&Operand::Copy(mir::Place::from(place.local))))
			.then_some(BYTES)
	}

	/// Whether the pointer `op` is one `alloc` or `realloc` returned, as the body shows.
	pub(super) fn allocated(&self, op: &Operand<'tcx>) -> bool {
		let Some(local) = self.traced(op) else {
			return false;
		};
		if self.def_at[local].is_some() {
			return match self.def_rvalue(local) {
				Some(Rvalue::Use(op, _) | Rvalue::Cast(mir::CastKind::PtrToPtr, op, _)) => match op.place() {
					// A payload out of an `Option`, a `Result`, or `?`'s `ControlFlow`.
					Some(place)
						if let [ProjectionElem::Downcast(..), ProjectionElem::Field(..)] =
							place.projection.as_slice() =>
					{
						self.allocated(&Operand::Copy(mir::Place::from(place.local)))
					}
					_ => self.allocated(op),
				},
				_ => false,
			};
		}
		let Some((def_id, _, args)) = self.def_call(local) else {
			return false;
		};
		match library::key(self.tcx, def_id) {
			Some((owner, name)) => match (owner.as_str(), name.as_str()) {
				("alloc", "alloc" | "realloc") => library::is_std(self.tcx, def_id),
				("NonNull", "new" | "new_unchecked" | "as_ptr")
				| ("Option", "ok_or" | "ok_or_else" | "unwrap" | "expect" | "unwrap_unchecked")
				| ("Result", "unwrap" | "expect" | "unwrap_unchecked")
				| ("Try", "branch") => self.allocated(&args[0].node),
				_ => false,
			},
			None => false,
		}
	}

	/// Whether the cast of the pointer `op` gives only what `dealloc` or `realloc` takes.
	fn freed(&self, op: &Operand<'tcx>) -> bool {
		self.mir.basic_blocks.iter().any(|data| {
			let TerminatorKind::Call { func, args, .. } = &data.terminator().kind else {
				return false;
			};
			let ty::FnDef(def_id, _) = *func.ty(&self.mir.local_decls, self.tcx).kind() else {
				return false;
			};
			let freeing = library::is_std(self.tcx, def_id)
				&& matches!(
					library::key(self.tcx, def_id),
					Some((owner, name)) if owner == "alloc" && matches!(name.as_str(), "dealloc" | "realloc")
				);
			freeing && args.first().is_some_and(|arg| self.cast_of(&arg.node, op))
		})
	}

	/// Whether `arg` is read only here, and is the cast of `op` alone.
	fn cast_of(&self, arg: &Operand<'tcx>, op: &Operand<'tcx>) -> bool {
		let Some(local): Option<Local> = arg.place().and_then(|p| p.as_local()) else {
			return false;
		};
		self.uses[local] == 1
			&& self.traced(arg).is_some()
			&& matches!(self.def_rvalue(local), Some(Rvalue::Cast(mir::CastKind::PtrToPtr, cast, _)) if std::ptr::eq(cast, op))
	}
}
