//! Raw pointers, `NonNull`, and `MaybeUninit`.
//!
//! A raw pointer is what a `&mut` to its pointee is where JS keeps the pointee where Rust
//! does: the object, a tuple's or array's or the crate's own struct's, or the
//! `rt.ByteSlice` of a `[u8; N]`'s bytes, which may be part of a larger array or a slice.
//! To anything else it is an `rt.Place` of the place holding the value, as a `&mut` to a
//! value JS copies is, which a `Vec` or an `Arc` also is here: its JS object is what it
//! owns, not where Rust keeps it. Null is `rt.NULL`, and a dangling pointer `rt.DANGLING`.
//!
//! A pointer to `()`, as std erases one, is an identity that compares as itself: null,
//! `rt.DANGLING_UNIT`, or a waker's data. One to a place of `()` or from a reference to
//! one is refused, as an `rt.Place` of it would be an identity of its own each borrow.
//!
//! A whole value written through a pointer to an object is copied into that object, so
//! every pointer to it sees the write. That goes one object deep, as `*r = v` does through
//! a `&mut`, so a pointer to a part kept as an object is refused wherever the crate may
//! give that part another object. One to a variable's object is refused wherever the
//! function may give the variable another after: by assigning it, by writing it whole
//! through an alias of a borrow of it, or by `mem::replace`, `swap`, or `take` of one. A
//! value read out is a copy of what the crate writes into in place, so a later write
//! leaves it be.
//!
//! A `MaybeUninit<T>` is a `T`: `undefined` where JS copies a `T` and it holds none yet,
//! or an object whose parts hold none yet where a pointer to it is the object. An
//! `UnsafeCell<T>` is an `rt.Cell`, which is the place a pointer to `T` is, or else holds
//! the object it is.

use rustc_middle::ty::{self, Ty, TypeVisitableExt};
use rustc_span::Span;

use super::body::Body;
use super::library::{Call, Std, targ};
use super::{AdtRepr, ObjectKind, Translator};
use crate::ts::{Expr, Key, Stmt, StmtKind, Type};

/// What a raw pointer to a type is in JS.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pointee {
	/// An `rt.Place` of the value, which JS copies.
	Place,
	/// The object it points at.
	Object,
	/// An `rt.ByteSlice` of the `[u8; N]` it points at, as a reference to one is, which
	/// compares by where its bytes are.
	Bytes,
}

impl<'a, 'tcx> Translator<'a, 'tcx> {
	/// What a raw pointer to `ty` is.
	pub fn pointee(&self, ty: Ty<'tcx>) -> Pointee {
		// Only an instance decides what a parameter is.
		if self.polymorphic() && ty.has_param() {
			return Pointee::Object;
		}
		let ty = self.uninit_inner(ty);
		if super::byte_array(ty) {
			return Pointee::Bytes;
		}
		let object = match (self.object_kind(ty), ty.kind()) {
			(Some(ObjectKind::Array | ObjectKind::Bytes), ty::Array(..) | ty::Tuple(_)) => true,
			// An async body passes its context as a pointer to it, and a waker's vtable is
			// one that is its identity.
			(Some(ObjectKind::Class), ty::Adt(adt, _)) => match Std::of(self.tcx, adt.did()) {
				Some(std) => matches!(std, Std::Context | Std::WakerVTable),
				None => self.bound_drops(adt.did()).is_none(),
			},
			_ => false,
		};
		match object {
			true => Pointee::Object,
			false => Pointee::Place,
		}
	}

	/// Whether a pointer to `ty` is an identity: one to `()`, through any `MaybeUninit`s.
	pub fn identity_pointee(&self, ty: Ty<'tcx>) -> bool {
		self.uninit_inner(ty).is_unit()
	}

