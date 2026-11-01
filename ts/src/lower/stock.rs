//! Well-known crates that translation maps itself, as it maps std, so a consumer binds
//! none of them: `bytes`' `Bytes`, which is the `rt.ByteSlice` a `&[u8]` is, and
//! `BytesMut`, which is `rt.BytesMut`, and `rand`'s thread-local generator, which draws
//! from the platform's cryptographic source. The rest of `bytes`, such as most of `Buf`'s
//! provided methods of the crate's own buffers, translates from its MIR. `tracing`'s
//! events are left out where they are written, by [`super::Translator::ignored_site`].

use rustc_hir::def::DefKind;
use rustc_hir::def_id::{CrateNum, DefId};
use rustc_middle::ty::{self, Instance, Ty, TyCtxt, TypeVisitableExt};

use super::body::Body;
use super::library::{self, Call, Std, key, string, targ};
use super::num::Int;
use crate::ts::{Expr, Stmt};

/// The type `did` is, if it is one of these crates' that has a representation of its own.
pub fn of(tcx: TyCtxt<'_>, did: DefId) -> Option<Std> {
	if !is_stock(tcx, did) {
		return None;
	}
	match (tcx.crate_name(did.krate).as_str(), tcx.opt_item_name(did)?.as_str()) {
		("bytes", "Bytes") => Some(Std::Bytes),
		("bytes", "BytesMut") => Some(Std::BytesMut),
		("rand", "ThreadRng") => Some(Std::ThreadRng),
		_ => None,
	}
}

/// Whether `did` is an item of one of these crates, rather than of the crate translated,
/// whatever its name.
pub fn is_stock(tcx: TyCtxt<'_>, did: DefId) -> bool {
	!did.is_local() && is_stock_crate(tcx, did.krate)
}

/// Whether `krate`, a dependency, is one of these crates, whose functions map, or translate
/// only where [`maps`] says they don't.
pub fn is_stock_crate(tcx: TyCtxt<'_>, krate: CrateNum) -> bool {
	matches!(tcx.crate_name(krate).as_str(), "bytes" | "rand")
}

/// Whether `krate` is one of these crates of which nothing translates: `rand`, whose
/// generators would have to be its own.
pub fn is_opaque_crate(tcx: TyCtxt<'_>, krate: CrateNum) -> bool {
	tcx.crate_name(krate).as_str() == "rand"
}

/// Whether `did`, an item of `bytes`, has a mapping in place of its MIR: `Bytes`,
/// `BytesMut`, and their functions and impls, and the crate's impls of std's byte slices
/// and vectors. The mapping of a function stands in for it or refuses it. The impls that
/// forward through a reference or a `Box`, and the crate's other types and functions,
/// translate.
pub fn mapped(tcx: TyCtxt<'_>, did: DefId) -> bool {
	if tcx.crate_name(did.krate).as_str() != "bytes" {
		return false;
	}
	match tcx
		.opt_parent(did)
		.filter(|&imp| matches!(tcx.def_kind(imp), DefKind::Impl { .. }))
	{
		Some(imp) => mapped_self(tcx, tcx.type_of(imp).instantiate_identity().skip_norm_wip()),
		None => of(tcx, did).is_some(),
	}
}

/// Whether `instance`, a provided method of `Buf` or `BufMut`, maps for its `Self`, which
/// [`mapped`] can't tell from the method alone: one the mapping names, of a buffer whose
/// impl maps, and `copy_to_bytes` of any other, whose `BytesMut::put` reads a capacity.
/// The others lower from their MIR, as for the crate's own buffers.
pub fn maps<'tcx>(tcx: TyCtxt<'tcx>, instance: Instance<'tcx>) -> bool {
	let did = instance.def_id();
	if tcx.crate_name(did.krate).as_str() != "bytes" || tcx.trait_of_assoc(did).is_none() {
		return false;
	}
	let name = tcx.item_name(did);
	match mapped_self(tcx, instance.args.type_at(0)) {
		true => BUF.contains(&name.as_str()) || BUF_MUT.contains(&name.as_str()),
		false => name.as_str() == "copy_to_bytes",
	}
}

