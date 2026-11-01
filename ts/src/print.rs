//! TypeScript text and its source map, from the AST in `ts`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

use oxc_sourcemap::SourceMapBuilder;

use crate::program::{HEADER, ModPath, Program, SymKind, module_file, unique};
use crate::ts::{Ctor, Expr, FieldInit, Function, Item, Key, Loc, Param, Stmt, StmtKind, SymId, Type};

pub struct Printed {
	pub code: String,
	/// The source map, as JSON.
	pub map: String,
}

/// Print one module's items to the file at `path`.
pub fn module(program: &Program, module: &ModPath, items: &[&Item], path: &Path) -> Result<Printed, String> {
	let bindings = Bindings::new(program, module, items)?;
	let mut p = Printer {
		program,
		bindings: &bindings,
		out: String::new(),
		line: 0,
		col: 0,
		indent: 0,
		tokens: Vec::new(),
		uses_rt: false,
		renames: HashMap::new(),
		aliases: Vec::new(),
	};
	for (i, item) in items.iter().enumerate() {
		if i > 0 {
			p.newline();
		}
		p.item(item);
	}
	if !p.aliases.is_empty() {
		let aliases: Vec<String> = p
			.aliases
			.iter()
			.map(|(name, alias)| format!("{name} as {alias}"))
			.collect();
		p.newline();
		p.write(&format!("export {{ {} }};\n", aliases.join(", ")));
	}

	let file = module_file(module);
	let depth = file.components().count() - 1;
	let up = |target: &Path| -> String {
		let mut rel = String::new();
		match depth {
			0 => rel.push_str("./"),
			_ => (0..depth).for_each(|_| rel.push_str("../")),
		}
		rel.push_str(&target.to_string_lossy());
		rel
	};

	let mut header = format!("{HEADER}\n\n");
	if p.uses_rt {
		writeln!(header, "import * as rt from \"{}\";", up(Path::new("rt.ts"))).unwrap();
	}
	let mut targets: Vec<&ModPath> = bindings.imports.keys().chain(bindings.namespaces.keys()).collect();
	targets.sort();
	for target in targets {
		if let Some((binding, value)) = bindings.namespaces.get(target) {
			let kind = if *value { "" } else { "type " };
			writeln!(
				header,
				"import {kind}* as {binding} from \"{}\";",
				up(&module_file(target))
			)
			.unwrap();
			continue;
		}
		let names: Vec<String> = bindings.imports[target]
			.iter()
			.map(|(sym, binding, value)| {
				let name = &program.syms[*sym].name;
				let kind = if *value { "" } else { "type " };
				match binding == name {
					true => format!("{kind}{name}"),
					false => format!("{kind}{name} as {binding}"),
				}
			})
			.collect();
		writeln!(
			header,
			"import {{ {} }} from \"{}\";",
			names.join(", "),
			up(&module_file(target))
		)
		.unwrap();
	}
	if p.uses_rt || !bindings.imports.is_empty() || !bindings.namespaces.is_empty() {
		header.push('\n');
	}
	let offset = header.lines().count() as u32;

	let file_name = path.file_name().unwrap().to_string_lossy().into_owned();
	let map_name = format!("{file_name}.map");
	let code = format!("{header}{}\n//# sourceMappingURL={map_name}\n", p.out);

	let dir = path.parent().unwrap();
	let source_names: Vec<String> = program
		.sources
		.iter()
		.map(|source| relative(dir, &source.path).to_string_lossy().into_owned())
		.collect();
	let mut map = SourceMapBuilder::default();
	map.set_file(&file_name);
	let mut ids = HashMap::new();
	for (line, col, loc) in &p.tokens {
		let id = *ids.entry(loc.file).or_insert_with(|| {
			let source = &program.sources[loc.file as usize];
			map.add_source_and_content(&source_names[loc.file as usize], &source.text)
		});
		map.add_token(line + offset, *col, loc.line, loc.col, Some(id), None);
	}
	Ok(Printed {
		code,
		map: map.into_sourcemap().to_json_string(),
	})
}

/// Functions printed apart from any module, each item they refer to by the name given here
/// and the function itself unnamed: the same text for two functions that print the same.
pub struct Texts {
	bindings: Bindings,
}

impl Texts {
	pub fn new(names: HashMap<SymId, String>) -> Self {
		Self {
			bindings: Bindings {
				names,
				imports: BTreeMap::new(),
				namespaces: BTreeMap::new(),
			},
		}
	}

	pub fn set(&mut self, id: SymId, name: String) {
		self.bindings.names.insert(id, name);
	}

	pub fn text(&self, program: &Program, f: &Function) -> String {
		let mut p = Printer {
			program,
			bindings: &self.bindings,
			out: String::new(),
			line: 0,
			col: 0,
			indent: 0,
			tokens: Vec::new(),
			uses_rt: false,
			renames: HashMap::new(),
			aliases: Vec::new(),
		};
		p.write("(");
		p.params(&f.params);
		let ret = p.ty(&f.ret);
		p.write(&format!("): {ret} {{\n"));
		p.block_body(&f.body);
		p.out
	}
}

