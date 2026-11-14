//! The C ABI the IR becomes: every C name, and every function's C signature, decided
//! once so the header and the Rust side agree.
//!
//! Names follow one rule: the prefix, then the item's path from the facade's root, each
//! segment in Rust's case, joined by `_`; a method's path goes through its type's, and a
//! field's getter and setter add `_get` and `_set` to it. Case keeps a type apart from a
//! function or module of its name, and Rust's verb-first names keep a method apart from a
//! field's accessors. Joining still loses where segments split (`audio_encoder::finish`
//! and `audio::encoder_finish` both join as `moq_audio_encoder_finish`), so every name is
//! registered, and of the items whose names clash, every one but the first by rank takes a
//! trailing `_`, as do the names made from it: `moq_audio_encoder_finish` and
//! `moq_audio_encoder_finish_`. A `_` doubled at the split would read better, but C++
//! reserves every name containing `__`. Only the names that clash change.

use std::collections::{BTreeMap, HashMap};

use rustc_span::Span;

use crate::ir::{Access, Api, Closure, Dyn, Function, Home, Int, Literal, On, Receiver, Type};

/// A C name one item claims twice, which no rename can tell apart: the name, then what
/// claimed it first and second.
pub type Clash = (String, (String, Span), (String, Span));

/// What a C name names, which a clash renames: a type, a function by its address in the
/// API, a constant by its path, or a tuple by its elements.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum Item {
	Record(usize),
	Enum(usize),
	Object(usize),
	Callback(usize),
	Function(usize),
	Constant(Vec<String>),
	Tuple(Vec<Type>),
}

/// The item a claimed name belongs to: its own C name, and whether the claimed name is
/// that one, unrenamed, rather than one made from it.
struct Owner {
	item: Item,
	base: String,
	own: bool,
}

/// A C name claimed, by what, and the item a clash renames, or none for the runtime's.
struct Claim {
	name: String,
	what: String,
	span: Span,
	owner: Option<Owner>,
}

/// A C type, as a parameter, field, or return value takes it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Ty {
	Void,
	Bool,
	Int(Int),
	F32,
	F64,
	/// A `char`, as its code point.
	Char,
	/// `<prefix>_buf`: a view of bytes passed in, an owned buffer returned.
	Buf,
	/// `<prefix>_u128`
	U128,
	/// `<prefix>_duration`
	Duration,
	/// `<prefix>_socket_addr`
	SocketAddr,
	/// A struct: a record, a data enum, an option, or a callback's table and data.
	Struct(String),
	/// A plain enum, passed as its C enum, which is an `int`.
	Enum(String),
	/// A handle: `const` when only borrowed.
	Handle {
		name: String,
		constant: bool,
	},
	/// A future's handle.
	Future(String),
	/// `<prefix>_waker`
	Waker,
	/// `<prefix>_foreign_future`: a future C implements, which Rust polls and frees.
	ForeignFuture,
	/// `<prefix>_foreign_reader`: a reader C implements, which Rust reads and seeks.
	ForeignReader,
	/// `<prefix>_poll`
	Poll,
	/// A pointer the callee writes a result through.
	Out(Box<Ty>),
	/// A pointer the callee only reads through: a value's receiver.
	In(Box<Ty>),
	/// A pointer the callee reads and writes back through: a value's `&mut self`.
	InOut(Box<Ty>),
	/// A function lent for the call, taking its data first: what it takes and returns.
	Fn(Vec<Ty>, Box<Ty>),
	/// The data a lent function takes, which Rust only passes back to it.
	Data,
}

/// A C function: its name, parameters, and return type.
pub struct Sig {
	pub name: String,
	pub params: Vec<(String, Ty)>,
	pub ret: Ty,
}

/// The C side of an exported function.
pub struct Export {
	/// The function itself: for a future, the call that makes it.
	pub call: Sig,
	/// The C name its path gives it, which another language names it by, as a clash in C
	/// renames only C's.
	pub named: String,
	/// For a future: its handle type and the functions over it.
	pub future: Option<Future>,
}

/// How the glue calls a value's `&mut self` method for a language whose values are
/// immutable, which [`Abi::handed_back`] makes.
#[derive(Clone)]
pub struct Handed {
	/// The C name of the method's function, which a clash may have renamed.
	name: String,
	/// How many parameters both take after `self`, before the outs.
	lead: usize,
	/// Whether the method returns nothing.
	unit: bool,
	/// Whether it returns a tuple, which the C function writes through its outs.
	tuple: bool,
}

impl Handed {
	/// The glue's expression, of the handed-back export's C return, calling the C function
	/// in place on a copy of the value that `args[0]` points to and handing the copy back.
	/// `args` are of that export's C signature: `self`, what both take after it, then the outs the value and
	/// each element Rust returns are written through, which the C function takes from the
	/// one after the value's.
	pub fn call(&self, args: &[String]) -> String {
		let lead = self.lead;
		let mut real = vec!["&mut handed_".to_string()];
		real.extend(args[1..=lead].iter().cloned());
		if self.tuple {
			real.extend(args[lead + 2..].iter().cloned());
		}
		let call = format!("crate::{}({})", self.name, real.join(", "));
		// Apart from the names the glue gives its arguments, which are numbered.
		let body = match (self.unit, self.tuple) {
			(true, _) => format!("{call};\n\thanded_"),
			(_, true) => format!("{call};\n\tstd::ptr::write({}, handed_)", args[lead + 1]),
			(_, false) => format!(
				"let handed_r_ = {call};\n\tstd::ptr::write({}, handed_);\n\tstd::ptr::write({}, handed_r_)",
				args[lead + 1],
				args[lead + 2]
			),
		};
		format!("unsafe {{\n\tlet mut handed_ = *({});\n\t{body}\n}}", args[0])
	}
}

/// The C functions of a future besides the call that makes it.
pub struct Future {
	/// The handle's type.
	pub handle: String,
	/// The call and its first poll in one, which writes a ready result as the poll does and
	/// hands back the handle only if the future pends.
	pub start: Sig,
	pub poll: Sig,
	pub free: Sig,
}

/// `ty` as a clone of each borrow it holds.
fn owned(ty: &Type) -> Type {
	match ty {
		Type::Ref(_, inner) => owned(inner),
		Type::Option(inner) => Type::Option(Box::new(owned(inner))),
		Type::Box(inner) => Type::Box(Box::new(owned(inner))),
		Type::List(c, inner) => Type::List(*c, Box::new(owned(inner))),
		Type::Tuple(elements) => Type::Tuple(elements.iter().map(owned).collect()),
		ty => ty.clone(),
	}
}

/// The suffix naming each instance of a generic item, by its arguments.
type Suffixes = HashMap<Vec<Type>, String>;

/// The `cfg` the crate's build script sets to the target it builds for.
pub const CFG: &str = "rust2_ffi_target";

pub struct Abi<'a> {
	pub api: &'a Api,
	pub prefix: String,
	/// Each generic item's instances, by whether it is a function and its C name, with
	/// the suffix that names each by its arguments.
	suffixes: std::cell::RefCell<HashMap<(bool, String), Suffixes>>,
	/// Each `Option<T>` struct by C name, with `T`.
	pub options: BTreeMap<String, Type>,
	/// Each range, bound, and pair of bounds struct by C name, with its type.
	pub ranges: BTreeMap<String, Type>,
	/// Each `Vec<T>` struct by C name, with `T`.
	pub lists: BTreeMap<String, Type>,
	/// Each tuple struct by C name, with its elements.
	pub tuples: BTreeMap<String, Vec<Type>>,
	/// Each `[u8; N]` struct by C name, with `N`.
	pub arrays: BTreeMap<String, usize>,
	/// Every C name claimed.
	claims: Vec<Claim>,
	/// The C name of each item whose own clashed.
	renames: HashMap<Item, String>,
	/// Each name one item claims twice.
	pub clashes: Vec<Clash>,
}

