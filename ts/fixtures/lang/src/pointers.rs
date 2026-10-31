//! Raw pointers: a circular list of boxed nodes as `hashlink`'s `LinkedHashMap` keeps one,
//! linked through a union and holding a `MaybeUninit` entry, with a free list of nodes to
//! reuse; pointers to numbers, to fields, and to whole values, read, written, swapped,
//! and compared; pointers to the parts of a struct kept as objects; the pointers
//! `UnsafeCell::get` gives; slots of fresh memory written whole before any pointer
//! into them is taken, by index or through std's iterators over them; and pointers into
//! nodes that an `AtomicPtr` and memory hold pointers to.

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::ptr::{self, NonNull};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::atomic::{AtomicBool, AtomicPtr};

struct Links<V> {
	next: NonNull<Node<V>>,
	prev: NonNull<Node<V>>,
}

impl<V> Clone for Links<V> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<V> Copy for Links<V> {}

struct Free<V> {
	next: Option<NonNull<Node<V>>>,
}

impl<V> Clone for Free<V> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<V> Copy for Free<V> {}

/// A node's links while it is in the list, or while it is free.
union Link<V> {
	value: Links<V>,
	free: Free<V>,
}

struct Node<V> {
	entry: MaybeUninit<(u8, V)>,
	link: Link<V>,
}

/// Entries in the order they were pushed, but for those moved to the front.
struct List<V> {
	/// The guard: `next` is the first entry and `prev` the last, or itself if none.
	guard: Option<NonNull<Node<V>>>,
	free: Option<NonNull<Node<V>>>,
	len: usize,
}

impl<V> List<V> {
	fn new() -> Self {
		List {
			guard: None,
			free: None,
			len: 0,
		}
	}

	fn guard(&mut self) -> NonNull<Node<V>> {
		if let Some(guard) = self.guard {
			return guard;
		}
		let mut guard = unsafe { NonNull::new_unchecked(Box::into_raw(Box::new(Node::new()))) };
		unsafe {
			guard.as_mut().link.value = Links {
				next: guard,
				prev: guard,
			}
		};
		self.guard = Some(guard);
		guard
	}

	fn alloc(&mut self) -> NonNull<Node<V>> {
		match self.free {
			Some(free) => {
				self.free = unsafe { free.as_ref().link.free.next };
				free
			}
			None => NonNull::from(Box::leak(Box::new(Node::new()))),
		}
	}

	fn push_back(&mut self, key: u8, value: V) {
		let guard = self.guard();
		let mut node = self.alloc();
		unsafe {
			node.as_mut().entry.as_mut_ptr().write((key, value));
			attach_before(node, guard);
		}
		self.len += 1;
	}

	fn find(&self, key: u8) -> Option<NonNull<Node<V>>> {
		let guard = self.guard?;
		let mut at = unsafe { guard.as_ref().link.value.next };
		while at != guard {
			unsafe {
				if (*at.as_ptr()).entry.assume_init_ref().0 == key {
					return Some(at);
				}
				at = at.as_ref().link.value.next;
			}
		}
		None
	}

	fn remove(&mut self, key: u8) -> Option<(u8, V)> {
		let mut node = self.find(key)?;
		unsafe {
			detach(node);
			let entry = node.as_mut().entry.as_ptr().read();
			node.as_mut().link.free = Free { next: self.free };
			self.free = Some(node);
			self.len -= 1;
			Some(entry)
		}
	}

	fn to_front(&mut self, key: u8) -> bool {
		let Some(node) = self.find(key) else {
			return false;
		};
		unsafe {
			let Some(guard) = self.guard else { return false };
			detach(node);
			attach_before(node, guard.as_ref().link.value.next);
		}
		true
	}

	fn get_mut(&mut self, key: u8) -> Option<&mut V> {
		let node = self.find(key)?;
		unsafe { Some(&mut (*(*node.as_ptr()).entry.as_mut_ptr()).1) }
	}
}

impl List<String> {
	fn each(&self, out: &mut Vec<u8>) {
		let Some(guard) = self.guard else {
			return;
		};
		let mut at = unsafe { guard.as_ref().link.value.next };
		while at != guard {
			let (key, value) = unsafe { at.as_ref().entry.assume_init_ref() };
			out.push(*key);
			out.extend_from_slice(value.as_bytes());
			at = unsafe { at.as_ref().link.value.next };
		}
	}
}

impl<V> Node<V> {
	fn new() -> Self {
		Node {
			entry: MaybeUninit::uninit(),
			link: Link {
				value: Links {
					next: NonNull::dangling(),
					prev: NonNull::dangling(),
				},
			},
		}
	}
}

/// Put `node` in the list just before `at`.
unsafe fn attach_before<V>(mut node: NonNull<Node<V>>, mut at: NonNull<Node<V>>) {
	unsafe {
		node.as_mut().link.value = Links {
			prev: at.as_ref().link.value.prev,
			next: at,
		};
		at.as_mut().link.value.prev = node;
		(*node.as_ref().link.value.prev.as_ptr()).link.value.next = node;
	}
}

unsafe fn detach<V>(mut node: NonNull<Node<V>>) {
	unsafe {
		node.as_mut().link.value.prev.as_mut().link.value.next = node.as_ref().link.value.next;
		node.as_mut().link.value.next.as_mut().link.value.prev = node.as_ref().link.value.prev;
	}
}

