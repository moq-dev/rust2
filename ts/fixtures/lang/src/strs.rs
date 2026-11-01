//! `str` and `String` over `char`: iterated from both ends, searched by a `char`, a
//! predicate, or a `&str`, trimmed, split, sliced by byte offsets, built a char at a
//! time, edited in place, and parsed into a `bool`, a `char`, or a `NonZero`. Text is made of chars of every UTF-8 width and those next to UTF-16's surrogates,
//! so byte offsets and code units part. Predicates log each char they are called with, so
//! where a search stops is output too.

use std::ops::Bound;

use crate::fmt::put;

/// Separators, digits, and chars where a UTF-8 or UTF-16 length changes.
const ALPHABET: [char; 16] = [
	'/',
	'\\',
	'a',
	'0',
	'9',
	'\u{7f}',
	'\u{80}',
	'é',
	'\u{7ff}',
	'\u{800}',
	'\u{d7ff}',
	'\u{e000}',
	'\u{ffff}',
	'\u{10000}',
	'😀',
	'\u{10ffff}',
];

/// A char of the alphabet for each byte, by its low bits.
pub fn text(data: &[u8]) -> String {
	let mut s = String::new();
	for &b in data {
		s.push(ALPHABET[(b % 16) as usize]);
	}
	s
}

/// Whether the `i`th step reads from the back: by the byte's high bit.
fn from_back(data: &[u8], i: usize) -> bool {
	matches!(data.get(i), Some(&b) if b & 0x80 != 0)
}

fn put_char(out: &mut Vec<u8>, c: Option<char>) {
	match c {
		Some(c) => out.extend_from_slice(&u32::from(c).to_be_bytes()),
		None => out.push(0xff),
	}
}

fn put_offset(out: &mut Vec<u8>, i: Option<usize>) {
	match i {
		Some(i) => out.extend_from_slice(&(i as u32).to_be_bytes()),
		None => out.push(0xfe),
	}
}

/// What a predicate saw, then where it stopped.
fn put_seen(out: &mut Vec<u8>, seen: &[char]) {
	for &c in seen {
		put_char(out, Some(c));
	}
	out.push(0xfd);
}