impl<'a> Abi<'a> {
	pub fn new(api: &'a Api, prefix: &str) -> Self {
		let mut abi = Self {
			api,
			prefix: prefix.to_string(),
			suffixes: Default::default(),
			options: BTreeMap::new(),
			ranges: BTreeMap::new(),
			lists: BTreeMap::new(),
			tuples: BTreeMap::new(),
			arrays: BTreeMap::new(),
			claims: Vec::new(),
			renames: HashMap::new(),
			clashes: Vec::new(),
		};
		// Each round renames what clashed, until a round finds nothing more to rename.
		loop {
			abi.claims.clear();
			abi.claim_all();
			if !abi.rename() {
				break;
			}
		}
		abi.clashes = abi.residue();
		abi
	}

	/// The `cfg` predicate a crate's Rust compiles what is on `on` under, or none if it is
	/// on every target: its build script sets [`CFG`] to the target it builds for.
	pub fn cfg(&self, on: On) -> Option<String> {
		if self.api.everywhere(on) {
			return None;
		}
		let targets: Vec<String> = self.api.named(on).iter().map(|t| format!("{CFG} = {t:?}")).collect();
		Some(format!("any({})", targets.join(", ")))
	}

	/// The targets every type `ty` names is on, which a value of it is on.
	pub fn on(&self, ty: &Type) -> On {
		let api = self.api;
		let mut on = On::ALL;
		crate::ir::each(ty, &mut |ty| {
			let of = match ty {
				Type::Record(i) => api.records[*i].on,
				Type::Enum(i) => api.enums[*i].on,
				Type::Object(i) => api.objects[*i].on,
				Type::Callback(i, _) => api.callbacks[*i].on,
				_ => On::ALL,
			};
			on.0 &= of.0;
		});
		on
	}

	/// The C name of `item`, renamed if its own clashed, else `name`.
	fn renamed(&self, item: Item, name: impl FnOnce() -> String) -> String {
		self.renames.get(&item).cloned().unwrap_or_else(name)
	}

	/// The claims of each name claimed more than once.
	fn groups(&self) -> BTreeMap<&str, Vec<&Claim>> {
		let mut groups: BTreeMap<&str, Vec<&Claim>> = BTreeMap::new();
		for claim in &self.claims {
			groups.entry(&claim.name).or_default().push(claim);
		}
		groups.retain(|_, claims| claims.len() > 1);
		groups
	}

	/// Rename the items whose names clash, returning whether any was. The runtime's names
	/// come first, then names made from an item, then items' own, each by what claims it:
	/// the first keeps the name and every later one takes a trailing `_`.
	fn rename(&mut self) -> bool {
		let mut renames: HashMap<Item, String> = HashMap::new();
		for group in self.groups().into_values() {
			let mut group = group;
			let rank = |claim: &Claim| match &claim.owner {
				None => 0,
				Some(Owner { own: false, .. }) => 1,
				Some(_) => 2,
			};
			group.sort_by(|a, b| (rank(a), &a.what).cmp(&(rank(b), &b.what)));
			let first = group[0].owner.as_ref().map(|owner| &owner.item);
			for claim in &group[1..] {
				// A name an item claims twice, no rename of it tells apart.
				if let Some(owner) = claim.owner.as_ref().filter(|owner| Some(&owner.item) != first) {
					renames.entry(owner.item.clone()).or_insert(format!("{}_", owner.base));
				}
			}
		}
		let renamed = !renames.is_empty();
		self.renames.extend(renames);
		renamed
	}

	/// Each name still claimed twice once renaming is done: one item's, twice.
	fn residue(&self) -> Vec<Clash> {
		let mut clashes = Vec::new();
		for (name, group) in self.groups() {
			for claim in &group[1..] {
				clashes.push((
					name.to_string(),
					(group[0].what.clone(), group[0].span),
					(claim.what.clone(), claim.span),
				));
			}
		}
		clashes
	}

	/// The C name for a Rust path, which starts with the facade's root.
	pub fn name(&self, path: &[String]) -> String {
		std::iter::once(self.prefix.clone())
			.chain(self.segments(path))
			.collect::<Vec<_>>()
			.join("_")
	}

	/// The segments of the C name of the item at `shown` after the prefix: each of its
	/// path's after the root, without an instance's arguments.
	fn segments(&self, shown: &[String]) -> Vec<String> {
		(shown[1..].iter())
			.map(|segment| segment.split('<').next().unwrap_or(segment).to_string())
			.collect()
	}

	/// A constant's name for a path: the name in upper case.
	pub fn constant(&self, path: &[String]) -> String {
		self.renamed(Item::Constant(path.to_vec()), || self.name(path).to_uppercase())
	}

	pub fn record(&self, i: usize) -> String {
		self.renamed(Item::Record(i), || self.unrenamed(&Item::Record(i)))
	}

	pub fn enumeration(&self, i: usize) -> String {
		self.renamed(Item::Enum(i), || self.unrenamed(&Item::Enum(i)))
	}

	pub fn object(&self, i: usize) -> String {
		self.renamed(Item::Object(i), || self.unrenamed(&Item::Object(i)))
	}

	/// The C name of the type `item` as no clash renames it.
	fn unrenamed(&self, item: &Item) -> String {
		let api = self.api;
		match *item {
			Item::Record(i) => self.type_instance(&api.records[i].path, &api.records[i].shown, &api.records[i].args),
			Item::Enum(i) => self.type_instance(&api.enums[i].path, &api.enums[i].shown, &api.enums[i].args),
			Item::Object(i) => self.type_instance(&api.objects[i].path, &api.objects[i].shown, &api.objects[i].args),
			Item::Callback(i) => self.name(&api.callbacks[i].path),
			ref other => unreachable!("{other:?} is no type"),
		}
	}

	/// The segments the unrenamed C name of the type `item` joins after the prefix: its
	/// path's, then an instance's suffix.
	fn type_segments(&self, item: &Item) -> Vec<String> {
		let api = self.api;
		let shown = match *item {
			Item::Record(i) => &api.records[i].shown,
			Item::Enum(i) => &api.enums[i].shown,
			Item::Object(i) => &api.objects[i].shown,
			Item::Callback(i) => &api.callbacks[i].path,
			ref other => unreachable!("{other:?} is no type"),
		};
		let mut segments = self.segments(shown);
		let name = self.unrenamed(item);
		segments.extend(name[self.name(shown).len()..].strip_prefix('_').map(str::to_string));
		segments
	}

	/// The C name of the instance of the type at `path` at `args`, shown as `shown`, among
	/// the type's every instance, whichever kind each is.
	fn type_instance(&self, path: &[String], shown: &[String], args: &[Type]) -> String {
		let api = self.api;
		self.instance(false, self.name(shown), args, || {
			let records = api.records.iter().map(|r| (&r.path, &r.args));
			let enums = api.enums.iter().map(|e| (&e.path, &e.args));
			let objects = api.objects.iter().map(|o| (&o.path, &o.args));
			(records.chain(enums).chain(objects))
				.filter(|(other, _)| other[..] == *path)
				.map(|(_, args)| args.clone())
				.collect()
		})
	}

	/// An instance's C name: its generic item's, `name`, then the suffix naming `args`
	/// apart from the arguments of the item's other instances, which `group` lists.
	fn instance(&self, function: bool, name: String, args: &[Type], group: impl FnOnce() -> Vec<Vec<Type>>) -> String {
		if args.is_empty() {
			return name;
		}
		let key = (function, name);
		if let Some(suffix) = self.suffixes.borrow().get(&key).and_then(|suffixes| suffixes.get(args)) {
			return format!("{}_{suffix}", key.1);
		}
		let group = group();
		let forms: Vec<Vec<Vec<String>>> = (group.iter())
			.map(|args| args.iter().map(|arg| self.arg_forms(arg)).collect())
			.collect();
		let suffixes: Suffixes = group.into_iter().zip(shortest(&forms)).collect();
		let suffix = suffixes[args].clone();
		self.suffixes.borrow_mut().insert(key.clone(), suffixes);
		format!("{}_{suffix}", key.1)
	}

