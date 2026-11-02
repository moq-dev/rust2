//! Go types: Rust's integers are Go's of the same width, a reference is a pointer, or a
//! slice for an unsized pointee, an ADT is a struct, generic where Rust's is, and an enum
//! is a struct of its tag and one field per variant with fields.

use rustc_hir::def_id::{DefId, LOCAL_CRATE};
use rustc_middle::ty::util::IntTypeExt;
use rustc_middle::ty::{self, AdtDef, GenericArgKind, Ty, TyCtxt, TypeVisitableExt};
use rustc_span::Span;

use super::library::{self, Mapped};
use super::{Translator, TypeKey, names};

impl<'tcx> Translator<'tcx> {
	/// The Go type of a monomorphic Rust type.
	pub fn ty(&mut self, ty: Ty<'tcx>, span: Span) -> String {
		self.go_ty(ty, span, false)
	}

	/// Whether values of `ty` have no Go result: `()` and `!`, which a function returning
	/// them declares no result for.
	pub fn no_value(ty: Ty<'tcx>) -> bool {
		ty.is_unit() || ty.is_never()
	}

	/// The Go type of `ty`, in which a type parameter is the Go type parameter of the
	/// declaration being written if `generic`.
	fn go_ty(&mut self, ty: Ty<'tcx>, span: Span, generic: bool) -> String {
		let tcx = self.tcx;
		match *ty.kind() {
			ty::Bool => "bool".into(),
			ty::Char => "rune".into(),
			ty::Int(i) => match i {
				ty::IntTy::Isize => "int".into(),
				ty::IntTy::I128 => self.refuse(span, "`i128`, which Go has no integer for"),
				_ => i.name_str().replace('i', "int"),
			},
			ty::Uint(u) => match u {
				ty::UintTy::Usize => "uint".into(),
				ty::UintTy::U128 => self.refuse(span, "`u128`, which Go has no integer for"),
				_ => u.name_str().replace('u', "uint"),
			},
			ty::Float(ty::FloatTy::F32) => "float32".into(),
			ty::Float(ty::FloatTy::F64) => "float64".into(),
			// What a reference to one holds: its bytes.
			ty::Str => "[]uint8".into(),
			ty::Slice(elem) => format!("[]{}", self.go_ty(elem, span, generic)),
			ty::Array(elem, len) => match len.try_to_target_usize(tcx) {
				Some(len) => format!("[{len}]{}", self.go_ty(elem, span, generic)),
				None => self.refuse(span, format!("`{ty}`, an array of a length generic in its type")),
			},
			ty::Ref(_, inner, _) | ty::RawPtr(inner, _) => match inner.kind() {
				ty::Str | ty::Slice(_) | ty::Dynamic(..) => self.go_ty(inner, span, generic),
				_ => format!("*{}", self.go_ty(inner, span, generic)),
			},
			ty::Tuple(tys) if tys.is_empty() => "struct{}".into(),
			ty::Tuple(tys) if tys.len() <= 8 => {
				let args: Vec<String> = tys.iter().map(|t| self.go_ty(t, span, generic)).collect();
				format!("tuple{}[{}]", tys.len(), args.join(", "))
			}
			ty::Never => "struct{}".into(),
			ty::FnDef(..) => "struct{}".into(),
			ty::FnPtr(..) => {
				let sig = tcx.instantiate_bound_regions_with_erased(ty.fn_sig(tcx));
				if sig.abi() != rustc_abi::ExternAbi::Rust {
					return self.refuse(span, format!("`{ty}`, a function pointer of a foreign ABI"));
				}
				let params: Vec<String> = sig.inputs().iter().map(|t| self.go_ty(*t, span, generic)).collect();
				let ret = match Self::no_value(sig.output()) {
					true => String::new(),
					false => format!(" {}", self.go_ty(sig.output(), span, generic)),
				};
				format!("func({}){ret}", params.join(", "))
			}
			ty::Dynamic(preds, ..) => match preds.principal_def_id() {
				Some(def_id) if library::fmt_trait(tcx, def_id) => "fmtDyn".into(),
				_ => self.refuse(span, format!("`{ty}`, a trait object of a trait other than `Display` and `Debug`")),
			},
			ty::Closure(..) => self.closure_ty(ty, span),
			ty::Param(p) if generic => names::ident(p.name.as_str()),
			ty::Adt(adt, args) => {
				if let Some(mapped) = library::mapped(tcx, adt.did()) {
					return self.mapped_ty(mapped, ty, args, span, generic);
				}
				let name = self.adt_decl(adt, span);
				let args: Vec<String> = args
					.iter()
					.filter_map(|a| match a.kind() {
						GenericArgKind::Type(t) => Some(t),
						_ => None,
					})
					.map(|t| self.go_ty(t, span, generic))
					.collect();
				match args.is_empty() {
					true => name,
					false => format!("{name}[{}]", args.join(", ")),
				}
			}
			_ => self.refuse(span, format!("a value of type `{ty}`")),
		}
	}

	/// Report a type translation refuses, giving a placeholder.
	fn refuse(&mut self, span: Span, what: impl Into<String>) -> String {
		self.err(span, format!("{} is not supported", what.into()));
		"struct{}".into()
	}

