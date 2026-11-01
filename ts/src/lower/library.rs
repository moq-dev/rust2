//! Rust's standard library: its types' representations, and its functions as runtime
//! helpers or JS. A function missing here fails translation with its path.
//!
//! Functions are keyed by their owner and name: an inherent method by its type
//! (`slice::split_first`), a trait method by its trait (`From::from`), and a free
//! function by its module (`mem::take`).

use rustc_hir::def::DefKind;
use rustc_hir::def_id::DefId;
use rustc_middle::ty::{self, GenericArgsRef, Instance, Ty, TyCtxt, TypingEnv};
use rustc_span::Span;

use super::Translator;
use super::body::Body;
use super::fmt;
use super::num::{self, Int, Operand, Overflow};
use crate::ts::{Expr, Stmt, StmtKind};

/// A type with a representation of its own: std's, or a well-known crate's that
/// [`super::stock`] maps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Std {
	/// `T | undefined`.
	Option,
	/// `rt.Result<T, E>`.
	Result,
	/// `rt.ByteVec` for bytes, an array otherwise.
	Vec,
	/// A JS string.
	String,
	/// A JS string too, for `Cow<str>`, which keeps no variant: either is the text.
	Cow,
	/// Its contents.
	Box,
	FromUtf8Error,
	Utf8Error,
	TryFromIntError,
	/// `rt.ParseIntError`: `{ kind }`, its `IntErrorKind`.
	ParseIntError,
	/// Why one of std's errors failed, as `rt.IntErrorKind` or `rt.CharErrorKind`: the
	/// variant's name.
	ErrorKind,
	/// `rt.ParseBoolError`.
	ParseBoolError,
	/// `rt.ParseCharError`: `{ kind }`, its `CharErrorKind`'s name.
	ParseCharError,
	ParseFloatError,
	TryFromFloatSecsError,
	/// `rt.Formatter`.
	Formatter,
	/// `rt.Arguments`: what `format_args!` makes.
	Arguments,
	/// `rt.FmtError`.
	FmtError,
	/// The `Debug` builders: `rt.DebugStruct` and the like.
	DebugStruct,
	DebugTuple,
	DebugList,
	/// What `assert_eq!` and `assert_ne!` compare by: its operator, as a string.
	AssertKind,
	/// `rt.Arc`, counted: an `Arc` or an `Rc`, which one thread can't tell apart.
	Arc,
	/// `rt.Weak`, of either.
	Weak,
	/// `rt.Mutex`, which is also its own guard.
	Mutex,
	MutexGuard,
	/// What it holds, a lock's guard or value.
	PoisonError,
	/// `rt.TryLockError<T>`, tagged as `Bound` is.
	TryLockError,
	/// `rt.RefCell`: a `RefCell` or an `RwLock`, which one thread can't tell apart, and
	/// also its own guard, shared or exclusive.
	RefCell,
	Ref,
	RefMut,
	/// `rt.BorrowError` and `rt.BorrowMutError`: why `try_borrow` and `try_borrow_mut` failed.
	BorrowError,
	BorrowMutError,
	/// `rt.Cell<T>`, a place holding its value: a `Cell`'s or an `UnsafeCell`'s.
	Cell,
	/// `rt.LocalKey<T>`: a thread-local, a module's value.
	LocalKey,
	/// `rt.Atomic<T>`, a place holding its `bool` or integer.
	Atomic,
	/// `rt.AtomicOrdering`: its discriminant.
	AtomicOrdering,
	/// `rt.OnceCell<T>` and `rt.OnceLock<T>`: the `Option<T>` each holds.
	OnceCell,
	OnceLock,
	/// `rt.Once`, an `rt.OnceLock` of `true`, and `rt.OnceState`, the `Once` it reports on.
	Once,
	OnceState,
	/// `rt.LazyCell<T>` and `rt.LazyLock<T>`: the function that makes the value, then the value.
	LazyCell,
	LazyLock,
	/// `rt.Waker`.
	Waker,
	/// `rt.WakerVTable`: a waker's vtable, of which only its identity is read.
	WakerVTable,
	/// `rt.Context`: its waker.
	Context,
	/// `rt.Poll<T>`: `T` itself, or `rt.Pending`.
	Poll,
	/// Its pointer, or the value `pin!` pins in a `PinMacroHelper`.
	Pin,
	/// `rt.PollFn`: `poll_fn`'s future, which owns its function.
	PollFn,
	/// `future::Pending`: `rt.NEVER`, a future never ready.
	Pending,
	/// `rt.Range`, an iterator with `start` and `end` fields.
	Range,
	/// `rt.RangeInclusive`.
	RangeInclusive,
	/// `rt.Peekable`, an iterator with `peek`.
	Peekable,
	/// What `?` branches on, as MIR spells it, and what else makes one: see [`Flow`].
	ControlFlow,
	/// `..end`, `start..`, and `..`, as MIR makes them to slice by: `{ end }`, `{ start }`,
	/// and `undefined`.
	RangeTo,
	RangeFrom,
	RangeFull,
	/// `..=end`: `{ end, inclusive: true }`.
	RangeToInclusive,
	/// `rt.RangeBound<T>`, tagged as the crate's own enums are: `{ tag: "Included", _0 }`.
	Bound,
	/// What it holds, which orders the other way.
	Reverse,
	/// Its integer, which is never 0.
	NonZero,
	/// What it holds, whose arithmetic wraps.
	Wrapping,
	/// What it holds, whose arithmetic saturates at its bounds.
	Saturating,
	/// A `format_args!` argument: a function writing its value, `rt.Fmt`.
	Argument,
	/// A raw pointer to its pointee, as `pointer` has it.
	NonNull,
	/// What it holds, or nothing yet, as `pointer` has it.
	MaybeUninit,
	/// No value: `never`.
	Infallible,
	/// `rt.HashMap`, and `rt.HashSet`, a map to `undefined`.
	HashMap,
	HashSet,
	/// `rt.BTreeMap`, and `rt.BTreeSet`, a map to `undefined`.
	BTreeMap,
	BTreeSet,
	/// `rt.VecDeque`, a ring buffer.
	VecDeque,
	/// `rt.LinkedList`, nodes linked both ways.
	LinkedList,
	/// `rt.BinaryHeap`, std's max-heap in an array, and `rt.PeekMut`, its greatest element lent.
	BinaryHeap,
	PeekMut,
	/// `rt.Entry`: a map's `Entry`, `OccupiedEntry`, or `VacantEntry`.
	Entry,
	/// `rt.Ordering`: its discriminant, -1, 0, or 1.
	Ordering,
	/// `rt.Hasher`: `DefaultHasher`, which hash collections hash their keys with.
	Hasher,
	/// `rt.RandomState`: the keys of the `DefaultHasher` it builds, never changed once made.
	RandomState,
	/// `rt.Duration`: its seconds and nanoseconds, never changed once made.
	Duration,
	/// `bytes::Bytes`: an `rt.ByteSlice`, as `&[u8]` is.
	Bytes,
	/// `bytes::BytesMut`: `rt.BytesMut`, a window of storage it shares with what it splits off.
	BytesMut,
	/// `rand::rngs::ThreadRng`: `rt.ThreadRng`, which holds no state.
	ThreadRng,
}

impl Std {
	/// Whether it is one of std's errors, whose derived `PartialEq` lowers from its MIR.
	pub fn error(self) -> bool {
		matches!(
			self,
			Self::FromUtf8Error
				| Self::Utf8Error
				| Self::TryFromIntError
				| Self::ParseIntError
				| Self::ParseBoolError
				| Self::ParseCharError
				| Self::ParseFloatError
				| Self::TryFromFloatSecsError
				| Self::FmtError
		)
	}

	/// How its operators overflow, if it is a `Wrapping` or a `Saturating`.
	pub fn overflow(self) -> Option<Overflow> {
		match self {
			Self::Wrapping => Some(Overflow::Wrap),
			Self::Saturating => Some(Overflow::Saturate),
			_ => None,
		}
	}

	pub fn of(tcx: TyCtxt<'_>, did: DefId) -> Option<Self> {
		if !is_std(tcx, did) {
			return super::stock::of(tcx, did);
		}
		use rustc_span::sym;
		if tcx.is_diagnostic_item(sym::Option, did) {
			return Some(Self::Option);
		}
		if tcx.is_diagnostic_item(sym::Result, did) {
			return Some(Self::Result);
		}
		if tcx.is_diagnostic_item(sym::Vec, did) {
			return Some(Self::Vec);
		}
		if tcx.is_lang_item(did, rustc_attr_ir::LangItem::String) {
			return Some(Self::String);
		}
		if tcx.is_lang_item(did, rustc_attr_ir::LangItem::OwnedBox) {
			return Some(Self::Box);
		}
		if tcx.is_diagnostic_item(sym::Cow, did) {
			return Some(Self::Cow);
		}
		for (item, std) in [
			(sym::Arc, Self::Arc),
			(sym::ArcWeak, Self::Weak),
			(sym::Rc, Self::Arc),
			(sym::RcWeak, Self::Weak),
			(sym::Mutex, Self::Mutex),
			(sym::MutexGuard, Self::MutexGuard),
			(sym::RefCell, Self::RefCell),
			(sym::RwLock, Self::RefCell),
			(rustc_span::Symbol::intern("RefCellRef"), Self::Ref),
			(sym::RwLockReadGuard, Self::Ref),
			(rustc_span::Symbol::intern("RefCellRefMut"), Self::RefMut),
			(sym::RwLockWriteGuard, Self::RefMut),
			(sym::Cell, Self::Cell),
			(rustc_span::Symbol::intern("LocalKey"), Self::LocalKey),
			(rustc_span::Symbol::intern("Atomic"), Self::Atomic),
			(rustc_span::Symbol::intern("Waker"), Self::Waker),
			(rustc_span::Symbol::intern("PinMacroHelper"), Self::Pin),
			(rustc_span::Symbol::intern("HashMap"), Self::HashMap),
			(rustc_span::Symbol::intern("HashSet"), Self::HashSet),
			(rustc_span::Symbol::intern("BTreeMap"), Self::BTreeMap),
			(rustc_span::Symbol::intern("BTreeSet"), Self::BTreeSet),
			(rustc_span::Symbol::intern("VecDeque"), Self::VecDeque),
			(rustc_span::Symbol::intern("LinkedList"), Self::LinkedList),
			(rustc_span::Symbol::intern("BinaryHeap"), Self::BinaryHeap),
			(rustc_span::Symbol::intern("Duration"), Self::Duration),
		] {
			if tcx.is_diagnostic_item(item, did) {
				return Some(std);
			}
		}
		for (item, std) in [
			(rustc_attr_ir::LangItem::UnsafeCell, Self::Cell),
			(rustc_attr_ir::LangItem::Context, Self::Context),
			(rustc_attr_ir::LangItem::ResumeTy, Self::Context),
			(rustc_attr_ir::LangItem::Poll, Self::Poll),
			(rustc_attr_ir::LangItem::Pin, Self::Pin),
			(rustc_attr_ir::LangItem::Range, Self::Range),
			(rustc_attr_ir::LangItem::RangeInclusiveStruct, Self::RangeInclusive),
			(rustc_attr_ir::LangItem::RangeTo, Self::RangeTo),
			(rustc_attr_ir::LangItem::RangeFrom, Self::RangeFrom),
			(rustc_attr_ir::LangItem::RangeFull, Self::RangeFull),
			(rustc_attr_ir::LangItem::RangeToInclusive, Self::RangeToInclusive),
		] {
			if tcx.is_lang_item(did, item) {
				return Some(std);
			}
		}
		match tcx.item_name(did).as_str() {
			"FromUtf8Error" => return Some(Self::FromUtf8Error),
			"TryFromIntError" => return Some(Self::TryFromIntError),
			"ParseFloatError" => return Some(Self::ParseFloatError),
			"TryFromFloatSecsError" => return Some(Self::TryFromFloatSecsError),
			"PoisonError" => return Some(Self::PoisonError),
			"TryLockError" => return Some(Self::TryLockError),
			_ => {}
		}
		match std_path(tcx, did)
			.iter()
			.map(String::as_str)
			.collect::<Vec<_>>()
			.as_slice()
		{
			["str", "error", "Utf8Error"] => Some(Self::Utf8Error),
			["num", "error", "ParseIntError"] => Some(Self::ParseIntError),
			["num", "error", "IntErrorKind"]
			| ["char", "convert", "CharErrorKind"]
			| ["num", "float_parse", "FloatErrorKind"]
			| ["time", "TryFromFloatSecsErrorKind"] => Some(Self::ErrorKind),
			["str", "error", "ParseBoolError"] => Some(Self::ParseBoolError),
			["char", "convert", "ParseCharError"] => Some(Self::ParseCharError),
			["fmt", "Formatter"] => Some(Self::Formatter),
			["fmt", "Arguments"] => Some(Self::Arguments),
			["fmt", "Error"] => Some(Self::FmtError),
			["fmt", "builders", "DebugStruct"] => Some(Self::DebugStruct),
			["fmt", "builders", "DebugTuple"] => Some(Self::DebugTuple),
			["fmt", "builders", "DebugList"] => Some(Self::DebugList),
			["panicking", "AssertKind"] => Some(Self::AssertKind),
			["iter", "adapters", "peekable", "Peekable"] => Some(Self::Peekable),
			["ops", "control_flow", "ControlFlow"] => Some(Self::ControlFlow),
			["fmt", "rt", "Argument"] => Some(Self::Argument),
			["ptr", "non_null", "NonNull"] => Some(Self::NonNull),
			["mem", "maybe_uninit", "MaybeUninit"] => Some(Self::MaybeUninit),
			["future", "poll_fn", "PollFn"] => Some(Self::PollFn),
			["future", "pending", "Pending"] => Some(Self::Pending),
			["task", "wake", "RawWakerVTable"] => Some(Self::WakerVTable),
			["convert", "Infallible"] => Some(Self::Infallible),
			// A thread-local on wasm32 is never destroyed, so `try_with` never fails.
			["thread", "local", "AccessError"] => Some(Self::Infallible),
			["cmp", "Ordering"] => Some(Self::Ordering),
			["sync", "atomic", "Ordering"] => Some(Self::AtomicOrdering),
			["cell", "once", "OnceCell"] => Some(Self::OnceCell),
			["cell", "BorrowError"] => Some(Self::BorrowError),
			["cell", "BorrowMutError"] => Some(Self::BorrowMutError),
			["sync", "once_lock", "OnceLock"] => Some(Self::OnceLock),
			["sync", "once", "Once"] => Some(Self::Once),
			["sync", "once", "OnceState"] => Some(Self::OnceState),
			["cell", "lazy", "LazyCell"] => Some(Self::LazyCell),
			["sync", "lazy_lock", "LazyLock"] => Some(Self::LazyLock),
			["cmp", "Reverse"] => Some(Self::Reverse),
			["collections", "binary_heap", "PeekMut"] => Some(Self::PeekMut),
			["ops", "range", "Bound"] => Some(Self::Bound),
			["num", "nonzero", "NonZero"] => Some(Self::NonZero),
			["num", "wrapping", "Wrapping"] => Some(Self::Wrapping),
			["num", "saturating", "Saturating"] => Some(Self::Saturating),
			["hash", "random", "DefaultHasher"] => Some(Self::Hasher),
			["hash", "random", "RandomState"] => Some(Self::RandomState),
			["collections", "hash", "map", "Entry" | "OccupiedEntry" | "VacantEntry"]
			| [
				"collections",
				"btree",
				"map",
				"entry",
				"Entry" | "OccupiedEntry" | "VacantEntry",
			] => Some(Self::Entry),
			_ => None,
		}
	}
}

/// How a `ControlFlow<B, C>` is represented. What `?` branches on is the value it was
/// applied to, which breaks with what can only be its `Err` or `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
	/// Breaking with a `Result<!, E>`: the `rt.Result<C, E>` `?` was applied to.
	Result,
	/// Breaking with an `Option<!>`: the `Option<C>` `?` was applied to.
	Option,
	/// Any other: an `rt.Result<C, B>`, `Ok` to continue and `Err` to break.
	Other,
}

impl Flow {
	/// The representation of a `ControlFlow` with `args`, its break's type first.
	pub fn of<'tcx>(tcx: TyCtxt<'tcx>, args: GenericArgsRef<'tcx>) -> Self {
		let ty::Adt(adt, residual) = args.type_at(0).kind() else {
			return Self::Other;
		};
		let uninhabited =
			|t: Ty<'tcx>| t.is_never() || matches!(t.kind(), ty::Adt(e, _) if e.is_enum() && e.variants().is_empty());
		match Std::of(tcx, adt.did()) {
			Some(Std::Result) if uninhabited(residual.type_at(0)) => Self::Result,
			Some(Std::Option) if uninhabited(residual.type_at(0)) => Self::Option,
			_ => Self::Other,
		}
	}

	/// The test that `value`, a `ControlFlow` with `args`, continues, or breaks.
	pub fn test<'tcx>(t: &Translator<'_, 'tcx>, args: GenericArgsRef<'tcx>, value: Expr, continues: bool) -> Expr {
		match (Self::of(t.tcx, args), continues) {
			(Self::Result | Self::Other, true) => value.member("ok"),
			(Self::Result | Self::Other, false) => value.member("ok").not(),
			(Self::Option, true) => value.binary("!==", t.none(args.type_at(1))),
			(Self::Option, false) => value.binary("===", t.none(args.type_at(1))),
		}
	}
}