/// `to` relative to the directory `from`, both absolute.
fn relative(from: &Path, to: &Path) -> PathBuf {
	let from: Vec<Component> = from.components().collect();
	let to: Vec<Component> = to.components().collect();
	let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
	let mut rel = PathBuf::new();
	for _ in common..from.len() {
		rel.push("..");
	}
	for part in &to[common..] {
		rel.push(part);
	}
	rel
}

/// The names a module refers to other modules' items by.
struct Bindings {
	/// How the module names each top-level symbol it refers to: by a binding of its own,
	/// or as a member of a type's module it imports whole, `Decoder.slice`.
	names: HashMap<SymId, String>,
	/// For each module imported from by name: each symbol, in that module's order, its
	/// binding, and whether it is used as a value rather than only as a type.
	imports: BTreeMap<ModPath, Vec<(SymId, String, bool)>>,
	/// For each type's module imported whole: its binding, and whether any of it is used as
	/// a value.
	namespaces: BTreeMap<ModPath, (String, bool)>,
}

impl Bindings {
	fn new(program: &Program, module: &ModPath, items: &[&Item]) -> Result<Self, String> {
		let mut refs = Refs::default();
		for item in items {
			refs.item(program, item);
		}

		let mut names = HashMap::new();
		// What the module declares, with the runtime's binding.
		let mut taken: HashSet<String> = HashSet::from(["rt".to_string()]);
		for (id, sym) in program.syms.iter().enumerate() {
			if sym.owner.is_none() && &sym.module == module {
				names.insert(id, sym.name.clone());
				taken.insert(sym.name.clone());
			}
		}
		let declared = taken.clone();
		let mut bind = |name: &str, from: &ModPath| -> Result<String, String> {
			let qualify = |always| -> Result<String, String> {
				Ok(match program.names.qualifier(name, from, always)? {
					Some(qualifier) => format!("{qualifier}_{name}"),
					None => name.to_string(),
				})
			};
			let mut binding = qualify(false)?;
			// A name the module declares that no other could, as a trait object's class is
			// named for the type it wraps.
			if declared.contains(&binding) {
				binding = qualify(true)?;
			}
			match taken.insert(binding.clone()) {
				true => Ok(binding),
				false => Err(format!(
					"{} would bind two items as `{binding}`",
					module_file(module).display()
				)),
			}
		};

		let mut imports: BTreeMap<ModPath, Vec<(SymId, String, bool)>> = BTreeMap::new();
		let mut namespaces: BTreeMap<ModPath, (String, bool)> = BTreeMap::new();
		let mut foreign: Vec<(SymId, bool)> = refs
			.used
			.into_iter()
			.filter(|(id, _)| &program.syms[*id].module != module)
			.collect();
		let key = |id: SymId| (&program.syms[id].module, program.syms[id].key());
		foreign.sort_by(|a, b| key(a.0).cmp(&key(b.0)));
		for (id, value) in foreign {
			let sym = &program.syms[id];
			if let Some((_, ty)) = program.namespace(&sym.module) {
				let (binding, used) = match namespaces.entry(sym.module.clone()) {
					std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
					std::collections::btree_map::Entry::Vacant(entry) => entry.insert((bind(ty, &sym.module)?, false)),
				};
				*used |= value;
				let member = sym.alias.as_ref().unwrap_or(&sym.name);
				names.insert(id, format!("{binding}.{member}"));
				continue;
			}
			let binding = bind(&sym.name, &sym.module)?;
			names.insert(id, binding.clone());
			imports
				.entry(sym.module.clone())
				.or_default()
				.push((id, binding, value));
		}
		Ok(Self {
			names,
			imports,
			namespaces,
		})
	}
}

/// The top-level symbols a module's items name, and whether each is used as a value.
#[derive(Default)]
struct Refs {
	used: HashMap<SymId, bool>,
}

impl Refs {
	fn use_sym(&mut self, program: &Program, id: SymId, value: bool) {
		let top = program.syms[id].owner.unwrap_or(id);
		// A union has no value to import.
		let value = value && program.syms[top].kind != SymKind::Union;
		let used = self.used.entry(top).or_insert(false);
		*used |= value;
	}

	fn item(&mut self, program: &Program, item: &Item) {
		match item {
			Item::Struct { sym, fields, .. } => {
				for field in fields {
					self.ty(program, &field.ty);
				}
				for f in program.methods.get(sym).into_iter().flatten() {
					self.function(program, f);
				}
			}
			Item::Union { sym, ty, .. } => {
				self.ty(program, ty);
				for f in program.methods.get(sym).into_iter().flatten() {
					self.function(program, f);
				}
			}
			Item::Function(f) => self.function(program, f),
			Item::Value { ty, value, .. } => {
				self.ty(program, ty);
				self.expr(program, value);
			}
		}
	}

	fn function(&mut self, program: &Program, f: &Function) {
		self.params(program, &f.params);
		self.ty(program, &f.ret);
		self.stmts(program, &f.body);
	}

