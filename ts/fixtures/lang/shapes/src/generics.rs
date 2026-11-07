//! Methods on generic structs and enums, each instance its own method.

use std::borrow::Cow;
use std::cell::{RefCell, UnsafeCell};
use std::rc::Rc;
use std::task::Poll;

use crate::fmt::Pair;
use crate::harness::put;
use crate::shapes::{Area, Meters, Point, Shape, Track};

/// A generic struct, with methods on every instance, on some, and on one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wrap<T> {
	pub value: T,
	pub count: u8,
}

impl<T> Wrap<T> {
	pub fn new(value: T) -> Self {
		Self { value, count: 0 }
	}

	pub fn get(&self) -> &T {
		&self.value
	}

	pub fn bump(&mut self) -> u8 {
		self.count = self.count.wrapping_add(1);
		self.count
	}

	pub fn into_inner(self) -> T {
		self.value
	}

	/// A method with its own type parameter.
	pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Wrap<U> {
		Wrap {
			value: f(self.value),
			count: self.count,
		}
	}
}

impl<T: Area> Wrap<T> {
	pub fn scaled(&self) -> u64 {
		self.value.area().saturating_add(self.count as u64)
	}
}

impl Wrap<u8> {
	pub fn double(&self) -> u16 {
		self.value as u16 * 2
	}
}

impl<T: Area> Area for Wrap<T> {
	const SIDES: u8 = T::SIDES;

	fn area(&self) -> u64 {
		self.value.area()
	}
}

/// A generic enum, with a companion function per instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Either<L, R> {
	Left(L),
	Right { value: R },
	Neither,
}

impl<L: Copy, R: Copy> Either<L, R> {
	pub fn left(&self) -> Option<L> {
		match *self {
			Either::Left(l) => Some(l),
			_ => None,
		}
	}

	pub fn flip(self) -> Either<R, L> {
		match self {
			Either::Left(l) => Either::Right { value: l },
			Either::Right { value } => Either::Left(value),
			Either::Neither => Either::Neither,
		}
	}
}

/// Two parameters, one nested in another generic type.
#[derive(Debug)]
pub struct Tagged<K, V> {
	pub key: K,
	pub inner: Wrap<V>,
	pub all: Vec<Wrap<V>>,
}

impl<K: Copy, V: Copy> Tagged<K, V> {
	fn first(&self) -> Option<Wrap<V>> {
		self.all.first().copied()
	}
}

/// Each method once per instance: `Wrap` of a struct, an enum, a number, and itself.
pub fn methods(data: &[u8], out: &mut Vec<u8>) {
	let at = |i: usize| data.get(i).copied().unwrap_or(0);
	let mut point = Wrap::new(Point { x: at(0) as i32, y: -2 });
	point.bump();
	let mut byte = Wrap::new(at(1));
	byte.bump();
	byte.bump();
	let nested = Wrap::new(Wrap::new(Pair(at(2), -3)));
	let meters = Wrap {
		value: Meters(at(3) as u64),
		count: at(4),
	};
	let shape = Wrap::new(Shape::Rect {
		corner: point.into_inner(),
		width: at(5) as u16,
		height: 3,
	});
	put(out, &format!("{point:?} {byte:?} {nested:?} {shape:?}"));
	put(out, &format!("{nested:#?}"));
	out.extend_from_slice(&byte.double().to_be_bytes());
	out.push(*byte.get());
	out.push(nested.get().get().0);
	out.extend_from_slice(&meters.scaled().to_be_bytes());
	out.extend_from_slice(&shape.scaled().to_be_bytes());
	meters.describe(out);
	shape.describe(out);
	let mapped = byte.map(|b| Pair(b, at(6) as i64));
	put(out, &format!("{mapped:?} {:?}", mapped == Wrap::new(Pair(at(1), 0))));

	let eithers: [Either<u8, Point>; 3] = [
		Either::Left(at(7)),
		Either::Right {
			value: Point::new(1, -1),
		},
		Either::Neither,
	];
	for either in eithers {
		let flipped = either.flip();
		put(
			out,
			&format!("{either:?} {flipped:#?} {:?} {:?}", either.left(), flipped.left()),
		);
	}
	// Literals TypeScript would widen, as a class parameter infers from them.
	let mut list = Wrap {
		value: Vec::new(),
		count: 1,
	};
	list.value.push(at(10) as u16);
	list.bump();
	let none = Wrap {
		value: None::<u8>,
		count: 0,
	};
	let some = Wrap::new(data.get(11).copied());
	let err: Wrap<Result<u8, Shape>> = Wrap {
		value: Err(Shape::Dot),
		count: 2,
	};
	put(
		out,
		&format!(
			"{list:?} {none:?} {some:?} {err:?} {:?}",
			none.get().unwrap_or(some.count)
		),
	);

	let tagged = Tagged {
		key: Either::<u8, u16>::Right { value: 500 },
		inner: Wrap::new(at(8) as i16 - 100),
		all: [Wrap::new(-1), Wrap::new(at(9) as i16)].to_vec(),
	};
	put(out, &format!("{tagged:?} {:?}", tagged.first()));
	out.extend_from_slice(&tagged.inner.value.to_be_bytes());
	out.push(tagged.key.left().unwrap_or(0));
}

