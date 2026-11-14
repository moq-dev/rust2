//! Go over the C ABI through cgo: a package per crate whose items the facade exports,
//! `go/<package>/`, and the runtime they share, `go/rt/`, each including the C header from
//! `include/` beside `go/`. A crate's package is its name without the prefix and its `_`
//! (moq-mux's is `mux`), so callers write `mux.CodecOpusConfig_`. Cargo keeps crates from
//! depending on each other in a cycle, so their packages import none in one either, as Go
//! requires; Rust's modules may, and so cannot each be a package. An instance of a generic
//! type, or a function naming one, belongs to the last of its own and its arguments'
//! crates, which can name them all. The import path of `go/` is the module of the `go.mod`
//! above it, or else the module `<prefix>` of the `go.mod` the export writes there.
//!
//! Each name is its C name without the prefix and the segments outside its crate, in
//! Pascal case, kept [`apart`](abi::apart) from another type's of the package that would
//! match: `moq_track_producer` is `TrackProducer`, and its method
//! `moq_track_producer_append_group` is `TrackProducer.AppendGroup`. A function of a type
//! that takes no receiver is a function of the package, named through its type
//! (`CounterSeeded`), but `new`, which is `NewCounter`. An object is a pointer to a struct
//! owning its handle, freed by `Close` or else once the GC collects it, and a call that
//! meets it closed returns an error matching `rt.ErrClosed`, so every call that takes one,
//! at any depth, returns an `error`; a record a struct; a plain enum an integer type and its
//! constants; an enum with data an interface each variant's struct implements; a trait an
//! interface; a closure a function; a reader an `io.ReadSeeker`; a future a call taking a
//! `context.Context` that blocks until the future is ready; a `Result` an `error`. A `Copy`
//! type with private fields is a struct over its bytes, which only Rust makes or reads,
//! and a zero one panics where Rust would read it. The
//! runtime they share,
//! [`runtime/rt.go`](../runtime/rt.go), holds the handles and the bytes Rust returns, and
//! each package's copy of [`runtime/ffi_cgo.go`](../runtime/ffi_cgo.go) the arena a call lowers
//! nested values into, and the wait of a future: cgo gives each package C types of its
//! own, so packages reach each other only through Go's.
//!
//! A call passes bytes and text as views of Go's own memory, pinned where C reads them
//! through other memory, so neither is copied in. Bytes Rust returns, or a record holds,
//! are an `rt.Bytes`, which from 2 KiB reads Rust's buffer in place and frees it only once
//! the GC finds it unreachable, so no Go code reads freed memory, and below copies it,
//! which costs less than the GC's cleanup. An object instead frees at `Close`, since what
//! it holds, such as a session, must end promptly. Text Rust returns is copied once into a
//! `string`, and Rust's freed at once.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write;

use crate::abi::{self, Abi, Ty, pascal};
use crate::ir::{Access, Closure, Collection, Family, Function, Home, Int, Literal, On, Param, Receiver, Type};

const RUNTIME: &str = include_str!("../runtime/rt.go");
const CONVERSIONS: &str = include_str!("../runtime/ffi_cgo.go");

/// The standard library's packages the generated items name.
const STD: &[(&str, &str)] = &[
	("big", "math/big"),
	("context", "context"),
	("cgo", "runtime/cgo"),
	("io", "io"),
	("netip", "net/netip"),
	("runtime", "runtime"),
	("time", "time"),
	("unsafe", "unsafe"),
];

/// The Go package of each crate the export's items belong to, and of the runtime, which
/// [`Packages::RT`] keys.
struct Packages {
	/// By crate: its Go name, and its import path.
	by_crate: BTreeMap<String, (String, String)>,
}

impl Packages {
	const RT: &'static str = "";

	fn new(abi: &Abi<'_>, module: &str) -> Self {
		let mut by_crate = BTreeMap::from([(Self::RT.to_string(), ("rt".to_string(), format!("{module}/rt")))]);
		let strip = format!("{}_", abi.prefix);
		for krate in &abi.api.crates {
			// A later crate of an earlier one's name takes its count after it.
			let (base, count) = krate.split_once('#').unwrap_or((krate, ""));
			// A package's name is an identifier: a crate the prefix's strip leaves no leading
			// letter, such as `moq_2d` or a lib `moq_` beside `moq`, keeps the prefix, and a
			// name whose first digit no letter precedes, such as `_2d`, follows `crate`.
			let letter = |name: &String| name.starts_with(|c: char| c.is_alphabetic());
			let mut name = Some(base.strip_prefix(&strip).unwrap_or(base).replace('_', ""))
				.filter(letter)
				.unwrap_or_else(|| base.replace('_', ""));
			if !letter(&name) {
				name.insert_str(0, "crate");
			}
			name += count;
			// A name Go, cgo, or the runtime takes, or one an earlier crate's matches ignoring
			// case, as a case-insensitive filesystem would, takes a trailing `_`, which no
			// crate's name keeps.
			while ["C", "main"].contains(&name.as_str())
				|| keyword(&name)
				|| by_crate.values().any(|(n, _)| n.eq_ignore_ascii_case(&name))
			{
				name.push('_');
			}
			by_crate.insert(krate.clone(), (name.clone(), format!("{module}/{name}")));
		}
		Self { by_crate }
	}

	fn name(&self, krate: &str) -> &str {
		&self.by_crate[krate].0
	}
}

/// The generated Go files and their public names.
pub struct Output {
	pub files: Vec<(String, String)>,
	pub names: BTreeMap<String, String>,
}

/// Each crate's Go package name, by crate, which also names its C++ header.
pub fn packages(abi: &Abi<'_>) -> BTreeMap<String, String> {
	let mut by_crate = Packages::new(abi, "").by_crate;
	by_crate.remove(Packages::RT);
	by_crate.into_iter().map(|(krate, (name, _))| (krate, name)).collect()
}

/// The Go for `abi`, whose header's contract is `contract`, as each file under `go/` by its
/// path there, for the module path `module` of `go/`; or the names two items claim alike.
pub fn generate(abi: &Abi<'_>, packages: &str, contract: u64, module: &str) -> Result<Output, String> {
	let set = Packages::new(abi, module);
	let types = abi.classes(|home| (home.krate.clone(), home.outside - 1), |_| false);
	let mut locked = crate::lock::types(abi, &types);
	let classes = classes(abi, &types);
	let thrown = thrown(abi, &classes);
	// Every error a function returns, under each spelling that shares its exception.
	let api = abi.api;
	let errors = (api.records.iter().flat_map(|r| &r.methods))
		.chain(api.enums.iter().flat_map(|e| &e.methods))
		.chain(api.objects.iter().flat_map(|o| &o.methods))
		.chain(&api.functions)
		.filter_map(|f| f.err.as_ref());
	for err in errors {
		let key = abi.key(err);
		if let Some((name, (home, _))) = thrown.iter().find(|(_, (_, ty))| abi.key(ty) == key) {
			locked.insert(crate::lock::error_key(abi, err), format!("{home}::{name}"));
		}
	}
	let variants = variants(abi, &types, &classes, &thrown);
	crate::lock::variants(abi, &mut locked, |c, v| {
		variants.get(&(c.to_string(), v.to_string())).cloned()
	});
	// What each package's types, their variants and tables, and its errors take.
	let mut taken: BTreeMap<String, BTreeSet<String>> = classes.clone();
	for ((c, _), name) in &variants {
		let krate = (0..abi.api.enums.len())
			.find(|&i| abi.enumeration(i) == *c)
			.map(|i| &abi.api.enums[i].home.krate);
		taken
			.entry(krate.expect("a variant's enum").clone())
			.or_default()
			.insert(name.clone());
	}
	for (name, (home, _)) in &thrown {
		taken.entry(home.clone()).or_default().insert(name.clone());
	}
	// Each callback's table, `<name>Vtable_`, unless a name of its package takes that.
	let mut vtables = HashMap::new();
	for i in 0..abi.api.callbacks.len() {
		let krate = &abi.api.callbacks[i].home.krate;
		let names = taken.entry(krate.clone()).or_default();
		let mut vtable = format!("{}Vtable_", types[&abi.callback(i)]);
		while !names.insert(vtable.clone()) {
			vtable.push('_');
		}
		vtables.insert(abi.callback(i), vtable);
	}
	let shared = Shared {
		thrown,
		type_names: types,
		classes,
		variants,
		vtables,
		taken,
	};
	let mut files = Vec::new();
	let mut errors = Vec::new();
	let mut tuples = BTreeSet::new();
	let mut rt = Gen::new(abi, &set, Packages::RT, &shared);
	rt.write_rt();
	for krate in &abi.api.crates {
		let mut g = Gen::new(abi, &set, krate, &shared);
		g.write();
		locked.append(&mut g.locked);
		errors.append(&mut g.errors);
		tuples.extend(g.tuples.iter().copied());
		let name = set.name(krate);
		locked.insert(format!("crate {krate}"), name.to_string());
		let doc = format!(
			"// Package {name} is the pub API of the crate {krate} in {packages}, over its C ABI\n\
			// through cgo. Link the library built with {p}.h, through CGO_LDFLAGS.\n",
			p = abi.prefix
		);
		files.push((format!("{name}/{name}.go"), g.file(&doc)));
		// No package name holds a `_` but at its end, so the helpers never take the API's file.
		files.push((format!("{name}/ffi_cgo.go"), g.conversions()));
	}
	errors.append(&mut rt.errors);
	if !errors.is_empty() {
		return Err(errors.join("\n"));
	}
	for n in tuples {
		let params: Vec<String> = (0..n).map(|k| format!("T{k}")).collect();
		let fields: String = (0..n).map(|k| format!("\tV{k} T{k}\n")).collect();
		let _ = write!(
			rt.types,
			"// Tuple{n} is a Rust tuple of {n} elements.\ntype Tuple{n}[{} any] struct {{\n{fields}}}\n\n",
			params.join(", ")
		);
	}
	files.push(("rt/rt.go".to_string(), rt.runtime(packages, contract)));
	files.push(("rt/types.go".to_string(), rt.file("")));
	Ok(Output { files, names: locked })
}

/// Each error type that is no exported type, named by its key as its C structs are, so two
/// types never share one, with its type and the crate it belongs to: the last of those
/// its types do, or the runtime for none. A name its package's types or runtime take
/// takes a trailing `_`.
fn thrown(abi: &Abi<'_>, classes: &BTreeMap<String, BTreeSet<String>>) -> BTreeMap<String, (String, Type)> {
	let api = abi.api;
	let all = (api.records.iter().flat_map(|r| &r.methods))
		.chain(api.enums.iter().flat_map(|e| &e.methods))
		.chain(api.objects.iter().flat_map(|o| &o.methods))
		.chain(&api.functions);
	let mut keys = BTreeMap::new();
	for function in all {
		let Some(err) = function.err.as_ref().map(strip_refs) else {
			continue;
		};
		if matches!(err.base(), Type::Record(_) | Type::Enum(_) | Type::Object(_)) {
			continue;
		}
		keys.entry(abi.key(&err)).or_insert(err);
	}
	let mut thrown = BTreeMap::new();
	let mut names = BTreeMap::new();
	for (key, err) in keys {
		let home = last(abi, &err);
		let types = classes.get(&home);
		let taken =
			|name: &str| types.is_some_and(|types| types.contains(name)) || (home == Packages::RT && taken(name));
		let name = abi::claim(
			names.entry(home.clone()).or_default(),
			taken,
			&format!("{}Error", pascal(&key)),
			key,
		);
		thrown.insert(name, (home, err));
	}
	thrown
}

/// The Go name of each variant of each enum, by its C name and the variant's: the enum's
/// name then the variant's, unless another type, error, or variant of its package takes it.
fn variants(
	abi: &Abi<'_>,
	types: &HashMap<String, String>,
	classes: &BTreeMap<String, BTreeSet<String>>,
	thrown: &BTreeMap<String, (String, Type)>,
) -> HashMap<(String, String), String> {
	let api = abi.api;
	let mut variants = HashMap::new();
	for krate in &api.crates {
		let enums = (0..api.enums.len()).filter(|&i| api.enums[i].home.krate == *krate);
		let mut keys: Vec<(String, String)> = (enums)
			.flat_map(|i| {
				api.enums[i]
					.variants
					.iter()
					.map(move |v| (abi.enumeration(i), v.name.clone()))
			})
			.collect();
		keys.sort();
		let names = (keys.iter())
			.map(|(c, variant)| format!("{}{}", types[c], pascal(variant)))
			.collect();
		let taken = classes.get(krate);
		let names = abi::unique(names, |name| {
			taken.is_some_and(|taken| taken.contains(name)) || thrown.get(name).is_some_and(|(home, _)| home == krate)
		});
		variants.extend(keys.into_iter().zip(names));
	}
	variants
}

/// The last crate, in dependency order, any type within `ty` belongs to, or the runtime's
/// key for none.
fn last(abi: &Abi<'_>, ty: &Type) -> String {
	let api = abi.api;
	let rank = |k: &str| api.crates.iter().position(|c| c == k);
	let mut last = Packages::RT.to_string();
	crate::ir::each(ty, &mut |ty| {
		let krate = match ty {
			Type::Record(i) => &api.records[*i].home.krate,
			Type::Enum(i) => &api.enums[*i].home.krate,
			Type::Object(i) => &api.objects[*i].home.krate,
			Type::Callback(i, _) => &api.callbacks[*i].home.krate,
			_ => return,
		};
		if rank(krate) > rank(&last) {
			last = krate.clone();
		}
	});
	last
}

/// Each exported type's Go name, of those by C name in `types`, by package.
fn classes(abi: &Abi<'_>, types: &HashMap<String, String>) -> BTreeMap<String, BTreeSet<String>> {
	let api = abi.api;
	let mut classes: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
	let names = (0..api.records.len()).map(|i| (abi.record(i), &api.records[i].home));
	let names = names.chain((0..api.enums.len()).map(|i| (abi.enumeration(i), &api.enums[i].home)));
	let names = names.chain((0..api.objects.len()).map(|i| (abi.object(i), &api.objects[i].home)));
	let names = names.chain((0..api.callbacks.len()).map(|i| (abi.callback(i), &api.callbacks[i].home)));
	for (c, home) in names {
		classes.entry(home.krate.clone()).or_default().insert(types[&c].clone());
	}
	classes
}

/// The C name `c` of an item shown as `shown`, belonging to `home`, without the prefix and
/// the segments outside its crate.
fn rest<'c>(prefix: &str, c: &'c str, shown: &[String], home: &Home) -> &'c str {
	let bare = &c[prefix.len() + 1..];
	let outside = shown[1..home.outside].join("_");
	match outside.is_empty() {
		true => bare,
		false => (bare.strip_prefix(&outside))
			.and_then(|rest| rest.strip_prefix('_'))
			.unwrap_or_else(|| panic!("`{c}` starts with `{outside}`")),
	}
}

/// The name the generated code imports the generated package `name` by, which no local,
/// predeclared name, or other import takes: no parameter's name holds a `_` but at its
/// end, and no other local starts `p_`.
fn alias(name: &str) -> String {
	format!("p_{name}")
}

/// Whether Go reads `name` as a keyword.
fn keyword(name: &str) -> bool {
	KEYWORDS.contains(&name)
}

/// The names the runtime's package declares, besides its `Tuple<N>` generics, which a type
/// of its own would clash with.
const TAKEN: &[&str] = &[
	"Bound",
	"BoundUnbounded",
	"BoundIncluded",
	"BoundExcluded",
	"Bytes",
	"NewBytes",
	"ErrClosed",
	"Handle_",
	"NewHandle_",
	"LiftBytes_",
	"BytesBuf_",
	"GoCall_",
	"GoRelease_",
];