/// Whether `ty` holds nothing, so its value is `undefined`: `()`, `PhantomData`,
/// `PhantomPinned`, or `..`, whose impls each compare equal and hash nothing.
pub fn nothing<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
	match ty.kind() {
		ty::Tuple(tys) => tys.is_empty(),
		ty::Adt(adt, _) => {
			adt.is_phantom_data() || pinned(tcx, adt.did()) || Std::of(tcx, adt.did()) == Some(Std::RangeFull)
		}
		_ => false,
	}
}

/// Whether `did` is `PhantomPinned`.
pub fn pinned(tcx: TyCtxt<'_>, did: DefId) -> bool {
	tcx.is_diagnostic_item(rustc_span::Symbol::intern("PhantomPinned"), did)
}

/// Whether `did` is `iter::Empty`, an `rt.Iter` that holds nothing.
pub fn empty_iter(tcx: TyCtxt<'_>, did: DefId) -> bool {
	is_std(tcx, did) && std_path(tcx, did) == ["iter", "sources", "empty", "Empty"]
}

/// Whether a `ty`'s value is a JS string: a `str`, a `String`, a `Box<str>`, or a `Cow<str>`.
pub fn string<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
	match ty.kind() {
		ty::Str => true,
		ty::Adt(adt, args) => match Std::of(tcx, adt.did()) {
			Some(Std::String) => true,
			// A `Cow`'s first argument is its lifetime.
			Some(Std::Cow | Std::Box) => args.types().next().is_some_and(|t| t.is_str()),
			_ => false,
		},
		_ => false,
	}
}

/// What a value of `ty` is in JS if `ty` wraps one value it is represented as: `Reverse<T>`'s,
/// `NonZero<T>`'s, `Wrapping<T>`'s, and `Saturating<T>`'s `T`, which differ only in their
/// order, range, and arithmetic, a pattern type's base, and the field of each of core's
/// `niche_types` structs that hold one.
pub fn wrapped<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
	match ty.kind() {
		ty::Pat(base, _) => Some(*base),
		ty::Adt(adt, args) => match Std::of(tcx, adt.did()) {
			Some(Std::Reverse | Std::NonZero | Std::Wrapping | Std::Saturating) => Some(args.type_at(0)),
			Some(_) => None,
			None => {
				let path = std_path(tcx, adt.did());
				if !(path.len() == 3 && path[0] == "num" && path[1] == "niche_types" && adt.is_struct()) {
					return None;
				}
				let field = adt.non_enum_variant().fields.iter().next()?;
				Some(field.ty(tcx, args).skip_norm_wip())
			}
		},
		_ => None,
	}
}

/// A `Bound` or a `TryLockError` of type `ty`, tagged as the crate's own enums are: its
/// variant, by name, and the value it holds, if any.
pub fn tagged(variant: &str, value: Option<Expr>, ty: crate::ts::Type) -> Expr {
	let mut props = vec![(crate::ts::Key::Name("tag".into()), Expr::Str(variant.into()))];
	props.extend(value.map(|v| (crate::ts::Key::Name("_0".into()), v)));
	Expr::As(Box::new(Expr::Object(props)), ty)
}

/// `..end`, or `..=end` if `inclusive`: `{ end }`, or `{ end, inclusive: true }`.
pub fn range_to(end: Expr, inclusive: bool) -> Expr {
	let mut props = vec![(crate::ts::Key::Name("end".into()), end)];
	if inclusive {
		props.push((crate::ts::Key::Name("inclusive".into()), Expr::lit("true")));
	}
	Expr::Object(props)
}

/// What a `Box<[T]>` of type `ty` holds, `T`, or `None` for any other type.
pub fn box_slice<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
	match ty.kind() {
		ty::Adt(adt, args) if Std::of(tcx, adt.did()) == Some(Std::Box) => match args.type_at(0).kind() {
			ty::Slice(elem) => Some(*elem),
			_ => None,
		},
		_ => None,
	}
}

/// What a `Cow<[T]>` of type `ty` holds, `T`, or `None` for any other type.
pub fn cow_slice<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
	match ty.kind() {
		// Its first argument is its lifetime.
		ty::Adt(adt, args) if Std::of(tcx, adt.did()) == Some(Std::Cow) => match args.types().next()?.kind() {
			ty::Slice(elem) => Some(*elem),
			_ => None,
		},
		_ => None,
	}
}

/// The slice `cow`, a `Cow<[elem]>`, derefs to: an `rt.ByteSlice` of bytes, or an array.
pub fn cow_deref(cow: Expr, elem: Ty<'_>) -> Expr {
	match elem.kind() {
		ty::Uint(ty::UintTy::U8) => Expr::rt("byteCowSlice", vec![cow]),
		_ => Expr::rt("cowSlice", vec![cow]),
	}
}

/// A `Cow<[elem]>` made by its variant `variant` of `value`: the `Vec` it owns, or the
/// slice it borrows, wrapped in an `rt.Borrowed` unless it is bytes.
pub fn cow_made(elem: Ty<'_>, variant: rustc_abi::VariantIdx, value: Expr) -> Expr {
	match variant.index() == 0 && !matches!(elem.kind(), ty::Uint(ty::UintTy::U8)) {
		true => Expr::New(Box::new(Expr::Rt("Borrowed".into())), vec![value]),
		false => value,
	}
}

/// Whether `cow`, a `Cow<[elem]>`, borrows: whether it is of the class its borrowed variant is.
pub fn cow_borrows(cow: Expr, elem: Ty<'_>) -> Expr {
	let class = match elem.kind() {
		ty::Uint(ty::UintTy::U8) => "ByteSlice",
		_ => "Borrowed",
	};
	cow.binary("instanceof", Expr::Rt(class.into()))
}

/// The refusal of code that tells a `Cow<str>`'s variants apart.
pub fn cow_variant(ty: Ty<'_>) -> String {
	format!("a variant of `{ty}`, which is not supported: either is the same JS string")
}

/// The path of a std item within its crate, as `["slice", "iter", "Iter"]`; empty for
/// any other crate's.
pub fn std_path(tcx: TyCtxt<'_>, did: DefId) -> Vec<String> {
	if !is_std(tcx, did) {
		return Vec::new();
	}
	tcx.def_path(did)
		.data
		.iter()
		.filter_map(|d| d.data.get_opt_name())
		.map(|n| n.to_string())
		.collect()
}

pub fn is_std(tcx: TyCtxt<'_>, did: DefId) -> bool {
	matches!(tcx.crate_name(did.krate).as_str(), "core" | "alloc" | "std")
}

/// If `instance` is a method of std's `PartialEq`, `PartialOrd`, `Ord`, or `Hash` impl
/// for references, which calls its referents' with the operands dereferenced, the
/// generic arguments of that call. A reference is its referent in JS, so the operands
/// pass as they are.
pub fn referent<'tcx>(
	tcx: TyCtxt<'tcx>,
	def_id: DefId,
	gargs: GenericArgsRef<'tcx>,
	instance: Instance<'tcx>,
) -> Option<GenericArgsRef<'tcx>> {
	let tr = tcx.trait_of_assoc(def_id)?;
	// A method the impl leaves to its trait's default, as `Ord::max` is, is not forwarded.
	let imp = tcx.impl_of_assoc(instance.def_id())?;
	if !is_std(tcx, imp) {
		return None;
	}
	let operands = match tcx.get_diagnostic_name(tr)?.as_str() {
		"PartialEq" | "PartialOrd" => 2,
		"Ord" | "Hash" => 1,
		_ => return None,
	};
	let mut args = gargs.to_vec();
	for arg in &mut args[..operands] {
		let ty::Ref(_, pointee, _) = arg.as_type()?.kind() else {
			return None;
		};
		*arg = (*pointee).into();
	}
	Some(tcx.mk_args(&args))
}

/// A comparison an impl of `PartialEq` or `PartialOrd` inherits from its trait, as a
/// derived one inherits all but `eq` and `partial_cmp`.
#[derive(Clone, Copy)]
pub enum Inherited {
	Ne,
	Lt,
	Le,
	Gt,
	Ge,
}

impl Inherited {
	/// The comparison `instance` is, if it is the trait's own default.
	pub fn of(tcx: TyCtxt<'_>, instance: Instance<'_>) -> Option<Self> {
		let ty::InstanceKind::Item(did) = instance.def else {
			return None;
		};
		let tr = tcx.trait_of_assoc(did)?;
		let inherited = match (tcx.get_diagnostic_name(tr)?.as_str(), tcx.item_name(did).as_str()) {
			("PartialEq", "ne") => Self::Ne,
			("PartialOrd", "lt") => Self::Lt,
			("PartialOrd", "le") => Self::Le,
			("PartialOrd", "gt") => Self::Gt,
			("PartialOrd", "ge") => Self::Ge,
			_ => return None,
		};
		Some(inherited)
	}

	/// What the default calls, with the same generic arguments and operands: `eq` or
	/// `partial_cmp`.
	pub fn via(self, tcx: TyCtxt<'_>) -> DefId {
		match self {
			Self::Ne => super::collections::trait_fn(tcx, "PartialEq", "eq"),
			_ => super::collections::trait_fn(tcx, "PartialOrd", "partial_cmp"),
		}
	}

	/// The comparison, from what `via` returned: `!eq`, or whether the `partial_cmp` is
	/// `Some` and of the order wanted, as std's defaults test it.
	pub fn test(self, b: &mut Body<'_, '_, '_>, value: Expr, out: &mut Vec<Stmt>) -> Expr {
		let (op, order) = match self {
			Self::Ne => return value.not(),
			Self::Lt => return value.binary("===", Expr::lit("-1")),
			Self::Gt => return value.binary("===", Expr::lit("1")),
			Self::Le => ("<=", "0"),
			Self::Ge => (">=", "0"),
		};
		let value = match value.is_path() {
			true => value,
			false => b.spill(value, "order", out),
		};
		let some = value.clone().binary("!==", Expr::undefined());
		some.binary("&&", value.binary(op, Expr::lit(order)))
	}
}

/// Whether `Clone` of `ty` only copies: `ty` is `Copy`, and no `Clone` written by hand
/// runs for it or anything in it, as a derived or std `Clone` clones each field. `env`
/// holds the bounds of the body naming `ty`.
pub fn clone_copies<'tcx>(t: &super::Translator<'_, 'tcx>, env: TypingEnv<'tcx>, ty: Ty<'tcx>) -> bool {
	t.tcx.type_is_copy_modulo_regions(env, ty) && clone_only_copies(t, env, ty)
}

/// Whether `Clone` of `ty` is the value itself, as JS holds it: a string, a function, or a
/// closure that writes none of its captures, each of which only copies or clones so, as
/// nothing else reaches what a closure owns to write it.
pub fn clone_shares<'tcx>(t: &super::Translator<'_, 'tcx>, env: TypingEnv<'tcx>, ty: Ty<'tcx>) -> bool {
	match ty.kind() {
		ty::FnDef(..) => true,
		ty::Closure(..) | ty::CoroutineClosure(..) => {
			!super::mir::writes_captures(t.tcx, ty)
				&& (super::captured(ty).unwrap_or_default().iter())
					.all(|capture| clone_copies(t, env, capture) || clone_shares(t, env, capture))
		}
		_ => string(t.tcx, ty),
	}
}

/// Whether `Clone` of `ty`, a `Copy` type, only copies.
fn clone_only_copies<'tcx>(t: &super::Translator<'_, 'tcx>, env: TypingEnv<'tcx>, ty: Ty<'tcx>) -> bool {
	let tcx = t.tcx;
	match ty.kind() {
		ty::Bool | ty::Char | ty::Int(_) | ty::Uint(_) | ty::Float(_) | ty::Ref(..) | ty::FnPtr(..) | ty::Never => true,
		ty::Tuple(fields) => fields.iter().all(|field| clone_only_copies(t, env, field)),
		ty::Array(elem, _) => clone_only_copies(t, env, *elem),
		// A binding's `Clone` is its own.
		ty::Adt(adt, _) if t.bound_drops(adt.did()).is_some() => false,
		ty::Adt(adt, args) if !t.crates.translates(adt.did()) => args.types().all(|arg| clone_only_copies(t, env, arg)),
		ty::Adt(adt, args) => {
			let Some(clone) = tcx.lang_items().clone_fn() else {
				return false;
			};
			let derived = Instance::try_resolve(tcx, env, clone, tcx.mk_args(&[ty.into()]))
				.ok()
				.flatten()
				.and_then(|instance| tcx.impl_of_assoc(instance.def_id()))
				.is_some_and(|imp| tcx.is_automatically_derived(imp));
			derived
				&& adt
					.all_fields()
					.all(|field| clone_only_copies(t, env, super::copy::field_ty(tcx, env, field.did, args)))
		}
		_ => false,
	}
}

/// The standard type an inherent method is of, such as `Range` for `Range::contains`.
pub fn inherent_std(tcx: TyCtxt<'_>, def_id: DefId) -> Option<Std> {
	let imp = tcx.inherent_impl_of_assoc(def_id)?;
	match tcx.type_of(imp).instantiate_identity().skip_norm_wip().kind() {
		ty::Adt(adt, _) => Std::of(tcx, adt.did()),
		_ => None,
	}
}

/// Whether std's `instance` lowers from its own MIR, as the crate's functions do, rather
/// than mapping here: a method of `bool`, `Option`, `Result`, `Poll`, or `LocalKey`, or
/// their `Clone`, or the derived `PartialEq::eq` of a `Result`, a `Poll`, or one of std's
/// errors, or a `Result`'s or a `Poll`'s derived `PartialOrd::partial_cmp` and `Ord::cmp`,
/// or `cmp`'s `max` and `min` and their `_by` and `_by_key` forms, or what
/// parses an integer, a `NonZero`, or a `bool`: `str::parse` of one, its `FromStr`,
/// `from_str_radix`, and what they call, `char::to_digit` and the radix's panic among it.
/// Not one taking a `&mut` or a raw pointer, whose writes the crate's analysis of what it
/// writes into would miss, nor an `unsafe` one, nor one mapped below as one expression or
/// as the runtime's panic.
pub fn from_mir<'tcx>(t: &Translator<'_, 'tcx>, env: TypingEnv<'tcx>, instance: Instance<'tcx>) -> bool {
	let tcx = t.tcx;
	let ty::InstanceKind::Item(did) = instance.def else {
		return false;
	};
	if !may_from_mir(tcx, did) {
		return false;
	}
	let Some(imp) = tcx.impl_of_assoc(did) else {
		return true;
	};
	let name = tcx.item_name(did);
	let self_ty = tcx.type_of(imp).instantiate_identity().skip_norm_wip();
	if self_ty.is_str() {
		let target = instance.args.type_at(0);
		return int(tcx, target) || target.is_bool();
	}
	if let Some(tr) = tcx.trait_impl_of_assoc(did).map(|imp| tcx.impl_trait_id(imp))
		&& tcx.is_lang_item(tr, rustc_attr_ir::LangItem::Clone)
	{
		return !clone_copies(t, env, super::copy::instantiate(tcx, env, self_ty, instance.args));
	}
	let std = match self_ty.kind() {
		ty::Adt(adt, _) => Std::of(tcx, adt.did()),
		_ => None,
	};
	// A `RefCell` key's `set` maps, as its body writes the cell in place.
	let refcell_set = std == Some(Std::LocalKey)
		&& name.as_str() == "set"
		&& matches!(super::copy::instantiate(tcx, env, self_ty, instance.args).kind(), ty::Adt(_, args)
			if matches!(args.type_at(0).kind(), ty::Adt(adt, _) if Std::of(tcx, adt.did()) == Some(Std::RefCell)));
	// A lock's `unwrap_or_else` maps, to unwrap its guard in place.
	let lock = std == Some(Std::Result)
		&& name.as_str() == "unwrap_or_else"
		&& matches!(instance.args.type_at(1).kind(), ty::Adt(adt, _) if Std::of(tcx, adt.did()) == Some(Std::PoisonError));
	!lock && !refcell_set
}

/// The generic arguments of the first function `callee` picks that std's body of `def_id`,
/// instantiated by `args`, calls: what that body passes on, such as an adaptor it wraps an
/// iterator in.
pub fn callee_args<'tcx>(
	t: &Translator<'_, 'tcx>,
	def_id: DefId,
	args: GenericArgsRef<'tcx>,
	callee: impl Fn(DefId) -> bool,
) -> Option<GenericArgsRef<'tcx>> {
	let tcx = t.tcx;
	let env = t.typing_env_pub();
	let instance = Instance::try_resolve(tcx, env, def_id, args).ok()??;
	let called = tcx.instance_mir(instance.def).basic_blocks.iter().find_map(|block| {
		let rustc_middle::mir::TerminatorKind::Call { func, .. } = &block.terminator().kind else {
			return None;
		};
		func.const_fn_def().filter(|&(f, _)| callee(f)).map(|(_, args)| args)
	})?;
	instance
		.try_instantiate_mir_and_normalize_erasing_regions(tcx, env, ty::EarlyBinder::bind(tcx, called))
		.ok()
}