pub fn strs(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let s = text(data);
	// The pattern: a char of the alphabet, by the kind's high bits.
	let sep = ALPHABET[(kind / 8 % 16) as usize];
	match kind % 8 {
		0 => {
			// `chars` from both ends, until they meet.
			let mut it = s.chars();
			let mut i = 0;
			loop {
				let c = match from_back(data, i) {
					true => it.next_back(),
					false => it.next(),
				};
				put_char(out, c);
				if c.is_none() {
					break;
				}
				i += 1;
			}
			out.push(s.chars().count() as u8);
			for c in s.chars().rev() {
				out.push(c.is_ascii_digit() as u8);
			}
			out.push(s.is_empty() as u8);
			out.push(s.as_str().is_empty() as u8);
		}
		1 => {
			// `split` by a char, from both ends.
			let mut parts = s.split(sep);
			let mut i = 0;
			loop {
				let part = match from_back(data, i) {
					true => parts.next_back(),
					false => parts.next(),
				};
				let Some(part) = part else {
					break;
				};
				put(out, part);
				out.push(part.len() as u8);
				i += 1;
			}
			out.push(i as u8);
			put(out, s.split(sep).next().unwrap_or("none"));
		}
		2 => {
			// `split` by a predicate, which sees each char once, in the order it is read.
			let mut seen = Vec::new();
			let mut parts = s.split(|c: char| {
				seen.push(c);
				c == sep || c.is_ascii_digit()
			});
			let mut i = 0;
			loop {
				let part = match from_back(data, i) {
					true => parts.next_back(),
					false => parts.next(),
				};
				let Some(part) = part else {
					break;
				};
				put(out, part);
				i += 1;
			}
			put_seen(out, &seen);
		}
		3 => {
			// Trimmed, tested, and found by a char.
			put(out, s.trim_start_matches(sep));
			put(out, s.trim_end_matches(sep));
			put(out, s.trim_start_matches(sep).trim_end_matches(sep));
			out.push(s.starts_with(sep) as u8);
			put_offset(out, s.rfind(sep));
		}
		4 => {
			// Trimmed, tested, and found by a predicate.
			let mut seen = Vec::new();
			let trimmed = s.trim_start_matches(|c: char| {
				seen.push(c);
				c == sep
			});
			put(out, trimmed);
			put_seen(out, &seen);
			let trimmed = s.trim_end_matches(|c: char| {
				seen.push(c);
				c != sep
			});
			put(out, trimmed);
			put_seen(out, &seen);
			out.push(s.starts_with(|c: char| c.is_ascii_digit()) as u8);
			put(out, s.trim_start_matches(digit));
			let found = s.rfind(|c: char| {
				seen.push(c);
				c == sep
			});
			put_offset(out, found);
			put_seen(out, &seen);
		}
		5 => {
			// `match_indices` by a char, from both ends: byte offsets, not code units.
			let mut found = s.match_indices(sep);
			let mut i = 0;
			loop {
				let item = match from_back(data, i) {
					true => found.next_back(),
					false => found.next(),
				};
				let Some((at, matched)) = item else {
					break;
				};
				put_offset(out, Some(at));
				put(out, matched);
				i += 1;
			}
			let ends: Vec<usize> = s.match_indices('/').map(|(i, _)| i).collect();
			for end in ends {
				put_offset(out, Some(end));
			}
		}
		6 => {
			// A path's parts, unescaped and escaped again, as moq's namespaces are.
			let parts = unescape(&s);
			for part in &parts {
				put(out, part);
			}
			put(out, &escape(&parts));
		}
		_ => {
			// `push` through a `&mut String`: to a borrowed local, a field, a field through a
			// `&mut self`, and an element, of a char made at run time.
			let mut built = String::new();
			let mut named = Named { name: String::new() };
			let mut each = vec![String::new(), String::new()];
			for (i, c) in s.chars().enumerate() {
				let target = &mut built;
				target.push(c);
				named.name.push(if i % 2 == 0 { c } else { sep });
				named.add(c);
				each[i % 2].push(c);
				if let Some(c) = char::from_u32(u32::from(c) + 1) {
					built.push(c);
				}
			}
			named.name.push('\u{10ffff}');
			put(out, &built);
			put(out, &named.name);
			put(out, &each[0]);
			put(out, &each[1]);
			// Reads from before a push, which must not see it.
			let before = built.clone();
			let len = built.len();
			let empty = built.is_empty();
			built.push(sep);
			put(out, &before);
			out.push(len as u8);
			out.push(empty as u8);
			let taken = std::mem::take(&mut built);
			put(out, &taken);
			out.push(built.is_empty() as u8);
		}
	}
}

/// Up to three chars of `s`, from the char the kind's high bits pick: empty a quarter of
/// the time, and otherwise found at least once unless the text is shorter.
fn pattern(kind: u8, s: &str) -> String {
	let mut p = String::new();
	for c in s.chars().skip((kind / 32) as usize).take((kind / 8 % 4) as usize) {
		p.push(c);
	}
	p
}

