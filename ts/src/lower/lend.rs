//! The references to `[u8; N]`s that cross between translated code and std. Such an array
//! is a `Uint8Array`, where a `&[u8; N]` is an `rt.ByteSlice` of its bytes, as a `&[u8]`
//! is, but std's mappings take and hand out a reference to an array as the array itself.
//! So each `&[u8; N]` std makes of an array it holds, which it returns or hands a closure,
//! is lent as a slice of it, and each std is given as a `&T` of a `T = [u8; N]` is given
//! as a view of the slice's bytes. The function's generic signature tells such a
//! reference from one std takes as it is, which is an `rt.ByteSlice` on both sides: a `&T`
//! where `T = [u8; N]`, not a `T` where `T = &[u8; N]`.

use rustc_hir::def_id::DefId;
use rustc_middle::ty::{self, EarlyBinder, GenericArgsRef, Instance, Ty, TypingEnv};
use rustc_span::Span;

use super::Translator;
use super::body::Body;
use super::library::{self, Std};
use crate::ts::{Expr, Param, Stmt, StmtKind};

/// What a call of std's function makes of the `[u8; N]`s it holds.
pub enum Plan<'tcx> {
	/// It lends none, or its mapping makes each slice itself.
	Untouched,
	/// It lends some, which [`lend_closure`] and [`lend`] make slices, or is given some,
	/// which [`give`] makes views.
	Lend(Lent<'tcx>),
	/// It lends or is given one where none of them can.
	Refused,
}

/// A type in std's generic code, and as instantiated.
pub type Pair<'tcx> = (Ty<'tcx>, Ty<'tcx>);

/// Where a call lends `&[u8; N]`s, or is given them.
pub struct Lent<'tcx> {
	/// The closures it lends them to.
	pub closures: Vec<Closure<'tcx>>,
	/// What the call returns, if it lends one.
	pub output: Option<Pair<'tcx>>,
	/// The inputs, by index, it is given them in.
	pub given: Vec<(usize, Pair<'tcx>)>,
}

/// A closure std lends a `&[u8; N]` to.
pub struct Closure<'tcx> {
	/// Its index among the call's inputs.
	pub index: usize,
	/// Its type.
	pub ty: Ty<'tcx>,
	/// What it is given.
	pub params: Vec<Pair<'tcx>>,
	/// Whether what the call returns holds it.
	pub held: bool,
}

/// What a call of std's `def_id` lends.
pub fn plan<'tcx>(t: &Translator<'_, 'tcx>, def_id: DefId, args: GenericArgsRef<'tcx>) -> Plan<'tcx> {
	let tcx = t.tcx;
	if t.translated(def_id, args) || !args.types().any(owns_bytes) {
		return Plan::Untouched;
	}
	let self_ty = args.types().next();
	// A collection's runtime finds a key by a reference to its bytes in either form, and
	// reads the other set of a set operation as it holds it.
	let reads = collection(t, def_id, args);
	if !reads {
		// Mappings that make the slice, or take it.
		if let Some((owner, name)) = library::key(tcx, def_id) {
			let handled = match (owner.as_str(), name.as_str()) {
				("Index" | "IndexMut", "index" | "index_mut") | ("Clone", "clone") | ("Argument", _) => true,
				// The runtime compares and hashes bytes in whichever form it is given them.
				("PartialOrd" | "Ord" | "PartialEq", _) | ("Hash", "hash") => true,
				("slice", "iter" | "iter_mut") | ("array", "each_ref" | "each_mut") => true,
				// Lowered from MIR, which writes through the slice.
				("mem", "swap" | "replace" | "take") => true,
				("IntoIterator", "into_iter") => self_ty.is_some_and(|s| {
					let s = super::iter::unbox_ref(tcx, s).peel_refs();
					matches!(s.kind(), ty::Slice(_) | ty::Array(..)) || t.std_of(s) == Some(Std::Vec)
				}),
				_ => false,
			};
			if handled {
				return Plan::Untouched;
			}
		}
		// A closure is the crate's own.
		if tcx.trait_of_assoc(def_id).is_some_and(|tr| tcx.is_fn_trait(tr)) {
			return Plan::Untouched;
		}
	}
	let sig = Sig::of(t, def_id, args);
	let mut lent = Lent {
		closures: Vec::new(),
		output: None,
		given: Vec::new(),
	};
	for (i, (generic, ty)) in sig.inputs().enumerate() {
		if let Some((params, output, by_ref)) = sig.called(t, i) {
			let Some(lends_any) = params
				.iter()
				.try_fold(false, |any, &(g, p)| Some(lends(t, g, p)? || any))
			else {
				return Plan::Refused;
			};
			if !lends_any {
				continue;
			}
			// What the closure returns std holds as it is given it, and a closure std takes by
			// reference is no value to wrap.
			let returned = match output {
				Some((g, out)) => lends(t, g, out) == Some(false),
				None => false,
			};
			if !returned || by_ref {
				return Plan::Refused;
			}
			lent.closures.push(Closure {
				index: i,
				ty,
				params,
				held: sig.generic.output().walk().any(|a| a.as_type() == Some(generic)),
			});
			continue;
		}
		match lends(t, generic, ty) {
			Some(false) => {}
			Some(true) if reads => {}
			Some(true) if gives(t, generic, ty) => lent.given.push((i, (generic, ty))),
			_ => return Plan::Refused,
		}
	}
	let (generic, ty) = (sig.generic.output(), sig.sig.output());
	match lends(t, generic, ty) {
		Some(false) => {}
		Some(true) => lent.output = Some((generic, ty)),
		None => return Plan::Refused,
	}
	match lent.closures.is_empty() && lent.output.is_none() && lent.given.is_empty() {
		true => Plan::Untouched,
		false => Plan::Lend(lent),
	}
}