/// Moves `track`'s mark on: the crate's only write into a `Mark`, in another module than
/// the copy of one that must not see it.
pub fn advance(track: &mut Track) {
	track.mark.at = track.mark.at.wrapping_add(1);
}

/// Resets each of `xs`, writing into each one in place, which only an instance knows the
/// type of.
pub fn reset<X: Default>(xs: &mut [X]) {
	for x in xs.iter_mut() {
		*x = X::default();
	}
}

/// A generic type holding runs of its parameter in each form JS keeps a run of bytes
/// apart in, which only an instance decides.
#[derive(Clone, Debug, PartialEq)]
pub struct Runs<'a, T: Clone> {
	pub array: [T; 3],
	pub list: Vec<T>,
	pub view: &'a [T],
	pub boxed: Box<[T]>,
	pub cow: Cow<'a, [T]>,
	pub pairs: Vec<[T; 2]>,
	pub maybe: Option<Vec<T>>,
}

/// An enum holding a run of its parameter in a variant.
#[derive(Clone, Debug, PartialEq)]
pub enum Run<T> {
	Listed(Vec<T>),
	Fixed { array: [T; 2] },
	Empty,
}

impl<T: Copy + Default> Runs<'_, T> {
	fn total(&self) -> usize {
		self.array.len() + self.list.len() + self.view.len() + self.boxed.len() + self.cow.len() + self.pairs.len()
	}

	fn push(&mut self, item: T) {
		self.list.push(item);
		self.cow.to_mut().push(item);
		self.pairs.push([item; 2]);
		self.array.rotate_left(1);
		self.array[2] = item;
		self.boxed[0] = item;
		if let Some(maybe) = &mut self.maybe {
			maybe.push(item);
		}
	}

	fn run(&self) -> Run<T> {
		match self.list.len() % 3 {
			0 => Run::Listed(self.list.clone()),
			1 => Run::Fixed {
				array: [self.array[0], self.array[2]],
			},
			_ => Run::Empty,
		}
	}
}

impl<T: Copy> Run<T> {
	fn first(&self) -> Option<T> {
		match self {
			Run::Listed(list) => list.first().copied(),
			Run::Fixed { array } => Some(array[0]),
			Run::Empty => None,
		}
	}
}

/// A generic type JS copies, holding an array of its parameter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fixed<T> {
	pub pair: [T; 2],
	pub tag: u8,
}

/// `fixed` with its pair swapped, in a copy of it.
fn swapped<T: Copy>(fixed: Fixed<T>) -> Fixed<T> {
	let mut copy = fixed;
	copy.pair.swap(0, 1);
	copy
}

/// A generic type lending a run of its parameter, which it writes through, and a function
/// of one.
struct Lends<'a, T> {
	run: &'a mut [T],
	last: fn(&[T]) -> Option<&T>,
}