/// `&s[a..b]` by byte offsets, and the `str` methods by a `&str` pattern, which an empty
/// one matches before each char and after the last.
pub fn slices(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let s = text(data);
	let p = pattern(kind, &s);
	match kind % 8 {
		0 => {
			// `split`, by a `&str` and by a `&String`.
			let mut n = 0;
			for part in s.split(p.as_str()) {
				put(out, part);
				n += 1;
			}
			out.push(n as u8);
			put(out, s.split(&p).last().unwrap_or("none"));
		}
		1 => {
			// `match_indices`: byte offsets, then the text before and after each match.
			for (at, matched) in s.match_indices(p.as_str()) {
				put_offset(out, Some(at));
				put(out, matched);
				put(out, &s[..at]);
				put(out, &s[at + matched.len()..]);
				put(out, &s[at..at + matched.len()]);
			}
		}
		2 => {
			// Trimmed, tested, and found.
			put(out, s.trim_start_matches(p.as_str()));
			put(out, s.trim_end_matches(p.as_str()));
			put(out, s.trim_start_matches(&p).trim_end_matches(&p));
			out.push(s.starts_with(p.as_str()) as u8);
			out.push(s.contains(p.as_str()) as u8);
			out.push(s.contains(&p) as u8);
			out.push(s.contains('/') as u8);
			out.push(s.contains(|c: char| c.is_ascii_digit()) as u8);
			put_offset(out, s.rfind(p.as_str()));
		}
		3 => {
			// A path and each parent of it, as moq-net's discovery walks them.
			let covering = std::iter::successors(Some(s.as_str()), |path| {
				(!path.is_empty()).then(|| path.rfind('/').map_or("", |slash| &path[..slash]))
			});
			for path in covering {
				put(out, path);
			}
		}
		4 => {
			// A path cut after its `keep`th part, and the rest, as moq-net's compression
			// cuts one.
			let keep = (kind / 8 % 4) as usize + 1;
			let head = s.match_indices('/').nth(keep - 1).map(|(end, _)| &s[..end]);
			put(out, head.unwrap_or("none"));
			let start = s.match_indices('/').nth(keep - 1).map_or(s.len(), |(i, _)| i + 1);
			put(out, &s[start..]);
		}
		5 => {
			// Parts joined again, by the pattern.
			let parts: Vec<&str> = s.split('/').collect();
			put(out, &parts.join(p.as_str()));
			let owned: Vec<String> = parts.iter().map(|part| part.to_string()).collect();
			put(out, &owned.join(p.as_str()));
			put(out, &["a", "", &s].join("/"));
		}
		_ => {
			// A slice at any offsets, or at ones about the end by the kind's bit 5, which
			// panics as std does past the end, backwards, or inside a char, whichever comes
			// first.
			let at = |b: u8| match kind & 0x20 {
				0 => b as usize % (s.len() + 2),
				_ => (s.len() + 1).saturating_sub(b as usize % 4),
			};
			let (a, b) = match data {
				[a, b, ..] => (at(*a), at(*b)),
				_ => (0, 0),
			};
			let sliced = match kind / 8 % 4 {
				0 => &s[a..b],
				1 => &s[a..],
				2 => &s.as_str()[..b],
				_ => &s.as_str()[..],
			};
			put(out, sliced);
		}
	}
}

/// A predicate that is a function, not a closure.
fn digit(c: char) -> bool {
	c.is_ascii_digit()
}

struct Named {
	name: String,
}

impl Named {
	fn add(&mut self, c: char) {
		self.name.push(c);
	}
}

/// `/` separates parts, and `\` escapes a `/` or `\` within one.
fn unescape(path: &str) -> Vec<String> {
	if path.is_empty() {
		return Vec::new();
	}
	let mut parts = Vec::new();
	let mut part = String::new();
	let mut chars = path.chars();
	while let Some(ch) = chars.next() {
		match ch {
			'/' => parts.push(std::mem::take(&mut part)),
			'\\' => match chars.next() {
				Some(next @ ('/' | '\\')) => part.push(next),
				Some(next) => {
					part.push('\\');
					part.push(next);
				}
				None => part.push('\\'),
			},
			_ => part.push(ch),
		}
	}
	parts.push(part);
	parts
}

fn escape(parts: &[String]) -> String {
	let mut path = String::new();
	for (i, part) in parts.iter().enumerate() {
		if i > 0 {
			path.push('/');
		}
		for ch in part.chars() {
			if matches!(ch, '/' | '\\') {
				path.push('\\');
			}
			path.push(ch);
		}
	}
	path
}

/// A byte offset into `s` from `b`: mostly where a char starts or the end, and otherwise
/// any offset up to two past the end, inside a char or out of bounds.
fn offset(s: &str, b: u8) -> usize {
	if b & 0x70 == 0 {
		return usize::from(b) % (s.len() + 3);
	}
	let mut skip = usize::from(b) % (s.chars().count() + 1);
	for i in 0..s.len() {
		if s.is_char_boundary(i) {
			if skip == 0 {
				return i;
			}
			skip -= 1;
		}
	}
	s.len()
}

/// `String::retain` by a function behind a `&mut`, behind two, and boxed behind one.
fn retain_by(s: &mut String, f: &mut fn(char) -> bool, how: u8) {
	match how % 3 {
		0 => s.retain(f),
		1 => s.retain(&mut &mut *f),
		_ => s.retain(Box::new(f)),
	}
}

fn odd(c: char) -> bool {
	u32::from(c) % 2 == 1
}