	fn params(&mut self, program: &Program, params: &[Param]) {
		for param in params {
			if let Some(ty) = &param.ty {
				self.ty(program, ty);
			}
		}
	}

	fn ty(&mut self, program: &Program, ty: &Type) {
		match ty {
			Type::Builtin(_) | Type::Param(_) | Type::StrLit(_) => {}
			Type::Rt(_, args) => args.iter().for_each(|t| self.ty(program, t)),
			Type::Sym(id, args) => {
				self.use_sym(program, *id, false);
				args.iter().for_each(|t| self.ty(program, t));
			}
			Type::Union(tys) | Type::Tuple(tys) => tys.iter().for_each(|t| self.ty(program, t)),
			Type::Array(t) => self.ty(program, t),
			Type::Func(params, ret) => {
				params.iter().for_each(|t| self.ty(program, t));
				self.ty(program, ret);
			}
			Type::Object(fields) => fields.iter().for_each(|(_, t)| self.ty(program, t)),
		}
	}

	fn stmts(&mut self, program: &Program, stmts: &[Stmt]) {
		for stmt in stmts {
			match &stmt.kind {
				StmtKind::Expr(e) | StmtKind::Return(Some(e)) => self.expr(program, e),
				StmtKind::Decl { ty, init, .. } => {
					if let Some(ty) = ty {
						self.ty(program, ty);
					}
					if let Some(init) = init {
						self.expr(program, init);
					}
				}
				StmtKind::If { cond, then, els } => {
					self.expr(program, cond);
					self.stmts(program, then);
					self.stmts(program, els);
				}
				StmtKind::While { cond: e, body, .. } | StmtKind::ForOf { iter: e, body, .. } => {
					self.expr(program, e);
					self.stmts(program, body);
				}
				StmtKind::Block { body, .. } => self.stmts(program, body),
				StmtKind::Switch {
					disc, cases, default, ..
				} => {
					self.expr(program, disc);
					for (value, body) in cases {
						self.expr(program, value);
						self.stmts(program, body);
					}
					self.stmts(program, default);
				}
				StmtKind::Declare { ty, .. } => self.ty(program, ty),
				StmtKind::Return(None) | StmtKind::Break(_) | StmtKind::Continue(_) => {}
			}
		}
	}

	fn expr(&mut self, program: &Program, e: &Expr) {
		match e {
			Expr::Ident(_) | Expr::This | Expr::Rt(_) | Expr::Lit(_) | Expr::Str(_) => {}
			Expr::Sym(id) => self.use_sym(program, *id, true),
			Expr::Array(es) => es.iter().for_each(|e| self.expr(program, e)),
			Expr::Object(props) => props.iter().for_each(|(_, e)| self.expr(program, e)),
			Expr::Member(e, _) | Expr::Unary(_, e) | Expr::Located(_, e) | Expr::Postfix(e, _) => self.expr(program, e),
			Expr::Index(a, b) | Expr::Binary(a, _, b) | Expr::Assign(a, b) => {
				self.expr(program, a);
				self.expr(program, b);
			}
			Expr::Call(f, args) | Expr::New(f, args) => {
				self.expr(program, f);
				args.iter().for_each(|e| self.expr(program, e));
			}
			Expr::Cond(a, b, c) => {
				self.expr(program, a);
				self.expr(program, b);
				self.expr(program, c);
			}
			Expr::Arrow(params, ret, body) => {
				self.params(program, params);
				self.ty(program, ret);
				self.stmts(program, body);
			}
			Expr::Lambda(params, body) => {
				self.params(program, params);
				self.expr(program, body);
			}
			Expr::As(e, ty) => {
				self.expr(program, e);
				self.ty(program, ty);
			}
			Expr::Instantiate(e, tys) => {
				self.expr(program, e);
				tys.iter().for_each(|ty| self.ty(program, ty));
			}
		}
	}
}

struct Printer<'p> {
	program: &'p Program,
	bindings: &'p Bindings,
	out: String,
	line: u32,
	/// In UTF-16 units, as source maps count.
	col: u32,
	indent: usize,
	tokens: Vec<(u32, u32, Loc)>,
	uses_rt: bool,
	/// Locals of the function being printed renamed away from the module's bindings.
	renames: HashMap<String, String>,
	/// Each function declared under an escaped name, and the name it is exported as.
	aliases: Vec<(String, String)>,
}

const INDENT: &str = "\t";

