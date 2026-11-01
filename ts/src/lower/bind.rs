//! Bindings: hand-written TypeScript that `ts.toml` names in place of a Rust function or
//! type. A call of a bound function, or of a function of a bound type that the type's crate
//! defines, trait impls included, calls the binding file's export named for the Rust
//! function: one export serves every instance, so a generic one is a generic TS function.
//! `tsc` checks each export against every instance's translated signature once the output
//! is written, as [`crate::check`] does.

use std::path::{Component, Path, PathBuf};

use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::{CRATE_DEF_ID, DefId};
use rustc_metadata::creader::CStore;
use rustc_middle::ty::{self, Instance, InstanceKind, TyCtxt, TypeVisitableExt};
use rustc_span::Span;

use super::{Translator, camel, names};
use crate::program::{Bound, Check, ModPath, SymKind, Symbol, module_file};
use crate::ts::{Expr, Function, Item, Param, StmtKind, SymId, Type};

/// An item `ts.toml` binds.
struct Binding {
	/// Its path, as `ts.toml` writes it.
	path: String,
	/// The module of its file, which is that file's path in the output.
	module: ModPath,
	kind: Kind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
	Function,
	/// A type, and whether its value drops through its `[Symbol.dispose]`.
	Type {
		drop: bool,
	},
}

/// A binding's file.
struct File {
	/// Relative to `ts.toml`.
	path: PathBuf,
	/// What its importers call it: the type bound to it, or its file's name.
	namespace: String,
	/// Its file's name as an identifier.
	prefix: String,
}

/// What the translation knows of the config's bindings.
#[derive(Default)]
pub struct Bindings<'tcx> {
	items: rustc_data_structures::fx::FxHashMap<DefId, Binding>,
	files: rustc_data_structures::fx::FxHashMap<ModPath, File>,
	/// The symbol of each bound type, and of each export, by its module and name.
	types: rustc_data_structures::fx::FxHashMap<DefId, SymId>,
	exports: rustc_data_structures::fx::FxHashMap<(ModPath, String), (DefId, SymId)>,
	/// The instances checked against their export.
	checked: rustc_data_structures::fx::FxHashSet<Instance<'tcx>>,
	/// The instantiations of bound types whose value is checked.
	values: rustc_data_structures::fx::FxHashSet<ty::Ty<'tcx>>,
}

impl<'a, 'tcx> Translator<'a, 'tcx> {
	/// Resolve every binding the config names, reporting each it can't.
	pub(super) fn bind(&mut self) {
		let tcx = self.tcx;
		let config = self.config;
		for (path, bind) in &config.bind {
			let fail = |t: &mut Self, msg: String| {
				t.tcx.dcx().err(format!("rust2-ts: binding `{path}`: {msg}"));
				t.failed = true;
			};
			let Some(def_id) = self.resolve_bound(path) else {
				fail(
					self,
					"no such item, or more than one, in the crate or a crate it depends on".into(),
				);
				continue;
			};
			let kind = match tcx.def_kind(def_id) {
				DefKind::Fn => Kind::Function,
				DefKind::AssocFn
					if tcx.trait_impl_of_assoc(def_id).is_none() && tcx.impl_of_assoc(def_id).is_some() =>
				{
					Kind::Function
				}
				DefKind::Struct | DefKind::Enum | DefKind::Union => {
					if super::library::Std::of(tcx, def_id).is_some() {
						fail(self, "the type has a representation of its own".into());
						continue;
					}
					Kind::Type { drop: bind.drop }
				}
				kind => {
					fail(self, format!("a binding is of a function or a type, not {kind:?}"));
					continue;
				}
			};
			if bind.drop && kind == Kind::Function {
				fail(self, "only a type's value drops".into());
				continue;
			}
			let module = match module_of(&bind.file) {
				Ok(module) => module,
				Err(msg) => {
					fail(self, msg);
					continue;
				}
			};
			if let Some(other) = self.bindings.items.get(&def_id) {
				let msg = format!("`{}` binds the same item", other.path);
				fail(self, msg);
				continue;
			}
			let name = tcx.item_name(def_id).to_string();
			let file = self.bindings.files.entry(module.clone()).or_insert_with(|| {
				let prefix = ident(module.last().expect("a file name"));
				File {
					path: bind.file.clone(),
					namespace: prefix.clone(),
					prefix,
				}
			});
			if let Kind::Type { .. } = kind {
				let types = self
					.bindings
					.items
					.values()
					.filter(|b| b.module == module && b.kind != Kind::Function)
					.count();
				if types > 0 {
					fail(self, format!("{} binds another type", bind.file.display()));
					continue;
				}
				file.namespace = super::escape(&name);
			}
			self.bindings.items.insert(
				def_id,
				Binding {
					path: path.clone(),
					module,
					kind,
				},
			);
		}
		let bodies = tcx.mir_keys(()).iter().map(|def_id| def_id.to_def_id());
		// A binding's code, a bound type's methods' included, is code it doesn't read.
		self.writes = super::copy::Writes::of_bodies(self, bodies, &self.read, &|def_id| {
			self.crates.translates(def_id) && !self.bound(def_id)
		});
	}

