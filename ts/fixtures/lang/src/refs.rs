//! `&mut` to values JS copies, a number, an `Option`, a `Box`, a tuple, an enum, or a
//! `String`, passed on, returned, kept in a struct, captured, and replaced by std: each
//! borrows a local, a field, an element, or a map's value, and what is written through it
//! must land there.

use std::collections::{BTreeMap, HashMap, VecDeque};

fn bump(x: &mut u32, by: u8) {
	*x += u32::from(by);
}

/// A parameter passed on twice, and once from a closure that captures it.
fn bump_thrice(x: &mut u32, by: u8) {
	bump(x, by);
	bump(&mut *x, 1);
	let mut again = |n: u8| bump(x, n);
	again(by);
}

/// Over 2^53, where the runtime switches a `u64` to limbs.
fn shift_wide(x: &mut u64, by: u8) {
	*x = x.wrapping_add(1 << 53).wrapping_add(u64::from(by));
}

/// moq-net's cache shape: the largest seen so far, read, replaced, and taken.
fn take_largest(seen: &mut Option<u8>, next: u8) -> Option<u8> {
	if seen.map_or(true, |s| next > s) {
		return std::mem::replace(seen, Some(next));
	}
	if let Some(s) = seen {
		*s = s.wrapping_add(1);
	}
	if next == 0 { seen.take() } else { None }
}

/// A name replaced through a parameter, its old one returned.
fn rename(name: &mut Option<String>, b: u8) -> Option<String> {
	name.replace(char::from(b'a' + b % 26).to_string())
}

/// A `&mut Option` passed on from inside a closure std calls.
fn largest_of(seen: &mut Option<u8>, items: &[u8]) -> u8 {
	let mut taken = 0u8;
	items.iter().for_each(|&b| {
		if take_largest(seen, b).is_some() {
			taken = taken.wrapping_add(1);
		}
	});
	taken
}

/// moq-net's `&mut Box<Subscriber>`: changed through, and replaced whole.
fn rebox(b: &mut Box<Pair>, by: u8) {
	b.n = b.n.wrapping_add(by);
	if by % 2 == 0 {
		*b = Box::new(Pair {
			n: b.n.wrapping_add(b.n),
		});
	}
}

fn swap_pair(p: &mut (u8, u8)) {
	std::mem::swap(&mut p.0, &mut p.1);
	p.1 = p.1.wrapping_add(1);
	// The whole tuple, replaced in the caller's.
	if p.0 % 3 == 0 {
		*p = (p.1, p.0);
	}
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Fetch {
	Idle,
	Pending(u8),
	Refused,
}

fn refuse(f: &mut Fetch) {
	*f = match *f {
		Fetch::Idle => Fetch::Pending(1),
		Fetch::Pending(n) if n > 2 => Fetch::Refused,
		Fetch::Pending(n) => Fetch::Pending(n + 1),
		Fetch::Refused => Fetch::Idle,
	};
}

fn push_to(s: &mut String, c: char) {
	s.push(c);
	if c == 'z' {
		let old = std::mem::take(s);
		for c in old.chars().take(2) {
			s.push(c);
		}
	}
}

struct Counts {
	hits: usize,
	misses: usize,
}

impl Counts {
	/// A `&mut` returned to one of two fields, as moq-net's `count` returns one.
	fn count(&mut self, hit: bool) -> &mut usize {
		match hit {
			true => &mut self.hits,
			false => &mut self.misses,
		}
	}
}

/// A struct that keeps a `&mut` to its position.
struct Cursor<'a> {
	at: &'a mut usize,
	data: &'a [u8],
}

impl Cursor<'_> {
	fn next(&mut self) -> Option<u8> {
		let b = *self.data.get(*self.at)?;
		*self.at += 1;
		Some(b)
	}
}

fn skip(remaining: &mut usize, data: &[u8]) -> usize {
	let mut skipped = 0;
	for &b in data {
		if *remaining == 0 {
			break;
		}
		*remaining -= 1;
		skipped += usize::from(b % 2);
	}
	skipped
}

struct Pair {
	n: u8,
}

/// Places std reads through: its impls for references compare, order, and format what
/// they point at.
fn read_through(a: &mut u32, b: &mut u32, out: &mut Vec<u8>) {
	out.push(u8::from(a == b));
	out.push(u8::from(a < b));
	out.push(a.cmp(&b) as u8);
	out.extend_from_slice(format!("{a}|{b:?}").as_bytes());
}

