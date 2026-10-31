//! Arrays `array::from_fn` makes, calling its function on each index in order: of bytes,
//! wide integers, atomics each its own, strings, nested arrays, none at all, and a length
//! a const parameter gives; arrays `map` makes; arrays' elements each lent; arrays as
//! slices, written through; and arrays of bytes std is given by reference.

use std::borrow::{Borrow, BorrowMut};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

/// Each byte an operation, its value the operand; each puts what it read.
pub fn from_fns(data: &[u8], out: &mut Vec<u8>) {
	for &op in data {
		let s = match op % 5 {
			// The order the function sees, through what it captured by `&mut`.
			0 => {
				let mut seen = Vec::new();
				let bytes: [u8; 4] = std::array::from_fn(|i| {
					seen.push(i);
					op.wrapping_mul(i as u8 + 1)
				});
				let wide: [u64; 3] = std::array::from_fn(|i| u64::from(op) << (i * 30));
				format!("{bytes:?} {wide:?} {seen:?}")
			}
			// kio's: each atomic its own, which a store to one doesn't change in another.
			1 => {
				let counts: [AtomicU64; 8] = std::array::from_fn(|_| AtomicU64::new(u64::from(op)));
				counts[usize::from(op % 8)].fetch_add(1, Ordering::Relaxed);
				let copy: [AtomicU64; 8] = std::array::from_fn(|i| AtomicU64::new(counts[i].load(Ordering::Relaxed)));
				copy[0].store(0, Ordering::Relaxed);
				let loads: [u64; 8] = std::array::from_fn(|i| counts[i].load(Ordering::Relaxed));
				format!("{loads:?} {}", copy[0].load(Ordering::Relaxed))
			}
			// A function that owns what it captured, which drops after the last call.
			2 => {
				let word = format!("w{op}");
				let words: [String; 3] = std::array::from_fn(move |i| format!("{word}{i}"));
				let none: [String; 0] = std::array::from_fn(|i| i.to_string());
				format!("{words:?} {none:?}")
			}
			3 => {
				let grid: [[u8; 2]; 3] =
					std::array::from_fn(|i| std::array::from_fn(|j| op.wrapping_add((i * 2 + j) as u8)));
				let named: [u16; 4] = std::array::from_fn(doubled);
				format!("{grid:?} {named:?}")
			}
			_ => format!("{:?} {:?}", counted::<2>(op), counted::<5>(op)),
		};
		out.extend_from_slice(s.as_bytes());
		out.push(b'\n');
	}
}

fn doubled(i: usize) -> u16 {
	i as u16 * 2
}

/// An array as long as `N` says, of `N` instances.
fn counted<const N: usize>(op: u8) -> [usize; N] {
	std::array::from_fn(|i| i * N + usize::from(op))
}

/// Arrays `map` makes, calling its closure on each element in order; each byte an
/// operation, its value the operand. From bytes and to them, of none at all, through what
/// the closure captured by reference and by move, of `Option`s whose `None` is `rt.None`,
/// and of arrays, each element the closure's own to change.
pub fn maps(data: &[u8], out: &mut Vec<u8>) {
	for &op in data {
		let s = match op % 5 {
			0 => {
				let mut seen = Vec::new();
				let bytes = [op, op ^ 1, op ^ 2, op ^ 3];
				let wide: [u16; 4] = bytes.map(|b| {
					seen.push(b);
					u16::from(b) << 8 | u16::from(op)
				});
				let narrow: [u8; 4] = wide.map(|w| (w >> 4) as u8);
				let same = narrow.map(|b| b.wrapping_mul(op));
				let words = [u32::from(op), 7, 9].map(|w| w.wrapping_mul(0x0100_0193));
				format!("{bytes:?} {wide:?} {narrow:?} {same:?} {words:?} {seen:?}")
			}
			1 => {
				let none: [u8; 0] = [];
				let wide: [u16; 0] = none.map(u16::from);
				let narrow: [u8; 0] = wide.map(|w| w as u8);
				let words: [String; 0] = [0u32; 0].map(|n| n.to_string());
				format!("{wide:?} {narrow:?} {words:?}")
			}
			2 => {
				let word = format!("w{op}");
				let words = [1u8, 2, 3].map(move |i| format!("{word}{i}"));
				let shown = format!("{words:?}");
				let lens = words.map(|w| w.len() as u8 + op % 3);
				format!("{shown} {lens:?}")
			}
			3 => {
				let options = [Some(op), None, Some(op.wrapping_add(60))];
				let nested: [Option<Option<u8>>; 3] = options.map(|o| o.map(|b| b.checked_add(200)));
				let flat = nested.map(|o| match o {
					Some(inner) => inner,
					None => Some(0),
				});
				let units: [Option<()>; 3] = nested.map(|o| o.map(|_| ()));
				format!("{options:?} {nested:?} {flat:?} {units:?}")
			}
			_ => {
				let grid = [[op, 1], [2, 3], [4, op]];
				let flipped = grid.map(|mut row| {
					row[0] ^= 0xff;
					row.reverse();
					row
				});
				let sums = flipped.map(|row| u16::from(row[0]) + u16::from(row[1]));
				let mut same = grid.map(|row| row);
				same[usize::from(op) % 3][0] ^= 0x80;
				format!("{grid:?} {flipped:?} {sums:?} {same:?} {:?}", shifted::<3>(op))
			}
		};
		out.extend_from_slice(s.as_bytes());
		out.push(b'\n');
	}
}