/// Whether the runtime's package takes the Go name `name`: one of [`TAKEN`], or a tuple's
/// generic, `Tuple<N>`.
fn taken(name: &str) -> bool {
	let tuple = name
		.strip_prefix("Tuple")
		.is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
	tuple || TAKEN.contains(&name)
}

/// Whether the Go code `code` names the package `name`.
fn uses(code: &str, name: &str) -> bool {
	let needle = format!("{name}.");
	code.match_indices(&needle).any(|(at, _)| {
		!code[..at]
			.chars()
			.next_back()
			.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.')
	})
}

/// A Go function: its doc comment, receiver, name, parameters, results, and body.
struct Method {
	key: Option<String>,
	docs: String,
	receiver: Option<String>,
	name: String,
	params: Vec<(String, String)>,
	results: Vec<String>,
	body: String,
}

impl Method {
	fn signature(&self) -> String {
		let params: Vec<String> = self.params.iter().map(|(n, ty)| format!("{n} {ty}")).collect();
		let results = match &self.results[..] {
			[] => String::new(),
			[one] => format!(" {one}"),
			many => format!(" ({})", many.join(", ")),
		};
		format!("{}({}){results}", self.name, params.join(", "))
	}

	fn write(&self) -> String {
		let receiver = match &self.receiver {
			Some(r) => format!("({r}) "),
			None => String::new(),
		};
		format!(
			"{}func {receiver}{} {{\n{}\n}}\n\n",
			docs(self.docs.trim(), ""),
			self.signature(),
			indent(&self.body)
		)
	}
}

struct Gen<'a> {
	locked: BTreeMap<String, String>,
	abi: &'a Abi<'a>,
	set: &'a Packages,
	/// The crate whose package this is, or [`Packages::RT`].
	pkg: &'a str,
	/// The types, then the package's functions and constants, then the helpers by name,
	/// then the functions Go exports to C.
	types: String,
	top: String,
	helpers: BTreeMap<String, String>,
	exports: String,
	/// The C declarations of what Go exports, for the preamble.
	decls: Vec<String>,
	/// Each closure type's trampoline, by its Go type.
	closures: HashMap<Closure, String>,
	/// The arity of each tuple type the package names.
	tuples: BTreeSet<usize>,
	/// Each package-level Go name, with what claimed it.
	names: BTreeMap<String, String>,
	/// Each exported type's Go name, by its C name.
	type_names: &'a HashMap<String, String>,
	/// Each exported type's Go name, by package.
	classes: &'a BTreeMap<String, BTreeSet<String>>,
	/// Each enum's variant's Go name, by the enum's C name and the variant's.
	variants: &'a HashMap<(String, String), String>,
	/// Each callback's table's Go name, by the callback's C name.
	vtables: &'a HashMap<String, String>,
	/// The other packages the package's items name, by crate.
	imports: BTreeSet<String>,
	errors: Vec<String>,
	/// The struct each error type that is no exported type returns as, by name, with the
	/// crate it belongs to.
	thrown: &'a BTreeMap<String, (String, Type)>,
}

/// The names every package's generator shares.
struct Shared {
	thrown: BTreeMap<String, (String, Type)>,
	type_names: HashMap<String, String>,
	classes: BTreeMap<String, BTreeSet<String>>,
	variants: HashMap<(String, String), String>,
	/// Each callback's table's Go name, by the callback's C name.
	vtables: HashMap<String, String>,
	/// The names each package's types, their variants and tables, and its errors take.
	taken: BTreeMap<String, BTreeSet<String>>,
}

impl<'a> Gen<'a> {
	fn new(abi: &'a Abi<'a>, set: &'a Packages, pkg: &'a str, shared: &'a Shared) -> Self {
		let names = (shared.taken.get(pkg).into_iter().flatten())
			.map(|name| (name.clone(), "a type".to_string()))
			.collect();
		Self {
			locked: BTreeMap::new(),
			abi,
			set,
			pkg,
			types: String::new(),
			top: String::new(),
			helpers: BTreeMap::new(),
			exports: String::new(),
			decls: Vec::new(),
			closures: HashMap::new(),
			tuples: BTreeSet::new(),
			names,
			type_names: &shared.type_names,
			classes: &shared.classes,
			variants: &shared.variants,
			vtables: &shared.vtables,
			imports: BTreeSet::new(),
			errors: Vec::new(),
			thrown: &shared.thrown,
		}
	}

	/// The package's file of its items, opening with `doc`.
	fn file(&self, doc: &str) -> String {
		let name = self.set.name(self.pkg);
		let mut body = String::new();
		body.push_str(&self.types);
		body.push_str(&self.top);
		for helper in self.helpers.values() {
			body.push_str(helper);
		}
		body.push_str(&self.exports);
		let mut out = format!("// Code generated by rust2-ffi. DO NOT EDIT.\n\n{doc}package {name}\n\n");
		// The runtime's types need no C.
		if self.pkg != Packages::RT {
			let p = &self.abi.prefix;
			let _ = write!(
				out,
				"/*\n#cgo CFLAGS: -I${{SRCDIR}}/../../include\n#include <stdlib.h>\n#include \"{p}.h\"\n\n"
			);
			for decl in &self.decls {
				out.push_str(decl);
				out.push('\n');
			}
			out.push_str("*/\nimport \"C\"\n\n");
		}
		let code: String = (body.lines())
			.filter(|line| !line.trim_start().starts_with("//"))
			.collect::<Vec<_>>()
			.join("\n");
		let mut imports: Vec<String> = (STD.iter())
			.filter(|(name, _)| uses(&code, name))
			.map(|(_, path)| format!("\t\"{path}\""))
			.collect();
		imports.sort();
		let mut ours: Vec<String> = (self.imports.iter())
			.map(|krate| {
				let (name, path) = &self.set.by_crate[krate];
				format!("\t{} \"{path}\"", alias(name))
			})
			.collect();
		ours.sort();
		if !imports.is_empty() && !ours.is_empty() {
			imports.push(String::new());
		}
		imports.extend(ours);
		if !imports.is_empty() {
			let _ = write!(out, "import (\n{}\n)\n\n", imports.join("\n"));
		}
		out.push_str(&body);
		while out.ends_with("\n\n") {
			out.pop();
		}
		out
	}

	/// The package's copy of what converts the export's C types, which cgo makes its own.
	fn conversions(&self) -> String {
		let name = self.set.name(self.pkg);
		let rt = &self.set.by_crate[Packages::RT].1;
		let p = &self.abi.prefix;
		let mut out = format!(
			"// Code generated by rust2-ffi. DO NOT EDIT.\n\n\
			package {name}\n\n\
			/*\n#cgo CFLAGS: -I${{SRCDIR}}/../../include\n#include <stdlib.h>\n#include \"{p}.h\"\n*/\nimport \"C\"\n\n\
			import (\n\t\"context\"\n\t\"math\"\n\t\"math/big\"\n\t\"net\"\n\t\"net/netip\"\n\t\"runtime\"\n\t\"runtime/cgo\"\n\t\"strconv\"\n\t\"time\"\n\t\"unsafe\"\n\n\t\"{rt}\"\n)\n\n\
			// The runtime's names for the export's C types and functions.\n\
			type (\n\tcBuf           = C.{p}_buf\n\tcWaker         = C.{p}_waker\n\tcForeignFuture = C.{p}_foreign_future\n\tcForeignReader = C.{p}_foreign_reader\n\tcU128          = C.{p}_u128\n\tcDuration      = C.{p}_duration\n\tcAddr          = C.{p}_socket_addr\n)\n\n\
			var (\n\tgoCall    = (*[0]byte)(rt.GoCall_())\n\tgoRelease = (*[0]byte)(rt.GoRelease_())\n\tgoPoll    = (*[0]byte)(rt.GoPoll_())\n\tgoRead    = (*[0]byte)(rt.GoRead_())\n\tgoSeek    = (*[0]byte)(rt.GoSeek_())\n)\n\n\
			func cBufFree(buf cBuf) {{\n\tC.{p}_buf_free(buf)\n}}\n\n\
			// cBufCopy is a copy of s Rust owns, as a closure returns it.\n\
			func cBufCopy(s string) cBuf {{\n\treturn C.{p}_buf_copy(bufString_(s))\n}}\n\n"
		);
		out.push_str(CONVERSIONS);
		out
	}

	/// The runtime's package, which checks the library's contract once it loads.
	fn runtime(&self, packages: &str, contract: u64) -> String {
		let p = &self.abi.prefix;
		let upper = p.to_uppercase();
		let go = format!("{p}Go");
		let mut out = format!(
			"// Code generated by rust2-ffi. DO NOT EDIT.\n\n\
			// Package rt is what the Go packages of the pub API of {packages} share: the bytes\n\
			// Rust returns, the error a closed object returns, and the types of no crate's.\n\
			package rt\n\n\
			/*\n#cgo CFLAGS: -I${{SRCDIR}}/../../include\n#include <stdlib.h>\n#include \"{p}.h\"\n\n\
			extern void {go}_call(void *data);\nextern void {go}_release(void *data);\nextern bool {go}_poll(void *data, {p}_waker waker);\n\
			extern bool {go}_read(void *data, uint8_t *buf, size_t len, size_t *n, {p}_buf *err);\n\
			extern bool {go}_seek(void *data, int whence, int64_t offset, uint64_t *position, {p}_buf *err);\n\n\
			static void {go}_bufs_free({p}_buf *bufs, size_t n) {{\n\tfor (size_t i = 0; i < n; i++) {{\n\t\t{p}_buf_free(bufs[i]);\n\t}}\n}}\n\n\
			static void {go}_waker_wake({p}_waker w) {{\n\tif (w.wake != NULL) {{\n\t\tw.wake(w.data);\n\t}}\n}}\n\n\
			static void {go}_waker_drop({p}_waker w) {{\n\tif (w.drop != NULL) {{\n\t\tw.drop(w.data);\n\t}}\n}}\n\
			*/\nimport \"C\"\n\n\
			import (\n\t\"context\"\n\t\"errors\"\n\t\"io\"\n\t\"runtime\"\n\t\"runtime/cgo\"\n\t\"sync\"\n\t\"sync/atomic\"\n\t\"unsafe\"\n)\n\n\
			type (\n\tcBuf   = C.{p}_buf\n\tcWaker = C.{p}_waker\n)\n\n\
			var (\n\tgoCall    = (*[0]byte)(C.{go}_call)\n\tgoRelease = (*[0]byte)(C.{go}_release)\n\tgoPoll    = (*[0]byte)(C.{go}_poll)\n\tgoRead    = (*[0]byte)(C.{go}_read)\n\tgoSeek    = (*[0]byte)(C.{go}_seek)\n)\n\n\
			func cBufFree(buf cBuf) {{\n\tC.{p}_buf_free(buf)\n}}\n\n\
			// cBufCopy is a copy of s Rust owns, as a reader's error returns it.\n\
			func cBufCopy(s string) cBuf {{\n\tif len(s) == 0 {{\n\t\treturn C.{p}_buf_copy(cBuf{{}})\n\t}}\n\treturn C.{p}_buf_copy(cBuf{{ptr: (*C.uint8_t)(unsafe.Pointer(unsafe.StringData(s))), len: C.size_t(len(s))}})\n}}\n\n\
			// cBufClone is a second owner of buf, which Rust returned, sharing its bytes.\n\
			func cBufClone(buf cBuf) cBuf {{\n\treturn C.{p}_buf_clone(buf)\n}}\n\n\
			// cBufsFree frees bufs, which are not empty, in one call.\n\
			func cBufsFree(bufs []cBuf) {{\n\tC.{go}_bufs_free(&bufs[0], C.size_t(len(bufs)))\n}}\n\n\
			func cWakerWake(w cWaker) {{\n\tC.{go}_waker_wake(w)\n}}\n\n\
			func cWakerDrop(w cWaker) {{\n\tC.{go}_waker_drop(w)\n}}\n\n\
			// Go packages and a library from different builds must not mix: checked when the\n\
			// runtime loads, before any other call.\n\
			func init() {{\n\tif C.{p}_contract() != 0x{contract:016x} || C.{upper}_CONTRACT != 0x{contract:016x} {{\n\t\tpanic(\"the Go of {p} and the library are from different builds\")\n\t}}\n\ttarget_ = uint64(C.{p}_target())\n}}\n\n\
			// target_ is the index of the library's target among those the packages serve.\n\
			var target_ uint64\n\n\
			// On_ panics unless the library's target is among those of on, as the item what is.\n\
			func On_(on uint64, what string) {{\n\tif on>>target_&1 == 0 {{\n\t\tpanic(what + \": it is not on this library's target\")\n\t}}\n}}\n\n\
			//export {go}_call\nfunc {go}_call(data unsafe.Pointer) {{\n\tgiven_(data).(func())()\n}}\n\n\
			//export {go}_release\nfunc {go}_release(data unsafe.Pointer) {{\n\trelease_(data)\n}}\n\n\
			//export {go}_poll\nfunc {go}_poll(data unsafe.Pointer, waker cWaker) C.bool {{\n\treturn C.bool(given_(data).(*pollable_).poll(waker))\n}}\n\n\
			//export {go}_read\nfunc {go}_read(data unsafe.Pointer, buf *C.uint8_t, n C.size_t, read *C.size_t, err *cBuf) C.bool {{\n\
			\tgot, e := given_(data).(*reader_).read(unsafe.Slice((*byte)(unsafe.Pointer(buf)), int(n)))\n\
			\tif e != nil {{\n\t\t*err = cBufCopy(e.Error())\n\t\treturn false\n\t}}\n\
			\t*read = C.size_t(got)\n\treturn true\n}}\n\n\
			//export {go}_seek\nfunc {go}_seek(data unsafe.Pointer, whence C.int, offset C.int64_t, position *C.uint64_t, err *cBuf) C.bool {{\n\
			\tat, e := given_(data).(*reader_).seek(int64(offset), int(whence))\n\
			\tif e != nil {{\n\t\t*err = cBufCopy(e.Error())\n\t\treturn false\n\t}}\n\
			\t*position = C.uint64_t(at)\n\treturn true\n}}\n\n"
		);
		out.push_str(RUNTIME);
		out
	}

	/// The statement that panics unless the library's target is among those of `on`, as the
	/// item `what` is, or none if it is on every target.
	fn on(&mut self, on: On, what: &str) -> Option<String> {
		let api = self.abi.api;
		if api.everywhere(on) {
			return None;
		}
		let check = self.qualify(Packages::RT, "On_");
		Some(format!("{check}({}, \"{what}\")", on.0 & api.all().0))
	}

	/// The Go name of `name`, an item of `krate`'s package, from this package: imported, or
	/// its own.
	fn qualify(&mut self, krate: &str, name: &str) -> String {
		if krate == self.pkg {
			return name.to_string();
		}
		self.imports.insert(krate.to_string());
		format!("{}.{name}", alias(self.set.name(krate)))
	}

	/// The crate an exported type belongs to, and its Go name there.
	fn named(&self, ty: &Type) -> (&'a str, String) {
		let abi = self.abi;
		let api = abi.api;
		let (krate, c) = match ty {
			Type::Record(i) => (&api.records[*i].home.krate, abi.record(*i)),
			Type::Enum(i) => (&api.enums[*i].home.krate, abi.enumeration(*i)),
			Type::Object(i) => (&api.objects[*i].home.krate, abi.object(*i)),
			Type::Callback(i, _) => (&api.callbacks[*i].home.krate, abi.callback(*i)),
			Type::Range(_) | Type::Bound(_) | Type::Bounds(_) => return (Packages::RT, self.class(&abi.range(ty))),
			other => unreachable!("{other:?} is no exported type"),
		};
		(krate, self.type_names[&c].clone())
	}

