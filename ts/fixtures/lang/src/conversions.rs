//! std's functions that take a reference and convert, copy, or measure what it points at:
//! `to_owned`, `clone_into`, and `clone_from` into a local, a field, an element, and a
//! parameter; `binary_search` over runs of equal elements; `size_of_val` and
//! `align_of_val`; slices, arrays, and cells of one value, written through; arrays as
//! slices by `AsRef`, `AsMut`, `Borrow`, and `BorrowMut`; and raw pointers of references.
//! Each is generic over the value, run for numbers JS copies, strings, `Option`s, arrays
//! of bytes and of numbers, a struct, and one that logs its clones and drops.

use std::borrow::{Borrow, BorrowMut};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A value made from the case's bytes, put back as bytes.
trait Value: Clone {
	/// The value `data` makes from `at`.
	fn make(data: &[u8], at: usize) -> Self;

	/// Its bytes.
	fn put(&self, out: &mut Vec<u8>);
}

/// The byte at `at`, past `data`'s end its index.
fn byte(data: &[u8], at: usize) -> u8 {
	data.get(at).copied().unwrap_or(at as u8)
}

impl Value for u8 {
	fn make(data: &[u8], at: usize) -> Self {
		byte(data, at)
	}

	fn put(&self, out: &mut Vec<u8>) {
		out.push(*self);
	}
}

impl Value for u32 {
	fn make(data: &[u8], at: usize) -> Self {
		u32::from(byte(data, at)) * 0x0101_0101
	}

	fn put(&self, out: &mut Vec<u8>) {
		out.extend(self.to_be_bytes());
	}
}

/// Past 2^53, where the runtime holds it as limbs.
impl Value for u64 {
	fn make(data: &[u8], at: usize) -> Self {
		u64::from(byte(data, at)) << 56 | u64::from(byte(data, at + 1))
	}

	fn put(&self, out: &mut Vec<u8>) {
		out.extend(self.to_be_bytes());
	}
}

impl Value for String {
	fn make(data: &[u8], at: usize) -> Self {
		format!("s{}", byte(data, at) % 16)
	}

	fn put(&self, out: &mut Vec<u8>) {
		out.extend(self.as_bytes());
		out.push(0xff);
	}
}

impl Value for Option<u16> {
	fn make(data: &[u8], at: usize) -> Self {
		let b = byte(data, at);
		(b % 4 != 0).then_some(u16::from(b) << 4)
	}

	fn put(&self, out: &mut Vec<u8>) {
		match self {
			Some(x) => out.extend(x.to_be_bytes()),
			None => out.push(0xff),
		}
	}
}

impl<const N: usize> Value for [u8; N] {
	fn make(data: &[u8], at: usize) -> Self {
		std::array::from_fn(|i| byte(data, at + i) % 4)
	}

	fn put(&self, out: &mut Vec<u8>) {
		out.extend_from_slice(self);
	}
}

impl Value for [u32; 2] {
	fn make(data: &[u8], at: usize) -> Self {
		[u32::from(byte(data, at) % 4), u32::from(byte(data, at + 1)) << 20]
	}

	fn put(&self, out: &mut Vec<u8>) {
		for x in self {
			out.extend(x.to_be_bytes());
		}
	}
}

/// A struct, which JS holds as an object.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Named {
	id: u16,
	name: String,
}

impl Value for Named {
	fn make(data: &[u8], at: usize) -> Self {
		Named {
			id: u16::from(byte(data, at) % 4),
			name: String::make(data, at + 1),
		}
	}

	fn put(&self, out: &mut Vec<u8>) {
		out.extend(self.id.to_be_bytes());
		self.name.put(out);
	}
}

/// What [`Logged`]s log to.
type Log = Rc<RefCell<Vec<u8>>>;

thread_local! {
	static LOG: Log = Rc::new(RefCell::new(Vec::new()));
}

/// Logs its id as it drops, and as it is cloned, the id with the high bit set: its clone's
/// id is one more.
struct Logged {
	id: u8,
	log: Log,
}

impl Clone for Logged {
	fn clone(&self) -> Self {
		RefCell::borrow_mut(&self.log).push(0x80 | self.id);
		Self {
			id: self.id.wrapping_add(1),
			log: self.log.clone(),
		}
	}
}

impl Drop for Logged {
	fn drop(&mut self) {
		RefCell::borrow_mut(&self.log).push(self.id);
	}
}

impl Value for Logged {
	fn make(data: &[u8], at: usize) -> Self {
		Logged {
			id: byte(data, at) & 0x7f,
			log: LOG.with(Rc::clone),
		}
	}

