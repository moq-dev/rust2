//! What stands in for std where its MIR reaches what Go does itself: a `Vec`'s storage,
//! raw pointers into a slice, UTF-8 validation, formatting's type-erased arguments, and
//! the intrinsics. Everything else of std translates from its own MIR, so each binding
//! is a cut as low as Go allows.

use rustc_hir::def::DefKind;
use rustc_hir::def_id::DefId;
use rustc_middle::mir::Operand;
use rustc_middle::ty::{self, Instance, InstanceKind, Ty, TyCtxt, TypingEnv};
use rustc_span::Span;

use super::body::{Fx, int_bits};

/// Names the runtime declares at the package level.
pub const RUNTIME_NAMES: &[&str] = &[
	"tuple1", "tuple2", "tuple3", "tuple4", "tuple5", "tuple6", "tuple7", "tuple8", "Option", "Result", "Ordering",
	"fmtError", "RustPanic", "rustPanic", "panicBounds", "signed", "unsigned", "integer", "addU", "subU", "mulU",
	"addS", "subS", "mulS", "satAddU", "satSubU", "satAddS", "satSubS", "cmp3", "boolTo", "replace", "ptrTo",
	"sliceIter", "sliceIterNext", "vecPop", "vecTruncate", "sliceCopyFrom", "sliceCopyWithin", "sliceIndexFail",
	"utf8Check", "be16", "be32", "be64", "le16", "le32", "le64", "fromBe16", "fromBe32", "fromBe64", "fromLe16",
	"fromLe32", "fromLe64", "rotl", "rotr", "clz", "ctz", "popcount", "bswap", "Formatter", "Arguments", "Argument",
	"argumentOf", "argumentCount", "argumentsStr", "fmtDyn", "DebugBuilder", "fmtUint", "fmtSint", "fmtUintDebug",
	"fmtSintDebug", "fmtHex", "formatBytes", "displayBytes", "rustPanicFmt", "sink", "byteSink", "padAdapter",
	"fmtOk", "fmtErr", "fmtResult", "newFormatter",
];

