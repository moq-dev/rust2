//! The exported API, as each generator reads it: what the exported crates' `pub` items
//! became, with no language's names or calling convention decided yet.
//!
//! Every item keeps its Rust path, so a generator names it by its own rule, and its span,
//! so a generator refuses it with one.

use rustc_span::Span;

/// The exported crates' API.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Api {
	pub records: Vec<Record>,
	pub enums: Vec<Enum>,
	pub objects: Vec<Object>,
	pub callbacks: Vec<Callback>,
	pub functions: Vec<Function>,
	pub constants: Vec<Constant>,
	/// The crates items belong to, each after every crate it depends on.
	pub crates: Vec<String>,
	/// The crates the facade does not re-export whose types an export names, which the
	/// glue depends on directly. An API from before it reached none.
	#[serde(default)]
	pub reached: Vec<Reached>,
	/// The targets it serves, sorted: one for a single target's, several merged.
	pub targets: Vec<String>,
}

/// A crate the glue depends on beside the facade, as the facade resolves it.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Reached {
	/// Its crate name, its items' second path segment.
	pub krate: String,
	/// The targets whose API names its types, sorted.
	pub targets: Vec<String>,
}

impl Api {
	/// Whether a value of `ty` holds a float anywhere, its records' and enums' included, so a
	/// language comparing it itself must use IEEE `==`, as Rust does: NaN unequal even to
	/// itself, and -0.0 equal to 0.0.
	pub fn holds_float(&self, ty: &Type) -> bool {
		self.floats(ty, &mut std::collections::HashSet::new())
	}

	fn floats(&self, ty: &Type, seen: &mut std::collections::HashSet<(bool, usize)>) -> bool {
		match ty {
			Type::F32 | Type::F64 => true,
			Type::Option(inner) | Type::Box(inner) | Type::Ref(_, inner) | Type::List(_, inner) => {
				self.floats(inner, seen)
			}
			Type::Tuple(elements) => elements.iter().any(|e| self.floats(e, seen)),
			// A type already seen is being looked through, so it adds nothing new.
			Type::Record(i) => {
				seen.insert((false, *i)) && self.records[*i].fields.iter().any(|f| self.floats(&f.ty, seen))
			}
			Type::Enum(i) => {
				seen.insert((true, *i))
					&& (self.enums[*i].variants.iter())
						.flat_map(|v| &v.fields)
						.any(|f| self.floats(&f.ty, seen))
			}
			_ => false,
		}
	}

	/// Whether an item on `on` is on every target the API serves.
	pub fn everywhere(&self, on: On) -> bool {
		on.0 & self.all().0 == self.all().0
	}

	/// Every target the API serves.
	pub fn all(&self) -> On {
		On(u64::MAX >> (64 - self.targets.len()))
	}

	/// The targets of `on`, by name.
	pub fn named(&self, on: On) -> Vec<&str> {
		(self.targets.iter().enumerate())
			.filter(|(i, _)| on.0 >> i & 1 == 1)
			.map(|(_, target)| target.as_str())
			.collect()
	}

	/// A type's Rust path with its instance's arguments, such as
	/// `moq_mux::catalog::Producer<moq_mux::catalog::hang::Extra>`.
	pub fn rust(&self, path: &Path, args: &[Type]) -> String {
		let path = path.join("::");
		match args {
			[] => path,
			args => {
				let args: Vec<String> = args.iter().map(|arg| self.arg(arg)).collect();
				format!("{path}<{}>", args.join(", "))
			}
		}
	}

	/// A type argument, as [`Self::rust`] spells it.
	fn arg(&self, ty: &Type) -> String {
		match ty {
			Type::Unit => "()".to_string(),
			Type::Bool => "bool".to_string(),
			Type::Int(int) => int.rust().to_string(),
			Type::NonZero(int) => format!("std::num::NonZero<{}>", int.rust()),
			Type::F32 => "f32".to_string(),
			Type::F64 => "f64".to_string(),
			Type::Char => "char".to_string(),
			Type::String => "String".to_string(),
			Type::Bytes => "bytes::Bytes".to_string(),
			Type::Duration => "std::time::Duration".to_string(),
			Type::Record(i) => self.rust(&self.records[*i].path, &self.records[*i].args),
			Type::Enum(i) => self.rust(&self.enums[*i].path, &self.enums[*i].args),
			Type::Object(i) => self.rust(&self.objects[*i].path, &self.objects[*i].args),
			Type::Const(value) => value.to_string(),
			other => unreachable!("no type argument is {other:?}"),
		}
	}
}