	fn put(&self, out: &mut Vec<u8>) {
		out.push(self.id);
		// What was logged so far, after a marker.
		out.push(0xfd);
		out.extend(RefCell::borrow_mut(&self.log).drain(..));
	}
}

/// Holds a value in a field.
struct Held<T> {
	value: T,
}

/// Runs each conversion of the type `kind` picks, but for `u8`, whose `&mut` no slice can
/// view, and [`searched`] of those that order.
pub fn conversions(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	match kind % 12 {
		0 => ordered::<u32>(data, out),
		1 => ordered::<u64>(data, out),
		2 => ordered::<String>(data, out),
		3 => ordered::<Option<u16>>(data, out),
		4 => ordered::<[u8; 0]>(data, out),
		5 => ordered::<[u8; 1]>(data, out),
		6 => ordered::<[u8; 3]>(data, out),
		7 => ordered::<[u8; 40]>(data, out),
		8 => ordered::<[u32; 2]>(data, out),
		9 => ordered::<Named>(data, out),
		10 => {
			converted::<u8>(data, out);
			searched::<u8>(data, out);
		}
		_ => {
			// What an earlier case dropped at its end, which JS runs on after.
			LOG.with(|log| RefCell::borrow_mut(log).clear());
			converted::<Logged>(data, out);
			lent_one::<Logged>(data, out);
		}
	}
	measured(data, out);
	pointed(data, out);
}

/// Each conversion of a `T` that orders.
fn ordered<T: Value + Ord>(data: &[u8], out: &mut Vec<u8>) {
	converted::<T>(data, out);
	lent_one::<T>(data, out);
	searched::<T>(data, out);
}

/// `clone_from` through a `&mut` whose place only the caller knows.
fn clone_from_param<T: Value>(to: &mut T, from: &T) {
	to.clone_from(from);
}

/// `clone_into` through a `&mut` whose place only the caller knows.
fn clone_into_param<T: Value>(from: &T, to: &mut T) {
	from.clone_into(to);
}

/// A whole write into what a slice holds, as a function given any slice makes it.
fn write_first<T: Value>(s: &mut [T], value: T) -> T {
	let old = std::mem::replace(&mut s[0], value.clone());
	s[0] = value;
	old
}

/// A cell of what a `&mut` whose place only the caller knows points at, written through.
fn set_through<T: Value>(to: &mut T, value: T) -> T {
	Cell::from_mut(to).replace(value)
}

/// Each conversion of a `T`, its copies put after a write to what they copied.
fn converted<T: Value>(data: &[u8], out: &mut Vec<u8>) {
	let mut a = T::make(data, 0);
	let b = T::make(data, 3);
	// A copy shares nothing with what it copied.
	let owned = a.to_owned();
	a.clone_from(&b);
	owned.put(out);
	a.put(out);
	let mut c = T::make(data, 6);
	owned.clone_into(&mut c);
	c.put(out);
	clone_from_param(&mut c, &b);
	clone_into_param(&owned, &mut a);
	c.put(out);
	a.put(out);
	// Through a field and an element.
	let mut held = Held {
		value: T::make(data, 1),
	};
	held.value.clone_from(&c);
	let mut v = vec![T::make(data, 2), T::make(data, 4)];
	v[1].clone_from(&held.value);
	b.clone_into(&mut v[0]);
	held.value.put(out);
	v[0].put(out);
	v[1].put(out);
	// Read, and its clone a value of its own.
	let one = std::array::from_ref(&b);
	out.push(one.len() as u8);
	let copied = one[0].clone();
	one[0].put(out);
	a.clone_from(&copied);
	copied.put(out);
	a.put(out);
	// A cell of a local and of what a parameter points at, its old value read out.
	let cell = Cell::from_mut(&mut c);
	let old = cell.replace(T::make(data, 10));
	cell.set(T::make(data, 11));
	old.put(out);
	c.put(out);
	set_through(&mut v[0], T::make(data, 12)).put(out);
	v[0].put(out);
	Cell::from_mut(&mut held.value).set(T::make(data, 17));
	held.value.put(out);
	// Arrays as slices.
	let mut array = [T::make(data, 13), T::make(data, 14), T::make(data, 15)];
	let lens = [AsRef::<[T]>::as_ref(&array).len(), Borrow::<[T]>::borrow(&array).len()];
	out.extend(lens.map(|n| n as u8));
	AsMut::<[T]>::as_mut(&mut array)[0] = b.clone();
	BorrowMut::<[T]>::borrow_mut(&mut array)[2].clone_from(&c);
	for x in &array {
		x.put(out);
	}
	// A pointer, read and written through.
	let p = std::ptr::from_ref(&b);
	unsafe { (*p).put(out) };
	let q = std::ptr::from_mut(&mut c);
	unsafe { *q = T::make(data, 16) };
	c.put(out);
}