	/// The names an instance's argument `ty` may take, shortest first: a type's path's
	/// last segment, then its own arguments' names, lengthened in step until whole, then
	/// its path's segments, one more at a time. They read only `ty` itself, never another
	/// instance's name, so two items whose instances name each other, as `A<B<u8>>` and
	/// `B<A<u8>>`, name each without the other.
	fn arg_forms(&self, ty: &Type) -> Vec<String> {
		let (shown, args) = match ty {
			Type::Record(i) => (&self.api.records[*i].shown, &self.api.records[*i].args),
			Type::Enum(i) => (&self.api.enums[*i].shown, &self.api.enums[*i].args),
			Type::Object(i) => (&self.api.objects[*i].shown, &self.api.objects[*i].args),
			Type::Ref(_, inner) | Type::Box(inner) => return self.arg_forms(inner),
			// Apart from its integer's, which an instance of the same item may also take.
			Type::NonZero(int) => return vec![format!("non_zero_{}", int.rust())],
			ty => return vec![self.key(ty)],
		};
		let segments = self.segments(shown);
		let nested: Vec<Vec<String>> = args.iter().map(|arg| self.arg_forms(arg)).collect();
		let steps = nested.iter().map(Vec::len).max().unwrap_or(1);
		let form = |len: usize, step: usize| {
			std::iter::once(segments[segments.len() - len..].join("_"))
				.chain(nested.iter().map(|forms| forms[step.min(forms.len() - 1)].clone()))
				.collect::<Vec<_>>()
				.join("_")
		};
		let mut forms: Vec<String> = (0..steps).map(|step| form(1, step)).collect();
		forms.extend((2..=segments.len()).map(|len| form(len, steps - 1)));
		forms.dedup();
		forms
	}

	/// A function's C name: a method's through its type's, then each argument of the
	/// instance it is. A field's getter adds `_get`, and its setter `_set`.
	fn function_name(&self, function: &Function, owner: Option<&Ty>) -> String {
		// An item of the API it is, which outlives this, so its address names it.
		let item = Item::Function(function as *const Function as usize);
		self.renamed(item, || self.unrenamed_function(function, owner))
	}

	fn unrenamed_function(&self, function: &Function, owner: Option<&Ty>) -> String {
		let owner = match owner {
			Some(Ty::Struct(owner) | Ty::Enum(owner) | Ty::Handle { name: owner, .. }) => Some(owner.as_str()),
			_ => None,
		};
		let name = match owner {
			Some(owner) => format!("{owner}_{}", function.name()),
			None => self.name(&function.shown),
		};
		match function.access {
			Access::Field { set: false, .. } => return format!("{name}_get"),
			Access::Field { set: true, .. } => return format!("{name}_set"),
			_ => {}
		}
		let Some(instance) = function.instance.as_ref() else {
			return name;
		};
		self.instance(true, name, &instance.named, || {
			let api = self.api;
			let records = (0..api.records.len()).map(|i| (Some(self.record(i)), &api.records[i].methods));
			let enums = (0..api.enums.len()).map(|i| (Some(self.enumeration(i)), &api.enums[i].methods));
			let objects = (0..api.objects.len()).map(|i| (Some(self.object(i)), &api.objects[i].methods));
			let free = std::iter::once((None, &api.functions));
			(records.chain(enums).chain(objects).chain(free))
				.filter(|(other, _)| other.as_deref() == owner)
				.flat_map(|(_, functions)| functions)
				.filter(|other| other.path == function.path)
				.filter_map(|other| other.instance.as_ref().map(|instance| instance.named.clone()))
				.collect()
		})
	}

	pub fn callback(&self, i: usize) -> String {
		self.renamed(Item::Callback(i), || self.unrenamed(&Item::Callback(i)))
	}

	/// Each exported type's name in Pascal case, by its C name, kept [`apart`] from those
	/// beside it and [`unique`] among them, the ranges' and arrays' classes, and the language's `taken`
	/// names: `scope` gives
	/// the scope a type's [`Home`] puts it in, and how many leading segments of its path its
	/// name there leaves out.
	pub fn classes(
		&self,
		scope: impl Fn(&Home) -> (String, usize),
		taken: impl Fn(&str) -> bool,
	) -> HashMap<String, String> {
		let api = self.api;
		let records = (0..api.records.len()).map(|i| (Item::Record(i), &api.records[i].home));
		let enums = (0..api.enums.len()).map(|i| (Item::Enum(i), &api.enums[i].home));
		let objects = (0..api.objects.len()).map(|i| (Item::Object(i), &api.objects[i].home));
		let callbacks = (0..api.callbacks.len()).map(|i| (Item::Callback(i), &api.callbacks[i].home));
		let ranges: std::collections::BTreeSet<String> = self
			.ranges
			.keys()
			.map(|c| pascal(&c[self.prefix.len() + 1..]))
			.chain(self.arrays.values().map(|n| Self::array_class(*n)))
			.collect();
		let taken = move |name: &str| taken(name) || ranges.contains(name);
		let mut scopes: BTreeMap<String, Vec<(String, Vec<String>, usize)>> = BTreeMap::new();
		for (item, home) in records.chain(enums).chain(objects).chain(callbacks) {
			let (scope, outside) = scope(home);
			// Named as Rust's path is, whatever C's clashes rename.
			let segments = self.type_segments(&item);
			let c = self.renamed(item.clone(), || self.unrenamed(&item));
			scopes.entry(scope).or_default().push((c, segments, outside));
		}
		(scopes.into_values())
			.flat_map(move |types| {
				// Names still shared once `apart` has split what it can keep the segment before
				// them, a module of their own, so each of `connect::Config` and `listen::Config`
				// keeps its module. Only the names a clash would refuse change.
				let mut outside: Vec<usize> = types.iter().map(|(_, _, outside)| *outside).collect();
				let names = loop {
					let names = apart(
						types
							.iter()
							.zip(&outside)
							.map(|((_, segments, _), &outside)| &segments[outside..]),
					);
					let mut count: HashMap<&str, usize> = HashMap::new();
					for name in &names {
						*count.entry(name).or_default() += 1;
					}
					let mut grew = false;
					for (i, name) in names.iter().enumerate() {
						if count[name.as_str()] > 1 && outside[i] > 0 {
							outside[i] -= 1;
							grew = true;
						}
					}
					if !grew {
						break names;
					}
				};
				// Those still alike, as segments apart only by case are, the one Rust names in
				// Pascal case first, then by C name.
				let mut named: Vec<(bool, String, String)> = (types.into_iter().zip(names))
					.map(|((c, segments, _), name)| {
						let last = segments.last().map_or("", String::as_str);
						(pascal(last) != last, c, name)
					})
					.collect();
				named.sort();
				let (cs, names): (Vec<String>, Vec<String>) = named.into_iter().map(|(_, c, name)| (c, name)).unzip();
				cs.into_iter().zip(unique(names, &taken))
			})
			.collect()
	}

	/// The prefixed name of a runtime item: `buf` is `moq_buf`.
	pub fn rt(&self, name: &str) -> String {
		format!("{}_{name}", self.prefix)
	}

	/// The C name of `Option<inner>`'s struct.
	pub fn option(&self, inner: &Type) -> String {
		format!("{}_option_{}", self.prefix, self.key(inner))
	}

	/// The C name of `Vec<elem>`'s struct: `<prefix>_vec_string`.
	pub fn list(&self, elem: &Type) -> String {
		format!("{}_vec_{}", self.prefix, self.key(elem))
	}

