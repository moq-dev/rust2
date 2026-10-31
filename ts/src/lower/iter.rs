//! Iterators, as `rt/iter.ts` gives them: std's iterators are `rt.Iter` objects whose
//! `next` returns an item or `rt.DONE`, and whose adaptors and consumers are runtime
//! functions taking the iterator first: `rt.iterMap(it, f)`. The crate's own `impl
//! Iterator` keeps that protocol in its `next`, and joins std's adaptors through
//! `rt.iterFrom`.

use rustc_hir::def_id::DefId;
use rustc_middle::ty::{self, Instance, Ty, TyCtxt, TypeVisitableExt};
use rustc_span::Span;

use super::Translator;
use super::body::Body;
use super::library::{self, Call, Std};
use super::num::Int;
use crate::ts::{Expr, Key, Param, Type};

/// `Iterator::next`, and the trait's `Item`.
fn trait_items(tcx: ty::TyCtxt<'_>) -> Option<(DefId, DefId)> {
	let iterator = tcx.get_diagnostic_item(rustc_span::sym::Iterator)?;
	let items = tcx.associated_items(iterator);
	let next = items.filter_by_name_unhygienic(rustc_span::sym::next).next()?.def_id;
	let item = items.filter_by_name_unhygienic(rustc_span::sym::Item).next()?.def_id;
	Some((next, item))
}

/// `Iterator::next`.
pub fn next_fn(tcx: ty::TyCtxt<'_>) -> Option<DefId> {
	Some(trait_items(tcx)?.0)
}

/// Whether `def_id` is `Iterator::next`, or an impl's.
pub fn is_next(tcx: ty::TyCtxt<'_>, def_id: DefId) -> bool {
	let Some((next, _)) = trait_items(tcx) else {
		return false;
	};
	def_id == next || tcx.trait_item_of(def_id) == Some(next)
}

impl<'tcx> Translator<'_, 'tcx> {
	/// `Iterator::Item` of `ty`, if `ty` is an iterator. A type over parameters, which a
	/// generic type's fields and a census have, may leave it a projection.
	pub fn iter_item(&self, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
		let tcx = self.tcx;
		let (_, item) = trait_items(tcx)?;
		let projection = Ty::new_projection(tcx, ty::IsRigid::No, item, [ty]);
		tcx.try_normalize_erasing_regions(self.typing_env_pub(), ty::Unnormalized::new_wip(projection))
			.ok()
	}
}

/// The crate's own iterator a `ty` is, through any references: its `next`, whose
/// instance the translation names.
fn own_next<'tcx>(b: &Body<'_, '_, 'tcx>, ty: Ty<'tcx>) -> Option<Instance<'tcx>> {
	let tcx = b.t.tcx;
	let ty = ty.peel_refs();
	// Over parameters, only a census's instance, which never runs, would call it.
	if !matches!(ty.kind(), ty::Adt(adt, _) if b.t.own(adt.did())) || ty.has_param() || ty.has_aliases() {
		return None;
	}
	let (next, _) = trait_items(tcx)?;
	Instance::try_resolve(tcx, b.t.typing_env_pub(), next, tcx.mk_args(&[ty.into()])).ok()?
}

/// Refuse an `impl Iterator` of the crate, whose `next` is `next_fn`, that std's adaptors
/// would read other than through `next`: one that overrides `fold`, `nth`, or `size_hint`,
/// which they call in its place or beside it, that has a `next_back` or `len`, which this
/// protocol lacks, or that is a `FusedIterator`, which `fuse` reads on past its end.
pub fn check_impl(t: &mut Translator<'_, '_>, next_fn: DefId) {
	let tcx = t.tcx;
	let Some(imp) = tcx.impl_of_assoc(next_fn) else {
		return;
	};
	for item in tcx.associated_items(imp).in_definition_order() {
		let name = item.name();
		if item.is_fn() && name != rustc_span::sym::next {
			t.err(
				tcx.def_span(item.def_id),
				format!("overriding `Iterator::{name}`, which std's adaptors call in place of `next` or beside it"),
			);
		}
	}
	let ty::Adt(adt, _) = tcx.type_of(imp).instantiate_identity().skip_norm_wip().kind() else {
		return;
	};
	// Only the type's crate can implement std's traits for it.
	for &other in tcx.trait_impls_in_crate(adt.did().krate) {
		let self_ty = tcx.type_of(other).instantiate_identity().skip_norm_wip();
		if !matches!(self_ty.kind(), ty::Adt(other, _) if other.did() == adt.did()) {
			continue;
		}
		let path = library::std_path(tcx, tcx.impl_trait_id(other));
		if let Some(name) = path.last().filter(|n| {
			matches!(
				n.as_str(),
				"DoubleEndedIterator" | "ExactSizeIterator" | "FusedIterator"
			)
		}) {
			t.err(
				tcx.def_span(other),
				format!("an iterator of the crate implementing `{name}`, which is not supported yet"),
			);
		}
	}
}

/// `iter.next()` by the protocol, for an iterator of type `ty`: an item or `rt.DONE`.
pub fn next<'tcx>(b: &mut Body<'_, '_, 'tcx>, ty: Ty<'tcx>, iter: Expr) -> Expr {
	match own_next(b, ty) {
		Some(instance) => {
			let sym = b.t.instance(instance);
			Expr::Sym(sym).call(vec![iter])
		}
		None => iter.member("next").call(Vec::new()),
	}
}

/// What `IntoIterator::into_iter` makes of `value`, a `ty`: an `rt.Iter`, or `None` for a
/// type without a mapping. One that owns what it yields, by value, is given what drops
/// each, and the crate's own iterator what drops it.
pub fn into_iter<'tcx>(b: &mut Body<'_, '_, 'tcx>, ty: Ty<'tcx>, value: Expr, span: Span) -> Option<Expr> {
	let tcx = b.t.tcx;
	let (ty, value) = peel_mut(ty, value);
	let (ty, value) = match unbox_ref(tcx, ty) {
		unboxed if unboxed != ty => (unboxed, b.t.pointee_value(ty, value)),
		_ => (ty, value),
	};
	// A boxed slice owns its elements, as the `Vec` std's `into_iter` makes of it does.
	let ty = library::box_slice(tcx, ty).map_or(ty, |elem| Ty::new_slice(tcx, elem));
	let owned = !ty.is_ref();
	if b.t.iter_item(ty).is_some() {
		not_bound(b, ty, span);
		return Some(match own_next(b, ty) {
			Some(instance) => {
				let sym = b.t.instance(instance);
				let param_ty = b.t.ty(ty.peel_refs(), span);
				let it = "it".to_string();
				let next = Expr::Sym(sym).call(vec![Expr::ident(&it)]);
				let param = Param {
					name: it,
					ty: Some(param_ty),
				};
				let mut args = vec![value, Expr::Lambda(vec![param], Box::new(next))];
				if owned && b.t.drops(ty) {
					args.push(b.t.dropper(ty, span));
				}
				Expr::rt("iterFrom", args)
			}
			// One borrowed by `&mut`, which std's adaptors may drop once it ends, but which
			// is its owner's to drop, and which a consumer reads on through the borrow
			// rather than by its by-value overrides, as `Chain::count` ends both halves.
			None if !owned => Expr::rt("iterBorrow", vec![value]),
			None => value,
		});
	}
	// What drops each item of one that owns them.
	let each = |b: &mut Body<'_, '_, 'tcx>, elem: Ty<'tcx>, mut args: Vec<Expr>| {
		if owned && b.t.drops(elem) {
			args.push(b.t.dropper(elem, span));
		}
		args
	};
	// Items borrowed from what `ty` points at.
	let items = |iter: Expr, elem: Ty<'tcx>| match ty.is_ref() {
		true => by_ref(iter, elem),
		false => iter,
	};
	// Elements a `&mut` lends as their places, which JS copies.
	let places = |b: &Body<'_, '_, 'tcx>, elem: Ty<'tcx>| {
		ty.ref_mutability() == Some(ty::Mutability::Mut) && b.t.needs_place(elem)
	};
	match ty.peel_refs().kind() {
		ty::Slice(elem) | ty::Array(elem, _) if places(b, *elem) => {
			Some(Expr::rt("iterSliceMut", vec![library::elements(value, ty)]))
		}
		ty::Slice(elem) | ty::Array(elem, _) => {
			let values = each(b, *elem, vec![library::elements(value, ty)]);
			Some(items(Expr::rt("iterSlice", values), *elem))
		}
		ty::Adt(adt, args) => match Std::of(tcx, adt.did())? {
			Std::Vec if places(b, args.type_at(0)) => {
				Some(Expr::rt("iterSliceMut", vec![library::elements(value, ty)]))
			}
			Std::Vec => {
				let values = each(b, args.type_at(0), vec![library::elements(value, ty)]);
				Some(items(Expr::rt("iterSlice", values), args.type_at(0)))
			}
			// A `&mut` lends the payload JS copies as the `Option`'s own place, while it has one.
			Std::Option if places(b, args.type_at(0)) => {
				let mut values = vec![value];
				values.extend(b.t.none_arg(args.type_at(0)));
				Some(Expr::rt("iterOption", vec![Expr::rt("optionAsMut", values)]))
			}
			// A `&mut` to one of another payload is its place, and yields the payload itself.
			Std::Option => {
				let inner = args.type_at(0);
				let mut values = each(b, inner, vec![b.t.pointee_value(ty, value)]);
				if let Some(none) = b.t.none_arg(inner) {
					values.resize(2, Expr::undefined());
					values.push(none);
				}
				Some(Expr::rt("iterOption", values))
			}
			_ => super::collections::into_iter(b, ty, value, span),
		},
		_ => None,
	}
}

