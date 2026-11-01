//! The capacity std's `Vec`, `VecDeque`, and `BinaryHeap` hold as each step of a case
//! grows, shrinks, remakes, or reads one, of elements of each size its growth tells apart:
//! a byte, at most 1 KiB, past it, and none.

use std::collections::{BinaryHeap, VecDeque};
use std::mem;

/// A length or capacity, as four bytes.
fn put(out: &mut Vec<u8>, n: usize) {
	out.extend_from_slice(&(n as u32).to_le_bytes());
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
	put(out, v.capacity());
	for step in steps.chunks(2) {
		let (op, arg) = (step[0], step.get(1).copied().unwrap_or(0));
		match op % 24 {
			0 | 1 => v.push(make(arg)),
			2 => drop(v.pop()),
			3 => v.reserve(amount(arg, v.len(), size)),
			4 => v.reserve_exact(amount(arg, v.len(), size)),
			5 => v.shrink_to(usize::from(arg % 40)),
			6 => v.shrink_to_fit(),
			7 => v.truncate(usize::from(arg % 8)),
			8 => v.insert(usize::from(arg) % (v.len() + 1), make(arg)),
			9 if !v.is_empty() => drop(v.remove(usize::from(arg) % v.len())),
			// Not `TrustedLen`, so it reserves only once full, and a `TrustedLen` one.
			10 => v.extend((0..arg % 24).filter(|i| i % 3 != 0).map(make)),
			11 => v.extend((0..arg % 24).map(make)),
			12 => v.extend_from_slice(&[make(arg), make(arg)]),
			13 => v.clear(),
			14 => {
				let mut keep = alternate();
				v.retain(|_| keep());
			}
			15 => drop(v.drain(..v.len().min(usize::from(arg % 4)))),
			16 => v = v.clone(),
			17 => v = v.into_boxed_slice().into_vec(),
			// In place of its own, and of its own once read from either end.
			18 => {
				let mut keep = alternate();
				v = v.into_iter().filter(|_| keep()).collect();
			}
			19 => {
				let mut it = v.into_iter();
				for i in 0..arg % 8 {
					drop(if i % 3 == 2 { it.next_back() } else { it.next() });
				}
				v = it.collect();
			}
			20 => {
				let mut taken = mem::take(&mut v);
				taken.push(make(arg));
				put(out, taken.capacity());
				v = mem::replace(&mut taken, Vec::with_capacity(usize::from(arg % 6)));
				put(out, taken.capacity());
			}
			21 if !v.is_empty() => drop(v.swap_remove(usize::from(arg) % v.len())),
			22 => v.dedup(),
			_ => v = vec![make(arg); usize::from(arg % 6)],
		}
		put(out, v.len());
		put(out, v.capacity());
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
	put(out, q.capacity());
	for step in steps.chunks(2) {
		let (op, arg) = (step[0], step.get(1).copied().unwrap_or(0));
		match op % 20 {
			0 => q.push_back(make(arg)),
			1 => q.push_front(make(arg)),
			2 => drop(q.pop_front()),
			3 => q.reserve(amount(arg, q.len(), size)),
			4 => q.reserve_exact(amount(arg, q.len(), size)),
			5 => q.shrink_to(usize::from(arg % 40)),
			6 => q.shrink_to_fit(),
			7 => q.truncate(usize::from(arg % 8)),
			8 => q.insert(usize::from(arg) % (q.len() + 1), make(arg)),
			9 => drop(q.remove(usize::from(arg) % (q.len() + 1))),
			10 => q.extend((0..arg % 24).filter(|i| i % 3 != 0).map(make)),
			11 => q.extend((0..arg % 24).map(make)),
			12 => q.clear(),
			13 => {
				let mut keep = alternate();
				q.retain(|_| keep());
			}
			14 => drop(q.drain(..q.len().min(usize::from(arg % 4)))),
			15 => q = q.clone(),
			// Its own, and a `Vec`'s in place of one, and one it collects first.
			16 => q = q.into_iter().collect(),
			17 => q = VecDeque::from(q.into_iter().collect::<Vec<T>>()),
			18 => {
				let mut keep = alternate();
				q = q.into_iter().filter(|_| keep()).collect();
			}
			_ => q = VecDeque::from([make(arg), make(arg), make(arg)]),
		}
		put(out, q.len());
		put(out, q.capacity());
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
	put(out, h.capacity());
	let mut other = BinaryHeap::new();
	for step in steps.chunks(2) {
		let (op, arg) = (step[0], step.get(1).copied().unwrap_or(0));
		match op % 12 {
			0 | 1 => h.push(make(arg)),
			2 => drop(h.pop()),
			3 => h.reserve(amount(arg, h.len(), size)),
			4 => h.reserve_exact(amount(arg, h.len(), size)),
			5 => h.shrink_to(usize::from(arg % 40)),
			6 => h.shrink_to_fit(),
			7 => h.clear(),
			8 => h.extend((0..arg % 24).filter(|i| i % 3 != 0).map(make)),
			9 => {
				other.push(make(arg));
				h.append(&mut other);
				put(out, other.capacity());
			}
			10 => drop(h.drain()),
			_ => h = BinaryHeap::from(h.into_vec()),
		}
		put(out, h.len());
		put(out, h.capacity());
	}
	put(out, h.clone().into_vec().capacity());
	put(out, h.into_sorted_vec().capacity());
}

/// What collections each way std's makes them from an iterator hold room for: in place of
/// a `Vec` of elements of another size, by a tuple's `Extend`, by `vec!`, and by
/// `with_capacity` past what one may hold, which panics.
pub fn collects(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let n = data.first().map_or(0, |&b| usize::from(b % 24));
	let pairs: Vec<(u32, u32)> = Vec::with_capacity(n + usize::from(kind % 4));
	let mut pairs = pairs;
	pairs.extend((0..n as u32).map(|i| (i, i)));
	match kind % 8 {
		// Two `u32`s to one, which fit twice as many.
		0 => {
			let halves: Vec<u32> = pairs.into_iter().map(|(a, _)| a).collect();
			put(out, halves.capacity());
		}
		// Bytes in place of bytes, and pairs of bytes in place of bytes, which fit half.
		1 => {
			let mut bytes = Vec::with_capacity(n + 3);
			bytes.extend_from_slice(&data[..n.min(data.len())]);
			let odd: Vec<u8> = bytes.clone().into_iter().filter(|b| b % 2 == 1).collect();
			put(out, odd.capacity());
			let wide: Vec<u16> = bytes.into_iter().map(u16::from).collect();
			put(out, wide.capacity());
		}
		2 => {
			let (a, b): (Vec<u32>, VecDeque<u32>) = pairs.iter().filter(|(a, _)| a % 2 == 0).copied().unzip();
			put(out, a.capacity());
			put(out, b.capacity());
			let (c, d): (Vec<u8>, BinaryHeap<u32>) = pairs.iter().map(|&(a, b)| (a as u8, b)).unzip();
			put(out, c.capacity());
			put(out, d.capacity());
		}
		3 => {
			let filled = vec![0u16; n];
			put(out, filled.capacity());
			let units = vec![(); n];
			put(out, units.capacity());
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
			put(out, VecDeque::from(bytes.clone()).capacity());
			let heap = BinaryHeap::from(bytes);
			put(out, heap.capacity());
			put(out, heap.into_vec().capacity());
		}
		_ => {
			let bytes = vec![7u8; n];
			let mut grown = bytes.clone();
			grown.extend((0..n as u8).filter(|b| b % 2 == 0));
			put(out, grown.capacity());
			grown.shrink_to(n / 2);
			put(out, grown.capacity());
		}
	}
}