impl<V> Drop for List<V> {
	fn drop(&mut self) {
		if let Some(guard) = self.guard {
			unsafe {
				let mut at = guard.as_ref().link.value.next;
				while at != guard {
					let next = at.as_ref().link.value.next;
					let mut node = Box::from_raw(at.as_ptr());
					node.entry.assume_init_drop();
					at = next;
				}
				drop(Box::from_raw(guard.as_ptr()));
			}
		}
		while let Some(free) = self.free {
			unsafe {
				self.free = free.as_ref().link.free.next;
				drop(Box::from_raw(free.as_ptr()));
			}
		}
	}
}

/// Steps on a list of the operation and key each pair of bytes picks, putting what each
/// finds and then the list in order.
pub fn lists(data: &[u8], out: &mut Vec<u8>) {
	let mut list = List::<String>::new();
	let mut data = data;
	while let [op, key, rest @ ..] = data {
		data = rest;
		let (op, key) = (*op, *key % 8);
		match op % 4 {
			0 => list.push_back(key, String::from(["", "v", "vv"][usize::from(op % 3)])),
			1 => match list.remove(key) {
				Some((k, v)) => {
					out.push(k);
					out.extend_from_slice(v.as_bytes());
				}
				None => out.push(0xff),
			},
			2 => out.push(list.to_front(key) as u8),
			_ => match list.get_mut(key) {
				Some(value) => value.push('w'),
				None => out.push(0xfe),
			},
		}
	}
	out.push(list.len as u8);
	list.each(out);
}

/// Steps on a list of shared counts, putting how many hold the one each push clones
/// while the list lives, and after it drops.
pub fn shared(data: &[u8], out: &mut Vec<u8>) {
	let held = Arc::new(0u8);
	{
		let mut list = List::<Arc<u8>>::new();
		let mut data = data;
		while let [op, key, rest @ ..] = data {
			data = rest;
			let (op, key) = (*op, *key % 4);
			match op % 3 {
				0 => list.push_back(key, Arc::clone(&held)),
				1 => out.push(list.remove(key).map_or(0xff, |(_, v)| *v)),
				_ => out.push(list.to_front(key) as u8),
			}
			out.push(Arc::strong_count(&held) as u8);
		}
	}
	out.push(Arc::strong_count(&held) as u8);
}

#[derive(Clone, Copy)]
struct Pair {
	a: u32,
	b: u32,
}

/// Pointers to numbers, to a field, and to a whole struct: read, written, swapped,
/// replaced, compared, and null.
pub fn places(data: &[u8], out: &mut Vec<u8>) {
	let mut x = 0u32;
	let mut y = 1u32;
	let mut pair = Pair { a: 2, b: 3 };
	let px = &raw mut x;
	let py = &raw mut y;
	let pa = &raw mut pair.a;
	let pp = &raw mut pair;
	let mut held: *const u32 = ptr::null();
	for &byte in data {
		let v = u32::from(byte);
		unsafe {
			match byte % 9 {
				0 => *px += v,
				1 => py.write(v),
				2 => ptr::swap(px, py),
				3 => out.push(ptr::replace(pa, v) as u8),
				4 => (*pp).b = v,
				5 => {
					let old = pp.read();
					pp.write(Pair { a: old.b, b: old.a });
				}
				6 => held = if byte & 1 == 0 { px } else { pa },
				7 => out.push(held.is_null() as u8 | ((pp == &raw mut pair) as u8) << 1),
				_ => out.push(if held.is_null() { 0xff } else { *held as u8 }),
			}
		}
	}
	out.extend_from_slice(&[x as u8, y as u8, pair.a as u8, pair.b as u8]);
}

/// Unchecked indexing, a boxed number behind a pointer, a `MaybeUninit` number, and a
/// value dropped in place.
pub fn unchecked(data: &[u8], out: &mut Vec<u8>) {
	let mut counts = [0u32; 4];
	let boxed = Box::into_raw(Box::new(0u32));
	let mut slot = MaybeUninit::<u32>::uninit();
	slot.write(7);
	let held = Arc::new(0u8);
	let mut dropped = MaybeUninit::new(Arc::clone(&held));
	for &byte in data {
		let at = usize::from(byte % 4);
		unsafe {
			*counts.get_unchecked_mut(at) += 1;
			*boxed += u32::from(byte);
			*slot.as_mut_ptr() ^= u32::from(byte);
			out.push(*counts.get_unchecked(at) as u8);
		}
	}
	unsafe {
		out.push(*Box::from_raw(boxed) as u8);
		out.push(slot.assume_init_read() as u8);
		out.push(Arc::strong_count(&held) as u8);
		ptr::drop_in_place(dropped.as_mut_ptr());
	}
	out.push(Arc::strong_count(&held) as u8);
}

#[derive(Clone, Copy)]
struct Inner {
	a: u32,
	b: u32,
}

struct Outer {
	inner: Inner,
	pair: (u32, u32),
	n: u32,
}