	/// Why pointers to `ty` that are not null don't compare as their JS values, if they
	/// don't. Two `rt.Place`s of one place are two objects, and Rust leaves where a value of
	/// no bytes is unspecified, but a pointer to `()` is an identity.
	pub fn incomparable(&self, ty: Ty<'tcx>) -> Option<String> {
		if self.identity_pointee(ty) {
			return None;
		}
		if self.zero_sized(ty) {
			return Some(format!(
				"comparing pointers to `{ty}`, which has no bytes and so no address of its own"
			));
		}
		match self.pointee(ty) {
			Pointee::Object | Pointee::Bytes => None,
			Pointee::Place => Some(format!(
				"comparing pointers to `{ty}`, each a place JS makes of its own"
			)),
		}
	}

	/// What a `MaybeUninit` holds, through any number of them, or `ty` itself.
	pub fn uninit_inner(&self, ty: Ty<'tcx>) -> Ty<'tcx> {
		let mut ty = ty;
		while let ty::Adt(adt, args) = ty.kind()
			&& Std::of(self.tcx, adt.did()) == Some(Std::MaybeUninit)
		{
			ty = args.type_at(0);
		}
		ty
	}

	/// The TypeScript type of a raw pointer or `NonNull` to `pointee`.
	pub fn pointer_ty(&mut self, pointee: Ty<'tcx>, span: Span) -> Type {
		match self.pointee(pointee) {
			Pointee::Place => Type::Rt("Place", vec![self.ty(pointee, span)]),
			Pointee::Object => self.ty(pointee, span),
			Pointee::Bytes => Type::Rt("ByteSlice", Vec::new()),
		}
	}

	/// Whether a `ty` has no bytes, so a write of one through any pointer, even a dangling
	/// one, writes nothing.
	pub fn zero_sized(&self, ty: Ty<'tcx>) -> bool {
		let env = self.typing_env_pub();
		self.tcx
			.layout_of(env.as_query_input(ty))
			.is_ok_and(|layout| layout.is_zst())
	}

	/// Whether a `ty` of no bytes is the value [`Self::shell`] makes of it: built of
	/// tuples, arrays, structs, and `()`, with no enum to tag.
	fn empty(&self, ty: Ty<'tcx>) -> bool {
		match ty.kind() {
			ty::Tuple(tys) => tys.iter().all(|t| self.empty(t)),
			ty::Array(elem, _) => self.empty(*elem),
			ty::Adt(adt, args) => match Std::of(self.tcx, adt.did()) {
				Some(Std::MaybeUninit) => self.empty(args.type_at(0)),
				Some(_) => false,
				None => adt.is_struct() && adt.all_fields().all(|f| self.empty(self.field_ty(f.did, args))),
			},
			_ => false,
		}
	}

	/// What `ty` points at, if it is a raw pointer that is an `rt.Place`.
	pub fn raw_place(&self, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
		match ty.kind() {
			ty::RawPtr(inner, _) if self.pointee(*inner) == Pointee::Place => Some(*inner),
			_ => None,
		}
	}

	/// A `MaybeUninit<ty>` that holds nothing yet: its parts kept as objects are there, so
	/// a pointer to one can initialize it, holding nothing yet themselves.
	pub fn uninit(&mut self, ty: Ty<'tcx>, span: Span) -> Expr {
		let ts_ty = self.ty(ty, span);
		let empty = self.shell(ty, span);
		Expr::As(Box::new(Expr::As(Box::new(empty), Type::Builtin("unknown"))), ts_ty)
	}

	/// What a `MaybeUninit<ty>` that holds nothing yet is, untyped.
	pub fn shell(&mut self, ty: Ty<'tcx>, span: Span) -> Expr {
		let ty = self.uninit_inner(ty);
		match (self.pointee(ty), ty.kind()) {
			(Pointee::Object, ty::Tuple(tys)) => Expr::Array(tys.iter().map(|t| self.shell(t, span)).collect()),
			(Pointee::Object | Pointee::Bytes, ty::Array(elem, len)) => {
				let len = Expr::lit(len.try_to_target_usize(self.tcx).unwrap_or(0).to_string());
				match (elem.kind(), self.pointee(*elem)) {
					(ty::Uint(ty::UintTy::U8), _) => Expr::New(Box::new(Expr::ident("Uint8Array")), vec![len]),
					(_, Pointee::Place) => Expr::New(Box::new(Expr::ident("Array")), vec![len]),
					(_, Pointee::Object | Pointee::Bytes) => {
						let elem_ty = self.ty(*elem, span);
						let each = Expr::Arrow(
							Vec::new(),
							elem_ty,
							vec![StmtKind::Return(Some(self.uninit(*elem, span))).at(None)],
						);
						let length = Expr::Object(vec![(Key::Name("length".into()), len)]);
						Expr::ident("Array").member("from").call(vec![length, each])
					}
				}
			}
			(Pointee::Object, ty::Adt(adt, args)) => {
				let info = self.adt(*adt, span);
				match info.repr {
					AdtRepr::Struct { ctor, fields } => {
						let tys: Vec<Ty<'tcx>> = adt
							.non_enum_variant()
							.fields
							.iter()
							.map(|f| self.field_ty(f.did, args))
							.collect();
						// A class's constructor types each of its parameters.
						let plain = self.plain(info.sym);
						let values = tys
							.into_iter()
							.map(|t| match plain {
								true => self.shell(t, span),
								false => self.uninit(t, span),
							})
							.collect();
						self.construct(info.sym, &ctor, &fields, values)
					}
					AdtRepr::Slots(_) => Expr::Object(Vec::new()),
					_ => unreachable!("only a struct or union is an object a pointer points at"),
				}
			}
			_ => Expr::undefined(),
		}
	}
}

/// Whether std's `def_id` with `args` makes a raw pointer to what its first argument, a
/// reference, points at: `UnsafeCell::get`, `MaybeUninit::as_ptr` and `as_mut_ptr`, and
/// `NonNull::from` and `into`.
pub fn from_reference<'tcx>(
	tcx: ty::TyCtxt<'tcx>,
	def_id: rustc_hir::def_id::DefId,
	args: ty::GenericArgsRef<'tcx>,
) -> bool {
	let Some((owner, name)) = super::library::key(tcx, def_id) else {
		return false;
	};
	match (owner.as_str(), name.as_str()) {
		("UnsafeCell", "get") | ("MaybeUninit", "as_ptr" | "as_mut_ptr") => true,
		("From", "from") | ("Into", "into") => {
			let (to, from) = match owner.as_str() {
				"From" => (args.type_at(0), args.type_at(1)),
				_ => (args.type_at(1), args.type_at(0)),
			};
			matches!(to.kind(), ty::Adt(adt, _) if Std::of(tcx, adt.did()) == Some(Std::NonNull)) && from.is_ref()
		}
		_ => false,
	}
}

