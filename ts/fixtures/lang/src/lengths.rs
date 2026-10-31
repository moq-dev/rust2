//! Functions generic over an array's length whose calls take values of types holding it:
//! arrays of bytes, of a type parameter's values, and of arrays, each at a length of none,
//! one, and more. Their writes are judged in the bounds of the body making them, as
//! filling memory nothing points into yet or giving what they replace another object.

use std::mem::MaybeUninit;

/// Each case at the length `kind` picks.
pub fn lengths(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	match kind % 4 {
		0 => each::<0>(data, out),
		1 => each::<1>(data, out),
		2 => each::<3>(data, out),
		_ => each::<17>(data, out),
	}
}

fn each<const N: usize>(data: &[u8], out: &mut Vec<u8>) {
	let rows: Vec<[u8; N]> = data
		.chunks(N.max(1))
		.map(|c| std::array::from_fn(|i| c.get(i).copied().unwrap_or(i as u8)))
		.collect();
	let wide: Vec<[u32; N]> = rows.iter().map(|r| r.map(|b| u32::from(b) << 8 | 1)).collect();
	joined(data, rows.clone(), out);
	joined(data, vec![[0xa5; N], std::array::from_fn(|i| i as u8)], out);
	retagged(&rows, out);
	recloned(&wide, out);
	filled(data, [0u8; N], out);
	filled(data, [0x0102_0304u32; N], out);
	filled(data, [[0x5au8; N]; 2], out);
	swapped(data, &rows, out);
	swapped(data, &wide, out);
}

/// The bytes of `data` and then of each row, as one run.
fn joined<const N: usize>(data: &[u8], rows: Vec<[u8; N]>, out: &mut Vec<u8>) {
	out.extend(data.iter().chain(rows.iter().flatten()));
}

#[derive(Clone, Copy)]
struct Tagged<T>(T, u8);

/// Each row cloned out of an `Option` kept in a pair and written back whole, after which
/// the pair and the copy each change apart.
fn retagged<const N: usize>(rows: &[[u8; N]], out: &mut Vec<u8>) {
	for (at, &row) in rows.iter().enumerate() {
		let mut pair = (Some(Tagged(row, at as u8)), 0u8);
		#[allow(clippy::clone_on_copy, reason = "the clone `Option`'s `Clone` makes")]
		let mut copy = pair.0.clone();
		pair.0 = copy;
		if let (Some(Tagged(a, _)), Some(Tagged(b, n))) = (&mut pair.0, &mut copy) {
			a.reverse();
			*n ^= 0xff;
			out.push(*n);
			out.extend(a.iter().chain(b.iter()));
		}
	}
}

/// [`retagged`] of a type parameter's values.
fn recloned<T: Copy + Into<u32>, const N: usize>(rows: &[[T; N]], out: &mut Vec<u8>) {
	for (at, &row) in rows.iter().enumerate() {
		let mut pair = (Some(Tagged(row, at as u8)), 0u8);
		#[allow(clippy::clone_on_copy, reason = "the clone `Option`'s `Clone` makes")]
		let mut copy = pair.0.clone();
		pair.0 = copy;
		if let (Some(Tagged(a, _)), Some(Tagged(b, n))) = (&mut pair.0, &mut copy) {
			a.reverse();
			*n ^= 0xff;
			out.push(*n);
			out.extend(a.iter().chain(b.iter()).map(|&x| x.into() as u8));
		}
	}
}

/// Slots of `MaybeUninit` arrays, each written whole through an iterator while nothing
/// points into them, then changed through a pointer to a part of one and read back.
fn filled<T: Copy + Put, const N: usize>(data: &[u8], fill: [T; N], out: &mut Vec<u8>) {
	let mut slots: [MaybeUninit<([T; N], u8)>; 3] = [const { MaybeUninit::uninit() }; 3];
	for (slot, at) in slots.iter_mut().zip(0u8..) {
		slot.write((fill, at));
	}
	for &byte in data {
		let slot = unsafe { slots[usize::from(byte) % 3].assume_init_mut() };
		let tag = &raw mut slot.1;
		unsafe {
			*tag ^= byte;
			out.push(*tag);
		}
	}
	for slot in &slots {
		let (values, tag) = unsafe { slot.assume_init_ref() };
		values.iter().for_each(|v| v.put(out));
		out.push(*tag);
	}
}

/// A value's bytes.
trait Put {
	fn put(&self, out: &mut Vec<u8>);
}

impl Put for u8 {
	fn put(&self, out: &mut Vec<u8>) {
		out.push(*self);
	}
}

impl Put for u32 {
	fn put(&self, out: &mut Vec<u8>) {
		out.extend(self.to_le_bytes());
	}
}

impl<const N: usize> Put for [u8; N] {
	fn put(&self, out: &mut Vec<u8>) {
		out.extend_from_slice(self);
	}
}

/// Rows of a grid swapped whole, each copy kept apart from the row it was read from as
/// either changes.
fn swapped<T: Copy + Into<u32>, const N: usize>(data: &[u8], rows: &[[T; N]], out: &mut Vec<u8>) {
	let Some((&first, rest)) = rows.split_first() else {
		return;
	};
	let mut grid = [first; 2];
	if let Some(&next) = rest.first() {
		grid[1] = next;
	}
	#[allow(clippy::manual_swap, reason = "the whole writes of each row")]
	for &byte in data {
		let row = grid[0];
		grid[0] = grid[1];
		grid[1] = row;
		if let (Some(x), Some(&y)) = (grid[usize::from(byte) % 2].first_mut(), rows[usize::from(byte) % rows.len()].last()) {
			*x = y;
		}
		out.extend(row.iter().chain(grid.iter().flatten()).map(|&x| x.into() as u8));
	}
}