/// Whether some instance of std's `did` may lower from its MIR, as [`from_mir`] decides
/// for each: what only the instance decides is left to it.
pub fn may_from_mir(tcx: TyCtxt<'_>, did: DefId) -> bool {
	if !is_std(tcx, did) {
		return false;
	}
	if tcx.def_kind(did) == DefKind::Fn {
		let path = std_path(tcx, did);
		return matches!(
			path.iter().map(String::as_str).collect::<Vec<_>>().as_slice(),
			["cmp", "max" | "min" | "max_by" | "min_by" | "max_by_key" | "min_by_key"]
				| ["num", "can_not_overflow"]
				// With the functions `const_panic!` nests in it.
				| ["num", "from_ascii_bytes_radix_panic", ..]
		);
	}
	if tcx.def_kind(did) != DefKind::AssocFn {
		return false;
	}
	let Some(imp) = tcx.impl_of_assoc(did) else {
		return false;
	};
	let name = tcx.item_name(did);
	let self_ty = tcx.type_of(imp).instantiate_identity().skip_norm_wip();
	let parses = match self_ty.kind() {
		_ if int(tcx, self_ty) => {
			matches!(
				name.as_str(),
				"from_str" | "from_str_radix" | "from_ascii_bytes_radix_impl"
			)
		}
		// Of an integer or a `bool`, which the instance says.
		ty::Str => name.as_str() == "parse",
		ty::Bool => name.as_str() == "from_str",
		ty::Char => name.as_str() == "to_digit",
		_ => false,
	};
	if parses {
		return true;
	}
	// A std error's derived `PartialEq`, which compares its fields.
	if let ty::Adt(adt, _) = self_ty.kind()
		&& Std::of(tcx, adt.did()).is_some_and(Std::error)
		&& tcx.is_automatically_derived(imp)
	{
		return name == rustc_span::sym::eq;
	}
	// `None` for `bool`'s.
	let std = match self_ty.kind() {
		ty::Adt(adt, _) => match Std::of(tcx, adt.did()) {
			std @ Some(Std::Option | Std::Result | Std::Poll | Std::LocalKey) => std,
			_ => return false,
		},
		ty::Bool => None,
		_ => return false,
	};
	let sig = tcx.fn_sig(did).instantiate_identity().skip_norm_wip().skip_binder();
	let writes = |ty: Ty<'_>| {
		ty.walk()
			.filter_map(|arg| arg.as_type())
			.any(|t| matches!(t.kind(), ty::Ref(_, _, ty::Mutability::Mut) | ty::RawPtr(..)))
	};
	if sig.safety().is_unsafe() || sig.inputs().iter().any(|&ty| writes(ty)) {
		return false;
	}
	if let Some(tr) = tcx.trait_impl_of_assoc(did).map(|imp| tcx.impl_trait_id(imp)) {
		// A `Result`'s or a `Poll`'s derived `PartialEq`, `PartialOrd`, and `Ord`, which compare
		// by variant and then what each holds by its own.
		if matches!(std, Some(Std::Result | Std::Poll)) {
			if tcx.is_lang_item(tr, rustc_attr_ir::LangItem::PartialEq) {
				return name == rustc_span::sym::eq;
			}
			if tcx.is_lang_item(tr, rustc_attr_ir::LangItem::PartialOrd) {
				return name == rustc_span::sym::partial_cmp;
			}
			if tcx.is_diagnostic_item(rustc_span::sym::Ord, tr) {
				return name == rustc_span::sym::cmp;
			}
		}
		// A `Clone` that only copies is the copy, which the instance says.
		return tcx.is_lang_item(tr, rustc_attr_ir::LangItem::Clone) && name == rustc_span::sym::clone;
	}
	!matches!(
		(std, name.as_str()),
		(
			Some(Std::Option),
			"unwrap" | "expect" | "is_some" | "is_none" | "as_ref" | "copied" | "cloned" | "iter"
		) | (
			Some(Std::Result),
			"unwrap" | "expect" | "unwrap_err" | "is_ok" | "is_err"
		) | (
			Some(Std::LocalKey),
			"with" | "try_with" | "initialize_with" | "with_borrow_mut"
		)
	)
}

/// An integer or a `NonZero` of one, which parse alike.
fn int<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
	match ty.kind() {
		ty::Int(_) | ty::Uint(_) => true,
		ty::Adt(adt, _) => Std::of(tcx, adt.did()) == Some(Std::NonZero),
		_ => false,
	}
}

/// The key `def_id` makes the value of, if it is what `thread_local!` declares in a key's
/// own initializer to: a `const { .. }` key's `const`, or a lazy key's function.
pub fn local_init(tcx: TyCtxt<'_>, def_id: DefId) -> Option<DefId> {
	let key = tcx.opt_parent(def_id)?;
	let is_key = tcx.def_kind(key) == DefKind::Const
		&& matches!(tcx.type_of(key).instantiate_identity().skip_norm_wip().kind(),
			ty::Adt(adt, _) if Std::of(tcx, adt.did()) == Some(Std::LocalKey));
	is_key.then_some(key)
}

pub struct Call<'tcx> {
	/// The function as written, before resolving a trait method to its impl.
	pub def_id: DefId,
	/// Its generic arguments: the impl's or trait's, then the function's.
	pub args: GenericArgsRef<'tcx>,
	pub span: Span,
}

/// What the mapping of `owner::name` may discard of the operands it takes by value, rather
/// than move into its result or a closure, as it returns: the types, by their index among
/// the call's type arguments, whose droppers its runtime function takes after its own
/// arguments, as `unwrap_or` drops its fallback, or a function it always calls once
/// through `FnOnce`, which that call consumes. `None` for any other mapping, which is
/// refused an operand that drops. Iterators refuse their own.
fn discards(owner: &str, name: &str) -> Option<&'static [usize]> {
	Some(match (owner, name) {
		("Option", "unwrap" | "expect")
		| ("Vec", "push" | "into_boxed_slice")
		| ("slice", "into_vec")
		| ("Box", "new" | "pin")
		| ("mem", "replace" | "take" | "swap" | "forget")
		| ("ptr", "write" | "write_unaligned" | "write_volatile" | "replace")
		| ("MaybeUninit", "new" | "write" | "assume_init")
		| ("Box", "into_raw" | "leak" | "from_raw")
		| ("hint", "must_use")
		| ("From", "from")
		| ("Into", "into")
		// What a panic would drop, which aborts.
		| ("Result", "unwrap" | "expect" | "unwrap_err")
		| ("Vec", "insert")
		| ("RangeInclusive", "new")
		// Each element moves into the closure.
		| ("array", "map") => &[],
		// `Vec<T>` drops each element `retain` rejects, `dedup` removes, and `truncate` and
		// `clear` cut.
		("Vec", "retain" | "retain_mut" | "dedup" | "dedup_by" | "dedup_by_key" | "truncate" | "clear") => &[0],
		// A slice's `fill`, `fill_with`, and `clone_from_slice` drop each element they write
		// over, and `fill` its value if there is no slot for it.
		("slice", "fill" | "fill_with" | "clone_from_slice") => &[0],
		// `get_or_init::<T, F>` drops its function uncalled if the cell is set.
		("OnceCell" | "OnceLock", "get_or_init" | "get_or_try_init") => &[1],
		("Ref", "map") => &[],
		("Once", "call_once" | "call_once_force") => &[0],
		("Arc" | "Rc", "new_cyclic") => &[1],
		// `pop_if::<F>` drops its test uncalled if there is no element.
		("Vec", "pop_if") => &[2],
		// `is_sorted_by::<F>` drops its comparator before the last item, as `all` takes it.
		("Iterator", "is_sorted_by") => &[1],
		("LocalKey", "with" | "try_with" | "initialize_with" | "with_borrow_mut") => &[1],
		_ => return None,
	})
}

/// Whether the mapping of `owner::name` tells an `Option`'s `None` from its `Some` where
/// the `None` is `rt.None`, as the runtime's functions that take `None` to be `undefined`
/// do not.
fn takes_none(owner: &str, name: &str) -> bool {
	none_last(owner, name)
		|| matches!(
			(owner, name),
			(
				"Option",
				"is_some" | "is_none" | "as_ref" | "copied" | "cloned" | "iter" | "iter_mut"
			) | (
				"Iterator",
				"next" | "nth" | "last" | "find" | "max" | "min" | "max_by_key" | "min_by_key"
			) | (
				"Iterator",
				"max_by" | "min_by" | "reduce" | "find_map" | "filter_map" | "map_while"
			) | ("Peekable", "peek" | "next_if" | "next_if_eq")
				| (
					"OnceCell" | "OnceLock",
					"get" | "get_mut" | "set" | "get_or_init" | "get_or_try_init" | "take" | "into_inner"
				)
				| ("Arc" | "Rc", "into_inner")
				| ("DoubleEndedIterator", "next_back" | "nth_back" | "rfind")
				| ("dyn Any", "downcast_ref" | "downcast_mut")
				| ("ptr", "as_ref" | "as_mut")
				| ("from_fn", "from_fn")
				| ("successors", "successors")
		)
}

/// Whether the runtime function `owner::name` maps to takes the `None` of the `Option` it
/// makes or reads, where that is `rt.None`, last: after what drops its operands.
pub(super) fn none_last(owner: &str, name: &str) -> bool {
	matches!(
		(owner, name),
		("Option", "unwrap" | "expect")
			| ("slice", "first" | "last" | "get" | "first_mut" | "last_mut" | "get_mut")
			| ("Vec", "pop" | "pop_if")
			| (
				"VecDeque",
				"pop_front"
					| "pop_back"
					| "front"
					| "back"
					| "front_mut"
					| "back_mut"
					| "get"
					| "get_mut"
					| "remove"
					| "swap_remove_front"
					| "swap_remove_back"
			)
			| (
				"LinkedList",
				"pop_front" | "pop_back" | "front" | "back" | "front_mut" | "back_mut"
			)
			| ("BinaryHeap", "pop" | "peek")
			| ("HashSet", "get" | "take" | "replace")
			| (
				"BTreeSet",
				"first" | "last" | "pop_first" | "pop_last" | "get" | "take" | "replace"
			)
			| ("HashMap" | "BTreeMap", "get" | "get_mut" | "insert" | "remove")
	)
}

/// The type of the first `Some` whose `Option`'s `None` is `rt.None`, of those in the
/// signature and bounds of the function `call` calls, over its type arguments: what the
/// mapping of a std function would make or read with `undefined` for `None`.
pub(super) fn sentinel_option<'tcx>(t: &Translator<'_, 'tcx>, call: &Call<'tcx>) -> Option<Ty<'tcx>> {
	let tcx = t.tcx;
	let sig = tcx.fn_sig(call.def_id).instantiate_identity().skip_norm_wip();
	let sig = tcx.instantiate_bound_regions_with_erased(sig);
	let bounds = tcx.param_env(call.def_id).caller_bounds();
	let bound_args = bounds.flat_map(|clause| {
		let trait_args = clause.as_trait_clause().map(|c| c.skip_binder().trait_ref.args);
		let projection = clause.as_projection_clause().map(|c| c.skip_binder());
		let projection_args = projection.map(|p| p.projection_term.args);
		let term = projection.and_then(|p| p.term.as_type());
		(trait_args.into_iter().chain(projection_args))
			.flat_map(|args| args.types().collect::<Vec<_>>())
			.chain(term)
	});
	let env = t.typing_env_pub();
	sig.inputs_and_output
		.iter()
		.chain(bound_args)
		.flat_map(|ty| ty.walk().filter_map(|arg| arg.as_type()).collect::<Vec<_>>())
		.filter_map(|ty| match ty.kind() {
			ty::Adt(adt, args) if Std::of(tcx, adt.did()) == Some(Std::Option) => Some(args.type_at(0)),
			_ => None,
		})
		.map(|inner| {
			let inner = ty::EarlyBinder::bind(tcx, inner)
				.instantiate(tcx, call.args)
				.skip_norm_wip();
			let inner = tcx.erase_and_anonymize_regions(inner);
			tcx.try_normalize_erasing_regions(env, ty::Unnormalized::new_wip(inner))
				.unwrap_or(inner)
		})
		.find(|&inner| t.erases_to(inner).undefined)
}

/// What a std function does with the closures it calls that own a value that drops.
#[derive(Default)]
struct Closures<'tcx> {
	/// The first it would neither drop where std's does nor keep: refused.
	leaked: Option<Ty<'tcx>>,
	/// Those it drops as it returns, having kept nothing of them, in the order
	/// their parameters are declared.
	spent: Vec<Expr>,
}

/// What an argument of type `ty` that std calls holds, through any references and boxes,
/// whether a reference lends it, and `value`, the argument, read as that: a `&mut` to a
/// function JS copies is its place, so std calls the function it holds.
pub fn called_through<'tcx>(
	t: &Translator<'_, 'tcx>,
	ty: Ty<'tcx>,
	mut value: Option<Expr>,
) -> (Ty<'tcx>, bool, Option<Expr>) {
	let (mut owned, mut lent) = (ty, false);
	loop {
		owned = match *owned.kind() {
			ty::Ref(_, inner, _) => {
				if t.place_pointee(owned).is_some() {
					value = value.map(|f| f.member("v"));
				}
				lent = true;
				inner
			}
			ty::Adt(adt, args) if Std::of(t.tcx, adt.did()) == Some(Std::Box) => args.type_at(0),
			_ => break,
		};
	}
	(owned, lent, value)
}

/// The closures `call` calls that own a value that drops, by what its mapping does with
/// each: kept by the iterator it returns, if `iter::holds_closure`; dropped by the mapping,
/// if `released`, as `iter::releases` says; dropped as it returns, if `Fn` or `FnMut` call
/// it and nothing else it takes by value drops, or the mapping discards none of it, as then
/// nothing it drops is ordered against it; dropped where `discarded` says, by the type's
/// index among the call's type arguments; or else leaked. A closure that `Fn` or `FnMut`
/// call, given to one that calls it through `FnOnce`, becomes `rt.once` of it, which drops
/// it after the call, as rustc's shim does. A reference to one is the closure in JS, so an
/// iterator, which drops what it holds, keeps `rt.lent` of it instead.
fn called_closures<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	call: &Call<'tcx>,
	discarded: Option<&[usize]>,
	released: bool,
	args: &mut [Expr],
) -> Closures<'tcx> {
	let tcx = b.t.tcx;
	let mut closures = Closures::default();
	let sig = tcx.fn_sig(call.def_id).instantiate_identity().skip_norm_wip();
	let sig = tcx.instantiate_bound_regions_with_erased(sig);
	let inputs = sig.inputs().to_vec();
	let output = sig.output();
	for (i, &input) in inputs.iter().enumerate() {
		let ty::Param(param) = *input.kind() else {
			continue;
		};
		let ty = call.args[param.index as usize].expect_ty();
		let Some(bound) = called_by(tcx, call.def_id, input) else {
			continue;
		};
		let value = (i < args.len()).then(|| std::mem::replace(&mut args[i], Expr::undefined()));
		let (owned, lent, value) = called_through(b.t, ty, value);
		if let Some(value) = value {
			args[i] = value;
		}
		let Some(called) = b.t.owned_fn(owned) else {
			continue;
		};
		if !b.t.drops(owned) {
			continue;
		}
		let mentions = |t: Ty<'tcx>| t.walk().any(|a| a.as_type() == Some(input));
		if mentions(output) {
			// An `rt.Iter` drops the closure it holds, and an `rt.PollFn` one it owns.
			let holder = match output.kind() {
				ty::Adt(adt, args) if args.types().any(|a| a == input) => Some(adt.did()),
				_ => None,
			};
			let iter = holder.is_some_and(|did| super::iter::holds_closure(tcx, did));
			if lent && iter && i < args.len() {
				let f = std::mem::replace(&mut args[i], Expr::undefined());
				args[i] = Expr::rt("lent", vec![f]);
			} else if !lent && !iter && holder.and_then(|did| Std::of(tcx, did)) != Some(Std::PollFn) {
				closures.leaked = closures.leaked.or(Some(ty));
			}
			continue;
		}
		// Its owner drops a borrowed closure, and the mapping a released one.
		if lent || released {
			continue;
		}
		let index = call.args[..param.index as usize]
			.iter()
			.filter(|a| a.as_type().is_some())
			.count();
		let discards = discarded.is_some_and(|d| d.contains(&index));
		let kept = inputs.iter().enumerate().any(|(j, &t)| j != i && mentions(t));
		if bound != ty::ClosureKind::FnOnce && !kept && !discards && i < args.len() {
			let dropping = match discarded {
				Some([]) => None,
				_ => dropping_operand(b, call),
			};
			match dropping {
				None => closures.spent.push(args[i].clone()),
				Some(_) => closures.leaked = closures.leaked.or(Some(ty)),
			}
			continue;
		}
		if !discards {
			closures.leaked = closures.leaked.or(Some(ty));
			continue;
		}
		if bound == ty::ClosureKind::FnOnce && called != ty::ClosureKind::FnOnce && i < args.len() {
			let f = std::mem::replace(&mut args[i], Expr::undefined());
			args[i] = Expr::rt("once", vec![f]);
		}
	}
	closures
}