/// `iter`, over the elements of a slice or `Vec` of `elem`s, yielding references to them:
/// a slice of each array of bytes.
pub(super) fn by_ref(iter: Expr, elem: Ty<'_>) -> Expr {
	match super::byte_array(elem) {
		true => Expr::rt("iterLend", vec![iter, Expr::Rt("byteSliceFrom".into())]),
		false => iter,
	}
}

/// A function doing `into_iter` on each `ty`, or `None` where it is the value itself.
fn into_fn<'tcx>(b: &mut Body<'_, '_, 'tcx>, ty: Ty<'tcx>, span: Span) -> Option<Option<Expr>> {
	let body = into_iter(b, ty, Expr::ident("value"), span)?;
	if matches!(&body, Expr::Ident(name) if name == "value") {
		return Some(None);
	}
	let param = Param {
		name: "value".into(),
		ty: Some(b.t.ty(ty, span)),
	};
	Some(Some(Expr::Lambda(vec![param], Box::new(body))))
}

/// `<ty as IntoIterator>::IntoIter`: the iterator `into_iter` makes of a `ty`.
pub(super) fn iter_of<'tcx>(t: &Translator<'_, 'tcx>, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
	let tcx = t.tcx;
	let into = tcx.get_diagnostic_item(rustc_span::sym::IntoIterator)?;
	let item = tcx
		.associated_items(into)
		.filter_by_name_unhygienic(rustc_span::Symbol::intern("IntoIter"))
		.next()?
		.def_id;
	let projection = Ty::new_projection(tcx, ty::IsRigid::No, item, [ty]);
	tcx.try_normalize_erasing_regions(t.typing_env_pub(), ty::Unnormalized::new_wip(projection))
		.ok()
}

/// `ty`, a reference to a box of a slice or an array, as one to what the box holds, which
/// JS keeps as the box.
pub fn unbox_ref<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Ty<'tcx> {
	match *ty.kind() {
		ty::Ref(region, inner, mutability)
			if let Some(held) = inner.boxed_ty()
				&& matches!(held.kind(), ty::Slice(_) | ty::Array(..)) =>
		{
			Ty::new_ref(tcx, region, held, mutability)
		}
		_ => ty,
	}
}

/// Whether `ty` is a `&mut` to a `&mut`, which is an `rt.Place` of it, or a box of one,
/// which is what it holds.
pub fn through_mut(ty: Ty<'_>) -> bool {
	match ty.boxed_ty() {
		Some(inner) => through_mut(inner),
		None => matches!(*ty.kind(), ty::Ref(_, inner, ty::Mutability::Mut) if inner.is_ref()),
	}
}

/// `value`, a `ty`, read through each box and `&mut` to a `&mut` that std's `Iterator` for
/// `Box<I>` and `&mut I` forward through, down to the last `&mut` and its JS value.
pub fn peel_mut<'tcx>(mut ty: Ty<'tcx>, mut value: Expr) -> (Ty<'tcx>, Expr) {
	while through_mut(ty) {
		ty = match ty.boxed_ty() {
			Some(inner) => inner,
			None => {
				value = value.member("v");
				ty.builtin_deref(true).expect("a reference")
			}
		};
	}
	(ty, value)
}

/// Refuse iterating `ty`, an iterator, if a binding stands in for it: it is no `rt.Iter`,
/// which std's adaptors and `for` take every iterator but the crate's own for.
fn not_bound<'tcx>(b: &mut Body<'_, '_, 'tcx>, ty: Ty<'tcx>, span: Span) {
	if let ty::Adt(adt, _) = ty.peel_refs().kind()
		&& b.t.bound_drops(adt.did()).is_some()
	{
		b.t.err(span, "iterating a bound type, which is not supported yet");
	}
}