/// Writes `item` over each element of `run`, through a `&mut` to a field.
fn fill<T: Copy>(run: &mut [T], item: T) {
	for x in run {
		*x = item;
	}
}

/// [`Runs`] at bytes and at another element: made, written through its fields and
/// through `&mut`s to them, copied, compared, and printed.
macro_rules! runs_of {
	($data:expr, $out:expr, $ty:ty) => {{
		let data: Vec<$ty> = $data.iter().map(|&b| <$ty>::from(b)).collect();
		let n = data.first().copied().unwrap_or_default();
		let mut runs = Runs {
			array: [n; 3],
			list: data.clone(),
			view: &data,
			boxed: data.clone().into_boxed_slice(),
			cow: Cow::Borrowed(&data),
			pairs: Vec::new(),
			maybe: (data.len() % 2 == 0).then(Vec::new),
		};
		let before = runs.clone();
		for &x in data.iter().take(3) {
			runs.push(x);
		}
		if let Some(pair) = runs.pairs.first_mut() {
			fill(pair, n);
		}
		fill(&mut runs.array[1..], <$ty>::from(7u8));
		let mut taken = std::mem::take(&mut runs.list);
		let lent = Lends {
			run: &mut taken,
			last: <[$ty]>::last,
		};
		let last = (lent.last)(runs.view).copied();
		if let Some(x) = lent.run.last_mut() {
			*x = n;
		}
		fill(lent.run, <$ty>::from(3u8));
		let run = runs.run();
		let fixed = Fixed { pair: [n, n], tag: 1 };
		let mut copy = fixed;
		copy.pair[0] = <$ty>::from(9u8);
		let swapped = swapped(copy);
		put(
			$out,
			&format!(
				"{runs:?} {before:?} {taken:?} {run:?} {fixed:?} {copy:?} {swapped:?} {} {} {:?} {last:?}",
				runs.total(),
				runs == before,
				run.first(),
			),
		);
	}};
}

/// Logs its id as it drops, and as it is cloned, the id with the high bit set.
struct Ticked {
	id: u8,
	log: Rc<RefCell<Vec<u8>>>,
}

impl Clone for Ticked {
	fn clone(&self) -> Self {
		self.log.borrow_mut().push(0x80 | self.id);
		Self {
			id: self.id,
			log: self.log.clone(),
		}
	}
}

impl Drop for Ticked {
	fn drop(&mut self) {
		self.log.borrow_mut().push(self.id);
	}
}

/// [`Runs`], [`Run`], and [`Fixed`] at `u8`, whose runs JS keeps as bytes, and at
/// `u16`, then [`Runs`] and [`Run`] of values that drop, in the order of their fields.
pub fn runs(data: &[u8], out: &mut Vec<u8>) {
	runs_of!(data, out, u8);
	runs_of!(data, out, u16);
	let log = Rc::new(RefCell::new(Vec::new()));
	let tick = |id: u8| Ticked { id, log: log.clone() };
	let ticks: Vec<Ticked> = data.iter().map(|&id| tick(id)).collect();
	let runs = Runs {
		array: [tick(1), tick(2), tick(3)],
		list: ticks.clone(),
		view: &ticks,
		boxed: Box::new([tick(4)]),
		cow: match data.len() % 2 {
			0 => Cow::Borrowed(&ticks),
			_ => Cow::Owned(vec![tick(7)]),
		},
		pairs: vec![[tick(5), tick(6)]],
		maybe: data.first().map(|&id| vec![tick(!id)]),
	};
	drop(runs);
	let run = match data.len() % 3 {
		0 => Run::Listed(vec![tick(8)]),
		1 => Run::Fixed {
			array: [tick(9), tick(10)],
		},
		_ => Run::Empty,
	};
	drop(run);
	drop(ticks);
	out.extend(log.borrow().iter());
}

/// A read's parked state, as moq-net's `Idle` and tokio's `oneshot::Inner` hold theirs: an
/// `Option` and a `Poll` of its parameter, directly and in a cell, whose `None` and
/// `Pending` each instance decides. Of `()` and `Option<u8>` the `None` is `rs.None`, and
/// of `Poll<u8>` the `Pending` is `rs.OuterPending`.
struct Parked<T> {
	parked: Option<T>,
	ready: Poll<T>,
	value: UnsafeCell<Option<T>>,
}