/// `Buf`'s methods that the mapping names for its buffers.
const BUF: &[&str] = &[
	"remaining",
	"chunk",
	"advance",
	"has_remaining",
	"copy_to_slice",
	"copy_to_bytes",
	"get_u8",
	"get_i8",
	"get_u16",
	"get_u16_le",
	"get_u16_ne",
	"get_i16",
	"get_i16_le",
	"get_i16_ne",
	"get_u32",
	"get_u32_le",
	"get_u32_ne",
	"get_i32",
	"get_i32_le",
	"get_i32_ne",
	"get_u64",
	"get_u64_le",
	"get_u64_ne",
	"get_i64",
	"get_i64_le",
	"get_i64_ne",
	"get_u128",
	"get_u128_le",
	"get_u128_ne",
	"get_i128",
	"get_i128_le",
	"get_i128_ne",
	"get_f32",
	"get_f32_le",
	"get_f32_ne",
	"get_f64",
	"get_f64_le",
	"get_f64_ne",
	"get_uint",
	"get_uint_le",
	"get_uint_ne",
	"get_int",
	"get_int_le",
	"get_int_ne",
];

/// `BufMut`'s methods that the mapping names for its buffers.
const BUF_MUT: &[&str] = &[
	"remaining_mut",
	"has_remaining_mut",
	"put",
	"put_slice",
	"put_bytes",
	"put_u8",
	"put_i8",
	"put_u16",
	"put_u16_le",
	"put_u16_ne",
	"put_i16",
	"put_i16_le",
	"put_i16_ne",
	"put_u32",
	"put_u32_le",
	"put_u32_ne",
	"put_i32",
	"put_i32_le",
	"put_i32_ne",
	"put_u64",
	"put_u64_le",
	"put_u64_ne",
	"put_i64",
	"put_i64_le",
	"put_i64_ne",
	"put_u128",
	"put_u128_le",
	"put_u128_ne",
	"put_i128",
	"put_i128_le",
	"put_i128_ne",
	"put_f32",
	"put_f32_le",
	"put_f32_ne",
	"put_f64",
	"put_f64_le",
	"put_f64_ne",
	"put_uint",
	"put_uint_le",
	"put_uint_ne",
	"put_int",
	"put_int_le",
	"put_int_ne",
];

/// A buffer that the mapping of `Buf` or `BufMut` reads or writes.
#[derive(Clone, Copy, PartialEq)]
enum Buffer {
	/// A `Bytes`, whose `&mut` is an `rt.Place`.
	Bytes,
	BytesMut,
	/// A `&[u8]`, whose `&mut` is an `rt.Place` too.
	Slice,
	/// A `Vec<u8>`, which `BufMut` writes.
	Vec,
}

impl Buffer {
	fn of<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Self> {
		match ty.kind() {
			ty::Adt(adt, args) => match library::Std::of(tcx, adt.did())? {
				Std::Bytes => Some(Self::Bytes),
				Std::BytesMut => Some(Self::BytesMut),
				Std::Vec if is_u8(args.type_at(0)) => Some(Self::Vec),
				_ => None,
			},
			ty::Ref(_, inner, ty::Mutability::Not) if matches!(inner.kind(), ty::Slice(e) if is_u8(*e)) => {
				Some(Self::Slice)
			}
			_ => None,
		}
	}
}

/// Whether `ty`, the `Self` of a function of `bytes`, has a representation of its own or
/// is std's, so a mapping stands in for its impls: not a type parameter, through which an
/// impl forwards, nor the crate's own buffer or another crate's, which translate.
fn mapped_self<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
	match ty.kind() {
		ty::Adt(adt, _) => library::Std::of(tcx, adt.did()).is_some() && !ty.has_param(),
		ty::Ref(_, inner, _) => matches!(inner.kind(), ty::Slice(e) if is_u8(*e)),
		_ => false,
	}
}