/// One edit of `s` by `op`, taking offsets and text from `a` and `b`, through a `&mut`.
fn edit(s: &mut String, op: u8, a: u8, b: u8, out: &mut Vec<u8>) {
	let piece = text(&[a, b, a ^ b][..usize::from(a % 4)]);
	let (i, j) = (offset(s, a), offset(s, b));
	match op % 14 {
		0 => s.push_str(&piece),
		1 => *s += &piece,
		2 if a % 4 == 0 => s.clear(),
		2 => s.push(ALPHABET[usize::from(b % 16)]),
		3 => s.truncate(i),
		4 => put_char(out, s.pop()),
		5 => put_char(out, Some(s.remove(i))),
		6 => s.insert(i, ALPHABET[usize::from(b % 16)]),
		7 => s.insert_str(i, &piece),
		8 if b % 2 == 1 => {
			let mut f: fn(char) -> bool = odd;
			retain_by(s, &mut f, a);
		}
		8 => {
			let mut seen = Vec::new();
			s.retain(|c| {
				seen.push(c);
				u32::from(c) % 3 != u32::from(a % 3)
			});
			put_seen(out, &seen);
		}
		9 => put(out, &s.split_off(i)),
		10 => match b % 7 {
			0 => s.replace_range(i..j, &piece),
			1 => s.replace_range(i.., &piece),
			2 => s.replace_range(..j, &piece),
			3 => s.replace_range(.., &piece),
			4 => s.replace_range(i..=j, &piece),
			5 => s.replace_range(..=j, &piece),
			_ => s.replace_range((Bound::Excluded(i), Bound::Included(j)), &piece),
		},
		11 => match b % 5 {
			0 => s.extend_from_within(i..j),
			1 => s.extend_from_within(i..),
			2 => s.extend_from_within(..=j),
			3 => s.extend_from_within((Bound::Included(i), Bound::Excluded(j))),
			_ => s.extend_from_within(..),
		},
		// Past `isize::MAX`, a capacity overflows; native Rust would allocate one up to it.
		12 => match a % 4 {
			0 => s.reserve(usize::MAX - usize::from(b)),
			1 => s.reserve_exact(isize::MAX as usize - s.len() + 1 + usize::from(b % 2)),
			_ => {
				s.reserve(usize::from(a));
				s.reserve_exact(usize::from(b));
				s.shrink_to(usize::from(a));
				s.shrink_to_fit();
			}
		},
		_ => {
			let capacity = match a % 4 {
				0 => isize::MAX as usize + 1 + usize::from(b % 2),
				_ => usize::from(a),
			};
			let mut fresh = String::with_capacity(capacity);
			fresh.push_str(s);
			*s = fresh;
		}
	}
}

/// `String`'s edits in place, three bytes each, on text of chars of every UTF-8 width:
/// through a `&mut String`, on a local, and on a field, each printed after each edit.
/// An offset off a char boundary or out of bounds panics as std's does.
pub fn edits(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let (head, ops) = data.split_at(data.len().min(usize::from(kind % 8)));
	let mut s = text(head);
	let mut named = Named { name: text(head) };
	for op in ops.chunks(3) {
		let [op, a, b] = [op[0], *op.get(1).unwrap_or(&0), *op.get(2).unwrap_or(&0x10)];
		match op / 64 {
			0 => edit(&mut named.name, op, a, b, out),
			// On the local itself, with no `&mut` between.
			1 => match op % 6 {
				0 => s.push_str("é/"),
				1 => s.truncate(offset(&s, a)),
				2 => put_char(out, s.pop()),
				3 => s.insert_str(offset(&s, a), "😀"),
				4 => put(out, &s.split_off(offset(&s, a))),
				_ => s.clear(),
			},
			_ => edit(&mut s, op, a, b, out),
		}
		put(out, &s);
		put(out, &named.name);
	}
}

/// Where `bool`, `char`, and `NonZero` parse or fail: case, whitespace, each UTF-8 width,
/// a sign, zero, and every integer type's bounds and one past them.
const PARSE_EDGES: &[&str] = &[
	"true",
	"false",
	"True",
	"FALSE",
	"",
	" true",
	"true ",
	"t",
	"truefalse",
	"a",
	"\u{e9}",
	"\u{800}",
	"\u{1f600}",
	"\u{10ffff}",
	"ab",
	"a\u{1f600}",
	"\u{1f600}a",
	"\u{e9}\u{e9}",
	"0",
	"+0",
	"-0",
	"000",
	"1",
	"+1",
	"-1",
	"+",
	"-",
	"--1",
	"+-1",
	"1_0",
	"0x1",
	"127",
	"128",
	"-128",
	"-129",
	"255",
	"256",
	"32767",
	"32768",
	"-32768",
	"-32769",
	"65535",
	"65536",
	"2147483647",
	"2147483648",
	"-2147483648",
	"-2147483649",
	"4294967295",
	"4294967296",
	"9223372036854775807",
	"9223372036854775808",
	"-9223372036854775808",
	"-9223372036854775809",
	"18446744073709551615",
	"18446744073709551616",
	"170141183460469231731687303715884105727",
	"170141183460469231731687303715884105728",
	"-170141183460469231731687303715884105728",
	"-170141183460469231731687303715884105729",
	"340282366920938463463374607431768211455",
	"340282366920938463463374607431768211456",
];

