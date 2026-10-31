//! Locals of byte slices held as their parts. A `&[u8]` is an `rt.ByteSlice`, so each
//! split, and each tuple, `Option`, `Result`, or `ControlFlow` around one, makes objects
//! V8 doesn't remove. A local built of them that nothing borrows or writes into, and
//! that only splits, `ok_or`, `?`, aggregates, and copies assign, is instead a value per
//! part: a slice's bytes, start, and length, a flag for which variant an enum is, and
//! anything else, as an error, whole. A read that takes it apart reads its parts; any
//! other read makes the object there. A local whose object would be made more than once
//! on a path, or in a loop that doesn't make its parts again, stays an object, so
//! nothing is made more often than before, as does one only ever assigned an object's
//! parts, which saves none. A function that hands back a `Result` in a slot holds its
//! result so, to hand back the `Ok`'s parts, and its callers read them back as parts.

use rustc_abi::VariantIdx;
use rustc_data_structures::fx::{FxHashMap, FxHashSet};
use rustc_middle::mir::visit::{MutatingUseContext, NonMutatingUseContext, PlaceContext, Visitor};
use rustc_middle::mir::{
	self, AggregateKind, BasicBlock, CastKind, Local, Location, Operand, Place, PlaceElem, ProjectionElem, Rvalue,
	StatementKind, TerminatorKind,
};
use rustc_middle::ty::adjustment::PointerCoercion;
use rustc_middle::ty::{self, Ty};
use rustc_span::{Span, Spanned};

use super::Fx;
use crate::lower::library::{self, Std};
use crate::ts::{Expr, Key, Stmt, StmtKind, Type};

/// How a local held as its parts lays them out.
#[derive(Clone, Debug)]
pub enum Shape<'tcx> {
	/// No JS value: `()`.
	Unit,
	/// One JS value, held whole: a number, a reference to one, or an error.
	Leaf(Ty<'tcx>),
	/// `&[u8]` or `&[u8; N]`: its bytes, start, and length.
	Slice,
	Tuple(Ty<'tcx>, Vec<Shape<'tcx>>),
	/// An `Option`, `Result`, or `ControlFlow`: whether it is its second variant, if both
	/// can be, then the fields of each variant that can be, in order.
	Enum(Ty<'tcx>, [Option<Vec<Shape<'tcx>>>; 2]),
}

impl<'tcx> Shape<'tcx> {
	fn size(&self) -> usize {
		match self {
			Shape::Unit => 0,
			Shape::Leaf(_) => 1,
			Shape::Slice => 3,
			Shape::Tuple(_, fields) => fields.iter().map(Shape::size).sum(),
			Shape::Enum(_, variants) => {
				let flag = usize::from(variants.iter().all(Option::is_some));
				flag + variants.iter().flatten().flatten().map(Shape::size).sum::<usize>()
			}
		}
	}

	fn has_slice(&self) -> bool {
		match self {
			Shape::Slice => true,
			Shape::Unit | Shape::Leaf(_) => false,
			Shape::Tuple(_, fields) => fields.iter().any(Shape::has_slice),
			Shape::Enum(_, variants) => variants.iter().flatten().flatten().any(Shape::has_slice),
		}
	}

	fn has_enum(&self) -> bool {
		match self {
			Shape::Enum(..) => true,
			Shape::Unit | Shape::Leaf(_) | Shape::Slice => false,
			Shape::Tuple(_, fields) => fields.iter().any(Shape::has_enum),
		}
	}

	/// Whether it has a flag, as an enum both of whose variants can be.
	fn flagged(&self) -> bool {
		matches!(self, Shape::Enum(_, [Some(_), Some(_)]))
	}

	/// Where the fields of variant `v` of an enum start.
	fn variant_at(&self, v: usize) -> usize {
		let Shape::Enum(_, variants) = self else {
			unreachable!("a variant of an enum")
		};
		let before: usize = variants[..v].iter().flatten().flatten().map(Shape::size).sum();
		usize::from(self.flagged()) + before
	}

	/// Each part: its name's suffix, and what it holds.
	fn parts(&self, suffix: &str, out: &mut Vec<(String, Part<'tcx>)>) {
		match self {
			Shape::Unit => {}
			Shape::Leaf(ty) => out.push((suffix.to_string(), Part::Value(*ty))),
			Shape::Slice => {
				for (key, part) in [("Data", Part::Data), ("Start", Part::Number), ("Len", Part::Number)] {
					out.push((format!("{suffix}{key}"), part));
				}
			}
			Shape::Tuple(_, fields) => {
				for (i, field) in fields.iter().enumerate() {
					field.parts(&format!("{suffix}_{i}"), out);
				}
			}
			Shape::Enum(ty, variants) => {
				if self.flagged() {
					out.push((format!("{suffix}{}", flag_name(*ty)), Part::Flag));
				}
				for fields in variants.iter().flatten() {
					for (i, field) in fields.iter().enumerate() {
						let suffix = match fields.len() {
							1 => suffix.to_string(),
							_ => format!("{suffix}_{i}"),
						};
						field.parts(&suffix, out);
					}
				}
			}
		}
	}

	/// The parts that are JS objects as the local's object holds them: each slice, by its
	/// bytes, and each value held whole that is no number.
	fn objects(&self, at: usize, out: &mut Vec<usize>) {
		match self {
			Shape::Unit => {}
			Shape::Leaf(ty) => {
				if !number(*ty) {
					out.push(at);
				}
			}
			Shape::Slice => out.push(at),
			Shape::Tuple(_, fields) => {
				let mut at = at;
				for field in fields {
					field.objects(at, out);
					at += field.size();
				}
			}
			Shape::Enum(_, variants) => {
				let mut at = at + usize::from(self.flagged());
				for field in variants.iter().flatten().flatten() {
					field.objects(at, out);
					at += field.size();
				}
			}
		}
	}
}

/// What a part holds, which types its variable.
#[derive(Clone, Copy, Debug)]
enum Part<'tcx> {
	Flag,
	Value(Ty<'tcx>),
	Data,
	Number,
}

/// The name of the flag of an enum, for which variant it is its second.
fn flag_name(ty: Ty<'_>) -> &'static str {
	match ty.kind() {
		ty::Adt(adt, _) if adt.variants().len() == 2 => match adt.variant(VariantIdx::from_usize(1)).name.as_str() {
			"Some" => "Some",
			"Err" => "Err",
			_ => "Break",
		},
		_ => "Flag",
	}
}

/// Whether a `ty`'s JS value is a number or flag, which making its object makes nothing of.
fn number(ty: Ty<'_>) -> bool {
	match ty.kind() {
		ty::Bool | ty::Char | ty::Int(_) | ty::Uint(_) | ty::Float(_) => true,
		ty::Ref(_, inner, _) => number(*inner),
		_ => false,
	}
}

/// Whether no value of `ty` can be made.
fn uninhabited(ty: Ty<'_>) -> bool {
	ty.is_never() || matches!(ty.kind(), ty::Adt(adt, _) if adt.is_enum() && adt.variants().is_empty())
}

impl<'tcx> crate::lower::Translator<'_, 'tcx> {
	/// The function of `instance` if only translated code calls it directly, so it can
	/// take and return what the outside can't: a function or method, not a closure or a
	/// `const`, and not a root, an `async fn`, or the crate's own `next`; and not in a
	/// census, which writes nothing.
	fn inside(&self, instance: ty::Instance<'tcx>) -> Option<rustc_hir::def_id::DefId> {
		let tcx = self.tcx;
		let ty::InstanceKind::Item(did) = instance.def else {
			return None;
		};
		let outside = !matches!(
			tcx.def_kind(did),
			rustc_hir::def::DefKind::Fn | rustc_hir::def::DefKind::AssocFn
		) || self.census
			|| self.roots.contains(&instance)
			|| !self.lowers(instance)
			|| tcx.asyncness(did).is_async()
			|| crate::lower::iter::is_next(tcx, did);
		(!outside).then_some(did)
	}

	/// `instance`'s parameter types, then its result's, their regions erased and their
	/// types normalized.
	fn inside_sig(&self, did: rustc_hir::def_id::DefId, instance: ty::Instance<'tcx>) -> (Vec<Ty<'tcx>>, Ty<'tcx>) {
		let tcx = self.tcx;
		let sig = tcx.fn_sig(did).instantiate(tcx, instance.args).skip_norm_wip();
		let sig = tcx.instantiate_bound_regions_with_erased(sig);
		let normalize = |ty: Ty<'tcx>| {
			tcx.try_normalize_erasing_regions(self.typing_env_pub(), ty::Unnormalized::new_wip(ty))
				.unwrap_or(ty)
		};
		(
			sig.inputs().iter().map(|&ty| normalize(ty)).collect(),
			normalize(sig.output()),
		)
	}

	/// Which of `instance`'s parameters it takes as a byte slice's parts, its `data`,
	/// `start`, and `length`, rather than an `rt.ByteSlice`: each `&[u8]` or `&[u8; N]` of a
	/// function only translated code calls directly. Empty if none.
	pub(crate) fn parted(&self, instance: ty::Instance<'tcx>) -> Vec<bool> {
		let Some(did) = self.inside(instance) else {
			return Vec::new();
		};
		let parted: Vec<bool> = self.inside_sig(did, instance).0.into_iter().map(slice_ref).collect();
		match parted.contains(&true) {
			true => parted,
			false => Vec::new(),
		}
	}