/// A call of one of these crates' functions, or of a std trait's on one of their types:
/// its mapping, `None` if it has none, or its arguments back if it is no such call.
pub fn call<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	call: &Call<'tcx>,
	args: Vec<Expr>,
	out: &mut Vec<Stmt>,
) -> Result<Option<Expr>, Vec<Expr>> {
	let tcx = b.t.tcx;
	let Some((owner, name)) = key(tcx, call.def_id) else {
		return Err(args);
	};
	let self_std = call.args.types().next().and_then(|t| b.t.std_of(t.peel_refs()));
	if is_stock(tcx, call.def_id) {
		return Ok(match tcx.crate_name(call.def_id.krate).as_str() {
			"bytes" => bytes(b, call, &owner, &name, args, out),
			_ => rand(b, call, &owner, &name, args, out),
		});
	}
	let mut args = args.into_iter();
	let mut arg = || args.next().expect("an argument");
	Ok(match (owner.as_str(), name.as_str(), self_std) {
		// A clone of either shares what it holds, and a `Bytes` is the slice it derefs to.
		("Clone", "clone", Some(Std::Bytes | Std::ThreadRng))
		| ("Deref", "deref", Some(Std::Bytes))
		| ("AsRef", "as_ref", Some(Std::Bytes))
		| ("Borrow", "borrow", Some(Std::Bytes)) => Some(arg()),
		// A `BytesMut` lends a slice of its storage, and its clone copies it.
		("Deref", "deref", Some(Std::BytesMut))
		| ("DerefMut", "deref_mut", Some(Std::BytesMut))
		| ("AsRef", "as_ref", Some(Std::BytesMut))
		| ("AsMut", "as_mut", Some(Std::BytesMut))
		| ("Borrow", "borrow", Some(Std::BytesMut))
		| ("BorrowMut", "borrow_mut", Some(Std::BytesMut)) => Some(view(arg())),
		("Clone", "clone", Some(Std::BytesMut)) => Some(Expr::rt("bytesMutFrom", vec![view(arg())])),
		("Default", "default", Some(Std::Bytes)) => Some(Expr::rt("bytesNew", vec![])),
		("Default", "default", Some(Std::BytesMut)) => Some(Expr::rt("bytesMutWithCapacity", vec![Expr::lit("0")])),
		("Default", "default", Some(Std::ThreadRng)) => Some(Expr::rt("randRng", vec![])),
		("PartialEq", "eq", _) => {
			let (l, r) = (targ(call.args, 0).peel_refs(), targ(call.args, 1).peel_refs());
			let stock = |t: Ty<'tcx>| matches!(b.t.std_of(t), Some(Std::Bytes | Std::BytesMut));
			if !stock(l) && !stock(r) {
				return Err(args.collect());
			}
			let (a, c) = (arg(), arg());
			match (as_slice(b, l, a), as_slice(b, r, c)) {
				(Some(a), Some(c)) => Some(Expr::rt("byteSliceEq", vec![a, c])),
				_ => None,
			}
		}
		("From", "from", _) => return from(b, call, args.collect()),
		_ => return Err(args.collect()),
	})
}

/// The bytes of `value`, a `ty`, as an `rt.ByteSlice`, if it is a run of bytes.
fn as_slice<'tcx>(b: &Body<'_, '_, 'tcx>, ty: Ty<'tcx>, value: Expr) -> Option<Expr> {
	Some(match ty.kind() {
		ty::Slice(e) if is_u8(*e) => value,
		ty::Array(e, _) if is_u8(*e) => Expr::rt("byteSliceFrom", vec![value]),
		_ if string(b.t.tcx, ty) => Expr::rt("stringAsBytes", vec![value]),
		_ => match b.t.std_of(ty)? {
			Std::Bytes => value,
			Std::BytesMut => view(value),
			Std::Vec if targ_is_u8(ty) => Expr::rt("byteVecSlice", vec![value]),
			_ => return None,
		},
	})
}

/// The bytes of a `BytesMut`.
fn view(value: Expr) -> Expr {
	Expr::rt("bytesMutView", vec![value])
}

