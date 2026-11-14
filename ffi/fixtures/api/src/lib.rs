//! Each construct rust2-ffi exports, for `tests/api.c` to drive through the C ABI.

use std::borrow::Cow;
use std::fmt;
use std::num::NonZero;
use std::ops::{Bound, Range, RangeBounds};
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use bytes::Bytes;
// Another crate's items, which the export follows this glob of its root to.
pub use dep::*;

/// The largest value a [`Counter`] reaches.
pub const LIMIT: u64 = 1000;

/// A point: a record, since every field is `pub`. Its docs reach the header, where a
/// comment such as `/* this */` must not close the header's own.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Point {
	pub x: i32,
	pub y: i32,
}

impl Point {
	/// The point moved by `dx` and `dy`.
	pub fn offset(&self, dx: i32, dy: i32) -> Point {
		Point {
			x: self.x + dx,
			y: self.y + dy,
		}
	}
}

/// A record that owns memory: its `free` releases the name.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
	pub name: String,
	pub at: Option<Point>,
	pub ttl: Option<Duration>,
}

/// A note whose text may be present but empty, which crosses as itself, not as none.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Note {
	pub text: Option<String>,
}

/// Whether `note` has text, even empty.
pub fn has_text(note: Note) -> bool {
	note.text.is_some()
}

/// A reading: a field of each scalar kind at the edge of its range, a nested record, and
/// text, so a binding that passes a record's scalars apart from its other fields passes each
/// exactly.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
	pub ok: bool,
	pub level: f32,
	pub ratio: f64,
	pub mark: char,
	pub shape: Shape,
	pub at: Point,
	pub note: String,
	pub count: u64,
	pub offset: i8,
	pub len: usize,
	pub tag: u16,
}

impl Reading {
	/// The reading after this one, by its `tag`.
	pub fn next(&self) -> Option<Reading> {
		reading(u32::from(u16::MAX - self.tag) + 1)
	}
}

/// The reading `n`, or none for 0.
pub fn reading(n: u32) -> Option<Reading> {
	(n != 0).then(|| Reading {
		ok: n % 2 == 1,
		level: -(n as f32) / 2.0,
		ratio: f64::MAX / f64::from(n),
		mark: char::from_u32(0x10FFFF - n).unwrap_or(char::REPLACEMENT_CHARACTER),
		shape: match n % 2 {
			1 => Shape::Circle,
			_ => Shape::Square,
		},
		at: Point {
			x: i32::MIN + n as i32,
			y: -(n as i32),
		},
		note: format!("reading {n}"),
		count: u64::MAX - u64::from(n),
		offset: i8::MIN.wrapping_add(n as i8),
		len: usize::MAX - n as usize,
		tag: u16::MAX - n as u16,
	})
}

/// The `Debug` form of `reading(n)`, or empty for none, which a binding compares its own
/// copy's with.
pub fn reading_debug(n: u32) -> String {
	reading(n).map_or_else(String::new, |r| format!("{r:?}"))
}

/// `reading`, passed through Rust and back.
pub fn reading_echo(reading: Reading) -> Reading {
	reading
}

/// Declares `crowded`, whose parameters take 243 of a JVM method's 255 slots, so flattening
/// its `Reading` would take 15 more: a binding that counts a `Stamp` as one slot, not the
/// two of the `long` it crosses as, flattens it past the limit.
macro_rules! crowded {
	($($n:ident)*) => {
		/// The sum of the reading's count, both stamps' millis, and every number.
		pub fn crowded(reading: Reading, a: Stamp, b: Stamp, $($n: u64),*) -> u64 {
			reading.count.wrapping_add(a.millis).wrapping_add(b.millis)$(.wrapping_add($n))*
		}
	};
}

crowded!(n0 n1 n2 n3 n4 n5 n6 n7 n8 n9 n10 n11 n12 n13 n14 n15 n16 n17 n18 n19 n20 n21 n22 n23 n24 n25 n26 n27 n28 n29 n30 n31 n32 n33 n34 n35 n36 n37 n38 n39 n40 n41 n42 n43 n44 n45 n46 n47 n48 n49 n50 n51 n52 n53 n54 n55 n56 n57 n58 n59 n60 n61 n62 n63 n64 n65 n66 n67 n68 n69 n70 n71 n72 n73 n74 n75 n76 n77 n78 n79 n80 n81 n82 n83 n84 n85 n86 n87 n88 n89 n90 n91 n92 n93 n94 n95 n96 n97 n98 n99 n100 n101 n102 n103 n104 n105 n106 n107 n108 n109 n110 n111 n112 n113 n114 n115 n116 n117 n118);

/// A plain enumeration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
	Circle = 1,
	Square = 4,
}

/// An enum with data.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
	Moved(Point),
	Named { name: String },
	Done,
}

/// An enum whose variants are named as types its Kotlin sealed class names: builtins,
/// the runtime's, and exported types, each of which a nested variant class would shadow.
#[derive(Clone, Debug, PartialEq)]
pub enum Clash {
	Point(Point),
	Shape(Shape),
	Event(Event),
	Double(f64),
	Float(f32),
	String(String),
	Int(i32),
	Boolean(bool),
	Native(u32),
	Bytes(Vec<u8>),
	Any,
	Unit,
	Waker,
	Mut,
	Pair(u8, u8),
	Stamp(i64),
	Moment(Moment),
}

impl Clash {
	/// The clash's point, if it has one.
	pub fn point(&self) -> Option<Point> {
		match self {
			Clash::Point(p) => Some(*p),
			_ => None,
		}
	}

	/// A double variant of `value`.
	pub fn of(value: f64) -> Clash {
		Clash::Double(value)
	}

	/// Its bytes, its text's or its own.
	pub fn raw(&self) -> Vec<u8> {
		match self {
			Clash::Bytes(b) => b.clone(),
			Clash::String(s) => s.as_bytes().to_vec(),
			_ => Vec::new(),
		}
	}

	/// The reading of `kotlin`, whose float fields the sealed class reads back from their bits
	/// under a parameter named as the package of Kotlin's builtins.
	pub fn reading(&self, kotlin: u32) -> Reading {
		reading(kotlin).unwrap()
	}

	/// `java` with `kotlin` appended, and its old length through `kotlin`, under parameters
	/// named as the packages that qualify Kotlin's builtins and the JDK.
	pub fn listed(mut java: Vec<u32>, kotlin: &mut u32) -> Vec<Vec<u32>> {
		java.push(*kotlin);
		*kotlin = java.len() as u32 - 1;
		vec![java]
	}

	/// The double clash of `kotlin`, once `first` finishes in the caller's context, which the
	/// sealed class's body reaches under a parameter named as the package of Kotlin's builtins.
	pub async fn after(first: impl Future<Output = ()> + Send, kotlin: f64) -> Clash {
		first.await;
		Clash::Double(kotlin)
	}

	/// `lent` then `owned`, each of the byte parameters a sealed class's body names.
	pub fn joined(&self, lent: &[u8], owned: Vec<u8>, maybe: Option<Vec<u8>>) -> Vec<u8> {
		[lent, &owned, &maybe.unwrap_or_default()].concat()
	}

	/// `tag` with this clash's form, from a future that borrows both.
	#[expect(clippy::manual_async_fn, reason = "the non-`async` form a borrowing future takes")]
	pub fn tagged<'a>(&'a self, tag: &'a str) -> impl Future<Output = String> + 'a {
		async move { format!("{tag} {self:?}") }
	}

	/// A stamp `kotlin` milliseconds in, a value only Rust makes in a word, which the sealed
	/// class's body builds from its `Long` beside a `Stamp` variant that also takes one.
	pub fn stamp(&self, kotlin: u64) -> Stamp {
		Stamp::from_millis(kotlin)
	}

	/// Its moment, or `kotlin` ticks at a millisecond's rate: a value only Rust makes, past a
	/// word, which the sealed class's body builds beside its `Moment` variant.
	pub fn moment(&self, kotlin: u64) -> Moment {
		match self {
			Clash::Moment(m) => *m,
			_ => Moment::new(kotlin, 1000).unwrap(),
		}
	}

	/// A point clash at `api`, a parameter named as the Kotlin package that qualifies its type.
	pub fn at(api: Point) -> Clash {
		Clash::Point(api)
	}

	/// `text` with this clash's form and `n`, and `n` doubled through `out`.
	pub fn describe(&self, text: &str, n: u32, shape: Shape, out: &mut u32) -> String {
		*out = n * 2;
		format!("{text} {self:?} {n} {shape:?}")
	}
}

/// A record whose field is named as the Kotlin package, which a qualifier in its methods'
/// bodies would resolve to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Packaged {
	pub api: u32,
}

impl Packaged {
	/// The point at `api` on both axes.
	pub fn point(&self) -> Point {
		Point {
			x: self.api as i32,
			y: self.api as i32,
		}
	}

	/// The shape of `api`'s parity.
	pub fn shape(&self) -> Shape {
		match self.api % 2 {
			0 => Shape::Square,
			_ => Shape::Circle,
		}
	}
}

/// An enum whose variant `Init` names an initializer in Swift, and the C union member
/// holding its data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stage {
	Init(u32),
	Done,
}

/// The stage after `stage`: `Init` counts down to `Done`.
pub fn advance(stage: Stage) -> Stage {
	match stage {
		Stage::Init(0) | Stage::Done => Stage::Done,
		Stage::Init(n) => Stage::Init(n - 1),
	}
}

/// The error a [`Counter`] returns.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
	Underflow { by: u64 },
	Overflow,
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Error::Underflow { by } => write!(f, "underflow by {by}"),
			Error::Overflow => write!(f, "overflow"),
		}
	}
}

/// A counter: an object, since its field is private. Clones share nothing.
#[derive(Clone, Debug, Default)]
pub struct Counter {
	value: u64,
	name: String,
}

impl Counter {
	pub fn new(name: &str, start: u64) -> Self {
		Self {
			value: start,
			name: name.to_string(),
		}
	}

	pub fn get(&self) -> u64 {
		self.value
	}

	/// The value plus ten `handle`s and `h`: parameters named as a binding's receiver and
	/// its held handle might be.
	pub fn offset(&self, handle: u64, h: u64) -> u64 {
		self.value + handle * 10 + h
	}

	/// The value plus `close`, consuming the counter: a parameter named as the method a
	/// binding closes it with might be.
	pub fn into_sum(self, close: u64) -> u64 {
		self.value + close
	}

	pub fn name(&self) -> &str {
		&self.name
	}

	pub fn add(&mut self, n: u64) -> Result<u64, Error> {
		match self.value.checked_add(n) {
			Some(v) if v <= LIMIT => {
				self.value = v;
				Ok(v)
			}
			_ => Err(Error::Overflow),
		}
	}

	pub fn sub(&mut self, n: u64) -> Result<u64, Error> {
		match self.value.checked_sub(n) {
			Some(v) => {
				self.value = v;
				Ok(v)
			}
			None => Err(Error::Underflow { by: n - self.value }),
		}
	}

	/// Whether this is the counter [`Registry::edit`] lends, by its address, which a handle
	/// lent for the call keeps.
	pub fn edited(&self) -> bool {
		std::ptr::from_ref(self) as usize == EDITING.load(std::sync::atomic::Ordering::Relaxed)
	}

	/// This count plus `other`'s, if any: an optional handle, lent shared.
	pub fn plus(&self, other: Option<&Counter>) -> u64 {
		self.value + other.map_or(0, |other| other.value)
	}

	/// This count, raised to `low`'s and lowered to `high`'s, each if any: two optional
	/// handles, lent shared.
	pub fn clamped(&self, low: Option<&Counter>, high: Option<&Counter>) -> u64 {
		let low = low.map_or(self.value, |low| self.value.max(low.value));
		high.map_or(low, |high| low.min(high.value))
	}

	/// [`Self::plus`], once [`yielded`] finishes: a future borrowing both handles meanwhile.
	pub async fn plus_later(&self, other: Option<&Counter>) -> u64 {
		yielded(0).await;
		self.plus(other)
	}

	/// Adds this count to `other`, if any: an optional handle, lent exclusively.
	pub fn add_to(&self, other: Option<&mut Counter>) -> bool {
		other.map(|other| other.value += self.value).is_some()
	}

	/// Consumes the counter.
	pub fn into_label(self, at: Option<Point>) -> Label {
		Label {
			name: self.name,
			at,
			ttl: None,
		}
	}

	pub fn rename(&mut self, name: impl Into<String>) {
		self.name = name.into();
	}

	/// Consumes the counter into two, the second holding what is above `at`: a tuple
	/// returns through one out pointer per element.
	pub fn split(self, at: u64) -> (Counter, Counter) {
		let above = self.value.saturating_sub(at);
		let below = Counter::new(&self.name, self.value - above);
		(below, Counter::new(&format!("{}+", self.name), above))
	}
}

/// An impl lifetime its type does not take is the call's, as a method's own is.
impl<'a> Counter {
	/// Whether the counter is named `name`, which borrows C's text for the call.
	pub fn is(&self, name: &'a str) -> bool {
		self.name == name
	}
}

/// A tuple of one: its out pointer holds the counter, not the tuple.
pub fn only(counter: Counter) -> (Counter,) {
	(counter,)
}

/// `n` narrowed to each width, a tuple longer than any a language may name.
pub fn widths(n: u64) -> (u8, i16, u32, u64) {
	(n as u8, n as i16, n as u32, n)
}

/// The quotient and remainder, failing on a zero divisor.
pub fn divide(a: u64, b: u64) -> Result<(u64, u64), Error> {
	match b {
		0 => Err(Error::Overflow),
		b => Ok((a / b, a % b)),
	}
}

/// The unit corner, borrowed for good, failing unless `unit`: a borrowed tuple returns as
/// one value, not an out per element.
pub fn corner(unit: bool) -> Result<&'static (u32, u32), Error> {
	match unit {
		true => Ok(&(1, 1)),
		false => Err(Error::Overflow),
	}
}

