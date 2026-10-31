//! Iterators: std's adaptors and consumers, ranges, iterators of `Option`, and the
//! crate's own `impl Iterator`. Closures with effects log each call, so laziness and
//! where a consumer stops are part of the output.

use std::cmp::{Ordering, Reverse};
use std::fmt;

/// Adaptor chains over the input bytes, each consumed one way.
pub fn adaptors(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	match kind % 8 {
		0 => {
			let doubled: Vec<u8> = data.iter().map(|&b| b / 2).filter(|b| b % 3 != 0).collect();
			out.extend_from_slice(&doubled);
			let small: Vec<u16> = data
				.iter()
				.filter_map(|&b| (b < 100).then_some(b as u16 * 200))
				.collect();
			for v in small {
				out.extend_from_slice(&v.to_be_bytes());
			}
		}
		1 => {
			for (i, &b) in data.iter().enumerate().skip(1).step_by(2) {
				out.push(i as u8);
				out.push(b);
			}
			for (i, b) in data.iter().rev().enumerate().take(3) {
				out.push(i as u8 ^ b);
			}
		}
		2 => {
			let (front, back) = data.split_at(data.len() / 2);
			let zipped: Vec<u8> = front.iter().zip(back.iter().rev()).map(|(a, b)| a ^ b).collect();
			out.extend_from_slice(&zipped);
			let chained: Vec<u8> = front.iter().chain(back).copied().rev().collect();
			out.extend_from_slice(&chained);
		}
		3 => {
			let taken: Vec<u8> = data.iter().copied().take_while(|&b| b != 0).collect();
			let skipped: Vec<u8> = data.iter().copied().skip_while(|&b| b < 0x80).collect();
			let mapped: Vec<u8> = data.iter().map_while(|&b| b.checked_add(0x40)).collect();
			out.push(taken.len() as u8);
			out.push(skipped.len() as u8);
			out.extend_from_slice(&mapped);
		}
		4 => {
			let words: Vec<Vec<u8>> = data.chunks_by_zero();
			let flat: Vec<u8> = words.iter().flat_map(|w| w.iter().rev()).copied().collect();
			out.extend_from_slice(&flat);
			let nested = words.iter().map(|w| w.iter().map(|&b| b.wrapping_add(3))).flatten();
			out.extend(nested.rev());
		}
		5 => {
			let mut it = data.iter().peekable();
			while let Some(&b) = it.next() {
				let mut run = 1u8;
				while it.next_if(|&&next| next == b).is_some() {
					run += 1;
				}
				out.push(b);
				out.push(run);
				if let Some(&&next) = it.peek() {
					out.push(next);
				}
			}
		}
		6 => {
			let mut it = data.iter();
			let firsts: Vec<u8> = it.by_ref().take(2).copied().collect();
			out.extend_from_slice(&firsts);
			out.push(it.len() as u8);
			match it.next_back() {
				Some(&last) => out.push(last),
				None => out.push(0xee),
			}
			let rest: Vec<u8> = it.rev().step_by(3).copied().collect();
			out.extend_from_slice(&rest);
		}
		_ => {
			// Each element of `pairs` is a copy: changing one changes nothing else.
			let pairs: Vec<[u8; 2]> = data.iter().map(|&b| [b, !b]).collect();
			let mut copies: Vec<[u8; 2]> = pairs.iter().copied().collect();
			if !copies.is_empty() {
				copies[0][0] = 0x55;
			}
			for pair in pairs.iter().chain(copies.iter()) {
				out.extend_from_slice(pair);
			}
			let mut v: Vec<u16> = Vec::new();
			v.extend(data.iter().map(|&b| b as u16 + 1));
			out.push(v.len() as u8);
			out.extend(data.iter().filter(|&&b| b > 0x80));
		}
	}
}

trait ChunksByZero {
	fn chunks_by_zero(&self) -> Vec<Vec<u8>>;
}

impl ChunksByZero for [u8] {
	/// The runs between zero bytes.
	fn chunks_by_zero(&self) -> Vec<Vec<u8>> {
		let mut words = Vec::new();
		let mut word = Vec::new();
		for &b in self {
			if b == 0 {
				words.push(std::mem::take(&mut word));
			} else {
				word.push(b);
			}
		}
		words.push(word);
		words
	}
}

/// Consumers, and how far each reads: `log` records every item a closure sees.
pub fn consumers(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let mut log = Vec::new();
	match kind % 6 {
		0 => {
			let any = data.iter().inspect(|&&b| log.push(b)).any(|&b| b > 0xf0);
			out.push(any as u8);
			let all = data.iter().all(|&b| b != 0);
			out.push(all as u8);
			out.push(data.iter().count() as u8);
			out.push(data.iter().filter(|&&b| b % 2 == 0).count() as u8);
		}
		1 => {
			let found = data.iter().inspect(|&&b| log.push(b)).find(|&&b| b % 7 == 3);
			out.push(found.copied().unwrap_or(0xff));
			let mapped = data
				.iter()
				.find_map(|&b| if b > 0x50 { b.checked_mul(3) } else { None });
			out.push(mapped.unwrap_or(0));
			let position = data.iter().position(|&b| b == 0x40);
			out.push(position.map_or(0xff, |p| p as u8));
			// A zip of slices reads the longer side's closure only up to the shorter's length.
			let short = &data[data.len() / 2..];
			let logged = |b: &u8| *b;
			let pairs = data
				.iter()
				.map(|&b| {
					log.push(b);
					b
				})
				.zip(short)
				.count();
			out.push(pairs as u8);
			let walked = data.iter().map(|&b| {
				log.push(!b);
				b
			});
			for (a, b) in walked.zip(short.iter()) {
				out.push(a ^ b);
			}
			for (a, &b) in data.iter().inspect(|&&b| log.push(b)).map(logged).zip(short.iter()) {
				out.push(a ^ b);
			}
			let mut back = data.iter().map(|&b| b.rotate_free()).zip(short.iter().copied());
			if let Some((a, b)) = back.next_back() {
				out.push(a);
				out.push(b);
			}
			out.push(back.len() as u8);
		}
		2 => {
			out.push(data.iter().copied().max().unwrap_or(0));
			out.push(data.iter().min().copied().unwrap_or(0xff));
			out.push(*data.iter().max_by_key(|&&b| b % 16).unwrap_or(&0xee));
			out.push(*data.iter().min_by_key(|&&b| b / 16).unwrap_or(&0xee));
			out.push(data.iter().last().copied().unwrap_or(0xdd));
			out.push(data.iter().nth(3).copied().unwrap_or(0xcc));
			// `take`'s `nth` and `nth_back` read on past its limit as std's do.
			let mut it = data.iter();
			out.push(it.by_ref().take(data.len() % 3).nth_back(1).copied().unwrap_or(0xbb));
			out.push(it.len() as u8);
			out.push(it.by_ref().take(data.len() % 5).nth(2).copied().unwrap_or(0xbb));
			out.push(it.len() as u8);
		}
		3 => {
			let wide: u64 = data.iter().map(|&b| b as u64 * 0x0101_0101_0101).sum();
			out.extend_from_slice(&wide.to_be_bytes());
			let folded = data
				.iter()
				.fold(0u32, |acc, &b| (acc << 3).wrapping_add(b as u32 ^ acc));
			out.extend_from_slice(&folded.to_be_bytes());
			data.iter().for_each(|&b| log.push(b.rotate_free()));
		}
		4 => {
			// Panics on overflow, as a debug build's `sum` does.
			let total: u8 = data.iter().sum();
			out.push(total);
			let product: u16 = data.iter().map(|&b| b as u16 % 4 + 1).product();
			out.extend_from_slice(&product.to_be_bytes());
		}
		_ => {
			let parsed: Result<Vec<u8>, u8> = data.iter().map(|&b| if b < 0xf0 { Ok(b) } else { Err(b) }).collect();
			match parsed {
				Ok(v) => out.extend_from_slice(&v),
				Err(e) => out.push(e),
			}
			let some: Option<Vec<u16>> = data
				.iter()
				.inspect(|&&b| log.push(b))
				.map(|&b| (b != 0x20).then_some(b as u16))
				.collect();
			out.push(some.map_or(0xff, |v| v.len() as u8));
		}
	}
	out.extend_from_slice(&log);
}

