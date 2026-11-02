//! `Box<[T]>` and `Box<str>`: made by `From` of a slice, a `Vec`, an array, and a `Cow`,
//! by `collect`, `Vec::into_boxed_slice`, `Default`, `Clone`, and coercing a boxed
//! array, read and written through `Deref`, compared, hashed, and printed as their
//! slice is, and turned back by `into_vec` and `into_iter`, of bytes and of other values,
//! empty or not, with what drops dropped in std's order. Then writes reaching what a box
//! of a value JS copies holds.

use std::borrow::Cow;
use std::cell::RefCell;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Deref;
use std::rc::Rc;

use crate::fmt::put;
use crate::strs::text;

/// Logs its id as it drops, and as it is cloned, the id with the high bit set: its clone's
/// id is one more.
struct Logged {
	id: u8,
	log: Rc<RefCell<Vec<u8>>>,
}

impl Clone for Logged {
	fn clone(&self) -> Self {
		self.log.borrow_mut().push(0x80 | self.id);
		Self {
			id: self.id.wrapping_add(1),
			log: self.log.clone(),
		}
	}
}

impl Drop for Logged {
	fn drop(&mut self) {
		self.log.borrow_mut().push(self.id);
	}
}

/// A boxed slice in a field, beside one that drops after it.
struct Held {
	items: Box<[Logged]>,
	last: Logged,
}

/// Puts what was logged since the last flush, after a marker.
fn flush(log: &RefCell<Vec<u8>>, out: &mut Vec<u8>) {
	out.push(0xfd);
	out.extend(log.borrow_mut().drain(..));
}

fn hash<T: Hash + ?Sized>(value: &T) -> u64 {
	let mut h = DefaultHasher::new();
	value.hash(&mut h);
	h.finish()
}

/// Puts `b`'s length and bytes.
fn bytes(b: &[u8], out: &mut Vec<u8>) {
	out.push(b.len() as u8);
	out.extend_from_slice(b);
}

/// Puts `b`'s length and each `u16`'s bytes.
fn wides(b: &[u16], out: &mut Vec<u8>) {
	out.push(b.len() as u8);
	for x in b {
		out.extend(x.to_le_bytes());
	}
}

/// Puts the ids of `b`.
fn ids(b: &[Logged], out: &mut Vec<u8>) {
	out.push(b.len() as u8);
	out.extend(b.iter().map(|l| l.id));
}

/// Writes through `DerefMut` of a boxed slice of bytes.
fn scramble(b: &mut Box<[u8]>) {
	b.reverse();
	if let Some(first) = b.first_mut() {
		*first ^= 0xff;
	}
	for x in b.iter_mut() {
		*x = x.wrapping_add(1);
	}
	b.chunks_exact_mut(2).for_each(|c| c.swap(0, 1));
	let n = b.len();
	if n > 2 {
		b[n - 1] = 0x42;
		b[..2].copy_from_slice(&[7, 8]);
	}
}

/// A whole boxed slice written through a `&mut`, of another length.
fn set<T>(slot: &mut Box<[T]>, value: Box<[T]>) {
	*slot = value;
}