/// One parse of `s` as `$t` each way std offers: its value, or its error by `Display`
/// and `Debug`, padded, truncated, and pretty.
macro_rules! parse_as {
	($t:ty, $s:expr, $out:expr) => {{
		let (s, out): (&str, &mut Vec<u8>) = ($s, $out);
		for parsed in [s.parse::<$t>(), <$t as std::str::FromStr>::from_str(s)] {
			match parsed {
				Ok(v) => put(out, &format!("{v:?}")),
				Err(e) => put(out, &format!("{e}|{e:?}|{e:>50}|{e:<3.5}|{e:>20?}|{e:#?}")),
			}
		}
		s.parse::<$t>()
	}};
}

/// `NonZero`'s parse, then its error's `IntErrorKind`, and `from_str_radix` in `radix`.
macro_rules! parse_nonzero {
	($t:ty, $s:expr, $radix:expr, $out:expr) => {{
		let (s, radix, out): (&str, u32, &mut Vec<u8>) = ($s, $radix, $out);
		if let Err(e) = parse_as!(std::num::NonZero<$t>, s, out) {
			put(out, &format!("{:?}", e.kind()));
		}
		match std::num::NonZero::<$t>::from_str_radix(s, radix) {
			Ok(v) => put(out, &format!("{v}")),
			Err(e) => put(out, &format!("{e}|{e:?}")),
		}
	}};
}

/// `str::parse` of the type `kind` picks, `bool`, `char`, or a `NonZero` of each integer,
/// of every edge, of text of chars of every UTF-8 width, or of bytes mapped into an
/// alphabet of signs, digits, and the letters of `true` and `false`.
pub fn parses(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	const ALPHABET: &[u8] = b"0123456789+-truefalsTF ";
	let radix = 2 + u32::from(data.first().copied().unwrap_or(0)) % 35;
	let one = match kind / 14 % 3 {
		0 => None,
		1 => Some(text(data)),
		_ => {
			let mut s = String::new();
			for &b in data {
				s.push(char::from(ALPHABET[usize::from(b) % ALPHABET.len()]));
			}
			Some(s)
		}
	};
	let all: Vec<&str> = match &one {
		Some(s) => vec![s],
		None => PARSE_EDGES.to_vec(),
	};
	for s in all {
		match kind % 14 {
			0 => {
				let _ = parse_as!(bool, s, out);
			}
			1 => {
				let _ = parse_as!(char, s, out);
			}
			2 => parse_nonzero!(u8, s, radix, out),
			3 => parse_nonzero!(u16, s, radix, out),
			4 => parse_nonzero!(u32, s, radix, out),
			5 => parse_nonzero!(u64, s, radix, out),
			6 => parse_nonzero!(u128, s, radix, out),
			7 => parse_nonzero!(usize, s, radix, out),
			8 => parse_nonzero!(i8, s, radix, out),
			9 => parse_nonzero!(i16, s, radix, out),
			10 => parse_nonzero!(i32, s, radix, out),
			11 => parse_nonzero!(i64, s, radix, out),
			12 => parse_nonzero!(i128, s, radix, out),
			_ => parse_nonzero!(isize, s, radix, out),
		}
	}
}

/// A crate's own iterator over `items`, which counts each `next` it is asked.
struct Counted<'a, T> {
	items: &'a [T],
	nexts: &'a std::cell::Cell<u8>,
}

impl<T: Clone> Iterator for Counted<'_, T> {
	type Item = T;

	fn next(&mut self) -> Option<T> {
		self.nexts.set(self.nexts.get() + 1);
		let (first, rest) = self.items.split_first()?;
		self.items = rest;
		Some(first.clone())
	}
}

