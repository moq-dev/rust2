//! Methods on generic structs and enums, each instance its own method.

use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;

use crate::fmt::{Pair, put};
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