/// `reduce`, `max_by`, and `min_by`, whose comparators log each pair they see, and
/// `Peekable`'s `peek_mut` and `next_if_eq`.
pub fn reducers(data: &[u8], out: &mut Vec<u8>) {
	let mut log = Vec::new();
	// Of keys that tie, `max_by` keeps the last and `min_by` the first.
	let most = data.iter().max_by(|&&a, &&b| {
		log.extend([a, b]);
		(a % 4).cmp(&(b % 4))
	});
	out.push(most.copied().unwrap_or(0xee));
	let least = data.iter().copied().min_by(|a, b| {
		log.extend([*a, *b]);
		(a / 64).cmp(&(b / 64))
	});
	out.push(least.unwrap_or(0xee));
	let folded = data.iter().copied().reduce(|acc, b| {
		log.push(acc);
		acc.wrapping_mul(3) ^ b
	});
	out.push(folded.unwrap_or(0xdd));
	let longest = data
		.iter()
		.map(|&b| vec![b; usize::from(b % 3)])
		.max_by(|a, b| a.len().cmp(&b.len()));
	out.push(longest.map_or(0xcc, |v| v.len() as u8));
	let joined = data.iter().map(|&b| vec![b]).reduce(|mut a, b| {
		a.extend(b);
		a
	});
	out.extend(joined.unwrap_or_default());
	let mut it = data.iter().copied().peekable();
	if let Some(b) = it.peek_mut() {
		*b = b.wrapping_add(1);
	}
	// Written again through a second `peek_mut` of the item already peeked.
	if let Some(b) = it.peek_mut() {
		*b ^= 0x80;
	}
	out.push(it.next().unwrap_or(0xbb));
	// The second byte matches the first only in a repeated input; a mismatch keeps it.
	let first = data.first().copied().unwrap_or(0);
	out.push(it.next_if_eq(&first).unwrap_or(0xaa));
	out.push(it.next().unwrap_or(0xaa));
	out.push(it.peek_mut().is_none() as u8);
	let mut words = data.iter().map(|&b| vec![b]).peekable();
	if let Some(w) = words.peek_mut() {
		w.push(0x99);
	}
	out.extend(words.next().unwrap_or_default());
	out.push(words.next_if_eq(&vec![first]).map_or(0x88, |w| w[0]));
	out.push(words.count() as u8);
	// Items that are places, written through the one `min_by` keeps: the first of the greatest.
	let mut bytes = data.to_vec();
	if let Some(b) = bytes.iter_mut().min_by(|a, b| b.cmp(a)) {
		*b = 0x77;
	}
	out.extend(bytes);
	out.extend(log);
}

/// A byte that equals a `u8` of the same remainder by 4, counting each comparison.
struct Quarter<'a>(u8, &'a std::cell::Cell<u8>);

impl PartialEq<u8> for Quarter<'_> {
	fn eq(&self, other: &u8) -> bool {
		self.1.set(self.1.get().wrapping_add(1));
		self.0 % 4 == other % 4
	}
}

impl PartialEq<Quarter<'_>> for u8 {
	fn eq(&self, other: &Quarter<'_>) -> bool {
		other.1.set(other.1.get().wrapping_add(0x10));
		self % 4 == other.0 % 4
	}
}

/// By the pair's sum, which reads the array through the reference `eq` takes.
impl PartialEq<Quarter<'_>> for [u8; 2] {
	fn eq(&self, other: &Quarter<'_>) -> bool {
		other.1.set(other.1.get().wrapping_add(0x40));
		self.iter().sum::<u8>() == other.0
	}
}

/// By the string's length, a std type compared by the crate's impl rather than std's.
impl PartialEq<Quarter<'_>> for String {
	fn eq(&self, other: &Quarter<'_>) -> bool {
		other.1.set(other.1.get().wrapping_add(4));
		self.len() == usize::from(other.0)
	}
}

/// `Peekable::next_if_eq` against another type than the item's, by the item's
/// `PartialEq<T>`: std's across strings and slices, and the crate's own. Each matches the
/// first item, and a mismatch keeps the item for the next `next`.
pub fn peeked_others(data: &[u8], out: &mut Vec<u8>) {
	use std::borrow::Cow;
	let letter = |b: u8| char::from(b'a' + b % 3).to_string();
	let head = |w: Option<&str>| w.map_or(0xaa, |w| w.as_bytes()[0]);
	let words: Vec<String> = data.iter().map(|&b| letter(b)).collect();
	let first = words.first().cloned().unwrap_or_default();
	let mut it = words.clone().into_iter().peekable();
	out.push(head(it.next_if_eq("a").as_deref()));
	out.push(head(it.next_if_eq(&"b").as_deref()));
	out.push(head(it.next().as_deref()));
	// By `&String: PartialEq<&str>`, which compares what each points at.
	let mut it = words.iter().peekable();
	out.push(head(it.next_if_eq(&first.as_str()).map(String::as_str)));
	out.push(head(it.next_if_eq(&"c").map(String::as_str)));
	out.push(head(it.next().map(String::as_str)));
	let mut it = words.iter().map(String::as_str).peekable();
	out.push(head(it.next_if_eq(&first)));
	out.push(head(it.next()));
	let mut it = data.iter().map(|&b| vec![letter(b)]).peekable();
	out.push(head(
		it.next_if_eq(&vec![first.as_str()]).as_ref().map(|v| v[0].as_str()),
	));
	out.push(head(it.next().as_ref().map(|v| v[0].as_str())));
	let mut it = words.iter().map(|w| Cow::Borrowed(w.as_str())).peekable();
	out.push(head(it.next_if_eq(&"b").as_deref()));
	out.push(head(it.next().as_deref()));
	let mut it = words.clone().into_iter().peekable();
	out.push(head(it.next_if_eq(&Cow::Owned(first.clone())).as_deref()));

	let len = |v: Option<Vec<u16>>| v.map_or(0xaa, |v| v.len() as u8);
	let runs = |b: u8| vec![u16::from(b % 2) << 9; usize::from(b % 3)];
	let mut it = data.iter().map(|&b| runs(b)).peekable();
	out.push(len(it.next_if_eq(&[0u16][..])));
	out.push(len(it.next_if_eq(&[0x200, 0x200])));
	out.push(len(it.next_if_eq(&&[][..])));
	out.push(len(it.next()));
	let mut it = data.iter().map(|&b| vec![b % 2; usize::from(b % 3)]).peekable();
	out.push(it.next_if_eq(&[1][..]).map_or(0xaa, |v| v.len() as u8));
	out.push(it.next().map_or(0xaa, |v| v.len() as u8));
	let calls = std::cell::Cell::new(0);
	let mut it = data.chunks_exact(2).map(|c| [c[0] % 2, c[1] % 2]).peekable();
	out.push(it.next_if_eq(&[0, 1][..]).map_or(0xaa, |p| p[1]));
	out.push(it.next().map_or(0xaa, |p| p[0]));
	out.push(it.next_if_eq(&Quarter(1, &calls)).map_or(0xaa, |p| p[0]));
	out.push(it.next().map_or(0xaa, |p| p[1]));
	// `&mut u8` items, each a place, against a `&u8`, written through once taken.
	let mut bytes = data.to_vec();
	let mut it = bytes.iter_mut().peekable();
	if let Some(b) = it.next_if_eq(&data.first().unwrap_or(&0)) {
		*b ^= 0x40;
	}
	out.push(it.next_if_eq(&&3).map_or(0xaa, |b| *b));
	out.extend(bytes);

	let mut it = data.iter().map(|&b| Quarter(b, &calls)).peekable();
	let want = data.first().copied().unwrap_or(0);
	while let Some(q) = it.next_if_eq(&want) {
		out.push(q.0);
	}
	out.push(it.next().map_or(0xaa, |q| q.0));
	let mut it = data.iter().copied().peekable();
	out.push(it.next_if_eq(&Quarter(want ^ 1, &calls)).unwrap_or(0xaa));
	out.push(it.next_if_eq(&Quarter(want, &calls)).unwrap_or(0xaa));
	let mut it = data.iter().map(|&b| "ab"[..usize::from(b % 3)].to_string()).peekable();
	out.push(it.next_if_eq(&Quarter(1, &calls)).map_or(0xaa, |w| w.len() as u8));
	out.push(it.next().map_or(0xaa, |w| w.len() as u8));
	out.push((String::from("ab") == Quarter(want, &calls)) as u8);
	out.push(calls.get());
}

trait RotateFree {
	fn rotate_free(self) -> Self;
}

impl RotateFree for u8 {
	fn rotate_free(self) -> u8 {
		(self << 1) | (self >> 7)
	}
}