	/// Where `instance` hands back its `Ok`'s parts, if it does: a function of the crate's
	/// that only translated code calls directly, and that returns a `Result` of byte slices
	/// and scalars, such as `Result<(u64, &[u8]), E>`, returns the `Ok`'s first part, which
	/// is never `undefined`, or `undefined` for an error. It writes the rest, or the error,
	/// which must be one value, into a module-level slot its caller reads at once.
	pub(crate) fn slotted(&mut self, instance: ty::Instance<'tcx>) -> Option<Slotted<'tcx>> {
		if let Some(slotted) = self.slotted.get(&instance) {
			return slotted.clone();
		}
		let slotted = self.find_slot(instance);
		self.slotted.insert(instance, slotted.clone());
		slotted
	}

	fn find_slot(&mut self, instance: ty::Instance<'tcx>) -> Option<Slotted<'tcx>> {
		let did = self.inside(instance)?;
		// Std's are small, and their callers take the result apart as they are.
		if !self.crates.translates(did) {
			return None;
		}
		let ret = self.inside_sig(did, instance).1;
		let ty::Adt(adt, args) = ret.kind() else {
			return None;
		};
		if Std::of(self.tcx, adt.did()) != Some(Std::Result) || self.drops(ret) {
			return None;
		}
		let (ok, err) = (args.type_at(0), args.type_at(1));
		if uninhabited(ok) || uninhabited(err) || matches!(err.kind(), ty::FnDef(..)) {
			return None;
		}
		let shape = self.shape(ok)?;
		if !shape.has_slice() || shape.has_enum() || !matches!(self.shape(err), Some(Shape::Leaf(_))) {
			return None;
		}
		let mut parts = Vec::new();
		shape.parts("", &mut parts);
		// The first part is returned, so `undefined` must mean an error.
		if let Part::Value(first) = parts[0].1 {
			let erased = self.erases_to(first);
			if erased.undefined || erased.none || erased.param || erased.pending || erased.outer_pending {
				return None;
			}
		}
		let fields: Vec<String> = parts.iter().map(|(suffix, _)| format!("p{suffix}")).collect();
		let sym = match self.slots.get(&ok) {
			Some(&sym) => sym,
			None => self.slot(ok, instance, &parts, &fields),
		};
		self.place_slot(sym, instance);
		Some(Slotted {
			sym,
			ret,
			err,
			ok: shape,
			fields,
		})
	}

	/// The slot of `Ok` payloads of type `ok`, which every function returning one shares:
	/// the error, of any of their error types, then each of `fields` but the first part's,
	/// which the function returns. Each starts `undefined`, so V8 keeps it tagged rather
	/// than boxing every number past 2^31 that a function too big to inline writes.
	fn slot(
		&mut self,
		ok: Ty<'tcx>,
		instance: ty::Instance<'tcx>,
		parts: &[(String, Part<'tcx>)],
		fields: &[String],
	) -> crate::ts::SymId {
		let span = self.tcx.def_span(instance.def_id());
		let sym = self.program.sym(crate::program::Symbol {
			module: Vec::new(),
			base: String::new(),
			suffix: String::new(),
			owner: None,
			kind: crate::program::SymKind::Value,
			order: u32::MAX,
			span,
			instance: String::new(),
			name: String::new(),
			alias: None,
		});
		let tys: Vec<Type> = parts.iter().map(|(_, part)| self.part_ty(*part, span)).collect();
		let never = Expr::As(Box::new(Expr::undefined()), Type::Builtin("never"));
		let held = std::iter::zip(&fields[1..], tys.into_iter().skip(1));
		self.program.items.push(crate::ts::Item::Value {
			sym,
			ty: Type::Object(
				std::iter::once((ERR.to_string(), Type::Builtin("unknown")))
					.chain(held.map(|(field, ty)| (field.clone(), ty)))
					.collect(),
			),
			doc: None,
			value: Expr::Object(
				std::iter::once((Key::Name(ERR.into()), Expr::undefined()))
					.chain(fields[1..].iter().map(|field| (Key::Name(field.clone()), never.clone())))
					.collect(),
			),
		});
		self.slots.insert(ok, sym);
		sym
	}

	/// Put the slot `sym` beside `instance`, named for it and suffixed as it is, if it comes
	/// first in the source of the functions that share it, whatever order translation
	/// reaches them in.
	fn place_slot(&mut self, sym: crate::ts::SymId, instance: ty::Instance<'tcx>) {
		let owner = self.instance(instance);
		let owner = &self.program.syms[owner];
		let (order, module, suffix) = (owner.order, owner.module.clone(), owner.suffix.clone());
		let slot = &self.program.syms[sym];
		if !slot.base.is_empty() && (order, &module, &suffix) >= (slot.order, &slot.module, &slot.suffix) {
			return;
		}
		let base = crate::lower::names::slot_base(self.tcx, instance.def_id());
		let slot = &mut self.program.syms[sym];
		(slot.order, slot.module, slot.base, slot.suffix) = (order, module, base, suffix);
	}

	/// What a function that hands back its `Ok`'s parts in `slotted` returns: the first
	/// part, or `undefined` for an error.
	pub(crate) fn slot_ret_ty(&mut self, slotted: &Slotted<'tcx>, span: Span) -> Type {
		let mut parts = Vec::new();
		slotted.ok.parts("", &mut parts);
		Type::Union(vec![self.part_ty(parts[0].1, span), Type::Builtin("undefined")])
	}

	/// The type of a variable or field that holds `part`.
	fn part_ty(&mut self, part: Part<'tcx>, span: Span) -> Type {
		match part {
			Part::Flag => Type::Builtin("boolean"),
			Part::Data => Type::Builtin("Uint8Array"),
			Part::Number => Type::Builtin("number"),
			Part::Value(ty) => self.held_ty(ty, span),
		}
	}

	/// How a local of `ty` is laid out as its parts, if it can be.
	pub(crate) fn shape(&self, ty: Ty<'tcx>) -> Option<Shape<'tcx>> {
		let tcx = self.tcx;
		match ty.kind() {
			_ if slice_ref(ty) => Some(Shape::Slice),
			ty::Tuple(tys) if tys.is_empty() => Some(Shape::Unit),
			ty::Tuple(tys) => {
				let fields = tys.iter().map(|t| self.shape(t)).collect::<Option<Vec<_>>>()?;
				Some(Shape::Tuple(ty, fields))
			}
			ty::Adt(adt, args)
				if matches!(
					Std::of(tcx, adt.did()),
					Some(Std::Option | Std::Result | Std::ControlFlow)
				) =>
			{
				let variant = |v: usize| -> Option<Option<Vec<Shape<'tcx>>>> {
					let tys: Vec<Ty<'tcx>> = adt
						.variant(VariantIdx::from_usize(v))
						.fields
						.iter()
						.map(|f| f.ty(tcx, args).skip_norm_wip())
						.collect();
					if tys.iter().any(|t| uninhabited(*t)) {
						return Some(None);
					}
					Some(Some(
						tys.into_iter().map(|t| self.shape(t)).collect::<Option<Vec<_>>>()?,
					))
				};
				Some(Shape::Enum(ty, [variant(0)?, variant(1)?]))
			}
			ty::Ref(_, _, rustc_ast::Mutability::Mut) | ty::RawPtr(..) => None,
			_ if self.needs_copy(ty) => None,
			_ => Some(Shape::Leaf(ty)),
		}
	}
}

/// The slot's field that holds an error.
const ERR: &str = "err";

/// Where a function hands back its `Ok`'s parts: it returns the first, or `undefined` for
/// an error, and writes the rest, or the error, into a module-level slot.
#[derive(Clone, Debug)]
pub struct Slotted<'tcx> {
	pub sym: crate::ts::SymId,
	/// The `Result` the function returns in Rust.
	pub ret: Ty<'tcx>,
	/// Its error type.
	pub err: Ty<'tcx>,
	/// The `Ok` payload's layout, which has no enum.
	pub ok: Shape<'tcx>,
	/// Each part's field in the slot, in order, but the first's, which is returned.
	pub fields: Vec<String>,
}

impl<'tcx> Slotted<'tcx> {
	/// The slot's field of part `i`, after the first, read or written.
	pub fn field(&self, i: usize) -> Expr {
		assert!(i > 0, "the first part is returned");
		Expr::Sym(self.sym).member(self.fields[i].clone())
	}

	/// The slot's field of the error.
	pub fn err_field(&self) -> Expr {
		Expr::Sym(self.sym).member(ERR)
	}

	/// What a caller that keeps the result whole makes of what the call returned, `first`
	/// (a name), read at once: the `Ok` of it and the slot's parts, or the `Err`.
	pub fn rebuild(&self, t: &mut crate::lower::Translator<'_, 'tcx>, first: &str, span: Span) -> Expr {
		let parts: Vec<Expr> = std::iter::once(Expr::ident(first))
			.chain((1..self.fields.len()).map(|i| self.field(i)))
			.collect();
		let ok = build_plain(t, &self.ok, &parts, span);
		let err_ty = t.ty(self.err, span);
		Expr::Cond(
			Box::new(Expr::ident(first).binary("===", Expr::undefined())),
			Box::new(Expr::rt("Err", vec![Expr::As(Box::new(self.err_field()), err_ty)])),
			Box::new(Expr::rt("Ok", vec![ok])),
		)
	}
}

/// The value of `shape`, which has no enum, its `parts` make.
fn build_plain<'tcx>(
	t: &mut crate::lower::Translator<'_, 'tcx>,
	shape: &Shape<'tcx>,
	parts: &[Expr],
	span: Span,
) -> Expr {
	match shape {
		Shape::Unit => Expr::undefined(),
		Shape::Leaf(_) => parts[0].clone(),
		Shape::Slice => Expr::New(Box::new(Expr::Rt("ByteSlice".into())), parts.to_vec()),
		Shape::Tuple(ty, fields) => {
			let mut at = 0;
			let mut items = Vec::with_capacity(fields.len());
			for field in fields {
				items.push(build_plain(t, field, &parts[at..at + field.size()], span));
				at += field.size();
			}
			Expr::As(Box::new(Expr::Array(items)), t.ty(*ty, span))
		}
		Shape::Enum(..) => unreachable!("a slot's payload has no enum"),
	}
}

/// Return `undefined` from a function that hands back its `Ok`'s parts, with its error
/// `err` in the slot.
fn slot_fail(slot: &Slotted<'_>, err: Expr, loc: Option<crate::ts::Loc>) -> Vec<Stmt> {
	vec![
		StmtKind::Expr(slot.err_field().assign(err)).at(None),
		StmtKind::Return(Some(Expr::undefined())).at(loc),
	]
}

/// Whether `ty` is a `&[u8]` or `&[u8; N]`, which is an `rt.ByteSlice`.
fn slice_ref(ty: Ty<'_>) -> bool {
	matches!(ty.kind(), ty::Ref(_, inner, rustc_ast::Mutability::Not)
		if super::place::byte_slice(*inner) || crate::lower::byte_array(*inner))
}

/// A local held as its parts.
#[derive(Debug)]
pub(super) struct Scalar<'tcx> {
	shape: Shape<'tcx>,
	/// Each part's variable, which each assignment sets.
	vars: Vec<String>,
	/// For a local assigned once, once its assignment is lowered, each part's value, which
	/// reads the same wherever its local is read: a variable assigned only there, or what
	/// it was assigned.
	held: Option<Vec<Expr>>,
	/// Whether some statement assigns a variable, which then needs a declaration.
	assigned: bool,
}

/// The part of a shape a place reaches: where its parts start, and the variant it was
/// cast to, if it is an enum.
#[derive(Clone, Copy)]
struct Node<'s, 'tcx> {
	shape: &'s Shape<'tcx>,
	at: usize,
	variant: Option<usize>,
}

impl Node<'_, '_> {
	fn parts(&self) -> std::ops::Range<usize> {
		self.at..self.at + self.shape.size()
	}
}

/// The part of `shape` the projections `elems` reach, and how many of them that takes:
/// fields and casts to a variant, up to a slice's bytes, a value held whole, or the end.
fn walk<'s, 'tcx>(shape: &'s Shape<'tcx>, elems: &[PlaceElem<'tcx>]) -> (Node<'s, 'tcx>, usize) {
	let mut node = Node {
		shape,
		at: 0,
		variant: None,
	};
	for (i, elem) in elems.iter().enumerate() {
		match (node.shape, elem) {
			(Shape::Enum(..), ProjectionElem::Downcast(_, v)) => node.variant = Some(v.as_usize()),
			(Shape::Tuple(_, fields), ProjectionElem::Field(f, _)) => {
				let f = f.as_usize();
				node = Node {
					shape: &fields[f],
					at: node.at + fields[..f].iter().map(Shape::size).sum::<usize>(),
					variant: None,
				};
			}
			(Shape::Enum(_, variants), ProjectionElem::Field(f, _)) => {
				let Some(v) = node.variant else {
					return (node, i);
				};
				let Some(fields) = &variants[v] else {
					return (node, i);
				};
				let f = f.as_usize();
				node = Node {
					shape: &fields[f],
					at: node.at + node.shape.variant_at(v) + fields[..f].iter().map(Shape::size).sum::<usize>(),
					variant: None,
				};
			}
			_ => return (node, i),
		}
	}
	(node, elems.len())
}

/// A std call whose result a local holds as its parts.
#[derive(Clone, Copy, Debug)]
enum Producer {
	SplitFirst,
	SplitFirstChunk(u64),
	SplitAt {
		checked: bool,
	},
	OkOr,
	Branch,
	/// `?`'s `Err` of a `Result`'s residual, its error converted.
	FromResidual,
	/// A function that hands back its `Ok`'s parts in its slot.
	Slotted,
}

/// A part of a local, by index, and the part of another held local it copies, if any.
type PartCopy = (Local, usize, Option<(Local, usize)>);

/// A read of a local that makes its object, or part of it.
struct Made {
	block: BasicBlock,
	local: Local,
	parts: std::ops::Range<usize>,
}

impl<'tcx> Fx<'_, '_, '_, 'tcx> {
	/// How a local of `ty` is laid out as its parts, if it can be.
	fn shape(&self, ty: Ty<'tcx>) -> Option<Shape<'tcx>> {
		self.b.t.shape(ty)
	}

	/// The call `func`, ending `block`, makes into a local of `shape`, if its parts can be
	/// made from its arguments': a split of bytes, `ok_or` of an `Option`, or `?` on a
	/// `Result` or `Option`, each of a local held as its parts, or the `Err` that `?`
	/// returns; or from a slot.
	fn producer(
		&self,
		block: BasicBlock,
		func: &Operand<'tcx>,
		args: &[Spanned<Operand<'tcx>>],
		shape: &Shape<'tcx>,
		shapes: &FxHashMap<Local, Shape<'tcx>>,
	) -> Option<Producer> {
		if self.slotted_calls.contains_key(&block) {
			return Some(Producer::Slotted);
		}
		let tcx = self.tcx;
		let ty::FnDef(def_id, gargs) = *func.ty(&self.mir.local_decls, tcx).kind() else {
			return None;
		};
		if !library::is_std(tcx, def_id) {
			return None;
		}
		let gargs = gargs.skip_binder();
		let (owner, name) = library::key(tcx, def_id)?;
		// What `ok_or` and `?` take apart: a local held as its parts, read whole, an enum
		// whose variants both can be, as the result's.
		let held = |op: &Operand<'tcx>| match op {
			Operand::Copy(p) | Operand::Move(p) => shapes.get(&p.local).is_some_and(|source| {
				let (node, n) = walk(source, p.projection);
				n == p.projection.len() && node.shape.flagged() && shape.flagged()
			}),
			_ => false,
		};
		let bytes = || {
			gargs
				.types()
				.next()
				.is_some_and(|t| matches!(t.kind(), ty::Uint(ty::UintTy::U8)))
		};
		match (owner.as_str(), name.as_str()) {
			("slice", "split_first") if bytes() => Some(Producer::SplitFirst),
			("slice", "split_first_chunk") if bytes() => {
				let n = gargs.consts().next()?.try_to_target_usize(tcx)?;
				Some(Producer::SplitFirstChunk(n))
			}
			("slice", "split_at") if bytes() => Some(Producer::SplitAt { checked: false }),
			("slice", "split_at_checked") if bytes() => Some(Producer::SplitAt { checked: true }),
			("Option", "ok_or") if held(&args[0].node) && self.ok_or_error(&args[1].node, shape, shapes) => {
				Some(Producer::OkOr)
			}
			("Try", "branch")
				if held(&args[0].node)
					&& gargs
						.types()
						.next()
						.is_some_and(|t| matches!(self.b.t.std_of(t), Some(Std::Option | Std::Result))) =>
			{
				Some(Producer::Branch)
			}
			("FromResidual", "from_residual")
				if matches!(shape, Shape::Enum(ty, [Some(_), Some(_)]) if self.b.t.std_of(*ty) == Some(Std::Result))
					&& gargs
						.types()
						.nth(1)
						.is_some_and(|t| self.b.t.std_of(t) == Some(Std::Result)) =>
			{
				Some(Producer::FromResidual)
			}
			_ => None,
		}
	}

	/// Whether `local`'s definition `rvalue` can assign its parts.
	fn def_ok(&self, local: Local, rvalue: &Rvalue<'tcx>, shapes: &FxHashMap<Local, Shape<'tcx>>) -> bool {
		let shape = &shapes[&local];
		// A value of `shape` from `op`: a local held as its parts, read whole, or else one
		// without an enum, which reading its fields takes apart.
		let source = |op: &Operand<'tcx>, shape: &Shape<'tcx>| match op {
			Operand::Copy(p) | Operand::Move(p) => {
				shapes
					.get(&p.local)
					.is_some_and(|s| walk(s, p.projection).1 == p.projection.len())
					|| !shape.has_enum()
			}
			Operand::Constant(_) => !shape.has_enum(),
			Operand::RuntimeChecks(_) => false,
		};
		match rvalue {
			// A local that is a constant is read as one, as `format_args!` reads its template,
			// and holding it as its parts saves no object.
			Rvalue::Use(Operand::Constant(_), _) => false,
			Rvalue::Use(op, _) => source(op, shape),
			Rvalue::CopyForDeref(p) => source(&Operand::Copy(*p), shape),
			Rvalue::Cast(CastKind::PointerCoercion(PointerCoercion::Unsize, _), op, _) => {
				matches!(shape, Shape::Slice) && crate::lower::bytes_ref(op.ty(&self.mir.local_decls, self.tcx))
			}
			Rvalue::Ref(_, mir::BorrowKind::Shared, p) => matches!(shape, Shape::Slice) && self.reborrow(*p).is_some(),
			Rvalue::Aggregate(kind, ops) => match (&**kind, shape) {
				(AggregateKind::Tuple, Shape::Tuple(_, fields)) => ops.iter().zip(fields).all(|(op, f)| source(op, f)),
				(AggregateKind::Adt(_, v, ..), Shape::Enum(_, variants)) => variants[v.as_usize()]
					.as_ref()
					.is_some_and(|fields| ops.iter().zip(fields).all(|(op, f)| source(op, f))),
				_ => false,
			},
			_ => false,
		}
	}

	/// Whether `ok_or`'s error `op` can be the `Err` part of `shape`: a local held as its
	/// parts, or a value with no enum in it to take apart.
	fn ok_or_error(&self, op: &Operand<'tcx>, shape: &Shape<'tcx>, shapes: &FxHashMap<Local, Shape<'tcx>>) -> bool {
		let Shape::Enum(_, [_, Some(err)]) = shape else {
			return false;
		};
		let [err] = err.as_slice() else {
			return false;
		};
		match op {
			Operand::Copy(p) | Operand::Move(p) if let Some(source) = shapes.get(&p.local) => {
				walk(source, p.projection).1 == p.projection.len() || !err.has_enum()
			}
			_ => !err.has_enum(),
		}
	}

	/// Whether `rvalue` gives parts that are not those of one object: a variant, whose
	/// flag stands for one, a subslice, or a copy of a local held as its parts, alone or
	/// in a tuple. A tuple of objects alone already folds into its reads.
	fn parts_def(&self, rvalue: &Rvalue<'tcx>, shapes: &FxHashMap<Local, Shape<'tcx>>) -> bool {
		let held = |p: &Place<'tcx>| {
			shapes
				.get(&p.local)
				.is_some_and(|s| walk(s, p.projection).1 == p.projection.len())
		};
		match rvalue {
			Rvalue::Use(Operand::Copy(p) | Operand::Move(p), _) | Rvalue::CopyForDeref(p) => held(p),
			Rvalue::Ref(_, _, p) => self
				.reborrow(*p)
				.is_some_and(|(base, range)| range.is_some() || held(&base)),
			Rvalue::Aggregate(kind, ops) => {
				matches!(**kind, AggregateKind::Adt(..))
					|| ops
						.iter()
						.any(|op| matches!(op, Operand::Copy(p) | Operand::Move(p) if held(p)))
			}
			_ => false,
		}
	}

	/// What a shared borrow of bytes through a reference to them borrows: the reference,
	/// and the range of them, as `&*s` or `&s[a..b]` of an array or slice pattern make it.
	fn reborrow(&self, p: Place<'tcx>) -> Option<(Place<'tcx>, Option<PlaceElem<'tcx>>)> {
		let (base, range) = match p.projection.as_slice() {
			[.., ProjectionElem::Deref] => (p.projection.len() - 1, None),
			[.., ProjectionElem::Deref, elem @ ProjectionElem::Subslice { .. }] => {
				(p.projection.len() - 2, Some(*elem))
			}
			_ => return None,
		};
		let base = Place {
			local: p.local,
			projection: self.tcx.mk_place_elems(&p.projection[..base]),
		};
		let ty = base.ty(&self.mir.local_decls, self.tcx).ty;
		match ty.kind() {
			ty::Ref(_, inner, _) if super::place::byte_slice(*inner) || crate::lower::byte_array(*inner) => {
				Some((base, range))
			}
			_ => None,
		}
	}

	/// Find the calls of functions that hand back their `Ok`'s parts in a slot.
	pub(super) fn find_slotted_calls(&mut self) {
		for (bb, data) in self.mir.basic_blocks.iter_enumerated() {
			if let TerminatorKind::Call { func, .. } = &data.terminator().kind
				&& let ty::FnDef(def_id, args) = *func.ty(&self.mir.local_decls, self.tcx).kind()
				&& let Ok(Some(instance)) =
					ty::Instance::try_resolve(self.tcx, self.b.t.typing_env_pub(), def_id, args.skip_binder())
				&& let Some(slotted) = self.b.t.slotted(instance)
			{
				self.slotted_calls.insert(bb, (instance, slotted));
			}
		}
	}

	/// Which arguments a call of `func` takes as a byte slice's parts, as
	/// [`Translator::parted`](crate::lower::Translator::parted) says.
	pub(super) fn parted_call(&self, func: &Operand<'tcx>) -> Vec<bool> {
		let ty::FnDef(def_id, args) = *func.ty(&self.mir.local_decls, self.tcx).kind() else {
			return Vec::new();
		};
		match ty::Instance::try_resolve(self.tcx, self.b.t.typing_env_pub(), def_id, args.skip_binder()) {
			Ok(Some(instance)) => self.b.t.parted(instance),
			_ => Vec::new(),
		}
	}

	/// The parameters a byte slice `local`, named `name`, is passed as: its parts' variables
	/// if it is held as its parts, or else names for them, from which it is made on entry.
	pub(super) fn part_params(&mut self, local: Local, name: String) -> Vec<crate::ts::Param> {
		let names: [String; 3] = match self.scalars.get(&local) {
			Some(scalar) => <[String; 3]>::try_from(scalar.vars.clone()).expect("a slice's three parts"),
			None => {
				let names = ["Data", "Start", "Len"].map(|key| self.b.fresh(&format!("{name}{key}")));
				self.rebuilt.push((local, name, names.clone()));
				names
			}
		};
		self.part_params.extend(names.iter().cloned());
		let tys = [
			Type::Builtin("Uint8Array"),
			Type::Builtin("number"),
			Type::Builtin("number"),
		];
		names
			.into_iter()
			.zip(tys)
			.map(|(name, ty)| crate::ts::Param { name, ty: Some(ty) })
			.collect()
	}

	/// Mark the parameters that are parts `body` never reads, as `noUnusedParameters`
	/// wants: a slice of which only the length is read, say.
	pub(super) fn mark_unread_parts(&mut self, params: &mut [crate::ts::Param], body: &[Stmt]) {
		for param in params {
			if self.part_params.contains(&param.name)
				&& !param.name.starts_with('_')
				&& super::list_names(body, &param.name) == 0
			{
				param.name = self.b.fresh(&format!("_{}", param.name));
			}
		}
	}

	/// Find the locals to hold as their parts.
	pub(super) fn find_scalars(&mut self) {
		let mir = self.mir;
		let mut shapes: FxHashMap<Local, Shape<'tcx>> = FxHashMap::default();
		for local in mir.local_decls.indices() {
			let parted = self.parted.contains(&local);
			if self.is_arg(local) && !parted
				|| local == mir::RETURN_PLACE && self.slot.is_none()
				|| self.defs[local] == 0 && !parted
				|| self.borrowed[local]
				|| self.stepped.contains(&local)
				|| self.next_results.contains(&local)
				|| self.checked.contains(&local)
				|| self.joined.contains(&local)
			{
				continue;
			}
			let ty = mir.local_decls[local].ty;
			if self.b.t.drops(ty) {
				continue;
			}
			if let Some(shape) = self.shape(ty)
				&& shape.has_slice()
			{
				shapes.insert(local, shape);
			}
		}
		while !shapes.is_empty() {
			let bad = self.unheld(&shapes);
			if bad.is_empty() {
				break;
			}
			shapes.retain(|l, _| !bad.contains(l));
		}
		let mut shapes: Vec<(Local, Shape<'tcx>)> = shapes.into_iter().collect();
		shapes.sort_by_key(|(l, _)| *l);
		for (local, shape) in shapes {
			self.scalars.insert(
				local,
				Scalar {
					shape,
					vars: Vec::new(),
					held: None,
					assigned: false,
				},
			);
		}
	}

	/// The locals of `shapes` that can't be held as their parts, given the others are: one
	/// assigned other than by what makes its parts, or written into, or whose object would
	/// be made more often than now.
	fn unheld(&self, shapes: &FxHashMap<Local, Shape<'tcx>>) -> FxHashSet<Local> {
		let mir = self.mir;
		let tcx = self.tcx;
		let mut bad = FxHashSet::default();
		// Where each is assigned, and, per part, the parts it copies there.
		let mut def_blocks: FxHashMap<Local, FxHashSet<BasicBlock>> = FxHashMap::default();
		let mut copies: Vec<PartCopy> = Vec::new();
		// Statements and terminators that take their reads of these apart.
		let mut taken: FxHashSet<Location> = FxHashSet::default();
		// And the slices calls take as their parts.
		let mut passed: FxHashSet<(Location, Place<'tcx>)> = FxHashSet::default();
		// Those some def gives parts that are not one object's already.
		let mut parted: FxHashSet<Local> = FxHashSet::default();
		for (bb, data) in mir.basic_blocks.iter_enumerated() {
			for (i, stmt) in data.statements.iter().enumerate() {
				if self.noops.contains(&(bb, i)) {
					continue;
				}
				let at = Location {
					block: bb,
					statement_index: i,
				};
				let StatementKind::Assign(assign) = &stmt.kind else {
					continue;
				};
				let (place, rvalue) = &**assign;
				match place.as_local().filter(|l| shapes.contains_key(l)) {
					Some(local) => {
						def_blocks.entry(local).or_default().insert(bb);
						taken.insert(at);
						if self.parts_def(rvalue, shapes) {
							parted.insert(local);
						}
						match self.def_ok(local, rvalue, shapes) {
							true => self.def_copies(local, rvalue, shapes, &mut copies),
							false => {
								bad.insert(local);
							}
						}
					}
					None => {
						if let Rvalue::Discriminant(_) = rvalue {
							taken.insert(at);
						}
						// A length.
						if let Rvalue::UnaryOp(mir::UnOp::PtrMetadata, Operand::Copy(p) | Operand::Move(p)) = rvalue
							&& let Some(shape) = shapes.get(&p.local)
							&& let (node, n) = walk(shape, p.projection)
							&& n == p.projection.len()
							&& matches!(node.shape, Shape::Slice)
						{
							taken.insert(at);
						}
					}
				}
			}
			let at = Location {
				block: bb,
				statement_index: data.statements.len(),
			};
			if let TerminatorKind::Call {
				func,
				args,
				destination,
				..
			} = &data.terminator().kind
			{
				if let Some(local) = destination.as_local().filter(|l| shapes.contains_key(l)) {
					def_blocks.entry(local).or_default().insert(bb);
					taken.insert(at);
					match self.producer(bb, func, args, &shapes[&local], shapes) {
						Some(producer) => {
							parted.insert(local);
							let size = shapes[&local].size();
							for part in 0..size {
								copies.push((local, part, self.produced_copy(producer, part, &args[0].node, shapes)));
							}
						}
						None => {
							bad.insert(local);
						}
					}
				}
				// A slice passed as its parts.
				for (arg, parted) in args.iter().zip(self.parted_call(func)) {
					if let (Operand::Copy(p) | Operand::Move(p), true) = (&arg.node, parted)
						&& let Some(shape) = shapes.get(&p.local)
						&& let (node, n) = walk(shape, p.projection)
						&& n == p.projection.len()
						&& matches!(node.shape, Shape::Slice)
					{
						passed.insert((at, *p));
					}
				}
				// A slice's length, read by std.
				if let ty::FnDef(def_id, _) = *func.ty(&mir.local_decls, tcx).kind()
					&& library::is_std(tcx, def_id)
					&& library::key(tcx, def_id)
						.is_some_and(|(o, n)| o == "slice" && matches!(n.as_str(), "len" | "is_empty"))
				{
					taken.insert(at);
				}
			}
		}
		// Held as the parts of the one object it is, a local saves nothing, but for the
		// result, whose parts go to the slot; a parameter passed as its parts is made on
		// entry.
		bad.extend(
			shapes
				.keys()
				.filter(|&&l| !parted.contains(&l) && !self.parted.contains(&l) && l != mir::RETURN_PLACE),
		);
		for &local in shapes.keys().filter(|l| self.parted.contains(l)) {
			copies.extend((0..3).map(|part| (local, part, None)));
		}
		// What reads them otherwise, and which reads make their objects.
		struct Reads<'s, 'tcx> {
			shapes: &'s FxHashMap<Local, Shape<'tcx>>,
			taken: &'s FxHashSet<Location>,
			passed: &'s FxHashSet<(Location, Place<'tcx>)>,
			made: Vec<Made>,
			bad: FxHashSet<Local>,
		}
		impl<'tcx> Visitor<'tcx> for Reads<'_, 'tcx> {
			fn visit_place(&mut self, place: &Place<'tcx>, cx: PlaceContext, at: Location) {
				let Some(shape) = self.shapes.get(&place.local) else {
					return;
				};
				match cx {
					PlaceContext::NonUse(_)
					| PlaceContext::NonMutatingUse(
						NonMutatingUseContext::Inspect | NonMutatingUseContext::PlaceMention,
					)
					| PlaceContext::MutatingUse(MutatingUseContext::Drop) => return,
					// Assigned whole, as checked above, or written into.
					PlaceContext::MutatingUse(_) => {
						if !place.projection.is_empty() {
							self.bad.insert(place.local);
						}
						return;
					}
					PlaceContext::NonMutatingUse(_) => {}
				}
				if self.taken.contains(&at) || self.passed.contains(&(at, *place)) {
					return;
				}
				let (node, n) = walk(shape, place.projection);
				let rest = &place.projection[n..];
				let part = match (node.shape, rest) {
					(Shape::Unit | Shape::Leaf(_), _) => true,
					// A byte, read where it is.
					(
						Shape::Slice,
						[
							ProjectionElem::Deref,
							ProjectionElem::Index(_) | ProjectionElem::ConstantIndex { .. },
							..,
						],
					) => true,
					_ => false,
				};
				if !part {
					self.made.push(Made {
						block: at.block,
						local: place.local,
						parts: node.parts(),
					});
				}
			}
		}
		let mut reads = Reads {
			shapes,
			taken: &taken,
			passed: &passed,
			made: Vec::new(),
			bad: FxHashSet::default(),
		};
		reads.visit_body(mir);
		bad.extend(reads.bad);
		// Each part's origins: the parts whose assignment made what it may hold, through
		// copies, each made again only where its local is assigned.
		let mut origins: FxHashMap<(Local, usize), FxHashSet<(Local, usize)>> = FxHashMap::default();
		for &(local, part, from) in &copies {
			if from.is_none() {
				origins.entry((local, part)).or_default().insert((local, part));
			}
		}
		let mut changed = true;
		while std::mem::take(&mut changed) {
			for &(local, part, from) in &copies {
				let Some(from) = from else {
					continue;
				};
				let more: Vec<(Local, usize)> = origins.get(&from).into_iter().flatten().copied().collect();
				let set = origins.entry((local, part)).or_default();
				for o in more {
					changed |= set.insert(o);
				}
			}
		}
		let none = FxHashSet::default();
		let objects = |made: &Made| -> FxHashSet<(Local, usize)> {
			let mut parts = Vec::new();
			shapes[&made.local].objects(0, &mut parts);
			parts
				.into_iter()
				.filter(|p| made.parts.contains(p))
				.flat_map(|p| origins.get(&(made.local, p)).unwrap_or(&none).iter().copied())
				.collect()
		};
		let made: Vec<(&Made, FxHashSet<(Local, usize)>)> = reads.made.iter().map(|m| (m, objects(m))).collect();
		let barrier = |locals: &mut dyn Iterator<Item = Local>| -> FxHashSet<BasicBlock> {
			locals
				.flat_map(|l| def_blocks.get(&l).into_iter().flatten().copied())
				.collect()
		};
		for (i, (m, objects)) in made.iter().enumerate() {
			// In a loop that doesn't make what it makes an object of again.
			for &(origin, _) in objects {
				let defs = barrier(&mut std::iter::once(origin));
				if !defs.contains(&m.block) && self.flows(m.block, m.block, &defs) {
					bad.insert(m.local);
				}
			}
			// Twice on a path, of the same parts.
			for (n, others) in &made[i + 1..] {
				let common: Vec<Local> = objects.intersection(others).map(|&(l, _)| l).collect();
				if common.is_empty() {
					continue;
				}
				let defs = barrier(&mut common.into_iter());
				if m.block == n.block || self.flows(m.block, n.block, &defs) || self.flows(n.block, m.block, &defs) {
					bad.insert(m.local);
					bad.insert(n.local);
				}
			}
		}
		bad
	}

	/// For each part of `local` that its definition `rvalue` copies from another local held
	/// as its parts, which one; `None` for each it makes.
	fn def_copies(
		&self,
		local: Local,
		rvalue: &Rvalue<'tcx>,
		shapes: &FxHashMap<Local, Shape<'tcx>>,
		out: &mut Vec<PartCopy>,
	) {
		let shape = &shapes[&local];
		let from = |op: &Operand<'tcx>| -> Option<(Local, usize)> {
			let (Operand::Copy(p) | Operand::Move(p)) = op else {
				return None;
			};
			let (node, n) = walk(shapes.get(&p.local)?, p.projection);
			(n == p.projection.len()).then_some((p.local, node.at))
		};
		let mut parts: Vec<Option<(Local, usize)>> = vec![None; shape.size()];
		let copy = |op: &Operand<'tcx>, at: usize, size: usize, parts: &mut Vec<Option<(Local, usize)>>| {
			if let Some((l, start)) = from(op) {
				for i in 0..size {
					parts[at + i] = Some((l, start + i));
				}
			}
		};
		match (rvalue, shape) {
			(Rvalue::Use(op, _), _) => copy(op, 0, shape.size(), &mut parts),
			(Rvalue::CopyForDeref(p), _) => copy(&Operand::Copy(*p), 0, shape.size(), &mut parts),
			(Rvalue::Cast(_, op, _), _) => copy(op, 0, 3, &mut parts),
			// A reborrow is the same slice; a range of it is another.
			(Rvalue::Ref(_, _, p), _) => {
				if let Some((base, None)) = self.reborrow(*p) {
					copy(&Operand::Copy(base), 0, 3, &mut parts);
				}
			}
			(Rvalue::Aggregate(_, ops), Shape::Tuple(_, fields)) => {
				let mut at = 0;
				for (op, field) in ops.iter().zip(fields) {
					copy(op, at, field.size(), &mut parts);
					at += field.size();
				}
			}
			(Rvalue::Aggregate(kind, ops), Shape::Enum(_, variants)) => {
				let AggregateKind::Adt(_, v, ..) = **kind else {
					unreachable!("checked by def_ok")
				};
				let mut at = shape.variant_at(v.as_usize());
				for (op, field) in ops.iter().zip(variants[v.as_usize()].iter().flatten()) {
					copy(op, at, field.size(), &mut parts);
					at += field.size();
				}
			}
			_ => {}
		}
		out.extend(parts.into_iter().enumerate().map(|(i, p)| (local, i, p)));
	}

