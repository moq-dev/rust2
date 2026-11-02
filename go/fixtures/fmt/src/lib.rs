//! Formatting's failures and edges: an implementation that returns an error, which
//! `format!` and `to_string` panic on and a panic's message stops at, and a width or
//! precision past what a count holds.

use std::fmt;

/// Writes its text, then fails.
struct Failing(&'static str);

impl fmt::Display for Failing {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.0)?;
		Err(fmt::Error)
	}
}

/// Run the case the first byte picks on a count from the next eight.
pub fn run(input: &[u8]) -> Vec<u8> {
	let Some((&pick, data)) = input.split_first() else {
		return Vec::new();
	};
	let mut buf = [0u8; 8];
	let n = data.len().min(8);
	buf[..n].copy_from_slice(&data[..n]);
	let count = u64::from_be_bytes(buf) as usize;
	match pick % 6 {
		0 => format!("{:count$}|", 1).into_bytes(),
		1 => format!("{:.count$}|", "abcdef").into_bytes(),
		2 => format!("{:>count$.2}|", "xyz").into_bytes(),
		3 => format!("a{}b", Failing("partial")).into_bytes(),
		4 => Failing("partial").to_string().into_bytes(),
		_ => panic!("before {} after", Failing("partial")),
	}
}