/// `From::from` into or out of a `Bytes` or a `BytesMut`.
fn from<'tcx>(b: &mut Body<'_, '_, 'tcx>, call: &Call<'tcx>, args: Vec<Expr>) -> Result<Option<Expr>, Vec<Expr>> {
	let tcx = b.t.tcx;
	let (to, from) = (targ(call.args, 0), targ(call.args, 1));
	let byte_vec = |t: Ty<'tcx>| b.t.std_of(t) == Some(Std::Vec) && targ_is_u8(t);
	let mut args = args.into_iter();
	let f = match (b.t.std_of(to), b.t.std_of(from)) {
		(Some(Std::Bytes), Some(Std::BytesMut)) => "bytesMutFreeze",
		(Some(Std::Bytes), _) if byte_vec(from) => "byteVecSlice",
		(Some(Std::Bytes), _) if string(tcx, from.peel_refs()) => "stringAsBytes",
		(Some(Std::Bytes), _) if is_bytes_ref(from) => return Ok(args.next()),
		(Some(Std::Bytes), _) => return Ok(None),
		(Some(Std::BytesMut), _) => {
			let bytes = match as_slice(b, from.peel_refs(), args.next().expect("an operand")) {
				Some(bytes) if from.is_ref() => Expr::rt("bytesMutFrom", vec![bytes]),
				Some(bytes) if b.t.std_of(from) == Some(Std::Bytes) => Expr::rt("bytesMutFromBytes", vec![bytes]),
				_ => return Ok(None),
			};
			return Ok(Some(bytes));
		}
		(_, Some(Std::Bytes)) if byte_vec(to) => "byteSliceToByteVec",
		(_, Some(Std::BytesMut)) if byte_vec(to) => "bytesMutToByteVec",
		(_, Some(Std::Bytes | Std::BytesMut)) => return Ok(None),
		_ => return Err(args.collect()),
	};
	Ok(Some(Expr::rt(f, args.collect())))
}

/// A function of `bytes`: `Bytes`'s or `BytesMut`'s own, or a method of `Buf` or `BufMut`
/// on a buffer the mapping reads or writes.
fn bytes<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	call: &Call<'tcx>,
	owner: &str,
	name: &str,
	args: Vec<Expr>,
	out: &mut Vec<Stmt>,
) -> Option<Expr> {
	let tcx = b.t.tcx;
	match super::library::inherent_std(tcx, call.def_id) {
		Some(Std::Bytes) => return bytes_own(b, call, name, args, out),
		Some(Std::BytesMut) => return bytes_mut(b, call, name, args, out),
		_ => {}
	}
	let Some(buffer) = Buffer::of(tcx, targ(call.args, 0)) else {
		return match (owner, name) {
			("Buf", "copy_to_bytes") => provided_copy_to_bytes(b, call, args),
			_ => None,
		};
	};
	match owner {
		"Buf" => buf(b, call, buffer, name, args),
		"BufMut" => buf_mut(b, call, buffer, name, args),
		_ => None,
	}
}

