//! What one rustc invocation tells the CLI about a local crate, and what the CLI tells
//! the next invocations to narrow: files both sides read and write as JSON.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The environment the CLI hands the wrapper its job in.
pub mod env {
	/// The directory each compiled local crate's [`Unit`] goes into.
	pub const FACTS: &str = "RUST2_PUB_FACTS";
	/// A JSON list of the local packages' directories, whose crates are collected.
	pub const LOCAL: &str = "RUST2_PUB_LOCAL";
	/// An [`super::Edits`] file: the `pub` tokens rustc reads as `pub(crate)`.
	pub const EDITS: &str = "RUST2_PUB_EDITS";
}

/// A definition's identity in one compilation: rustc's `DefPathHash`, which a crate
/// that depends on it computes the same.
pub type Hash = [u64; 2];

/// One compiled local crate: its definitions and what each refers to.
#[derive(Serialize, Deserialize)]
pub struct Unit {
	pub crate_name: String,
	/// The compilation's `StableCrateId`, the crate half of each of its own hashes.
	pub crate_id: u64,
	/// The package's directory, from `CARGO_MANIFEST_DIR`.
	pub package: PathBuf,
	/// Whether it is a test harness, whose roots are tests.
	pub test: bool,
	/// Whether another crate can depend on it.
	pub lib: bool,
	/// Every file a span here names, absolute; a span's file is its index.
	pub files: Vec<PathBuf>,
	pub defs: Vec<Def>,
	/// Each definition another refers to, once per pair, with the first site.
	pub refs: Vec<Ref>,
	/// What each import supplies.
	pub imports: Vec<Import>,
	/// Every resolved name in the source, in order, which a narrowed build must resolve
	/// alike.
	pub sites: Vec<Site>,
	/// What runs without anything calling it: `main`, tests, exported symbols.
	pub roots: Vec<Hash>,
	/// Trait impls, each live once all it requires is.
	pub impls: Vec<Impl>,
	/// Every item as rustc prints it once macros have expanded, which a narrowed build
	/// must print alike.
	pub printed: Vec<Printed>,
	/// The macro expansions the items came from, by index.
	pub expansions: Vec<Expansion>,
	/// The files read as bytes, absolute, as `include_bytes!` and `include_str!` read.
	pub embedded: Vec<PathBuf>,
}

/// A source range: the file's index in [`Unit::files`], and its bytes.
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Span {
	pub file: u32,
	pub lo: u32,
	pub hi: u32,
}

/// A definition.
#[derive(Serialize, Deserialize)]
pub struct Def {
	pub hash: Hash,
	/// What it is, as rustc's `DefKind` prints.
	pub kind: String,
	/// Its path, as rustc prints it.
	pub path: String,
	/// Its header, the span rustc's diagnostics point at.
	pub span: Option<Span>,
	/// The `pub` that makes it public, if written in source.
	pub token: Option<Span>,
	/// The expansion that made it, if a macro did.
	pub expansion: Option<u32>,
	/// Whether another crate can name it: public all the way from its crate's root.
	pub exported: bool,
	/// Whether it is a symbol code outside Rust links to, which rustc keeps whatever its
	/// visibility.
	pub symbol: bool,
	/// Its enclosing definition, live whenever it is.
	pub parent: Option<Hash>,
	/// The type an inherent impl's item belongs to, whose API it is part of.
	pub member_of: Option<Hash>,
}

/// A definition `from` refers to `to`, first at `site`.
#[derive(Serialize, Deserialize)]
pub struct Ref {
	pub from: Hash,
	pub to: Hash,
	pub site: Option<Span>,
	/// Whether `from`'s interface names it, rather than only its body: its signature,
	/// its type, or what it imports.
	pub interface: bool,
}

/// A definition an import supplies, which the import is live once it is.
#[derive(Serialize, Deserialize)]
pub struct Import {
	pub import: Hash,
	pub supplied: Hash,
	/// The re-exports a glob supplied it through, which it names as it names `supplied`.
	pub through: Vec<Hash>,
}

/// A name the source resolves: where, and to what.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Site {
	pub span: Span,
	pub to: Hash,
}

/// A trait impl, live once its trait and self type, where those are local, are.
#[derive(Serialize, Deserialize)]
pub struct Impl {
	pub hash: Hash,
	pub requires: Vec<Hash>,
}

/// An item, an impl's or trait's own items apart, as rustc's HIR printer prints it, which
/// leaves out visibility: narrowing changes it only through a macro that read a token.
#[derive(Serialize, Deserialize)]
pub struct Printed {
	pub hash: Hash,
	/// A hash of the print, the attributes, and which binding or loop each name in it
	/// picks, bodies included.
	pub output: u64,
	/// The expansions any of it came from, by index in [`Unit::expansions`].
	pub expansions: Vec<u32>,
	/// Its lines, from the start of its first to its end: `column!` and `line!` leave no
	/// expansion behind, but read only where they are.
	pub lines: Option<Span>,
}

/// A macro call, and what its output may have read.
#[derive(Serialize, Deserialize)]
pub struct Expansion {
	/// The expansion the call came from, if not the source.
	pub parent: Option<u32>,
	/// Its call's line, up to the call's end: a bang macro's input, and the columns a
	/// narrowed `pub` before it shifts, which `column!` and a proc macro may read.
	pub call: Option<Span>,
	/// The expansion whose items carry the item an attribute or derive was given: an
	/// attribute's own, and a derive's `#[derive]`.
	pub input: Option<u32>,
}

/// The `pub` tokens to read as `pub(crate)`: each file, absolute, and the byte offsets of
/// its tokens.
pub type Edits = BTreeMap<PathBuf, Vec<u32>>;
