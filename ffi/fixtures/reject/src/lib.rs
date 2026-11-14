//! Items rust2-ffi cannot export: each is a compile error at its span.
#![feature(pattern)]
use std::collections::HashMap;

/// A tuple of references taken: C lends nothing inside a value.
pub fn pair(pair: (&str, u8)) -> u8 {
	pair.1
}

/// Another crate's type, which has no mapping.
pub fn map() -> HashMap<u8, u8> {
	HashMap::new()
}

/// A type parameter no rule picks a type for.
pub fn generic<T: Copy>(value: T) -> T {
	value
}

/// An option inside an option, through a `Box`: C has one `None` per option.
pub fn nested(value: Option<Box<Option<u32>>>) -> bool {
	value.is_some()
}

/// An object, since its field is private.
pub struct Opaque(u8);

/// A handle lent inside a `Box`: it would cross by value, moving it out of the caller's
/// handle for a call that only reads it.
pub fn peek(value: &Box<Opaque>) -> u8 {
	value.0
}

/// A static.
pub static COUNT: u8 = 0;

/// A macro.
#[macro_export]
macro_rules! noop {
	() => {};
}

/// Left out by the config, so it refuses nothing itself, but a signature naming it is
/// refused.
pub struct Excluded;

pub fn excluded() -> Excluded {
	Excluded
}

/// A range of what is not an integer.
pub fn letters(range: std::ops::Range<char>) -> usize {
	range.count()
}

/// A text that borrows: only its `'static` form crosses, held by a handle.
pub struct Text<'a>(&'a str);

impl<'a> Text<'a> {
	/// Its `'static` form would borrow what C lends for the call past it.
	pub fn new(text: &'a str) -> Self {
		Self(text)
	}

	/// A text that borrows its handle, which a new handle would outlive.
	pub fn view(&self) -> Text<'_> {
		Text(self.0)
	}
}

/// A borrow `text` could keep, since it lives as long as what it borrows.
pub fn retext<'a>(text: &mut Text<'a>, with: &'a str) {
	text.0 = with;
}

/// A borrowed return that is not `Clone`, so it has no copy to return.
pub fn first(opaque: &Opaque) -> Option<&Opaque> {
	Some(opaque)
}

impl Opaque {
	pub fn new() -> Self {
		Self(0)
	}
}

impl<'a> Text<'a> {
	pub fn empty() -> Self {
		Self("")
	}

	/// `'b` outlives the handle's `'a`, so it is `'static` too, which a string is once
	/// interned.
	pub fn replace<'b: 'a>(&mut self, text: &'b str) {
		self.0 = text;
	}

	/// What a future resolves to borrows its handle, as `view` does.
	pub async fn later(&self) -> Text<'_> {
		Text(self.0)
	}
}

/// Bytes lent mutably for as long as the program runs, which interning would alias: every
/// call with equal bytes would share one copy.
pub fn forever_mut(bytes: &'static mut [u8]) -> usize {
	bytes.len()
}

/// A boxed borrow: C's value is lifted into an owned one, lent only as the parameter
/// itself or `Some` of it.
pub fn boxed(text: Box<&str>) -> usize {
	text.len()
}

/// A record, a type of the facade's own.
pub struct Size {
	pub bytes: u64,
}

/// Another crate's error borrowing `text`, the output of a future: the glue awaits the
/// future past the call that owns `text`, so the error's borrow would outlive it.
pub async fn parse_later(text: &str) -> Result<u32, tls::held::Malformed<'_>> {
	match text.is_empty() {
		true => Err(tls::held::Malformed(text)),
		false => Ok(text.len() as u32),
	}
}

/// A waiter of another crate, which a handle holds by value.
pub fn waiter() -> tls::held::Waiter {
	tls::held::Waiter::new(std::task::Waker::noop().clone())
}

/// Not a poll through [`waiter`]'s: the glue cannot name the type a handle holds to make one
/// from the poll's waker.
pub fn poll_ready(_waiter: &tls::held::Waiter) -> std::task::Poll<u32> {
	std::task::Poll::Ready(1)
}

/// An object holding another crate's session, which C holds as a handle.
pub struct Link {
	session: tls::held::Handoff,
}

impl Link {
	pub fn new() -> Self {
		Self {
			session: tls::held::Handoff { id: 1 },
		}
	}

	/// Not lent: a handle holds another crate's value only erased, and by value.
	pub fn session(&self) -> &tls::held::Handoff {
		&self.session
	}

	/// What makes the session a value C holds.
	pub fn take(self) -> tls::held::Handoff {
		self.session
	}
}

impl Default for Link {
	fn default() -> Self {
		Self::new()
	}
}

/// Another crate's type that is not `Send`, which no handle may hold.
pub fn measure(local: tls::held::Local) -> u32 {
	local.get()
}

/// A generic type, which exports at the instances its exports name.
pub struct Cell<T>(T);