/// `String` made from and extended by iterators of chars and of strings, each the kind
/// picks, from text of chars of every UTF-8 width and its parts between `/`s.
pub fn iters(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let s = text(data);
	let chars: Vec<char> = s.chars().collect();
	let parts: Vec<&str> = s.split('/').collect();
	let nexts = std::cell::Cell::new(0);
	let mut named = Named {
		name: text(&data[..data.len() / 2]),
	};
	match kind % 4 {
		0 => {
			put(out, &s.chars().collect::<String>());
			put(
				out,
				&s.chars().rev().filter(|c| !c.is_ascii_digit()).collect::<String>(),
			);
			put(out, &chars.iter().collect::<String>());
			put(out, &String::from_iter(chars.iter().copied().skip(1)));
			put(out, &std::iter::empty::<char>().collect::<String>());
			put(out, &char::from_u32(u32::from(kind)).into_iter().collect::<String>());
			let counted = Counted {
				items: &chars,
				nexts: &nexts,
			};
			put(out, &counted.collect::<String>());
		}
		1 => {
			put(out, &parts.iter().copied().collect::<String>());
			put(out, &s.split('/').rev().collect::<String>());
			put(out, &parts.iter().map(|p| p.to_string()).collect::<String>());
			put(
				out,
				&s.split('/')
					.map(|p| format!("{p}{p}"))
					.filter(|p| !p.is_empty())
					.collect::<String>(),
			);
			put(out, &std::iter::empty::<String>().collect::<String>());
			let cow: std::borrow::Cow<str> = s.chars().rev().collect();
			put(out, &cow);
			let boxed: Box<str> = parts.iter().copied().collect();
			put(out, &boxed);
			let owned: Vec<String> = parts.iter().map(|p| p.to_string()).collect();
			put(
				out,
				&owned
					.iter()
					.map(|p| std::borrow::Cow::Borrowed(p.as_str()))
					.collect::<String>(),
			);
			let counted = Counted {
				items: &owned,
				nexts: &nexts,
			};
			put(out, &counted.collect::<String>());
		}
		2 => {
			// Through a `&mut`, on a local, and on a field.
			let mut t = String::from("é");
			extend_chars(&mut t, &chars, kind);
			t.extend(s.chars().rev());
			named.name.extend(&chars);
			named.name.extend(std::iter::empty::<char>());
			let counted = Counted {
				items: &chars,
				nexts: &nexts,
			};
			t.extend(counted);
			put(out, &t);
		}
		_ => {
			let mut t = String::new();
			t.extend(s.split('/'));
			t.extend(parts.iter().map(|p| format!("{p}é")));
			named.name.extend(parts.iter().rev().copied());
			named
				.name
				.extend(parts.iter().map(|p| std::borrow::Cow::Owned(p.to_string())));
			named.name.extend(std::iter::empty::<&str>());
			let counted = Counted {
				items: &parts,
				nexts: &nexts,
			};
			t.extend(counted);
			put(out, &t);
		}
	}
	put(out, &named.name);
	out.push(nexts.get());
}

fn extend_chars(t: &mut String, chars: &[char], kind: u8) {
	match kind / 4 % 3 {
		0 => t.extend(chars.iter()),
		1 => t.extend(chars.iter().copied().filter(|c| c.is_ascii_digit())),
		_ => t.extend(Some('😀')),
	}
}

/// `String::drain` of `s` by the range `form` picks, between offsets from `a` and `b`,
/// read from either end by `steps`' bits, then dropped, forgotten, or read to its end.
fn drain_by(s: &mut String, form: u8, a: u8, b: u8, steps: u8, out: &mut Vec<u8>) {
	let (i, j) = (offset(s, a), offset(s, b));
	let mut drain = match form % 8 {
		0 => s.drain(i..j),
		1 => s.drain(i..),
		2 => s.drain(..j),
		3 => s.drain(..),
		4 => s.drain(i..=j),
		5 => s.drain(..=j),
		6 => s.drain((Bound::Excluded(i), Bound::Included(j))),
		_ => s.drain((Bound::Included(i), Bound::Unbounded)),
	};
	// Up to three chars, each from the front or the back by a bit of `steps`.
	for k in 0..steps % 4 {
		let c = match steps >> (2 + k) & 1 {
			0 => drain.next(),
			_ => drain.next_back(),
		};
		put_char(out, c);
	}
	match form / 8 % 4 {
		0 => {}
		// Forgotten, a drain leaves the string as it was.
		1 => std::mem::forget(drain),
		2 => put(out, &drain.collect::<String>()),
		_ => {
			put(out, drain.as_str());
			for c in drain.rev() {
				put_char(out, Some(c));
			}
		}
	}
}