/// [`corner`]'s, from a future.
pub async fn corner_later() -> &'static (u32, u32) {
	&(1, 1)
}

/// `n` narrowed as [`widths`] does, unless 0: a tuple inside an `Option` is a struct.
pub fn spread(n: u64) -> Option<(u8, i16, u32, u64)> {
	(n != 0).then(|| widths(n))
}

/// Where `needle` first starts in `haystack`, and its length, as moq-mux's
/// `find_start_code` returns it.
pub fn find(haystack: &[u8], needle: &[u8]) -> Option<(usize, usize)> {
	(haystack.windows(needle.len()))
		.position(|window| window == needle)
		.map(|at| (at, needle.len()))
}

/// Each point named by its index, with its shape: rows of tuples, as moq-net's stats
/// snapshots return them.
pub fn rows(points: &[Point]) -> Vec<(String, Shape, Point)> {
	(points.iter().enumerate())
		.map(|(i, p)| {
			let shape = match p.x == p.y {
				true => Shape::Square,
				false => Shape::Circle,
			};
			(format!("p{i}"), shape, *p)
		})
		.collect()
}

/// Counters by name, which takes each counter's handle in a tuple.
pub struct Board {
	counters: std::collections::BTreeMap<String, Counter>,
}

impl Board {
	pub fn new(counters: Vec<(String, Counter)>) -> Self {
		Self {
			counters: counters.into_iter().collect(),
		}
	}

	/// Each name and its counter in name order, each handle moved out of the board.
	pub fn into_counters(self) -> Vec<(String, Counter)> {
		self.counters.into_iter().collect()
	}

	/// Each name and its counter, highest first, as hang's `ranked`: each pair borrowed,
	/// so each crosses as a clone.
	pub fn ranked(&self) -> impl Iterator<Item = (&String, &Counter)> {
		let mut ranked: Vec<_> = self.counters.iter().collect();
		ranked.sort_by_key(|(_, counter)| std::cmp::Reverse(counter.get()));
		ranked.into_iter()
	}

	/// Its highest counter, through an `impl Trait` that captures no borrow of the board, so
	/// it crosses as the counter's handle.
	pub fn best(&self) -> impl Into<Counter> + use<> {
		(self.counters.values().max_by_key(|counter| counter.get()).cloned()).unwrap_or_default()
	}
}

/// Points by name and the ids in use, as hang's renditions and moq-mux's selections: a map
/// and a set, which cross as their items in order, each key once.
#[derive(Clone, Debug, PartialEq)]
pub struct Atlas {
	pub points: std::collections::BTreeMap<String, Point>,
	pub ids: std::collections::BTreeSet<u32>,
}

impl Atlas {
	/// It with `at` named `name`, replacing any point of that name.
	pub fn with(mut self, name: &str, at: Point) -> Self {
		self.points.insert(name.to_string(), at);
		self
	}
}

/// `text` as a port: another crate's error crosses as its message.
pub fn port(text: &str) -> Result<u16, std::num::ParseIntError> {
	text.parse()
}

/// A span: a record whose fields are tuples, one holding text.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
	pub bounds: (u32, u32),
	pub named: Option<(String, Shape)>,
}

/// How far a span from `start` to `end` reaches: a function named as a type of its crate
/// that does not return it.
pub fn span(start: u32, end: u32) -> u32 {
	end - start
}

/// The span's two halves, each a tuple inside the tuple returned.
pub fn halves(span: &Span) -> ((u32, u32), (u32, u32)) {
	let (start, end) = span.bounds;
	let mid = start + (end - start) / 2;
	((start, mid), (mid, end))
}

/// `value` doubled: a tuple of one inside an `Option` keeps `None` apart from
/// `Some((None,))`.
pub fn doubled(value: Option<(Option<u32>,)>) -> Option<(Option<u32>,)> {
	value.map(|(n,)| (n.map(|n| n * 2),))
}

/// A title, for formal speech: a tuple of one borrowed string, whose C struct one of an
/// owned string shares.
pub fn title(formal: bool) -> Option<(&'static str,)> {
	formal.then_some(("Dr",))
}

/// `name`, unless empty: a tuple of one owned string.
pub fn named(name: &str) -> Option<(String,)> {
	(!name.is_empty()).then(|| (name.to_string(),))
}

/// `value` lent back: a borrowed tuple of one, returned as a clone, is a tuple, not its
/// element, as only a tuple returned whole is.
pub fn lent_back(value: &(Vec<u8>,)) -> &(Vec<u8>,) {
	value
}

/// `bytes` in a boxed tuple of one, which is a tuple too.
pub fn boxed_tuple(bytes: Vec<u8>) -> Box<(Vec<u8>,)> {
	Box::new((bytes,))
}

/// `pair` with its sides swapped: a tuple taken by value.
pub fn swapped(pair: (String, u32)) -> (u32, String) {
	(pair.1, pair.0)
}

/// A meter that is deliberately not `Clone`, as moq's group writers are, so a borrowed
/// return of one is lent from its owner's handle rather than cloned.
#[derive(Default)]
pub struct Meter(std::sync::atomic::AtomicU64);

impl Meter {
	/// Adds `n`, returning the new count: through `&self`, as a lent handle allows.
	pub fn add(&self, n: u64) -> u64 {
		self.0.fetch_add(n, std::sync::atomic::Ordering::Relaxed) + n
	}

	pub fn count(&self) -> u64 {
		self.0.load(std::sync::atomic::Ordering::Relaxed)
	}

	/// Zeroes the count: through `&mut self`, which a lent handle refuses.
	pub fn reset(&mut self) {
		*self.0.get_mut() = 0;
	}
}

/// Owns a meter it lends, and one in a field, lent as the method's is.
#[derive(Default)]
pub struct Meters {
	first: Meter,
	pub last: Meter,
}

impl Meters {
	pub fn new() -> Self {
		Self::default()
	}

	/// The first meter, lent: a handle to it lives until these meters are borrowed
	/// exclusively, moved out, or freed.
	pub fn first(&self) -> &Meter {
		&self.first
	}

	/// Replaces the first meter, borrowing these exclusively, which ends what `first` lent.
	pub fn clear(&mut self) {
		self.first = Meter::default();
	}
}

/// Holds counters and lends them: each borrowed return is a new handle to a clone.
#[derive(Default)]
pub struct Registry {
	pub first: Counter,
	last: Option<Counter>,
}

impl Registry {
	pub fn new(first: Counter) -> Self {
		Self { first, last: None }
	}

	pub fn last(&self) -> Option<&Counter> {
		self.last.as_ref()
	}

	pub fn push(&mut self, counter: Counter) {
		self.last = Some(counter);
	}

	/// The first counter lent to `edit` for the call, returning what `edit` does: a closure
	/// called only during the call, which C lends as a function.
	pub fn edit(&mut self, edit: impl FnOnce(&mut Counter) -> bool) -> bool {
		EDITING.store(
			std::ptr::from_ref(&self.first) as usize,
			std::sync::atomic::Ordering::Relaxed,
		);
		edit(&mut self.first)
	}
}

/// The address of the counter [`Registry::edit`] lends.
static EDITING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A name that may borrow its text: its only generic parameter is a lifetime, so a
/// handle holds `Name<'static>`, which borrows nothing.
#[derive(Clone, Debug, PartialEq)]
pub struct Name<'a>(Cow<'a, str>);

impl Name<'_> {
	pub fn owned(text: String) -> Name<'static> {
		Name(Cow::Owned(text))
	}

	pub fn as_str(&self) -> &str {
		&self.0
	}

	/// `other` after this name, as a name that borrows neither.
	pub fn join(&self, other: &Name<'_>) -> Name<'static> {
		Name(Cow::Owned(format!("{}/{}", self.0, other.0)))
	}

	/// A name borrowing text that lives as long as the program, which a binding's string
	/// does once interned.
	pub fn borrowed(text: &'static str) -> Name<'static> {
		Name(Cow::Borrowed(text))
	}

	/// Whether both names borrow the same text, where it lies, not only equal text.
	pub fn shares(&self, other: &Name<'_>) -> bool {
		matches!((&self.0, &other.0), (Cow::Borrowed(a), Cow::Borrowed(b)) if std::ptr::eq(*a, *b))
	}

	/// Takes a parameter that must outlive the call, so a `String`, not a borrow of C's.
	pub fn rename(&mut self, text: impl AsRef<str> + Send + 'static) {
		self.0 = Cow::Owned(text.as_ref().to_string());
	}
}

/// The longer name: it takes only the lifetime of the names given, `'static` in a handle.
pub fn longer<'a>(a: Name<'a>, b: &Name<'a>) -> Name<'a> {
	match b.0.len() > a.0.len() {
		true => b.clone(),
		false => a,
	}
}

/// A name of no text. A lifetime only the return takes is the caller's to pick, so the
/// handle's `'static`.
pub fn unnamed<'a>() -> Name<'a> {
	Name(Cow::Borrowed(""))
}

/// The sum of a range.
pub fn total(range: Range<u64>) -> u64 {
	range.sum()
}

/// How far `start` reaches to `end`, past a `()` C does not pass.
pub fn reach(start: u32, _: (), end: u32) -> u32 {
	end - start
}

/// `v` times `by`, unless past `u128::MAX`.
pub fn scaled(v: u128, by: u64) -> Option<u128> {
	v.checked_mul(by.into())
}

/// Up to `N` numbers, as moq's `frame::Buffer` holds up to `N` frames: a const parameter
/// with a default, which exports at it and at each other value a signature names.
pub struct Batch<const N: usize = 4>(Vec<u32>);

impl<const N: usize> Batch<N> {
	/// An empty batch.
	pub fn new() -> Self {
		Self(Vec::new())
	}

	/// Append `v`, unless the batch already holds `N`.
	pub fn push(&mut self, v: u32) -> bool {
		let room = self.0.len() < N;
		if room {
			self.0.push(v);
		}
		room
	}

	/// How many numbers it holds at most.
	pub fn capacity(&self) -> usize {
		N
	}
}

/// A batch of two, an instance at a const argument other than the default.
pub fn pair() -> Batch<2> {
	Batch::new()
}

/// The sum of a batch's numbers, draining it: a const parameter at each instance's value.
pub fn drain<const N: usize>(batch: &mut Batch<N>) -> u64 {
	batch.0.drain(..).map(u64::from).sum()
}

/// Both batches' capacities: two const parameters, each at every instance's value.
pub fn capacities<const N: usize, const M: usize>(a: &Batch<N>, b: &Batch<M>) -> usize {
	a.capacity() + b.capacity()
}

/// Fill a batch with `v` until it holds `N`, lending what it holds: a future whose result
/// borrows a parameter, as moq's `group::Consumer::read_frames` does.
pub async fn fill<const N: usize>(batch: &mut Batch<N>, v: u32) -> &mut [u32] {
	while batch.push(v) {}
	&mut batch.0
}

/// A range, widened by `by` at each end.
pub fn widen(range: Range<u64>, by: u64) -> Range<u64> {
	range.start.saturating_sub(by)..range.end + by
}

/// How many of `0..10` a range takes, in any of the forms `RangeBounds` takes.
pub fn count(range: impl RangeBounds<u64>) -> usize {
	(0..10).filter(|n| range.contains(n)).count()
}

/// The end a limit puts on a range: none for 0.
pub fn end(limit: u64) -> Bound<u64> {
	match limit {
		0 => Bound::Unbounded,
		limit => Bound::Excluded(limit),
	}
}

/// A range's two bounds, up to the end `limit` puts on it: returned whole, a tuple, not
/// the pair a `RangeBounds` parameter takes.
pub fn ends(limit: u64) -> (Bound<u64>, Bound<u64>) {
	(Bound::Included(0), end(limit))
}

/// `n` if below `limit`, or else the bounds it falls outside: an error is the pair a
/// `RangeBounds` parameter takes, not a tuple.
pub fn below(n: u64, limit: u64) -> Result<u64, (Bound<u64>, Bound<u64>)> {
	match n < limit {
		true => Ok(n),
		false => Err(ends(limit)),
	}
}

/// A record of a range and a bound, which C holds after the structs they are.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
	pub range: Range<u64>,
	pub end: Bound<u64>,
}

impl Window {
	pub fn contains(&self, n: u64) -> bool {
		self.range.contains(&n) && (Bound::Unbounded, self.end).contains(&n)
	}
}

/// A one-slot channel: its receive is a future that waits for a send.
#[derive(Clone, Default)]
pub struct Channel {
	state: Arc<Mutex<(Option<Bytes>, Option<Waker>, Option<Weak<Waker>>)>>,
}

/// What a poll registers, made from a task's waker: it wakes the task only while it lives,
/// as kio's `Waiter` does, so a future polling with one must keep it between polls.
pub struct Waiter(Arc<Waker>);

impl Waiter {
	pub fn new(waker: Waker) -> Self {
		Self(Arc::new(waker))
	}

	/// The waker it was made from: returned as a clone, which wakes the same task.
	pub fn waker(&self) -> &Waker {
		&self.0
	}

	/// A context of its waker: returned as that waker's clone.
	pub fn context(&self) -> Context<'_> {
		Context::from_waker(&self.0)
	}

	/// Poll `future` with its waker, as kio's `Waiter::poll_future` does: a poll of the
	/// waiter it is a method of, and of a future the caller lends, which exports as a future
	/// owning it.
	pub fn poll_future<F: Future + ?Sized>(&self, future: std::pin::Pin<&mut F>) -> Poll<F::Output> {
		future.poll(&mut self.context())
	}
}