/// An instance naming another crate's type that a handle holds by value, erased.
pub fn sealed(cert: tls::held::Cert) -> Cell<tls::held::Cert> {
	Cell(cert)
}

/// A future returned by a function that is not async, whose output borrows what the
/// future holds only until it completes.
pub fn echo(tag: &str) -> impl std::future::Future<Output = &str> + '_ {
	async move { tag }
}

/// As `echo`, but the output borrows only inside an error that crosses as its message.
pub fn blame(tag: &str) -> impl std::future::Future<Output = Result<u32, tls::held::Wrapped<'_, u8>>> + '_ {
	async move { Err(tls::held::Wrapped(tag, 0)) }
}

/// As `echo`, but its borrowed output named only by a supertrait of the bound.
pub fn relay(tag: &str) -> impl tls::held::Borrowing<'_> {
	async move { tag }
}

/// As `echo`, but through a future that is not an `impl Future`.
pub fn resolve(tag: &str) -> tls::held::Lend<'_, u8> {
	tls::held::Lend(tag, std::marker::PhantomData)
}

/// A closure taking a `Copy` value of another crate's, which a handle holds.
pub fn spend(f: impl FnOnce(tls::held::Token) -> u32) -> u32 {
	f(tls::held::Token(1))
}

/// A closure returning one.
pub fn mint(f: impl FnOnce() -> tls::held::Token) -> u32 {
	f().0
}

/// Returns another crate's `Copy` value, which a handle holds.
pub fn token() -> tls::held::Token {
	tls::held::Token(2)
}

/// Borrows a slice of them, whose handles C would have to give up.
pub fn spent(tokens: &[tls::held::Token]) -> u32 {
	tokens.iter().map(|t| t.0).sum()
}

/// Returns another crate's value, which a handle holds.
pub fn issue() -> tls::held::Cert {
	tls::held::Cert::new(b"der")
}

/// Takes what converts into it, which only inference could name in the glue.
pub fn adopt(cert: impl Into<tls::held::Cert>) -> usize {
	cert.into().der().len()
}

impl<T> Cell<T> {
	pub fn new(value: T) -> Self {
		Self(value)
	}
}

impl Cell<u8> {
	/// An instance holding an instance of its own type, which could name ever deeper ones.
	pub fn nest(self) -> Cell<Cell<u8>> {
		Cell(self)
	}
}

/// Two generic types whose methods each wrap the other's instance in their own: inferring
/// from `Ping<u8>` names `Pong<Ping<u8>>`, whose `ping` names `Ping<Pong<Ping<u8>>>`, which
/// holds a `Ping` inside a `Ping` and ends inference there.
pub struct Ping<T>(T);

pub struct Pong<T>(T);

impl<T> Ping<T> {
	pub fn pong(self) -> Pong<Ping<T>> {
		Pong(self)
	}
}

impl<T> Pong<T> {
	pub fn ping(self) -> Ping<Pong<T>> {
		Ping(self)
	}
}

pub fn ping(value: u8) -> Ping<u8> {
	Ping(value)
}

/// A generic type whose method wraps its argument in a tuple: `Grow<(u32,)>` is refused,
/// as a tuple is no type argument, so inference names nothing through its `grow`.
pub struct Grow<T>(T);

impl<T> Grow<T> {
	pub fn grow(self) -> Grow<(T,)> {
		Grow((self.0,))
	}
}

pub fn grow(value: u32) -> Grow<u32> {
	Grow(value)
}

/// A record whose instance at a tuple is refused before its fields map, so `Core<u8>`,
/// which only those fields name, is neither inferred nor needed.
pub struct Shell<T> {
	pub core: Core<u8>,
	pub tag: T,
}

pub struct Core<T> {
	pub value: T,
}

pub fn shell() -> Shell<(u32,)> {
	Shell {
		core: Core { value: 1 },
		tag: (2,),
	}
}

/// A const generic function whose bound names an instance nothing else does: refused at
/// its const parameter, which nothing names, before any candidate for the other is mapped.
pub struct Probe<T>(T);

pub fn probe<const N: usize>(x: impl Into<Probe<u8>>) {
	let _ = x;
}

/// An instance the config excludes.
pub fn excluded_cell(cell: Cell<u16>) -> u16 {
	cell.0
}

/// An instance whose type argument is no value or handle, so it cannot name it.
pub fn listed(cell: Cell<Vec<String>>) -> usize {
	cell.0.len()
}

/// A type whose const argument is a `char`, which only a `bool` or integer may be.
pub struct Fixed<const C: char>(pub u8);

/// An instance of it, which a signature names.
pub fn fixed() -> Fixed<'a'> {
	Fixed(2)
}

/// The standard library's item, whose types the export maps by role instead.
pub use std::collections::HashSet;