	/// The item a binding's path names: from the crate root, or else from a crate the crate
	/// depends on, through modules and then a type's inherent functions. A dependency goes
	/// by the name the crate's source gives it, which a renamed one's isn't its own; a crate
	/// reached only through another goes by its own name, where one crate of it is built.
	fn resolve_bound(&self, path: &str) -> Option<DefId> {
		let segments: Vec<&str> = path.split("::").collect();
		if let Some(def_id) = resolve_in(self.tcx, CRATE_DEF_ID.to_def_id(), &segments) {
			return Some(def_id);
		}
		let (name, rest) = segments.split_first()?;
		let tcx = self.tcx;
		let named = CStore::from_tcx(tcx).resolved_extern_crate(rustc_span::Symbol::intern(name));
		let cnum = match named {
			Some(cnum) => cnum,
			None => {
				let mut own = tcx.crates(()).iter().copied().filter(|&cnum| {
					tcx.crate_name(cnum).as_str() == *name && !tcx.extern_crate(cnum).is_some_and(|e| e.is_direct())
				});
				match (own.next(), own.next()) {
					(Some(cnum), None) => cnum,
					_ => return None,
				}
			}
		};
		resolve_in(tcx, cnum.as_def_id(), rest)
	}

	/// Whether `def_id` is a bound type or function, or a function of a bound type in an
	/// impl of the type's own crate: translation never reads it.
	pub fn bound(&self, def_id: DefId) -> bool {
		self.binder(def_id).is_some()
	}

	/// The bound item whose binding stands in for `def_id`: itself, or the bound type an
	/// impl of the type's own crate is of. A trait's provided function the impl doesn't
	/// override, and a blanket impl's, are the trait's, as for any other type.
	fn binder(&self, def_id: DefId) -> Option<DefId> {
		let tcx = self.tcx;
		if self.bindings.items.contains_key(&def_id) {
			return Some(def_id);
		}
		let imp = tcx.impl_of_assoc(def_id)?;
		let ty::Adt(adt, _) = tcx
			.type_of(imp)
			.instantiate_identity()
			.skip_norm_wip()
			.peel_refs()
			.kind()
		else {
			return None;
		};
		(adt.did().krate == def_id.krate && self.bound_drops(adt.did()).is_some()).then_some(adt.did())
	}

	/// Each file the config binds: its module, what importers call it, and its file's name
	/// as an identifier.
	pub(super) fn bound_files(&self) -> impl Iterator<Item = (&ModPath, &str, &str)> {
		(self.bindings.files.iter()).map(|(module, file)| (module, file.namespace.as_str(), file.prefix.as_str()))
	}

	/// Whether `def_id` is of a crate that translates, rather than bound.
	pub fn own(&self, def_id: DefId) -> bool {
		self.crates.translates(def_id) && !self.bindings.items.contains_key(&def_id)
	}