/// The targets an item compiles on, as a bit for each index into [`Api::targets`]: at
/// most 64 targets.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub struct On(pub u64);

impl On {
	/// On every target, as a single target's API is.
	pub const ALL: On = On(u64::MAX);
}

/// A Rust path: its export's package name in snake case, unique among the exports, then
/// the path from that crate's root, such as `["moq_net", "track", "Producer", "append_group"]`.
pub type Path = Vec<String>;

/// The crate an item belongs to, and how much of its path lies outside that crate.
#[derive(Clone, Default, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct Home {
	/// The crate's name, followed by `#` and a number for a later crate of a name an
	/// earlier one has: the one defining the item, or for an instance, the last of its own
	/// and its arguments' crates in [`Api::crates`], so a crate's items name only types of
	/// crates before it.
	pub krate: String,
	/// How many leading segments of its `shown` path, the facade's root among them, name
	/// no module of that crate: the facade's own modules.
	pub outside: usize,
}

/// A struct whose fields are all `pub`, or a `Copy` type C cannot see inside: a value,
/// copied across.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Record {
	pub path: Path,
	pub home: Home,
	/// Its path with its instance's arguments, which the config names it by, such as
	/// `moq_mux::catalog::Producer<moq_mux::catalog::hang::Extra>`.
	pub shown: Path,
	/// The type and const arguments of the instance it is, empty unless the type is generic.
	pub args: Vec<Type>,
	#[serde(with = "crate::merge::span")]
	pub span: Span,
	/// The targets it compiles on.
	pub on: On,
	pub docs: String,
	pub fields: Vec<Field>,
	/// Whether its fields are positional, named `0`, `1`, ...
	pub tuple: bool,
	pub methods: Vec<Function>,
	pub traits: Traits,
	/// For a `Copy` type with private fields, which has no `fields`: its bytes, which only
	/// Rust reads or writes, its `pub` fields through accessors among its `methods`.
	pub opaque: Option<Opaque>,
	/// For an opaque value, its padding on each target, by target, as `(offset, length)`
	/// ranges, which the glue zeroes, so no byte Rust leaves uninitialized reaches a
	/// language. A baseline written before it existed has none, as its records had.
	#[serde(default)]
	pub padding: std::collections::BTreeMap<String, Vec<(u64, u64)>>,
}

/// A `Copy` type held as its bytes, which every language copies and none reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct Opaque {
	/// Its size and alignment: on several targets, the largest of each, so one binding
	/// holds it on all of them.
	pub size: u64,
	pub align: u64,
	/// Whether its bytes are its fields' with no padding between them, each an integer, a
	/// `bool`, or a `char`, and each struct's `PartialEq` in it, if any, derived or std's,
	/// so two values Rust calls equal have equal bytes: on several targets, on all of them.
	pub dense: bool,
	/// How many lifetime parameters it has, each `'static` in the export, as an object's.
	pub lifetimes: usize,
	/// Whether it is a foreign type the facade maps, such as `std::time::Instant`: its path
	/// is the type's in its own crate.
	pub foreign: bool,
}

impl Opaque {
	/// Whether it fits a 64-bit word, which a language may hold it in with no allocation.
	pub fn word(&self) -> bool {
		self.size <= 8
	}

	/// Whether a language may hold it in a word and compare that word: it fits one and is
	/// dense, so values Rust calls equal have equal bits. Kotlin's type for it turns on
	/// this, so a change of it breaks Kotlin's compiled callers.
	pub fn compared(&self) -> bool {
		self.word() && self.dense
	}

	/// The bytes of each unit its C struct holds it in, by its alignment, at most 8.
	pub fn unit(&self) -> u64 {
		self.align.clamp(1, 8)
	}

	/// How many units its C struct holds: at least one, as C has no empty struct.
	pub fn units(&self) -> u64 {
		self.size.div_ceil(self.unit()).max(1)
	}

	/// How many 64-bit words hold its C struct.
	pub fn words(&self) -> u64 {
		(self.units() * self.unit()).div_ceil(8)
	}
}

#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Field {
	/// Its Rust name: a number for a positional field.
	pub name: String,
	pub docs: String,
	pub ty: Type,
}

