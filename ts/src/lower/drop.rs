//! Drops: which values JS can see drop, and where each one drops.
//!
//! A type drops if dropping it does something JS can see: runs a `Drop` impl, counts an
//! `Arc` down, or drops a waker. Such a value has `[Symbol.dispose]`, which a class gets
//! as drop glue: its `Drop` impl, then each field in order. Where each value drops is
//! MIR's: drop elaboration put a terminator at each drop, with a flag wherever a value is
//! moved on some paths only.

use rustc_middle::ty::{self, AdtDef, Instance, Ty, TypeVisitableExt};
use rustc_span::Span;

use super::library::{self, Std};
use super::mir::Capture;
use super::{Adt, AdtRepr, Translator};
use crate::program::{SymKind, Symbol};
use crate::ts::{self, Expr, Param, Stmt, StmtKind, Type};

impl<'a, 'tcx> Translator<'a, 'tcx> {
	/// Whether dropping a `ty` does something JS can see.
	pub fn drops(&mut self, ty: Ty<'tcx>) -> bool {
		if let Some(&drops) = self.drops.get(&ty) {
			return drops;
		}
		// A type that holds itself drops through another field if at all.
		if !self.drops_open.insert(ty) {
			self.drops_cycle = true;
			return false;
		}
		let outer_cycle = std::mem::replace(&mut self.drops_cycle, false);
		let tcx = self.tcx;
		let drops = match ty.kind() {
			ty::Adt(adt, _) if let Some(drops) = self.bound_drops(adt.did()) => drops,
			ty::Adt(adt, args) => match Std::of(tcx, adt.did()) {
				// A guard unlocks its mutex or ends its borrow, a `Weak` uncounts itself, and a
				// `PeekMut` sifts what it lent.
				Some(Std::Arc | Std::Weak | Std::Waker | Std::MutexGuard | Std::Ref | Std::RefMut | Std::PeekMut) => {
					true
				}
				Some(
					Std::Option
					| Std::Result
					| Std::ControlFlow
					| Std::Box
					| Std::Vec
					| Std::Mutex
					| Std::RefCell
					| Std::Cell
					| Std::PoisonError
					| Std::OnceCell
					| Std::OnceLock
					| Std::LazyCell
					| Std::LazyLock
					| Std::Poll
					| Std::Pin
					| Std::PollFn
					| Std::HashMap
					| Std::HashSet
					| Std::BTreeMap
					| Std::BTreeSet
					| Std::VecDeque
					| Std::LinkedList
					| Std::BinaryHeap
					| Std::Bound
					| Std::TryLockError
					| Std::Reverse
					| Std::Wrapping
					| Std::Saturating
					| Std::Range
					| Std::RangeInclusive
					| Std::RangeFrom
					| Std::RangeTo
					| Std::RangeToInclusive,
				) => args.types().any(|t| self.drops(t)),
				// An entry borrows its map, and holds only the key it was given.
				Some(Std::Entry) => args.types().next().is_some_and(|k| self.drops(k)),
				// The items of the `Vec` it owns.
				Some(Std::Cow) => library::cow_slice(tcx, ty).is_some_and(|elem| self.drops(elem)),
				// A `Vec`'s drain moves the vector's tail back as it drops, and a `String`'s takes
				// the drained text out of it.
				None if {
					let path = library::std_path(tcx, adt.did());
					path == ["vec", "drain", "Drain"] || path == ["string", "Drain"]
				} =>
				{
					true
				}
				None | Some(Std::Peekable) if self.std_iter(ty) => match adt.has_dtor(tcx) {
					// What owns its items, an `IntoIter` or a `Drain`, drops those it still holds.
					true => args.types().any(|t| self.drops(t)),
					false => self.fields_drop(*adt, args, true),
				},
				Some(_) => false,
				None if self.crates.translates(adt.did()) => adt.has_dtor(tcx) || self.fields_drop(*adt, args, true),
				None => false,
			},
			ty::Tuple(tys) => tys.iter().any(|t| self.drops(t)),
			// A slice, which a box owns.
			ty::Array(elem, _) | ty::Slice(elem) => self.drops(*elem),
			ty::Closure(..) | ty::CoroutineClosure(..) => {
				super::captured(ty).is_some_and(|tys| tys.iter().any(|t| self.drops(t)))
			}
			// Whatever it was made of may drop.
			ty::Coroutine(..) | ty::Param(_) | ty::Dynamic(..) => true,
			_ => false,
		};
		self.drops_open.remove(&ty);
		// A `false` that read a type still being decided holds only once the first of
		// them is decided: if it drops, so does every type it reached that reaches it.
		let cycle = std::mem::replace(&mut self.drops_cycle, outer_cycle);
		if drops || !cycle || self.drops_open.is_empty() {
			self.drops.insert(ty, drops);
		} else {
			self.drops_cycle = true;
		}
		drops
	}

