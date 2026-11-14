//! Python over the C ABI, through a CPython extension: `python/<prefix>.py`, and the
//! extension functions it calls, which the crate builds with its `python` feature on
//! CPython's stable ABI, so one build loads in every CPython from 3.11.
//!
//! Each name is its C name without the prefix, as Python spells it: `moq_track_producer`
//! is `moq.TrackProducer`, and its method `moq_track_producer_append_group` is
//! `TrackProducer.append_group`. A keyword, or a member Python gives every such class,
//! takes a trailing `_`. An object is a class owning its handle, freed once its last
//! reference goes; a record a dataclass; an enum an `Enum`, or for
//! one with data a class with a dataclass per variant; a trait an abstract class; a
//! closure any callable; a reader a `ForeignReader`; a future an awaitable; an error an
//! exception. The runtime they
//! share, [`runtime/rt.py`](../runtime/rt.py), holds the awaitable and the queue an event
//! loop reads its wakes from; the glue's, [`runtime/python.rs`](../runtime/python.rs),
//! the conversions and the types the extension defines.
//!
//! The glue converts each Python value to its C value and back itself, so a call is one
//! crossing: a record crosses as its fields, read and set by name, and bytes returned as a
//! `memoryview` lending Rust's buffer. Each function but an `async def` is the
//! extension's own, which `_register` sets on its class or the module binds, so Python
//! calls it with no frame of its own; the Python only declares it, for type checkers.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write;
use std::hash::Hasher;

use rustc_data_structures::stable_hash::StableHasher;

use crate::abi::{self, Abi, Ty, pascal};
use crate::ir::{Access, Family, Function, Int, Literal, On, Receiver, Traits, Type};
use crate::kotlin::{indent_by, rust_c, strip_refs};

const RUNTIME: &str = include_str!("../runtime/rt.py");
const NATIVE: &str = include_str!("../runtime/native.py");
pub const GLUE_RUNTIME: &str = include_str!("../runtime/python.rs");

/// What the generator writes: the Python module, and the Rust module of the extension it
/// loads.
pub struct Output {
	pub names: BTreeMap<String, String>,
	pub python: String,
	pub glue: String,
}

/// The runtime's names the generated Python uses unqualified, which no exported item may
/// take. Its `Waker`, `Mut`, and `Bound` need no entry: the C ABI claims `<prefix>_waker`,
/// `<prefix>_mut`, and `<prefix>_bound` already.
const TAKEN: &[&str] = &[
	"_n",
	"_load",
	"_T",
	"_Future",
	"_Pollable",
	"_Wakeup",
	"_wakeups",
	"_wakeup",
	"_resume",
	"_Const",
	"_PENDING",
	"_consts",
	"_abc",
	"_array",
	"_asyncio",
	"_dataclass",
	"_same",
	"_all",
	"_dict",
	"_float",
	"_int",
	"_memoryview",
	"_TypeError",
	"_list",
	"_tuple",
	"_type",
	"_zip",
	"_datetime",
	"_enum",
	"_socket",
	"_sys",
	"_typing",
	"_weakref",
	"_get_running_loop",
	"_property",
	"_staticmethod",
	"_isinstance",
	"_str",
	"_bytes",
	"_len",
	"_Exception",
	"_ValueError",
	"_AttributeError",
	"ForeignReader",
];

/// Any buffer, as Python before 3.12's `collections.abc.Buffer` spells it.
const BUFFER: &str = "bytes | bytearray | memoryview";

/// A socket address as Python's `socket` module holds it: IPv4's, and IPv6's.
const V4: &str = "tuple[str, int]";
const V6: &str = "tuple[str, int, int, int]";

/// The registry's first entries, the runtime's: the glue's `T_FUTURE`, `T_TIMEDELTA`,
/// `T_RESUME`, `T_WAKER`, and `T_POLLABLE`.
const RUNTIME_TYPES: &[&str] = &["_Future", "_datetime.timedelta", "_resume", "Waker", "_Pollable"];

/// The Python for `abi`, whose header's contract is `contract`, or the names two items
/// claim alike. The Python's own contract hashes the header's with everything it and its
/// glue add, as their conversions change with no C declaration changing.
pub fn generate(abi: &Abi<'_>, packages: &str, contract: u64) -> Result<Output, String> {
	let mut g = Gen {
		locked: BTreeMap::new(),
		abi,
		classes: abi.classes(|_| (String::new(), 0), |name| TAKEN.contains(&name)),
		py: String::new(),
		top: String::new(),
		consts: Vec::new(),
		natives: Vec::new(),
		glue: String::new(),
		table: Vec::new(),
		members: Vec::new(),
		types: RUNTIME_TYPES.iter().map(|t| t.to_string()).collect(),
		index: BTreeMap::new(),
		keys: BTreeSet::new(),
		exceptions: BTreeMap::new(),
		thrown: HashMap::new(),
		names: BTreeMap::new(),
		converters: HashMap::new(),
		errors: Vec::new(),
		on: On::ALL,
	};
	for name in TAKEN {
		g.names.insert(name.to_string(), "the runtime".to_string());
	}
	for name in g.classes.values() {
		g.names.insert(name.clone(), "a type".to_string());
	}
	g.locked = crate::lock::types(abi, &g.classes);
	g.write();
	if !g.errors.is_empty() {
		return Err(g.errors.join("\n"));
	}
	let p = &abi.prefix;
	let registry: String = g.types.iter().map(|t| format!("\t\t{t},\n")).collect();
	let consts: String = g
		.consts
		.iter()
		.map(|c| format!("\t\t\"{}\": _n.{},\n", c.name, c.bare))
		.collect();
	let declared: String = match g.consts.is_empty() {
		true => "\tpass\n".to_string(),
		false => g
			.consts
			.iter()
			.map(|c| format!("\t{}: {}\n{}", c.name, c.ty, docstring(&c.docs, "\t")))
			.collect(),
	};
	// What only runs: the registry, and the module's constants, declared above for type
	// checkers and made by `__getattr__` on each read.
	let mut body = format!(
		"{RUNTIME}\n{}{}if _typing.TYPE_CHECKING:\n{declared}else:\n\t_n._register(\n{registry}\t)\n\n\t_consts = {{\n{consts}\t}}\n\n\tdef __getattr__(name: str) -> object:\n\t\t\"\"\"A constant, made anew each time it is read, as Rust makes it.\"\"\"\n\t\tget = _consts.get(name)\n\t\tif get is None:\n\t\t\traise _AttributeError(f\"module {{__name__!r}} has no attribute {{name!r}}\")\n\t\treturn get()\n",
		g.py, g.top
	);
	let mut hasher = StableHasher::new();
	hasher.write_u64(contract);
	for part in [&body, &g.glue, GLUE_RUNTIME] {
		hasher.write(part.as_bytes());
	}
	let contract = Hasher::finish(&hasher);
	let native = indent_by(NATIVE, 1);
	let natives: String = g.natives.iter().map(|n| format!("\n{}", indent_by(n, 2))).collect();
	let head = format!(
		"# Generated by rust2-ffi: Python over the C ABI of the pub API of {packages}. Do not edit.\n\
		\"\"\"The pub API of {packages}, over the CPython extension built into the library beside this file.\"\"\"\n\n\
		from __future__ import annotations\n\n\
		import abc as _abc\n\
		import array as _array\n\
		import asyncio as _asyncio\n\
		import datetime as _datetime\n\
		import enum as _enum\n\
		import inspect as _inspect\n\
		import socket as _socket\n\
		import sys as _sys\n\
		import typing as _typing\n\
		import weakref as _weakref\n\
		from builtins import AttributeError as _AttributeError\n\
		from builtins import Exception as _Exception\n\
		from builtins import TypeError as _TypeError\n\
		from builtins import ValueError as _ValueError\n\
		from builtins import all as _all\n\
		from builtins import bytes as _bytes\n\
		from builtins import dict as _dict\n\
		from builtins import float as _float\n\
		from builtins import int as _int\n\
		from builtins import isinstance as _isinstance\n\
		from builtins import len as _len\n\
		from builtins import list as _list\n\
		from builtins import memoryview as _memoryview\n\
		from builtins import property as _property\n\
		from builtins import staticmethod as _staticmethod\n\
		from builtins import str as _str\n\
		from builtins import tuple as _tuple\n\
		from builtins import type as _type\n\
		from builtins import zip as _zip\n\
		from dataclasses import dataclass as _dataclass\n\n\
		_get_running_loop = _asyncio.get_running_loop\n\
		_T = _typing.TypeVar(\"_T\")\n\n\n\
		def _same(a: object, b: object, /) -> bool:\n\
		\t\"\"\"Whether a and b are equal as Rust's `==` compares them: never by identity, so a NaN\n\
		\tequals nothing, while a list's, tuple's, or dict's own `==` takes an item as equal to itself.\"\"\"\n\
		\tif _isinstance(a, (_list, _tuple)):\n\
		\t\txs = _typing.cast(\"_typing.Sequence[object]\", a)\n\
		\t\tys = _typing.cast(\"_typing.Sequence[object]\", b)\n\
		\t\treturn _type(xs) is _type(ys) and _len(xs) == _len(ys) and _all(_same(x, y) for x, y in _zip(xs, ys))\n\
		\tif _isinstance(a, _dict):\n\
		\t\tif not _isinstance(b, _dict):\n\
		\t\t\treturn False\n\
		\t\tm = _typing.cast(\"_typing.Mapping[object, object]\", a)\n\
		\t\tn = _typing.cast(\"_typing.Mapping[object, object]\", b)\n\
		\t\treturn m.keys() == n.keys() and _all(_same(v, n[k]) for k, v in m.items())\n\
		\tif a is None or b is None or _isinstance(a, (_int, _float, _str)):\n\
		\t\treturn a == b\n\
		\ttry:\n\
		\t\t# Any buffer, through a memoryview, which compares numbers as Rust does, where\n\
		\t\t# numpy's own `==` compares item by item into another array.\n\
		\t\treturn _memoryview(_typing.cast(\"bytes\", a)) == _memoryview(_typing.cast(\"bytes\", b))\n\
		\texcept _TypeError:\n\
		\t\treturn a == b\n\n\n\
		def _load() -> object:\n\
		\timport importlib.machinery\n\
		\timport importlib.util\n\
		\timport os\n\n\
		\tname = {{\"win32\": \"{p}.dll\", \"darwin\": \"lib{p}.dylib\"}}.get(_sys.platform, \"lib{p}.so\")\n\
		\tpath = os.path.join(os.path.dirname(os.path.abspath(__file__)), name)\n\
		\tloader = importlib.machinery.ExtensionFileLoader(\"_{p}\", path)\n\
		\tmodule = importlib.util.module_from_spec(importlib.machinery.ModuleSpec(\"_{p}\", loader, origin=path))\n\
		\tloader.exec_module(module)\n\
		\treturn module\n\n\n\
		if _typing.TYPE_CHECKING:\n\
		{native}{natives}\
		else:\n\
		\t_n = _load()\n\
		\t# Checked before any other call: a Python file and a library from different builds\n\
		\t# must not mix.\n\
		\tif _n.contract() != 0x{contract:016x}:\n\
		\t\traise ImportError(\"{p}.py and the library are from different builds\")\n\
		_PENDING = _n.PENDING\n\n\n"
	);
	body.insert_str(0, &head);
	let glue = format!(
		"//! Generated by rust2-ffi: the CPython extension `python/{p}.py` loads, over the C ABI of {packages}.\n\n\
		#![allow(non_snake_case, non_upper_case_globals, unused_mut, unused_unsafe, unused_parens, unused_variables, clippy::all)]\n\n\
		mod rt;\n\n\
		use rt::*;\n\n\
		unsafe extern \"C\" fn f_contract(_: *mut PyObject, _: *mut *mut PyObject, _: Py_ssize_t) -> *mut PyObject {{\n\
		\trt::wrap(|| rt::uint_out(0x{contract:016x}))\n\
		}}\n\n\
		{}\
		static METHODS: rt::Methods<{}> = rt::Methods(std::cell::UnsafeCell::new([\n\
		\trt::method(c\"contract\", f_contract),\n\
		{}\
		\trt::END,\n\
		]));\n\n\
		static MEMBERS: rt::Gil<[rt::Member; {}]> = rt::Gil::new([\n\
		{}\
		]);\n\n\
		static MODULE: rt::Gil<PyModuleDef> = rt::Gil::new(PyModuleDef {{\n\
		\tm_base: PyModuleDef_HEAD_INIT,\n\
		\tm_name: c\"_{p}\".as_ptr(),\n\
		\tm_doc: std::ptr::null(),\n\
		\tm_size: -1,\n\
		\tm_methods: std::ptr::null_mut(),\n\
		\tm_slots: std::ptr::null_mut(),\n\
		\tm_traverse: None,\n\
		\tm_clear: None,\n\
		\tm_free: None,\n\
		}});\n\n\
		#[unsafe(no_mangle)]\n\
		pub unsafe extern \"C\" fn PyInit__{p}() -> *mut PyObject {{\n\
		\tunsafe {{\n\
		\t\tlet def = MODULE.get();\n\
		\t\tdef.m_methods = METHODS.ptr();\n\
		\t\trt::module(def, MEMBERS.get())\n\
		\t}}\n\
		}}\n",
		{
			let keys: String = g
				.keys
				.iter()
				.map(|k| format!("static K_{k}: rt::Key = rt::Key::new(c\"{k}\");\n"))
				.collect();
			format!("{keys}\n{}{}", g.index_consts(), g.glue)
		},
		g.table.len() + 2,
		g.table.iter().map(|t| format!("\t{t},\n")).collect::<String>(),
		g.members.len(),
		g.members.iter().map(|m| format!("\t{m},\n")).collect::<String>(),
	);
	Ok(Output {
		names: g.locked,
		python: body,
		glue,
	})
}

