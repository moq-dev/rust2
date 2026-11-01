//! The ASCII methods of `u8`, `char`, byte slices, and `str`: their tests, case maps, and
//! case-insensitive equality, each case changed in place through a `&mut` to where the
//! value is, slices trimmed, and what `escape_ascii` yields from either end and writes.
//! Every byte is tried, and the chars at each edge of ASCII and past it. A `String`'s case
//! also changes through a `&mut str` to it wherever it is, and to parts of it.

use std::ascii;
use std::borrow::BorrowMut;
use std::fmt::Write;

use crate::fmt::put;

/// Chars either side of each class's ends, of ASCII's and Latin-1's, and of the surrogates
/// that are no char, a Latin-1 letter JS's case maps change, and those whose Unicode case
/// maps to or from ASCII: the long s, the Kelvin sign, and the dotted capital I.
const CHARS: [char; 41] = [
	'\0',
	'\t',
	'\n',
	'\u{b}',
	'\u{c}',
	'\r',
	'\u{1f}',
	' ',
	'!',
	'/',
	'0',
	'7',
	'8',
	'9',
	':',
	'@',
	'A',
	'F',
	'G',
	'Z',
	'[',
	'`',
	'a',
	'f',
	'g',
	'z',
	'{',
	'~',
	'\u{7f}',
	'\u{80}',
	'\u{c0}',
	'\u{ff}',
	'\u{100}',
	'\u{17f}',
	'\u{130}',
	'\u{212a}',
	'\u{d7ff}',
	'\u{e000}',
	'\u{ffff}',
	'\u{10000}',
	'\u{10ffff}',
];

/// Each test of `b`, a bit apiece.
fn byte_tests(b: u8) -> u16 {
	[
		b.is_ascii(),
		b.is_ascii_alphabetic(),
		b.is_ascii_uppercase(),
		b.is_ascii_lowercase(),
		b.is_ascii_alphanumeric(),
		b.is_ascii_digit(),
		b.is_ascii_hexdigit(),
		b.is_ascii_punctuation(),
		b.is_ascii_graphic(),
		b.is_ascii_whitespace(),
		b.is_ascii_control(),
	]
	.iter()
	.enumerate()
	.fold(0, |bits, (i, &t)| bits | (u16::from(t) << i))
}

/// Each test of `c`, a bit apiece.
fn char_tests(c: char) -> u16 {
	[
		c.is_ascii(),
		c.is_ascii_alphabetic(),
		c.is_ascii_uppercase(),
		c.is_ascii_lowercase(),
		c.is_ascii_alphanumeric(),
		c.is_ascii_digit(),
		c.is_ascii_hexdigit(),
		c.is_ascii_punctuation(),
		c.is_ascii_graphic(),
		c.is_ascii_whitespace(),
		c.is_ascii_control(),
	]
	.iter()
	.enumerate()
	.fold(0, |bits, (i, &t)| bits | (u16::from(t) << i))
}

fn upper_byte(b: &mut u8) {
	b.make_ascii_uppercase();
}

fn lower_char(c: &mut char) {
	c.make_ascii_lowercase();
}

/// A struct whose fields change case where they are.
struct Held {
	n: u8,
	text: String,
}

fn upper_text(s: &mut String) {
	s.make_ascii_uppercase();
}

/// Read `e` alternately from the front and the back as `data`'s bits say, putting its
/// length before each step, then what `Display` writes of what is left.
fn steps<I: DoubleEndedIterator<Item = u8> + ExactSizeIterator + Clone + std::fmt::Display>(
	mut e: I,
	data: &[u8],
	out: &mut Vec<u8>,
) {
	let mut i = 0;
	loop {
		out.push(e.len() as u8);
		let (lo, hi) = e.size_hint();
		out.push(lo as u8);
		out.push(hi.map_or(0xff, |h| h as u8));
		let back = data.get(i % data.len().max(1)).is_some_and(|b| b >> (i % 8) & 1 == 1);
		let next = if back { e.next_back() } else { e.next() };
		let Some(b) = next else {
			break;
		};
		out.push(b);
		let mut shown = String::new();
		write!(shown, "{e}").unwrap();
		put(out, &shown);
		i += 1;
	}
}