/// `String::drain`s, four bytes each, on text of chars of every UTF-8 width: through a
/// `&mut String`, on a local, and on a field, each printed after each drain. An offset off
/// a char boundary or out of bounds panics as std's does.
pub fn drains(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let (head, ops) = data.split_at(data.len().min(usize::from(kind % 8)));
	let mut s = text(head);
	let mut named = Named { name: text(head) };
	for op in ops.chunks(4) {
		let [form, a, b, steps] = [
			op[0],
			*op.get(1).unwrap_or(&0),
			*op.get(2).unwrap_or(&0x10),
			*op.get(3).unwrap_or(&0),
		];
		match form / 32 % 4 {
			0 => drain_by(&mut named.name, form, a, b, steps, out),
			// On the local itself, with no `&mut` between.
			1 => match form % 4 {
				0 => put(out, &s.drain(..offset(&s, a)).collect::<String>()),
				1 => out.push(s.drain(offset(&s, a)..).count() as u8),
				2 => put_char(out, s.drain(..).last()),
				_ => {
					s.drain(offset(&s, a)..offset(&s, b));
				}
			},
			_ => drain_by(&mut s, form, a, b, steps, out),
		}
		put(out, &s);
		put(out, &named.name);
	}
}

/// Range lengths whose size hint, through `hinted`'s adaptors, is either a few chars or
/// past where `isize::MAX` bytes of text start: std's `String` would reserve any between,
/// up to 2 GiB.
const HINTED: [u32; 7] = [0, 5, 8, 9, 0x7fff_fffd, 0x8000_0000, u32::MAX];

/// Chars from ranges as long as `data`'s first two bytes pick, through the adaptors `kind`
/// picks, into a `String` holding `aé` and the text of the rest: std's reserves the size
/// hint's lower bound first, which panics past `isize::MAX` before it reads a char, and
/// else the chars panic with `consumed` past the eighth.
pub fn hinted(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let pick = |i: usize| HINTED[usize::from(data.get(i).copied().unwrap_or(0)) % HINTED.len()];
	let (n, m) = (pick(0), pick(1));
	let sub = usize::from(data.get(1).copied().unwrap_or(0)) / HINTED.len();
	let mut t = String::from("aé");
	t.push_str(&text(data.get(2..).unwrap_or(&[])));
	let calls = std::cell::Cell::new(0usize);
	let f = |_: u32| {
		calls.set(calls.get() + 1);
		assert!(calls.get() <= 8, "consumed");
		ALPHABET[calls.get()]
	};
	match kind % 10 {
		0 => t.extend((0..n).map(f)),
		1 => t = (0..=n).chain(0..n).map(f).collect(),
		2 => t.extend(
			(0..u64::from(n) << 8 | u64::from(m))
				.map(|i| f(i as u32))
				.skip(m as usize),
		),
		3 => t.extend((0..n).map(f).take(m as usize)),
		// Only twice the longest is past `isize::MAX` stepped by two.
		4 => t.extend(
			(0..n)
				.step_by(if n == u32::MAX { 1 + m as usize % 2 } else { 1 })
				.map(f),
		),
		5 => match m % 2 {
			0 => t.extend((0..n).map(f).chain((0..m).map(f))),
			_ => t.extend((0..n).zip(0..m).map(|(i, _)| f(i))),
		},
		6 => t.extend((0..n).flat_map(|i| [f(i), f(i)])),
		// Sources whose hint is a few chars, which make it `n` only if each counts its own.
		7 => t.extend(
			(0..n.saturating_sub(5))
				.map(f)
				.chain(std::iter::once('x'))
				.chain(Some('y'))
				.chain("é😀".chars())
				.chain(['z'].iter().copied()),
		),
		// Integers past 2^53.
		8 => match sub % 3 {
			0 => t.extend((1u64 << 60..(1u64 << 60) + u64::from(n)).map(|_| f(0))),
			1 => t.extend((-(1i128 << 100)..-(1i128 << 100) + i128::from(n)).map(|_| f(0))),
			_ => t.extend((1u64 << 60..=(1u64 << 60) + u64::from(n)).map(|_| f(0))),
		},
		_ => {
			let mut chars = (0..n).map(f).enumerate().map(|(_, c)| c).rev().fuse().peekable();
			put_char(out, chars.peek().copied());
			t.extend(chars);
		}
	}
	put(out, &t);
}