/// `nth` and `nth_back` through each adaptor, of a source whose reads `by_ref` or
/// `inspect` shows: each reads on past a `take`'s limit, or stops at it, as std's does.
pub fn skips(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let n = data.first().map_or(0, |&b| b as usize % 4);
	let limit = data.get(1).map_or(0, |&b| b as usize % 3);
	let back = kind & 1 == 1;
	// `nth` or `nth_back` of `$it`, as a byte by `$f`.
	macro_rules! skip {
		($it:expr, $f:expr) => {{
			let mut it = $it;
			let item = if back { it.nth_back(n) } else { it.nth(n) };
			out.push(item.map_or(0xee, $f));
		}};
	}
	// `skip!` of `$adapt` over a `by_ref` source, then over an `inspect`ed one, and what each read.
	macro_rules! sources {
		(|$src:ident| $adapt:expr, $f:expr) => {{
			let mut rest = data.iter();
			{
				let $src = rest.by_ref();
				skip!($adapt, $f);
			}
			out.push(rest.len() as u8);
			let mut log = Vec::new();
			{
				let $src = data.iter().inspect(|&&b| log.push(b));
				skip!($adapt, $f);
			}
			out.extend_from_slice(&log);
		}};
	}
	match (kind / 2) % 14 {
		0 => sources!(|src| src.take(limit).fuse(), |&b| b),
		1 => sources!(|src| src.take(limit + 1).skip(1), |&b| b),
		2 => sources!(|src| src.take(limit).chain([0x11].iter()), |&b| b),
		3 => sources!(|src| [0x11].iter().chain(src.take(limit)), |&b| b),
		4 => sources!(|src| src.take(limit + 1).step_by(2), |&b| b),
		5 => sources!(|src| src.take(limit).peekable(), |&b| b),
		6 => sources!(
			|src| {
				let mut it = src.take(limit + 1).peekable();
				it.peek();
				it
			},
			|&b| b
		),
		7 => sources!(|src| std::iter::once(src.take(limit)).flatten(), |&b| b),
		8 => sources!(|src| src.take(limit).copied(), |b| b),
		9 => sources!(|src| src.take(limit).cloned(), |b| b),
		10 => sources!(|src| src.take(limit).enumerate(), |(i, &b)| b ^ i as u8),
		11 => sources!(|src| src.take(limit).rev(), |&b| b),
		// `fuse` advances by `next`, where `take` would read on past its limit.
		12 => sources!(|src| [0x11].iter().chain(src.take(limit).fuse()), |&b| b),
		_ => {
			// Copies that JS makes by a function, which std's `copied` skips through.
			let pairs: Vec<(u8, u8)> = data.iter().map(|&b| (b, !b)).collect();
			let mut rest = pairs.iter();
			skip!(rest.by_ref().take(limit).copied(), |(a, b)| a ^ b);
			out.push(rest.len() as u8);
			let arrays: Vec<[u8; 2]> = data.iter().map(|&b| [b, !b]).collect();
			let mut rest = arrays.iter();
			skip!(rest.by_ref().take(limit).copied(), |[a, _]| a);
			out.push(rest.len() as u8);
		}
	}
}

/// An order whose `cmp` leans on its first operand, an odd one less than any, so which
/// operand std's `min_by_key`, `max_by_key`, `min`, and `max` put first shows.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Tilt(u8);

impl PartialOrd for Tilt {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(other))
	}
}

impl Ord for Tilt {
	fn cmp(&self, other: &Self) -> Ordering {
		match self.0 % 2 {
			1 => Ordering::Less,
			_ => self.0.cmp(&other.0),
		}
	}
}

/// `min_by_key` and `max_by_key` by keys that are tuples, `Reverse`s, `Option`s, and the
/// crate's own orders, `min` and `max` of such items, and `rposition` and `rfind`.
pub fn ranked(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	// Where the item found was, by its index.
	let put = |out: &mut Vec<u8>, found: Option<(usize, u8)>| out.push(found.map_or(0xff, |(i, _)| i as u8));
	let tier = |b: u8| match b % 3 {
		0 => None,
		1 => Some(Reverse(b / 3 % 4)),
		_ => Some(Reverse(0)),
	};
	let items = || data.iter().copied().enumerate();
	let tilts: Vec<Tilt> = data.iter().map(|&b| Tilt(b % 8)).collect();
	match kind % 4 {
		0 => {
			// Of equal keys, `min_by_key` keeps the first and `max_by_key` the last.
			put(out, items().min_by_key(|&(_, b)| (Reverse(b % 4), tier(b))));
			put(out, items().max_by_key(|&(_, b)| (Reverse(b % 4), tier(b))));
			put(out, items().filter(|&(_, b)| b > 0x40).max_by_key(|&(_, b)| tier(b)));
			put(out, items().min_by_key(|&(_, b)| (b % 2 == 0, Reverse((b % 3) as u64))));
		}
		1 => {
			// By the crate's own order, and by keys that borrow from the items.
			put(out, items().min_by_key(|&(_, b)| Tilt(b % 8)));
			put(out, items().max_by_key(|&(_, b)| (b % 2 == 0, Tilt(b % 8))));
			let first = |t: Option<&Tilt>| t.map_or(0xff, |t| t.0);
			out.push(first(tilts.iter().min_by_key(|&t| Reverse(Some(t)))));
			out.push(first(tilts.iter().max_by_key(|&t| (Reverse(t), t.0 / 2))));
		}
		2 => {
			// Items that are tuples, and the crate's own.
			let pair = |b: u8| (b % 4, Reverse(Some(b / 4).filter(|b| b % 3 != 0)));
			for best in [data.iter().map(|&b| pair(b)).max(), data.iter().map(|&b| pair(b)).min()] {
				match best {
					Some((a, Reverse(b))) => out.extend_from_slice(&[a, b.unwrap_or(0xff)]),
					None => out.push(0xfe),
				}
			}
			out.push(tilts.iter().copied().max().map_or(0xff, |t| t.0));
			out.push(tilts.iter().min().map_or(0xff, |t| t.0));
		}
		_ => {
			// From the back, counting from the front, of what is left.
			out.push(data.iter().rposition(|&b| b % 3 == 0).map_or(0xff, |i| i as u8));
			out.push(data.iter().rfind(|&&b| b > 0x80).copied().unwrap_or(0));
			let mut seen = Vec::new();
			let mut rest = tilts.iter();
			rest.next();
			let found = rest.rposition(|t| {
				seen.push(t.0);
				t.0 == 2
			});
			out.push(found.map_or(0xff, |i| i as u8));
			out.extend_from_slice(&seen);
			out.push(rest.len() as u8);
			out.push(rest.rfind(|t| t.0 > 3).map_or(0xff, |t| t.0));
			out.push(rest.len() as u8);
		}
	}
}

/// Ranges as values and as iterators.
pub fn ranges(data: &[u8], out: &mut Vec<u8>) {
	let a = data.first().copied().unwrap_or(0);
	let b = data.get(1).copied().unwrap_or(0);
	let range = a..b;
	out.push(range.start);
	out.push(range.end);
	out.push(range.is_empty() as u8);
	out.push(range.contains(&0x40) as u8);
	let mut copy = range.clone();
	out.push(copy.next().unwrap_or(0xff));
	out.push(range.len() as u8);
	for i in range.rev().step_by(3) {
		out.push(i);
	}
	for i in (a..b).skip(1).step_by(4) {
		out.push(i);
	}
	out.push((a..b).nth(5).unwrap_or(0xee));
	// `..=` at the top of `u8`, which ends without overflowing.
	let top: Vec<u8> = (a.max(250)..=255).collect();
	out.extend_from_slice(&top);
	let inclusive = a..=b;
	out.push(*inclusive.start());
	out.push(*inclusive.end());
	out.push(inclusive.contains(&b) as u8);
	// Of constants, which is itself a constant.
	const LOW: u64 = 0x20;
	const HIGH: u64 = 0x7f;
	out.push((LOW..=HIGH).contains(&u64::from(a)) as u8);
	// `nth` past the end, or past the type's last value, leaves the range as std's does.
	let mut near = a.max(200)..=255;
	out.push(near.nth(b as usize).unwrap_or(0xee));
	out.push(*near.start());
	out.push(near.is_empty() as u8);
	out.push(near.contains(&255) as u8);
	out.push(near.contains(&100) as u8);
	let mut low = 0..=b;
	out.push(low.nth_back(a as usize % 16).unwrap_or(0xee));
	out.push(*low.end());
	out.push(low.contains(&(b / 2)) as u8);
	let mut reversed = a..=b;
	out.push(reversed.by_ref().rev().nth(a as usize).unwrap_or(0xee));
	out.push(*reversed.end());
	out.push(reversed.contains(&a) as u8);
	let mut counted = (a..=b).enumerate();
	let (i, n) = counted.nth(b as usize / 4).unwrap_or((0xdd, 0xdd));
	out.push(i as u8);
	out.push(n);
	out.push(counted.next().map_or(0xcc, |(i, _)| i as u8));
	let mut stepped = a..=b;
	out.push(stepped.by_ref().step_by(3).count() as u8);
	out.push(*stepped.start());
	let signed = (a as i8)..=(b as i8);
	out.extend(signed.rev().map(|i| i as u8));
	let mut wide = (a as u64)..(b as u64 * 3);
	out.push(wide.by_ref().map(|n| n % 5).filter(|&n| n == 0).count() as u8);
	out.push(wide.is_empty() as u8);
	let total: u64 = (0..a as u64).sum();
	out.extend_from_slice(&total.to_be_bytes());
	// Wide ranges stepping past 2^53, where a number becomes limbs, and up to the maximum.
	let edge = (1u64 << 53) - 2 + a as u64 % 4;
	let mut past = edge..edge + 4;
	out.extend_from_slice(&past.nth(b as usize % 3).unwrap_or(0).to_be_bytes());
	out.extend_from_slice(&past.next_back().unwrap_or(0).to_be_bytes());
	out.extend_from_slice(&(edge..=edge + 3).nth(b as usize % 5).unwrap_or(0).to_be_bytes());
	let top: u64 = (u64::MAX - a as u64 % 3..=u64::MAX).map(|n| n % 7).sum();
	out.extend_from_slice(&top.to_be_bytes());
	let most = (edge..=edge + 2).rev().max_by_key(|&n| n).unwrap_or(0);
	out.extend_from_slice(&most.to_be_bytes());
	let least = (-(1i64 << 53) - 1..-(1i64 << 53) + 1).min().unwrap_or(0);
	out.extend_from_slice(&least.to_be_bytes());
	let steps: Vec<u32> = (0..48).step_by(8).rev().collect();
	for s in steps {
		out.push(s as u8);
	}
	updates(a, b, data, out);
}