	/// The part of its argument a producer's `part` copies, if any: `ok_or`'s value, and
	/// all `?` branches on but an `Option`'s flag.
	fn produced_copy(
		&self,
		producer: Producer,
		part: usize,
		arg: &Operand<'tcx>,
		shapes: &FxHashMap<Local, Shape<'tcx>>,
	) -> Option<(Local, usize)> {
		let (Operand::Copy(p) | Operand::Move(p)) = arg else {
			return None;
		};
		let (node, _) = walk(shapes.get(&p.local)?, p.projection);
		match producer {
			Producer::SplitFirst
			| Producer::SplitFirstChunk(_)
			| Producer::SplitAt { .. }
			| Producer::FromResidual
			| Producer::Slotted => None,
			// The `Option`'s flag, then its value, then the error.
			Producer::OkOr => (part >= 1 && part < node.shape.size()).then_some((p.local, node.at + part)),
			Producer::Branch => {
				let option = matches!(node.shape, Shape::Enum(ty, _) if self.b.t.std_of(*ty) == Some(Std::Option));
				(part >= usize::from(option)).then_some((p.local, node.at + part))
			}
		}
	}

	/// Whether `to` can run after `from` without passing a block of `barrier`.
	fn flows(&self, from: BasicBlock, to: BasicBlock, barrier: &FxHashSet<BasicBlock>) -> bool {
		let mut seen = FxHashSet::default();
		let mut stack = self.successors(self.mir.basic_blocks[from].terminator());
		while let Some(bb) = stack.pop() {
			if bb == to {
				return true;
			}
			if !seen.insert(bb) || barrier.contains(&bb) || self.mir.basic_blocks[bb].is_cleanup {
				continue;
			}
			stack.extend(self.successors(self.mir.basic_blocks[bb].terminator()));
		}
		false
	}