pub fn boxes(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	// As many bytes as `kind`'s high bits pick, none included.
	let data = &data[..usize::from(kind >> 3) % (data.len() + 1)];
	match kind % 6 {
		// Bytes, made each way, each written through `DerefMut` apart from what it came from.
		0 => {
			let mut source = data.to_vec();
			let mut from_ref: Box<[u8]> = Box::from(data);
			scramble(&mut from_ref);
			bytes(&from_ref, out);
			let mut from_mut: Box<[u8]> = Box::from(&mut source[..]);
			from_mut.sort();
			bytes(&from_mut, out);
			bytes(&source, out);
			let into: Box<[u8]> = data.into();
			bytes(&into, out);
			let mut spare = Vec::with_capacity(64);
			spare.extend_from_slice(data);
			let mut from_vec = Box::<[u8]>::from(spare);
			scramble(&mut from_vec);
			bytes(&from_vec, out);
			let mut boxed = source.clone().into_boxed_slice();
			scramble(&mut boxed);
			bytes(&boxed, out);
			let array = [data.first().copied().unwrap_or(1), 2, 3, 4];
			let mut from_array = Box::<[u8]>::from(array);
			scramble(&mut from_array);
			bytes(&from_array, out);
			bytes(&array, out);
			let mut coerced: Box<[u8]> = Box::new(array);
			coerced[0] = 9;
			bytes(&coerced, out);
			bytes(&array, out);
			let collected: Box<[u8]> = data.iter().map(|&x| x ^ 0x55).collect();
			bytes(&collected, out);
			let mut cloned = collected.clone();
			scramble(&mut cloned);
			bytes(&cloned, out);
			bytes(&collected, out);
			let empty: Box<[u8]> = Default::default();
			bytes(&empty, out);
			bytes(&empty.clone(), out);
			let borrowed: Box<[u8]> = Box::from(Cow::Borrowed(data));
			bytes(&borrowed, out);
			let owned: Box<[u8]> = Box::from(Cow::<[u8]>::Owned(data.to_vec()));
			bytes(&owned, out);
			// Back to a `Vec`, which grows as any does.
			let mut v = from_ref.into_vec();
			v.push(0xee);
			v.extend_from_slice(&from_vec);
			bytes(&v, out);
			let v: Vec<u8> = Vec::from(cloned);
			bytes(&v, out);
			// Replaced whole through a `&mut`, by one of another length.
			let mut slot: Box<[u8]> = Box::from(data);
			set(&mut slot, Box::from([1, 2, 3]));
			bytes(&slot, out);
			let old = std::mem::replace(&mut slot, Box::from(data));
			bytes(&old, out);
			bytes(&std::mem::take(&mut slot), out);
			bytes(&slot, out);
			let mut other: Box<[u8]> = Box::from([9]);
			std::mem::swap(&mut slot, &mut other);
			bytes(&slot, out);
			bytes(&other, out);
			let mut sum = 0u8;
			for x in boxed {
				sum = sum.wrapping_add(x);
			}
			out.push(sum);
		}
		// Bytes compared, ordered, hashed, and printed as their slice is.
		1 => {
			let a: Box<[u8]> = data.into();
			let half = &data[..data.len() / 2];
			let b: Box<[u8]> = half.into();
			out.push(u8::from(a == b));
			out.push(u8::from(a != b));
			out.push(u8::from(a < b));
			out.push(u8::from(a >= b));
			out.push(a.cmp(&b) as u8);
			out.push(b.partial_cmp(&a).map_or(9, |o| o as u8));
			out.push(u8::from(*a == *data));
			out.push(u8::from(hash(&a) == hash(data)));
			out.extend(hash(&b).to_le_bytes());
			out.push(a.len() as u8);
			out.push(u8::from(a.is_empty()));
			out.push(a.iter().copied().fold(0, u8::wrapping_add));
			out.push(a.contains(&0) as u8);
			out.push(a.first().copied().unwrap_or(0xaa));
			put(out, &format!("{a:?}|{b:?}|{:?}", Box::<[u8]>::default()));
		}
		// `u16`s, each way, written, compared, and turned back.
		2 => {
			let wide: Vec<u16> = data.iter().map(|&x| u16::from(x) * 257).collect();
			let mut from_ref: Box<[u16]> = Box::from(&wide[..]);
			from_ref.reverse();
			for x in from_ref.iter_mut() {
				*x ^= 0x0f0f;
			}
			wides(&from_ref, out);
			wides(&wide, out);
			let mut from_vec = Box::<[u16]>::from(wide.clone());
			if let Some(last) = from_vec.last_mut() {
				*last = 0xbeef;
			}
			wides(&from_vec, out);
			let array = [1u16, 2, 0x300];
			let mut from_array = Box::<[u16]>::from(array);
			from_array[2] = 7;
			wides(&from_array, out);
			wides(&array, out);
			let collected: Box<[u16]> = wide.iter().map(|&x| x.rotate_left(3)).collect();
			let mut cloned = collected.clone();
			cloned.sort_unstable();
			wides(&cloned, out);
			wides(&collected, out);
			let empty: Box<[u16]> = Default::default();
			wides(&empty, out);
			out.push(u8::from(collected == cloned));
			out.push(collected.cmp(&cloned) as u8);
			out.push(u8::from(hash(&collected) == hash(&collected[..])));
			put(out, &format!("{collected:?}"));
			let mut v = from_vec.into_vec();
			v.push(5);
			wides(&v, out);
			let mut slot = from_array;
			set(&mut slot, collected.clone());
			wides(&slot, out);
			wides(&std::mem::take(&mut slot), out);
			let mut it = wide.into_boxed_slice().into_iter();
			out.extend(it.next_back().unwrap_or(0).to_le_bytes());
			out.push(it.len() as u8);
			out.extend(it.map(|x| x as u8));
		}
		// Items that log their clones and drops: each boxed slice drops its items in order,
		// as it is dropped, overwritten, and as its fields' owner drops, and its `IntoIterator`
		// drops those it still holds.
		3 => {
			let log = Rc::new(RefCell::new(Vec::new()));
			let logged = |id: u8| Logged { id, log: log.clone() };
			let items: Vec<Logged> = data.iter().map(|&b| logged(b & 0x7e)).collect();
			let boxed = items.into_boxed_slice();
			ids(&boxed, out);
			let cloned = boxed.clone();
			flush(&log, out);
			drop(boxed);
			flush(&log, out);
			let from_ref: Box<[Logged]> = Box::from(&cloned[..]);
			flush(&log, out);
			let mut collected: Box<[Logged]> = data.iter().map(|&b| logged(b | 1)).collect();
			ids(&collected, out);
			collected = Box::from([logged(0x10), logged(0x12)]);
			flush(&log, out);
			collected[0] = logged(0x14);
			flush(&log, out);
			let mut it = cloned.into_iter();
			if let Some(first) = it.next() {
				out.push(first.id);
			}
			let back = it.next_back();
			flush(&log, out);
			drop(it);
			flush(&log, out);
			drop(back);
			let v = from_ref.into_vec();
			ids(&v, out);
			drop(v);
			flush(&log, out);
			let held = Held {
				items: Box::new([logged(0x20), logged(0x22)]),
				last: logged(0x24),
			};
			out.push(held.items.len() as u8);
			out.push(held.last.id);
			drop(held);
			flush(&log, out);
			let empty: Box<[Logged]> = Box::default();
			ids(&empty.clone(), out);
			set(&mut collected, Box::from([logged(0x30)]));
			flush(&log, out);
			drop(collected);
			flush(&log, out);
		}
		// Values JS copies, held in a boxed slice apart from what they came from.
		4 => {
			let pairs: Vec<(u8, u16)> = data.iter().map(|&b| (b, u16::from(b) << 4)).collect();
			let mut boxed: Box<[(u8, u16)]> = Box::from(&pairs[..]);
			for p in boxed.iter_mut() {
				p.0 ^= 0xff;
			}
			if let Some(p) = boxed.first_mut() {
				p.1 = 1;
			}
			let array = [(1u8, 2u16), (3, 4)];
			let mut from_array = Box::<[(u8, u16)]>::from(array);
			from_array[0].0 = 9;
			let quads: Box<[[u8; 2]]> = data.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
			let mut quads_v = quads.clone().into_vec();
			if let Some(q) = quads_v.first_mut() {
				q[0] = 0x77;
			}
			for (a, b) in boxed
				.iter()
				.chain(pairs.iter())
				.chain(from_array.iter())
				.chain(array.iter())
			{
				out.push(*a);
				out.extend(b.to_le_bytes());
			}
			for q in quads.iter().chain(quads_v.iter()) {
				out.extend_from_slice(q);
			}
			out.push(u8::from(boxed[..] == pairs[..]));
			// One value twice, each its own once boxed.
			let p = (data.first().copied().unwrap_or(5), 6u16);
			let mut twins = Box::<[(u8, u16)]>::from([p, p]);
			twins[0].0 ^= 1;
			let mut coerced: Box<[(u8, u16)]> = Box::new([p, p]);
			coerced[1].1 = 0;
			for (a, b) in twins.iter().chain(coerced.iter()).chain([p].iter()) {
				out.push(*a);
				out.extend(b.to_le_bytes());
			}
		}
		// `Box<str>`, made from text and a `String`, and turned back.
		_ => {
			let s = text(data);
			let from_ref: Box<str> = Box::from(s.as_str());
			let from_string: Box<str> = Box::from(s.clone());
			let into: Box<str> = s.clone().into_boxed_str();
			let collected: Box<str> = s.chars().rev().collect();
			let empty: Box<str> = Box::default();
			put(out, &from_ref);
			put(out, &from_string.clone());
			put(out, &into);
			put(out, &collected);
			put(out, &empty);
			out.push(u8::from(from_ref == into));
			out.push(collected.cmp(&from_ref) as u8);
			out.push(u8::from(hash(&from_ref) == hash(s.as_str())));
			put(out, &format!("{from_ref}|{collected:?}"));
			let mut back = into.into_string();
			back.push('!');
			put(out, &back);
			put(out, &String::from(collected));
		}
	}
}