impl<T: Copy + PartialEq> Parked<T> {
	fn new() -> Self {
		Self {
			parked: None,
			ready: Poll::Pending,
			value: UnsafeCell::new(None),
		}
	}

	/// Parks `read`, returning whether it parked just so already.
	fn park(&mut self, read: T) -> bool {
		let again = self.parked == Some(read);
		self.parked = Some(read);
		again
	}

	/// Readies `value`, returning what was ready before.
	fn ready(&mut self, value: T) -> Poll<T> {
		std::mem::replace(&mut self.ready, Poll::Ready(value))
	}

	/// Sends `value` through the cell, returning what was there, as a oneshot's sender does.
	fn send(&self, value: Option<T>) -> Option<T> {
		// SAFETY: nothing else borrows the cell.
		unsafe { std::mem::replace(&mut *self.value.get(), value) }
	}

	/// Unparks it and takes what the cell holds, leaving `Pending`.
	fn disarm(&mut self) -> (Option<T>, Option<T>) {
		self.ready = Poll::Pending;
		(self.parked.take(), self.value.get_mut().take())
	}
}

/// [`Parked`] of `$items`, each parked, readied, and sent by the bits of a byte.
macro_rules! parked_at {
	($data:expr, $out:expr, $items:expr) => {{
		let items = $items;
		let mut parked = Parked::new();
		for &b in $data {
			let item = items[usize::from(b >> 4) % items.len()];
			match b % 4 {
				0 => $out.push(u8::from(parked.park(item))),
				1 => $out.push(u8::from(parked.ready(item).is_pending())),
				2 => $out.push(u8::from(parked.send((b & 4 != 0).then_some(item)).is_some())),
				_ => {
					let (was, sent) = parked.disarm();
					$out.extend([u8::from(was.is_some()), u8::from(sent.is_some())]);
				}
			}
		}
		let again = parked.parked == Some(items[0]);
		let (was, ready, sent) = (parked.parked, parked.ready, parked.send(None));
		put($out, &format!("{was:?} {ready:?} {sent:?} {again}"));
	}};
}

/// [`Parked`] at `()`, `Option<u8>`, `Poll<u8>`, and `u8`, whose `None` and `Pending` are
/// each a value JS keeps apart from what the instance holds.
pub fn units(data: &[u8], out: &mut Vec<u8>) {
	parked_at!(data, out, [()]);
	parked_at!(data, out, [None, Some(1u8), Some(2)]);
	parked_at!(data, out, [Poll::Pending, Poll::Ready(1u8)]);
	parked_at!(data, out, [3u8, 4]);
}

/// What a fixed buffer holds, as smallvec's `Array` names its arrays' items.
pub trait Store {
	type Item;
	const LEN: usize;
}

impl<T, const N: usize> Store for [T; N] {
	type Item = T;
	const LEN: usize = N;
}

/// A store of flags, whose items are no array's.
struct Flags;

impl Store for Flags {
	type Item = bool;
	const LEN: usize = 8;
}

/// A drain over a store's items, as smallvec's `Drain` holds `<T as Array>::Item`s:
/// directly, in an `Option`, whose `None` each instance decides, and through a slice's
/// iterator.
struct Drained<'a, S: Store> {
	last: Option<S::Item>,
	iter: std::slice::Iter<'a, S::Item>,
	first: S::Item,
}

impl<'a, S: Store> Drained<'a, S>
where
	S::Item: Copy,
{
	fn new(first: S::Item, items: &'a [S::Item]) -> Self {
		Self {
			last: None,
			iter: items.iter(),
			first,
		}
	}

	fn next(&mut self) -> Option<S::Item> {
		let item = *self.iter.next()?;
		self.last = Some(item);
		Some(item)
	}
}

/// A [`Drained`] in another generic type, which names the projection only through it.
struct Batch<'a, S: Store> {
	drain: Drained<'a, S>,
	taken: u8,
}