/// An enum: a value, copied across. Without fields in any variant, it is a plain
/// enumeration.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Enum {
	pub path: Path,
	pub home: Home,
	pub shown: Path,
	pub args: Vec<Type>,
	#[serde(with = "crate::merge::span")]
	pub span: Span,
	/// The targets it compiles on.
	pub on: On,
	pub docs: String,
	pub variants: Vec<Variant>,
	/// Whether another crate can match it without a wildcard arm.
	pub exhaustive: bool,
	pub methods: Vec<Function>,
	pub traits: Traits,
	/// Whether it is a foreign type the facade maps rather than exports, such as
	/// `tracing::Level`: its path is the type's in its own crate, and each variant is the
	/// type's constant of that name in upper snake case.
	pub foreign: bool,
}

#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Variant {
	pub name: String,
	/// Its discriminant, for an enum without fields.
	pub discriminant: i128,
	pub fields: Vec<Field>,
	pub tuple: bool,
	/// The targets it compiles on.
	pub on: On,
}

impl Enum {
	/// Whether no variant has fields.
	pub fn plain(&self) -> bool {
		self.variants.iter().all(|v| v.fields.is_empty())
	}
}

/// Any other type: a handle to a value the foreign side never sees inside, shared behind
/// an `Arc`, whose receivers borrow it as Rust's do, checked at run time.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Object {
	pub path: Path,
	pub home: Home,
	pub shown: Path,
	pub args: Vec<Type>,
	#[serde(with = "crate::merge::span")]
	pub span: Span,
	/// The targets it compiles on.
	pub on: On,
	pub docs: String,
	/// Whether it is `Sync`: if not, `&self` borrows it on one thread at a time.
	pub sync: bool,
	/// Whether it is `Copy`, which only one whose padding depends on its value is, any
	/// other being a value: taken by value, it is copied out of its handle, as Rust copies
	/// it, rather than moved out.
	pub copy: bool,
	/// Whether copies of its handle may share one value, since no call tells that apart
	/// from `Clone` but by an `Arc`'s or `Rc`'s count: it is `Clone` and `Sync`, its clone owns no state that changes
	/// behind `&` or counts clones, and no export borrows it mutably.
	pub shared: bool,
	/// How many lifetime parameters it has, each `'static` in the export.
	pub lifetimes: usize,
	pub methods: Vec<Function>,
	pub traits: Traits,
	/// Whether it is a foreign type, such as `url::Url`, that the facade maps rather than
	/// exports: its path is the type's in its own crate, not in the facade.
	pub foreign: bool,
	/// What its handle holds of another crate's type, held erased, which the glue never
	/// names: a handle with none of its API. A baseline written before the by-value form
	/// has a `bool`, the `Arc` form's.
	#[serde(deserialize_with = "held")]
	pub opaque: Option<Held>,
}

/// An [`Object::opaque`], read from a baseline written before the by-value form as the
/// `bool` the `Arc` form was.
fn held<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Option<Held>, D::Error> {
	#[derive(serde::Deserialize)]
	#[serde(untagged)]
	enum Either {
		Arc(bool),
		Held(Option<Held>),
	}
	Ok(match serde::Deserialize::deserialize(de)? {
		Either::Arc(arc) => arc.then_some(Held::Arc),
		Either::Held(held) => held,
	})
}

/// What an opaque handle holds of another crate's type.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Held {
	/// An `Arc` of it, which Rust takes back as the `Arc`, and clones share.
	Arc,
	/// The value itself, which Rust takes back by value or borrows.
	Value,
}

/// A `pub` trait: implemented by the foreign side, through a table of functions.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Callback {
	pub path: Path,
	pub home: Home,
	#[serde(with = "crate::merge::span")]
	pub span: Span,
	/// The targets it compiles on.
	pub on: On,
	pub docs: String,
	/// Its required methods, each taking `&self` or `&mut self`.
	pub methods: Vec<Function>,
}

/// The std traits a type's impls export.
#[derive(Default, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Traits {
	pub clone: bool,
	pub eq: bool,
	pub display: bool,
	pub debug: bool,
	pub default: bool,
}