/// Struct update syntax on std's ranges: the fields written run before the base, and
/// the rest are read from it.
fn updates(a: u8, b: u8, data: &[u8], out: &mut Vec<u8>) {
	use std::ops::Range;
	let mut base = a..b;
	let made = Range {
		end: {
			out.push(0xd0);
			b / 2
		},
		..{
			out.push(0xd1);
			base.clone()
		}
	};
	let whole = Range { ..base.clone() };
	out.extend_from_slice(&[made.start, made.end, whole.start, whole.end]);
	// The loop's end is read once, though the base it came from changes.
	for i in (Range { start: a / 2, ..base }) {
		base.end = base.end.wrapping_add(1);
		out.push(i);
	}
	out.push(base.end);
	let len = data.len();
	let (to, from) = ((b as usize).min(len), (a as usize).min(len));
	let all = 0..len;
	// The sliced value, the fields written, then the base.
	let sliced = &{
		out.push(0xd2);
		data
	}[Range {
		start: {
			out.push(0xd3);
			from
		},
		..{
			out.push(0xd4);
			all.clone()
		}
	}];
	out.extend_from_slice(sliced);
	let full = &data[std::ops::RangeFull {
		..{
			out.push(0xd5);
			std::ops::RangeFull
		}
	}];
	out.push(full.len() as u8);
	let full = &data[{
		out.push(0xd6);
		..
	}];
	out.push(full.len() as u8);
	let full = &data[full_range(out)];
	out.push(full.len() as u8);
	let full = &data[..];
	out.push(full.len() as u8);
	let mut copied = data.to_vec();
	copied.copy_within(Range { end: to, ..all }, 0);
	out.extend_from_slice(&copied);
}

/// `..`, logging that it was made.
#[inline(never)]
fn full_range(out: &mut Vec<u8>) -> std::ops::RangeFull {
	out.push(0xd7);
	..
}

/// Iterators of `Option`, which keep each `None` apart from their end.
pub fn options(data: &[u8], out: &mut Vec<u8>) {
	let maybe = |b: &u8| if b % 3 == 0 { None } else { Some(*b) };
	let all: Vec<Option<u8>> = data.iter().map(maybe).collect();
	out.push(all.len() as u8);
	for item in data.iter().map(maybe) {
		match item {
			Some(b) => out.push(b),
			None => out.push(0),
		}
	}
	let flat: Vec<u8> = data.iter().map(maybe).flatten().collect();
	out.extend_from_slice(&flat);
	let somes = all.iter().filter(|o| o.is_some()).count();
	out.push(somes as u8);
	out.push(second(data.iter().map(maybe)).unwrap_or(0xcd));
	let first = data.first().copied();
	for b in first.iter().chain(std::iter::once(&0x99)) {
		out.push(*b);
	}
	out.extend(first.into_iter().chain(std::iter::empty()));
	let mut n = 0u8;
	let counted: Vec<u8> = std::iter::from_fn(|| {
		n += 1;
		(n < 4).then_some(n)
	})
	.collect();
	out.extend_from_slice(&counted);
	// Counted through a borrow, which leaves a `Chain` to read on past a gap.
	let mut k = 0u8;
	let mut gappy = std::iter::empty().chain(std::iter::from_fn(|| {
		k += 1;
		(k != 2).then_some(k)
	}));
	out.push(gappy.by_ref().count() as u8);
	out.push(gappy.next().unwrap_or(0xee));
	// `successors` calls its closure on each item as it yields it, and never again once
	// it returned `None`.
	let mut calls = Vec::new();
	let halving = |b: &u8, calls: &mut Vec<u8>| {
		calls.push(*b);
		b.checked_sub(1).map(|b| b / 2)
	};
	let first = data.first().copied();
	let taken: Vec<u8> = std::iter::successors(first, |b| halving(b, &mut calls))
		.take(data.len() % 4)
		.collect();
	out.extend_from_slice(&taken);
	out.extend_from_slice(&calls);
	let count = std::iter::successors(first, |b| halving(b, &mut calls)).count();
	out.push(count as u8);
	out.extend_from_slice(&calls);
	// The crate's own iterator, whose items are options.
	let gaps = Gaps { data, at: 0 };
	for gap in gaps {
		out.push(gap.map_or(0xaa, |b| b));
	}
}

/// The second item of an iterator of options, through `?`, which tests for its end.
fn second(mut items: impl Iterator<Item = Option<u8>>) -> Option<u8> {
	items.next()?;
	let item = items.next()?;
	Some(item.unwrap_or(0xab))
}

/// Every byte, or `None` where it is zero.
struct Gaps<'a> {
	data: &'a [u8],
	at: usize,
}

impl Iterator for Gaps<'_> {
	type Item = Option<u8>;

	fn next(&mut self) -> Option<Option<u8>> {
		let &b = self.data.get(self.at)?;
		self.at += 1;
		if b == 0 {
			return Some(None);
		}
		Some(Some(b))
	}
}

/// A countdown: the crate's own iterator.
struct Countdown {
	left: u8,
	step: u8,
}

impl Iterator for Countdown {
	type Item = u8;

	fn next(&mut self) -> Option<u8> {
		match self.left {
			0 => None,
			left => {
				self.left = left.saturating_sub(self.step);
				Some(left)
			}
		}
	}
}

/// The even bytes, read through another iterator, whose `None` ends this one.
struct Evens<'a> {
	inner: std::iter::Copied<std::slice::Iter<'a, u8>>,
}

impl Iterator for Evens<'_> {
	type Item = u8;

	fn next(&mut self) -> Option<u8> {
		loop {
			let b = self.inner.next()?;
			if b % 2 == 0 {
				return Some(b);
			}
		}
	}
}

/// The crate's own iterators, through `for`, adaptors, and `next`.
pub fn own(data: &[u8], out: &mut Vec<u8>) {
	let left = data.first().copied().unwrap_or(0);
	let step = data.get(1).copied().unwrap_or(1).max(1);
	for n in (Countdown { left, step }) {
		out.push(n);
	}
	let total: u64 = Countdown { left, step }.map(|n| n as u64 * 1000 / 7).sum();
	out.extend_from_slice(&total.to_be_bytes());
	let mut down = Countdown { left, step };
	while let Some(n) = down.next() {
		if n % 5 == 0 {
			break;
		}
	}
	out.push(down.left);
	let evens: Vec<u8> = Evens {
		inner: data.iter().copied(),
	}
	.collect();
	out.extend_from_slice(&evens);
	let mut by_ref = Evens {
		inner: data[data.len().min(1)..].iter().copied(),
	};
	let firsts: Vec<u8> = by_ref.by_ref().take(2).collect();
	out.extend_from_slice(&firsts);
	// `for` over a `&mut` to one, which reads on from where it stopped.
	for n in by_ref.by_ref() {
		if n % 4 == 0 {
			break;
		}
		out.push(n);
	}
	out.push(by_ref.count() as u8);
	// Two instances of one generic iterator, each with its own `next` on one class.
	let halves: Vec<u8> = Halving { value: left }.collect();
	out.extend_from_slice(&halves);
	let total: u64 = Halving {
		value: left as u64 * 0x1_0000_0001,
	}
	.sum();
	out.extend_from_slice(&total.to_be_bytes());
}

