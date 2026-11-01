//! Pointers into arrays: a `Vec`'s, a slice's, an array's, a boxed slice's, and a
//! `MaybeUninit` array's elements, moved by `add`, `sub`, `offset`, and their wrapping
//! forms, measured by `offset_from`, copied between by `ptr::copy` and
//! `copy_nonoverlapping`, overlapping either way, and made slices again, as `smallvec`'s
//! inline storage and moq-net's frame segments use them, and the edges: addresses that
//! wrap, a value in no array, and a slice of none.

use std::mem::MaybeUninit;
use std::ptr::{self, NonNull};
use std::slice;
use std::sync::Arc;

/// A `Vec`'s elements moved within it as `insert` and `remove` move them, and copied to
/// another, read back through slices made of the pointers.
pub fn vecs(data: &[u8], out: &mut Vec<u8>) {
	let mut v: Vec<u32> = data.iter().map(|&b| u32::from(b) * 3).collect();
	v.extend([7, 8]);
	let len = v.len();
	let at = usize::from(data.first().copied().unwrap_or(0)) % (len - 1);
	let p = v.as_mut_ptr();
	unsafe {
		// The tail moved right over the last element, and the gap filled.
		ptr::copy(p.add(at), p.add(at + 1), len - at - 1);
		*p.add(at) = 1000;
		emit(slice::from_raw_parts(p, len), out);
		// And moved back left.
		ptr::copy(p.add(at + 1), p.add(at), len - at - 1);
		*p.add(len - 1).sub(0) = 2000;
		emit(slice::from_raw_parts(p, len), out);
		// Pointers moved forward and back, and measured.
		let end = p.add(len);
		let last = end.offset(-1);
		*last.wrapping_sub(1) += 1;
		*p.wrapping_add(1).wrapping_offset(-1) += 5;
		out.push(end.offset_from(p) as u8);
		out.push(p.offset_from(last) as u8);
		out.push(last.offset_from(last) as u8);
		// A part of it, written through a mutable slice of the pointers.
		let part = slice::from_raw_parts_mut(p.add(at), len - at);
		part[0] += 3;
		part.reverse();
		let mut w = vec![0u32; len];
		ptr::copy_nonoverlapping(p, w.as_mut_ptr(), len);
		w.as_mut_ptr().add(at).copy_from(p, 1);
		p.copy_to_nonoverlapping(w.as_mut_ptr().add(len - 1), 1);
		emit(&w, out);
	}
	emit(&v, out);
	let shared: &[u32] = &v;
	let q = shared.as_ptr();
	out.push(unsafe { *q.add(len / 2) } as u8);
	// A slice of none from a dangling pointer, as an empty collection's is.
	let none = unsafe { slice::from_raw_parts(NonNull::<u32>::dangling().as_ptr(), 0) };
	out.push(none.len() as u8);
}

/// An array's elements, from a pointer to one, to the array, and to its first.
pub fn arrays(data: &[u8], out: &mut Vec<u8>) {
	for &n in data {
		let mut a = [u16::from(n), 1, 2, 3, 4];
		let i = usize::from(n) % 4;
		// Each pointer is read and written through before the next borrow of `a`.
		let at = &raw mut a[i];
		unsafe { *at = 99 };
		let whole = (&raw mut a).cast::<u16>();
		unsafe {
			*whole.add(i + 1) += 10;
			*whole.add(4) += u16::from(n);
		}
		let first = a.as_mut_ptr();
		unsafe {
			out.push(at.offset_from(whole) as u8);
			out.push(first.offset_from(at) as u8);
			let p = NonNull::new_unchecked(first);
			*p.add(2).as_ptr() += 1;
			*p.add(3).sub(3).as_ptr() += 2;
			out.push(p.add(4).offset_from(p) as u8);
			let tail = NonNull::slice_from_raw_parts(p.add(i), 5 - i);
			out.push(tail.as_ref().len() as u8);
			tail.as_ptr().cast::<u16>().write(7);
		}
		// A pointer to `()` stays where it is.
		let unit = NonNull::<()>::dangling();
		let moved = unsafe { unit.add(usize::from(n)) };
		out.push(u8::from(moved == unit));
		out.extend(a.iter().flat_map(|x| x.to_le_bytes()));
	}
}