	/// The Go name of an exported type from this package.
	fn qualified(&mut self, ty: &Type) -> String {
		let (krate, name) = self.named(ty);
		self.qualify(krate, &name)
	}

	/// A C name without the prefix.
	fn bare<'n>(&self, c: &'n str) -> &'n str {
		&c[self.abi.prefix.len() + 1..]
	}

	/// The Go name of the runtime's C type `c`, such as a range's.
	fn class(&self, c: &str) -> String {
		pascal(self.bare(c))
	}

	/// The Go names of parameters, each [`local`], [`abi::distinct`].
	fn locals(&self, params: &[Param]) -> Vec<String> {
		abi::distinct(params.iter().map(|p| local(&p.name)))
	}

	/// The package-level Go name `what` takes, `name` unless that is taken.
	fn claim(&mut self, name: &str, what: String) -> String {
		let rt = self.pkg == Packages::RT;
		abi::claim(&mut self.names, |name| rt && taken(name), name, what)
	}

	/// Check that no two members of the type `rust` share a Go name.
	fn distinct(&mut self, rust: &str, members: &[String]) {
		let mut seen = BTreeSet::new();
		for member in members {
			if !seen.insert(member) {
				self.errors.push(format!(
					"two members of `{rust}` are both `{member}` in Go, which rust2-ffi should keep apart"
				));
			}
		}
	}

	/// The Go type of `ty`, as a value, a parameter, or a result.
	fn go(&mut self, ty: &Type) -> String {
		match ty {
			Type::Const(_) => unreachable!("a const argument is no value"),
			Type::Array(n) => format!("[{n}]byte"),
			Type::Unit => unreachable!("no value is unit"),
			Type::Bool => "bool".to_string(),
			Type::Int(int) | Type::NonZero(int) => go_int(*int).to_string(),
			Type::F32 => "float32".to_string(),
			Type::F64 => "float64".to_string(),
			Type::Char => "rune".to_string(),
			Type::Str | Type::String | Type::Path | Type::PathBuf => "string".to_string(),
			Type::Slice | Type::Vec | Type::Bytes => self.qualify(Packages::RT, "Bytes"),
			Type::U128 => "*big.Int".to_string(),
			Type::Duration => "time.Duration".to_string(),
			Type::SocketAddr(_) => "netip.AddrPort".to_string(),
			Type::List(_, inner) => format!("[]{}", self.go(inner)),
			Type::Waker => "func()".to_string(),
			Type::Reader => "io.ReadSeeker".to_string(),
			Type::Future => "func(context.Context)".to_string(),
			Type::Option(inner) => match inner.base() {
				// Already a pointer, nil for none.
				Type::Object(_) | Type::U128 => self.go(inner),
				_ => format!("*{}", self.go(inner)),
			},
			Type::Box(inner) | Type::Ref(_, inner) => self.go(inner),
			Type::Record(_) | Type::Enum(_) | Type::Callback(..) => self.qualified(ty),
			Type::Object(_) => format!("*{}", self.qualified(ty)),
			// A `Result` returns an `error` beside its value, whose message crosses.
			Type::Closure(closure) => {
				let params: Vec<String> = closure.params.iter().map(|p| self.go_param(p)).collect();
				let params = params.join(", ");
				match (&closure.ok, closure.err.is_some()) {
					(Type::Unit, false) => format!("func({params})"),
					(Type::Unit, true) => format!("func({params}) error"),
					(ok, false) => format!("func({params}) {}", self.go(ok)),
					(ok, true) => format!("func({params}) ({}, error)", self.go(ok)),
				}
			}
			Type::Range(_) | Type::Bound(_) | Type::Bounds(_) => self.qualified(ty),
			Type::Tuple(elements) => match &elements[..] {
				// Go has no tuple of one: it is its element, which loses nothing.
				[element] => self.go(element),
				elements => {
					self.tuples.insert(elements.len());
					let elements: Vec<String> = elements.iter().map(|e| self.go(e)).collect();
					let tuple = self.qualify(Packages::RT, &format!("Tuple{}", elements.len()));
					format!("{tuple}[{}]", elements.join(", "))
				}
			},
		}
	}

	/// The Go type of a parameter of a function Go calls: as [`Self::go`] has it, so bytes
	/// Rust returned pass back without a copy.
	fn go_arg(&mut self, ty: &Type) -> String {
		match ty.buffer() {
			Some(Type::Bytes) => self.go(&Type::Bytes),
			Some(Type::Option(inner)) if *inner == Type::Bytes => format!("*{}", self.go(&Type::Bytes)),
			_ => self.go_param(ty),
		}
	}

	/// The Go type of a parameter Rust lends a callback or closure: bytes alone or in an
	/// `Option`, which C passes as a buffer, are a `[]byte`, copied, and anything else as
	/// [`Self::go`] has it.
	fn go_param(&mut self, ty: &Type) -> String {
		match ty.buffer() {
			Some(Type::Bytes) => "[]byte".to_string(),
			Some(Type::Option(inner)) if *inner == Type::Bytes => "*[]byte".to_string(),
			_ => self.go(ty),
		}
	}

	/// The zero value of the Go type of `ty`, which a call returns beside an error.
	fn zero(&mut self, ty: &Type) -> String {
		match ty {
			Type::Bool => "false".to_string(),
			Type::Int(_) | Type::NonZero(_) | Type::F32 | Type::F64 | Type::Char | Type::Duration => "0".to_string(),
			Type::Str | Type::String | Type::Path | Type::PathBuf => "\"\"".to_string(),
			Type::Slice | Type::Vec | Type::Bytes => format!("{}{{}}", self.qualify(Packages::RT, "Bytes")),
			Type::Enum(i) if self.abi.api.enums[*i].plain() => "0".to_string(),
			Type::Box(inner) | Type::Ref(_, inner) => self.zero(inner),
			Type::Tuple(elements) if elements.len() == 1 => self.zero(&elements[0]),
			Type::Record(_)
			| Type::Array(_)
			| Type::SocketAddr(_)
			| Type::Range(_)
			| Type::Bound(_)
			| Type::Bounds(_)
			| Type::Tuple(_) => {
				format!("{}{{}}", self.go(ty))
			}
			_ => "nil".to_string(),
		}
	}

	/// The Go spelling of a C type, for a local the call writes.
	fn c_go(&self, ty: &Ty) -> String {
		match ty {
			Ty::Bool => "C.bool".to_string(),
			Ty::Int(int) => format!("C.{}", c_int(*int)),
			Ty::F32 => "C.float".to_string(),
			Ty::F64 => "C.double".to_string(),
			Ty::Char => "C.uint32_t".to_string(),
			Ty::Buf => "cBuf".to_string(),
			Ty::U128 => "cU128".to_string(),
			Ty::Duration => "cDuration".to_string(),
			Ty::SocketAddr => "cAddr".to_string(),
			Ty::Struct(n) | Ty::Enum(n) => format!("C.{n}"),
			Ty::Handle { name, .. } | Ty::Future(name) => format!("*C.{name}"),
			Ty::Waker => "cWaker".to_string(),
			Ty::ForeignFuture => "cForeignFuture".to_string(),
			// What a closure Go implements writes its value and error through.
			Ty::Out(inner) => format!("*{}", self.c_go(inner)),
			other => unreachable!("no local is {other:?}"),
		}
	}

	/// Whether lowering a `ty` needs the call's arena: to pin bytes, build an array, or hold
	/// a handle.
	fn arena(&self, ty: &Type) -> bool {
		let api = self.abi.api;
		match ty {
			ty if Abi::is_buf(ty) => true,
			Type::List(..) | Type::Object(_) => true,
			Type::Option(inner) | Type::Box(inner) | Type::Ref(_, inner) => self.arena(inner),
			Type::Tuple(elements) => elements.iter().any(|e| self.arena(e)),
			Type::Record(i) => api.records[*i].fields.iter().any(|f| self.arena(&f.ty)),
			Type::Enum(i) => (api.enums[*i].variants.iter()).any(|v| v.fields.iter().any(|f| self.arena(&f.ty))),
			_ => false,
		}
	}

	/// Whether a value of `ty` holds an object, whose handle a call holds through the arena
	/// and which may be closed.
	fn holds(&self, ty: &Type) -> bool {
		let api = self.abi.api;
		match ty {
			Type::Object(_) => true,
			Type::Option(inner) | Type::Box(inner) | Type::Ref(_, inner) | Type::List(_, inner) => self.holds(inner),
			Type::Tuple(elements) => elements.iter().any(|e| self.holds(e)),
			Type::Record(i) => api.records[*i].fields.iter().any(|f| self.holds(&f.ty)),
			Type::Enum(i) => (api.enums[*i].variants.iter()).any(|v| v.fields.iter().any(|f| self.holds(&f.ty))),
			_ => false,
		}
	}

	/// The arena argument, and its comma, a helper lowering `ty` takes, if it needs one.
	fn arena_arg(&self, ty: &Type) -> &'static str {
		match self.arena(ty) {
			true => "a_, ",
			false => "",
		}
	}

	/// The arena parameter, and its comma, a helper lowering `ty` takes, if it needs one.
	fn arena_param(&self, ty: &Type) -> &'static str {
		match self.arena(ty) {
			true => "a_ *arena_, ",
			false => "",
		}
	}

	/// Add the helper `name` once, writing it with `write` the first time.
	fn helper(&mut self, name: &str, write: impl FnOnce(&mut Self) -> String) {
		if self.helpers.contains_key(name) {
			return;
		}
		// Claimed first, so a helper that names itself in its body is written once.
		self.helpers.insert(name.to_string(), String::new());
		let code = write(self);
		self.helpers.insert(name.to_string(), code);
	}

	/// The C value of the Go value `e` of type `ty`, which the arena `a_` pins and holds
	/// what it needs of until the call returns.
	fn lower(&mut self, ty: &Type, e: &str) -> String {
		let abi = self.abi;
		match ty {
			Type::Const(_) => unreachable!("a const argument is no value"),
			Type::Array(n) => {
				self.array(*n);
				format!("lower_{}({e})", abi.array(*n))
			}
			Type::Str | Type::String | Type::Path | Type::PathBuf => format!("a_.bufString({e})"),
			Type::Slice | Type::Vec | Type::Bytes => format!("a_.bufView({e})"),
			Type::Unit => unreachable!("no value is unit"),
			Type::Bool => format!("C.bool({e})"),
			Type::Int(int) | Type::NonZero(int) => format!("C.{}({e})", c_int(*int)),
			Type::F32 => format!("C.float({e})"),
			Type::F64 => format!("C.double({e})"),
			Type::Char => format!("C.uint32_t({e})"),
			Type::U128 => format!("lowerU128_({e})"),
			Type::Duration => format!("lowerDuration_({e})"),
			Type::SocketAddr(Family::Any) => format!("lowerAddr_({e})"),
			Type::SocketAddr(Family::V4) => format!("lowerAddrV4_({e})"),
			Type::SocketAddr(Family::V6) => format!("lowerAddrV6_({e})"),
			Type::Waker => format!("waker_({e})"),
			Type::Reader | Type::Future => unreachable!("{ty:?} crosses only as a parameter"),
			Type::Range(_) | Type::Bound(_) | Type::Bounds(_) => {
				self.convert(ty);
				format!("lower_{}({e})", abi.range(ty))
			}
			Type::List(_, inner) => {
				let c = abi.list(inner);
				self.lower_list(&c, inner);
				format!("lower_{c}(a_, {e})")
			}
			Type::Option(inner) => match inner.base() {
				object @ Type::Object(i) => {
					self.convert(object);
					format!("holdOption_{}(a_, {e})", abi.object(*i))
				}
				Type::Str | Type::String | Type::Path | Type::PathBuf => format!("a_.bufStringOption({e})"),
				Type::Slice | Type::Vec | Type::Bytes => format!("a_.bufViewOption({e})"),
				Type::List(_, elem) => {
					let c = abi.list(elem);
					self.lower_list(&c, elem);
					let name = format!("lower_option_{c}");
					let go = self.go(inner);
					self.helper(&name, |_| {
						format!(
							"func {name}(a_ *arena_, v *{go}) C.{c} {{\n\tif v == nil {{\n\t\treturn C.{c}{{}}\n\t}}\n\treturn lower_{c}(a_, *v)\n}}\n\n"
						)
					});
					format!("{name}(a_, {e})")
				}
				_ => {
					let c = abi.option(inner);
					let arena = self.arena(inner);
					let inner = (**inner).clone();
					self.helper(&format!("lower_{c}"), |g| {
						// A Go pointer already, which nil is none of.
						let (go, value) = match inner.base() {
							Type::U128 => (g.go(&inner), g.lower(&inner, "v")),
							_ => (format!("*{}", g.go(&inner)), g.lower(&inner, "*v")),
						};
						let param = g.arena_param(&inner);
						format!(
							"func lower_{c}({param}v {go}) C.{c} {{\n\tif v == nil {{\n\t\treturn C.{c}{{}}\n\t}}\n\treturn C.{c}{{some: true, value: {value}}}\n}}\n\n"
						)
					});
					match arena {
						true => format!("lower_{c}(a_, {e})"),
						false => format!("lower_{c}({e})"),
					}
				}
			},
			Type::Box(inner) | Type::Ref(_, inner) => self.lower(inner, e),
			Type::Record(_) | Type::Enum(_) => {
				self.convert(ty);
				let c = match abi.ty(ty) {
					Ty::Struct(c) | Ty::Enum(c) => c,
					other => unreachable!("a value is a struct or enum, not {other:?}"),
				};
				format!("lower_{c}({}{e})", self.arena_arg(ty))
			}
			Type::Object(i) => {
				let e = match e.starts_with('*') {
					true => format!("({e})"),
					false => e.to_string(),
				};
				format!("(*C.{})(a_.hold({e}.Handle_, {e}))", abi.object(*i))
			}
			Type::Callback(i, _) => {
				self.convert(ty);
				format!("lower_{}({e})", abi.callback(*i))
			}
			Type::Tuple(elements) => match &elements[..] {
				[element] => {
					let c = abi.tuple(elements);
					let value = self.lower(element, e);
					format!("C.{c}{{_0: {value}}}")
				}
				elements => {
					let c = abi.tuple(elements);
					let arena = self.arena(ty);
					let elements = elements.to_vec();
					let ty = ty.clone();
					self.helper(&format!("lower_{c}"), |g| {
						let go = g.go(&ty);
						let fields: Vec<String> = (elements.iter().enumerate())
							.map(|(k, el)| format!("_{k}: {}", g.lower(el, &format!("v.V{k}"))))
							.collect();
						let param = g.arena_param(&ty);
						format!(
							"func lower_{c}({param}v {go}) C.{c} {{\n\treturn C.{c}{{{}}}\n}}\n\n",
							fields.join(", ")
						)
					});
					match arena {
						true => format!("lower_{c}(a_, {e})"),
						false => format!("lower_{c}({e})"),
					}
				}
			},
			Type::Closure(..) => unreachable!("a closure lowers to its trampoline"),
		}
	}

	/// The helper lowering a `[]elem` into the C array struct `c`: a block of numbers lent
	/// in place, pinned, as bytes are.
	fn lower_list(&mut self, c: &str, elem: &Type) {
		let elem = elem.clone();
		self.helper(&format!("lower_{c}"), |g| {
			let go = g.go(&elem);
			if block(&elem) {
				let elem_c = g.c_go(&g.abi.ty(&elem));
				return format!(
					"func lower_{c}(a_ *arena_, v []{go}) C.{c} {{\n\tptr, n := block_[{go}, {elem_c}](a_, v)\n\treturn C.{c}{{ptr: ptr, len: n}}\n}}\n\n"
				);
			}
			let item = g.lower(&elem, "x");
			let elem_c = g.c_go(&g.abi.ty(&elem));
			format!(
				"func lower_{c}(a_ *arena_, v []{go}) C.{c} {{\n\tptr, n := list_(a_, v, func(x {go}) {elem_c} {{\n\t\treturn {item}\n\t}})\n\treturn C.{c}{{ptr: ptr, len: n}}\n}}\n\n"
			)
		});
	}

	/// The Go value of the C value `e` of type `ty`, which it takes ownership of.
	fn lift(&mut self, ty: &Type, e: &str) -> String {
		let abi = self.abi;
		match ty {
			Type::Str | Type::String | Type::Path | Type::PathBuf => format!("liftString_({e})"),
			Type::Slice | Type::Vec | Type::Bytes => format!("liftBytes_({e})"),
			Type::Bool => format!("bool({e})"),
			Type::Int(int) | Type::NonZero(int) => format!("{}({e})", go_int(*int)),
			Type::F32 => format!("float32({e})"),
			Type::F64 => format!("float64({e})"),
			Type::Char => format!("rune({e})"),
			Type::U128 => format!("liftU128_({e})"),
			Type::Duration => format!("liftDuration_({e})"),
			Type::Array(n) => {
				self.array(*n);
				format!("lift_{}({e})", abi.array(*n))
			}
			Type::SocketAddr(_) => format!("liftAddr_({e})"),
			Type::Range(_) | Type::Bound(_) | Type::Bounds(_) => {
				self.convert(ty);
				format!("lift_{}({e})", abi.range(ty))
			}
			Type::List(_, inner) => {
				let c = abi.list(inner);
				self.lift_list(&c, inner);
				format!("lift_{c}({e})")
			}
			Type::Option(inner) => match inner.base() {
				object @ Type::Object(i) => {
					self.convert(object);
					format!("own_{}({e})", abi.object(*i))
				}
				Type::Str | Type::String | Type::Path | Type::PathBuf => format!("liftStringOption_({e})"),
				Type::Slice | Type::Vec | Type::Bytes => format!("liftBytesOption_({e})"),
				Type::List(_, elem) => {
					let c = abi.list(elem);
					self.lift_list(&c, elem);
					let name = format!("lift_option_{c}");
					let go = self.go(inner);
					self.helper(&name, |_| {
						format!(
							"func {name}(c C.{c}) *{go} {{\n\tif c.ptr == nil {{\n\t\treturn nil\n\t}}\n\tv := lift_{c}(c)\n\treturn &v\n}}\n\n"
						)
					});
					format!("{name}({e})")
				}
				_ => {
					let c = abi.option(inner);
					let inner = (**inner).clone();
					self.helper(&format!("lift_{c}"), |g| {
						let value = g.lift(&inner, "c.value");
						let (go, v) = match inner.base() {
							Type::U128 => (g.go(&inner), "v"),
							_ => (format!("*{}", g.go(&inner)), "&v"),
						};
						format!(
							"func lift_{c}(c C.{c}) {go} {{\n\tif !c.some {{\n\t\treturn nil\n\t}}\n\tv := {value}\n\treturn {v}\n}}\n\n"
						)
					});
					format!("lift_{c}({e})")
				}
			},
			Type::Box(inner) | Type::Ref(_, inner) => self.lift(inner, e),
			Type::Record(_) | Type::Enum(_) => {
				self.convert(ty);
				let c = match abi.ty(ty) {
					Ty::Struct(c) | Ty::Enum(c) => c,
					other => unreachable!("a value is a struct or enum, not {other:?}"),
				};
				format!("lift_{c}({e})")
			}
			Type::Object(i) => {
				self.convert(ty);
				format!("own_{}({e})", abi.object(*i))
			}
			Type::Waker => format!("liftWaker_({e})"),
			Type::Tuple(elements) => match &elements[..] {
				[element] => self.lift(element, &format!("{e}._0")),
				elements => {
					let c = abi.tuple(elements);
					let elements = elements.to_vec();
					let ty = ty.clone();
					self.helper(&format!("lift_{c}"), |g| {
						let go = g.go(&ty);
						let fields: Vec<String> = (elements.iter().enumerate())
							.map(|(k, el)| format!("V{k}: {}", g.lift(el, &format!("c._{k}"))))
							.collect();
						format!(
							"func lift_{c}(c C.{c}) {go} {{\n\treturn {go}{{{}}}\n}}\n\n",
							fields.join(", ")
						)
					});
					format!("lift_{c}({e})")
				}
			},
			other => unreachable!("{other:?} is never returned"),
		}
	}

	/// The helper lifting the C array struct `c` of `elem`s, then freeing the array alone.
	fn lift_list(&mut self, c: &str, elem: &Type) {
		let elem = elem.clone();
		self.helper(&format!("lift_{c}"), |g| {
			let go = g.go(&elem);
			// A block of numbers is copied out whole.
			if block(&elem) {
				return format!(
					"func lift_{c}(c C.{c}) []{go} {{\n\tv := liftBlock_[{go}](c.ptr, c.len)\n\tC.{c}_free(c)\n\treturn v\n}}\n\n"
				);
			}
			let item = g.lift(&elem, "x");
			let elem_c = g.c_go(&g.abi.ty(&elem));
			format!(
				"func lift_{c}(c C.{c}) []{go} {{\n\tv := liftList_(c.ptr, c.len, func(x {elem_c}) {go} {{\n\t\treturn {item}\n\t}})\n\tc.len = 0\n\tC.{c}_free(c)\n\treturn v\n}}\n\n"
			)
		});
	}

	/// The Go value of the C value `e` of type `ty`, which C lends a callback or closure
	/// for its call: bytes and text copied, and a scalar as it is.
	fn lift_lent(&mut self, ty: &Type, e: &str) -> String {
		match (ty.buffer(), ty) {
			(Some(Type::Bytes), _) => format!("lentBytes_({e})"),
			(Some(Type::Option(inner)), _) if *inner == Type::Bytes => format!("lentBytesOption_({e})"),
			(Some(Type::Option(_)), _) => format!("lentStringOption_({e})"),
			(Some(_), _) => format!("lentString_({e})"),
			(None, Type::Ref(_, inner)) => self.lift_lent(inner, e),
			(None, ty) => self.lift(ty, e),
		}
	}

	/// The package's items: those of its crate.
	fn write(&mut self) {
		let abi = self.abi;
		let api = abi.api;
		let pkg = self.pkg;
		let mine = |home: &Home| home.krate == pkg;
		for constant in &api.constants {
			if !mine(&constant.home) {
				continue;
			}
			let c = abi.name(&constant.path);
			let name = rest(&abi.prefix, &c, &constant.path, &constant.home).to_uppercase();
			let name = self.claim(&name, format!("const `{}`", constant.path.join("::")));
			self.locked
				.insert(format!("const {}", constant.path.join("::")), name.clone());
			let (ty, value) = match constant.value {
				Literal::Bool(b) => ("bool", b.to_string()),
				Literal::Int(int, v) => (go_int(int), v.to_string()),
			};
			let _ = write!(self.top, "{}const {name} {ty} = {value}\n\n", docs(&constant.docs, ""));
		}
		for i in (0..api.records.len()).filter(|&i| mine(&api.records[i].home)) {
			self.record(i);
		}
		for i in (0..api.enums.len()).filter(|&i| mine(&api.enums[i].home)) {
			self.enumeration(i);
		}
		for i in (0..api.objects.len()).filter(|&i| mine(&api.objects[i].home)) {
			self.object(i);
		}
		for i in (0..api.callbacks.len()).filter(|&i| mine(&api.callbacks[i].home)) {
			self.callback(i);
		}
		for function in api.functions.iter().filter(|f| mine(&f.home)) {
			let mut method = self.function(function, None, None);
			method.name = self.claim(&method.name, format!("fn `{}`", function.path.join("::")));
			crate::lock::function(&mut self.locked, function, &method.name);
			self.top.push_str(&method.write());
		}
		self.moved();
		self.errors_of();
	}

	/// Each method of another package's type whose instance names a type of this one's,
	/// which Go keeps from that package, as Go keeps a type's methods in its own: a function
	/// of this one, named through its type and taking its receiver first.
	fn moved(&mut self) {
		let abi = self.abi;
		let api = abi.api;
		let records = (0..api.records.len()).map(|i| {
			let c = abi.record(i);
			(
				Type::Record(i),
				Ty::Struct(c),
				&api.records[i].home,
				&api.records[i].methods,
			)
		});
		let enums = (0..api.enums.len()).map(|i| {
			let c = abi.enumeration(i);
			let owner = match api.enums[i].plain() {
				true => Ty::Enum(c),
				false => Ty::Struct(c),
			};
			(Type::Enum(i), owner, &api.enums[i].home, &api.enums[i].methods)
		});
		let objects = (0..api.objects.len()).map(|i| {
			let owner = Ty::Handle {
				name: abi.object(i),
				constant: false,
			};
			(Type::Object(i), owner, &api.objects[i].home, &api.objects[i].methods)
		});
		let pkg = self.pkg;
		for (ty, owner, home, methods) in records.chain(enums).chain(objects) {
			if home.krate == pkg {
				continue;
			}
			for function in methods.iter().filter(|f| f.home.krate == pkg) {
				let mut method = self.function(function, Some(&owner), Some(&ty));
				if let Some(receiver) = method.receiver.take() {
					let go = receiver
						.strip_prefix("self ")
						.expect("a receiver is `self`")
						.to_string();
					method.params.insert(0, ("self".to_string(), go));
					method.name = format!("{}{}", self.named(&ty).1, method.name);
				}
				method.name = self.claim(&method.name, format!("fn `{}`", function.path.join("::")));
				crate::lock::function(&mut self.locked, function, &method.name);
				self.top.push_str(&method.write());
			}
		}
	}

	/// The runtime's types: each range's and bound's, and each error's of no crate's.
	fn write_rt(&mut self) {
		let abi = self.abi;
		for name in TAKEN {
			self.names.insert(name.to_string(), "the runtime".to_string());
		}
		if abi
			.ranges
			.values()
			.any(|ty| matches!(ty, Type::Bound(_) | Type::Bounds(_)))
		{
			self.bound();
		}
		for (c, ty) in &abi.ranges.clone() {
			self.range(c, ty);
		}
		self.errors_of();
	}

	/// The struct of each error of no exported type the package's crate returns.
	fn errors_of(&mut self) {
		for (name, (home, ty)) in self.thrown.iter() {
			if home != self.pkg {
				continue;
			}
			let go = self.go(ty);
			let message = match ty {
				Type::String => "e.Value".to_string(),
				_ => format!("\"{}\"", self.describe(ty)),
			};
			let _ = write!(
				self.types,
				"// {name} is a `{}` Rust returned as an error.\ntype {name} struct {{\n\tValue {go}\n}}\n\nfunc (e {name}) Error() string {{\n\treturn {message}\n}}\n\n",
				self.describe(ty)
			);
		}
	}

	/// How `ty` reads in Rust, for a doc comment.
	fn describe(&self, ty: &Type) -> String {
		let abi = self.abi;
		match ty {
			Type::Record(i) => abi.api.rust(&abi.api.records[*i].path, &abi.api.records[*i].args),
			Type::Enum(i) => abi.api.rust(&abi.api.enums[*i].path, &abi.api.enums[*i].args),
			Type::Object(i) => abi.api.rust(&abi.api.objects[*i].path, &abi.api.objects[*i].args),
			Type::String => "String".to_string(),
			Type::Tuple(elements) => {
				let elements: Vec<String> = elements.iter().map(|e| self.describe(e)).collect();
				format!("({})", elements.join(", "))
			}
			Type::Bound(int) => format!("Bound<{}>", int.rust()),
			Type::Range(int) => format!("Range<{}>", int.rust()),
			Type::Bounds(int) => format!("(Bound<{0}>, Bound<{0}>)", int.rust()),
			Type::Option(inner) => format!("Option<{}>", self.describe(inner)),
			Type::List(_, inner) => format!("Vec<{}>", self.describe(inner)),
			Type::Int(int) | Type::NonZero(int) => int.rust().to_string(),
			Type::Ref(_, inner) | Type::Box(inner) => self.describe(inner),
			other => format!("{other:?}"),
		}
	}

	/// A struct for a range or pair of bounds, or one of a kind and a value for a bound.
	fn range(&mut self, c: &str, ty: &Type) {
		let name = self.class(c);
		self.names.insert(name.clone(), format!("`{c}`"));
		let code = match ty {
			Type::Range(int) => {
				let go = go_int(*int);
				format!(
					"// {name} is Rust's Range: from Start, up to but not including End.\ntype {name} struct {{\n\tStart {go}\n\tEnd   {go}\n}}\n\n"
				)
			}
			Type::Bound(int) => {
				let go = go_int(*int);
				format!(
					"// {name} is Rust's Bound: what a range's start or end bound does with Value.\ntype {name} struct {{\n\tKind  Bound\n\tValue {go}\n}}\n\n"
				)
			}
			Type::Bounds(int) => {
				let bound = self.class(&self.abi.range(&Type::Bound(*int)));
				format!(
					"// {name} is a range's start and end bounds, as Rust's RangeBounds gives them.\ntype {name} struct {{\n\tStart {bound}\n\tEnd   {bound}\n}}\n\n"
				)
			}
			other => unreachable!("{other:?} is no range"),
		};
		self.types.push_str(&code);
	}

	/// The conversions of a range, bound, or pair of bounds, whose C struct is `c`.
	/// The conversions of `[u8; n]` between Go's array and C's struct of the same bytes.
	fn array(&mut self, n: usize) {
		let c = self.abi.array(n);
		self.helper(&format!("convert_{c}"), |_| {
			format!(
				"func lift_{c}(c C.{c}) [{n}]byte {{\n\treturn *(*[{n}]byte)(unsafe.Pointer(&c.items))\n}}\n\n\
				func lower_{c}(v [{n}]byte) C.{c} {{\n\treturn *(*C.{c})(unsafe.Pointer(&v))\n}}\n\n"
			)
		});
	}

	fn range_conv(&mut self, c: &str, ty: &Type) -> String {
		let name = self.qualified(ty);
		match ty {
			Type::Range(int) => {
				let go = go_int(*int);
				let cint = c_int(*int);
				format!(
					"func lift_{c}(c C.{c}) {name} {{\n\treturn {name}{{Start: {go}(c.start), End: {go}(c.end)}}\n}}\n\n\
					func lower_{c}(v {name}) C.{c} {{\n\treturn C.{c}{{start: C.{cint}(v.Start), end: C.{cint}(v.End)}}\n}}\n\n"
				)
			}
			Type::Bound(int) => {
				let go = go_int(*int);
				let cint = c_int(*int);
				let kind = self.abi.rt("bound");
				let bound = self.qualify(Packages::RT, "Bound");
				format!(
					"func lift_{c}(c C.{c}) {name} {{\n\treturn {name}{{Kind: {bound}(c.kind), Value: {go}(c.value)}}\n}}\n\n\
					func lower_{c}(v {name}) C.{c} {{\n\treturn C.{c}{{kind: C.{kind}(v.Kind), value: C.{cint}(v.Value)}}\n}}\n\n"
				)
			}
			Type::Bounds(int) => {
				let bound = Type::Bound(*int);
				self.convert(&bound);
				let bound_c = self.abi.range(&bound);
				format!(
					"func lift_{c}(c C.{c}) {name} {{\n\treturn {name}{{Start: lift_{bound_c}(c.start), End: lift_{bound_c}(c.end)}}\n}}\n\n\
					func lower_{c}(v {name}) C.{c} {{\n\treturn C.{c}{{start: lower_{bound_c}(v.Start), end: lower_{bound_c}(v.End)}}\n}}\n\n"
				)
			}
			other => unreachable!("{other:?} is no range"),
		}
	}

	/// The kind of a bound, which the runtime's names claim.
	fn bound(&mut self) {
		self.types.push_str(
			"// Bound is what a range's start or end bound does: nothing, include its value, or\n// exclude it.\ntype Bound int32\n\nconst (\n\tBoundUnbounded Bound = 0\n\tBoundIncluded  Bound = 1\n\tBoundExcluded  Bound = 2\n)\n\n",
		);
	}

	/// Add the conversions of the exported type `ty` to the package, once: its lift and
	/// lower, an object's own and lend, a callback's lower.
	fn convert(&mut self, ty: &Type) {
		let abi = self.abi;
		let (key, ty) = match ty {
			Type::Record(i) => (abi.record(*i), ty.clone()),
			Type::Enum(i) => (abi.enumeration(*i), ty.clone()),
			Type::Object(i) => (abi.object(*i), ty.clone()),
			Type::Callback(i, _) => (abi.callback(*i), Type::Callback(*i, crate::ir::Dyn::Impl)),
			Type::Range(_) | Type::Bound(_) | Type::Bounds(_) => (abi.range(ty), ty.clone()),
			other => unreachable!("{other:?} converts as it is"),
		};
		self.helper(&format!("convert_{key}"), |g| match &ty {
			Type::Record(i) => g.record_conv(*i),
			Type::Enum(i) => g.enum_conv(*i),
			Type::Object(i) => g.object_conv(*i),
			Type::Callback(i, _) => g.callback_conv(*i),
			ty => g.range_conv(&key, ty),
		});
	}

	/// The methods a type's std traits export: `Display` is `String`, `Debug` `GoString`,
	/// `PartialEq` `Equal`, and `Clone` an object's `Clone`; and the package's function for
	/// `Default`.
	fn traits(&mut self, ty: &Type, c: &str, t: crate::ir::Traits) -> (Vec<Method>, Option<Method>) {
		let name = self.go(ty);
		let object = matches!(ty, Type::Object(_));
		let arena = self.arena(ty);
		// A value's conversion lowers its receiver, and an object's lifts a clone.
		if !object || t.clone || t.default {
			self.convert(ty);
		}
		// A value holding an object fails once that object is closed, as an object does.
		let closable = object || self.holds(ty);
		// The receiver, a held handle or the value lowered, and the return of `failed`, an
		// expression of the error, once a closed object fails the call.
		let open = |failed: &dyn Fn(&str) -> String| match (object, arena) {
			(true, _) => (
				format!(
					"h_ := self.Handle_.Hold()\nif h_ == nil {{\n\treturn {}\n}}\ndefer unhold_(self.Handle_, self)\n",
					failed("self.Handle_.Closed()")
				),
				format!("(*C.{c})(h_)"),
			),
			(false, true) if closable => (
				format!(
					"a_ := &arena_{{}}\ndefer a_.finish()\nc_self_ := lower_{c}(a_, self)\nif a_.err != nil {{\n\treturn {}\n}}\n",
					failed("a_.err")
				),
				"&c_self_".to_string(),
			),
			(false, true) => (
				format!("a_ := &arena_{{}}\ndefer a_.finish()\nc_self_ := lower_{c}(a_, self)\n"),
				"&c_self_".to_string(),
			),
			(false, false) => (format!("c_self_ := lower_{c}(self)\n"), "&c_self_".to_string()),
		};
		let method =
			|docs: &str, member: &str, params: Vec<(String, String)>, results: Vec<String>, body: String| Method {
				key: None,
				docs: docs.to_string(),
				receiver: Some(format!("self {name}")),
				name: member.to_string(),
				params,
				results,
				body,
			};
		// fmt's String and GoString return no error, so a closed one reads as the error.
		let text = |e: &str| format!("{e}.Error()");
		let mut methods = Vec::new();
		if t.display {
			let (open, this) = open(&text);
			methods.push(method(
				"String is Rust's Display form.",
				"String",
				Vec::new(),
				vec!["string".to_string()],
				format!("{open}return liftString_(C.{c}_to_string({this}))"),
			));
		}
		if t.debug {
			let (open, this) = open(&text);
			methods.push(method(
				"GoString is Rust's Debug form.",
				"GoString",
				Vec::new(),
				vec!["string".to_string()],
				format!("{open}return liftString_(C.{c}_debug({this}))"),
			));
		}
		if object && t.clone {
			let (open, this) = open(&|e: &str| format!("nil, {e}"));
			methods.push(method(
				"Clone is a new handle to a clone.",
				"Clone",
				Vec::new(),
				vec![name.clone(), "error".to_string()],
				format!("{open}return own_{c}(C.{c}_clone({this})), nil"),
			));
		}
		if t.eq {
			let failed = |e: &str| format!("false, {e}");
			let (open, this) = match closable {
				true => open(&failed),
				false => open(&|_: &str| unreachable!("a value holding no object is never closed")),
			};
			let (results, ok) = match closable {
				true => (vec!["bool".to_string(), "error".to_string()], ", nil"),
				false => (vec!["bool".to_string()], ""),
			};
			let body = match (object, arena) {
				(true, _) => format!(
					"{open}o_ := other.Handle_.Hold()\nif o_ == nil {{\n\treturn {}\n}}\ndefer unhold_(other.Handle_, other)\nreturn bool(C.{c}_eq({this}, (*C.{c})(o_))){ok}",
					failed("other.Handle_.Closed()")
				),
				(false, true) if closable => format!(
					"{open}c_other_ := lower_{c}(a_, other)\nif a_.err != nil {{\n\treturn {}\n}}\nreturn bool(C.{c}_eq({this}, &c_other_)){ok}",
					failed("a_.err")
				),
				(false, true) => {
					format!("{open}c_other_ := lower_{c}(a_, other)\nreturn bool(C.{c}_eq({this}, &c_other_))")
				}
				(false, false) => {
					format!("{open}c_other_ := lower_{c}(other)\nreturn bool(C.{c}_eq({this}, &c_other_))")
				}
			};
			methods.push(method(
				"Equal is Rust's PartialEq.",
				"Equal",
				vec![("other".to_string(), name.clone())],
				results,
				body,
			));
		}
		let default = t.default.then(|| {
			let func = format!("{}Default", name.trim_start_matches('*'));
			let lift = match object {
				true => format!("own_{c}(C.{c}_default())"),
				false => format!("lift_{c}(C.{c}_default())"),
			};
			Method {
				key: None,
				docs: format!("{func} is Rust's Default."),
				receiver: None,
				name: func,
				params: Vec::new(),
				results: vec![name.clone()],
				body: format!("return {lift}"),
			}
		});
		let (mut methods, mut default) = (methods, default);
		// On a target a value's type is not on, only the binding makes one, which calls in
		// for none of these.
		if let Some(check) = self.on(self.abi.on(ty), c) {
			for method in methods.iter_mut().chain(default.as_mut()) {
				method.body = format!("{check}\n{}", method.body);
			}
		}
		(methods, default)
	}

	fn record(&mut self, i: usize) {
		let abi = self.abi;
		let record = &abi.api.records[i];
		let c = abi.record(i);
		let ty = Type::Record(i);
		let name = self.named(&ty).1;
		let rust = abi.api.rust(&record.path, &record.args);
		if let Some(opaque) = &record.opaque {
			let text = format!(
				"{name} is `{rust}`, its bytes, which only Rust makes or reads: copy it, but never\nmake or change one. A zero {name} panics where Rust would read it, and == is a compile\nerror, since bytes alike need not be equal: Equal, if it has one, is Rust's.\n\n{}",
				record.docs
			);
			let _ = write!(
				self.types,
				"{}type {name} struct {{\n\t_ [0]func()\n\t// Bytes_ is its bytes, for the generated packages: use none of it.\n\tBytes_ [{}]uint{}\n\t// Made_ is whether Rust made it, rather than it being Go's zero value.\n\tMade_ bool\n}}\n\n",
				docs(text.trim(), ""),
				opaque.units(),
				opaque.unit() * 8
			);
			let mut members = vec!["Bytes_".to_string(), "Made_".to_string()];
			let fields = members.clone();
			members.extend(self.methods_of(
				&ty,
				&c,
				&rust,
				record.traits,
				&record.methods,
				&Ty::Struct(c.clone()),
				&fields,
			));
			self.distinct(&rust, &members);
			return;
		}
		let fields: Vec<(String, String)> = (self.record_fields(i).into_iter().zip(&record.fields))
			.map(|(name, f)| (name, self.go(&f.ty)))
			.collect();
		let mut members: Vec<String> = fields.iter().map(|(f, _)| f.clone()).collect();
		let body: String = (record.fields.iter().zip(&fields))
			.map(|(f, (name, go))| format!("{}\t{name} {go}\n", docs(&f.docs, "\t")))
			.collect();
		let text = format!("{name} is `{rust}`.\n\n{}", record.docs);
		let _ = write!(
			self.types,
			"{}type {name} struct {{\n{body}}}\n\n",
			docs(text.trim(), "")
		);
		let fields: Vec<String> = members.clone();
		members.extend(self.methods_of(
			&ty,
			&c,
			&rust,
			record.traits,
			&record.methods,
			&Ty::Struct(c.clone()),
			&fields,
		));
		self.distinct(&rust, &members);
	}

	/// The Go names of a record's fields: each [`field_name`], with a trailing `_` while
	/// a method its traits give it, which an interface requires by name, or an earlier
	/// field takes it.
	fn record_fields(&mut self, i: usize) -> Vec<String> {
		let record = &self.abi.api.records[i];
		let t = record.traits;
		let methods = [
			(t.display, "String"),
			(t.debug, "GoString"),
			(t.eq, "Equal"),
			(self.errored(&Type::Record(i)), "Error"),
		];
		let mut taken: BTreeSet<String> = (methods.iter())
			.filter(|(has, _)| *has)
			.map(|(_, name)| name.to_string())
			.collect();
		(record.fields.iter())
			.map(|f| {
				let mut name = field_name(&f.name);
				while !taken.insert(name.clone()) {
					name.push('_');
				}
				self.locked.insert(
					format!("{} / field {}", crate::lock::type_key(&record.shown), f.name),
					name.clone(),
				);
				name
			})
			.collect()
	}

	/// A record's lift and lower.
	fn record_conv(&mut self, i: usize) -> String {
		let abi = self.abi;
		let record = &abi.api.records[i];
		let c = abi.record(i);
		let ty = Type::Record(i);
		let name = self.qualified(&ty);
		if let Some(opaque) = &record.opaque {
			let (local, bytes) = (
				self.named(&ty).1,
				format!("[{}]uint{}", opaque.units(), opaque.unit() * 8),
			);
			return format!(
				"func lift_{c}(c C.{c}) {name} {{\n\treturn {name}{{Bytes_: *(*{bytes})(unsafe.Pointer(&c)), Made_: true}}\n}}\n\n\
				func lower_{c}(v {name}) C.{c} {{\n\tif !v.Made_ {{\n\t\tpanic(\"a zero {local}, which only Rust makes\")\n\t}}\n\treturn *(*C.{c})(unsafe.Pointer(&v.Bytes_))\n}}\n\n"
			);
		}
		let names = self.record_fields(i);
		let lifted: Vec<String> = (record.fields.iter().zip(&names))
			.map(|(f, field)| {
				let value = self.lift(&f.ty, &format!("c.{}", cgo_field(&abi::escape(&f.name))));
				format!("{field}: {value}")
			})
			.collect();
		let lowered: Vec<String> = (record.fields.iter().zip(&names))
			.map(|(f, field)| {
				let value = self.lower(&f.ty, &format!("v.{field}"));
				format!("{}: {value}", cgo_field(&abi::escape(&f.name)))
			})
			.collect();
		let param = self.arena_param(&ty);
		format!(
			"func lift_{c}(c C.{c}) {name} {{\n\treturn {name}{{{}}}\n}}\n\nfunc lower_{c}({param}v {name}) C.{c} {{\n\treturn C.{c}{{{}}}\n}}\n\n",
			lifted.join(", "),
			lowered.join(", ")
		)
	}

	/// The methods and functions of a record, plain enum, or object, written after its type,
	/// returning its methods' names. A method named as one of its `fields` or an earlier
	/// method, as its traits' come first, takes a trailing `_`.
	#[allow(
		clippy::too_many_arguments,
		reason = "a type's every part, which its three kinds each pass"
	)]
	fn methods_of(
		&mut self,
		ty: &Type,
		c: &str,
		rust: &str,
		traits: crate::ir::Traits,
		functions: &[Function],
		owner: &Ty,
		fields: &[String],
	) -> Vec<String> {
		let (methods, statics) = self.members(ty, c, rust, traits, functions, owner);
		let mut members = Vec::new();
		let mut taken: BTreeSet<String> = fields.iter().cloned().collect();
		for mut method in methods {
			while !taken.insert(method.name.clone()) {
				method.name.push('_');
			}
			if let Some(key) = &method.key {
				self.locked.insert(key.clone(), method.name.clone());
			}
			members.push(method.name.clone());
			self.types.push_str(&method.write());
		}
		for function in statics {
			self.types.push_str(&function.write());
		}
		members
	}

	/// The methods of a type, and its functions, each of which it claims: its traits', its
	/// own, and `Error` if a function returns it as its error.
	fn members(
		&mut self,
		ty: &Type,
		c: &str,
		rust: &str,
		traits: crate::ir::Traits,
		functions: &[Function],
		owner: &Ty,
	) -> (Vec<Method>, Vec<Method>) {
		let (mut methods, default) = self.traits(ty, c, traits);
		let mut statics = Vec::new();
		if let Some(mut default) = default {
			default.name = self.claim(&default.name, format!("`{rust}`'s Default"));
			let api = self.abi.api;
			let shown = match ty {
				Type::Record(i) => &api.records[*i].shown,
				Type::Enum(i) => &api.enums[*i].shown,
				Type::Object(i) => &api.objects[*i].shown,
				_ => unreachable!("only an exported type has members"),
			};
			self.locked.insert(
				format!("{} / Default", crate::lock::type_key(shown)),
				default.name.clone(),
			);
			statics.push(default);
		}
		let pkg = self.pkg;
		for function in functions.iter().filter(|f| f.home.krate == pkg) {
			let mut method = self.function(function, Some(owner), Some(ty));
			match method.receiver {
				Some(_) => methods.push(method),
				None => {
					method.name = self.claim(&method.name, format!("fn `{}`", function.path.join("::")));
					crate::lock::function(&mut self.locked, function, &method.name);
					statics.push(method);
				}
			}
		}
		if self.errored(ty) {
			let message = match (traits.display, traits.debug) {
				(true, _) => "self.String()".to_string(),
				(false, true) => "self.GoString()".to_string(),
				(false, false) => format!("{rust:?}"),
			};
			methods.push(Method {
				key: None,
				docs: "Error is its message, as an error Rust returned.".to_string(),
				receiver: Some(format!("self {}", self.go(ty))),
				name: "Error".to_string(),
				params: Vec::new(),
				results: vec!["string".to_string()],
				body: format!("return {message}"),
			});
		}
		(methods, statics)
	}

	/// Whether a function returns `ty` as its error, which it then implements.
	fn errored(&self, ty: &Type) -> bool {
		let api = self.abi.api;
		let mut all = (api.records.iter().flat_map(|r| &r.methods))
			.chain(api.enums.iter().flat_map(|e| &e.methods))
			.chain(api.objects.iter().flat_map(|o| &o.methods))
			.chain(&api.functions);
		all.any(|f| f.err.as_ref().is_some_and(|e| e.base() == ty))
	}

	fn enumeration(&mut self, i: usize) {
		let abi = self.abi;
		let e = &abi.api.enums[i];
		let c = abi.enumeration(i);
		let ty = Type::Enum(i);
		let name = self.named(&ty).1;
		let rust = abi.api.rust(&e.path, &e.args);
		let text = format!("{name} is `{rust}`.\n\n{}", e.docs);
		if e.plain() {
			let consts: String = (e.variants.iter())
				.map(|v| {
					let variant = &self.variants[&(c.clone(), v.name.clone())];
					format!("\t{variant} {name} = {}\n", v.discriminant)
				})
				.collect();
			let _ = write!(
				self.types,
				"{}type {name} int32\n\nconst (\n{consts})\n\n",
				docs(text.trim(), "")
			);
			let members = self.methods_of(&ty, &c, &rust, e.traits, &e.methods, &Ty::Enum(c.clone()), &[]);
			self.distinct(&rust, &members);
			return;
		}
		// An interface each variant's struct implements, sealed by a method of its own. Each
		// method is a function of the interface, which each variant's forwards to.
		let seal = format!("is_{c}");
		let (methods, statics) = self.members(&ty, &c, &rust, e.traits, &e.methods, &Ty::Struct(c.clone()));
		let mut interface = format!("\t{seal}()\n");
		let mut shared = String::new();
		for method in &methods {
			let _ = writeln!(interface, "{}\t{}", docs(method.docs.trim(), "\t"), method.signature());
			let mut function = Method {
				key: None,
				docs: String::new(),
				receiver: None,
				name: format!("{c}_{}", abi::snake(&method.name)),
				params: method.params.clone(),
				results: method.results.clone(),
				body: method.body.clone(),
			};
			function.params.insert(0, ("self".to_string(), name.clone()));
			shared.push_str(&function.write());
		}
		let _ = write!(
			self.types,
			"{}type {name} interface {{\n{interface}}}\n\n{shared}",
			docs(text.trim(), "")
		);
		for variant in &e.variants {
			let vname = self.variants[&(c.clone(), variant.name.clone())].clone();
			let fields: Vec<(String, String)> = (variant.fields.iter())
				.map(|f| (field_name(&f.name), self.go(&f.ty)))
				.collect();
			let mut members: Vec<String> = fields.iter().map(|(f, _)| f.clone()).collect();
			members.extend(methods.iter().map(|m| m.name.clone()));
			self.distinct(&format!("{rust}::{}", variant.name), &members);
			let body: String = fields.iter().map(|(f, go)| format!("\t{f} {go}\n")).collect();
			let _ = write!(
				self.types,
				"// {vname} is `{rust}::{}`.\ntype {vname} struct {{\n{body}}}\n\nfunc ({vname}) {seal}() {{}}\n\n",
				variant.name
			);
			for method in &methods {
				let mut args = vec!["self".to_string()];
				args.extend(method.params.iter().map(|(n, _)| n.clone()));
				let call = format!("{c}_{}({})", abi::snake(&method.name), args.join(", "));
				let forward = Method {
					key: None,
					docs: String::new(),
					receiver: Some(format!("self {vname}")),
					name: method.name.clone(),
					params: method.params.clone(),
					results: method.results.clone(),
					body: match method.results.is_empty() {
						true => call,
						false => format!("return {call}"),
					},
				};
				self.types.push_str(&forward.write());
			}
		}
		for function in statics {
			self.types.push_str(&function.write());
		}
	}

	/// An enum's lift and lower.
	fn enum_conv(&mut self, i: usize) -> String {
		let abi = self.abi;
		let e = &abi.api.enums[i];
		let c = abi.enumeration(i);
		let ty = Type::Enum(i);
		let name = self.qualified(&ty);
		// A value of a variant the library's target lacks is refused as it crosses.
		let checks: Vec<Option<String>> = (e.variants.iter())
			.map(|v| self.on(v.on, &format!("{c}.{}", v.name)))
			.collect();
		if e.plain() {
			let cases: String = (e.variants.iter().zip(&checks))
				.filter_map(|(v, check)| Some(format!("\tcase {}:\n\t\t{}\n", v.discriminant, check.as_ref()?)))
				.collect();
			let check = match cases.is_empty() {
				true => String::new(),
				false => format!("\tswitch v {{\n{cases}\t}}\n"),
			};
			return format!(
				"func lift_{c}(c C.{c}) {name} {{\n\treturn {name}(c)\n}}\n\nfunc lower_{c}(v {name}) C.{c} {{\n{check}\treturn C.{c}(v)\n}}\n\n"
			);
		}
		let mut lifts = Vec::new();
		let mut lowers = Vec::new();
		let (krate, _) = self.named(&ty);
		for ((n, variant), check) in e.variants.iter().enumerate().zip(checks) {
			let vname = self.qualify(krate, &self.variants[&(c.clone(), variant.name.clone())]);
			let payload = abi::payload(&c, &variant.name);
			let check = check.map(|check| format!("\t\t{check}\n")).unwrap_or_default();
			if variant.fields.is_empty() {
				lifts.push(format!("\tcase {n}:\n\t\treturn {vname}{{}}\n"));
				lowers.push(format!("\tcase {vname}:\n{check}\t\tc.tag = {n}\n"));
				continue;
			}
			let lifted: Vec<String> = (variant.fields.iter())
				.map(|f| {
					let value = self.lift(&f.ty, &format!("p.{}", cgo_field(&abi::escape(&f.name))));
					format!("{}: {value}", field_name(&f.name))
				})
				.collect();
			let lowered: Vec<String> = (variant.fields.iter())
				.map(|f| {
					let value = self.lower(&f.ty, &format!("v.{}", field_name(&f.name)));
					format!("{}: {value}", cgo_field(&abi::escape(&f.name)))
				})
				.collect();
			lifts.push(format!(
				"\tcase {n}:\n\t\tp := (*C.{payload})(unsafe.Pointer(&c.anon0))\n\t\treturn {vname}{{{}}}\n",
				lifted.join(", ")
			));
			lowers.push(format!(
				"\tcase {vname}:\n{check}\t\tc.tag = {n}\n\t\t*(*C.{payload})(unsafe.Pointer(&c.anon0)) = C.{payload}{{{}}}\n",
				lowered.join(", ")
			));
		}
		let param = self.arena_param(&ty);
		format!(
			"func lift_{c}(c C.{c}) {name} {{\n\tswitch c.tag {{\n{}\t}}\n\tpanic(\"{c}: an invalid tag\")\n}}\n\n\
			func lower_{c}({param}v {name}) C.{c} {{\n\tvar c C.{c}\n\tswitch v := v.(type) {{\n{}\tdefault:\n\t\tpanic(\"a nil {name}\")\n\t}}\n\treturn c\n}}\n\n",
			lifts.join(""),
			lowers.join("")
		)
	}

	fn object(&mut self, i: usize) {
		let abi = self.abi;
		let object = &abi.api.objects[i];
		let c = abi.object(i);
		let ty = Type::Object(i);
		let name = self.named(&ty).1;
		let rust = abi.api.rust(&object.path, &object.args);
		let handle = self.qualify(Packages::RT, "Handle_");
		let text = format!(
			"{name} is `{rust}`, a handle Close frees, or else the GC once it collects it.\n\n{}",
			object.docs
		);
		let _ = write!(
			self.types,
			"{}type {name} struct {{\n\t// Handle_ is the handle it owns, for the generated packages: use none of it.\n\tHandle_ *{handle}\n}}\n\n\
			// Close frees the handle once no call holds it, rather than once the GC collects it: a\n\
			// call after it panics.\n\
			func (self *{name}) Close() {{\n\tself.Handle_.Close()\n}}\n\n",
			docs(text.trim(), "")
		);
		let mut members = vec!["Close".to_string(), "Handle_".to_string()];
		let owner = Ty::Handle {
			name: c.clone(),
			constant: false,
		};
		let fields = members.clone();
		members.extend(self.methods_of(&ty, &c, &rust, object.traits, &object.methods, &owner, &fields));
		self.distinct(&rust, &members);
	}

	/// An object's free, own, lend, and hold of one that may be nil.
	fn object_conv(&mut self, i: usize) -> String {
		let c = self.abi.object(i);
		let ty = Type::Object(i);
		let (_, local) = self.named(&ty);
		let name = self.qualified(&ty);
		let handle = self.qualify(Packages::RT, "Handle_");
		let new = self.qualify(Packages::RT, "NewHandle_");
		format!(
			"func free_{c}(p unsafe.Pointer) {{\n\tC.{c}_free((*C.{c})(p))\n}}\n\n\
			// own_{c} owns a handle Rust returned, or is nil for none.\n\
			func own_{c}(raw *C.{c}) *{name} {{\n\tif raw == nil {{\n\t\treturn nil\n\t}}\n\tv := &{name}{{Handle_: {new}(unsafe.Pointer(raw), free_{c}, \"{local}\")}}\n\tv.Handle_.Cleanup = runtime.AddCleanup(v, (*{handle}).Close, v.Handle_)\n\treturn v\n}}\n\n\
			// lend_{c} is a handle Rust lends for a call, which it frees itself.\n\
			func lend_{c}(raw *C.{c}) *{name} {{\n\treturn &{name}{{Handle_: {new}(unsafe.Pointer(raw), nil, \"{local}\")}}\n}}\n\n\
			// holdOption_{c} is v's handle, held until the call returns, or nil for none.\n\
			func holdOption_{c}(a_ *arena_, v *{name}) *C.{c} {{\n\tif v == nil {{\n\t\treturn nil\n\t}}\n\treturn (*C.{c})(a_.hold(v.Handle_, v))\n}}\n\n"
		)
	}

	fn callback(&mut self, i: usize) {
		let abi = self.abi;
		let callback = &abi.api.callbacks[i];
		let c = abi.callback(i);
		let ty = Type::Callback(i, crate::ir::Dyn::Impl);
		let name = self.named(&ty).1;
		let rust = callback.path.join("::");
		let vtable = self.vtables[&c].clone();
		let p = &abi.prefix;
		let mut body = Vec::new();
		let mut members = Vec::new();
		let mut table = vec!["\tv.free = goRelease\n".to_string()];
		for method in &callback.methods {
			let member = pascal(method.name());
			crate::lock::function(&mut self.locked, method, &member);
			members.push(member.clone());
			let names = self.locals(&method.params);
			let params: Vec<String> = (method.params.iter().zip(&names))
				.map(|(p, name)| format!("{name} {}", self.go_param(&p.ty)))
				.collect();
			let ret = match method.ok {
				Type::Unit => String::new(),
				_ => format!(" {}", self.go(&method.ok)),
			};
			body.push(format!(
				"{}\t{member}({}){ret}",
				docs(&method.docs, "\t"),
				params.join(", ")
			));
			let export = format!("{p}Go_{}_{}", self.bare(&c), method.name());
			let ps: Vec<String> = (0..method.params.len()).map(|k| format!("p{k}")).collect();
			let args: Vec<String> = (method.params.iter().zip(&ps))
				.map(|(p, e)| self.lift_lent(&p.ty, e))
				.collect();
			let call = format!("given_(data).({name}).{member}({})", args.join(", "));
			let ret_ty = abi.ty(&method.ok);
			let c_params: Vec<(String, Ty)> = (method.params.iter().zip(&ps))
				.map(|(param, n)| (n.clone(), abi.ty(&param.ty)))
				.collect();
			let call = match method.ok {
				Type::Unit => format!("\t{call}\n"),
				_ => format!("\treturn {}\n", self.lower_scalar(&method.ok, &call)),
			};
			self.export(&export, &c_params, &ret_ty, &call);
			table.push(format!("\tv.{} = (*[0]byte)(C.{export})\n", cgo_field(method.name())));
		}
		self.distinct(&rust, &members);
		let text = format!(
			"{name} is `{rust}`, implemented in Go. Rust calls its methods from any goroutine's\nthread, and lets go of the implementation once it drops it, closing it if it is an\nio.Closer.\n\n{}",
			callback.docs
		);
		let _ = write!(
			self.types,
			"{}type {name} interface {{\n{}\n}}\n\n\
			var vtable_{c} = func() *C.{c}_vtable {{\n\tv := (*C.{c}_vtable)(C.malloc(C.size_t(unsafe.Sizeof(C.{c}_vtable{{}}))))\n{}\treturn v\n}}()\n\n\
			// {vtable} is the table of {name}'s methods Rust calls, for the generated packages.\n\
			func {vtable}() unsafe.Pointer {{\n\treturn unsafe.Pointer(vtable_{c})\n}}\n\n",
			docs(text.trim(), ""),
			body.join("\n"),
			table.join("")
		);
	}

	/// A callback's lower: a Go implementation, kept for Rust until it drops it.
	fn callback_conv(&mut self, i: usize) -> String {
		let c = self.abi.callback(i);
		let ty = Type::Callback(i, crate::ir::Dyn::Impl);
		let (krate, local) = self.named(&ty);
		let name = self.qualify(krate, &local);
		let vtable = self.qualify(krate, &self.vtables[&c]);
		format!(
			"func lower_{c}(v {name}) C.{c} {{\n\tif v == nil {{\n\t\tpanic(\"a nil {local}\")\n\t}}\n\treturn C.{c}{{vtable: (*C.{c}_vtable)({vtable}()), data: give_(v)}}\n}}\n\n"
		)
	}

	/// The C value of a scalar `e` of type `ty` a callback returns, needing no arena.
	fn lower_scalar(&mut self, ty: &Type, e: &str) -> String {
		match ty {
			Type::Bool | Type::Int(_) | Type::NonZero(_) | Type::F32 | Type::F64 | Type::Char => self.lower(ty, e),
			Type::Enum(i) if self.abi.api.enums[*i].plain() => self.lower(ty, e),
			other => unreachable!("a callback returns a scalar, not {other:?}"),
		}
	}

	/// A function Go exports to C as `name`, taking its data then `params`, returning
	/// `ret`, with `body`, and its declaration for the preamble, whose handles are not
	/// `const`, as cgo declares them.
	fn export(&mut self, name: &str, params: &[(String, Ty)], ret: &Ty, body: &str) {
		let unconst = |ty: &Ty| match ty {
			Ty::Handle { name, .. } => Ty::Handle {
				name: name.clone(),
				constant: false,
			},
			ty => ty.clone(),
		};
		let decls: Vec<String> = std::iter::once("void *data".to_string())
			.chain(params.iter().map(|(n, ty)| crate::c::decl(self.abi, &unconst(ty), n)))
			.collect();
		self.decls.push(format!(
			"extern {};",
			crate::c::decl(self.abi, &unconst(ret), &format!("{name}({})", decls.join(", ")))
		));
		let go_params: Vec<String> = std::iter::once("data unsafe.Pointer".to_string())
			.chain(params.iter().map(|(n, ty)| format!("{n} {}", self.c_go(ty))))
			.collect();
		let ret = match ret {
			Ty::Void => String::new(),
			ret => format!(" {}", self.c_go(ret)),
		};
		let _ = write!(
			self.exports,
			"//export {name}\nfunc {name}({}){ret} {{\n{body}}}\n\n",
			go_params.join(", ")
		);
	}

	/// The trampoline C calls for a closure of type `ty`, which Rust lends for a call or
	/// keeps: it lends each object it takes by reference as its type, ended once it returns,
	/// and hands Rust what the closure returns, writing a `Result` through its outs.
	fn trampoline(&mut self, ty: &Type, closure: &Closure) -> String {
		let go = self.go(ty);
		// Keyed by the Rust closure, since Go spells a borrowed and a copied object alike.
		if let Some(name) = self.closures.get(closure) {
			return name.clone();
		}
		let name = format!(
			"{}Go_{}_closure{}",
			self.abi.prefix,
			self.set.name(self.pkg),
			self.closures.len()
		);
		self.closures.insert(closure.clone(), name.clone());
		let lent = &closure.params;
		let ps: Vec<String> = (0..lent.len()).map(|k| format!("p{k}")).collect();
		let mut body = format!("\tf_ := lent_(data).({go})\n");
		let mut args = Vec::new();
		for (k, (ty, p)) in lent.iter().zip(&ps).enumerate() {
			match ty {
				Type::Ref(_, object) if matches!(**object, Type::Object(_)) => {
					let Type::Object(i) = **object else { unreachable!() };
					self.convert(object);
					let l = format!("l{k}_");
					let _ = writeln!(body, "\t{l} := lend_{}({p})", self.abi.object(i));
					let _ = writeln!(body, "\tdefer {l}.Handle_.End()");
					args.push(l);
				}
				ty => args.push(self.lift_lent(ty, p)),
			}
		}
		let call = format!("f_({})", args.join(", "));
		let mut params: Vec<(String, Ty)> = lent
			.iter()
			.zip(&ps)
			.map(|(ty, p)| (p.clone(), self.abi.ty(ty)))
			.collect();
		let ret_ty = match &closure.err {
			None => {
				match &closure.ok {
					Type::Unit => {
						let _ = writeln!(body, "\t{call}");
					}
					ok => {
						let _ = writeln!(body, "\treturn {}", self.returned(ok, &call));
					}
				}
				self.abi.ty(&closure.ok)
			}
			Some(err) => {
				let ok = match &closure.ok {
					Type::Unit => {
						let _ = writeln!(body, "\terr_ := {call}");
						None
					}
					ok => {
						let _ = writeln!(body, "\tr_, err_ := {call}");
						Some(ok)
					}
				};
				let _ = writeln!(
					body,
					"\tif err_ != nil {{\n\t\t*e_ = {}\n\t\treturn false\n\t}}",
					self.returned(err, "err_.Error()")
				);
				if let Some(ok) = ok {
					let _ = writeln!(body, "\t*o_ = {}", self.returned(ok, "r_"));
					params.push(("o_".to_string(), Ty::Out(Box::new(self.abi.ty(ok)))));
				}
				let _ = writeln!(body, "\treturn true");
				params.push(("e_".to_string(), Ty::Out(Box::new(self.abi.ty(err)))));
				Ty::Bool
			}
		};
		self.export(&name, &params, &ret_ty, &body);
		name
	}

	/// The C value Rust takes from the Go value `e` of type `ty` a closure returned, which
	/// Rust then owns: a string's copy, and a value as it lowers.
	fn returned(&mut self, ty: &Type, e: &str) -> String {
		match ty {
			Type::String => format!("cBufCopy({e})"),
			Type::Record(_) | Type::Option(_) => self.lower(ty, e),
			ty => self.lower_scalar(ty, e),
		}
	}

	/// The Go statement closing each object the value `e` of type `ty` holds, once a call
	/// moved them out.
	fn close(&self, ty: &Type, e: &str, depth: usize) -> String {
		match ty {
			Type::Option(inner) => match inner.base() {
				Type::Object(_) => format!("if {e} != nil {{ {e}.Close() }}"),
				_ => {
					let v = format!("v{depth}_");
					format!("if {e} != nil {{ {v} := *{e}; {} }}", self.close(inner, &v, depth + 1))
				}
			},
			Type::Box(inner) => self.close(inner, e, depth),
			Type::Tuple(elements) if elements.len() == 1 => self.close(&elements[0], e, depth),
			Type::Tuple(elements) => {
				let closes: Vec<String> = (elements.iter().enumerate())
					.filter(|(_, ty)| moved(self.abi, ty))
					.map(|(k, ty)| self.close(ty, &format!("{e}.V{k}"), depth))
					.collect();
				closes.join("; ")
			}
			Type::List(_, inner) => {
				let v = format!("v{depth}_");
				format!("for _, {v} := range {e} {{ {} }}", self.close(inner, &v, depth + 1))
			}
			_ => format!("{e}.Close()"),
		}
	}

	/// `function`'s Go form: a method of `owner` (whose IR type is `owner_ty`) if it has one
	/// and a receiver, or else a function of the package.
	fn function(&mut self, function: &Function, owner: Option<&Ty>, owner_ty: Option<&Type>) -> Method {
		let abi = self.abi;
		let export = abi.export(function, owner);
		let c_name = export.call.name.clone();
		let owner_c = match owner {
			Some(Ty::Struct(n) | Ty::Enum(n) | Ty::Handle { name: n, .. }) => Some(n.clone()),
			_ => None,
		};
		let own = match &owner_c {
			Some(o) => export.named.strip_prefix(o.as_str()).and_then(|r| r.strip_prefix('_')),
			None => None,
		};
		let receiver = function.receiver.filter(|_| owner.is_some());
		let type_name = owner_ty.map(|ty| self.named(ty).1);
		let member = match (own, receiver, &type_name) {
			(Some(own), Some(_), _) => pascal(own),
			(Some("new"), None, Some(t)) => format!("New{t}"),
			(Some(own), None, Some(t)) => match function.access {
				Access::Const => format!("{t}{}", own.to_uppercase()),
				_ => format!("{t}{}", pascal(own)),
			},
			_ => {
				let rest = rest(&abi.prefix, &export.named, &function.shown, &function.home);
				match function.access {
					Access::Const => rest.to_uppercase(),
					_ => pascal(rest),
				}
			}
		};
		let classes = self.classes.get(self.pkg);
		let member = match (receiver.is_some(), member.as_str()) {
			// What every object or value of the type already has.
			(true, "Close" | "Error") => format!("{member}_"),
			// A type's name, which Go's case cannot keep apart from a function's as C's does:
			// one that returns that type makes it, as `new` does, and any other is escaped.
			(false, name) if classes.is_some_and(|classes| classes.contains(name)) => {
				let made = match &function.ok {
					Type::Option(inner) => &**inner,
					ok => ok,
				};
				let made = match made {
					Type::Record(_) | Type::Enum(_) | Type::Object(_) => Some(self.named(made)),
					_ => None,
				};
				match made {
					Some((krate, made)) if krate == self.pkg && made == name => format!("New{member}"),
					_ => format!("{member}_"),
				}
			}
			_ => member,
		};

		crate::lock::function(&mut self.locked, function, &member);

		// A tuple of one returned whole is its element, written through its one out pointer.
		// Only a tuple returned whole is written element by element.
		let oks: Vec<Type> = match &function.ok {
			Type::Unit => Vec::new(),
			Type::Tuple(elements) => elements.iter().map(strip_refs).collect(),
			ok => vec![strip_refs(ok)],
		};
		let err = function.err.as_ref().map(strip_refs);
		let mut results: Vec<String> = oks.iter().map(|ty| self.go(ty)).collect();
		// A call that takes an object, as its receiver or held in any value it takes,
		// returns an error once that object is closed.
		let closable = receiver.is_some() && owner_ty.is_some_and(|ty| self.holds(ty))
			|| function.params.iter().any(|p| self.holds(&p.ty));
		if function.err.is_some() || function.future || closable {
			results.push("error".to_string());
		}
		// What a call returns before it is made: each result's zero, then the error.
		let mut zeros: Vec<String> = oks.iter().map(|ty| self.zero(ty)).collect();
		zeros.push(String::new());
		let fail = |error: &str| {
			let mut values = zeros.clone();
			*values.last_mut().expect("the error") = error.to_string();
			format!("return {}", values.join(", "))
		};

		let mut prelude: Vec<String> = Vec::new();
		let mut after: Vec<String> = Vec::new();
		let mut checked = false;
		let mut arena = false;
		let mut args: Vec<String> = Vec::new();
		let mut c_params = export.call.params.iter();
		let mut go_receiver = None;
		if let Some(receiver) = receiver {
			let (_, c) = c_params.next().expect("a method has its receiver");
			let ty = owner_ty.expect("a method has its type").clone();
			let go = self.go(&ty);
			// A value's `&mut self` is a pointer receiver, which Rust writes back through.
			let pointer = matches!(c, Ty::InOut(_));
			go_receiver = Some(match pointer {
				true => format!("self *{go}"),
				false => format!("self {go}"),
			});
			match c {
				Ty::Handle { name, .. } => {
					prelude.push(format!(
						"h_ := self.Handle_.Hold()\nif h_ == nil {{\n\t{}\n}}\ndefer unhold_(self.Handle_, self)",
						fail("self.Handle_.Closed()")
					));
					args.push(format!("(*C.{name})(h_)"));
					let copy = matches!(ty, Type::Object(i) if abi.api.objects[i].copy);
					if receiver == Receiver::Owned && !copy {
						after.push("self.Close()".to_string());
					}
				}
				_ => {
					let c = match abi.ty(&ty) {
						Ty::Struct(c) | Ty::Enum(c) => c,
						other => unreachable!("a value is a struct or enum, not {other:?}"),
					};
					self.convert(&ty);
					arena |= self.arena(&ty);
					let a = match self.arena(&ty) {
						true => "a_, ",
						false => "",
					};
					let deref = match pointer {
						true => "*",
						false => "",
					};
					prelude.push(format!("c_self_ := lower_{c}({a}{deref}self)"));
					args.push("&c_self_".to_string());
					checked |= self.holds(&ty);
				}
			}
		}
		// Past the receiver, which Rust only reads, Rust takes each buffer passed by value.
		let take_at = prelude.len();
		// Whether an argument is a parameter's own buffer, and the arguments converted from
		// Go values, which may panic, such as on a nil enum.
		let mut bufs = false;
		let mut lowered = Vec::new();
		let mut params = Vec::new();
		if function.future {
			params.push(("ctx".to_string(), "context.Context".to_string()));
		}
		let names = self.locals(&function.params);
		for (k, (param, name)) in function.params.iter().zip(&names).enumerate() {
			let (_, c) = c_params.next().expect("each parameter has its C one");
			if let Type::Closure(closure) = &param.ty {
				c_params.next().expect("a closure has its data");
				if closure.kept {
					c_params.next().expect("a kept closure has its free");
				}
			}
			let name = name.clone();
			// A value passed for `&mut`: the call reads it and writes back what it left.
			if param.ty.written() {
				let inner = param.ty.base();
				params.push((name.clone(), format!("*{}", self.go(inner))));
				arena |= self.arena(inner);
				lowered.push(args.len());
				args.push(self.lower(inner, &format!("(*{name})")));
				continue;
			}
			params.push((name.clone(), self.go_arg(&param.ty)));
			if moved(abi, &param.ty) {
				after.push(self.close(&param.ty, &name, 0));
			}
			match (&param.ty, c) {
				(Type::Closure(closure), _) => {
					let trampoline = self.trampoline(&param.ty, closure);
					args.push(format!("(*[0]byte)(C.{trampoline})"));
					match closure.kept {
						// Kept until Rust drops it, which releases its handle.
						true => {
							args.push(format!("give_({name})"));
							args.push("goRelease".to_string());
						}
						false => {
							let k_ = format!("k{k}_");
							prelude.push(format!("{k_} := cgo.NewHandle({name})\ndefer {k_}.Delete()"));
							args.push(format!("unsafe.Pointer(&{k_})"));
						}
					}
				}
				// Lent for the call through a handle the call deletes once it returns.
				(_, Ty::ForeignReader) => {
					let k_ = format!("k{k}_");
					let reader = self.qualify(Packages::RT, "Reader_");
					prelude.push(format!("{k_} := cgo.NewHandle({reader}({name}))\ndefer {k_}.Delete()"));
					args.push(format!("reader_(&{k_})"));
				}
				(ty @ Type::Callback(i, _), _) => {
					self.convert(ty);
					lowered.push(args.len());
					args.push(format!("lower_{}({name})", abi.callback(*i)))
				}
				(Type::Waker, _) => args.push(format!("waker_({name})")),
				// Its body runs under the caller's context, when the call takes one.
				(Type::Future, _) => {
					let ctx = match function.future {
						true => "ctx",
						false => "context.Background()",
					};
					args.push(format!("future_({ctx}, {name})"))
				}
				(ty, Ty::Buf) => {
					let call = match ty.buffer() {
						Some(Type::String | Type::PathBuf) => format!("bufString_({name})"),
						Some(Type::Bytes) => format!("bufTake_({name})"),
						Some(Type::Option(inner)) => match *inner {
							Type::Bytes => format!("bufTakeOption_({name})"),
							_ => format!("bufStringOption_({name})"),
						},
						other => unreachable!("a buffer holds {other:?}"),
					};
					bufs = true;
					args.push(call);
				}
				(ty, Ty::Handle { name: c, .. }) => {
					let h = format!("h{k}_");
					let failed = fail(&format!("{name}.Handle_.Closed()"));
					match ty.base() {
						Type::Option(_) => prelude.push(format!(
							"var {h} unsafe.Pointer\nif {name} != nil {{\n\tif {h} = {name}.Handle_.Hold(); {h} == nil {{\n\t\t{failed}\n\t}}\n\tdefer unhold_({name}.Handle_, {name})\n}}"
						)),
						_ => prelude.push(format!(
							"{h} := {name}.Handle_.Hold()\nif {h} == nil {{\n\t{failed}\n}}\ndefer unhold_({name}.Handle_, {name})"
						)),
					}
					args.push(format!("(*C.{c})({h})"));
				}
				(ty, _) if self.holds(ty) => {
					// Lowered before the call, which a closed object it holds fails.
					arena = true;
					let local = format!("c{k}_");
					prelude.push(format!("{local} := {}", self.lower(ty, &name)));
					args.push(local);
					checked = true;
				}
				(ty, _) => {
					arena |= self.arena(ty);
					lowered.push(args.len());
					args.push(self.lower(ty, &name));
				}
			}
		}
		let outs: Vec<(String, Ty)> = c_params.cloned().collect();
		let mut body = Vec::new();
		if arena {
			body.push("a_ := &arena_{}\ndefer a_.finish()".to_string());
			prelude.insert(take_at, "a_.take = true".to_string());
		}
		body.extend(prelude);
		if checked {
			body.push(format!("if a_.err != nil {{\n\t{}\n}}", fail("a_.err")));
		}
		// Converted first, as a conversion may panic: then the arena frees Rust's buffers it
		// lowered, and no parameter's own buffer is shared yet.
		if arena || bufs {
			for &i in &lowered {
				body.push(format!("l{i}_ := {}", args[i]));
				args[i] = format!("l{i}_");
			}
		}
		// Nothing from here fails before the call, which Rust's buffers given it go to.
		if arena {
			body.push("a_.made = true".to_string());
		}
		if !after.is_empty() {
			match &after[..] {
				[one] if one.ends_with("Close()") && !one.contains(' ') => body.push(format!("defer {one}")),
				_ => body.push(format!("defer func() {{\n\t{}\n}}()", after.join("\n\t"))),
			}
		}

		if let Some(abi::Future {
			handle,
			start,
			poll,
			free,
		}) = &export.future
		{
			let mut decls = Vec::new();
			for (k, ty) in oks.iter().enumerate() {
				let go = self.go(ty);
				decls.push(format!("var r{k}_ {go}"));
			}
			body.extend(decls);
			let zeros: String = (0..oks.len()).map(|k| format!("r{k}_, ")).collect();
			let poll_outs: Vec<(String, Ty)> = poll.params[2..].to_vec();
			let mut ready: Vec<String> = Vec::new();
			for (k, ty) in oks.iter().enumerate() {
				let out = out_name(&function.ok, k);
				ready.push(format!("r{k}_ = {}", self.lift(ty, &out)));
			}
			ready.push("return true, nil".to_string());
			let mut cases = vec![format!("case {}:\n\t{}", poll_ready(abi), ready.join("\n\t"))];
			if let Some(err) = &err {
				let lifted = self.lift_err(err, "c_err_");
				cases.push(format!("case {}:\n\treturn true, {lifted}", poll_error(abi)));
			}
			let cases = cases.join("\n");
			// Lifts what a call or a poll wrote, once either reports the future finished.
			let decls: Vec<String> = poll_outs.iter().map(|(n, ty)| self.out_decl(n, ty)).collect();
			let ptrs: Vec<String> = poll_outs.iter().map(|(n, _)| format!("&c_{n}_")).collect();
			let mut call_args = vec!["f_".to_string(), "w_".to_string()];
			call_args.extend(ptrs.iter().cloned());
			let mut polled = decls.clone();
			polled.push(format!(
				"switch C.{}({}) {{\n{cases}\n}}\nreturn false, nil",
				poll.name,
				call_args.join(", ")
			));
			let polled = indent(&polled.join("\n"));
			if function.eager {
				// An eager call runs Rust, and any callback it makes, before its future
				// exists, and a callback may cancel the context: so it is made, then the
				// context checked, then polled, never started.
				body.push(format!(
					"f_ := C.{c_name}({})\ndefer C.{}(f_)",
					args.join(", "),
					free.name
				));
				body.push(format!(
					"if err_ := ctx.Err(); err_ != nil {{\n\treturn {zeros}err_\n}}"
				));
				body.push(format!("poll_ := func(w_ cWaker) (bool, error) {{\n{polled}\n}}"));
				body.push("done_, err_ := poll_(cWaker{})\nif !done_ {\n\terr_ = await_(ctx, poll_)\n}".to_string());
			} else {
				// A context done already takes nothing, even what is ready: the future is made
				// and dropped unpolled, as Rust drops one it never awaits. Making an `async fn`'s
				// future runs nothing, so the check needs no repeat after it.
				body.push(format!(
					"if err_ := ctx.Err(); err_ != nil {{\n\tC.{}(C.{c_name}({}))\n\treturn {zeros}err_\n}}",
					free.name,
					args.join(", ")
				));
				let mut start_args = args.clone();
				start_args.extend(ptrs.iter().cloned());
				start_args.push("&f_".to_string());
				let mut inner = decls;
				inner.push(format!(
					"switch C.{}({}) {{\n{cases}\n}}\nreturn false, nil",
					start.name,
					start_args.join(", ")
				));
				body.push(format!("var f_ *C.{handle}"));
				body.push(format!(
					"done_, err_ := func() (bool, error) {{\n{}\n}}()",
					indent(&inner.join("\n"))
				));
				body.push(format!(
					"if !done_ {{\n\tdefer C.{}(f_)\n\terr_ = await_(ctx, func(w_ cWaker) (bool, error) {{\n{}\n\t}})\n}}",
					free.name,
					indent(&polled)
				));
			}
			let mut returns: Vec<String> = (0..oks.len()).map(|k| format!("r{k}_")).collect();
			returns.push("err_".to_string());
			body.push(format!("return {}", returns.join(", ")));
		} else {
			for (n, ty) in &outs {
				body.push(self.out_decl(n, ty));
			}
			let mut call_args = args.clone();
			call_args.extend(outs.iter().map(|(n, _)| format!("&c_{n}_")));
			let mut call = format!("C.{c_name}({})", call_args.join(", "));
			// Each value passed for `&mut` takes what Rust wrote back, on every path, as does a
			// value's own `&mut self`, which Rust wrote back in place.
			let mut writes: Vec<String> = match export.call.params.first() {
				Some((_, Ty::InOut(_))) => {
					let ty = owner_ty.expect("a method has its type");
					vec![format!("*self = {}", self.lift(ty, "c_self_"))]
				}
				_ => Vec::new(),
			};
			writes.extend(
				(function.params.iter().zip(&names))
					.filter(|(param, _)| param.ty.written())
					.map(|(param, name)| {
						let out = Abi::written(function, &abi::escape(&param.name));
						format!("*{name} = {}", self.lift(param.ty.base(), &format!("c_{out}_")))
					}),
			);
			if !writes.is_empty() {
				let made = match &err {
					Some(_) => {
						body.push(format!("ok_ := {call}"));
						"ok_".to_string()
					}
					// A value C returns, beside what Rust wrote back, is the out the result
					// lifts from.
					None if !matches!(export.call.ret, Ty::Void) => {
						body.push(format!("c_out_ := {call}"));
						String::new()
					}
					None => {
						body.push(call);
						String::new()
					}
				};
				body.extend(writes);
				call = made;
			}
			match (&err, oks.is_empty()) {
				(None, false) if call.is_empty() => {
					let mut values: Vec<String> = (oks.iter().enumerate())
						.map(|(k, ty)| {
							let out = out_name(&function.ok, k);
							self.lift(ty, &out)
						})
						.collect();
					if closable {
						values.push("nil".to_string());
					}
					body.push(format!("return {}", values.join(", ")));
				}
				(Some(err), _) => {
					let mut values: Vec<String> = Vec::new();
					for (k, ty) in oks.iter().enumerate() {
						let out = out_name(&function.ok, k);
						values.push(self.lift(ty, &out));
					}
					values.push("nil".to_string());
					body.push(format!("if {call} {{\n\treturn {}\n}}", values.join(", ")));
					let mut zeros: Vec<String> = oks.iter().map(|ty| self.zero(ty)).collect();
					zeros.push(self.lift_err(err, "c_err_"));
					body.push(format!("return {}", zeros.join(", ")));
				}
				(None, true) => {
					if !call.is_empty() {
						body.push(call);
					}
					if closable {
						body.push("return nil".to_string());
					}
				}
				(None, false) if !outs.is_empty() => {
					body.push(call);
					let mut values: Vec<String> = (oks.iter().enumerate())
						.map(|(k, ty)| {
							let out = out_name(&function.ok, k);
							self.lift(ty, &out)
						})
						.collect();
					if closable {
						values.push("nil".to_string());
					}
					body.push(format!("return {}", values.join(", ")));
				}
				(None, false) => {
					let value = self.lift(&oks[0], &call);
					match closable {
						true => body.push(format!("return {value}, nil")),
						false => body.push(format!("return {value}")),
					}
				}
			}
		}

		let mut text = function.docs.clone();
		for (param, name) in function.params.iter().zip(&names) {
			if let Type::Closure(closure) = &param.ty {
				text.push_str(&match closure.kept {
					true => format!("\n\nRust keeps {name} until it drops it, calling it from any thread."),
					false => format!("\n\nRust calls {name} only during the call, from any goroutine's thread."),
				});
				if closure.err.is_some() {
					text.push_str(&format!("\nAn error {name} returns crosses as its message."));
				}
				if (closure.params.iter())
					.any(|p| matches!(p, Type::Ref(_, object) if matches!(**object, Type::Object(_))))
				{
					text.push_str(&format!(
						"\nAn object it lends lives only for that call of {name}: keep none past it."
					));
				}
			}
		}
		for (param, name) in function.params.iter().zip(&names) {
			if matches!(&param.ty, Type::Ref(_, reader) if **reader == Type::Reader) {
				text.push_str(&format!(
					"\n\nRust reads and seeks {name} only during the call, from any thread, one call at a time:\na read returning bytes and an error returns the error on the next read, and its\nerrors cross as their messages."
				));
			}
		}
		if function.future {
			text.push_str(
				"\n\nIt blocks until Rust's future is ready, or ctx is done: then it cancels the future in\nRust, losing nothing, and returns ctx's error.",
			);
		}
		if let Some(check) = self.on(function.on, &c_name) {
			body.insert(0, check);
		}
		Method {
			key: Some(crate::lock::function_key(function)),
			docs: text,
			receiver: go_receiver,
			name: member,
			params,
			results,
			body: body.join("\n"),
		}
	}

	/// The declaration of the local the call writes the out value `name` of C type `ty` to.
	fn out_decl(&self, name: &str, ty: &Ty) -> String {
		let Ty::Out(inner) = ty else {
			unreachable!("an out parameter is a pointer")
		};
		format!("var c_{name}_ {}", self.c_go(inner))
	}

	/// The Go error of the C error `e` of type `err`: itself if an exported type, which then
	/// implements `error`, or else in a struct of its own.
	fn lift_err(&mut self, err: &Type, e: &str) -> String {
		let lifted = self.lift(err, e);
		match err.base() {
			Type::Record(_) | Type::Enum(_) | Type::Object(_) => lifted,
			_ => {
				let key = self.abi.key(err);
				let (name, (home, _)) = (self.thrown.iter())
					.find(|(_, (_, thrown))| self.abi.key(thrown) == key)
					.expect("each error is thrown");
				let name = self.qualify(home, name);
				format!("{name}{{Value: {lifted}}}")
			}
		}
	}
}