/// A borrowed waker: C passes one only by value, which Rust keeps.
pub fn lend(waker: &std::task::Waker) {
	waker.wake_by_ref();
}

/// A field holding a `'static` borrow, which a setter cannot take from C's borrow for
/// the call.
#[non_exhaustive]
pub struct Tag {
	pub label: &'static str,
}

impl Tag {
	pub fn new() -> Self {
		Self { label: "tag" }
	}
}

/// A closure returning bytes, which C hands Rust only as a `String`.
pub fn gather(f: impl Fn() -> Vec<u8>) -> usize {
	f().len()
}

/// A closure failing with an error other than a `String`.
pub fn attempt(f: impl Fn() -> Result<u32, u8>) -> bool {
	f().is_ok()
}

/// A reader taken by value, which C lends only behind `&mut`.
pub fn consume(mut file: impl std::io::Read + std::io::Seek) -> bool {
	file.read(&mut [0]).is_ok()
}

/// A reader kept past the call, as a `'static` bound lets it be.
pub fn retain<R: std::io::Read + std::io::Seek + 'static>(file: &mut R) -> bool {
	file.stream_position().is_ok()
}

/// A reader bound by more than C's reader can meet.
pub fn rewrite<F: std::io::Read + std::io::Seek + std::io::Write>(file: &mut F) -> bool {
	file.flush().is_ok()
}

/// A reader a future holds past the call.
pub async fn read_later(file: &mut (impl std::io::Read + std::io::Seek)) -> bool {
	file.stream_position().is_ok()
}

/// A closure bound by more than C's function can meet.
pub fn twice(f: impl Fn() + Clone) {
	f.clone()();
	f()
}

/// One closure type for two parameters, which C's two functions cannot be.
pub fn both<F: Fn()>(a: F, b: F) {
	a();
	b()
}

/// An object lent only for as long as it borrows, which a handle's `'static` outlives.
pub fn read(f: impl FnOnce(&Text<'_>)) {
	f(&Text(""))
}

/// Words C would lend inside an array, which C lends nothing inside.
pub fn count_words(words: Vec<&str>) -> usize {
	words.len()
}

/// Words C would lend inside an `Option` inside an array.
pub fn count_some(words: Vec<Option<&str>>) -> usize {
	words.iter().flatten().count()
}

/// A slice of references C cannot lend, as a `Vec` of them.
pub fn count_slice(words: &[&str]) -> usize {
	words.len()
}

/// A slice of handles: lifting each would move it out of its caller's handle.
pub fn peek_all(values: &[Opaque]) -> usize {
	values.len()
}

/// A borrowed slice whose elements have no clone to return.
pub fn none_opaque() -> &'static [Opaque] {
	&[]
}

/// A conversion only from what outlives every call, which C cannot lend.
impl From<&'static u32> for Excluded {
	fn from(_: &'static u32) -> Self {
		Excluded
	}
}

/// Takes only a `'static` borrow, through `Excluded`'s `From`.
pub fn stash(value: impl Into<Excluded>) -> u32 {
	let _ = value.into();
	0
}

/// Excluded, borrowing what it converts from.
pub struct Held<'a>(pub &'a u32);

impl<'a> From<&'a u32> for Held<'a> {
	fn from(value: &'a u32) -> Self {
		Held(value)
	}
}

/// Takes a borrow only for as long as `'static`, which the target's lifetime demands.
pub fn hold(value: impl Into<Held<'static>>) -> u32 {
	*value.into().0
}

/// Takes a borrow only for as long as `'static`, which its own lifetime bound demands.
pub fn bound<'a: 'static>(value: impl Into<Held<'a>>) -> u32 {
	*value.into().0
}

/// Takes a borrow only for as long as `'static`, which its parameter's bound demands.
pub fn kept<'a>(value: impl Into<Held<'a>> + 'static) -> u32 {
	*value.into().0
}

/// Excluded, what converts into a `Held<'static>`.
pub struct Holder(pub Held<'static>);

impl<T: Into<Held<'static>>> From<T> for Holder {
	fn from(value: T) -> Self {
		Holder(value.into())
	}
}

/// Takes a borrow only for as long as `'static`, which a level of its conversion demands.
pub fn holder(value: impl Into<Holder>) -> u32 {
	*value.into().0.0
}

/// Excluded, what converts into a `Held<'a>`, at the lifetime the parameter names.
pub struct Lender<'a>(pub Held<'a>);

impl<'a, T: Into<Held<'a>>> From<T> for Lender<'a> {
	fn from(value: T) -> Self {
		Lender(value.into())
	}
}

/// Takes a borrow only for as long as `'static`, which the target ties a level's lifetime to.
pub fn lender(value: impl Into<Lender<'static>>) -> u32 {
	*value.into().0.0
}

/// Excluded, what converts into a `Held<'b>` for a `'b` that outlives its own `'a`.
pub struct Outlived<'a>(pub Held<'a>);

