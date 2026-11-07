//! Values JS can't represent as Rust does.

use crate::places::Bare;

pub fn float(a: f16) -> f16 {
	a * 2.0
}

/// `None` and `Some(None)` would both be `rs.None`, as `Some(Some(None))` is `undefined`.
pub fn nested(x: Option<Option<Option<u8>>>) -> bool {
	x.is_some()
}

/// A generic type's `Option<T>` is an `rs.OptionOf<T>`, which decides its `None` by
/// whether TypeScript's type of `T` can be `undefined`, as a `MaybeUninit<u32>`'s `number`
/// can't, though its value can, so an `Option<MaybeUninit<u32>>`'s `None` is `rs.None`.
pub struct Maybe<T> {
	pub value: Option<T>,
}

pub fn generic_uninit(value: u32) -> bool {
	Maybe {
		value: Some(std::mem::MaybeUninit::new(value)),
	}
	.value
	.is_some()
}

/// A generic type's `Option<T>` of an `Option<()>`, whose `None` and `Some(None)` would
/// both be `rs.None`.
pub fn generic_nested(unit: bool) -> bool {
	let maybe: Maybe<Option<()>> = Maybe { value: None };
	drop(maybe);
	unit
}

/// A generic type's `Option<MaybeUninit<T>>` is `T | rs.None`, as a `MaybeUninit` can be
/// `undefined`, but for a byte, whose `None` is `undefined`.
pub struct MaybeByte<T> {
	pub value: Option<std::mem::MaybeUninit<T>>,
}

pub fn generic_uninit_byte(b: u8) -> bool {
	MaybeByte {
		value: Some(std::mem::MaybeUninit::new(b)),
	}
	.value
	.is_some()
}

/// The same in a generic type's field, whose other parameter is still open.
pub struct ByteAnd<T, U> {
	pub value: Option<std::mem::MaybeUninit<T>>,
	pub other: U,
}

pub struct HoldsByte<U> {
	pub held: ByteAnd<u8, U>,
}

pub fn generic_uninit_byte_open(b: u8) -> bool {
	HoldsByte {
		held: ByteAnd {
			value: Some(std::mem::MaybeUninit::new(b)),
			other: 1u32,
		},
	}
	.held
	.value
	.is_some()
}

/// What a type holds, named by an associated type.
pub trait Holds {
	type Held;
}

impl Holds for u32 {
	type Held = std::mem::MaybeUninit<u32>;
}

impl Holds for () {
	type Held = Option<()>;
}

/// A generic type's `Option` of a projection is an `rs.OptionOf` of it too, which each
/// instance resolves as [`Maybe`]'s.
pub struct Projected<S: Holds> {
	pub value: Option<S::Held>,
}

/// Its `MaybeUninit<u32>`, whose `None` `rs.OptionOf` would make `undefined`.
pub fn projected_uninit(value: u32) -> bool {
	Projected::<u32> {
		value: Some(std::mem::MaybeUninit::new(value)),
	}
	.value
	.is_some()
}

/// Its `Option<()>`, whose `None` and `Some(None)` would both be `rs.None`.
pub fn projected_nested(unit: bool) -> bool {
	let projected: Projected<()> = Projected { value: None };
	drop(projected);
	unit
}

/// What a source yields, named by an associated type.
pub trait Source {
	type Item;
}

/// What a source holds next.
pub struct Held<S: Source> {
	pub next: Option<Box<S::Item>>,
}

/// A source whose items hold it again.
pub struct Looped;

impl Source for Looped {
	type Item = Held<Looped>;
}

/// `Held<Looped>`'s projection argument is `Held<Looped>` itself, which TypeScript can name
/// only through a type alias of its own.
pub fn projected_self(deep: bool) -> bool {
	let held: Held<Looped> = Held {
		next: deep.then(|| Box::new(Held { next: None })),
	};
	held.next.is_some()
}

/// What a source lends next, which drops nothing itself.
pub struct Lent<S: Source + 'static> {
	pub next: Option<&'static S::Item>,
}

/// A source whose items lend it again.
pub struct Lends;