impl<S: Store> Batch<'_, S>
where
	S::Item: Copy,
{
	fn take(&mut self) -> Option<S::Item> {
		let item = self.drain.next()?;
		self.taken = self.taken.wrapping_add(1);
		Some(item)
	}
}

/// A std iterator's items, which only its `Iterator` impl names, beside the same
/// projection named again; a parameter named for an associated type, so each projection's
/// name is numbered past it.
struct Ahead<Item: Store, I: Iterator> {
	iter: std::iter::Peekable<I>,
	seen: Vec<I::Item>,
	stored: Option<Item::Item>,
}

impl<Item: Store, I: Iterator> Ahead<Item, I>
where
	I::Item: Copy + PartialEq,
{
	/// Takes each item that repeats the one before it.
	fn repeats(&mut self) -> usize {
		while let Some(item) = self.iter.next() {
			if self.iter.peek() == Some(&item) {
				self.seen.push(item);
			}
		}
		self.seen.len()
	}
}

/// A projection in an enum's variants, and in a type that holds itself.
enum Pile<S: Store> {
	Empty,
	Full(S::Item),
	Linked { item: S::Item, next: Box<Pile<S>> },
}

impl<S: Store> Pile<S>
where
	S::Item: Copy,
{
	fn push(self, item: S::Item) -> Self {
		match self {
			Pile::Empty => Pile::Full(item),
			next => Pile::Linked {
				item,
				next: Box::new(next),
			},
		}
	}

	fn items(&self, out: &mut Vec<S::Item>) {
		match self {
			Pile::Empty => {}
			Pile::Full(item) => out.push(*item),
			Pile::Linked { item, next } => {
				out.push(*item);
				next.items(out);
			}
		}
	}
}

/// A projection from a projection, which is one parameter whole.
struct Deep<S: Store>
where
	S::Item: Store,
{
	inner: Option<<S::Item as Store>::Item>,
	len: usize,
}

impl<S: Store> Deep<S>
where
	S::Item: Store,
{
	fn new(inner: Option<<S::Item as Store>::Item>) -> Self {
		Self {
			inner,
			len: S::LEN * <S::Item as Store>::LEN,
		}
	}
}

/// A store of [`Ticked`]s, which drops nothing itself.
struct Ticks;

impl Store for Ticks {
	type Item = Ticked;
	const LEN: usize = 1;
}

/// Items that drop beside the store that names them, which drops nothing, so only the
/// items' type decides whether and how each instance drops.
struct Kept<S: Store> {
	item: S::Item,
	store: S,
	rest: Vec<S::Item>,
	maybe: Option<S::Item>,
}

/// A store that lends [`Ticked`]s, whose items drop nothing.
struct Lent<'a>(std::marker::PhantomData<&'a ()>);

impl<'a> Store for Lent<'a> {
	type Item = &'a Ticked;
	const LEN: usize = 1;
}

/// A store of what translation types no other way, which only a `PhantomData` names.
struct Paths;

impl Store for Paths {
	type Item = std::path::PathBuf;
	const LEN: usize = 0;
}

/// A projection that only a `PhantomData` holds, which types nothing, in an `Option` too.
struct Marked<S: Store> {
	marker: std::marker::PhantomData<S::Item>,
	maybe: std::marker::PhantomData<Option<S::Item>>,
	count: u8,
}

/// [`Drained`] and [`Batch`] of `$store`, of the items `$item` makes of each byte.
macro_rules! drained_at {
	($data:expr, $out:expr, $store:ty, $item:expr) => {{
		let items: Vec<<$store as Store>::Item> = $data.iter().map($item).collect();
		let mut drain = Drained::<$store>::new($item(&0x5a), &items);
		let mut count = 0usize;
		while drain.next().is_some() {
			count += 1;
		}
		let mut batch = Batch {
			drain: Drained::<$store>::new($item(&1), &items[items.len() / 2..]),
			taken: 0,
		};
		let half = batch.take();
		put(
			$out,
			&format!(
				"{:?} {:?} {count} {} {half:?} {:?} {}",
				drain.last,
				drain.first,
				<$store>::LEN,
				batch.drain.last,
				batch.taken
			),
		);
	}};
}