/// Whether `def_id` is a method of a collection whose runtime reads the bytes it is given
/// in either form.
fn collection<'tcx>(t: &Translator<'_, 'tcx>, def_id: DefId, args: GenericArgsRef<'tcx>) -> bool {
	let tcx = t.tcx;
	// An inherent method's type, or a trait method's `Self`.
	let coll = match tcx.inherent_impl_of_assoc(def_id) {
		Some(imp) => {
			let generics = tcx.generics_of(imp).count();
			tcx.type_of(imp).instantiate(tcx, &args[..generics]).skip_norm_wip()
		}
		None => match args.types().next() {
			Some(ty) => ty.peel_refs(),
			None => return false,
		},
	};
	t.std_of(coll).is_some_and(super::collections::lending)
}

/// Whether `ty` holds a `[u8; N]` other than behind a reference or in a closure, which std
/// only calls.
fn owns_bytes(ty: Ty<'_>) -> bool {
	match ty.kind() {
		_ if super::byte_array(ty) => true,
		ty::Array(elem, _) | ty::Slice(elem) => owns_bytes(*elem),
		ty::Tuple(tys) => tys.iter().any(owns_bytes),
		ty::Adt(_, args) | ty::Alias(_, ty::AliasTy { args, .. }) => args.types().any(owns_bytes),
		_ => false,
	}
}

/// A function's signature for its arguments, beside its generic one. A trait method's is
/// the trait's, whose `Self::Item` is an iterator's item as it was made, not as an impl's
/// `next` makes it.
pub struct Sig<'tcx> {
	def_id: DefId,
	args: GenericArgsRef<'tcx>,
	generic: ty::FnSig<'tcx>,
	sig: ty::FnSig<'tcx>,
}

impl<'tcx> Sig<'tcx> {
	pub fn of(t: &Translator<'_, 'tcx>, def_id: DefId, args: GenericArgsRef<'tcx>) -> Self {
		let tcx = t.tcx;
		let generic = tcx.fn_sig(def_id).instantiate_identity().skip_norm_wip();
		let generic = normalize(
			tcx,
			TypingEnv::post_analysis(tcx, def_id),
			tcx.instantiate_bound_regions_with_erased(generic),
		);
		Sig {
			def_id,
			args,
			generic,
			sig: t.sig(def_id, args),
		}
	}

	fn inputs(&self) -> impl Iterator<Item = (Ty<'tcx>, Ty<'tcx>)> {
		std::iter::zip(self.generic.inputs().iter().copied(), self.sig.inputs().iter().copied())
	}