/// Pointers to the parts of a struct kept as objects: initialized one by one from nothing,
/// written through the struct's pointer and their own, and read out before a write.
pub fn nested(data: &[u8], out: &mut Vec<u8>) {
	let mut slot = MaybeUninit::<Outer>::uninit();
	let p = slot.as_mut_ptr();
	let (inner, pair) = unsafe { (&raw mut (*p).inner, &raw mut (*p).pair) };
	unsafe {
		inner.write(Inner { a: 1, b: 2 });
		pair.write((3, 4));
		(&raw mut (*p).n).write(5);
	}
	for &byte in data {
		let v = u32::from(byte);
		unsafe {
			match byte % 6 {
				0 => (*p).inner.a = v,
				1 => inner.write(Inner { a: v, b: (*inner).a }),
				2 => (*pair).1 += v,
				3 => {
					let old = p.read();
					(*inner).b = v;
					out.push(old.inner.b as u8);
				}
				4 => {
					let old = pair.read();
					(*p).pair.0 = v;
					out.push(old.0 as u8);
				}
				_ => out.push((*inner).a as u8 ^ (*pair).0 as u8),
			}
		}
	}
	let outer = unsafe { slot.assume_init() };
	let Outer { inner, pair, n } = outer;
	out.extend_from_slice(&[inner.a as u8, inner.b as u8, pair.0 as u8, pair.1 as u8, n as u8]);
}

#[derive(Clone, Copy)]
union Halves {
	a: [u32; 2],
	b: [u32; 2],
}

/// Copies of a `MaybeUninit` and of a union holding arrays, each changed after, and values
/// read out through pointers before what they point at changes.
pub fn copies(data: &[u8], out: &mut Vec<u8>) {
	for &byte in data {
		let v = u32::from(byte);
		let mut x = MaybeUninit::new([v; 4]);
		let y = x;
		x.write([v ^ 1; 4]);
		let h = Halves { a: [v, v + 1] };
		let mut g = h;
		unsafe { g.b[0] = v + 2 };
		let mut held = Some(Pair { a: v, b: 0 });
		let p = &raw mut held;
		let old = unsafe { p.read() };
		if let Some(pair) = unsafe { &mut *p } {
			pair.a = v + 3;
		}
		let (before, after) = snapshot(v);
		unsafe {
			out.extend_from_slice(&[
				y.assume_init()[0] as u8,
				x.assume_init()[0] as u8,
				h.a[0] as u8,
				g.a[0] as u8,
				old.map_or(0, |o| o.a as u8),
				held.map_or(0, |o| o.a as u8),
				before,
				after,
			]);
		}
	}
}

/// What a pointer to a tuple holding an array reads, and what the tuple holds after.
fn snapshot(v: u32) -> (u8, u8) {
	let mut nest = ([v, 1u32], 0u8);
	let q = &raw mut nest;
	let before = unsafe { q.read() };
	nest.0[0] = v + 4;
	(before.0[0] as u8, nest.0[0] as u8)
}

/// Null and dangling pointers to values JS keeps as `undefined`: an `Option` read through
/// `as_ref`, and `()` written through a dangling pointer; and values of no bytes read
/// through dangling pointers.
pub fn sentinels(data: &[u8], out: &mut Vec<u8>) {
	for &byte in data {
		let some = Some(u32::from(byte)).filter(|v| v % 2 == 0);
		let p = match byte % 3 {
			0 => ptr::null(),
			_ => &raw const some,
		};
		let unit = NonNull::<()>::dangling().as_ptr();
		unsafe {
			unit.write(());
			*unit = ();
			out.push(match p.as_ref() {
				None => 0,
				Some(None) => 1,
				Some(Some(v)) => 2 + (*v % 100) as u8,
			});
			out.push(p.as_ref().is_none() as u8);
			let units = NonNull::<[(); 2]>::dangling();
			out.push(units.as_ptr().read().into_iter().count() as u8);
			out.push(units.as_ref().len() as u8);
		}
	}
}

/// A struct JS keeps as an object, which Rust copies. It has no `Default`, which
/// `generics::reset` would count as replaced whole.
#[derive(Clone, Copy)]
struct Duo(u8, u8);

/// A struct JS keeps as an object, owning a count.
struct Held {
	n: u8,
	count: Rc<()>,
}

/// crossbeam-epoch's own wrapper over `UnsafeCell`, which lends its pointer to a closure.
struct Wrapped<T>(UnsafeCell<T>);

impl<T> Wrapped<T> {
	const fn new(value: T) -> Self {
		Self(UnsafeCell::new(value))
	}

	fn with<R>(&self, f: impl FnOnce(*const T) -> R) -> R {
		f(self.0.get())
	}

	fn with_mut<R>(&self, f: impl FnOnce(*mut T) -> R) -> R {
		f(self.0.get())
	}
}