/// What the value std's `def_id` returns points at, if it is a raw pointer, `NonNull`,
/// `MaybeUninit`, `UnsafeCell`, or `Box` function that runs none of the crate's code and
/// keeps what it is given nowhere but in what it returns.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Derives {
	/// Where its first argument points, as a whole.
	Same,
	/// Anywhere its arguments reach, if anywhere.
	Other,
	/// Wherever what it reads where its first argument points does.
	Loaded,
}

/// How std's `def_id` derives what it returns, if [`Derives`] describes it.
pub fn derives(tcx: ty::TyCtxt<'_>, def_id: rustc_hir::def_id::DefId) -> Option<Derives> {
	if !super::library::is_std(tcx, def_id) {
		return None;
	}
	let (owner, name) = super::library::key(tcx, def_id)?;
	match (owner.as_str(), name.as_str()) {
		("ptr", "cast" | "cast_const" | "cast_mut" | "as_ref" | "as_mut")
		| ("NonNull", "new" | "new_unchecked" | "as_ptr" | "as_ref" | "as_mut" | "cast" | "cast_const" | "cast_mut")
		| ("MaybeUninit", "as_ptr" | "as_mut_ptr" | "assume_init_ref" | "assume_init_mut" | "write")
		| ("Box", "into_raw" | "leak" | "from_raw") => Some(Derives::Same),
		(
			"ptr",
			"null" | "null_mut" | "dangling" | "dangling_mut" | "is_null" | "eq" | "add" | "sub" | "offset"
			| "wrapping_add" | "wrapping_sub" | "wrapping_offset" | "write" | "write_unaligned" | "write_volatile",
		)
		| ("NonNull", "dangling" | "add" | "sub" | "offset" | "write")
		| ("MaybeUninit", "new" | "uninit" | "assume_init")
		| ("UnsafeCell", "new" | "get" | "raw_get" | "get_mut" | "into_inner")
		| ("Box", "new") => Some(Derives::Other),
		("ptr", "read" | "read_unaligned" | "read_volatile")
		| ("NonNull", "read")
		| ("MaybeUninit", "assume_init_read") => Some(Derives::Loaded),
		_ => None,
	}
}