/// The mapping of a call on or to an iterator, if it is one. Where std drops an item it
/// skips, rejects, or holds, or an iterator it takes by value, the runtime function is
/// given what drops it after its own arguments.
pub fn call<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	call: &Call<'tcx>,
	owner: &str,
	name: &str,
	mut args: Vec<Expr>,
) -> Option<Expr> {
	let tcx = b.t.tcx;
	let span = call.span;
	let gargs = call.args;
	let targ = |n: usize| gargs.types().nth(n);
	let path = library::std_path(tcx, call.def_id);
	let path: Vec<&str> = path.iter().map(String::as_str).collect();

	// Where iterators come from.
	match (owner, name) {
		("slice", "iter_mut") if b.t.needs_place(targ(0)?) => {
			let s = library::elements(args.pop()?, Ty::new_slice(tcx, targ(0)?));
			return Some(Expr::rt("iterSliceMut", vec![s]));
		}
		("slice", "iter" | "iter_mut") => {
			let s = args.pop()?;
			let s = library::elements(s, Ty::new_slice(tcx, targ(0)?));
			return Some(by_ref(Expr::rt("iterSlice", vec![s]), targ(0)?));
		}
		("Option", "iter" | "iter_mut") => {
			let mut args = args;
			if let Some(none) = b.t.none_arg(targ(0)?) {
				args.extend([Expr::undefined(), none]);
			}
			return Some(Expr::rt("iterOption", args));
		}
		// A `&mut` to a `&mut` iterator is itself, an `rt.Place` that each use reads through.
		("IntoIterator", "into_iter") if through_mut(targ(0)?) => return args.pop(),
		// So is a `&mut` iterator, which what takes it by value borrows as it does.
		("IntoIterator", "into_iter")
			if targ(0)?.ref_mutability() == Some(ty::Mutability::Mut) && b.t.iter_item(targ(0)?).is_some() =>
		{
			not_bound(b, targ(0)?, span);
			return args.pop();
		}
		("IntoIterator", "into_iter") => {
			let value = args.pop()?;
			return into_iter(b, targ(0)?, value, span);
		}
		_ => {}
	}
	match path.as_slice() {
		["iter", "sources", "once", "once"] => {
			let dropper = b.t.dropper(targ(0)?, span);
			return Some(Expr::rt("iterOnce", tail(args, vec![dropper])));
		}
		["iter", "sources", "empty", "empty"] => return Some(Expr::rt("iterEmpty", Vec::new())),
		// A closure that owns a value that drops is refused as any std mapping refuses one
		// it calls but never drops.
		// Each takes its closure's `None` last, where that is `rt.None`.
		["iter", "sources", "from_fn", "from_fn"] => {
			let none = b.t.none_arg(targ(0)?);
			return Some(Expr::rt("iterFromFn", [args, none.into_iter().collect()].concat()));
		}
		// What drops the item it holds next.
		["iter", "sources", "successors", "successors"] => {
			let droppers = [b.t.dropper(targ(0)?, span)].into_iter().chain(b.t.none_arg(targ(0)?));
			return Some(Expr::rt("iterSuccessors", tail(args, droppers.collect())));
		}
		_ => {}
	}

	// What `collect` calls.
	if (owner, name) == ("FromIterator", "from_iter") {
		let source = targ(2)?;
		let iter = iter_of(b.t, source)?;
		let items = into_iter(b, source, args.pop()?, span)?;
		let trusted = implements(b.t, iter, "TrustedLen");
		return collect(b, targ(0)?, targ(1)?, items, iter, trusted, span);
	}

	// `Vec::extend` with an iterator, which it drops once done, `()`'s, and a tuple's of collections.
	if (owner, name) == ("Extend", "extend") {
		let (coll, item, source) = (targ(0)?, targ(1)?, targ(2)?);
		if !matches!(coll.kind(), ty::Tuple(_)) && b.t.std_of(coll) != Some(Std::Vec) {
			return None;
		}
		let [target, items] = <[Expr; 2]>::try_from(args).ok()?;
		let iter = iter_of(b.t, source)?;
		let mut items = into_iter(b, source, items, span)?;
		// `Extend<&T>` copies each element.
		if let Some(copy) = super::collections::copy_borrowed(b, coll, item, span) {
			items = Expr::rt("iterMap", vec![items, copy]);
		}
		return super::collections::extend(b, coll, item, target, items, iter, span);
	}

	let self_ty = match (owner, name) {
		("Iterator" | "DoubleEndedIterator" | "ExactSizeIterator", _) => targ(0)?,
		("Peekable", "peek" | "peek_mut" | "next_if" | "next_if_eq")
			if library::inherent_std(tcx, call.def_id) == Some(Std::Peekable) =>
		{
			let imp = tcx.inherent_impl_of_assoc(call.def_id)?;
			let generics = tcx.generics_of(imp).count();
			tcx.type_of(imp).instantiate(tcx, &gargs[..generics]).skip_norm_wip()
		}
		_ => return None,
	};
	let item = b.t.iter_item(self_ty.peel_refs())?;
	not_bound(b, self_ty, span);
	let mut values = args.into_iter();
	let (self_ty, recv) = peel_mut(self_ty, values.next()?);
	let rest: Vec<Expr> = values.collect();
	// What a consumer returns for `None`, after its other arguments, where that is `rt.None`.
	let none = b.t.none_arg(item);

	// `next` by the protocol, as an `Option`.
	if matches!(name, "next" | "next_back" | "nth_back") {
		let item = match name {
			"next" => next(b, self_ty, recv),
			"next_back" => recv.member("nextBack").call(Vec::new()),
			_ => {
				let droppers = [b.t.dropper(item, span)].into_iter().chain(none).collect();
				return Some(Expr::rt("iterNthBack", tail([vec![recv], rest].concat(), droppers)));
			}
		};
		return Some(Expr::rt("iterToOption", [item].into_iter().chain(none).collect()));
	}
	// The same object, borrowed.
	if name == "by_ref" {
		return Some(recv);
	}

	let recv = into_iter(b, self_ty, recv, span)?;
	// `rt.iter<Name>(recv, ...args)`, or `peekable<Name>` of a `Peekable`'s own.
	let method = |name: &str, args: Vec<Expr>| {
		let of = match owner {
			"Peekable" => "peekable",
			_ => "iter",
		};
		Some(Expr::rt_of(of, name).call([vec![recv.clone()], args].concat()))
	};
	let camel = super::camel(name);
	match name {
		"map" | "enumerate" | "rev" | "inspect" | "any" | "all" | "position" => method(&camel, rest),
		// Each takes its closure's `None` last, where that is `rt.None`.
		"filter_map" | "map_while" | "find_map" => {
			let none = b.t.none_arg(targ(1)?);
			method(&camel, [rest, none.into_iter().collect()].concat())
		}
		// What each drops: an item it skips or rejects, or one it peeked.
		"filter" | "skip_while" | "take_while" | "peekable" | "skip" | "take" | "step_by" => {
			// std reads such a `skip` by index where what wraps it does, making and dropping each
			// skipped pair whole, and else by the zip's `nth`, a side at a time.
			if name == "skip"
				&& b.t.drops(item)
				&& implements(b.t, self_ty, "TrustedRandomAccess")
				&& nth_zips(tcx, self_ty)
			{
				return Some(b.err(
					span,
					format!(
						"a `skip` of `{self_ty}`, a zip std reads by index, of items that drop, which is not supported yet"
					),
				));
			}
			let dropper = b.t.dropper(item, span);
			method(&camel, tail(rest, vec![dropper]))
		}
		"fuse" => {
			let fused = fused(b, self_ty, None);
			method(name, tail(rest, vec![fused]))
		}
		// What clones its iterator, as it is made and each time a clone of it ends.
		"cycle" => {
			let clones = clones(b, self_ty, span)?;
			method(name, tail(rest, vec![clones]))
		}
		// `rt.owned<Name>`, which drops what `self_ty` holds and its items by `dropper` as
		// std's own does, where either drops, or counts a `Cycle` as std's `Take::count`
		// does, by its `advance_by`, which stops at a clone that yields nothing where
		// `try_fold` reads on: else `rt.iter<Name>`, so a bundle that never drops doesn't
		// carry it.
		"count" | "last" => {
			let dropper = b.t.dropper(item, span);
			let none: Vec<Expr> = none.into_iter().filter(|_| name == "last").collect();
			let cycles = name == "count"
				&& (self_ty.walk().filter_map(|arg| arg.as_type()))
					.any(|t| std_iter_path(tcx, t).is_some_and(|p| p == ["iter", "adapters", "cycle", "Cycle"]));
			match drops_iter(b, self_ty, span) || !undefined(&dropper) || cycles {
				true => Some(Expr::rt_of("owned", name).call(tail(vec![recv.clone()], [vec![dropper], none].concat()))),
				false => method(name, none),
			}
		}
		// Each owns a closure it is given by value, which drops as its fold ends.
		"for_each" | "fold" => {
			let drops = iter_drops(b, self_ty, span);
			let owns = match b.t.drops(targ(gargs.types().count() - 1)?) {
				true => Expr::lit("true"),
				false => Expr::undefined(),
			};
			method(&camel, tail(rest, vec![drops, owns]))
		}
		// std's `Sum` and `Product` are of the type or a reference to it: another is the crate's.
		"sum" | "product" if item.peel_refs() != targ(1)? => None,
		// `Duration`'s carries its nanoseconds and panics as std's does, not as `+` would.
		"sum" if b.t.std_of(targ(1)?) == Some(Std::Duration) => {
			let drops = iter_drops(b, self_ty, span);
			Some(Expr::rt("durationSum", tail(vec![recv.clone()], vec![drops])))
		}
		// std's float sum starts from -0.0, so a sum of none, or of -0.0s, is -0.0.
		"sum" | "product" if let Some(float) = super::float::Float::of(targ(1)?) => {
			let (op, unit) = match name {
				"sum" => ("add", -0.0),
				_ => ("mul", 1.0),
			};
			let drops = iter_drops(b, self_ty, span);
			method(
				name,
				tail(vec![Expr::rt_of(float.name(), op), float.lit(unit)], vec![drops]),
			)
		}
		// A `Wrapping`'s wraps and a `Saturating`'s saturates, as its `+` and `*` do.
		"sum" | "product" => {
			let overflowing = super::library::overflowing(b.t.tcx, targ(1)?);
			let int = Int::of(overflowing.map_or(targ(1)?, |(_, int)| int))?;
			let (op, unit) = match name {
				"sum" => ("Add", 0),
				_ => ("Mul", 1),
			};
			let op = match overflowing {
				Some((overflow, _)) => overflow.method(op),
				None => op.to_lowercase(),
			};
			let drops = iter_drops(b, self_ty, span);
			method(name, tail(vec![int.func(&op), int.lit(unit)], vec![drops]))
		}
		// `ExactSizeIterator::len` and `size_hint` are each iterator's own.
		"len" => Some(recv.member("len").call(rest)),
		"size_hint" => Some(recv.member("sizeHint").call(rest)),
		"nth" | "find" | "rfind" => {
			let dropper = b.t.dropper(item, span);
			method(&camel, tail(rest, [dropper].into_iter().chain(none).collect()))
		}
		"peek" | "next_if" => method(&camel, [rest, none.into_iter().collect()].concat()),
		// An item JS copies is lent as its place, and an object as itself.
		"peek_mut" => method(&camel, vec![Expr::lit(b.t.needs_place(item).to_string())]),
		// By the item's `PartialEq<T>`: `eq_fn` of its own type, else the impl's `eq`, as a
		// call to it lowers, given a `&T`.
		"next_if_eq" => {
			let expected = targ(1)?;
			let eq = match expected == item {
				true => super::collections::eq_fn(b, item, span),
				false => {
					let by = Ty::new_imm_ref(tcx, tcx.lifetimes.re_erased, expected);
					super::collections::impl_eq(b, "eq", item, expected, [item, by], span)
				}
			};
			method(&camel, [rest, vec![eq], none.into_iter().collect()].concat())
		}
		// By the comparator, which `best` calls with the best so far and then the next, as
		// std's `cmp::max_by` and `min_by` do.
		"max_by" | "min_by" => {
			let droppers = [iter_drops(b, self_ty, span), b.t.dropper(item, span)]
				.into_iter()
				.chain(none);
			method(&camel, tail(rest, droppers.collect()))
		}
		"reduce" => {
			let drops = iter_drops(b, self_ty, span);
			method(name, tail(rest, [drops].into_iter().chain(none).collect()))
		}
		// By the items' `Ord::max` or `min`, or the keys', which test `<` of the next and the
		// best. Not of places, which the runtime would compare as the objects they are.
		"max" | "min" | "max_by_key" | "min_by_key"
			if !b.t.has_place(if name.ends_with("_by_key") { targ(1)? } else { item }) =>
		{
			let by_key = name.ends_with("_by_key");
			let key = if by_key { targ(1)? } else { item };
			if by_key && b.t.drops(key) {
				return Some(b.err(
					span,
					format!("`{name}` by `{key}`, which drops, which is not supported yet"),
				));
			}
			// A `BTreeMap`'s `into_keys` reads the one end and drops the rest with it.
			if !by_key
				&& std_iter_path(tcx, self_ty).is_some_and(|p| p == ["collections", "btree", "map", "IntoKeys"])
				&& b.t.drops(self_ty)
			{
				return Some(b.err(
					span,
					format!("`{name}` of `{self_ty}`, which drops its values, which is not supported yet"),
				));
			}
			// A B-tree's iterators read an end, comparing nothing.
			if !by_key && btree_end(tcx, self_ty, name) {
				let end = match name {
					"max" => recv.member("nextBack").call(Vec::new()),
					_ => recv.member("next").call(Vec::new()),
				};
				return Some(Expr::rt("iterToOption", [end].into_iter().chain(none).collect()));
			}
			if !by_key && own_ord(b, key, name) {
				return Some(b.err(
					span,
					format!("`{name}` of `{key}` through its own `Ord::{name}`, which is not supported yet"),
				));
			}
			let lt = super::collections::test_fn(b, key, "lt", span);
			let droppers = [iter_drops(b, self_ty, span), b.t.dropper(item, span)]
				.into_iter()
				.chain(none);
			method(&camel, tail([rest, vec![lt]].concat(), droppers.collect()))
		}
		// Whether each item and the next are in order: by `PartialOrd::le`, by a comparator,
		// or by `le` of the keys a function makes, as std's maps to them. Not of places, as
		// `max` is not.
		"is_sorted" | "is_sorted_by" | "is_sorted_by_key" => {
			let (items, compare, each) = match name {
				"is_sorted_by" => (recv.clone(), rest.into_iter().next()?, item),
				_ if b.t.has_place(item) => return None,
				"is_sorted" => (recv.clone(), super::collections::test_fn(b, item, "le", span), item),
				_ => {
					let key = targ(2)?;
					let le = super::collections::test_fn(b, key, "le", span);
					// The map drops its function as the iterator drops, so one through a reference
					// or box is lent, as one owned that drops is refused with such an iterator.
					let (mut f, f_ty) = (rest.into_iter().next()?, targ(1)?);
					let owned = matches!(f_ty.kind(), ty::Closure(..) | ty::FnDef(..) | ty::FnPtr(..));
					if !owned && drops_iter(b, self_ty, span) {
						f = Expr::rt("lent", vec![f]);
					}
					(Expr::rt("iterMap", vec![recv.clone(), f]), le, key)
				}
			};
			let mut droppers = vec![iter_drops(b, self_ty, span), b.t.dropper(each, span)];
			if name == "is_sorted_by" {
				droppers.push(b.t.dropper(targ(1)?, span));
			}
			Some(Expr::rt("iterIsSortedBy", tail(vec![items, compare], droppers)))
		}
		"rposition" => method(&camel, rest),
		"eq" | "ne" | "lt" | "le" | "gt" | "ge" | "cmp" | "partial_cmp" | "eq_by" | "cmp_by" | "partial_cmp_by" => {
			compare(b, call, name, self_ty, item, recv, rest)
		}
		"chain" => {
			let other = into_iter(b, targ(1)?, rest.into_iter().next()?, span)?;
			method(name, vec![other])
		}
		"zip" => {
			let other_ty = targ(1)?;
			let other = into_iter(b, other_ty, rest.into_iter().next()?, span)?;
			let zip = b.t.sig(call.def_id, gargs).output();
			let ty::Adt(_, sides) = zip.kind() else {
				return None;
			};
			let (a, other_iter) = (sides.type_at(0), sides.type_at(1));
			let other_item = b.t.iter_item(other_iter)?;
			let droppers = vec![
				zip_trusted(b, a, other_iter, span),
				b.t.dropper(item, span),
				b.t.dropper(other_item, span),
			];
			method(name, tail(vec![other], droppers))
		}
		// `flat_map::<U, F>`, of the source mapped by `F`, which drops as the source does.
		"flat_map" => {
			let inner = iter_of(b.t, targ(1)?)?;
			flattens(b, inner, span);
			let into = into_fn(b, targ(1)?, span)?.unwrap_or_else(Expr::undefined);
			let fused = fused(b, self_ty, Some(targ(2)?));
			let size = array_size(b, targ(1)?);
			let one_shot = Expr::lit(implements(b.t, inner, "OneShot").to_string());
			method("flatMap", tail(rest, vec![into, fused, size, one_shot]))
		}
		"flatten" => {
			let inner = iter_of(b.t, item)?;
			flattens(b, inner, span);
			let into = into_fn(b, item, span)?.unwrap_or_else(Expr::undefined);
			let fused = fused(b, self_ty, None);
			let size = array_size(b, item);
			let one_shot = Expr::lit(implements(b.t, inner, "OneShot").to_string());
			method("flatten", tail(Vec::new(), vec![into, fused, size, one_shot]))
		}
		"copied" | "cloned" => {
			let ty::Ref(_, inner, _) = item.kind() else {
				return None;
			};
			let clone = |b: &mut Body<'_, '_, 'tcx>| {
				let clone = tcx.lang_items().clone_fn()?;
				Some(b.fn_value(clone, tcx.mk_args(&[(*inner).into()]), span))
			};
			// A `Clone` written by hand may do other than copy, even for a `Copy` type. A JS
			// string is a value already.
			let copies = name == "copied" || library::clone_copies(b.t, *inner) || library::string(tcx, *inner);
			let copy = match () {
				// Each item through its `Clone`.
				_ if !copies => Some(clone(b)?),
				// Out of the slice each item is.
				_ if super::byte_array(*inner) => Some(Expr::Rt("byteSliceToArray".into())),
				_ => b.t.copy_fn(*inner, span),
			};
			// std's `copied` skips through its iterator's `nth`, and `cloned` by its own `next`.
			match (name, copy) {
				("copied", Some(copy)) => method("copied", vec![copy]),
				("copied", None) => Some(recv),
				(_, Some(copy)) => method("map", vec![copy]),
				(_, None) => method("cloned", Vec::new()),
			}
		}
		"collect" => {
			let trusted = implements(b.t, self_ty, "TrustedLen");
			collect(b, targ(1)?, item, recv, self_ty, trusted, span)
		}
		// `extend` of the pair of collections' defaults.
		"unzip" => {
			let tuple = Ty::new_tup(tcx, &[targ(3)?, targ(4)?]);
			super::collections::tuple_extend(b, tuple, item, None, recv, self_ty, span)
		}
		_ => None,
	}
}

