//! A function body being lowered: its names, and the calls the library mappings make
//! through it, to the crate's own functions, constructors, and std's.

use rustc_data_structures::fx::FxHashSet;
use rustc_middle::ty::{self, GenericArgsRef, Instance, Ty, TypeVisitableExt};
use rustc_span::Span;

use super::library;
use super::{AdtRepr, Translator, escape};
use crate::ts::{Expr, Key, Param, Stmt, StmtKind};

pub struct Body<'b, 'a, 'tcx> {
	pub t: &'b mut Translator<'a, 'tcx>,
	/// Every name in the function, so none shadows another.
	names: FxHashSet<String>,
}

impl<'b, 'a, 'tcx> Body<'b, 'a, 'tcx> {
	pub fn new(t: &'b mut Translator<'a, 'tcx>) -> Self {
		Body {
			t,
			names: FxHashSet::default(),
		}
	}

	pub fn err(&mut self, span: Span, msg: impl Into<String>) -> Expr {
		self.t.err(span, msg);
		Expr::undefined()
	}

	/// A name no other binding in the function has.
	pub fn fresh(&mut self, base: &str) -> String {
		let base = escape(base);
		let mut name = base.clone();
		let mut n = 2;
		while !self.names.insert(name.clone()) {
			name = format!("{base}{n}");
			n += 1;
		}
		name
	}

	pub fn spill(&mut self, value: Expr, base: &str, out: &mut Vec<Stmt>) -> Expr {
		let name = self.fresh(base);
		out.push(StmtKind::decl(false, name.clone(), None, Some(value)).at(None));
		Expr::ident(name)
	}

	/// `From::from(value)`, converting a `from` into a `to`.
	pub fn convert(&mut self, value: Expr, from: Ty<'tcx>, to: Ty<'tcx>, span: Span, out: &mut Vec<Stmt>) -> Expr {
		if from == to {
			return value;
		}
		let tcx = self.t.tcx;
		let Some(from_trait) = tcx.get_diagnostic_item(rustc_span::sym::From) else {
			return self.err(span, "no `From` trait");
		};
		let Some(from_fn) = tcx
			.associated_items(from_trait)
			.in_definition_order()
			.find(|i| i.is_fn())
		else {
			return self.err(span, "no `From::from`");
		};
		let args = tcx.mk_args(&[to.into(), from.into()]);
		self.call_def(from_fn.def_id, args, vec![value], span, out)
	}

	/// A call of `def_id` with `gargs`, from wherever it resolves.
	pub fn call_def(
		&mut self,
		def_id: rustc_hir::def_id::DefId,
		gargs: GenericArgsRef<'tcx>,
		args: Vec<Expr>,
		span: Span,
		out: &mut Vec<Stmt>,
	) -> Expr {
		let tcx = self.t.tcx;

		// A tuple struct or variant's constructor, called or passed as a value.
		if let rustc_hir::def::DefKind::Ctor(..) = tcx.def_kind(def_id) {
			return self.ctor(def_id, gargs, args, span);
		}

		let instance = match Instance::try_resolve(tcx, self.t.typing_env_pub(), def_id, gargs) {
			Ok(Some(instance)) => instance,
			// A trait method of a type parameter: each instance resolves it.
			Ok(None) if self.t.polymorphic() && gargs.has_param() => return Expr::undefined(),
			_ => return self.err(span, format!("`{}` does not resolve", tcx.def_path_str(def_id))),
		};
		if let Some(sym) = self.t.bound_fn(instance, span) {
			return Expr::Sym(sym).call(args.into_iter().map(Expr::typed).collect());
		}
		if let ty::InstanceKind::Virtual(method, _) = instance.def
			&& self.t.dispatches(instance)
		{
			return self.virtual_call(method, gargs.type_at(0), args, span);
		}
		// An `async` body's `Future::poll`, which its future has itself.
		if let ty::InstanceKind::Item(did) = instance.def
			&& tcx.is_coroutine(did)
		{
			let mut values = args;
			let future = values.remove(0);
			return future.member("poll").call(values);
		}
		if let ty::InstanceKind::Item(did) = instance.def
			&& self.t.lowers(self.t.typing_env_pub(), instance)
		{
			let values: Vec<Expr> = args.into_iter().map(Expr::typed).collect();
			// An `async fn` that only makes its future: the future, made here.
			if let Some(class) = super::mir::coroutine::of_async_fn(self.t, instance, span) {
				return Expr::New(Box::new(Expr::Sym(class)), values);
			}
			let sym = self.t.callee(instance, span);
			let call = Expr::Sym(sym).call(values);
			// The crate's own `next` returns an item or `rt.DONE`, and its caller an `Option`.
			if super::iter::is_next(tcx, did) {
				let option = self.t.sig(did, instance.args).output();
				self.t.ty(option, span);
				let ty::Adt(_, option_args) = option.kind() else {
					unreachable!("`next` returns an `Option`")
				};
				let none = self.t.none_arg(option_args.type_at(0));
				return Expr::rt("iterToOption", [call].into_iter().chain(none).collect());
			}
			return call;
		}
		if let Some(referent) = library::referent(tcx, def_id, gargs, instance) {
			// An operand that was a reference to an `rt.Place`, as `&&mut u32` is, passes
			// what the Place points at.
			let inputs = |gargs| self.t.sig(def_id, gargs).inputs().to_vec();
			let args = std::iter::zip(args, std::iter::zip(inputs(gargs), inputs(referent)))
				.map(|(arg, (was, now))| match was.kind() {
					ty::Ref(_, inner, _) if was != now => self.t.pointee_value(*inner, arg),
					_ => arg,
				})
				.collect();
			return self.call_def(def_id, referent, args, span, out);
		}
		if let Some(inherited) = library::Inherited::of(tcx, instance) {
			let value = self.call_def(inherited.via(tcx), gargs, args, span, out);
			return inherited.test(self, value, out);
		}
		let call = library::Call {
			def_id,
			args: gargs,
			span,
		};
		match library::call(self, &call, args, out) {
			Some(e) => e,
			None => self.err(
				span,
				format!("unsupported call to `{}`", tcx.def_path_str_with_args(def_id, gargs)),
			),
		}
	}