/// `UnsafeCell`s of a number, a string, a struct Rust copies, and one that owns a count: written
/// through `get` and `get_mut`, read through another `get` and before a write, assigned
/// through `get`, which drops what each held, replaced whole, and their values taken out.
pub fn cells(data: &[u8], out: &mut Vec<u8>) {
	let count = Rc::new(());
	let mut num = UnsafeCell::new(u32::MAX - 1);
	let mut pair = UnsafeCell::new(Duo(0, 0xff));
	let mut text = UnsafeCell::new(String::new());
	let mut held = Wrapped::new(Held {
		n: 0,
		count: Rc::clone(&count),
	});
	for &op in data {
		let n = op / 8;
		match op % 8 {
			// A write through one pointer, seen through another, and a value read out before it.
			0 => {
				let (a, b) = (&num, &num);
				let before = unsafe { *b.get() };
				unsafe { *a.get() = (*b.get()).wrapping_add(u32::from(n) << 26) };
				out.extend(before.to_le_bytes());
				out.extend(unsafe { *b.get() }.to_le_bytes());
			}
			1 => {
				*num.get_mut() ^= u32::from(n);
				out.extend(num.get_mut().to_le_bytes());
			}
			// A part written, then the whole, with a copy read out before each.
			2 => {
				let before = unsafe { *pair.get() };
				unsafe { (*pair.get()).0 = n };
				let between = unsafe { *pair.get() };
				let p = pair.get();
				unsafe { *p = Duo((*p).1, (*p).0) };
				pair.get_mut().1 ^= 0x80;
				out.extend([
					before.0,
					before.1,
					between.0,
					between.1,
					pair.get_mut().0,
					pair.get_mut().1,
				]);
			}
			3 => {
				unsafe { (*text.get()).push(char::from(b'a' + n % 26)) };
				text.get_mut().push('!');
				out.extend(unsafe { &*text.get() }.as_bytes());
			}
			// Assigned through the pointer, which drops what it held, or written over, which
			// leaks it.
			4 => {
				let fresh = Held {
					n,
					count: Rc::clone(&count),
				};
				match n % 2 {
					0 => held.with_mut(|p| unsafe { *p = fresh }),
					_ => held.with_mut(|p| unsafe { ptr::write(p, fresh) }),
				}
				out.extend([held.with(|p| unsafe { (*p).n }), Rc::strong_count(&count) as u8]);
			}
			5 => {
				held.with_mut(|p| unsafe { (*p).n = (*p).n.wrapping_add(n) });
				held.0.get_mut().n ^= 1;
				out.push(held.with(|p| unsafe { (*p).n }));
			}
			// Each of a number and a string replaced whole, its old value taken out, while
			// the pointers `get` gave in other arms are dead. A cell of an object can't be,
			// as a pointer `get` gave would keep the old object.
			6 => {
				let old = std::mem::replace(&mut num, UnsafeCell::new(u32::from(n)));
				out.extend(old.into_inner().to_le_bytes());
				let old = std::mem::replace(&mut text, UnsafeCell::new(n.to_string()));
				out.extend(old.into_inner().as_bytes());
				*pair.get_mut() = Duo(n, !n);
			}
			// One made, written through `get`, and its value taken out and dropped.
			_ => {
				let made = Wrapped::new(Held {
					n,
					count: Rc::clone(&count),
				});
				made.with_mut(|p| unsafe { (*p).n ^= 0x55 });
				out.push(Rc::strong_count(&count) as u8);
				let made = made.0.into_inner();
				out.push(made.n);
				drop(made);
				out.push(Rc::strong_count(&count) as u8);
			}
		}
	}
	out.extend(num.into_inner().to_le_bytes());
	let Duo(a, b) = pair.into_inner();
	out.extend([a, b]);
	out.extend(text.into_inner().as_bytes());
	drop(held);
	out.push(Rc::strong_count(&count) as u8);
}

fn renew_num(cell: &mut UnsafeCell<u32>, value: u32) -> u32 {
	std::mem::replace(cell, UnsafeCell::new(value)).into_inner()
}

/// Pointers that keep pointing where Rust's do across `mem::replace`, `mem::swap`,
/// `mem::take`, and `*r = v`: one taken after its variable is replaced, one whose number
/// parts are, ones a helper writes through a `&mut` into the object there, and one taken
/// through two helpers and read while the variable is borrowed, which is assigned after;
/// one to a part a `&mut` given as `mem::replace`'s value borrows, which it leaves be;
/// and one through a reference `mem::replace` pointed elsewhere first.
pub fn rebound(data: &[u8], out: &mut Vec<u8>) {
	for &n in data {
		match n % 7 {
			0 => out.extend(replaced_before(n)),
			6 => out.extend(retargeted(n)),
			5 => out.extend(lent_as_value(n)),
			1 => out.extend(parts_replaced(n)),
			2 => out.extend(written_in_place(n)),
			3 => out.extend(cell_written_in_place(n)),
			_ => out.extend(borrowed_through(n)),
		}
	}
}

fn replaced_before(n: u8) -> [u8; 2] {
	let mut pair = (n, !n);
	let old = std::mem::replace(&mut pair, (n ^ 1, n));
	let p = &raw mut pair;
	unsafe { (*p).1 ^= old.0 };
	[pair.0, pair.1]
}

struct Span {
	range: (u8, u8),
}

fn lent_as_value(n: u8) -> [u8; 4] {
	let mut span = Span { range: (n, !n) };
	let mut other = (0, 0);
	let mut r = &mut other;
	let p = &raw mut span.range;
	let old = std::mem::replace(&mut r, &mut span.range);
	old.0 = n ^ 1;
	r.0 ^= 3;
	unsafe { [(*p).0, (*p).1, other.0, span.range.0] }
}

fn retargeted(n: u8) -> [u8; 2] {
	let mut a = (n, !n);
	let mut b = (n ^ 5, n);
	let mut r = &mut a;
	let _ = std::mem::replace(&mut r, &mut b);
	let p = &raw mut *r;
	a = (0, 0);
	unsafe { [(*p).0 ^ a.0, (*p).1] }
}