impl Printer<'_> {
	fn write(&mut self, text: &str) {
		for ch in text.chars() {
			if ch == '\n' {
				self.line += 1;
				self.col = 0;
			} else {
				self.col += ch.len_utf16() as u32;
			}
		}
		self.out.push_str(text);
	}

	fn newline(&mut self) {
		self.write("\n");
	}

	/// Start a line at the current indent.
	fn start(&mut self) {
		for _ in 0..self.indent {
			self.write(INDENT);
		}
	}

	fn mark(&mut self, loc: Option<Loc>) {
		if let Some(loc) = loc {
			self.tokens.push((self.line, self.col, loc));
		}
	}

	fn doc(&mut self, doc: &Option<String>) {
		let Some(doc) = doc else {
			return;
		};
		let lines: Vec<&str> = doc.lines().collect();
		self.start();
		if let [line] = lines.as_slice() {
			let line = line.trim().replace("*/", "*\\/");
			self.write(&format!("/** {line} */\n"));
			return;
		}
		self.write("/**\n");
		for line in lines {
			self.start();
			let line = line.strip_prefix(' ').unwrap_or(line).replace("*/", "*\\/");
			match line.is_empty() {
				true => self.write(" *\n"),
				false => self.write(&format!(" * {line}\n")),
			}
		}
		self.start();
		self.write(" */\n");
	}

	fn sym_name(&self, id: SymId) -> String {
		let sym = &self.program.syms[id];
		match sym.owner {
			Some(owner) => format!("{}.{}", self.bindings.names[&owner], sym.name),
			None => self.bindings.names[&id].clone(),
		}
	}

	fn key(&self, key: &Key) -> String {
		match key {
			Key::Name(name) => name.clone(),
			Key::Field(id) => self.program.fields[*id].name.clone(),
		}
	}

	fn item(&mut self, item: &Item) {
		match item {
			Item::Struct {
				sym,
				params,
				fields,
				ctor,
				doc,
				..
			} => {
				self.doc(doc);
				let name = self.program.syms[*sym].name.clone();
				self.start();
				if self.program.plain.contains(sym) {
					self.write(&format!("export interface {name}{} {{\n", type_params(params)));
					self.indent += 1;
					for field in fields {
						self.doc(&field.doc);
						self.start();
						let ty = self.ty(&field.ty);
						self.write(&format!("{}: {ty};\n", self.program.fields[field.id].name));
					}
					self.indent -= 1;
					self.start();
					self.write("}\n");
					return;
				}
				self.write(&format!("export class {name}{} {{\n", type_params(params)));
				self.indent += 1;
				for field in fields {
					self.doc(&field.doc);
					self.start();
					let ty = self.ty(&field.ty);
					let name = self.program.fields[field.id].name.clone();
					match &field.init {
						FieldInit::Ctor => self.write(&format!("{name}: {ty};\n")),
						FieldInit::Later => self.write(&format!("{name}!: {ty};\n")),
						FieldInit::Value(value) => {
							self.write(&format!("{name}: {ty} = "));
							self.expr(value, 2);
							self.write(";\n");
						}
					}
				}
				let fields: Vec<&crate::ts::Field> =
					fields.iter().filter(|f| matches!(f.init, FieldInit::Ctor)).collect();
				let names: Vec<String> = fields.iter().map(|f| self.program.fields[f.id].name.clone()).collect();
				match ctor {
					Ctor::Unit => {}
					Ctor::Positional => {
						// Each field's parameter is its name without the `#`, escaped where
						// strict code reserves it, and numbered where escaping made it another's.
						let mut taken = HashSet::new();
						let args: Vec<String> = names
							.iter()
							.map(|name| unique(&mut taken, &crate::lower::escape(name.trim_start_matches('#'))))
							.collect();
						let params: Vec<String> = (args.iter().zip(fields))
							.map(|(arg, field)| format!("{arg}: {}", self.ty(&field.ty)))
							.collect();
						self.newline();
						self.start();
						self.write(&format!("constructor({}) {{\n", params.join(", ")));
						for (name, arg) in names.iter().zip(&args) {
							self.start();
							self.write(&format!("{INDENT}this.{name} = {arg};\n"));
						}
						self.start();
						self.write("}\n");
					}
				}
				for method in self.program.methods.get(sym).into_iter().flatten() {
					self.newline();
					self.function(method, false);
				}
				self.indent -= 1;
				self.start();
				self.write("}\n");
			}
			Item::Union {
				sym, params, ty, doc, ..
			} => {
				self.doc(doc);
				let name = self.program.syms[*sym].name.clone();
				self.start();
				let header = format!("export type {name}{} =", type_params(params));
				let variants: Vec<String> = match ty {
					Type::Union(variants) => variants.iter().map(|v| self.ty(v)).collect(),
					ty => vec![self.ty(ty)],
				};
				let line = format!("{header} {};", variants.join(" | "));
				// A variant a line once they don't fit one.
				if line.len() <= 100 {
					self.write(&format!("{line}\n"));
				} else {
					self.write(&format!("{header}\n"));
					for (i, variant) in variants.iter().enumerate() {
						let end = if i + 1 == variants.len() { ";" } else { "" };
						self.start();
						self.write(&format!("{INDENT}| {variant}{end}\n"));
					}
				}
			}
			Item::Function(f) => self.function(f, true),
			Item::Value { sym, ty, value, doc } => {
				self.doc(doc);
				self.start();
				let name = self.program.syms[*sym].name.clone();
				let decl = match &self.program.syms[*sym].alias {
					Some(alias) => {
						self.aliases.push((name.clone(), alias.clone()));
						"const"
					}
					None => "export const",
				};
				let ty = self.ty(ty);
				// Only a call or `new` takes the mark: a literal is pure already, and Rollup
				// warns of a mark anywhere else.
				let pure = match value {
					Expr::Call(..) | Expr::New(..) => "/* @__PURE__ */ ",
					_ => "",
				};
				self.write(&format!("{decl} {name}: {ty} = {pure}"));
				self.expr(value, 2);
				self.write(";\n");
			}
		}
	}

	/// Print `f`, a module's function, or else a class's method.
	fn function(&mut self, f: &Function, module: bool) {
		self.renames = self.renames_for(f);
		self.doc(&f.doc);
		self.start();
		self.mark(f.loc);
		let name = self.program.syms[f.sym].name.clone();
		match (module, &self.program.syms[f.sym].alias) {
			(true, Some(alias)) => {
				self.aliases.push((name.clone(), alias.clone()));
				self.write(&format!("function {name}("));
			}
			(true, None) => self.write(&format!("export function {name}(")),
			(false, _) => self.write(&format!("{name}(")),
		}
		self.params(&f.params);
		let ret = self.ty(&f.ret);
		self.write(&format!("): {ret} {{\n"));
		self.block_body(&f.body);
		self.start();
		self.write("}\n");
	}

	/// Renames for the locals of `f` that would shadow a binding it refers to.
	fn renames_for(&self, f: &Function) -> HashMap<String, String> {
		let mut locals = Vec::new();
		for param in &f.params {
			locals.push(param.name.clone());
		}
		collect_locals(&f.body, &mut locals);
		let mut refs = Refs::default();
		refs.function(self.program, f);
		// The runtime's too, which printing may call anywhere.
		let referred: HashSet<&str> = (refs.used.keys())
			.map(|id| self.bindings.names[id].split('.').next().unwrap())
			.chain(["rt"])
			.collect();
		let mut taken: HashSet<String> = locals.iter().cloned().collect();
		taken.extend(referred.iter().map(|name| name.to_string()));
		let mut renames = HashMap::new();
		for local in locals {
			if referred.contains(local.as_str()) {
				let mut name = format!("{local}_");
				while taken.contains(&name) {
					name.push('_');
				}
				taken.insert(name.clone());
				renames.insert(local, name);
			}
		}
		renames
	}

	fn local<'n>(&'n self, name: &'n str) -> &'n str {
		self.renames.get(name).map_or(name, String::as_str)
	}

	fn params(&mut self, params: &[Param]) {
		for (i, param) in params.iter().enumerate() {
			if i > 0 {
				self.write(", ");
			}
			let name = self.local(&param.name).to_string();
			self.write(&name);
			if let Some(ty) = &param.ty {
				let ty = self.ty(ty);
				self.write(&format!(": {ty}"));
			}
		}
	}

	fn block_body(&mut self, stmts: &[Stmt]) {
		self.indent += 1;
		for stmt in stmts {
			self.stmt(stmt);
		}
		self.indent -= 1;
	}

	fn label(&mut self, label: &Option<String>) {
		if let Some(label) = label {
			self.write(&format!("{label}: "));
		}
	}

	fn stmt(&mut self, stmt: &Stmt) {
		self.start();
		self.mark(stmt.loc);
		match &stmt.kind {
			StmtKind::Expr(e) => {
				// A statement can't start with `{`, which would open a block.
				let prec = match starts_with_object(e) {
					true => 20,
					false => 0,
				};
				self.expr(e, prec);
				self.write(";\n");
			}
			StmtKind::Decl {
				mutable,
				name,
				ty,
				init,
			} => {
				self.write(if *mutable { "let " } else { "const " });
				let name = self.local(name).to_string();
				self.write(&name);
				if let Some(ty) = ty {
					let ty = self.ty(ty);
					self.write(&format!(": {ty}"));
				}
				if let Some(init) = init {
					self.write(" = ");
					self.expr(init, 2);
				}
				self.write(";\n");
			}
			StmtKind::Return(value) => {
				self.write("return");
				if let Some(value) = value {
					self.write(" ");
					self.expr(value, 0);
				}
				self.write(";\n");
			}
			StmtKind::If { cond, then, els } => {
				self.write("if (");
				self.expr(cond, 0);
				self.write(") {\n");
				self.block_body(then);
				let mut els = els;
				loop {
					match els.as_slice() {
						[] => break,
						[
							Stmt {
								kind: StmtKind::If { cond, then, els: next },
								loc,
							},
						] => {
							self.start();
							self.write("} else ");
							self.mark(*loc);
							self.write("if (");
							self.expr(cond, 0);
							self.write(") {\n");
							self.block_body(then);
							els = next;
						}
						_ => {
							self.start();
							self.write("} else {\n");
							self.block_body(els);
							break;
						}
					}
				}
				self.start();
				self.write("}\n");
			}
			StmtKind::While { label, cond, body } => {
				self.label(label);
				self.write("while (");
				self.expr(cond, 0);
				self.write(") {\n");
				self.block_body(body);
				self.start();
				self.write("}\n");
			}
			StmtKind::ForOf {
				label,
				name,
				iter,
				body,
			} => {
				self.label(label);
				let name = self.local(name).to_string();
				self.write(&format!("for (const {name} of "));
				self.expr(iter, 2);
				self.write(") {\n");
				self.block_body(body);
				self.start();
				self.write("}\n");
			}
			StmtKind::Block { label, body } => {
				self.label(label);
				self.write("{\n");
				self.block_body(body);
				self.start();
				self.write("}\n");
			}
			StmtKind::Switch {
				label,
				disc,
				cases,
				default,
			} => {
				self.label(label);
				self.write("switch (");
				self.expr(disc, 0);
				self.write(") {\n");
				self.indent += 1;
				for (value, body) in cases {
					self.start();
					self.write("case ");
					self.expr(value, 0);
					self.write(":\n");
					self.block_body(body);
				}
				if !default.is_empty() {
					self.start();
					self.write("default:\n");
					self.block_body(default);
				}
				self.indent -= 1;
				self.start();
				self.write("}\n");
			}
			StmtKind::Declare { name, ty } => {
				let name = self.local(name).to_string();
				let ty = self.ty(ty);
				self.write(&format!("let {name}!: {ty};\n"));
			}
			StmtKind::Break(label) => match label {
				Some(label) => self.write(&format!("break {label};\n")),
				None => self.write("break;\n"),
			},
			StmtKind::Continue(label) => match label {
				Some(label) => self.write(&format!("continue {label};\n")),
				None => self.write("continue;\n"),
			},
		}
	}

	/// Print `e`, parenthesized if it binds looser than `min`.
	fn expr(&mut self, e: &Expr, min: u8) {
		let prec = prec(e);
		let parens = prec < min;
		if parens {
			self.write("(");
		}
		match e {
			Expr::Ident(name) => {
				let name = self.local(name).to_string();
				self.write(&name);
			}
			Expr::This => self.write("this"),
			Expr::Sym(id) => {
				let name = self.sym_name(*id);
				self.write(&name);
			}
			Expr::Rt(name) => {
				self.uses_rt = true;
				self.write(&format!("rt.{name}"));
			}
			Expr::Lit(text) => self.write(text),
			Expr::Str(s) => self.write(&quote(s)),
			Expr::Array(items) => {
				self.write("[");
				self.list(items);
				self.write("]");
			}
			Expr::Object(props) => {
				if props.is_empty() {
					self.write("{}");
				} else {
					self.write("{ ");
					for (i, (key, value)) in props.iter().enumerate() {
						if i > 0 {
							self.write(", ");
						}
						let key = self.key(key);
						match value {
							Expr::Ident(name) if self.local(name) == key => self.write(&key),
							_ => {
								self.write(&format!("{key}: "));
								self.expr(value, 2);
							}
						}
					}
					self.write(" }");
				}
			}
			Expr::Member(base, key) => {
				self.base(base);
				let key = self.key(key);
				self.write(&format!(".{key}"));
			}
			Expr::Index(base, index) => {
				self.base(base);
				self.write("[");
				self.expr(index, 0);
				self.write("]");
			}
			Expr::Call(f, args) => {
				self.base(f);
				self.write("(");
				self.list(args);
				self.write(")");
			}
			Expr::New(f, args) => {
				self.write("new ");
				self.base(f);
				self.write("(");
				self.list(args);
				self.write(")");
			}
			Expr::Unary(op, arg) => {
				self.write(op);
				// `- -x` and `!(-x)` read better spaced or grouped.
				let nested = matches!(&**arg, Expr::Unary(..)) || matches!(&**arg, Expr::Lit(t) if t.starts_with('-'));
				match nested {
					true => {
						self.write("(");
						self.expr(arg, 0);
						self.write(")");
					}
					false => self.expr(arg, 17),
				}
			}
			Expr::Postfix(arg, op) => {
				self.expr(arg, 19);
				self.write(op);
			}
			Expr::Binary(l, op, r) => {
				let p = binary_prec(op);
				// The type after `as` would read on into `&`, `|`, or `<` as its own.
				let left = match &**l {
					Expr::As(..) => 21,
					Expr::Located(_, e) if matches!(&**e, Expr::As(..)) => 21,
					_ => p,
				};
				self.operand(l, op, left);
				self.write(&format!(" {op} "));
				// `&&` and `||` are associative, so a chain reads without groups.
				let right = match is_logical(op) && matches!(&**r, Expr::Binary(_, inner, _) if inner == op) {
					true => p,
					false => p + 1,
				};
				self.operand(r, op, right);
			}
			Expr::Cond(c, a, b) => {
				self.expr(c, 5);
				self.write(" ? ");
				self.expr(a, 3);
				self.write(" : ");
				self.expr(b, 3);
			}
			Expr::Assign(target, value) => {
				self.expr(target, 19);
				self.write(" = ");
				self.expr(value, 3);
			}
			Expr::Arrow(params, ret, body) => {
				self.write("(");
				self.params(params);
				let ret = self.ty(ret);
				self.write(&format!("): {ret} => {{"));
				if body.is_empty() {
					self.write("}");
				} else {
					self.newline();
					self.block_body(body);
					self.start();
					self.write("}");
				}
			}
			Expr::Lambda(params, body) => {
				self.write("(");
				self.params(params);
				self.write(") => ");
				// A body that is an object literal would read as a block.
				let min = if matches!(**body, Expr::Object(_)) { 21 } else { 3 };
				self.expr(body, min);
			}
			Expr::As(inner, ty) => {
				self.expr(inner, 13);
				let ty = self.ty(ty);
				self.write(&format!(" as {ty}"));
			}
			Expr::Instantiate(f, tys) => {
				self.base(f);
				let tys: Vec<String> = tys.iter().map(|ty| self.ty(ty)).collect();
				self.write(&format!("<{}>", tys.join(", ")));
			}
			Expr::Located(loc, inner) => {
				self.mark(Some(*loc));
				self.expr(inner, min.max(prec));
			}
		}
		if parens {
			self.write(")");
		}
	}

	/// The object of a member access, call, or `new`.
	fn base(&mut self, e: &Expr) {
		match e {
			// `1.x` would read as a fraction.
			Expr::Lit(text) if text.starts_with(|c: char| c.is_ascii_digit() || c == '-') => {
				self.write("(");
				self.expr(e, 0);
				self.write(")");
			}
			_ => self.expr(e, 19),
		}
	}

	/// An operand of `op`: grouped where JS precedence holds but a reader would have to
	/// look it up, such as bitwise operators mixed with each other.
	fn operand(&mut self, e: &Expr, op: &str, min: u8) {
		let inner = match e {
			Expr::Located(_, e) => &**e,
			e => e,
		};
		let confusing = match inner {
			Expr::Binary(_, inner, _) => {
				inner != &op && (is_bitwise(op) || is_bitwise(inner) || (is_logical(op) && is_logical(inner)))
			}
			_ => false,
		};
		match confusing {
			true => {
				self.write("(");
				self.expr(e, 0);
				self.write(")");
			}
			false => self.expr(e, min),
		}
	}

	fn list(&mut self, items: &[Expr]) {
		for (i, item) in items.iter().enumerate() {
			if i > 0 {
				self.write(", ");
			}
			self.expr(item, 2);
		}
	}

	fn ty(&mut self, ty: &Type) -> String {
		match ty {
			Type::Builtin(name) => name.to_string(),
			Type::Rt(name, args) => {
				self.uses_rt = true;
				format!("rt.{name}{}", self.type_args(args))
			}
			Type::Sym(id, args) => format!("{}{}", self.sym_name(*id), self.type_args(args)),
			Type::Param(name) => name.clone(),
			Type::Union(tys) => {
				let parts: Vec<String> = tys
					.iter()
					.map(|t| match t {
						Type::Func(..) => format!("({})", self.ty(t)),
						_ => self.ty(t),
					})
					.collect();
				parts.join(" | ")
			}
			Type::Tuple(tys) => {
				let parts: Vec<String> = tys.iter().map(|t| self.ty(t)).collect();
				format!("[{}]", parts.join(", "))
			}
			Type::Array(t) => match &**t {
				Type::Union(_) | Type::Func(..) => format!("Array<{}>", self.ty(t)),
				_ => format!("{}[]", self.ty(t)),
			},
			Type::Func(params, ret) => {
				let parts: Vec<String> = params
					.iter()
					.enumerate()
					.map(|(i, t)| format!("a{i}: {}", self.ty(t)))
					.collect();
				format!("({}) => {}", parts.join(", "), self.ty(ret))
			}
			Type::StrLit(s) => quote(s),
			Type::Object(fields) => {
				let parts: Vec<String> = fields
					.iter()
					.map(|(name, t)| format!("{name}: {}", self.ty(t)))
					.collect();
				format!("{{ {} }}", parts.join("; "))
			}
		}
	}

	fn type_args(&mut self, args: &[Type]) -> String {
		if args.is_empty() {
			return String::new();
		}
		let parts: Vec<String> = args.iter().map(|t| self.ty(t)).collect();
		format!("<{}>", parts.join(", "))
	}
}