/// std's comparison `name` of `recv`, an iterator of type `self_ty` and items of type
/// `item`, against the `IntoIterator` first in `rest`, by the closure after it in a `_by`
/// form. Each is `iter_compare`'s: `ne` is `!eq`, and `lt`, `le`, `gt`, and `ge` test
/// `partial_cmp`. The rest compare each pair by the items' `PartialEq`, `Ord`, or
/// `PartialOrd`, in a closure that drops the pair.
fn compare<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	call: &Call<'tcx>,
	name: &str,
	self_ty: Ty<'tcx>,
	item: Ty<'tcx>,
	recv: Expr,
	rest: Vec<Expr>,
) -> Option<Expr> {
	let span = call.span;
	let other_ty = call.args.types().nth(1)?;
	let other_iter = iter_of(b.t, other_ty)?;
	let other_item = b.t.iter_item(other_iter)?;
	let mut rest = rest.into_iter();
	let other = into_iter(b, other_ty, rest.next()?, span)?;
	let test = name.trim_end_matches("_by");
	let drops = [b.t.dropper(item, span), b.t.dropper(other_item, span)];
	let (f, owns) = match rest.next() {
		Some(f) => (f, b.t.drops(call.args.types().nth(2)?)),
		None => {
			let f = match test {
				"eq" | "ne" => super::collections::pair_eq(b, item, other_item, false, span),
				"cmp" => super::collections::cmp_fn(b, item, false, span),
				_ => super::collections::pair_partial_cmp(b, item, other_item, span),
			};
			let f = match drops.iter().all(undefined) {
				true => f,
				false => Expr::rt("consuming", [vec![f], drops.to_vec()].concat()),
			};
			(f, false)
		}
	};
	let [drop_x, drop_y] = drops;
	let compared = vec![
		iter_drops(b, self_ty, span),
		iter_drops(b, other_iter, span),
		if owns { Expr::lit("true") } else { Expr::undefined() },
		drop_x,
		drop_y,
	];
	Some(match test {
		// std's `eq` of two `TrustedLen` iterators reads neither unless their size hints agree.
		"eq" | "ne" => {
			let trusted = implements(b.t, self_ty, "TrustedLen") && implements(b.t, other_iter, "TrustedLen");
			let trusted = if trusted { Expr::lit("true") } else { Expr::undefined() };
			let eq = Expr::rt("iterEq", tail(vec![recv, other, f], [vec![trusted], compared].concat()));
			if name == "ne" { eq.not() } else { eq }
		}
		"cmp" | "partial_cmp" => Expr::rt("iterCmp", tail(vec![recv, other, f], compared)),
		_ => Expr::rt_of("iter", test).call(tail(vec![recv, other, f], compared)),
	})
}