/// An array as long as `N` says, mapped by a function.
fn shifted<const N: usize>(op: u8) -> [u64; N] {
	[op; N].map(u64::from)
}

/// Arrays' elements lent by `each_ref` and `each_mut`, each byte an operation, its value
/// the operand: bytes, wide integers, and strings, each lent as its place, arrays of bytes
/// as their views, and other arrays as themselves. Writes through each land in the array.
pub fn lent(data: &[u8], out: &mut Vec<u8>) {
	let mut bytes = [0u8; 3];
	let mut wide = [0u16; 2];
	let mut words = [String::new(), String::new()];
	let mut grid = [[0u8; 2]; 2];
	let mut pairs = [[0u32; 2]; 2];
	let mut none: [u8; 0] = [];
	for &op in data {
		let i = usize::from(op);
		match op % 6 {
			0 => {
				*bytes.each_mut()[i % 3] ^= op;
				for b in bytes.each_mut() {
					*b = b.wrapping_add(1);
				}
			}
			1 => {
				let [first, second] = wide.each_mut();
				*first = first.wrapping_add(u16::from(op) << 7);
				std::mem::swap(first, second);
			}
			2 => words.each_mut()[i % 2].push(char::from(b'a' + op % 26)),
			3 => {
				let rows = grid.each_mut();
				rows[i % 2][i % 3 % 2] = op;
				rows[(i + 1) % 2].reverse();
				let [left, right] = pairs.each_mut();
				left[i % 2] = u32::from(op) << 20;
				right.copy_from_slice(&[left[1], u32::from(op)]);
			}
			4 => {
				for b in none.each_mut() {
					*b = op;
				}
				let lens = words.each_ref().map(|w| w.len() as u8);
				let rows = grid.each_ref().map(|row| row[0] ^ row[1]);
				let sums = pairs.each_ref().map(|pair| pair[0].wrapping_add(pair[1]));
				out.extend_from_slice(&lens);
				out.extend_from_slice(&rows);
				out.extend_from_slice(&sums[i % 2].to_le_bytes());
			}
			_ => {
				let read: [&u8; 3] = bytes.each_ref();
				let wides: [&u16; 2] = wide.each_ref();
				let s = format!("{read:?} {wides:?} {:?}", none.each_ref());
				out.extend_from_slice(s.as_bytes());
			}
		}
	}
	let s = format!("{bytes:?} {wide:?} {words:?} {grid:?} {pairs:?}");
	out.extend_from_slice(s.as_bytes());
}