/// Calls `poll` until it fails, returning its error: with `start`, then with each instant
/// it asks for, as if the clock reached it at once, or once its waiter is woken, with the
/// last one again. A closure its future keeps, as kio's `clock::drive` keeps one.
pub async fn drive<E>(
	start: std::time::Instant,
	mut poll: impl FnMut(std::time::Instant, &Waiter) -> Result<Option<std::time::Instant>, E> + Unpin,
) -> E {
	let mut now = start;
	std::future::poll_fn(move |cx| {
		let waiter = Waiter::new(cx.waker().clone());
		loop {
			match poll(now, &waiter) {
				Ok(Some(at)) => now = at,
				Ok(None) => return Poll::Pending,
				Err(err) => return Poll::Ready(err),
			}
		}
	})
	.await
}

/// A thread running a closure it keeps past the call that made it, which
/// [`Spawned::join`] waits on.
pub struct Spawned {
	thread: std::thread::JoinHandle<u32>,
}

impl Spawned {
	pub fn new(task: impl FnOnce() -> u32 + Send + 'static) -> Self {
		Self {
			thread: std::thread::spawn(task),
		}
	}

	/// What the closure returned.
	pub fn join(self) -> u32 {
		self.thread.join().expect("the thread finishes")
	}
}

/// What `make` returns for `n`, as kio's `Waiter::batch` returns its closure's: a type
/// parameter no bound fixes.
pub fn made<R>(n: u32, make: impl FnOnce(u32) -> R) -> R {
	make(n)
}

/// How many times a [`Ticket`]'s `Clone` ran.
static TICKET_CLONES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// A `Copy` value whose `Clone` counts its calls, which a closure taking or returning one
/// never runs, as Rust copies it instead.
#[derive(Copy, Debug, PartialEq)]
pub struct Ticket {
	number: u32,
}

#[expect(clippy::expl_impl_clone_on_copy, reason = "the count shows a copy never clones")]
impl Clone for Ticket {
	fn clone(&self) -> Self {
		TICKET_CLONES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
		*self
	}
}

impl Ticket {
	pub fn new(number: u32) -> Self {
		Self { number }
	}

	pub fn number(&self) -> u32 {
		self.number
	}

	/// How many times any ticket's `Clone` ran.
	pub fn clones() -> u32 {
		TICKET_CLONES.load(std::sync::atomic::Ordering::Relaxed)
	}
}

/// Each name and its ticket, as given, so a binding passes back what it returned.
pub fn tickets(entries: Vec<(String, Ticket)>) -> Vec<(String, Ticket)> {
	entries
}

/// The ticket `swap` returns for `ticket`.
pub fn reissue(ticket: Ticket, swap: impl Fn(Ticket) -> Ticket) -> Ticket {
	swap(ticket)
}

/// The ticket `swap` returns for a lent `ticket`, which Go spells as `reissue`'s closure.
pub fn reissue_lent(ticket: &Ticket, swap: impl Fn(&Ticket) -> Ticket) -> Ticket {
	swap(ticket)
}

/// What `check` returns, a closure with no value but its error.
pub fn attempt(check: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
	check()
}

/// The last `len` bytes of `file`, and where they start, which it seeks back to: a reader
/// the caller implements, lent for the call, as moq-mux's `container::mp4::repair` takes.
pub fn tail<F: std::io::Read + std::io::Seek>(file: &mut F, len: u64) -> std::io::Result<(Vec<u8>, u64)> {
	use std::io::SeekFrom;
	let end = file.seek(SeekFrom::End(0))?;
	file.seek(SeekFrom::Start(end.saturating_sub(len)))?;
	let mut tail = Vec::new();
	file.read_to_end(&mut tail)?;
	let start = file.seek(SeekFrom::Current(-(tail.len() as i64)))?;
	Ok((tail, start))
}

/// Where seeking `file` to `at` from its start lands: any `u64`, as `Cursor` takes.
pub fn seek_to<F: std::io::Read + std::io::Seek>(file: &mut F, at: u64) -> std::io::Result<u64> {
	file.seek(std::io::SeekFrom::Start(at))
}

impl Channel {
	pub fn new() -> Self {
		Self::default()
	}

	/// Drop any payload not yet received. Exclusive, so it cannot run while a receive
	/// borrows the channel.
	pub fn clear(&mut self) {
		self.state.lock().unwrap().0 = None;
	}

	pub fn send(&self, payload: Bytes) {
		let mut state = self.state.lock().unwrap();
		state.0 = Some(payload);
		if let Some(waker) = state.1.take() {
			waker.wake();
		}
		if let Some(waiter) = state.2.take().and_then(|waiter| waiter.upgrade()) {
			waiter.wake_by_ref();
		}
	}

	/// Sends from another thread, before it returns, so the wake lands from that thread, as
	/// an async runtime's worker wakes a task: the one way Dart, whose isolates share no
	/// object, has to wake from another thread.
	pub fn send_from_thread(&self, payload: Bytes) {
		std::thread::scope(|scope| {
			scope.spawn(|| self.send(payload));
		});
	}

	/// The next payload sent, waiting for it.
	pub async fn recv(&self) -> Bytes {
		std::future::poll_fn(|cx| {
			let mut state = self.state.lock().unwrap();
			match state.0.take() {
				Some(payload) => Poll::Ready(payload),
				None => {
					state.1 = Some(cx.waker().clone());
					Poll::Pending
				}
			}
		})
		.await
	}

	/// Wakes `waker` on the next send, keeping it until then, as a driver's poll keeps the
	/// waker it is given.
	pub fn notify(&self, waker: Waker) {
		self.state.lock().unwrap().1 = Some(waker);
	}

	/// Sends at once, as a function that is not `async` runs when called, and returns a
	/// future that borrows nothing.
	pub fn send_later(&self, payload: Bytes) -> impl Future<Output = ()> + use<> {
		self.send(payload);
		std::future::ready(())
	}

	/// Sends `tag` at once, as a function that is not `async` runs when called, and returns a
	/// future that borrows this channel and `tag`: what it next receives, after `tag`.
	pub fn tagged<'a>(&'a self, tag: &'a str) -> impl Future<Output = String> + 'a {
		self.send(Bytes::copy_from_slice(tag.as_bytes()));
		async move {
			let payload = self.recv().await;
			format!("{tag} {}", String::from_utf8_lossy(&payload))
		}
	}

	/// Sends the sum of `items`, if any, at once, and returns a future that borrows them:
	/// their count once this channel next receives.
	pub fn itemized<'a>(&'a self, items: Option<&'a [u32]>) -> impl Future<Output = usize> + 'a {
		let sum: u32 = items.unwrap_or_default().iter().sum();
		self.send(Bytes::copy_from_slice(&sum.to_le_bytes()));
		async move {
			self.recv().await;
			items.map_or(0, <[u32]>::len)
		}
	}

	/// `recv`'s poll, which does not map: left out, since `recv` exports as a future.
	pub fn poll_recv(&self, cx: &mut std::task::Context<'_>) -> Poll<Bytes> {
		let mut state = self.state.lock().unwrap();
		match state.0.take() {
			Some(payload) => Poll::Ready(payload),
			None => {
				state.1 = Some(cx.waker().clone());
				Poll::Pending
			}
		}
	}

	/// The next payload, waiting for it through `waiter`: a poll with no async twin, which
	/// exports as a future.
	pub fn poll_take(&self, waiter: &Waiter) -> Poll<Bytes> {
		let mut state = self.state.lock().unwrap();
		match state.0.take() {
			Some(payload) => Poll::Ready(payload),
			None => {
				state.2 = Some(Arc::downgrade(&waiter.0));
				Poll::Pending
			}
		}
	}

	/// The next payload's length past `extra`, failing once it is empty: a poll through a
	/// context, of `&mut self`, which passes `extra` again at each poll.
	pub fn poll_measure(&mut self, cx: &mut Context<'_>, extra: usize) -> Poll<Result<usize, Error>> {
		let mut state = self.state.lock().unwrap();
		match state.0.take() {
			Some(payload) if payload.is_empty() => Poll::Ready(Err(Error::Underflow { by: 1 })),
			Some(payload) => Poll::Ready(Ok(payload.len() + extra)),
			None => {
				state.1 = Some(cx.waker().clone());
				Poll::Pending
			}
		}
	}

	/// Wakes the context's task on the next send, as `notify` wakes its waker: C passes the
	/// waker the context is made of, and nothing for the `()` before it.
	pub fn listen(&self, _: (), cx: &Context<'_>) {
		self.state.lock().unwrap().1 = Some(cx.waker().clone());
	}

	/// The next payload and its length: a future's tuple is written through its poll's
	/// out pointers.
	pub async fn recv_sized(&self) -> (Bytes, usize) {
		let payload = self.recv().await;
		let len = payload.len();
		(payload, len)
	}

	/// Fails once the payload is empty.
	pub async fn recv_nonempty(&self) -> Result<Bytes, Error> {
		match self.recv().await {
			payload if payload.is_empty() => Err(Error::Underflow { by: 1 }),
			payload => Ok(payload),
		}
	}
}

/// `n` halved if even, at once: a future whose output may be `None`.
pub async fn halved(n: u32) -> Option<u32> {
	n.is_multiple_of(2).then_some(n / 2)
}

/// `value`, on its third poll: each before wakes its waker before it returns pending, so
/// the wake lands before the poller waits for one, even past a first poll Kotlin makes
/// with no waker.
pub async fn yielded(value: u32) -> u32 {
	let mut polls = 0;
	std::future::poll_fn(|cx| {
		polls += 1;
		if polls == 3 {
			return Poll::Ready(());
		}
		cx.waker().wake_by_ref();
		Poll::Pending
	})
	.await;
	value
}

/// `value` doubled once `first` finishes: an `async fn` polling a future the caller
/// implements until it is done.
pub async fn after(first: impl Future<Output = ()> + Send, value: u32) -> u32 {
	first.await;
	value * 2
}

/// Runs `future` until it pends more than `polls` times, which fails with `Overflow`: a
/// function that is not `async` returning a future made of the caller's, as kio's
/// `clock::timeout` is.
pub fn within<F: IntoFuture>(polls: u32, future: F) -> Within<F::IntoFuture> {
	Within {
		future: Box::pin(future.into_future()),
		left: polls,
	}
}

/// The future [`within`] returns.
pub struct Within<F> {
	future: std::pin::Pin<Box<F>>,
	left: u32,
}

impl<F: Future> Future for Within<F> {
	type Output = Result<F::Output, Error>;

	fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		if let Poll::Ready(output) = self.future.as_mut().poll(cx) {
			return Poll::Ready(Ok(output));
		}
		if self.left == 0 {
			return Poll::Ready(Err(Error::Overflow));
		}
		self.left -= 1;
		Poll::Pending
	}
}

/// A time: `Copy` with a private field, so a value only Rust makes, copied across.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stamp {
	millis: u64,
}

impl Stamp {
	pub fn from_millis(millis: u64) -> Self {
		Self { millis }
	}

	pub fn secs(self) -> u64 {
		self.millis / 1000
	}

	/// It rounded down to whole seconds, through an `impl Trait`, so it crosses as its bytes.
	pub fn whole(self) -> impl Into<Stamp> + use<> {
		Self {
			millis: self.millis / 1000 * 1000,
		}
	}

	pub fn later(self, other: Stamp) -> bool {
		self.millis > other.millis
	}
}

/// A time on a clock of its own rate, as moq's `Timestamp`: `Copy` with private fields, past
/// a word on every target, and never all zero bytes, so only a value Rust made may cross.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moment {
	ticks: u64,
	rate: std::num::NonZero<u64>,
}

impl Moment {
	/// `ticks` of `rate` a second, or none at a rate of zero.
	pub fn new(ticks: u64, rate: u64) -> Option<Self> {
		std::num::NonZero::new(rate).map(|rate| Self { ticks, rate })
	}

	/// Whole milliseconds since zero.
	pub fn millis(&self) -> u64 {
		(u128::from(self.ticks) * 1000 / u128::from(self.rate.get())) as u64
	}

	/// The moment `ticks` later, at the same rate, wrapping as `u64` does.
	pub fn after(self, ticks: u64) -> Self {
		Self {
			ticks: self.ticks.wrapping_add(ticks),
			..self
		}
	}
}

/// The moment `shift` returns for `moment`, which C copies in and out.
pub fn shifted(moment: Moment, shift: impl Fn(Moment) -> Moment) -> Moment {
	shift(moment)
}

/// A value with padding, after its `u8`: made from bytes all `0xff`, so its padding likely
/// holds them, which the export zeroes before any language sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gap {
	wide: u64,
	narrow: u8,
}

impl Gap {
	pub fn new(wide: u64, narrow: u8) -> Self {
		// SAFETY: every byte pattern is a `Gap`, whose fields are integers.
		let mut gap: Gap = unsafe { std::mem::transmute([0xffu8; std::mem::size_of::<Gap>()]) };
		gap.wide = wide;
		gap.narrow = narrow;
		gap
	}

	pub fn sum(&self) -> u64 {
		self.wide.wrapping_add(u64::from(self.narrow))
	}
}

/// A value named as Kotlin's `LongArray`, which holds a value's words, with a method named
/// as C++'s member holding its bytes: each generator keeps its own apart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LongArray {
	words: [u64; 2],
}

impl LongArray {
	pub fn new(low: u64, high: u64) -> Self {
		LongArray { words: [low, high] }
	}

	pub fn c_(&self) -> u64 {
		self.words[0].wrapping_add(self.words[1])
	}
}

/// A `Copy` type whose `None` leaves its payload's bytes uninitialized, so no padding table
/// covers it: a handle, rather than a value.
#[derive(Clone, Copy, Debug)]
pub struct Berth {
	at: Option<u32>,
}

impl Berth {
	pub fn new(at: Option<u32>) -> Self {
		Self { at }
	}

	pub fn at(&self) -> Option<u32> {
		self.at
	}
}

/// A running count, as moq's `stats::Traffic`: `Copy` with private fields, changed in place
/// through `&mut self`, so a value whose every copy changes alone.
#[derive(Clone, Copy, Debug)]
pub struct Score {
	count: u64,
}