	/// The C name of a tuple's struct: `<prefix>_tuple_string_u64`.
	pub fn tuple(&self, elements: &[Type]) -> String {
		self.renamed(Item::Tuple(elements.iter().map(owned).collect()), || {
			let keys: Vec<String> = elements.iter().map(|e| self.key(e)).collect();
			format!("{}_tuple_{}", self.prefix, keys.join("_"))
		})
	}

	/// The C name of `[u8; n]`'s struct: `<prefix>_array_u8_32`.
	pub fn array(&self, n: usize) -> String {
		format!("{}_array_u8_{n}", self.prefix)
	}

	/// The class a language with no fixed-size array of its own generates for `[u8; n]`:
	/// `ArrayU8x32`, which no exported type may take.
	pub fn array_class(n: usize) -> String {
		format!("ArrayU8x{n}")
	}

	/// The C name of a range, bound, or pair of bounds struct: `<prefix>_range_u64`.
	pub fn range(&self, ty: &Type) -> String {
		format!("{}_{}", self.prefix, self.key(ty))
	}

	/// A type's name within the prefix, for naming the structs and instances built from it.
	pub fn key(&self, ty: &Type) -> String {
		let strip = |name: String| name[self.prefix.len() + 1..].to_string();
		match ty {
			Type::Unit => "unit".to_string(),
			Type::Bool => "bool".to_string(),
			Type::Str | Type::String => "string".to_string(),
			Type::Slice | Type::Vec | Type::Bytes => "bytes".to_string(),
			Type::Path | Type::PathBuf => "path".to_string(),
			Type::Int(int) | Type::NonZero(int) => int.rust().to_string(),
			Type::F32 => "f32".to_string(),
			Type::F64 => "f64".to_string(),
			Type::Char => "char".to_string(),
			Type::U128 => "u128".to_string(),
			Type::Duration => "duration".to_string(),
			Type::Array(n) => format!("array_u8_{n}"),
			Type::SocketAddr(_) => "socket_addr".to_string(),
			Type::Option(inner) => format!("option_{}", self.key(inner)),
			Type::List(_, inner) => format!("vec_{}", self.key(inner)),
			Type::Tuple(elements) => strip(self.tuple(elements)),
			Type::Record(i) => strip(self.record(*i)),
			Type::Enum(i) => strip(self.enumeration(*i)),
			Type::Object(i) => strip(self.object(*i)),
			Type::Range(int) => format!("range_{}", int.rust()),
			Type::Bound(int) => format!("bound_{}", int.rust()),
			Type::Bounds(int) => format!("bounds_{}", int.rust()),
			Type::Ref(_, inner) | Type::Box(inner) => self.key(inner),
			// Only an instance's name holds one, where a C identifier has no `-`.
			Type::Const(Literal::Int(_, v)) if *v < 0 => format!("neg{}", v.unsigned_abs()),
			Type::Const(value) => value.to_string(),
			other => unreachable!("no option struct holds {other:?}"),
		}
	}

	/// A type by its key, with each tuple spelled as Rust spells it, which tells apart
	/// tuples that share a C name.
	pub fn spell(&self, ty: &Type) -> String {
		match ty {
			Type::Tuple(elements) => {
				let elements: Vec<String> = elements.iter().map(|e| self.spell(e)).collect();
				match elements.len() {
					1 => format!("({},)", elements[0]),
					_ => format!("({})", elements.join(", ")),
				}
			}
			ty => self.key(ty),
		}
	}

	/// Whether `ty` crosses as a `<prefix>_buf`.
	pub fn is_buf(ty: &Type) -> bool {
		matches!(
			ty.base(),
			Type::Str | Type::String | Type::Slice | Type::Vec | Type::Bytes | Type::Path | Type::PathBuf
		)
	}

	/// Whether `Option<ty>` crosses as `ty`, with a null pointer for `None`: a handle, a
	/// buffer, or a `Vec`'s array.
	pub fn nullable(ty: &Type) -> bool {
		matches!(ty.base(), Type::Object(_) | Type::List(..)) || Self::is_buf(ty)
	}

	/// The C type of `ty` as a value: a parameter, field, or what a function returns.
	pub fn ty(&self, ty: &Type) -> Ty {
		match ty {
			Type::Const(_) => unreachable!("a const argument is no value"),
			Type::Unit => Ty::Void,
			Type::Bool => Ty::Bool,
			Type::Int(int) | Type::NonZero(int) => Ty::Int(*int),
			Type::F32 => Ty::F32,
			Type::F64 => Ty::F64,
			Type::Char => Ty::Char,
			Type::Str | Type::String | Type::Slice | Type::Vec | Type::Bytes | Type::Path | Type::PathBuf => Ty::Buf,
			Type::U128 => Ty::U128,
			Type::Duration => Ty::Duration,
			Type::Array(n) => Ty::Struct(self.array(*n)),
			Type::SocketAddr(_) => Ty::SocketAddr,
			Type::Waker => Ty::Waker,
			Type::Future => Ty::ForeignFuture,
			Type::Reader => Ty::ForeignReader,
			Type::List(_, inner) => Ty::Struct(self.list(inner)),
			Type::Option(inner) => match Self::nullable(inner) {
				true => self.ty(inner),
				false => Ty::Struct(self.option(inner)),
			},
			Type::Box(inner) => self.ty(inner),
			Type::Ref(m, inner) => match &**inner {
				Type::Object(i) => Ty::Handle {
					name: self.object(*i),
					constant: *m == crate::ir::Mutability::Not,
				},
				inner => self.ty(inner),
			},
			Type::Record(i) => Ty::Struct(self.record(*i)),
			Type::Enum(i) => match self.api.enums[*i].plain() {
				true => Ty::Enum(self.enumeration(*i)),
				false => Ty::Struct(self.enumeration(*i)),
			},
			Type::Object(i) => Ty::Handle {
				name: self.object(*i),
				constant: false,
			},
			Type::Callback(i, _) => Ty::Struct(self.callback(*i)),
			Type::Closure(closure) => self.closure(closure),
			Type::Range(_) | Type::Bound(_) | Type::Bounds(_) => Ty::Struct(self.range(ty)),
			Type::Tuple(elements) => Ty::Struct(self.tuple(elements)),
		}
	}

	/// The C function a closure is: after its data, each value it takes, lent for its
	/// call; then for a `Result`, an out pointer for its value and one for its error,
	/// returning whether it succeeded, or else its value. A buffer it returns is Rust's,
	/// one `<prefix>_buf_copy` made.
	pub fn closure(&self, closure: &Closure) -> Ty {
		let mut params: Vec<Ty> = closure.params.iter().map(|ty| self.ty(ty)).collect();
		let ok = self.ty(&closure.ok);
		match &closure.err {
			None => Ty::Fn(params, Box::new(ok)),
			Some(err) => {
				if ok != Ty::Void {
					params.push(Ty::Out(Box::new(ok)));
				}
				params.push(Ty::Out(Box::new(self.ty(err))));
				Ty::Fn(params, Box::new(Ty::Bool))
			}
		}
	}

	/// The C type of `ty` returned: a borrow is returned as a clone, so a handle in it is
	/// the caller's own.
	fn returned(&self, ty: &Type) -> Ty {
		self.ty(&owned(ty))
	}