/// The trait `def_id` calls its parameter of type `input` by: `Fn`, then `FnMut`,
/// whichever it is bound by, or else `FnOnce`, or `None` if no function trait bounds it.
fn called_by<'tcx>(tcx: TyCtxt<'tcx>, def_id: DefId, input: Ty<'tcx>) -> Option<ty::ClosureKind> {
	tcx.param_env(def_id)
		.caller_bounds()
		.filter_map(|p| p.as_trait_clause())
		.filter(|t| t.self_ty().skip_binder() == input)
		.filter_map(|t| tcx.fn_trait_kind_from_def_id(t.def_id()))
		.min_by_key(|kind| match kind {
			ty::ClosureKind::Fn => 0,
			ty::ClosureKind::FnMut => 1,
			ty::ClosureKind::FnOnce => 2,
		})
}

/// The first type `call` takes by value that drops, but for a closure a function trait
/// bounds, which `called_closures` decides.
fn dropping_operand<'tcx>(b: &mut Body<'_, '_, 'tcx>, call: &Call<'tcx>) -> Option<Ty<'tcx>> {
	let tcx = b.t.tcx;
	let generic = tcx.fn_sig(call.def_id).instantiate_identity().skip_norm_wip();
	let generic = tcx.instantiate_bound_regions_with_erased(generic).inputs().to_vec();
	b.t.sig(call.def_id, call.args)
		.inputs()
		.iter()
		.copied()
		.enumerate()
		.find(|&(i, ty)| {
			let called = b.t.owned_fn(ty).is_some() && called_by(tcx, call.def_id, generic[i]).is_some();
			!called && b.t.drops(ty)
		})
		.map(|(_, ty)| ty)
}

/// `owner::name` for a standard library function.
pub fn key(tcx: TyCtxt<'_>, def_id: DefId) -> Option<(String, String)> {
	let name = tcx.item_name(def_id).to_string();
	if let Some(trait_id) = tcx.trait_of_assoc(def_id) {
		return Some((tcx.item_name(trait_id).to_string(), name));
	}
	if let Some(imp) = tcx.inherent_impl_of_assoc(def_id) {
		let self_ty = tcx.type_of(imp).instantiate_identity().skip_norm_wip();
		let owner = match self_ty.kind() {
			ty::Adt(adt, _) => tcx.item_name(adt.did()).to_string(),
			ty::Slice(_) => "slice".into(),
			ty::Array(..) => "array".into(),
			ty::Str => "str".into(),
			ty::Bool => "bool".into(),
			ty::Char => "char".into(),
			ty::Int(_) | ty::Uint(_) | ty::Float(_) => self_ty.to_string(),
			// `<*const T>::read` is `ptr::read`.
			ty::RawPtr(..) => "ptr".into(),
			ty::Dynamic(preds, ..) => format!("dyn {}", tcx.item_name(preds.principal_def_id()?)),
			_ => return None,
		};
		return Some((owner, name));
	}
	let parent = tcx.parent(def_id);
	let owner = tcx
		.opt_item_name(parent)
		.map_or_else(|| tcx.crate_name(def_id.krate).to_string(), |n| n.to_string());
	Some((owner, name))
}

pub fn call<'tcx>(b: &mut Body<'_, '_, 'tcx>, call: &Call<'tcx>, args: Vec<Expr>, out: &mut Vec<Stmt>) -> Option<Expr> {
	let tcx = b.t.tcx;
	let args = match super::stock::call(b, call, args, out) {
		Ok(e) => return e,
		Err(args) => args,
	};
	if !is_std(tcx, call.def_id) {
		return None;
	}
	let (owner, name) = key(tcx, call.def_id)?;
	if !takes_none(&owner, &name)
		&& let Some(inner) = sentinel_option(b.t, call)
	{
		return Some(b.err(
			call.span,
			format!("`{owner}::{name}` of `Option<{inner}>`, whose `None` is `rt.None`, which is not supported yet"),
		));
	}
	let discarded = discards(&owner, &name);
	let mut args = args;
	let released = super::iter::releases(&owner, &name);
	let closures = called_closures(b, call, discarded, released, &mut args);
	// What std lends of the `[u8; N]`s it holds, its closures are given and it returns as
	// slices of them.
	let lent = match super::lend::plan(b.t, call.def_id, call.args) {
		super::lend::Plan::Lend(lent) => Some(lent),
		_ => None,
	};
	for closure in lent.iter().flat_map(|lent| &lent.closures) {
		let holds = closure.held || b.t.drops(closure.ty);
		if let Some(value) = args.get_mut(closure.index) {
			let f = std::mem::replace(value, Expr::undefined());
			*value = super::lend::lend_closure(b, f, &closure.params, holds, call.span, out);
		}
	}
	let mut e = mapping(b, call, owner.clone(), name.clone(), discarded, args, out)?;
	if let Some((generic, ty)) = lent.and_then(|lent| lent.output) {
		e = super::lend::lend(b, generic, ty, e, call.span);
	}
	if let Some(ty) = closures.leaked {
		return Some(b.err(
			call.span,
			format!(
				"passing `{ty}`, a closure that owns a value that drops, to `{owner}::{name}`, which is not supported yet"
			),
		));
	}
	// The last parameter drops first, as a function's do.
	for f in closures.spent.into_iter().rev() {
		e = Expr::rt("dropAfter", vec![f, Expr::Lambda(Vec::new(), Box::new(e))]);
	}
	Some(e)
}