fn parts_replaced(n: u8) -> [u8; 4] {
	let mut pair = (n, !n);
	let p = &raw mut pair;
	let old = std::mem::replace(&mut pair.0, n.wrapping_add(1));
	std::mem::swap(&mut pair.0, &mut pair.1);
	let taken = std::mem::take(&mut pair.1);
	unsafe { [(*p).0, (*p).1, old, taken] }
}

fn written_in_place(n: u8) -> [u8; 6] {
	let mut a = Duo(n, 1);
	let mut b = Duo(2, n);
	let p = &raw mut a;
	let old = renew(&mut a, Duo(n ^ 3, n ^ 4));
	let between = unsafe { *p };
	exchange(&mut a, &mut b);
	let swapped = unsafe { *p };
	assign(&mut a, Duo(old.1, b.0));
	unsafe { [between.0, between.1, swapped.0, swapped.1, (*p).0, (*p).1] }
}

fn cell_written_in_place(n: u8) -> [u8; 4] {
	let mut cell = UnsafeCell::new(u32::from(n) << 9);
	let p = cell.get();
	let old = renew_num(&mut cell, u32::from(n) + 7);
	let between = unsafe { *p };
	let taken = clear(&mut cell);
	[
		old as u8,
		(old >> 9) as u8,
		between as u8,
		taken as u8 ^ unsafe { *p } as u8,
	]
}

fn borrowed_through(n: u8) -> [u8; 3] {
	let mut d = Duo(n, !n);
	let first = flipped(&mut d);
	d = Duo(d.1, d.0);
	[first, d.0, d.1]
}

fn renew(at: &mut Duo, value: Duo) -> Duo {
	std::mem::replace(at, value)
}

fn exchange(a: &mut Duo, b: &mut Duo) {
	std::mem::swap(a, b)
}

fn assign(at: &mut Duo, value: Duo) {
	*at = value;
}

fn clear(cell: &mut UnsafeCell<u32>) -> u32 {
	std::mem::take(cell).into_inner()
}

/// The first of `d`, flipped through the pointer `pointer_of` gives.
fn flipped(d: &mut Duo) -> u8 {
	let p = pointer_of(d);
	unsafe {
		(*p).0 ^= 0x5a;
		(*p).0
	}
}

fn pointer_of(d: &mut Duo) -> *mut Duo {
	&raw mut *d
}

/// A struct JS keeps as an object, holding bytes.
struct Log {
	bytes: Vec<u8>,
}

/// Pointers to variables JS keeps as objects, each dead before its variable is assigned
/// again, so it reads the object the variable held while it was made: one made in each
/// iteration of a loop that assigns the variable after, one an arm makes where another
/// arm replaces its variable, and ones nested loops make.
pub fn lapsed(data: &[u8], out: &mut Vec<u8>) {
	let mut pair = Duo(0, 0xff);
	let mut log = Log { bytes: Vec::new() };
	let mut window = [0u8; 3];
	for &n in data {
		let p = &raw mut pair;
		unsafe { (*p).0 = (*p).0.wrapping_add(n) };
		out.extend(unsafe { [(*p).0, (*p).1] });
		pair = Duo(pair.1 ^ n, pair.0);
		match n % 4 {
			0 => {
				let q = &raw mut log;
				unsafe { (*q).bytes.push(n) };
				out.extend(unsafe { &(*q).bytes });
			}
			1 => {
				let old = std::mem::replace(&mut log, Log { bytes: vec![n, !n] });
				out.extend(old.bytes);
				let old = std::mem::replace(&mut pair, Duo(n, n ^ 0x0f));
				out.extend([old.0, old.1]);
			}
			// The outer loop's pointer, read in the inner one and after it, and the inner
			// one's own, whose variable each inner iteration assigns.
			2 => {
				let mut duo = Duo(n, 0);
				let outer = &raw mut duo;
				for i in 0..n % 5 {
					let inner = &raw mut window;
					unsafe {
						(*inner)[usize::from(i % 3)] ^= n;
						(*outer).1 = (*outer).1.wrapping_add((*inner)[0]);
					}
					window = [window[2], window[0], window[1].wrapping_add(i)];
				}
				out.extend(unsafe { [(*outer).0, (*outer).1] });
				duo = Duo(duo.1, duo.0);
				out.extend([duo.0, duo.1]);
			}
			_ => {
				let p = &raw mut window;
				let copy = p;
				out.push(unsafe { (*copy)[0] });
				window = [n; 3];
			}
		}
	}
	out.extend([pair.0, pair.1]);
	out.extend(log.bytes);
	out.extend(window);
}

/// Structs JS keeps as objects, which Rust copies, and which only the generic bodies below
/// write into, each reached at one of them: by a call, through a trait object, and through
/// a closure. Each copy of one is copied in JS too, as are none of the types they aren't
/// reached at, such as `nested`'s, whose pointers translate.
#[derive(Clone, Copy, Default)]
struct ByCall(u8, u8);

#[derive(Clone, Copy, Default)]
struct ByObject(u8, u8);

#[derive(Clone, Copy, Default)]
struct ByClosure(u8, u8);

fn put<T>(slot: &mut T, value: T) {
	*slot = value;
}

/// A value of a `T` in a cell, which `reset` gives `T`'s default.
struct Bin<T>(UnsafeCell<T>);