/// `args`, then `tail` without the `undefined`s and `false`s it ends with, which a runtime
/// function's optional parameters default to.
pub(super) fn tail(mut args: Vec<Expr>, mut tail: Vec<Expr>) -> Vec<Expr> {
	while matches!(tail.last(), Some(Expr::Lit(l)) if l == "undefined" || l == "false") {
		tail.pop();
	}
	args.extend(tail);
	args
}

/// Whether `e` is `undefined`, as the dropper of what doesn't drop is.
fn undefined(e: &Expr) -> bool {
	matches!(e, Expr::Lit(l) if l == "undefined")
}

/// The size and alignment of a `ty` in bytes on wasm32, by which std's collections of it
/// reserve, or else what stands in for a figure of them: `undefined` over parameters, which
/// only a census's instance, which never runs, has.
pub(super) fn layout<'tcx>(b: &mut Body<'_, '_, 'tcx>, ty: Ty<'tcx>, span: Span) -> Result<(u64, u64), Expr> {
	match b.t.tcx.layout_of(b.t.typing_env_pub().as_query_input(ty)) {
		Ok(layout) => Ok((layout.size.bytes(), layout.align.abi.bytes())),
		Err(_) if ty.has_param() => Err(Expr::undefined()),
		Err(_) => Err(b.err(span, format!("reserving for `{ty}`, whose layout is unknown"))),
	}
}

/// The size of a `ty` in bytes, as `layout` gives it, by which std's `Vec` of it reserves.
pub(super) fn size_of<'tcx>(b: &mut Body<'_, '_, 'tcx>, ty: Ty<'tcx>, span: Span) -> Expr {
	layout(b, ty, span).map_or_else(|e| e, |(size, _)| Expr::lit(size.to_string()))
}

/// Whether an iterator of type `ty`, with `held` beside it, drops and is a `FusedIterator`,
/// which std's `fuse` and `flatten` keep once it ends rather than drop: `true` or `false`.
fn fused<'tcx>(b: &mut Body<'_, '_, 'tcx>, ty: Ty<'tcx>, held: Option<Ty<'tcx>>) -> Expr {
	let drops = b.t.drops(ty) || held.is_some_and(|held| b.t.drops(held));
	Expr::lit(match drops && implements(b.t, ty, "FusedIterator") {
		true => "true",
		false => "false",
	})
}

/// How many items `ty` has, if it is an array or a reference to one, which std's `flatten`
/// counts toward its size hint: else `undefined`.
fn array_size<'tcx>(b: &Body<'_, '_, 'tcx>, ty: Ty<'tcx>) -> Expr {
	let ty = match *ty.kind() {
		ty::Ref(_, inner, _) => inner,
		_ => ty,
	};
	match ty.kind() {
		ty::Array(_, len) => len
			.try_to_target_usize(b.t.tcx)
			.map_or_else(Expr::undefined, |n| Expr::lit(n.to_string())),
		_ => Expr::undefined(),
	}
}

/// The path of the std type `ty` is, if it is one.
fn std_iter_path(tcx: ty::TyCtxt<'_>, ty: Ty<'_>) -> Option<Vec<String>> {
	let ty::Adt(adt, _) = ty.kind() else {
		return None;
	};
	Some(library::std_path(tcx, adt.did())).filter(|p| !p.is_empty())
}

/// Whether `nth` of `ty`, an iterator, is that of a zip it wraps, through the adaptors
/// whose `nth` is their iterator's.
fn nth_zips(tcx: ty::TyCtxt<'_>, ty: Ty<'_>) -> bool {
	let ty::Adt(adt, args) = ty.kind() else {
		return false;
	};
	match library::std_path(tcx, adt.did()).last().map(String::as_str) {
		Some("Zip") => true,
		Some("Enumerate" | "Fuse" | "Skip") => nth_zips(tcx, args.type_at(0)),
		_ => false,
	}
}

/// Whether std's `owner::name` takes a closure by value that it drops as its fold ends,
/// before the iterators, which its mapping drops there too.
pub(super) fn releases(owner: &str, name: &str) -> bool {
	matches!(
		(owner, name),
		("Iterator", "for_each" | "fold" | "eq_by" | "cmp_by" | "partial_cmp_by")
	)
}

/// The function or `Iterator` method named for `ty`, one of std's iterators of zero-sized
/// fields, as `from_fn` makes a `FromFn` and `map` a `Map`, and its generic arguments: what
/// MIR reads as a constant, rather than from that call, makes it of its fields in order.
pub(super) fn maker<'tcx>(t: &Translator<'_, 'tcx>, ty: Ty<'tcx>) -> Option<(DefId, ty::GenericArgsRef<'tcx>)> {
	let tcx = t.tcx;
	let ty::Adt(adt, args) = ty.kind() else {
		return None;
	};
	if !library::is_std(tcx, adt.did()) || !adt.is_struct() || !t.zero_sized(ty) {
		return None;
	}
	let item = t.iter_item(ty)?;
	let name = tcx
		.item_name(adt.did())
		.as_str()
		.chars()
		.enumerate()
		.fold(String::new(), |mut name, (i, c)| {
			if c.is_uppercase() && i > 0 {
				name.push('_');
			}
			name.extend(c.to_lowercase());
			name
		});
	let iterator = tcx.get_diagnostic_item(rustc_span::sym::Iterator)?;
	let method = (tcx.associated_items(iterator))
		.filter_by_name_unhygienic(rustc_span::Symbol::intern(&name))
		.find(|i| i.is_fn())
		.map(|i| i.def_id);
	let make = method.or_else(|| super::bind::resolve_in(tcx, tcx.parent(adt.did()), &[&name]))?;
	if !matches!(
		tcx.def_kind(make),
		rustc_hir::def::DefKind::Fn | rustc_hir::def::DefKind::AssocFn
	) || ty::GenericArgs::identity_for_item(tcx, make).consts().next().is_some()
	{
		return None;
	}
	// What its result names, and the item for what it doesn't, as `map`'s `B` is.
	let output = tcx
		.fn_sig(make)
		.instantiate_identity()
		.skip_norm_wip()
		.skip_binder()
		.output();
	let mut found = Vec::new();
	bind_params(output, ty, &mut found);
	let gargs = ty::GenericArgs::for_item(tcx, make, |param, _| match param.kind {
		ty::GenericParamDefKind::Lifetime => tcx.lifetimes.re_erased.into(),
		_ => (found.iter())
			.find(|&&(index, _)| index == param.index)
			.map_or(item, |&(_, ty)| ty)
			.into(),
	});
	let sig = tcx.fn_sig(make).instantiate(tcx, gargs).skip_norm_wip();
	let sig = tcx.instantiate_bound_regions_with_erased(sig);
	let fields = adt.non_enum_variant().fields.iter().map(|f| t.field_ty(f.did, args));
	(sig.output() == ty && sig.inputs().iter().copied().eq(fields)).then_some((make, gargs))
}