	fn ctor(
		&mut self,
		def_id: rustc_hir::def_id::DefId,
		gargs: GenericArgsRef<'tcx>,
		args: Vec<Expr>,
		span: Span,
	) -> Expr {
		let tcx = self.t.tcx;
		let values = args;
		let parent = tcx.parent(def_id);
		let (adt_id, variant) = match tcx.def_kind(parent) {
			rustc_hir::def::DefKind::Variant => (
				tcx.parent(parent),
				tcx.adt_def(tcx.parent(parent)).variant_index_with_id(parent),
			),
			_ => (parent, rustc_abi::FIRST_VARIANT),
		};
		let adt = tcx.adt_def(adt_id);
		if let Some(std) = library::Std::of(tcx, adt_id) {
			let mut values = values;
			return match (std, variant.index()) {
				(library::Std::Option, 1) | (library::Std::Poll, 0) => values.pop().unwrap(),
				(library::Std::Cow, _) if library::string(tcx, Ty::new_adt(tcx, adt, gargs)) => values.pop().unwrap(),
				(library::Std::Cow, _) if let Some(elem) = library::cow_slice(tcx, Ty::new_adt(tcx, adt, gargs)) => {
					library::cow_made(elem, variant, values.pop().unwrap())
				}
				(library::Std::Result, i) => Expr::result(i == 0, values.pop().unwrap()),
				(library::Std::Reverse | library::Std::Wrapping | library::Std::Saturating, _) => values.pop().unwrap(),
				(library::Std::Bound | library::Std::TryLockError, _) => {
					let ts_ty = self.t.ty(Ty::new_adt(tcx, adt, gargs), span);
					library::tagged(adt.variant(variant).name.as_str(), values.pop(), ts_ty)
				}
				_ => self.err(span, format!("constructing `{}`", tcx.def_path_str(adt_id))),
			};
		}
		if !self.t.own(adt_id) {
			return self.err(
				span,
				format!("constructing `{}` {}", tcx.def_path_str(adt_id), self.t.foreign(adt_id)),
			);
		}
		let info = self.t.adt(adt, span);
		let ty = tcx.type_of(adt_id).instantiate(tcx, gargs).skip_norm_wip();
		match info.repr {
			AdtRepr::Struct { ctor, fields } => {
				let args = (adt.variant(variant).fields.iter().zip(values))
					.map(|(field, value)| field_arg(tcx, field, value))
					.collect();
				self.t.construct(info.sym, &ctor, &fields, args)
			}
			AdtRepr::Tags => {
				let ts_ty = self.t.ty(ty, span);
				Expr::As(Box::new(Expr::Str(adt.variant(variant).name.to_string())), ts_ty)
			}
			AdtRepr::Slots(_) => self.err(span, format!("constructing union `{}`", tcx.def_path_str(adt_id))),
			AdtRepr::Union => {
				let mut props = vec![(
					Key::Name("tag".into()),
					Expr::Str(adt.variant(variant).name.to_string()),
				)];
				for (i, value) in values.into_iter().enumerate() {
					props.push((Key::Name(format!("_{i}")), value));
				}
				let ts_ty = self.t.ty(ty, span);
				Expr::As(Box::new(Expr::Object(props)), ts_ty)
			}
		}
	}