/// Whether std's `def_id`, which [`derives`] describes, keeps its second argument where its
/// first points.
pub fn stores(tcx: ty::TyCtxt<'_>, def_id: rustc_hir::def_id::DefId) -> bool {
	matches!(
		super::library::key(tcx, def_id),
		Some((owner, name)) if matches!(owner.as_str(), "ptr" | "NonNull" | "MaybeUninit")
			&& matches!(name.as_str(), "write" | "write_unaligned" | "write_volatile")
	)
}

/// Whether std's `def_id` returns memory nothing points into yet.
pub fn allocates(tcx: ty::TyCtxt<'_>, def_id: rustc_hir::def_id::DefId) -> bool {
	super::library::is_std(tcx, def_id)
		&& matches!(
			super::library::key(tcx, def_id),
			Some((owner, name)) if matches!(
				(owner.as_str(), name.as_str()),
				("alloc", "alloc" | "alloc_zeroed") | ("Box", "new" | "new_uninit" | "new_zeroed")
			)
		)
}

/// How many of std's `def_id`'s first arguments are `&mut`s whose lowering gives what
/// they borrow another value, rather than writing into it, where one is an alias of a
/// borrowed place: the place is assigned again. `mem::replace`'s value is no such one.
pub fn rebinds(tcx: ty::TyCtxt<'_>, def_id: rustc_hir::def_id::DefId) -> usize {
	match super::library::key(tcx, def_id) {
		Some((owner, name)) if owner == "mem" => match name.as_str() {
			"replace" | "take" => 1,
			"swap" => 2,
			_ => 0,
		},
		_ => 0,
	}
}

/// Whether std's `def_id` is one this module maps that lends a place: it makes the
/// `rt.Place` of the `&mut` it returns.
pub fn lends(tcx: ty::TyCtxt<'_>, def_id: rustc_hir::def_id::DefId) -> bool {
	matches!(
		super::library::key(tcx, def_id),
		Some((owner, name)) if matches!(
			(owner.as_str(), name.as_str()),
			("MaybeUninit", "as_mut_ptr" | "write" | "assume_init_mut" | "assume_init_drop")
				| ("UnsafeCell", "get_mut")
				| ("NonNull" | "ptr", "as_mut")
				| ("Box", "leak")
		)
	)
}

/// Whether std's `def_id` with `args` is one that only makes a pointer or points one
/// elsewhere, reads through it, or drops what it points at, as this module maps them, or
/// an atomic's, which holds a pointer as a value, so it gives no part another object.
pub fn keeps<'tcx>(tcx: ty::TyCtxt<'tcx>, def_id: rustc_hir::def_id::DefId, args: ty::GenericArgsRef<'tcx>) -> bool {
	let keeps = matches!(
		super::library::key(tcx, def_id),
		Some((owner, name)) if matches!(
			(owner.as_str(), name.as_str()),
			(
				"MaybeUninit",
				"new" | "uninit" | "as_ptr" | "as_mut_ptr" | "assume_init" | "assume_init_ref" | "assume_init_mut"
					| "assume_init_read" | "assume_init_drop"
			) | (
				"NonNull",
				"new" | "new_unchecked" | "dangling" | "as_ptr" | "as_ref" | "as_mut" | "cast" | "cast_const" | "cast_mut"
			) | (
				"ptr",
				"null" | "null_mut" | "dangling" | "dangling_mut" | "is_null" | "as_ref" | "as_mut" | "cast" | "cast_const"
					| "cast_mut" | "eq" | "read" | "read_unaligned" | "read_volatile" | "drop_in_place"
			) | ("Box", "into_raw" | "leak" | "from_raw")
				| ("UnsafeCell", "new" | "get" | "get_mut" | "into_inner")
		) || owner == "Atomic" && super::library::is_std(tcx, def_id)
	);
	keeps || from_reference(tcx, def_id, args)
}

