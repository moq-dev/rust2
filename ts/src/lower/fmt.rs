//! Formatting: `format_args!`, the formatting traits, and std's impls of them.
//!
//! The runtime ports `core::fmt`, so translated output matches native Rust's byte for
//! byte. A placeholder's value becomes `(f) => ...`, a function writing it to a
//! formatter, and `fmt::Arguments` an array of literal pieces and such functions. Each
//! call goes to the impl of the value's type directly, as monomorphization allows.

use rustc_hir::def_id::DefId;
use rustc_middle::ty::{self, Ty, TyCtxt};
use rustc_span::Span;

use super::body::Body;
use super::library::{self, Std};
use super::num::Int;
use crate::ts::{Expr, Key, Param, Stmt, Type};

/// A formatting trait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trait {
	Display,
	Debug,
	LowerHex,
	UpperHex,
	Octal,
	Binary,
	LowerExp,
	UpperExp,
}

impl Trait {
	/// The trait `did` is, if it is one of these.
	pub fn of(tcx: TyCtxt<'_>, did: DefId) -> Option<Self> {
		match library::std_path(tcx, did)
			.iter()
			.map(String::as_str)
			.collect::<Vec<_>>()
			.as_slice()
		{
			["fmt", "Display"] => Some(Self::Display),
			["fmt", "Debug"] => Some(Self::Debug),
			["fmt", "LowerHex"] => Some(Self::LowerHex),
			["fmt", "UpperHex"] => Some(Self::UpperHex),
			["fmt", "Octal"] => Some(Self::Octal),
			["fmt", "Binary"] => Some(Self::Binary),
			["fmt", "LowerExp"] => Some(Self::LowerExp),
			["fmt", "UpperExp"] => Some(Self::UpperExp),
			_ => None,
		}
	}

	fn name(self) -> &'static str {
		match self {
			Self::Display => "Display",
			Self::Debug => "Debug",
			Self::LowerHex => "LowerHex",
			Self::UpperHex => "UpperHex",
			Self::Octal => "Octal",
			Self::Binary => "Binary",
			Self::LowerExp => "LowerExp",
			Self::UpperExp => "UpperExp",
		}
	}

	/// The trait's `fmt` method.
	fn method(self, tcx: TyCtxt<'_>) -> Option<DefId> {
		let module = tcx.parent(tcx.get_diagnostic_item(rustc_span::sym::Debug)?);
		let child = tcx
			.module_children(module)
			.iter()
			.find(|child| child.ident.as_str() == self.name())?;
		let trait_id = child.res.opt_def_id()?;
		let method = tcx
			.associated_items(trait_id)
			.in_definition_order()
			.find(|item| item.is_fn())?;
		Some(method.def_id)
	}
}

/// What a `format_args!` argument's constructor, `Argument::new_display` and the like,
/// makes: a value written by a trait, or `None` for a width or precision, as in `{:1$}`.
/// Otherwise, the construct outside the subset that it is.
pub fn argument(name: &str) -> Result<Option<Trait>, &'static str> {
	Ok(Some(match name {
		"new_display" => Trait::Display,
		"new_debug" => Trait::Debug,
		"new_lower_hex" => Trait::LowerHex,
		"new_upper_hex" => Trait::UpperHex,
		"new_octal" => Trait::Octal,
		"new_binary" => Trait::Binary,
		"new_lower_exp" => Trait::LowerExp,
		"new_upper_exp" => Trait::UpperExp,
		"from_usize" => return Ok(None),
		"new_pointer" => return Err("`Pointer` formatting, as in `{:p}`,"),
		_ => return Err("an unexpected `format_args!` lowering"),
	}))
}

/// A formatter's type, for the parameters of the functions written here.
fn formatter() -> Type {
	Type::Rt("Formatter", Vec::new())
}

