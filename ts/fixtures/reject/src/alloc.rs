//! Memory from `alloc` and raw pointers read as other types' bytes or as addresses.

use crate::pointers::Halves;

/// A pointer read as another type's bytes.
pub fn reinterpreted(n: u32) -> u8 {
	unsafe { *(&raw const n).cast::<u8>() }
}

/// A pointer's address, as an integer.
pub fn addressed(n: u32) -> usize {
	(&raw const n).addr()
}

/// Memory from the allocator as bytes, which JS keeps only as values a cast types it as.
pub fn allocated() -> u8 {
	let layout = std::alloc::Layout::new::<u8>();
	unsafe {
		let p = std::alloc::alloc(layout);
		*p = 1;
		let n = *p;
		std::alloc::dealloc(p, layout);
		n
	}
}

/// A whole value written through a pointer from a shared reference to a `Result` of a cell
/// whose only inhabited variant leaves it no tag, all of it in an `UnsafeCell`, which JS
/// can't write into in place.
pub fn infallible_cell(r: &Result<std::cell::Cell<u32>, std::convert::Infallible>) {
	unsafe { std::ptr::from_ref(r).cast_mut().write(Ok(std::cell::Cell::new(19))) }
}

/// A pointer from a reference the body is given, which JS keeps in no array, moved past
/// it.
pub fn past_reference(r: &u32) -> u32 {
	unsafe { *(r as *const u32).add(2) }
}

/// Casts between pointers to bytes and to values outside memory from `alloc`, which read
/// the bytes of one type as another: of an array's bytes to `u32`s, of memory from
/// `alloc` to the crate's struct, which JS keeps as an object, and of a pointer to values
/// to one to their bytes, read rather than freed; and a byte of memory from `alloc` written
/// by `ptr::write`.
pub fn bytes_as_values(bytes: [u8; 4]) -> u32 {
	let layout = std::alloc::Layout::new::<Halves>();
	unsafe {
		std::ptr::write(std::alloc::alloc(layout), 2);
		let words = bytes.as_ptr() as *const u32;
		let halves = std::alloc::alloc(layout) as *mut Halves;
		let n = 7u32;
		let byte = *((&raw const n) as *const u8);
		*words + (*halves).low + u32::from(byte)
	}
}

/// Memory from `alloc` cast to pointers to two types, which reads one's bytes as the
/// other's, also once `realloc` grows it.
pub fn alloc_retyped() -> i32 {
	let layout = std::alloc::Layout::new::<u32>();
	unsafe {
		let raw = std::alloc::alloc(layout);
		raw.cast::<u32>().write(u32::MAX);
		let n = (raw as *mut i32).read();
		let grown = std::alloc::realloc(raw, layout, 8);
		let m = grown.cast::<i32>().read();
		let words = grown.cast::<u32>();
		let again = std::alloc::realloc(words.cast::<u8>(), std::alloc::Layout::array::<u32>(2).unwrap(), 12);
		let k = again.cast::<i32>().read();
		std::alloc::dealloc(again, std::alloc::Layout::array::<u32>(3).unwrap());
		n + m + k
	}
}

/// Memory from `alloc_zeroed`, whose zeroed bytes JS can't type.
pub fn zeroed() -> u32 {
	unsafe { *(std::alloc::alloc_zeroed(std::alloc::Layout::new::<u32>()) as *mut u32) }
}