	/// Why `def_id`, which isn't [`own`](Self::own), isn't the crate's to translate.
	pub fn foreign(&self, def_id: DefId) -> &'static str {
		match self.bindings.items.contains_key(&def_id) {
			true => "bound to TypeScript",
			false => "from another crate",
		}
	}

	/// Whether a bound type's value drops through its `[Symbol.dispose]`, if it is one.
	pub fn bound_drops(&self, def_id: DefId) -> Option<bool> {
		match self.bindings.items.get(&def_id)?.kind {
			Kind::Type { drop } => Some(drop),
			Kind::Function => None,
		}
	}

	/// The symbol of a bound type, from its binding's file. `span` is where `args` reach it.
	pub fn bound_type(&mut self, adt: ty::AdtDef<'tcx>, args: ty::GenericArgsRef<'tcx>, span: Span) -> Option<SymId> {
		let def_id = adt.did();
		let drop = self.bound_drops(def_id)?;
		// What it holds drops only where it does.
		if !drop && let Some(arg) = args.types().find(|&t| !t.has_param() && self.drops(t)) {
			let path = &self.bindings.items[&def_id].path;
			let msg = format!("`{path}` binds without drop, so it would never drop its `{arg}`");
			self.err(span, msg);
		}
		let module = self.bindings.items[&def_id].module.clone();
		let sym = match self.bindings.types.get(&def_id) {
			Some(&sym) => sym,
			None => {
				let name = self.tcx.item_name(def_id).to_string();
				let sym = self.bound_sym(&module, name, SymKind::Struct, span);
				self.bindings.types.insert(def_id, sym);
				sym
			}
		};
		// Each instantiation translation reaches, with its own type arguments: a generic
		// binding may be one value for one and another for the next, as `Held<T> = T` is
		// `undefined` for `Held<()>`. Over type parameters, only a census's, which checks
		// nothing, reaches one.
		let ty = ty::Ty::new_adt(self.tcx, adt, args);
		if !ty.has_param() && self.bindings.values.insert(ty) {
			let value = Type::Sym(sym, args.types().map(|t| self.ty(t, span)).collect());
			let flag = |b: bool| Type::Builtin(if b { "true" } else { "false" });
			let ret = Type::Rt("Bound", vec![value.clone(), flag(self.is_copy(ty)), flag(drop)]);
			let value = Param {
				name: "value".into(),
				ty: Some(value),
			};
			let body = vec![StmtKind::Return(Some(Expr::ident("value"))).at(None)];
			let what = ty::print::with_no_trimmed_paths!(format!(
				"the binding of `{ty}` in {}",
				module_file(&module).display()
			));
			self.check(vec![value], ret, body, self.tcx.def_span(def_id), what);
		}
		Some(sym)
	}

	/// The export a call of `instance` calls, if a binding stands in for it, checked
	/// against the instance's signature. `span` is where it is reached.
	pub fn bound_fn(&mut self, instance: Instance<'tcx>, span: Span) -> Option<SymId> {
		let tcx = self.tcx;
		let module = self.bound_module(instance)?.clone();
		let def_id = instance.def_id();
		let name = export_name(tcx, def_id);
		let sym = match self.bindings.exports.get(&(module.clone(), name.clone())) {
			Some(&(other, sym)) if other == def_id => sym,
			Some(&(other, _)) => {
				self.err(
					tcx.def_span(def_id),
					format!(
						"`{}` and `{}` would both be the export `{name}` of {}",
						tcx.def_path_str(def_id),
						tcx.def_path_str(other),
						module_file(&module).display()
					),
				);
				return None;
			}
			None => {
				if let Some(param) = self.phantom(def_id) {
					let msg = format!(
						"a binding of `{}`, whose `{param}` one export for every instance can't recover from its arguments",
						tcx.def_path_str(def_id)
					);
					self.err(tcx.def_span(def_id), msg);
				}
				let sym = self.bound_sym(&module, name.clone(), SymKind::Function, span);
				self.bindings.exports.insert((module.clone(), name), (def_id, sym));
				sym
			}
		};
		if self.bindings.checked.insert(instance) {
			if crate::lower::iter::is_next(tcx, def_id) {
				self.err(span, "iterating a bound type, which is not supported yet");
			}
			let sig = self.sig(def_id, instance.args);
			// `=> undefined` for `()`, not `fn_ty`'s `=> void`, which accepts any return value.
			let params = sig.inputs().iter().map(|t| self.ty(*t, span)).collect();
			let ty = Type::Func(params, Box::new(self.ty(sig.output(), span)));
			let body = vec![StmtKind::Return(Some(Expr::Sym(sym))).at(None)];
			let what = ty::print::with_no_trimmed_paths!(format!(
				"the binding of `{instance}` in {}",
				module_file(&module).display()
			));
			self.check(Vec::new(), ty, body, tcx.def_span(def_id), what);
		}
		Some(sym)
	}

	/// The module of the binding that stands in for `instance`, if one does.
	pub fn bound_module(&self, instance: Instance<'tcx>) -> Option<&ModPath> {
		let InstanceKind::Item(def_id) = instance.def else {
			return None;
		};
		Some(&self.bindings.items[&self.binder(def_id)?].module)
	}

	/// A parameter of `def_id` that one export serving every instance, seeing each generic
	/// argument only in the values it takes, can't recover: a const parameter, whose value
	/// even an argument whose type holds it may not carry, as `None` of an
	/// `Option<[u8; N]>` doesn't, or a type parameter with a trait bound that no argument's
	/// type holds, whose impls it can't call.
	fn phantom(&self, def_id: DefId) -> Option<rustc_span::Symbol> {
		let tcx = self.tcx;
		let mut generics = Some(tcx.generics_of(def_id));
		while let Some(g) = generics {
			if let Some(param) = g
				.own_params
				.iter()
				.find(|param| matches!(param.kind, ty::GenericParamDefKind::Const { .. }))
			{
				return Some(param.name);
			}
			generics = g.parent.map(|parent| tcx.generics_of(parent));
		}
		let sig = tcx.fn_sig(def_id).instantiate_identity().skip_norm_wip().skip_binder();
		let held: Vec<u32> = sig
			.inputs()
			.iter()
			.flat_map(|t| t.walk())
			.filter_map(|arg| match arg.as_type()?.kind() {
				ty::Param(p) => Some(p.index),
				_ => None,
			})
			.collect();
		tcx.param_env(def_id).caller_bounds().find_map(|clause| {
			let pred = clause.as_trait_clause()?.skip_binder();
			match pred.self_ty().kind() {
				ty::Param(p) if !held.contains(&p.index) && !tcx.is_sizedness_trait(pred.def_id()) => Some(p.name),
				_ => None,
			}
		})
	}

	/// A symbol of a binding's file, named as the file exports it, which reaches the file.
	fn bound_sym(&mut self, module: &ModPath, name: String, kind: SymKind, span: Span) -> SymId {
		if !self.program.bound.contains_key(module) {
			let file = &self.bindings.files[module];
			// A census writes nothing, so it reads no file.
			let text = match self.census {
				true => Ok(String::new()),
				false => std::fs::read_to_string(self.config.dir.join(&file.path)),
			};
			let (namespace, prefix) = (file.namespace.clone(), file.prefix.clone());
			let source = self.config.dir.join(&file.path);
			let text = match text {
				Ok(text) => text,
				Err(err) => {
					let msg = format!("reading {}: {err}", file.path.display());
					self.err(span, msg);
					String::new()
				}
			};
			let bound = Bound {
				source,
				text,
				namespace,
				prefix,
			};
			self.program.bound.insert(module.clone(), bound);
		}
		self.program.sym(Symbol {
			module: module.clone(),
			base: name.clone(),
			suffix: String::new(),
			owner: None,
			kind,
			order: 0,
			span: rustc_span::DUMMY_SP,
			instance: String::new(),
			name,
			alias: None,
		})
	}

	/// A function of the check module, which type-checks only if the binding it reads is
	/// what translated code takes it for.
	fn check(&mut self, params: Vec<Param>, ret: Type, body: Vec<crate::ts::Stmt>, span: Span, what: String) {
		let sym = self.program.sym(Symbol {
			module: check_module(),
			base: "check".into(),
			suffix: self.program.checks.len().to_string(),
			owner: None,
			kind: SymKind::Function,
			order: self.program.checks.len() as u32,
			span,
			instance: String::new(),
			name: String::new(),
			alias: None,
		});
		self.program.checks.push(Check {
			item: Item::Function(Function {
				sym,
				params,
				ret,
				body,
				loc: None,
				doc: None,
			}),
			span,
			what,
		});
	}
}

