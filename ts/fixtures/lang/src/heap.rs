//! Memory from `alloc`, typed by a cast of its pointer, grown and shrunk by `realloc`, and
//! freed by `dealloc`, as smallvec's spilled storage is, and the `Layout`s it is laid out by.

use std::alloc::{self, Layout};
use std::mem;
use std::ptr::{self, NonNull};
use std::sync::Arc;

#[derive(Debug)]
enum Fault {
	Overflow,
	Alloc { layout: Layout },
}

const WORD: Layout = Layout::new::<u32>();

fn layout_array<T>(n: usize) -> Result<Layout, Fault> {
	let size = mem::size_of::<T>().checked_mul(n).ok_or(Fault::Overflow)?;
	Layout::from_size_align(size, mem::align_of::<T>()).map_err(|_| Fault::Overflow)
}

unsafe fn deallocate<T>(ptr: NonNull<T>, cap: usize) {
	let layout = layout_array::<T>(cap).unwrap();
	unsafe { alloc::dealloc(ptr.as_ptr() as *mut u8, layout) }
}

/// A spilled vector's storage: `len` of `cap` values written into memory from `alloc`.
struct Spill<T> {
	ptr: NonNull<T>,
	cap: usize,
	len: usize,
}

impl<T> Spill<T> {
	fn new(cap: usize) -> Result<Self, Fault> {
		let layout = layout_array::<T>(cap)?;
		let ptr = NonNull::new(unsafe { alloc::alloc(layout) })
			.ok_or(Fault::Alloc { layout })?
			.cast();
		Ok(Spill { ptr, cap, len: 0 })
	}

	/// Grows or shrinks to `cap`, dropping what no longer fits.
	fn resize(&mut self, cap: usize) -> Result<(), Fault> {
		while self.len > cap {
			self.len -= 1;
			unsafe { ptr::drop_in_place(self.ptr.as_ptr().add(self.len)) };
		}
		let old = layout_array::<T>(self.cap)?;
		let layout = layout_array::<T>(cap)?;
		let p = unsafe { alloc::realloc(self.ptr.as_ptr() as *mut u8, old, layout.size()) };
		self.ptr = NonNull::new(p).ok_or(Fault::Alloc { layout })?.cast();
		self.cap = cap;
		Ok(())
	}

	fn push(&mut self, value: T) -> Result<(), Fault> {
		if self.len == self.cap {
			self.resize(self.cap * 2 + 1)?;
		}
		unsafe { self.ptr.as_ptr().add(self.len).write(value) };
		self.len += 1;
		Ok(())
	}

	fn as_slice(&self) -> &[T] {
		unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
	}
}

impl<T> Drop for Spill<T> {
	fn drop(&mut self) {
		unsafe {
			ptr::drop_in_place(ptr::slice_from_raw_parts_mut(self.ptr.as_ptr(), self.len));
			deallocate(self.ptr, self.cap);
		}
	}
}

/// Values pushed into, and kept or dropped by resizing, storage of `u32`s, of `Arc`s,
/// whose strong counts show each drop, and of `Option` pairs.
pub fn spills(data: &[u8], out: &mut Vec<u8>) {
	let shared = Arc::new(7u8);
	let mut ints = Spill::<u32>::new(1).unwrap();
	let mut arcs = Spill::<Arc<u8>>::new(2).unwrap();
	let mut pairs = Spill::<Option<(u8, u16)>>::new(1).unwrap();
	for &b in data {
		match b % 4 {
			0 => ints.push(u32::from(b) * 1000).unwrap(),
			1 => arcs.push(shared.clone()).unwrap(),
			2 => pairs.push((b & 8 != 0).then_some((b, u16::from(b) << 4))).unwrap(),
			_ => {
				let cap = usize::from(b >> 2) % 5 + 1;
				ints.resize(cap).unwrap();
				arcs.resize(cap).unwrap();
			}
		}
		out.push(Arc::strong_count(&shared) as u8);
	}
	out.extend(ints.as_slice().iter().flat_map(|x| x.to_le_bytes()));
	out.push(ints.cap as u8);
	out.extend(pairs.as_slice().iter().map(|p| p.map_or(0, |(a, b)| a ^ b as u8)));
	drop(arcs);
	out.push(Arc::strong_count(&shared) as u8);
}

/// `Layout`s made of sizes and alignments up to wasm32's `isize::MAX`, and of arrays of
/// `u64`s and of values of no bytes, and what `Debug` and `Display` write of each.
pub fn layouts(data: &[u8], out: &mut Vec<u8>) {
	for part in data.chunks(5) {
		let mut word = [0u8; 4];
		word[..part.len().min(4)].copy_from_slice(&part[..part.len().min(4)]);
		let size = u32::from_le_bytes(word) as usize;
		let b = part.get(4).copied().unwrap_or(0);
		let align = match b & 0x80 {
			0 => usize::from(b),
			_ => 1 << (b % 32),
		};
		let made = Layout::from_size_align(size, align);
		out.extend(format!("{made:?}").bytes());
		if let Err(e) = made {
			out.extend(e.to_string().bytes());
		}
		out.extend(format!("{:?}", Layout::array::<u64>(size >> (b % 8))).bytes());
		let unit = Layout::array::<()>(size).unwrap();
		out.extend([unit.size() as u8, unit.align() as u8]);
	}
	out.extend(
		format!(
			"{:?} {WORD:?}",
			Fault::Alloc {
				layout: Layout::new::<(u8, u64)>()
			}
		)
		.bytes(),
	);
	out.push(mem::align_of::<u64>() as u8);
	out.extend(format!("{:?}", Fault::Overflow).bytes());
}