/// The mapping of a call of std's `owner::name`, which may discard what `discarded` says.
fn mapping<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	call: &Call<'tcx>,
	owner: String,
	name: String,
	discarded: Option<&'static [usize]>,
	args: Vec<Expr>,
	out: &mut Vec<Stmt>,
) -> Option<Expr> {
	let tcx = b.t.tcx;
	let span = call.span;
	let gargs = call.args;
	let args = match super::task::call(b, call, &owner, &name, args, out) {
		Ok(e) => return e,
		Err(args) => args,
	};
	let args = match super::dynamic::call(b, call, &owner, &name, args, out) {
		Ok(e) => return e,
		Err(args) => args,
	};
	let args = match super::pointer::call(b, call, &owner, &name, args, out) {
		Ok(e) => return e,
		Err(args) => args,
	};
	let args = match super::time::call(b, call, &owner, &name, args) {
		Ok(e) => return e,
		Err(args) => args,
	};
	let mut args = match super::collections::call(b, call, &owner, &name, args, out) {
		Ok(e) => return e,
		Err(args) => args,
	};
	let dropping = match discarded {
		Some(_) => None,
		None => dropping_operand(b, call),
	};

	// Methods on integers.
	if let Some(int) = int_named(&owner) {
		return int_method(b, call, int, &name, args);
	}
	let float = match owner.as_str() {
		"f32" => Some(super::float::Float::F32),
		"f64" => Some(super::float::Float::F64),
		_ => None,
	};
	if let Some(float) = float {
		return super::float::method(float, &name, args);
	}

	let self_ty = || gargs.types().next();
	let is_u8 = |t: Ty<'tcx>| matches!(t.kind(), ty::Uint(ty::UintTy::U8));
	let byte_slice = |t: Ty<'tcx>| matches!(t.kind(), ty::Slice(e) if is_u8(*e));
	let range = matches!(inherent_std(tcx, call.def_id), Some(Std::Range | Std::RangeInclusive));

	let e = match (owner.as_str(), name.as_str()) {
		// Conversions.
		("From", "from") => {
			// A `NonZero` converts as its integer.
			let (to, from) = (targ(gargs, 0), unwrapped(tcx, targ(gargs, 1)));
			let mut values = args;
			let value = values.pop()?;
			// One between strings, such as into a `Cow<str>` or out of one, keeps the text, and
			// `Some` of a value is the value.
			let some = matches!(to.kind(), ty::Adt(adt, args)
				if Std::of(tcx, adt.did()) == Some(Std::Option) && args.type_at(0) == targ(gargs, 1));
			if to == from || some {
				return Some(value);
			}
			// A `&mut str` is the `rt.Place` of its text.
			if string(tcx, to) && string(tcx, from.peel_refs()) {
				return Some(b.t.pointee_value(targ(gargs, 1), value));
			}
			// A `Vec` is owned, and a reference to a slice, an array, or a `Vec` borrows its slice.
			if let Some(elem) = cow_slice(tcx, to) {
				let owned = b.t.std_of(targ(gargs, 1)) == Some(Std::Vec);
				let value = match b.t.std_of(from) == Some(Std::Vec) && !owned {
					true => vec_view(value, elem),
					false => value,
				};
				return Some(cow_made(elem, rustc_abi::VariantIdx::from_u32(owned.into()), value));
			}
			if let Some(elem) = cow_slice(tcx, from)
				&& b.t.std_of(to) == Some(Std::Vec)
			{
				return Some(cow_cloned(b, "IntoOwned", value, elem, span));
			}
			// A boxed slice clones what it borrows, moves an array's or a `Vec`'s elements, and
			// owns the `Vec` a `Cow` gives up.
			if let Some(elem) = box_slice(tcx, to) {
				return match targ(gargs, 1).kind() {
					ty::Ref(_, s, _) if s.is_slice() => slice_boxed(b, value, elem, span),
					ty::Array(..) => Some(array_boxed(value, elem)),
					_ if b.t.std_of(from) == Some(Std::Vec) => Some(vec_boxed(value, elem)),
					_ if cow_slice(tcx, from).is_some() => {
						Some(vec_boxed(cow_cloned(b, "IntoOwned", value, elem, span), elem))
					}
					_ => None,
				};
			}
			if let Some(elem) = box_slice(tcx, from)
				&& b.t.std_of(to) == Some(Std::Vec)
			{
				return Some(boxed_vec(value, elem));
			}
			// A heap's `Vec` is its elements.
			if let ty::Adt(adt, args) = from.kind()
				&& Std::of(tcx, adt.did()) == Some(Std::BinaryHeap)
			{
				return Some(super::collections::heap_vec(value.member("d"), is_u8(args.type_at(0))));
			}
			// Each `From` between integers, `char`, and floats, or from `bool`, is as `as`.
			let scalar = |t: Ty<'tcx>| Int::of(t).is_some() || t.is_char() || super::float::is(t);
			if !scalar(to) || !(scalar(from) || from.is_bool()) {
				return None;
			}
			let operand = Operand {
				expr: value,
				value: None,
			};
			num::cast(b, operand, from, to, span)
		}
		("Into", "into") => {
			let (from, to) = (targ(gargs, 0), targ(gargs, 1));
			let mut values = args;
			let value = values.pop()?;
			b.convert(value, from, to, span, out)
		}
		// A scalar value, but not a surrogate.
		("char", "from_u32") => Expr::rt("charFromU32", args),
		("TryFrom", "try_from") | ("TryInto", "try_into") => {
			let (to, from) = match owner.as_str() {
				"TryFrom" => (targ(gargs, 0), targ(gargs, 1)),
				_ => (targ(gargs, 1), targ(gargs, 0)),
			};
			// Where `From` holds, std's blanket impl converts by it and can't fail: its error
			// is `Infallible`, which is `!`.
			if let ty::Adt(_, result) = b.t.sig(call.def_id, call.args).output().kind()
				&& result.type_at(1).is_never()
			{
				let mut values = args;
				let value = b.convert(values.pop()?, from, to, span, out);
				return Some(Expr::rt("Ok", vec![value]));
			}
			let (Some(int), Some(_)) = (Int::of(to), Int::of(from)) else {
				return None;
			};
			let values = args;
			int.method("tryFrom", values)
		}

		// A `Wrapping`'s or a `Saturating`'s operators, which wrap or saturate as its integer's
		// `wrapping_*` or `saturating_*` do.
		(_, _)
			if let Some(op) = operator(&owner)
				&& let Some((overflow, int)) = overflowing(tcx, targ(gargs, 0).peel_refs()) =>
		{
			let [l, r] = <[Expr; 2]>::try_from(args).ok()?;
			// A constant shift amount, which masks to a constant.
			let value = match &r {
				Expr::Lit(l) => l.parse().ok(),
				_ => None,
			};
			num::overflowing(
				b,
				overflow,
				op,
				Operand { expr: l, value: None },
				Operand { expr: r, value },
				int,
				span,
			)
		}
		("Neg", "neg") if let Some((overflow, int)) = overflowing(tcx, targ(gargs, 0).peel_refs()) => {
			let int = Int::of(int)?;
			int.method(&overflow.method("Sub"), vec![int.lit(0), args.pop()?])
		}
		("Not", "not") if let Some((_, int)) = overflowing(tcx, targ(gargs, 0).peel_refs()) => {
			num::unary(b, rustc_middle::mir::UnOp::Not, args.pop()?, int, span)
		}
		// Operators on references to primitives.
		(_, _)
			if let Some(op) = operator(&owner)
				&& primitive(targ(gargs, 0).peel_refs()) =>
		{
			let (lty, rty) = (targ(gargs, 0), targ(gargs, 1));
			let [l, r] = <[Expr; 2]>::try_from(args).ok()?;
			num::binary(
				b,
				op,
				Operand { expr: l, value: None },
				Operand { expr: r, value: None },
				lty,
				rty,
				span,
			)
		}
		// Every value of a type holding nothing is equal, and no `!` is ever compared.
		("PartialEq", "eq" | "ne")
			if nothing(tcx, unwrapped(tcx, targ(gargs, 0))) || unwrapped(tcx, targ(gargs, 0)).is_never() =>
		{
			Expr::lit(if name == "eq" { "true" } else { "false" })
		}
		("PartialEq", "eq" | "ne") | ("PartialOrd", "lt" | "le" | "gt" | "ge")
			if primitive(unwrapped(tcx, targ(gargs, 0))) =>
		{
			let js = match name.as_str() {
				"eq" => "===",
				"ne" => "!==",
				"lt" => "<",
				"le" => "<=",
				"gt" => ">",
				_ => ">=",
			};
			let values = args;
			let [l, r] = <[Expr; 2]>::try_from(values).ok()?;
			match Int::of(unwrapped(tcx, targ(gargs, 0))) {
				Some(int) => int.compare(js, l, r),
				None => l.binary(js, r),
			}
		}
		("Ord", "max" | "min") if let Some(int) = Int::of(unwrapped(tcx, targ(gargs, 0))) => {
			let values = args;
			match int.big() {
				true => int.method(name.as_str(), values),
				false => Expr::rt(if name == "max" { "max" } else { "min" }, values),
			}
		}
		("Ord", "clamp") if let Some(int) = Int::of(targ(gargs, 0)) => match int.big() {
			true => int.method("clamp", args),
			false => Expr::rt("clamp", args),
		},
		("Clone", "clone") => {
			let ty = targ(gargs, 0);
			let values = args;
			let value = values.into_iter().next()?;
			match ty.kind() {
				ty::Adt(adt, args) if Std::of(tcx, adt.did()) == Some(Std::Vec) && is_u8(args.type_at(0)) => {
					Expr::rt("byteVecFrom", vec![Expr::rt("byteVecView", vec![value])])
				}
				ty::Adt(..) if string(tcx, ty) => value,
				ty::Adt(adt, args) if Std::of(tcx, adt.did()) == Some(Std::Vec) => {
					return vec_clone(b, value, args.type_at(0), span);
				}
				_ if let Some(elem) = cow_slice(tcx, ty) => cow_cloned(b, "Clone", value, elem, span),
				ty::Tuple(tys) if !clone_copies(b.t, b.t.typing_env_pub(), ty) => {
					let clones = tys.iter().map(|t| super::collections::clone_fn(b, t, span)).collect();
					Expr::rt("tupleClone", vec![value, Expr::Array(clones)])
				}
				_ if let Some(elem) = box_slice(tcx, ty) => return slice_boxed(b, value, elem, span),
				// A box is what it holds, cloned.
				ty::Adt(adt, args) if Std::of(tcx, adt.did()) == Some(Std::Box) => {
					let inner = args.type_at(0);
					if !inner.is_sized(tcx, b.t.typing_env_pub()) {
						return None;
					}
					match super::collections::clone_fn(b, inner, span) {
						Expr::Lit(l) if l == "undefined" => value,
						clone => clone.call(vec![value]),
					}
				}
				ty::Adt(adt, _) if Std::of(tcx, adt.did()) == Some(Std::FromUtf8Error) => {
					Expr::rt("fromUtf8ErrorClone", vec![value])
				}
				// Another that holds nothing.
				ty::Adt(adt, _) if empty_iter(tcx, adt.did()) => b.zero_sized_std(ty, Vec::new(), span).ok()?,
				// Through the reference, a slice of the bytes.
				_ if super::byte_array(ty) => Expr::rt("byteSliceToArray", vec![value]),
				_ if b.t.std_iter(ty) => return super::iter::clone(b, ty, value, span),
				ty::Closure(def_id, _) | ty::CoroutineClosure(def_id, _)
					if !clone_shares(b.t, b.t.typing_env_pub(), ty) =>
				{
					b.t.clone_closure(*def_id);
					Expr::rt("closureClone", vec![value])
				}
				_ if clone_shares(b.t, b.t.typing_env_pub(), ty) => value,
				_ if clone_copies(b.t, b.t.typing_env_pub(), ty) => b.t.copied(value, ty, span),
				_ => return None,
			}
		}
		("Default", "default") => default_value(b, targ(gargs, 0), span, out)?,

		// Options, whose other combinators lower from core's MIR.
		("Option", "unwrap" | "expect") => rt2(args, "option", &name),
		("Option", "is_some") => args.pop()?.binary("!==", b.t.none(targ(gargs, 0))),
		("Option", "is_none") => args.pop()?.binary("===", b.t.none(targ(gargs, 0))),
		("Option", "as_ref" | "as_mut") => args.pop()?,
		// A box is what it holds.
		("Option", "as_deref_mut") if targ(gargs, 0).is_box() => args.pop()?,
		// `Option<&T>` or `Option<&mut T>` of the `T` it points at, through `T`'s `Clone`
		// where that does more than copy, as std's `cloned` does.
		("Option", "copied" | "cloned") => {
			let sig = b.t.sig(call.def_id, call.args);
			let (option, inner) = (sig.output(), targ(gargs, 0));
			let ty::Adt(_, of) = sig.inputs()[0].kind() else {
				return None;
			};
			let reference = of.type_at(0);
			let value = args.pop()?;
			let copies = name == "copied" || clone_copies(b.t, b.t.typing_env_pub(), inner);
			let place = b.t.place_pointee(reference).is_some();
			if copies && !place {
				return Some(b.t.copied(value, option, span));
			}
			let value = match value.is_path() {
				true => value,
				false => b.spill(value, "option", out),
			};
			let some = b.t.pointee_value(reference, value.clone());
			let some = match copies {
				true => b.t.copied(some, inner, span),
				false => {
					let clone = super::collections::trait_fn(tcx, "Clone", "clone");
					let mut body = Vec::new();
					let some = b.call_def(clone, tcx.mk_args(&[inner.into()]), vec![some], span, &mut body);
					match body.is_empty() {
						true => some,
						// Only once it is `Some`.
						false => {
							body.push(StmtKind::Return(Some(some.typed())).at(None));
							Expr::Arrow(Vec::new(), b.t.ret_ty(inner, span), body).call(Vec::new())
						}
					}
				}
			};
			let none = value.binary("===", b.t.none(reference));
			Expr::Cond(Box::new(none), Box::new(b.t.none(inner)), Box::new(some))
		}

		// Results, whose other combinators lower from core's MIR.
		("Result", "is_ok") => args.pop()?.member("ok"),
		("Result", "is_err") => args.pop()?.member("ok").not(),
		// Their panics show the other side with its `Debug`.
		("Result", "unwrap" | "expect" | "unwrap_err") => {
			let (method, shown) = match name.as_str() {
				"unwrap" => ("unwrap", targ(gargs, 1)),
				"expect" => ("expect", targ(gargs, 1)),
				_ => ("unwrapErr", targ(gargs, 0)),
			};
			let mut values = args;
			values.push(b.fmt_fn(fmt::Trait::Debug, shown, span));
			Expr::rt_of("result", method).call(values)
		}

		("ControlFlow", "is_continue" | "is_break") => Flow::test(b.t, gargs, args.pop()?, name == "is_continue"),

		// Ranges, which are their own iterators.
		// An integer range wraps within its type's bounds where std's `nth` does.
		("RangeInclusive", "new") if range => {
			let mut values = args;
			values.extend(self_ty().and_then(|t| range_step(tcx, t, true)));
			Expr::New(Box::new(Expr::Rt("RangeInclusive".into())), values)
		}
		("Range" | "RangeInclusive", "start" | "end") | ("RangeInclusive", "into_inner") if range => {
			let of = range_of(inherent_std(tcx, call.def_id)?)?;
			Expr::rt_of(of, &super::camel(&name)).call(args)
		}
		// Of bounds JS compares as Rust does, by `<` and `<=`, against an item of their type.
		("Range" | "RangeInclusive", "is_empty") | (_, "contains")
			if let Some(of) = inherent_std(tcx, call.def_id).and_then(range_of)
				&& let Some(bound) = compared(tcx, targ(gargs, 0))
				&& (name == "is_empty" || compared(tcx, targ(gargs, 1)) == Some(bound)) =>
		{
			Expr::rt_of(of, &super::camel(&name)).call(args)
		}
		("RangeBounds", "contains")
			if compared(tcx, targ(gargs, 1)).is_some_and(|bound| compared(tcx, targ(gargs, 2)) == Some(bound)) =>
		{
			let range = targ(gargs, 0).peel_refs();
			match b.t.std_of(range) {
				Some(Std::RangeFull) => Expr::lit("true"),
				Some(std) => Expr::rt_of(range_of(std)?, "contains").call(args),
				None if super::collections::plain_bounds(b.t, range) => Expr::rt("boundsContains", args),
				None => return None,
			}
		}

		// A `NonZero` is its integer.
		("NonZero", "get") => args.pop()?,
		("NonZero", "new") => Expr::rt("nonZero", args),
		// Its integer's, of each width's own impl, which has no type argument to read the
		// integer from, but for the bits it always has.
		("NonZero", _) => {
			let int = Int::of(unwrapped(tcx, impl_self(tcx, call.def_id)?))?;
			match name.as_str() {
				"highest_one" => {
					let zeros = int.method("leadingZeros", args);
					Expr::lit((int.bits - 1).to_string()).binary("-", zeros)
				}
				"lowest_one" => int.method("trailingZeros", args),
				_ => return int_method(b, call, int, &name, args),
			}
		}
		// A `Wrapping`'s and a `Saturating`'s are its integer's, of each width's own impl, but
		// that each overflows as its operators do.
		(_, _) if let Some((overflow, int)) = impl_self(tcx, call.def_id).and_then(|ty| overflowing(tcx, ty)) => {
			let name = match name.as_str() {
				"pow" | "abs" | "next_power_of_two" => format!("{}_{name}", overflow.prefix()),
				_ => name,
			};
			return int_method(b, call, Int::of(int)?, &name, args);
		}
		// `Bound<&T>::cloned::<T>`: what it holds, cloned.
		("Bound", "cloned") => {
			let value = args.pop()?;
			let bound = b.t.sig(call.def_id, gargs).output();
			match super::collections::clone_fn(b, targ(gargs, 0), span) {
				Expr::Lit(l) if l == "undefined" => b.t.copied(value, bound, span),
				clone => Expr::rt("boundMap", vec![value, clone]),
			}
		}
		("Bound", "as_ref") => args.pop()?,
		("RangeBounds", "start_bound" | "end_bound") => {
			let start = name == "start_bound";
			let range = targ(gargs, 0).peel_refs();
			let ts = b.t.ty(b.t.sig(call.def_id, gargs).output(), span);
			let r = args.pop()?;
			let (variant, field) = match (range.kind(), b.t.std_of(range)) {
				// `(Bound<T>, Bound<T>)`, whose bounds are its own.
				(ty::Tuple(_), _) => return Some(r.index(Expr::lit(if start { "0" } else { "1" }))),
				(_, Some(Std::Range)) if start => ("Included", Some("start")),
				(_, Some(Std::Range)) => ("Excluded", Some("end")),
				(_, Some(Std::RangeFrom)) if start => ("Included", Some("start")),
				(_, Some(Std::RangeTo)) if !start => ("Excluded", Some("end")),
				(_, Some(Std::RangeToInclusive)) if !start => ("Included", Some("end")),
				(_, Some(Std::RangeFrom | Std::RangeTo | Std::RangeToInclusive | Std::RangeFull)) => {
					("Unbounded", None)
				}
				// Exhausted, it ends before its end.
				(_, Some(Std::RangeInclusive)) => {
					let f = if start {
						"rangeInclusiveStartBound"
					} else {
						"rangeInclusiveEndBound"
					};
					return Some(Expr::As(Box::new(Expr::rt(f, vec![r])), ts));
				}
				_ => return None,
			};
			tagged(variant, field.map(|f| r.member(f)), ts)
		}

		// Slices, whose byte functions take an `rt.ByteSlice`.
		("slice", "len") => super::mir::slice_member(args.pop()?, "length"),
		("slice", "is_empty") => super::mir::slice_member(args.pop()?, "length").binary("===", Expr::lit("0")),
		("slice", "split_first" | "split_at" | "split_at_checked" | "first" | "get")
			if name != "get" || Int::of(targ(gargs, 1)).is_some() =>
		{
			let of = match is_u8(targ(gargs, 0)) {
				true => "byteSlice",
				false => "slice",
			};
			rt2(args, of, &super::camel(&name))
		}
		// An element JS copies is lent as its place, and an object as itself.
		("slice", "first_mut" | "last_mut" | "get_mut") if name != "get_mut" || Int::of(targ(gargs, 1)).is_some() => {
			let elem = targ(gargs, 0);
			let mut values = args;
			values[0] = elements(values[0].clone(), Ty::new_slice(tcx, elem));
			let name = match b.t.needs_place(elem) {
				true => name.as_str(),
				false => name.strip_suffix("_mut")?,
			};
			rt2(values, "slice", &super::camel(name))
		}
		("slice", "windows" | "chunks" | "chunks_exact" | "rchunks" | "rchunks_exact") => match is_u8(targ(gargs, 0)) {
			true => rt2(args, "byteSlice", &super::camel(&name)),
			false => rt2(args, "slice", &super::camel(&name)),
		},
		// A part of bytes is a view already, and one of other values the runtime's view.
		("slice", "chunks_mut" | "chunks_exact_mut" | "rchunks_mut" | "rchunks_exact_mut") => {
			match is_u8(targ(gargs, 0)) {
				true => rt2(args, "byteSlice", &super::camel(name.strip_suffix("_mut")?)),
				false => rt2(args, "slice", &super::camel(&name)),
			}
		}
		// Each part of bytes is a view already, and one of other values lent mutably the
		// runtime's view.
		(
			"slice",
			"split"
			| "split_mut"
			| "rsplit"
			| "rsplit_mut"
			| "split_inclusive"
			| "split_inclusive_mut"
			| "splitn"
			| "splitn_mut"
			| "rsplitn"
			| "rsplitn_mut",
		) => match is_u8(targ(gargs, 0)) {
			true => rt2(
				args,
				"byteSlice",
				&super::camel(name.strip_suffix("_mut").unwrap_or(&name)),
			),
			false => rt2(args, "slice", &super::camel(&name)),
		},
		("Split", "as_slice") => Expr::rt("splitAsSlice", args),
		// Halves of bytes are views already, and of other values the runtime's views.
		("slice", "split_at_mut" | "split_at_mut_checked") => match is_u8(targ(gargs, 0)) {
			true => rt2(args, "byteSlice", &super::camel(&name.replace("_mut", ""))),
			false => rt2(args, "slice", &super::camel(&name)),
		},
		// The element is lent as `first_mut` lends it, and the rest as a view.
		("slice", "split_first_mut" | "split_last_mut") => match is_u8(targ(gargs, 0)) {
			true => rt2(args, "byteSlice", &super::camel(&name)),
			false => {
				let mut values = args;
				values.push(Expr::lit(b.t.needs_place(targ(gargs, 0)).to_string()));
				rt2(values, "slice", &super::camel(&name))
			}
		},
		("ChunksExact" | "RChunksExact", "remainder") | ("ChunksExactMut" | "RChunksExactMut", "into_remainder") => {
			Expr::rt("partsRemainder", args)
		}
		// The first `N` elements are `get(..N)`, and split off, `split_at_checked(N)`. A part
		// of bytes is a view already, and one of other values lent mutably the runtime's view.
		(
			"slice",
			"first_chunk"
			| "first_chunk_mut"
			| "last_chunk"
			| "last_chunk_mut"
			| "split_first_chunk"
			| "split_first_chunk_mut"
			| "split_last_chunk"
			| "split_last_chunk_mut",
		) => {
			let n = gargs.consts().next()?.try_to_target_usize(tcx)?;
			let (of, name) = match is_u8(targ(gargs, 0)) {
				true => ("byteSlice", name.strip_suffix("_mut").unwrap_or(&name)),
				false => ("slice", name.as_str()),
			};
			let mut values = args;
			if name.starts_with("first") {
				values.push(Expr::lit("0"));
			}
			values.push(Expr::lit(n.to_string()));
			let name = match name {
				"first_chunk" => "get_range",
				"first_chunk_mut" => "get_range_mut",
				"split_first_chunk" => "split_at_checked",
				"split_first_chunk_mut" => "split_at_mut_checked",
				name => name,
			};
			rt2(values, of, &super::camel(name))
		}
		// A slice of the one value.
		("raw", "from_ref") if std_path(tcx, call.def_id) == ["slice", "raw", "from_ref"] => {
			let value = args.pop()?;
			match is_u8(targ(gargs, 0)) {
				true => Expr::rt(
					"byteSliceFrom",
					vec![Expr::ident("Uint8Array").member("of").call(vec![value])],
				),
				false => Expr::Array(vec![value]),
			}
		}
		// Each slot but the last a clone, as `clone_from` makes it, and the last the value;
		// a `TrivialClone`'s copies, as std specializes `fill` to.
		("slice", "fill") => {
			let elem = targ(gargs, 0);
			if is_u8(elem) {
				return Some(rt2(args, "byteSlice", "fill"));
			}
			let clone = clone_into(b, elem, &name, span);
			let mut values = args;
			// A value its own clone, which nothing drops, is JS's `fill`.
			if matches!(&clone, Expr::Lit(l) if l == "undefined") && !b.t.drops(elem) {
				let slice = values.remove(0);
				return Some(slice.method("fill", values));
			}
			values.push(clone);
			Expr::rt("sliceFill", values)
		}
		("slice", "fill_with" | "rotate_left" | "rotate_right") => match is_u8(targ(gargs, 0)) {
			true => rt2(args, "byteSlice", &super::camel(&name)),
			false => rt2(args, "slice", &super::camel(&name)),
		},
		// A `TrivialClone`'s `clone_from_slice` is std's `copy_from_slice`, panics included.
		("slice", "copy_from_slice" | "clone_from_slice")
			if name == "copy_from_slice" || super::iter::implements(b.t, targ(gargs, 0), "TrivialClone") =>
		{
			let elem = targ(gargs, 0);
			if is_u8(elem) {
				return Some(rt2(args, "byteSlice", "copyFromSlice"));
			}
			let mut values = args;
			values.extend(b.t.copy_fn(elem, span));
			Expr::rt("sliceCopyFromSlice", values)
		}
		("slice", "clone_from_slice") => {
			let mut values = args;
			values.push(clone_into(b, targ(gargs, 0), &name, span));
			Expr::rt("sliceCloneFromSlice", values)
		}
		("PartialEq", "eq") if byte_slice(targ(gargs, 0)) && byte_slice(targ(gargs, 1)) => {
			Expr::rt("byteSliceEq", args)
		}
		("slice", "to_vec") => {
			let elem = targ(gargs, 0);
			let slice = args.pop()?;
			match is_u8(elem) {
				true => Expr::rt("byteSliceToByteVec", vec![slice]),
				false => return vec_clone(b, slice, elem, span),
			}
		}
		("slice", "last" | "swap") => match is_u8(targ(gargs, 0)) {
			true => rt2(args, "byteSlice", &super::camel(&name)),
			false => rt2(args, "slice", &super::camel(&name)),
		},
		("slice", "reverse") => elements(args.pop()?, Ty::new_slice(tcx, targ(gargs, 0))).method("reverse", Vec::new()),
		("slice", "contains") => {
			let elem = targ(gargs, 0);
			let [s, x] = <[Expr; 2]>::try_from(args).ok()?;
			match is_u8(elem) {
				true => Expr::rt("byteSliceView", vec![s]).method("includes", vec![x]),
				false => {
					let eq = super::collections::eq_fn(b, elem, span);
					Expr::rt("sliceContains", vec![s, x, eq])
				}
			}
		}
		// Sorted by `PartialOrd::lt`, or by a comparator, or by `lt` of the keys a function makes.
		(
			"slice",
			"sort" | "sort_unstable" | "sort_by" | "sort_unstable_by" | "sort_by_key" | "sort_unstable_by_key",
		) => {
			let elem = targ(gargs, 0);
			let mut values = args;
			values[0] = elements(values[0].clone(), Ty::new_slice(tcx, elem));
			match name.as_str() {
				// A typed array sorts its numbers by value, and equal bytes are alike.
				"sort" | "sort_unstable" if is_u8(elem) => values.remove(0).method("sort", Vec::new()),
				"sort" | "sort_unstable" => {
					values.push(super::collections::test_fn(b, targ(gargs, 0), "lt", span));
					Expr::rt("sliceSort", values)
				}
				"sort_by" | "sort_unstable_by" => Expr::rt("sliceSortBy", values),
				_ => {
					let key = targ(gargs, 1);
					if b.t.drops(key) {
						return Some(b.err(
							span,
							format!("`slice::{name}` by `{key}`, which drops, which is not supported yet"),
						));
					}
					values.push(super::collections::test_fn(b, key, "lt", span));
					Expr::rt("sliceSortByKey", values)
				}
			}
		}
		// Whether each element and the next are in order, as `sort` would leave them: by
		// `PartialOrd::le` in std's chunks, or as the slice's iterator's `is_sorted_by` and
		// `is_sorted_by_key` test them, which std's are.
		("slice", "is_sorted") => {
			let elem = targ(gargs, 0);
			let mut values = args;
			values[0] = elements(values[0].clone(), Ty::new_slice(tcx, elem));
			values.push(super::collections::test_fn(b, elem, "le", span));
			Expr::rt("sliceIsSorted", values)
		}
		("slice", "is_sorted_by" | "is_sorted_by_key") => {
			let elem = targ(gargs, 0);
			let mut values = args.into_iter();
			let slice = elements(values.next()?, Ty::new_slice(tcx, elem));
			let items = super::iter::by_ref(Expr::rt("iterSlice", vec![slice]), elem);
			let f = values.next()?;
			let (items, compare, drop) = match name.as_str() {
				"is_sorted_by" => (items, f, Expr::undefined()),
				_ => {
					let key = targ(gargs, 2);
					let le = super::collections::test_fn(b, key, "le", span);
					(Expr::rt("iterMap", vec![items, f]), le, b.t.dropper(key, span))
				}
			};
			Expr::rt(
				"iterIsSortedBy",
				super::iter::tail(vec![items, compare], vec![Expr::undefined(), drop]),
			)
		}
		("slice", "into_vec") => boxed_vec(args.pop()?, targ(gargs, 0)),
		("Vec", "into_boxed_slice") => vec_boxed(args.pop()?, targ(gargs, 0)),
		// `vec![a, b]`'s array once `mir::vec` took the box away. A `Copy` aggregate's operand
		// may be shared with where it came from, which the vector must not change.
		("boxed", "box_assume_init_into_vec_unsafe") => {
			let elem = targ(gargs, 0);
			let value = args.pop()?;
			if is_u8(elem) {
				return Some(Expr::rt("byteVecFrom", vec![value]));
			}
			match (b.t.copy_fn(elem, span), value) {
				(None, value) => value,
				(Some(_), Expr::Array(items)) => {
					Expr::Array(items.into_iter().map(|item| owned(b, item, elem, span)).collect())
				}
				(Some(copy), value) => value.member("map").call(vec![copy]),
			}
		}
		("Box", "new") => args.pop()?,
		// `vec![x; n]`: `n - 1` clones of `x`, then `x` itself, which the caller gives as its
		// own, as it does whatever std keeps.
		("vec", "from_elem") => {
			let elem = targ(gargs, 0);
			let mut values = args;
			match is_u8(elem) {
				true => Expr::rt("byteVecOf", values),
				false => {
					values.push(super::collections::clone_fn(b, elem, span));
					Expr::rt("vecFromElem", values)
				}
			}
		}

		// Vectors.
		("Vec", "new") => match is_u8(targ(gargs, 0)) {
			true => Expr::New(Box::new(Expr::Rt("ByteVec".into())), Vec::new()),
			false => Expr::Array(Vec::new()),
		},
		("Vec", "with_capacity") => match is_u8(targ(gargs, 0)) {
			true => Expr::New(Box::new(Expr::Rt("ByteVec".into())), args),
			false => Expr::Array(Vec::new()),
		},
		("Vec", "push") => {
			let mut values = args;
			match is_u8(targ(gargs, 0)) {
				true => Expr::rt("byteVecPush", values),
				false => {
					let v = values.remove(0);
					v.method("push", values)
				}
			}
		}
		("Vec", "pop") if is_u8(targ(gargs, 0)) => Expr::rt("byteVecPop", args),
		// JS's `pop` returns `undefined` for no element, where an `rt.None` must follow.
		("Vec", "pop") if sentinel_option(b.t, call).is_some() => Expr::rt("vecPop", args),
		("Vec", "pop") => args.pop()?.method("pop", vec![]),
		// The last element is lent to the test as its place where JS copies it.
		("Vec", "pop_if") if is_u8(targ(gargs, 0)) => Expr::rt("byteVecPopIf", args),
		("Vec", "pop_if") if b.t.needs_place(targ(gargs, 0)) => Expr::rt("vecPopIfMut", args),
		("Vec", "pop_if") => Expr::rt("vecPopIf", args),
		("Vec", "len") => args.pop()?.member("length"),
		("Vec", "is_empty") => args.pop()?.member("length").binary("===", Expr::lit("0")),
		("Vec", "truncate") if is_u8(targ(gargs, 0)) => Expr::rt("byteVecTruncate", args),
		("Vec", "clear") if is_u8(targ(gargs, 0)) => Expr::rt("byteVecClear", args),
		("Vec", "extend_from_slice") if is_u8(targ(gargs, 0)) => Expr::rt("byteVecExtendFromSlice", args),
		("Vec", "extend_from_slice") => {
			let mut values = args;
			let clone = super::collections::clone_fn(b, targ(gargs, 0), span);
			if !matches!(&clone, Expr::Lit(l) if l == "undefined") {
				values.push(clone);
			}
			Expr::rt("vecExtendFromSlice", values)
		}
		("Vec", "insert" | "remove" | "swap_remove") if is_u8(targ(gargs, 0)) => {
			rt2(args, "byteVec", &super::camel(&name))
		}
		("Vec", "insert" | "remove" | "swap_remove" | "truncate") => rt2(args, "vec", &super::camel(&name)),
		("Vec", "clear") => {
			let mut values = args;
			values.push(Expr::lit("0"));
			Expr::rt("vecTruncate", values)
		}
		// What each takes by value is a closure, which it drops once done. An element JS
		// copies is lent to the closure as its place where it takes a `&mut`.
		("Vec", "retain" | "retain_mut" | "dedup" | "dedup_by" | "dedup_by_key") => {
			if let Some(ty) = dropping_operand(b, call) {
				return Some(b.err(
					span,
					format!("passing `{ty}`, which drops, to `{owner}::{name}`, which is not supported yet"),
				));
			}
			let elem = targ(gargs, 0);
			let of = match is_u8(elem) {
				true => "byteVec",
				false => "vec",
			};
			let lend = match b.t.needs_place(elem) {
				true => "Mut",
				false => "",
			};
			let mut values = args;
			let f = match name.as_str() {
				"retain" => "retain".to_string(),
				"retain_mut" => format!("retain{lend}"),
				"dedup" if is_u8(elem) => "dedup".to_string(),
				"dedup" => {
					values.push(super::collections::eq_fn(b, elem, span));
					"dedupBy".to_string()
				}
				"dedup_by" => format!("dedupBy{lend}"),
				_ => {
					// `Vec<T, A>::dedup_by_key::<F, K>`.
					let key = targ(gargs, 3);
					if b.t.drops(key) {
						return Some(b.err(
							span,
							format!("`Vec::{name}` by `{key}`, which drops, which is not supported yet"),
						));
					}
					values.push(super::collections::eq_fn(b, key, span));
					format!("dedupByKey{lend}")
				}
			};
			rt2(values, of, &f)
		}
		// Of a range, which `Vec<T, A>::drain::<R>` takes, as the other operand is the vector.
		("Vec", "drain") => {
			let ranged = super::collections::bounds(b, call, args.pop()?);
			args.push(ranged.bounds.clone());
			return Some(ranged.after(match is_u8(targ(gargs, 0)) {
				true => Expr::rt("byteVecDrain", args),
				false => {
					let dropper = b.t.dropper(targ(gargs, 0), span);
					Expr::rt("vecDrain", super::iter::tail(args, vec![dropper]))
				}
			}));
		}
		("Vec", "as_slice" | "as_mut_slice") => {
			let v = args.pop()?;
			vec_view(v, targ(gargs, 0))
		}
		("Deref", "deref") | ("DerefMut", "deref_mut") => {
			let ty = targ(gargs, 0);
			let v = args.pop()?;
			match ty.kind() {
				ty::Adt(adt, vargs) => match Std::of(tcx, adt.did()) {
					Some(Std::Vec) => vec_view(v, vargs.type_at(0)),
					Some(Std::String | Std::Box) => v,
					Some(Std::Cow) if string(tcx, ty) => v,
					Some(Std::Cow) if let Some(elem) = cow_slice(tcx, ty) => cow_deref(v, elem),
					// The greatest element of a heap, which `DerefMut` marks to sift once lent.
					Some(Std::PeekMut) => {
						let p = match v.is_path() {
							true => v,
							false => b.spill(v, "peek", out),
						};
						if name == "deref_mut" {
							out.push(StmtKind::Expr(Expr::rt("peekMutDerefMut", vec![p.clone()])).at(None));
						}
						p.member("v")
					}
					_ => return None,
				},
				_ => return None,
			}
		}

		// Strings.
		("String" | "str", "as_bytes") => Expr::rt("stringAsBytes", args),
		("String", "from_utf8") => Expr::rt("stringFromUtf8", args),
		// `str::from_utf8` is `core::str::converts::from_utf8`, or `str`'s own.
		("converts" | "str", "from_utf8") => Expr::rt("strFromUtf8", args),
		("Utf8Error", "valid_up_to") => args.pop()?.member("validUpTo"),
		("Utf8Error", "error_len") => args.pop()?.member("errorLen"),
		("String" | "str", "len") => Expr::rt("stringLen", args),
		// A `Box<str>` is its text, as a `String` is.
		("String", "as_str" | "into_boxed_str") | ("str", "into_string") => args.pop()?,
		("String", "new") => Expr::Str(String::new()),
		("String", "with_capacity") => Expr::rt("stringWithCapacity", args),
		// Of the place the `&mut` is, which it writes as it drops, and of a range, as `Vec::drain`.
		("String", "drain") => {
			let ranged = super::collections::bounds(b, call, args.pop()?);
			let s = args.pop()?;
			let s = match s.is_path() {
				true => s,
				false => b.spill(s, "s", out),
			};
			let len = Expr::rt("stringLen", vec![s.clone().member("v")]);
			return Some(Expr::rt("stringDrain", vec![s, ranged.within(len)]));
		}
		("Drain", "as_str") => Expr::rt("stringDrainAsStr", args),
		("ParseIntError", "kind") => args.pop()?.member("kind"),
		// `str::parse` of a float, by std's grammar rather than `Number`'s.
		("str", "parse") | ("FromStr", "from_str") if let Some(float) = super::float::Float::of(targ(gargs, 0)) => {
			Expr::rt_of(float.name(), "fromStr").call(args)
		}
		// A `char`'s steps through its chars by `&mut`, which no body of std's may.
		("str", "parse") | ("FromStr", "from_str") if targ(gargs, 0).is_char() => Expr::rt("charFromStr", args),
		("String" | "str", "is_empty") => args.pop()?.member("length").binary("===", Expr::lit("0")),
		("str", "chars") => Expr::rt("strChars", args),
		("str", "bytes") => Expr::rt("strBytes", args),
		("str", "is_char_boundary") => Expr::rt("strIsCharBoundary", args),
		// Searches by a pattern.
		("str", "split" | "match_indices" | "contains" | "starts_with" | "ends_with" | "rfind" | "split_once")
		| ("str", "trim_start_matches" | "trim_end_matches") => {
			let [s, p] = <[Expr; 2]>::try_from(args).ok()?;
			Expr::rt_of("str", &super::camel(&name)).call(vec![s, pattern(b.t, targ(gargs, 0), p)?])
		}
		// A slice of strings concatenated, or joined by a `&str`.
		("slice", "concat" | "join")
			if string(tcx, targ(gargs, 0).peel_refs()) && targ(gargs, 1).peel_refs().is_str() =>
		{
			let mut values = args;
			if name == "concat" {
				values.push(Expr::Str(String::new()));
			}
			Expr::rt("strsJoin", values)
		}
		("slice", "concat" | "join") => {
			return slices_join(b, targ(gargs, 0), targ(gargs, 1), name == "join", args, span);
		}
		("char", _) if let Some(method) = ascii(&name) => Expr::rt(method, args),
		("ascii", "escape_default") => Expr::rt("escapeAscii", args),
		// Those of `[u8]`, a `Vec<u8>`'s by `Deref`, and of `str`, a `String`'s, but `str`'s
		// `make_ascii_*`, which MIR lowers as a byte's and a char's.
		(
			"slice" | "str",
			"is_ascii"
			| "eq_ignore_ascii_case"
			| "to_ascii_uppercase"
			| "to_ascii_lowercase"
			| "trim_ascii_start"
			| "trim_ascii_end"
			| "trim_ascii",
		)
		| ("slice", "escape_ascii" | "make_ascii_uppercase" | "make_ascii_lowercase") => {
			let of = if owner == "slice" { "byteSlice" } else { "str" };
			Expr::rt_of(of, &super::camel(&name)).call(args)
		}
		// A `Cow<str>` is its text, whichever variant holds it.
		("Cow", "into_owned") if targ(gargs, 0).is_str() => args.pop()?,
		("AsRef", "as_ref") if string(tcx, targ(gargs, 0)) && targ(gargs, 1).is_str() => args.pop()?,
		// A `Cow<[T]>` borrows its slice, and its `Vec` is its own or a clone of what it
		// borrows, so it drops nothing.
		("Cow", "into_owned") if let ty::Slice(elem) = targ(gargs, 0).kind() => {
			return Some(cow_cloned(b, "IntoOwned", args.pop()?, *elem, span));
		}
		("AsRef", "as_ref") | ("Borrow", "borrow") if let Some(elem) = cow_slice(tcx, targ(gargs, 0)) => {
			cow_deref(args.pop()?, elem)
		}
		// A slice's are unstable, and left to the refusal of any other call.
		("Cow", "is_borrowed" | "is_owned") if !targ(gargs, 0).is_slice() => {
			let cow = b.t.sig(call.def_id, gargs).inputs()[0].peel_refs();
			return Some(b.err(span, cow_variant(cow)));
		}
		("ToString", "to_string") | ("ToOwned", "to_owned") if matches!(self_ty()?.peel_refs().kind(), ty::Str) => {
			b.t.pointee_value(self_ty()?, args.pop()?)
		}

		// Panics, which format their message as std does.
		("panicking", "panic_fmt") => {
			let message = args.pop()?;
			match literal(&message) {
				Some(s) => Expr::rt("panic", vec![Expr::Str(s)]),
				None => Expr::rt("panicFmt", vec![message]),
			}
		}
		("panicking", "panic") | ("rt", "begin_panic") => Expr::rt("panic", args),
		("panicking", "panic_explicit") => Expr::rt("panic", vec![Expr::Str("explicit panic".into())]),
		("panicking", "panic_display" | "unreachable_display") => {
			let value = args.pop()?;
			let value = b.fmt_value(fmt::Trait::Display, targ(gargs, 0), value, span, out);
			let mut message = vec![value];
			if name == "unreachable_display" {
				message.insert(0, Expr::Str("internal error: entered unreachable code: ".into()));
			}
			Expr::rt("panicFmt", vec![Expr::Array(message)])
		}
		// `assert_eq!` and `assert_ne!`: the operator, both sides, and any message.
		("panicking", "assert_failed") => {
			let [kind, left, right, message] = <[Expr; 4]>::try_from(args).ok()?;
			let left = b.fmt_value(fmt::Trait::Debug, targ(gargs, 0), left, span, out);
			let right = b.fmt_value(fmt::Trait::Debug, targ(gargs, 1), right, span, out);
			let mut values = vec![kind, left, right];
			if !matches!(&message, Expr::Lit(text) if text == "undefined") {
				values.push(message);
			}
			Expr::rt("assertFailed", values)
		}

		// Formatting.
		("hint", "must_use") => args.pop()?,
		// The size wasm32 lays the type out in, which translation targets.
		("mem", "size_of") => {
			let env = b.t.typing_env_pub();
			let layout = tcx.layout_of(env.as_query_input(targ(gargs, 0))).ok()?;
			Expr::lit(layout.size.bytes().to_string())
		}
		// A reference to an array of bytes is an `rt.ByteSlice` already, and to one of other
		// values its JS array, so each is its own slice, and writes through it land in it.
		("array", "as_slice" | "as_mut_slice") => args.pop()?,
		("array", "from_fn") => {
			let n = gargs.consts().next()?.try_to_target_usize(tcx)?;
			let f = match targ(gargs, 0).kind() {
				ty::Uint(ty::UintTy::U8) => "bytesFromFn",
				_ => "arrayFromFn",
			};
			Expr::rt(f, vec![Expr::lit(n.to_string()), args.pop()?])
		}
		("array", "map") => {
			let f = match targ(gargs, 2).kind() {
				ty::Uint(ty::UintTy::U8) => "bytesMap",
				_ => "arrayMap",
			};
			Expr::rt(f, args)
		}
		// Each element lent as `first_mut` lends it: an element JS copies as its place, an
		// array of bytes as its view, and an object as itself.
		("array", "each_ref" | "each_mut") => {
			let elem = targ(gargs, 0);
			let a = elements(args.pop()?, Ty::new_slice(tcx, elem));
			match () {
				_ if name == "each_mut" && b.t.needs_place(elem) => Expr::rt("arrayPlaces", vec![a]),
				_ if super::byte_array(elem) => Expr::rt("arrayMap", vec![a, Expr::Rt("byteSliceFrom".into())]),
				_ => Expr::rt("arrayFrom", vec![a]),
			}
		}
		("fmt", "format") => {
			let parts = args.pop()?;
			match literal(&parts) {
				Some(s) => Expr::Str(s),
				None => Expr::rt("fmtFormat", vec![parts]),
			}
		}
		("Display" | "Debug" | "LowerHex" | "UpperHex" | "Octal" | "Binary" | "LowerExp" | "UpperExp", "fmt") => {
			let tr = fmt::Trait::of(tcx, tcx.trait_of_assoc(call.def_id)?)?;
			let [value, f] = <[Expr; 2]>::try_from(args).ok()?;
			fmt::std_impl(b, tr, targ(gargs, 0), value, f, span, out)?
		}
		("ToString", "to_string") => {
			let ty = targ(gargs, 0);
			let value = args.pop()?;
			match ty.kind() {
				ty::Adt(..) if string(tcx, ty) => value,
				_ => {
					let value = b.fmt_value(fmt::Trait::Display, ty, value, span, out);
					Expr::rt("fmtToString", vec![value])
				}
			}
		}
		("Formatter", "write_fmt") => {
			let [f, parts] = <[Expr; 2]>::try_from(args).ok()?;
			match literal(&parts) {
				Some(s) => Expr::rt("fmtWriteStr", vec![f, Expr::Str(s)]),
				None => Expr::rt("fmtWriteFmt", vec![f, parts]),
			}
		}
		// Each takes its receiver first: `rt.fmtPad(f, s)`, `rt.debugStructField(b, name, value)`.
		("Formatter", "write_str" | "pad" | "alternate" | "width" | "debug_struct" | "debug_tuple" | "debug_list")
		| ("DebugStruct" | "DebugTuple", "field" | "finish" | "finish_non_exhaustive")
		| ("DebugList", "entry" | "finish" | "finish_non_exhaustive") => {
			let of = match owner.as_str() {
				"Formatter" => "fmt",
				"DebugStruct" => "debugStruct",
				"DebugTuple" => "debugTuple",
				_ => "debugList",
			};
			Expr::rt_of(of, &super::camel(&name)).call(args)
		}
		// Derived `Debug`: `debug_struct_field2_finish(f, name, "a", &a, "b", &b)`.
		("Formatter", "debug_struct_fields_finish") => Expr::rt("fmtDebugStructFields", args),
		("Formatter", "debug_tuple_fields_finish") => Expr::rt("fmtDebugTupleFields", args),
		("Formatter", n) if n.starts_with("debug_struct_field") || n.starts_with("debug_tuple_field") => {
			let mut values = args.into_iter();
			let (f, type_name) = (values.next()?, values.next()?);
			let structure = n.starts_with("debug_struct");
			let (of, start) = match structure {
				true => ("debugStruct", "fmtDebugStruct"),
				false => ("debugTuple", "fmtDebugTuple"),
			};
			let mut builder = Expr::rt(start, vec![f, type_name]);
			let values: Vec<Expr> = values.collect();
			let fields = match structure {
				true => values.chunks(2).map(<[Expr]>::to_vec).collect::<Vec<_>>(),
				false => values.into_iter().map(|v| vec![v]).collect(),
			};
			for field in fields {
				builder = Expr::rt_of(of, "field").call([vec![builder], field].concat());
			}
			Expr::rt_of(of, "finish").call(vec![builder])
		}
		_ => return super::iter::call(b, call, &owner, &name, args),
	};
	if let Some(ty) = dropping {
		return Some(b.err(
			span,
			format!("passing `{ty}`, which drops, to `{owner}::{name}`, which is not supported yet"),
		));
	}
	// What drops each operand the mapping may discard, then `rt.None` where it is one.
	let lost = discarded.unwrap_or_default().iter().map(|&n| targ(gargs, n));
	let Some(mut droppers) = b.t.droppers(lost, span) else {
		return Some(Expr::undefined());
	};
	let none = match none_last(&owner, &name) {
		true => sentinel_option(b.t, call).and_then(|inner| b.t.none_arg(inner)),
		false => None,
	};
	if let Some(none) = none {
		droppers.resize(discarded.unwrap_or_default().len(), Expr::undefined());
		droppers.push(none);
	}
	Some(match e {
		Expr::Call(f, mut args) if !droppers.is_empty() => {
			args.extend(droppers);
			Expr::Call(f, args)
		}
		e => e,
	})
}