impl<'a, 'b: 'a, T: Into<Held<'b>>> From<T> for Outlived<'a> {
	fn from(value: T) -> Self {
		Outlived(value.into())
	}
}

/// Takes a borrow only for as long as `'static`, which its target's lifetime passes on
/// through a lifetime bound.
pub fn outlived(value: impl Into<Outlived<'static>>) -> u32 {
	*value.into().0.0
}

/// Excluded, two lifetimes, what converts into a `Held` for the second converting into it.
pub struct Paired<'a, 'b>(pub &'a u32, pub Held<'b>);

impl<'a, 'b, T: Into<Held<'b>>> From<T> for Paired<'a, 'b> {
	fn from(value: T) -> Self {
		Paired(&0, value.into())
	}
}

/// Takes a borrow only for as long as `'static`, which the second of its target's two
/// `'static` lifetimes demands.
pub fn paired(value: impl Into<Paired<'static, 'static>>) -> u32 {
	*value.into().1.0
}

/// Excluded, a `'static` borrow, from what converts into one.
pub struct Pinned(pub &'static u32);

impl<T: Into<&'static u32>> From<T> for Pinned {
	fn from(value: T) -> Self {
		Pinned(value.into())
	}
}

/// Excluded, what converts into a `Pinned`.
pub struct Pinner(pub Pinned);

impl<T: Into<Pinned>> From<T> for Pinner {
	fn from(value: T) -> Self {
		Pinner(value.into())
	}
}

/// Takes a borrow only for as long as `'static`, which the last level's target demands.
pub fn pinner(value: impl Into<Pinner>) -> u32 {
	*value.into().0.0
}

/// Excluded, what converts into one of a `Vec` of its type, and so into ever larger ones.
pub struct Grows<T>(pub Vec<T>);

impl<T, U: Into<Grows<Vec<T>>> + Copy> From<U> for Grows<T> {
	fn from(value: U) -> Self {
		let _ = value.into();
		Grows(Vec::new())
	}
}

/// Takes what converts into a `Grows`, a chain that grows without end.
pub fn grows(value: impl Into<Grows<u8>>) -> usize {
	value.into().0.len()
}

/// Excluded, a name from a string, or from bytes that need not be UTF-8.
pub struct Called(pub String);

impl From<String> for Called {
	fn from(name: String) -> Self {
		Called(name)
	}
}

impl From<Vec<u8>> for Called {
	fn from(name: Vec<u8>) -> Self {
		Called(String::from_utf8_lossy(&name).into_owned())
	}
}

/// Excluded, what converts into a name.
pub struct Caller(pub Called);

impl<T: Into<Called>> From<T> for Caller {
	fn from(value: T) -> Self {
		Caller(value.into())
	}
}

/// Takes a string or bytes through `Called`'s conversions, which C, passing either as one
/// buffer, cannot pick between.
pub fn call(caller: impl Into<Caller>) -> String {
	caller.into().0.0
}

/// A range is a value, not an iterator to collect: one that maps to no range is refused.
pub fn alphabet() -> std::ops::Range<char> {
	'a'..'z'
}

/// A range with no end, which collecting would never finish.
pub fn naturals() -> std::ops::RangeFrom<u64> {
	0..
}

/// An iterator of handles with no clone to return.
pub fn opaques() -> std::slice::Iter<'static, Opaque> {
	[].iter()
}

/// An iterator inside a `Result`, which crosses only as a whole return.
pub fn tried() -> Result<std::vec::IntoIter<u8>, String> {
	Ok(Vec::new().into_iter())
}

/// An iterator a future resolves to.
pub async fn eventually() -> std::vec::IntoIter<u8> {
	Vec::new().into_iter()
}

/// A callback returning an iterator, which C would have to make, so C cannot implement it.
pub trait Source: Send + Sync {
	fn items(&self) -> std::vec::IntoIter<u8>;
}

/// A count, which C cannot implement as it needs `Copy`.
pub trait Tallied: Copy {
	fn tally(self) -> u32;
}

impl Tallied for u32 {
	fn tally(self) -> u32 {
		self
	}
}

/// What a tally counts, which C implements.
pub trait Measured {
	fn measure(&self) -> u32;
}

/// A tally made from `counted`, though no type implementing `Tallied` converts from one.
pub fn tallied<T: Tallied + From<C>, C: Measured>(counted: C) -> u32 {
	T::from(counted).tally()
}

/// How many items `source` has: a trait object C cannot make.
pub fn drain(source: Box<dyn Source>) -> usize {
	source.items().count()
}

/// The sum of items borrowed forever, which a slice C lends for the call cannot be.
pub fn summed_forever<I: IntoIterator<Item = &'static u32>>(items: I) -> u32 {
	items.into_iter().sum()
}

