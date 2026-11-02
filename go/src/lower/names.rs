//! Go names: items resolved by their paths from the crate root, and every Rust item's
//! package-level Go name.

use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::{CRATE_DEF_ID, DefId, LOCAL_CRATE};
use rustc_middle::ty::{self, GenericArgKind, Instance, InstanceKind, ShimKind, Ty, TyCtxt};

/// Go's keywords and predeclared identifiers, which a package-level name must not shadow.
pub const PREDECLARED: &[&str] = &[
	"break",
	"case",
	"chan",
	"const",
	"continue",
	"default",
	"defer",
	"else",
	"fallthrough",
	"for",
	"func",
	"go",
	"goto",
	"if",
	"import",
	"interface",
	"map",
	"package",
	"range",
	"return",
	"select",
	"struct",
	"switch",
	"type",
	"var",
	"bool",
	"byte",
	"complex64",
	"complex128",
	"error",
	"float32",
	"float64",
	"int",
	"int8",
	"int16",
	"int32",
	"int64",
	"rune",
	"string",
	"uint",
	"uint8",
	"uint16",
	"uint32",
	"uint64",
	"uintptr",
	"true",
	"false",
	"iota",
	"nil",
	"append",
	"cap",
	"clear",
	"close",
	"complex",
	"copy",
	"delete",
	"imag",
	"len",
	"make",
	"max",
	"min",
	"new",
	"panic",
	"print",
	"println",
	"real",
	"recover",
	"any",
	"comparable",
];

/// The item a path from the crate root names, through modules.
pub fn resolve(tcx: TyCtxt<'_>, path: &str) -> Option<DefId> {
	let mut item = CRATE_DEF_ID.to_def_id();
	for segment in path.split("::") {
		let local = item.as_local()?;
		item = tcx
			.module_children_local(local)
			.iter()
			.find_map(|child| match child.res {
				Res::Def(kind, def_id)
					if child.ident.as_str() == segment && !matches!(kind, DefKind::Ctor(..) | DefKind::Macro(_)) =>
				{
					Some(def_id)
				}
				_ => None,
			})?;
	}
	Some(item)
}

/// The item a path names in another crate, from its root through modules.
pub fn resolve_in(tcx: TyCtxt<'_>, krate: rustc_hir::def_id::CrateNum, path: &[&str]) -> Option<DefId> {
	let mut item = krate.as_def_id();
	for segment in path {
		item = tcx.module_children(item).iter().find_map(|child| match child.res {
			Res::Def(kind, def_id)
				if child.ident.as_str() == *segment && !matches!(kind, DefKind::Ctor(..) | DefKind::Macro(_)) =>
			{
				Some(def_id)
			}
			_ => None,
		})?;
	}
	Some(item)
}

/// A root's exported Go name: its path in camel case, `harness::run` as `HarnessRun`.
pub fn exported(path: &str) -> String {
	path.split("::")
		.flat_map(|segment| segment.split('_'))
		.map(|word| {
			let mut chars = word.chars();
			match chars.next() {
				Some(first) => first.to_uppercase().chain(chars).collect(),
				None => String::new(),
			}
		})
		.collect()
}

/// `name` as a Go identifier: anything but letters, digits, and `_` becomes `_`.
pub fn ident(name: &str) -> String {
	let mut out: String = name
		.chars()
		.map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
		.collect();
	while out.contains("__") {
		out = out.replace("__", "_");
	}
	let out = out.trim_matches('_').to_string();
	if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
		return format!("x{out}");
	}
	out
}

/// A field's Go name: its Rust name, or `F<i>` for a tuple's, escaped as [`escaped`].
pub fn field(name: &str, index: usize) -> String {
	if name.parse::<usize>().is_ok() || name.is_empty() {
		return format!("F{index}");
	}
	escaped(name)
}

/// A field's or variant's name kept off Go's keywords and the enum's `tag`, by a trailing
/// `_`, which a name ending in one takes too, so two names never escape to one.
pub fn escaped(name: &str) -> String {
	match name == "tag" || name.ends_with('_') || PREDECLARED.contains(&name) {
		true => format!("{name}_"),
		false => name.to_string(),
	}
}