/// How a function's Python form is held.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
	/// A method of a class, taking `self`.
	Member,
	/// A static method of a class.
	Static,
	/// A module-level function.
	Top,
}

/// What a native function Python calls directly is.
struct Direct<'s> {
	/// Its Python name.
	member: &'s str,
	place: Place,
	/// Whether it is a read-only property.
	property: bool,
	/// The C name of the class it is a member of, which `_register` sets it on.
	owner: Option<&'s str>,
	/// Its parameters' Python names, after its `self`.
	params: &'s [String],
	docs: &'s str,
}

/// A module-level constant that is no literal, made by its native function on each read.
struct Const {
	name: String,
	bare: String,
	/// Its Python annotation.
	ty: String,
	docs: String,
}

/// A function's Python form.
enum Form {
	/// Python's own definition.
	Python(String),
	/// The glue's, declared for type checkers, and the statement binding it at run time
	/// unless `_register` sets it on its class.
	Native(String, Option<String>),
}

/// A class's members: those Python defines, and those the glue does, which the class
/// declares only for type checkers and binds in `runtime` unless `_register` does.
#[derive(Default)]
struct Body {
	python: Vec<String>,
	native: Vec<String>,
	runtime: Vec<String>,
}

impl Body {
	fn codes(&self) -> impl Iterator<Item = &String> {
		self.python.iter().chain(&self.native)
	}

	/// Each member's code, the glue's in one block.
	fn members(&self) -> Vec<String> {
		let mut members = self.python.clone();
		if !self.native.is_empty() {
			let native: Vec<String> = self.native.iter().map(|m| indent_by(m, 1)).collect();
			members.push(format!("\tif _typing.TYPE_CHECKING:\n{}", native.join("\n")));
		}
		if !self.runtime.is_empty() {
			let runtime: Vec<String> = self.runtime.iter().map(|m| indent_by(m, 1)).collect();
			members.push(format!("\telse:\n{}", runtime.join("")));
		}
		members
	}
}

struct Gen<'a> {
	locked: BTreeMap<String, String>,
	abi: &'a Abi<'a>,
	/// Each exported type's name, by its C name.
	classes: HashMap<String, String>,
	/// The classes, then the module-level functions.
	py: String,
	top: String,
	/// The module-level constants that are no literal.
	consts: Vec<Const>,
	/// The glue's functions the Python calls by name, declared for type checkers.
	natives: Vec<String>,
	glue: String,
	/// The extension's method table entries.
	table: Vec<String>,
	/// The members `_register` sets on their classes.
	members: Vec<String>,
	/// The classes the glue makes values of, in the order the Python registers them.
	types: Vec<String>,
	/// Each registered class's index, by the C name or key it is registered under.
	index: BTreeMap<String, usize>,
	/// The attribute names the glue reads and sets.
	keys: BTreeSet<String>,
	/// The exception class of each error type, by name.
	exceptions: BTreeMap<String, Type>,
	/// Each exception class's name, by the name it would take but for a clash.
	thrown: HashMap<String, String>,
	/// Each module-level Python name, with what claimed it.
	names: BTreeMap<String, String>,
	/// The suffix of the glue's `r_` and `w_` of each option, tuple, and list type by
	/// its C struct and what the struct holds: types of one struct, such as a `Vec` of
	/// `SocketAddrV4` and one of `SocketAddr`, convert apart.
	converters: HashMap<(String, Vec<Type>), String>,
	errors: Vec<String>,
	/// The targets of the item being written, whose native functions check theirs.
	on: On,
}

