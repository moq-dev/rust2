//! The entry point both sides of the differential test run. The first byte picks a
//! case and the rest is its input.

use bytes::Bytes;
use rand::RngExt;

/// What a case runs on the rest of the input.
type Case = fn(&[u8], &mut Vec<u8>);

/// Each case's name and what it runs. The first byte picks one modulo their count, so a
/// case added at the end moves no other.
fn cases() -> Vec<(&'static str, Case)> {
	vec![
		("slices", slices),
		("made", made),
		("events", events),
		("random", random),
		("buf::bytes_mut", crate::buf::bytes_mut),
		("buf::reads", crate::buf::reads),
		("buf::writes", crate::buf::writes),
		("buf::edits", crate::buf::edits),
		("buf::reserves", crate::buf::reserves),
	]
}

/// Run one case.
pub fn run(input: &[u8]) -> Vec<u8> {
	let mut out = Vec::new();
	let Some((&op, data)) = input.split_first() else {
		return out;
	};
	let cases = cases();
	let (_, case) = cases[usize::from(op) % cases.len()];
	case(data, &mut out);
	out
}

/// The first byte that picks the case named `name`, which `caller.ts` targets by.
pub fn op(name: &str) -> u8 {
	match cases().iter().position(|&(n, _)| n == name) {
		Some(i) => i as u8,
		None => panic!("no case {name}"),
	}
}

/// `Bytes`'s slices by every kind of range, out of bounds included, which panic as
/// `bytes` does, and what they compare equal to.
fn slices(data: &[u8], out: &mut Vec<u8>) {
	let [kind, a, b, rest @ ..] = data else {
		return;
	};
	let bytes = Bytes::copy_from_slice(rest);
	// One past `usize::MAX`, which an inclusive range can't end at.
	let past = *b == 0xff;
	let (a, b) = (*a as usize % (rest.len() + 2), *b as usize % (rest.len() + 2));
	if kind & 0x80 != 0 {
		// Unused, but still checked: past the end, it panics.
		let _ = bytes.slice(b..);
	}
	let part = match kind % 7 {
		0 => bytes.slice(a..b),
		1 => bytes.slice(a..),
		2 => bytes.slice(..b),
		3 => bytes.slice(..),
		4 if past => bytes.slice(a..=usize::MAX),
		4 => bytes.slice(a..=b),
		5 if past => bytes.slice(..=usize::MAX),
		5 => bytes.slice(..=b),
		_ => {
			// Drained: `start` moves past `end`, or at `usize::MAX` the range is exhausted.
			let mut range = if past { usize::MAX..=usize::MAX } else { a..=b };
			while range.next().is_some() {}
			bytes.slice(range)
		}
	};
	out.push(part.len() as u8);
	out.push(part.is_empty() as u8);
	out.extend_from_slice(&part);
	out.push((part == bytes) as u8);
	out.push((part.clone() == part[..]) as u8);
	out.push((bytes.slice(a.min(rest.len())..).slice(..0) == Bytes::new()) as u8);
	let vec: Vec<u8> = part.into();
	out.extend_from_slice(&vec);
}

/// An owner `Bytes::from_owner` reads once, and drops nothing JS sees.
struct Owner {
	data: Vec<u8>,
	skip: usize,
}

impl AsRef<[u8]> for Owner {
	fn as_ref(&self) -> &[u8] {
		&self.data[self.skip.min(self.data.len())..]
	}
}

/// `Bytes` made each way, printed with `Debug`.
fn made(data: &[u8], out: &mut Vec<u8>) {
	let Some((&skip, rest)) = data.split_first() else {
		return;
	};
	let made = [
		Bytes::from(rest.to_vec()),
		Bytes::from_static(b"\0\t\r\n\\\"'~\x7f\x80\xff"),
		Bytes::from(String::from("string")),
		Bytes::from("text"),
		Bytes::from(&b"static"[..]),
		Bytes::default(),
		Bytes::from_owner(Owner {
			data: rest.to_vec(),
			skip: skip as usize,
		}),
	];
	let mut source = rest.to_vec();
	let copied = Bytes::copy_from_slice(&source);
	// Written in place, where a `Bytes` that shared it would see the write.
	source.clear();
	source.push(0);
	out.extend_from_slice(format!("{copied:?}").as_bytes());
	for bytes in made {
		out.extend_from_slice(format!("{bytes:?} {}\n", bytes.len()).as_bytes());
	}
}

/// `tracing`'s events, which evaluate nothing without a subscriber: no field marks `out`,
/// one in a branch included.
fn events(data: &[u8], out: &mut Vec<u8>) {
	if data.len() > 1 {
		tracing::debug!("branch");
	}
	tracing::trace!(n = mark(out, 1), "trace");
	tracing::debug!(?data, n = mark(out, 2));
	tracing::info!(n = %mark(out, 3), "info {}", data.len());
	tracing::warn!(target: "stock", "warn {}", mark(out, 4));
	tracing::error!(n = mark(out, 5));
	tracing::event!(tracing::Level::INFO, n = mark(out, 6), "event");
	out.push(data.len() as u8);
}

/// Push `n`, and return it.
fn mark(out: &mut Vec<u8>, n: u8) -> u8 {
	out.push(n);
	n
}

/// `rand::rng()`'s values: each in its range, an empty range panicking as `rand`'s does,
/// and every value of a small range drawn.
fn random(data: &[u8], out: &mut Vec<u8>) {
	let [kind, a, b, ..] = *data else {
		return;
	};
	let mut rng = rand::rng();
	let in_range = match kind % 10 {
		0 => {
			let x = rng.random_range(a..b);
			a <= x && x < b
		}
		1 => {
			let x = rng.random_range(a..=b);
			a <= x && x <= b
		}
		2 => {
			let (lo, hi) = (i32::from(a as i8) * 1000, i32::from(b as i8) * 1000);
			let x = rng.random_range(lo..hi);
			lo <= x && x < hi
		}
		3 => {
			let lo = u64::MAX - u64::from(a) * (1 << 40);
			let x = rng.random_range(lo..=u64::MAX);
			lo <= x
		}
		4 => {
			let (lo, hi) = (i128::from(a as i8) << 100, i128::from(b as i8) << 100);
			let x = rng.random_range(lo..hi);
			lo <= x && x < hi
		}
		5 => {
			// Each of the four values, past 2^53, within 256 draws.
			let lo = (1u64 << 60) + u64::from(a);
			let mut seen = [false; 4];
			for _ in 0..256 {
				seen[(rng.random_range(lo..lo + 4) - lo) as usize] = true;
			}
			seen.iter().all(|&s| s)
		}
		6 => {
			let (lo, hi) = (i64::MIN + i64::from(a), i64::MAX - i64::from(b));
			let x = rng.random_range(lo..=hi);
			lo <= x && x <= hi
		}
		7 => rng.random_range(..b) < b,
		8 => {
			// Up to each type's maximum, as `b` of 255 is.
			let hi = u64::MAX - u64::from(a);
			rng.random_range(..=b) <= b && rng.random_range(..=hi) <= hi
		}
		_ => {
			let (x, y): (u64, i16) = (rng.random(), rng.random());
			let flag: bool = rand::rng().random();
			x == x && y == y && (flag || !flag)
		}
	};
	out.push(in_range as u8);
}