/// A number from its big-endian bytes, each a value: shifted and or-ed.
pub fn be_word(int: Int, values: Vec<Expr>) -> Expr {
	let last = values.len() - 1;
	let mut word: Option<Expr> = None;
	let mut top = false;
	for (i, value) in values.into_iter().enumerate() {
		if matches!(&value, Expr::Lit(l) if l == "0") {
			continue;
		}
		top |= i == 0;
		let shift = 8 * (last - i);
		let term = match shift {
			0 => value,
			_ => value.binary("<<", Expr::lit(shift.to_string())),
		};
		word = Some(match word {
			Some(word) => word.binary("|", term),
			None => term,
		});
	}
	let Some(word) = word else {
		return Expr::lit("0");
	};
	// JS's `|` and `<<` make an `i32`, so only a `u32` with its top byte set and the
	// narrower signed types need their bits read again.
	match (int.signed, int.bits) {
		(false, 32) if top => int.wrap(word),
		(true, bits) if bits < 32 => int.wrap(word),
		_ => word,
	}
}

/// The `n`th type among a call's generic arguments, skipping lifetimes.
pub fn targ<'tcx>(gargs: GenericArgsRef<'tcx>, n: usize) -> Ty<'tcx> {
	gargs.types().nth(n).expect("a type argument")
}