impl<'b, 'a, 'tcx> Body<'b, 'a, 'tcx> {
	/// The formatting trait of a `&dyn Debug` or the like.
	pub fn fmt_object(&self, ty: Ty<'tcx>) -> Option<Trait> {
		let ty::Ref(_, inner, rustc_ast::Mutability::Not) = ty.kind() else {
			return None;
		};
		let ty::Dynamic(preds, ..) = inner.kind() else {
			return None;
		};
		Trait::of(self.t.tcx, preds.principal_def_id()?)
	}

	/// `value`, a `ty`, written by `tr` to `f`, through the impl for `ty`.
	pub fn fmt_call(&mut self, tr: Trait, ty: Ty<'tcx>, value: Expr, f: Expr, span: Span) -> Expr {
		let tcx = self.t.tcx;
		let Some(method) = tr.method(tcx) else {
			return self.err(span, format!("no `{}::fmt`", tr.name()));
		};
		let args = tcx.mk_args(&[ty.into()]);
		let mut pre = Vec::new();
		let call = self.call_def(method, args, vec![value, f], span, &mut pre);
		match pre.is_empty() {
			true => call,
			false => self.err(span, format!("`{}` of `{ty}` needs statements", tr.name())),
		}
	}

	/// A function writing a `ty` by `tr`: `(value, f) => ...`, or the runtime's own.
	pub fn fmt_fn(&mut self, tr: Trait, ty: Ty<'tcx>, span: Span) -> Expr {
		let body = self.fmt_call(tr, ty, Expr::ident("value"), Expr::ident("f"), span);
		if let Expr::Call(callee, args) = &body
			&& let [Expr::Ident(value), Expr::Ident(f)] = args.as_slice()
			&& value == "value"
			&& f == "f"
			&& runtime(callee)
		{
			return (**callee).clone();
		}
		let params = vec![
			Param {
				name: "value".into(),
				ty: Some(self.t.ty(ty, span)),
			},
			Param {
				name: "f".into(),
				ty: Some(formatter()),
			},
		];
		Expr::Lambda(params, Box::new(body))
	}

	/// `value`, a `ty`, ready to be written by `tr`: `(f) => ...`. It is read now, once,
	/// as a placeholder's or a `&dyn Debug`'s value is.
	pub fn fmt_value(&mut self, tr: Trait, ty: Ty<'tcx>, value: Expr, span: Span, out: &mut Vec<Stmt>) -> Expr {
		let value = match value.is_path() {
			true => value,
			false => self.spill(value, "arg", out),
		};
		self.thunk(tr, ty, value, span)
	}

	/// `(f) => ...` writing `value`, which must be [`pure`].
	fn thunk(&mut self, tr: Trait, ty: Ty<'tcx>, value: Expr, span: Span) -> Expr {
		let f = match mentions(&value, "f") {
			true => self.fresh("f"),
			false => "f".to_string(),
		};
		let body = self.fmt_call(tr, ty, value, Expr::ident(&f), span);
		Expr::Lambda(
			vec![Param {
				name: f,
				ty: Some(formatter()),
			}],
			Box::new(body),
		)
	}
}

/// Whether `e` is a runtime function, which reads no `this`.
fn runtime(e: &Expr) -> bool {
	matches!(e, Expr::Rt(_))
}

/// Whether the path `e` reads the variable `name`.
fn mentions(e: &Expr, name: &str) -> bool {
	match e {
		Expr::Ident(ident) => ident == name,
		Expr::Member(base, _) | Expr::Located(_, base) => mentions(base, name),
		Expr::Index(base, index) => mentions(base, name) || mentions(index, name),
		_ => false,
	}
}

/// Whether reading `e` again gives the same value and does nothing else: a path, or an
/// element of one.
fn pure(e: &Expr) -> bool {
	match e {
		Expr::Index(base, index) => pure(base) && index.is_constant(),
		e => e.is_path(),
	}
}