/// A `&mut Box` dereferenced through `DerefMut`, as generic code over it would.
fn deref_box(b: &mut Box<Option<u8>>, to: u8) {
	let r = <Box<Option<u8>> as std::ops::DerefMut>::deref_mut(b);
	*r = Some(to);
}

/// Methods that take `&self`, called through a `&mut`.
fn fill(seen: &mut Option<u8>, s: &mut String, next: u8) -> u8 {
	if seen.is_none() {
		*seen = Some(next);
	}
	seen.unwrap_or(0xee).wrapping_add(s.len() as u8)
}

pub fn refs(data: &[u8], out: &mut Vec<u8>) {
	let by = data.first().copied().unwrap_or(3);
	// A local, a field, and elements of an array and a `Vec`.
	let mut local = 7u32;
	bump_thrice(&mut local, by);
	let mut counts = Counts { hits: 0, misses: 0 };
	let mut held = [1u32, 2, 3];
	let mut v: Vec<u32> = data.iter().map(|&b| u32::from(b)).collect();
	for (i, &b) in data.iter().enumerate() {
		*counts.count(b % 3 == 0) += 1;
		let c = counts.count(b % 2 == 0);
		bump_slot(c, b);
		bump(&mut held[i % 3], b);
		bump(&mut v[i], 1);
	}
	out.extend_from_slice(&local.to_be_bytes());
	out.extend_from_slice(&(counts.hits as u32).to_be_bytes());
	out.extend_from_slice(&(counts.misses as u32).to_be_bytes());
	for x in held.iter().chain(&v) {
		out.extend_from_slice(&x.to_be_bytes());
	}
	// Wide integers, in a local and a field.
	let mut wide = u64::MAX - 3;
	shift_wide(&mut wide, by);
	out.extend_from_slice(&wide.to_be_bytes());
	// `Option`s, in a local and through a field's payload.
	let mut seen = None;
	out.push(largest_of(&mut seen, data));
	out.push(seen.unwrap_or(0xee));
	let mut slot = Some(by);
	if let Some(s) = &mut slot {
		bump_byte(s);
	}
	out.push(slot.unwrap_or(0xee));
	// Replaced, in a local, through a parameter, and where `None` is `rt.None`.
	let mut last = None;
	let mut name = None;
	for &b in data {
		out.push(last.replace(b).unwrap_or(0xee));
		out.push(rename(&mut name, b).map_or(0xee, |n| n.as_bytes()[0]));
	}
	let mut nested: Option<Option<u8>> = None;
	let show = |o: Option<Option<u8>>| o.map_or(0xdd, |o| o.unwrap_or(0xee));
	out.push(show(nested.replace(Some(by))));
	out.push(show(nested.replace(None)));
	out.push(show(nested));
	// A box, a tuple, an enum, and a string.
	let mut boxed = Box::new(Pair { n: by });
	let mut pair = (by, by.wrapping_add(1));
	let mut fetch = Fetch::Idle;
	let mut text = String::new();
	for &b in data {
		rebox(&mut boxed, b);
		swap_pair(&mut pair);
		refuse(&mut fetch);
		push_to(&mut text, char::from(b'a' + b % 26));
	}
	out.extend_from_slice(&[boxed.n, pair.0, pair.1]);
	out.push(match fetch {
		Fetch::Idle => 0,
		Fetch::Pending(n) => n,
		Fetch::Refused => 0xff,
	});
	out.extend_from_slice(text.as_bytes());
	let mut empty = None;
	out.push(fill(&mut empty, &mut text, by));
	out.push(fill(&mut empty, &mut text, by.wrapping_add(1)));
	// A struct keeping a `&mut`, and one passed on in a loop.
	let mut at = 0;
	let mut cursor = Cursor { at: &mut at, data };
	let mut sum = 0u8;
	while let Some(b) = cursor.next() {
		sum = sum.wrapping_add(b);
		if b > 200 {
			break;
		}
	}
	out.push(sum);
	out.push(at as u8);
	let mut remaining = usize::from(by % 5);
	out.push(skip(&mut remaining, data) as u8);
	out.push(remaining as u8);
	// A borrow through a reference reassigned while the borrow lives.
	let (mut a, mut b) = (Pair { n: 1 }, Pair { n: 2 });
	let mut p = &mut a;
	let r = &mut p.n;
	p = &mut b;
	bump_byte(r);
	p.n += 10;
	out.extend_from_slice(&[a.n, b.n]);
	// A map's values, passed on.
	let mut hash: HashMap<u8, u32> = HashMap::new();
	let mut tree: BTreeMap<u8, u32> = BTreeMap::new();
	for &b in data {
		bump(hash.entry(b % 4).or_insert(0), b);
		bump(tree.entry(b % 3).or_default(), 1);
		if let Some(x) = hash.get_mut(&(b % 2)) {
			bump(x, 2);
		}
	}
	let sorted: BTreeMap<u8, u32> = hash.into_iter().collect();
	for (k, x) in sorted.into_iter().chain(tree) {
		out.push(k);
		out.extend_from_slice(&x.to_be_bytes());
	}
	// A closure that takes a `&mut`.
	let mut total = 0u32;
	let add = |x: &mut u32, b: u8| bump(x, b);
	for &b in data {
		add(&mut total, b);
	}
	out.extend_from_slice(&total.to_be_bytes());
	// Read through by std, and dereferenced through `DerefMut`.
	let (mut x, mut y) = (u32::from(by), u32::from(data.get(1).copied().unwrap_or(5)));
	read_through(&mut x, &mut y, out);
	out.extend_from_slice(&[x as u8, y as u8]);
	let mut boxed = Box::new(None);
	deref_box(&mut boxed, total as u8);
	out.push(std::ops::Deref::deref(&boxed).unwrap_or(0xee));
	lent(data, out);
}