/// Whether std's `def_id` is one a mapping takes the `rt.Place` a `&mut` is for, rather
/// than what it points at: a `MaybeUninit`'s, which it points into, an `Arc`'s that
/// `make_mut` replaces, a `String`'s that a drain edits as it drops, and a well-known
/// crate's, such as a `Bytes` that `Buf::advance` moves past its front.
pub fn takes_place(tcx: ty::TyCtxt<'_>, def_id: rustc_hir::def_id::DefId) -> bool {
	matches!(
		super::library::key(tcx, def_id),
		Some((owner, name)) if owner == "MaybeUninit"
			|| matches!(owner.as_str(), "Arc" | "Rc") && name == "make_mut"
			|| (owner.as_str(), name.as_str()) == ("String", "drain")
	) || super::stock::is_stock(tcx, def_id)
}

/// Which of std's `def_id`'s parameters its mapping takes as what the `&mut` it declares
/// points at, rather than the `rt.Place` that `&mut` may be: not a type parameter that
/// one instantiates, as `Pin::new`'s pointer.
pub fn pointees(tcx: ty::TyCtxt<'_>, def_id: rustc_hir::def_id::DefId) -> Vec<bool> {
	let takes_place = takes_place(tcx, def_id);
	let sig = tcx.fn_sig(def_id).instantiate_identity().skip_norm_wip();
	tcx.instantiate_bound_regions_with_erased(sig)
		.inputs()
		.iter()
		.map(|t| !takes_place && matches!(t.kind(), ty::Ref(_, _, rustc_ast::Mutability::Mut)))
		.collect()
}