	/// The Go type a library type stands for.
	fn mapped_ty(
		&mut self,
		mapped: Mapped,
		ty: Ty<'tcx>,
		args: ty::GenericArgsRef<'tcx>,
		span: Span,
		generic: bool,
	) -> String {
		let arg = |i: usize, t: &mut Self| match args.types().nth(i) {
			Some(a) => t.go_ty(a, span, generic),
			None => t.refuse(span, format!("`{ty}`")),
		};
		match mapped {
			Mapped::Vec => format!("[]{}", arg(0, self)),
			Mapped::SliceIter => format!("sliceIter[{}]", arg(0, self)),
			Mapped::Box => {
				let inner = args.type_at(0);
				match inner.kind() {
					ty::Str | ty::Slice(_) | ty::Dynamic(..) => self.go_ty(inner, span, generic),
					_ => format!("*{}", self.go_ty(inner, span, generic)),
				}
			}
			Mapped::Transparent => arg(0, self),
			Mapped::Option => format!("Option[{}]", arg(0, self)),
			Mapped::Result => {
				let ok = arg(0, self);
				format!("Result[{ok}, {}]", arg(1, self))
			}
			Mapped::Runtime(name) => name.into(),
			Mapped::Refused(why) => self.refuse(span, format!("`{ty}`, {why}")),
		}
	}

	/// The Go name of an ADT's declaration, declaring it if new: generic over its type
	/// parameters, as the Rust type is.
	fn adt_decl(&mut self, adt: AdtDef<'tcx>, span: Span) -> String {
		let tcx = self.tcx;
		let did = adt.did();
		if let Some(name) = self.types.get(&TypeKey::Adt(did)) {
			return name.clone();
		}
		let base = tcx.item_name(did).to_string();
		let base = match did.krate == LOCAL_CRATE && self.local_names.get(&base).is_some_and(|&n| n > 1) {
			// Its module's path, so two modules' `Version` stay apart.
			true => module_path(tcx, did),
			false => base,
		};
		let name = self.fresh(&base);
		self.types.insert(TypeKey::Adt(did), name.clone());
		let generics = tcx.generics_of(did);
		let mut params = Vec::new();
		for param in &generics.own_params {
			match param.kind {
				ty::GenericParamDefKind::Lifetime => {}
				ty::GenericParamDefKind::Type { .. } => {
					params.push(format!("{} any", names::ident(param.name.as_str())))
				}
				ty::GenericParamDefKind::Const { .. } => {
					self.err(tcx.def_span(did), "a type with a const parameter is not supported");
				}
			}
		}
		let params = match params.is_empty() {
			true => String::new(),
			false => format!("[{}]", params.join(", ")),
		};
		let identity = ty::GenericArgs::identity_for_item(tcx, did);
		let mut fields = String::new();
		if adt.is_union() {
			self.err(tcx.def_span(did), "a union is not supported");
		} else if adt.is_struct() {
			for (i, field) in adt.non_enum_variant().fields.iter().enumerate() {
				let fty = self.field_ty(field.did, identity, tcx.def_span(field.did));
				fields.push_str(&format!("\t{} {fty}\n", names::field(field.name.as_str(), i)));
			}
		} else {
			let tag = self.go_ty(adt.repr().discr_type().to_ty(tcx), span, false);
			fields.push_str(&format!("\ttag {tag}\n"));
			for variant in adt.variants() {
				if variant.fields.is_empty() {
					continue;
				}
				let mut inner = Vec::new();
				for (i, field) in variant.fields.iter().enumerate() {
					let fty = self.field_ty(field.did, identity, tcx.def_span(field.did));
					inner.push(format!("{} {fty}", names::field(field.name.as_str(), i)));
				}
				fields.push_str(&format!("\t{} struct{{ {} }}\n", variant_field(variant.name.as_str()), inner.join("; ")));
			}
		}
		let doc = format!("// {} is Rust's `{}`.\n", name, tcx.def_path_str(did));
		self.declare(did.krate, &name, format!("{doc}type {name}{params} struct {{\n{fields}}}\n"));
		name
	}

	/// A field's Go type in its ADT's declaration, over the ADT's type parameters.
	fn field_ty(&mut self, field: DefId, identity: ty::GenericArgsRef<'tcx>, span: Span) -> String {
		let tcx = self.tcx;
		let fty = tcx.type_of(field).instantiate(tcx, identity).skip_norm_wip();
		let fty = tcx.normalize_erasing_regions(ty::TypingEnv::post_analysis(tcx, field), ty::Unnormalized::new_wip(fty));
		if fty.has_aliases() {
			return self.refuse(span, format!("a field of type `{fty}`, a projection Go generics can't name"));
		}
		self.go_ty(fty, span, true)
	}

	/// The Go type of a closure: a struct of what it captures.
	fn closure_ty(&mut self, ty: Ty<'tcx>, span: Span) -> String {
		if let Some(name) = self.types.get(&TypeKey::Mono(ty)) {
			return name.clone();
		}
		let ty::Closure(def_id, args) = *ty.kind() else { unreachable!() };
		let name = self.fresh(&format!("{}_env", names::item_base(self.tcx, def_id)));
		self.types.insert(TypeKey::Mono(ty), name.clone());
		let mut fields = String::new();
		for (i, upvar) in args.as_closure().upvar_tys().iter().enumerate() {
			let fty = self.ty(upvar, span);
			fields.push_str(&format!("\tF{i} {fty}\n"));
		}
		self.declare(def_id.krate, &name, format!("type {name} struct {{\n{fields}}}\n"));
		name
	}
}

/// The Go field holding a variant's fields.
pub fn variant_field(name: &str) -> String {
	match name {
		"tag" => "tag_".into(),
		_ => name.to_string(),
	}
}

/// A local item's module path, with its name: `lite_Version`.
fn module_path(tcx: TyCtxt<'_>, did: DefId) -> String {
	tcx.def_path(did)
		.data
		.iter()
		.filter_map(|d| d.data.get_opt_name())
		.map(|s| s.to_string())
		.collect::<Vec<_>>()
		.join("_")
}