/// The type parameters `pattern` names, with what `ty` has in their place.
fn bind_params<'tcx>(pattern: Ty<'tcx>, ty: Ty<'tcx>, found: &mut Vec<(u32, Ty<'tcx>)>) {
	match (pattern.kind(), ty.kind()) {
		(ty::Param(param), _) => found.push((param.index, ty)),
		(ty::Adt(a, pattern_args), ty::Adt(b, args)) if a == b => {
			for (p, t) in pattern_args.types().zip(args.types()) {
				bind_params(p, t, found);
			}
		}
		_ => {}
	}
}

/// `Clone::clone` of `value`, one of std's iterators of type `ty`: its `clone`, given what
/// clones what the runtime can't clone itself, or `None` for one std's has no mapping of.
pub(super) fn clone<'tcx>(b: &mut Body<'_, '_, 'tcx>, ty: Ty<'tcx>, value: Expr, span: Span) -> Option<Expr> {
	let clones = clones(b, ty, span)?;
	Some(value.member("clone").call(tail(Vec::new(), vec![clones])))
}

/// The `rt.Clones` an iterator of type `ty` is cloned by, as std's `Clone` of it clones
/// each part, or `undefined` where each part clones as itself: `i` and `o` for the
/// iterators it wraps, `f` for its closure, and `v` for an item or another value it holds.
/// The crate's own iterator, which `into_iter` wraps, is that value, by its `Clone`.
fn clones<'tcx>(b: &mut Body<'_, '_, 'tcx>, ty: Ty<'tcx>, span: Span) -> Option<Expr> {
	let tcx = b.t.tcx;
	let value = |b: &mut Body<'_, '_, 'tcx>, ty: Ty<'tcx>| super::collections::clone_fn(b, ty, span);
	if own_next(b, ty).is_some() {
		return Some(parts(vec![("v", value(b, ty))]));
	}
	let ty::Adt(adt, args) = ty.kind() else {
		return None;
	};
	let arg = |n: usize| args.types().nth(n);
	let path = library::std_path(tcx, adt.did());
	let path: Vec<&str> = path.iter().map(String::as_str).collect();
	Some(match path.as_slice() {
		// Those that borrow what they read, as their clones do too, or hold nothing.
		["iter", "sources", "empty", "Empty"]
		| [
			"slice",
			"iter",
			"Iter" | "Windows" | "Chunks" | "ChunksExact" | "RChunks" | "RChunksExact",
		]
		| ["str", "iter", "Chars" | "Bytes"]
		| ["ascii", "EscapeDefault"]
		| ["slice", "ascii", "EscapeAscii"]
		| ["option", "Iter"]
		| ["ops", "range", "Range" | "RangeFrom" | "RangeInclusive"]
		| ["collections", "vec_deque", "iter", "Iter"]
		| ["collections", "binary_heap", "Iter"]
		| ["collections", "btree", "map", "Iter" | "Keys" | "Values" | "Range"]
		| ["collections", "hash", "map", "Iter" | "Keys" | "Values"]
		| [
			"collections",
			"btree" | "hash",
			"set",
			"Iter" | "Range" | "Difference" | "Intersection" | "Union" | "SymmetricDifference",
		] => Expr::undefined(),
		// Those that own their items, each cloned.
		["vec", "into_iter", "IntoIter"]
		| ["array", "iter", "IntoIter"]
		| ["option", "IntoIter"]
		| ["iter", "sources", "once", "Once"]
		| ["collections", "vec_deque", "into_iter", "IntoIter"]
		| ["collections", "binary_heap", "IntoIter"] => parts(vec![("v", value(b, arg(0)?))]),
		// A closure's searcher clones the closure; another holds what it searches for.
		["str", "iter", "Split" | "MatchIndices"] => match arg(0)?.kind() {
			ty::Closure(..) => parts(vec![("f", value(b, arg(0)?))]),
			_ => Expr::undefined(),
		},
		// A slice's parts by a closure clone the closure.
		["slice", "iter", "Split" | "SplitInclusive" | "RSplit"] => parts(vec![("f", value(b, arg(1)?))]),
		// Its closure is what `rt.iterFrom` reads by.
		["iter", "sources", "from_fn", "FromFn"] => parts(vec![("v", value(b, arg(0)?))]),
		["iter", "sources", "successors", "Successors"] => {
			parts(vec![("v", value(b, arg(0)?)), ("f", value(b, arg(1)?))])
		}
		[
			"iter",
			"adapters",
			_,
			"Map" | "Filter" | "FilterMap" | "SkipWhile" | "TakeWhile" | "MapWhile" | "Inspect",
		] => parts(vec![("i", clones(b, arg(0)?, span)?), ("f", value(b, arg(1)?))]),
		[
			"iter",
			"adapters",
			_,
			"Enumerate" | "Skip" | "Take" | "StepBy" | "Fuse" | "Rev" | "Copied" | "Cloned" | "Cycle",
		] => parts(vec![("i", clones(b, arg(0)?, span)?)]),
		["iter", "adapters", _, "Chain" | "Zip"] => {
			parts(vec![("i", clones(b, arg(0)?, span)?), ("o", clones(b, arg(1)?, span)?)])
		}
		["iter", "adapters", "peekable", "Peekable"] => {
			let item = b.t.iter_item(arg(0)?)?;
			parts(vec![("i", clones(b, arg(0)?, span)?), ("v", value(b, item))])
		}
		// Its source, as `map` makes it, and the iterators of what that yields.
		["iter", "adapters", "flatten", "FlatMap"] => {
			let source = parts(vec![("i", clones(b, arg(0)?, span)?), ("f", value(b, arg(2)?))]);
			let inner = iter_of(b.t, arg(1)?)?;
			parts(vec![("i", source), ("o", clones(b, inner, span)?)])
		}
		["iter", "adapters", "flatten", "Flatten"] => {
			let inner = iter_of(b.t, b.t.iter_item(arg(0)?)?)?;
			parts(vec![("i", clones(b, arg(0)?, span)?), ("o", clones(b, inner, span)?)])
		}
		_ => return None,
	})
}

/// An `rt.Clones` of `parts` but those that clone as themselves, or `undefined` if all do.
fn parts(parts: Vec<(&str, Expr)>) -> Expr {
	let parts: Vec<(Key, Expr)> = (parts.into_iter())
		.filter(|(_, e)| !undefined(e))
		.map(|(name, e)| (Key::Name(name.into()), e))
		.collect();
	match parts.is_empty() {
		true => Expr::undefined(),
		false => Expr::Object(parts),
	}
}

/// Whether std's iterator `did` holds a closure that its `rt.Iter` drops, after the iterator
/// it wraps if any, as std's fields order them.
pub(super) fn holds_closure(tcx: ty::TyCtxt<'_>, did: DefId) -> bool {
	let path = library::std_path(tcx, did);
	let path: Vec<&str> = path.iter().map(String::as_str).collect();
	matches!(
		path.as_slice(),
		[
			"iter",
			"adapters",
			_,
			"Map" | "Filter" | "FilterMap" | "SkipWhile" | "TakeWhile" | "MapWhile" | "Inspect"
		] | ["iter", "adapters", "flatten", "FlatMap"]
			| ["iter", "sources", "from_fn", "FromFn"]
			| ["iter", "sources", "successors", "Successors"]
			| [
				"slice",
				"iter",
				"Split"
					| "SplitMut"
					| "RSplit"
					| "RSplitMut"
					| "SplitInclusive"
					| "SplitInclusiveMut"
					| "SplitN"
					| "SplitNMut"
					| "RSplitN"
					| "RSplitNMut"
			]
	)
}

/// Refuse a `flatten` or `flat_map` of `inner` iterators that drop and may hold what drops
/// once they end: std's `try_fold` drops each such one only as it reads the next, where
/// `next` drops it as it ends. What std's collections' and `Option`'s `into_iter` make
/// holds nothing then.
fn flattens<'tcx>(b: &mut Body<'_, '_, 'tcx>, inner: Ty<'tcx>, span: Span) {
	if !b.t.drops(inner) {
		return;
	}
	let path = std_iter_path(b.t.tcx, inner).unwrap_or_default();
	let path: Vec<&str> = path.iter().map(String::as_str).collect();
	let empties = matches!(
		path.as_slice(),
		["vec", "into_iter", "IntoIter"]
			| ["array", "iter", "IntoIter"]
			| ["option", "IntoIter"]
			| ["iter", "sources", "once", "Once"]
			| ["collections", "hash", "map", "IntoIter" | "IntoKeys" | "IntoValues"]
			| ["collections", "hash", "set", "IntoIter"]
			| ["collections", "btree", "map", "IntoIter" | "IntoKeys" | "IntoValues"]
			| ["collections", "btree", "set", "IntoIter"]
			| ["collections", "vec_deque", "into_iter", "IntoIter"]
	);
	if !empties {
		b.t.err(
			span,
			format!("a `flatten` of `{inner}`, which drops what it may hold once it ends, which is not supported yet"),
		);
	}
}