	/// Whether `ty` is one of std's iterators, or a type they are built of, which an `rt.Iter`
	/// stands for: it drops what it owns, of its fields or, with a `Drop` impl, its items.
	pub(super) fn std_iter(&self, ty: Ty<'tcx>) -> bool {
		let ty::Adt(adt, _) = ty.kind() else {
			return false;
		};
		matches!(
			self.tcx.crate_name(adt.did().krate).as_str(),
			"core" | "alloc" | "std" | "hashbrown"
		) && self.iter_item(ty).is_some()
	}

	/// Whether any field of `adt` with `args` drops, counting one of a type parameter only
	/// if `params`.
	pub fn fields_drop(&mut self, adt: AdtDef<'tcx>, args: ty::GenericArgsRef<'tcx>, params: bool) -> bool {
		let tcx = self.tcx;
		let env = self.typing_env_pub();
		adt.all_fields().any(|field| {
			// A projection from a type parameter normalizes only where its bounds are known.
			let field = ty::EarlyBinder::bind(tcx, tcx.type_of(field.did).instantiate_identity().skip_norm_wip());
			let ty = tcx
				.try_instantiate_and_normalize_erasing_regions(args, env, field)
				.unwrap_or_else(|_| field.instantiate(tcx, args).skip_norm_wip());
			(params || !ty.has_param()) && self.drops(ty)
		})
	}

	/// Whether whatever holds a `ty` can drop it without knowing its type, through the
	/// `[Symbol.dispose]` its JS value has, as an `Arc` and a generic class's glue do. A
	/// class whose glue that needs is given it.
	fn disposable(&mut self, ty: Ty<'tcx>, span: Span) -> bool {
		if !self.drops(ty) {
			return self.inert(ty);
		}
		match ty.kind() {
			// Its binding's `[Symbol.dispose]`.
			ty::Adt(adt, _) if self.bound_drops(adt.did()).is_some() => true,
			ty::Adt(adt, args) => match Std::of(self.tcx, adt.did()) {
				// Each drops by what it was given, or a `Weak` by its own count.
				Some(Std::Waker | Std::PollFn | Std::Weak) => true,
				Some(
					Std::Arc
					| Std::Mutex
					| Std::RefCell
					| Std::Cell
					| Std::PoisonError
					| Std::OnceCell
					| Std::OnceLock
					| Std::Option
					| Std::Box
					| Std::Pin
					| Std::Reverse
					| Std::Wrapping
					| Std::Saturating,
				) => self.disposable(args.type_at(0), span),
				// An `rt.Iter` drops what it was given to.
				None | Some(Std::Peekable) if self.std_iter(ty) => self.closures_held(ty),
				Some(_) => false,
				// A generic `Drop` impl has an instance per type, which one class's glue can't call.
				None if adt.is_struct() && !self.generic_dtor(*adt) => {
					if !args.types().all(|t| self.disposable(t, span)) {
						return false;
					}
					let info = self.adt(*adt, span);
					self.glue(*adt, &info, span);
					true
				}
				None => false,
			},
			// A closure's `[Symbol.dispose]` drops what it owns, a `dyn Future`'s what it is, and
			// another trait object's what it wraps.
			ty::Closure(..) | ty::CoroutineClosure(..) | ty::Coroutine(..) | ty::Param(_) | ty::Dynamic(..) => true,
			_ => false,
		}
	}