/// What std lends of an `Option` a `&mut` borrows: its payload, written through in place,
/// passed on, and kept.
fn lent(data: &[u8], out: &mut Vec<u8>) {
	let mut seen: Option<u32> = None;
	let mut last: Option<(u8, u8)> = None;
	let mut text: Option<String> = None;
	let mut held: Vec<&mut u32> = Vec::new();
	let mut spare = [None, Some(9u32)];
	for &b in data {
		match b % 6 {
			0 => *seen.get_or_insert(u32::from(b)) += 1,
			1 => bump(seen.get_or_insert_with(|| u32::from(b) * 2), b),
			2 => *seen.get_or_insert_default() += u32::from(b),
			3 => {
				let x = seen.insert(u32::from(b));
				*x = x.wrapping_add(*x);
			}
			4 => {
				if let Some(x) = seen.as_mut() {
					bump(x, b);
				}
			}
			_ => seen = None,
		}
		last.get_or_insert((b, 0)).1 ^= b;
		if let Some((x, _)) = last.as_mut() {
			*x = x.wrapping_add(1);
		}
		// Through `Option`'s combinators, which hand the closure the place.
		if let Some(p) = last.as_mut().filter(|p| p.0 % 2 == 0).map(|p| {
			p.1 = p.1.wrapping_add(2);
			p
		}) {
			p.0 ^= 1;
		}
		let doubled = seen.as_mut().and_then(|x| (*x % 3 == 0).then_some(x)).map_or(0, |x| {
			*x *= 2;
			*x
		});
		out.extend_from_slice(&doubled.to_be_bytes());
		text.get_or_insert_with(String::new).push(char::from(b'a' + b % 26));
		out.extend_from_slice(&seen.unwrap_or(0xeeee).to_be_bytes());
	}
	let (x, y) = last.unwrap_or((0xee, 0xee));
	out.extend_from_slice(&[x, y]);
	out.extend_from_slice(text.unwrap_or_default().as_bytes());
	// A payload lent through a field and an element, then kept past its loan.
	let mut counts = Pair2 { a: None, b: None };
	for (i, &b) in data.iter().enumerate() {
		let at = match i % 2 {
			0 => counts.a.get_or_insert(0),
			_ => counts.b.get_or_insert_with(|| 100),
		};
		*at += u32::from(b);
		bump(spare[i % 2].get_or_insert(1), b);
	}
	for (i, x) in spare.iter_mut().enumerate() {
		held.push(x.insert(i as u32));
	}
	for x in held {
		*x += 7;
	}
	for x in [counts.a, counts.b, spare[0], spare[1]] {
		out.extend_from_slice(&x.unwrap_or(0xeeee).to_be_bytes());
	}
	defaulted(data, out);
	taken(data, out);
	elements(data, out);
	kept(data, out);
}