/// std's impl of `tr` for `ty`, writing `value` to `f`; `None` if it has none here.
pub fn std_impl<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	tr: Trait,
	ty: Ty<'tcx>,
	value: Expr,
	f: Expr,
	span: Span,
	out: &mut Vec<Stmt>,
) -> Option<Expr> {
	let tcx = b.t.tcx;
	let rt = |name: &str, args: Vec<Expr>| Expr::rt_of("fmt", name).call(args);
	let text = matches!(tr, Trait::Display | Trait::Debug);
	Some(match ty.kind() {
		// A reference to an array of bytes is a slice of them, written as a slice is.
		ty::Ref(_, inner, _) if super::byte_array(*inner) => {
			b.fmt_call(tr, Ty::new_slice(tcx, tcx.types.u8), value, f, span)
		}
		ty::Ref(_, inner, _) => {
			let value = b.t.pointee_value(ty, value);
			b.fmt_call(tr, *inner, value, f, span)
		}
		ty::Int(_) | ty::Uint(_) => {
			let method = match tr {
				Trait::Display => "display",
				Trait::Debug => "debug",
				Trait::LowerHex => "lowerHex",
				Trait::UpperHex => "upperHex",
				Trait::Octal => "octal",
				Trait::Binary => "binary",
				Trait::LowerExp => "lowerExp",
				Trait::UpperExp => "upperExp",
			};
			Int::of(ty)?.method(method, vec![value, f])
		}
		_ if let Some(float) = super::float::Float::of(ty) => {
			let method = match tr {
				Trait::Display => "display",
				Trait::Debug => "debug",
				Trait::LowerExp => "lowerExp",
				Trait::UpperExp => "upperExp",
				_ => return None,
			};
			Expr::rt_of(float.name(), method).call(vec![value, f])
		}
		// `!`, and `Infallible`, which is it: never written, as none exists.
		ty::Never if text => Expr::rt("unreachable", vec![value]),
		ty::Bool if text => rt("bool", vec![value, f]),
		ty::Char if tr == Trait::Display => rt("charDisplay", vec![value, f]),
		ty::Char if tr == Trait::Debug => rt("charDebug", vec![value, f]),
		ty::Str if tr == Trait::Display => Expr::rt("fmtPad", vec![f, value]),
		ty::Str if tr == Trait::Debug => rt("str", vec![value, f]),
		ty::Tuple(tys) if tys.is_empty() && tr == Trait::Debug => rt("unit", vec![value, f]),
		ty::Tuple(tys) if tr == Trait::Debug => {
			// Each element's function reads the tuple.
			let value = match pure(&value) {
				true => value,
				false => b.spill(value, "arg", out),
			};
			let fields = tys
				.iter()
				.enumerate()
				.map(|(i, t)| b.thunk(tr, t, value.clone().index(Expr::lit(i.to_string())), span))
				.collect();
			rt("tuple", vec![Expr::Array(fields), f])
		}
		ty::Array(elem, _) | ty::Slice(elem) if tr == Trait::Debug => {
			let item = b.fmt_fn(tr, *elem, span);
			rt("list", vec![library::elements(value, ty), f, item])
		}
		// A `&dyn Debug` is already a function writing its value.
		ty::Dynamic(preds, ..) if preds.principal_def_id().and_then(|did| Trait::of(tcx, did)) == Some(tr) => {
			value.call(vec![f])
		}
		// Unpadded, naming its type as `type_name` does.
		ty::Adt(adt, args) if tr == Trait::Debug && adt.is_phantom_data() => {
			let name = rustc_const_eval::util::type_name(tcx, args.type_at(0));
			rt("writeStr", vec![f, Expr::Str(format!("PhantomData<{name}>"))])
		}
		// Each its name alone, unpadded.
		ty::Adt(adt, _)
			if tr == Trait::Debug && (library::pinned(tcx, adt.did()) || library::empty_iter(tcx, adt.did())) =>
		{
			rt("writeStr", vec![f, Expr::Str(tcx.item_name(adt.did()).to_string())])
		}
		// An ASCII escape writes what it has left, unpadded, and `Debug` only its name.
		ty::Adt(adt, _)
			if matches!(
				library::std_path(tcx, adt.did())
					.iter()
					.map(String::as_str)
					.collect::<Vec<_>>()
					.as_slice(),
				["ascii", "EscapeDefault"] | ["slice", "ascii", "EscapeAscii"]
			) =>
		{
			match tr {
				Trait::Display => Expr::rt("fmtEscapeAscii", vec![value, f]),
				Trait::Debug => rt(
					"writeStr",
					vec![f, Expr::Str(format!("{} {{ .. }}", tcx.item_name(adt.did())))],
				),
				_ => return None,
			}
		}
		ty::Adt(adt, args)
			if tr == Trait::Debug
				&& let Some(e) = iter_debug(b, adt.did(), args, &value, &f, span) =>
		{
			e
		}
		ty::Adt(adt, args) => match (Std::of(tcx, adt.did())?, tr) {
			(Std::String | Std::Cow, Trait::Display) if library::string(tcx, ty) => Expr::rt("fmtPad", vec![f, value]),
			(Std::String | Std::Cow, Trait::Debug) if library::string(tcx, ty) => rt("str", vec![value, f]),
			// A `NonZero`, a `Wrapping`, and a `Saturating` write their integer, by each trait.
			(Std::Box | Std::NonZero | Std::Wrapping | Std::Saturating, _) => {
				b.fmt_call(tr, args.type_at(0), value, f, span)
			}
			// A `Cow<[T]>` writes its slice.
			(Std::Cow, Trait::Debug) if let Some(elem) = library::cow_slice(tcx, ty) => {
				b.fmt_call(tr, Ty::new_slice(tcx, elem), library::cow_deref(value, elem), f, span)
			}
			(Std::Arc, Trait::Display | Trait::Debug) => {
				b.fmt_call(tr, args.type_at(0), value.member("value"), f, span)
			}
			// Each bound is written with the range's options.
			(
				std @ (Std::Range | Std::RangeInclusive | Std::RangeFrom | Std::RangeTo | Std::RangeToInclusive),
				Trait::Debug,
			) => {
				let bound = b.fmt_fn(tr, args.type_at(0), span);
				let of = match std {
					Std::Range => "range",
					Std::RangeInclusive => "rangeInclusive",
					Std::RangeFrom => "rangeFrom",
					Std::RangeTo => "rangeTo",
					_ => "rangeToInclusive",
				};
				rt(of, vec![value, f, bound])
			}
			(Std::RangeFull, Trait::Debug) => rt("rangeFull", vec![value, f]),
			(Std::Bound, Trait::Debug) => {
				let held = b.fmt_fn(tr, args.type_at(0), span);
				rt("bound", vec![value, f, held])
			}
			(Std::Ordering, Trait::Debug) => rt("ordering", vec![value, f]),
			// Erased to what it holds.
			(Std::Reverse, Trait::Debug) => {
				let held = b.fmt_value(tr, args.type_at(0), value, span, out);
				rt(
					"debugTupleFields",
					vec![f, Expr::Str("Reverse".into()), Expr::Array(vec![held])],
				)
			}
			(Std::ControlFlow, Trait::Debug) => {
				let go = b.fmt_fn(tr, args.type_at(1), span);
				let stop = b.fmt_fn(tr, args.type_at(0), span);
				match library::Flow::of(tcx, args) {
					library::Flow::Other => rt("controlFlow", vec![value, f, go, stop]),
					library::Flow::Result => rt("controlFlow", vec![value, f, go, stop, Expr::lit("true")]),
					library::Flow::Option => {
						let none = b.t.none_arg(args.type_at(1));
						rt(
							"controlFlowOption",
							[vec![value, f, go, stop], none.into_iter().collect()].concat(),
						)
					}
				}
			}
			(Std::Vec, Trait::Debug) => {
				let item = b.fmt_fn(tr, args.type_at(0), span);
				rt("list", vec![library::elements(value, ty), f, item])
			}
			(Std::BinaryHeap, Trait::Debug) => {
				let item = b.fmt_fn(tr, args.type_at(0), span);
				rt("list", vec![value.member("d"), f, item])
			}
			// Each writes its items as its iterator reads them: a deque from its head.
			(Std::VecDeque, Trait::Debug) => {
				let item = b.fmt_fn(tr, args.type_at(0), span);
				rt("list", vec![Expr::rt("dequeIter", vec![value]), f, item])
			}
			(Std::LinkedList, Trait::Debug) => {
				let item = b.fmt_fn(tr, args.type_at(0), span);
				rt("list", vec![Expr::rt("listIter", vec![value]), f, item])
			}
			(std @ (Std::HashSet | Std::BTreeSet), Trait::Debug) => {
				let item = b.fmt_fn(tr, args.type_at(0), span);
				let keys = if std == Std::HashSet { "hashKeys" } else { "btreeKeys" };
				rt("set", vec![Expr::rt(keys, vec![value]), f, item])
			}
			(std @ (Std::HashMap | Std::BTreeMap), Trait::Debug) => {
				let key = b.fmt_fn(tr, args.type_at(0), span);
				let val = b.fmt_fn(tr, args.type_at(1), span);
				let entries = if std == Std::HashMap { "hashIter" } else { "btreeIter" };
				rt("map", vec![Expr::rt(entries, vec![value]), f, key, val])
			}
			(Std::Option, Trait::Debug) => {
				let some = b.fmt_fn(tr, args.type_at(0), span);
				let none = b.t.none_arg(args.type_at(0));
				rt("option", [vec![value, f, some], none.into_iter().collect()].concat())
			}
			(Std::Result, Trait::Debug) => {
				let ok = b.fmt_fn(tr, args.type_at(0), span);
				let err = b.fmt_fn(tr, args.type_at(1), span);
				rt("result", vec![value, f, ok, err])
			}
			(Std::Arguments, Trait::Display | Trait::Debug) => rt("arguments", vec![value, f]),
			(Std::FmtError, Trait::Debug) => rt("errorDebug", vec![value, f]),
			(Std::PoisonError, Trait::Debug) => rt("poisonErrorDebug", vec![value, f]),
			(Std::PoisonError, Trait::Display) => rt("poisonErrorDisplay", vec![value, f]),
			(Std::FmtError, Trait::Display) => rt("errorDisplay", vec![value, f]),
			(Std::TryFromIntError, Trait::Debug) => rt("tryFromIntErrorDebug", vec![value, f]),
			(Std::TryFromIntError, Trait::Display) => rt("tryFromIntErrorDisplay", vec![value, f]),
			(Std::ParseIntError, Trait::Debug) => rt("parseIntErrorDebug", vec![value, f]),
			(Std::ParseIntError, Trait::Display) => rt("parseIntErrorDisplay", vec![value, f]),
			(Std::ErrorKind, Trait::Debug) => rt("writeStr", vec![f, value]),
			(Std::ParseBoolError, Trait::Debug) => rt("parseBoolErrorDebug", vec![value, f]),
			(Std::ParseBoolError, Trait::Display) => rt("parseBoolErrorDisplay", vec![value, f]),
			(Std::BorrowError, Trait::Debug) => Expr::rt("borrowErrorDebug", vec![value, f]),
			(Std::BorrowError, Trait::Display) => Expr::rt("borrowErrorDisplay", vec![value, f]),
			(Std::BorrowMutError, Trait::Debug) => Expr::rt("borrowMutErrorDebug", vec![value, f]),
			(Std::BorrowMutError, Trait::Display) => Expr::rt("borrowMutErrorDisplay", vec![value, f]),
			// What it holds, or `<uninit>`.
			(std @ (Std::OnceCell | Std::OnceLock), Trait::Debug) => {
				let held = b.fmt_fn(tr, args.type_at(0), span);
				let none = b.t.none_arg(args.type_at(0));
				let name = Expr::Str(format!("{std:?}"));
				Expr::rt(
					"onceDebug",
					[vec![value, f, name, held], none.into_iter().collect()].concat(),
				)
			}
			(Std::ParseCharError, Trait::Debug) => rt("parseCharErrorDebug", vec![value, f]),
			(Std::ParseCharError, Trait::Display) => rt("parseCharErrorDisplay", vec![value, f]),
			(Std::ParseFloatError, Trait::Debug) => rt("parseFloatErrorDebug", vec![value, f]),
			(Std::ParseFloatError, Trait::Display) => rt("parseFloatErrorDisplay", vec![value, f]),
			(Std::TryFromFloatSecsError, Trait::Debug) => rt("tryFromFloatSecsErrorDebug", vec![value, f]),
			(Std::TryFromFloatSecsError, Trait::Display) => rt("tryFromFloatSecsErrorDisplay", vec![value, f]),
			(Std::FromUtf8Error, Trait::Debug) => rt("fromUtf8ErrorDebug", vec![value, f]),
			(Std::FromUtf8Error, Trait::Display) => rt("fromUtf8ErrorDisplay", vec![value, f]),
			(Std::Utf8Error, Trait::Debug) => rt("utf8ErrorDebug", vec![value, f]),
			(Std::Utf8Error, Trait::Display) => rt("utf8ErrorDisplay", vec![value, f]),
			(Std::Duration, Trait::Debug) => Expr::rt("durationDebug", vec![value, f]),
			(Std::Bytes, Trait::Debug) => Expr::rt("bytesDebug", vec![value, f]),
			(Std::BytesMut, Trait::Debug) => Expr::rt("bytesDebug", vec![Expr::rt("bytesMutView", vec![value]), f]),
			_ => return None,
		},
		_ => return None,
	})
}