impl Score {
	pub fn of(count: u64) -> Self {
		Self { count }
	}

	pub fn count(&self) -> u64 {
		self.count
	}

	/// Adds `other`'s count to this one, as Rust copies `other`.
	pub fn add(&mut self, other: Score) {
		self.count = self.count.wrapping_add(other.count);
	}

	/// Twice the count, as Rust copies `self`.
	pub fn doubled(self) -> Score {
		Self {
			count: self.count.wrapping_mul(2),
		}
	}

	/// The count, leaving zero in its place.
	pub fn take(&mut self) -> u64 {
		std::mem::take(&mut self.count)
	}

	/// Moves `other`'s count into this one, as `&mut` of a value Rust writes back too, and
	/// returns both counts before, as a tuple.
	pub fn absorb(&mut self, other: &mut Score) -> (u64, u64) {
		let before = (self.count, other.count);
		self.count = self.count.wrapping_add(std::mem::take(&mut other.count));
		before
	}
}

/// A tally, as moq's `stats::Presence`: `Copy`, its fields `pub` but `#[non_exhaustive]`,
/// so a value whose fields read and write through accessors.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Hits {
	pub hits: u64,
	pub misses: u32,
}

impl Hits {
	pub fn of(hits: u64, misses: u32) -> Self {
		Self { hits, misses }
	}

	/// Adds `other` into this one, as moq's `add` does.
	pub fn add(&mut self, other: Hits) {
		self.hits = self.hits.wrapping_add(other.hits);
		self.misses = self.misses.wrapping_add(other.misses);
	}

	/// Whether `other` equals this one, counting it a hit: named as `PartialEq`'s C
	/// function, which the clash renames.
	pub fn eq(&mut self, other: Hits) -> bool {
		let same = *self == other;
		self.hits = self.hits.wrapping_add(1);
		same
	}
}

/// A span whose `pub` start sits beside a private length, which no record could hold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extent {
	pub start: u64,
	len: u32,
}

impl Extent {
	pub fn new(start: u64, len: u32) -> Self {
		Self { start, len }
	}

	pub fn end(&self) -> u64 {
		self.start.wrapping_add(u64::from(self.len))
	}

	/// Moves the window `by` later, returning its old start.
	pub fn slide(&mut self, by: u64) -> u64 {
		let start = self.start;
		self.start = start.wrapping_add(by);
		start
	}
}

/// A score and its weight: a record holding a `Copy` handle, copied in and out.
pub struct Weighted {
	pub score: Score,
	pub weight: u32,
}

impl Weighted {
	pub fn total(&self) -> u64 {
		self.score.count.wrapping_mul(u64::from(self.weight))
	}
}

/// A stamp and a count: a record holding a value only Rust makes, whose method reads it.
pub struct Stamped {
	pub at: Stamp,
	pub count: u32,
}

impl Stamped {
	/// Whole seconds at the stamp, plus the count.
	pub fn total(&self) -> u64 {
		self.at.secs() + u64::from(self.count)
	}
}

/// A message: an object, since one field is private and it is not `Copy`, whose `pub`
/// fields read as getters.
#[derive(Clone, Debug)]
pub struct Message {
	pub stamp: Option<Stamp>,
	pub payload: Bytes,
	sent: bool,
}

impl Message {
	pub fn new(stamp: Option<Stamp>, payload: Bytes) -> Self {
		Self {
			stamp,
			payload,
			sent: false,
		}
	}

	pub fn sent(&self) -> bool {
		self.sent
	}
}

/// A sink: a callback interface, implemented in C.
pub trait Log: Send + Sync {
	/// Whether the line was kept.
	fn log(&self, level: u8, line: &str) -> bool;
}

/// Each line to `log` as a type parameter takes it, returning how many it kept.
pub fn emit(log: impl Log, lines: Vec<u8>) -> u32 {
	lines.iter().filter(|&&n| log.log(n, &format!("line {n}"))).count() as u32
}

/// [`emit`] on another thread, which the call waits on.
pub fn emit_from_thread(log: impl Log, lines: Vec<u8>) -> u32 {
	on_thread(&log, &lines)
}

/// [`emit_from_thread`] before it returns its future, and again once polled.
pub fn emit_later(log: Box<dyn Log>, lines: Vec<u8>) -> impl Future<Output = u32> + use<> {
	let eager = on_thread(&*log, &lines);
	async move { eager + on_thread(&*log, &lines) }
}

fn on_thread(log: &(impl Log + ?Sized), lines: &[u8]) -> u32 {
	std::thread::scope(|scope| {
		scope
			.spawn(|| lines.iter().filter(|&&n| log.log(n, &format!("line {n}"))).count() as u32)
			.join()
			.expect("the thread finishes")
	})
}

/// A record named as Swift's own error a cancelled task throws, which the binding renames.
pub struct CancellationError {
	pub code: u32,
}

/// Each of `0..n` to `count` before it returns its future, then `n` doubled once polled,
/// or `Overflow`: an eager, fallible future that lends a closure.
pub fn counted(n: u32, mut count: impl FnMut(u32)) -> std::future::Ready<Result<u32, Error>> {
	(0..n).for_each(&mut count);
	std::future::ready(n.checked_mul(2).ok_or(Error::Overflow))
}

/// Each line to `keep` on another thread, returning how many it kept: a lent closure may
/// run on any thread until the call returns.
pub fn filter(lines: Vec<u8>, mut keep: impl FnMut(u8, &str) -> bool + Send) -> u32 {
	std::thread::scope(|scope| {
		let kept = scope.spawn(move || lines.iter().filter(|&&n| keep(n, &format!("line {n}"))).count());
		kept.join().expect("the thread finishes") as u32
	})
}

/// Logs a line once polled, returning whether it was kept: the future holds `log` until it
/// is dropped.
pub fn log_later(log: Box<dyn Log>) -> impl Future<Output = bool> + use<> {
	async move { log.log(0, "later") }
}

/// Logs the count of `counter`, then of each of `more`, to `log` at `level`, returning how
/// many it kept.
pub fn log_counts(log: Box<dyn Log>, level: u8, counter: &Counter, more: Vec<Counter>) -> u32 {
	let counts = std::iter::once(counter).chain(&more).map(Counter::get);
	counts.filter(|n| log.log(level, &n.to_string())).count() as u32
}

/// Whether this runs on the thread its first call ran on.
pub fn on_first_thread() -> bool {
	static FIRST: std::sync::OnceLock<std::thread::ThreadId> = std::sync::OnceLock::new();
	let here = std::thread::current().id();
	*FIRST.get_or_init(|| here) == here
}

/// Holds a foreign implementation as a trait object, dropping it with itself.
pub struct Logger {
	sink: Box<dyn Log>,
}

impl Logger {
	pub fn new(sink: Box<dyn Log>) -> Self {
		Self { sink }
	}

	pub fn write(&self, line: &str) -> bool {
		self.sink.log(0, line)
	}

	/// [`Logger::write`] on another thread, which the call waits on.
	pub fn write_from_thread(&self, line: &str) -> bool {
		std::thread::scope(|scope| scope.spawn(|| self.write(line)).join().expect("the thread finishes"))
	}
}

/// A thread logging a line to a foreign implementation, which [`LogThread::join`] waits on.
pub struct LogThread {
	thread: std::thread::JoinHandle<bool>,
}

impl LogThread {
	pub fn new(log: Box<dyn Log>, line: String) -> Self {
		Self {
			thread: std::thread::spawn(move || log.log(0, &line)),
		}
	}

	/// Whether the line was kept.
	pub fn join(self) -> bool {
		self.thread.join().expect("the thread finishes")
	}
}

/// A future that never finishes, whose drop logs a line to `log` on another thread it
/// waits on.
pub fn log_on_drop(log: Box<dyn Log>) -> impl Future<Output = ()> + use<> {
	let guard = LogOnDrop(log);
	async move {
		let _guard = guard;
		std::future::pending().await
	}
}

struct LogOnDrop(Box<dyn Log>);

impl Drop for LogOnDrop {
	fn drop(&mut self) {
		on_thread(&*self.0, &[0]);
	}
}

/// A list: every field is `pub`, but it holds itself, which C cannot hold in place, so it
/// is an object.
#[derive(Debug)]
pub struct Node {
	pub value: u64,
	pub next: Option<Box<Node>>,
}

impl Node {
	pub fn new(value: u64, next: Option<Box<Node>>) -> Self {
		Self { value, next }
	}

	/// Prepend `value`.
	pub fn push(&mut self, value: u64) {
		let rest = std::mem::replace(self, Node { value, next: None });
		self.next = Some(Box::new(rest));
	}

	pub fn sum(&self) -> u64 {
		self.value + self.next.as_ref().map_or(0, |next| next.sum())
	}
}

/// A tally: `Send` but not `Sync`, so even `&self` borrows it exclusively, and comparing
/// a handle with itself is one borrow.
#[derive(Debug, Default, PartialEq)]
pub struct Tally {
	count: std::cell::Cell<u32>,
}

impl Tally {
	pub fn new() -> Self {
		Self::default()
	}

	/// Counts through `&self`, which only an exclusive borrow makes sound across threads.
	pub fn bump(&self) -> u32 {
		self.count.set(self.count.get() + 1);
		self.count.get()
	}
}

/// Owns a tally it lends, and so is not `Sync` either: its handle lends on one thread at a
/// time.
#[derive(Default)]
pub struct Tallies {
	first: Tally,
}

impl Tallies {
	pub fn new() -> Self {
		Self::default()
	}

	/// The first tally, lent, as [`Meters::first`] lends its meter.
	pub fn first(&self) -> &Tally {
		&self.first
	}
}

static DROPPED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// A level its clones share through an `Arc`, with no `&mut self` method: a clone no call
/// tells apart from the original, so copies may share one handle.
#[derive(Clone)]
pub struct Gauge {
	level: Arc<Level>,
}

/// What a gauge's clones share, whose drop counts in [`dropped`], as a token's does.
struct Level(std::sync::atomic::AtomicU64);

impl Drop for Level {
	fn drop(&mut self) {
		DROPPED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
	}
}

impl Gauge {
	pub fn new(level: u64) -> Self {
		Self {
			level: Arc::new(Level(level.into())),
		}
	}

	/// Set the level each clone reads.
	pub fn set(&self, level: u64) {
		self.level.0.store(level, std::sync::atomic::Ordering::Relaxed);
	}

	pub fn level(&self) -> u64 {
		self.level.0.load(std::sync::atomic::Ordering::Relaxed)
	}

	/// The level, moving this clone out, which leaves the others.
	pub fn into_level(self) -> u64 {
		self.level()
	}

	/// The level, read once [`yielded`] finishes: a future borrowing this clone meanwhile.
	pub async fn later(&self) -> u64 {
		yielded(0).await;
		self.level()
	}

	/// Whether the level reaches `this`, read once [`yielded`] finishes: a parameter named
	/// as a binding might name the receiver.
	pub async fn reaches(&self, this: u64) -> bool {
		yielded(0).await;
		self.level() >= this
	}
}

/// The gauge lent shared to `watch` for the call.
pub fn watch(gauge: &Gauge, watch: impl FnOnce(&Gauge) -> u64) -> u64 {
	watch(gauge)
}

/// A value set once, through `&self`: its lock sits behind a `Box`, so the type is `Freeze`,
/// yet each clone copies it into a lock of its own, which the clone then sets apart.
#[derive(Clone, Default)]
pub struct Latch {
	value: Box<std::sync::OnceLock<u32>>,
}

impl Latch {
	/// Set the value, unless it was set.
	pub fn set(&self, value: u32) -> bool {
		self.value.set(value).is_ok()
	}

	pub fn get(&self) -> Option<u32> {
		self.value.get().copied()
	}
}

/// A token: every field is `pub`, but its `Drop` must run when C frees it, so it is an
/// object rather than a record.
#[derive(Clone)]
pub struct Token {
	pub id: u32,
}

impl Token {
	pub fn new(id: u32) -> Self {
		Self { id }
	}

	/// The token twice, borrowed: C gets a clone of each, which the array frees.
	pub fn twice(&self) -> Vec<&Token> {
		vec![self, self]
	}
}

impl Drop for Token {
	fn drop(&mut self) {
		DROPPED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
	}
}

/// `len` bytes of `id`'s low byte, whose drop counts in [`dropped`], as a token's does.
pub fn token_bytes(id: u32, len: u32) -> Bytes {
	// The token is held only for its drop.
	struct Owner(#[allow(dead_code)] Token, Vec<u8>);

	impl AsRef<[u8]> for Owner {
		fn as_ref(&self) -> &[u8] {
			&self.1
		}
	}

	Bytes::from_owner(Owner(Token::new(id), vec![id as u8; len as usize]))
}

/// A payload and its sequence number: a record, whose bytes cross in both ways.
pub struct Packet {
	pub seq: u32,
	pub payload: Bytes,
}

/// The packet after `packet`, which carries its payload on.
pub fn next_packet(packet: Packet) -> Packet {
	Packet {
		seq: packet.seq + 1,
		payload: packet.payload,
	}
}

/// Retry settings: each field is unset until C sets it, and the method of its name
/// resolves it, as Rust's accessors do, and `set_limit` sets it, as Rust's setters do: C
/// names each apart from the field's accessors. Not exhaustive, so an object with getters.
#[non_exhaustive]
pub struct Retry {
	pub limit: Option<u32>,
	pub delay: Option<u32>,
	/// Its field exports, but the method of its name is refused, so the config excludes
	/// both, and the census lists only `field@Retry::jitter` as excluding what exports.
	pub jitter: u32,
}

impl Retry {
	pub fn new(limit: Option<u32>) -> Self {
		Self {
			limit,
			delay: None,
			jitter: 0,
		}
	}

	pub fn limit(&self) -> u32 {
		self.limit.unwrap_or(3)
	}

	pub fn delay(&self) -> u32 {
		self.delay.unwrap_or(100)
	}

	pub fn set_limit(&mut self, limit: u32) {
		self.limit = Some(limit);
	}

	/// The jitter capped by `cap`, if any: a nested `Option`, which the export refuses.
	pub fn jitter(&self, cap: Option<Box<Option<u32>>>) -> u32 {
		cap.and_then(|cap| *cap).map_or(self.jitter, |cap| self.jitter.min(cap))
	}
}

/// A digest, as moq's FLAC config holds its MD5: an object, whose array field C reads and
/// assigns by value.
#[non_exhaustive]
pub struct Digest {
	pub sum: [u8; 4],
}

impl Digest {
	pub fn new() -> Self {
		Self { sum: [0; 4] }
	}

	/// The sum with each byte flipped: an array taken and returned by value.
	pub fn flipped(&self, mask: [u8; 4]) -> [u8; 4] {
		std::array::from_fn(|i| self.sum[i] ^ mask[i])
	}
}

/// The 4 bytes that 8 hex digits spell, as moq parses a certificate's fingerprint into 32.
pub fn fingerprint(hex: &str) -> Result<[u8; 4], String> {
	if hex.len() != 8 {
		return Err(format!("{} hex digits, not 8", hex.len()));
	}
	let mut sum = [0; 4];
	for (i, byte) in sum.iter_mut().enumerate() {
		*byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).map_err(|e| e.to_string())?;
	}
	Ok(sum)
}