/// moq-net's listing: a crate type's default, made in place only while there is none.
#[derive(Default, Debug)]
struct Listing {
	source: Option<u8>,
	listed: Vec<u8>,
	stale: Vec<u8>,
}

/// Filled in place by its default, through a `&mut` and from a closure, then reset.
fn defaulted(data: &[u8], out: &mut Vec<u8>) {
	fn list(listing: &mut Option<Listing>, b: u8) {
		let state = listing.get_or_insert_default();
		state.source = Some(b);
		state.stale = state.listed.clone();
		state.listed.push(b);
	}
	let mut listing: Option<Listing> = None;
	for &b in data {
		match b % 5 {
			0 => listing = None,
			1 => list(&mut listing, b),
			2 => {
				let state = listing.get_or_insert_with(Listing::default);
				state.source = Some(state.source.unwrap_or(b) ^ b);
			}
			3 => {
				let mut relist = |b: u8| listing.get_or_insert_default().listed.push(b);
				relist(b);
				relist(b ^ 1);
			}
			_ => {
				let state = listing.get_or_insert_default();
				state.source = None;
				state.listed.pop();
			}
		}
	}
	out.extend_from_slice(format!("{listing:?}").as_bytes());
}

/// moq-net's ingest: a field of what a `&mut` parameter points at, taken by a closure.
fn restock(listing: &mut Listing, b: u8) -> Option<Vec<u8>> {
	(b % 2 == 0).then(|| std::mem::replace(&mut listing.listed, vec![b]))
}

/// moq-net's front: a `Vec` a closure captured by reference, taken, replaced, and
/// refilled on each call, read after by the caller.
fn taken(data: &[u8], out: &mut Vec<u8>) {
	fn poll<F: FnMut(u8) -> bool>(data: &[u8], mut f: F) -> usize {
		data.iter().take_while(|&&b| !f(b)).count()
	}
	let mut held = data.to_vec();
	let mut wide: Vec<u32> = data.iter().map(|&b| u32::from(b) * 1000).collect();
	let polls = poll(data, |b| {
		for x in std::mem::take(&mut held) {
			if x % 3 != b % 3 {
				held.push(x / 2);
			}
		}
		// From a closure inside this one, and replaced whole.
		let mut halve = || {
			let old = std::mem::replace(&mut wide, Vec::new());
			wide = old.into_iter().filter(|&x| x > u32::from(b)).map(|x| x / 2).collect();
		};
		halve();
		held.is_empty()
	});
	out.extend_from_slice(&(polls as u32).to_be_bytes());
	out.extend_from_slice(&held);
	out.extend_from_slice(format!("{wide:?}").as_bytes());
	// A `&mut` moved into the closure, and a field.
	let mut pair = (data.to_vec(), Listing::default());
	let first = &mut pair.0;
	let mut swap = move |b: u8| std::mem::replace(first, vec![b; usize::from(b % 4)]);
	let mut swapped = Vec::new();
	for &b in data {
		swapped.extend(swap(b));
	}
	let mut list = |b: u8| {
		let listed = std::mem::take(&mut pair.1.listed);
		pair.1.stale = listed;
		pair.1.listed.push(b);
	};
	data.iter().for_each(|&b| list(b));
	for &b in data {
		swapped.extend(restock(&mut pair.1, b).unwrap_or_default());
	}
	let direct = std::mem::take(&mut pair.1.stale);
	out.extend_from_slice(&direct);
	out.extend_from_slice(&swapped);
	out.extend_from_slice(format!("{pair:?}").as_bytes());
}