	/// Whether every JS value of `ty`, a type that drops and is [`Self::disposable`], is an
	/// object with `[Symbol.dispose]`, which a drop calls directly rather than through
	/// `rt.drop`'s one megamorphic site. An `Option` may be `None`, and a type parameter, a
	/// trait object, a closure, or a future may be any value.
	fn always_disposes(&self, ty: Ty<'tcx>) -> bool {
		let ty::Adt(adt, args) = ty.kind() else {
			return false;
		};
		match Std::of(self.tcx, adt.did()) {
			_ if self.bound_drops(adt.did()).is_some() => true,
			Some(Std::Box | Std::Pin | Std::Reverse | Std::Wrapping | Std::Saturating | Std::PoisonError) => {
				self.always_disposes(args.type_at(0))
			}
			Some(
				Std::Waker
				| Std::PollFn
				| Std::Weak
				| Std::Arc
				| Std::Mutex
				| Std::RefCell
				| Std::Cell
				| Std::OnceCell
				| Std::OnceLock,
			) => true,
			Some(_) => false,
			// An `rt.Iter` has one only if it was given what drops.
			None if self.std_iter(ty) => false,
			// The class its glue made.
			None => adt.is_struct(),
		}
	}

	/// Whether each closure that drops in `ty`, one of std's iterators, is one an adaptor's
	/// `rt.Iter` drops, as `iter::holds_closure` says.
	fn closures_held(&mut self, ty: Ty<'tcx>) -> bool {
		let ty::Adt(adt, args) = ty.kind() else {
			return true;
		};
		let holds = super::iter::holds_closure(self.tcx, adt.did());
		args.types().all(|t| match t.kind() {
			ty::Closure(..) | ty::CoroutineClosure(..) => holds || !self.drops(t),
			_ => self.closures_held(t),
		})
	}

	/// Whether `adt` is generic and implements `Drop`.
	fn generic_dtor(&self, adt: AdtDef<'tcx>) -> bool {
		adt.has_dtor(self.tcx) && self.tcx.generics_of(adt.did()).requires_monomorphization(self.tcx)
	}

	/// Whether dropping a `ty` through its JS value's `[Symbol.dispose]` does nothing. A
	/// reference is its referent in JS, so dropping a borrow of a value that drops would
	/// drop the value.
	fn inert(&mut self, ty: Ty<'tcx>) -> bool {
		if self.drops(ty) {
			return false;
		}
		match ty.kind() {
			ty::Ref(_, inner, _) => self.inert(*inner),
			// A binding's value that drops nothing has no `[Symbol.dispose]` to call.
			ty::Adt(adt, _) if self.bound_drops(adt.did()).is_some() => true,
			ty::Adt(adt, args) => match Std::of(self.tcx, adt.did()) {
				// Each is its contents in JS.
				Some(
					Std::Option
					| Std::Box
					| Std::Mutex
					| Std::RefCell
					| Std::Cell
					| Std::PoisonError
					| Std::OnceCell
					| Std::OnceLock
					| Std::Poll
					| Std::Pin
					| Std::Reverse
					| Std::Wrapping
					| Std::Saturating,
				) => args.types().all(|t| self.inert(t)),
				// Its waker.
				Some(Std::Context) => false,
				Some(_) => true,
				// Its glue, if an instance that drops gave it one, drops its type parameters' fields.
				None if adt.is_struct() => args.types().all(|t| self.inert(t)),
				None => true,
			},
			_ => true,
		}
	}