/// A function or method.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Function {
	/// Its path, ending in its own name: a method's goes through its type's.
	pub path: Path,
	/// Its path with the arguments of its type's instance and its own, which the config
	/// names it by: `moq_mux::catalog::Producer<moq_mux::catalog::hang::Extra>::stats`.
	pub shown: Path,
	#[serde(with = "crate::merge::span")]
	pub span: Span,
	/// The targets it compiles on.
	pub on: On,
	/// Its doc comment, as Markdown.
	pub docs: String,
	pub receiver: Option<Receiver>,
	pub params: Vec<Param>,
	/// Where among Rust's parameters each `()` one sits, which C does not pass.
	pub units: Vec<usize>,
	/// What it returns, or for a future, what the future does.
	pub ok: Type,
	/// The error of a `Result` it returns: `String` for another crate's error, as its message.
	pub err: Option<Type>,
	/// Whether it returns a future: an `async fn`, or a value that implements `Future`.
	pub future: bool,
	/// For a future, whether the call itself runs code, as a function that is not
	/// `async` returning one does: the export calls it at once, and polls what it returns.
	pub eager: bool,
	/// For an eager future, whether what it returns borrows its arguments: the export owns
	/// them, as it does an `async fn`'s, and keeps them until the future is dropped. A
	/// baseline written before it has none.
	#[serde(default)]
	pub holds: bool,
	/// The parameter it polls its task through, if any: C passes a waker for it or, for a
	/// future made of a returned `Poll`, the future's poll supplies it.
	pub task: Option<Task>,
	/// Whether it returns another crate's iterator, whose items the export collects into
	/// the array it returns.
	pub collect: bool,
	/// Whether it returns a borrow of an object that is not `Clone`, which C holds as a handle
	/// lent from its receiver's: shared, until the receiver's value is borrowed
	/// exclusively, moved out, or freed.
	pub lends: bool,
	/// What it reads: Rust's function, a field it reads or writes, or a constant named as it is.
	pub access: Access,
	/// The instance it is, if a type parameter exports at several.
	pub instance: Option<Instance>,
	/// Where it belongs: a method with its type, unless its instance names a later crate's
	/// type, which it then belongs with.
	pub home: Home,
}

/// An instance of a generic function or method, at one of the arguments a type or const
/// parameter exports at.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Instance {
	/// The arguments of its declared type and const parameters, which the call names, as
	/// Rust cannot infer one no argument names; its argument infers an `impl Trait` one,
	/// and one left `None`, which takes a lent closure or a foreign implementation.
	pub args: Vec<Option<Type>>,
	/// The arguments that tell it from its other instances, which name it.
	pub named: Vec<Type>,
}

/// What an exported function reads.
#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Access {
	/// It calls the function.
	Call,
	/// It reads a `pub` field of its receiver, or with `set` assigns it. `shared` when a
	/// method or constant of its type takes its name, which its path then names, so the
	/// config names the field `field@<path>`, as rustdoc does.
	Field { shared: bool, set: bool },
	/// It reads a constant that is not a `bool` or integer.
	Const,
	/// It parses its receiver's type from text, with `FromStr`, failing with the parse
	/// error's message: a foreign type exported as an object.
	Parse,
	/// It writes its receiver as text, with `Display`: a foreign type exported as an object.
	Text,
}

impl Function {
	pub fn name(&self) -> &str {
		self.path.last().expect("a function has a name")
	}
}

/// A parameter a function polls its task through.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Task {
	/// Its place among the parameters after `self`.
	pub at: usize,
	pub kind: TaskKind,
}

/// What a [`Task`] parameter is.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum TaskKind {
	/// `&Context` or `&mut Context`: made from C's waker, or the future's own.
	Context(Mutability),
	/// A reference to an object made from a `Waker` by its associated function `make`, which
	/// the future makes from its waker at each poll and keeps until the next.
	Made { object: usize, make: String },
}

/// How a method takes `self`.
#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Receiver {
	/// `&self`
	Ref,
	/// `&mut self`
	Mut,
	/// `self`
	Owned,
}

#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Param {
	pub name: String,
	pub ty: Type,
	/// Whether it is a `&str` that must be `'static`, which the glue interns: it leaks each
	/// distinct string once and lends that copy to every later call with equal content.
	pub interned: bool,
}

/// A `pub const`.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Constant {
	pub path: Path,
	pub home: Home,
	#[serde(with = "crate::merge::span")]
	pub span: Span,
	/// The targets it compiles on.
	pub on: On,
	pub docs: String,
	pub value: Literal,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum Literal {
	Bool(bool),
	Int(Int, i128),
}