/// A library type Go holds as a type of its own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mapped {
	/// `Vec<T>`: a Go slice, whose `append` grows it as std's does, by doubling.
	Vec,
	/// `slice::Iter<T>`: the slice left.
	SliceIter,
	/// `Box<T>`: a pointer, or what a reference to its unsized `T` is.
	Box,
	/// `MaybeUninit<T>` and `ManuallyDrop<T>`: the `T`, which Go zeroes until written.
	Transparent,
	/// `Option<T>` and `Result<T, E>`: the runtime's declarations, laid out as any enum's.
	Option,
	Result,
	/// A type the runtime declares, by its Go name. `Ordering` and `fmt::Error` are laid
	/// out as any enum and struct are.
	Runtime(&'static str),
	/// A type translation refuses, and why.
	Refused(&'static str),
}

impl Mapped {
	/// Whether its Go type is laid out as the translation lays out any ADT, so its fields
	/// and variants are reached as theirs are.
	pub fn is_runtime_layout(self) -> bool {
		matches!(self, Mapped::Option | Mapped::Result | Mapped::Runtime("Ordering" | "fmtError"))
	}
}

/// An item's path, with its crate: `alloc::vec::Vec`.
pub fn path(tcx: TyCtxt<'_>, did: DefId) -> String {
	format!("{}{}", tcx.crate_name(did.krate), tcx.def_path(did).to_string_no_crate_verbose())
}

/// The Go type a library ADT stands for, if it is one.
pub fn mapped(tcx: TyCtxt<'_>, did: DefId) -> Option<Mapped> {
	if did.is_local() {
		return None;
	}
	Some(match path(tcx, did).as_str() {
		"alloc::vec::Vec" => Mapped::Vec,
		"core::slice::iter::Iter" => Mapped::SliceIter,
		"core::option::Option" => Mapped::Option,
		"core::result::Result" => Mapped::Result,
		"core::cmp::Ordering" => Mapped::Runtime("Ordering"),
		"core::fmt::Error" => Mapped::Runtime("fmtError"),
		"core::fmt::Formatter" => Mapped::Runtime("Formatter"),
		"core::fmt::Arguments" => Mapped::Runtime("Arguments"),
		"core::fmt::rt::Argument" => Mapped::Runtime("Argument"),
		"core::fmt::builders::DebugStruct" | "core::fmt::builders::DebugTuple" | "core::fmt::builders::DebugList" => {
			Mapped::Runtime("DebugBuilder")
		}
		"core::marker::PhantomData" => Mapped::Runtime("struct{}"),
		"alloc::boxed::Box" | "alloc::boxed::BoxRaw" => Mapped::Box,
		"core::mem::maybe_uninit::MaybeUninit" | "core::mem::manually_drop::ManuallyDrop" => Mapped::Transparent,
		"alloc::raw_vec::RawVec" | "core::ptr::non_null::NonNull" | "core::ptr::unique::Unique" => {
			Mapped::Refused("a raw pointer's wrapper")
		}
		_ => return None,
	})
}

/// Whether `did` is `Display` or `Debug`, whose trait objects the runtime's `fmtDyn` is.
pub fn fmt_trait(tcx: TyCtxt<'_>, did: DefId) -> bool {
	matches!(path(tcx, did).as_str(), "core::fmt::Display" | "core::fmt::Debug")
}

/// Why dropping a value of `ty` is refused, if it runs code: a `Drop` impl translation does
/// not run. Freeing memory is Go's collector's.
pub fn drop_refused<'tcx>(tcx: TyCtxt<'tcx>, env: TypingEnv<'tcx>, ty: Ty<'tcx>) -> Option<String> {
	if !ty.needs_drop(tcx, env) {
		return None;
	}
	match ty.kind() {
		ty::Adt(adt, args) => {
			if matches!(mapped(tcx, adt.did()), Some(Mapped::Vec | Mapped::Box)) {
				return drop_refused(tcx, env, args.type_at(0));
			}
			if adt.has_dtor(tcx) {
				return Some(format!("whose `Drop` impl translation does not run yet ({})", path(tcx, adt.did())));
			}
			for field in adt.all_fields() {
				let fty = field.ty(tcx, args).skip_norm_wip();
				let fty = tcx.normalize_erasing_regions(env, ty::Unnormalized::new_wip(fty));
				if let Some(why) = drop_refused(tcx, env, fty) {
					return Some(why);
				}
			}
			None
		}
		ty::Tuple(tys) => tys.iter().find_map(|t| drop_refused(tcx, env, t)),
		ty::Array(elem, _) | ty::Slice(elem) => drop_refused(tcx, env, *elem),
		ty::Closure(_, args) => args.as_closure().upvar_tys().iter().find_map(|t| drop_refused(tcx, env, t)),
		_ => Some("which translation can't drop".into()),
	}
}

/// What a bound call is in Go: an expression assigned to the call's destination, or a
/// statement.
pub enum Bound {
	Expr(String),
	Stmt(String),
}

/// The key a call is bound by: a free function's path, or `<Self>::name` for an inherent
/// method, or `<Self as Trait>::name` for a trait's, with `Self` by its head, `[T]` for a
/// slice and `T` for a parameter.
fn key(tcx: TyCtxt<'_>, did: DefId) -> String {
	let parent = tcx.parent(did);
	let name = tcx.item_name(did);
	match tcx.def_kind(parent) {
		DefKind::Impl { of_trait } => {
			let self_ty = tcx.type_of(parent).instantiate_identity().skip_norm_wip();
			let head = head(tcx, self_ty);
			match of_trait {
				true => format!("<{head} as {}>::{name}", path(tcx, tcx.impl_trait_id(parent))),
				false => format!("<{head}>::{name}"),
			}
		}
		_ => path(tcx, did),
	}
}

fn head<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> String {
	match ty.kind() {
		ty::Adt(adt, _) => path(tcx, adt.did()),
		ty::Slice(_) => "[T]".into(),
		ty::Array(..) => "[T; N]".into(),
		ty::Str => "str".into(),
		ty::Bool => "bool".into(),
		ty::Char => "char".into(),
		ty::Int(i) => i.name_str().into(),
		ty::Uint(u) => u.name_str().into(),
		ty::Ref(_, inner, _) => format!("&{}", head(tcx, *inner)),
		ty::Param(_) => "T".into(),
		_ => "?".into(),
	}
}