/// A value halved until it is zero: generic, with an `impl Iterator` per instance.
struct Halving<T> {
	value: T,
}

impl Iterator for Halving<u8> {
	type Item = u8;

	fn next(&mut self) -> Option<u8> {
		let value = self.value;
		self.value /= 2;
		(value != 0).then_some(value)
	}
}

impl Iterator for Halving<u64> {
	type Item = u64;

	fn next(&mut self) -> Option<u64> {
		match self.value {
			0 => None,
			value => {
				self.value = value >> 1;
				Some(value)
			}
		}
	}
}

/// The last two bytes of `data`, as a `slice::Iter` made from the second argument: a
/// `for` must iterate it, not the first.
fn tail<'a>(_head: &'a [u8], data: &'a [u8]) -> std::slice::Iter<'a, u8> {
	data[data.len().saturating_sub(2)..].iter()
}

/// The odd bytes, from a function returning `impl Iterator`.
fn odds(data: &[u8]) -> impl Iterator<Item = u8> + '_ {
	data.iter().copied().filter(|b| b % 2 == 1)
}

/// Iterators as values: returned, stored, and printed.
pub fn values(data: &[u8], out: &mut Vec<u8>) {
	for &b in tail(&[1, 2, 3], data) {
		out.push(b);
	}
	let mut held = Held { it: data.iter() };
	out.push(held.skip_one());
	for b in odds(data) {
		out.push(b);
	}
	let printed = format!("{:?}|{:#?}", Doubled(data), Doubled(&data[..data.len().min(2)]));
	out.extend_from_slice(printed.as_bytes());
	// Clones that change leave what they cloned alone.
	let tags: Vec<Tag> = data
		.iter()
		.map(|&b| Tag {
			bytes: [b, !b].to_vec(),
		})
		.collect();
	let mut cloned: Vec<Tag> = tags.iter().cloned().collect();
	for tag in cloned.iter_mut() {
		tag.bytes.push(0x77);
	}
	for tag in tags.iter().chain(cloned.iter()) {
		out.extend_from_slice(&tag.bytes);
	}
	// A `Copy` type's own `Clone`, which `cloned` calls and `copied` does not.
	let bumps: Vec<Bumped> = data.iter().map(|&b| Bumped(b)).collect();
	out.extend(bumps.iter().cloned().map(|b| b.0));
	out.extend(bumps.iter().copied().map(|b| b.0));
	// A derived `Clone` over a type parameter clones the field, by its own `Clone`.
	let wrapped: Vec<Wrapper<Bumped>> = data.iter().map(|&b| Wrapper(Bumped(b))).collect();
	out.extend(wrapped.iter().cloned().map(|w| w.0.0));
}

#[derive(Clone, Copy)]
struct Wrapper<T>(T);

/// A `Copy` type whose `Clone` does more than copy.
#[derive(Copy)]
pub struct Bumped(pub u8);

impl Clone for Bumped {
	fn clone(&self) -> Self {
		Bumped(self.0.wrapping_add(1))
	}
}

/// A `Clone` that is not `Copy`, which `cloned` calls for each item.
#[derive(Clone)]
struct Tag {
	bytes: Vec<u8>,
}

/// An iterator in a field.
struct Held<'a> {
	it: std::slice::Iter<'a, u8>,
}

impl Held<'_> {
	fn skip_one(&mut self) -> u8 {
		self.it.next();
		self.it.next().copied().unwrap_or(0xbb)
	}
}

/// Each byte doubled, as a `Debug` list built from an iterator.
struct Doubled<'a>(&'a [u8]);

impl fmt::Debug for Doubled<'_> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_list().entries(self.0.iter().map(|&b| b as u16 * 2)).finish()
	}
}

/// What std's `Option`-returning calls give, returned as is: an `Option`, whose `None`
/// stays apart from an item's own.
pub fn returned(data: &[u8], out: &mut Vec<u8>) {
	let maybe = |b: &u8| if b % 3 == 0 { None } else { Some(*b) };
	let push = |b: Option<u8>, out: &mut Vec<u8>| out.push(b.map_or(0xaa, |b| b));
	let nested = |b: Option<Option<u8>>, out: &mut Vec<u8>| match b {
		None => out.push(0xbb),
		Some(None) => out.push(0xcc),
		Some(Some(b)) => out.push(b),
	};
	let n = data.first().map_or(0, |&b| usize::from(b % 4));
	push(first(data.iter().copied()), out);
	push(back(data.iter().copied()), out);
	push(nth(data.iter().copied(), n), out);
	push(last(data.iter().copied()), out);
	nested(first(data.iter().map(maybe)), out);
	nested(back(data.iter().map(maybe)), out);
	nested(first(Gaps { data, at: n }), out);
	push(first(Countdown { left: n as u8, step: 1 }), out);
	let text = std::str::from_utf8(data).unwrap_or("é");
	out.extend(first(text.chars()).map_or(0, u32::from).to_le_bytes());
	let mut copied = data.iter().copied();
	push(by_mut(&mut copied), out);
	let mut gaps = Gaps { data, at: 0 };
	nested(by_mut(&mut gaps), out);
	nested(by_mut(&mut gaps), out);
	let mut it = data.iter();
	push(borrowed(&mut it).copied(), out);
	push(borrowed(&mut it).copied(), out);
	push(second_of(data).copied(), out);
	push(from_one(n as u8), out);
	let mut peekable = data.iter().copied().peekable();
	push(peeked(&mut peekable).copied(), out);
	push(at(data, n).copied(), out);
	let maybes: Vec<Option<u8>> = data.iter().map(maybe).collect();
	nested(at(&maybes, n).copied(), out);
}

fn first<I: Iterator>(mut it: I) -> Option<I::Item> {
	it.next()
}

fn back<I: DoubleEndedIterator>(mut it: I) -> Option<I::Item> {
	it.next_back()
}

fn nth<I: Iterator>(mut it: I, n: usize) -> Option<I::Item> {
	it.nth(n)
}

fn last<I: Iterator>(it: I) -> Option<I::Item> {
	it.last()
}

/// Through a `&mut` to the iterator.
fn by_mut<I: Iterator>(it: &mut I) -> Option<I::Item> {
	it.next()
}

/// Through a borrow of a std iterator the function is not generic over.
fn borrowed<'a>(it: &mut std::slice::Iter<'a, u8>) -> Option<&'a u8> {
	it.next()
}

/// Of a slice's iterator the function makes.
fn second_of(data: &[u8]) -> Option<&u8> {
	let mut it = data.iter();
	it.next();
	it.next()
}

/// Of a small range the function makes.
fn from_one(n: u8) -> Option<u8> {
	let mut range = 1..n;
	range.next()
}

fn peeked<I: Iterator>(it: &mut std::iter::Peekable<I>) -> Option<&I::Item> {
	it.peek()
}

fn at<T>(items: &[T], i: usize) -> Option<&T> {
	items.get(i)
}