/// An enum whose variant Dart would name as the class of `[u8; 4]`, which keeps them apart.
pub enum Array {
	U8x4(u8),
}

/// A type named as Swift's, which an array's `Hashable` conformance must not take for it.
pub struct Hasher {
	pub seed: u8,
}

/// How many tokens have dropped.
pub fn dropped() -> u32 {
	DROPPED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Both tallies bumped: C may pass one handle twice, which Rust allows of `&`.
pub fn bump_both(a: &Tally, b: &Tally) -> u32 {
	a.bump() + b.bump()
}

/// What `by` returns, plus `by_data`: a closure taking nothing, whose data C names apart.
pub fn offset(by: impl Fn() -> u32, by_data: u32) -> u32 {
	by() + by_data
}

/// What `by_` returns, plus `c_by_`: names C++ gives a closure's trampoline and a parameter
/// renamed apart from a generated name, which must stay apart.
pub fn shift(by_: impl Fn() -> u32, c_by_: u32) -> u32 {
	by_() + c_by_
}

/// A position, which C reaches only as [`walk`] lends it.
pub struct Cursor {
	at: u64,
}

impl Cursor {
	pub fn at(&self) -> u64 {
		self.at
	}
}

/// A cursor at each of `0..steps` lent shared to `visit`, summing what it returns.
pub fn walk(steps: u64, mut visit: impl FnMut(&Cursor) -> u64) -> u64 {
	(0..steps).map(|at| visit(&Cursor { at })).sum()
}

/// An enum without variants: no value is one, so C never holds one, as an object no export
/// returns, and the bindings leave it out.
pub enum Void {}

/// An object no export returns but its own method, so C never holds one: left out of the
/// bindings, with what takes it, and listed in the census.
pub struct Lease(u8);

impl Lease {
	pub fn split(&mut self) -> Lease {
		Lease(self.0)
	}
}

/// What C can never call, since it never holds a `Lease`.
pub fn release(lease: Lease) -> u8 {
	lease.0
}

/// A generic type no export instantiates, so C never holds one either.
pub struct Unnamed<T>(pub T);

/// Each of `0..steps` passed to `visit`, which returns nothing; how many it visited.
pub fn each(steps: u32, mut visit: impl FnMut(u32)) -> u32 {
	(0..steps).for_each(&mut visit);
	steps
}

/// The tally lent shared to `peek` for the call, on one thread, as it is not `Sync`.
pub fn peek(tally: &Tally, peek: impl FnOnce(&Tally) -> u32) -> u32 {
	peek(tally)
}

/// The length of a payload, borrowed for the call.
pub fn len(payload: &[u8]) -> usize {
	payload.len()
}

/// Converts into owned bytes, as moq's `IntoBytes` does: text implements it too, but its
/// supertrait makes a parameter it bounds bytes, and owned ones, as it must be `'static`.
pub trait Payload: AsRef<[u8]> + 'static {
	fn into_bytes(self) -> Bytes;
}

impl Payload for Bytes {
	fn into_bytes(self) -> Bytes {
		self
	}
}

impl Payload for Vec<u8> {
	fn into_bytes(self) -> Bytes {
		self.into()
	}
}

impl Payload for String {
	fn into_bytes(self) -> Bytes {
		self.into()
	}
}

impl Payload for &'static str {
	fn into_bytes(self) -> Bytes {
		Bytes::from_static(self.as_bytes())
	}
}

impl Payload for &'static [u8] {
	fn into_bytes(self) -> Bytes {
		Bytes::from_static(self)
	}
}

/// A payload's bytes in hex, which need not be UTF-8.
pub fn hex(payload: impl Payload) -> String {
	payload.into_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

/// A value and when it was stamped, as moq's `Timed` is, which bytes convert into.
pub struct Timed<P> {
	pub value: P,
	pub at: Option<Stamp>,
}

impl<B: AsRef<[u8]>> From<B> for Timed<Bytes> {
	fn from(value: B) -> Self {
		Self {
			value: Bytes::copy_from_slice(value.as_ref()),
			at: None,
		}
	}
}

/// A timed payload's bytes in hex, which need not be UTF-8, owned as it is `'static`.
pub fn timed_hex(payload: impl Into<Timed<Bytes>> + 'static) -> String {
	hex(payload.into().value)
}

/// A reference stamped with no time, as moq's `Timed` converts one.
impl<'a, V> From<&'a V> for Timed<&'a V> {
	fn from(value: &'a V) -> Self {
		Self { value, at: None }
	}
}

/// A timed value's compact JSON text, taken as moq's JSON tracks take theirs.
pub fn timed_text<'a, T: serde::Serialize + 'a>(value: impl Into<Timed<&'a T>>) -> String {
	serde_json::to_string(value.into().value).expect("a JSON document serializes")
}

/// A timed payload on a track, which what converts into one converts into.
pub struct Sample(Timed<Bytes>);

impl<T: Into<Timed<Bytes>>> From<T> for Sample {
	fn from(value: T) -> Self {
		Self(value.into())
	}
}

/// A sample's bytes in hex, which need not be UTF-8, two conversions from what C lends.
pub fn sample_hex(sample: impl Into<Sample>) -> String {
	hex(sample.into().0.value)
}

/// A sample queued past the conversion, so what converts into it is owned.
pub struct Queued(Sample);

impl<T: Into<Sample> + 'static> From<T> for Queued {
	fn from(value: T) -> Self {
		Self(value.into())
	}
}

/// A queued sample's bytes in hex, which need not be UTF-8, three conversions from owned bytes.
pub fn queued_hex(queued: impl Into<Queued>) -> String {
	hex(queued.into().0.0.value)
}

/// A name from a label, or from what converts into a `Tock`, which converts from what
/// converts into a `Tick`: a cycle, which only the label leaves.
pub struct Tick(String);

impl From<Label> for Tick {
	fn from(label: Label) -> Self {
		Self(label.name)
	}
}

impl<T: Into<Tock> + Copy> From<T> for Tick {
	fn from(value: T) -> Self {
		Self(value.into().0)
	}
}

/// What converts into a `Tick` converts into a `Tock`.
pub struct Tock(String);

impl<T: Into<Tick> + Copy> From<T> for Tock {
	fn from(value: T) -> Self {
		Self(value.into().0)
	}
}

/// The name a `Tick` takes.
pub fn tick_name(tick: impl Into<Tick>) -> String {
	tick.into().0
}

/// A label's name, scoped to a lifetime it borrows nothing for.
pub struct Scoped<'a>(String, std::marker::PhantomData<&'a ()>);

impl From<Label> for Scoped<'_> {
	fn from(label: Label) -> Self {
		Self(label.name, std::marker::PhantomData)
	}
}

/// What converts into a `Scoped<'static>`, which an owned label does.
pub struct Rooted(Scoped<'static>);

impl<T: Into<Scoped<'static>>> From<T> for Rooted {
	fn from(value: T) -> Self {
		Self(value.into())
	}
}

/// The name a `Rooted` takes, through a target whose lifetime is `'static`.
pub fn rooted_name(rooted: impl Into<Rooted>) -> String {
	rooted.into().0.0
}

/// Text copied from a borrow, so `'static` whatever it borrowed.
pub struct Copied<'a>(String, std::marker::PhantomData<&'a ()>);

impl<'a> From<&'a str> for Copied<'static> {
	fn from(text: &'a str) -> Self {
		Self(text.to_string(), std::marker::PhantomData)
	}
}

/// What converts into a `Copied<'static>`, which a borrow C lends does.
pub struct Stored(Copied<'static>);

impl<T: Into<Copied<'static>>> From<T> for Stored {
	fn from(value: T) -> Self {
		Self(value.into())
	}
}

/// The text a `Stored` takes, through a target whose lifetime its source does not borrow for.
pub fn stored_text(stored: impl Into<Stored>) -> String {
	stored.into().0.0
}

/// A label's name, layered over a `Vec` of its first type once more for each conversion,
/// until its second type is no longer `u8`.
pub struct Layer<T, U>(String, std::marker::PhantomData<(T, U)>);

impl From<Label> for Layer<u8, u8> {
	fn from(label: Label) -> Self {
		Self(label.name, std::marker::PhantomData)
	}
}

impl<T, S: Into<Layer<Vec<T>, T>> + Copy> From<S> for Layer<T, u8> {
	fn from(value: S) -> Self {
		Self(value.into().0, std::marker::PhantomData)
	}
}

/// The name a `Layer<u8, u8>` takes, through a chain that grows twice and ends.
pub fn layer_name(layer: impl Into<Layer<u8, u8>>) -> String {
	layer.into().0
}

pub fn greet(name: impl Into<String>, shape: Shape) -> String {
	format!("hello {}, a {shape:?}", name.into())
}

pub fn describe(event: &Event) -> String {
	format!("{event:?}")
}

/// Two values: a generic record, which exports at each instance it infers. Its second
/// parameter defaults to its first, as an instance's name leaves it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pair<L, R = L> {
	pub left: L,
	pub right: R,
}

impl<L: Copy, R: Copy> Pair<L, R> {
	/// The pair swapped: another instance, unless both are of one type.
	pub fn swap(&self) -> Pair<R, L> {
		Pair {
			left: self.right,
			right: self.left,
		}
	}

	/// Whether the sides are equal: a method of only the instances whose sides compare.
	pub fn even(&self) -> bool
	where
		L: PartialEq<R>,
	{
		self.left == self.right
	}
}

/// An impl of one instance, whose methods no other instance has, and which infers it.
impl Pair<u32> {
	pub fn sum(&self) -> u32 {
		self.left + self.right
	}

	/// The left value as a byte, beside `right`: the one instance that names the result
	/// fills the type parameter, with `Size`, and names this instance.
	pub fn beside<T: Copy>(&self, right: T) -> Pair<u8, T> {
		Pair {
			left: self.left as u8,
			right,
		}
	}
}

/// A size tagged with a byte: a signature that names an instance, which infers it, and
/// its `swap` the other way around.
pub fn tag(size: Size, tag: u8) -> Pair<Size, u8> {
	Pair { left: size, right: tag }
}

/// A stack: a generic object, at its default of objects, whose handles move in and out.
#[derive(Debug, Default)]
pub struct Stack<T = Counter> {
	items: Vec<T>,
}

impl<T> Stack<T> {
	pub fn new() -> Self {
		Self { items: Vec::new() }
	}

	pub fn push(&mut self, item: T) {
		self.items.push(item);
	}

	pub fn pop(&mut self) -> Option<T> {
		self.items.pop()
	}

	pub fn depth(&self) -> usize {
		self.items.len()
	}

	/// The depth past `from`'s seed: a generic type's generic method.
	pub fn depth_past(&self, from: impl Seed<u64>) -> u64 {
		self.items.len() as u64 + from.seed()
	}
}

/// What a [`Counter`] can start from: generic, so no foreign implementation takes it, and a
/// parameter it bounds exports at each type the exports implement it for, by reference,
/// which also implements it, so C keeps its handle.
pub trait Seed<T> {
	fn seed(&self) -> T;
}

/// A weight, implemented for `Point` and for `&Point` apart, which both choose `&Point`:
/// one instance of a parameter it bounds.
impl Seed<u32> for Point {
	fn seed(&self) -> u32 {
		self.x.unsigned_abs() + self.y.unsigned_abs()
	}
}

impl Seed<u32> for &Point {
	fn seed(&self) -> u32 {
		Seed::<u32>::seed(*self)
	}
}

/// Two generic records whose instances name each other, as `Left<Right<u8>>` and
/// `Right<Left<u8>>` do: each instance is named without the other's name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Left<T> {
	pub value: T,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Right<T> {
	pub value: T,
}

/// The sum of what each holds inside.
pub fn cross(left: Left<Right<u8>>, right: Right<Left<u8>>) -> u8 {
	left.value.value + right.value.value
}

/// A handle made only by an impl of an instance holding another, which nothing else
/// names: inferring the outer instance infers the inner one.
pub struct Outer<T>(T);

impl Outer<Inner<u8>> {
	pub fn new(value: u8) -> Self {
		Self(Inner { value })
	}

	pub fn get(&self) -> u8 {
		self.0.value
	}
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inner<T> {
	pub value: T,
}

/// `from`'s seed, which outlives the call, so a reference to a value C passes cannot be
/// it: each instance takes its value.
pub fn kept(from: impl Seed<u64> + 'static) -> u64 {
	from.seed()
}

/// Outlives what it is called with, as its supertrait says, and consumes itself, so no
/// callback implements it.
pub trait Kept: 'static {
	fn kept(self) -> i32;
}