	/// Whether `local`, which a switch reads alone, holds the discriminant of a local held
	/// as its parts, which its flag is.
	pub(super) fn scalar_discr(&self, local: Local) -> bool {
		matches!(self.def_rvalue(local), Some(Rvalue::Discriminant(p)) if self.scalar_enum(*p))
	}

	/// Whether `place` is an enum held as its parts, whose flag is its discriminant, rather
	/// than one a part holds whole, such as an error.
	pub(super) fn scalar_enum(&self, place: Place<'tcx>) -> bool {
		self.scalars.get(&place.local).is_some_and(|scalar| {
			let (node, n) = walk(&scalar.shape, place.projection);
			n == place.projection.len() && matches!(node.shape, Shape::Enum(..))
		})
	}

	/// Whether `local`'s one assignment reads a part of a local held as its parts that
	/// makes an object, so it is no alias: each read would make it again.
	pub(super) fn reads_scalar_object(&self, local: Local) -> bool {
		let place = match self.def_rvalue(local) {
			Some(
				Rvalue::Use(Operand::Copy(p) | Operand::Move(p), _) | Rvalue::CopyForDeref(p) | Rvalue::Ref(_, _, p),
			) => *p,
			Some(Rvalue::Cast(_, Operand::Copy(p) | Operand::Move(p), _)) => *p,
			_ => return false,
		};
		let Some(scalar) = self.scalars.get(&place.local) else {
			return false;
		};
		let (node, n) = walk(&scalar.shape, place.projection);
		let rest = &place.projection[n..];
		!matches!(
			(node.shape, rest),
			(Shape::Unit | Shape::Leaf(_), _)
				| (
					Shape::Slice,
					[
						ProjectionElem::Deref,
						ProjectionElem::Index(_) | ProjectionElem::ConstantIndex { .. },
						..
					]
				)
		)
	}