/// A zip of each kind std specializes, of sides whose closures log each item they make, and
/// either side shorter or empty: by index (`TrustedRandomAccess`), folded by its size hint
/// (`TrustedLen`, or only `TrustedRandomAccessNoCoerce`), and by `next` (neither), read by
/// each of std's consumers, and through adaptors that fold it or read it by `try_fold`.
pub fn zips(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let split = (kind >> 5) as usize % (data.len() + 1);
	let (a, b) = data.split_at(split);
	let (a, b) = if kind & 8 == 0 { (a, b) } else { (b, a) };
	let other = kind & 4 != 0;
	let n = data.first().map_or(0, |&b| b as usize % 4);
	let log = std::cell::RefCell::new(Vec::new());
	macro_rules! seen {
		($tag:expr) => {
			|&x: &u8| {
				log.borrow_mut().push(x ^ $tag);
				x
			}
		};
	}
	macro_rules! owned {
		($tag:expr) => {
			|x: u8| {
				log.borrow_mut().push(x ^ $tag);
				x
			}
		};
	}
	// `$zip` read one way, by the case's bits.
	macro_rules! read {
		($zip:expr) => {{
			let z = $zip;
			let at = |(x, y): (u8, u8)| x ^ y;
			match data.len() % 8 {
				0 => out.push(z.count() as u8),
				1 => out.push(z.last().map_or(0xee, at)),
				2 => z.for_each(|p| out.push(at(p))),
				3 => out.extend(z.map(|(x, y)| x as u32 + y as u32).sum::<u32>().to_le_bytes()),
				4 => out.extend(z.map(at).collect::<Vec<_>>()),
				5 => out.push(z.fold(0u8, |acc, p| acc.wrapping_mul(3) ^ at(p))),
				6 => out.push(z.map(at).any(|x| x == 0) as u8),
				_ => {
					let mut z = z;
					out.push(z.position(|(x, y)| x > y).map_or(0xee, |i| i as u8));
					out.push(z.find(|&(x, _)| x % 3 == 0).map_or(0xee, at));
					out.push(z.nth(n).map_or(0xee, at));
					out.push(z.next().map_or(0xee, at));
				}
			}
		}};
	}
	// `$zip` and then adaptors of it, each folding it or reading it by `try_fold`.
	macro_rules! adapt {
		($zip:expr) => {{
			match kind & 16 {
				0 => read!($zip),
				_ => match data.len() % 8 {
					0 => out.push($zip.filter(|&(x, _)| x % 2 == 0).count() as u8),
					1 => out.push($zip.filter_map(|(x, y)| x.checked_sub(y)).count() as u8),
					2 => out.push($zip.skip_while(|&(x, _)| x < 0x80).count() as u8),
					3 => out.push($zip.enumerate().map(|(i, _)| i as u8).last().unwrap_or(0xee)),
					4 => out.push($zip.chain([(1, 2)]).count() as u8),
					5 => out.push($zip.take(n).count() as u8),
					6 => out.push($zip.skip(n).count() as u8),
					_ => {
						let mut p = $zip.peekable();
						let first = p.peek().is_some() as u8;
						out.push(first ^ p.count() as u8);
					}
				},
			}
		}};
	}
	match kind % 4 {
		// By index, never past the shorter side.
		0 => adapt!(a.iter().map(seen!(1)).zip(b.iter().map(seen!(2)))),
		// `TrustedLen`, not by index: `fold` stops at the shorter side, `next` reads the first one past it.
		1 => match other {
			false => adapt!(a.iter().rev().map(seen!(3)).zip(b.iter().rev().map(seen!(4)))),
			true => adapt!(
				a.iter()
					.map(seen!(5))
					.chain(b.iter().copied())
					.zip(b.iter().rev().map(seen!(6)))
			),
		},
		// Only `TrustedRandomAccessNoCoerce`, whose `fold` reads by index and `next` as any other zip's.
		2 => match other {
			false => adapt!(
				a.to_vec()
					.into_iter()
					.map(owned!(7))
					.zip(b.to_vec().into_iter().map(owned!(8)))
			),
			true => adapt!(
				a.to_vec()
					.into_iter()
					.skip(1)
					.map(owned!(9))
					.zip(b.iter().map(seen!(10)))
			),
		},
		// Neither, read by `next`, and nested: a zip of zips.
		_ => match other {
			false => adapt!(a.iter().map(seen!(11)).filter(|_| true).zip(b.iter().map(seen!(12)))),
			true => adapt!(
				a.iter()
					.map(seen!(13))
					.zip(b.iter().rev().map(seen!(14)))
					.zip(a.iter().rev().map(seen!(15)))
					.map(|((x, y), z)| (x ^ z, y))
			),
		},
	}
	out.extend_from_slice(&log.borrow());
}

/// `next_back` of a zip of sides that each log what they make, or of ranges, slices, their
/// `chunks` and `enumerate`, and zips of those, either side longer, read from either end:
/// std trims the longer side on the first pair it reads from the back, and a by-index zip
/// reads no item it never yields from a side that does not log, nor through `nth` it skips.
pub fn zips_back(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let split = data.first().map_or(0, |&b| b as usize) % (data.len() + 1);
	let (a, b) = data.split_at(split);
	let n = (kind >> 5) as usize;
	let log = std::cell::RefCell::new(Vec::new());
	macro_rules! seen {
		($tag:expr) => {
			|&x: &u8| {
				log.borrow_mut().push(x ^ $tag);
				x
			}
		};
	}
	// `$zip`, its pairs made bytes by `$at`, read from the back and, by the case's bits,
	// from the front first, through `nth`, and between back reads.
	macro_rules! back {
		($zip:expr, $at:expr) => {{
			let mut z = $zip;
			let at = $at;
			if kind & 8 != 0 {
				out.push(z.next().map_or(0xee, at));
			}
			if kind & 16 != 0 {
				out.push(z.nth(n).map_or(0xee, at));
			}
			while let Some(p) = z.next_back() {
				out.push(at(p));
				let (lower, upper) = z.size_hint();
				out.extend([lower as u8, upper.map_or(0xee, |u| u as u8)]);
				if kind & 24 == 24 {
					out.push(z.next().map_or(0xee, at));
				}
			}
			out.push(z.next().map_or(0xee, at));
		}};
	}
	let len = |c: &[u8]| c.len() as u8;
	match kind % 8 {
		0 => back!(a.iter().map(seen!(1)).zip(b.iter().copied()), |(x, y)| x ^ y),
		1 => back!(
			a.iter().copied().zip(b.iter().map(seen!(2)).zip(0..data.len())),
			|(x, (y, i)): (u8, (u8, usize))| x ^ y ^ i as u8
		),
		2 => back!(a.iter().map(seen!(3)).zip(b.iter().map(seen!(4))), |(x, y)| x ^ y),
		// `TrustedLen`, not by index: `next_back` trims either side.
		3 => back!(a.iter().rev().map(seen!(5)).zip(b.iter().map(seen!(6))), |(x, y)| x ^ y),
		4 => {
			let at = |(i, (&y, c)): (usize, (&u8, &[u8]))| i as u8 ^ y ^ len(c);
			back!((0..a.len()).zip(b.iter().zip(data.chunks(2))), at)
		}
		5 => {
			let at = |(c, (i, &y)): (&[u8], (usize, &u8))| len(c) ^ i as u8 ^ y;
			back!(a.chunks(2).zip(b.iter().enumerate()), at)
		}
		6 => {
			let logged = |c| {
				log.borrow_mut().push(len(c));
				c
			};
			let at = |(x, c): (u8, &[u8])| x ^ len(c);
			back!((split as u8..data.len() as u8).zip(a.chunks(3).map(logged)), at)
		}
		_ => {
			let logged = |(i, &x): (usize, &u8)| {
				log.borrow_mut().push(i as u8);
				x
			};
			let at = |(x, (i, c)): (u8, (usize, &[u8]))| x ^ i as u8 ^ len(c);
			back!(a.iter().enumerate().map(logged).zip(b.chunks(2).enumerate()), at)
		}
	}
	out.extend_from_slice(&log.borrow());
}

/// `start..` of each integer width and `char`, as the iterator an adaptor or consumer
/// takes and as its own, stepping up to its type's last value, past which std's debug
/// build panics, and ranges of `char` across the surrogates, which they skip.
pub fn range_froms(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let a = data.first().copied().unwrap_or(0);
	let near = a % 4;
	let how = kind >> 6;
	match kind % 16 {
		0 => from_top(u8::MAX - near, data, how, out, |x, out| out.push(x)),
		1 => from_top(u16::MAX - u16::from(near), data, how, out, |x, out| {
			out.extend(x.to_le_bytes())
		}),
		2 => from_top(u32::MAX - u32::from(near), data, how, out, |x, out| {
			out.extend(x.to_le_bytes())
		}),
		3 => from_top(u64::MAX - u64::from(near), data, how, out, |x, out| {
			out.extend(x.to_le_bytes())
		}),
		4 => from_top(u128::MAX - u128::from(near), data, how, out, |x, out| {
			out.extend(x.to_le_bytes())
		}),
		5 => from_top(usize::MAX - usize::from(near), data, how, out, |x, out| {
			out.extend(x.to_le_bytes())
		}),
		6 => from_top(i8::MAX - near as i8, data, how, out, |x, out| {
			out.extend(x.to_le_bytes())
		}),
		7 => from_top(i16::MAX - i16::from(near), data, how, out, |x, out| {
			out.extend(x.to_le_bytes())
		}),
		8 => from_top(i32::MAX - i32::from(near), data, how, out, |x, out| {
			out.extend(x.to_le_bytes())
		}),
		9 => from_top(i64::MAX - i64::from(near), data, how, out, |x, out| {
			out.extend(x.to_le_bytes())
		}),
		10 => from_top(i128::MAX - i128::from(near), data, how, out, |x, out| {
			out.extend(x.to_le_bytes())
		}),
		11 => from_top(isize::MAX - near as isize, data, how, out, |x, out| {
			out.extend(x.to_le_bytes())
		}),
		// Past 2^53, where a number becomes limbs, and past 2^64 for `u128`.
		12 => {
			let past = (1u64 << 53) - 2 + u64::from(near);
			from_top(past, data, how, out, |x, out| out.extend(x.to_le_bytes()));
			let wide = (1u128 << 64) - 2 + u128::from(near);
			from_top(wide, data, how ^ 2, out, |x, out| out.extend(x.to_le_bytes()));
			out.extend((-(1i64 << 53) - 2..).nth(usize::from(a)).unwrap_or(0).to_le_bytes());
		}
		13 => chars(a, data, how, out),
		14 => adapted(a, data, out),
		_ => overflowed(a, data, out),
	}
}