impl<'a> Gen<'a> {
	/// A C name without the prefix.
	fn bare<'n>(&self, c: &'n str) -> &'n str {
		&c[self.abi.prefix.len() + 1..]
	}

	/// The Python class name of the C type `c`.
	fn class(&self, c: &str) -> String {
		self.classes.get(c).cloned().unwrap_or_else(|| pascal(self.bare(c)))
	}

	/// The module-level Python name `what` takes, `name` unless that is taken.
	fn claim(&mut self, name: &str, what: String) -> String {
		abi::claim(&mut self.names, |_| false, name, what)
	}

	/// Register the class the Python expression `py` names under `key`, returning the
	/// glue's constant for its index.
	fn register(&mut self, key: &str, py: String) -> String {
		if !self.index.contains_key(key) {
			self.index.insert(key.to_string(), self.types.len());
			self.types.push(py);
		}
		format!("T_{key}")
	}

	/// The glue's constant for the index of the class registered under `key`.
	fn t(&self, key: &str) -> String {
		format!("T_{key}")
	}

	fn index_consts(&self) -> String {
		let mut out = String::new();
		for (key, n) in &self.index {
			let _ = writeln!(out, "const T_{key}: usize = {n};");
		}
		out.push('\n');
		out
	}

	/// The glue's interned attribute name `name`.
	fn key(&mut self, name: &str) -> String {
		self.keys.insert(name.to_string());
		format!("&K_{name}")
	}

	/// The Python annotation of `ty` as a value, owning what it holds.
	fn py_ty(&self, ty: &Type) -> String {
		let abi = self.abi;
		match ty {
			Type::Const(_) => unreachable!("a const argument is no value"),
			Type::Array(n) => Abi::array_class(*n),
			Type::Unit => "None".to_string(),
			Type::Bool => "bool".to_string(),
			Type::Int(_) | Type::NonZero(_) | Type::U128 => "int".to_string(),
			Type::F32 | Type::F64 => "float".to_string(),
			Type::Char | Type::Str | Type::String | Type::Path | Type::PathBuf => "str".to_string(),
			Type::Slice | Type::Vec | Type::Bytes => "memoryview".to_string(),
			Type::Duration => "_datetime.timedelta".to_string(),
			Type::SocketAddr(family) => match family {
				Family::V4 => V4.to_string(),
				Family::V6 => V6.to_string(),
				Family::Any => format!("{V4} | {V6}"),
			},
			Type::List(_, inner) if ty.block().is_some() => format!("_array.array[{}]", self.py_ty(inner)),
			Type::List(_, inner) => format!("list[{}]", self.py_ty(inner)),
			Type::Waker => "Waker".to_string(),
			Type::Reader => "ForeignReader".to_string(),
			Type::Future => "_typing.Awaitable[None]".to_string(),
			Type::Option(inner) => format!("{} | None", self.py_ty(inner)),
			Type::Box(inner) | Type::Ref(_, inner) => self.py_ty(inner),
			Type::Record(i) => self.class(&abi.record(*i)),
			Type::Enum(i) => self.class(&abi.enumeration(*i)),
			Type::Object(i) => self.class(&abi.object(*i)),
			Type::Callback(i, _) => self.class(&abi.callback(*i)),
			Type::Closure(closure_) => {
				let (params, ret) = (&closure_.params, &closure_.ok);
				let params: Vec<String> = params.iter().map(|p| self.py_lent(p)).collect();
				format!("_typing.Callable[[{}], {}]", params.join(", "), self.py_param(ret))
			}
			Type::Range(_) => "range".to_string(),
			Type::Bound(_) | Type::Bounds(_) => self.class(&abi.range(ty)),
			Type::Tuple(elements) => {
				let elements: Vec<String> = elements.iter().map(|e| self.py_ty(e)).collect();
				format!("tuple[{}]", elements.join(", "))
			}
		}
	}

	/// The Python annotation of a parameter of type `ty`, or of what Python returns to Rust:
	/// its bytes are any buffer, its path any path, and its list any iterable, which
	/// `rt::list_in` reads once.
	fn py_param(&self, ty: &Type) -> String {
		self.py_in(ty, "_typing.Iterable")
	}

	/// The Python annotation of a record's field of type `ty`: as a parameter's, but its list
	/// is a sequence, since the record keeps it and Rust may read it more than once.
	fn py_field(&self, ty: &Type) -> String {
		self.py_in(ty, "_typing.Sequence")
	}

	/// The Python annotation of a value of type `ty` that Python passes in, each list as the
	/// covariant container `list` names, so a caller's `list` of one alternative of a widened
	/// element type checks.
	fn py_in(&self, ty: &Type, list: &str) -> String {
		match ty {
			Type::Slice | Type::Vec | Type::Bytes => BUFFER.to_string(),
			Type::Path | Type::PathBuf => "str".to_string(),
			// Python's own IPv6 address may leave out its flowinfo and scope_id.
			Type::SocketAddr(Family::V6) => format!("{V4} | {V6}"),
			// Any buffer of its numbers, as numpy's arrays are too.
			Type::List(..) if ty.block().is_some() => format!("{} | memoryview", self.py_ty(ty)),
			Type::List(_, inner) => format!("{list}[{}]", self.py_in(inner, list)),
			Type::Option(inner) => format!("{} | None", self.py_in(inner, list)),
			Type::Box(inner) | Type::Ref(_, inner) => self.py_in(inner, list),
			Type::Tuple(elements) => {
				let elements: Vec<String> = elements.iter().map(|e| self.py_in(e, list)).collect();
				format!("tuple[{}]", elements.join(", "))
			}
			_ => self.py_ty(ty),
		}
	}

	/// The glue's expression, an `R<C>`, reading the Python object `o` as the C value of
	/// `ty`, with what it borrows held in the scope `s`; `m` is the glue's `bool`, whether
	/// the position moves an object's value out, closing it, and `t`, whether Rust takes
	/// what the position holds, as it takes a value passed by value but not one read
	/// through a pointer or an array's item, so a buffer crosses there as its own.
	fn lift(&mut self, ty: &Type, o: &str, m: &str, t: &str) -> String {
		let abi = self.abi;
		match ty {
			Type::Const(_) => unreachable!("a const argument is no value"),
			Type::Array(n) => format!(
				"rt::array_in::<{n}>({o}).map(|items| crate::{} {{ items }})",
				abi.array(*n)
			),
			Type::Bool => format!("rt::bool_in({o})"),
			Type::Int(int) | Type::NonZero(int) => format!("rt::int_in::<{}>({o})", int.rust()),
			Type::F32 => format!("rt::float_in({o}).map(|f| f as f32)"),
			Type::F64 => format!("rt::float_in({o})"),
			Type::Char => format!("rt::char_in({o})"),
			Type::Str | Type::String => format!("rt::str_in({o})"),
			Type::Slice | Type::Vec | Type::Bytes => format!("rt::bytes_in({o}, s, {t})"),
			Type::Path | Type::PathBuf => format!("rt::path_in({o}, s)"),
			Type::U128 => format!("rt::u128_in({o})"),
			Type::Duration => format!("rt::duration_in({o})"),
			Type::SocketAddr(family) => format!("rt::addr_in({o}, {})", family_of(*family)),
			Type::List(_, inner) if ty.block().is_some() => format!("rt::block_in::<{}>({o}, s)", number(inner)),
			Type::List(..) => format!("r_{}({o}, s, {m}, {t})", self.converter(ty)),
			Type::Option(inner) => match inner.base() {
				Type::Object(i) => {
					let m = match abi.api.objects[*i].copy || matches!(**inner, Type::Ref(..)) {
						true => "false",
						false => m,
					};
					format!("rt::hold_opt({o}, {}, s, {m})", self.t(&abi.object(*i)))
				}
				base if Abi::is_buf(base) => {
					let some = self.lift(inner, o, m, t);
					format!("(if rt::is_none({o}) {{ Ok(crate::rt::Buf::NONE) }} else {{ {some} }})")
				}
				Type::List(..) => {
					let some = self.lift(inner, o, m, t);
					format!("(if rt::is_none({o}) {{ Ok(crate::rt::List::NONE) }} else {{ {some} }})")
				}
				_ => format!("r_{}({o}, s, {m}, {t})", self.converter(ty)),
			},
			Type::Box(inner) => self.lift(inner, o, m, t),
			Type::Ref(_, inner) => self.lift(inner, o, "false", t),
			Type::Record(_) => format!("r_{}({o}, s, {t})", self.c_struct(ty)),
			Type::Enum(i) if abi.api.enums[*i].variants.iter().any(|v| !abi.api.everywhere(v.on)) => {
				// Checks the member a value is against the library's target.
				let c = abi.enumeration(*i);
				match abi.api.enums[*i].plain() {
					true => format!("rt::member_in({o}, {}).and_then(on_{c})", self.t(&c)),
					false => format!("r_{c}({o}, s, {t})"),
				}
			}
			Type::Enum(i) => match abi.api.enums[*i].plain() {
				true => format!("rt::member_in({o}, {})", self.t(&abi.enumeration(*i))),
				false => format!("r_{}({o}, s, {t})", abi.enumeration(*i)),
			},
			Type::Object(i) => {
				let m = match abi.api.objects[*i].copy {
					true => "false",
					false => m,
				};
				format!("rt::hold({o}, {}, s, {m})", self.t(&abi.object(*i)))
			}
			Type::Callback(i, _) => {
				let c = abi.callback(*i);
				format!(
					"Ok(crate::{c} {{ vtable: &VTABLE_{c}, data: rt::callback({o}, {}, s)? }})",
					self.t(&c)
				)
			}
			Type::Waker => format!("rt::waker_in({o}, s)"),
			Type::Reader => format!("rt::reader_in({o})"),
			Type::Future => format!("rt::future_in({o}, s)"),
			Type::Range(int) => format!("rt::range_in::<{}>({o})", int.rust()),
			Type::Bound(_) | Type::Bounds(_) => format!("r_{}({o}, s)", abi.range(ty)),
			Type::Tuple(_) => format!("r_{}({o}, s, {m}, {t})", self.converter(ty)),
			Type::Unit | Type::Closure(..) => unreachable!("{ty:?} is no value Python passes"),
		}
	}

	/// The glue's expression, an `R<Obj>`, of the C value `v` of `ty`, which Rust handed
	/// over: what it owns is freed or moves into the Python value.
	fn lower(&mut self, ty: &Type, v: &str) -> String {
		let abi = self.abi;
		match ty {
			Type::Const(_) => unreachable!("a const argument is no value"),
			Type::Array(n) => format!("rt::array_out(&{v}.items, {})", self.t(&abi.array(*n))),
			Type::Unit => "rt::none()".to_string(),
			Type::Bool => format!("rt::bool_out({v})"),
			Type::Int(int) | Type::NonZero(int) if signed(*int) => format!("rt::int_out({v} as i64)"),
			Type::Int(_) | Type::NonZero(_) => format!("rt::uint_out({v} as u64)"),
			Type::F32 | Type::F64 => format!("rt::float_out({v} as f64)"),
			Type::Char => format!("rt::char_out({v})"),
			Type::Str | Type::String => format!("rt::str_out({v})"),
			Type::Slice | Type::Vec | Type::Bytes => format!("rt::bytes_out({v})"),
			Type::Path | Type::PathBuf => format!("rt::path_out({v})"),
			Type::U128 => format!("rt::u128_out({v})"),
			Type::Duration => format!("rt::duration_out({v})"),
			Type::SocketAddr(_) => format!("rt::addr_out({v})"),
			Type::List(_, inner) if ty.block().is_some() => format!("rt::block_out::<{}>({v})", number(inner)),
			Type::List(..) => format!("w_{}({v})", self.converter(ty)),
			Type::Option(inner) => match inner.base() {
				Type::Object(i) => format!(
					"rt::object_opt({v}, {}, crate::{}_free)",
					self.t(&abi.object(*i)),
					abi.object(*i)
				),
				base if Abi::is_buf(base) || matches!(base, Type::List(..)) => {
					let some = self.lower(inner, "v");
					format!("{{ let v = {v}; if v.is_none() {{ rt::none() }} else {{ {some} }} }}")
				}
				_ => format!("w_{}({v})", self.converter(ty)),
			},
			Type::Box(inner) | Type::Ref(_, inner) => self.lower(inner, v),
			Type::Record(_) => format!("w_{}({v})", self.c_struct(ty)),
			Type::Enum(i) => match abi.api.enums[*i].plain() {
				true => format!("rt::member({}, {v} as i64)", self.t(&abi.enumeration(*i))),
				false => format!("w_{}({v})", abi.enumeration(*i)),
			},
			Type::Object(i) => format!(
				"rt::object({v}, {}, crate::{}_free)",
				self.t(&abi.object(*i)),
				abi.object(*i)
			),
			Type::Range(int) => {
				let start = self.lower(&Type::Int(*int), "r.start");
				let end = self.lower(&Type::Int(*int), "r.end");
				format!("{{ let r = {v}; rt::range_out({start}, {end}) }}")
			}
			Type::Bound(_) | Type::Bounds(_) => format!("w_{}({v})", abi.range(ty)),
			Type::Tuple(_) => format!("w_{}({v})", self.converter(ty)),
			Type::Waker => format!("rt::waker_out({v})"),
			Type::Callback(..) | Type::Closure(..) | Type::Future | Type::Reader => {
				unreachable!("{ty:?} is no value Rust returns")
			}
		}
	}

	/// The C name of the struct `ty` crosses as.
	fn c_struct(&self, ty: &Type) -> String {
		match self.abi.ty(&strip_refs(ty)) {
			Ty::Struct(name) => name,
			other => unreachable!("{ty:?} crosses as {other:?}, no struct"),
		}
	}

	/// The suffix of the glue's `r_` and `w_` converting the option, tuple, or list `ty`,
	/// written here the first time, so a block's struct gets no item-by-item glue: its C
	/// struct's name for the type the struct was made for, else a name of its own.
	fn converter(&mut self, ty: &Type) -> String {
		let abi = self.abi;
		let c = self.c_struct(ty);
		let (held, made): (Vec<Type>, Vec<Type>) = match strip_refs(ty) {
			Type::Option(inner) => (vec![*inner], vec![abi.options[&c].clone()]),
			Type::List(_, elem) => (vec![*elem], vec![abi.lists[&c].clone()]),
			Type::Tuple(elements) => (elements, abi.tuples[&c].clone()),
			other => unreachable!("{other:?} has no converter"),
		};
		let held: Vec<Type> = held.iter().map(strip_refs).collect();
		let key = (c.clone(), held.clone());
		if let Some(name) = self.converters.get(&key) {
			return name.clone();
		}
		// A numbered name must miss every converter's: each type's own, and the others numbered.
		let taken = |name: &str| {
			abi.options.contains_key(name)
				|| abi.lists.contains_key(name)
				|| abi.tuples.contains_key(name)
				|| abi.ranges.contains_key(name)
				|| abi.arrays.contains_key(name)
				|| (0..abi.api.records.len()).any(|i| abi.record(i) == name)
				|| (0..abi.api.enums.len()).any(|i| abi.enumeration(i) == name)
				|| (0..abi.api.objects.len()).any(|i| abi.object(i) == name)
				|| self.converters.values().any(|n| n == name)
		};
		let made: Vec<Type> = made.iter().map(strip_refs).collect();
		let name = match held == made {
			true => c.clone(),
			false => (self.converters.len() + 2..)
				.map(|k| format!("{c}_{k}"))
				.find(|name| !taken(name))
				.expect("some number is free"),
		};
		self.converters.insert(key, name.clone());
		match strip_refs(ty) {
			Type::Option(_) => self.option(&name, &c, &held[0]),
			Type::List(..) => self.list(&name, &c, &held[0]),
			_ => self.tuple(&name, &c, &held),
		}
		name
	}

	fn write(&mut self) {
		let abi = self.abi;
		let api = abi.api;

		if abi.ranges.values().any(|ty| matches!(ty, Type::Bound(_))) {
			let bound = abi.rt("bound");
			self.register(&bound, "Bound".to_string());
			self.py.push_str(
				"class Bound(_enum.Enum):\n\t\"\"\"What a range's start or end bound does: nothing, include its value, or exclude it.\"\"\"\n\n\tUNBOUNDED = 0\n\tINCLUDED = 1\n\tEXCLUDED = 2\n\n\n",
			);
		}
		for (c, ty) in &abi.ranges.clone() {
			self.range(c, ty);
		}
		for (c, n) in &abi.arrays {
			let name = Abi::array_class(*n);
			self.register(c, name.clone());
			let _ = write!(
				self.py,
				"class {name}(_bytes):\n\t\"\"\"A Rust `[u8; {n}]`: exactly {n} bytes.\"\"\"\n\n\t__slots__ = ()\n\n\tdef __new__(cls, value: {BUFFER}) -> _typing.Self:\n\t\tself = _bytes.__new__(cls, value)\n\t\tif _len(self) != {n}:\n\t\t\traise _ValueError(f\"{{_len(self)}} bytes, not {n}\")\n\t\treturn self\n\n\n"
			);
		}
		for constant in &api.constants {
			let name = self.bare(&abi.name(&constant.path)).to_uppercase();
			let name = self.claim(&name, format!("const `{}`", constant.path.join("::")));
			self.locked
				.insert(format!("const {}", constant.path.join("::")), name.clone());
			let value = match constant.value {
				Literal::Bool(true) => "True".to_string(),
				Literal::Bool(false) => "False".to_string(),
				Literal::Int(_, v) => v.to_string(),
			};
			let _ = write!(self.top, "{name} = {value}\n{}\n", docstring(&constant.docs, ""));
		}
		for i in 0..api.records.len() {
			self.record(i);
		}
		for i in 0..api.enums.len() {
			self.enumeration(i);
		}
		for i in 0..api.objects.len() {
			self.object(i);
		}
		for i in 0..api.callbacks.len() {
			self.callback(i);
		}
		// The glue's functions, declared for type checkers and bound to its own.
		let (mut declared, mut bound) = (Vec::new(), Vec::new());
		for function in &api.functions {
			match self.function(function, None, None, Place::Top) {
				Some(Form::Native(code, binding)) => {
					bound.push(indent_by(&binding.expect("a module's function is bound"), 1));
					declared.push(indent_by(&code, 1));
				}
				Some(Form::Python(code)) => {
					self.top.push_str(&code);
					self.top.push_str("\n\n");
				}
				None => {}
			}
		}
		if !declared.is_empty() {
			let _ = write!(
				self.top,
				"if _typing.TYPE_CHECKING:\n{}\nelse:\n{}\n\n",
				declared.join("\n\n"),
				bound.concat()
			);
		}
		for (name, ty) in &self.exceptions.clone() {
			let py = self.py_ty(ty);
			let _ = write!(
				self.py,
				"class {name}(_Exception):\n\t\"\"\"A `{py}` Rust returned as an error, in `error`, whose `str` is the exception's.\"\"\"\n\n\tdef __init__(self, error: {py}) -> None:\n\t\t_Exception.__init__(self, error)\n\t\tself.error = error\n\n\tdef __str__(self) -> str:\n\t\treturn _str(self.error)\n\n\n"
			);
		}
	}

	/// A range's bound or pair of bounds as a dataclass, and its glue; a `Range` is
	/// Python's `range`.
	fn range(&mut self, c: &str, ty: &Type) {
		let name = self.class(c);
		let (fields, read, write) = match ty {
			Type::Range(_) => return,
			Type::Bound(int) => {
				let bound = self.abi.rt("bound");
				let (kind, value) = (self.key("kind"), self.key("value"));
				let t_bound = self.t(&bound);
				(
					"\tkind: Bound\n\tvalue: int\n".to_string(),
					format!(
						"let k = rt::field(o, {t}, {kind}, s)?;\n\tlet v = rt::attr(o, {value}, s)?;\n\tOk(crate::{c} {{ kind: rt::member_in(k, {t_bound})?, value: rt::int_in::<{r}>(v)? }})",
						t = self.t(c),
						r = int.rust()
					),
					format!(
						"rt::record({t}, vec![({kind}, rt::member({t_bound}, v.kind as i64)), ({value}, {})])",
						self.lower(&Type::Int(*int), "v.value"),
						t = self.t(c)
					),
				)
			}
			Type::Bounds(int) => {
				let bound = self.abi.range(&Type::Bound(*int));
				let (start, end) = (self.key("start"), self.key("end"));
				let class = self.class(&bound);
				(
					format!("\tstart: {class}\n\tend: {class}\n"),
					format!(
						"let a = rt::field(o, {t}, {start}, s)?;\n\tlet b = rt::attr(o, {end}, s)?;\n\tOk(crate::{c} {{ start: r_{bound}(a, s)?, end: r_{bound}(b, s)? }})",
						t = self.t(c)
					),
					format!(
						"rt::record({t}, vec![({start}, w_{bound}(v.start)), ({end}, w_{bound}(v.end))])",
						t = self.t(c)
					),
				)
			}
			other => unreachable!("{other:?} is no range"),
		};
		self.register(c, name.clone());
		let _ = write!(self.py, "@_dataclass(slots=True)\nclass {name}:\n{fields}\n\n");
		let _ = write!(
			self.glue,
			"fn r_{c}(o: *mut PyObject, s: &mut rt::Scope) -> R<crate::{c}> {{\n\t{read}\n}}\n\nfn w_{c}(v: crate::{c}) -> R<Obj> {{\n\t{write}\n}}\n\n"
		);
	}

	/// The glue of an option struct `c` holding `inner`, as `r_<name>` and `w_<name>`.
	fn option(&mut self, name: &str, c: &str, inner: &Type) {
		let read = self.lift(inner, "o", "m", "take");
		let write = self.lower(inner, "v.value");
		let _ = write!(
			self.glue,
			"fn r_{name}(o: *mut PyObject, s: &mut rt::Scope, m: bool, take: bool) -> R<crate::{c}> {{\n\tmatch rt::is_none(o) {{\n\t\ttrue => Ok(crate::{c} {{ some: false, value: unsafe {{ std::mem::zeroed() }} }}),\n\t\tfalse => Ok(crate::{c} {{ some: true, value: {}? }}),\n\t}}\n}}\n\nfn w_{name}(v: crate::{c}) -> R<Obj> {{\n\tmatch v.some {{\n\t\ttrue => {write},\n\t\tfalse => rt::none(),\n\t}}\n}}\n\n",
			read
		);
	}

	/// The glue of a tuple struct `c`, element by element, as `r_<name>` and `w_<name>`.
	fn tuple(&mut self, name: &str, c: &str, elements: &[Type]) {
		let n = elements.len();
		let mut reads = Vec::new();
		let mut fields = Vec::new();
		let mut writes = Vec::new();
		for (k, ty) in elements.iter().enumerate() {
			let lift = self.lift(ty, &format!("x{k}"), "m", "take");
			reads.push(format!("let x{k} = rt::tuple_item(o, {k}, {n})?;"));
			fields.push(format!("_{k}: {lift}?"));
			writes.push(self.lower(ty, &format!("v._{k}")));
		}
		let _ = write!(
			self.glue,
			"fn r_{name}(o: *mut PyObject, s: &mut rt::Scope, m: bool, take: bool) -> R<crate::{c}> {{\n\t{}\n\tOk(crate::{c} {{ {} }})\n}}\n\nfn w_{name}(v: crate::{c}) -> R<Obj> {{\n\trt::tuple(vec![{}])\n}}\n\n",
			reads.join("\n\t"),
			fields.join(", "),
			writes.join(", ")
		);
	}

	/// The glue of a `Vec`'s array `c`, as `r_<name>` and `w_<name>`.
	fn list(&mut self, name: &str, c: &str, elem: &Type) {
		// An array's items are behind its pointer, which Rust only reads.
		let read = self.lift(elem, "o", "m", "false");
		let write = self.lower(elem, "e");
		let _ = write!(
			self.glue,
			"fn r_{name}(o: *mut PyObject, s: &mut rt::Scope, m: bool, take: bool) -> R<crate::{c}> {{\n\trt::list_in(o, s, |o, s| {read})\n}}\n\nfn w_{name}(v: crate::{c}) -> R<Obj> {{\n\trt::list_out(v, |e| {write})\n}}\n\n"
		);
	}

	/// The Python members a type's std traits export beyond Python's own, with their glue:
	/// `__str__` from `Display`, `__repr__` from `Debug`, a static `default` from
	/// `Default`, and `clone` and `__eq__` for an object.
	fn traits(&mut self, ty: &Type, c: &str, t: Traits, body: &mut Body) {
		let bare = self.bare(c).to_string();
		let class = self.class(c);
		let object = matches!(ty, Type::Object(_));
		let opaque = matches!(ty, Type::Record(i) if self.abi.api.records[*i].opaque.is_some());
		// The receiver as the glue reads it: a handle, held for the call, or a value.
		let this = match object {
			true => self.lift(
				&Type::Ref(crate::ir::Mutability::Not, Box::new(ty.clone())),
				"a[0]",
				"false",
				"false",
			),
			false => self.lift(ty, "a[0]", "false", "false"),
		};
		let pass = match object {
			true => "this",
			false => "&this",
		};
		let unary = |g: &mut Self, suffix: &str, member: &str, docs: &str, lower: String| {
			let d = Direct {
				member,
				place: Place::Member,
				property: false,
				owner: Some(c),
				params: &[],
				docs,
			};
			g.direct(
				&format!("{bare}_{suffix}"),
				d,
				&format!(
					"let this = {this}?;\n\t\tlet ret = rt::call(|| crate::{c}_{suffix}({pass}));\n\t\ts.done();\n\t\t{lower}"
				),
			);
		};
		if t.display {
			unary(self, "to_string", "__str__", "", "rt::str_out(ret)".to_string());
			body.native.push("\tdef __str__(self) -> str: ...\n".to_string());
		}
		if t.debug {
			unary(self, "debug", "__repr__", "", "rt::str_out(ret)".to_string());
			body.native.push("\tdef __repr__(self) -> str: ...\n".to_string());
		}
		if object && t.clone {
			let lower = self.lower(ty, "ret");
			let docs = "A new handle to a clone.";
			unary(self, "clone", "clone", docs, lower);
			body.native.push(format!(
				"\tdef clone(self) -> {class}:\n\t\t\"\"\"{docs}\"\"\"\n\t\t...\n"
			));
		}
		// An opaque value compares through Rust, as an object does; a record's fields
		// compare as a dataclass's.
		if (object || opaque) && t.eq {
			let other = match object {
				true => self.lift(
					&Type::Ref(crate::ir::Mutability::Not, Box::new(ty.clone())),
					"a[1]",
					"false",
					"false",
				),
				false => self.lift(ty, "a[1]", "false", "false"),
			};
			let (a, b) = match object {
				true => ("this", "other"),
				false => ("&this", "&other"),
			};
			self.native(
				&format!("{bare}_eq"),
				2,
				&format!(
					"let this = {this}?;\n\t\tlet other = {other}?;\n\t\tlet ret = rt::call(|| crate::{c}_eq({a}, {b}));\n\t\ts.done();\n\t\trt::bool_out(ret)"
				),
			);
			// Python makes a class that defines `__eq__` alone unhashable, as `Eq` without `Hash` is.
			body.python.push(format!(
				"\tdef __eq__(self, other: object) -> bool:\n\t\treturn _isinstance(other, {class}) and _n.{bare}_eq(self, other)\n"
			));
			self.natives.push(format!(
				"@_staticmethod\ndef {bare}_eq(this: {class}, other: {class}, /) -> bool: ...\n"
			));
		}
		if t.default {
			let lower = self.lower(ty, "ret");
			let docs = "Rust's `Default`.";
			let d = Direct {
				member: "default",
				place: Place::Static,
				property: false,
				owner: Some(c),
				params: &[],
				docs,
			};
			self.direct(
				&format!("{bare}_default"),
				d,
				&format!("let ret = rt::call(|| crate::{c}_default());\n\t\ts.done();\n\t\t{lower}"),
			);
			body.native.push(format!(
				"\t@_staticmethod\n\tdef default() -> {class}:\n\t\t\"\"\"{docs}\"\"\"\n\t\t...\n"
			));
		}
	}

	/// The methods of a type, each a member or a static method by its receiver, after its
	/// members `body`, checked to differ from them and its data's names `data`, which a
	/// method's name takes a trailing `_` to keep apart from, as from those of `reserved`.
	fn methods(
		&mut self,
		methods: &[Function],
		(ty, owner, rust): (&Type, &Ty, &str),
		data: &[String],
		reserved: &[&str],
		body: &mut Body,
	) {
		for method in methods {
			let place = match method.receiver {
				Some(_) => Place::Member,
				None => Place::Static,
			};
			// A name its data or an earlier member takes, its own takes a trailing `_` against.
			let earlier: Vec<String> = body.codes().filter_map(|code| member_name(code)).collect();
			let reserved: Vec<&str> = (reserved.iter().copied())
				.chain(data.iter().map(String::as_str))
				.chain(earlier.iter().map(String::as_str))
				.collect();
			match self.function(method, Some(owner), Some((ty, &reserved)), place) {
				Some(Form::Native(code, binding)) => {
					body.native.push(indent_by(&code, 1));
					body.runtime.extend(binding.map(|b| indent_by(&b, 1)));
				}
				Some(Form::Python(code)) => body.python.push(indent_by(&code, 1)),
				None => {}
			}
		}
		self.distinct(rust, data, body.codes());
	}

	/// Check that the members `codes` of the type `rust` and its data's names `data` differ.
	fn distinct<'c>(&mut self, rust: &str, data: &[String], codes: impl Iterator<Item = &'c String>) {
		let mut seen = BTreeSet::new();
		for name in data.iter().cloned().chain(codes.filter_map(|code| member_name(code))) {
			if !seen.insert(name.clone()) {
				self.errors.push(format!(
					"two members of `{rust}` are both `{name}` in Python, which rust2-ffi should keep apart"
				));
			}
		}
	}

	fn record(&mut self, i: usize) {
		let abi = self.abi;
		let record = &abi.api.records[i];
		self.on = record.on;
		let c = abi.record(i);
		let name = self.class(&c);
		let rust = abi.api.rust(&record.path, &record.args);
		let t = self.register(&c, name.clone());
		let ty = Type::Record(i);
		let mut body = Body::default();
		if record.opaque.is_some() {
			body.python.push("\t__slots__ = ()\n".to_string());
		}
		self.traits(&ty, &c, record.traits, &mut body);
		let owner = Ty::Struct(c.clone());
		let data: Vec<String> = record.fields.iter().map(|f| field_name(&f.name)).collect();
		self.methods(&record.methods, (&ty, &owner, &rust), &data, &[], &mut body);
		if record.fields.iter().any(|f| own_eq(abi, &f.ty)) {
			body.python.push(ieee_eq(&name, &data));
		}
		if record.opaque.is_some() {
			let text = format!(
				"{rust}\n\n{}\n\nIts bytes, which only Rust makes or reads: copy it, but never make one.",
				record.docs
			);
			let _ = write!(
				self.py,
				"class {name}(_n.Value):\n{}{}\n\n",
				docstring(text.trim(), "\t"),
				body.members().join("\n")
			);
			let _ = write!(
				self.glue,
				"fn r_{c}(o: *mut PyObject, _s: &mut rt::Scope, _take: bool) -> R<crate::{c}> {{\n\trt::value_in(o, {t})\n}}\n\nfn w_{c}(v: crate::{c}) -> R<Obj> {{\n\trt::value_out({t}, v)\n}}\n\n"
			);
			return;
		}
		let fields: String = record
			.fields
			.iter()
			.map(|f| {
				format!(
					"\t{}: {}\n{}",
					field_name(&f.name),
					self.py_field(&f.ty),
					docstring(&f.docs, "\t")
				)
			})
			.collect();
		let repr = match record.traits.debug {
			true => ", repr=False",
			false => "",
		};
		let _ = write!(
			self.py,
			"@_dataclass(slots=True{repr})\nclass {name}:\n{}{fields}{}\n\n",
			docstring(&format!("{rust}\n\n{}", record.docs), "\t"),
			body.members().iter().map(|m| format!("\n{m}")).collect::<String>()
		);
		// The glue.
		let mut reads = Vec::new();
		let mut fields = Vec::new();
		let mut writes = Vec::new();
		for (n, field) in record.fields.iter().enumerate() {
			let key = self.key(&field_name(&field.name));
			let read = match n {
				0 => format!("rt::field(o, {t}, {key}, s)?"),
				_ => format!("rt::attr(o, {key}, s)?"),
			};
			reads.push(format!("let x{n} = {read};"));
			fields.push(format!(
				"f{n}: {}?",
				self.lift(&field.ty, &format!("x{n}"), "false", "take")
			));
			writes.push(format!("({key}, {})", self.lower(&field.ty, &format!("v.f{n}"))));
		}
		if record.fields.is_empty() {
			reads.push(format!("rt::check(o, {t})?;"));
		}
		let _ = write!(
			self.glue,
			"fn r_{c}(o: *mut PyObject, s: &mut rt::Scope, take: bool) -> R<crate::{c}> {{\n\t{}\n\tOk(crate::{c} {{ {} }})\n}}\n\nfn w_{c}(v: crate::{c}) -> R<Obj> {{\n\trt::record({t}, vec![{}])\n}}\n\n",
			reads.join("\n\t"),
			fields.join(", "),
			writes.join(", ")
		);
	}

	fn enumeration(&mut self, i: usize) {
		let abi = self.abi;
		let e = &abi.api.enums[i];
		self.on = e.on;
		let c = abi.enumeration(i);
		let name = self.class(&c);
		let rust = abi.api.rust(&e.path, &e.args);
		let t = self.register(&c, name.clone());
		let ty = Type::Enum(i);
		let mut body = Body::default();
		self.traits(&ty, &c, e.traits, &mut body);
		let owner = abi.ty(&ty);
		if e.plain() {
			let missing: String = (e.variants.iter())
				.filter_map(|v| {
					let cfg = abi.cfg(v.on)?;
					let d = v.discriminant;
					Some(format!(
						"\t\t#[cfg(not({cfg}))]\n\t\t{d} => rt::missing(\"{c}.{}\"),\n",
						v.name
					))
				})
				.collect();
			if !missing.is_empty() {
				let _ = write!(
					self.glue,
					"fn on_{c}(v: i32) -> R<i32> {{\n\tmatch v {{\n{missing}\t\t_ => Ok(v),\n\t}}\n}}\n\n"
				);
			}
			let data: Vec<String> = e.variants.iter().map(|v| abi::snake(&v.name).to_uppercase()).collect();
			// An `Enum`'s members have these.
			self.methods(&e.methods, (&ty, &owner, &rust), &data, &["name", "value"], &mut body);
			let entries: String = e
				.variants
				.iter()
				.map(|v| format!("\t{} = {}\n", abi::snake(&v.name).to_uppercase(), v.discriminant))
				.collect();
			let _ = write!(
				self.py,
				"class {name}(_enum.Enum):\n{}{entries}{}\n\n",
				docstring(&format!("{rust}\n\n{}", e.docs), "\t"),
				body.members().iter().map(|m| format!("\n{m}")).collect::<String>()
			);
			return;
		}
		// Each variant's class, and its fields, which on an instance hide the enum's members.
		let mut data: Vec<String> = e.variants.iter().map(|v| pascal(&v.name)).collect();
		let fields: BTreeSet<String> = e
			.variants
			.iter()
			.flat_map(|v| &v.fields)
			.map(|f| field_name(&f.name))
			.collect();
		data.extend(fields);
		self.methods(&e.methods, (&ty, &owner, &rust), &data, &[], &mut body);
		// Each variant's class is bound on the enum's once made, after the enum's own.
		for variant in &e.variants {
			let vname = pascal(&abi::snake(&variant.name));
			body.native
				.push(format!("\t{vname}: _typing.TypeAlias = \"_{name}_{vname}\"\n"));
		}
		let _ = write!(
			self.py,
			"class {name}:\n{}\t__slots__ = ()\n{}\n\n",
			docstring(&format!("{rust}\n\n{}", e.docs), "\t"),
			body.members().iter().map(|m| format!("\n{m}")).collect::<String>()
		);
		let repr = match e.traits.debug {
			true => ", repr=False",
			false => "",
		};
		let mut reads = Vec::new();
		let mut writes = Vec::new();
		for (n, variant) in e.variants.iter().enumerate() {
			let vname = pascal(&variant.name);
			let class = format!("_{name}_{vname}");
			let key = abi::payload(&c, &variant.name);
			let tv = self.register(&key, class.clone());
			let fields: String = variant
				.fields
				.iter()
				.map(|f| format!("\t{}: {}\n", field_name(&f.name), self.py_field(&f.ty)))
				.collect();
			let fields = match fields.is_empty() {
				true => "\tpass\n".to_string(),
				false if variant.fields.iter().any(|f| own_eq(abi, &f.ty)) => {
					let names: Vec<String> = variant.fields.iter().map(|f| field_name(&f.name)).collect();
					format!("{fields}\n{}", ieee_eq(&class, &names))
				}
				false => fields,
			};
			let _ = write!(
				self.py,
				"@_dataclass(slots=True{repr})\nclass {class}({name}):\n{fields}\n\n{class}.__name__ = \"{vname}\"\n{class}.__qualname__ = \"{name}.{vname}\"\nif not _typing.TYPE_CHECKING:\n\t{name}.{vname} = {class}\n\n\n"
			);
			// A value of a variant the library's target lacks is refused as it crosses.
			let missing = match abi.cfg(variant.on) {
				Some(cfg) => format!(
					"if !cfg!({cfg}) {{\n\t\t\treturn rt::missing(\"{c}.{}\");\n\t\t}}\n\t\t",
					variant.name
				),
				None => String::new(),
			};
			match variant.fields.is_empty() {
				true => {
					reads.push(format!(
						"if rt::is(o, {tv}) {{\n\t\t{missing}return Ok(crate::{c} {{ tag: {n}, u: unsafe {{ std::mem::zeroed() }} }});\n\t}}"
					));
					writes.push(format!("\t\t{n} => rt::record({tv}, vec![]),"));
				}
				false => {
					let payload = abi::payload(&c, &variant.name);
					let mut binds = Vec::new();
					let mut fields = Vec::new();
					let mut lowers = Vec::new();
					for (k, f) in variant.fields.iter().enumerate() {
						let key = self.key(&field_name(&f.name));
						binds.push(format!("let x{k} = rt::attr(o, {key}, s)?;"));
						fields.push(format!(
							"f{k}: {}?",
							self.lift(&f.ty, &format!("x{k}"), "false", "take")
						));
						lowers.push(format!("({key}, {})", self.lower(&f.ty, &format!("p.f{k}"))));
					}
					reads.push(format!(
						"if rt::is(o, {tv}) {{\n\t\t{missing}{}\n\t\treturn Ok(crate::{c} {{ tag: {n}, u: crate::{c}_union {{ v{n}: crate::{payload} {{ {} }} }} }});\n\t}}",
						binds.join("\n\t\t"),
						fields.join(", ")
					));
					writes.push(format!(
						"\t\t{n} => {{\n\t\t\tlet p = unsafe {{ v.u.v{n} }};\n\t\t\trt::record({tv}, vec![{}])\n\t\t}}",
						lowers.join(", ")
					));
				}
			}
		}
		let _ = write!(
			self.glue,
			"fn r_{c}(o: *mut PyObject, s: &mut rt::Scope, take: bool) -> R<crate::{c}> {{\n\t{}\n\trt::check(o, {t})?;\n\tcrate::rt::fail(\"python\", \"a variant is of no class of its enum\")\n}}\n\nfn w_{c}(v: crate::{c}) -> R<Obj> {{\n\tmatch v.tag {{\n{}\n\t\t_ => crate::rt::fail(\"python\", \"an enum's tag is invalid\"),\n\t}}\n}}\n\n",
			reads.join("\n\t"),
			writes.join("\n")
		);
	}

	fn object(&mut self, i: usize) {
		let abi = self.abi;
		let object = &abi.api.objects[i];
		self.on = object.on;
		let c = abi.object(i);
		let name = self.class(&c);
		let rust = abi.api.rust(&object.path, &object.args);
		self.register(&c, name.clone());
		let ty = Type::Object(i);
		let mut body = Body {
			python: vec!["\t__slots__ = ()\n".to_string()],
			..Body::default()
		};
		self.traits(&ty, &c, object.traits, &mut body);
		let owner = Ty::Handle {
			name: c.clone(),
			constant: false,
		};
		self.methods(&object.methods, (&ty, &owner, &rust), &[], &[], &mut body);
		let text = format!(
			"{rust}\n\n{}\n\nIts handle is freed once its last reference goes. A call after one that moved its value out raises.",
			object.docs
		);
		let _ = write!(
			self.py,
			"class {name}(_n.Object):\n{}{}\n\n",
			docstring(text.trim(), "\t"),
			body.members().join("\n")
		);
	}

	fn callback(&mut self, i: usize) {
		let abi = self.abi;
		let callback = &abi.api.callbacks[i];
		self.on = callback.on;
		let c = abi.callback(i);
		let name = self.class(&c);
		let rust = callback.path.join("::");
		self.register(&c, name.clone());
		let mut body = Vec::new();
		let mut table = vec!["free: Some(rt::free_callback)".to_string()];
		for (n, method) in callback.methods.iter().enumerate() {
			let member = member(&abi::snake(method.name()), &[]);
			crate::lock::function(&mut self.locked, method, &member);
			let params: Vec<String> = std::iter::once("self".to_string())
				.chain(
					method
						.params
						.iter()
						.map(|p| format!("{}: {}", param_name(&p.name), self.py_lent(&p.ty))),
				)
				.collect();
			body.push(format!(
				"\t@_abc.abstractmethod\n\tdef {member}({}) -> {}:\n{}\t\t...\n",
				params.join(", "),
				self.py_param(&method.ok),
				docstring(&method.docs, "\t\t")
			));
			let c_params: Vec<Ty> = method.params.iter().map(|p| abi.ty(&p.ty)).collect();
			let c_ret = abi.ty(&method.ok);
			let lent: Vec<Type> = method.params.iter().map(|p| p.ty.clone()).collect();
			let trampoline = format!("py_{c}_{n}");
			let key = self.key(&member);
			self.trampoline(
				&trampoline,
				&format!("{rust}::{}", method.name()),
				(&lent, &method.ok),
				(&c_params, &c_ret),
				Some(&key),
				None,
			);
			table.push(format!("{}: Some({trampoline})", method.name()));
		}
		self.distinct(&rust, &[], body.iter());
		let text = format!(
			"{rust}: implemented in Python. Rust calls its methods from any thread, and once it\ndrops the implementation, calls its `close()` if it has one.\n\n{}",
			callback.docs
		);
		let _ = write!(
			self.py,
			"class {name}(_abc.ABC):\n{}\t__slots__ = ()\n\n{}\n\n",
			docstring(text.trim(), "\t"),
			body.join("\n")
		);
		let _ = write!(
			self.glue,
			"static VTABLE_{c}: crate::{c}_vtable = crate::{c}_vtable {{ {} }};\n\n",
			table.join(", ")
		);
	}

	/// The Python annotation of a value Rust lends a callback or closure: its bytes copied.
	fn py_lent(&self, ty: &Type) -> String {
		match ty.base() {
			Type::Slice | Type::Vec | Type::Bytes => "bytes".to_string(),
			Type::Option(inner) if matches!(inner.base(), Type::Slice | Type::Vec | Type::Bytes) => {
				"bytes | None".to_string()
			}
			_ => self.py_ty(ty),
		}
	}

	/// The Rust function C calls for a callback's method or a closure, named `trampoline`,
	/// which calls the Python value Rust holds: for a callback, its method `key`, and for a
	/// closure, the callable itself. A closure returning a `Result` writes its value
	/// through an out, and fails with the `error` of the exception class `exception`
	/// registers that it raises.
	fn trampoline(
		&mut self,
		trampoline: &str,
		what: &str,
		(lent, ret): (&[Type], &Type),
		(c_params, c_ret): (&[Ty], &Ty),
		key: Option<&str>,
		exception: Option<&str>,
	) {
		let mut rust_params = vec!["data: *mut std::ffi::c_void".to_string()];
		let mut args = Vec::new();
		for (k, (ty, c)) in lent.iter().zip(c_params).enumerate() {
			rust_params.push(format!("p{k}: {}", rust_c(self.abi, c)));
			let p = format!("p{k}");
			args.push(match (c, strip_refs(&strip_box(ty))) {
				(Ty::Handle { .. }, Type::Object(i)) => format!("rt::lent({p}, {}, s)", self.t(&self.abi.object(i))),
				(Ty::Buf, _) => self.lent_buf(ty, &p),
				_ => self.lower(ty, &p),
			});
		}
		let call = match key {
			Some(key) => format!("rt::call_method(data.cast(), {key}, vec![{}])", args.join(", ")),
			None => format!("rt::call_py(data.cast(), vec![{}])", args.join(", ")),
		};
		let outs = &c_params[lent.len()..];
		let (call, back) = match exception {
			Some(exception) => {
				let err = outs.len() - 1;
				for (k, c) in outs.iter().enumerate() {
					let Ty::Out(inner) = c else {
						unreachable!("a closure's {c:?} after what it takes is an out")
					};
					rust_params.push(format!("out{k}: *mut {}", rust_c(self.abi, inner)));
				}
				let call = format!(
					"match {call} {{\n\t\t\tOk(r) => r,\n\t\t\tErr(rt::Raised) => {{\n\t\t\t\tout{err}.write(rt::closure_error({exception})?);\n\t\t\t\treturn Ok(false);\n\t\t\t}}\n\t\t}}"
				);
				let back = match err {
					0 => "Ok(true)".to_string(),
					_ => format!("out0.write({}?);\n\t\tOk(true)", self.given(ret, "r.ptr()")),
				};
				(call, back)
			}
			None => {
				let back = match c_ret {
					Ty::Void => "Ok(())".to_string(),
					_ => self.given(ret, "r.ptr()"),
				};
				(format!("{call}?"), back)
			}
		};
		let rust_ret = rust_c(self.abi, c_ret);
		let _ = write!(
			self.glue,
			"unsafe extern \"C\" fn {trampoline}({}) -> {rust_ret} {{\n\trt::upcall(\"{what}\", || unsafe {{\n\t\tlet s = &mut rt::Scope::default();\n\t\tlet r = {call};\n\t\t{back}\n\t}})\n}}\n\n",
			rust_params.join(", "),
		);
	}

	/// The glue's expression, an `R<C>`, of the Python object `o` a closure returns as the
	/// C value of `ty` that Rust then owns: a string copied, as `<prefix>_buf_copy` makes
	/// one.
	fn given(&mut self, ty: &Type, o: &str) -> String {
		match ty {
			Type::String => format!("rt::str_given({o})"),
			_ => self.lift(ty, o, "false", "false"),
		}
	}

	/// The registry's index of the exception class an error of type `e` raises.
	fn exception(&mut self, e: &Type) -> String {
		let name = self.thrown(e);
		self.t(&format!("exc_{name}"))
	}

	/// The name of the exception class an error of type `e` raises, made once per type.
	fn thrown(&mut self, e: &Type) -> String {
		let py = self.py_ty(e);
		// Named for Rust's type, as Python's `str` reads as no type of its own.
		let name = match e {
			Type::Str | Type::String => "StringException".to_string(),
			_ => format!("{}Exception", pascal_type(&py)),
		};
		let thrown = match self.thrown.get(&name) {
			Some(thrown) => thrown.clone(),
			None => {
				let thrown = self.claim(&name, format!("the exception of `{py}`"));
				self.thrown.insert(name, thrown.clone());
				self.exceptions.insert(thrown.clone(), e.clone());
				self.register(&format!("exc_{thrown}"), thrown.clone());
				thrown
			}
		};
		self.locked.insert(crate::lock::error_key(self.abi, e), thrown.clone());
		thrown
	}

	/// The glue's expression, an `R<Obj>`, of a buffer `p` Rust lends for a call: text
	/// is copied as when returned, and bytes into `bytes`, since a `memoryview` could
	/// outlive the call.
	fn lent_buf(&mut self, ty: &Type, p: &str) -> String {
		match strip_refs(ty) {
			Type::Slice | Type::Vec | Type::Bytes => format!("rt::lent_bytes({p})"),
			Type::Option(inner) if matches!(*inner, Type::Slice | Type::Vec | Type::Bytes) => {
				format!("(if {p}.is_none() {{ rt::none() }} else {{ rt::lent_bytes({p}) }})")
			}
			_ => self.lower(ty, p),
		}
	}

	/// A native function `bare`, which the generated Python calls by position with `n`
	/// arguments as `a`, whose glue runs `body` with a scope `s`, returning an `R<Obj>`.
	fn native(&mut self, bare: &str, n: usize, body: &str) {
		let body = self.checked(bare, body);
		let _ = write!(
			self.glue,
			"unsafe extern \"C\" fn f_{bare}(_: *mut PyObject, a: *mut *mut PyObject, n: Py_ssize_t) -> *mut PyObject {{\n\trt::wrap(|| unsafe {{\n\t\tlet a = rt::args(a, n, {n}, \"{bare}\")?;\n\t\tlet s = &mut rt::Scope::default();\n\t\t{body}\n\t}})\n}}\n\n"
		);
		self.table.push(format!("rt::method(c\"{bare}\", f_{bare})"));
	}

	/// `body`, raising first on a target the item being written is not on.
	fn checked(&self, bare: &str, body: &str) -> String {
		match self.abi.cfg(self.on) {
			Some(cfg) => format!(
				"if !cfg!({cfg}) {{\n\t\t\treturn rt::missing(\"{}_{bare}\");\n\t\t}}\n\t\t{body}",
				self.abi.prefix
			),
			None => body.to_string(),
		}
	}

	/// A native function `bare` that Python calls as `d` describes, with no Python
	/// frame between. Its glue runs `body` like a [`Gen::native`]'s, with `a` holding
	/// `self` first if it has one.
	fn direct(&mut self, bare: &str, d: Direct<'_>, body: &str) {
		let body = &self.checked(bare, body);
		let member = d.member;
		let class = d.owner.map(|c| (self.t(c), self.class(c)));
		if d.property {
			let (t, _) = class.expect("a property has its class");
			let _ = write!(
				self.glue,
				"unsafe extern \"C\" fn f_{bare}(this: *mut PyObject, _: *mut std::ffi::c_void) -> *mut PyObject {{\n\trt::wrap(|| unsafe {{\n\t\tlet a = [this];\n\t\tlet s = &mut rt::Scope::default();\n\t\t{body}\n\t}})\n}}\n\n"
			);
			let doc = match d.docs.trim() {
				"" => "None".to_string(),
				docs => format!("Some({})", c_str(docs)),
			};
			self.members.push(format!(
				"rt::Member::Get({t}, rt::property(c\"{member}\", {doc}, f_{bare}))"
			));
			return;
		}
		let method = d.place == Place::Member;
		let qual = match &class {
			Some((_, name)) => format!("{name}.{member}"),
			None => member.to_string(),
		};
		let names: Vec<String> = d.params.iter().map(|p| format!("\"{p}\"")).collect();
		let _ = write!(
			self.glue,
			"unsafe extern \"C\" fn f_{bare}(this: *mut PyObject, a: *const *mut PyObject, n: Py_ssize_t, kw: *mut PyObject) -> *mut PyObject {{\n\trt::wrap(|| unsafe {{\n\t\tlet a = rt::params::<{}>({}, a, n, kw, &[{}], \"{qual}\")?;\n\t\tlet s = &mut rt::Scope::default();\n\t\t{body}\n\t}})\n}}\n\n",
			d.params.len() + method as usize,
			if method { "Some(this)" } else { "None" },
			names.join(", "),
		);
		// The signature `inspect.signature` reads, before the docstring.
		let params: Vec<&str> = method
			.then_some("$self")
			.into_iter()
			.chain(d.params.iter().map(String::as_str))
			.collect();
		let doc = c_str(&format!("{member}({})\n--\n\n{}", params.join(", "), d.docs.trim()));
		let def = format!("rt::function(c\"{member}\", {doc}, f_{bare})");
		match (class, d.place) {
			(Some((t, _)), Place::Member) => self.members.push(format!("rt::Member::Method({t}, {def})")),
			(Some((t, _)), Place::Static) => self.members.push(format!("rt::Member::Static({t}, {def})")),
			_ => self.table.push(def),
		}
	}

	/// `function`'s Python form, a member of `owner` if it has one (whose IR type and
	/// reserved member names are `owner_ty`), with its native function and glue.
	fn function(
		&mut self,
		function: &Function,
		owner: Option<&Ty>,
		owner_ty: Option<(&Type, &[&str])>,
		place: Place,
	) -> Option<Form> {
		let on = std::mem::replace(&mut self.on, function.on);
		let form = self.function_on(function, owner, owner_ty, place);
		self.on = on;
		form
	}

	fn function_on(
		&mut self,
		function: &Function,
		owner: Option<&Ty>,
		owner_ty: Option<(&Type, &[&str])>,
		place: Place,
	) -> Option<Form> {
		let abi = self.abi;
		// A value is immutable in Python, so its `&mut self` method returns it as the call
		// left it, calling the C function in place on a copy it hands back.
		let handed = (owner.zip(owner_ty)).and_then(|(owner, (ty, _))| abi.handed_back(function, owner, ty));
		let (back, export, handed) = match handed {
			Some((back, export, handed)) => (Some(back), export, Some(handed)),
			None => (None, abi.export(function, owner), None),
		};
		let function = back.as_ref().unwrap_or(function);
		let c_name = export.call.name.clone();
		let bare = self.bare(&c_name).to_string();
		let owner_c = match owner {
			Some(Ty::Struct(n) | Ty::Enum(n) | Ty::Handle { name: n, .. }) => Some(n.clone()),
			_ => None,
		};
		let own = match &owner_c {
			Some(o) => export.named.strip_prefix(o.as_str()).and_then(|r| r.strip_prefix('_')),
			None => None,
		};
		let reserved = owner_ty.map_or(&[][..], |(_, r)| r);
		// Named as Rust's path is, whatever C's clashes rename.
		let named = self.bare(&export.named).to_string();
		let snake = own.unwrap_or(&named);
		let member = member(&abi::snake(snake), reserved);
		let constant = function.access == Access::Const && function.params.is_empty() && place != Place::Member;
		let member = match (place, constant) {
			(Place::Top, true) => self.claim(&member.to_uppercase(), format!("const `{}`", function.path.join("::"))),
			(Place::Top, false) => self.claim(&member, format!("fn `{}`", function.path.join("::"))),
			_ => member,
		};

		crate::lock::function(&mut self.locked, function, &member);

		// The glue: each argument read, the call, and its value lowered.
		let mut stmts = Vec::new();
		let mut c_args = Vec::new();
		let mut py_params = Vec::new();
		let mut py_args = Vec::new();
		// Its native function's parameters, declared for type checkers by position, so no
		// Rust name clashes with the receiver's.
		let mut native_params = Vec::new();
		let mut n = 0;
		let mut c_params = export.call.params.iter();
		let receiver = function.receiver.filter(|_| owner.is_some());
		if let Some(receiver) = receiver {
			let (_, c) = c_params.next().expect("a method has its receiver");
			let ty = owner_ty.expect("a method has its type").0.clone();
			py_params.push("self".to_string());
			py_args.push("self".to_string());
			native_params.push(format!("p0: {}", self.py_ty(&ty)));
			match c {
				Ty::Handle { .. } => {
					let ty = match receiver {
						Receiver::Owned => ty,
						Receiver::Ref => Type::Ref(crate::ir::Mutability::Not, Box::new(ty)),
						Receiver::Mut => Type::Ref(crate::ir::Mutability::Mut, Box::new(ty)),
					};
					stmts.push(format!(
						"let p{n} = {}?;",
						self.lift(&ty, &format!("a[{n}]"), "true", "true")
					));
					c_args.push(format!("p{n}"));
				}
				_ => {
					// Read through a pointer, so Rust takes nothing of it.
					stmts.push(format!(
						"let p{n} = {}?;",
						self.lift(&ty, &format!("a[{n}]"), "false", "false")
					));
					c_args.push(format!("&p{n}"));
				}
			}
			n += 1;
		}
		let mut closures = Vec::new();
		// Each parameter Rust writes back, by its argument's index, and what it holds.
		let mut written: Vec<(usize, Type)> = Vec::new();
		for param in &function.params {
			let (_, c) = c_params.next().expect("each parameter has its C one");
			let name = param_name(&param.name);
			let annotation = match &param.ty {
				Type::Ref(_, inner) if param.ty.written() => format!("Mut[{}]", self.py_ty(inner)),
				ty => self.py_param(ty),
			};
			py_params.push(format!("{name}: {annotation}"));
			native_params.push(format!("p{}: {annotation}", native_params.len()));
			py_args.push(name.clone());
			match (&param.ty, c) {
				(Type::Ref(_, inner), _) if param.ty.written() => {
					stmts.push(format!(
						"let h{n} = rt::mut_value(a[{n}], s)?;\n\t\tlet p{n} = {}?;",
						self.lift(inner, &format!("h{n}"), "true", "true")
					));
					c_args.push(format!("p{n}"));
					written.push((n, (**inner).clone()));
				}
				(Type::Closure(closure_), Ty::Fn(c_lent, c_ret)) => {
					c_params.next().expect("a closure has its data");
					let trampoline = format!("py_{c_name}_{}", abi::escape(&param.name));
					closures.push((
						(**closure_).clone(),
						c_lent.clone(),
						(**c_ret).clone(),
						trampoline.clone(),
					));
					c_args.push(format!("Some({trampoline})"));
					c_args.push(format!("p{n}"));
					match closure_.kept {
						true => {
							c_params.next().expect("a kept closure has its free");
							stmts.push(format!("let p{n} = rt::kept(a[{n}], s)?;"));
							c_args.push("Some(rt::free_closure)".to_string());
						}
						false => stmts.push(format!("let p{n} = rt::closure(a[{n}])?;")),
					}
				}
				_ => {
					stmts.push(format!(
						"let p{n} = {}?;",
						self.lift(&param.ty, &format!("a[{n}]"), "true", "true")
					));
					c_args.push(format!("p{n}"));
				}
			}
			n += 1;
		}
		for (closure_, c_lent, c_ret, trampoline) in closures {
			let exception = closure_.err.as_ref().map(|e| self.exception(e));
			self.trampoline(
				&trampoline,
				&c_name,
				(&closure_.params, &closure_.ok),
				(&c_lent, &c_ret),
				None,
				exception.as_deref(),
			);
		}
		let outs: Vec<(String, Ty)> = c_params.cloned().collect();

		// The outs follow the unstripped return: a borrowed tuple is one struct, not its elements.
		let ok = strip_refs(&function.ok);
		let err = function.err.as_ref().map(strip_refs);
		let exception = err.as_ref().map(|e| self.exception(e));
		if let Some(abi::Future { poll, free, .. }) = &export.future {
			let poll_outs = &poll.params[2..];
			let (decl, ptrs, value) = self.outs(
				poll_outs,
				&[],
				&function.ok,
				err.as_ref(),
				exception.as_deref(),
				"r == 1",
			);
			let _ = write!(
				self.glue,
				"unsafe fn poll_{c_name}(f: *mut std::ffi::c_void, w: crate::rt::Waker) -> R<Option<Obj>> {{\n\tunsafe {{\n\t\t{decl}\n\t\tlet r = rt::call(|| crate::{}(f as _, w{}));\n\t\tif r == 0 {{\n\t\t\treturn Ok(None);\n\t\t}}\n\t\t{value}.map(Some)\n\t}}\n}}\n\nunsafe fn free_{c_name}(f: *mut std::ffi::c_void) {{\n\tunsafe {{ crate::{}(f as _) }}\n}}\n\n",
				poll.name,
				ptrs.iter().map(|p| format!(", {p}")).collect::<String>(),
				free.name,
			);
			stmts.push(format!(
				"let f = rt::call(|| crate::{c_name}({}));\n\t\ts.done();\n\t\trt::start(f.cast(), poll_{c_name}, free_{c_name}, s, {})",
				c_args.join(", "),
				function.eager
			));
		} else {
			let (decl, ptrs, value) = match outs.is_empty() {
				true => {
					let value = self.lower(&ok, "ret");
					(String::new(), Vec::new(), value)
				}
				false => self.outs(&outs, &written, &function.ok, err.as_ref(), exception.as_deref(), "ret"),
			};
			let args: Vec<String> = c_args.iter().cloned().chain(ptrs).collect();
			let call = match &handed {
				Some(h) => h.call(&args),
				None => format!("crate::{c_name}({})", args.join(", ")),
			};
			stmts.push(format!(
				"{decl}\n\t\tlet ret = rt::call(|| {call});\n\t\ts.done();\n\t\t{value}"
			));
		}
		// The Python.
		let mut text = function.docs.clone();
		if handed.is_some() {
			text.push_str(
				"\n\nRust changes a copy of this value, which it returns first, as the call left it: this one never changes.",
			);
		}
		for param in &function.params {
			let name = param_name(&param.name);
			if let Type::Closure(closure_) = &param.ty {
				let lent = &closure_.params;
				match closure_.kept {
					true => text.push_str(&format!(
						"\n\nRust keeps `{name}` past the call, and calls it from any thread, one call at a time, until it lets go of it."
					)),
					false => text.push_str(&format!(
						"\n\nRust calls `{name}` only during the call, from any thread."
					)),
				}
				if lent.iter().any(|p| matches!(p.base(), Type::Object(_))) {
					text.push_str(&format!(
						"\nAn object it lends lives only for that call of `{name}`: keep none past it."
					));
				}
				if let Some(err) = &closure_.err {
					let name = self.thrown(err);
					text.push_str(&format!(
						"\nTo fail, it raises `{name}`, whose error Rust takes; any other exception aborts."
					));
				}
			}
			if param.ty == Type::Ref(crate::ir::Mutability::Mut, Box::new(Type::Reader)) {
				text.push_str(&format!(
					"\n\nRust reads and seeks `{name}` only during the call, from any thread, one call at a time: an `OSError` either raises fails Rust's read or seek with its message."
				));
			}
		}
		let body = stmts.join("\n\t\t");
		// An `async def` stays Python's, as callers may need a coroutine.
		let direct = !constant && (export.future.is_none() || function.eager);
		let property = matches!(function.access, Access::Field { set: false, .. }) && place == Place::Member;
		match direct {
			true => {
				let names: Vec<String> = function.params.iter().map(|p| param_name(&p.name)).collect();
				let d = Direct {
					member: &member,
					place,
					property,
					owner: owner_c.as_deref(),
					params: &names,
					docs: &text,
				};
				self.direct(&bare, d, &body);
			}
			false => self.native(&bare, n, &body),
		}
		let native = format!("_n.{bare}({})", py_args.join(", "));
		let ret = self.py_ty(&ok);
		let docs = docstring(text.trim(), "\t");
		let params = py_params.join(", ");
		// The glue's own is only declared, for type checkers.
		let code = match (export.future.is_some(), function.eager) {
			(true, false) => {
				if !native_params.is_empty() {
					native_params.push("/".to_string());
				}
				self.natives.push(format!(
					"@_staticmethod\ndef {bare}({}) -> {ret} | _Future[{ret}]: ...\n",
					native_params.join(", ")
				));
				format!(
					"async def {member}({params}) -> {ret}:\n{docs}\tr = {native}\n\treturn (await r) if _isinstance(r, _Future) else r"
				)
			}
			(true, true) => format!("def {member}({params}) -> _typing.Awaitable[{ret}]:\n{docs}\t..."),
			(false, _) => format!("def {member}({params}) -> {ret}:\n{docs}\t..."),
		};
		let code = match (function.access, place) {
			(Access::Const, Place::Top) if constant => {
				let name = member.clone();
				self.consts.push(Const {
					name,
					bare,
					ty: ret,
					docs: text,
				});
				return None;
			}
			(Access::Const, Place::Static) if constant => {
				let name = member.to_uppercase();
				self.natives
					.push(format!("@_staticmethod\ndef {bare}() -> {ret}: ...\n"));
				// An `Enum` makes a member of any other value, to a type checker as at run time.
				let value = match owner_ty {
					Some((Type::Enum(i), _)) if abi.api.enums[*i].plain() => format!("_enum.nonmember(_n.{bare}())"),
					_ => format!("_n.{bare}()"),
				};
				return Some(Form::Native(
					format!("{name} = {value}\n{}", docstring(text.trim(), "")),
					Some(format!("{name} = _Const(_n.{bare})\n")),
				));
			}
			_ if property => format!("@_property\n{code}"),
			(_, Place::Static) => format!("@_staticmethod\n{code}"),
			_ => code,
		};
		Some(match (direct, place) {
			(true, Place::Top) => Form::Native(code, Some(format!("{member} = _n.{member}\n"))),
			(true, _) => Form::Native(code, None),
			(false, _) => Form::Python(code),
		})
	}

	/// The out values of `outs`, for a function returning `ok` or raising `err` as
	/// `exception`: their declarations, the pointers the C call takes after its arguments,
	/// and the glue's expression of the value, an `R<Obj>`, where `ok` says whether it
	/// returned ok. What Rust wrote back into each of `written`, by its argument's index,
	/// is set in its `Mut` first, whether or not the call failed.
	fn outs(
		&mut self,
		outs: &[(String, Ty)],
		written: &[(usize, Type)],
		ok: &Type,
		err: Option<&Type>,
		exception: Option<&str>,
		is_ok: &str,
	) -> (String, Vec<String>, String) {
		let mut decl = Vec::new();
		let mut ptrs = Vec::new();
		let types: Vec<&Type> = match ok {
			Type::Unit => Vec::new(),
			Type::Tuple(elements) => elements.iter().collect(),
			ok => vec![ok],
		};
		let mut values = Vec::new();
		let mut error = String::new();
		let mut holds = Vec::new();
		let back = written.iter().map(|(_, ty)| ty);
		for (k, ((name, _), ty)) in outs.iter().zip(back.chain(types).chain(err)).enumerate() {
			decl.push(format!("let mut o_{name} = std::mem::MaybeUninit::zeroed();"));
			ptrs.push(format!("o_{name}.as_mut_ptr()"));
			let value = self.lower(ty, &format!("o_{name}.assume_init()"));
			match written.get(k) {
				Some((n, _)) => holds.push(format!("rt::set_mut(a[{n}], {value})?; ")),
				None if name == "err" => error = value,
				None => values.push(value),
			}
		}
		let value = match ok {
			Type::Unit => "rt::none()".to_string(),
			Type::Tuple(_) => format!("rt::tuple(vec![{}])", values.join(", ")),
			// A value C returns, beside what Rust wrote back.
			ok if err.is_none() && !written.is_empty() => self.lower(&strip_refs(ok), "ret"),
			_ => values.pop().expect("an out value"),
		};
		let value = match exception {
			Some(exception) => format!("(if {is_ok} {{ {value} }} else {{ rt::raise_error({exception}, {error}) }})"),
			None => value,
		};
		let value = match holds.is_empty() {
			true => value,
			false => format!("{{ {}{value} }}", holds.concat()),
		};
		(decl.join("\n\t\t"), ptrs, value)
	}
}