/// Items `text` could keep, as `retext` keeps its borrow.
pub fn retext_last<'a, I: IntoIterator<Item = &'a String>>(text: &mut Text<'a>, items: I) {
	if let Some(item) = items.into_iter().last() {
		text.0 = item;
	}
}

/// Items outliving `text`'s borrow, which is `'static` in a handle, so they are too.
pub fn outlast<'a, 'b: 'a, I: IntoIterator<Item = &'b String>>(text: &mut Text<'a>, items: I) {
	if let Some(item) = items.into_iter().last() {
		text.0 = item;
	}
}

/// Items a text taken by value could keep, once its parameter is chosen as a handle.
pub fn retext_into<'a, T: Into<Text<'a>>, I: IntoIterator<Item = &'a String>>(text: T, items: I) -> usize {
	let mut text = text.into();
	if let Some(item) = items.into_iter().last() {
		text.0 = item;
	}
	text.0.len()
}

/// One byte forever.
pub fn ones() -> std::iter::Repeat<u8> {
	std::iter::repeat(1)
}

/// Long words, through a filter that drops the rest at the call, where a caller who stops
/// early would have dropped them later or never.
pub fn long_words(words: Vec<String>) -> impl Iterator<Item = String> {
	words.into_iter().filter(|word| word.len() > 3)
}

/// The values `keep` holds, through a closure whose capture drops at the call rather than
/// with the caller's iterator.
pub fn kept_in(values: Vec<u32>, keep: Vec<u32>) -> impl Iterator<Item = u32> {
	values.into_iter().filter(move |value| keep.contains(value))
}

/// Counts down to one, as its own code reads it.
struct Countdown(u8);

impl Iterator for Countdown {
	type Item = u8;

	fn next(&mut self) -> Option<u8> {
		self.0 = self.0.checked_sub(1)?;
		Some(self.0 + 1)
	}
}

/// A countdown inside core's adapter, which still runs its own code.
pub fn launched() -> impl Iterator<Item = u8> {
	Countdown(3).chain([0])
}

/// A countdown to begin from, which turns into one only as it is flattened.
struct Start(u8);

impl IntoIterator for Start {
	type Item = u8;
	type IntoIter = Countdown;

	fn into_iter(self) -> Countdown {
		Countdown(self.0)
	}
}

/// Countdowns, each made by its own code as `flatten` reaches it.
pub fn launches() -> impl Iterator<Item = u8> {
	vec![Start(1), Start(2)].into_iter().flatten()
}

/// How often the code below ran.
static RAN: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// A pump, which runs its own code each time a borrow of it is turned into an iterator.
struct Pump([u8; 1]);

impl<'a> IntoIterator for &'a Pump {
	type Item = &'a u8;
	type IntoIter = std::slice::Iter<'a, u8>;

	fn into_iter(self) -> std::slice::Iter<'a, u8> {
		RAN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
		self.0.iter()
	}
}

static PUMPS: [Pump; 1] = [Pump([1])];

/// Each pump's bytes, through code only the items' borrows name.
pub fn pumped() -> impl Iterator<Item = &'static u8> {
	PUMPS.iter().flatten()
}

/// A token that counts its own drop.
pub struct Token(pub u8);

impl Drop for Token {
	fn drop(&mut self) {
		RAN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
	}
}

/// The second token: `skip` drops the first only as the iterator is read.
pub fn skipped() -> impl Iterator<Item = Token> {
	vec![Token(1), Token(2)].into_iter().skip(1)
}

/// The bytes in either set, which `union` compares as the iterator is read.
pub fn merged() -> impl Iterator<Item = u8> {
	static A: std::collections::BTreeSet<u8> = std::collections::BTreeSet::new();
	static B: std::collections::BTreeSet<u8> = std::collections::BTreeSet::new();
	A.union(&B).copied()
}

/// A map's values, which drops each key as the iterator is read.
pub fn values() -> std::collections::btree_map::IntoValues<u8, u8> {
	std::collections::BTreeMap::new().into_values()
}

/// How many floats a set holds, which Rust could not build from C's floats, as they are
/// not `Ord`.
pub fn floats(set: std::collections::BTreeSet<f64>) -> usize {
	set.len()
}

/// As many zeros as the caller asks, held nowhere until made.
pub fn zeros(n: usize) -> std::iter::RepeatN<u8> {
	std::iter::repeat_n(0, n)
}

/// The successes, where turning each failure into an iterator drops its token.
pub fn successes() -> impl Iterator<Item = u8> {
	vec![Err(Token(0)), Ok(7)].into_iter().flatten()
}

/// An allocator of the crate's own, which counts each call into it.
struct Counted;

unsafe impl std::alloc::Allocator for Counted {
	fn allocate(&self, layout: std::alloc::Layout) -> Result<std::ptr::NonNull<[u8]>, std::alloc::AllocError> {
		RAN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
		std::alloc::Global.allocate(layout)
	}