impl Kept for Point {
	fn kept(self) -> i32 {
		self.x - self.y
	}
}

impl Kept for &'static Point {
	fn kept(self) -> i32 {
		self.y - self.x
	}
}

/// As `kept`, with the bound that outlives the call implied by `Kept`: it takes a value.
pub fn keep(kept: impl Kept) -> i32 {
	kept.kept()
}

/// `kept`'s, or 0 without one: `K` exports at each type implementing `Kept`, and the
/// parameter naming it at the option of each.
pub fn kept_or<K: Kept>(kept: impl Into<Option<K>>) -> i32 {
	kept.into().map_or(0, Kept::kept)
}

/// Implemented for what `Kept` is, as moq's `Boxable` is for each `Session`: a blanket impl,
/// which C cannot implement apart, so a parameter it bounds exports at each type implementing
/// what the impl requires.
pub trait Keeper: Kept + Copy {}

impl<K: Kept + Copy> Keeper for K {}

/// As `keep`, through the blanket impl.
pub fn keeper(keeper: impl Keeper) -> i32 {
	keeper.kept()
}

/// A generic record only a constant names, which infers its instance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Boxed<T> {
	pub value: T,
}

pub const BOXED: Boxed<u16> = Boxed { value: 42 };

/// Whether `log` kept a line, boxed: an instance only a function taking a foreign
/// implementation names.
pub fn keeps(log: impl Log) -> Boxed<u8> {
	Boxed {
		value: u8::from(log.log(2, "keeps")),
	}
}

/// `from`'s weight.
pub fn weigh(from: impl Seed<u32>) -> u32 {
	from.seed()
}

impl<S: Seed<u64>> Seed<u64> for &S {
	fn seed(&self) -> u64 {
		(**self).seed()
	}
}

impl Seed<u64> for Counter {
	fn seed(&self) -> u64 {
		self.value
	}
}

impl Seed<u64> for Point {
	fn seed(&self) -> u64 {
		u64::from(self.x.unsigned_abs()) + u64::from(self.y.unsigned_abs())
	}
}

/// A point scales an area of dep-fixture's by its distance from the origin along each axis.
impl dep::Factor<u32> for Point {
	fn factor(&self) -> u32 {
		self.x.unsigned_abs() + self.y.unsigned_abs()
	}
}

impl Seed<u64> for Size {
	fn seed(&self) -> u64 {
		u64::from(self.width) * u64::from(self.height)
	}
}

/// Each side of `pair` through `f`, summed: a function inferred at each instance of
/// `Pair` whose sides are alike, taking a lent closure of the inferred type.
pub fn apply<T: Copy, F: Fn(T) -> u32>(pair: Pair<T>, f: F) -> u32 {
	f(pair.left) + f(pair.right)
}

/// Each side of `pair` to `log`, returning how many it kept: inferred as `apply` is,
/// taking a foreign implementation.
pub fn report<T: Copy + fmt::Debug, L: Log>(pair: Pair<T>, log: L) -> u32 {
	[pair.left, pair.right]
		.iter()
		.filter(|side| log.log(1, &format!("{side:?}")))
		.count() as u32
}

/// What a log kept, made from any log.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LogCount {
	pub kept: u32,
}

/// A count, which C cannot implement as it needs `Copy`: a parameter it bounds exports at
/// each type implementing it.
pub trait Tallied: Copy {
	fn tally(self) -> u32;
}

impl Tallied for LogCount {
	fn tally(self) -> u32 {
		self.kept
	}
}

impl Tallied for Point {
	fn tally(self) -> u32 {
		self.x.unsigned_abs()
	}
}

impl<L: Log> From<L> for LogCount {
	fn from(log: L) -> Self {
		Self {
			kept: u32::from(log.log(3, "tally")),
		}
	}
}

/// What `log` kept, as each tally that converts from a log: `LogCount`, and not `Point`,
/// which is `Tallied` too but converts from no log.
pub fn tally<T: Tallied + From<L>, L: Log>(log: L) -> u32 {
	T::from(log).tally()
}

/// A wrapped value, which only its impl of `Seed<u64>` at `Wrap<u32>` names: each
/// function taking a seed infers that instance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wrap<T> {
	pub value: T,
}

impl Seed<u64> for Wrap<u32> {
	fn seed(&self) -> u64 {
		u64::from(self.value) * 10
	}
}

/// A second `Point`, whose last segment clashes with the crate root's: an instance named
/// by it takes its module too, and one named by the other keeps the shorter name.
pub mod polar {
	/// A point by its angle in turns and its distance from the origin.
	#[derive(Clone, Copy, Debug, PartialEq)]
	pub struct Point {
		pub turns: u8,
		pub distance: u32,
	}

	impl super::Seed<u64> for Point {
		fn seed(&self) -> u64 {
			u64::from(self.turns) + u64::from(self.distance)
		}
	}

	/// The point `distance` out at turn zero, or none at the origin, which has no angle: a
	/// function named as the type it returns.
	pub fn point(distance: u32) -> Option<Point> {
		(distance > 0).then_some(Point { turns: 0, distance })
	}

	/// A part of a turn, in 256ths.
	#[derive(Clone, Copy, Debug, PartialEq)]
	pub struct Turn {
		pub part: u8,
	}
}

/// A count of whole turns, whose name joins in Pascal case as `polar::Turn`'s does: a
/// language that joins them keeps where the other's path splits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PolarTurn {
	pub whole: u8,
}

/// `whole` and `part` turns together, in 256ths: two types whose paths join alike.
pub fn wind(whole: PolarTurn, part: polar::Turn) -> u32 {
	u32::from(whole.whole) * 256 + u32::from(part.part)
}

/// The area of `size`: a function named as a type it does not return.
pub fn area(size: Size) -> u64 {
	u64::from(size.width) * u64::from(size.height)
}

impl Counter {
	/// A counter named `name` at `from`'s seed: an `impl Trait` argument, at an instance
	/// for each implementation.
	pub fn seeded(name: &str, from: impl Seed<u64>) -> Counter {
		Counter::new(name, from.seed())
	}
}

/// The seed of `T`'s default and `from`'s: a type parameter no argument names, which the
/// call names, beside an `impl Trait` one.
pub fn seeds<T: Seed<u64> + Default>(from: impl Seed<u64>) -> u64 {
	T::default().seed() + from.seed()
}

/// An area grown by `by` on each side: another export's types, across crates.
pub fn grow(area: &Area, by: u32) -> Area {
	let size = area.size();
	Area::new(Size {
		width: size.width + by,
		height: size.height + by,
	})
}

/// A slot whose method `value` only `Slot<u8>` has, so its path alone names `Slot<u32>`'s
/// field and `Slot<u8>`'s method. Not exhaustive, so an object with getters.
#[non_exhaustive]
pub struct Slot<T> {
	pub value: T,
}

impl<T> Slot<T> {
	pub fn new(value: T) -> Self {
		Self { value }
	}
}

impl Slot<u8> {
	/// The value widened and doubled, apart from the field.
	pub fn value(&self) -> u16 {
		u16::from(self.value) * 2
	}
}

/// A wide slot: a signature that names `Slot<u32>`, which infers it.
pub fn wide(value: u32) -> Slot<u32> {
	Slot::new(value)
}

/// A peer: a record of a socket address and its names, which holds a `Vec`.
#[derive(Clone, Debug, PartialEq)]
pub struct Peer {
	pub addr: std::net::SocketAddr,
	pub names: Vec<String>,
}

impl Peer {
	/// Its names, each borrowed.
	pub fn aliases(&self) -> Vec<&str> {
		self.names.iter().map(String::as_str).collect()
	}

	/// The peer at the next port, also named `name`.
	pub fn next(&self, name: &str) -> Peer {
		let mut addr = self.addr;
		addr.set_port(addr.port() + 1);
		let mut names = self.names.clone();
		names.push(name.to_string());
		Peer { addr, names }
	}
}

/// `url`'s host, if it has one.
pub fn host(url: &url::Url) -> Option<String> {
	url.host_str().map(str::to_string)
}

/// `path` resolved against `base`, or why it does not resolve.
pub fn resolve(base: url::Url, path: &str) -> Result<url::Url, String> {
	base.join(path).map_err(|err| err.to_string())
}

/// Each address's port.
pub fn ports(addrs: Vec<std::net::SocketAddr>) -> Vec<u16> {
	addrs.iter().map(std::net::SocketAddr::port).collect()
}

/// `ttl` doubled, saturating, so `Duration::MAX` stays forever.
pub fn doubled_ttl(ttl: Duration) -> Duration {
	ttl.saturating_mul(2)
}

/// `name` in `dir`.
pub fn file(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
	dir.join(name)
}

/// A path whose bytes are not UTF-8, as a Unix path may be.
#[cfg(unix)]
pub fn stray() -> std::path::PathBuf {
	std::path::PathBuf::from(<std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(vec![
		0xff,
	]))
}

/// Lends [`stray`]'s path to `visit`.
#[cfg(unix)]
pub fn visit_stray(visit: impl Fn(&std::path::Path)) {
	visit(&stray());
}

/// The Unicode scalar after `c`, if any.
pub fn next_char(c: char) -> Option<char> {
	char::from_u32(c as u32 + 1)
}

/// `duration` plus `by`, unless past [`Duration::MAX`]: Rust's seconds are a `u64`.
pub fn extended(duration: Duration, by: Duration) -> Option<Duration> {
	duration.checked_add(by)
}

/// Each path's parent, if it has one.
pub fn parents(paths: Vec<std::path::PathBuf>) -> Vec<Option<std::path::PathBuf>> {
	paths
		.iter()
		.map(|path| path.parent().map(std::path::Path::to_path_buf))
		.collect()
}

/// The words of `text`.
pub fn words(text: &str) -> Vec<String> {
	text.split_whitespace().map(str::to_string).collect()
}

/// `words` joined by spaces, or `none` for none.
pub fn sentence(words: Option<Vec<String>>) -> String {
	words.map_or_else(|| "none".to_string(), |words| words.join(" "))
}

/// Appends `value` in decimal and a comma to `buf`, as moq's encoders append to an
/// `impl BufMut`, which takes a `Vec<u8>`: C passes the bytes in, and Rust writes back
/// what the call left in them.
pub fn put_number(value: u32, buf: &mut impl bytes::BufMut) {
	buf.put_slice(format!("{value},").as_bytes());
}

/// `w235` a second later: a parameter named as Kotlin's generated writer of it would be,
/// whose name the writer must take apart from it.
pub fn second_later(w235: Duration) -> Duration {
	w235.saturating_add(Duration::from_secs(1))
}

/// Takes the first byte of `r` as a decimal digit, as moq's decoders take from an
/// `impl Buf`, which takes `Bytes`. It takes the byte even when it fails, and Rust writes
/// back what the call left on either path. Named `r`, as moq's `varint::decode_quic`'s
/// is, the name a binding's reader of what Rust wrote back might take.
pub fn digit(r: &mut impl bytes::Buf) -> Result<u8, Error> {
	match r.has_remaining() {
		false => Err(Error::Underflow { by: 1 }),
		true => match r.get_u8() {
			digit @ b'0'..=b'9' => Ok(digit - b'0'),
			_ => Err(Error::Overflow),
		},
	}
}

/// Appends `byte` to an entry's bytes and counts it, returning the count: a tuple holding
/// bytes, written back beside the value C returns.
pub fn push_counted(entry: &mut (Vec<u8>, u32), byte: u8) -> u32 {
	entry.0.push(byte);
	entry.1 += 1;
	entry.1
}

/// Moves `point` by `by` on each axis: a record, written back.
pub fn nudge(point: &mut Point, by: i32) {
	point.x += by;
	point.y += by;
}

/// A config of a crate the facade re-exports nothing from, behind an `Arc`, as moq-tokio's
/// listener returns rustls's: an opaque handle, which Rust takes back.
pub fn secure(alpn: &str) -> Arc<tls::Config> {
	Arc::new(tls::Config::new(alpn))
}

/// The ALPN of a config [`secure`] made, taken back by value.
pub fn alpn(config: Arc<tls::Config>) -> String {
	config.alpn().to_string()
}

/// [`alpn`], lent.
pub fn alpn_lent(config: &Arc<tls::Config>) -> String {
	config.alpn().to_string()
}

/// A certificate of a crate the facade re-exports nothing from, by value, with no public
/// path there, as moq-tokio's rustls root certificates: an opaque handle, which Rust takes
/// back or borrows.
pub fn certify(der: &[u8]) -> tls::held::Cert {
	tls::held::Cert::new(der)
}

/// The length of `bytes`, through another crate's future that borrows them but resolves
/// to an owned count: a future, not an opaque handle.
pub fn byte_len(bytes: &[u8]) -> tls::held::Pending<'_> {
	tls::held::Pending(bytes)
}

/// The length of a certificate [`certify`] made, lent.
pub fn cert_len(cert: &tls::held::Cert) -> usize {
	cert.der().len()
}

/// The bytes of a certificate [`certify`] made, taken back by value.
pub fn cert_der(cert: tls::held::Cert) -> Vec<u8> {
	cert.der().to_vec()
}

/// The total length of a chain of certificates, each taken back by value.
pub fn chain_len(chain: Vec<tls::held::Cert>) -> usize {
	chain.iter().map(|cert| cert.der().len()).sum()
}

/// Another crate's session with no public path there, which no export returns: left out, since C can never
/// hold one.
pub fn handover(session: tls::held::Handoff) -> u32 {
	session.id
}

/// A certificate's length, or why it is refused: another crate's error, which crosses as
/// its message though `blame` names it as a handle no export returns.
pub fn verify(cert: &tls::held::Cert) -> Result<u32, tls::held::Invalid> {
	match cert.der().len() {
		0 => Err(tls::held::Invalid {
			reason: "empty".to_string(),
		}),
		len => Ok(len as u32),
	}
}