/// A function of `Bytes`: a `&mut self` one takes its `rt.Place`.
fn bytes_own<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	call: &Call<'tcx>,
	name: &str,
	args: Vec<Expr>,
	out: &mut Vec<Stmt>,
) -> Option<Expr> {
	let tcx = b.t.tcx;
	let mut args = args.into_iter();
	Some(match name {
		"new" => Expr::rt("bytesNew", vec![]),
		// A `&'static [u8]` never changes, so the slice is the `Bytes`.
		"from_static" => args.next()?,
		"copy_from_slice" => Expr::rt("bytesCopyFromSlice", vec![args.next()?]),
		"len" => args.next()?.member("length"),
		"is_empty" => args.next()?.member("length").binary("===", Expr::lit("0")),
		// The other ranges' bounds are read where the range is made, by the body's lowering.
		"slice" if b.t.std_of(targ(call.args, 0)) == Some(Std::RangeInclusive) => {
			Expr::rt("bytesSliceInclusive", vec![args.next()?, args.next()?])
		}
		"slice_ref" => Expr::rt("bytesSliceRef", vec![args.next()?, args.next()?]),
		"split_to" => Expr::rt("bytesSplitTo", vec![args.next()?, args.next()?]),
		"split_off" => Expr::rt("bytesSplitOff", vec![args.next()?, args.next()?]),
		"truncate" => Expr::rt("bytesTruncate", vec![args.next()?, args.next()?]),
		"clear" => Expr::rt("bytesTruncate", vec![args.next()?, Expr::lit("0")]),
		// The owner is read once, as `bytes` reads it, and would drop with the last `Bytes`
		// that shares it, which no value here tracks.
		"from_owner" => {
			let owner = targ(call.args, 0);
			if b.t.drops(owner) {
				let msg = format!(
					"`Bytes::from_owner` of `{owner}`, which would drop with the last clone of what it makes, which is not supported"
				);
				return Some(b.err(call.span, msg));
			}
			let as_ref = trait_fn(tcx, rustc_span::sym::AsRef)?;
			let slice = Ty::new_slice(tcx, tcx.types.u8);
			b.call_def(
				as_ref,
				tcx.mk_args(&[owner.into(), slice.into()]),
				vec![args.next()?],
				call.span,
				out,
			)
		}
		_ => return None,
	})
}

/// A function of `BytesMut`. What reads its capacity is refused: `bytes`' depends on
/// whether the other handles of its storage dropped, such as a `Bytes` it froze, which
/// none here counts.
fn bytes_mut(
	b: &mut Body<'_, '_, '_>,
	call: &Call<'_>,
	name: &str,
	args: Vec<Expr>,
	out: &mut Vec<Stmt>,
) -> Option<Expr> {
	if matches!(
		name,
		"capacity" | "split_off" | "try_reclaim" | "try_unsplit" | "spare_capacity_mut"
	) {
		let msg = format!(
			"`BytesMut::{name}`, which reads a capacity that depends on whether the other handles of its storage dropped, which is not supported"
		);
		return Some(b.err(call.span, msg));
	}
	let mut args = args.into_iter();
	let mut arg = || args.next();
	let f = match name {
		"new" => return Some(Expr::rt("bytesMutWithCapacity", vec![Expr::lit("0")])),
		"with_capacity" => "bytesMutWithCapacity",
		"zeroed" => "bytesMutZeroed",
		"len" => return Some(arg()?.member("length")),
		"is_empty" => return Some(arg()?.member("length").binary("===", Expr::lit("0"))),
		"freeze" => "bytesMutFreeze",
		"split_to" => "bytesMutSplitTo",
		"split" => {
			let this = b.spill(arg()?, "buf", out);
			return Some(Expr::rt("bytesMutSplitTo", vec![this.clone(), this.member("length")]));
		}
		"truncate" => "bytesMutTruncate",
		"clear" => return Some(Expr::rt("bytesMutTruncate", vec![arg()?, Expr::lit("0")])),
		"resize" => "bytesMutResize",
		"reserve" => "bytesMutReserve",
		"extend_from_slice" => "bytesMutExtend",
		"unsplit" => "bytesMutUnsplit",
		_ => return None,
	};
	Some(Expr::rt(f, args.collect()))
}