/// The glue's name for a socket address family.
fn family_of(family: Family) -> &'static str {
	match family {
		Family::Any => "rt::Family::Any",
		Family::V4 => "rt::Family::V4",
		Family::V6 => "rt::Family::V6",
	}
}

fn signed(int: Int) -> bool {
	matches!(int, Int::I8 | Int::I16 | Int::I32 | Int::I64 | Int::Isize)
}

/// `ty` without its `Box`es.
fn strip_box(ty: &Type) -> Type {
	match ty {
		Type::Box(inner) => strip_box(inner),
		ty => ty.clone(),
	}
}

/// A Python annotation as the start of a class name: `list[str] | None` is `ListStrNone`.
fn pascal_type(py: &str) -> String {
	py.split(|c: char| !c.is_ascii_alphanumeric())
		.map(|word| {
			let mut chars = word.chars();
			match chars.next() {
				Some(first) => first.to_uppercase().chain(chars).collect(),
				None => String::new(),
			}
		})
		.collect()
}

/// Python's hard keywords, and the builtin constants.
const KEYWORDS: &[&str] = &[
	"False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
	"elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal",
	"not", "or", "pass", "raise", "return", "try", "while", "with", "yield",
];

/// A name with `_` appended if Python reads it as a keyword.
fn kw(name: &str) -> String {
	match KEYWORDS.contains(&name) {
		true => format!("{name}_"),
		false => name.to_string(),
	}
}