/// `start..` zipped with `data`, each item written by `put`: the range second or, if `how`
/// has bit 1, first, and read by `next` or, with bit 0, folded. First and read by `next`,
/// it steps once more than the data has, so it panics one item sooner; folded, std's zip
/// of two `TrustedLen` iterators reads only as many pairs as the data has.
fn from_top<T: Copy>(start: T, data: &[u8], how: u8, out: &mut Vec<u8>, put: impl Fn(T, &mut Vec<u8>))
where
	std::ops::RangeFrom<T>: Iterator<Item = T>,
{
	match how % 4 {
		0 => {
			for (&b, x) in data.iter().zip(start..) {
				out.push(b);
				put(x, out);
			}
		}
		1 => data.iter().zip(start..).for_each(|(&b, x)| {
			out.push(b);
			put(x, out);
		}),
		2 => {
			for (x, &b) in (start..).zip(data) {
				put(x, out);
				out.push(b);
			}
		}
		_ => (start..).zip(data).for_each(|(x, &b)| {
			put(x, out);
			out.push(b);
		}),
	}
}

/// `char` ranges of each kind across the surrogates, which std's `Step` skips, and up to
/// `char::MAX`, past which `start..` panics.
fn chars(a: u8, data: &[u8], how: u8, out: &mut Vec<u8>) {
	let near = u32::from(a % 4);
	let put = |c: char, out: &mut Vec<u8>| out.extend((c as u32).to_le_bytes());
	let gap = char::from_u32(0xd7fd + near).unwrap();
	from_top(gap, data, how, out, put);
	let top = char::from_u32(char::MAX as u32 - near).unwrap();
	from_top(top, data, how, out, put);
	let n = data.len();
	let end = char::from_u32(0xe000 + near).unwrap();
	for c in (gap..end).rev().step_by(2) {
		put(c, out);
	}
	put((gap..end).nth(n % 8).unwrap_or('-'), out);
	put((gap..=end).nth_back(n % 8).unwrap_or('-'), out);
	put((gap..).nth(n % 8).unwrap_or('-'), out);
	let (lo, hi) = (gap..=end).size_hint();
	out.extend([lo as u8, hi.map_or(0xee, |h| h as u8), (gap..end).count() as u8]);
	out.extend((gap..=end).map(|c| c as u32 as u8));
	out.push(('\u{d7ff}'..'\u{e000}').count() as u8);
	// Inclusive ones stepped past either end of the chars, which leaves the bound there.
	let mut last = top..=char::MAX;
	put(last.nth(n % 6).unwrap_or('-'), out);
	put(*last.start(), out);
	let mut first = '\0'..=char::from_u32(near).unwrap();
	put(first.nth_back(n % 6).unwrap_or('-'), out);
	put(*first.end(), out);
}

/// `start..` as what each adaptor and consumer taking an `IntoIterator` takes, and as its
/// own receiver.
fn adapted(a: u8, data: &[u8], out: &mut Vec<u8>) {
	let lo = a % 64;
	let b = data.get(1).copied().unwrap_or(0);
	let it = || data.iter().copied();
	let n = data.len();
	out.extend(it().chain(lo..).take(n + 3));
	let mut v: Vec<u8> = Vec::new();
	v.extend((lo..).take(3));
	out.extend(v);
	out.extend(vec![lo.., b / 2..].into_iter().flatten().take(5));
	out.extend(it().flat_map(|x| x / 4..).take(n * 2));
	out.extend(it().map(|x| x / 32..x / 16).flatten());
	out.extend(it().map(|x| x / 64..=x / 32).flatten());
	let keyed: std::collections::BTreeMap<u8, usize> = it().zip(0..).collect();
	out.extend(keyed.into_iter().map(|(k, v)| k ^ v as u8));
	out.extend(Vec::from_iter((lo..).step_by(3).take(4)));
	out.extend(String::from_iter(('a'..).take(n % 6)).bytes());
	let (low, high) = it().zip(lo..).size_hint();
	out.extend([low as u8, high.map_or(0xee, |h| h as u8)]);
	let (low, high) = (lo..).size_hint();
	out.extend([low as u8, high.map_or(0xee, |h| h as u8)]);
	// Its own iterator, read by reference and then by its `start`.
	let mut r = lo..;
	out.push(r.next().unwrap_or(0xee));
	out.push(r.nth(usize::from(b % 8)).unwrap_or(0xee));
	out.push(r.by_ref().take(2).count() as u8);
	out.push(r.start);
	for i in lo.. {
		if i > lo + 5 {
			break;
		}
		out.push(i);
	}
	out.push(
		(lo..)
			.position(|i| usize::from(i) >= n + usize::from(lo))
			.map_or(0xee, |i| i as u8),
	);
	out.extend(
		(lo..)
			.skip(2)
			.map(|i| i ^ 1)
			.filter(|i| i % 3 != 0)
			.take_while(|&i| i < lo + 20),
	);
}

/// Consumers that step `start..` past its type's last value, which panics in std's
/// debug build, as the fixture is built.
fn overflowed(a: u8, data: &[u8], out: &mut Vec<u8>) {
	let top = a.max(250);
	match data.len() % 8 {
		0 => out.extend(top..),
		1 => out.push((top..).take(10).count() as u8),
		2 => out.push((a..).nth(300).unwrap_or(0xee)),
		3 => out.push((a..).nth(usize::from(255 - a)).unwrap_or(0xee)),
		4 => out.extend((a..).step_by(100).take(5)),
		5 => out.push((i8::MAX - (a % 4) as i8..).take(5).count() as u8),
		6 => out.push((char::MAX..).next().map_or(0xee, |c| c as u8)),
		_ => out.extend(Vec::from_iter(top..)),
	}
}

/// The methods of each of std's range types over bounds `a` and `b` of the table `kind`
/// picks, at `data`'s first two bytes, against each of its values: integers of each
/// width at their ends and past 2^53 and 2^64, `char`s at either side of the surrogates,
/// floats with NaN and the zeros, and unsigned `NonZero`s.
pub fn range_methods(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	use std::num::NonZero;
	let at = [0, 1].map(|i| usize::from(data.get(i).copied().unwrap_or(0)));
	let past53 = (1 << 53) + 1;
	let past64 = (1u128 << 64) + 1;
	match kind % 19 {
		0 => stepped(&[0u8, 1, 2, 254, 255], at, out),
		1 => stepped(&[0u16, 1, 0x7fff, u16::MAX - 1, u16::MAX], at, out),
		2 => stepped(&[0u32, 1, 0x8000_0000, u32::MAX - 1, u32::MAX], at, out),
		3 => stepped(
			&[0u64, 1, past53 - 2, past53 - 1, past53, u64::MAX - 1, u64::MAX],
			at,
			out,
		),
		4 => stepped(
			&[0, 1, past64 - 2, past64 - 1, past64, u128::MAX - 1, u128::MAX],
			at,
			out,
		),
		5 => stepped(&[0usize, 1, 0x8000_0000, usize::MAX - 1, usize::MAX], at, out),
		6 => stepped(&[i8::MIN, i8::MIN + 1, -1, 0, i8::MAX], at, out),
		7 => stepped(&[i16::MIN, -1, 0, 1, i16::MAX - 1, i16::MAX], at, out),
		8 => stepped(&[i32::MIN, i32::MIN + 1, -1, 0, i32::MAX], at, out),
		9 => {
			let past53 = past53 as i64;
			stepped(&[i64::MIN, -past53, -1, 0, past53, i64::MAX - 1, i64::MAX], at, out)
		}
		10 => {
			let past64 = past64 as i128;
			stepped(&[i128::MIN, i128::MIN + 1, -past64, -1, 0, past64, i128::MAX], at, out)
		}
		11 => stepped(&[isize::MIN, -1, 0, 1, isize::MAX - 1, isize::MAX], at, out),
		12 => stepped(
			&['\0', 'a', '\u{d7fe}', '\u{d7ff}', '\u{e000}', '\u{e001}', char::MAX],
			at,
			out,
		),
		13 => drop(bounded(
			&[f64::NAN, f64::NEG_INFINITY, -1.5, -0.0, 0.0, 2.5, f64::INFINITY],
			at,
			out,
		)),
		14 => drop(bounded(
			&[f32::NAN, f32::NEG_INFINITY, -1.5, -0.0, 0.0, 2.5, f32::INFINITY],
			at,
			out,
		)),
		15 => stepped(&[1, 2, 254, 255].map(|n| NonZero::<u8>::new(n).unwrap()), at, out),
		16 => stepped(&[1, past53, u64::MAX].map(|n| NonZero::new(n).unwrap()), at, out),
		17 => stepped(&[1, past64, u128::MAX].map(|n| NonZero::new(n).unwrap()), at, out),
		_ => stepped(&[1, 2, usize::MAX].map(|n| NonZero::new(n).unwrap()), at, out),
	}
}