trait Reset {
	fn reset(&self);
}

impl<T: Default> Reset for Bin<T> {
	fn reset(&self) {
		unsafe { *self.0.get() = T::default() };
	}
}

fn resetter<T: Default>(slot: *mut T) -> impl FnOnce() {
	move || unsafe { *slot = T::default() }
}

/// Each byte a copy of one of them, the original written whole after by its generic body,
/// and both put. Each is a function of its own, as a loop assigns its variables again.
pub fn writers(data: &[u8], out: &mut Vec<u8>) {
	for &byte in data {
		let n = byte / 3;
		out.extend(match byte % 3 {
			0 => by_call(n),
			1 => by_object(n),
			_ => by_closure(n),
		});
	}
}

fn by_call(n: u8) -> [u8; 4] {
	let mut a = ByCall(n, 1);
	let b = a;
	put(&mut a, ByCall(2, !n));
	[a.0, a.1, b.0, b.1]
}

fn by_object(n: u8) -> [u8; 4] {
	let bin = Bin(UnsafeCell::new(ByObject(n, 1)));
	let before = unsafe { *bin.0.get() };
	let object: &dyn Reset = &bin;
	object.reset();
	let after = bin.0.into_inner();
	[after.0, after.1, before.0, before.1]
}

fn by_closure(n: u8) -> [u8; 4] {
	let mut a = ByClosure(n, 1);
	let b = a;
	resetter(&raw mut a)();
	[a.0, a.1, b.0, b.1]
}

struct Gauge {
	range: (u8, u8),
	slot: MaybeUninit<(u8, u8)>,
}

/// Each part of a `Gauge` written into, never given another object.
fn nudge(g: &mut Gauge, n: u8) {
	g.range.0 ^= n;
	unsafe { (*g.slot.as_mut_ptr()).1 ^= n };
}

/// Pointers std makes to parts kept as objects, which a helper writes into but never gives
/// another object: `NonNull::from` of a `&mut` and of a `&`, `into` of a `&mut`, and
/// `MaybeUninit::as_mut_ptr` and `as_ptr`. Each is a function of its own, as a loop assigns
/// its variables again.
pub fn made(data: &[u8], out: &mut Vec<u8>) {
	for &n in data {
		out.extend(made_parts(n));
	}
}

fn made_parts(n: u8) -> [u8; 7] {
	let mut g = Gauge {
		range: (n, !n),
		slot: MaybeUninit::new((!n, n)),
	};
	let p = NonNull::from(&mut g.range);
	let q = g.slot.as_mut_ptr();
	nudge(&mut g, n ^ 0x5a);
	unsafe { (*q).0 = p.as_ref().1 };
	let r = NonNull::from(&g.range);
	let s = g.slot.as_ptr();
	let t: NonNull<(u8, u8)> = (&mut g.range).into();
	nudge(&mut g, n);
	unsafe {
		[
			p.as_ref().0,
			p.as_ref().1,
			r.as_ref().0,
			t.as_ref().1,
			(*s).0,
			(*s).1,
			g.slot.assume_init().0,
		]
	}
}

/// A cell read and written only through the pointer it lends a closure, as kio's
/// `sync::UnsafeCell` is.
struct Lent<T>(UnsafeCell<T>);

impl<T> Lent<T> {
	const fn new(value: T) -> Self {
		Self(UnsafeCell::new(value))
	}

	fn with<R>(&self, f: impl FnOnce(*const T) -> R) -> R {
		f(self.0.get())
	}

	fn with_mut<R>(&self, f: impl FnOnce(*mut T) -> R) -> R {
		f(self.0.get())
	}
}

type Entry = Lent<MaybeUninit<(u8, u8)>>;

/// A run of slots as kio's log keeps one: each filled whole with an empty cell while
/// nothing points into the fresh allocation yet, then written and read through the
/// pointer its cell lends.
struct Segment {
	filled: usize,
	slots: Box<[MaybeUninit<Entry>; 4]>,
}

impl Segment {
	fn new() -> Self {
		let mut slots = Box::new([const { MaybeUninit::uninit() }; 4]);
		for at in 0..4 {
			slots[at].write(Lent::new(MaybeUninit::uninit()));
		}
		Segment { filled: 0, slots }
	}

	/// As [`Self::new`], with kio's loop written with an iterator.
	fn iterated() -> Self {
		let mut slots = Box::new([const { MaybeUninit::uninit() }; 4]);
		for slot in slots.iter_mut() {
			slot.write(Lent::new(MaybeUninit::uninit()));
		}
		Segment { filled: 0, slots }
	}

	fn slot(&self, at: usize) -> &Entry {
		unsafe { self.slots[at].assume_init_ref() }
	}
}