impl std::fmt::Display for Literal {
	/// The value as Rust writes it.
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		match self {
			Literal::Bool(b) => write!(f, "{b}"),
			Literal::Int(_, v) => write!(f, "{v}"),
		}
	}
}

/// A type as the API uses it, in the exact form the Rust side takes or returns.
#[derive(Clone, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum Type {
	Unit,
	Bool,
	Int(Int),
	/// `std::num::NonZero<T>` of an integer, which crosses as `T`: a zero passed in fails
	/// the binding's check.
	NonZero(Int),
	/// `u128`, which C passes as its two `u64` halves.
	U128,
	F32,
	F64,
	Char,
	/// `str`, only behind a reference.
	Str,
	String,
	/// `[u8]`, only behind a reference.
	Slice,
	/// `Vec<u8>`
	Vec,
	/// `bytes::Bytes`
	Bytes,
	/// `std::time::Duration`
	Duration,
	/// `[u8; N]`, which C passes by value in a struct of its `N` bytes.
	Array(usize),
	/// `std::path::Path`, only behind a reference.
	Path,
	/// `std::path::PathBuf`
	PathBuf,
	/// `std::net::SocketAddr`, or `SocketAddrV4` or `SocketAddrV6` for one family.
	SocketAddr(Family),
	/// A collection of `T`, which C passes and returns as an array of its items: `Vec<T>` of
	/// any `T` but `u8`, which is [`Type::Vec`], and behind a reference, `[T]` too; a boxed
	/// slice; a set; or a map, whose `T` is the tuple of a key and its value.
	List(Collection, Box<Type>),
	/// `std::task::Waker`, only as a parameter taken by value or a return: a returned
	/// `&Waker` is its clone, and a returned `Context` its waker's.
	Waker,
	/// A future the foreign side implements, of `()`, which Rust polls and frees: only as a
	/// parameter, which Rust takes as `Pin<Box<dyn Future<Output = ()> + Send>>`. A
	/// function returning a `Poll` takes it as `Pin<&mut _>`, which its future owns and
	/// lends to each poll.
	Future,
	/// A reader the foreign side implements, `io::Read` and `io::Seek`: only behind `&mut`
	/// as a parameter, lent for the call.
	Reader,
	Option(Box<Type>),
	/// `Box<T>`, of a non-trait type.
	Box(Box<Type>),
	Ref(Mutability, Box<Type>),
	Record(usize),
	Enum(usize),
	Object(usize),
	/// A foreign implementation of a callback trait, in the form the parameter takes.
	Callback(usize, Dyn),
	/// A closure the foreign side implements as a function and its data.
	Closure(Box<Closure>),
	/// `std::ops::Range` of an integer.
	Range(Int),
	/// `std::ops::Bound` of an integer.
	Bound(Int),
	/// `(Bound<T>, Bound<T>)` of an integer, the form a `RangeBounds<T>` parameter takes.
	Bounds(Int),
	/// A tuple of one element or more.
	Tuple(Vec<Type>),
	/// A const argument of an instance, such as the `8` of `Buffer<8>`.
	Const(Literal),
}

/// A [`Type::Closure`]: what it takes and returns.
#[derive(Clone, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub struct Closure {
	/// Each value it takes, which Rust lends for the call: a value or a borrowed object.
	pub params: Vec<Type>,
	/// What it returns, or for a `Result`, its value: a value or a `String`, which the
	/// foreign side hands Rust.
	pub ok: Type,
	/// The error of a `Result` it returns: a `String`.
	pub err: Option<Type>,
	/// Whether Rust keeps it past the call, calling it from any thread until it frees it,
	/// rather than only during the call.
	pub kept: bool,
}

/// The Rust collection a [`Type::List`] is, which Rust builds from its items in order.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum Collection {
	/// `Vec<T>`
	Vec,
	/// `[T]`, only behind a reference.
	Slice,
	/// `Box<[T]>`
	Box,
	/// `std::collections::BTreeSet<T>`
	BTreeSet,
	/// `std::collections::BTreeMap<K, V>`
	BTreeMap,
	/// `serde_json::Map`, of `String` keys and `serde_json::Value`s.
	JsonMap,
}

/// The addresses a socket address type holds.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum Family {
	/// IPv4 or IPv6: `SocketAddr`.
	Any,
	V4,
	V6,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum Mutability {
	Not,
	Mut,
}

