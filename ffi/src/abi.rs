//! The C ABI the IR becomes: every C name, and every function's C signature, decided
//! once so the header and the Rust side agree.
//!
//! Names follow one rule: the prefix, then the item's path from its crate's root, each
//! segment in snake case, joined by `_`; a method's path goes through its type's. Joining
//! loses where segments split (`audio_encoder::finish` and `audio::encoder_finish` are both
//! `moq_audio_encoder_finish`), and every export shares the prefix, so every name is
//! registered, and a second item claiming one is a compile error at both spans rather
//! than a silent clash.

use std::collections::{BTreeMap, HashMap};

use rustc_span::Span;

use crate::config;
use crate::ir::{Access, Api, Dyn, Function, Int, Receiver, Type};

/// A C name claimed twice: the name, then what claimed it first and second.
pub type Clash = (String, (String, Span), (String, Span));

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
	/// `<prefix>_poll`
	Poll,
	/// A pointer the callee writes a result through.
	Out(Box<Ty>),
	/// A pointer the callee only reads through: a value's receiver.
	In(Box<Ty>),
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
	/// For a future: its handle type, its poll, and its free.
	pub future: Option<(String, Sig, Sig)>,
}

/// `ty` as a clone of each borrow it holds.
fn owned(ty: &Type) -> Type {
	match ty {
		Type::Ref(_, inner) => owned(inner),
		Type::Option(inner) => Type::Option(Box::new(owned(inner))),
		Type::Box(inner) => Type::Box(Box::new(owned(inner))),
		Type::List(inner) => Type::List(Box::new(owned(inner))),
		Type::Tuple(elements) => Type::Tuple(elements.iter().map(owned).collect()),
		ty => ty.clone(),
	}
}

/// The suffix naming each instance of a generic item, by its arguments.
type Suffixes = HashMap<Vec<Type>, String>;

pub struct Abi<'a> {
	pub api: &'a Api,
	pub prefix: String,
	/// C names for Rust paths, each starting with its crate's name, in place of the
	/// rule's: an item's, and its methods' through it, or an instance's alone.
	rename: &'a BTreeMap<String, String>,
	/// The renamed paths some name went through.
	renamed: std::cell::RefCell<std::collections::BTreeSet<String>>,
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
	/// Every C name claimed, with what claimed it.
	names: HashMap<String, (String, Span)>,
	/// Each name claimed twice.
	pub clashes: Vec<Clash>,
}

impl<'a> Abi<'a> {
	pub fn new(api: &'a Api, prefix: &str, rename: &'a BTreeMap<String, String>) -> Self {
		let mut abi = Self {
			api,
			prefix: prefix.to_string(),
			rename,
			renamed: Default::default(),
			suffixes: Default::default(),
			options: BTreeMap::new(),
			ranges: BTreeMap::new(),
			lists: BTreeMap::new(),
			tuples: BTreeMap::new(),
			names: HashMap::new(),
			clashes: Vec::new(),
		};
		abi.claim_all();
		abi
	}

	/// The C name for a Rust path, which starts with its crate's name.
	pub fn name(&self, path: &[String]) -> String {
		self.named(path).0
	}

	/// The C name for the item at `shown`, its path with its instances' arguments, and
	/// whether a rename named the instance it is alone, which no suffix then names.
	fn named(&self, shown: &[String]) -> (String, bool) {
		let (segments, alone) = self.segments(shown);
		let name = std::iter::once(self.prefix.clone()).chain(segments).collect::<Vec<_>>();
		(name.join("_"), alone)
	}

	/// The segments of the C name of the item at `shown` after the prefix: the longest
	/// renamed part, as `exclude` matches it, as one, then each segment past it in snake
	/// case; and whether the rename named the instance it is alone.
	fn segments(&self, shown: &[String]) -> (Vec<String>, bool) {
		let mut segments = Vec::new();
		let mut rest = &shown[1..];
		let mut alone = false;
		for (len, key) in config::keys(shown).take_while(|&(len, _)| len >= 2) {
			if let Some(renamed) = self.rename.get(&key) {
				alone = len == shown.len() && key.ends_with('>');
				self.renamed.borrow_mut().insert(key);
				segments.push(renamed.clone());
				rest = &shown[len..];
				break;
			}
			// No config names a foreign type, which is named by its role instead.
			if key == "serde_json::Value" {
				segments.push("json".to_string());
				rest = &shown[len..];
				break;
			}
		}
		segments.extend(
			rest.iter()
				.map(|segment| snake(segment.split('<').next().unwrap_or(segment))),
		);
		(segments, alone)
	}