/// The C name of the out value holding the `k`th of what `declared` returns.
fn out_name(declared: &Type, k: usize) -> String {
	match declared {
		Type::Tuple(_) => format!("c_out{k}_"),
		_ => "c_out_".to_string(),
	}
}

fn poll_ready(abi: &Abi<'_>) -> String {
	format!("C.{}_POLL_READY", abi.prefix.to_uppercase())
}

fn poll_error(abi: &Abi<'_>) -> String {
	format!("C.{}_POLL_ERROR", abi.prefix.to_uppercase())
}

/// Whether a list of `elem` crosses as one block, whose Go type is laid out as its C type.
fn block(elem: &Type) -> bool {
	Type::List(Collection::Vec, Box::new(elem.clone())).block().is_some()
}

/// Whether passing `ty` moves an object's value out: by value, boxed, or in an `Option`,
/// a list, or a tuple, unless `Copy`, and never through a reference.
fn moved(abi: &Abi<'_>, ty: &Type) -> bool {
	match ty {
		Type::Object(i) => !abi.api.objects[*i].copy,
		Type::Option(inner) | Type::Box(inner) | Type::List(_, inner) => moved(abi, inner),
		Type::Tuple(elements) => elements.iter().any(|ty| moved(abi, ty)),
		_ => false,
	}
}