/// A method of `Buf` on a buffer the mapping reads.
fn buf<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	call: &Call<'tcx>,
	buffer: Buffer,
	name: &str,
	args: Vec<Expr>,
) -> Option<Expr> {
	let mut args = args.into_iter();
	let this = args.next()?;
	// What reads past the front: a `Bytes`'s or a `&[u8]`'s place, which `bytes` reads
	// alike but to advance and copy out, or a `BytesMut`.
	let prefix = match buffer {
		Buffer::Bytes | Buffer::Slice => Some("bytes"),
		Buffer::BytesMut => Some("bytesMut"),
		Buffer::Vec => None,
	};
	let rt = |f: &str, args: Vec<Expr>| Some(Expr::rt(format!("{}{f}", prefix?), args));
	Some(match name {
		"remaining" => this.member("length"),
		"has_remaining" => this.member("length").binary("!==", Expr::lit("0")),
		"chunk" if buffer == Buffer::BytesMut => view(this),
		"chunk" => this,
		"advance" if buffer == Buffer::Slice => Expr::rt("byteSliceAdvance", vec![this, args.next()?]),
		"advance" => rt("Advance", vec![this, args.next()?])?,
		"copy_to_slice" => rt("CopyToSlice", vec![this, args.next()?])?,
		"copy_to_bytes" if buffer == Buffer::BytesMut => Expr::rt(
			"bytesMutFreeze",
			vec![Expr::rt("bytesMutSplitTo", vec![this, args.next()?])],
		),
		"copy_to_bytes" if buffer == Buffer::Slice => Expr::rt("byteSliceCopyToBytes", vec![this, args.next()?]),
		"copy_to_bytes" => rt("SplitTo", vec![this, args.next()?])?,
		_ => {
			let (ty, le) = scalar(name.strip_prefix("get_")?);
			let out = b.t.sig(call.def_id, call.args).output();
			let le = Expr::lit(le.to_string());
			match ty {
				"uint" | "int" => {
					let int = Int::of(out)?;
					let (size, signed) = (Expr::lit("8"), Expr::lit(int.signed.to_string()));
					let read = rt("ReadInt", vec![this, size, args.next()?, signed, le])?;
					int.method("fromBeBytes", vec![read])
				}
				"f32" => Expr::rt("f32FromBeBytes", vec![rt("Read", vec![this, Expr::lit("4"), le])?]),
				"f64" => Expr::rt("f64FromBeBytes", vec![rt("Read", vec![this, Expr::lit("8"), le])?]),
				_ => {
					let int = Int::of(out)?;
					let n = Expr::lit((int.bits / 8).to_string());
					if int.big() {
						int.method("fromBeBytes", vec![rt("Read", vec![this, n, le])?])
					} else {
						let word = rt("ReadWord", vec![this, n, le])?;
						let word = if int.signed { int.wrap(word) } else { word };
						Expr::As(Box::new(word), int.ty())
					}
				}
			}
		}
	})
}

/// A method of `BufMut` on a buffer the mapping writes.
fn buf_mut<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	call: &Call<'tcx>,
	buffer: Buffer,
	name: &str,
	args: Vec<Expr>,
) -> Option<Expr> {
	let mut args = args.into_iter();
	let this = args.next()?;
	let prefix = match buffer {
		Buffer::BytesMut => "bytesMut",
		Buffer::Vec => "byteVec",
		Buffer::Bytes | Buffer::Slice => return None,
	};
	let rt = |f: &str, args: Vec<Expr>| Expr::rt(format!("{prefix}{f}"), args);
	// A vector's or a `BytesMut`'s length is at most `isize::MAX`.
	let isize_max = Expr::lit("0x7fffffff");
	Some(match name {
		"remaining_mut" => Expr::As(
			Box::new(isize_max.binary("-", this.member("length"))),
			Int::of(b.t.tcx.types.usize)?.ty(),
		),
		"has_remaining_mut" => this.member("length").binary("<", isize_max),
		"put_slice" if buffer == Buffer::BytesMut => Expr::rt("bytesMutExtend", vec![this, args.next()?]),
		"put_slice" => Expr::rt("byteVecExtendFromSlice", vec![this, args.next()?]),
		"put" => put(b, call, buffer, this, args.next()?)?,
		"put_bytes" => rt("PutFill", vec![this, args.next()?, args.next()?]),
		_ => {
			let (ty, le) = scalar(name.strip_prefix("put_")?);
			let input = *b.t.sig(call.def_id, call.args).inputs().get(1)?;
			let value = args.next()?;
			let le = Expr::lit(le.to_string());
			match ty {
				"uint" | "int" => {
					let be = Int::of(input)?.method("toBeBytes", vec![value]);
					rt("PutInt", vec![this, be, args.next()?, le])
				}
				"f32" => rt("PutBytes", vec![this, Expr::rt("f32ToBeBytes", vec![value]), le]),
				"f64" => rt("PutBytes", vec![this, Expr::rt("f64ToBeBytes", vec![value]), le]),
				_ => {
					let int = Int::of(input)?;
					match int.big() {
						true => rt("PutBytes", vec![this, int.method("toBeBytes", vec![value]), le]),
						false => rt("PutWord", vec![this, value, Expr::lit((int.bits / 8).to_string()), le]),
					}
				}
			}
		}
	})
}

