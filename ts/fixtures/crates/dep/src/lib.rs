//! The dependency `crates-fixture` translates with: a type the package copies, which only
//! this crate writes into, a generic function over the package's type, drops, an iterator,
//! `unsafe` with a safe twin where it translates and without one, and a trait for objects.

use std::sync::{Arc, Mutex};

#[derive(Clone, Copy)]
pub struct Point {
	pub x: i32,
	pub y: i32,
}

impl Point {
	/// Move it by `(dx, dy)`: the only write into a `Point` in either crate.
	pub fn shift(&mut self, dx: i32, dy: i32) {
		self.x += dx;
		self.y += dy;
	}
}

/// What [`heaviest`] ranks by.
pub trait Weigh {
	fn weight(&self) -> u32;
}

/// The heaviest of `items`, the first of equals.
pub fn heaviest<T: Weigh>(items: &[T]) -> Option<&T> {
	let mut best: Option<&T> = None;
	for item in items {
		match best {
			Some(b) if b.weight() >= item.weight() => {}
			_ => best = Some(item),
		}
	}
	best
}

/// Notes its id in a log when it drops.
pub struct Guard {
	id: u8,
	log: Arc<Mutex<Vec<u8>>>,
}

impl Guard {
	pub fn new(id: u8, log: &Arc<Mutex<Vec<u8>>>) -> Self {
		Self { id, log: log.clone() }
	}
}

impl Drop for Guard {
	fn drop(&mut self) {
		self.log.lock().unwrap().push(self.id);
	}
}

/// Counts down from where it starts to one.
pub struct Countdown {
	next: u8,
	last: u8,
}

impl Countdown {
	pub fn new(from: u8) -> Self {
		Self { next: from, last: 0 }
	}
}

impl Iterator for Countdown {
	type Item = u8;

	fn next(&mut self) -> Option<u8> {
		if self.next == self.last {
			return None;
		}
		self.next -= 1;
		Some(self.next + 1)
	}
}

#[cfg(feature = "rev")]
impl DoubleEndedIterator for Countdown {
	fn next_back(&mut self) -> Option<u8> {
		if self.next == self.last {
			return None;
		}
		self.last += 1;
		Some(self.last)
	}
}

/// Writes into the arrays it is given, where a binding stands in for it.
pub struct Slots {
	base: i16,
}

impl Slots {
	pub fn with_base(base: i16) -> Self {
		Self { base }
	}

	/// Fill `out` from the base: the only write into a `[i16; 2]`.
	pub fn fill(&self, out: &mut [i16; 2]) {
		*out = [self.base, self.base - 1];
	}
}

/// The first byte of `bytes`.
pub fn first(bytes: &[u8]) -> u8 {
	assert!(!bytes.is_empty(), "no first byte");
	read(bytes)
}

#[cfg(not(rust2_ts))]
fn read(bytes: &[u8]) -> u8 {
	// SAFETY: `first` checked it holds a byte.
	unsafe { *bytes.as_ptr() }
}

/// `read`'s safe twin, which translates.
#[cfg(rust2_ts)]
fn read(bytes: &[u8]) -> u8 {
	bytes[0]
}

/// The last byte of `bytes`, which only a binding translates.
pub fn last(bytes: &[u8]) -> u8 {
	assert!(!bytes.is_empty(), "no last byte");
	// SAFETY: it holds a byte.
	unsafe { *bytes.as_ptr().byte_add(bytes.len() - 1) }
}

/// What [`tally`] counts with, through trait objects of either crate's types.
pub trait Tally {
	fn add(&mut self, by: u8);
	fn count(&self) -> u32;
	/// Provided, and overridden by one impl.
	fn scale(&self) -> u32 {
		1
	}
}

/// A number, which JS copies, so `&mut self` reaches it through a place.
impl Tally for u32 {
	fn add(&mut self, by: u8) {
		*self = self.wrapping_add(u32::from(by));
	}
	fn count(&self) -> u32 {
		*self
	}
}

impl Tally for Guard {
	fn add(&mut self, by: u8) {
		self.id = self.id.wrapping_add(by);
	}
	fn count(&self) -> u32 {
		u32::from(self.id)
	}
	fn scale(&self) -> u32 {
		2
	}
}

/// Add `by` to each of `tallies`, then sum their counts by their scales.
pub fn tally(tallies: &mut [&mut dyn Tally], by: u8) -> u32 {
	let mut sum = 0u32;
	for i in 0..tallies.len() {
		let t = &mut *tallies[i];
		t.add(by);
		sum = sum.wrapping_add(t.count().wrapping_mul(t.scale()));
	}
	sum
}