/// What std lends of the elements and values of slices and collections, which JS copies:
/// one, each in turn, or each to a closure.
fn elements(data: &[u8], out: &mut Vec<u8>) {
	let mut v: Vec<u32> = data.iter().map(|&b| u32::from(b)).collect();
	let mut bytes = data.to_vec();
	let mut pairs: Vec<(u8, Option<u8>)> = data.iter().map(|&b| (b, b.checked_sub(100))).collect();
	let by = data.first().copied().unwrap_or(1);
	if let Some(x) = v.first_mut() {
		*x += 1;
	}
	if let Some(x) = v.last_mut() {
		bump(x, by);
	}
	if let Some(x) = v.get_mut(usize::from(by) % 4) {
		*x *= 2;
	}
	if let Some(b) = bytes.last_mut() {
		*b = b.wrapping_add(1);
	}
	if let Some(b) = bytes.get_mut(1) {
		bump_byte(b);
	}
	for x in v.iter_mut().rev().step_by(2) {
		*x += 3;
	}
	for x in &mut v {
		bump(x, 1);
	}
	for (i, b) in bytes.iter_mut().enumerate() {
		*b ^= i as u8;
	}
	for (b, x) in bytes.iter_mut().zip(&mut v) {
		*x += u32::from(*b);
		*b = b.wrapping_add(*b);
	}
	// Places found, kept, and compared.
	if let Some((_, slot)) = pairs.iter_mut().find(|(k, _)| *k % 3 == 0) {
		*slot = Some(slot.map_or(0, |s| s.wrapping_add(1)));
	}
	let most = v.iter_mut().max_by_key(|x| **x);
	if let Some(x) = most {
		*x = 0;
	}
	let every: Vec<&mut u32> = v.iter_mut().filter(|x| **x % 2 == 0).collect();
	for x in every {
		*x += 1;
	}
	let mut grid = [[0u32; 2]; 3];
	for (row, &b) in grid.iter_mut().zip(data) {
		for (i, x) in row.iter_mut().enumerate() {
			*x = u32::from(b) << i;
		}
	}
	for x in &v {
		out.extend_from_slice(&x.to_be_bytes());
	}
	out.extend_from_slice(&bytes);
	for (k, slot) in &pairs {
		out.extend_from_slice(&[*k, slot.unwrap_or(0xee)]);
	}
	for x in grid.iter().flatten() {
		out.extend_from_slice(&x.to_be_bytes());
	}
	// Maps' and a deque's values.
	let mut hash: HashMap<u8, (u8, u8)> = HashMap::new();
	let mut tree: BTreeMap<u8, u32> = BTreeMap::new();
	let mut queue: VecDeque<u32> = VecDeque::new();
	for &b in data {
		hash.entry(b % 5)
			.and_modify(|x| x.1 = x.1.wrapping_add(b))
			.or_insert((b, 0));
		*tree.entry(b % 4).and_modify(|x| *x += 1).or_default() += u32::from(b);
		queue.push_back(u32::from(b));
		if b % 3 == 0 {
			queue.push_front(u32::from(b) + 1);
		}
	}
	for x in hash.values_mut() {
		x.0 = x.0.wrapping_add(1);
	}
	for (k, x) in hash.iter_mut() {
		if k % 2 == 0 {
			*x = (x.1, x.0);
		}
	}
	for (k, x) in &mut tree {
		*x += u32::from(*k);
	}
	for x in tree.values_mut().rev() {
		*x *= 2;
	}
	for (k, x) in tree.range_mut(1..3) {
		*x += u32::from(*k) << 8;
	}
	// An `Option` whose payload can be `undefined`, whose `None` is apart from it.
	let mut nested: Option<Option<u8>> = None;
	for &b in data {
		match b % 3 {
			0 => *nested.get_or_insert(None) = b.checked_sub(50),
			1 => {
				if let Some(x) = nested.as_mut() {
					*x = x.map(|x| x.wrapping_add(1));
				}
			}
			_ => drop(nested.insert(Some(b))),
		}
		for x in &mut nested {
			*x = x.or(Some(1));
		}
	}
	out.push(match nested {
		None => 0xee,
		Some(None) => 0xdd,
		Some(Some(x)) => x,
	});
	// A borrowed `Option` iterated: its payload's place, or nothing.
	for o in [seen_of(data), None] {
		let mut o = o;
		for x in &mut o {
			*x = x.wrapping_add(7);
		}
		out.push((&mut o).into_iter().count() as u8);
		out.push(o.unwrap_or(0xee));
	}
	// Of a payload JS doesn't copy, a tuple or a `&mut`: the payload itself, or nothing.
	for o in [seen_of(data).map(|b| (b, 1u8)), None] {
		let mut o = o;
		for p in &mut o {
			p.0 ^= 0x5a;
		}
		out.push((&mut o).into_iter().count() as u8);
		out.push(o.map_or(0xee, |p| p.0));
	}
	let mut n = seen_of(data).unwrap_or(0);
	for o in [Some(&mut n), None] {
		let mut o = o;
		out.push((&mut o).into_iter().count() as u8);
	}
	out.push(n);
	for x in seen_of(data).iter_mut() {
		*x ^= 0xff;
		tree.insert(9, u32::from(*x));
	}
	hash.retain(|k, x| {
		x.1 ^= *k;
		*k != by % 5
	});
	tree.retain(|k, x| {
		*x += 1;
		*k != by % 4
	});
	for x in queue.iter_mut() {
		*x += 1;
	}
	if let Some(x) = queue.front_mut() {
		*x += 100;
	}
	if let Some(x) = queue.back_mut() {
		*x += 200;
	}
	if let Some(x) = queue.get_mut(1) {
		*x += 300;
	}
	queue.retain_mut(|x| {
		*x += 1;
		*x % 3 != 0
	});
	// Keys that are their own JS map keys, of values JS copies.
	let mut scalars: HashMap<u8, u32> = HashMap::new();
	for &b in data {
		*scalars.entry(b % 5).or_default() += u32::from(b);
	}
	for (k, x) in scalars.iter_mut() {
		*x += u32::from(*k);
	}
	for x in scalars.values_mut() {
		*x *= 3;
	}
	let scalars: BTreeMap<u8, u32> = scalars.into_iter().collect();
	for (k, x) in scalars {
		out.push(k);
		out.extend_from_slice(&x.to_be_bytes());
	}
	// Keys that are objects, which the runtime chains by hash.
	let mut paired: HashMap<(u8, u8), u32> = HashMap::new();
	for &b in data {
		*paired.entry((b % 3, b % 2)).or_default() += u32::from(b);
	}
	for (k, x) in paired.iter_mut() {
		*x += u32::from(k.0);
	}
	for x in paired.values_mut() {
		*x *= 2;
	}
	paired.retain(|k, x| {
		*x += 1;
		k.1 == 0
	});
	let paired: BTreeMap<(u8, u8), u32> = paired.into_iter().collect();
	for ((a, b), x) in paired {
		out.extend_from_slice(&[a, b]);
		out.extend_from_slice(&x.to_be_bytes());
	}
	let hash: BTreeMap<u8, (u8, u8)> = hash.into_iter().collect();
	for (k, (x, y)) in hash {
		out.extend_from_slice(&[k, x, y]);
	}
	for (k, x) in tree {
		out.push(k);
		out.extend_from_slice(&x.to_be_bytes());
	}
	for x in queue {
		out.extend_from_slice(&x.to_be_bytes());
	}
}