/// A boxed slice of bytes kept behind a pointer to its first, as moq-net's frame
/// segment keeps its storage: written by copies at offsets, read by slices of it, and
/// freed as the slice it was.
pub fn segments(data: &[u8], out: &mut Vec<u8>) {
	let segment = Storage::new(data.len() + 4);
	unsafe {
		segment.write(0, data);
		segment.write(data.len(), &[1, 2, 3, 4]);
		out.extend_from_slice(segment.read(data.len() + 4));
		// Overlapping moves of its bytes, either way.
		let p = segment.data;
		ptr::copy(p, p.add(2), data.len());
		out.extend_from_slice(segment.read(data.len() + 4));
		ptr::copy(p.add(3), p.add(1), data.len());
		*p.add(data.len() + 3) ^= 0xff;
		out.extend_from_slice(segment.read(data.len() + 4));
	}
	// A `Vec`'s bytes and an array's, through pointers to them.
	let mut v = data.to_vec();
	v.push(9);
	let mut a = [0u8; 4];
	unsafe {
		let p = v.as_mut_ptr();
		*p.add(v.len() - 1) += 1;
		ptr::copy_nonoverlapping(p, (&raw mut a).cast::<u8>(), v.len().min(4));
		let byte = (&raw mut a).cast::<u8>().add(1);
		*byte.add(2) = 0x55;
		out.extend_from_slice(slice::from_raw_parts(byte.sub(1), 4));
		let one = &raw mut a[2];
		*one ^= 0x0f;
		out.push(*one);
	}
	out.extend(v);
}

struct Storage {
	data: *mut u8,
	capacity: usize,
}

impl Storage {
	fn new(capacity: usize) -> Self {
		let boxed: Box<[u8]> = vec![0u8; capacity].into_boxed_slice();
		let capacity = boxed.len();
		let data = Box::into_raw(boxed) as *mut u8;
		Self { data, capacity }
	}

	unsafe fn write(&self, offset: usize, src: &[u8]) {
		assert!(offset + src.len() <= self.capacity);
		unsafe { ptr::copy_nonoverlapping(src.as_ptr(), self.data.add(offset), src.len()) };
	}

	unsafe fn read(&self, len: usize) -> &[u8] {
		unsafe { slice::from_raw_parts(self.data, len) }
	}
}

impl Drop for Storage {
	fn drop(&mut self) {
		unsafe {
			let slice = ptr::slice_from_raw_parts_mut(self.data, self.capacity);
			drop(Box::from_raw(slice));
		}
	}
}

/// Up to four values kept inline in a `MaybeUninit` array, as `smallvec` keeps them,
/// inserted and removed by moving the rest through pointers, and dropped in place.
struct Inline<T> {
	len: usize,
	items: MaybeUninit<[T; 4]>,
}

impl<T> Inline<T> {
	fn new() -> Self {
		Inline {
			len: 0,
			items: MaybeUninit::uninit(),
		}
	}

	fn ptr(&mut self) -> *mut T {
		self.items.as_mut_ptr() as *mut T
	}

	fn as_slice(&self) -> &[T] {
		unsafe { slice::from_raw_parts(self.items.as_ptr() as *const T, self.len) }
	}

	fn insert(&mut self, at: usize, value: T) {
		assert!(self.len < 4 && at <= self.len);
		let len = self.len;
		unsafe {
			let p = self.ptr().add(at);
			ptr::copy(p, p.add(1), len - at);
			ptr::write(p, value);
		}
		self.len += 1;
	}

	fn remove(&mut self, at: usize) -> T {
		assert!(at < self.len);
		self.len -= 1;
		let len = self.len;
		unsafe {
			let p = self.ptr().add(at);
			let value = ptr::read(p);
			ptr::copy(p.add(1), p, len - at);
			value
		}
	}
}

impl<T> Drop for Inline<T> {
	fn drop(&mut self) {
		let len = self.len;
		unsafe { ptr::drop_in_place(ptr::slice_from_raw_parts_mut(self.ptr(), len)) };
	}
}