/// Boxes of values JS copies, held in a struct a `&mut` reaches.
struct Owner {
	n: Box<u32>,
	s: Box<String>,
	nested: Box<Box<u32>>,
}

/// Adds `by` to what `x` borrows.
fn bump(x: &mut u32, by: u8) {
	*x = x.wrapping_add(u32::from(by));
}

/// Writes through a `&mut` to what a `Box<str>` holds, and to a part of it.
fn shout(s: &mut str, by: u8) {
	let half = s.len() / 2;
	match by % 2 {
		0 => s.make_ascii_uppercase(),
		_ => s[..half].make_ascii_lowercase(),
	}
}

/// A box taken by value and written through before it is given back.
fn took(mut b: Box<u32>, by: u8) -> Box<u32> {
	bump(&mut *b, by);
	b
}

/// A write through a pointer into what `a` holds, made before the reference holding `a`
/// is aimed at `b`, still reaches `a`.
fn reaimed(a: &mut Box<u32>, b: &mut Box<u32>, by: u8) {
	use std::ops::DerefMut;
	let mut r = a;
	let p = match by % 2 {
		0 => &mut **r,
		_ => r.deref_mut(),
	};
	r = b;
	*p ^= u32::from(by);
	**r = r.wrapping_add(1);
}

/// What `&mut Owner` reaches of its boxes, each written by `by`.
fn reach(b: &mut Owner, by: u8) {
	match by % 4 {
		0 => bump(&mut *b.n, by),
		1 => *b.n ^= u32::from(by),
		2 => bump(&mut **b.nested, by),
		_ => b.s.push(char::from(b'a' + by % 26)),
	}
}