impl Source for Lends {
	type Item = Lent<Lends>;
}

/// Dropping an `Rc` of `Lent<Lends>` asks whether its glue is inert, through its projection,
/// which ends rather than overflow; it translates, as nothing names the instance's type.
pub fn projected_inert(deep: bool) -> bool {
	let lent = std::rc::Rc::new(Lent::<Lends> { next: None });
	drop(lent);
	deep
}

/// What a source lends next, beside what it owns, so it drops.
pub struct Owning<S: Source + 'static> {
	pub next: Option<&'static S::Item>,
	pub owned: std::rc::Rc<()>,
}

/// A source whose items own it again.
pub struct Owns;

impl Source for Owns {
	type Item = Owning<Owns>;
}

/// Dropping an `Rc` of `Owning<Owns>` asks whether its glue is shared, through its projection,
/// which ends rather than overflow; it translates, as nothing names the instance's type.
pub fn projected_disposable(deep: bool) -> bool {
	let owning = std::rc::Rc::new(Owning::<Owns> {
		next: None,
		owned: std::rc::Rc::new(()),
	});
	drop(owning);
	deep
}

/// A source wrapped once more.
pub struct Wrapped<T>(pub T);

impl<T: 'static> Source for Wrapped<T> {
	type Item = Spread<Wrapped<Wrapped<T>>>;
}

/// A type that holds itself at a wrapped source, whose item is it at a source wrapped again.
pub struct Spread<S: Source + 'static> {
	pub item: Option<&'static S::Item>,
	pub next: Option<&'static Spread<Wrapped<S>>>,
}

/// `Spread<S>` would name an instance at each wrapping.
pub fn projected_spread(deep: bool) -> bool {
	let spread: Spread<Wrapped<u8>> = Spread { item: None, next: None };
	spread.next.is_none() && deep
}

/// What a duo's ping and pong both are at a wrapped duo: the same type at one wrapped again.
pub struct Fork<D: Duo + 'static> {
	pub ping: Option<&'static D::Ping>,
	pub pong: Option<&'static D::Pong>,
}

impl<T: 'static> Duo for Wrapped<T> {
	type Ping = Fork<Wrapped<Wrapped<T>>>;
	type Pong = Fork<Wrapped<Wrapped<T>>>;
}

/// `Fork<Wrapped<u8>>` names ever deeper instances twice over at each level.
pub fn projected_fork(deep: bool) -> Option<&'static Fork<Wrapped<u8>>> {
	deep.then_some(None).flatten()
}

/// The end of a [`Links`], which drops.
pub struct Loud;

impl Drop for Loud {
	fn drop(&mut self) {}
}

/// The source past the last link.
pub struct Zero;

impl Source for Zero {
	type Item = &'static Loud;
}

/// One link more than its source.
pub struct Succ<T>(pub T);

impl<T: Source + 'static> Source for Succ<T> {
	type Item = &'static Links<T>;
}

/// A link lending the next, down to a [`Loud`].
pub struct Links<S: Source + 'static> {
	pub item: S::Item,
}

/// Ten links more.
type Ten<T> = Succ<Succ<Succ<Succ<Succ<Succ<Succ<Succ<Succ<Succ<T>>>>>>>>>>;

/// A chain of 140 links, which ends, but past the recursion limit.
pub type Deep = Ten<Ten<Ten<Ten<Ten<Ten<Ten<Ten<Ten<Ten<Ten<Ten<Ten<Ten<Zero>>>>>>>>>>>>>>;

/// Dropping a shared chain drops none of what it lends.
pub fn projected_deep(chain: std::rc::Rc<Links<Deep>>) {
	drop(chain);
}

/// Two types a source names, each holding the other.
pub trait Duo {
	type Ping;
	type Pong;
}

/// What holds a [`Duo`]'s pong.
pub struct Ping<D: Duo> {
	pub pong: Option<Box<D::Pong>>,
}

/// What holds a [`Duo`]'s ping.
pub struct Pong<D: Duo> {
	pub ping: Option<Box<D::Ping>>,
}

/// A duo of its own ping and pong.
pub struct Rally;