/// Entries pushed into segments of slots that each fill whole first, read back, and
/// changed in place through the pointer each slot's cell lends.
pub fn segments(data: &[u8], out: &mut Vec<u8>) {
	let mut log: Vec<Segment> = Vec::new();
	let mut len = 0;
	for &byte in data {
		match byte % 4 {
			0 => {
				if len % 8 == 0 {
					log.push(Segment::new());
				} else if len % 4 == 0 {
					log.push(Segment::iterated());
				}
				let segment = &mut log[len / 4];
				segment
					.slot(segment.filled)
					.with_mut(|p| unsafe { (*p).write((byte, !byte)) });
				segment.filled += 1;
				len += 1;
			}
			1 if len > 0 => {
				let at = usize::from(byte) % len;
				let (a, b) = log[at / 4].slot(at % 4).with(|p| unsafe { *(*p).assume_init_ref() });
				out.extend([a, b]);
			}
			2 if len > 0 => {
				let at = usize::from(byte) % len;
				log[at / 4]
					.slot(at % 4)
					.with_mut(|p| unsafe { (*(*p).as_mut_ptr()).1 ^= byte });
			}
			_ => out.push(log.iter().map(|s| s.filled as u8).sum()),
		}
	}
}

/// A struct JS keeps as an object, with a part it keeps as one too.
struct Run {
	span: (u8, u8),
	n: u8,
}

/// A `MaybeUninit` array and a `MaybeUninit` variable, each slot written whole before any
/// pointer into it is taken, then changed through pointers to a part of each and read back.
pub fn uninit_runs(data: &[u8], out: &mut Vec<u8>) {
	let mut runs: [MaybeUninit<Run>; 3] = [const { MaybeUninit::uninit() }; 3];
	for at in 0..3 {
		let n = data.get(at).copied().unwrap_or(0);
		runs[at].write(Run {
			span: (n, !n),
			n: at as u8,
		});
	}
	let mut last = MaybeUninit::<Run>::uninit();
	unsafe {
		last.as_mut_ptr().write(Run {
			span: (data.len() as u8, 0),
			n: 3,
		})
	};
	let tail = unsafe { &raw mut (*last.as_mut_ptr()).span };
	for &byte in data {
		let run = unsafe { runs[usize::from(byte) % 3].assume_init_mut() };
		let span = &raw mut run.span;
		unsafe {
			(*span).0 ^= byte;
			(*span).1 = (*span).1.wrapping_add(run.n);
			(*tail).1 = (*tail).1.wrapping_add((*span).0);
			out.push((*span).0);
		}
	}
	for run in &runs {
		let run = unsafe { run.assume_init_ref() };
		out.extend([run.span.0, run.span.1, run.n]);
	}
	let last = unsafe { last.assume_init() };
	out.extend([last.span.0, last.span.1, last.n]);
}

/// `MaybeUninit` slots each written whole through std's iterators over them before any
/// pointer into them is taken: an array's `iter_mut`, a boxed slice's `&mut`, its
/// `chunks_exact_mut`, and a `zip` with an index range; then changed through pointers to
/// a part of each and read back.
pub fn iterated_runs(data: &[u8], out: &mut Vec<u8>) {
	let byte = |at: usize| data.get(at).copied().unwrap_or(0);
	let mut runs: [MaybeUninit<Run>; 3] = [const { MaybeUninit::uninit() }; 3];
	for slot in runs.iter_mut() {
		slot.write(Run {
			span: (byte(0), !byte(0)),
			n: 0,
		});
	}
	let mut boxed: Box<[MaybeUninit<Run>]> = Box::new([const { MaybeUninit::uninit() }; 4]);
	for slot in &mut boxed {
		slot.write(Run {
			span: (byte(1), 1),
			n: 1,
		});
	}
	for (chunk, at) in boxed.chunks_exact_mut(2).zip(0u8..) {
		for slot in chunk {
			slot.write(Run {
				span: (byte(2), at),
				n: at.wrapping_add(byte(3)),
			});
		}
	}
	for (slot, at) in runs.iter_mut().zip(0..) {
		slot.write(Run {
			span: (byte(at), at as u8),
			n: at as u8,
		});
	}
	for &byte in data {
		let run = unsafe { runs[usize::from(byte) % 3].assume_init_mut() };
		let span = &raw mut run.span;
		unsafe {
			(*span).0 ^= byte;
			(*span).1 = (*span).1.wrapping_add(run.n);
			out.push((*span).0);
		}
		let run = unsafe { boxed[usize::from(byte) % 4].assume_init_mut() };
		let span = &raw mut run.span;
		unsafe {
			(*span).1 ^= byte;
			out.push((*span).1);
		}
	}
	for run in runs.iter().chain(boxed.iter()) {
		let run = unsafe { run.assume_init_ref() };
		out.extend([run.span.0, run.span.1, run.n]);
	}
}

/// A struct JS keeps as an object, holding an `rt.Cell`.
struct Stable(UnsafeCell<u8>);

fn first_set<T>(pair: &mut (T, Stable), value: T) {
	pair.0 = value;
}

fn stable_cell(stable: &Stable) -> *mut u8 {
	stable.0.get()
}

/// A pointer to a cell beside a field a generic body writes whole, which leaves the cell's
/// struct be.
pub fn beside_written(data: &[u8], out: &mut Vec<u8>) {
	let mut pair = (0u16, Stable(UnsafeCell::new(0)));
	let cell = stable_cell(&pair.1);
	for &byte in data {
		first_set(&mut pair, u16::from(byte));
		unsafe { *cell = (*cell).wrapping_add(byte) };
		out.push(unsafe { *cell } ^ pair.0 as u8);
	}
}

/// A node a list links through an `AtomicPtr`, holding a cell it lends a pointer into, as
/// wasmtimer's timer nodes hold futures' `AtomicWaker`.
struct Waiter {
	next: AtomicPtr<Waiter>,
	enqueued: AtomicBool,
	slot: UnsafeCell<(u8, u8)>,
}