/// A member's Python name: its snake-case name, with `_` appended if a keyword, and until
/// it is none of `reserved`, such as one Python gives every such class.
fn member(snake: &str, reserved: &[&str]) -> String {
	if !reserved.contains(&snake) {
		return kw(snake);
	}
	let mut name = format!("{snake}_");
	while reserved.contains(&name.as_str()) {
		name.push('_');
	}
	name
}

/// A field's Python name: a positional one's is `_0`, `_1`, ...
fn field_name(name: &str) -> String {
	match name.starts_with(|c: char| c.is_ascii_digit()) {
		true => format!("_{name}"),
		false => kw(&abi::snake(name)),
	}
}

/// A parameter's Python name, apart from the runtime's names a body reads.
fn param_name(name: &str) -> String {
	let name = kw(&abi::snake(name));
	match TAKEN.contains(&name.as_str()) || name == "self" {
		true => format!("{name}_"),
		false => name,
	}
}

/// The name a member's code declares, for [`Gen::distinct`].
fn member_name(code: &str) -> Option<String> {
	let line = code.lines().find(|l| {
		let l = l.trim_start();
		!l.starts_with('@') && !l.is_empty()
	})?;
	let line = line.trim_start();
	let line = line.strip_prefix("async ").unwrap_or(line);
	if let Some(rest) = line.strip_prefix("def ") {
		return Some(rest.chars().take_while(|c| *c != '(').collect());
	}
	let name: String = line.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
	(!name.is_empty()).then_some(name)
}