	/// What drops the `ty` in `value`, or `None` if JS would see nothing drop.
	pub fn drop_value(&mut self, value: Expr, ty: Ty<'tcx>, span: Span) -> Option<Expr> {
		// Lifetimes don't change how a value drops, so a type generic over them alone, as a
		// class's field may be, has the glue of any instance.
		let ty = self.tcx.erase_and_anonymize_regions(ty);
		if !self.drops(ty) {
			return None;
		}
		// An array or `Vec`, or one in an `Option` or a box, which is it or `undefined`.
		if let Some(elem) = self.elements(ty) {
			let mut args = vec![value];
			if !self.disposable(elem, span) {
				args.push(self.drop_fn(elem, span)?);
			}
			return Some(Expr::rt("dropAll", args));
		}
		if self.disposable(ty, span) {
			return Some(match self.always_disposes(ty) {
				true => value.index(Expr::ident("Symbol").member("dispose")).call(Vec::new()),
				false => Expr::rt("drop", vec![value]),
			});
		}
		if let ty::Adt(adt, args) = ty.kind()
			&& let Some(std) = Std::of(self.tcx, adt.did())
		{
			// What drops each key, value, or item, or `undefined` for one that drops nothing.
			// A type argument by its index among the types, past an entry's lifetime.
			let arg = |n: usize| args.types().nth(n).expect("a type argument");
			let each = |t: &mut Self, n: usize| match t.drops(arg(n)) {
				true => t.drop_fn(arg(n), span),
				false => Some(Expr::undefined()),
			};
			let (name, args) = match std {
				Std::Arc => ("arcDrop", vec![value, each(self, 0)?]),
				Std::Mutex => return self.drop_value(value.member("value"), args.type_at(0), span),
				Std::MutexGuard => ("mutexUnlock", vec![value]),
				Std::RefCell => return self.drop_value(value.member("value"), args.type_at(0), span),
				Std::Ref => ("refRelease", vec![value]),
				Std::RefMut => ("refMutRelease", vec![value]),
				Std::Cell => return self.drop_value(value.member("v"), args.type_at(0), span),
				Std::OnceCell | Std::OnceLock => {
					let option = Ty::new_option(self.tcx, args.type_at(0));
					return self.drop_value(value.member("v"), option, span);
				}
				Std::Box | Std::Pin | Std::Reverse | Std::Wrapping | Std::Saturating | Std::PoisonError => {
					return self.drop_value(value, args.type_at(0), span);
				}
				// Its value, or the function that never made one.
				Std::LazyCell | Std::LazyLock => ("lazyDrop", vec![value, each(self, 0)?, each(self, 1)?]),
				Std::HashMap => ("hashDrop", vec![value, each(self, 0)?, each(self, 1)?]),
				Std::BTreeMap => ("btreeDrop", vec![value, each(self, 0)?, each(self, 1)?]),
				Std::HashSet => ("hashDrop", vec![value, each(self, 0)?]),
				Std::BTreeSet => ("btreeDrop", vec![value, each(self, 0)?]),
				Std::VecDeque => ("dequeDrop", vec![value, each(self, 0)?]),
				Std::LinkedList => ("listDrop", vec![value, each(self, 0)?]),
				Std::BinaryHeap => ("heapDrop", vec![value, each(self, 0)?]),
				Std::PeekMut => ("peekMutDrop", vec![value]),
				// What it owns, and nothing it borrows.
				Std::Cow => {
					let elem = library::cow_slice(self.tcx, ty)?;
					let mut args = vec![value];
					if !self.disposable(elem, span) {
						args.push(self.drop_fn(elem, span)?);
					}
					("cowDrop", args)
				}
				// Its bounds, in the order std's fields are, `rt.RangeInclusive`'s by its own names.
				Std::Range | Std::RangeInclusive => {
					let (lo, hi) = match std {
						Std::Range => ("start", "end"),
						_ => ("lo", "hi"),
					};
					let bounds = Expr::Array(vec![value.clone().member(lo), value.member(hi)]);
					let mut args = vec![bounds];
					if !self.disposable(arg(0), span) {
						args.push(self.drop_fn(arg(0), span)?);
					}
					("dropAll", args)
				}
				Std::RangeFrom => return self.drop_value(value.member("start"), arg(0), span),
				Std::RangeTo | Std::RangeToInclusive => return self.drop_value(value.member("end"), arg(0), span),
				// A vacant entry holds the key it was given; an occupied one dropped it as
				// `entry` found the map's.
				Std::Entry => match self.tcx.item_name(adt.did()).as_str() {
					"VacantEntry" => return self.drop_value(value.member("k"), arg(0), span),
					"OccupiedEntry" => return None,
					_ => ("entryDrop", vec![value, each(self, 0)?]),
				},
				_ => ("", vec![value]),
			};
			if !name.is_empty() {
				let mut args = args;
				while args.len() > 1 && matches!(args.last(), Some(Expr::Lit(l)) if l == "undefined") {
					args.pop();
				}
				return Some(Expr::rt(name, args));
			}
			return self.drop_glue_call(args.into_iter().next()?, ty, span);
		}
		self.drop_glue_call(value, ty, span)
	}

