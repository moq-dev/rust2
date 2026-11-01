//! The capacity std's `Vec`, `VecDeque`, and `BinaryHeap` hold as each step of a case
//! grows, shrinks, remakes, or reads one, of elements of each size its growth tells apart:
//! a byte, at most 1 KiB, past it, and none. std fixes it exactly only through steps its
//! doubling decides, such as pushes, `with_capacity`, `reserve`, and `shrink_to`; past
//! any other step, a case checks only what std guarantees.

use std::collections::{BinaryHeap, VecDeque};
use std::mem;

/// A length or capacity, as four bytes.
fn put(out: &mut Vec<u8>, n: usize) {
	out.extend_from_slice(&(n as u32).to_le_bytes());
}

/// What std fixes of a vector's capacity as steps change it: its value while `exact`, and
/// else that it holds at least the length and `floor`, the most `with_capacity` or
/// `reserve` asked for since a shrink.
struct Known {
	exact: bool,
	floor: usize,
}

impl Known {
	/// A vector just made, with room for at least `floor`.
	fn new(floor: usize) -> Self {
		Self { exact: true, floor }
	}

	/// Whatever capacity a step left, holding at least `floor` still.
	fn loose(&mut self) {
		self.exact = false;
	}

	/// A vector made anew, which holds what std guarantees of it.
	fn remade(&mut self, exact: bool) {
		*self = Self { exact, floor: 0 };
	}

	/// After `reserve` made room for `additional` past `len`.
	fn reserved(&mut self, len: usize, additional: usize) {
		self.floor = self.floor.max(len + additional);
	}

	/// After `shrink_to` of `min`, holding `len`.
	fn shrunk(&mut self, len: usize, min: usize) {
		self.floor = self.floor.min(len.max(min));
	}

	/// `len` and what this fixes of `cap`.
	fn put(&self, out: &mut Vec<u8>, len: usize, cap: usize) {
		put(out, len);
		match self.exact {
			true => put(out, cap),
			false => put(out, usize::from(cap >= len.max(self.floor))),
		}
	}
}

/// How many to reserve by `arg`, beside `len` of `size`-byte elements: a few, or one to
/// eight past what a vector of them may hold, or within eight of `usize::MAX`, either of
/// which panics before it allocates.
fn amount(arg: u8, len: usize, size: usize) -> usize {
	match arg >> 5 {
		7 => usize::MAX - usize::from(arg & 7),
		6 if size > 0 => isize::MAX as usize / size - len + 1 + usize::from(arg & 7),
		_ => usize::from(arg & 0x1f),
	}
}

/// A test that keeps every other element it is asked about.
fn alternate() -> impl FnMut() -> bool {
	let mut kept = false;
	move || {
		kept = !kept;
		kept
	}
}

/// What the element type `kind` picks makes of a byte.
macro_rules! by_kind {
	($kind:expr, $f:ident, $data:expr, $out:expr) => {
		match $kind % 5 {
			0 => $f($data, |b| b, $out),
			1 => $f($data, u16::from, $out),
			2 => $f($data, |b| (u64::from(b), b), $out),
			3 => $f($data, |b| [u64::from(b); 160], $out),
			_ => $f($data, |_| (), $out),
		}
	};
}

/// A `Vec` that `data`'s first byte makes, by `with_capacity` if odd, and each pair after
/// it changes, its length and capacity after each.
pub fn vecs(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	by_kind!(kind, vec_steps, data, out);
}