/// `ty` returned: a borrow is returned as a clone, so owned.
fn strip_refs(ty: &Type) -> Type {
	match ty {
		Type::Ref(_, inner) => strip_refs(inner),
		Type::Option(inner) => Type::Option(Box::new(strip_refs(inner))),
		Type::Box(inner) => Type::Box(Box::new(strip_refs(inner))),
		Type::List(c, inner) => Type::List(*c, Box::new(strip_refs(inner))),
		Type::Tuple(elements) => Type::Tuple(elements.iter().map(strip_refs).collect()),
		ty => ty.clone(),
	}
}

/// The Go type of an integer.
fn go_int(int: Int) -> &'static str {
	match int {
		Int::U8 => "uint8",
		Int::U16 => "uint16",
		Int::U32 => "uint32",
		Int::U64 => "uint64",
		Int::Usize => "uint",
		Int::I8 => "int8",
		Int::I16 => "int16",
		Int::I32 => "int32",
		Int::I64 => "int64",
		Int::Isize => "int",
	}
}

/// The C type of an integer, as cgo names it.
fn c_int(int: Int) -> &'static str {
	match int {
		Int::U8 => "uint8_t",
		Int::U16 => "uint16_t",
		Int::U32 => "uint32_t",
		Int::U64 => "uint64_t",
		Int::Usize => "size_t",
		Int::I8 => "int8_t",
		Int::I16 => "int16_t",
		Int::I32 => "int32_t",
		Int::I64 => "int64_t",
		Int::Isize => "ptrdiff_t",
	}
}