/// `value`, a `ty`, as a value of its own: copied if it is a `Copy` aggregate that is not
/// made right here.
fn owned<'tcx>(b: &mut Body<'_, '_, 'tcx>, value: Expr, ty: Ty<'tcx>, span: Span) -> Expr {
	fn made(e: &Expr) -> bool {
		match e {
			Expr::Array(_) | Expr::Object(_) | Expr::New(..) => true,
			Expr::As(e, _) | Expr::Located(_, e) => made(e),
			_ => false,
		}
	}
	match made(&value) {
		true => value,
		false => b.t.copied(value, ty, span),
	}
}

/// What clones an `elem` into a slot of a slice, as `fill` and `clone_from_slice` do by
/// `clone_from`: a copy for a `TrivialClone`, or else `clone_fn`, where `clone_from` does
/// as `*self = source.clone()` does. Refused for any other `clone_from`.
fn clone_into<'tcx>(b: &mut Body<'_, '_, 'tcx>, elem: Ty<'tcx>, name: &str, span: Span) -> Expr {
	if super::iter::implements(b.t, elem, "TrivialClone") {
		return b.t.copy_fn(elem, span).unwrap_or_else(Expr::undefined);
	}
	if !clone_from_clones(b.t, elem) {
		return b.err(
			span,
			format!("`slice::{name}` of `{elem}`, whose `clone_from` is its own, which is not supported yet"),
		);
	}
	super::collections::clone_fn(b, elem, span)
}

/// Whether `clone_from` of `ty` clones, and drops what it writes over, as
/// `*self = source.clone()` would: `Clone`'s own, and std's own over such types, where it
/// only reuses memory, if nothing drops or it is an `Option`'s.
pub(super) fn clone_from_clones<'tcx>(t: &mut Translator<'_, 'tcx>, ty: Ty<'tcx>) -> bool {
	let tcx = t.tcx;
	let clone_from = super::collections::trait_fn(tcx, "Clone", "clone_from");
	let resolved = Instance::try_resolve(tcx, t.typing_env_pub(), clone_from, tcx.mk_args(&[ty.into()]));
	let Some(instance) = resolved.ok().flatten() else {
		return false;
	};
	if instance.def_id() == clone_from {
		return true;
	}
	let inner: Vec<Ty<'tcx>> = match ty.kind() {
		ty::Adt(adt, args) if is_std(tcx, adt.did()) => args.types().collect(),
		ty::Array(elem, _) => vec![*elem],
		_ => return false,
	};
	(t.std_of(ty) == Some(Std::Option) || !t.drops(ty)) && inner.into_iter().all(|arg| clone_from_clones(t, arg))
}

/// `concat` of a slice of `item`s, or with `join`, its `join` by `by`, a `&T` or a
/// `&[T]`, where each item borrows a slice of `T`s: a `Vec<T>` of clones of their elements
/// and the separator's between them, in order. `None` for an item with no slice.
fn slices_join<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	item: Ty<'tcx>,
	by: Ty<'tcx>,
	join: bool,
	mut args: Vec<Expr>,
	span: Span,
) -> Option<Expr> {
	let sep = match join {
		true => args.pop()?,
		false => Expr::undefined(),
	};
	// A `&T` separator is a slice of one.
	let (elem, one) = match by.builtin_deref(true) {
		Some(by) if join => match by.kind() {
			ty::Slice(elem) => (*elem, false),
			_ => (by, true),
		},
		_ => (by, false),
	};
	let bytes = matches!(elem.kind(), ty::Uint(ty::UintTy::U8));
	let sep = match (one, bytes) {
		(true, true) => Expr::rt(
			"byteSliceFrom",
			vec![Expr::ident("Uint8Array").member("of").call(vec![sep])],
		),
		(true, false) => Expr::Array(vec![sep]),
		(false, _) => sep,
	};
	let items = args.pop()?;
	// What reads an item as its slice, unless it is one.
	let slice = match item.kind() {
		ty::Array(..) => bytes.then_some("byteSliceFrom"),
		ty::Ref(_, s, _) if s.is_slice() => None,
		_ if b.t.std_of(item) == Some(Std::Vec) => bytes.then_some("byteVecSlice"),
		_ if cow_slice(b.t.tcx, item).is_some() => Some(if bytes { "byteCowSlice" } else { "cowSlice" }),
		_ if box_slice(b.t.tcx, item).is_some() => None,
		_ => return None,
	};
	let slice = slice.map_or_else(Expr::undefined, |f| Expr::Rt(f.into()));
	if bytes {
		return Some(Expr::rt("byteSlicesJoin", trimmed(vec![items, sep, slice])));
	}
	let size = super::iter::size_of(b, elem, span);
	let clone = super::collections::clone_fn(b, elem, span);
	Some(Expr::rt("slicesJoin", trimmed(vec![items, size, sep, slice, clone])))
}

/// `args` without the `undefined`s that end them, which a runtime function's optional
/// parameters take as left out.
fn trimmed(mut args: Vec<Expr>) -> Vec<Expr> {
	while matches!(args.last(), Some(Expr::Lit(l)) if l == "undefined") {
		args.pop();
	}
	args
}

/// `cow`, a `Cow<[elem]>`, through the runtime's `cow{op}`, `IntoOwned` or `Clone`, which
/// clones what it borrows, or for `Clone` what it owns, by each item's `Clone`.
pub fn cow_cloned<'tcx>(b: &mut Body<'_, '_, 'tcx>, op: &str, cow: Expr, elem: Ty<'tcx>, span: Span) -> Expr {
	if matches!(elem.kind(), ty::Uint(ty::UintTy::U8)) {
		return Expr::rt(format!("byteCow{op}"), vec![cow]);
	}
	let mut args = vec![cow];
	match super::collections::clone_fn(b, elem, span) {
		Expr::Lit(l) if l == "undefined" => {}
		clone => args.push(clone),
	}
	Expr::rt(format!("cow{op}"), args)
}

/// A clone of `value`, a slice, array, or `Vec` of `elem` other than bytes, as a `Vec`: a
/// copy of the array if each element's clone is itself, or else each element cloned.
fn vec_clone<'tcx>(b: &mut Body<'_, '_, 'tcx>, value: Expr, elem: Ty<'tcx>, span: Span) -> Option<Expr> {
	let clone = super::collections::clone_fn(b, elem, span);
	if matches!(&clone, Expr::Lit(l) if l == "undefined") {
		return Some(value.method("slice", Vec::new()));
	}
	Some(value.method("map", vec![clone]))
}

/// A `Box<[elem]>` of clones of the elements of `slice`, as std's `From<&[T]>` and `Clone`
/// make one.
fn slice_boxed<'tcx>(b: &mut Body<'_, '_, 'tcx>, slice: Expr, elem: Ty<'tcx>, span: Span) -> Option<Expr> {
	match elem.kind() {
		ty::Uint(ty::UintTy::U8) => Some(Expr::rt(
			"byteSliceFrom",
			vec![Expr::rt("byteSliceToArray", vec![slice])],
		)),
		_ => vec_clone(b, slice, elem, span),
	}
}

/// The `Box<[elem]>` that a `Vec<elem>`, `v`, becomes: its elements, as many as it holds.
/// std's shrinks its capacity to them, which no translated `Vec` reports.
pub(super) fn vec_boxed(v: Expr, elem: Ty<'_>) -> Expr {
	match elem.kind() {
		ty::Uint(ty::UintTy::U8) => Expr::rt("byteVecSlice", vec![v]),
		_ => v,
	}
}

/// The `Vec<elem>` that a `Box<[elem]>`, `boxed`, becomes, as `into_vec` makes it.
fn boxed_vec(boxed: Expr, elem: Ty<'_>) -> Expr {
	match elem.kind() {
		ty::Uint(ty::UintTy::U8) => Expr::rt("byteSliceToByteVec", vec![boxed]),
		_ => boxed,
	}
}

/// The `Box<[elem]>` that `From<[elem; N]>` moves `array` into: its elements.
fn array_boxed(array: Expr, elem: Ty<'_>) -> Expr {
	match elem.kind() {
		ty::Uint(ty::UintTy::U8) => Expr::rt("byteSliceFrom", vec![array]),
		_ => array,
	}
}

/// The operator a binary operator trait, `Add` or another, applies.
pub fn operator(owner: &str) -> Option<rustc_middle::mir::BinOp> {
	use rustc_middle::mir::BinOp;
	Some(match owner {
		"Add" => BinOp::Add,
		"Sub" => BinOp::Sub,
		"Mul" => BinOp::Mul,
		"Div" => BinOp::Div,
		"Rem" => BinOp::Rem,
		"BitAnd" => BinOp::BitAnd,
		"BitOr" => BinOp::BitOr,
		"BitXor" => BinOp::BitXor,
		"Shl" => BinOp::Shl,
		"Shr" => BinOp::Shr,
		_ => return None,
	})
}

/// How the operators of `ty` overflow and the integer it holds, if it is a `Wrapping` or a
/// `Saturating` of an integer.
pub fn overflowing<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<(Overflow, Ty<'tcx>)> {
	let ty::Adt(adt, args) = ty.kind() else {
		return None;
	};
	let overflow = Std::of(tcx, adt.did())?.overflow()?;
	Int::of(args.type_at(0)).map(|_| (overflow, args.type_at(0)))
}

/// A `str` method's pattern as the runtime takes it, an `rt.Pattern`: a `char`'s text, a
/// string or a predicate as it is, or a predicate for an array or slice of chars. `None`
/// for any other pattern.
fn pattern<'tcx>(t: &Translator<'_, 'tcx>, ty: Ty<'tcx>, p: Expr) -> Option<Expr> {
	match ty.kind() {
		ty::Char => Some(char_text(p)),
		ty::Closure(..) | ty::FnDef(..) => Some(p),
		ty::Ref(..) if is_text(t, ty) => Some(p),
		// Any of the chars of an array or slice.
		ty::Array(c, _) | ty::Slice(c) if c.is_char() => Some(Expr::rt("patternChars", vec![p])),
		ty::Ref(_, inner, _) if matches!(inner.kind(), ty::Array(c, _) | ty::Slice(c) if c.is_char()) => {
			Some(Expr::rt("patternChars", vec![p]))
		}
		_ => None,
	}
}

/// Whether `ty` is a `str` or a `String`, behind any references: a JS string.
fn is_text<'tcx>(t: &Translator<'_, 'tcx>, ty: Ty<'tcx>) -> bool {
	let ty = ty.peel_refs();
	ty.is_str() || t.std_of(ty) == Some(Std::String)
}

/// A `char`'s text: a literal for a constant, or `String.fromCodePoint(c)`.
pub fn char_text(c: Expr) -> Expr {
	if let Expr::Lit(n) = &c
		&& let Some(c) = n.parse().ok().and_then(char::from_u32)
	{
		return Expr::Str(c.to_string());
	}
	Expr::ident("String").member("fromCodePoint").call(vec![c])
}

/// `rt.<of><Name>(args)`.
fn rt2(args: Vec<Expr>, of: &str, name: &str) -> Expr {
	Expr::rt_of(of, name).call(args)
}

/// What the runtime's functions of a std range type start with.
fn range_of(std: Std) -> Option<&'static str> {
	match std {
		Std::Range => Some("range"),
		Std::RangeInclusive => Some("rangeInclusive"),
		Std::RangeFrom => Some("rangeFrom"),
		Std::RangeTo | Std::RangeToInclusive => Some("rangeTo"),
		_ => None,
	}
}

/// The `Step` a range of `ty`s is made with: a `char`'s, which skips the surrogates, an
/// unsigned `NonZero`'s, which stops at either end where its integer's wraps, and a wide
/// integer's, which steps past 2^53, or any integer's where `every` integer needs it to
/// step within its type.
pub fn range_step<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>, every: bool) -> Option<Expr> {
	if let ty::Adt(adt, args) = ty.kind()
		&& Std::of(tcx, adt.did()) == Some(Std::NonZero)
	{
		let int = Int::of(args.type_at(0)).filter(|int| !int.signed)?;
		let step = vec![int.func("step"), int.lit(int.max() as i128)];
		return (every || int.big()).then(|| Expr::rt("nonZeroStep", step));
	}
	match Int::of(ty) {
		Some(int) => (every || int.big()).then(|| int.func("step")),
		None => ty.is_char().then(|| Expr::Rt("charStep".into())),
	}
}