/// How a parameter takes a foreign implementation of a trait.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum Dyn {
	/// A type parameter or `impl Trait`, given the foreign type.
	Impl,
	/// `Box<dyn Trait>`
	Box,
	/// `Arc<dyn Trait>`
	Arc,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, serde::Serialize, serde::Deserialize)]
pub enum Int {
	U8,
	U16,
	U32,
	U64,
	Usize,
	I8,
	I16,
	I32,
	I64,
	Isize,
}

impl Int {
	/// Its Rust name.
	pub fn rust(self) -> &'static str {
		match self {
			Int::U8 => "u8",
			Int::U16 => "u16",
			Int::U32 => "u32",
			Int::U64 => "u64",
			Int::Usize => "usize",
			Int::I8 => "i8",
			Int::I16 => "i16",
			Int::I32 => "i32",
			Int::I64 => "i64",
			Int::Isize => "isize",
		}
	}
}

/// Call `f` on `ty` and on every type within it.
pub fn each(ty: &Type, f: &mut dyn FnMut(&Type)) {
	f(ty);
	match ty {
		Type::List(_, inner) | Type::Option(inner) | Type::Box(inner) | Type::Ref(_, inner) => each(inner, f),
		Type::Closure(closure) => {
			for param in &closure.params {
				each(param, f);
			}
			each(&closure.ok, f);
			if let Some(err) = &closure.err {
				each(err, f);
			}
		}
		Type::Tuple(elements) => {
			for element in elements {
				each(element, f);
			}
		}
		_ => {}
	}
}

impl Type {
	/// Whether a parameter of this type is a value C passes in that Rust writes back
	/// through an out pointer: `&mut` of anything but an object, whose handle it borrows,
	/// or a reader, which C implements.
	pub fn written(&self) -> bool {
		matches!(self, Type::Ref(Mutability::Mut, inner) if !matches!(**inner, Type::Object(_) | Type::Reader))
	}

	/// The type behind any reference or `Box`.
	pub fn base(&self) -> &Type {
		match self {
			Type::Ref(_, inner) | Type::Box(inner) => inner.base(),
			ty => ty,
		}
	}

	/// The element of a list that crosses as one block of its C values, which a binding
	/// holds as the language's primitive array: a `Vec`, slice, or boxed slice of a
	/// fixed-width integer or a float. `usize` and `isize` differ in width by target, so no
	/// one array type holds them on every target.
	pub fn block(&self) -> Option<&Type> {
		match self {
			Type::List(Collection::Vec | Collection::Slice | Collection::Box, inner) => match &**inner {
				Type::Int(Int::Usize | Int::Isize) => None,
				Type::Int(_) | Type::F32 | Type::F64 => Some(inner),
				_ => None,
			},
			_ => None,
		}
	}

	/// Whether this type holds a block anywhere but in a record or enum, which compare their own.
	pub fn holds_block(&self) -> bool {
		match self {
			_ if self.block().is_some() => true,
			Type::Option(inner) | Type::Box(inner) | Type::Ref(_, inner) | Type::List(_, inner) => inner.holds_block(),
			Type::Tuple(elements) => elements.iter().any(Type::holds_block),
			_ => false,
		}
	}

	/// What the buffer C passes for this type holds, in whatever form Rust takes it: a
	/// string, bytes, or a path, or an `Option` of one, which C passes as a null buffer.
	pub fn buffer(&self) -> Option<Type> {
		match self.base() {
			Type::Str | Type::String => Some(Type::String),
			Type::Slice | Type::Vec | Type::Bytes => Some(Type::Bytes),
			Type::Path | Type::PathBuf => Some(Type::PathBuf),
			Type::Option(inner) => inner.buffer().map(|buffer| Type::Option(Box::new(buffer))),
			_ => None,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[derive(serde::Deserialize)]
	struct Opaque {
		#[serde(deserialize_with = "held")]
		opaque: Option<Held>,
	}

	#[test]
	fn held_reads_an_earlier_baseline() {
		let read = |json: &str| serde_json::from_str::<Opaque>(json).unwrap().opaque;
		assert_eq!(read(r#"{"opaque":true}"#), Some(Held::Arc));
		assert_eq!(read(r#"{"opaque":false}"#), None);
		assert_eq!(read(r#"{"opaque":"Value"}"#), Some(Held::Value));
		assert_eq!(read(r#"{"opaque":null}"#), None);
	}
}