	/// Input `i`'s parameters and output, generic and instantiated, if it is a closure a
	/// function trait bounds, and whether it is taken by reference.
	fn called(&self, t: &Translator<'_, 'tcx>, i: usize) -> Option<(Vec<Pair<'tcx>>, Option<Pair<'tcx>>, bool)> {
		let tcx = t.tcx;
		let input = self.generic.inputs()[i];
		let f = input.peel_refs();
		if !matches!(f.kind(), ty::Param(_)) {
			return None;
		}
		let bounds = tcx.param_env(self.def_id).caller_bounds();
		let trait_pred = (bounds.clone().filter_map(|c| c.as_trait_clause()))
			.map(|p| tcx.instantiate_bound_regions_with_erased(p))
			.find(|p| p.self_ty() == f && tcx.is_fn_trait(p.def_id()))?;
		let ty::Tuple(params) = trait_pred.trait_ref.args.type_at(1).kind() else {
			return None;
		};
		let output = (bounds.filter_map(|c| c.as_projection_clause()))
			.map(|p| tcx.instantiate_bound_regions_with_erased(p))
			.find(|p| p.self_ty() == f && tcx.is_lang_item(p.def_id(), rustc_attr_ir::LangItem::FnOnceOutput))
			.and_then(|p| p.term.as_type());
		let pair = |g: Ty<'tcx>| {
			let ty = EarlyBinder::bind(tcx, g).instantiate(tcx, self.args).skip_norm_wip();
			(g, normalize(tcx, t.typing_env_pub(), ty))
		};
		Some((params.iter().map(pair).collect(), output.map(pair), input.is_ref()))
	}
}

fn normalize<'tcx, T: ty::TypeFoldable<ty::TyCtxt<'tcx>> + Copy>(
	tcx: ty::TyCtxt<'tcx>,
	env: TypingEnv<'tcx>,
	value: T,
) -> T {
	tcx.try_normalize_erasing_regions(env, ty::Unnormalized::new_wip(value))
		.unwrap_or(value)
}

/// Whether a `ty`, whose type in std's generic code is `generic`, holds a `&[u8; N]` std
/// made of an array it holds: `Some(true)` if it does, by itself or in a tuple, an
/// `Option`, or an iterator's items, which [`lend`] reaches, `Some(false)` if it holds
/// none, and `None` if it holds one [`lend`] cannot reach.
pub fn lends<'tcx>(t: &Translator<'_, 'tcx>, generic: Ty<'tcx>, ty: Ty<'tcx>) -> Option<bool> {
	match (generic.kind(), ty.kind()) {
		// What std was given, as it was given it.
		(ty::Param(_) | ty::Alias(..), _) => Some(false),
		(_, ty::Ref(_, inner, _)) if super::byte_array(*inner) => Some(true),
		// A reference to what was given holds none std made.
		(ty::Ref(_, generic, _), ty::Ref(_, inner, _)) => lends(t, *generic, *inner).filter(|&lent| !lent),
		(ty::Tuple(generics), ty::Tuple(parts)) => std::iter::zip(generics.iter(), parts.iter())
			.try_fold(false, |any, (g, part)| Some(lends(t, g, part)? || any)),
		_ if let (Some(g), Some(inner)) = (option_of(t, generic), option_of(t, ty)) => lends(t, g, inner),
		_ if let Some((g, item)) = item(t, ty) => match g {
			Some(g) => lends(t, g, item),
			None => Some(false),
		},
		// Another of std's types, holding what it was given as it was.
		(ty::Adt(g, generics), ty::Adt(adt, args)) if g.did() == adt.did() => {
			let held = std::iter::zip(generics.types(), args.types()).all(|(g, arg)| lends(t, g, arg) == Some(false));
			held.then_some(false)
		}
		_ => (!holds_bytes_ref(ty)).then_some(false),
	}
}

/// What an `Option` holds.
fn option_of<'tcx>(t: &Translator<'_, 'tcx>, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
	match ty.kind() {
		ty::Adt(_, args) if t.std_of(ty) == Some(Std::Option) => Some(args.type_at(0)),
		_ => None,
	}
}