fn int_named(name: &str) -> Option<Int> {
	let (signed, bits, pointer) = match name {
		"u8" => (false, 8, false),
		"u16" => (false, 16, false),
		"u32" => (false, 32, false),
		"u64" => (false, 64, false),
		"u128" => (false, 128, false),
		"usize" => (false, 32, true),
		"i8" => (true, 8, false),
		"i16" => (true, 16, false),
		"i32" => (true, 32, false),
		"i64" => (true, 64, false),
		"i128" => (true, 128, false),
		"isize" => (true, 32, true),
		_ => return None,
	};
	Some(Int { bits, signed, pointer })
}

/// The runtime function of `u8`'s or `char`'s ASCII method `name`, which the two share:
/// std's char ones are its byte ones' for an ASCII char, and past ASCII none holds or
/// changes either. Their `make_ascii_*` write where the `&mut` borrows, as MIR lowers them.
fn ascii(name: &str) -> Option<String> {
	match name {
		"is_ascii"
		| "is_ascii_alphabetic"
		| "is_ascii_uppercase"
		| "is_ascii_lowercase"
		| "is_ascii_alphanumeric"
		| "is_ascii_digit"
		| "is_ascii_hexdigit"
		| "is_ascii_punctuation"
		| "is_ascii_graphic"
		| "is_ascii_whitespace"
		| "is_ascii_control"
		| "to_ascii_uppercase"
		| "to_ascii_lowercase"
		| "eq_ignore_ascii_case"
		| "escape_ascii" => Some(super::camel(name)),
		_ => None,
	}
}

/// The type whose inherent impl holds `def_id`.
fn impl_self(tcx: TyCtxt<'_>, def_id: DefId) -> Option<Ty<'_>> {
	let imp = tcx.inherent_impl_of_assoc(def_id)?;
	Some(tcx.type_of(imp).instantiate_identity().skip_norm_wip())
}

/// The integer methods whose runtime function serves every width, `intCheckedPow` for
/// `checked_pow`, given the type's width and sign after the method's arguments. A
/// `_signed` or `_unsigned` form is its plain one's, which takes any integer.
const SHARED: &[&str] = &[
	"abs",
	"abs_diff",
	"bit_width",
	"borrowing_sub",
	"carrying_add",
	"carrying_mul",
	"carrying_mul_add",
	"checked_abs",
	"checked_add",
	"checked_div",
	"checked_div_euclid",
	"checked_ilog",
	"checked_ilog10",
	"checked_ilog2",
	"checked_isqrt",
	"checked_neg",
	"checked_next_multiple_of",
	"checked_next_power_of_two",
	"checked_pow",
	"checked_rem",
	"checked_rem_euclid",
	"checked_shl",
	"checked_shr",
	"checked_signed_diff",
	"checked_sub",
	"count_ones",
	"count_zeros",
	"div_euclid",
	"funnel_shl",
	"funnel_shr",
	"highest_one",
	"ilog",
	"ilog10",
	"ilog2",
	"is_power_of_two",
	"isolate_highest_one",
	"isolate_lowest_one",
	"isqrt",
	"lowest_one",
	"midpoint",
	"next_multiple_of",
	"next_power_of_two",
	"overflowing_abs",
	"overflowing_add",
	"overflowing_div",
	"overflowing_div_euclid",
	"overflowing_mul",
	"overflowing_neg",
	"overflowing_pow",
	"overflowing_rem",
	"overflowing_rem_euclid",
	"overflowing_shl",
	"overflowing_shr",
	"overflowing_sub",
	"pow",
	"rem_euclid",
	"reverse_bits",
	"rotate_left",
	"rotate_right",
	"saturating_abs",
	"saturating_add",
	"saturating_pow",
	"saturating_sub",
	"signum",
	"strict_add",
	"strict_neg",
	"strict_sub",
	"swap_bytes",
	"trailing_ones",
	"unbounded_shl",
	"unbounded_shr",
	"unsigned_abs",
	"wrapping_abs",
	"wrapping_add",
	"wrapping_div_euclid",
	"wrapping_neg",
	"wrapping_next_power_of_two",
	"wrapping_pow",
	"wrapping_rem_euclid",
	"wrapping_shl",
	"wrapping_shr",
	"wrapping_sub",
];

/// Of [`SHARED`], what only an unsigned type has, which a signed one's of the same name,
/// where it has one, does otherwise.
const UNSIGNED: &[&str] = &[
	"bit_width",
	"borrowing_sub",
	"carrying_add",
	"carrying_mul",
	"carrying_mul_add",
	"checked_next_multiple_of",
	"checked_next_power_of_two",
	"checked_signed_diff",
	"funnel_shl",
	"funnel_shr",
	"is_power_of_two",
	"next_multiple_of",
	"next_power_of_two",
	"wrapping_next_power_of_two",
];

/// A call of `int`'s inherent method `name` with `args`, or of a `NonZero`'s, a
/// `Wrapping`'s, or a `Saturating`'s that is its integer's: `None` for one the runtime
/// lacks.
fn int_method<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	call: &Call<'tcx>,
	int: Int,
	name: &str,
	mut args: Vec<Expr>,
) -> Option<Expr> {
	let tcx = b.t.tcx;
	let method = match name {
		"from_be_bytes" | "from_le_bytes" | "from_ne_bytes" => "fromBeBytes",
		"to_be_bytes" | "to_le_bytes" | "to_ne_bytes" => "toBeBytes",
		"leading_ones" => "leadingOnes",
		"leading_zeros" => "leadingZeros",
		"trailing_zeros" => "trailingZeros",
		"checked_add" => "checkedAdd",
		"checked_sub" => "checkedSub",
		"checked_mul" => "checkedMul",
		"saturating_add" => "saturatingAdd",
		"saturating_sub" => "saturatingSub",
		"saturating_mul" => "saturatingMul",
		// Only a signed one can overflow, at `MIN / -1`.
		"saturating_div" | "wrapping_div" if int.signed => &super::camel(name),
		"wrapping_rem" if int.signed => "wrappingRem",
		"saturating_div" | "wrapping_div" => "div",
		"wrapping_rem" => "rem",
		"saturating_neg" => return Some(int.method("saturatingSub", [vec![int.lit(0)], args].concat())),
		"wrapping_add" => "wrappingAdd",
		"wrapping_sub" => "wrappingSub",
		"wrapping_mul" => "wrappingMul",
		"div_ceil" => "divCeil",
		"is_multiple_of" => "isMultipleOf",
		_ if int.bits == 8
			&& !int.signed
			&& let Some(method) = ascii(name) =>
		{
			return Some(Expr::rt(method, args));
		}
		// The operator's, which panics with the message each of these does.
		"strict_add" => "add",
		"strict_sub" => "sub",
		"strict_mul" => "mul",
		"strict_div" => "div",
		"strict_rem" => "rem",
		"strict_shl" => "shl",
		"strict_shr" => "shr",
		// wasm32 is little-endian.
		"to_le" | "from_le" => return args.pop(),
		"max_value" => return Some(int.lit(int.max() as i128)),
		"min_value" => return Some(int.lit(int.min())),
		"is_positive" => return Some(int.compare(">", args.pop()?, int.lit(0))),
		"is_negative" => return Some(int.compare("<", args.pop()?, int.lit(0))),
		"cast_signed" | "cast_unsigned" => {
			let sig = b.t.sig(call.def_id, call.args);
			let (from, to) = (unwrapped(tcx, sig.inputs()[0]), unwrapped(tcx, sig.output()));
			let value = Operand {
				expr: args.pop()?,
				value: None,
			};
			return Some(num::cast(b, value, from, to, call.span));
		}
		_ => {
			let shared = match name {
				"to_be" | "from_be" => "swap_bytes",
				// Each panics as its plain form does.
				"strict_pow" | "strict_abs" | "strict_div_euclid" | "strict_rem_euclid" => &name["strict_".len()..],
				_ => name
					.strip_suffix("_signed")
					.or_else(|| name.strip_suffix("_unsigned"))
					.unwrap_or(name),
			};
			if !SHARED.contains(&shared) || (int.signed && UNSIGNED.contains(&shared)) {
				return None;
			}
			args.push(Expr::lit(int.bits.to_string()));
			args.push(Expr::lit(int.signed.to_string()));
			let e = Expr::rt(super::camel(&format!("int_{shared}")), args);
			// What gives another type's integer is any integer to the runtime.
			return Some(match shared {
				"abs_diff" | "unsigned_abs" | "checked_signed_diff" => {
					let output = b.t.sig(call.def_id, call.args).output();
					Expr::As(Box::new(e), b.t.ty(output, call.span))
				}
				_ => e,
			});
		}
	};
	// wasm32 is little-endian, so the native bytes are the little-endian ones: the
	// big-endian bytes, the other way around. The caller may still hold the array it
	// reads, so it reverses a copy; the bytes it makes are its own.
	let args = match name {
		"from_le_bytes" | "from_ne_bytes" => args
			.into_iter()
			.map(|a| a.method("slice", Vec::new()).method("reverse", Vec::new()))
			.collect(),
		_ => args,
	};
	let e = int.method(method, args);
	Some(match name {
		"to_le_bytes" | "to_ne_bytes" => e.method("reverse", Vec::new()),
		_ => e,
	})
}

/// What a JS value of `ty` is through references and `NonZero`, which is its integer.
fn unwrapped<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Ty<'tcx> {
	let ty = ty.peel_refs();
	match ty.kind() {
		ty::Adt(adt, args) if Std::of(tcx, adt.did()) == Some(Std::NonZero) => args.type_at(0),
		_ => ty,
	}
}

/// The primitive JS compares a range's bounds of type `ty` as, through shared references
/// and a `NonZero`: not through a `&mut`, which may be an `rt.Place`.
fn compared<'tcx>(tcx: TyCtxt<'tcx>, mut ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
	while let ty::Ref(_, inner, ty::Mutability::Not) = ty.kind() {
		ty = *inner;
	}
	let ty = (!ty.is_ref()).then(|| unwrapped(tcx, ty))?;
	primitive(ty).then_some(ty)
}

fn primitive(ty: Ty<'_>) -> bool {
	matches!(ty.kind(), ty::Bool | ty::Int(_) | ty::Uint(_) | ty::Char) || super::float::is(ty)
}

/// A `Vec` of `elem`s as a slice: an `rt.ByteSlice` of a byte vector, or the array itself.
pub fn vec_view(v: Expr, elem: Ty<'_>) -> Expr {
	match elem.kind() {
		ty::Uint(ty::UintTy::U8) => Expr::rt("byteVecSlice", vec![v]),
		_ => v,
	}
}

/// The elements of a slice, array, or `Vec`, a `ty`, as a JS array or `Uint8Array`: a
/// view of a byte slice's or byte vector's, which the runtime's iterators and formatting
/// index.
pub fn elements(v: Expr, ty: Ty<'_>) -> Expr {
	match ty.peel_refs().kind() {
		ty::Slice(elem) if matches!(elem.kind(), ty::Uint(ty::UintTy::U8)) => Expr::rt("byteSliceView", vec![v]),
		_ if super::bytes_ref(ty) => Expr::rt("byteSliceView", vec![v]),
		ty::Adt(_, args)
			if args
				.types()
				.next()
				.is_some_and(|t| matches!(t.kind(), ty::Uint(ty::UintTy::U8))) =>
		{
			Expr::rt("byteVecView", vec![v])
		}
		_ => v,
	}
}

trait MethodCall {
	fn method(self, name: &str, args: Vec<Expr>) -> Expr;
}

impl MethodCall for Expr {
	fn method(self, name: &str, args: Vec<Expr>) -> Expr {
		self.member(name).call(args)
	}
}

/// `Default::default()` for a standard type, of its parts' defaults where it has parts.
pub fn default_value<'tcx>(b: &mut Body<'_, '_, 'tcx>, ty: Ty<'tcx>, span: Span, out: &mut Vec<Stmt>) -> Option<Expr> {
	let tcx = b.t.tcx;
	let part = |b: &mut Body<'_, '_, 'tcx>, ty: Ty<'tcx>, out: &mut Vec<Stmt>| {
		let f = super::collections::trait_fn(tcx, "Default", "default");
		b.call_def(f, tcx.mk_args(&[ty.into()]), Vec::new(), span, out)
	};
	Some(match ty.kind() {
		_ if nothing(tcx, ty) => Expr::undefined(),
		ty::Adt(adt, _) if empty_iter(tcx, adt.did()) => b.zero_sized_std(ty, Vec::new(), span).ok()?,
		// A list's iterators, over an empty list.
		ty::Adt(adt, _)
			if is_std(tcx, adt.did())
				&& matches!(
					std_path(tcx, adt.did())
						.iter()
						.map(String::as_str)
						.collect::<Vec<_>>()
						.as_slice(),
					["collections", "linked_list", "Iter" | "IterMut" | "IntoIter"]
				) =>
		{
			Expr::rt("listIter", vec![Expr::rt("listNew", Vec::new())])
		}
		ty::Tuple(parts) => Expr::Array(parts.iter().map(|t| part(b, t, out)).collect()),
		ty::Array(elem, n) => {
			let n = n.try_to_target_usize(tcx)?;
			match elem.kind() {
				ty::Uint(ty::UintTy::U8) => return Some(Expr::rt("bytes", vec![Expr::lit(n.to_string())])),
				// Whatever its element, which needn't have a default.
				_ if n == 0 => return Some(Expr::Array(Vec::new())),
				_ => {}
			}
			// Each element its own, as an atomic must be.
			let mut body = Vec::new();
			let item = part(b, *elem, &mut body);
			body.push(StmtKind::Return(Some(item.typed())).at(None));
			let item_ty = b.t.ty(*elem, span);
			Expr::ident("Array").member("from").call(vec![
				Expr::Object(vec![(crate::ts::Key::Name("length".into()), Expr::lit(n.to_string()))]),
				Expr::Arrow(Vec::new(), item_ty, body),
			])
		}
		ty::Bool => Expr::lit("false"),
		ty::Int(_) | ty::Uint(_) => Int::of(ty)?.lit(0),
		_ if let Some(float) = super::float::Float::of(ty) => float.lit(0.0),
		ty::Char => Expr::lit("0"),
		ty::Ref(_, inner, _) => match inner.kind() {
			ty::Slice(elem) if matches!(elem.kind(), ty::Uint(ty::UintTy::U8)) => Expr::rt(
				"byteSliceFrom",
				vec![Expr::New(Box::new(Expr::ident("Uint8Array")), vec![Expr::lit("0")])],
			),
			ty::Slice(_) => Expr::Array(Vec::new()),
			ty::Str => Expr::Str(String::new()),
			_ => return None,
		},
		ty::Adt(adt, args) => match Std::of(tcx, adt.did())? {
			Std::Option => b.t.none(args.type_at(0)),
			Std::Vec => match args.type_at(0).kind() {
				ty::Uint(ty::UintTy::U8) => Expr::New(Box::new(Expr::Rt("ByteVec".into())), Vec::new()),
				_ => Expr::Array(Vec::new()),
			},
			Std::String => Expr::Str(String::new()),
			Std::Cow if string(tcx, ty) => Expr::Str(String::new()),
			// An owned empty `Vec`.
			Std::Cow if let Some(elem) = cow_slice(tcx, ty) => match elem.kind() {
				ty::Uint(ty::UintTy::U8) => Expr::New(Box::new(Expr::Rt("ByteVec".into())), Vec::new()),
				_ => Expr::Array(Vec::new()),
			},
			Std::Duration => super::time::zero(),
			Std::Atomic => Expr::New(
				Box::new(Expr::Rt("Atomic".into())),
				vec![default_value(b, args.type_at(0), span, out)?],
			),
			Std::OnceCell | Std::OnceLock => super::task::once_new(b, ty),
			// An empty slice or text, its own.
			Std::Box if !args.type_at(0).is_sized(tcx, b.t.typing_env_pub()) => {
				return default_value(
					b,
					Ty::new_imm_ref(tcx, tcx.lifetimes.re_erased, args.type_at(0)),
					span,
					out,
				);
			}
			Std::Box | Std::Wrapping | Std::Saturating => part(b, args.type_at(0), out),
			Std::Arc => Expr::New(Box::new(Expr::Rt("Arc".into())), vec![part(b, args.type_at(0), out)]),
			Std::Mutex => Expr::New(Box::new(Expr::Rt("Mutex".into())), vec![part(b, args.type_at(0), out)]),
			Std::RefCell => Expr::New(
				Box::new(Expr::Rt("RefCell".into())),
				vec![part(b, args.type_at(0), out)],
			),
			Std::Cell => Expr::New(Box::new(Expr::Rt("Cell".into())), vec![part(b, args.type_at(0), out)]),
			Std::Weak => Expr::New(Box::new(Expr::Rt("Weak".into())), Vec::new()),
			_ => return None,
		},
		_ => return None,
	})
}

/// The text of a `fmt::Arguments` with only literal pieces, which writes it as is.
pub fn literal(parts: &Expr) -> Option<String> {
	let Expr::Array(parts) = parts else {
		return None;
	};
	parts
		.iter()
		.map(|part| match part {
			Expr::Str(s) => Some(s.as_str()),
			_ => None,
		})
		.collect()
}
