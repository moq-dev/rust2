//! The TypeScript that lowering builds and `print` writes: only what lowering emits.
//!
//! Translated items and class fields are referenced by id, and named once every item
//! is known, since a later item can force a rename.

use std::collections::HashSet;

use rust2_driver::cfg;

/// A translated item: a function, struct, or type alias.
pub type SymId = usize;

/// A field of a translated struct, named with the other names its class may define.
pub type FieldId = usize;

/// A position in a Rust source file, for the source map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Loc {
	/// An index into the program's source files.
	pub file: u32,
	/// Zero-based line.
	pub line: u32,
	/// Zero-based UTF-16 column, as source maps count them.
	pub col: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Expr {
	Ident(String),
	This,
	/// A translated item, named at print time.
	Sym(SymId),
	/// A runtime export: `rs.<name>`.
	Rs(String),
	/// Literal text, printed as is: `1`, `0x40`, `5n`, `true`, `undefined`.
	Lit(String),
	Str(String),
	Array(Vec<Expr>),
	Object(Vec<(Key, Expr)>),
	Member(Box<Expr>, Key),
	Index(Box<Expr>, Box<Expr>),
	Call(Box<Expr>, Vec<Expr>),
	New(Box<Expr>, Vec<Expr>),
	Unary(&'static str, Box<Expr>),
	/// `x++` and the like: the operand's value, before the operator changes it.
	Postfix(Box<Expr>, &'static str),
	Binary(Box<Expr>, &'static str, Box<Expr>),
	Cond(Box<Expr>, Box<Expr>, Box<Expr>),
	Assign(Box<Expr>, Box<Expr>),
	Arrow(Vec<Param>, Type, Vec<Stmt>),
	/// An arrow function whose body is one expression, its return type inferred:
	/// `(f: rs.Formatter) => rs.fmtPad(f, s)`.
	Lambda(Vec<Param>, Box<Expr>),
	/// `e as T`: a literal whose type TypeScript would widen, or an integer cast to
	/// another integer type with the same value.
	As(Box<Expr>, Type),
	/// `f<A, B>`: a generic function given its type arguments, which TypeScript cannot
	/// infer from its call alone.
	Instantiate(Box<Expr>, Vec<Type>),
	Located(Loc, Box<Expr>),
}

/// A property name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
	Name(String),
	Field(FieldId),
}

impl Expr {
	pub fn ident(name: impl Into<String>) -> Self {
		Self::Ident(name.into())
	}

	pub fn lit(text: impl Into<String>) -> Self {
		Self::Lit(text.into())
	}

	pub fn undefined() -> Self {
		Self::lit("undefined")
	}

	pub fn member(self, name: impl Into<String>) -> Self {
		Self::Member(Box::new(self), Key::Name(name.into()))
	}

	/// What `self` reads, if it is an `rs.Place` made here: `rs.place(() => read, set)`.
	pub fn place_read(&self) -> Option<&Expr> {
		let Self::Call(f, args) = self else { return None };
		match (&**f, args.as_slice()) {
			(Self::Rs(name), [Self::Lambda(params, read), _]) if name == "place" && params.is_empty() => Some(read),
			_ => None,
		}
	}

	pub fn field(self, field: FieldId) -> Self {
		Self::Member(Box::new(self), Key::Field(field))
	}

	pub fn index(self, index: Expr) -> Self {
		Self::Index(Box::new(self), Box::new(index))
	}

	pub fn call(self, args: Vec<Expr>) -> Self {
		Self::Call(Box::new(self), args)
	}

	/// A call of the runtime function `rs.<name>`.
	pub fn rs(name: impl Into<String>, args: Vec<Expr>) -> Self {
		Self::Rs(name.into()).call(args)
	}

	/// `rs.Ok(value)`, or `rs.Err(value)` if not `ok`: for `undefined`, `rs.OK` or `rs.ERR`,
	/// one object every evaluation shares, since nothing writes into a result in place and
	/// `undefined` holds nothing to write into.
	pub fn result(ok: bool, value: Expr) -> Self {
		let unit = matches!(&value, Self::Lit(text) if text == "undefined");
		match (ok, unit) {
			(true, true) => Self::Rs("OK".into()),
			(false, true) => Self::Rs("ERR".into()),
			(true, false) => Self::rs("Ok", vec![value]),
			(false, false) => Self::rs("Err", vec![value]),
		}
	}

	/// The runtime function `name` of what `of` names: `rs.u8Add` for `("u8", "add")`.
	pub fn rs_of(of: &str, name: &str) -> Self {
		let mut chars = name.chars();
		let first = chars.next().map(|c| c.to_ascii_uppercase()).into_iter();
		Self::Rs(format!("{of}{}{}", first.collect::<String>(), chars.as_str()))
	}

	pub fn binary(self, op: &'static str, rhs: Expr) -> Self {
		Self::Binary(Box::new(self), op, Box::new(rhs))
	}

	pub fn unary(op: &'static str, arg: Expr) -> Self {
		Self::Unary(op, Box::new(arg))
	}

	pub fn assign(self, value: Expr) -> Self {
		Self::Assign(Box::new(self), Box::new(value))
	}

	pub fn not(self) -> Self {
		match self {
			Self::Unary("!", inner) => *inner,
			Self::Lit(t) if t == "true" => Self::lit("false"),
			Self::Lit(t) if t == "false" => Self::lit("true"),
			Self::Binary(l, "===", r) => Self::Binary(l, "!==", r),
			Self::Binary(l, "!==", r) => Self::Binary(l, "===", r),
			other => Self::unary("!", other),
		}
	}

	/// This without an outer `as`, where the context already gives the type.
	pub fn typed(self) -> Self {
		match self {
			// Limbs keep theirs, which the brand of their type requires, and an `rs.RangeBound` its
			// own, without which a declaration narrows to the one variant it is made as.
			Self::As(e, ty)
				if matches!(*e, Self::Str(_) | Self::Array(_) | Self::Object(_))
					&& !wide(&ty)
					&& !matches!(ty, Type::Rs("RangeBound", _)) =>
			{
				*e
			}
			Self::Located(loc, e) => Self::Located(loc, Box::new(e.typed())),
			// `rs.Ok(v)` and `rs.Err(e)` infer their type from the context too.
			Self::Call(f, args) if matches!(&*f, Self::Rs(name) if name == "Ok" || name == "Err") => {
				Self::Call(f, args.into_iter().map(Self::typed).collect())
			}
			e => e,
		}
	}

	/// Whether evaluating this can neither do anything nor see anything change:
	/// a literal, or `this`.
	pub fn is_constant(&self) -> bool {
		match self {
			Self::Lit(_) | Self::Str(_) | Self::This | Self::Sym(_) | Self::Rs(_) => true,
			Self::Located(_, e) => e.is_constant(),
			// A wide integer literal past 2^53, `{ hi: 0xffffffff, lo: 0xffffffff } as rs.u64`:
			// immutable limbs, which compare by value.
			Self::As(e, ty) if wide(ty) => {
				matches!(&**e, Self::Object(fields) if fields.iter().all(|(_, v)| matches!(v, Self::Lit(_))))
			}
			_ => false,
		}
	}

	/// The text of a path of names, the same for two paths that read the same place.
	pub fn path_key(&self) -> Option<String> {
		match self {
			Self::Ident(name) => Some(name.clone()),
			Self::This => Some("this".into()),
			Self::Member(base, key) => {
				let key = match key {
					Key::Name(name) => name.clone(),
					Key::Field(id) => format!("#{id}"),
				};
				Some(format!("{}.{key}", base.path_key()?))
			}
			Self::Located(_, e) => e.path_key(),
			_ => None,
		}
	}

	/// The expression under its source locations.
	pub fn unlocated(&self) -> &Self {
		match self {
			Self::Located(_, e) => e.unlocated(),
			e => e,
		}
	}

	/// Whether evaluating this twice is free and gives the same value both times, so a
	/// pattern may read it more than once: a name, or a member path from one.
	pub fn is_path(&self) -> bool {
		match self {
			Self::Ident(_) | Self::This => true,
			Self::Member(base, _) => base.is_path(),
			Self::Located(_, e) => e.is_path(),
			e => e.is_constant(),
		}
	}
}

/// Whether `ty` is a 64- or 128-bit integer type, whose values past 2^53 are limbs.
fn wide(ty: &Type) -> bool {
	matches!(ty, Type::Rs("u64" | "i64" | "u128" | "i128", _))
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Param {
	pub name: String,
	pub ty: Option<Type>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Stmt {
	pub kind: StmtKind,
	pub loc: Option<Loc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum StmtKind {
	Expr(Expr),
	Decl {
		mutable: bool,
		name: String,
		ty: Option<Type>,
		init: Option<Expr>,
	},
	Return(Option<Expr>),
	If {
		cond: Expr,
		then: Vec<Stmt>,
		els: Vec<Stmt>,
	},
	While {
		label: Option<String>,
		cond: Expr,
		body: Vec<Stmt>,
	},
	/// `for (const name of iter) body`, over an array.
	ForOf {
		label: Option<String>,
		name: String,
		iter: Expr,
		body: Vec<Stmt>,
	},
	Block {
		label: Option<String>,
		body: Vec<Stmt>,
	},
	Break(Option<String>),
	Continue(Option<String>),
	/// `switch (disc) { case a: ... default: ... }`, whose cases never fall through.
	Switch {
		label: Option<String>,
		disc: Expr,
		cases: Vec<(Expr, Vec<Stmt>)>,
		default: Vec<Stmt>,
	},
	/// `let name!: ty;`: a variable every read of which some write precedes, in ways
	/// `tsc` can't follow.
	Declare {
		name: String,
		ty: Type,
	},
}

impl StmtKind {
	pub fn at(self, loc: Option<Loc>) -> Stmt {
		Stmt { kind: self, loc }
	}

	/// `const name = init`, or `let` if `mutable`.
	pub fn decl(mutable: bool, name: impl Into<String>, ty: Option<Type>, init: Option<Expr>) -> Self {
		let init = match ty {
			Some(_) => init.map(Expr::typed),
			None => init,
		};
		Self::Decl {
			mutable,
			name: name.into(),
			ty,
			init,
		}
	}
}

impl cfg::Syntax for Stmt {
	type Cond = Expr;

	fn block(label: String, body: Vec<Self>) -> Self {
		StmtKind::Block {
			label: Some(label),
			body,
		}
		.at(None)
	}

	fn forever(label: String, body: Vec<Self>) -> Self {
		StmtKind::While {
			label: Some(label),
			cond: Expr::lit("true"),
			body,
		}
		.at(None)
	}

	fn branch(cond: Expr, then: Vec<Self>, els: Vec<Self>) -> Self {
		StmtKind::If { cond, then, els }.at(None)
	}

	fn jump(jump: cfg::Jump, label: String) -> Self {
		match jump {
			cfg::Jump::Break => StmtKind::Break(Some(label)),
			cfg::Jump::Continue => StmtKind::Continue(Some(label)),
		}
		.at(None)
	}

	fn dispatch(state: &str, label: &str, cases: Vec<(usize, Vec<Self>)>, out: &mut Vec<Self>) {
		let cases = cases
			.into_iter()
			.map(|(node, body)| (Expr::lit(node.to_string()), body))
			.collect();
		out.push(StmtKind::decl(true, state, Some(Type::Builtin("number")), Some(Expr::lit("0"))).at(None));
		out.push(
			StmtKind::While {
				label: Some(label.to_string()),
				cond: Expr::lit("true"),
				body: vec![
					StmtKind::Switch {
						label: None,
						disc: Expr::ident(state),
						cases,
						default: Vec::new(),
					}
					.at(None),
				],
			}
			.at(None),
		);
	}

	fn set(state: &str, node: usize) -> Self {
		StmtKind::Expr(Expr::ident(state).assign(Expr::lit(node.to_string()))).at(None)
	}

	fn view(&self) -> cfg::View<'_, Self> {
		match &self.kind {
			StmtKind::Block { label, body } => cfg::Form::Block { label, body },
			StmtKind::While { label, cond, body } => cfg::Form::Loop {
				label,
				cond: Some(cond),
				body,
			},
			StmtKind::ForOf { label, body, .. } => cfg::Form::Loop {
				label,
				cond: None,
				body,
			},
			StmtKind::If { cond, then, els } => cfg::Form::If { cond, then, els },
			StmtKind::Switch { cases, default, .. } => {
				cfg::Form::Switch(cases.iter().map(|(_, body)| body).chain([default]).collect())
			}
			StmtKind::Break(label) => cfg::Form::Break(label),
			StmtKind::Continue(label) => cfg::Form::Continue(label),
			StmtKind::Return(value) => cfg::Form::Return(value.is_some()),
			_ => cfg::Form::Other,
		}
	}

	fn view_mut(&mut self) -> cfg::ViewMut<'_, Self> {
		match &mut self.kind {
			StmtKind::Block { label, body } => cfg::Form::Block { label, body },
			StmtKind::While { label, cond, body } => cfg::Form::Loop {
				label,
				cond: Some(cond),
				body,
			},
			StmtKind::ForOf { label, body, .. } => cfg::Form::Loop {
				label,
				cond: None,
				body,
			},
			StmtKind::If { cond, then, els } => cfg::Form::If { cond, then, els },
			StmtKind::Switch { cases, default, .. } => {
				cfg::Form::Switch(cases.iter_mut().map(|(_, body)| body).chain([default]).collect())
			}
			StmtKind::Break(label) => cfg::Form::Break(label),
			StmtKind::Continue(label) => cfg::Form::Continue(label),
			StmtKind::Return(value) => cfg::Form::Return(value.is_some()),
			_ => cfg::Form::Other,
		}
	}
}

impl cfg::Cond for Expr {
	fn negate(&mut self) {
		*self = std::mem::replace(self, Expr::undefined()).not();
	}

	fn is_true(&self) -> bool {
		matches!(self, Expr::Lit(l) if l == "true")
	}

	fn pure(&self) -> bool {
		match self {
			Expr::Ident(_) | Expr::This | Expr::Sym(_) | Expr::Rs(_) | Expr::Lit(_) | Expr::Str(_) => true,
			Expr::Member(base, _)
			| Expr::Unary(_, base)
			| Expr::Located(_, base)
			| Expr::As(base, _)
			| Expr::Instantiate(base, _) => base.pure(),
			Expr::Binary(l, _, r) => l.pure() && r.pure(),
			Expr::Cond(c, a, b) => c.pure() && a.pure() && b.pure(),
			_ => false,
		}
	}
}

/// Each item `stmts` name as a value, to rename.
pub fn syms_mut(stmts: &mut [Stmt], f: &mut impl FnMut(&mut SymId)) {
	for stmt in stmts {
		match &mut stmt.kind {
			StmtKind::Expr(e) | StmtKind::Return(Some(e)) => e.syms_mut(f),
			StmtKind::Decl { init, .. } => init.iter_mut().for_each(|e| e.syms_mut(f)),
			StmtKind::If { cond, then, els } => {
				cond.syms_mut(f);
				syms_mut(then, f);
				syms_mut(els, f);
			}
			StmtKind::While { cond: e, body, .. } | StmtKind::ForOf { iter: e, body, .. } => {
				e.syms_mut(f);
				syms_mut(body, f);
			}
			StmtKind::Block { body, .. } => syms_mut(body, f),
			StmtKind::Switch {
				disc, cases, default, ..
			} => {
				disc.syms_mut(f);
				for (case, body) in cases {
					case.syms_mut(f);
					syms_mut(body, f);
				}
				syms_mut(default, f);
			}
			StmtKind::Return(None) | StmtKind::Break(_) | StmtKind::Continue(_) | StmtKind::Declare { .. } => {}
		}
	}
}

impl Expr {
	/// Each item the expression names as a value, to rename.
	pub fn syms_mut(&mut self, f: &mut impl FnMut(&mut SymId)) {
		match self {
			Expr::Sym(id) => f(id),
			Expr::Arrow(_, _, body) => syms_mut(body, f),
			Expr::Lambda(_, e)
			| Expr::Member(e, _)
			| Expr::Unary(_, e)
			| Expr::Postfix(e, _)
			| Expr::As(e, _)
			| Expr::Instantiate(e, _)
			| Expr::Located(_, e) => e.syms_mut(f),
			Expr::Array(es) => es.iter_mut().for_each(|e| e.syms_mut(f)),
			Expr::Object(props) => props.iter_mut().for_each(|(_, e)| e.syms_mut(f)),
			Expr::Index(a, b) | Expr::Binary(a, _, b) | Expr::Assign(a, b) => {
				a.syms_mut(f);
				b.syms_mut(f);
			}
			Expr::Call(callee, args) | Expr::New(callee, args) => {
				callee.syms_mut(f);
				args.iter_mut().for_each(|e| e.syms_mut(f));
			}
			Expr::Cond(a, b, c) => {
				a.syms_mut(f);
				b.syms_mut(f);
				c.syms_mut(f);
			}
			Expr::Ident(_) | Expr::This | Expr::Rs(_) | Expr::Lit(_) | Expr::Str(_) => {}
		}
	}
}

/// Mark `_name` each parameter that `body` never names, as `noUnusedParameters` wants,
/// and those of each function `body` makes: decided from the output, whatever reads
/// lowering left out.
pub fn mark_unread(params: &mut [Param], body: &mut [Stmt]) {
	let free = free_names(body);
	mark_params(params, &free, declared(body));
}

/// Mark `_name` each parameter not in `free`, the names its body reads from outside it,
/// choosing a name that is none of those, nothing the body `declared`, and no other
/// parameter.
fn mark_params(params: &mut [Param], free: &HashSet<String>, declared: HashSet<String>) {
	let mut taken = declared;
	taken.extend(free.iter().chain(params.iter().map(|p| &p.name)).cloned());
	for param in params {
		if !param.name.starts_with('_') && !free.contains(&param.name) {
			param.name = crate::program::unique(&mut taken, &format!("_{}", param.name));
		}
	}
}

/// The names `stmts` declare in their own block.
fn declared(stmts: &[Stmt]) -> HashSet<String> {
	(stmts.iter())
		.filter_map(|s| match &s.kind {
			StmtKind::Decl { name, .. } | StmtKind::Declare { name, .. } => Some(name.clone()),
			_ => None,
		})
		.collect()
}

/// The names `stmts` read or assign from outside them, marking the parameters of each
/// function they make.
fn free_names(stmts: &mut [Stmt]) -> HashSet<String> {
	let mut free = HashSet::new();
	for stmt in stmts.iter_mut() {
		match &mut stmt.kind {
			StmtKind::Expr(e) | StmtKind::Return(Some(e)) => e.free_names(&mut free),
			StmtKind::Decl { init, .. } => init.iter_mut().for_each(|e| e.free_names(&mut free)),
			StmtKind::If { cond, then, els } => {
				cond.free_names(&mut free);
				free.extend(free_names(then));
				free.extend(free_names(els));
			}
			StmtKind::While { cond: e, body, .. } => {
				e.free_names(&mut free);
				free.extend(free_names(body));
			}
			StmtKind::ForOf { name, iter, body, .. } => {
				iter.free_names(&mut free);
				let mut inner = free_names(body);
				inner.remove(name);
				free.extend(inner);
			}
			StmtKind::Block { body, .. } => free.extend(free_names(body)),
			StmtKind::Switch {
				disc, cases, default, ..
			} => {
				disc.free_names(&mut free);
				for (case, body) in cases {
					case.free_names(&mut free);
					free.extend(free_names(body));
				}
				free.extend(free_names(default));
			}
			StmtKind::Declare { .. } | StmtKind::Return(None) | StmtKind::Break(_) | StmtKind::Continue(_) => {}
		}
	}
	for name in declared(stmts) {
		free.remove(&name);
	}
	free
}

impl Expr {
	/// Mark the unread parameters of each function this makes, as [`mark_unread`] does.
	pub fn mark_unread(&mut self) {
		self.free_names(&mut HashSet::new());
	}

	/// Add the names the expression reads or assigns from outside it to `free`, marking
	/// the parameters of each function it makes.
	fn free_names(&mut self, free: &mut HashSet<String>) {
		match self {
			Expr::Ident(name) => {
				free.insert(name.clone());
			}
			Expr::Arrow(params, _, body) => {
				let inner = free_names(body);
				function_free(params, inner, declared(body), free);
			}
			Expr::Lambda(params, body) => {
				let mut inner = HashSet::new();
				body.free_names(&mut inner);
				function_free(params, inner, HashSet::new(), free);
			}
			Expr::Member(e, _)
			| Expr::Unary(_, e)
			| Expr::Postfix(e, _)
			| Expr::As(e, _)
			| Expr::Instantiate(e, _)
			| Expr::Located(_, e) => e.free_names(free),
			Expr::Array(es) => es.iter_mut().for_each(|e| e.free_names(free)),
			Expr::Object(props) => props.iter_mut().for_each(|(_, e)| e.free_names(free)),
			Expr::Index(a, b) | Expr::Binary(a, _, b) | Expr::Assign(a, b) => {
				a.free_names(free);
				b.free_names(free);
			}
			Expr::Call(callee, args) | Expr::New(callee, args) => {
				callee.free_names(free);
				args.iter_mut().for_each(|e| e.free_names(free));
			}
			Expr::Cond(a, b, c) => {
				a.free_names(free);
				b.free_names(free);
				c.free_names(free);
			}
			Expr::This | Expr::Sym(_) | Expr::Rs(_) | Expr::Lit(_) | Expr::Str(_) => {}
		}
	}
}

/// Mark the parameters of a function made in an expression, whose body reads `inner` from
/// outside it and `declared` its own, adding to `free` what it reads from outside itself.
fn function_free(params: &mut [Param], inner: HashSet<String>, declared: HashSet<String>, free: &mut HashSet<String>) {
	mark_params(params, &inner, declared);
	free.extend(inner.into_iter().filter(|name| params.iter().all(|p| p.name != *name)));
}

impl Expr {
	/// Each item the expression reads as it is evaluated, outside the functions it makes.
	pub fn eager_syms(&self, f: &mut impl FnMut(SymId)) {
		match self {
			Expr::Sym(id) => f(*id),
			Expr::Arrow(..) | Expr::Lambda(..) => {}
			Expr::Member(e, _)
			| Expr::Unary(_, e)
			| Expr::Postfix(e, _)
			| Expr::As(e, _)
			| Expr::Instantiate(e, _)
			| Expr::Located(_, e) => e.eager_syms(f),
			Expr::Array(es) => es.iter().for_each(|e| e.eager_syms(f)),
			Expr::Object(props) => props.iter().for_each(|(_, e)| e.eager_syms(f)),
			Expr::Index(a, b) | Expr::Binary(a, _, b) | Expr::Assign(a, b) => {
				a.eager_syms(f);
				b.eager_syms(f);
			}
			Expr::Call(callee, args) | Expr::New(callee, args) => {
				callee.eager_syms(f);
				args.iter().for_each(|e| e.eager_syms(f));
			}
			Expr::Cond(a, b, c) => {
				a.eager_syms(f);
				b.eager_syms(f);
				c.eager_syms(f);
			}
			Expr::Ident(_) | Expr::This | Expr::Rs(_) | Expr::Lit(_) | Expr::Str(_) => {}
		}
	}
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type {
	/// A built-in type: `number`, `bigint`, `boolean`, `string`, `void`, `never`,
	/// `undefined`, `Uint8Array`.
	Builtin(&'static str),
	/// A runtime type: `rs.<name><args>`.
	Rs(&'static str, Vec<Type>),
	Sym(SymId, Vec<Type>),
	Param(String),
	Union(Vec<Type>),
	Tuple(Vec<Type>),
	Array(Box<Type>),
	Func(Vec<Type>, Box<Type>),
	/// A string literal type.
	StrLit(String),
	/// An object type; a `Some` key is a class field.
	Object(Vec<(String, Type)>),
	/// In a generic type's arguments, the projection arguments of an instance of a type
	/// whose definition is still being typed, which [`Type::fill`] splices in once it is.
	Later(usize),
}

impl Type {
	/// Whether it names the type parameter `param`.
	pub fn names(&self, param: &str) -> bool {
		self.any(&|t| matches!(t, Type::Param(name) if name == param))
	}

	/// Whether a type `f` picks is where TypeScript takes no other: in a function type's
	/// parameter, or in a type argument of a generic type.
	pub fn takes(&self, f: &impl Fn(&Type) -> bool) -> bool {
		match self {
			Type::Func(params, ret) => params.iter().any(|t| t.any(f)) || ret.takes(f),
			Type::Param(_) | Type::Builtin(_) | Type::StrLit(_) | Type::Later(_) => false,
			Type::Rs(_, tys) | Type::Sym(_, tys) if !f(self) => tys.iter().any(|t| t.any(f)),
			Type::Rs(_, tys) | Type::Sym(_, tys) | Type::Union(tys) | Type::Tuple(tys) => {
				tys.iter().any(|t| t.takes(f))
			}
			Type::Array(t) => t.takes(f),
			Type::Object(fields) => fields.iter().any(|(_, t)| t.takes(f)),
		}
	}

	/// It with each type parameter `f` gives a type for replaced.
	pub fn subst(&self, f: &impl Fn(&str) -> Option<Type>) -> Type {
		let all = |tys: &[Type]| tys.iter().map(|t| t.subst(f)).collect();
		match self {
			Type::Param(name) => f(name).unwrap_or_else(|| self.clone()),
			Type::Builtin(_) | Type::StrLit(_) | Type::Later(_) => self.clone(),
			Type::Rs(name, tys) => Type::Rs(name, all(tys)),
			Type::Sym(sym, tys) => Type::Sym(*sym, all(tys)),
			Type::Union(tys) => Type::Union(all(tys)),
			Type::Tuple(tys) => Type::Tuple(all(tys)),
			Type::Array(t) => Type::Array(Box::new(t.subst(f))),
			Type::Func(tys, ret) => Type::Func(all(tys), Box::new(ret.subst(f))),
			Type::Object(fields) => Type::Object(fields.iter().map(|(k, t)| (k.clone(), t.subst(f))).collect()),
		}
	}

	/// Whether it, or any type in it, is one `f` picks.
	pub fn any(&self, f: &impl Fn(&Type) -> bool) -> bool {
		f(self)
			|| match self {
				Type::Param(_) | Type::Builtin(_) | Type::StrLit(_) | Type::Later(_) => false,
				Type::Rs(_, tys) | Type::Sym(_, tys) | Type::Union(tys) | Type::Tuple(tys) => {
					tys.iter().any(|t| t.any(f))
				}
				Type::Array(t) => t.any(f),
				Type::Func(tys, ret) => ret.any(f) || tys.iter().any(|t| t.any(f)),
				Type::Object(fields) => fields.iter().any(|(_, t)| t.any(f)),
			}
	}

	/// It with each [`Type::Later`] replaced by the arguments `later` gives it, or `None`
	/// if one reaches itself.
	pub fn fill(&mut self, later: &impl Fn(usize) -> Vec<Type>) -> Option<()> {
		self.fill_from(later, &mut Vec::new())
	}

	fn fill_from(&mut self, later: &impl Fn(usize) -> Vec<Type>, open: &mut Vec<usize>) -> Option<()> {
		match self {
			Type::Param(_) | Type::Builtin(_) | Type::StrLit(_) => Some(()),
			Type::Later(_) => unreachable!("projection arguments outside a generic type's arguments"),
			Type::Rs(_, tys) | Type::Union(tys) | Type::Tuple(tys) => {
				tys.iter_mut().try_for_each(|t| t.fill_from(later, open))
			}
			Type::Sym(_, tys) => {
				let mut filled = Vec::with_capacity(tys.len());
				for mut t in std::mem::take(tys) {
					let Type::Later(id) = t else {
						t.fill_from(later, open)?;
						filled.push(t);
						continue;
					};
					if open.contains(&id) {
						return None;
					}
					open.push(id);
					for mut t in later(id) {
						t.fill_from(later, open)?;
						filled.push(t);
					}
					open.pop();
				}
				*tys = filled;
				Some(())
			}
			Type::Array(t) => t.fill_from(later, open),
			Type::Func(tys, ret) => {
				tys.iter_mut().try_for_each(|t| t.fill_from(later, open))?;
				ret.fill_from(later, open)
			}
			Type::Object(fields) => fields.iter_mut().try_for_each(|(_, t)| t.fill_from(later, open)),
		}
	}
}

#[derive(Clone, Debug)]
pub struct Function {
	pub sym: SymId,
	pub params: Vec<Param>,
	pub ret: Type,
	pub body: Vec<Stmt>,
	pub loc: Option<Loc>,
	pub doc: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Field {
	pub id: FieldId,
	pub ty: Type,
	pub doc: Option<String>,
	/// What the field starts as, if not a constructor argument.
	pub init: FieldInit,
}

#[derive(Clone, Debug)]
pub enum FieldInit {
	/// The constructor's argument.
	Ctor,
	/// `name: ty = value`.
	Value(Expr),
	/// `name!: ty`: set before every read, in ways `tsc` can't follow.
	Later,
}

/// How a struct is constructed, as a class; a plain object is a literal of its fields.
#[derive(Clone, Debug)]
pub enum Ctor {
	/// `new C()`, for a struct without fields.
	Unit,
	/// `new C(a, b)`: its fields in declaration order, which allocates no object for them.
	Positional,
}

#[derive(Clone, Debug)]
pub enum Item {
	/// A struct: an interface, or a class if it may drop.
	Struct {
		sym: SymId,
		params: Vec<String>,
		fields: Vec<Field>,
		ctor: Ctor,
		doc: Option<String>,
	},
	/// A type alias.
	Union {
		sym: SymId,
		params: Vec<String>,
		ty: Type,
		doc: Option<String>,
	},
	Function(Function),
	/// A module's value that every use shares, made with no effect but itself, so that a
	/// bundler drops it unused: `export const NAME: T = /* @__PURE__ */ new ...`, or a
	/// literal.
	Value {
		sym: SymId,
		ty: Type,
		value: Expr,
		doc: Option<String>,
	},
}