	unsafe fn deallocate(&self, ptr: std::ptr::NonNull<u8>, layout: std::alloc::Layout) {
		RAN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
		unsafe { std::alloc::Global.deallocate(ptr, layout) }
	}
}

/// Bytes in the crate's own allocation, which collecting frees through its allocator.
pub fn allocated() -> impl Iterator<Item = u8> {
	let mut bytes = Vec::new_in(Counted);
	bytes.push(1);
	bytes.into_iter()
}

/// The same bytes, whose allocation `flatten` frees through the allocator as it reads them.
pub fn flattened() -> impl Iterator<Item = u8> {
	let mut bytes = Vec::new_in(Counted);
	bytes.push(1);
	vec![bytes].into_iter().flatten()
}

/// An allocator of the crate's own, which only a borrow of it implements.
struct Local;

unsafe impl std::alloc::Allocator for &Local {
	fn allocate(&self, layout: std::alloc::Layout) -> Result<std::ptr::NonNull<[u8]>, std::alloc::AllocError> {
		RAN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
		std::alloc::Global.allocate(layout)
	}

	unsafe fn deallocate(&self, ptr: std::ptr::NonNull<u8>, layout: std::alloc::Layout) {
		RAN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
		unsafe { std::alloc::Global.deallocate(ptr, layout) }
	}
}

static LOCAL: Local = Local;

/// Bytes in an allocation a borrowed allocator frees as collecting finishes.
pub fn lent() -> impl Iterator<Item = u8> {
	let mut bytes = Vec::new_in(&LOCAL);
	bytes.push(1);
	bytes.into_iter()
}

/// A pattern of the crate's own, whose searcher a split runs as it is read.
struct Slash;

unsafe impl std::str::pattern::Pattern for Slash {
	type Searcher<'a> = std::str::pattern::CharSearcher<'a>;

	fn into_searcher(self, haystack: &str) -> std::str::pattern::CharSearcher<'_> {
		'/'.into_searcher(haystack)
	}
}

/// Halves split where the crate's own pattern says.
pub fn halves() -> impl Iterator<Item = &'static str> {
	"a/b".split(Slash)
}

/// A `Poll` with nothing to register a wake through: its future would pend forever.
pub fn settled() -> std::task::Poll<u8> {
	std::task::Poll::Ready(1)
}

/// A poll taking an owned value, which the future cannot pass again at its next poll.
pub fn poll_named(cx: &mut std::task::Context<'_>, name: String) -> std::task::Poll<usize> {
	cx.waker().wake_by_ref();
	std::task::Poll::Ready(name.len())
}

/// A poll whose value borrows what the future owns.
pub fn poll_first<'a>(items: &'a [u8], cx: &mut std::task::Context<'_>) -> std::task::Poll<&'a u8> {
	cx.waker().wake_by_ref();
	items.first().map_or(std::task::Poll::Pending, std::task::Poll::Ready)
}

/// A poll lent a closure, which C lends only for the call, and the future outlives.
pub fn poll_until(cx: &mut std::task::Context<'_>, done: impl Fn(u8) -> bool) -> std::task::Poll<()> {
	match done(1) {
		true => std::task::Poll::Ready(()),
		false => {
			cx.waker().wake_by_ref();
			std::task::Poll::Pending
		}
	}
}

/// A future holding a context, which C's waker makes only for the call.
pub async fn context_later(cx: &std::task::Context<'_>) {
	cx.waker().wake_by_ref();
}

/// A ticket made from a waker at only one instance of its type.
pub struct Ticket<T>(Option<std::task::Waker>, T);

impl Ticket<u8> {
	pub fn new(waker: std::task::Waker) -> Self {
		Self(Some(waker), 0)
	}
}

/// The instance C holds, which no constructor makes from a waker.
pub fn ticket() -> Ticket<u16> {
	Ticket(None, 1)
}

/// A poll through an instance no constructor makes from a waker.
pub fn poll_ticket(ticket: &Ticket<u16>) -> std::task::Poll<u16> {
	std::task::Poll::Ready(ticket.1)
}

/// A poll through two contexts, which one future's poll cannot both be.
pub fn poll_both(a: &mut std::task::Context<'_>, b: &std::task::Context<'_>) -> std::task::Poll<()> {
	a.waker().wake_by_ref();
	b.waker().wake_by_ref();
	std::task::Poll::Ready(())
}

/// A poll lent a value mutably, which its future owns, so would never write back.
pub fn poll_bump(cx: &mut std::task::Context<'_>, count: &mut u32) -> std::task::Poll<()> {
	*count += 1;
	cx.waker().wake_by_ref();
	std::task::Poll::Ready(())
}

/// An optional handle lent inside a `Box`, as [`peek`] lends one.
pub fn peek_some(value: Option<&Box<Opaque>>) -> u8 {
	value.map_or(0, |value| value.0)
}