/// Every byte, through each method of `u8`.
pub fn bytes(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let mut all: Vec<u8> = (0..=255).collect();
	match kind % 4 {
		0 => {
			for b in 0..=255u8 {
				out.extend_from_slice(&byte_tests(b).to_be_bytes());
				out.push(b.to_ascii_uppercase());
				out.push(b.to_ascii_lowercase());
				let other = data.first().copied().unwrap_or(b ^ 0x20);
				out.push(b.eq_ignore_ascii_case(&other) as u8);
				out.push(b.eq_ignore_ascii_case(&(b ^ 0x20)) as u8);
			}
		}
		1 => {
			// In place: a local, a `Vec`'s element, a tuple's field, each element as
			// `iter_mut` lends it, and through a `&mut` a function is given.
			for b in 0..=255u8 {
				let mut local = b;
				local.make_ascii_uppercase();
				out.push(local);
				let mut pair = (b, b);
				pair.1.make_ascii_lowercase();
				out.extend_from_slice(&[pair.0, pair.1]);
			}
			for i in 0..all.len() {
				if i % 2 == 0 {
					all[i].make_ascii_lowercase();
				} else {
					upper_byte(&mut all[i]);
				}
			}
			for b in all.iter_mut().skip(usize::from(data.first().copied().unwrap_or(0))) {
				b.make_ascii_uppercase();
			}
			out.extend_from_slice(&all);
		}
		2 => {
			for b in 0..=255u8 {
				let e = b.escape_ascii();
				let mut shown = String::new();
				write!(shown, "{e}|{:>6}|{}", ascii::escape_default(b), e.clone().count()).unwrap();
				put(out, &shown);
				out.extend(e.clone());
				out.extend(e.clone().rev());
				out.push(e.clone().last().unwrap());
				out.push(e.to_string().len() as u8);
			}
		}
		_ => {
			for b in 0..=255u8 {
				steps(b.escape_ascii(), data, out);
			}
		}
	}
}

/// The chars at each edge, then each of `data`'s big-endian `u32`s that is one, through
/// each method of `char`.
pub fn chars(_: u8, data: &[u8], out: &mut Vec<u8>) {
	let mut all = CHARS.to_vec();
	all.extend(data.chunks(4).filter_map(|c| {
		char::from_u32(u32::from_be_bytes(core::array::from_fn(|i| {
			c.get(i).copied().unwrap_or(0)
		})))
	}));
	let others = all.clone();
	for (i, &c) in all.iter().enumerate() {
		out.extend_from_slice(&char_tests(c).to_be_bytes());
		out.extend_from_slice(&u32::from(c.to_ascii_uppercase()).to_be_bytes());
		out.extend_from_slice(&u32::from(c.to_ascii_lowercase()).to_be_bytes());
		let mut local = c;
		local.make_ascii_uppercase();
		out.extend_from_slice(&u32::from(local).to_be_bytes());
		let mut held = vec![c, c];
		lower_char(&mut held[1]);
		out.extend_from_slice(&u32::from(held[1]).to_be_bytes());
		for other in [others[(i + 1) % others.len()], local, held[1]] {
			out.push(c.eq_ignore_ascii_case(&other) as u8);
		}
	}
}

/// `data` as bytes and as text, through the ASCII methods of slices and `str`, and those
/// of a `Vec<u8>` and a `String` they deref to.
pub fn slices(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let half = data.len() / 2;
	let (a, b) = data.split_at(half);
	match kind % 4 {
		0 => {
			out.push(data.is_ascii() as u8);
			// Each as a function, as an adaptor takes it.
			out.push(data.iter().all(u8::is_ascii_hexdigit) as u8);
			out.extend(data.iter().map(u8::to_ascii_uppercase));
			out.push(a.eq_ignore_ascii_case(b) as u8);
			out.push(a.eq_ignore_ascii_case(&b.to_ascii_lowercase()) as u8);
			out.extend_from_slice(&data.to_ascii_uppercase());
			out.extend_from_slice(&data.to_ascii_lowercase());
			for t in [data.trim_ascii_start(), data.trim_ascii_end(), data.trim_ascii()] {
				out.push(t.len() as u8);
				out.extend_from_slice(t);
			}
			let mut v = data.to_vec();
			v[half..].make_ascii_uppercase();
			out.extend_from_slice(&v);
			v.make_ascii_lowercase();
			out.extend_from_slice(&v);
			let mut array = [0u8; 4];
			array.copy_from_slice(
				&data[..4.min(data.len())]
					.iter()
					.chain([b'a'; 4].iter())
					.copied()
					.take(4)
					.collect::<Vec<u8>>(),
			);
			array.make_ascii_uppercase();
			out.extend_from_slice(&array);
		}
		1 => {
			let mut s = String::new();
			for &b in data {
				s.push(CHARS[usize::from(b) % CHARS.len()]);
			}
			let mid = (0..=s.len() / 2).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0);
			let (x, y) = (&s[..mid], &s[mid..]);
			out.push(s.is_ascii() as u8);
			out.push(x.eq_ignore_ascii_case(y) as u8);
			out.push(x.eq_ignore_ascii_case(&y.to_ascii_uppercase()) as u8);
			put(out, &s.to_ascii_uppercase());
			put(out, &s.chars().filter(char::is_ascii_alphanumeric).collect::<String>());
			put(out, &s.to_ascii_lowercase());
			put(out, s.trim_ascii_start());
			put(out, s.trim_ascii_end());
			put(out, s.trim_ascii());
			let mut t = s.clone();
			t.make_ascii_uppercase();
			put(out, &t);
			let held = &mut t;
			held.make_ascii_lowercase();
			put(out, &t);
			let mut held = Held { n: 0, text: t };
			upper_text(&mut held.text);
			put(out, &held.text);
			held.text.make_ascii_lowercase();
			put(out, &held.text);
			held.n.make_ascii_uppercase();
			out.push(held.n);
		}
		2 => {
			let e = data.escape_ascii();
			put(out, &e.to_string());
			put(out, &format!("{e:>40}|{}", e.clone().count()));
			out.extend(e.clone().rev());
			out.push(e.clone().last().unwrap_or(0xff));
		}
		_ => {
			let mut e = data.escape_ascii();
			let mut i = 0;
			loop {
				let (lo, hi) = e.size_hint();
				out.extend_from_slice(&(lo as u32).to_be_bytes());
				out.extend_from_slice(&hi.map_or(u32::MAX, |h| h as u32).to_be_bytes());
				let next = if data.get(i).is_some_and(|b| b & 1 == 1) {
					e.next_back()
				} else {
					e.next()
				};
				let Some(b) = next else {
					break;
				};
				out.push(b);
				put(out, &e.to_string());
				out.extend(e.clone());
				i += 1;
			}
		}
	}
}