/// A field's Go name: in Pascal case, and `V0`, `V1`, ... for a positional one.
fn field_name(name: &str) -> String {
	match name.starts_with(|c: char| c.is_ascii_digit()) {
		true => format!("V{name}"),
		false => pascal(name),
	}
}

/// A parameter's Go name: in camel case, with `_` appended if Go reads it as a keyword or
/// it would hide a predeclared name or a package the call uses.
fn local(name: &str) -> String {
	const RESERVED: &[&str] = &[
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
		"any",
		"append",
		"bool",
		"byte",
		"cap",
		"clear",
		"close",
		"complex",
		"copy",
		"delete",
		"error",
		"false",
		"float32",
		"float64",
		"imag",
		"int",
		"int8",
		"int16",
		"int32",
		"int64",
		"iota",
		"len",
		"make",
		"max",
		"min",
		"new",
		"nil",
		"panic",
		"print",
		"println",
		"real",
		"recover",
		"rune",
		"string",
		"true",
		"uint",
		"uint8",
		"uint16",
		"uint32",
		"uint64",
		"uintptr",
		"context",
		"errors",
		"io",
		"ctx",
		"cgo",
		"math",
		"net",
		"netip",
		"runtime",
		"strconv",
		"sync",
		"atomic",
		"big",
		"time",
		"unsafe",
		"self",
		"other",
	];
	let pascal = pascal(name);
	let mut chars = pascal.chars();
	let camel: String = match chars.next() {
		Some(first) => first.to_lowercase().chain(chars).collect(),
		None => name.to_string(),
	};
	match RESERVED.contains(&camel.as_str()) {
		true => format!("{camel}_"),
		false => camel,
	}
}