fn type_params(params: &[String]) -> String {
	match params.is_empty() {
		true => String::new(),
		false => format!("<{}>", params.join(", ")),
	}
}

fn is_bitwise(op: &str) -> bool {
	matches!(op, "&" | "|" | "^" | "<<" | ">>" | ">>>")
}

fn is_logical(op: &str) -> bool {
	matches!(op, "&&" | "||")
}

fn binary_prec(op: &str) -> u8 {
	match op {
		"||" => 6,
		"&&" => 7,
		"|" => 8,
		"^" => 9,
		"&" => 10,
		"==" | "!=" | "===" | "!==" => 11,
		"<" | ">" | "<=" | ">=" | "instanceof" | "in" => 12,
		"<<" | ">>" | ">>>" => 13,
		"+" | "-" => 14,
		"*" | "/" | "%" => 15,
		"**" => 16,
		_ => unreachable!("unknown operator {op}"),
	}
}

fn prec(e: &Expr) -> u8 {
	match e {
		Expr::Assign(..) | Expr::Arrow(..) | Expr::Lambda(..) => 3,
		Expr::Cond(..) => 4,
		Expr::Binary(_, op, _) => binary_prec(op),
		Expr::As(..) => 12,
		Expr::Unary(..) => 17,
		Expr::Postfix(..) => 18,
		Expr::Lit(text) if text.starts_with('-') => 17,
		Expr::New(..) | Expr::Call(..) | Expr::Member(..) | Expr::Index(..) | Expr::Instantiate(..) => 19,
		Expr::Located(_, e) => prec(e),
		_ => 20,
	}
}