	/// Whether `local` is assigned once, and not a parameter, which its caller assigns first.
	fn once(&self, local: Local) -> bool {
		self.defs[local] == 1 && !self.is_arg(local)
	}

	/// Name the variables of `local`'s parts, from its own name `base`.
	pub(super) fn name_scalar(&mut self, local: Local, base: &str) {
		let Some(scalar) = self.scalars.get(&local) else {
			return;
		};
		let mut parts = Vec::new();
		scalar.shape.parts("", &mut parts);
		let vars: Vec<String> = parts
			.iter()
			.map(|(suffix, _)| self.b.fresh(&format!("{base}{suffix}")))
			.collect();
		// A parameter's parts are the parameters, which hold the same until assigned.
		if self.parted.contains(&local) && self.defs[local] == 0 {
			self.steady.extend(vars.iter().cloned());
			self.scalars.get_mut(&local).expect("checked above").held = Some(vars.iter().map(Expr::ident).collect());
		}
		self.scalars.get_mut(&local).expect("checked above").vars = vars;
	}

	/// The declarations of the variables of the parts of locals held so that statements
	/// assign, and of the slices bound to take apart.
	pub(super) fn scalar_decls(&mut self, out: &mut Vec<Stmt>) {
		let mut locals: Vec<Local> = self
			.scalars
			.iter()
			.filter(|(l, s)| s.assigned && !self.is_arg(**l))
			.map(|(l, _)| *l)
			.collect();
		locals.sort();
		for local in locals {
			let span = self.local_span(local);
			let mut parts = Vec::new();
			self.scalars[&local].shape.parts("", &mut parts);
			let vars = self.scalars[&local].vars.clone();
			for ((_, part), name) in parts.into_iter().zip(vars) {
				let ty = self.b.t.part_ty(part, span);
				out.push(StmtKind::Declare { name, ty }.at(None));
			}
		}
		for (name, ty) in std::mem::take(&mut self.scalar_binds) {
			out.push(StmtKind::Declare { name, ty }.at(None));
		}
	}

	// Lowering

	/// Each part of `local` in `parts`, as last assigned, and whether reading it later reads
	/// the same.
	fn scalar_parts(&mut self, local: Local, parts: std::ops::Range<usize>, span: Span) -> Vec<(Expr, bool)> {
		let scalar = &self.scalars[&local];
		match &scalar.held {
			Some(held) => held[parts].iter().map(|e| (e.clone(), true)).collect(),
			None if self.once(local) => {
				let n = parts.len();
				self.err(span, "a local held as its parts read before its assignment");
				vec![(Expr::undefined(), true); n]
			}
			None => scalar.vars[parts].iter().map(|v| (Expr::ident(v), false)).collect(),
		}
	}

	/// The value of `place`, a place in a local held as its parts, as far as its parts go:
	/// a part's own, or the object they make; and how many of its projections that took.
	pub(super) fn scalar_place(&mut self, place: Place<'tcx>, span: Span) -> (Expr, usize) {
		let shape = self.scalars[&place.local].shape.clone();
		let (node, n) = walk(&shape, place.projection);
		let parts: Vec<Expr> = self
			.scalar_parts(place.local, node.parts(), span)
			.into_iter()
			.map(|(e, _)| e)
			.collect();
		(self.build(node.shape, node.variant, &parts, span), n)
	}