/// The integer types, by their Rust names.
const INTS: &[&str] = &[
	"u8", "u16", "u32", "u64", "usize", "i8", "i16", "i32", "i64", "isize",
];

/// The Go for a call of `instance` that a binding stands in for, if one does.
pub fn call<'tcx>(
	fx: &mut Fx<'_, 'tcx>,
	instance: Instance<'tcx>,
	args: &[&Operand<'tcx>],
	dest_ty: Ty<'tcx>,
	span: Span,
) -> Option<Bound> {
	let a: Vec<String> = args.iter().map(|op| fx.operand(op, span)).collect();
	let tys: Vec<Ty<'tcx>> = args.iter().map(|op| op.ty(fx.mir_decls(), fx.tcx)).collect();
	call_with(fx, instance, a, &tys, dest_ty, span)
}

/// [`call`] of arguments already in Go, of types `tys`.
pub fn call_with<'tcx>(
	fx: &mut Fx<'_, 'tcx>,
	instance: Instance<'tcx>,
	a: Vec<String>,
	tys: &[Ty<'tcx>],
	dest_ty: Ty<'tcx>,
	span: Span,
) -> Option<Bound> {
	let tcx = fx.tcx;
	if let InstanceKind::Intrinsic(def_id) = instance.def {
		return intrinsic(fx, def_id, instance, a, tys, dest_ty, span);
	}
	// A call through `&dyn Display` or `&dyn Debug`, which is the `fmt` it holds.
	if let InstanceKind::Virtual(def_id, _) = instance.def
		&& fmt_trait(tcx, tcx.parent(def_id))
	{
		return Some(Bound::Expr(format!("{}({})", a[0], a[1])));
	}
	if !matches!(instance.def, InstanceKind::Item(_)) {
		return None;
	}
	let did = instance.def_id();
	if did.is_local() {
		return None;
	}
	let key = key(tcx, did);
	let targ = |i: usize| instance.args.types().nth(i).expect("a binding's type argument");
	let dest = |fx: &mut Fx<'_, 'tcx>| fx.ty(dest_ty, span);
	let expr = Bound::Expr;
	let stmt = Bound::Stmt;
	let bound = match key.as_str() {
		"core::mem::replace" => expr(format!("replace({}, {})", a[0], a[1])),
		"core::mem::swap" => stmt(format!("*{0}, *{1} = *{1}, *{0}", a[0], a[1])),

		"<alloc::boxed::Box>::new_uninit" => expr(format!("new({})", fx.ty(targ(0), span))),
		"<alloc::boxed::Box>::new" => expr(format!("ptrTo({})", a[0])),
		"alloc::boxed::box_assume_init_into_vec_unsafe" => expr(format!("(*{})[:]", a[0])),
		"<alloc::vec::Vec>::new" => expr(format!("{}(nil)", dest(fx))),
		"<alloc::vec::Vec>::with_capacity" => expr(format!("make({}, 0, {})", dest(fx), a[0])),
		"<alloc::vec::Vec>::push" => stmt(format!("*{0} = append(*{0}, {1})", a[0], a[1])),
		"<alloc::vec::Vec>::pop" => expr(format!("vecPop({})", a[0])),
		"<alloc::vec::Vec>::len" => expr(format!("uint(len(*{}))", a[0])),
		"<alloc::vec::Vec>::capacity" => expr(format!("uint(cap(*{}))", a[0])),
		"<alloc::vec::Vec>::is_empty" => expr(format!("len(*{}) == 0", a[0])),
		"<alloc::vec::Vec>::truncate" => {
			fx_drop_check(fx, targ(0), span);
			stmt(format!("vecTruncate({}, {})", a[0], a[1]))
		}
		"<alloc::vec::Vec>::clear" => {
			fx_drop_check(fx, targ(0), span);
			stmt(format!("*{0} = (*{0})[:0]", a[0]))
		}
		"<alloc::vec::Vec>::extend_from_slice" => {
			copy_check(fx, targ(0), span);
			stmt(format!("*{0} = append(*{0}, {1}...)", a[0], a[1]))
		}
		"<alloc::vec::Vec>::as_slice"
		| "<alloc::vec::Vec>::as_mut_slice"
		| "<alloc::vec::Vec as core::ops::deref::Deref>::deref"
		| "<alloc::vec::Vec as core::ops::deref::DerefMut>::deref_mut" => expr(format!("*{}", a[0])),
		"<alloc::vec::Vec as core::clone::Clone>::clone" => {
			copy_check(fx, targ(0), span);
			expr(format!("append({}(nil), (*{})...)", dest(fx), a[0]))
		}
		"<alloc::vec::Vec as core::default::Default>::default" => expr(format!("{}(nil)", dest(fx))),

		"<[T]>::split_first_chunk" | "<[T]>::split_first_chunk_mut" => {
			let n = instance.args.consts().next().and_then(|c| c.try_to_target_usize(tcx))?;
			let elem = fx.ty(targ(0), span);
			let d = dest(fx);
			expr(format!(
				"func() {d} {{ if len({0}) < {n} {{ return {d}{{}} }}; return {d}{{tag: 1, Some: struct{{ F0 tuple2[*[{n}]{elem}, []{elem}] }}{{tuple2[*[{n}]{elem}, []{elem}]{{(*[{n}]{elem})({0}[:{n}]), {0}[{n}:]}}}}}} }}()",
				a[0]
			))
		}
		"<core::fmt::Formatter>::debug_lower_hex" => expr(format!("{}.flags&flagDebugLowerHex != 0", a[0])),
		"<core::fmt::Formatter>::debug_upper_hex" => expr(format!("{}.flags&flagDebugUpperHex != 0", a[0])),
		"<core::fmt::builders::DebugList>::entries" => {
			// Entries from a slice's iterator, each a reference to an element.
			let iter = targ(1);
			let ty::Adt(adt, iargs) = iter.kind() else { return None };
			if mapped(tcx, adt.did()) != Some(Mapped::SliceIter) {
				fx.err(span, format!("`DebugList::entries` of `{iter}`"));
				return None;
			}
			let elem = iargs.types().next()?;
			let item = Ty::new_imm_ref(tcx, tcx.lifetimes.re_erased, elem);
			let call = fmt_call(fx, "Debug", item, "&x", span)?;
			expr(format!(
				"func() *DebugBuilder {{ for i := range {1}.s {{ x := &{1}.s[i]; {0}.entry(fmtDyn({call})) }}; return {0} }}()",
				a[0], a[1]
			))
		}
		"<[T]>::iter" => expr(format!("{}{{s: {}}}", dest(fx), a[0])),
		"<core::slice::iter::Iter as core::iter::traits::iterator::Iterator>::next" => {
			expr(format!("sliceIterNext({})", a[0]))
		}
		"<[T]>::to_vec" => {
			copy_check(fx, targ(0), span);
			expr(format!("append({}(nil), {}...)", dest(fx), a[0]))
		}
		"<[T]>::split_at_unchecked" | "<[T]>::split_at_mut_unchecked" => {
			let d = dest(fx);
			expr(format!("{d}{{{0}[:{1}], {0}[{1}:]}}", a[0], a[1]))
		}
		"core::slice::index::get_offset_len_noubcheck" | "core::slice::index::get_offset_len_mut_noubcheck" => {
			expr(format!("{0}[{1}:{1}+{2}]", a[0], a[1], a[2]))
		}
		"core::slice::copy_from_slice_impl" => stmt(format!("sliceCopyFrom({}, {})", a[0], a[1])),
		"<[T]>::copy_within" => {
			let range = instance.args.type_at(1);
			match range.kind() {
				ty::Adt(adt, _) if super::library::path(tcx, adt.did()) == "core::ops::range::Range" => {
					stmt(format!("sliceCopyWithin({0}, {1}.start, {1}.end, {2})", a[0], a[1], a[2]))
				}
				_ => {
					fx.err(span, format!("`copy_within` of a `{range}`"));
					return None;
				}
			}
		}

		"core::str::converts::from_utf8" => {
			let r = dest(fx);
			let ty::Adt(_, rargs) = dest_ty.kind() else { return None };
			let e = fx.ty(rargs.type_at(1), span);
			expr(format!(
				"func() {r} {{ if n, l, ok := utf8Check({0}); !ok {{ return {r}{{tag: 1, Err: struct{{ F0 {e} }}{{{e}{{valid_up_to: n, error_len: l}}}}}} }}; return {r}{{tag: 0, Ok: struct{{ F0 []uint8 }}{{{0}}}}} }}()",
				a[0]
			))
		}

		"<core::fmt::Arguments>::new" => expr(format!("Arguments{{template: {}[:], args: {}[:]}}", a[0], a[1])),
		"<core::fmt::Arguments>::from_str" | "<core::fmt::Arguments>::from_str_nonconst" => {
			expr(format!("argumentsStr({})", a[0]))
		}
		"<core::fmt::rt::Argument>::from_usize" => expr(format!("argumentCount(*{})", a[0])),
		"<core::fmt::rt::Argument>::new_display"
		| "<core::fmt::rt::Argument>::new_debug"
		| "<core::fmt::rt::Argument>::new_lower_hex"
		| "<core::fmt::rt::Argument>::new_upper_hex"
		| "<core::fmt::rt::Argument>::new_octal"
		| "<core::fmt::rt::Argument>::new_binary" => {
			let trait_name = match tcx.item_name(did).as_str() {
				"new_display" => "Display",
				"new_debug" => "Debug",
				"new_lower_hex" => "LowerHex",
				"new_upper_hex" => "UpperHex",
				"new_octal" => "Octal",
				_ => "Binary",
			};
			let call = fmt_call(fx, trait_name, targ(0), &a[0], span)?;
			expr(format!("argumentOf({call})"))
		}
		"<core::fmt::Formatter>::write_str" => expr(format!("{}.writeStr({})", a[0], a[1])),
		"<core::fmt::Formatter>::write_fmt" => expr(format!("{}.writeFmt({})", a[0], a[1])),
		"<core::fmt::Formatter>::write_char" => expr(format!("{}.writeChar({})", a[0], a[1])),
		"<core::fmt::Formatter>::pad" => expr(format!("{}.pad({})", a[0], a[1])),
		"<core::fmt::Formatter>::pad_integral" => {
			expr(format!("{}.padIntegral({}, {}, {})", a[0], a[1], a[2], a[3]))
		}
		"<core::fmt::Formatter>::alternate" => expr(format!("{}.alternate()", a[0])),
		"<core::fmt::Formatter>::debug_struct" => expr(format!("{}.debugStruct({})", a[0], a[1])),
		"<core::fmt::Formatter>::debug_tuple" => expr(format!("{}.debugTuple({})", a[0], a[1])),
		"<core::fmt::Formatter>::debug_list" => expr(format!("{}.debugList()", a[0])),
		"<core::fmt::builders::DebugStruct>::field" => expr(format!("{}.field({}, {})", a[0], a[1], a[2])),
		"<core::fmt::builders::DebugTuple>::field" => expr(format!("{}.field(nil, {})", a[0], a[1])),
		"<core::fmt::builders::DebugList>::entry" => expr(format!("{}.entry({})", a[0], a[1])),
		"<core::fmt::builders::DebugStruct>::finish"
		| "<core::fmt::builders::DebugTuple>::finish"
		| "<core::fmt::builders::DebugList>::finish" => expr(format!("{}.finish()", a[0])),
		"<core::fmt::builders::DebugStruct>::finish_non_exhaustive" => expr(format!("{}.finishNonExhaustive()", a[0])),
		k if k.starts_with("<core::fmt::Formatter>::debug_tuple_field") && k.ends_with("_finish") => {
			expr(format!("{}.debugTupleFields({}, {})", a[0], a[1], a[2..].join(", ")))
		}
		"<core::fmt::Formatter>::debug_struct_fields_finish" => {
			expr(format!("{}.debugStructFields({}, {}, {})", a[0], a[1], a[2], a[3]))
		}
		k if k.starts_with("<core::fmt::Formatter>::debug_struct_field") && k.ends_with("_finish") => {
			let names: Vec<&str> = a[2..].iter().step_by(2).map(String::as_str).collect();
			let values: Vec<&str> = a[3..].iter().step_by(2).map(String::as_str).collect();
			expr(format!(
				"{}.debugStructFields({}, [][]uint8{{{}}}, []fmtDyn{{{}}})",
				a[0],
				a[1],
				names.join(", "),
				values.join(", ")
			))
		}
		"<str as core::fmt::Display>::fmt" => expr(format!("{}.pad({})", a[1], a[0])),
		"<bool as core::fmt::Display>::fmt" | "<bool as core::fmt::Debug>::fmt" => {
			expr(format!("{1}.pad(map[bool][]uint8{{true: []uint8(\"true\"), false: []uint8(\"false\")}}[*{0}])", a[0], a[1]))
		}
		k if let Some((int, tr)) = int_fmt(k) => {
			let ty = targ_self(tcx, did);
			let bits = ty.map_or(64, |t| int_bits(tcx, t));
			let signed = int.starts_with('i');
			expr(match (tr, signed) {
				("Display", false) => format!("fmtUint(uint64(*{}), {})", a[0], a[1]),
				("Display", true) => format!("fmtSint(int64(*{}), {})", a[0], a[1]),
				("Debug", false) => format!("fmtUintDebug(uint64(*{}), {bits}, {})", a[0], a[1]),
				("Debug", true) => format!("fmtSintDebug(int64(*{}), {bits}, {})", a[0], a[1]),
				("LowerHex", _) => format!("fmtHex(uint64(*{}), {bits}, false, {})", a[0], a[1]),
				_ => format!("fmtHex(uint64(*{}), {bits}, true, {})", a[0], a[1]),
			})
		}
		"alloc::fmt::format" => {
			let string = dest(fx);
			expr(format!("{string}{{vec: formatBytes({})}}", a[0]))
		}
		"<T as alloc::string::ToString>::to_string" => {
			let call = fmt_call(fx, "Display", targ(0), &a[0], span)?;
			let string = dest(fx);
			expr(format!("{string}{{vec: displayBytes({call})}}"))
		}
		"core::panicking::panic_fmt" | "core::panicking::panic_nounwind_fmt" => {
			stmt(format!("panic(rustPanicFmt({}))", a[0]))
		}
		k if let Some((int, method)) = int_bytes(k) => {
			let ty = targ_self(tcx, did)?;
			let go = fx.ty(ty, span);
			let bits = int_bits(tcx, ty);
			let _ = int;
			let order = match method.contains("be") {
				true => "be",
				// The translation is checked for a little-endian target, whose native order it keeps.
				false => "le",
			};
			match (bits, method.starts_with("from")) {
				(8, false) => expr(format!("[1]uint8{{uint8({})}}", a[0])),
				(8, true) => expr(format!("{go}({}[0])", a[0])),
				(_, false) => {
					let u = format!("uint{bits}");
					expr(format!("{order}{bits}({u}({}))", a[0]))
				}
				(_, true) => {
					let up = if order == "be" { "Be" } else { "Le" };
					expr(format!("{go}(from{up}{bits}({}))", a[0]))
				}
			}
		}
		_ => {
			if std::env::var_os("RUST2_GO_KEYS").is_some() {
				eprintln!("unbound: {key}");
			}
			return None;
		}
	};
	Some(bound)
}