/// Arrays as slices, of bytes and of other values, empty ones included; each byte an
/// operation, its value the operand. Writes through `as_mut_slice` land in the array.
pub fn as_slices(data: &[u8], out: &mut Vec<u8>) {
	let mut bytes = [0u8; 4];
	let mut wide = [0u16; 3];
	let mut words = [String::new(), String::new()];
	let mut grid = [[0u8; 2]; 3];
	let mut pairs = [[0u32; 2]; 2];
	let mut none: [u16; 0] = [];
	let mut empty: [u8; 0] = [];
	for &op in data {
		let i = usize::from(op);
		match op % 8 {
			0 => {
				let s = bytes.as_mut_slice();
				s[i % 4] = op;
				s.rotate_left(1);
			}
			1 => {
				wide.as_mut_slice()[i % 3] = u16::from(op) << 7;
				wide.as_mut_slice().rotate_right(1);
			}
			2 => words.as_mut_slice()[i % 2].push(char::from(b'a' + op % 26)),
			// An element of one array as a slice, as the slice of the outer array lends it.
			3 => {
				let row = &mut grid.as_mut_slice()[i % 3];
				row.as_mut_slice()[i % 2] = op;
				let read = row.as_slice()[(i + 1) % 2];
				grid[(i + 1) % 3][0] ^= read;
			}
			4 => {
				let all = pairs.as_mut_slice();
				all[i % 2]
					.as_mut_slice()
					.copy_from_slice(&[u32::from(op), u32::from(op) << 20]);
				all.swap(0, 1);
			}
			5 => {
				none.as_mut_slice().reverse();
				empty.as_mut_slice().fill(op);
				out.push(none.as_slice().len() as u8 + empty.as_slice().len() as u8);
			}
			6 => written(&mut bytes, &mut wide, op),
			_ => {
				let sum: u32 = wide.as_slice().iter().map(|&w| u32::from(w)).sum();
				out.extend_from_slice(&sum.to_le_bytes());
				out.extend_from_slice(bytes.as_slice());
			}
		}
	}
	let s = format!(
		"{:?} {:?} {:?} {:?} {:?} {:?} {:?}",
		bytes.as_slice(),
		wide.as_slice(),
		words.as_slice(),
		grid.as_slice(),
		pairs.as_slice(),
		none.as_slice(),
		empty.as_slice()
	);
	out.extend_from_slice(s.as_bytes());
}

/// Arrays lent by `&mut`, written through their slices.
fn written(bytes: &mut [u8; 4], wide: &mut [u16; 3], op: u8) {
	let s = bytes.as_mut_slice();
	s[3] = s[0].wrapping_add(op);
	let w = wide.as_mut_slice();
	w[0] = u16::from(op);
	w.swap(1, 2);
	w[1..].fill(u16::from(op) + 1);
}

/// Arrays of bytes std is given by reference, as the `&T` of a `T = [u8; N]`: copied out of
/// an `Option`, found among others, lent back, formatted, compared by `assert_eq!` and
/// `assert_ne!`, and swapped, replaced, and taken through a `&mut`, of locals, elements,
/// fields, and parts of a longer slice. Writes through each land in the array, and a copy shares nothing with
/// it. Of no bytes, one, three, and many.
pub fn given(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	match kind % 4 {
		0 => {
			given_n::<0>(data, out);
			taken::<0>(data, out);
		}
		1 => {
			given_n::<1>(data, out);
			taken::<1>(data, out);
		}
		2 => {
			given_n::<3>(data, out);
			taken::<3>(data, out);
			fields(data, out);
		}
		_ => {
			given_n::<40>(data, out);
			taken::<32>(data, out);
		}
	}
}

/// `N` bytes of `data` from `at`, past its end each its index.
fn bytes_at<const N: usize>(data: &[u8], at: usize) -> [u8; N] {
	std::array::from_fn(|i| data.get(at + i).copied().unwrap_or((at + i) as u8))
}

/// A field std is given a `&mut` to.
struct Framed {
	bytes: [u8; 3],
}

/// An array formatted through its own `Debug`, given a reference to it.
struct Bracketed([u8; 3]);

impl fmt::Debug for Bracketed {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("<")?;
		fmt::Debug::fmt(&self.0, f)?;
		f.write_str(">")
	}
}

/// Each array's bytes, in order.
fn put<const N: usize>(out: &mut Vec<u8>, arrays: &[[u8; N]]) {
	for a in arrays {
		out.extend_from_slice(a);
	}
}

/// The bytes of a `Some`, or 0xff for a `None`.
fn put_some<const N: usize>(out: &mut Vec<u8>, a: Option<[u8; N]>) {
	match a {
		Some(a) => out.extend_from_slice(&a),
		None => out.push(0xff),
	}
}