/// A JS string literal.
pub fn quote(s: &str) -> String {
	let mut out = String::with_capacity(s.len() + 2);
	out.push('"');
	for ch in s.chars() {
		match ch {
			'"' => out.push_str("\\\""),
			'\\' => out.push_str("\\\\"),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\t' => out.push_str("\\t"),
			c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
				write!(out, "\\u{:04x}", c as u32).unwrap();
			}
			c => out.push(c),
		}
	}
	out.push('"');
	out
}

/// Every name a function declares, its closures' included.
fn collect_locals(stmts: &[Stmt], out: &mut Vec<String>) {
	for stmt in stmts {
		match &stmt.kind {
			StmtKind::Decl { name, init, .. } => {
				out.push(name.clone());
				if let Some(init) = init {
					collect_expr_locals(init, out);
				}
			}
			StmtKind::Expr(e) | StmtKind::Return(Some(e)) => collect_expr_locals(e, out),
			StmtKind::If { cond, then, els } => {
				collect_expr_locals(cond, out);
				collect_locals(then, out);
				collect_locals(els, out);
			}
			StmtKind::While { cond, body, .. } => {
				collect_expr_locals(cond, out);
				collect_locals(body, out);
			}
			StmtKind::ForOf { name, iter, body, .. } => {
				out.push(name.clone());
				collect_expr_locals(iter, out);
				collect_locals(body, out);
			}
			StmtKind::Block { body, .. } => collect_locals(body, out),
			StmtKind::Switch {
				disc, cases, default, ..
			} => {
				collect_expr_locals(disc, out);
				for (value, body) in cases {
					collect_expr_locals(value, out);
					collect_locals(body, out);
				}
				collect_locals(default, out);
			}
			StmtKind::Declare { name, .. } => out.push(name.clone()),
			StmtKind::Return(None) | StmtKind::Break(_) | StmtKind::Continue(_) => {}
		}
	}
}