/// A struct a reference points into.
struct Nest {
	pair: Pair,
}

/// A struct holding a reference, which is replaced.
struct Held<'a> {
	at: &'a Nest,
}

/// References kept in a tuple, read after the index or reference they were borrowed
/// through changes: each part reads what it borrowed when the tuple was made.
fn kept(data: &[u8], out: &mut Vec<u8>) {
	let by = data.first().copied().unwrap_or(4);
	let mut v = [1u8, 2, 3, 4];
	for (x, &b) in v.iter_mut().zip(data) {
		*x ^= b;
	}
	let (x, y) = (
		Nest { pair: Pair { n: by } },
		Nest {
			pair: Pair { n: by ^ 0xff },
		},
	);
	let mut i = usize::from(by) % v.len();
	let (mut r, q) = (&x, &y);
	let mut held = Held { at: &x };
	let copied = r;
	let parts = (&v[i], &r.pair, &q.pair, &held.at.pair, &copied.pair, held.at, copied);
	i = (i + 1) % v.len();
	r = &y;
	held.at = &y;
	out.extend_from_slice(&[
		*parts.0,
		parts.1.n,
		parts.2.n,
		parts.3.n,
		parts.4.n,
		parts.5.pair.n,
		parts.6.pair.n,
	]);
	out.extend_from_slice(&[*parts.0 ^ parts.1.n, v[i], r.pair.n, held.at.pair.n]);
}

fn seen_of(data: &[u8]) -> Option<u8> {
	data.get(2).copied()
}

struct Pair2 {
	a: Option<u32>,
	b: Option<u32>,
}

fn bump_slot(c: &mut usize, b: u8) {
	*c += usize::from(b % 4);
}

fn bump_byte(x: &mut u8) {
	*x = x.wrapping_add(1);
}

/// The front of a slice taken through a `&mut` to it, which it moves past: out of bounds
/// past the end, it panics.
fn take_front<'a>(s: &mut &'a [u8], n: usize) -> &'a [u8] {
	let (front, rest) = s.split_at(n);
	*s = rest;
	front
}

