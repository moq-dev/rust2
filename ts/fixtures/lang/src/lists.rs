//! `LinkedList`, driven by a script of operations read from the input: of bytes, which JS
//! copies and so lends `&mut` to as places, and of `Tally`s, which it changes in place.

use std::collections::{HashSet, LinkedList, linked_list};
use std::fmt::Write;
use std::hash::{DefaultHasher, Hash, Hasher};

/// A value changed in place through `front_mut`, `iter_mut`, and `extract_if`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Tally {
	n: u16,
}

fn hash_of<T: Hash>(value: &T) -> u64 {
	let mut h = DefaultHasher::new();
	value.hash(&mut h);
	h.finish()
}

/// A list's length, then its items front to back.
fn items(l: &LinkedList<u8>, out: &mut Vec<u8>) {
	out.push(l.len() as u8);
	out.extend(l.iter());
}

/// A script of pushes, pops, writes, splits, and reads from either end on two lists of
/// bytes, then how they compare, hash, iterate, and print. `kind` picks whether a split
/// past the end ends it, which panics.
pub fn lists(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let mut l: LinkedList<u8> = LinkedList::new();
	let mut m: LinkedList<u8> = LinkedList::default();
	for &b in data {
		match b % 13 {
			0 => out.push(l.pop_front().unwrap_or(0xee)),
			1 => out.push(l.pop_back().unwrap_or(0xed)),
			2 => l.push_front(b),
			3 => l.push_back(b),
			4 => {
				out.push(l.front().copied().unwrap_or(0xec));
				out.push(l.back().copied().unwrap_or(0xeb));
				out.push(l.is_empty() as u8);
			}
			// Writes through each end and what the `_mut` pushes lend.
			5 => {
				if let Some(x) = l.front_mut() {
					*x = x.wrapping_add(b);
				}
				if let Some(x) = l.back_mut() {
					*x ^= 0x55;
				}
				*l.push_back_mut(b) ^= 0x0f;
				let x = l.push_front_mut(b);
				*x = x.wrapping_mul(3);
			}
			// At 0, the end, or between, which std finds from the nearer end.
			6 => {
				let at = usize::from(b / 13) % (l.len() + 1);
				let mut tail = l.split_off(at);
				items(&tail, out);
				tail.append(&mut m);
				m = tail;
			}
			7 => {
				l.append(&mut m);
				out.push(m.len() as u8);
			}
			8 => {
				out.push(l.contains(&b) as u8);
				out.push(m.contains(&(b ^ 1)) as u8);
			}
			// Read from either end, as each bit says.
			9 => {
				let mut it = l.iter();
				for bit in 0..8 {
					let x = if b >> bit & 1 == 0 { it.next() } else { it.next_back() };
					out.push(x.copied().unwrap_or(0xea));
				}
				out.push(it.len() as u8);
				out.extend(it.clone().rev());
				out.push(it.size_hint().0 as u8);
			}
			// Written through from either end, as each bit says.
			10 => {
				let mut it = l.iter_mut();
				let mut i = 0u8;
				loop {
					let x = if (b >> (i % 8)) & 1 == 0 {
						it.next()
					} else {
						it.next_back()
					};
					let Some(x) = x else {
						break;
					};
					*x = x.wrapping_add(i);
					i += 1;
				}
				for x in &mut m {
					*x = x.rotate_left(1);
				}
			}
			11 => std::mem::swap(&mut l, &mut m),
			_ => {
				let taken: Vec<u8> = l
					.extract_if(|x| {
						*x = x.wrapping_add(1);
						*x % 3 == 0
					})
					.collect();
				out.extend(taken);
			}
		}
	}
	items(&l, out);
	items(&m, out);
	out.push((l == m) as u8);
	out.push((l != m) as u8);
	out.push(l.cmp(&m) as i8 as u8);
	out.push(l.partial_cmp(&m).map_or(9, |o| o as i8 as u8));
	out.push((l < m) as u8);
	out.push((l >= m) as u8);
	out.extend(hash_of(&l).to_le_bytes());
	let set: HashSet<LinkedList<u8>> = [l.clone(), m.clone(), l.clone()].into_iter().collect();
	out.push(set.len() as u8);
	// Taken from both ends.
	let mut it = l.clone().into_iter();
	out.push(it.next_back().unwrap_or(0xe9));
	out.push(it.next().unwrap_or(0xe8));
	out.push(it.len() as u8);
	out.extend(it.clone());
	out.extend(it.rev());
	out.push(l.iter().last().copied().unwrap_or(0xe7));
	out.push(l.iter().nth(usize::from(kind) % 4).copied().unwrap_or(0xe6));
	edges(data, out);
	printed(&l, &m, out);
	tallies(data, out);
	if kind % 4 == 0 {
		let past = m.len() + 1 + usize::from(kind / 4 % 2);
		out.extend(m.split_off(past));
	}
}

