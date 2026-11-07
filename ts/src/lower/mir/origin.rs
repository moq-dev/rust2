//! Where a raw pointer a body makes points, as far as the body shows. JS keeps a value in
//! no array, and a shared slice of a byte in no array, apart from what holds them in Rust,
//! so a pointer leaving one, or measured against another, fails at run time; where the
//! body proves that it does, translation refuses it with a span instead. A pointer the
//! body takes from elsewhere, such as a field or a parameter, is left to the runtime.
//!
//! So is whether a pointer to a value JS copies is an `Arc`'s, the one place of its value,
//! or a borrow's, a place of its own: only an `Arc`'s is an identity, which `from_raw`
//! takes back to it and a cast to `*const ()` keeps.

use rustc_middle::mir::{self, BasicBlock, Local, Location, Operand, ProjectionElem, Rvalue, TerminatorKind};
use rustc_middle::ty::{self, GenericArgsRef};
use rustc_span::Spanned;

use super::Fx;
use crate::lower::library::{self, Std};
use crate::lower::pointer::Pointee;
use crate::ts::{Expr, Stmt};

/// The array JS keeps what a pointer points at in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Array<'tcx> {
	/// An array of one, around the place a `&raw` here took, which each `&raw` makes anew.
	Lone(Location),
	/// A shared slice JS copies, of a byte in no array, made by the call ending this block,
	/// with its length if the body fixes it.
	Copy(BasicBlock, Option<i128>),
	/// The array of a variable of this body's own, or of a field of one, at this place.
	Owned(mir::Place<'tcx>),
}

impl<'tcx> Array<'tcx> {
	/// How many elements the array holds, if the body shows it.
	fn len(self) -> Option<i128> {
		match self {
			Array::Lone(_) => Some(1),
			Array::Copy(_, len) => len,
			Array::Owned(_) => None,
		}
	}

	/// What the array is, as a refusal names it.
	fn what(self) -> &'static str {
		match self {
			Array::Lone(_) => "a value in no array",
			Array::Copy(..) => "a shared slice JS copies",
			Array::Owned(_) => "an array of this body's own",
		}
	}
}

/// Where a pointer points: into which array, at which index if the body fixes it.
#[derive(Clone, Copy)]
struct Origin<'tcx> {
	array: Array<'tcx>,
	index: Option<i128>,
}