/// Writes reaching what boxes hold, each step by a byte, kept in a local, reassigned,
/// nested, in a field, a `Vec`, an `Option`, and a `Result`, captured, passed, and leaked: through
/// `&mut *b`, `DerefMut`, `AsMut`, `BorrowMut`, `as_deref_mut`, and a write through the box
/// itself.
pub fn places(_: u8, data: &[u8], out: &mut Vec<u8>) {
	use std::borrow::BorrowMut;
	use std::ops::DerefMut;
	let mut n = Box::new(7u32);
	let mut pair = Box::new([1u8, 2]);
	let mut s = Box::new(String::from("b"));
	let mut t: Box<str> = Box::from("aBc/D");
	let mut nested = Box::new(Box::new(3u32));
	let mut maybe = Box::new(None::<u32>);
	let mut held = Owner {
		n: Box::new(5),
		s: Box::new(String::new()),
		nested: Box::new(Box::new(9)),
	};
	let mut v = vec![Box::new(1u32), Box::new(2)];
	let mut o = Some(Box::new(4u32));
	let mut res: Result<Box<u32>, u8> = Ok(Box::new(2));
	let mut said: Result<Box<String>, u8> = Ok(Box::new(String::from("r")));
	let mut words: Box<[u32]> = Box::from([6, 8]);
	let mut some_words: Option<Box<[u32]>> = Some(Box::from([3, 5]));
	let mut named: Option<Box<str>> = Some(Box::from("xY/z"));
	let leaked = Box::leak(Box::new(11u32));
	let mut deep: Option<Box<Option<u32>>> = Some(Box::new(Some(1)));
	let mut tup: Result<Box<(u32, u32)>, u8> = Ok(Box::new((1, 2)));
	let mut boxed_opt: Box<Option<u32>> = Box::new(Some(3));
	for (i, &by) in data.iter().enumerate() {
		match by % 26 {
			0 => {
				let r = &mut *n;
				*r = r.wrapping_mul(3);
			}
			1 => bump(&mut *n, by),
			2 => bump(n.as_mut(), by),
			3 => bump(n.deref_mut(), by),
			4 => bump(n.borrow_mut(), by),
			5 => *n ^= u32::from(by),
			6 => n = Box::new(u32::from(by).wrapping_add(*n)),
			7 => {
				let r = &mut *pair;
				r[usize::from(by) % 2] ^= by;
			}
			8 => s.push(char::from(b'a' + by % 26)),
			9 => (&mut *s).push_str("!?"),
			10 => s.as_mut().insert(0, '<'),
			11 => shout(&mut *t, by),
			12 => shout(t.as_mut(), by),
			13 => BorrowMut::<str>::borrow_mut(&mut t).make_ascii_lowercase(),
			14 => t = Box::from(&s[..]),
			15 => bump(&mut **nested, by),
			16 => **nested = nested.wrapping_add(u32::from(by)),
			17 => {
				*maybe.get_or_insert(u32::from(by)) += 1;
				if let Some(x) = &mut *maybe {
					bump(x, by);
				}
			}
			18 => reach(&mut held, by),
			19 => {
				let at = usize::from(by) % v.len();
				bump(&mut *v[at], by);
				if let Some(last) = v.last_mut() {
					**last ^= 0x10;
				}
			}
			20 => {
				for b in v.iter_mut() {
					bump(b, by);
				}
				v.push(Box::new(u32::from(by)));
			}
			21 => {
				if let Some(b) = &mut o {
					**b = b.wrapping_add(u32::from(by));
				}
				if let Some(r) = o.as_deref_mut() {
					bump(r, by);
				}
				if i % 3 == 0 {
					o = o.is_none().then(|| Box::new(u32::from(by)));
				}
				if let Ok(b) = &mut res {
					**b = b.wrapping_mul(u32::from(by));
				}
				if let Ok(s) = &mut said {
					s.push(char::from(b'a' + by % 26));
				}
				if i % 5 == 4 {
					said = Err(by);
				}
			}
			22 => {
				let r = &mut *words;
				r[usize::from(by) % 2] += u32::from(by);
				if let Some(w) = some_words.as_deref_mut() {
					w[1] ^= u32::from(by);
				}
				if let Some(t) = named.as_deref_mut() {
					shout(t, by);
				}
				if let Some(Some(x)) = deep.as_deref_mut() {
					*x += u32::from(by);
				}
				if let Some(d) = deep.as_deref_mut() {
					if by % 4 == 0 {
						*d = None;
					} else if d.is_none() {
						*d = Some(9);
					}
				}
				if let Ok(b) = &mut tup {
					b.0 += u32::from(by);
					bump(&mut b.1, by);
				}
				if let Some(x) = boxed_opt.as_mut().as_mut() {
					bump(x, by);
				}
				if let Some(x) = &mut *boxed_opt {
					*x += 1;
				}
			}
			23 => {
				let mut f = |by| bump(&mut *n, by);
				f(by);
				f(1);
			}
			24 => {
				n = took(n, by);
				reaimed(&mut n, &mut held.n, by);
			}
			_ => bump(leaked, by),
		}
		out.extend(n.to_le_bytes());
		out.extend_from_slice(&*pair);
		put(out, &s);
		put(out, &t);
		out.extend(nested.to_le_bytes());
		out.extend(maybe.unwrap_or(0xee).to_le_bytes());
		out.extend(held.n.to_le_bytes());
		put(out, &held.s);
		out.extend(held.nested.to_le_bytes());
		out.extend(v.iter().flat_map(|b| b.to_le_bytes()));
		out.extend(o.as_deref().copied().unwrap_or(0xee).to_le_bytes());
		out.extend(res.as_deref().copied().unwrap_or(0xee).to_le_bytes());
		match &said {
			Ok(s) => put(out, s),
			Err(e) => out.push(*e),
		}
		out.extend(words.iter().flat_map(|w| w.to_le_bytes()));
		out.extend(some_words.iter().flatten().flat_map(|w| w.to_le_bytes()));
		put(out, named.as_deref().unwrap_or("-"));
		out.extend(leaked.to_le_bytes());
		out.extend(deep.as_deref().copied().flatten().unwrap_or(0xee).to_le_bytes());
		out.extend(tup.as_deref().map(|t| t.0 ^ t.1).unwrap_or(0xee).to_le_bytes());
		out.extend(boxed_opt.unwrap_or(0xee).to_le_bytes());
	}
}