fn vec_steps<T: Clone + Ord>(data: &[u8], make: fn(u8) -> T, out: &mut Vec<u8>) {
	let size = size_of::<T>();
	let (first, steps) = data.split_first().map_or((0, &[][..]), |(&b, rest)| (b, rest));
	let mut v: Vec<T> = match first & 1 {
		1 => Vec::with_capacity(usize::from(first >> 3)),
		_ => Vec::new(),
	};
	let mut k = Known::new(v.capacity());
	put(out, v.capacity());
	for step in steps.chunks(2) {
		let (op, arg) = (step[0], step.get(1).copied().unwrap_or(0));
		match op % 25 {
			0 | 1 => v.push(make(arg)),
			2 => {
				drop(v.pop());
				k.loose();
			}
			3 | 4 => {
				let additional = amount(arg, v.len(), size);
				match op % 25 {
					3 => v.reserve(additional),
					_ => v.reserve_exact(additional),
				}
				k.reserved(v.len(), additional);
			}
			5 => {
				v.shrink_to(usize::from(arg % 40));
				k.shrunk(v.len(), usize::from(arg % 40));
			}
			6 => {
				v.shrink_to_fit();
				k.shrunk(v.len(), 0);
				k.exact = true;
			}
			7 => {
				v.truncate(usize::from(arg % 8));
				k.loose();
			}
			8 => v.insert(usize::from(arg) % (v.len() + 1), make(arg)),
			9 if !v.is_empty() => {
				drop(v.remove(usize::from(arg) % v.len()));
				k.loose();
			}
			// Not `TrustedLen`, so it reserves only once full, and a `TrustedLen` one.
			10 => v.extend((0..arg % 24).filter(|i| i % 3 != 0).map(make)),
			11 => v.extend((0..arg % 24).map(make)),
			12 => {
				v.extend_from_slice(&[make(arg), make(arg)]);
				k.loose();
			}
			13 => {
				v.clear();
				k.loose();
			}
			14 => {
				let mut keep = alternate();
				v.retain(|_| keep());
				k.loose();
			}
			15 => {
				drop(v.drain(..v.len().min(usize::from(arg % 4))));
				k.loose();
			}
			16 => {
				v = v.clone();
				k.remade(true);
			}
			17 => {
				v = v.into_boxed_slice().into_vec();
				k.remade(false);
			}
			// In place of its own, and of its own once read from either end.
			18 => {
				let mut keep = alternate();
				v = v.into_iter().filter(|_| keep()).collect();
				k.remade(false);
			}
			19 => {
				let mut it = v.into_iter();
				for i in 0..arg % 8 {
					drop(if i % 3 == 2 { it.next_back() } else { it.next() });
				}
				v = it.collect();
				k.remade(false);
			}
			20 => {
				let mut taken = mem::take(&mut v);
				taken.push(make(arg));
				k.put(out, taken.len(), taken.capacity());
				v = mem::replace(&mut taken, Vec::with_capacity(usize::from(arg % 6)));
				Known::new(0).put(out, taken.len(), taken.capacity());
			}
			21 if !v.is_empty() => {
				drop(v.swap_remove(usize::from(arg) % v.len()));
				k.loose();
			}
			22 => {
				v.dedup();
				k.loose();
			}
			// A clone of one read from the front, a new `Vec` of what was left.
			23 => {
				let mut it = v.into_iter();
				for _ in 0..arg % 4 {
					drop(it.next());
				}
				v = it.clone().collect();
				k.remade(true);
				let rest = it.collect::<Vec<T>>();
				Known { exact: false, floor: 0 }.put(out, rest.len(), rest.capacity());
			}
			24 => {
				v = vec![make(arg); usize::from(arg % 6)];
				k.remade(false);
			}
			_ => {}
		}
		k.put(out, v.len(), v.capacity());
	}
}

/// A `VecDeque`, as `vecs` steps a `Vec`.
pub fn deques(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	by_kind!(kind, deque_steps, data, out);
}