/// The base of a function instance's Go name: its owner and name, its crate when not the
/// translated one, and its type arguments.
pub fn func_base<'tcx>(tcx: TyCtxt<'tcx>, instance: Instance<'tcx>) -> String {
	let def_id = instance.def_id();
	let mut parts = Vec::new();
	if def_id.krate != LOCAL_CRATE {
		parts.push(tcx.crate_name(def_id.krate).to_string());
	}
	parts.push(item_base(tcx, def_id));
	match instance.def {
		InstanceKind::Item(_) => {}
		InstanceKind::Shim(ShimKind::ClosureOnce { .. }) => parts.push("once".into()),
		_ => parts.push("shim".into()),
	}
	for arg in instance.args.iter() {
		if let GenericArgKind::Type(ty) = arg.kind() {
			parts.push(ty_short(tcx, ty));
		}
	}
	let mut name = parts.join("_");
	name.truncate(80);
	name
}

/// An item's name under the type or module that owns it.
pub fn item_base(tcx: TyCtxt<'_>, def_id: DefId) -> String {
	match tcx.def_kind(def_id) {
		DefKind::Closure => format!("{}_closure", item_base(tcx, tcx.parent(def_id))),
		DefKind::AssocFn | DefKind::AssocConst => {
			let parent = tcx.parent(def_id);
			let name = tcx.item_name(def_id).to_string();
			match tcx.def_kind(parent) {
				DefKind::Impl { of_trait } => {
					let self_ty = tcx.type_of(parent).instantiate_identity().skip_norm_wip();
					let mut s = ty_short(tcx, self_ty);
					if of_trait {
						let trait_id = tcx.impl_trait_id(parent);
						s.push('_');
						s.push_str(tcx.item_name(trait_id).as_str());
					}
					format!("{s}_{name}")
				}
				_ => format!("{}_{name}", tcx.item_name(parent)),
			}
		}
		DefKind::Ctor(..) => item_base(tcx, tcx.parent(def_id)),
		_ => {
			// Local free functions by their module path, so two modules' `read` stay apart.
			let path = tcx.def_path(def_id);
			path.data
				.iter()
				.filter_map(|d| d.data.get_opt_name())
				.map(|s| s.to_string())
				.collect::<Vec<_>>()
				.join("_")
		}
	}
}

/// A short name for a type, in names of functions instantiated with it.
pub fn ty_short<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> String {
	match ty.kind() {
		ty::Bool => "bool".into(),
		ty::Char => "char".into(),
		ty::Int(i) => i.name_str().into(),
		ty::Uint(u) => u.name_str().into(),
		ty::Float(f) => f.name_str().into(),
		ty::Str => "str".into(),
		ty::Never => "never".into(),
		ty::Adt(adt, args) => {
			let mut s = tcx.item_name(adt.did()).to_string();
			for arg in args.iter() {
				if let GenericArgKind::Type(t) = arg.kind() {
					s.push('_');
					s.push_str(&ty_short(tcx, t));
				}
			}
			s
		}
		ty::Ref(_, inner, _) | ty::RawPtr(inner, _) => format!("ref_{}", ty_short(tcx, *inner)),
		ty::Slice(elem) => format!("slice_{}", ty_short(tcx, *elem)),
		ty::Array(elem, _) => format!("array_{}", ty_short(tcx, *elem)),
		ty::Tuple(tys) if tys.is_empty() => "unit".into(),
		ty::Tuple(tys) => format!(
			"tuple_{}",
			tys.iter().map(|t| ty_short(tcx, t)).collect::<Vec<_>>().join("_")
		),
		ty::FnDef(def_id, _) => tcx.item_name(*def_id).to_string(),
		ty::Closure(..) => "closure".into(),
		ty::FnPtr(..) => "fn".into(),
		ty::Dynamic(..) => "dyn".into(),
		_ => "ty".into(),
	}
}