	/// `value`, a `ty`, dropped by its glue: rustc's, for the crate's types, tuples, and
	/// std's enums.
	fn drop_glue_call(&mut self, value: Expr, ty: Ty<'tcx>, span: Span) -> Option<Expr> {
		let rustc = match ty.kind() {
			ty::Tuple(_) => true,
			ty::Adt(adt, _) => adt.is_enum() || self.crates.translates(adt.did()),
			_ => false,
		};
		if rustc && let Some(sym) = super::mir::glue::shim(self, ty, span) {
			return Some(super::mir::glue::call(sym, value));
		}
		if let ty::Tuple(_) = ty.kind()
			&& ty.has_param()
		{
			return Some(self.drop_fn(ty, span)?.call(vec![value]));
		}
		// A census lowers a generic function over its type parameters, where only an
		// instance can say what the glue is.
		if rustc && self.census && (ty.has_param() || ty.has_aliases()) {
			return Some(Expr::rt("drop", vec![value]));
		}
		self.err(span, format!("dropping `{ty}`, which is not supported yet"));
		None
	}

	/// A function dropping a `ty`, or `None` if JS would see nothing drop or translation
	/// refused it: `rt.drop` for a value with `[Symbol.dispose]`, or else its glue.
	pub fn drop_fn(&mut self, ty: Ty<'tcx>, span: Span) -> Option<Expr> {
		let ty = self.tcx.erase_and_anonymize_regions(ty);
		if !self.drops(ty) {
			return None;
		}
		if self.disposable(ty, span) {
			return Some(Expr::Rt("drop".into()));
		}
		// A tuple over type parameters, as a generic class's glue holds, has no glue of
		// rustc's until an instance: each element in order, as that glue drops them.
		if let ty::Tuple(tys) = ty.kind()
			&& ty.has_param()
		{
			let value = Expr::ident("value");
			let mut steps = Vec::new();
			for (i, elem) in tys.iter().enumerate() {
				let element = value.clone().index(Expr::lit(i.to_string()));
				if let Some(dropped) = self.drop_value(element, elem, span) {
					steps.push(StmtKind::Expr(dropped).at(None));
				}
			}
			let param = Param {
				name: "value".into(),
				ty: Some(self.ty(ty, span)),
			};
			return Some(Expr::Arrow(vec![param], Type::Builtin("void"), steps));
		}
		// Over type parameters, as a generic class's glue is, a function of its own.
		if ty.has_param() || ty.has_aliases() {
			let param = Param {
				name: "value".into(),
				ty: Some(self.ty(ty, span)),
			};
			let body = self.drop_value(Expr::ident("value"), ty, span)?;
			return Some(Expr::Lambda(vec![param], Box::new(body)));
		}
		super::mir::glue::shim(self, ty, span).map(Expr::Sym)
	}

	/// What drops a `ty`, as a runtime function takes it after its own arguments:
	/// `undefined` if JS would see nothing drop, or if translation refused it.
	pub fn dropper(&mut self, ty: Ty<'tcx>, span: Span) -> Expr {
		self.drop_fn(ty, span).unwrap_or_else(Expr::undefined)
	}

	/// What drops each of `tys`, as a runtime function takes them after its own arguments:
	/// `undefined` for one that drops nothing, up to the last that drops. `None` if
	/// translation refused one.
	pub fn droppers(&mut self, tys: impl IntoIterator<Item = Ty<'tcx>>, span: Span) -> Option<Vec<Expr>> {
		let mut droppers = Vec::new();
		let mut used = 0;
		for ty in tys {
			match self.drops(ty) {
				true => {
					droppers.push(self.drop_fn(ty, span)?);
					used = droppers.len();
				}
				false => droppers.push(Expr::undefined()),
			}
		}
		droppers.truncate(used);
		Some(droppers)
	}