/// [`given`] of `N` bytes, all but `mem::take`.
fn given_n<const N: usize>(data: &[u8], out: &mut Vec<u8>) {
	let mut a: [u8; N] = bytes_at(data, 0);
	let mut b: [u8; N] = bytes_at(data, N);
	// A copy out of an `Option` of either reference shares nothing with the array.
	let copied = Some(&a).copied();
	let cloned = Some(&mut b).cloned();
	if let Some(x) = a.first_mut() {
		*x ^= 0x01;
	}
	if let Some(x) = b.last_mut() {
		*x ^= 0x02;
	}
	put(out, &[a, b]);
	put_some(out, copied);
	put_some(out, cloned);
	// Each holds the other's bytes, and a write to one after leaves the other.
	std::mem::swap(&mut a, &mut b);
	if let Some(x) = a.first_mut() {
		*x = x.wrapping_add(1);
	}
	put(out, &[a, b]);
	// Elements, swapped with a local and replaced by one.
	let mut v: Vec<[u8; N]> = (0..3).map(|k| bytes_at(data, k * 2)).collect();
	std::mem::swap(&mut v[0], &mut a);
	let old = std::mem::replace(&mut v[2], b);
	b.fill(0xbb);
	put(out, &[old, a, b]);
	put(out, &v);
	out.extend([v.contains(&a), v.contains(&b), v[..1].contains(&old)].map(u8::from));
	// One returned by `IndexMut`, swapped with another.
	let (first, rest) = v.split_at_mut(1);
	std::mem::swap(&mut first[0], &mut rest[1]);
	put(out, &v);
	// Parts of a longer slice, at an offset into it.
	let mut buf: Vec<u8> = (0..2 * N + 1)
		.map(|i| data.get(i).map_or(i as u8 ^ 0x55, |d| d ^ 0x80))
		.collect();
	out.push(u8::from(v.contains(buf.last_chunk::<N>().unwrap())));
	let part = buf[1..].first_chunk::<N>().copied();
	if let Some((x, rest)) = buf.split_first_chunk_mut::<N>()
		&& let Some(y) = rest.last_chunk_mut::<N>()
	{
		std::mem::swap(x, y);
	}
	out.extend_from_slice(&buf);
	let old = buf.last_chunk_mut::<N>().map(|t| std::mem::replace(t, a));
	let cloned = buf.last_chunk_mut::<N>().cloned();
	if let Some(x) = buf.last_mut() {
		*x ^= 0x40;
	}
	put_some(out, part);
	put_some(out, old);
	put_some(out, cloned);
	out.extend_from_slice(&buf);
	// Lent back as one, sharing the array's bytes.
	let one = std::slice::from_ref(&a);
	out.push(one.len() as u8);
	put(out, one);
	let borrowed: &[u8; N] = Borrow::borrow(&v[0]);
	out.extend_from_slice(borrowed);
	let lent: &mut [u8; N] = BorrowMut::borrow_mut(&mut b);
	if let Some(x) = lent.last_mut() {
		*x = 0xee;
	}
	put(out, &[b]);
	out.extend_from_slice(format!("{a:?} {:?}", std::slice::from_ref(&b)).as_bytes());
	// Panics with std's message where they differ.
	if data.first().is_some_and(|d| d % 3 == 0) {
		assert_eq!(a, b, "after {}", data.len());
	}
}

/// [`given`] of a field's `[u8; 3]`, swapped, replaced, taken, and formatted through its
/// own `Debug`.
fn fields(data: &[u8], out: &mut Vec<u8>) {
	let mut a: [u8; 3] = bytes_at(data, 2);
	let mut held = Framed {
		bytes: bytes_at(data, 0),
	};
	std::mem::swap(&mut held.bytes, &mut a);
	let old = std::mem::replace(&mut held.bytes, [a[2], a[1], a[0]]);
	held.bytes[0] ^= 0x10;
	put(out, &[a, old, held.bytes]);
	let shown = format!("{:?}", Bracketed(held.bytes));
	let taken = std::mem::take(&mut held.bytes);
	put(out, &[taken, held.bytes]);
	out.extend_from_slice(shown.as_bytes());
	if data.first().is_some_and(|d| d % 3 == 1) {
		assert_ne!(old, a, "fields of {}", data.len());
	}
}

/// [`given`] of `N` bytes, by `mem::take`, which `Default` bounds.
fn taken<const N: usize>(data: &[u8], out: &mut Vec<u8>)
where
	[u8; N]: Default,
{
	let mut a: [u8; N] = bytes_at(data, 1);
	let t = std::mem::take(&mut a);
	put(out, &[t, a]);
	let mut buf: Vec<u8> = data.iter().rev().copied().chain([0x77; N]).collect();
	if let Some(c) = buf.last_chunk_mut::<N>() {
		c.fill(0x66);
		let c = std::mem::take(c);
		out.extend_from_slice(&c);
	}
	out.extend_from_slice(&buf);
}