/// `rt.Trusted`, how std's `Zip` of `a` and `b` reads them, by the traits it specializes
/// on: by index if both are `TrustedRandomAccess`, as whether each may have side effects,
/// and folded by its size hint if both are `TrustedLen`, or `TrustedRandomAccessNoCoerce`,
/// whose `fold` reads by index as many pairs as their exact size hints, but whose `next`
/// reads as any other zip's does.
fn zip_trusted<'tcx>(b: &mut Body<'_, '_, 'tcx>, a: Ty<'tcx>, other: Ty<'tcx>, span: Span) -> Expr {
	let both = |name: &str| implements(b.t, a, name) && implements(b.t, other, name);
	if both("TrustedRandomAccess") {
		let effects = [a, other].map(|ty| match may_have_side_effect(b.t, ty, span) {
			Some(effect) => Expr::lit(effect.to_string()),
			None => b.err(
				span,
				format!("a zip of `{ty}`, whose `MAY_HAVE_SIDE_EFFECT` does not evaluate"),
			),
		});
		Expr::Array(effects.into())
	} else if both("TrustedLen") || both("TrustedRandomAccessNoCoerce") {
		Expr::Str("len".into())
	} else {
		Expr::undefined()
	}
}

/// `<ty as TrustedRandomAccessNoCoerce>::MAY_HAVE_SIDE_EFFECT`, whether std's `Zip` reads
/// an item of `ty` it never yields, evaluated as std's own constant.
fn may_have_side_effect<'tcx>(t: &Translator<'_, 'tcx>, ty: Ty<'tcx>, span: Span) -> Option<bool> {
	let tcx = t.tcx;
	let tr = core_trait(tcx, "TrustedRandomAccessNoCoerce")?;
	let item = tcx
		.associated_items(tr)
		.in_definition_order()
		.find(|item| item.name().as_str() == "MAY_HAVE_SIDE_EFFECT")?;
	let uv = rustc_middle::mir::UnevaluatedConst::new(item.def_id, tcx.mk_args(&[ty.into()]));
	let value = rustc_middle::mir::Const::Unevaluated(uv, tcx.types.bool);
	value.eval(tcx, t.typing_env_pub(), span).ok()?.try_to_bool()
}

/// Whether `ty` implements `core`'s trait `name`, one std specializes on: an iterator
/// marker it names nowhere else, or `Eq`.
pub(super) fn implements<'tcx>(t: &Translator<'_, 'tcx>, ty: Ty<'tcx>, name: &str) -> bool {
	use rustc_infer::infer::TyCtxtInferExt;
	use rustc_trait_selection::infer::InferCtxtExt;
	let tcx = t.tcx;
	let Some(tr) = core_trait(tcx, name) else {
		return false;
	};
	let (infcx, env) = tcx.infer_ctxt().build_with_typing_env(t.typing_env_pub());
	infcx.type_implements_trait(tr, [ty], env).must_apply_modulo_regions()
}

/// `core`'s trait `name`.
fn core_trait(tcx: ty::TyCtxt<'_>, name: &str) -> Option<DefId> {
	let &core = tcx
		.crates(())
		.iter()
		.find(|&&c| tcx.crate_name(c) == rustc_span::sym::core)?;
	tcx.traits(core)
		.iter()
		.copied()
		.find(|&tr| tcx.item_name(tr).as_str() == name)
}

/// `iter.collect::<ty>()` of `item`s, from an iterator of type `from`, which it drops, and
/// which std's reserves for as it does a `TrustedLen` one if `trusted`.
fn collect<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	ty: Ty<'tcx>,
	item: Ty<'tcx>,
	iter: Expr,
	from: Ty<'tcx>,
	trusted: bool,
	span: Span,
) -> Option<Expr> {
	let tcx = b.t.tcx;
	// `()`'s reads every item, each a `()` too, by `for_each`, which folds: from `undefined`,
	// so the result is typed as `()` is.
	if ty.is_unit() {
		let drops = iter_drops(b, from, span);
		let each = Expr::Lambda(Vec::new(), Box::new(Expr::undefined()));
		return Some(Expr::rt(
			"iterFold",
			tail(vec![iter, Expr::undefined(), each], vec![drops]),
		));
	}
	// A tuple's, which `extend`s the collections' defaults.
	if let ty::Tuple(_) = ty.kind() {
		return super::collections::tuple_extend(b, ty, item, None, iter, from, span);
	}
	let ty::Adt(adt, args) = ty.kind() else {
		return None;
	};
	let std = Std::of(tcx, adt.did())?;
	// A `Cow<[T]>` owns the `Vec` std's collects, and a `Box<[T]>` becomes it.
	let boxed = library::box_slice(tcx, ty);
	let elem = match std {
		Std::Vec => Some(args.type_at(0)),
		_ => library::cow_slice(tcx, ty).or(boxed),
	};
	if let Some(elem) = elem {
		let v = match elem.kind() {
			ty::Uint(ty::UintTy::U8) => {
				let (folds, by_index) = collect_folds(b, from, elem, span);
				let take = iter_take(b, from, folds, span);
				Expr::rt(
					"iterToByteVec",
					tail(vec![iter], vec![take, Expr::lit((trusted || by_index).to_string())]),
				)
			}
			_ => to_array(b, iter, from, elem, trusted, span),
		};
		return Some(match boxed {
			Some(elem) => library::vec_boxed(v, elem),
			None => v,
		});
	}
	match std {
		// The values, collected as the `Ok` or `Some` type is, until the first `Err` or `None`.
		std @ (Std::Result | Std::Option) => {
			let ty::Adt(_, item_args) = item.kind() else {
				return None;
			};
			let value = item_args.type_at(0);
			let value_ty = b.t.ty(value, span);
			// What it collects from is what `from` yields up to the first `Err` or `None`, which
			// std's collects in place of `from`'s source as it would `from`, but never trusts
			// the length of.
			let values = collect(b, args.type_at(0), value, Expr::ident("values"), from, false, span)?;
			let param = Param {
				name: "values".into(),
				ty: Some(Type::Rt("Iter", vec![value_ty])),
			};
			let dropper = b.t.dropper(args.type_at(0), span);
			let collected = vec![iter, Expr::Lambda(vec![param], Box::new(values))];
			Some(match std {
				Std::Result => Expr::rt("iterCollectResult", tail(collected, vec![dropper])),
				// The items' `None`, and the result's, each `rt.None` where `undefined` is a value.
				_ => {
					let nones = [value, args.type_at(0)].map(|t| b.t.none(t));
					Expr::rt(
						"iterCollectOption",
						tail(collected, [vec![dropper], nones.into()].concat()),
					)
				}
			})
		}
		// From chars or text, as `extend` of an empty one, but for an owned string's, which
		// the rest extend once `next` gives it. A `Cow<str>` and a `Box<str>` own the `String`
		// std's collects.
		Std::String | Std::Cow | Std::Box if std == Std::String || args.types().next().is_some_and(|t| t.is_str()) => {
			match b.t.std_of(item) {
				Some(Std::String | Std::Cow) => {
					let drops = iter_drops(b, from, span);
					Some(Expr::rt("stringFromStrings", tail(vec![iter], vec![drops])))
				}
				_ => string_extend(b, Expr::Str(String::new()), iter, item, from, span),
			}
		}
		_ => super::collections::collect(b, ty, iter, from, trusted, span),
	}
}

/// `s` extended by `items`, an iterator of type `iter` of `item`s, as `String`'s `Extend`
/// reads it, by `for_each`: of chars, through any `&`, or of text, a `&str`, `String`,
/// `Box<str>`, or `Cow<str>`.
pub(super) fn string_extend<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	s: Expr,
	items: Expr,
	item: Ty<'tcx>,
	iter: Ty<'tcx>,
	span: Span,
) -> Option<Expr> {
	let extend = if text_items(b.t, item)? {
		"stringExtend"
	} else {
		"stringExtendChars"
	};
	let drops = iter_drops(b, iter, span);
	Some(Expr::rt(extend, tail(vec![s, items], vec![drops])))
}