impl Duo for Rally {
	type Ping = Ping<Rally>;
	type Pong = Pong<Rally>;
}

/// `Ping<Rally>`'s projection argument is `Pong<Rally>`, whose own is `Ping<Rally>` again.
pub fn projected_rally(deep: bool) -> bool {
	let ping: Ping<Rally> = Ping {
		pong: deep.then(|| Box::new(Pong { ping: None })),
	};
	ping.pong.is_some()
}

/// A chain of types, each naming the next.
pub trait Chain {
	type Item;
	type Next: Chain;
}

/// A type that holds itself at its parameter's next, whose projections at each are new.
pub struct Nested<C: Chain> {
	pub item: C::Item,
	pub next: Option<Box<Nested<C::Next>>>,
}

impl Chain for u8 {
	type Item = u8;
	type Next = u8;
}

/// `Nested<C>` would need a projection argument for every link of the chain.
pub fn projected_chain(item: u8) -> u8 {
	let nested: Nested<u8> = Nested { item, next: None };
	nested.item
}

/// Parsing an address is not translated yet.
pub fn addr_parsed(s: &str) -> Result<std::net::Ipv4Addr, std::net::AddrParseError> {
	s.parse()
}

/// A union whose fields overlap in values JS keeps apart.
pub union Bits {
	pub int: u32,
	pub float: f64,
}

pub fn bits(n: u32) -> u32 {
	let b = Bits { int: n };
	unsafe { b.int }
}

/// A union over a type parameter whose fields overlap in values JS keeps apart in every
/// instance.
pub union Mixed<T: Copy> {
	pub pair: (u32, std::marker::PhantomData<T>),
	pub float: f32,
}

/// One whose fields that its definition lays out overlap in values JS keeps apart, beside
/// one only an instance lays out.
pub union Beside<T: Copy> {
	pub int: u32,
	pub float: f32,
	pub value: T,
}

pub fn beside<T: Copy>(b: &Beside<T>) -> u32 {
	unsafe { b.int }
}

pub fn mixed<T: Copy>(m: &Mixed<T>) -> u32 {
	unsafe { m.pair.0 }
}

/// An instance of it, refused only where it is defined.
pub fn mixed_byte(m: &Mixed<u8>) -> u32 {
	mixed(m)
}

/// A union over a type parameter that only an instance lays out, which a census lowering
/// over the parameter refuses nothing in.
pub union Paired<T: Copy> {
	pub pair: (T, u8),
	pub value: T,
}

pub fn tag_flipped<T: Copy>(p: &mut Paired<T>) {
	flip(unsafe { &mut p.pair.1 });
}

fn flip(tag: &mut u8) {
	*tag ^= 0x80;
}

/// Pointers to places of no bytes, which Rust may put anywhere, `()`'s refused as made.
pub fn units_compared() -> bool {
	let (a, b, c, d) = ((), (), Bare, Bare);
	std::ptr::eq(&raw const a, std::ptr::NonNull::from(&b).as_ptr()) && std::ptr::eq(&raw const c, &raw const d)
}

pub enum Only {
	One,
}

/// A dangling pointer to an enum of no bytes, whose value is its variant's tag.
pub fn dangling_enum() -> bool {
	unsafe { std::ptr::NonNull::<Only>::dangling().as_ptr().as_ref().is_some() }
}

/// `u8`'s unstable ASCII methods are not translated.
pub fn ascii_octal(a: u8) -> bool {
	a.is_ascii_octdigit()
}

/// `LinkedList`'s unstable `retain`, which no mapping covers, as none does its other
/// unstable methods.
pub fn lists(l: &mut std::collections::LinkedList<u8>) {
	l.retain(|x| *x > 1);
}

/// `ManuallyDrop`'s impls beyond holding a value: compared, formatted, and a constant.
pub fn manual_impls(a: std::mem::ManuallyDrop<u32>, b: std::mem::ManuallyDrop<u32>) -> String {
	const HELD: std::mem::ManuallyDrop<u32> = std::mem::ManuallyDrop::new(3);
	format!("{a:?} {} {}", a == b, *HELD)
}