	/// The flag of the enum at `place`, in a local held as its parts: whether it is its
	/// second variant.
	pub(super) fn scalar_flag(&mut self, place: Place<'tcx>, span: Span) -> Expr {
		let shape = self.scalars[&place.local].shape.clone();
		let (node, _) = walk(&shape, place.projection);
		match node.shape {
			Shape::Enum(_, [None, Some(_)]) => Expr::lit("true"),
			Shape::Enum(_, [_, None]) => Expr::lit("false"),
			_ => self.scalar_parts(place.local, node.at..node.at + 1, span).remove(0).0,
		}
	}

	/// The object of `shape` its `parts` make, of `variant` if it is an enum known to be one.
	fn build(&mut self, shape: &Shape<'tcx>, variant: Option<usize>, parts: &[Expr], span: Span) -> Expr {
		match shape {
			Shape::Unit => Expr::undefined(),
			Shape::Leaf(_) => parts[0].clone(),
			// The parts of a slice bound whole are that slice.
			Shape::Slice => match members(parts) {
				Some(slice) => Expr::ident(slice),
				None => Expr::New(Box::new(Expr::Rt("ByteSlice".into())), parts.to_vec()),
			},
			Shape::Tuple(ty, fields) => {
				let mut at = 0;
				let mut items = Vec::with_capacity(fields.len());
				for field in fields {
					items.push(self.build(field, None, &parts[at..at + field.size()], span));
					at += field.size();
				}
				let ts_ty = self.b.t.ty(*ty, span);
				Expr::As(Box::new(Expr::Array(items)), ts_ty)
			}
			Shape::Enum(ty, variants) => {
				let ty::Adt(adt, args) = ty.kind() else {
					unreachable!("an enum's shape is of an enum")
				};
				let known = variant.or(match variants {
					[None, Some(_)] => Some(1),
					[_, None] => Some(0),
					_ => match &parts[0] {
						Expr::Lit(t) if t == "true" => Some(1),
						Expr::Lit(t) if t == "false" => Some(0),
						_ => None,
					},
				});
				let make = |fx: &mut Self, v: usize| {
					let mut at = shape.variant_at(v);
					let mut values = Vec::new();
					for field in variants[v].iter().flatten() {
						values.push(fx.build(field, None, &parts[at..at + field.size()], span));
						at += field.size();
					}
					fx.adt(adt.did(), VariantIdx::from_usize(v), args, values, *ty, span)
				};
				match known {
					Some(v) => make(self, v),
					None => {
						let second = make(self, 1);
						let first = make(self, 0);
						let ts_ty = self.b.t.ty(*ty, span);
						Expr::As(
							Box::new(Expr::Cond(
								Box::new(parts[0].clone()),
								Box::new(second),
								Box::new(first),
							)),
							ts_ty,
						)
					}
				}
			}
		}
	}

	/// Assign `local`, held as its parts, `rvalue`.
	pub(super) fn scalar_assign(&mut self, local: Local, rvalue: &Rvalue<'tcx>, span: Span, out: &mut Vec<Stmt>) {
		let shape = self.scalars[&local].shape.clone();
		let parts: Vec<Option<(Expr, bool)>> = match rvalue {
			Rvalue::Use(op, _) => self
				.scalar_operand(op, &shape, span, out)
				.into_iter()
				.map(Some)
				.collect(),
			Rvalue::CopyForDeref(p) => self
				.scalar_source(*p, &shape, span, out)
				.into_iter()
				.map(Some)
				.collect(),
			Rvalue::Cast(_, op, _) => self.slice_parts(op, span, out).into_iter().map(Some).collect(),
			Rvalue::Ref(_, _, p) => self.borrowed_parts(*p, span, out).into_iter().map(Some).collect(),
			Rvalue::Aggregate(kind, ops) => self.scalar_aggregate(&shape, kind, ops, span, out),
			_ => unreachable!("checked by find_scalars"),
		};
		self.set_or_return(local, parts, span, out);
	}

	/// Set `local`'s parts to `parts`, or return them, if this assigns the result right
	/// before a return.
	fn set_or_return(&mut self, local: Local, parts: Vec<Option<(Expr, bool)>>, span: Span, out: &mut Vec<Stmt>) {
		match local == mir::RETURN_PLACE && self.returning.contains(&self.at) {
			true => self.slot_return_parts(parts.into_iter().map(|p| p.map(|(e, _)| e)).collect(), span, out),
			false => self.set_scalar(local, parts, out),
		}
	}