	/// The key, with `prefix` before it, of the rename naming the item at `shown` itself
	/// rather than a module or type it is in.
	fn renames(&self, shown: &[String], prefix: &str) -> Option<String> {
		(config::keys(shown).take_while(|&(len, _)| len == shown.len()))
			.map(|(_, key)| format!("{prefix}{key}"))
			.find(|key| self.rename.contains_key(key))
	}

	/// The config's renames that name nothing exported.
	pub fn unused_renames(&self) -> Vec<&str> {
		let renamed = self.renamed.borrow();
		self.rename
			.keys()
			.filter(|key| !renamed.contains(*key))
			.map(String::as_str)
			.collect()
	}

	/// A constant's name for a path: the name in upper case.
	pub fn constant(&self, path: &[String]) -> String {
		self.name(path).to_uppercase()
	}

	pub fn record(&self, i: usize) -> String {
		let item = &self.api.records[i];
		self.type_instance(&item.path, &item.shown, &item.args)
	}

	pub fn enumeration(&self, i: usize) -> String {
		let item = &self.api.enums[i];
		self.type_instance(&item.path, &item.shown, &item.args)
	}

	pub fn object(&self, i: usize) -> String {
		let item = &self.api.objects[i];
		self.type_instance(&item.path, &item.shown, &item.args)
	}