	/// The C signature of `function`, a method of the type named `owner` if it has one.
	pub fn export(&self, function: &Function, owner: Option<&Ty>) -> Export {
		let name = self.function_name(function, owner);
		let named = self.unrenamed_function(function, owner);
		let mut params = Vec::new();
		if let (Some(receiver), Some(owner)) = (function.receiver, owner) {
			let ty = match owner {
				Ty::Handle { name, .. } => Ty::Handle {
					name: name.clone(),
					constant: receiver == Receiver::Ref
						|| (receiver == Receiver::Owned
							&& (0..self.api.objects.len())
								.any(|i| self.api.objects[i].copy && &self.object(i) == name)),
				},
				// A value's receiver is read through a pointer, and written back in place for
				// `&mut self`.
				value if receiver == Receiver::Mut => Ty::InOut(Box::new(value.clone())),
				value => Ty::In(Box::new(value.clone())),
			};
			params.push(("self".to_string(), ty));
		}
		// What Rust writes back for each value C passes for `&mut`, after every parameter.
		let mut written = Vec::new();
		for param in &function.params {
			let copied = match &param.ty {
				Type::Object(i) => Some(*i),
				Type::Option(inner) => match **inner {
					Type::Object(i) => Some(i),
					_ => None,
				},
				_ => None,
			};
			let ty = match copied {
				// Taken by value, a `Copy` object is copied out, leaving its handle as it was.
				Some(i) if self.api.objects[i].copy => Ty::Handle {
					name: self.object(i),
					constant: true,
				},
				_ => self.ty(&param.ty),
			};
			let name = escape(&param.name);
			if let Type::Closure(closure) = &param.ty {
				// Named apart from every parameter of the function, which C requires.
				let apart = |suffix: &str| {
					let mut named = format!("{name}_{suffix}");
					while function.params.iter().any(|p| escape(&p.name) == named) {
						named.push('_');
					}
					named
				};
				let (data, free) = (apart("data"), apart("free"));
				params.push((name, ty));
				params.push((data, Ty::Data));
				// A kept closure's free, which Rust calls with its data once it drops it.
				if closure.kept {
					params.push((free, Ty::Fn(Vec::new(), Box::new(Ty::Void))));
				}
				continue;
			}
			if param.ty.written() {
				written.push((Self::written(function, &name), Ty::Out(Box::new(ty.clone()))));
			}
			params.push((name, ty));
		}
		// A value returned is C's return, after which only the written outs follow.
		let writes = written.len();
		let mut outs = written;
		match &function.ok {
			Type::Unit => {}
			Type::Tuple(elements) => {
				for (n, element) in elements.iter().enumerate() {
					outs.push((format!("out{n}"), Ty::Out(Box::new(self.returned(element)))));
				}
			}
			ok => outs.push(("out".to_string(), Ty::Out(Box::new(self.returned(ok))))),
		}
		if let Some(err) = &function.err {
			outs.push(("err".to_string(), Ty::Out(Box::new(self.returned(err)))));
		}
		if function.future {
			let handle = format!("{name}_future");
			let mut poll_params = vec![
				("future".to_string(), Ty::Future(handle.clone())),
				("waker".to_string(), Ty::Waker),
			];
			poll_params.extend(outs.iter().cloned());
			let mut start_params = params.clone();
			start_params.extend(outs);
			start_params.push(("future".to_string(), Ty::Out(Box::new(Ty::Future(handle.clone())))));
			let start = Sig {
				name: format!("{name}_start"),
				params: start_params,
				ret: Ty::Poll,
			};
			let poll = Sig {
				name: format!("{name}_poll"),
				params: poll_params,
				ret: Ty::Poll,
			};
			let free = Sig {
				name: format!("{name}_future_free"),
				params: vec![("future".to_string(), Ty::Future(handle.clone()))],
				ret: Ty::Void,
			};
			return Export {
				call: Sig {
					name,
					params,
					ret: Ty::Future(handle.clone()),
				},
				future: Some(Future {
					handle,
					start,
					poll,
					free,
				}),
				named,
			};
		}
		let ret = match (&function.err, &function.ok) {
			(Some(_), _) => {
				params.extend(outs);
				Ty::Bool
			}
			(None, Type::Tuple(_)) => {
				params.extend(outs);
				Ty::Void
			}
			(None, ok) => {
				params.extend(outs.into_iter().take(writes));
				self.returned(ok)
			}
		};
		Export {
			call: Sig { name, params, ret },
			future: None,
			named,
		}
	}

	/// The out pointer Rust writes back the value C passed for `&mut` as parameter `name`
	/// through, named apart from every parameter of `function`.
	pub fn written(function: &Function, name: &str) -> String {
		let mut out = format!("{name}_out");
		while function.params.iter().any(|p| escape(&p.name) == out) {
			out.push('_');
		}
		out
	}

	/// A value's `&mut self` method as a language whose values are immutable calls it:
	/// through `&self`, returning the value as the call left it, before Rust's own return
	/// or each element of its tuple: that function, its export, and how the glue calls it.
	/// The export keeps the C name of the method's, which a clash may have renamed. `None`
	/// for any other function.
	pub fn handed_back(&self, function: &Function, owner: &Ty, owner_ty: &Type) -> Option<(Function, Export, Handed)> {
		if function.receiver != Some(Receiver::Mut) || matches!(owner, Ty::Handle { .. }) {
			return None;
		}
		// `written_back` in collect.rs refuses a future's and a `Result`'s.
		assert!(
			!function.future && function.err.is_none(),
			"a value's `&mut self` returns a plain value"
		);
		let real = self.export(function, Some(owner));
		let returned = match &function.ok {
			Type::Unit => Vec::new(),
			Type::Tuple(elements) => elements.clone(),
			ok => vec![ok.clone()],
		};
		let back = Function {
			receiver: Some(Receiver::Ref),
			ok: match returned.is_empty() {
				true => owner_ty.clone(),
				false => Type::Tuple([vec![owner_ty.clone()], returned].concat()),
			},
			..function.clone()
		};
		// Named as the method is: a rename is keyed by the API's own function.
		let mut export = self.export(&back, Some(owner));
		export.call.name = real.call.name.clone();
		export.named = real.named;
		let elements = match &function.ok {
			Type::Tuple(elements) => elements.len(),
			_ => 0,
		};
		let handed = Handed {
			name: real.call.name,
			lead: real.call.params.len() - 1 - elements,
			unit: function.ok == Type::Unit,
			tuple: elements > 0,
		};
		Some((back, export, handed))
	}

	fn claim(&mut self, name: String, what: String, span: Span, owner: Option<Owner>) {
		self.claims.push(Claim {
			name,
			what,
			span,
			owner,
		});
	}

	/// The owner of a name made from `item`, whose own C name is `base`.
	fn made(item: &Item, base: &str) -> Option<Owner> {
		Some(Owner {
			item: item.clone(),
			base: base.to_string(),
			own: false,
		})
	}

	/// The owner of `item`'s own C name, `base`, which joins the prefix and `segments`
	/// unless a rename already changed it.
	fn own(&self, item: &Item, base: &str, segments: Vec<String>) -> Option<Owner> {
		let prefix = match base.starts_with(&self.prefix) {
			true => self.prefix.clone(),
			false => self.prefix.to_uppercase(),
		};
		let segments: Vec<String> = std::iter::once(prefix).chain(segments).collect();
		Some(Owner {
			item: item.clone(),
			base: base.to_string(),
			own: segments.join("_") == base,
		})
	}

	/// The item a name made from `ty` belongs to, with its own C name: a type's, or a tuple's.
	fn item_of(&self, ty: &Type) -> Option<(Item, String)> {
		match ty {
			Type::Record(i) => Some((Item::Record(*i), self.record(*i))),
			Type::Enum(i) => Some((Item::Enum(*i), self.enumeration(*i))),
			Type::Object(i) => Some((Item::Object(*i), self.object(*i))),
			Type::Tuple(elements) => Some((Item::Tuple(elements.iter().map(owned).collect()), self.tuple(elements))),
			Type::Ref(_, inner) | Type::Box(inner) => self.item_of(inner),
			_ => None,
		}
	}