/// Clones `from` into `slot` by its `clone_from`.
fn refill<T: Clone>(slot: &mut T, from: &T) {
	slot.clone_from(from);
}

/// The sum of the bytes `x` lends by its `AsRef`.
fn sum<T: AsRef<[u8]> + ?Sized>(x: &T) -> u8 {
	x.as_ref().iter().fold(0, |a, &b| a.wrapping_add(b))
}

/// Writes through what `x` lends by its `AsMut`.
fn flip<T: AsMut<[u8]> + ?Sized>(x: &mut T) {
	for b in x.as_mut() {
		*b ^= 0x0f;
	}
}

/// Puts what boxing `r` into an array gave: its bytes, or a marker and what it gave back.
fn boxed<const N: usize, E: Deref<Target = [u8]>>(r: Result<Box<[u8; N]>, E>, out: &mut Vec<u8>) {
	match r {
		Ok(mut a) => {
			if let Some(first) = a.first_mut() {
				*first ^= 0xff;
			}
			bytes(&*a, out);
		}
		Err(e) => {
			out.push(0xee);
			bytes(&e, out);
		}
	}
}

/// The rest of a box's impls: `clone_from` of a boxed slice, into one of its length by
/// each item's `clone_from` and into one of another replaced whole; `AsRef` and `AsMut` of
/// a box of any value; `TryFrom` of a boxed slice and a `Vec` into a boxed array, of its
/// length or not; and a boxed text's bytes. Each of bytes and of other values, empty or
/// not, with what drops dropped in std's order.
pub fn impls(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let data = &data[..usize::from(kind >> 3) % (data.len() + 1)];
	let half = data.len() / 2;
	let log = Rc::new(RefCell::new(Vec::new()));
	let logged = |id: u8| Logged { id, log: log.clone() };
	match kind % 4 {
		0 => {
			let mut a: Box<[u8]> = Box::from(&data[..half]);
			let b: Box<[u8]> = Box::from(&data[half..half * 2]);
			a.clone_from(&b);
			if let Some(x) = a.first_mut() {
				*x ^= 0xff;
			}
			bytes(&a, out);
			bytes(&b, out);
			refill(&mut a, &Box::from(data));
			a.reverse();
			bytes(&a, out);
			let mut w: Box<[u16]> = data[half..].iter().map(|&x| u16::from(x) * 257).collect();
			let v: Box<[u16]> = data[..data.len() - half].iter().map(|&x| u16::from(x) << 3).collect();
			w.clone_from(&v);
			w.reverse();
			wides(&w, out);
			wides(&v, out);
			w.clone_from(&Box::from([1, 2, 3]));
			wides(&w, out);
			let mut p: Box<[(u8, u16)]> = data[..half].iter().map(|&x| (x, 7)).collect();
			let q: Box<[(u8, u16)]> = data[half..half * 2].iter().map(|&x| (x ^ 1, 9)).collect();
			p.clone_from(&q);
			if let Some(x) = p.first_mut() {
				x.0 = 0x33;
				x.1 = 0x44;
			}
			for (a, b) in p.iter().chain(q.iter()) {
				out.push(*a);
				out.extend(b.to_le_bytes());
			}
			let mut s: Box<[String]> = data[..half].iter().map(|&x| text(&[x])).collect();
			let t: Box<[String]> = data[half..half * 2].iter().map(|&x| text(&[x, x])).collect();
			s.clone_from(&t);
			if let Some(x) = s.first_mut() {
				x.push('!');
			}
			for x in s.iter().chain(t.iter()) {
				put(out, x);
			}
			// Each item by its `clone_from`, which clones and drops what it writes over, and
			// of another length, all cloned before the old drop.
			let mut l: Box<[Logged]> = data[..half].iter().map(|&x| logged(x & 0x7e)).collect();
			let m: Box<[Logged]> = data[half..half * 2].iter().map(|&x| logged(x | 1)).collect();
			flush(&log, out);
			l.clone_from(&m);
			flush(&log, out);
			ids(&l, out);
			refill(&mut l, &Box::from([logged(0x10), logged(0x12), logged(0x14)]));
			flush(&log, out);
			ids(&l, out);
			// Into a field, and into a `Vec`'s element.
			let mut held = Held {
				items: data[..data.len() - half].iter().map(|&x| logged(x & 0x7e)).collect(),
				last: logged(0x50),
			};
			flush(&log, out);
			held.items.clone_from(&m);
			flush(&log, out);
			ids(&held.items, out);
			drop(held);
			flush(&log, out);
			let mut v: Vec<Box<[u8]>> = vec![Box::from(&data[..half]), Box::from(data)];
			v[1].clone_from(&b);
			v[0].clone_from(&a);
			for x in &v {
				bytes(x, out);
			}
			drop(l);
			drop(m);
			flush(&log, out);
			let mut t: Box<str> = Box::from("ab");
			t.clone_from(&Box::from(text(data).as_str()));
			put(out, &t);
		}
		1 => {
			let mut a: Box<[u8]> = Box::from(data);
			bytes(a.as_ref(), out);
			a.as_mut().reverse();
			flip(&mut a);
			out.push(sum(&a));
			bytes(&a, out);
			let mut n = Box::new(data.len() as u32);
			*n.as_mut() += 7;
			out.extend(n.as_ref().to_le_bytes());
			let mut pair = Box::new((data.first().copied().unwrap_or(1), 2u16));
			pair.as_mut().1 ^= 0x0101;
			let copy = *pair.as_ref();
			pair.as_mut().0 = 9;
			out.push(copy.0);
			out.extend(copy.1.to_le_bytes());
			out.push(pair.0);
			let mut array = Box::new([1u8, 2, 3, 4]);
			array.as_mut()[usize::from(kind) % 4] = 0x55;
			array.as_mut().reverse();
			bytes(array.as_ref(), out);
			let mut v = Box::new(data.to_vec());
			v.as_mut().push(0x66);
			bytes(v.as_ref(), out);
			let mut s = Box::new(text(data));
			s.as_mut().push('#');
			put(out, s.as_ref());
			let mut w: Box<[u16]> = data.iter().map(|&x| u16::from(x) + 1).collect();
			w.as_mut().sort_unstable();
			wides(w.as_ref(), out);
			let mut l: Box<[Logged]> = data.iter().map(|&x| logged(x)).collect();
			if let [first, .., last] = l.as_mut() {
				std::mem::swap(first, last);
			}
			ids(l.as_ref(), out);
			let mut o: Box<Option<u8>> = Box::new(data.first().copied());
			if let Some(x) = o.as_mut() {
				*x = x.wrapping_add(1);
			}
			out.push(o.as_ref().unwrap_or(0xee));
		}
		2 => {
			boxed(<Box<[u8; 3]>>::try_from(Box::<[u8]>::from(data)), out);
			boxed(<Box<[u8; 0]>>::try_from(Box::<[u8]>::from(data)), out);
			let r: Result<Box<[u8; 2]>, Vec<u8>> = data.to_vec().try_into();
			match r {
				Ok(a) => bytes(&*a, out),
				Err(mut v) => {
					v.push(0x77);
					bytes(&v, out);
				}
			}
			let mut spare = Vec::with_capacity(32);
			spare.extend_from_slice(data);
			boxed(<Box<[u8; 4]>>::try_from(spare), out);
			let w: Box<[u16]> = data.iter().map(|&x| u16::from(x) * 3).collect();
			match <Box<[u16; 2]>>::try_from(w) {
				Ok(mut a) => {
					a[1] = 0x1234;
					wides(&*a, out);
				}
				Err(w) => wides(&w, out),
			}
			let l: Vec<Logged> = data.iter().map(|&x| logged(x & 0x7e)).collect();
			match <Box<[Logged; 2]>>::try_from(l) {
				Ok(a) => {
					ids(&*a, out);
					flush(&log, out);
					drop(a);
				}
				Err(l) => {
					ids(&l, out);
					flush(&log, out);
					drop(l);
				}
			}
			flush(&log, out);
			let l: Box<[Logged]> = data.iter().map(|&x| logged(x | 1)).collect();
			let r: Result<Box<[Logged; 1]>, _> = l.try_into();
			flush(&log, out);
			drop(r);
			flush(&log, out);
		}
		_ => {
			let s: Box<str> = text(data).into_boxed_str();
			let mut a: Box<[u8]> = s.clone().into_boxed_bytes();
			a.reverse();
			bytes(&a, out);
			let mut b: Box<[u8]> = Box::from(s.clone());
			if let Some(x) = b.last_mut() {
				*x = 0x21;
			}
			bytes(&b, out);
			let c: Box<[u8]> = s.into();
			let mut v = c.into_vec();
			v.push(0x2e);
			bytes(&v, out);
			let empty: Box<[u8]> = Box::<str>::default().into_boxed_bytes();
			bytes(&empty, out);
		}
	}
}