/// `BufMut::put` of a `Buf` into a buffer the mapping writes: all of it, read whole from
/// a buffer the mapping reads, and otherwise through its own methods, as `bytes` reads it
/// into a `Vec<u8>`.
fn put<'tcx>(b: &mut Body<'_, '_, 'tcx>, call: &Call<'tcx>, buffer: Buffer, this: Expr, src: Expr) -> Option<Expr> {
	let tcx = b.t.tcx;
	let ty = targ(call.args, 1);
	let source = Buffer::of(tcx, ty);
	let bytes = match source {
		Some(Buffer::BytesMut) => Some(view(src.clone())),
		Some(Buffer::Bytes | Buffer::Slice) => Some(src.clone()),
		_ => None,
	};
	if let Some(bytes) = bytes {
		let f = match (buffer, source) {
			(Buffer::BytesMut, Some(Buffer::Slice)) => "bytesMutExtend",
			(Buffer::BytesMut, _) => "bytesMutPutStorage",
			_ => "byteVecExtendFromSlice",
		};
		return Some(Expr::rt(f, vec![this, bytes]));
	}
	// `put` drops it between calls of its methods, which only an object JS can call them on
	// can stand for.
	if b.t.drops(ty) || b.t.needs_place(ty) {
		let msg = format!("`BufMut::put` of `{ty}`, which is not supported");
		return Some(b.err(call.span, msg));
	}
	// Into a `BytesMut`, `bytes` calls its `copy_to_bytes` or its `chunk` and `advance` by
	// whether the capacity is zero.
	if buffer == Buffer::BytesMut {
		let msg = format!(
			"`BufMut::put` of `{ty}` into a `BytesMut`, which picks the methods it calls by a capacity that depends on whether the other handles of its storage dropped, which is not supported"
		);
		return Some(b.err(call.span, msg));
	}
	let (has, remaining, chunk, advance) = (
		buf_method(b, call, ty, "has_remaining")?,
		buf_method(b, call, ty, "remaining")?,
		buf_method(b, call, ty, "chunk")?,
		buf_method(b, call, ty, "advance")?,
	);
	Some(Expr::rt(
		"byteVecPutBuf",
		vec![this, src, has, remaining, chunk, advance],
	))
}

/// `Buf::copy_to_bytes` as `bytes` provides it, of a buffer the mapping doesn't read: it
/// puts a `Take` of the buffer into a new `BytesMut` of the capacity asked for, which
/// copies chunk by chunk through the buffer's own methods, as the runtime does.
fn provided_copy_to_bytes<'tcx>(b: &mut Body<'_, '_, 'tcx>, call: &Call<'tcx>, args: Vec<Expr>) -> Option<Expr> {
	let ty = targ(call.args, 0);
	if b.t.needs_place(ty) {
		let msg = format!("`Buf::copy_to_bytes` of `{ty}`, a value JS copies, which is not supported");
		return Some(b.err(call.span, msg));
	}
	let mut args = args.into_iter();
	let (this, len) = (args.next()?, args.next()?);
	let (remaining, chunk, advance) = (
		buf_method(b, call, ty, "remaining")?,
		buf_method(b, call, ty, "chunk")?,
		buf_method(b, call, ty, "advance")?,
	);
	Some(Expr::rt("bufCopyToBytes", vec![this, len, remaining, chunk, advance]))
}