/// `text`'s length, or why it is no certificate: another crate's error borrowing `text`,
/// which crosses as its message, so nothing outlives the call.
pub fn parse_cert(text: &str) -> Result<u32, tls::held::Malformed<'_>> {
	match text.is_empty() {
		true => Err(tls::held::Malformed(text)),
		false => Ok(text.len() as u32),
	}
}

/// `parse_cert` as a future: its error borrows `text`, which the future owns, so the glue
/// makes it its message before the future ends.
pub async fn parse_later(text: &str) -> Result<u32, tls::held::Malformed<'_>> {
	parse_cert(text)
}

/// Another crate's error taken back by value, which no export returns: left out.
pub fn blame(err: tls::held::Invalid) -> String {
	err.to_string()
}

/// A session of `alpn` lasting `seconds`, from a crate the facade re-exports nothing from:
/// its type exports by its path there, and its error crosses as its message.
pub fn resume(alpn: &str, seconds: u32) -> Result<tls::Session, tls::Error> {
	tls::resume(alpn, seconds)
}

/// `text`'s length, or 0 for none: an optional borrow, lent as the borrow alone is.
pub fn either(text: Option<&str>) -> usize {
	text.map_or(0, str::len)
}

/// [`either`], once [`yielded`] finishes: a future owning what it lends.
pub async fn either_later(text: Option<&str>) -> usize {
	yielded(0).await;
	either(text)
}

/// `payload` after `prefix`, if any.
pub fn prefixed(payload: &[u8], prefix: Option<&[u8]>) -> Vec<u8> {
	[prefix.unwrap_or_default(), payload].concat()
}

/// The sum of `a`, `b`, and `c`, if any: a list owned, then lent as a slice and as itself.
pub fn sum_lists(a: Vec<i32>, b: Option<&[i32]>, c: Option<&Vec<i32>>) -> i32 {
	a.iter()
		.chain(b.unwrap_or_default())
		.chain(c.into_iter().flatten())
		.sum()
}

/// `point`'s coordinates summed, or 0 for none: an optional value, lent shared.
pub fn coordinates(point: Option<&Point>) -> i32 {
	point.map_or(0, |point| point.x + point.y)
}

/// `bytes` in chunks of up to `size`, which is not 0.
pub fn chunks(bytes: &[u8], size: usize) -> Vec<Vec<u8>> {
	bytes.chunks(size).map(<[u8]>::to_vec).collect()
}

/// A counter for each name, at 1.
pub fn counters(names: Vec<String>) -> Vec<Counter> {
	names.iter().map(|name| Counter::new(name, 1)).collect()
}

/// The counters' values summed, none counting 0: each moves in, through an `Option` or a
/// `Box`.
pub fn worth(some: Option<Vec<Counter>>, boxed: Box<Vec<Counter>>) -> u64 {
	some.into_iter().flatten().chain(*boxed).map(|c| c.get()).sum()
}

/// The counter's value, unless none: an object's option in a tuple of one.
pub fn held_count(held: Option<(Counter,)>) -> Option<u64> {
	held.map(|(counter,)| counter.get())
}

/// A counter at 1 named `name`, unless none: options behind a `Box`, of text in and of an
/// object out.
pub fn boxed_counter(name: Box<Option<String>>) -> Box<Option<Counter>> {
	Box::new(name.map(|name| Counter::new(&name, 1)))
}

/// The counter's name, unless none: options behind a `Box`, of an object in and of text out.
pub fn boxed_name(boxed: Box<Option<Counter>>) -> Box<Option<String>> {
	Box::new(boxed.map(|counter| counter.name))
}

/// `value` less `_value`, wrapping: two parameters whose names differ only in an
/// underscore.
pub fn less(value: u32, _value: u32) -> u32 {
	value.wrapping_sub(_value)
}

/// Each row's sum.
pub fn sums(rows: Vec<Vec<u32>>) -> Vec<u32> {
	rows.iter().map(|row| row.iter().sum()).collect()
}

/// Each set's least item, from sets, which no block holds, nested as `sums`' rows are.
pub fn least(sets: Vec<std::collections::BTreeSet<u32>>) -> Vec<std::collections::BTreeSet<u32>> {
	sets.into_iter().map(|set| set.into_iter().take(1).collect()).collect()
}

/// `items` without its last, beside that last, or 0 when empty: a tuple holding a block.
pub fn split_last(mut items: Vec<i32>) -> (Vec<i32>, u32) {
	let last = items.pop().unwrap_or(0);
	(items, last.unsigned_abs())
}

/// A record whose field takes the name a language's `equals` gives its argument.
#[derive(Clone, Debug, PartialEq)]
pub struct Shadowed {
	pub other: Vec<i32>,
	pub a: u8,
	pub b: u8,
}

/// A list of each fixed-width integer and float type, each crossing as one block.
#[derive(Clone, Debug, PartialEq)]
pub struct Numbers {
	pub u16s: Vec<u16>,
	pub u32s: Box<[u32]>,
	pub u64s: Vec<u64>,
	pub i8s: Vec<i8>,
	pub i16s: Vec<i16>,
	pub i32s: Vec<i32>,
	pub i64s: Vec<i64>,
	pub f32s: Vec<f32>,
	pub f64s: Vec<f64>,
	pub some: Option<Vec<i32>>,
}

impl Numbers {
	/// Each type's edges: its least, 0, 1, and its greatest; for floats, also -0.0, the
	/// least subnormal, the infinities, and a NaN with a payload.
	pub fn edges() -> Self {
		Self {
			u16s: vec![0, 1, u16::MAX],
			u32s: Box::new([0, 1, u32::MAX]),
			u64s: vec![0, 1, u64::MAX],
			i8s: vec![i8::MIN, -1, 0, 1, i8::MAX],
			i16s: vec![i16::MIN, -1, 0, 1, i16::MAX],
			i32s: vec![i32::MIN, -1, 0, 1, i32::MAX],
			i64s: vec![i64::MIN, -1, 0, 1, i64::MAX],
			f32s: vec![
				f32::MIN,
				-0.0,
				f32::from_bits(1),
				f32::MAX,
				f32::NEG_INFINITY,
				f32::INFINITY,
				f32::from_bits(0x7fc0_0001),
			],
			f64s: vec![
				f64::MIN,
				-0.0,
				f64::from_bits(1),
				f64::MAX,
				f64::NEG_INFINITY,
				f64::INFINITY,
				f64::from_bits(0x7ff8_0000_0000_0001),
			],
			some: Some(Vec::new()),
		}
	}

	/// Only the float lists hold anything: one zero each, negative when `negative` is.
	pub fn zero(negative: bool) -> Self {
		let zero = if negative { -0.0 } else { 0.0 };
		Self {
			u16s: Vec::new(),
			u32s: Box::new([]),
			u64s: Vec::new(),
			i8s: Vec::new(),
			i16s: Vec::new(),
			i32s: Vec::new(),
			i64s: Vec::new(),
			f32s: vec![zero as f32],
			f64s: vec![zero],
			some: None,
		}
	}

	/// Each list reversed, and `some` too where present.
	pub fn reversed(mut self) -> Self {
		self.u16s.reverse();
		self.u32s.reverse();
		self.u64s.reverse();
		self.i8s.reverse();
		self.i16s.reverse();
		self.i32s.reverse();
		self.i64s.reverse();
		self.f32s.reverse();
		self.f64s.reverse();
		if let Some(some) = &mut self.some {
			some.reverse();
		}
		self
	}
}

/// A float beside a record of float lists, both compared as Rust's `==` compares floats.
#[derive(Clone, Debug, PartialEq)]
pub struct Measure {
	pub value: f64,
	pub numbers: Numbers,
}

impl Measure {
	/// `value` beside [`Numbers::zero`] of `negative`.
	pub fn new(value: f64, negative: bool) -> Self {
		Self {
			value,
			numbers: Numbers::zero(negative),
		}
	}
}

/// The sum of `items`, borrowed in place.
pub fn sum_u32(items: &[u32]) -> u64 {
	items.iter().map(|&v| u64::from(v)).sum()
}

/// `items` doubled, wrapping, from a borrowed slice.
pub fn doubled_i64(items: &[i64]) -> Vec<i64> {
	items.iter().map(|v| v.wrapping_mul(2)).collect()
}

/// `items` back as Rust took them, boxed.
pub fn boxed_floats(items: Box<[f64]>) -> Box<[f64]> {
	items
}

/// The points added up, from anything that iterates them.
pub fn summed(points: impl IntoIterator<Item = Point>) -> Point {
	points.into_iter().fold(Point::default(), |sum, p| sum.offset(p.x, p.y))
}

/// Bounds on a measure, as moq's `health::Limits<Duration>` is: a generic record whose
/// instances are of types that map by role.
#[derive(Clone, Debug, PartialEq)]
pub struct Limits<T> {
	pub low: T,
	pub high: T,
}

/// How far apart `limits` are: an instance at `Duration`, passed in.
pub fn breadth(limits: Limits<Duration>) -> Duration {
	limits.high.saturating_sub(limits.low)
}

/// `low` and `high` as limits: an instance at `Bytes`, returned.
pub fn byte_limits(low: Bytes, high: Bytes) -> Limits<Bytes> {
	Limits { low, high }
}

/// The parts joined, from anything that lends them for the call: a slice C lends.
pub fn joined<'a, I: IntoIterator<Item = &'a Bytes>>(parts: I) -> Bytes {
	parts.into_iter().flat_map(|part| part.iter().copied()).collect()
}

/// The first of `items`, a borrow of what C lends that is copied out before the call ends.
pub fn first_of<'a, I: IntoIterator<Item = &'a String>>(items: I) -> Option<&'a String> {
	items.into_iter().next()
}

/// How many bytes `reader` holds: bytes C lends for the call.
pub fn read_len(mut reader: impl std::io::Read) -> u64 {
	std::io::copy(&mut reader, &mut std::io::sink()).expect("a slice reads")
}

/// The length of the longest word, naming its iterator.
pub fn longest<I>(words: I) -> usize
where
	I: IntoIterator<Item = String>,
{
	words.into_iter().map(|word| word.len()).max().unwrap_or(0)
}

/// `value` as compact JSON text: a serde bound takes a JSON document.
pub fn encode<T: serde::Serialize>(value: &T) -> String {
	serde_json::to_string(value).expect("a JSON document serializes")
}

/// `doc` under `tag`, as a new document.
pub fn tagged(doc: serde_json::Value, tag: &str) -> serde_json::Value {
	serde_json::json!({ "tag": tag, "doc": doc })
}

/// `entries` with `tag` added, as hang's `extra` fields hold a JSON object's entries.
pub fn entries(
	mut entries: serde_json::Map<String, serde_json::Value>,
	tag: &str,
) -> serde_json::Map<String, serde_json::Value> {
	entries.insert("tag".to_string(), tag.into());
	entries
}

/// The entries of a JSON object, taken in key order, as serde_json's iterator yields them.
pub fn sections(entries: serde_json::Map<String, serde_json::Value>) -> serde_json::map::IntoIter {
	entries.into_iter()
}

/// The time now, on std's clock: only Rust makes an instant.
pub fn now() -> std::time::Instant {
	std::time::Instant::now()
}

/// The instant `by` after `at`.
pub fn later(at: std::time::Instant, by: Duration) -> std::time::Instant {
	at + by
}

/// The time from `earlier` to `later`, or zero if `later` is earlier.
pub fn since(later: std::time::Instant, earlier: std::time::Instant) -> Duration {
	later.saturating_duration_since(earlier)
}

/// Whether `at` holds an instant, lent inside an `Option`: an instant crosses as a `Copy`
/// object, so lending one inside a value takes no handle.
pub fn has_instant(at: &Option<std::time::Instant>) -> bool {
	at.is_some()
}

/// How much to log, as moq-tokio's `Log` holds it: a field of a foreign enum, and its filter.
#[non_exhaustive]
pub struct Verbosity {
	pub level: tracing::Level,
}

impl Verbosity {
	pub fn new(level: tracing::Level) -> Self {
		Self { level }
	}

	/// Its level as a filter, which is never `OFF`.
	pub fn filter(&self) -> tracing::level_filters::LevelFilter {
		tracing::level_filters::LevelFilter::from_level(self.level)
	}
}

/// The filter one level more verbose than `filter`: `OFF` lets errors through, and
/// `TRACE` stays.
pub fn louder(filter: tracing::level_filters::LevelFilter) -> tracing::level_filters::LevelFilter {
	use tracing::{Level, level_filters::LevelFilter};
	match filter.into_level() {
		None => LevelFilter::ERROR,
		Some(Level::ERROR) => LevelFilter::WARN,
		Some(Level::WARN) => LevelFilter::INFO,
		Some(Level::INFO) => LevelFilter::DEBUG,
		Some(_) => LevelFilter::TRACE,
	}
}

/// `filter` as tracing prints it: a level's name in lower case, or `off`.
pub fn spell(filter: tracing::level_filters::LevelFilter) -> String {
	filter.to_string()
}

/// A site: every field is `pub`, but a URL is a handle C cannot copy, so it is an object
/// whose getter returns a clone.
pub struct Site {
	pub home: url::Url,
}

impl Site {
	pub fn new(home: url::Url) -> Self {
		Self { home }
	}
}

impl Shape {
	/// Every shape, as a constant slice.
	pub const ALL: &'static [Shape] = &[Shape::Circle, Shape::Square];
}

/// Greetings, as a constant slice of references, each returned as a clone.
pub const GREETINGS: &[&str] = &["hi", "hello"];

/// A trail through points, each labeled: slices passed in and lent back out.
pub struct Trail {
	points: Vec<Point>,
	labels: Vec<Label>,
	route: String,
}