/// Whether a `String` extends by `item`s of text, a `&str`, `String`, `Box<str>`, or
/// `Cow<str>`, rather than chars, through any `&`: `None` for any other.
pub(super) fn text_items<'tcx>(t: &Translator<'_, 'tcx>, item: Ty<'tcx>) -> Option<bool> {
	match *item.peel_refs().kind() {
		ty::Char => Some(false),
		ty::Str => Some(true),
		ty::Adt(_, args) => match t.std_of(item)? {
			Std::String => Some(true),
			Std::Box | Std::Cow => Some(args.types().next().is_some_and(|t| t.is_str())),
			_ => None,
		},
		_ => None,
	}
}

/// Whether an iterator of type `ty` that a runtime function takes by value drops what it
/// holds, which its `rt.Iter` drops as std's does: `true`, or `undefined` for one that
/// drops nothing.
pub(super) fn iter_drops<'tcx>(b: &mut Body<'_, '_, 'tcx>, ty: Ty<'tcx>, span: Span) -> Expr {
	match drops_iter(b, ty, span) {
		true => Expr::lit("true"),
		false => Expr::undefined(),
	}
}

/// How a runtime function takes an iterator of type `ty` by value, as `rt.Take` says: by
/// `next` unless std's `folds` it, and then by its own `fold`, or `undefined` for one that
/// drops nothing, which it reads as `folded` steps it.
pub(super) fn iter_take<'tcx>(b: &mut Body<'_, '_, 'tcx>, ty: Ty<'tcx>, folds: bool, span: Span) -> Expr {
	match (drops_iter(b, ty, span), folds) {
		(_, false) => Expr::Str("next".into()),
		(true, true) => Expr::Str("fold".into()),
		(false, true) => Expr::undefined(),
	}
}

/// Whether an iterator of type `ty` drops, refusing one it can't drop: the crate's own
/// drops through `rt.iterFrom`.
fn drops_iter<'tcx>(b: &mut Body<'_, '_, 'tcx>, ty: Ty<'tcx>, span: Span) -> bool {
	if !b.t.drops(ty) {
		return false;
	}
	if own_next(b, ty).is_none() {
		b.t.dropper(ty, span);
	}
	true
}

/// `rt.iterToArray` of `iter`, an iterator of type `from`, collected into a `Vec<elem>` as
/// `collect_folds` says std's reads it. It reserves as std's does for a `TrustedLen`
/// iterator if `trusted`.
pub(super) fn to_array<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	iter: Expr,
	from: Ty<'tcx>,
	elem: Ty<'tcx>,
	trusted: bool,
	span: Span,
) -> Expr {
	let (folds, by_index) = collect_folds(b, from, elem, span);
	let take = iter_take(b, from, folds, span);
	let size = size_of(b, elem, span);
	Expr::rt(
		"iterToArray",
		tail(
			vec![iter],
			vec![take, size, Expr::lit((trusted || by_index).to_string())],
		),
	)
}

/// Whether std's `collect` of an iterator of type `from` into a `Vec<elem>` folds it, and
/// whether by index, reading no item past its length. In place of the `Vec` its items come
/// from, it reads a `TrustedRandomAccessNoCoerce` one by index and any other by `try_fold`,
/// then drops it whole, as reading by `next` and then dropping it does but for a zip, whose
/// `next` reads a side once past the other: that one folds, its length reserved as a
/// `TrustedLen` one's, and is refused if it holds values that drop, which its `fold` drops
/// as each fold ends. Elsewhere, it folds a `TrustedLen` one by its own `fold`, which drops
/// them first, and reads any other by `next`.
fn collect_folds<'tcx>(b: &mut Body<'_, '_, 'tcx>, from: Ty<'tcx>, elem: Ty<'tcx>, span: Span) -> (bool, bool) {
	if !in_place(b.t, from, elem) {
		return (implements(b.t, from, "TrustedLen"), false);
	}
	let zips = from.walk().filter_map(|arg| arg.as_type()).any(|t| {
		matches!(t.kind(), ty::Adt(adt, _) if library::std_path(b.t.tcx, adt.did()).last().map(String::as_str) == Some("Zip"))
	});
	let by_index = zips && implements(b.t, from, "TrustedRandomAccessNoCoerce");
	if by_index && drops_iter(b, from, span) {
		b.t.err(
			span,
			format!(
				"a `collect` in place of `{from}`, a zip std reads by index, holding values that drop, which is not supported yet"
			),
		);
	}
	(by_index, by_index)
}

/// Whether std's `collect` of an iterator of type `iter` into a `Vec<elem>` writes in place
/// of the `Vec` its items come from, as its adaptors let it where each item's layout leaves
/// room.
fn in_place<'tcx>(t: &Translator<'_, 'tcx>, iter: Ty<'tcx>, elem: Ty<'tcx>) -> bool {
	let tcx = t.tcx;
	let env = t.typing_env_pub();
	let (Some(steps), Some(source)) = (core_trait(tcx, "InPlaceIterable"), core_trait(tcx, "SourceIter")) else {
		return false;
	};
	if !implements(t, iter, "InPlaceIterable") || !implements(t, iter, "SourceIter") {
		return false;
	}
	let assoc = |tr: DefId, name: &str| {
		tcx.associated_items(tr)
			.filter_by_name_unhygienic(rustc_span::Symbol::intern(name))
			.next()
			.map(|item| item.def_id)
	};
	// `<I as SourceIter>::Source`, a `vec::IntoIter` of what it collects in place of.
	let Some(src) = assoc(source, "Source").and_then(|item| {
		let projection = Ty::new_projection(tcx, ty::IsRigid::No, item, [iter]);
		tcx.try_normalize_erasing_regions(env, ty::Unnormalized::new_wip(projection))
			.ok()
	}) else {
		return false;
	};
	let ty::Adt(adt, src_args) = src.kind() else {
		return false;
	};
	if library::std_path(tcx, adt.did()) != ["vec", "into_iter", "IntoIter"] {
		return false;
	}
	// `MERGE_BY` and `EXPAND_BY`: how many source items make how many it collects, if known.
	let step = |name: &str| -> Option<u128> {
		let item = assoc(steps, name)?;
		let instance = Instance::try_resolve(tcx, env, item, tcx.mk_args(&[iter.into()])).ok()??;
		let value = tcx.const_eval_instance(env, instance, rustc_span::DUMMY_SP).ok()?;
		Some(value.try_to_scalar_int()?.to_bits_unchecked())
	};
	let (Some(merge), Some(expand)) = (step("MERGE_BY"), step("EXPAND_BY")) else {
		return false;
	};
	let layout = |ty: Ty<'tcx>| tcx.layout_of(env.as_query_input(ty)).ok();
	let (Some(from), Some(to)) = (layout(src_args.type_at(0)), layout(elem)) else {
		return false;
	};
	// As std's `in_place_collectible`, where `None`, an overflow, is 0.
	!from.is_zst()
		&& !to.is_zst()
		&& from.align.abi == to.align.abi
		&& merge > 0
		&& expand > 0
		&& u128::from(from.size.bytes()) * merge >= u128::from(to.size.bytes()) * expand
}

/// Whether the crate's own `Ord` of `ty` has its own `max` or `min`, by `name`, which
/// `Iterator::max` or `min` calls. A reference's that std's forwards through is the
/// default, which tests `<`.
fn own_ord<'tcx>(b: &Body<'_, '_, 'tcx>, ty: Ty<'tcx>, name: &str) -> bool {
	let tcx = b.t.tcx;
	let f = super::collections::trait_fn(tcx, "Ord", name);
	super::collections::by_own(b, super::collections::peel_ord(b, ty, false))
		&& Instance::try_resolve(tcx, b.t.typing_env_pub(), f, tcx.mk_args(&[ty.into()]))
			.ok()
			.flatten()
			.is_some_and(|instance| instance.def_id() != f)
}

/// Whether std's iterator `ty` has its own `min` or `max`, by `name`, which reads its first
/// or last item, as a B-tree's do.
fn btree_end(tcx: ty::TyCtxt<'_>, ty: Ty<'_>, name: &str) -> bool {
	let Some(path) = std_iter_path(tcx, ty) else {
		return false;
	};
	let path: Vec<&str> = path.iter().map(String::as_str).collect();
	match path[..] {
		[
			"collections",
			"btree",
			"map",
			"Iter" | "IterMut" | "Keys" | "Range" | "RangeMut" | "IntoKeys",
		]
		| ["collections", "btree", "set", "Iter" | "Range"] => true,
		[
			"collections",
			"btree",
			"set",
			"Difference" | "SymmetricDifference" | "Intersection" | "Union",
		] => name == "min",
		_ => false,
	}
}