/// Generic types whose fields are typed by an associated type of their parameters, at
/// arrays of bytes, of `u16`s, of `()`, whose `None` is `rs.None`, and of `Option<u8>`s,
/// and at a store that is no array.
pub fn projections(data: &[u8], out: &mut Vec<u8>) {
	drained_at!(data, out, [u8; 4], |b: &u8| *b);
	drained_at!(data, out, [u16; 2], |b: &u8| u16::from(*b) * 257);
	drained_at!(data, out, [(); 3], |_: &u8| ());
	drained_at!(data, out, [Option<u8>; 1], |b: &u8| (b % 3 != 0).then_some(*b));
	drained_at!(data, out, Flags, |b: &u8| b & 1 != 0);

	let mut ahead = Ahead::<Flags, _> {
		iter: data.iter().peekable(),
		seen: Vec::new(),
		stored: data.first().map(|b| b & 2 != 0),
	};
	let repeats = ahead.repeats();
	put(out, &format!("{repeats} {:?} {:?}", ahead.seen, ahead.stored));
	let mut ahead = Ahead::<[u16; 1], _> {
		iter: data.iter().map(|b| b % 4).peekable(),
		seen: Vec::new(),
		stored: data.last().map(|&b| u16::from(b)),
	};
	let repeats = ahead.repeats();
	put(out, &format!("{repeats} {:?} {:?}", ahead.seen, ahead.stored));

	let mut pile = Pile::<[u8; 2]>::Empty;
	let mut flags = Pile::<Flags>::Empty;
	for &b in data {
		pile = pile.push(b);
		flags = flags.push(b > 0x7f);
	}
	let (mut bytes, mut bits) = (Vec::new(), Vec::new());
	pile.items(&mut bytes);
	flags.items(&mut bits);
	put(out, &format!("{bytes:?} {bits:?}"));

	let deep = Deep::<[[u8; 2]; 3]>::new(data.first().copied());
	let units = Deep::<[[(); 2]; 4]>::new(data.get(1).map(|_| ()));
	put(
		out,
		&format!("{:?} {} {:?} {}", deep.inner, deep.len, units.inner, units.len),
	);

	let log = Rc::new(RefCell::new(Vec::new()));
	let tick = |id: u8| Ticked { id, log: log.clone() };
	for &b in data {
		let kept = Kept::<Ticks> {
			item: tick(b),
			store: Ticks,
			rest: vec![tick(b ^ 1), tick(b ^ 2)],
			maybe: (b % 2 == 0).then(|| tick(b ^ 3)),
		};
		drop(kept);
		let mut ticks = Pile::<Ticks>::Empty;
		for id in 0..b % 3 {
			ticks = Pile::Linked {
				item: tick(id),
				next: Box::new(ticks),
			};
		}
		log.borrow_mut().push(0xfe);
		drop(ticks);
		let kept = Kept::<[u8; 1]> {
			item: b,
			store: [b],
			rest: vec![b],
			maybe: None,
		};
		log.borrow_mut().extend([kept.item, kept.store[0], kept.rest[0]]);
		// What lends its items drops none of them, behind an `Rc` too.
		let owner = tick(b ^ 4);
		let lent = Rc::new(Kept::<Lent<'_>> {
			item: &owner,
			store: Lent(std::marker::PhantomData),
			rest: vec![&owner],
			maybe: (b % 2 == 1).then_some(&owner),
		});
		log.borrow_mut().push(lent.item.id);
		drop(lent);
		log.borrow_mut().push(0xfd);
		drop(owner);
	}
	let marked = Marked::<Paths> {
		marker: std::marker::PhantomData,
		maybe: std::marker::PhantomData,
		count: data.len() as u8,
	};
	let bytes = Marked::<[u8; 2]> {
		marker: std::marker::PhantomData,
		maybe: std::marker::PhantomData,
		count: !marked.count,
	};
	log.borrow_mut().extend([marked.count, bytes.count]);
	out.extend(log.borrow().iter());
}