/// A C string literal of `text`.
fn c_str(text: &str) -> String {
	format!("c{text:?}")
}

/// A docstring holding `docs`, each line indented by `indent`, or nothing for none.
fn docstring(docs: &str, indent: &str) -> String {
	let docs = docs.trim();
	if docs.is_empty() {
		return String::new();
	}
	let escaped = docs.replace('\\', "\\\\").replace("\"\"\"", "\\\"\\\"\\\"");
	let mut out = format!("{indent}\"\"\"");
	for (n, line) in escaped.lines().enumerate() {
		if n > 0 {
			out.push('\n');
			if !line.is_empty() {
				out.push_str(indent);
			}
		}
		out.push_str(line);
	}
	if escaped.ends_with('"') {
		out.push(' ');
	}
	out.push_str("\"\"\"\n");
	out
}

/// The Rust type of a block's number.
fn number(ty: &Type) -> &'static str {
	match ty {
		Type::Int(int) => int.rust(),
		Type::F32 => "f32",
		Type::F64 => "f64",
		other => unreachable!("{other:?} is no block's number"),
	}
}

/// Whether a record or variant holding `ty` needs [`ieee_eq`]: `ty` holds a float, or a
/// block, which may be a buffer passed in whose own `==` compares item by item.
fn own_eq(abi: &Abi, ty: &Type) -> bool {
	abi.api.holds_float(ty) || ty.holds_block()
}

/// The `__eq__` of the dataclass `class` of `fields`, comparing as Rust's `==` does. A
/// dataclass's own compares tuples of its fields, which take a value as equal to itself, so
/// a NaN it holds would equal itself, and a numpy array it holds would raise.
fn ieee_eq(class: &str, fields: &[String]) -> String {
	let same: Vec<String> = fields
		.iter()
		.map(|f| format!(" and _same(self.{f}, other.{f})"))
		.collect();
	format!(
		"\tdef __eq__(self, other: object) -> bool:\n\t\treturn _isinstance(other, {class}){}\n",
		same.concat()
	)
}