/// The items of an iterator `ty`, beside their type in the generic code of std's `next`
/// that makes them: `None` for an iterator not std's, whose items translated code made.
fn item<'tcx>(t: &Translator<'_, 'tcx>, ty: Ty<'tcx>) -> Option<(Option<Ty<'tcx>>, Ty<'tcx>)> {
	if matches!(ty.kind(), ty::Ref(..) | ty::Tuple(_)) {
		return None;
	}
	let tcx = t.tcx;
	let item = t.iter_item(ty)?;
	let next = super::iter::next_fn(tcx)?;
	let generic = match Instance::try_resolve(tcx, t.typing_env_pub(), next, tcx.mk_args(&[ty.into()])) {
		Ok(Some(Instance {
			def: ty::InstanceKind::Item(did),
			..
		})) if library::is_std(tcx, did) => {
			let sig = tcx.fn_sig(did).instantiate_identity().skip_norm_wip();
			let sig = tcx.instantiate_bound_regions_with_erased(sig);
			option_of(t, normalize(tcx, TypingEnv::post_analysis(tcx, did), sig.output()))
		}
		_ => None,
	};
	Some((generic, item))
}

/// Whether `ty` holds a `&[u8; N]`, as a closure's signature does if it takes one.
pub fn holds_bytes_ref(ty: Ty<'_>) -> bool {
	ty.walk().any(|arg| arg.as_type().is_some_and(super::bytes_ref))
}

/// `value`, a `ty` std's mapping hands out, whose type in std's generic code is `generic`,
/// as translated code reads it: each `&[u8; N]` std made of an array it holds, which the
/// mapping hands out as the `Uint8Array`, an `rt.ByteSlice` of it. It shares the bytes,
/// which nothing writes through a `&`, and a `&mut` writes where std holds them.
pub fn lend<'tcx>(b: &mut Body<'_, '_, 'tcx>, generic: Ty<'tcx>, ty: Ty<'tcx>, value: Expr, span: Span) -> Expr {
	cross(b, generic, ty, value, Way::Lent, span)
}

/// `value`, a `ty` translated code gives std's mapping as an input whose type in std's
/// generic code is `generic`, as the mapping reads it: each `&[u8; N]` where it takes a
/// `&T`, which it reads, copies, and writes through as the array, an `rt.byteSliceView`
/// of the `rt.ByteSlice`. It shares the bytes for as long as the reference lives, so
/// whatever the mapping keeps of it reads and writes where the reference does.
pub fn give<'tcx>(b: &mut Body<'_, '_, 'tcx>, generic: Ty<'tcx>, ty: Ty<'tcx>, value: Expr, span: Span) -> Expr {
	cross(b, generic, ty, value, Way::Given, span)
}

/// Which way a `&[u8; N]` crosses between translated code and std's mapping.
#[derive(Clone, Copy, PartialEq)]
enum Way {
	/// To translated code, from the `Uint8Array` std holds: [`lend`].
	Lent,
	/// To std's mapping, from the `rt.ByteSlice` translated code holds: [`give`].
	Given,
}

/// Whether [`give`] reaches each `&[u8; N]` std is given in a `ty`, whose type in std's
/// generic code is `generic`: by itself, or in a tuple or an `Option`. An iterator's items
/// it does not reach, which std's own iterator holds as arrays already.
fn gives<'tcx>(t: &Translator<'_, 'tcx>, generic: Ty<'tcx>, ty: Ty<'tcx>) -> bool {
	match (generic.kind(), ty.kind()) {
		(ty::Param(_) | ty::Alias(..), _) => true,
		(_, ty::Ref(_, inner, _)) if super::byte_array(*inner) => true,
		(ty::Tuple(generics), ty::Tuple(parts)) => {
			std::iter::zip(generics.iter(), parts.iter()).all(|(g, part)| gives(t, g, part))
		}
		_ if let (Some(g), Some(inner)) = (option_of(t, generic), option_of(t, ty)) => gives(t, g, inner),
		_ => lends(t, generic, ty) == Some(false),
	}
}