/// An optional value lent mutably.
pub fn bump_some(value: Option<&mut u32>) {
	if let Some(value) = value {
		*value += 1;
	}
}

/// Bytes written back that the return still borrows, which Rust's write would move.
pub fn rest(buf: &mut Vec<u8>) -> &[u8] {
	buf.as_slice()
}

/// Bytes written back by a future, which owns what it borrows, so has nothing to write.
pub async fn fill(buf: &mut Vec<u8>) {
	buf.push(0);
}

/// A slice lent mutably, which C cannot pass in and out as a value without a copy.
pub fn zero(buf: &mut [u8]) {
	buf.fill(0);
}

/// Another crate's `Arc` lent mutably, which its opaque handle cannot replace in place.
pub fn reseat(size: &mut std::sync::Arc<dep::Size>) {
	*size = std::sync::Arc::new(**size);
}

/// Holds a value it lends only as an error, which crosses as a clone, never lent.
pub struct Keeper(Opaque);

impl Keeper {
	pub fn new() -> Self {
		Self(Opaque(0))
	}

	/// Its value, lent as an error: only the value returned itself is lent.
	pub fn check(&self) -> Result<u8, &Opaque> {
		Err(&self.0)
	}
}

/// A callback taking bytes it writes back, which a foreign implementation cannot.
pub trait Sink: Send + Sync {
	fn fill(&self, buf: &mut Vec<u8>);
}

/// Another crate whole, whose every release would change this one's API.
pub use dep;

/// The same crate whole, by another name.
pub use ::dep as renamed;

/// A crate whole, which the config excludes, so nothing refuses it.
pub use ::dep as Shed;

/// A value whose poll changes it, which C passes as a copy.
pub struct Tally {
	pub count: u8,
}

impl Tally {
	pub fn poll_bump(&mut self, cx: &mut std::task::Context<'_>) -> std::task::Poll<u8> {
		cx.waker().wake_by_ref();
		self.count += 1;
		std::task::Poll::Ready(self.count)
	}
}

/// A waiter made from a waker only by an `unsafe` function, exported through a safe one.
pub struct Risky(std::task::Waker);

impl Risky {
	pub fn idle() -> Self {
		Self(std::task::Waker::noop().clone())
	}

	/// # Safety
	///
	/// Nothing, in this fixture.
	pub unsafe fn new(waker: std::task::Waker) -> Self {
		Self(waker)
	}
}

/// A poll through a waiter no safe function makes.
pub fn poll_risky(waiter: &Risky) -> std::task::Poll<()> {
	waiter.0.wake_by_ref();
	std::task::Poll::Ready(())
}

/// A waiter made from a waker only by a function whose type argument the call cannot infer.
pub struct Loose(std::task::Waker);

impl Loose {
	pub fn new<T: Default>(waker: std::task::Waker) -> Self {
		let _ = T::default();
		Self(waker)
	}
}

/// A poll through a waiter no non-generic function makes.
pub fn poll_loose(waiter: &Loose) -> std::task::Poll<()> {
	waiter.0.wake_by_ref();
	std::task::Poll::Ready(())
}

/// Three optional borrowed objects: Swift, whose optional holds an object only by consuming
/// it, would overload each with and without it, past the bound it takes.
pub fn among(first: Option<&Opaque>, second: Option<&Opaque>, third: Option<&Opaque>) -> u8 {
	[first, second, third].into_iter().flatten().map(|o| o.0).sum()
}

/// An error holding an object, which Swift holds as a noncopyable struct while
/// `Swift.Error` requires a copyable type.
pub fn wrapped(value: u8) -> Result<u8, Vec<Opaque>> {
	Err(vec![Opaque(value)])
}

/// An array of an element other than `u8`, which no export needs yet.
pub fn sum_pair(pair: [u16; 2]) -> u32 {
	u32::from(pair[0]) + u32::from(pair[1])
}

/// An empty array, which no C struct holds.
pub fn empty(bytes: [u8; 0]) -> usize {
	bytes.len()
}

/// A plain enum whose discriminant C's `int32_t` cannot hold.
#[repr(i64)]
pub enum Wide {
	Small = 1,
	Large = 1 << 40,
}

/// One past 64 bits.
#[repr(i128)]
pub enum Huge {
	Small = 1,
	Large = 1 << 64,
}

/// One past `i128::MAX`.
#[repr(u128)]
pub enum Top {
	Small = 1,
	Large = u128::MAX,
}

/// An `unsafe fn`, whose preconditions safe foreign code cannot uphold.
pub unsafe fn raw(x: u32) -> u32 {
	x
}

/// A C-variadic definition, whose variadic arguments C would silently drop.
pub unsafe extern "C" fn sum(n: u32, _args: ...) -> u32 {
	n
}

/// A custom-ABI function, which nothing can call from Rust.
#[unsafe(naked)]
pub unsafe extern "custom" fn custom() {
	core::arch::naked_asm!("ret")
}