	/// Claim every name the export defines, and note each option struct.
	fn claim_all(&mut self) {
		let api = self.api;
		let crate_span = api
			.records
			.first()
			.map(|r| r.span)
			.or_else(|| api.objects.first().map(|o| o.span))
			.unwrap_or(rustc_span::DUMMY_SP);
		// C's runtime, then what C++'s adds in the namespace its types share, Kotlin's in its
		// package, and Swift's in its module and the atomics header its runtime adds.
		for runtime in [
			"contract",
			"target",
			"buf",
			"buf_free",
			"u128",
			"duration",
			"socket_addr",
			"waker",
			"foreign_future",
			"foreign_reader",
			"buf_copy",
			"buf_clone",
			"poll",
			"view",
			"items",
			"taken",
			"borrow",
			"result",
			"task",
			"future",
			"poll_t",
			"detail",
			"native",
			"bytes",
			"input",
			"mut",
			"atomic_load",
			"atomic_store",
			"atomic_add",
			"atomic_exchange",
			"atomic_cas",
		] {
			self.claim(self.rt(runtime), "the runtime".to_string(), crate_span, None);
		}
		for constant in ["CONTRACT", "POLL_PENDING", "POLL_READY", "POLL_ERROR", "ATOMICS_H"] {
			let name = format!("{}_{constant}", self.prefix.to_uppercase());
			self.claim(name, "the runtime".to_string(), crate_span, None);
		}
		let mut types: Vec<&Type> = Vec::new();
		// Each function, with its type's C name and segments if it is a method.
		let mut functions: Vec<(&Function, Option<Ty>, Vec<String>)> = Vec::new();
		for (i, record) in api.records.iter().enumerate() {
			let item = Item::Record(i);
			let name = self.record(i);
			let segments = self.type_segments(&item);
			let what = format!("struct `{}`", api.rust(&record.path, &record.args));
			self.claim_traits(&item, &name, &what, record.span, record.traits, true);
			self.claim(
				name.clone(),
				what,
				record.span,
				self.own(&item, &name, segments.clone()),
			);
			types.extend(record.fields.iter().map(|f| &f.ty));
			functions.extend(
				record
					.methods
					.iter()
					.map(|m| (m, Some(Ty::Struct(name.clone())), segments.clone())),
			);
		}
		for (i, e) in api.enums.iter().enumerate() {
			let item = Item::Enum(i);
			let name = self.enumeration(i);
			let segments = self.type_segments(&item);
			let what = format!("enum `{}`", api.rust(&e.path, &e.args));
			let upper = name.to_uppercase();
			for variant in &e.variants {
				self.claim(
					format!("{upper}_{}", snake(&variant.name).to_uppercase()),
					what.clone(),
					e.span,
					Self::made(&item, &name),
				);
				types.extend(variant.fields.iter().map(|f| &f.ty));
			}
			let plain = e.plain();
			if !plain {
				self.claim(format!("{name}_tag"), what.clone(), e.span, Self::made(&item, &name));
				// Each variant's payload, which C++ names even when it has no fields.
				for variant in &e.variants {
					self.claim(
						payload(&name, &variant.name),
						what.clone(),
						e.span,
						Self::made(&item, &name),
					);
				}
			}
			self.claim_traits(&item, &name, &what, e.span, e.traits, !plain);
			self.claim(name.clone(), what, e.span, self.own(&item, &name, segments.clone()));
			let owner = match plain {
				true => Ty::Enum(name.clone()),
				false => Ty::Struct(name.clone()),
			};
			functions.extend(e.methods.iter().map(|m| (m, Some(owner.clone()), segments.clone())));
		}
		for (i, object) in api.objects.iter().enumerate() {
			let item = Item::Object(i);
			let name = self.object(i);
			let segments = self.type_segments(&item);
			let what = format!("type `{}`", api.rust(&object.path, &object.args));
			self.claim(
				format!("{name}_free"),
				what.clone(),
				object.span,
				Self::made(&item, &name),
			);
			if object.shared {
				self.claim(
					format!("{name}_borrowed"),
					what.clone(),
					object.span,
					Self::made(&item, &name),
				);
			}
			self.claim_traits(&item, &name, &what, object.span, object.traits, false);
			self.claim(
				name.clone(),
				what,
				object.span,
				self.own(&item, &name, segments.clone()),
			);
			let owner = Ty::Handle { name, constant: false };
			functions.extend(
				object
					.methods
					.iter()
					.map(|m| (m, Some(owner.clone()), segments.clone())),
			);
		}
		for (i, callback) in api.callbacks.iter().enumerate() {
			let item = Item::Callback(i);
			let name = self.callback(i);
			let segments = self.type_segments(&item);
			let what = format!("trait `{}`", callback.path.join("::"));
			self.claim(
				format!("{name}_vtable"),
				what.clone(),
				callback.span,
				Self::made(&item, &name),
			);
			self.claim(name.clone(), what, callback.span, self.own(&item, &name, segments));
			for method in &callback.methods {
				types.extend(method.params.iter().map(|p| &p.ty));
				types.push(&method.ok);
				types.extend(&method.err);
			}
		}
		functions.extend(api.functions.iter().map(|f| (f, None, Vec::new())));
		for (function, owner, mut segments) in functions {
			let item = Item::Function(function as *const Function as usize);
			let export = self.export(function, owner.as_ref());
			let name = export.call.name.clone();
			let what = match function.access {
				Access::Field { set: false, .. } => format!("field `{}`", function.path.join("::")),
				Access::Field { set: true, .. } => format!("setter of field `{}`", function.path.join("::")),
				_ => format!("fn `{}`", function.path.join("::")),
			};
			// Its own segments: its type's then its name's, or its path's, then a field's
			// accessor and an instance's suffix.
			match owner {
				Some(_) => segments.push(function.name().to_string()),
				None => segments = self.segments(&function.shown),
			}
			match function.access {
				Access::Field { set: false, .. } => segments.push("get".to_string()),
				Access::Field { set: true, .. } => segments.push("set".to_string()),
				_ => {}
			}
			let unrenamed = self.unrenamed_function(function, owner.as_ref());
			let joined = format!("{}_{}", self.prefix, segments.join("_"));
			segments.extend(
				unrenamed
					.strip_prefix(&joined)
					.and_then(|s| s.strip_prefix('_'))
					.map(str::to_string),
			);
			let own = self.own(&item, &name, segments);
			self.claim(name.clone(), what.clone(), function.span, own);
			if let Some(Future {
				handle,
				start,
				poll,
				free,
			}) = &export.future
			{
				for made in [handle, &start.name, &poll.name, &free.name] {
					self.claim(made.clone(), what.clone(), function.span, Self::made(&item, &name));
				}
			}
			types.extend(function.params.iter().map(|p| &p.ty));
			// A tuple returned whole is written element by element, with no struct of its own.
			match &function.ok {
				Type::Tuple(elements) => types.extend(elements),
				ok => types.push(ok),
			}
			types.extend(&function.err);
		}
		for constant in &api.constants {
			let item = Item::Constant(constant.path.clone());
			let name = self.constant(&constant.path);
			let segments: Vec<String> = self.segments(&constant.path).iter().map(|s| s.to_uppercase()).collect();
			let own = self.own(&item, &name, segments);
			let what = format!("const `{}`", constant.path.join("::"));
			self.claim(name, what, constant.span, own);
		}
		let mut options = BTreeMap::new();
		let mut ranges = BTreeMap::new();
		let mut lists = BTreeMap::new();
		let mut tuples: BTreeMap<String, Vec<Type>> = BTreeMap::new();
		let mut arrays = BTreeMap::new();
		let mut mixed = Vec::new();
		let mut stack = types;
		while let Some(ty) = stack.pop() {
			match ty {
				Type::Option(inner) => {
					if !Self::nullable(inner) {
						options.insert(self.option(inner), (**inner).clone());
					}
					stack.push(inner);
				}
				Type::List(_, inner) => {
					// A returned `Vec<&T>` holds clones, which its free frees.
					lists.insert(self.list(inner), owned(inner));
					stack.push(inner);
				}
				Type::Ref(_, inner) | Type::Box(inner) => stack.push(inner),
				Type::Tuple(elements) => {
					// A returned tuple of borrows holds clones, as a `Vec` does.
					let owned: Vec<Type> = elements.iter().map(owned).collect();
					let name = self.tuple(elements);
					let layout = |elements: &[Type]| elements.iter().map(|e| self.ty(e)).collect::<Vec<_>>();
					match tuples.get(&name) {
						// Its name runs its elements' together, which two nestings can share.
						Some(other) if layout(other) != layout(&owned) => mixed.push((name, owned)),
						Some(_) => {}
						None => {
							tuples.insert(name, owned);
						}
					}
					stack.extend(elements);
				}
				Type::Closure(closure) => {
					stack.extend(&closure.params);
					stack.push(&closure.ok);
					stack.extend(&closure.err);
				}
				Type::Range(_) | Type::Bound(_) => {
					ranges.insert(self.range(ty), ty.clone());
				}
				Type::Array(n) => {
					arrays.insert(self.array(*n), *n);
				}
				Type::Bounds(int) => {
					ranges.insert(self.range(ty), ty.clone());
					ranges.insert(self.range(&Type::Bound(*int)), Type::Bound(*int));
				}
				_ => {}
			}
		}
		for (name, inner) in &options {
			let owner = self.item_of(inner).and_then(|(item, base)| Self::made(&item, &base));
			self.claim(name.clone(), "an option".to_string(), crate_span, owner);
		}
		for name in ranges.keys() {
			self.claim(name.clone(), "a range".to_string(), crate_span, None);
		}
		for name in arrays.keys() {
			self.claim(name.clone(), "an array".to_string(), crate_span, None);
		}
		for (name, elements) in tuples
			.iter()
			.chain(mixed.iter().map(|(name, elements)| (name, elements)))
		{
			let ty = Type::Tuple(elements.clone());
			let what = format!("tuple `{}`", self.spell(&ty));
			let owner = self.item_of(&ty).and_then(|(item, base)| Self::made(&item, &base));
			self.claim(name.clone(), what, crate_span, owner);
		}
		for (name, inner) in &lists {
			let item = self.item_of(inner);
			let owner = || item.as_ref().and_then(|(item, base)| Self::made(item, base));
			self.claim(name.clone(), "a Vec".to_string(), crate_span, owner());
			self.claim(format!("{name}_free"), "a Vec".to_string(), crate_span, owner());
		}
		if ranges.values().any(|ty| matches!(ty, Type::Bound(_))) {
			let bound = self.rt("bound");
			for kind in ["", "_UNBOUNDED", "_INCLUDED", "_EXCLUDED"] {
				let name = match kind {
					"" => bound.clone(),
					kind => format!("{}{kind}", bound.to_uppercase()),
				};
				self.claim(name, "a bound".to_string(), crate_span, None);
			}
		}
		self.options = options;
		self.ranges = ranges;
		self.lists = lists;
		self.tuples = tuples;
		self.arrays = arrays;
	}