fn collect_expr_locals(e: &Expr, out: &mut Vec<String>) {
	match e {
		Expr::Arrow(params, _, body) => {
			out.extend(params.iter().map(|p| p.name.clone()));
			collect_locals(body, out);
		}
		Expr::Lambda(params, body) => {
			out.extend(params.iter().map(|p| p.name.clone()));
			collect_expr_locals(body, out);
		}
		Expr::Array(es) => es.iter().for_each(|e| collect_expr_locals(e, out)),
		Expr::Object(props) => props.iter().for_each(|(_, e)| collect_expr_locals(e, out)),
		Expr::Member(e, _)
		| Expr::Unary(_, e)
		| Expr::Located(_, e)
		| Expr::As(e, _)
		| Expr::Instantiate(e, _)
		| Expr::Postfix(e, _) => collect_expr_locals(e, out),
		Expr::Index(a, b) | Expr::Binary(a, _, b) | Expr::Assign(a, b) => {
			collect_expr_locals(a, out);
			collect_expr_locals(b, out);
		}
		Expr::Call(f, args) | Expr::New(f, args) => {
			collect_expr_locals(f, out);
			args.iter().for_each(|e| collect_expr_locals(e, out));
		}
		Expr::Cond(a, b, c) => {
			collect_expr_locals(a, out);
			collect_expr_locals(b, out);
			collect_expr_locals(c, out);
		}
		Expr::Ident(_) | Expr::This | Expr::Sym(_) | Expr::Rt(_) | Expr::Lit(_) | Expr::Str(_) => {}
	}
}

/// Whether `e` prints starting with an object literal, as `{ start: g }.start` does.
fn starts_with_object(e: &Expr) -> bool {
	match e {
		Expr::Object(_) => true,
		Expr::Member(e, _)
		| Expr::Index(e, _)
		| Expr::Call(e, _)
		| Expr::Postfix(e, _)
		| Expr::Binary(e, _, _)
		| Expr::Cond(e, _, _)
		| Expr::Assign(e, _)
		| Expr::As(e, _)
		| Expr::Instantiate(e, _)
		| Expr::Located(_, e) => starts_with_object(e),
		_ => false,
	}
}