/// Go's keywords.
const KEYWORDS: &[&str] = &[
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
];

/// A C field as cgo names it: one Go reads as a keyword takes a leading `_`.
fn cgo_field(name: &str) -> String {
	match KEYWORDS.contains(&name) {
		true => format!("_{name}"),
		false => name.to_string(),
	}
}

/// Go doc comments holding `docs`, each line indented by `indent`, or nothing for none.
fn docs(docs: &str, indent: &str) -> String {
	docs.lines()
		.map(|line| match line.is_empty() {
			true => format!("{indent}//\n"),
			false => format!("{indent}// {line}\n"),
		})
		.collect()
}

/// Code indented by one tab more, but for blank lines.
fn indent(code: &str) -> String {
	let mut out: String = code
		.lines()
		.map(|l| match l.is_empty() {
			true => "\n".to_string(),
			false => format!("\t{l}\n"),
		})
		.collect();
	if !code.ends_with('\n') {
		out.pop();
	}
	out
}

#[test]
fn package_names_are_locked() {
	let mut api = crate::ir::Api {
		records: vec![],
		enums: vec![],
		objects: vec![],
		callbacks: vec![],
		functions: vec![],
		constants: vec![],
		crates: vec!["lock_foobar".into()],
		targets: vec!["host".into()],
		reached: Vec::new(),
	};
	let base = api.clone();
	let old = generate(&Abi::new(&api, "lock"), "lock", 0, "lock").unwrap();
	assert_eq!(old.names["crate lock_foobar"], "foobar");
	api.crates.insert(0, "lock_foo_bar".into());
	let new = generate(&Abi::new(&api, "lock"), "lock", 0, "lock").unwrap();
	assert_eq!(new.names["crate lock_foobar"], "foobar_");
	assert!(new.files.iter().any(|(path, _)| path == "foobar_/foobar_.go"));
	let breaks = crate::lock::name_breaks(
		&BTreeMap::from([("Go".into(), old.names)]),
		&BTreeMap::from([("Go".into(), new.names)]),
		&base,
		&Abi::new(&api, "lock"),
	);
	assert_eq!(breaks.len(), 1);
	assert!(breaks[0].what.contains("its Go name `foobar` is now `foobar_`"));
}