/// The module `tsc` checks the bindings in, which no Rust module or binding can be: the
/// runtime's directory holds only the runtime.
pub fn check_module() -> ModPath {
	vec!["rt".into(), "bind".into()]
}

/// The export standing in for the function `def_id`: its name, then the trait it
/// implements with the trait's type arguments as the impl names them, as a translated
/// function's suffix has them, but not the type arguments of the type or the function,
/// which a generic TS function takes. `Bytes::slice` is `slice`, and `Bytes`'s
/// `PartialEq<[u8]>` is `eq$cmp_PartialEq_slice_u8`.
fn export_name(tcx: TyCtxt<'_>, def_id: DefId) -> String {
	let base = camel(tcx.item_name(def_id).as_str());
	let Some(imp) = tcx.trait_impl_of_assoc(def_id) else {
		return base;
	};
	let trait_ref = tcx.impl_trait_ref(imp).instantiate_identity().skip_norm_wip();
	format!("{base}${}", names::trait_name(tcx, trait_ref))
}

/// The item `segments` name under `module`, through modules, and then a type's inherent
/// functions.
pub fn resolve_in(tcx: TyCtxt<'_>, mut item: DefId, segments: &[&str]) -> Option<DefId> {
	for (i, segment) in segments.iter().enumerate() {
		item = match tcx.def_kind(item) {
			DefKind::Mod => {
				let children = match item.as_local() {
					Some(local) => tcx.module_children_local(local),
					None => tcx.module_children(item),
				};
				let named = children.iter().filter_map(|child| match child.res {
					// A tuple struct's constructor shares its name, as a macro may.
					Res::Def(kind, def_id)
						if child.ident.as_str() == *segment
							&& !matches!(kind, DefKind::Ctor(..) | DefKind::Macro(_)) =>
					{
						Some((kind, def_id))
					}
					_ => None,
				});
				// A function may share its name with a module, as `rand::rng` does: the path
				// goes on through the module, or ends at the function.
				let last = i + 1 == segments.len();
				let (first, rest): (Vec<_>, Vec<_>) = named.partition(|&(kind, _)| (kind == DefKind::Mod) != last);
				first.into_iter().chain(rest).next()?.1
			}
			// The one function of the name, which impls for different type arguments can
			// each define.
			DefKind::Struct | DefKind::Enum | DefKind::Union => {
				let mut fns = tcx.inherent_impls(item).iter().flat_map(|&imp| {
					tcx.associated_items(imp)
						.in_definition_order()
						.filter(|assoc| assoc.is_fn() && assoc.name().as_str() == *segment)
						.map(|assoc| assoc.def_id)
				});
				match (fns.next(), fns.next()) {
					(Some(def_id), None) => def_id,
					_ => return None,
				}
			}
			_ => return None,
		};
	}
	(!segments.is_empty()).then_some(item)
}

/// The module of a binding's file: its path in the output, which must stay inside it,
/// apart from the runtime.
fn module_of(file: &Path) -> Result<ModPath, String> {
	if file.extension().is_none_or(|ext| ext != "ts") {
		return Err(format!("{} is no TypeScript file", file.display()));
	}
	let mut module = Vec::new();
	for part in file.with_extension("").components() {
		match part {
			Component::Normal(name) => module.push(name.to_string_lossy().into_owned()),
			_ => {
				return Err(format!(
					"{} must be relative to ts.toml, inside its directory",
					file.display()
				));
			}
		}
	}
	if module.first().is_some_and(|first| first.eq_ignore_ascii_case("rt")) {
		return Err(format!("{} would replace the runtime, rt.ts and rt/", file.display()));
	}
	Ok(module)
}

/// `name` as a JS identifier.
fn ident(name: &str) -> String {
	let name: String = name
		.chars()
		.map(|c| {
			if c.is_alphanumeric() || c == '_' || c == '$' {
				c
			} else {
				'_'
			}
		})
		.collect();
	match name.starts_with(|c: char| c.is_ascii_digit()) {
		true => format!("_{name}"),
		false => super::escape(&name),
	}
}