	/// Claim the functions a type's std traits export.
	fn claim_traits(
		&mut self,
		item: &Item,
		name: &str,
		what: &str,
		span: Span,
		traits: crate::ir::Traits,
		owned: bool,
	) {
		let mut claim = |suffix: &str| {
			self.claim(
				format!("{name}_{suffix}"),
				what.to_string(),
				span,
				Self::made(item, name),
			)
		};
		if traits.clone && !owned {
			claim("clone");
		}
		if traits.eq {
			claim("eq");
		}
		if traits.display {
			claim("to_string");
		}
		if traits.debug {
			claim("debug");
		}
		if traits.default {
			claim("default");
		}
		if owned {
			claim("free");
		}
	}
}

/// The suffix naming each of a generic item's instances, given the names each of its
/// arguments may take, shortest first: each argument's shortest, lengthened by one at a
/// time only where two instances' suffixes are alike while their arguments differ there.
/// Each suffix depends on the whole group, not its order, so an instance added later can
/// lengthen another's name. Instances alike at every length keep their clash, which
/// claiming their C names reports.
fn shortest(group: &[Vec<Vec<String>>]) -> Vec<String> {
	let mut lens: Vec<Vec<usize>> = group.iter().map(|args| vec![1; args.len()]).collect();
	loop {
		let names: Vec<String> = (group.iter().zip(&lens))
			.map(|(args, lens)| {
				(args.iter().zip(lens))
					.map(|(forms, &len)| forms[len - 1].as_str())
					.collect::<Vec<_>>()
					.join("_")
			})
			.collect();
		let mut grow = std::collections::BTreeSet::new();
		for (a, b) in (0..group.len()).flat_map(|a| (0..group.len()).map(move |b| (a, b))) {
			if a == b || names[a] != names[b] {
				continue;
			}
			for (j, forms) in group[a].iter().enumerate() {
				if group[b].get(j) != Some(forms) && lens[a][j] < forms.len() {
					grow.insert((a, j));
				}
			}
		}
		if grow.is_empty() {
			return names;
		}
		for (a, j) in grow {
			lens[a][j] += 1;
		}
	}
}

/// Each name in Pascal case of items by their paths' segments: the segments joined, but
/// where two would join alike, each of those keeps a `_` at every split after the leading
/// segments they all share, so `tokio::ConnectError` and `tokio::connect::Error` are
/// `TokioConnectError` and `TokioConnect_Error`. Two that still match, as segments apart
/// only by case do, stay alike for the language's claim to refuse at both.
pub fn apart<'s>(items: impl IntoIterator<Item = &'s [String]>) -> Vec<String> {
	let items: Vec<&[String]> = items.into_iter().collect();
	let joined: Vec<String> = items.iter().map(|segments| pascal(&segments.join("_"))).collect();
	let mut groups: HashMap<&str, Vec<usize>> = HashMap::new();
	for (i, name) in joined.iter().enumerate() {
		groups.entry(name).or_default().push(i);
	}
	let mut names = joined.clone();
	for group in groups.values().filter(|group| group.len() > 1) {
		let first = items[group[0]];
		let shared = (0..first.len())
			.take_while(|&k| group.iter().all(|&i| items[i].get(k) == Some(&first[k])))
			.count();
		for &i in group {
			let rest: Vec<String> = items[i][shared..].iter().map(|segment| pascal(segment)).collect();
			names[i] = pascal(&items[i][..shared].join("_")) + &rest.join("_");
		}
	}
	names
}

/// A name in Pascal case, word by word as [`snake`] splits it: `track_Producer` and
/// `TrackProducer` are `TrackProducer`, and `BOXED` is `Boxed`.
pub fn pascal(name: &str) -> String {
	snake(name)
		.split('_')
		.filter(|s| !s.is_empty())
		.map(|s| {
			let mut chars = s.chars();
			match chars.next() {
				Some(first) => first.to_uppercase().chain(chars).collect(),
				None => String::new(),
			}
		})
		.collect()
}

/// The C name of the struct of an enum's variant's fields: its own in Rust's case, after
/// the enum's.
pub fn payload(name: &str, variant: &str) -> String {
	format!("{name}_{variant}")
}

/// A Rust identifier in snake case: `TrackProducer` is `track_producer`, and a name
/// already in snake case stays as it is.
pub fn snake(name: &str) -> String {
	let chars: Vec<char> = name.chars().collect();
	let mut out = String::new();
	for (i, &c) in chars.iter().enumerate() {
		if c.is_uppercase() {
			let prev = i.checked_sub(1).map(|j| chars[j]);
			let next = chars.get(i + 1);
			let boundary = match prev {
				Some(p) if p.is_lowercase() || p.is_ascii_digit() => true,
				Some(p) if p.is_uppercase() => next.is_some_and(|n| n.is_lowercase()),
				_ => false,
			};
			if boundary && !out.ends_with('_') {
				out.push('_');
			}
			out.extend(c.to_lowercase());
		} else {
			out.push(c);
		}
	}
	out
}