	/// The C name of the instance of the type at `path` at `args`, shown as `shown`, among
	/// the type's every instance, whichever kind each is.
	fn type_instance(&self, path: &[String], shown: &[String], args: &[Type]) -> String {
		let api = self.api;
		let (name, alone) = self.named(shown);
		if alone {
			return name;
		}
		self.instance(false, name, args, || {
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
			ty => return vec![self.key(ty)],
		};
		let (segments, alone) = self.segments(shown);
		if alone {
			return segments;
		}
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

	/// A function's C name: a method's through its type's, unless renamed itself, then
	/// each argument of the instance it is. A field is renamed by `field@` and its path,
	/// or by its path unless a method or constant takes it, and its setter is named as it
	/// is, with `set_` after its type's name.
	fn function_name(&self, function: &Function, owner: Option<&Ty>) -> String {
		let owner_name = match owner {
			Some(Ty::Struct(owner) | Ty::Enum(owner) | Ty::Handle { name: owner, .. }) => Some(owner.as_str()),
			_ => None,
		};
		let Access::Field { shared, set } = function.access else {
			return self.method_name(function, owner_name, false);
		};
		let name = match self.renames(&function.shown, "field@") {
			Some(key) => {
				let renamed = format!("{}_{}", self.prefix, self.rename[&key]);
				self.renamed.borrow_mut().insert(key);
				renamed
			}
			None => self.method_name(function, owner_name, shared),
		};
		if !set {
			return name;
		}
		// A rename need not start with the type's name, but always with the prefix.
		let at = owner_name
			.filter(|owner| name.starts_with(&format!("{owner}_")))
			.unwrap_or(&self.prefix);
		format!("{at}_set_{}", &name[at.len() + 1..])
	}

	/// A function's C name through its type's, unless renamed by its path and no method
	/// or constant `shared` its name, then each argument of the instance it is, unless
	/// renamed alone.
	fn method_name(&self, function: &Function, owner: Option<&str>, shared: bool) -> String {
		let (name, alone) = match owner {
			Some(owner) if shared || self.renames(&function.shown, "").is_none() => {
				(format!("{owner}_{}", snake(function.name())), false)
			}
			_ => self.named(&function.shown),
		};
		let Some(instance) = function.instance.as_ref().filter(|_| !alone) else {
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
		self.name(&self.api.callbacks[i].path)
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
		format!("{}_{}", self.prefix, self.key(&Type::Tuple(elements.to_vec())))
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
			Type::Int(int) => int.rust().to_string(),
			Type::F32 => "f32".to_string(),
			Type::F64 => "f64".to_string(),
			Type::Char => "char".to_string(),
			Type::Duration => "duration".to_string(),
			Type::SocketAddr(_) => "socket_addr".to_string(),
			Type::Option(inner) => format!("option_{}", self.key(inner)),
			Type::List(inner) => format!("vec_{}", self.key(inner)),
			Type::Tuple(elements) => {
				let keys: Vec<String> = elements.iter().map(|e| self.key(e)).collect();
				format!("tuple_{}", keys.join("_"))
			}
			Type::Record(i) => strip(self.record(*i)),
			Type::Enum(i) => strip(self.enumeration(*i)),
			Type::Object(i) => strip(self.object(*i)),
			Type::Range(int) => format!("range_{}", int.rust()),
			Type::Bound(int) => format!("bound_{}", int.rust()),
			Type::Bounds(int) => format!("bounds_{}", int.rust()),
			Type::Ref(_, inner) | Type::Box(inner) => self.key(inner),
			other => unreachable!("no option struct holds {other:?}"),
		}
	}

	/// A type by its key, with each tuple spelled as Rust spells it, which tells apart
	/// tuples that share a C name.
	fn spell(&self, ty: &Type) -> String {
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
		matches!(ty.base(), Type::Object(_) | Type::List(_)) || Self::is_buf(ty)
	}

	/// The C type of `ty` as a value: a parameter, field, or what a function returns.
	pub fn ty(&self, ty: &Type) -> Ty {
		match ty {
			Type::Unit => Ty::Void,
			Type::Bool => Ty::Bool,
			Type::Int(int) => Ty::Int(*int),
			Type::F32 => Ty::F32,
			Type::F64 => Ty::F64,
			Type::Char => Ty::Char,
			Type::Str | Type::String | Type::Slice | Type::Vec | Type::Bytes | Type::Path | Type::PathBuf => Ty::Buf,
			Type::Duration => Ty::Duration,
			Type::SocketAddr(_) => Ty::SocketAddr,
			Type::Waker => Ty::Waker,
			Type::List(inner) => Ty::Struct(self.list(inner)),
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
			Type::Closure(params, ret) => Ty::Fn(params.iter().map(|p| self.ty(p)).collect(), Box::new(self.ty(ret))),
			Type::Range(_) | Type::Bound(_) | Type::Bounds(_) => Ty::Struct(self.range(ty)),
			Type::Tuple(elements) => Ty::Struct(self.tuple(elements)),
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
				// A value's receiver is read through a pointer, and never changed.
				value => Ty::In(Box::new(value.clone())),
			};
			params.push(("self".to_string(), ty));
		}
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
				// Taken by value, a `Copy` value is copied out, leaving its handle as it was.
				Some(i) if self.api.objects[i].copy => Ty::Handle {
					name: self.object(i),
					constant: true,
				},
				_ => self.ty(&param.ty),
			};
			let name = escape(&param.name);
			if let Type::Closure(..) = param.ty {
				// Named apart from every parameter of the function, which C requires.
				let mut data = format!("{name}_data");
				while function.params.iter().any(|p| escape(&p.name) == data) {
					data.push('_');
				}
				params.push((name, ty));
				params.push((data, Ty::Data));
				continue;
			}
			params.push((name, ty));
		}
		let mut outs = Vec::new();
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
			poll_params.extend(outs);
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
				future: Some((handle, poll, free)),
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
			(None, ok) => self.returned(ok),
		};
		Export {
			call: Sig { name, params, ret },
			future: None,
		}
	}

	fn claim(&mut self, name: String, what: String, span: Span) {
		match self.names.get(&name) {
			Some(first) => self.clashes.push((name, first.clone(), (what, span))),
			None => {
				self.names.insert(name, (what, span));
			}
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
			"buf",
			"buf_free",
			"duration",
			"socket_addr",
			"waker",
			"poll",
			"view",
			"result",
			"task",
			"future",
			"poll_t",
			"detail",
			"native",
			"bytes",
			"atomic_load",
			"atomic_store",
			"atomic_add",
			"atomic_exchange",
			"atomic_cas",
		] {
			self.claim(self.rt(runtime), "the runtime".to_string(), crate_span);
		}
		for constant in ["CONTRACT", "POLL_PENDING", "POLL_READY", "POLL_ERROR", "ATOMICS_H"] {
			let name = format!("{}_{constant}", self.prefix.to_uppercase());
			self.claim(name, "the runtime".to_string(), crate_span);
		}
		let mut types: Vec<&Type> = Vec::new();
		let mut functions: Vec<(&Function, Option<Ty>)> = Vec::new();
		for (i, record) in api.records.iter().enumerate() {
			let name = self.record(i);
			let what = format!("struct `{}`", api.rust(&record.path, &record.args));
			self.claim_traits(&name, &what, record.span, record.traits, true);
			self.claim(name.clone(), what, record.span);
			types.extend(record.fields.iter().map(|f| &f.ty));
			functions.extend(record.methods.iter().map(|m| (m, Some(Ty::Struct(name.clone())))));
		}
		for (i, e) in api.enums.iter().enumerate() {
			let name = self.enumeration(i);
			let what = format!("enum `{}`", api.rust(&e.path, &e.args));
			let upper = name.to_uppercase();
			for variant in &e.variants {
				self.claim(
					format!("{upper}_{}", snake(&variant.name).to_uppercase()),
					what.clone(),
					e.span,
				);
				types.extend(variant.fields.iter().map(|f| &f.ty));
			}
			let plain = e.plain();
			if !plain {
				self.claim(format!("{name}_tag"), what.clone(), e.span);
				// Each variant's payload, which C++ names even when it has no fields.
				for variant in &e.variants {
					self.claim(format!("{name}_{}", snake(&variant.name)), what.clone(), e.span);
				}
			}
			self.claim_traits(&name, &what, e.span, e.traits, !plain);
			self.claim(name.clone(), what, e.span);
			let owner = match plain {
				true => Ty::Enum(name.clone()),
				false => Ty::Struct(name.clone()),
			};
			functions.extend(e.methods.iter().map(|m| (m, Some(owner.clone()))));
		}
		for (i, object) in api.objects.iter().enumerate() {
			let name = self.object(i);
			let what = format!("type `{}`", api.rust(&object.path, &object.args));
			self.claim(format!("{name}_free"), what.clone(), object.span);
			self.claim_traits(&name, &what, object.span, object.traits, false);
			self.claim(name.clone(), what, object.span);
			let owner = Ty::Handle { name, constant: false };
			functions.extend(object.methods.iter().map(|m| (m, Some(owner.clone()))));
		}
		for (i, callback) in api.callbacks.iter().enumerate() {
			let name = self.callback(i);
			let what = format!("trait `{}`", callback.path.join("::"));
			self.claim(format!("{name}_vtable"), what.clone(), callback.span);
			self.claim(name, what, callback.span);
			for method in &callback.methods {
				types.extend(method.params.iter().map(|p| &p.ty));
				types.push(&method.ok);
				types.extend(&method.err);
			}
		}
		functions.extend(api.functions.iter().map(|f| (f, None)));
		for (function, owner) in functions {
			let export = self.export(function, owner.as_ref());
			let what = match function.access {
				Access::Field { set: false, .. } => format!("field `{}`", function.path.join("::")),
				Access::Field { set: true, .. } => format!("setter of field `{}`", function.path.join("::")),
				_ => format!("fn `{}`", function.path.join("::")),
			};
			self.claim(export.call.name.clone(), what.clone(), function.span);
			if let Some((handle, poll, free)) = &export.future {
				self.claim(handle.clone(), what.clone(), function.span);
				self.claim(poll.name.clone(), what.clone(), function.span);
				self.claim(free.name.clone(), what.clone(), function.span);
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
			self.claim(
				self.constant(&constant.path),
				format!("const `{}`", constant.path.join("::")),
				constant.span,
			);
		}
		let mut options = BTreeMap::new();
		let mut ranges = BTreeMap::new();
		let mut lists = BTreeMap::new();
		let mut tuples: BTreeMap<String, Vec<Type>> = BTreeMap::new();
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
				Type::List(inner) => {
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
				Type::Closure(params, ret) => {
					stack.extend(params);
					stack.push(ret);
				}
				Type::Range(_) | Type::Bound(_) => {
					ranges.insert(self.range(ty), ty.clone());
				}
				Type::Bounds(int) => {
					ranges.insert(self.range(ty), ty.clone());
					ranges.insert(self.range(&Type::Bound(*int)), Type::Bound(*int));
				}
				_ => {}
			}
		}
		for name in options.keys() {
			self.claim(name.clone(), "an option".to_string(), crate_span);
		}
		for name in ranges.keys() {
			self.claim(name.clone(), "a range".to_string(), crate_span);
		}
		for (name, elements) in &tuples {
			let what = format!("tuple `{}`", self.spell(&Type::Tuple(elements.clone())));
			self.claim(name.clone(), what, crate_span);
		}
		for (name, elements) in mixed {
			let what = format!("tuple `{}`", self.spell(&Type::Tuple(elements)));
			self.claim(name, what, crate_span);
		}
		for name in lists.keys() {
			self.claim(name.clone(), "a Vec".to_string(), crate_span);
			self.claim(format!("{name}_free"), "a Vec".to_string(), crate_span);
		}
		if ranges.values().any(|ty| matches!(ty, Type::Bound(_))) {
			let bound = self.rt("bound");
			for kind in ["", "_UNBOUNDED", "_INCLUDED", "_EXCLUDED"] {
				let name = match kind {
					"" => bound.clone(),
					kind => format!("{}{kind}", bound.to_uppercase()),
				};
				self.claim(name, "a bound".to_string(), crate_span);
			}
		}
		self.options = options;
		self.ranges = ranges;
		self.lists = lists;
		self.tuples = tuples;
	}

	/// Claim the functions a type's std traits export.
	fn claim_traits(&mut self, name: &str, what: &str, span: Span, traits: crate::ir::Traits, owned: bool) {
		let mut claim = |suffix: &str| self.claim(format!("{name}_{suffix}"), what.to_string(), span);
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

/// A name that C or C++, which includes the header, would read as a keyword, with `_`
/// prepended.
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
	match out || KEYWORDS.contains(&name) || name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
		true => format!("_{name}"),
		false => name.to_string(),
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
	use super::shortest;

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