	/// The elements of an array, slice, or `Vec` that `ty` is, or holds in an `Option` or a box,
	/// whose JS value is the array or `undefined`.
	fn elements(&self, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
		match ty.kind() {
			ty::Array(elem, _) | ty::Slice(elem) => Some(*elem),
			ty::Adt(adt, args) => match Std::of(self.tcx, adt.did())? {
				Std::Vec => Some(args.type_at(0)),
				Std::Option if self.erases_to(args.type_at(0)).undefined => None,
				Std::Option | Std::Box | Std::Pin | Std::Reverse | Std::Wrapping | Std::Saturating => {
					self.elements(args.type_at(0))
				}
				_ => None,
			},
			_ => None,
		}
	}

	/// Statements dropping what a closure of type `ty` owns, whose captures are `captures`:
	/// each that drops, in order, as rustc's glue drops a closure. Its `[Symbol.dispose]`, and what ends an `FnOnce` body that drops
	/// its closure whole.
	pub fn closure_glue(&mut self, ty: Ty<'tcx>, captures: &[Capture], span: Span) -> Vec<Stmt> {
		let upvars = super::captured(ty).expect("closure glue of a closure");
		let mut steps = Vec::new();
		for (capture, ty) in captures.iter().zip(upvars) {
			if let Some(dropped) = self.drop_value(capture.value.clone(), ty, span) {
				steps.push(StmtKind::Expr(dropped).at(None));
			}
		}
		steps
	}

	/// Give a class whose own fields or `Drop` impl drop its drop glue up front, so JS
	/// that holds one can drop it. One that drops only through a type parameter gets
	/// it once an instance that drops does.
	pub fn drop_glue(&mut self, adt: AdtDef<'tcx>, info: &Adt, span: Span) {
		let identity = ty::GenericArgs::identity_for_item(self.tcx, adt.did());
		if adt.has_dtor(self.tcx) || self.fields_drop(adt, identity, false) {
			self.glue(adt, info, span);
		}
	}

	/// The `[Symbol.dispose]` of a class: its `Drop` impl, then each field that drops.
	fn glue(&mut self, adt: AdtDef<'tcx>, info: &Adt, span: Span) {
		let tcx = self.tcx;
		if !self.glued.insert(adt.did()) {
			return;
		}
		// An enum drops by rustc's glue, which switches on its variant, and a generic `Drop`
		// impl has an instance per type: each instance drops by glue of its own.
		let AdtRepr::Struct { fields, .. } = &info.repr else {
			return;
		};
		if self.generic_dtor(adt) {
			return;
		}
		let mut steps = Vec::new();
		if let Some(dtor) = adt.destructor(tcx) {
			let sym = self.instance(Instance::mono(tcx, dtor.did));
			steps.push(StmtKind::Expr(Expr::Sym(sym).call(vec![Expr::This])).at(None));
		}
		// Only an instance whose parameters drop runs the glue, so none is a byte: a field
		// whose definition's type an instance decides is read as an array of them holds it.
		let glue = std::mem::replace(&mut self.glue, true);
		for (field, &id) in adt.non_enum_variant().fields.iter().zip(fields) {
			let ty = tcx.type_of(field.did).instantiate_identity().skip_norm_wip();
			let read = match self.instanced_fields.contains_key(&field.did) {
				true => Expr::As(Box::new(Translator::read_field(Expr::This, id)), self.ty(ty, span)),
				false => Translator::read_field(Expr::This, id),
			};
			if let Some(dropped) = self.drop_value(read, ty, span) {
				steps.push(StmtKind::Expr(dropped).at(None));
			}
		}
		self.glue = glue;
		let sym = self.program.sym(Symbol {
			module: self.module_of(adt.did()),
			base: "[Symbol.dispose]".into(),
			suffix: String::new(),
			owner: Some(info.sym),
			kind: SymKind::Method,
			order: 0,
			span: rustc_span::DUMMY_SP,
			instance: String::new(),
			name: String::new(),
			alias: None,
		});
		self.program.methods.entry(info.sym).or_default().push(ts::Function {
			sym,
			params: Vec::new(),
			ret: Type::Builtin("void"),
			body: steps,
			loc: None,
			doc: None,
		});
	}
}