/// A call of std's raw pointer, `NonNull`, `MaybeUninit`, `UnsafeCell`, or `Box` functions
/// that make or take a raw pointer: its mapping, `None` if it has none, or its arguments
/// back if it is no such call.
pub fn call<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	call: &Call<'tcx>,
	owner: &str,
	name: &str,
	args: Vec<Expr>,
	out: &mut Vec<Stmt>,
) -> Result<Option<Expr>, Vec<Expr>> {
	let span = call.span;
	let gargs = call.args;
	let pointee = || targ(gargs, 0);
	let kind = |b: &Body<'_, '_, 'tcx>, ty: Ty<'tcx>| Some(b.t.pointee(ty));
	// A mapping holds a `[u8; N]`, and hands out a reference to one, as the `Uint8Array`,
	// which a pointer to it is an `rt.ByteSlice` of.
	let bytes = |b: &Body<'_, '_, 'tcx>, ty: Ty<'tcx>| b.t.pointee(ty) == Pointee::Bytes;
	let to_pointer = |e: Expr| Expr::rt("byteSliceFrom", vec![e]);
	let to_reference = |e: Expr| Expr::rt("byteSliceView", vec![e]);
	let mut args = args.into_iter();
	let mut arg = || args.next().expect("an argument");
	let e = match (owner, name) {
		("ptr", "null" | "null_mut") => Expr::Rt("NULL".into()),
		// A pointer to `()` is an identity, which the `rt.Place` of a place or a reference
		// is not. A box of one holds nothing, so its pointer is dangling.
		("MaybeUninit", "as_ptr" | "as_mut_ptr") | ("UnsafeCell", "get") if b.t.identity_pointee(pointee()) => b.err(
			span,
			"a raw pointer to a place of `()`, which has no address of its own",
		),
		("From", "from") if pointer_self(b, call).is_some_and(|t| b.t.identity_pointee(t)) => b.err(
			span,
			"a raw pointer to a place of `()`, which has no address of its own",
		),
		("Box", "into_raw" | "leak") if b.t.identity_pointee(pointee()) => Expr::Rt("DANGLING_UNIT".into()),
		// One to a value of no bytes points at one, which every read of it reads. One to
		// `()` is an identity, the same for every call, as its address is.
		("NonNull", "dangling") | ("ptr", "dangling" | "dangling_mut") if b.t.identity_pointee(pointee()) => {
			Expr::Rt("DANGLING_UNIT".into())
		}
		("NonNull", "dangling") | ("ptr", "dangling" | "dangling_mut") if b.t.zero_sized(pointee()) => {
			match (b.t.empty(pointee()), kind(b, pointee())) {
				(false, _) => b.err(
					span,
					format!(
						"a dangling pointer to `{}`, a value of no bytes made of more than structs and tuples",
						pointee()
					),
				),
				(true, Some(Pointee::Place)) => Expr::Object(vec![(Key::Name("v".into()), b.t.shell(pointee(), span))]),
				(true, _) => b.t.uninit(pointee(), span),
			}
		}
		("NonNull", "dangling") | ("ptr", "dangling" | "dangling_mut") => Expr::Rt("DANGLING".into()),
		// The same pointer, as one to what a `MaybeUninit` holds or the other way.
		("ptr" | "NonNull", "cast") => {
			let to = targ(gargs, 1);
			match b.t.uninit_inner(pointee()) == b.t.uninit_inner(to) {
				true => arg(),
				false => b.err(
					span,
					format!(
						"a cast from a pointer to `{}` to one to `{to}`, which reads the bytes of one type as another",
						pointee()
					),
				),
			}
		}
		("ptr" | "NonNull", "addr" | "expose_provenance" | "with_addr" | "map_addr" | "without_provenance") => {
			b.err(span, "a pointer's address, which JS has no counterpart of")
		}
		("ptr", "is_null") => arg().binary("===", Expr::Rt("NULL".into())),
		("NonNull", "as_mut") if bytes(b, pointee()) => to_reference(arg()),
		// The same pointer, as another type of the same pointee.
		("ptr", "cast_const" | "cast_mut")
		| ("NonNull", "as_ptr" | "new_unchecked" | "cast_const" | "cast_mut")
		| ("MaybeUninit", "new" | "assume_init" | "assume_init_mut")
		| ("NonNull", "as_mut")
		| ("Clone", "clone")
			if owner != "Clone" || pointer_self(b, call).is_some() =>
		{
			arg()
		}
		// A shared reference to a value JS copies is the value, which no write changes.
		("From", "from") if let Some(ty) = pointer_self(b, call) => {
			let from = targ(gargs, 1);
			match (kind(b, ty), from.kind()) {
				(Some(Pointee::Place), ty::Ref(_, _, ty::Mutability::Not)) => {
					Expr::Object(vec![(crate::ts::Key::Name("v".into()), arg())])
				}
				// A reference to a `MaybeUninit` of bytes is the `Uint8Array`.
				(Some(Pointee::Bytes), _) if !super::bytes_ref(from) => to_pointer(arg()),
				_ => arg(),
			}
		}
		("NonNull", "new") => {
			let p = b.spill(arg(), "pointer", out);
			Expr::Cond(
				Box::new(p.clone().binary("===", Expr::Rt("NULL".into()))),
				Box::new(Expr::undefined()),
				Box::new(p),
			)
		}
		// `Option<&T>`: a shared reference to a value JS copies is the value.
		("ptr", "as_ref" | "as_mut") => {
			let p = b.spill(arg(), "pointer", out);
			let shared = name == "as_ref" && kind(b, pointee()) == Some(super::pointer::Pointee::Place);
			let (some, none) = match shared {
				true => (p.clone().member("v"), b.t.none(pointee())),
				false if bytes(b, pointee()) => (to_reference(p.clone()), Expr::undefined()),
				false => (p.clone(), Expr::undefined()),
			};
			Expr::Cond(
				Box::new(p.binary("===", Expr::Rt("NULL".into()))),
				Box::new(none),
				Box::new(some),
			)
		}
		("NonNull", "as_ref") | ("MaybeUninit", "assume_init_ref") => match kind(b, pointee()) {
			Some(Pointee::Place) if owner == "NonNull" => arg().member("v"),
			Some(Pointee::Bytes) if owner == "NonNull" => to_reference(arg()),
			_ => arg(),
		},
		// A pointer to what a box held, which no box holds any longer.
		("Box", "into_raw" | "leak") => match kind(b, pointee()) {
			Some(Pointee::Place) => Expr::Object(vec![(crate::ts::Key::Name("v".into()), arg())]),
			Some(Pointee::Bytes) if name == "into_raw" => to_pointer(arg()),
			_ => arg(),
		},
		("Box", "from_raw") => match kind(b, pointee()) {
			Some(Pointee::Place) => arg().member("v"),
			Some(Pointee::Bytes) => to_reference(arg()),
			_ => arg(),
		},
		("ptr", "read" | "read_unaligned" | "read_volatile") => {
			let p = arg();
			read(b, p, pointee(), span)
		}
		("ptr", "write" | "write_unaligned" | "write_volatile") => {
			let (p, value) = (arg(), arg());
			write(b, p, value, pointee(), span)
		}
		// A place of no bytes has nothing to read or write, and every value of its type is
		// the one it holds.
		("ptr", "replace") if b.t.zero_sized(pointee()) => {
			let _ = arg();
			arg()
		}
		("ptr", "swap") if b.t.zero_sized(pointee()) => Expr::undefined(),
		("ptr", "replace") => {
			let p = b.spill(arg(), "pointer", out);
			let value = arg();
			let old = read(b, p.clone(), pointee(), span);
			let old = b.spill(old, "old", out);
			let write = write(b, p, value, pointee(), span);
			out.push(StmtKind::Expr(write).at(None));
			old
		}
		("ptr", "swap") => {
			let (x, y) = (b.spill(arg(), "x", out), b.spill(arg(), "y", out));
			let old = read(b, x.clone(), pointee(), span);
			let old = b.spill(old, "old", out);
			let from_y = read(b, y.clone(), pointee(), span);
			let into_x = write(b, x, from_y, pointee(), span);
			out.push(StmtKind::Expr(into_x).at(None));
			write(b, y, old, pointee(), span)
		}
		("ptr", "drop_in_place") => {
			let p = arg();
			let value = match kind(b, pointee()) {
				Some(Pointee::Place) => p.member("v"),
				_ => p,
			};
			b.t.drop_value(value, pointee(), span).unwrap_or_else(Expr::undefined)
		}
		("ptr", "eq") => {
			let (x, y) = (arg(), arg());
			compare(b, x, y, pointee(), span)
		}
		("PartialEq", "eq" | "ne") if let Some(ty) = pointer_self(b, call) => {
			let (x, y) = (arg(), arg());
			let same = compare(b, x, y, ty, span);
			match name {
				"eq" => same,
				_ => same.not(),
			}
		}
		("MaybeUninit", "uninit") => b.t.uninit(pointee(), span),
		// A shared reference to a value JS copies is the value, which no write changes.
		("MaybeUninit", "as_ptr") if kind(b, pointee()) == Some(Pointee::Place) => {
			Expr::Object(vec![(crate::ts::Key::Name("v".into()), arg())])
		}
		("MaybeUninit", "as_ptr" | "as_mut_ptr") if bytes(b, pointee()) => to_pointer(arg()),
		("MaybeUninit", "as_ptr" | "as_mut_ptr") => arg(),
		("MaybeUninit", "write") => {
			let p = b.spill(arg(), "uninit", out);
			let value = arg();
			let write = match bytes(b, pointee()) {
				true => Expr::rt("ptrWrite", vec![p.clone(), value]),
				false => write(b, p.clone(), value, pointee(), span),
			};
			out.push(StmtKind::Expr(write).at(None));
			p
		}
		// A shared reference to a value JS copies is the value.
		("MaybeUninit", "assume_init_read") => b.t.snapshot(arg(), pointee(), span),
		("MaybeUninit", "assume_init_drop") => {
			let p = arg();
			let value = match kind(b, pointee()) {
				Some(Pointee::Place) => p.member("v"),
				_ => p,
			};
			b.t.drop_value(value, pointee(), span).unwrap_or_else(Expr::undefined)
		}
		// An `rt.Cell`, which is the place a pointer to a value JS copies is, and holds the
		// object a pointer to any other is.
		("UnsafeCell", "new") => Expr::New(Box::new(Expr::Rt("Cell".into())), vec![arg()]),
		("UnsafeCell", "get") if kind(b, pointee()) == Some(Pointee::Place) => arg(),
		("UnsafeCell", "get") if bytes(b, pointee()) => to_pointer(arg().member("v")),
		("UnsafeCell", "get_mut") if b.t.needs_place(pointee()) => arg(),
		("UnsafeCell", "get" | "get_mut" | "into_inner") => arg().member("v"),
		("mem", "forget") => Expr::undefined(),
		_ => return Err(args.collect()),
	};
	Ok(Some(e))
}