/// [`lend`] or [`give`], as `way` says.
fn cross<'tcx>(b: &mut Body<'_, '_, 'tcx>, generic: Ty<'tcx>, ty: Ty<'tcx>, value: Expr, way: Way, span: Span) -> Expr {
	if lends(b.t, generic, ty) != Some(true) {
		return value;
	}
	if let (Some(g), Some(inner)) = (option_of(b.t, generic), option_of(b.t, ty)) {
		let f = cross_fn(b, g, inner, way, span);
		return Expr::rt("optionMap", vec![value, f]);
	}
	if way == Way::Lent
		&& let Some((Some(g), item)) = item(b.t, ty)
	{
		let f = cross_fn(b, g, item, way, span);
		return Expr::rt("iterLend", vec![value, f]);
	}
	if let (ty::Tuple(generics), ty::Tuple(parts)) = (generic.kind(), ty.kind())
		&& value.is_path()
	{
		let parts: Vec<Expr> = std::iter::zip(generics.iter(), parts.iter())
			.enumerate()
			.map(|(i, (g, part))| cross(b, g, part, value.clone().index(Expr::lit(i.to_string())), way, span))
			.collect();
		return Expr::Array(parts);
	}
	cross_fn(b, generic, ty, way, span).call(vec![value])
}

/// A function that [`cross`]es a `ty`.
fn cross_fn<'tcx>(b: &mut Body<'_, '_, 'tcx>, generic: Ty<'tcx>, ty: Ty<'tcx>, way: Way, span: Span) -> Expr {
	if let ty::Ref(_, inner, _) = ty.kind()
		&& super::byte_array(*inner)
	{
		return Expr::Rt(
			match way {
				Way::Lent => "byteSliceFrom",
				Way::Given => "byteSliceView",
			}
			.into(),
		);
	}
	let name = b.fresh("held");
	let body = cross(b, generic, ty, Expr::ident(&name), way, span);
	let param = Param { name, ty: None };
	match way {
		Way::Lent => Expr::Arrow(
			vec![param],
			b.t.ty(ty, span),
			vec![StmtKind::Return(Some(body)).at(None)],
		),
		// What the mapping takes, which it infers.
		Way::Given => Expr::Lambda(vec![param], Box::new(body)),
	}
}

/// `f`, a closure std calls with `params`, each generic and instantiated, given each as
/// [`lend`] makes it. One that what the call returns holds, or that drops, which the
/// mapping may drop through what it is given, is `rt.lending`, `holds`, which clones and
/// drops as `f` does.
pub fn lend_closure<'tcx>(
	b: &mut Body<'_, '_, 'tcx>,
	f: Expr,
	params: &[Pair<'tcx>],
	holds: bool,
	span: Span,
	out: &mut Vec<Stmt>,
) -> Expr {
	// Made once, as std takes it.
	let f = match f.is_path() {
		true => f,
		false => b.spill(f, "f", out),
	};
	let called = match holds {
		true => Expr::ident(b.fresh("f")),
		false => f.clone(),
	};
	let names: Vec<String> = params.iter().map(|_| b.fresh("held")).collect();
	let values = (params.iter().zip(&names))
		.map(|(&(g, p), name)| lend(b, g, p, Expr::ident(name), span))
		.collect();
	let names = names.into_iter().map(|name| Param { name, ty: None }).collect();
	let lent = Expr::Lambda(names, Box::new(called.clone().call(values)));
	match (holds, called) {
		(true, Expr::Ident(name)) => {
			let make = Expr::Lambda(vec![Param { name, ty: None }], Box::new(lent));
			Expr::rt("lending", vec![f, make])
		}
		_ => lent,
	}
}

/// A function copying a `ty` out of what `Extend<&T>` and `Extend<(&K, &V)>` are given,
/// where each `[u8; N]` is the `rt.ByteSlice` a `&[u8; N]` is: `None` if a copy is the
/// value itself.
pub fn copy_fn<'tcx>(b: &mut Body<'_, '_, 'tcx>, ty: Ty<'tcx>, span: Span) -> Option<Expr> {
	match ty.kind() {
		_ if super::byte_array(ty) => Some(Expr::Rt("byteSliceToArray".into())),
		ty::Tuple(parts) if parts.iter().any(super::byte_array) => {
			let name = b.fresh("held");
			let parts = (parts.iter().enumerate())
				.map(|(i, part)| {
					let value = Expr::ident(&name).index(Expr::lit(i.to_string()));
					match copy_fn(b, part, span) {
						Some(copy) => copy.call(vec![value]),
						None => value,
					}
				})
				.collect();
			Some(Expr::Arrow(
				vec![Param { name, ty: None }],
				b.t.ty(ty, span),
				vec![StmtKind::Return(Some(Expr::Array(parts))).at(None)],
			))
		}
		_ => b.t.copy_fn(ty, span),
	}
}