/// `Debug` of one of std's iterators that lists the items it has left, as std writes
/// them, or `None` for another type. A deque's `Iter` and `IterMut` write the two halves of
/// its ring buffer, which the runtime's deque splits elsewhere than std's, and its `Drain`
/// its indices, so they stay refused.
fn iter_debug<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	did: DefId,
	args: ty::GenericArgsRef<'tcx>,
	value: &Expr,
	f: &Expr,
	span: Span,
) -> Option<Expr> {
	/// What std writes around the list: a tuple struct's name, or a struct's and its field's.
	enum Wrap {
		Tuple(&'static str),
		Struct(&'static str, &'static str),
	}
	/// What it lists: a type argument, the map entry of the first two, or a `BTreeSet`'s
	/// entry of its key and `SetValZST`.
	enum Item {
		Arg(usize),
		Entry,
		SetVal,
		Byte,
	}
	use {Item::*, Wrap::*};
	let tcx = b.t.tcx;
	let path = library::std_path(tcx, did);
	let path: Vec<&str> = path.iter().map(String::as_str).collect();
	let (item, wraps): (Item, &[Wrap]) = match path.as_slice() {
		["str", "iter", "Chars"] => return Some(Expr::rt("fmtChars", vec![value.clone(), f.clone()])),
		// The list it has left, then its length.
		["collections", "linked_list", name @ ("Iter" | "IterMut")] => {
			let debug = b.fmt_fn(Trait::Debug, args.types().next()?, span);
			let len = b.fmt_fn(Trait::Debug, tcx.types.usize, span);
			let call = vec![value.clone(), f.clone(), debug, len, Expr::Str((*name).into())];
			return Some(Expr::rt("fmtListIter", call));
		}
		["str", "iter", "Bytes"] => (Byte, &[Tuple("Bytes"), Struct("Copied", "it"), Tuple("Iter")]),
		["slice", "iter", "Iter"]
		| ["collections", "binary_heap", "Iter"]
		| ["collections", "btree", "set", "Iter"] => (Arg(0), &[Tuple("Iter")]),
		["slice", "iter", "IterMut"] => (Arg(0), &[Tuple("IterMut")]),
		["vec", "into_iter", "IntoIter"]
		| ["collections", "binary_heap", "IntoIter"]
		| ["collections", "vec_deque", "into_iter", "IntoIter"]
		| ["collections", "linked_list", "IntoIter"] => (Arg(0), &[Tuple("IntoIter")]),
		["vec", "drain", "Drain"] => (Arg(0), &[Tuple("Drain")]),
		["collections", "binary_heap", "Drain"] => (Arg(0), &[Struct("Drain", "iter"), Tuple("Drain")]),
		["collections", "btree", "set", "Range"] => (SetVal, &[Struct("Range", "iter")]),
		["collections", "btree", "set", "IntoIter"] => (SetVal, &[Struct("IntoIter", "iter")]),
		[
			"collections",
			"btree",
			"map",
			"Iter" | "IterMut" | "Range" | "RangeMut" | "IntoIter",
		]
		| ["collections", "hash", "map", "Iter" | "IterMut" | "IntoIter" | "Drain"] => (Entry, &[]),
		["collections", "btree" | "hash", "map", "Keys" | "IntoKeys"]
		| ["collections", "hash", "set", "Iter" | "IntoIter" | "Drain"] => (Arg(0), &[]),
		[
			"collections",
			"btree" | "hash",
			"map",
			"Values" | "ValuesMut" | "IntoValues",
		] => (Arg(1), &[]),
		_ => return None,
	};
	let arg = |n: usize| args.types().nth(n);
	let debug = match item {
		Arg(n) => b.fmt_fn(Trait::Debug, arg(n)?, span),
		Entry => b.fmt_fn(Trait::Debug, Ty::new_tup(tcx, &[arg(0)?, arg(1)?]), span),
		SetVal => Expr::rt("fmtSetVal", vec![b.fmt_fn(Trait::Debug, arg(0)?, span)]),
		Byte => b.fmt_fn(Trait::Debug, tcx.types.u8, span),
	};
	let wraps: Vec<Expr> = (wraps.iter())
		.map(|wrap| match *wrap {
			Tuple(name) => Expr::Str(name.into()),
			Struct(name, field) => Expr::Array(vec![Expr::Str(name.into()), Expr::Str(field.into())]),
		})
		.collect();
	let mut call = vec![value.clone(), f.clone(), debug];
	if !wraps.is_empty() {
		call.push(Expr::Array(wraps));
	}
	Some(Expr::rt("fmtIter", call))
}

/// One part of a `format_args!` template.
pub enum Piece {
	Lit(String),
	/// A placeholder writing the `index`th argument.
	Arg {
		index: usize,
		spec: Spec,
	},
}

/// A placeholder's options, from `FormattingOptions`' flags.
pub struct Spec {
	fill: char,
	align: Option<char>,
	plus: bool,
	alternate: bool,
	zero: bool,
	debug_hex: Option<char>,
	width: Option<Count>,
	precision: Option<Count>,
}

/// A width or precision: in the template, or the `index`th argument's.
#[derive(Clone, Copy)]
pub enum Count {
	Is(u16),
	Arg(usize),
}

impl Spec {
	/// The runtime's `Spec` object, or `None` for the defaults. `arg` reads an argument.
	pub fn object(&self, arg: impl Fn(usize) -> Expr) -> Option<Expr> {
		let count = |count: Count| match count {
			Count::Is(n) => Expr::lit(n.to_string()),
			Count::Arg(index) => Expr::rt("fmtCountOf", vec![arg(index)]),
		};
		let mut props = Vec::new();
		if self.fill != ' ' {
			props.push(("fill", Expr::Str(self.fill.to_string())));
		}
		if let Some(align) = self.align {
			props.push(("align", Expr::Str(align.to_string())));
		}
		if self.plus {
			props.push(("plus", Expr::lit("true")));
		}
		if self.alternate {
			props.push(("alternate", Expr::lit("true")));
		}
		if self.zero {
			props.push(("zero", Expr::lit("true")));
		}
		if let Some(hex) = self.debug_hex {
			props.push(("debugHex", Expr::Str(hex.to_string())));
		}
		if let Some(width) = self.width {
			props.push(("width", count(width)));
		}
		if let Some(precision) = self.precision {
			props.push(("precision", count(precision)));
		}
		let props: Vec<(Key, Expr)> = props.into_iter().map(|(k, v)| (Key::Name(k.into()), v)).collect();
		(!props.is_empty()).then_some(Expr::Object(props))
	}
}

// `FormattingOptions`' flags, as `core::fmt` lays them out.
const SIGN_PLUS: u32 = 1 << 21;
const ALTERNATE: u32 = 1 << 23;
const ZERO_PAD: u32 = 1 << 24;
const DEBUG_LOWER_HEX: u32 = 1 << 25;
const DEBUG_UPPER_HEX: u32 = 1 << 26;
const WIDTH: u32 = 1 << 27;
const PRECISION: u32 = 1 << 28;
const FILL: u32 = (1 << 21) - 1;
const DEFAULT_FLAGS: u32 = ' ' as u32 | 3 << 29;

/// Decode a template as `core::fmt::write` reads it: literal pieces, and placeholders
/// with their argument and options. The `-` flag is dropped, as std's impls ignore it.
pub fn decode(template: &[u8]) -> Result<Vec<Piece>, &'static str> {
	const TRUNCATED: &str = "a truncated template";
	let byte = |i: usize| template.get(i).copied().ok_or(TRUNCATED);
	let u16_at = |i: usize| Ok::<_, &str>(u16::from_le_bytes([byte(i)?, byte(i + 1)?]));
	let lit = |start: usize, len: usize| {
		let bytes = template.get(start..start + len).ok_or(TRUNCATED)?;
		String::from_utf8(bytes.to_vec()).map_err(|_| "a template piece that is not UTF-8")
	};
	let mut pieces = Vec::new();
	let mut i = 0;
	let mut next = 0;
	loop {
		let n = byte(i)?;
		i += 1;
		match n {
			0 => return Ok(pieces),
			1..=0x7f => {
				pieces.push(Piece::Lit(lit(i, n.into())?));
				i += usize::from(n);
			}
			0x80 => {
				let len = usize::from(u16_at(i)?);
				pieces.push(Piece::Lit(lit(i + 2, len)?));
				i += 2 + len;
			}
			0xc0..=0xff => {
				let mut flags = DEFAULT_FLAGS;
				let mut width = 0;
				let mut precision = 0;
				if n & 1 != 0 {
					flags = u32::from_le_bytes([byte(i)?, byte(i + 1)?, byte(i + 2)?, byte(i + 3)?]);
					i += 4;
				}
				if n & 2 != 0 {
					width = u16_at(i)?;
					i += 2;
				}
				if n & 4 != 0 {
					precision = u16_at(i)?;
					i += 2;
				}
				if n & 8 != 0 {
					next = usize::from(u16_at(i)?);
					i += 2;
				}
				// An indirect count is the index of the argument that holds it.
				let count = |set: u32, indirect: u8, value: u16| {
					(flags & set != 0).then_some(match n & indirect != 0 {
						true => Count::Arg(value.into()),
						false => Count::Is(value),
					})
				};
				let spec = Spec {
					fill: char::from_u32(flags & FILL).ok_or("a fill that is not a char")?,
					align: match flags >> 29 & 3 {
						0 => Some('<'),
						1 => Some('>'),
						2 => Some('^'),
						_ => None,
					},
					plus: flags & SIGN_PLUS != 0,
					alternate: flags & ALTERNATE != 0,
					zero: flags & ZERO_PAD != 0,
					debug_hex: match flags {
						f if f & DEBUG_LOWER_HEX != 0 => Some('x'),
						f if f & DEBUG_UPPER_HEX != 0 => Some('X'),
						_ => None,
					},
					width: count(WIDTH, 16, width),
					precision: count(PRECISION, 32, precision),
				};
				pieces.push(Piece::Arg { index: next, spec });
				next += 1;
			}
			_ => return Err("an unknown template byte"),
		}
	}
}