/// The function value of `Buf`'s method `name` for the buffer `ty`, of the crate `call` is to.
fn buf_method<'tcx>(b: &mut Body<'_, '_, 'tcx>, call: &Call<'tcx>, ty: Ty<'tcx>, name: &str) -> Option<Expr> {
	let tcx = b.t.tcx;
	let buf = tcx
		.traits(call.def_id.krate)
		.iter()
		.copied()
		.find(|&tr| tcx.item_name(tr).as_str() == "Buf")?;
	let f = tcx
		.associated_items(buf)
		.in_definition_order()
		.find(|i| i.is_fn() && i.name().as_str() == name)?
		.def_id;
	Some(b.fn_value(f, tcx.mk_args(&[ty.into()]), call.span))
}

/// A `get_` or `put_` method's scalar, and whether its bytes are little-endian, as
/// wasm32's native order is.
fn scalar(name: &str) -> (&str, bool) {
	match name.strip_suffix("_le").or_else(|| name.strip_suffix("_ne")) {
		Some(ty) => (ty, true),
		None => (name, false),
	}
}

/// A function of `rand`: `rng`, or a method of its `Rng` or `RngExt` on a `ThreadRng`.
fn rand<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	call: &Call<'tcx>,
	owner: &str,
	name: &str,
	args: Vec<Expr>,
	out: &mut Vec<Stmt>,
) -> Option<Expr> {
	let tcx = b.t.tcx;
	if tcx.def_kind(call.def_id) == DefKind::Fn && matches!(name, "rng" | "thread_rng") {
		return Some(Expr::rt("randRng", vec![]));
	}
	if !matches!(owner, "Rng" | "RngExt") || b.t.std_of(targ(call.args, 0).peel_refs()) != Some(Std::ThreadRng) {
		return None;
	}
	// The generator holds no state, so only the arguments after it are read.
	let mut args = args.into_iter().skip(1);
	let ty = targ(call.args, 1);
	let width = |int: Int| (Expr::lit(int.bits.to_string()), Expr::lit(int.signed.to_string()));
	Some(match name {
		"random" if ty.is_bool() => Expr::rt("randBool", vec![]),
		"random" => {
			let int = Int::of(ty)?;
			let (bits, signed) = width(int);
			Expr::As(Box::new(Expr::rt("randInt", vec![bits, signed])), int.ty())
		}
		"random_range" => {
			let int = Int::of(ty)?;
			let (bits, signed) = width(int);
			let range = args.next()?;
			let value = match b.t.std_of(targ(call.args, 2))? {
				Std::Range => {
					let range = b.spill(range, "range", out);
					let (start, end) = (range.clone().member("start"), range.member("end"));
					Expr::rt("randRange", vec![start, end, bits, signed])
				}
				Std::RangeTo => Expr::rt("randRange", vec![int.lit(0), range.member("end"), bits, signed]),
				Std::RangeInclusive => Expr::rt("randRangeInclusive", vec![range, bits, signed]),
				// `rand` samples `..=b` only of unsigned integers.
				Std::RangeToInclusive => Expr::rt("randRangeToInclusive", vec![range.member("end"), bits]),
				_ => return None,
			};
			Expr::As(Box::new(value), int.ty())
		}
		_ => return None,
	})
}

/// The one function of the trait `item` names, such as `AsRef::as_ref`.
fn trait_fn(tcx: TyCtxt<'_>, item: rustc_span::Symbol) -> Option<DefId> {
	let tr = tcx.get_diagnostic_item(item)?;
	tcx.associated_items(tr)
		.in_definition_order()
		.find(|i| i.is_fn())
		.map(|i| i.def_id)
}

fn is_u8(ty: Ty<'_>) -> bool {
	matches!(ty.kind(), ty::Uint(ty::UintTy::U8))
}

fn targ_is_u8(ty: Ty<'_>) -> bool {
	matches!(ty.kind(), ty::Adt(_, args) if args.types().next().is_some_and(|t| matches!(t.kind(), ty::Uint(ty::UintTy::U8))))
}

/// Whether `ty` is `&[u8]`.
fn is_bytes_ref(ty: Ty<'_>) -> bool {
	matches!(ty.kind(), ty::Ref(_, inner, _) if matches!(inner.kind(), ty::Slice(e) if matches!(e.kind(), ty::Uint(ty::UintTy::U8))))
}