fn deque_steps<T: Clone + Ord>(data: &[u8], make: fn(u8) -> T, out: &mut Vec<u8>) {
	let size = size_of::<T>();
	let (first, steps) = data.split_first().map_or((0, &[][..]), |(&b, rest)| (b, rest));
	let mut q: VecDeque<T> = match first & 1 {
		1 => VecDeque::with_capacity(usize::from(first >> 3)),
		_ => VecDeque::new(),
	};
	let mut k = Known::new(q.capacity());
	put(out, q.capacity());
	for step in steps.chunks(2) {
		let (op, arg) = (step[0], step.get(1).copied().unwrap_or(0));
		match op % 21 {
			0 => q.push_back(make(arg)),
			1 => q.push_front(make(arg)),
			2 => {
				drop(q.pop_front());
				k.loose();
			}
			3 | 4 => {
				let additional = amount(arg, q.len(), size);
				match op % 21 {
					3 => q.reserve(additional),
					_ => q.reserve_exact(additional),
				}
				k.reserved(q.len(), additional);
			}
			5 => {
				q.shrink_to(usize::from(arg % 40));
				k.shrunk(q.len(), usize::from(arg % 40));
			}
			6 => {
				q.shrink_to_fit();
				k.shrunk(q.len(), 0);
				k.exact = true;
			}
			7 => {
				q.truncate(usize::from(arg % 8));
				k.loose();
			}
			8 => q.insert(usize::from(arg) % (q.len() + 1), make(arg)),
			9 => {
				drop(q.remove(usize::from(arg) % (q.len() + 1)));
				k.loose();
			}
			10 => q.extend((0..arg % 24).filter(|i| i % 3 != 0).map(make)),
			11 => q.extend((0..arg % 24).map(make)),
			12 => {
				q.clear();
				k.loose();
			}
			13 => {
				let mut keep = alternate();
				q.retain(|_| keep());
				k.loose();
			}
			14 => {
				drop(q.drain(..q.len().min(usize::from(arg % 4))));
				k.loose();
			}
			15 => {
				q = q.clone();
				k.remade(true);
			}
			// Its own, and a `Vec`'s in place of one, and one it collects first.
			16 => {
				q = q.into_iter().collect();
				k.remade(false);
			}
			17 => {
				q = VecDeque::from(q.into_iter().collect::<Vec<T>>());
				k.remade(false);
			}
			18 => {
				let mut keep = alternate();
				q = q.into_iter().filter(|_| keep()).collect();
				k.remade(true);
			}
			19 => {
				let mut it = q.into_iter();
				for _ in 0..arg % 4 {
					drop(it.next());
				}
				q = it.clone().collect();
				k.remade(false);
				let rest = it.collect::<VecDeque<T>>();
				Known { exact: false, floor: 0 }.put(out, rest.len(), rest.capacity());
			}
			_ => {
				q = VecDeque::from([make(arg), make(arg), make(arg)]);
				k.remade(false);
			}
		}
		k.put(out, q.len(), q.capacity());
	}
}

/// A `BinaryHeap`, as `vecs` steps a `Vec`, and the `Vec` it gives up at the end.
pub fn heaps(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	by_kind!(kind, heap_steps, data, out);
}

fn heap_steps<T: Clone + Ord>(data: &[u8], make: fn(u8) -> T, out: &mut Vec<u8>) {
	let size = size_of::<T>();
	let (first, steps) = data.split_first().map_or((0, &[][..]), |(&b, rest)| (b, rest));
	let mut h: BinaryHeap<T> = match first & 1 {
		1 => BinaryHeap::with_capacity(usize::from(first >> 3)),
		_ => BinaryHeap::new(),
	};
	let mut k = Known::new(h.capacity());
	put(out, h.capacity());
	let mut other = BinaryHeap::new();
	for step in steps.chunks(2) {
		let (op, arg) = (step[0], step.get(1).copied().unwrap_or(0));
		match op % 13 {
			0 | 1 => h.push(make(arg)),
			2 => {
				drop(h.pop());
				k.loose();
			}
			3 | 4 => {
				let additional = amount(arg, h.len(), size);
				match op % 13 {
					3 => h.reserve(additional),
					_ => h.reserve_exact(additional),
				}
				k.reserved(h.len(), additional);
			}
			5 => {
				h.shrink_to(usize::from(arg % 40));
				k.shrunk(h.len(), usize::from(arg % 40));
			}
			6 => {
				h.shrink_to_fit();
				k.shrunk(h.len(), 0);
				k.exact = true;
			}
			7 => {
				h.clear();
				k.loose();
			}
			8 => h.extend((0..arg % 24).filter(|i| i % 3 != 0).map(make)),
			// The larger of the two keeps its elements, and the other's join them.
			9 => {
				other.push(make(arg));
				h.append(&mut other);
				k.remade(false);
				Known { exact: false, floor: 0 }.put(out, other.len(), other.capacity());
			}
			10 => {
				drop(h.drain());
				k.loose();
			}
			// A guard that hid all but the top, forgotten, which leaks the rest.
			11 => {
				if let Some(mut top) = h.peek_mut() {
					*top = make(arg);
					mem::forget(top);
				}
				k.remade(false);
			}
			_ => h = BinaryHeap::from(h.into_vec()),
		}
		k.put(out, h.len(), h.capacity());
	}
	let cloned = h.clone().into_vec();
	Known::new(0).put(out, cloned.len(), cloned.capacity());
	let sorted = h.into_sorted_vec();
	k.put(out, sorted.len(), sorted.capacity());
}