/// Text with a char of each UTF-8 width between ASCII letters, so a part's ends land on
/// both sides of each.
const GOTTEN: &str = "a\u{e9}b\u{800}c\u{1f600}d";

/// A part a checked read gives, or 0xfe if it gave none.
fn put_part(out: &mut Vec<u8>, part: Option<&str>) {
	match part {
		Some(part) => put(out, part),
		None => out.push(0xfe),
	}
}

/// A part a checked `&mut` read gives, upper-cased and put, or 0xfe if it gave none.
fn put_upper(out: &mut Vec<u8>, part: Option<&mut str>) {
	match part {
		Some(part) => {
			part.make_ascii_uppercase();
			put(out, part);
		}
		None => out.push(0xfe),
	}
}

/// `a..=b` searched to its end, which leaves it at `b` but exhausted: `b + 1..b + 1` as an
/// index. Skipped to `b` first, so a search to `usize::MAX` is short.
fn exhausted(a: usize, b: usize) -> std::ops::RangeInclusive<usize> {
	let mut r = a..=b;
	if b > a {
		r.nth(b - a - 1);
	}
	r.find(|_| false);
	r
}

/// `str::get` and `get_mut` by every kind of range, and a `str` or a `String` indexed by an
/// inclusive range or a pair of `Bound`s, shared and `&mut`, at byte offsets `data` picks:
/// up to two past the end, or `usize::MAX` for 0xff. A checked read out of bounds, backwards,
/// or inside a char gives `None`, and indexing panics as std's does.
pub fn gets(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let text = match kind & 0x10 {
		0 => GOTTEN,
		_ => "",
	};
	let at = |i: usize| match data.get(i) {
		Some(0xff) => usize::MAX,
		Some(&b) => usize::from(b) % (text.len() + 3),
		None => 0,
	};
	let (a, b) = (at(0), at(1));
	let s: &str = text;
	let mut string = text.to_string();
	match kind % 16 {
		0 => {
			put_part(out, s.get(a..b));
			put_part(out, s.get(a..));
			put_part(out, s.get(..b));
			put_part(out, s.get(..));
		}
		1 => {
			put_part(out, s.get(a..=b));
			put_part(out, s.get(..=b));
			put_part(out, string.get(exhausted(a, b)));
		}
		2 => {
			put_part(out, s.get((Bound::Excluded(a), Bound::Included(b))));
			put_part(out, s.get((Bound::Included(a), Bound::Excluded(b))));
			put_part(out, s.get((Bound::Excluded(a), Bound::Unbounded)));
			put_part(out, s.get((Bound::Unbounded, Bound::Included(b))));
		}
		3 => put(out, &s[a..=b]),
		4 => put(out, &s[..=b]),
		5 => put(out, &string[a..=b]),
		6 => put(out, &string[..=b]),
		7 => put(out, &s[exhausted(a, b)]),
		8 => put(out, &s[(Bound::Excluded(a), Bound::Included(b))]),
		9 => put(out, &string[(Bound::Included(a), Bound::Excluded(b))]),
		10 => {
			put_upper(out, string.get_mut(a..=b));
			put_upper(out, string.as_mut_str().get_mut(..=b));
			put_upper(out, string.get_mut(exhausted(a, b)));
			put_upper(out, string.get_mut((Bound::Excluded(a), Bound::Included(b))));
			put(out, &string);
		}
		11 => {
			string[a..=b].make_ascii_uppercase();
			put(out, &string);
		}
		12 => {
			string.as_mut_str()[..=b].make_ascii_uppercase();
			put(out, &string);
		}
		13 => {
			string[exhausted(a, b)].make_ascii_uppercase();
			put(out, &string);
		}
		14 => {
			string.as_mut_str()[(Bound::Excluded(a), Bound::Excluded(b))].make_ascii_uppercase();
			put(out, &string);
		}
		_ => {
			// A part of a part, each end inclusive.
			let part = &mut string[..=b];
			let last = part.len().saturating_sub(1);
			part[a.min(last)..=last].make_ascii_uppercase();
			put(out, &string);
		}
	}
}