/// `split_off` and `append` at 0, either side of the middle, and the end, and the list
/// `collect`, `From`, and `extend` make, empty ones included.
fn edges(data: &[u8], out: &mut Vec<u8>) {
	let whole: LinkedList<u8> = data.iter().copied().collect();
	let len = whole.len();
	for at in [0, 1.min(len), len / 2, len.saturating_sub(1), len] {
		let mut front = whole.clone();
		let mut back = front.split_off(at);
		items(&front, out);
		items(&back, out);
		front.append(&mut back);
		out.push(back.is_empty() as u8);
		out.push((front == whole) as u8);
		// Onto an empty list, and an empty one onto it.
		let mut empty = LinkedList::new();
		empty.append(&mut front);
		empty.append(&mut LinkedList::new());
		out.push(front.len() as u8);
		out.push((empty == whole) as u8);
	}
	let mut from = LinkedList::from([1u8, 2, 3]);
	from.extend(data.iter().take(3));
	from.extend(whole.clone());
	from.extend(vec![7, 8]);
	items(&from, out);
	// A tuple's `Extend`, by each list's `extend_one`.
	let (evens, odds): (LinkedList<u8>, LinkedList<u8>) = data.iter().map(|&b| (b & !1, b | 1)).unzip();
	items(&evens, out);
	items(&odds, out);
	let mut copy = from.clone();
	copy.clone_from(&whole);
	items(&copy, out);
	copy.clone_from(&from);
	items(&copy, out);
	copy.clear();
	items(&copy, out);
	out.push(linked_list::IntoIter::<u8>::default().len() as u8);
	out.push(linked_list::Iter::<u8>::default().next().is_none() as u8);
	out.push(linked_list::IterMut::<Tally>::default().next().is_none() as u8);
	// A float's NaN orders as neither.
	let floats: LinkedList<f64> = data
		.iter()
		.map(|&b| if b == 0xff { f64::NAN } else { f64::from(b) })
		.collect();
	let other: LinkedList<f64> = data.iter().rev().map(|&b| f64::from(b)).collect();
	out.push(floats.partial_cmp(&other).map_or(9, |o| o as i8 as u8));
	out.push((floats == other) as u8);
}

/// `Debug` of the lists and of their iterators part read, plain, pretty, and in hex.
fn printed(l: &LinkedList<u8>, m: &LinkedList<u8>, out: &mut Vec<u8>) {
	let mut text = String::new();
	let mut it = l.iter();
	it.next();
	let mut owned = m.clone().into_iter();
	owned.next_back();
	let mut copy = m.clone();
	let mut lent = copy.iter_mut();
	lent.next();
	let _ = write!(text, "{l:?}|{m:#?}|{it:?}|{it:x?}|{it:#?}|{owned:?}|{lent:?}");
	out.extend(text.as_bytes());
}

/// A list of `Tally`s, which JS changes in place rather than through places.
fn tallies(data: &[u8], out: &mut Vec<u8>) {
	let mut l: LinkedList<Tally> = data.iter().map(|&b| Tally { n: u16::from(b) }).collect();
	let copy = l.clone();
	if let Some(t) = l.front_mut() {
		t.n ^= 0xff;
	}
	if let Some(t) = l.back_mut() {
		t.n += 1;
	}
	l.push_back_mut(Tally::default()).n = 0x77;
	l.push_front_mut(Tally { n: 3 }).n *= 5;
	for (i, t) in l.iter_mut().enumerate() {
		t.n = t.n.wrapping_add(i as u16);
	}
	let taken: LinkedList<Tally> = l
		.extract_if(|t| {
			t.n += 1;
			t.n % 4 == 0
		})
		.collect();
	let mut rest = l.clone();
	rest.clone_from(&taken);
	for list in [&l, &copy, &taken, &rest] {
		out.push(list.len() as u8);
		out.extend(list.iter().map(|t| t.n as u8));
	}
	out.push((l.contains(&Tally { n: 0x78 })) as u8);
	out.push(l.cmp(&copy) as i8 as u8);
	out.push((hash_of(&taken) == hash_of(&rest)) as u8);
	let mut text = String::new();
	let _ = write!(text, "{:?}", rest.iter().rev().take(2).collect::<Vec<_>>());
	let _ = write!(text, "{:#?}", taken);
	out.extend(text.as_bytes());
}