/// A count through `&mut self`, and the impl that forwards it through a `&mut`, as `bytes`'
/// `Buf` and `BufMut` forward theirs.
trait Tally {
	fn tally(&mut self, n: u8);
	fn total(&self) -> u32;
}

impl Tally for u32 {
	fn tally(&mut self, n: u8) {
		*self += u32::from(n);
	}

	fn total(&self) -> u32 {
		*self
	}
}

impl Tally for Pair {
	fn tally(&mut self, n: u8) {
		self.n = self.n.wrapping_add(n);
	}

	fn total(&self) -> u32 {
		self.n.into()
	}
}

impl<T: Tally + ?Sized> Tally for &mut T {
	fn tally(&mut self, n: u8) {
		(**self).tally(n);
	}

	fn total(&self) -> u32 {
		(**self).total()
	}
}

fn tally_all<T: Tally>(t: &mut T, data: &[u8]) -> u32 {
	for &b in data {
		t.tally(b);
	}
	t.total()
}

/// A tally through more `&mut`s than `run`'s, which only the `stable_names` test reaches:
/// its impls' instances spell their suffixes alike but for the references, and sort first.
pub(crate) fn deeper(n: u32) -> u32 {
	let mut count = n;
	tally_all(&mut &mut &mut &mut count, &[1])
}

/// A reader keeping a `&mut` to the slice it reads, which it lends on.
struct Reader<'a, 'b> {
	rest: &'b mut &'a [u8],
}

impl<'a> Reader<'a, '_> {
	fn rest(&mut self) -> &mut &'a [u8] {
		self.rest
	}
}

/// The larger of two references, replacing the one a `&mut` points at.
fn keep_larger<'a>(r: &mut &'a Pair, other: &'a Pair) {
	if other.n > r.n {
		*r = other;
	}
}

/// A `&mut` to a `&mut`, written through twice over, then pointed elsewhere.
fn retarget<'a>(r: &mut &'a mut u32, other: &'a mut u32, by: u8) {
	**r += u32::from(by);
	if by % 2 == 0 {
		*r = other;
	}
	**r += 1;
}

/// The next two items, through std's `Iterator` for `&mut I` when `it` is one.
fn next_two<I: Iterator<Item = u8>>(mut it: I) -> u8 {
	let first = it.next().unwrap_or(0xee);
	first.wrapping_add(it.next().unwrap_or(0xdd))
}

/// Called twice, through std's `FnMut` for `&mut F` when `f` is one.
fn call_twice<F: FnMut(u8) -> u8>(mut f: F) -> u8 {
	f(1).wrapping_add(f(2))
}

/// Called once, through std's `FnOnce` for `&mut F` when `f` is one.
fn call_once<F: FnOnce(u8) -> u8>(f: F) -> u8 {
	f(3)
}