/// Refuse a `Vec` operation that would drop elements whose drop runs code.
fn fx_drop_check<'tcx>(fx: &mut Fx<'_, 'tcx>, elem: Ty<'tcx>, span: Span) {
	if let Some(why) = drop_refused(fx.tcx, fx.t_env(), elem) {
		fx.err(span, format!("dropping `{elem}`, {why}"));
	}
}

/// Refuse copying elements Go's `copy` and `append` can't clone: those that are not `Copy`.
fn copy_check<'tcx>(fx: &mut Fx<'_, 'tcx>, elem: Ty<'tcx>, span: Span) {
	if !fx.tcx.type_is_copy_modulo_regions(fx.t_env(), elem) {
		fx.err(span, format!("cloning elements of `{elem}`, which are not `Copy`"));
	}
}

/// A Go `func(*Formatter) fmtResult` that formats `value`, a `&ty` in Go, by `ty`'s impl
/// of the formatting trait `name`, through its binding if it has one.
pub fn fmt_call<'tcx>(fx: &mut Fx<'_, 'tcx>, name: &str, ty: Ty<'tcx>, value: &str, span: Span) -> Option<String> {
	let tcx = fx.tcx;
	let trait_id = tcx.all_traits_including_private().find(|&t| path(tcx, t) == format!("core::fmt::{name}"))?;
	let fmt = tcx.associated_items(trait_id).filter_by_name_unhygienic(rustc_span::sym::fmt).next()?.def_id;
	let instance = match Instance::try_resolve(tcx, fx.t_env(), fmt, tcx.mk_args(&[ty.into()])) {
		Ok(Some(instance)) => instance,
		_ => {
			fx.err(span, format!("`{ty}`'s `{name}`, which does not resolve"));
			return None;
		}
	};
	let fmt_ty = fx.fmt_result_ty();
	let self_ty = Ty::new_imm_ref(tcx, tcx.lifetimes.re_erased, ty);
	let f_ty = fx.formatter_ref_ty();
	let body = match call_with(fx, instance, vec![value.to_string(), "f".into()], &[self_ty, f_ty], fmt_ty, span) {
		Some(Bound::Expr(e)) => e,
		Some(Bound::Stmt(_)) => return None,
		None => format!("{}({value}, f)", fx.t.func(instance, span)),
	};
	Some(format!("func(f *Formatter) fmtResult {{ return {body} }}"))
}

