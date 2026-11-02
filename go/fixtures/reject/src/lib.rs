//! Constructs outside the subset: each root fails translation with a span, and no
//! output is written.

#![allow(dead_code, reason = "each item is a translation root, not called from Rust")]

pub fn generic<T>(t: T) -> T {
	t
}

/// Go has no 128-bit integer.
pub fn wide(a: u64, b: u64) -> u128 {
	a as u128 * b as u128
}

struct Logged(u8);

impl Drop for Logged {
	fn drop(&mut self) {}
}

/// A `Drop` impl runs where Rust drops the value, which Go has no place for yet.
pub fn dropped(v: u8) -> u8 {
	let logged = Logged(v);
	logged.0
}

pub fn dynamic(f: &mut dyn Iterator<Item = u8>) -> Option<u8> {
	f.next()
}

union Bits {
	int: u32,
	float: f32,
}

pub fn unioned(v: u32) -> f32 {
	unsafe { Bits { int: v }.float }
}

/// An array's by-value iterator is generic over its length.
pub fn owned(v: [u8; 4]) -> u32 {
	let mut sum = 0;
	for b in v {
		sum += u32::from(b);
	}
	sum
}

pub fn float(a: f32) -> f32 {
	a * 2.0
}

/// Go's growth is not Rust's, so a capacity read back would differ.
pub fn capacity() -> usize {
	Vec::<u8>::with_capacity(4).capacity()
}

/// A recursive type whose drop runs no `Drop` impl, which translates beside those refused.
pub struct Node {
	next: Option<Box<Node>>,
	value: u8,
}

pub fn linked(value: u8) -> u8 {
	let node = Node { next: None, value };
	node.value
}

/// Go compares a slice's pointer and length not at all.
pub fn compared(a: *const [u8], b: *const [u8]) -> bool {
	a == b
}

/// Go orders no pointers.
pub fn ordered(a: *const u8, b: *const u8) -> bool {
	a < b
}

/// One constant's array and its second byte, which Go's two variables could not alias.
const PAIR: (&[u8; 3], &u8) = {
	let a = b"abc";
	(a, &a[1])
};

pub fn aliased() -> bool {
	let (a, b) = PAIR;
	std::ptr::eq(&a[1], b)
}