/// Shared values and `Option`s of pairs moved through an inline array, counting the
/// shared values' owners, and a pair changed in place after its copy moved.
pub fn inline(data: &[u8], out: &mut Vec<u8>) {
	let shared = Arc::new(0u8);
	let mut held: Inline<Arc<u8>> = Inline::new();
	let mut pairs = Inline::new();
	for (i, &n) in data.iter().enumerate() {
		if held.len == 4 {
			let gone = held.remove(usize::from(n) % 4);
			out.push(*gone);
			pairs.remove(usize::from(n) % 4);
		}
		let at = usize::from(n) % (held.len + 1);
		let item = match n % 3 {
			0 => shared.clone(),
			_ => Arc::new(n),
		};
		held.insert(at, item);
		pairs.insert(at, (n % 2 == 0).then_some((n, i as u8)));
		if let Some(Some(pair)) = pairs.as_slice().first() {
			out.push(pair.0);
		}
		if let Some(Some(pair)) = unsafe { pairs.ptr().as_mut() } {
			pair.1 = pair.1.wrapping_add(1);
		}
		out.push(Arc::strong_count(&shared) as u8);
		out.extend(held.as_slice().iter().map(|x| **x));
		out.extend(pairs.as_slice().iter().flat_map(|p| p.map_or([0, 0], |(a, b)| [a, b])));
	}
	drop(held);
	out.push(Arc::strong_count(&shared) as u8);
	// Copies that change apart.
	let mut src = vec![Some((1u8, 2u8)), None, data.first().map(|&n| (n, 4))];
	let mut dst = vec![None; 3];
	unsafe { ptr::copy_nonoverlapping(src.as_ptr(), dst.as_mut_ptr(), 3) };
	if let Some(pair) = &mut dst[0] {
		pair.0 = 9;
	}
	if let Some(pair) = &mut src[2] {
		pair.1 = 8;
	}
	out.extend(src.iter().chain(&dst).flat_map(|p| p.map_or([0, 0], |(a, b)| [a, b])));
}

/// Pointers wrapped around wasm32's 32-bit addresses and back, to a value in no array, as
/// the array of one Rust treats it as, and into a mutable part of an array, measured from
/// the array, the pointer an empty slice keeps, and within a shared part, which JS copies.
pub fn edges(data: &[u8], out: &mut Vec<u8>) {
	for &n in data {
		let bytes = [n, 1, 2, 3];
		let p = bytes.as_ptr();
		let wide = [u16::from(n), 7, 8];
		let q = wide.as_ptr();
		unsafe {
			out.push(*p.wrapping_add(usize::MAX).wrapping_add(1));
			out.push(*p.wrapping_add(1 << 31).wrapping_add(1 << 31));
			out.push(*p.wrapping_sub(usize::MAX).wrapping_offset(-1));
			out.extend(q.wrapping_add(1 << 31).wrapping_add(1).read().to_le_bytes());
			out.extend(
				q.wrapping_add(3 << 30)
					.wrapping_add(1 << 30)
					.wrapping_add(2)
					.read()
					.to_le_bytes(),
			);
		}
		let mut lone = u32::from(n) + 42;
		let at = &raw mut lone;
		unsafe {
			out.push(slice::from_raw_parts(at, 1)[0] as u8);
			slice::from_raw_parts_mut(at, 1)[0] += 1;
			out.push(at.add(1).sub(1).read() as u8);
			out.push(at.add(1).offset_from(at) as u8);
			out.push(slice::from_raw_parts(at, 0).as_ptr().offset_from(at) as u8);
		}
		out.push(lone as u8);
		let byte = n ^ 0x5a;
		out.push(unsafe { slice::from_raw_parts(&raw const byte, 1)[0] });
		let mut a = [u32::from(n), 1, 2, 3];
		let base = a.as_mut_ptr();
		let part = &mut a[1..3];
		let at = &raw mut part[1];
		unsafe {
			out.push(at.offset_from(base) as u8);
			*at += 5;
			let empty = slice::from_raw_parts(at.add(1), 0);
			out.push(empty.as_ptr().offset_from(base) as u8);
			let none = slice::from_raw_parts(bytes.as_ptr().add(2), 0);
			out.push(none.as_ptr().offset_from(p) as u8);
		}
		out.extend(a.iter().map(|&x| x as u8));
		// Within a shared subslice, a copy, and to its end.
		let lanes = [u32::from(n), 4, 5, 6];
		let shared = &lanes[1..3];
		let first = shared.as_ptr();
		unsafe {
			out.push(*first.add(1) as u8);
			out.push(first.add(2).offset_from(first) as u8);
			out.push(slice::from_raw_parts(first.add(1), 1)[0] as u8);
		}
	}
}

fn emit(values: &[u32], out: &mut Vec<u8>) {
	out.extend(values.iter().flat_map(|x| x.to_le_bytes()));
}