/// ASCII letters between chars of each wider UTF-8 length, so a case change shows at
/// each side of every part.
const MIXED: &str = "a\u{c9}b\u{800}c\u{1f600}dZ";

/// `s[a..b]`'s case changed through the `&mut str` it is given.
fn upper_part(s: &mut str, a: usize, b: usize) {
	s[a..b].make_ascii_uppercase();
}

/// A part `get_mut` or `split_at_mut_checked` may give, upper-cased and put, or 0xfe if
/// it gave none.
fn put_upper(out: &mut Vec<u8>, part: Option<&mut str>) {
	match part {
		Some(part) => {
			part.make_ascii_uppercase();
			put(out, part);
		}
		None => out.push(0xfe),
	}
}

/// Case changed through a `&mut str` to a `String` where a local, a tuple, an array, a
/// `Vec`, or an `Option` holds it, and to parts of one at byte offsets `data` picks up to
/// two past the end, the `String` put after each change. A part off a char boundary,
/// backwards, or out of bounds panics as std's does; a checked one gives `None`.
pub fn parts(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let text = match kind & 0x10 {
		0 => MIXED,
		_ => "",
	};
	let at = |i: usize| data.get(i).map_or(0, |&b| usize::from(b) % (text.len() + 3));
	let (a, b) = (at(0), at(1));
	let mut s = text.to_string();
	let mut pair = (7u8, text.to_string());
	let mut array = [String::from("q"), text.to_string()];
	let mut v = vec![text.to_string(), text.to_string()];
	match kind % 8 {
		0 => {
			s.as_mut_str().make_ascii_uppercase();
			put(out, &s);
			s[a..b].make_ascii_lowercase();
			put(out, &s);
		}
		1 => {
			pair.1.make_ascii_uppercase();
			put(out, &pair.1);
			pair.1[a..].make_ascii_lowercase();
			put(out, &pair.1);
			out.push(pair.0);
		}
		2 => {
			array[1].make_ascii_uppercase();
			put(out, &array[1]);
			array[1][..b].make_ascii_lowercase();
			put(out, &array[1]);
			put(out, &array[0]);
		}
		3 => {
			for t in v.iter_mut() {
				t.make_ascii_uppercase();
			}
			AsMut::<str>::as_mut(&mut v[0])[..].make_ascii_lowercase();
			v[1][a..b].make_ascii_lowercase();
			put(out, &v[0]);
			put(out, &v[1]);
		}
		4 => {
			// Two parts at once, each written after the other's write.
			let (x, y) = s.split_at_mut(a);
			x.make_ascii_uppercase();
			y.make_ascii_uppercase();
			x.make_ascii_lowercase();
			put(out, x);
			put(out, y);
			let (y0, y1) = y.split_at_mut(b.saturating_sub(a));
			y1.make_ascii_lowercase();
			put(out, y0);
			put(out, y1);
			put(out, &s);
		}
		5 => {
			put_upper(out, s.get_mut(a..b));
			put_upper(out, pair.1.get_mut(a..));
			put_upper(out, v[1].get_mut(..b));
			put(out, &s);
			put(out, &pair.1);
			put(out, &v[1]);
		}
		6 => {
			match BorrowMut::<str>::borrow_mut(&mut array[1]).split_at_mut_checked(a) {
				Some((x, y)) => {
					y.make_ascii_uppercase();
					put_upper(out, x.get_mut(b.min(x.len())..));
				}
				None => out.push(0xfd),
			}
			put(out, &array[1]);
			let mut held = Some(text.to_string());
			if let Some(t) = held.as_mut() {
				t.make_ascii_uppercase();
			}
			put(out, held.as_deref().unwrap_or("-"));
		}
		_ => {
			// A part of a part, through a `&mut str` given.
			upper_part(&mut v[1][a..], 0, b.saturating_sub(a));
			put(out, &v[1]);
			upper_part(&mut s, a, b);
			put(out, &String::from(&mut s[..a]));
			put(out, &(&mut s[a..]).to_string());
			let leaked = pair.1.leak();
			leaked[b.min(a)..].make_ascii_uppercase();
			put(out, leaked);
			let empty: &mut str = Default::default();
			empty.make_ascii_uppercase();
			put(out, empty);
		}
	}
}