/// The integer type and trait of an integer's formatting impl's `fmt`.
fn int_fmt(key: &str) -> Option<(&str, &str)> {
	let rest = key.strip_prefix('<')?;
	let (int, rest) = rest.split_once(" as core::fmt::")?;
	let tr = rest.strip_suffix(">::fmt")?;
	(INTS.contains(&int) && matches!(tr, "Display" | "Debug" | "LowerHex" | "UpperHex")).then_some((int, tr))
}

/// The integer type and method of an integer's byte conversion.
fn int_bytes(key: &str) -> Option<(&str, &str)> {
	let rest = key.strip_prefix('<')?;
	let (int, method) = rest.split_once(">::")?;
	let ok = INTS.contains(&int)
		&& matches!(
			method,
			"to_be_bytes" | "to_le_bytes" | "to_ne_bytes" | "from_be_bytes" | "from_le_bytes" | "from_ne_bytes"
		);
	ok.then_some((int, method))
}

/// The self type of the impl `did` is in.
fn targ_self<'tcx>(tcx: TyCtxt<'tcx>, did: DefId) -> Option<Ty<'tcx>> {
	let parent = tcx.parent(did);
	matches!(tcx.def_kind(parent), DefKind::Impl { .. })
		.then(|| tcx.type_of(parent).instantiate_identity().skip_norm_wip())
}