impl<'tcx> Fx<'_, '_, '_, 'tcx> {
	/// Why the call of std's `def_id` would fail at run time where Rust's is defined, if the
	/// body proves it would: `offset_from` between pointers JS keeps in different arrays, a
	/// raw or mutable slice of a byte in no array of bytes, or a pointer, slice, or copy
	/// reaching out of an array of one or a copied byte.
	pub(super) fn stray(
		&self,
		def_id: rustc_hir::def_id::DefId,
		gargs: GenericArgsRef<'tcx>,
		args: &[Spanned<Operand<'tcx>>],
	) -> Option<String> {
		if !library::is_std(self.tcx, def_id) {
			return None;
		}
		let (owner, name) = library::key(self.tcx, def_id)?;
		let arg = |i: usize| args.get(i).map(|a| &a.node);
		let pointee = gargs.types().next()?;
		if self.b.t.zero_sized(pointee) || self.b.t.pointee(pointee) != Pointee::Place {
			return None;
		}
		// Only a cast types memory from `alloc`, which has no bytes before or after.
		if matches!(pointee.kind(), ty::Uint(ty::UintTy::U8))
			&& matches!(owner.as_str(), "ptr" | "NonNull" | "raw")
			&& !matches!(name.as_str(), "cast" | "as_ptr" | "new" | "new_unchecked" | "is_null")
			&& args.iter().any(|a| self.allocated(&a.node))
		{
			return Some(super::alloc::BYTES.into());
		}
		let out = |o: Origin<'tcx>, n: Option<i128>, what: &str| {
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
			("ptr" | "NonNull", "cast") => self.erased_borrow(arg(0)?, pointee, gargs.types().nth(1)?),
			("Arc" | "Rc", "from_raw" | "increment_strong_count" | "decrement_strong_count")
				if self.arc_pointer(arg(0)?) == Some(false) =>
			{
				Some(format!(
					"`{owner}::{name}` of a pointer to `{pointee}` a borrow made, whose JS value is a place of its own, which no `{owner}` gave"
				))
			}
			("ptr" | "NonNull", "offset_from" | "offset_from_unsigned") => {
				let (a, b) = (self.origin(arg(0)?)?, self.origin(arg(1)?)?);
				self.apart(a.array, b.array).then(|| {
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
				let byte = crate::lower::byte(pointee);
				let none = self.const_int(arg(1)?) == Some(0);
				if byte && name != "from_raw_parts" && !none && matches!(o.array, Array::Lone(_)) {
					return Some("a raw or mutable slice of a byte in no array of bytes".into());
				}
				// A raw pointer to a slice reads nothing until a read through it.
				if owner != "raw" {
					return None;
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

	/// Whether JS keeps `a` and `b` in different arrays. Arrays of different variables are;
	/// of one variable's, those at different fields or constant indices are, where Rust's
	/// are one allocation.
	fn apart(&self, a: Array<'tcx>, b: Array<'tcx>) -> bool {
		let (Array::Owned(a), Array::Owned(b)) = (a, b) else {
			return a != b;
		};
		let index = |elem: ProjectionElem<Local, ty::Ty<'tcx>>| match elem {
			ProjectionElem::ConstantIndex {
				offset,
				from_end: false,
				..
			} => Some(i128::from(offset)),
			ProjectionElem::Index(local) => match self.def_rvalue(self.traced(&Operand::Copy(local.into()))?)? {
				Rvalue::Use(op, _) => self.const_int(op),
				_ => None,
			},
			_ => None,
		};
		a.local != b.local
			|| a.projection
				.iter()
				.zip(b.projection)
				.find(|(x, y)| x != y)
				.is_some_and(|pair| match pair {
					(ProjectionElem::Field(x, _), ProjectionElem::Field(y, _)) => x != y,
					(x, y) => index(x).zip(index(y)).is_some_and(|(x, y)| x != y),
				})
	}

	/// Where the pointer `op` points, if the body shows it.
	fn origin(&self, op: &Operand<'tcx>) -> Option<Origin<'tcx>> {
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
						[
							array @ ..,
							ProjectionElem::Index(_) | ProjectionElem::ConstantIndex { .. },
						] => (!place.projection.contains(&ProjectionElem::Deref)).then(|| Origin {
							array: Array::Owned(mir::Place {
								local: place.local,
								projection: self.tcx.mk_place_elems(array),
							}),
							index: None,
						}),
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
			("slice" | "Vec", "as_ptr" | "as_mut_ptr") => self.start(&args[0].node),
			_ => None,
		}
	}

	/// `o` moved by the offset `op` as `add`, `sub`, or `offset` moves it.
	fn moved(&self, o: Origin<'tcx>, name: &str, op: &Operand<'tcx>) -> Origin<'tcx> {
		let n = self.const_int(op).map(|n| match name {
			"sub" => -n,
			_ => n,
		});
		Origin {
			index: o.index.zip(n).map(|(i, n)| i + n),
			..o
		}
	}

	/// Where the slice or `Vec`, or reference to one, `op` starts.
	fn start(&self, op: &Operand<'tcx>) -> Option<Origin<'tcx>> {
		let local = self.traced(op)?;
		if self.def_at[local].is_some() {
			return match self.def_rvalue(local)? {
				Rvalue::Use(op, _) | Rvalue::Cast(mir::CastKind::PointerCoercion(..), op, _) => self.start(op),
				Rvalue::Ref(_, _, place) | Rvalue::CopyForDeref(place) => match place.projection.as_slice() {
					[ProjectionElem::Deref] => self.start(&Operand::Copy(mir::Place::from(place.local))),
					projection
						if !projection.contains(&ProjectionElem::Deref)
							&& match place.ty(&self.mir.local_decls, self.tcx).ty.kind() {
								ty::Array(..) => true,
								ty::Adt(adt, _) => Std::of(self.tcx, adt.did()) == Some(Std::Vec),
								_ => false,
							} =>
					{
						Some(Origin {
							array: Array::Owned(*place),
							index: Some(0),
						})
					}
					_ => None,
				},
				_ => None,
			};
		}
		let (def_id, gargs, args) = self.def_call(local)?;
		// A shared slice of a byte in no array is a copy of it, as no `Uint8Array` can share
		// it, unless empty, which keeps the pointer. Any other slice is a view of where it is.
		match library::key(self.tcx, def_id)? {
			(owner, name) if owner == "raw" && matches!(name.as_str(), "from_raw_parts" | "from_raw_parts_mut") => {
				let o = self.origin(&args[0].node)?;
				let len = self.const_int(&args[1].node);
				match (crate::lower::byte(gargs.types().next()?), o.array) {
					(true, Array::Lone(_)) if len != Some(0) => Some(Origin {
						array: Array::Copy(self.def_block(local)?, len),
						index: Some(0),
					}),
					_ => Some(o),
				}
			}
			_ => None,
		}
	}

	/// The place holding the `Arc` or `Rc` whose value the shared reference `op` is, as
	/// `Deref` lends it, and which of the two it is, if the body shows it. The borrow keeps
	/// that place holding the same one while `op` lives.
	pub(super) fn arc_of(&self, op: &Operand<'tcx>) -> Option<(mir::Place<'tcx>, String)> {
		let local = self.traced(op)?;
		if self.def_at[local].is_some() {
			return match self.def_rvalue(local)? {
				Rvalue::Use(op, _) => self.arc_of(op),
				// `&*r`, of the reference `r`.
				Rvalue::Ref(_, mir::BorrowKind::Shared, place) => match place.projection.as_slice() {
					[ProjectionElem::Deref] if self.mir.local_decls[place.local].ty.is_ref() => {
						self.arc_of(&Operand::Copy(mir::Place::from(place.local)))
					}
					_ => None,
				},
				_ => None,
			};
		}
		let (def_id, gargs, args) = self.def_call(local)?;
		if library::key(self.tcx, def_id)? != ("Deref".into(), "deref".into()) {
			return None;
		}
		let ty::Adt(adt, _) = gargs.type_at(0).kind() else {
			return None;
		};
		if Std::of(self.tcx, adt.did()) != Some(Std::Arc) {
			return None;
		}
		let owner = self.tcx.item_name(adt.did()).to_string();
		let arc = &args.first()?.node;
		let place = match self.traced(arc) {
			Some(r) => match self.def_rvalue(r)? {
				Rvalue::Ref(_, _, place) => *place,
				_ => return None,
			},
			// A reference the function is given and never assigns.
			None => {
				let r = arc.place()?.as_local()?;
				if !self.is_arg(r) || self.defs[r] != 0 {
					return None;
				}
				self.tcx.mk_place_deref(mir::Place::from(r))
			}
		};
		// Only through boxes, which the borrow keeps where they are, and a reference in a
		// variable the body never points elsewhere, by assigning or borrowing it, as it may
		// one the borrow went through.
		let l = place.local;
		let fixed = !self.borrowed[l]
			&& !self.mutated[l]
			&& match self.is_arg(l) {
				true => self.defs[l] == 0,
				false => self.defs[l] == 1,
			};
		let stays = place
			.iter_projections()
			.enumerate()
			.all(|(i, (base, elem))| match elem {
				ProjectionElem::Field(..) => true,
				ProjectionElem::Deref => {
					let ty = base.ty(&self.mir.local_decls, self.tcx).ty;
					ty.is_box() || (i == 0 && fixed && ty.is_ref())
				}
				_ => false,
			});
		stays.then_some((place, owner))
	}

	/// `Arc::as_ptr` of the `Arc` or `Rc` whose value of a type JS copies the shared
	/// reference `r` is, if the body shows it: that value's one place, which `from_raw`
	/// takes back to it, rather than a place of its own as a pointer from a reference is.
	pub(super) fn arc_raw(&mut self, r: &Operand<'tcx>, span: rustc_span::Span, out: &mut Vec<Stmt>) -> Option<Expr> {
		let pointee = r.ty(&self.mir.local_decls, self.tcx).builtin_deref(true)?;
		if self.b.t.pointee(pointee) != Pointee::Place {
			return None;
		}
		let (place, owner) = self.arc_of(r)?;
		let arc = self.place_expr(place, span, out);
		let arc = match arc.unlocated() {
			Expr::Ident(_) => arc,
			_ => self.b.spill(arc, "arc", out),
		};
		let p = Expr::rs("arcPlace", vec![arc.clone()]);
		Some(Expr::rs("arcRaw", vec![arc, p, Expr::Str(owner)]))
	}

	/// Whether the raw pointer `op` to a value JS copies is an `Arc`'s or `Rc`'s, as
	/// `into_raw` or `as_ptr` gives, rather than a borrow's, if the body shows where it came
	/// from: an `Arc`'s is the one place of its value, and a borrow's a place of its own.
	pub(super) fn arc_pointer(&self, op: &Operand<'tcx>) -> Option<bool> {
		let local = self.traced(op)?;
		if self.def_at[local].is_some() {
			return match self.def_rvalue(local)? {
				Rvalue::Use(op, _)
				| Rvalue::Cast(
					mir::CastKind::PtrToPtr
					| mir::CastKind::PointerCoercion(ty::adjustment::PointerCoercion::MutToConstPointer, _),
					op,
					_,
				) if self.same_pointee(op, local) => self.arc_pointer(op),
				Rvalue::RawPtr(mir::RawPtrKind::Const | mir::RawPtrKind::Mut, place) => {
					match place.projection.as_slice() {
						[ProjectionElem::Deref] => {
							let base = Operand::Copy(mir::Place::from(place.local));
							match self.mir.local_decls[place.local].ty.is_raw_ptr() {
								true => self.arc_pointer(&base),
								false => Some(self.arc_of(&base).is_some()),
							}
						}
						_ => Some(false),
					}
				}
				_ => None,
			};
		}
		let (def_id, _, args) = self.def_call(local)?;
		let (owner, name) = library::key(self.tcx, def_id)?;
		match (owner.as_str(), name.as_str()) {
			("Arc" | "Rc", "into_raw" | "as_ptr") => Some(true),
			("ptr", "from_ref" | "from_mut") | ("From", "from") => Some(self.arc_of(&args.first()?.node).is_some()),
			("ptr" | "NonNull", "cast_const" | "cast_mut") | ("NonNull", "as_ptr" | "new_unchecked") => {
				self.arc_pointer(&args.first()?.node)
			}
			("UnsafeCell", "get" | "raw_get") | ("Cell", "as_ptr") | ("MaybeUninit", "as_ptr" | "as_mut_ptr") => {
				Some(false)
			}
			_ => None,
		}
	}

	/// Why the cast of the raw pointer `op` to `from` to one to `to` can't be, if it erases
	/// one to a value JS copies that the body shows is a borrow's place to a `*const ()`, and
	/// the cast itself refuses no pointer to `from`.
	pub(super) fn erased_borrow(&self, op: &Operand<'tcx>, from: ty::Ty<'tcx>, to: ty::Ty<'tcx>) -> Option<String> {
		let t = &self.b.t;
		let erases = t.identity_pointee(to) && !t.identity_pointee(from) && t.pointee(from) == Pointee::Place;
		(erases && crate::lower::pointer::unerasable(self.b, from).is_none() && self.arc_pointer(op) == Some(false))
			.then(|| crate::lower::pointer::erased_place(t.uninit_inner(from)))
	}

	/// The local `op` reads whole, if the body assigns it once.
	pub(super) fn traced(&self, op: &Operand<'tcx>) -> Option<Local> {
		let local = op.place()?.as_local()?;
		(self.defs[local] == 1 && !self.is_arg(local)).then_some(local)
	}

	/// Whether `op` is a pointer to what `local` points at.
	fn same_pointee(&self, op: &Operand<'tcx>, local: Local) -> bool {
		let pointee = |ty: ty::Ty<'tcx>| ty.builtin_deref(true);
		let from = op.ty(&self.mir.local_decls, self.tcx);
		pointee(from).is_some() && pointee(from) == pointee(self.mir.local_decls[local].ty)
	}

	/// The function, its generic arguments, and its arguments, of the call of std's that
	/// assigns `local`.
	pub(super) fn def_call(
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
		library::is_std(self.tcx, def_id).then_some((def_id, gargs.skip_binder(), args))
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