/// A `&mut` to a reference: replaced, read, and written through, held, passed on, swapped,
/// and through an impl that forwards a trait through `&mut T`.
pub fn to_refs(data: &[u8], out: &mut Vec<u8>) {
	let [n, rest @ ..] = data else {
		return;
	};
	let n = usize::from(*n);
	// Out of bounds past the end, which panics.
	let mut bytes = rest;
	let front = take_front(&mut bytes, n % (rest.len() + 2));
	out.extend_from_slice(front);
	out.push(bytes.len() as u8);
	// Through a struct's field, lent on.
	let mut reader = Reader { rest: &mut bytes };
	while let Some((&b, tail)) = reader.rest().split_first() {
		*reader.rest() = tail;
		if b % 5 == 0 {
			break;
		}
		out.push(b);
	}
	out.push(bytes.len() as u8);
	// Of a slice that isn't bytes, and of a `str`.
	let words: Vec<u32> = rest.iter().map(|&b| u32::from(b) << 8).collect();
	let mut tail: &[u32] = &words;
	while let [first, more @ ..] = tail {
		out.extend_from_slice(&first.to_be_bytes());
		let tail_ref = &mut tail;
		*tail_ref = more;
	}
	let mut text = String::new();
	for &b in rest {
		text.push(char::from(b'a' + b % 26));
	}
	let mut s = text.as_str();
	let s_ref = &mut s;
	*s_ref = &s_ref[(n % 3).min(s_ref.len())..];
	out.extend_from_slice(s.as_bytes());
	// Swapped and replaced by std.
	let (mut a, mut b) = (rest, &rest[rest.len() / 2..]);
	std::mem::swap(&mut a, &mut b);
	let old = std::mem::replace(&mut b, &[]);
	out.extend_from_slice(&[a.len() as u8, b.len() as u8, old.len() as u8]);
	let taken = std::mem::take(&mut a);
	out.extend_from_slice(&[a.len() as u8, taken.len() as u8]);
	// Moved past by a closure that captured it by `&mut`.
	let mut left = rest;
	let mut skip = |n: usize| {
		let at = n.min(left.len());
		left = &left[at..];
		at
	};
	out.push(skip(n % 3) as u8);
	out.push(skip(1) as u8);
	out.push(left.len() as u8);
	// A reference to an object, replaced.
	let pairs: Vec<Pair> = rest.iter().map(|&n| Pair { n }).collect();
	let first = Pair { n: 0 };
	let mut largest = &first;
	for p in &pairs {
		keep_larger(&mut largest, p);
	}
	out.push(largest.n);
	// A `&mut` to a `&mut`: a number's place, and an object.
	let (mut x, mut y) = (0u32, 100u32);
	let mut r = &mut x;
	retarget(&mut r, &mut y, n as u8);
	*r += 1000;
	out.extend_from_slice(&x.to_be_bytes());
	out.extend_from_slice(&y.to_be_bytes());
	let mut count = 0u32;
	let mut pair = Pair { n: 1 };
	out.extend_from_slice(&tally_all(&mut &mut count, rest).to_be_bytes());
	out.extend_from_slice(&tally_all(&mut &mut pair, rest).to_be_bytes());
	out.extend_from_slice(&tally_all(&mut &mut &mut count, rest).to_be_bytes());
	out.push(pair.n);
	// std's impls forwarding through a `&mut`.
	let mut it = rest.iter().copied();
	out.push(next_two(&mut it));
	out.push(next_two(&mut &mut it));
	out.push(next_two(Box::new(&mut &mut it)));
	let mut r = &mut it;
	for b in &mut r {
		if b % 3 == 0 {
			break;
		}
		out.push(b);
	}
	for b in Box::new(&mut &mut it) {
		if b % 4 == 0 {
			break;
		}
		out.push(b);
	}
	out.push(it.count() as u8);
	let mut calls = 0u8;
	let mut counted = |b: u8| {
		calls = calls.wrapping_add(b);
		calls
	};
	out.push(call_twice(&mut counted));
	out.push(call_twice(&mut &mut counted));
	out.push(call_once(&mut counted));
	out.push(call_once(&mut &mut counted));
	out.push(call_once(&mut &mut &mut counted));
	out.push(call_twice(Box::new(&mut counted)));
	out.push(call_twice(Box::new(&mut &mut counted)));
}

/// `Ok` for an even byte, else `Err`.
fn parity(b: u8) -> Result<u8, u8> {
	if b % 2 == 0 { Ok(b) } else { Err(b) }
}

/// A `&mut` to a `Vec`, an `Option`, or a `Result` of bytes that an adaptor is given,
/// beside items of no places, lends each element as its place to write through. One a
/// consumer is given compares as values.
pub fn adapted(data: &[u8], out: &mut Vec<u8>) {
	let mut v = data.to_vec();
	let mut opt = data.first().copied();
	let mut res = parity(data.get(1).copied().unwrap_or(4));
	for (&b, x) in data.iter().rev().zip(&mut v) {
		*x = x.wrapping_add(b).rotate_left(1);
	}
	for (i, x) in (1u8..).zip(&mut opt) {
		*x ^= i << 6;
	}
	for (i, x) in (3u8..).zip(&mut res) {
		*x = x.wrapping_mul(i);
	}
	for ((x, y), z) in v.iter().zip(&mut opt).zip(&mut res) {
		std::mem::swap(y, z);
		*z ^= *x;
	}
	for x in None.into_iter().chain(&mut v).chain(&mut opt).chain(&mut res) {
		*x = x.wrapping_add(5);
	}
	out.extend(&v);
	out.extend(opt);
	out.extend(res.iter().chain(res.as_ref().err()));
	let copy = v.clone();
	let mut other = opt.map(|x| x ^ 1);
	let all = [
		copy.iter().eq(&mut v),
		copy.iter().ne(&mut v),
		copy.iter().skip(1).eq(&mut v),
		opt.iter().eq(&mut other),
		res.iter().eq(&mut opt),
		data.iter().ne(&mut res),
	];
	out.extend(all.map(u8::from));
}