	/// Assign `local`, held as its parts, the result of the std call `func`.
	pub(super) fn scalar_call(
		&mut self,
		local: Local,
		func: &Operand<'tcx>,
		args: &[Spanned<Operand<'tcx>>],
		span: Span,
		out: &mut Vec<Stmt>,
	) {
		let shapes: FxHashMap<Local, Shape<'tcx>> = self.scalars.iter().map(|(l, s)| (*l, s.shape.clone())).collect();
		let producer = self
			.producer(self.at.block, func, args, &shapes[&local], &shapes)
			.expect("checked by find_scalars");
		let shape = shapes[&local].clone();
		let pure = |e: Expr, steady: bool| Some((e, steady));
		let parts: Vec<Option<(Expr, bool)>> = match producer {
			Producer::SplitFirst | Producer::SplitFirstChunk(_) | Producer::SplitAt { .. } => {
				let [(d, ds), (s, ss), (n, ns)] = self.slice_parts(&args[0].node, span, out);
				let (first, rest) = match producer {
					Producer::SplitFirst => (None, Expr::lit("1")),
					Producer::SplitFirstChunk(k) => (Some(Expr::lit(k.to_string())), Expr::lit(k.to_string())),
					Producer::SplitAt { .. } => {
						let mid = self.operand(&args[1].node, span, out);
						let (mid, _) = self.steady_value(mid, "mid", Type::Builtin("number"), out);
						(Some(mid.clone()), mid)
					}
					_ => unreachable!(),
				};
				let steady = ds && ss && ns;
				let tail = vec![
					pure(d.clone(), ds),
					pure(add(s.clone(), rest.clone()), ss),
					pure(sub(n.clone(), rest.clone()), ns),
				];
				match (producer, first) {
					(Producer::SplitFirst, _) => {
						let mut parts = vec![
							pure(n.clone().binary("!==", Expr::lit("0")), ns),
							pure(d.clone().index(s.clone()), ds && ss),
						];
						parts.extend(tail);
						parts
					}
					(Producer::SplitFirstChunk(_), Some(k)) => {
						let mut parts = vec![
							pure(n.clone().binary(">=", k.clone()), ns),
							pure(d.clone(), ds),
							pure(s.clone(), ss),
							pure(k, true),
						];
						parts.extend(tail);
						parts
					}
					(Producer::SplitAt { checked }, Some(mid)) => {
						let mut parts = Vec::new();
						match checked {
							true => parts.push(pure(mid.clone().binary("<=", n.clone()), ns)),
							false => {
								let loc = self.b.t.loc(span);
								let check = StmtKind::If {
									cond: mid.clone().binary(">", n.clone()),
									then: vec![
										StmtKind::Expr(Expr::rt("panic", vec![Expr::Str("mid > len".into())])).at(loc),
									],
									els: Vec::new(),
								}
								.at(None);
								self.emit(vec![check], false, out);
							}
						}
						parts.extend([pure(d.clone(), ds), pure(s.clone(), ss), pure(mid, steady)]);
						parts.extend(tail);
						parts
					}
					_ => unreachable!(),
				}
			}
			// The `Option`'s flag, whether it is `Some`, is whether the `Result` is `Ok`; then
			// its value, then the error.
			Producer::OkOr => {
				let Shape::Enum(_, [_, Some(err)]) = &shape else {
					unreachable!("ok_or makes a Result")
				};
				let mut parts = self.held_arg(&args[0].node, span, out).into_iter();
				let err = self.scalar_operand(&args[1].node, &err[0], span, out);
				let (some, steady) = parts.next().expect("an `Option`'s flag");
				let mut parts: Vec<Option<(Expr, bool)>> =
					std::iter::once((some.not(), steady)).chain(parts).map(Some).collect();
				parts.extend(err.into_iter().map(Some));
				parts
			}
			Producer::Slotted => self.slotted_call(local, args, span, out),
			// The `Err` of the residual's error, converted.
			Producer::FromResidual => {
				let residual = args[0].node.ty(&self.mir.local_decls, self.tcx);
				let (ty::Adt(_, from), Shape::Enum(ty, _)) = (residual.kind(), &shape) else {
					unreachable!("checked by producer")
				};
				let ty::Adt(_, to) = ty.kind() else {
					unreachable!("a `Result` is an ADT")
				};
				let (from, to) = (from.type_at(1), to.type_at(1));
				let error = match &args[0].node {
					Operand::Copy(p) | Operand::Move(p) if self.scalar_enum(*p) => {
						self.held_arg(&args[0].node, span, out).remove(0).0
					}
					op => match self.operand(op, span, out) {
						// Held back, and read here, where folding it would read it.
						Expr::Ident(name)
							if let Some(i) = self.pending.iter().position(|p| p.name == name)
								&& let p = &self.pending[i]
								&& !p.effect
								&& !p.copy
								&& let Expr::Call(f, items) = &p.value
								&& matches!(&**f, Expr::Rt(n) if n == "Err")
								&& items.len() == 1 =>
						{
							let Expr::Call(_, mut items) = self.pending.remove(i).value else {
								unreachable!("checked above")
							};
							items.pop().expect("checked above")
						}
						// Made in place: its error.
						Expr::Call(f, mut items) if matches!(&*f, Expr::Rt(n) if n == "Err") && items.len() == 1 => {
							items.pop().expect("checked above")
						}
						value => {
							let value = self.path(value, "residual", out);
							// The residual is an `Err`.
							let err = Type::Object(vec![
								("ok".into(), Type::Builtin("false")),
								("error".into(), self.b.t.ty(from, span)),
							]);
							Expr::As(Box::new(value), err).member("error")
						}
					},
				};
				let mut pre = Vec::new();
				let error = self.b.convert(error, from, to, span, &mut pre);
				self.emit(pre, false, out);
				let mut parts = vec![None; shape.size()];
				parts[0] = Some((Expr::lit("true"), true));
				*parts.last_mut().expect("an error") = Some(self.leaf(error));
				parts
			}
			// The parts of what it was applied to, whose `Some` continues, where the flag is
			// whether it breaks.
			Producer::Branch => {
				let option = matches!(&args[0].node, Operand::Copy(p) | Operand::Move(p)
					if self.b.t.std_of(p.ty(&self.mir.local_decls, self.tcx).ty) == Some(Std::Option));
				let mut parts: Vec<Option<(Expr, bool)>> =
					self.held_arg(&args[0].node, span, out).into_iter().map(Some).collect();
				if option && let Some(Some((flag, _))) = parts.first_mut() {
					*flag = std::mem::replace(flag, Expr::undefined()).not();
				}
				parts
			}
		};
		self.set_or_return(local, parts, span, out);
	}

	/// The parts of what a function that hands back its `Ok`'s parts returns into `local`:
	/// whether it is an error, then what it returned and the slot's fields, read now before
	/// anything else can write them, then the error.
	fn slotted_call(
		&mut self,
		local: Local,
		args: &[Spanned<Operand<'tcx>>],
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Vec<Option<(Expr, bool)>> {
		let slotted = self.slotted_calls[&self.at.block].1.clone();
		let call = self.slotted_raw(local, args, span, out);
		let mut kinds = Vec::new();
		slotted.ok.parts("", &mut kinds);
		let first = self.b.fresh("part");
		let first_ty = self.b.t.part_ty(kinds[0].1, span);
		self.steady.insert(first.clone());
		self.scalar_binds.push((
			first.clone(),
			Type::Union(vec![first_ty.clone(), Type::Builtin("undefined")]),
		));
		let stmt = StmtKind::Expr(Expr::ident(&first).assign(call)).at(None);
		self.emit(vec![stmt], false, out);
		let failed = Expr::ident(&first).binary("===", Expr::undefined());
		let mut parts = vec![
			Some((failed.clone(), true)),
			Some((Expr::As(Box::new(Expr::ident(&first)), first_ty), true)),
		];
		for (i, (_, part)) in kinds.into_iter().enumerate().skip(1) {
			let ty = self.b.t.part_ty(part, span);
			parts.push(Some(self.steady_value(slotted.field(i), "part", ty, out)));
		}
		let err_ty = self.b.t.ty(slotted.err, span);
		let err = Expr::Cond(
			Box::new(failed),
			Box::new(Expr::As(Box::new(slotted.err_field()), err_ty.clone())),
			Box::new(Expr::undefined()),
		);
		let (err, _) = self.steady_value(err, "err", Type::Union(vec![err_ty.clone(), Type::Builtin("undefined")]), out);
		parts.push(Some((Expr::As(Box::new(err), err_ty), true)));
		parts
	}

	/// The call, ending the block being lowered, of a function that hands back its `Ok`'s
	/// parts, into `local`: what it returns, its `Ok`'s first part or `undefined`.
	fn slotted_raw(&mut self, local: Local, args: &[Spanned<Operand<'tcx>>], span: Span, out: &mut Vec<Stmt>) -> Expr {
		let instance = self.slotted_calls[&self.at.block].0;
		let result = self.mir.local_decls[local].ty;
		let values = self.operands(true, &[], args, result, span, out);
		let values = values.into_iter().map(Expr::typed).collect();
		let mut pre = Vec::new();
		let call = self.b.direct_call(instance, values, span, &mut pre);
		self.emit(pre, false, out);
		match self.b.t.loc(span) {
			Some(loc) => Expr::Located(loc, Box::new(call)),
			None => call,
		}
	}

	/// Whether the call ending the block being lowered returns what it returns as is, as a
	/// function's result right before a return, through the same slot.
	pub(super) fn forwards(&self, dest: Place<'tcx>) -> bool {
		dest.as_local() == Some(mir::RETURN_PLACE)
			&& self.returning.contains(&self.at)
			&& (self.slotted_calls.get(&self.at.block).zip(self.slot.as_ref()))
				.is_some_and(|((_, callee), slot)| callee.sym == slot.sym)
	}

	/// Return what the call ending the block being lowered returns, its `Ok`'s parts left
	/// in the slot it shares.
	pub(super) fn forward_call(&mut self, args: &[Spanned<Operand<'tcx>>], span: Span, out: &mut Vec<Stmt>) {
		let call = self.slotted_raw(mir::RETURN_PLACE, args, span, out);
		let loc = self.b.t.loc(span);
		self.emit(vec![StmtKind::Return(Some(call)).at(loc)], false, out);
	}

	/// The assignments of the result, a statement's or a call's, after which nothing runs
	/// before a return: each returns there, and its block ends. Those that only make the
	/// result's parts can if it is held as them, and a call that forwards the slot always.
	pub(super) fn find_returning(&mut self) {
		if self.slot.is_none() {
			return;
		}
		let held = self.scalars.contains_key(&mir::RETURN_PLACE);
		let mir = self.mir;
		for (bb, data) in mir.basic_blocks.iter_enumerated() {
			if data.is_cleanup {
				continue;
			}
			let n = data.statements.len();
			let def = match &data.terminator().kind {
				TerminatorKind::Call {
					destination,
					target: Some(target),
					..
				} if destination.as_local() == Some(mir::RETURN_PLACE) => {
					let forwards = (self.slotted_calls.get(&bb).zip(self.slot.as_ref()))
						.is_some_and(|((_, callee), slot)| callee.sym == slot.sym);
					((held || forwards) && self.reaches_return(*target)).then_some(n)
				}
				TerminatorKind::Goto { .. } | TerminatorKind::Return if held => (0..n)
					.rev()
					.find(|&i| !self.idle(bb, i))
					.filter(
						|&i| matches!(&data.statements[i].kind, StatementKind::Assign(a) if a.0.as_local() == Some(mir::RETURN_PLACE)),
					)
					.filter(|_| match data.terminator().kind {
						TerminatorKind::Goto { target, .. } => self.reaches_return(target),
						_ => true,
					}),
				_ => None,
			};
			if let Some(i) = def {
				self.returning.insert(Location {
					block: bb,
					statement_index: i,
				});
			}
		}
	}

	/// Whether statement `i` of `bb` does nothing.
	fn idle(&self, bb: BasicBlock, i: usize) -> bool {
		matches!(
			self.mir.basic_blocks[bb].statements[i].kind,
			StatementKind::StorageDead(_) | StatementKind::StorageLive(_) | StatementKind::Nop
		) || self.noops.contains(&(bb, i))
	}

	/// Whether `bb` returns, through blocks that do nothing but go on.
	fn reaches_return(&self, mut bb: BasicBlock) -> bool {
		let mut seen = FxHashSet::default();
		while seen.insert(bb) {
			let data = &self.mir.basic_blocks[bb];
			if data.is_cleanup || !(0..data.statements.len()).all(|i| self.idle(bb, i)) {
				return false;
			}
			match data.terminator().kind {
				TerminatorKind::Return => return true,
				TerminatorKind::Goto { target, .. } => bb = target,
				_ => return false,
			}
		}
		false
	}

	/// Return from a function that hands back its `Ok`'s parts, at a `Return`.
	pub(super) fn slot_return(&mut self, slot: &Slotted<'tcx>, span: Span, out: &mut Vec<Stmt>) {
		let n = slot.fields.len();
		if self.scalars.contains_key(&mir::RETURN_PLACE) {
			let parts = (self.scalar_parts(mir::RETURN_PLACE, 0..n + 2, span).into_iter())
				.map(|(e, _)| Some(e))
				.collect();
			return self.slot_return_parts(parts, span, out);
		}
		// A result made whole, as another call returns it, taken apart here.
		let loc = self.b.t.loc(span);
		let mut pre = Vec::new();
		let value = self.place_expr(Place::return_place(), span, &mut pre);
		self.emit(pre, false, out);
		let ty = self.b.t.ty(slot.ret, span);
		let (result, _) = self.steady_value(value, "result", ty, out);
		let check = StmtKind::If {
			cond: result.clone().member("ok").not(),
			then: slot_fail(slot, result.clone().member("error"), loc),
			els: Vec::new(),
		};
		self.emit(vec![check.at(None)], false, out);
		let parts = self.take_apart(result.member("value"), &slot.ok, span, out);
		self.slot_write(slot, parts.into_iter().map(|(e, _)| Some(e)).collect(), span, out);
	}

	/// Return the result's `parts`: `undefined` once its error is in the slot, if the flag
	/// says it is one, or else the `Ok`'s first part once the rest are.
	fn slot_return_parts(&mut self, parts: Vec<Option<Expr>>, span: Span, out: &mut Vec<Stmt>) {
		let slot = self.slot.clone().expect("a function with a slot");
		let loc = self.b.t.loc(span);
		let n = slot.fields.len();
		let flag = parts[0].clone().expect("a result's flag");
		let err = parts[n + 1].clone().unwrap_or_else(Expr::undefined);
		match &flag {
			Expr::Lit(t) if t == "false" => {}
			Expr::Lit(t) if t == "true" => {
				self.emit(slot_fail(&slot, err, loc), false, out);
				return;
			}
			_ => {
				let check = StmtKind::If {
					cond: flag,
					then: slot_fail(&slot, err, loc),
					els: Vec::new(),
				};
				self.emit(vec![check.at(None)], false, out);
			}
		}
		self.slot_write(&slot, parts[1..=n].to_vec(), span, out);
	}

	/// Write the `Ok`'s parts `ok` but the first into the slot, and return the first.
	fn slot_write(&mut self, slot: &Slotted<'tcx>, ok: Vec<Option<Expr>>, span: Span, out: &mut Vec<Stmt>) {
		let loc = self.b.t.loc(span);
		let mut ok = ok.into_iter();
		let first = ok.next().flatten().unwrap_or_else(Expr::undefined);
		let mut stmts: Vec<Stmt> = ok
			.enumerate()
			.map(|(i, part)| StmtKind::Expr(slot.field(i + 1).assign(part.unwrap_or_else(Expr::undefined))).at(None))
			.collect();
		stmts.push(StmtKind::Return(Some(first)).at(loc));
		self.emit(stmts, false, out);
	}

	/// The parts of `op`, a local held as its parts read whole, as `ok_or` and `?` take it.
	fn held_arg(&mut self, op: &Operand<'tcx>, span: Span, out: &mut Vec<Stmt>) -> Vec<(Expr, bool)> {
		let (Operand::Copy(p) | Operand::Move(p)) = op else {
			unreachable!("checked by find_scalars")
		};
		self.scalar_source(*p, &Shape::Unit, span, out)
	}

	/// The parts of `op`, a value of `shape`.
	fn scalar_operand(
		&mut self,
		op: &Operand<'tcx>,
		shape: &Shape<'tcx>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Vec<(Expr, bool)> {
		match op {
			Operand::Copy(p) | Operand::Move(p) => self.scalar_source(*p, shape, span, out),
			Operand::Constant(c) => {
				let value = self.constant(c, span);
				self.take_apart(value, shape, span, out)
			}
			Operand::RuntimeChecks(_) => unreachable!("checked by find_scalars"),
		}
	}

	/// The parts of the value at `place`, of `shape`: a local's held as its parts, or those
	/// its fields hold.
	fn scalar_source(
		&mut self,
		place: Place<'tcx>,
		shape: &Shape<'tcx>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Vec<(Expr, bool)> {
		if let Some(scalar) = self.scalars.get(&place.local) {
			let source = scalar.shape.clone();
			let (node, n) = walk(&source, place.projection);
			if n == place.projection.len() {
				return self.scalar_parts(place.local, node.parts(), span);
			}
		}
		let value = self.place_expr(place, span, out);
		self.take_apart(value, shape, span, out)
	}

	/// The parts of `value`, of a `shape` without an enum, read from its fields, each of
	/// which reads the same later if it is a slice's: a slice is no object anything
	/// changes.
	fn take_apart(&mut self, value: Expr, shape: &Shape<'tcx>, span: Span, out: &mut Vec<Stmt>) -> Vec<(Expr, bool)> {
		match shape {
			Shape::Unit => Vec::new(),
			Shape::Leaf(_) => vec![self.leaf(value)],
			Shape::Slice => {
				let (slice, _) = self.steady_value(value, "slice", Type::Rt("ByteSlice", Vec::new()), out);
				["data", "start", "length"]
					.into_iter()
					.map(|key| (super::place::slice_member(slice.clone(), key), true))
					.collect()
			}
			Shape::Tuple(ty, fields) => {
				let ts_ty = self.b.t.ty(*ty, span);
				let (tuple, _) = self.steady_value(value, "tuple", ts_ty, out);
				let mut parts = Vec::new();
				for (i, field) in fields.iter().enumerate() {
					let item = tuple.clone().index(Expr::lit(i.to_string()));
					// A tuple's element can change, so it is read now.
					let item = self.take_apart(item, field, span, out);
					parts.extend(
						item.into_iter()
							.map(|(e, steady)| (e, steady && !matches!(field, Shape::Leaf(_)))),
					);
				}
				parts
			}
			Shape::Enum(..) => unreachable!("checked by find_scalars"),
		}
	}

	/// The parts a slice operand is taken apart into: a slice held as its parts, or one
	/// read from a value that reads the same later.
	fn slice_parts(&mut self, op: &Operand<'tcx>, span: Span, out: &mut Vec<Stmt>) -> [(Expr, bool); 3] {
		let parts = self.scalar_operand(op, &Shape::Slice, span, out);
		<[(Expr, bool); 3]>::try_from(parts).expect("a slice's three parts")
	}

	/// The parts of the slice a shared borrow of bytes makes: the slice it reborrows, or a
	/// range of it, which its place's projection keeps in bounds.
	fn borrowed_parts(&mut self, place: Place<'tcx>, span: Span, out: &mut Vec<Stmt>) -> [(Expr, bool); 3] {
		let (base, range) = self.reborrow(place).expect("checked by find_scalars");
		let ty = base.ty(&self.mir.local_decls, self.tcx).ty;
		let [d, (s, ss), (n, ns)] = self.slice_parts(&Operand::Copy(base), span, out);
		let Some(ProjectionElem::Subslice { from, to, from_end }) = range else {
			return [d, (s, ss), (n, ns)];
		};
		let start = add(s, Expr::lit(from.to_string()));
		let len = match (from_end, ty.peel_refs().kind()) {
			(false, _) => Expr::lit((to - from).to_string()),
			(true, ty::Array(_, len)) if let Some(len) = len.try_to_target_usize(self.tcx) => {
				Expr::lit((len - to - from).to_string())
			}
			(true, _) => sub(n, Expr::lit((from + to).to_string())),
		};
		[d, (start, ss), (len, ns)]
	}

	/// The parts of an aggregate of `shape`: a tuple's operands, or a variant's, with the
	/// enum's flag.
	fn scalar_aggregate(
		&mut self,
		shape: &Shape<'tcx>,
		kind: &AggregateKind<'tcx>,
		ops: &rustc_index::IndexVec<rustc_abi::FieldIdx, Operand<'tcx>>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Vec<Option<(Expr, bool)>> {
		match (shape, kind) {
			(Shape::Tuple(_, fields), _) => {
				let mut parts = Vec::new();
				for (op, field) in ops.iter().zip(fields) {
					parts.extend(self.scalar_operand(op, field, span, out).into_iter().map(Some));
				}
				parts
			}
			(Shape::Enum(_, variants), AggregateKind::Adt(_, v, ..)) => {
				let v = v.as_usize();
				let mut parts = vec![None; shape.size()];
				if shape.flagged() {
					parts[0] = Some((Expr::lit((v == 1).to_string()), true));
				}
				let mut at = shape.variant_at(v);
				for (op, field) in ops.iter().zip(variants[v].iter().flatten()) {
					for part in self.scalar_operand(op, field, span, out) {
						parts[at] = Some(part);
						at += 1;
					}
				}
				parts
			}
			_ => unreachable!("checked by find_scalars"),
		}
	}

	/// A value held whole as a part, and whether reading it later reads the same: one
	/// held back that does and reads nothing is taken from where it is held, to build
	/// only where it is read, as an error `ok_or` makes in place.
	fn leaf(&mut self, value: Expr) -> (Expr, bool) {
		if let Expr::Ident(name) = &value
			&& let Some(i) = self.pending.iter().position(|p| p.name == *name)
		{
			let p = &self.pending[i];
			if !p.effect && !p.memory && !p.copy && self.steady_expr(&p.value) {
				return (self.pending.remove(i).value, true);
			}
			return (value, false);
		}
		let steady = self.steady_expr(&value);
		(value, steady)
	}

	/// Whether `e` reads the same wherever it is read later, and does nothing: a constant,
	/// a name assigned once and not held back, or what is made of those without reading
	/// memory, as a cast, a comparison, or an object literal.
	fn steady_expr(&self, e: &Expr) -> bool {
		match e {
			Expr::Lit(_) | Expr::Str(_) | Expr::Rt(_) => true,
			Expr::Ident(name) => {
				!self.pending.iter().any(|p| p.name == *name) && (self.steady.contains(name) || self.stable_name(e))
			}
			Expr::As(e, _) | Expr::Unary("!", e) => self.steady_expr(e),
			Expr::Binary(l, _, r) => self.steady_expr(l) && self.steady_expr(r),
			Expr::Object(props) => props.iter().all(|(_, v)| self.steady_expr(v)),
			Expr::Array(items) => items.iter().all(|v| self.steady_expr(v)),
			_ => false,
		}
	}

	/// `value`, held where reading it later reads the same: in place if it does, or else
	/// in a variable named after `base` of type `ty` assigned it now.
	fn steady_value(&mut self, value: Expr, base: &str, ty: Type, out: &mut Vec<Stmt>) -> (Expr, bool) {
		let made =
			matches!(&value, Expr::New(f, args) if matches!(&**f, Expr::Rt(n) if n == "ByteSlice") && args.len() == 3);
		if made || self.steady_expr(&value) {
			return (value, true);
		}
		let name = self.b.fresh(base);
		self.steady.insert(name.clone());
		self.scalar_binds.push((name.clone(), ty));
		let stmt = StmtKind::Expr(Expr::ident(&name).assign(value)).at(None);
		self.emit(vec![stmt], false, out);
		(Expr::ident(name), true)
	}

	/// Set `local`'s parts to `parts`, leaving those of a variant it isn't as they are.
	fn set_scalar(&mut self, local: Local, parts: Vec<Option<(Expr, bool)>>, out: &mut Vec<Stmt>) {
		let vars = self.scalars[&local].vars.clone();
		if self.once(local) {
			let mut held = Vec::with_capacity(parts.len());
			for (part, var) in parts.into_iter().zip(&vars) {
				held.push(match part {
					Some((value, true)) => value,
					Some((value, false)) => {
						self.assign_part(local, var, value, out);
						self.steady.insert(var.clone());
						Expr::ident(var)
					}
					None => Expr::undefined(),
				});
			}
			self.scalars.get_mut(&local).expect("a local held as its parts").held = Some(held);
			return;
		}
		// Each value is read before any variable is assigned, as they all are at once.
		let own: FxHashSet<&str> = vars.iter().map(String::as_str).collect();
		let mut values: Vec<Option<Expr>> = Vec::with_capacity(parts.len());
		for (part, var) in parts.into_iter().zip(&vars) {
			let value = part.map(|(value, _)| value);
			let value = match value {
				Some(Expr::Ident(name)) if name == *var => None,
				Some(value) => {
					let mut names = Vec::new();
					super::reads_of(&value, &mut names);
					match names.iter().any(|n| own.contains(n.as_str())) {
						true => {
							let name = self.b.fresh("part");
							let stmt = StmtKind::decl(false, name.clone(), None, Some(value)).at(None);
							self.emit(vec![stmt], false, out);
							Some(Expr::ident(name))
						}
						false => Some(value),
					}
				}
				None => None,
			};
			values.push(value);
		}
		for (value, var) in values.into_iter().zip(&vars) {
			if let Some(value) = value {
				self.assign_part(local, var, value, out);
			}
		}
	}

	fn assign_part(&mut self, local: Local, var: &str, value: Expr, out: &mut Vec<Stmt>) {
		self.scalars
			.get_mut(&local)
			.expect("a local held as its parts")
			.assigned = true;
		let stmt = StmtKind::Expr(Expr::ident(var).assign(value)).at(None);
		self.emit(vec![stmt], false, out);
	}
}

/// `e + k`, with literal offsets summed.
pub(super) fn add(e: Expr, k: Expr) -> Expr {
	let lit = |e: &Expr| match e {
		Expr::Lit(t) => t.parse::<u64>().ok(),
		_ => None,
	};
	match (e, lit(&k)) {
		(e, Some(0)) => e,
		(Expr::Binary(base, "+", a), Some(b)) if let Some(a) = lit(&a) => {
			Expr::Binary(base, "+", Box::new(Expr::lit((a + b).to_string())))
		}
		(e, _) => e.binary("+", k),
	}
}

/// `e - k`, a literal `k` folded into one `e` already subtracts.
pub(super) fn sub(e: Expr, k: Expr) -> Expr {
	let lit = |e: &Expr| match e {
		Expr::Lit(t) => t.parse::<u64>().ok(),
		_ => None,
	};
	match (e, lit(&k)) {
		(e, Some(0)) => e,
		(Expr::Binary(base, "-", a), Some(b)) if let Some(a) = lit(&a) => {
			Expr::Binary(base, "-", Box::new(Expr::lit((a + b).to_string())))
		}
		(e, _) => e.binary("-", k),
	}
}

/// The name of the slice whose `data`, `start`, and `length` `parts` read, in order.
fn members(parts: &[Expr]) -> Option<&str> {
	let mut name = None;
	for (part, key) in parts.iter().zip(["data", "start", "length"]) {
		let Expr::Member(base, Key::Name(k)) = part else {
			return None;
		};
		let Expr::Ident(base) = &**base else {
			return None;
		};
		if k != key || name.is_some_and(|n| n != base) {
			return None;
		}
		name = Some(base.as_str());
	}
	name
}