/// Each range type's comparisons of `a` and `b`, from `table`, against each of its
/// values, and their `Debug`.
fn bounded<T: Copy + PartialOrd + fmt::Debug>(table: &[T], at: [usize; 2], out: &mut Vec<u8>) -> (T, T) {
	use std::ops::{Bound, RangeBounds};
	let [a, b] = at.map(|i| table[i % table.len()]);
	out.extend([(a..b).is_empty(), (a..=b).is_empty()].map(u8::from));
	for x in table {
		let held = [
			(a..b).contains(x),
			(a..=b).contains(x),
			(a..).contains(x),
			(..b).contains(x),
			(..=b).contains(x),
			bounds_contain(a..b, x),
			bounds_contain(a..=b, x),
			bounds_contain(a.., x),
			bounds_contain(..b, x),
			bounds_contain(..=b, x),
			bounds_contain(.., x),
			(Bound::Excluded(a), Bound::Included(b)).contains(x),
			(Bound::Unbounded, Bound::Excluded(b)).contains(x),
			(a..b) == (*x..b),
			(a..=b) == (a..=*x),
			(a..) != (*x..),
			(..b) == (..*x),
			(..=b) != (..=*x),
		];
		out.extend(held.map(u8::from));
	}
	let (start, end) = (a..=b).into_inner();
	let inclusive = a..=b;
	let shown = format!(
		"{:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?}",
		a..b,
		inclusive,
		a..,
		..b,
		..=b,
		(start, end),
		(inclusive.start(), inclusive.end()),
		(.., (.., 1..2)),
	);
	out.extend(shown.bytes());
	let (from, to, through) = (a.., ..b, ..=b);
	let bounds = [
		inclusive.start_bound(),
		inclusive.end_bound(),
		from.start_bound(),
		from.end_bound(),
		to.end_bound(),
		through.end_bound(),
	];
	out.extend(format!("{bounds:?}").bytes());
	(a, b)
}

/// `RangeBounds::contains`, through the trait.
fn bounds_contain<T: PartialOrd>(r: impl std::ops::RangeBounds<T>, x: &T) -> bool {
	r.contains(x)
}

/// `bounded`'s, then an inclusive range of `a` and `b` read each way before its methods:
/// from either end, by `next` past its last item, which wraps it at its type's end, and
/// folded, which exhausts it at its end. Each is compared with fresh ranges and hashed.
fn stepped<T>(table: &[T], at: [usize; 2], out: &mut Vec<u8>)
where
	T: Copy + Eq + PartialOrd + fmt::Debug + std::hash::Hash,
	std::ops::RangeInclusive<T>: DoubleEndedIterator<Item = T>,
{
	use std::hash::{DefaultHasher, Hash, Hasher};
	use std::ops::RangeBounds;
	let (a, b) = bounded(table, at, out);
	let hash = |value: &dyn Fn(&mut DefaultHasher)| {
		let mut h = DefaultHasher::new();
		value(&mut h);
		h.finish().to_le_bytes()
	};
	out.extend(hash(&|h| {
		(a..b).hash(h);
		(a..=b).hash(h);
		(a..).hash(h);
		(..b).hash(h);
		(..=b).hash(h);
		(..).hash(h);
	}));
	for how in 0..9 {
		let mut r = a..=b;
		let n = at[0] % 4;
		let read = match how {
			0 => None,
			1 => r.next(),
			2 => r.next_back(),
			3 => r.by_ref().take(300).last(),
			4 => r.by_ref().rev().take(300).last(),
			5 => (0..300).map_while(|_| r.next()).last(),
			6 => (0..300).map_while(|_| r.next_back()).last(),
			7 => r.nth(n),
			_ => r.nth_back(n),
		};
		let (start, end) = (*r.start(), *r.end());
		let keyed = std::collections::HashSet::from([a..=b, start..=end, start..=end]);
		out.extend(format!("{read:?} {r:?} {}", keyed.len()).bytes());
		out.extend(format!("{:?} {:?}", r.start_bound(), r.end_bound()).bytes());
		out.extend([r.is_empty(), r == (a..=b), r == (start..=end), r != (start..=end)].map(u8::from));
		out.extend(table.iter().map(|x| u8::from(r.contains(x))));
		out.extend(hash(&|h| r.hash(h)));
		let (start, end) = r.into_inner();
		out.extend(format!("{start:?} {end:?}").bytes());
	}
}

/// std's comparisons of iterators, logging each item they read and each call of a `_by`
/// form's closure, which counts them in what it captures: against the data cut short, run
/// long, or changed at its end, by `kind >> 4`. Of two `TrustedLen` sides, whose `eq` reads
/// neither unless their lengths agree; of sides that are not, the first by `&mut` and read
/// on after; of floats with NaN for 0xff, against a `Vec` by value; and of `start..`, which
/// never ends, and panics past its type's last value.
pub fn compares(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let log = std::cell::RefCell::new(Vec::new());
	let mut other = data.to_vec();
	match (kind >> 4) % 4 {
		0 => {}
		1 => drop(other.pop()),
		2 => other.push(kind),
		_ => {
			if let Some(last) = other.last_mut() {
				*last = last.wrapping_add(1);
			}
		}
	}
	macro_rules! seen {
		($tag:expr) => {
			|&x: &u8| {
				log.borrow_mut().push(x ^ $tag);
				x
			}
		};
	}
	macro_rules! stepped {
		($tag:expr) => {
			|x: u8| {
				log.borrow_mut().push(x ^ $tag);
				x
			}
		};
	}
	macro_rules! float {
		($tag:expr) => {
			|&x: &u8| {
				log.borrow_mut().push(x ^ $tag);
				if x == 0xff { f32::NAN } else { f32::from(x) }
			}
		};
	}
	let order = |o: Option<std::cmp::Ordering>| o.map_or(9, |o| o as u8);
	// By `PartialEq` and `PartialOrd`, and the `_by` forms of each.
	macro_rules! partial {
		($a:expr, $b:expr) => {{
			let mut calls = 0u8;
			let r = match kind % 16 {
				0 => $a.eq($b) as u8,
				1 => $a.ne($b) as u8,
				2 => $a.lt($b) as u8,
				3 => $a.le($b) as u8,
				4 => $a.gt($b) as u8,
				5 => $a.ge($b) as u8,
				6 => order($a.partial_cmp($b)),
				7 => $a.eq_by($b, |x, y| {
					calls += 1;
					x == y
				}) as u8,
				_ => order($a.partial_cmp_by($b, |x, y| {
					calls += 1;
					x.partial_cmp(&y)
				})),
			};
			out.extend([r, calls]);
		}};
	}
	// And by `Ord`, and a comparison that overflows, which panics, past 255.
	macro_rules! total {
		($a:expr, $b:expr) => {{
			let mut calls = 0u8;
			match kind % 16 {
				9 => out.push($a.cmp($b) as u8),
				10 => out.push($a.cmp_by($b, |x, y| {
					calls += 1;
					x.cmp(&y)
				}) as u8),
				11 => out.push($a.cmp_by($b, |x: u8, y: u8| (x + y).cmp(&(y + x))) as u8),
				_ => partial!($a, $b),
			}
			out.push(calls);
		}};
	}
	match kind >> 6 {
		0 => total!(data.iter().map(seen!(1)), other.iter().map(seen!(2))),
		1 => {
			let mut a = data.iter().map(seen!(3)).filter(|_| true);
			total!(a.by_ref(), other.iter().map(seen!(4)).skip_while(|_| false));
			out.push(a.count() as u8);
		}
		2 => {
			let b: Vec<f32> = other.iter().map(float!(6)).collect();
			partial!(data.iter().map(float!(5)), b);
		}
		_ => {
			let lo = data.first().copied().unwrap_or(0);
			match (kind >> 4) % 4 {
				0 => total!(data.iter().map(seen!(7)), (lo..).map(stepped!(8))),
				1 => total!((lo..).map(stepped!(9)), data.iter().map(seen!(10))),
				2 => total!((lo..).map(stepped!(11)), (lo..).map(stepped!(12))),
				_ => total!((lo..).take(data.len()).map(stepped!(13)), (lo..).map(stepped!(14))),
			}
		}
	}
	out.extend_from_slice(&log.borrow());
}