/// What a `PartialEq` or `Clone` call's `Self` points at, if it is a raw pointer or a
/// `NonNull`.
fn pointer_self<'tcx>(b: &Body<'_, '_, 'tcx>, call: &Call<'tcx>) -> Option<Ty<'tcx>> {
	let this = call.args.types().next()?;
	match this.kind() {
		ty::RawPtr(pointee, _) => Some(*pointee),
		ty::Adt(adt, args) if Std::of(b.t.tcx, adt.did()) == Some(Std::NonNull) => Some(args.type_at(0)),
		_ => None,
	}
}

/// The value `p`, a pointer to `ty`, points at, as a value of its own.
fn read<'tcx>(b: &mut Body<'_, '_, 'tcx>, p: Expr, ty: Ty<'tcx>, span: Span) -> Expr {
	let value = match b.t.pointee(ty) {
		Pointee::Place => p.member("v"),
		Pointee::Object => p,
		Pointee::Bytes => return Expr::rt("byteSliceToArray", vec![p]),
	};
	b.t.snapshot(value, ty, span)
}

/// The write of `value`, a `ty`, where `p` points.
fn write<'tcx>(b: &mut Body<'_, '_, 'tcx>, p: Expr, value: Expr, ty: Ty<'tcx>, span: Span) -> Expr {
	let _ = span;
	if b.t.zero_sized(ty) {
		return Expr::undefined();
	}
	match b.t.pointee(ty) {
		Pointee::Place => p.member("v").assign(value),
		Pointee::Object => Expr::rt("ptrWrite", vec![p, value]),
		Pointee::Bytes => Expr::rt("byteSliceSet", vec![p, value]),
	}
}

/// Whether pointers `x` and `y` to `ty` point at the same place: the same object or
/// bytes, or null both.
pub fn compare<'tcx>(b: &mut Body<'_, '_, 'tcx>, x: Expr, y: Expr, ty: Ty<'tcx>, span: Span) -> Expr {
	let null = |e: &Expr| matches!(e, Expr::Rt(n) if n == "NULL");
	match b.t.incomparable(ty) {
		Some(why) if !null(&x) && !null(&y) => b.err(span, why),
		_ if b.t.pointee(ty) == Pointee::Bytes && !null(&x) && !null(&y) => Expr::rt("byteSliceSame", vec![x, y]),
		_ => x.binary("===", y),
	}
}