/// A value with an `unsafe` method.
pub struct Raw;

impl Raw {
	/// A value to call it on.
	pub fn new() -> Self {
		Self
	}

	/// An `unsafe` inherent method.
	pub unsafe fn peek(&self) -> u32 {
		0
	}
}

/// An `unsafe trait`, which foreign code would implement without upholding its contract.
pub unsafe trait Trusted: Send + Sync {
	fn get(&self) -> u32;
}

/// A trait with an `unsafe` method.
pub trait Hazard: Send + Sync {
	unsafe fn risk(&self) -> u32;
}

/// Takes the `unsafe trait`, so its foreign implementation would be exported.
pub fn trust(trusted: Box<dyn Trusted>) -> u32 {
	trusted.get()
}

/// Takes the trait with an `unsafe` method.
pub fn hazard(hazard: Box<dyn Hazard>) -> u32 {
	unsafe { hazard.risk() }
}

/// A `NonZero` of a 128-bit integer, which no integer of C's holds.
pub fn wide_non_zero(n: std::num::NonZero<u128>) -> u128 {
	n.get()
}

/// A `Copy` view of bytes it borrows, held across as its bytes.
#[derive(Clone, Copy)]
pub struct View<'a> {
	bytes: &'a [u8],
}

impl View<'_> {
	pub fn len(&self) -> usize {
		self.bytes.len()
	}
}

/// A view of nothing, which borrows nothing, so an export returns a `View`.
pub fn empty_view() -> View<'static> {
	View { bytes: &[] }
}

/// Returns a value that borrows its parameter, which C could keep past the borrow.
pub fn wrap(bytes: &[u8]) -> View<'_> {
	View { bytes }
}

/// The same, through an `impl Trait` hiding the value, which holds what it captures.
pub fn hidden_wrap(bytes: &[u8]) -> impl Clone + '_ {
	View { bytes }
}

/// A `Copy` mark held across as its bytes.
#[derive(Clone, Copy)]
pub struct Mark {
	at: u64,
}

/// A mark at `at`, so an export returns one.
pub fn mark(at: u64) -> Mark {
	Mark { at }
}

/// A closure returning a borrowed value, which C has no reference to hand back.
pub fn picked(pick: impl Fn() -> &'static Mark) -> u64 {
	pick().at
}

/// The same, in an `Option`.
pub fn maybe_picked(pick: impl Fn() -> Option<&'static Mark>) -> u64 {
	pick().map_or(0, |mark| mark.at)
}

/// A future whose output is an `impl Trait` hiding no object, which the glue cannot name.
pub async fn hidden_answer() -> impl Copy {
	1_u32
}

/// A collected iterator whose items are an `impl Trait` hiding no object.
pub fn hidden_items() -> impl Iterator<Item = impl Copy> {
	[1_u32].into_iter()
}

/// An `impl Trait` hiding an `Option`, which the glue cannot take apart.
pub fn hidden_option() -> impl Clone {
	Some(1_u32)
}

/// A boxed slice of bytes, which no buffer form takes.
pub fn boxed_bytes(bytes: Box<[u8]>) -> usize {
	bytes.len()
}

/// An iterator of borrowed strings a future keeps past the call that lent them.
pub async fn count_later<'a>(words: impl Iterator<Item = &'a str> + Send) -> usize {
	words.count()
}

/// An iterator of borrowed objects, which C lends none of inside an array.
pub fn count_opaques<'a>(items: impl Iterator<Item = &'a Opaque>) -> usize {
	items.count()
}

/// A trait object with another auto trait than the one form its handle names.
pub fn unpin_resolver() -> std::sync::Arc<dyn tls::held::Resolver + Unpin> {
	std::sync::Arc::new(tls::held::Named("unpin".into()))
}

/// A trait object that borrows, which no handle outliving the call may hold.
pub fn borrowed_resolver<'a>(_ids: &'a [u32]) -> std::sync::Arc<dyn tls::held::Resolver + 'a> {
	std::sync::Arc::new(tls::held::Named("borrowed".into()))
}

/// A stream an exported type implements, so the bound is met by a value C holds: refused,
/// not left out.
pub struct Pipe(pub u8);

impl tls::held::Stream for Pipe {
	fn read(&mut self) -> u8 {
		self.0
	}
}

/// Reads one byte of a stream.
pub fn pump_pipe<T: tls::held::Stream>(mut stream: T) -> u8 {
	stream.read()
}

/// A sink a reference to any exported type meets: refused, not left out.
pub fn pump_sink<T: tls::held::Sink>(sink: T) -> u8 {
	sink.write(1)
}

/// A width only `u32`, which C passes by role, meets: refused, not left out.
pub fn widen<T: tls::held::Width>(x: T) -> u32 {
	x.bits()
}