/// What collections each way std's makes them from an iterator hold room for: in place of
/// a `Vec` of elements of another size, by a tuple's `Extend`, by `vec!`, and by
/// `with_capacity` past what one may hold, which panics.
pub fn collects(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let n = data.first().map_or(0, |&b| usize::from(b % 24));
	let pairs: Vec<(u32, u32)> = Vec::with_capacity(n + usize::from(kind % 4));
	let mut pairs = pairs;
	pairs.extend((0..n as u32).map(|i| (i, i)));
	let exact = Known::new(0);
	let loose = Known { exact: false, floor: 0 };
	match kind % 8 {
		// Two `u32`s to one in place, which fit twice as many.
		0 => {
			let halves: Vec<u32> = pairs.into_iter().map(|(a, _)| a).collect();
			loose.put(out, halves.len(), halves.capacity());
		}
		// Bytes in place of bytes, and wider items than bytes, which collect anew.
		1 => {
			let mut bytes = Vec::with_capacity(n + 3);
			bytes.extend_from_slice(&data[..n.min(data.len())]);
			exact.put(out, bytes.len(), bytes.capacity());
			let odd: Vec<u8> = bytes.clone().into_iter().filter(|b| b % 2 == 1).collect();
			loose.put(out, odd.len(), odd.capacity());
			let wide: Vec<u16> = bytes.into_iter().map(u16::from).collect();
			exact.put(out, wide.len(), wide.capacity());
		}
		2 => {
			let (a, b): (Vec<u32>, VecDeque<u32>) = pairs.iter().filter(|(a, _)| a % 2 == 0).copied().unzip();
			exact.put(out, a.len(), a.capacity());
			exact.put(out, b.len(), b.capacity());
			let (c, d): (Vec<u8>, BinaryHeap<u32>) = pairs.iter().map(|&(a, b)| (a as u8, b)).unzip();
			exact.put(out, c.len(), c.capacity());
			exact.put(out, d.len(), d.capacity());
		}
		3 => {
			let filled = vec![0u16; n];
			loose.put(out, filled.len(), filled.capacity());
			let units = vec![(); n];
			exact.put(out, units.len(), units.capacity());
		}
		4 => {
			let v: Vec<u64> = Vec::with_capacity(usize::MAX - n);
			put(out, v.capacity());
		}
		5 => {
			let q: VecDeque<(u64, u64)> = VecDeque::with_capacity(0x1000_0000 + n);
			put(out, q.capacity());
		}
		// A deque and a heap in place of a `Vec` of bytes, and the bytes a heap gives up.
		6 => {
			let mut bytes = Vec::with_capacity(n + 5);
			bytes.extend_from_slice(&data[..n.min(data.len())]);
			let q = VecDeque::from(bytes.clone());
			loose.put(out, q.len(), q.capacity());
			let heap = BinaryHeap::from(bytes);
			loose.put(out, heap.len(), heap.capacity());
			let v = heap.into_vec();
			loose.put(out, v.len(), v.capacity());
		}
		_ => {
			let bytes = vec![7u8; n];
			let mut grown = bytes.clone();
			grown.extend((0..n as u8).filter(|b| b % 2 == 0));
			exact.put(out, grown.len(), grown.capacity());
			grown.shrink_to(n / 2);
			exact.put(out, grown.len(), grown.capacity());
		}
	}
}