/// The Go for an intrinsic's call.
fn intrinsic<'tcx>(
	fx: &mut Fx<'_, 'tcx>,
	def_id: DefId,
	instance: Instance<'tcx>,
	a: Vec<String>,
	tys: &[Ty<'tcx>],
	dest_ty: Ty<'tcx>,
	span: Span,
) -> Option<Bound> {
	let tcx = fx.tcx;
	let name = tcx.item_name(def_id);
	let ty0 = instance.args.types().next();
	let sign = |t: Option<Ty<'tcx>>| if t.is_some_and(|t| t.is_signed()) { "S" } else { "U" };
	let expr = Bound::Expr;
	Some(match name.as_str() {
		"likely" | "unlikely" | "black_box" => expr(a[0].clone()),
		"integer_min" => expr(format!("min({}, {})", a[0], a[1])),
		"integer_max" => expr(format!("max({}, {})", a[0], a[1])),
		"cold_path" | "assert_inhabited" | "assert_zero_valid" | "assert_mem_uninitialized_valid" | "forget" => {
			Bound::Stmt("// ".to_string() + name.as_str())
		}
		"ub_checks" | "overflow_checks" | "contract_checks" => expr("false".into()),
		"add_with_overflow" => expr(format!("add{}({}, {})", sign(ty0), a[0], a[1])),
		"sub_with_overflow" => expr(format!("sub{}({}, {})", sign(ty0), a[0], a[1])),
		"mul_with_overflow" => expr(format!("mul{}({}, {})", sign(ty0), a[0], a[1])),
		"saturating_add" => expr(format!("satAdd{}({}, {})", sign(ty0), a[0], a[1])),
		"saturating_sub" => expr(format!("satSub{}({}, {})", sign(ty0), a[0], a[1])),
		"unchecked_add" | "wrapping_add" => expr(format!("({} + {})", a[0], a[1])),
		"unchecked_sub" | "wrapping_sub" => expr(format!("({} - {})", a[0], a[1])),
		"unchecked_mul" | "wrapping_mul" => expr(format!("({} * {})", a[0], a[1])),
		"unchecked_div" | "exact_div" => expr(format!("({} / {})", a[0], a[1])),
		"unchecked_rem" => expr(format!("({} % {})", a[0], a[1])),
		"unchecked_shl" => expr(format!("({} << uint({}))", a[0], a[1])),
		"unchecked_shr" => expr(format!("({} >> uint({}))", a[0], a[1])),
		"three_way_compare" => expr(format!("cmp3({}, {})", a[0], a[1])),
		"ctlz" | "ctlz_nonzero" => expr(format!("{}(clz({}))", fx.ty(dest_ty, span), a[0])),
		"cttz" | "cttz_nonzero" => expr(format!("{}(ctz({}))", fx.ty(dest_ty, span), a[0])),
		"ctpop" => expr(format!("{}(popcount({}))", fx.ty(dest_ty, span), a[0])),
		"bswap" => expr(format!("bswap({})", a[0])),
		"rotate_left" => expr(format!("rotl({}, {})", a[0], a[1])),
		"rotate_right" => expr(format!("rotr({}, {})", a[0], a[1])),
		"discriminant_value" => {
			let ty = ty0?;
			match ty.kind() {
				ty::Adt(adt, _) if adt.is_enum() => {
					let go = fx.ty(dest_ty, span);
					expr(format!("{go}({}.tag)", a[0]))
				}
				_ => expr(format!("{}(0)", fx.ty(dest_ty, span))),
			}
		}
		"size_of" | "align_of" => {
			let ty = ty0?;
			let layout = tcx.layout_of(fx.t_env().as_query_input(ty)).ok()?;
			let n = match name.as_str() {
				"size_of" => layout.size.bytes(),
				_ => layout.align.abi.bytes(),
			};
			expr(format!("uint({n})"))
		}
		"needs_drop" => expr(ty0.is_some_and(|t| t.needs_drop(tcx, fx.t_env())).to_string()),
		"read_via_copy" => expr(format!("*{}", a[0])),
		"write_box_via_move" => {
			let go = fx.ty(dest_ty, span);
			expr(format!("func() {go} {{ *{0} = {1}; return {0} }}()", a[0], a[1]))
		}
		"write_via_move" => Bound::Stmt(format!("*{} = {}", a[0], a[1])),
		"transmute" | "transmute_unchecked" => {
			let from = fx.ty(ty0?, span);
			let to = fx.ty(dest_ty, span);
			if from != to {
				fx.err(span, format!("a transmute from `{}` to `{dest_ty}`", ty0?));
			}
			expr(a[0].clone())
		}
		"const_eval_select" => {
			// The runtime function, the third argument, called with the first's fields.
			let ty::FnDef(f, fargs) = *tys[2].kind() else {
				fx.err(span, "`const_eval_select` of a function that is no item");
				return None;
			};
			let ty::Tuple(tys) = tys[0].kind() else {
				return None;
			};
			let instance = match Instance::try_resolve(tcx, fx.t_env(), f, fargs.skip_binder()) {
				Ok(Some(instance)) => instance,
				_ => {
					fx.err(span, "`const_eval_select` of a function that does not resolve");
					return None;
				}
			};
			let name = fx.t.func(instance, span);
			let fields: Vec<String> = (0..tys.len()).map(|i| format!("{}.F{i}", a[0])).collect();
			let call = format!("{name}({})", fields.join(", "));
			match super::Translator::no_value(dest_ty) {
				true => Bound::Stmt(call),
				false => expr(call),
			}
		}
		"abort" => Bound::Stmt("panic(\"abort\")".into()),
		_ => return None,
	})
}