fn waiter(n: u8) -> *mut Waiter {
	Box::into_raw(Box::new(Waiter {
		next: AtomicPtr::new(ptr::null_mut()),
		enqueued: AtomicBool::new(false),
		slot: UnsafeCell::new((n, 0)),
	}))
}

fn slot_of(waiter: &Waiter) -> *mut (u8, u8) {
	waiter.slot.get()
}

/// Waiters pushed onto an atomic head and popped through `get_mut`, as wasmtimer's
/// `ArcList` keeps them, with the one popped last kept in memory too.
struct Waiters {
	head: AtomicPtr<Waiter>,
	last: *mut Waiter,
}

impl Waiters {
	fn push(&self, waiter: *mut Waiter) {
		let node = unsafe { &*waiter };
		if node.enqueued.swap(true, SeqCst) {
			return;
		}
		let mut head = self.head.load(SeqCst);
		loop {
			node.next.store(head, SeqCst);
			match self.head.compare_exchange(head, waiter, SeqCst, SeqCst) {
				Ok(_) => break,
				Err(now) => head = now,
			}
		}
	}

	fn pop(&mut self) -> bool {
		let head = *self.head.get_mut();
		if head.is_null() {
			return false;
		}
		*self.head.get_mut() = unsafe { (*head).next.load(SeqCst) };
		unsafe { (*head).enqueued.store(false, SeqCst) };
		self.last = head;
		true
	}
}

/// Waiters linked and unlinked, each changed and read through the pointer into its cell
/// taken when it was made, which nothing writing a waiter whole leaves stale.
pub fn waiters(data: &[u8], out: &mut Vec<u8>) {
	let nodes = [waiter(0), waiter(1), waiter(2), waiter(3)];
	let slots = unsafe {
		[
			slot_of(&*nodes[0]),
			slot_of(&*nodes[1]),
			slot_of(&*nodes[2]),
			slot_of(&*nodes[3]),
		]
	};
	let mut list = Waiters {
		head: AtomicPtr::new(ptr::null_mut()),
		last: ptr::null_mut(),
	};
	for &byte in data {
		let at = usize::from(byte >> 2) % 4;
		match byte % 4 {
			0 => list.push(nodes[at]),
			1 => out.push(u8::from(list.pop())),
			2 if !list.last.is_null() => unsafe {
				let slot = slot_of(&*list.last);
				(*slot).1 = (*slot).1.wrapping_add(byte);
			},
			_ => {
				let (a, b) = unsafe { *slots[at] };
				out.extend([a, b]);
			}
		}
	}
	let mut at = list.head.load(SeqCst);
	while !at.is_null() {
		let (a, b) = unsafe { *slot_of(&*at) };
		out.extend([a, b]);
		at = unsafe { (*at).next.load(SeqCst) };
	}
	for node in 0..4 {
		drop(unsafe { Box::from_raw(nodes[node]) });
	}
}

/// Pointers callees take through references, which keep pointing where Rust's do: one
/// read before its variable is assigned again, ones kept where liveness can't follow them
/// for a variable each iteration of a loop declares, one through a reference `mem::swap`
/// points at another variable, whose first is assigned after, and ones a callee gives back
/// only as a reference, through a borrow a closure keeps.
pub fn taken(data: &[u8], out: &mut Vec<u8>) {
	for &n in data {
		match n % 4 {
			0 => out.extend(read_before(n)),
			1 => per_iteration(n, out),
			2 => out.extend(swapped_away(n)),
			_ => out.extend(viewed(n)),
		}
	}
}

fn read_before(n: u8) -> [u8; 3] {
	let mut d = Duo(n, !n);
	let q = pointer_of(&mut d);
	let seen = unsafe {
		(*q).0 ^= 0x33;
		(*q).0
	};
	d = Duo(d.1, d.0);
	[seen, d.0, d.1]
}

fn keep(d: &mut Duo, kept: &mut Vec<*mut Duo>) {
	kept.push(&raw mut *d);
}

fn per_iteration(n: u8, out: &mut Vec<u8>) {
	let mut kept = Vec::new();
	for i in 0..n % 4 {
		let mut d = Duo(n ^ i, i);
		keep(&mut d, &mut kept);
		unsafe { (*kept[0]).1 ^= n };
		out.extend([d.0, d.1]);
		kept.clear();
	}
}

fn swapped_away(n: u8) -> [u8; 4] {
	let mut a = (n, 1);
	let mut b = (2, n);
	let mut r = &mut a;
	let mut s = &mut b;
	std::mem::swap(&mut r, &mut s);
	let p = &raw mut *r;
	a = (a.1, a.0);
	unsafe { (*p).0 ^= 7 };
	[a.0, a.1, b.0, b.1]
}

struct Viewed(u8, u8);

fn first_of(v: &mut Viewed) -> &mut u8 {
	unsafe { &mut (*(&raw mut *v)).0 }
}

fn viewed(n: u8) -> [u8; 2] {
	let mut v = Viewed(n, 3);
	let mut bump = |k| *first_of(&mut v) ^= k;
	bump(5);
	bump(n);
	v = Viewed(v.1, v.0);
	[v.0, v.1]
}