	/// A value of one of std's types made of zero-sized `values`, which MIR reads as a
	/// constant rather than from what made it, or `values` back for another type.
	pub fn zero_sized_std(&mut self, ty: Ty<'tcx>, values: Vec<Expr>, span: Span) -> Result<Expr, Vec<Expr>> {
		let tcx = self.t.tcx;
		let ty::Adt(adt, _) = ty.kind() else {
			return Err(values);
		};
		if library::nothing(tcx, ty) {
			return Ok(Expr::undefined());
		}
		// Typed, as what reads it may infer nothing of it.
		if library::empty_iter(tcx, adt.did()) {
			let ts_ty = self.t.ty(ty, span);
			return Ok(Expr::As(Box::new(Expr::rt("iterEmpty", Vec::new())), ts_ty));
		}
		let Some((make, gargs)) = super::iter::maker(self.t, ty) else {
			return Err(values);
		};
		let mut pre = Vec::new();
		let made = self.call_def(make, gargs, values, span, &mut pre);
		Ok(match pre.is_empty() {
			true => made,
			false => self.err(span, format!("a constant of type `{ty}` whose making takes statements")),
		})
	}

	/// A function item as a value, typed as one: a declared function's `()` result is `void`,
	/// which a value's type says is `undefined`.
	pub fn fn_value(&mut self, def_id: rustc_hir::def_id::DefId, gargs: GenericArgsRef<'tcx>, span: Span) -> Expr {
		let tcx = self.t.tcx;
		let sig = self.t.sig(def_id, gargs);
		let names: Vec<String> = (0..sig.inputs().len()).map(|i| self.fresh(&format!("a{i}"))).collect();
		// std's mappings take a `&mut` they declare that is an `rt.Place` as what it points
		// at, as a call's operands pass it.
		let pointees = match self.t.mapped(def_id, gargs) {
			true => super::pointer::pointees(tcx, def_id),
			false => Vec::new(),
		};
		let values: Vec<Expr> = std::iter::zip(&names, sig.inputs())
			.enumerate()
			.map(
				|(i, (name, &ty))| match pointees.get(i) == Some(&true) && self.t.place_pointee(ty).is_some() {
					true => Expr::ident(name).member("v"),
					false => Expr::ident(name),
				},
			)
			.collect();
		// A binding's export, or a function whose body translates, is named directly.
		if !matches!(tcx.def_kind(def_id), rustc_hir::def::DefKind::Ctor(..))
			&& let Ok(Some(instance)) = Instance::try_resolve(tcx, self.t.typing_env_pub(), def_id, gargs)
		{
			let sym = match self.t.bound_fn(instance, span) {
				Some(sym) => Some(sym),
				None => self
					.t
					.lowers(self.t.typing_env_pub(), instance)
					.then(|| self.t.callee(instance, span)),
			};
			if let Some(sym) = sym {
				return match sig.output().is_unit() {
					true => Expr::As(Box::new(Expr::Sym(sym)), self.t.fn_ty(sig.inputs(), sig.output(), span)),
					false => Expr::Sym(sym),
				};
			}
		}
		let mut body = Vec::new();
		let call = self.call_def(def_id, gargs, values, span, &mut body);
		body.push(
			match sig.output().is_unit() {
				true => StmtKind::Expr(call),
				false => StmtKind::Return(Some(call.typed())),
			}
			.at(None),
		);
		let params = names
			.into_iter()
			.zip(sig.inputs())
			.map(|(name, ty)| {
				let ty = self.t.ty(*ty, span);
				Param { name, ty: Some(ty) }
			})
			.collect();
		let ret = self.t.ty(sig.output(), span);
		Expr::Arrow(params, ret, body)
	}
}

/// A field's value as a class's constructor takes it: without an outer `as` where the
/// field's type gives the value its type, but not where TypeScript infers a class
/// parameter from it, which would widen a literal.
fn field_arg<'tcx>(tcx: ty::TyCtxt<'tcx>, field: &ty::FieldDef, value: Expr) -> Expr {
	match tcx
		.type_of(field.did)
		.instantiate_identity()
		.skip_norm_wip()
		.has_param()
	{
		true => value,
		false => value.typed(),
	}
}
