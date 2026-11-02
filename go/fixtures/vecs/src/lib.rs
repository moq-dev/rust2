//! A vector's capacity requests, which panic past `isize::MAX` bytes of elements and
//! allocate nothing for zero-sized ones, at any capacity.

/// Run the case the first byte picks on a capacity from the next eight.
pub fn run(input: &[u8]) -> Vec<u8> {
	let Some((&pick, data)) = input.split_first() else {
		return Vec::new();
	};
	let mut buf = [0u8; 8];
	let n = data.len().min(8);
	buf[..n].copy_from_slice(&data[..n]);
	let capacity = u64::from_be_bytes(buf) as usize;
	let mut out = Vec::new();
	match pick % 3 {
		0 => {
			let mut v = Vec::<()>::with_capacity(capacity);
			v.push(());
			out.extend_from_slice(&v.len().to_be_bytes());
		}
		1 => {
			let mut v = Vec::<u64>::with_capacity(capacity);
			v.push(7);
			out.extend_from_slice(&v.len().to_be_bytes());
		}
		_ => {
			// Signed constants to bytes, which Go would fold and refuse.
			out.extend_from_slice(&(-1i16).to_be_bytes());
			out.extend_from_slice(&(-2i32).to_le_bytes());
			out.extend_from_slice(&i64::MIN.to_be_bytes());
		}
	}
	out
}