/// The name `what` takes in a scope whose names so far are `names`: `name`, with a trailing
/// `_` until neither `taken` nor an earlier item names it.
pub fn claim(names: &mut BTreeMap<String, String>, taken: impl Fn(&str) -> bool, name: &str, what: String) -> String {
	let mut free = name.to_string();
	while taken(&free) || names.contains_key(&free) {
		free.push('_');
	}
	names.insert(free.clone(), what);
	free
}

/// Names made distinct from each other and from what `taken` names: one taken, or an
/// earlier one's, takes a trailing `_` until it is neither, never becoming another's own.
pub fn unique(names: Vec<String>, taken: impl Fn(&str) -> bool) -> Vec<String> {
	let mut used = std::collections::BTreeSet::new();
	(names.iter())
		.map(|name| {
			let mut unique = name.clone();
			while taken(&unique) || used.contains(&unique) || (unique != *name && names.contains(&unique)) {
				unique.push('_');
			}
			used.insert(unique.clone());
			unique
		})
		.collect()
}

/// A language's names of parameters made distinct where Rust's differ only in what its
/// case drops, such as underscores: a later one takes the lowest free number after it.
pub fn distinct(names: impl IntoIterator<Item = String>) -> Vec<String> {
	let names: Vec<String> = names.into_iter().collect();
	let mut taken = std::collections::BTreeSet::new();
	(names.iter())
		.map(|name| {
			let mut unique = name.clone();
			let mut n = 2;
			while taken.contains(&unique) || (unique != *name && names.contains(&unique)) {
				unique = format!("{name}{n}");
				n += 1;
			}
			taken.insert(unique.clone());
			unique
		})
		.collect()
}

/// A name that C or C++, which includes the header, would read as a keyword with `_`
/// appended, and one starting with a digit with `_` prepended. Appended, since MSVC reads
/// `_inline`, `_asm`, and `_try` as keywords too; a name already ending in `_` takes
/// another, so no two names escape to one.
pub fn escape(name: &str) -> String {
	const KEYWORDS: &[&str] = &[
		"auto",
		"bool",
		"break",
		"case",
		"char",
		"const",
		"continue",
		"default",
		"do",
		"double",
		"else",
		"enum",
		"extern",
		"false",
		"float",
		"for",
		"goto",
		"if",
		"inline",
		"int",
		"long",
		"register",
		"restrict",
		"return",
		"short",
		"signed",
		"sizeof",
		"static",
		"struct",
		"switch",
		"true",
		"typedef",
		"union",
		"unsigned",
		"void",
		"volatile",
		"while",
		"out",
		"err",
		"future",
		"waker",
		"data",
		"alignas",
		"alignof",
		"and",
		"and_eq",
		"asm",
		"bitand",
		"bitor",
		"catch",
		"char8_t",
		"char16_t",
		"char32_t",
		"class",
		"compl",
		"concept",
		"consteval",
		"constexpr",
		"constinit",
		"const_cast",
		"co_await",
		"co_return",
		"co_yield",
		"decltype",
		"delete",
		"dynamic_cast",
		"explicit",
		"export",
		"friend",
		"mutable",
		"namespace",
		"new",
		"noexcept",
		"not",
		"not_eq",
		"nullptr",
		"operator",
		"or",
		"or_eq",
		"private",
		"protected",
		"public",
		"reinterpret_cast",
		"requires",
		"static_assert",
		"static_cast",
		"template",
		"this",
		"thread_local",
		"throw",
		"try",
		"typeid",
		"typename",
		"using",
		"virtual",
		"wchar_t",
		"xor",
		"xor_eq",
	];
	// `out0`, `out1`, ... are a returned tuple's.
	let out = name
		.strip_prefix("out")
		.is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
	match name.chars().next() {
		Some(c) if c.is_ascii_digit() => format!("_{name}"),
		_ if out || name.ends_with('_') || KEYWORDS.contains(&name) => format!("{name}_"),
		_ => name.to_string(),
	}
}

/// What a callback parameter needs, by the form it takes.
pub fn dyn_wrap(form: Dyn, expr: &str) -> String {
	match form {
		Dyn::Impl => expr.to_string(),
		Dyn::Box => format!("::std::boxed::Box::new({expr})"),
		Dyn::Arc => format!("::std::sync::Arc::new({expr})"),
	}
}

#[cfg(test)]
mod tests {
	use super::{apart, shortest, unique};

	/// Each item's [`apart`] name, by its path's segments.
	fn names(items: &[&[&str]]) -> Vec<String> {
		let items: Vec<Vec<String>> = (items.iter())
			.map(|segments| segments.iter().map(|s| s.to_string()).collect())
			.collect();
		apart(items.iter().map(Vec::as_slice))
	}

	#[test]
	fn splits_only_names_that_join_alike() {
		// The split comes after the segments the clashing names share, so one whose rest is
		// a single segment keeps its name.
		assert_eq!(
			names(&[
				&["tokio", "ConnectError"],
				&["tokio", "connect", "Error"],
				&["tokio", "Client"]
			]),
			["TokioConnectError", "TokioConnect_Error", "TokioClient"]
		);
		// Both split where neither's rest is a single segment.
		assert_eq!(names(&[&["a", "b_c", "Dx"], &["a", "b", "CDx"]]), ["ABC_Dx", "AB_CDx"]);
		// Segments apart only by case still match, for the claim to refuse.
		assert_eq!(
			names(&[&["connect", "X"], &["Connect", "X"]]),
			["Connect_X", "Connect_X"]
		);
	}

	#[test]
	fn unique_keeps_names_their_own() {
		let names = |names: &[&str], taken: &[&str]| {
			unique(names.iter().map(|n| n.to_string()).collect(), |n| taken.contains(&n))
		};
		// A later alike one, or one taken, takes a `_`, past any that is another's own.
		assert_eq!(names(&["A", "A", "A_"], &[]), ["A", "A__", "A_"]);
		assert_eq!(names(&["String", "B"], &["String"]), ["String_", "B"]);
	}

	/// Each instance's arguments, each by its path's segments, whose forms are their
	/// suffixes, shortest first.
	fn group(instances: &[&[&[&str]]]) -> Vec<Vec<Vec<String>>> {
		(instances.iter())
			.map(|args| {
				(args.iter())
					.map(|segments| {
						(1..=segments.len())
							.map(|len| segments[segments.len() - len..].join("_"))
							.collect()
					})
					.collect()
			})
			.collect()
	}

	#[test]
	fn lengthens_only_where_instances_clash() {
		// Only the clashing arguments lengthen, a segment a round, until they differ.
		let names = shortest(&group(&[
			&[&["a", "x", "foo"], &["u8"]],
			&[&["b", "x", "foo"], &["u8"]],
			&[&["foo"], &["u16"]],
			&[&["c", "bar"], &["u8"]],
		]));
		assert_eq!(names, ["a_x_foo_u8", "b_x_foo_u8", "foo_u16", "bar_u8"]);
		// One with no more segments keeps its name while the other lengthens.
		assert_eq!(
			shortest(&group(&[&[&["foo"]], &[&["polar", "foo"]]])),
			["foo", "polar_foo"]
		);
	}

	#[test]
	fn names_apart_from_order() {
		let instances: [&[&[&str]]; 3] = [&[&["x", "foo"]], &[&["y", "foo"]], &[&["bar"]]];
		let forward = shortest(&group(&instances));
		let mut reversed = shortest(&group(&[instances[2], instances[1], instances[0]]));
		reversed.reverse();
		assert_eq!(forward, ["x_foo", "y_foo", "bar"]);
		assert_eq!(forward, reversed);
	}
}