/// Slices and arrays of one value of a local, a field, and an element, written through as
/// a whole, the old value read out, and through std's slice methods.
fn lent_one<T: Value>(data: &[u8], out: &mut Vec<u8>) {
	let (mut a, b, mut c) = (T::make(data, 0), T::make(data, 3), T::make(data, 6));
	let mut held = Held {
		value: T::make(data, 1),
	};
	let mut v = vec![T::make(data, 2), T::make(data, 4)];
	let s = std::slice::from_mut(&mut a);
	out.push(s.len() as u8);
	let old = std::mem::replace(&mut s[0], T::make(data, 5));
	old.put(out);
	a.put(out);
	let old = write_first(std::slice::from_mut(&mut held.value), T::make(data, 7));
	old.put(out);
	held.value.put(out);
	let old = write_first(std::array::from_mut(&mut v[1]), T::make(data, 8));
	old.put(out);
	v[1].put(out);
	// Through std's slice methods.
	std::slice::from_mut(&mut c).fill(b.clone());
	for x in std::array::from_mut(&mut a) {
		*x = T::make(data, 9);
	}
	c.put(out);
	a.put(out);
}

/// `binary_search` of each value `data` makes in a sorted run of them, where equal ones
/// repeat: `Ok` or `Err` as a byte, then the index.
fn searched<T: Value + Ord>(data: &[u8], out: &mut Vec<u8>) {
	let mut v: Vec<T> = (0..data.len().min(24)).map(|i| T::make(data, i)).collect();
	v.sort();
	for n in [0, 1, 2, 3, v.len()] {
		let s = &v[..n.min(v.len())];
		for i in 0..data.len().min(8) + 1 {
			let found = s.binary_search(&T::make(data, i * 2 + 1));
			let (tag, at) = match found {
				Ok(at) => (1, at),
				Err(at) => (0, at),
			};
			out.extend([tag, at as u8]);
		}
	}
}

/// What wasm32 lays out each value in, and a slice or a `str` of what `data` holds.
fn measured(data: &[u8], out: &mut Vec<u8>) {
	let n = data.len();
	let wide: Vec<u64> = data.iter().map(|&b| u64::from(b)).collect();
	// Past ASCII, where a `char` takes two bytes.
	let text: String = data.iter().map(|&b| char::from(b)).collect();
	let named = Named::make(data, 0);
	let sizes = [
		(size_of_val(&data[0..0]), align_of_val(&data[0..0])),
		(size_of_val(data), align_of_val(data)),
		(size_of_val(&wide[..]), align_of_val(&wide[..])),
		(size_of_val(&*text), align_of_val(&*text)),
		(size_of_val(&n), align_of_val(&n)),
		(size_of_val(&(n as u128)), align_of_val(&(n as u128))),
		(size_of_val(&(n as u8, n as u16)), align_of_val(&(n as u8, n as u16))),
		(size_of_val(&Some(n as u32)), align_of_val(&Some(n as u32))),
		(size_of_val(&[n as u16; 3]), align_of_val(&[n as u16; 3])),
		(size_of_val(&named), align_of_val(&named)),
		(size_of_val(&text), align_of_val(&text)),
		(size_of_val(&()), align_of_val(&())),
	];
	for (size, align) in sizes {
		out.extend([size as u8, align as u8]);
	}
}

/// Raw pointers of references to the bytes of arrays and to objects, compared by where
/// they point, and an array of an `u8`.
fn pointed(data: &[u8], out: &mut Vec<u8>) {
	let (mut a, b) = (<[u8; 2]>::make(data, 0), <[u8; 2]>::make(data, 2));
	let (x, y) = (Named::make(data, 0), Named::make(data, 3));
	let same = [
		std::ptr::eq(std::ptr::from_ref(&a), &a),
		std::ptr::eq(std::ptr::from_ref(&a), &b),
		std::ptr::eq(std::ptr::from_ref(&x), &x),
		std::ptr::eq(std::ptr::from_ref(&x), &y),
	];
	out.extend(same.map(u8::from));
	let p = std::ptr::from_mut(&mut a);
	unsafe { (*p)[0] ^= 0x10 };
	out.extend_from_slice(&a);
	let one = std::array::from_ref(&a[1]);
	out.extend_from_slice(one);
}