/// One branch of a [`Tree`], whose projection only the tree's other type names, typed with
/// it, so each learns its projections from the other.
struct Branch<S: Store> {
	item: S::Item,
	below: Option<Box<Tree<S>>>,
}

/// Branches that each may hold a tree again.
struct Tree<S: Store> {
	branches: Vec<Branch<S>>,
}

impl<S: Store> Tree<S>
where
	S::Item: Copy,
{
	fn items(&self, out: &mut Vec<S::Item>) {
		for branch in &self.branches {
			out.push(branch.item);
			if let Some(below) = &branch.below {
				below.items(out);
			}
		}
	}
}

/// Two types typed with each other, each naming a projection the other doesn't, so each
/// takes both, in the order it named them.
enum Even<S: Store, I: Iterator> {
	End,
	Next(S::Item, Box<Odd<S, I>>),
}

struct Odd<S: Store, I: Iterator> {
	seen: Option<I::Item>,
	next: Even<S, I>,
}

impl<S: Store, I: Iterator> Even<S, I>
where
	S::Item: Copy + std::fmt::Debug,
	I::Item: Copy + std::fmt::Debug,
{
	fn walk(&self, out: &mut String) {
		if let Even::Next(item, odd) = self {
			out.push_str(&format!("{item:?} {:?} ", odd.seen));
			odd.next.walk(out);
		}
	}
}

/// A type that holds itself with its parameters swapped, so its projections at the inner
/// instance are its own in the other order.
struct Swap<A: Store, B: Store> {
	item: A::Item,
	next: Option<Box<Swap<B, A>>>,
}

impl<A: Store, B: Store> Swap<A, B>
where
	A::Item: std::fmt::Debug,
	B::Item: std::fmt::Debug,
{
	fn show(&self, out: &mut String) {
		out.push_str(&format!("{:?} ", self.item));
		if let Some(next) = &self.next {
			next.show(out);
		}
	}
}

/// A projection beside an `Option` of it only a `PhantomData` holds, which types nothing, so
/// no instance is refused for what that `Option` would erase to.
struct Ghost<S: Store> {
	item: S::Item,
	marker: std::marker::PhantomData<Option<S::Item>>,
}

/// Types typed with themselves, or with each other, that name projections.
pub fn projected_cycles(data: &[u8], out: &mut Vec<u8>) {
	let tree = data
		.chunks(2)
		.fold(Tree::<[u8; 1]> { branches: Vec::new() }, |below, chunk| Tree {
			branches: chunk
				.iter()
				.map(|&item| Branch { item, below: None })
				.chain([Branch {
					item: 0xff,
					below: Some(Box::new(below)),
				}])
				.collect(),
		});
	let mut items = Vec::new();
	tree.items(&mut items);
	put(out, &format!("{items:?}"));

	let mut seen = data.iter().map(|b| b % 5);
	let mut even = Even::<Flags, std::vec::IntoIter<u8>>::End;
	for &b in data.iter().rev() {
		even = Even::Next(
			b & 1 != 0,
			Box::new(Odd {
				seen: seen.next(),
				next: even,
			}),
		);
	}
	let mut walked = String::new();
	even.walk(&mut walked);
	put(out, &walked);

	let mut swap = None::<Box<Swap<[u16; 1], Flags>>>;
	for (i, &b) in data.iter().enumerate().rev() {
		let inner = Swap::<Flags, [u16; 1]> {
			item: b > 0x40,
			next: swap.take(),
		};
		if i % 2 == 0 {
			swap = Some(Box::new(Swap {
				item: u16::from(b) << 4,
				next: Some(Box::new(inner)),
			}));
		}
	}
	let mut shown = String::new();
	if let Some(swap) = &swap {
		swap.show(&mut shown);
	}
	put(out, &shown);

	let ghost = Ghost::<[Option<()>; 2]> {
		item: data.first().map(|_| ()),
		marker: std::marker::PhantomData,
	};
	put(out, &format!("{:?}", ghost.item));
}