impl Trail {
	/// A trail through `points`, named by `names`, which it copies.
	pub fn new(points: &[Point], names: &[String]) -> Self {
		let labels = (points.iter().zip(names))
			.map(|(point, name)| Label {
				name: name.clone(),
				at: Some(*point),
				ttl: None,
			})
			.collect();
		Self {
			points: points.to_vec(),
			labels,
			route: names.join("/"),
		}
	}

	/// Its points.
	pub fn points(&self) -> &[Point] {
		&self.points
	}

	/// A label per point, each owning its name.
	pub fn labels(&self) -> &[Label] {
		&self.labels
	}

	/// Its points one by one, as moq's `Hops::iter` returns them: an array of copies.
	pub fn iter(&self) -> std::slice::Iter<'_, Point> {
		self.points.iter()
	}

	/// Each name, split from its route, through an iterator only its bounds name.
	pub fn names(&self) -> impl Iterator<Item = &str> {
		// Splitting the empty route would yield one empty name.
		(!self.route.is_empty())
			.then(|| self.route.split('/'))
			.into_iter()
			.flatten()
	}

	/// Its points past `x`, as hang's `enabled` filters its renditions: the predicate runs
	/// on every point at the call.
	pub fn beyond(&self, x: i32) -> impl Iterator<Item = &Point> {
		self.points.iter().filter(move |p| p.x > x)
	}

	/// The bytes of its route, which cross as a buffer.
	pub fn bytes(&self) -> std::str::Bytes<'_> {
		self.route.bytes()
	}

	/// Its labels, drained: every one is taken at the call, so none are left.
	pub fn drain(&mut self) -> std::vec::Drain<'_, Label> {
		self.labels.drain(..)
	}
}

/// A counter for each name, counting from its index, last first.
pub fn numbered(names: Vec<String>) -> impl Iterator<Item = Counter> {
	let counters: Vec<_> = (names.iter().enumerate())
		.map(|(i, name)| Counter::new(name, i as u64))
		.collect();
	counters.into_iter().rev()
}

/// Each address at the next port, one IPv4 and one IPv6.
pub fn nexts(
	v4: std::net::SocketAddrV4,
	mut v6: std::net::SocketAddrV6,
) -> (std::net::SocketAddrV4, std::net::SocketAddrV6) {
	v6.set_port(v6.port() + 1);
	(std::net::SocketAddrV4::new(*v4.ip(), v4.port() + 1), v6)
}

/// Why [`refuse`] refused, which Rust returns only boxed.
#[derive(Debug)]
pub struct Refusal {
	reason: String,
}

impl fmt::Display for Refusal {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "refused: {}", self.reason)
	}
}

/// Each counter's name with its refusal, `last`'s too if any: an object Swift holds as a
/// struct beside one it holds as a class, moved in inside a list and an option.
pub fn reasons(pairs: Vec<(Counter, Refusal)>, last: Option<(Counter, Refusal)>) -> String {
	let pairs = pairs.iter().chain(&last);
	pairs
		.map(|(counter, refusal)| format!("{} {refusal}", counter.name))
		.collect::<Vec<_>>()
		.join(", ")
}

/// Always refuses, for `reason`.
pub fn refuse(reason: &str) -> Result<u32, Box<Refusal>> {
	Err(Box::new(Refusal {
		reason: reason.to_string(),
	}))
}

/// The kind of target the library was built for. One binding serves the host and wasm32,
/// so it has every target's variants, and a target refuses one it lacks. As wasm32's comes
/// first, `Native`'s Rust discriminant differs between the two, which its C tag does not.
pub enum Platform {
	/// wasm32, which only that target has.
	#[cfg(target_arch = "wasm32")]
	Wasm { bits: u32 },
	/// A native target, with its pointers' width in bits.
	Native(u32),
}

/// The library's target's platform.
pub fn platform() -> Platform {
	#[cfg(target_arch = "wasm32")]
	return Platform::Wasm { bits: usize::BITS };
	#[cfg(not(target_arch = "wasm32"))]
	Platform::Native(usize::BITS)
}

/// The width of `platform`'s pointers, in bits.
pub fn pointer_bits(platform: Platform) -> u32 {
	match platform {
		Platform::Native(bits) => bits,
		#[cfg(target_arch = "wasm32")]
		Platform::Wasm { bits } => bits,
	}
}

/// The family of the library's target, a plain enum with a variant only wasm32 has.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Family {
	Native,
	#[cfg(target_arch = "wasm32")]
	Wasm,
}

/// Whether `family` is the library's target's.
pub fn is_family(family: Family) -> bool {
	#[cfg(target_arch = "wasm32")]
	return family == Family::Wasm;
	#[cfg(not(target_arch = "wasm32"))]
	{
		family == Family::Native
	}
}

/// Only wasm32 has it: a call on another target fails.
#[cfg(target_arch = "wasm32")]
pub fn wasm_only() -> u32 {
	32
}

/// Only wasm32 has it, and it consumes `counter`: a refused call leaves it open.
#[cfg(target_arch = "wasm32")]
pub fn wasm_take(counter: Counter) -> Result<u64, Error> {
	Ok(counter.get())
}

/// Only wasm32 has it: a call on another target fails before it takes `log`.
#[cfg(target_arch = "wasm32")]
pub fn wasm_log(log: Box<dyn Log>) {
	log.log(0, "wasm");
}

/// A type only wasm32 has.
#[cfg(target_arch = "wasm32")]
pub struct Worker {
	id: u32,
}

#[cfg(target_arch = "wasm32")]
impl Worker {
	/// A worker numbered `id`.
	pub fn new(id: u32) -> Self {
		Self { id }
	}

	/// Its number.
	pub fn id(&self) -> u32 {
		self.id
	}
}

/// A record only wasm32 has.
#[cfg(target_arch = "wasm32")]
pub struct Page {
	pub size: u32,
}

/// wasm32's page.
#[cfg(target_arch = "wasm32")]
pub fn page() -> Page {
	Page { size: 65536 }
}

/// A callback only wasm32 has.
#[cfg(target_arch = "wasm32")]
pub trait Beat: Send + Sync {
	fn beat(&self) -> u32;
}

/// What `beat` counts.
#[cfg(target_arch = "wasm32")]
pub fn beats(beat: Box<dyn Beat>) -> u32 {
	beat.beat()
}

/// A cancellation at a step, named as a class Dart's runtime declares: Dart's name takes a
/// trailing `_`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cancelled {
	pub step: u8,
}

/// Items whose names clash in some language, each kept apart there and only there.
/// A tone's absence, whose variant Go names as `clash::audible`'s error.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VecClashTone {
	Error,
}

/// An offset, named as Kotlin's interface for a reader seeking past `Long.MAX_VALUE`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StartSeekable {
	pub at: u64,
}

/// Whether two floats are equal as `1`, named as the helper Kotlin's records compare through.
pub fn same(a: f64, b: f64) -> i32 {
	i32::from(a == b)
}

/// A count of doubles, named as Kotlin's primitive array of them: Kotlin's name takes a
/// trailing `_`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DoubleArray {
	pub len: u32,
}

pub mod clash {
	/// A pitch in hertz, whose name Pascal case cannot tell from `TONE`'s.
	#[derive(Clone, Copy, Debug, PartialEq)]
	pub struct Tone {
		pub hz: u16,
	}

	/// A loudness in decibels, named as `Tone` but for case.
	#[allow(non_camel_case_types)]
	#[derive(Clone, Copy, Debug, PartialEq)]
	pub struct TONE {
		pub db: u8,
	}

	/// A span `width` wide: its method `width`, which Swift, Go, and Dart cannot name as the
	/// field, and its own `to_string`, which C cannot name as `Display`'s.
	#[derive(Clone, Copy, Debug, PartialEq)]
	pub struct Span {
		pub width: u32,
	}

	impl Span {
		/// Twice the width.
		pub fn width(&self) -> u32 {
			self.width * 2
		}

		/// The width in words.
		#[allow(clippy::inherent_to_string_shadow_display)]
		pub fn to_string(&self) -> String {
			format!("{} wide", self.width)
		}
	}

	impl std::fmt::Display for Span {
		fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			write!(f, "{}", self.width)
		}
	}

	/// A count, whose methods are named as what C and some languages give every handle.
	pub struct Store {
		count: u32,
	}

	impl Store {
		pub fn new(count: u32) -> Store {
			Store { count }
		}

		/// The count, named as the C function freeing the handle.
		pub fn free(&self) -> u32 {
			self.count
		}

		/// The count plus one, named as the method closing the handle.
		pub fn close(&self) -> u32 {
			self.count + 1
		}
	}

	pub mod audio {
		/// One, named in C as `audio_encoder::finish` is but for where the paths split.
		pub fn encoder_finish() -> u8 {
			1
		}
	}

	pub mod audio_encoder {
		/// Two.
		pub fn finish() -> u8 {
			2
		}
	}

	/// The tone, or no tones where it is silent: an error Go names as `VecClashTone`'s variant.
	pub fn audible(tone: Tone) -> Result<Tone, Vec<Tone>> {
		if tone.hz == 0 { Err(Vec::new()) } else { Ok(tone) }
	}

	/// The sum of both tuples, whose C names run their elements together alike.
	pub fn nested(a: Option<((u8, u16), u32)>, b: Option<((u8, u16, u32),)>) -> u32 {
		let a = a.map_or(0, |((x, y), z)| u32::from(x) + u32::from(y) + z);
		let b = b.map_or(0, |((x, y, z),)| u32::from(x) + u32::from(y) + z);
		a + b
	}
}

/// Changes both coordinates before returning mixed values or an error.
pub fn return_values(
	point: &mut Point,
	fail: bool,
) -> Result<(String, Option<(u8, i16, u32, u64)>, Vec<Vec<u8>>), Error> {
	point.x += 1;
	point.y -= 1;
	if fail {
		return Err(Error::Overflow);
	}
	Ok((
		"hé🙂\0".to_string(),
		Some(widths(u64::MAX)),
		vec![vec![0, 128, 255], (0..=255).cycle().take(10000).collect()],
	))
}

/// Moves the counter into the success value, or drops it on failure.
pub fn return_moved(counter: Counter, fail: bool) -> Result<Counter, Error> {
	if fail { Err(Error::Overflow) } else { Ok(counter) }
}

/// One `NonZero` of each integer, each crossing as its integer.
#[derive(Clone, Copy, Debug)]
pub struct NonZeros {
	pub u8: NonZero<u8>,
	pub u16: NonZero<u16>,
	pub u32: NonZero<u32>,
	pub u64: NonZero<u64>,
	pub usize: NonZero<usize>,
	pub i8: NonZero<i8>,
	pub i16: NonZero<i16>,
	pub i32: NonZero<i32>,
	pub i64: NonZero<i64>,
	pub isize: NonZero<isize>,
}

/// Each integer's extreme `NonZero`: its max, or a signed one's min.
pub fn non_zeros() -> NonZeros {
	NonZeros {
		u8: NonZero::<u8>::MAX,
		u16: NonZero::<u16>::MAX,
		u32: NonZero::<u32>::MAX,
		u64: NonZero::<u64>::MAX,
		usize: NonZero::<usize>::MAX,
		i8: NonZero::<i8>::MIN,
		i16: NonZero::<i16>::MIN,
		i32: NonZero::<i32>::MIN,
		i64: NonZero::<i64>::MIN,
		isize: NonZero::<isize>::MIN,
	}
}

/// `non_zeros` as Rust's `Debug` spells it, once it crossed back.
pub fn describe_non_zeros(non_zeros: NonZeros) -> String {
	format!("{non_zeros:?}")
}

/// The hop after `hop`, or `None` past the last, as moq's `Client::with_peer_hop` counts.
pub fn next_hop(hop: Option<NonZero<u64>>) -> Option<NonZero<u64>> {
	hop?.checked_add(1)
}

/// The farther of two hops, boxed: a `NonZero` as a generic instance's argument and
/// behind a `Box`.
pub fn farther_hop(hops: Pair<NonZero<u16>>) -> Box<NonZero<u16>> {
	Box::new(hops.left.max(hops.right))
}

/// Three optional borrowed counters, past what Swift takes, so the config leaves it out.
pub fn among(first: Option<&Counter>, second: Option<&Counter>, third: Option<&Counter>) -> u64 {
	[first, second, third].into_iter().flatten().map(|c| c.value).sum()
}

/// A record named as C++'s `==` names its first argument, which must not hide it.
#[allow(non_camel_case_types)]
#[derive(Clone, Debug, PartialEq)]
pub struct a {
	pub b: u8,
}

/// An enum named as C++'s `==` names its first argument.
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum b {
	One,
	Two(u8),
}

/// A handle named as C++'s `==` names its first argument.
#[allow(non_camel_case_types)]
#[derive(Debug, PartialEq)]
pub struct c {
	value: u8,
}

impl c {
	/// A handle holding `value`.
	pub fn new(value: u8) -> Self {
		c { value }
	}
}

/// An enum whose variants are named as what a Kotlin variant class's `equals` and
/// `hashCode` name, which must not take the variants for them.
#[derive(Clone, PartialEq)]
pub enum Builtin {
	Any(Vec<i32>),
	Boolean(f64),
	Int(f64),
	Native(f64),
}

/// A record whose fields are named as the package and as `kotlin`, which Kotlin's
/// generated `equals` and `hashCode` must not take for those.
#[derive(Clone, PartialEq)]
pub struct Qualifiers {
	pub api: f64,
	pub kotlin: f64,
}

/// An object named as a class of Kotlin's runtime once was, which keeps its name now the
/// runtime's take a leading `__`.
pub struct Handle {
	count: u32,
}

impl Handle {
	pub fn new(count: u32) -> Self {
		Handle { count }
	}

	pub fn count(&self) -> u32 {
		self.count
	}

	pub async fn later(count: u32) -> Self {
		Handle { count }
	}
}
