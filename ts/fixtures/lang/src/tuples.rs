//! `collect`, `unzip`, and `extend` of tuples of collections, through std's `FromIterator`
//! and `Extend` of tuples: a 1-tuple's collection by its own `extend`, and a longer
//! tuple's each by its `extend_reserve` and `extend_one`, in field order, of every
//! collection the runtime extends, borrowed items copied, items that drop, and the crate's
//! own collections, which keep std's defaults of both.

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// Logs its id as it drops. Equal, hashed, and ordered by `key` alone, so a collection
/// keeps one and drops another equal to it.
struct Logged {
	id: u8,
	key: u8,
	log: Rc<RefCell<Vec<u8>>>,
}

impl PartialEq for Logged {
	fn eq(&self, other: &Self) -> bool {
		self.key == other.key
	}
}

impl Eq for Logged {}

impl Hash for Logged {
	fn hash<H: Hasher>(&self, state: &mut H) {
		self.key.hash(state);
	}
}

impl Ord for Logged {
	fn cmp(&self, other: &Self) -> Ordering {
		self.key.cmp(&other.key)
	}
}

impl PartialOrd for Logged {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(other))
	}
}

impl Drop for Logged {
	fn drop(&mut self) {
		self.log.borrow_mut().push(self.id);
	}
}

/// Copied out of the references `Extend<&T>` is given.
#[derive(Clone, Copy)]
struct Spot {
	x: u8,
	y: u16,
}

/// Puts what was logged since the last flush, after a marker.
fn flush(log: &RefCell<Vec<u8>>, out: &mut Vec<u8>) {
	out.push(0xfd);
	out.extend(log.borrow_mut().drain(..));
}

/// Puts a hash set's keys, which it iterates in no order std promises, in order.
fn sorted(set: &HashSet<u8>, out: &mut Vec<u8>) {
	out.push(0xfc);
	out.extend(set.iter().copied().collect::<BTreeSet<u8>>());
}

/// Puts a hash map's entries in key order.
fn entries(map: &HashMap<u8, u8>, out: &mut Vec<u8>) {
	out.push(0xfb);
	let sorted: BTreeMap<u8, u8> = map.iter().map(|(k, v)| (*k, *v)).collect();
	out.extend(sorted.into_iter().flat_map(|(k, v)| [k, v]));
}

/// Collected, unzipped, or extended as `kind` picks, from at most 24 bytes of `data`.
pub fn tuples(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let data = &data[..data.len().min(24)];
	match kind % 12 {
		0 => ones(data, out),
		1 => pairs(data, out),
		2 => triples(data, out),
		3 => twelves(data, out),
		4 => extended(data, out),
		5 => borrowed(data, out),
		6 => dropped(data, out),
		7 => panicked(data, out),
		8 => reserved(data, out),
		9 => shunted(data, out),
		10 => own(data, out),
		_ => own_panicked(data, out),
	}
}

/// 1-tuples, each by its collection's own `extend`: a heap's rebuilds once, where a longer
/// tuple's pushes each item.
fn ones(data: &[u8], out: &mut Vec<u8>) {
	let (v,): (Vec<u16>,) = data.iter().map(|&b| (u16::from(b) << 4,)).collect();
	out.extend(v.iter().flat_map(|n| n.to_le_bytes()));
	let (s,): (String,) = data.iter().map(|&b| (char::from(b),)).collect();
	out.extend(s.bytes());
	let (heap,): (BinaryHeap<u8>,) = data.iter().map(|&b| (b,)).collect();
	out.extend(heap.into_vec());
	let (q,): (VecDeque<u8>,) = data.iter().filter(|&&b| b & 1 == 0).map(|&b| (b,)).collect();
	out.extend(q);
	let (set,): (HashSet<u8>,) = data.iter().map(|&b| (b % 16,)).collect();
	sorted(&set, out);
	let (tree,): (BTreeSet<u8>,) = data.iter().map(|&b| (b % 16,)).collect();
	out.extend(tree);
	let mut t = (vec![0xaa_u8],);
	t.extend(data.chunks(2).map(|c| (c[0],)));
	out.extend(t.0);
	let mut t = (String::from("ab"),);
	t.extend(data.iter().map(|&b| (["x", "", "yz"][usize::from(b % 3)],)));
	out.extend(t.0.bytes());
	// A tuple's own `extend`, of `()` too.
	let ((v, heap),): ((Vec<u8>, BinaryHeap<u8>),) = data.iter().map(|&b| ((b, b),)).collect();
	out.extend(v);
	out.extend(heap.into_vec());
	let ((),) = data.iter().map(|_| ((),)).collect();
	let mut seen = 0_u8;
	Extend::extend(&mut (), data.iter().map(|_| seen += 1));
	out.push(seen);
}

/// Pairs, by `collect` and `unzip`, of `()` too.
fn pairs(data: &[u8], out: &mut Vec<u8>) {
	let (v, s): (Vec<u8>, String) = data.iter().map(|&b| (b ^ 0x5a, char::from(b))).collect();
	out.extend(v);
	out.extend(s.bytes());
	let (v, tree): (Vec<u8>, BTreeSet<u8>) = data.iter().map(|&b| (b, b % 8)).unzip();
	out.extend(v);
	out.extend(tree);
	let (heap, q): (BinaryHeap<u8>, VecDeque<u16>) = data.iter().map(|&b| (b, u16::from(b) * 3)).unzip();
	out.extend(heap.into_vec());
	out.extend(q.iter().flat_map(|n| n.to_le_bytes()));
	let ((), v): ((), Vec<u8>) = data.iter().map(|&b| ((), b)).collect();
	out.extend(v);
	// A size hint whose lower bound is less than the items, from which each grows.
	let (v, q): (Vec<u16>, VecDeque<u8>) = data.iter().filter(|&&b| b > 0x40).map(|&b| (u16::from(b), b)).unzip();
	out.extend(v.iter().flat_map(|n| n.to_le_bytes()));
	out.extend(q);
}

/// Triples, of mixed collections, and tuples of collections within one.
fn triples(data: &[u8], out: &mut Vec<u8>) {
	let (v, q, s): (Vec<u8>, VecDeque<u16>, String) = data
		.iter()
		.map(|&b| (b, u16::from(b) << 8, char::from(b'a' + b % 26)))
		.collect();
	out.extend(v);
	out.extend(q.iter().flat_map(|n| n.to_le_bytes()));
	out.extend(s.bytes());
	let (map, tree, heap): (HashMap<u8, u8>, BTreeMap<u8, u16>, BinaryHeap<u8>) =
		data.iter().map(|&b| ((b % 8, b), (b % 5, u16::from(b)), b)).collect();
	entries(&map, out);
	out.extend(tree.into_iter().flat_map(|(k, v)| [k, v as u8]));
	out.extend(heap.into_vec());
	let (v, (s, w)): (Vec<u8>, (String, Vec<u16>)) =
		data.iter().map(|&b| (b, (char::from(b), u16::from(b) + 1))).collect();
	out.extend(v);
	out.extend(s.bytes());
	out.extend(w.iter().flat_map(|n| n.to_le_bytes()));
	let (v, (tree,)): (Vec<u8>, (BTreeSet<u8>,)) = data.iter().map(|&b| (b, (b / 32,))).collect();
	out.extend(v);
	out.extend(tree);
}

/// The longest tuple std's impls take, of every collection the runtime extends.
#[allow(clippy::type_complexity)]
fn twelves(data: &[u8], out: &mut Vec<u8>) {
	let (bytes, wide, chars, texts, q, heap, map, set, tree, keys, (), (v, (s,))): (
		Vec<u8>,
		Vec<u16>,
		String,
		String,
		VecDeque<u8>,
		BinaryHeap<u8>,
		HashMap<u8, u8>,
		HashSet<u8>,
		BTreeMap<u8, u16>,
		BTreeSet<u8>,
		(),
		(Vec<u8>, (String,)),
	) = data
		.iter()
		.map(|&b| {
			let text = ["", "q", "rs"][usize::from(b % 3)];
			(
				b,
				u16::from(b) * 257,
				char::from(b),
				text,
				!b,
				b,
				(b % 4, b),
				b % 6,
				(b % 3, u16::from(b)),
				b / 16,
				(),
				(b ^ 0x33, (char::from(b'A' + b % 26),)),
			)
		})
		.collect();
	out.extend(bytes);
	out.extend(wide.iter().flat_map(|n| n.to_le_bytes()));
	out.extend(chars.bytes());
	out.extend(texts.bytes());
	out.extend(q);
	out.extend(heap.into_vec());
	entries(&map, out);
	sorted(&set, out);
	out.extend(tree.into_iter().flat_map(|(k, v)| v.to_le_bytes().map(|x| x ^ k)));
	out.extend(keys);
	out.extend(v);
	out.extend(s.bytes());
}

/// `extend` onto collections that hold some already: a heap sifts each item up past them,
/// and a map or set keeps an equal key already there.
fn extended(data: &[u8], out: &mut Vec<u8>) {
	let mut t = (
		vec![9_u8, 8],
		String::from("pre"),
		BTreeMap::from([(3_u8, 30_u16)]),
		VecDeque::from([1_u16]),
		HashSet::from([5_u8]),
	);
	t.extend(
		data.iter()
			.map(|&b| (b, char::from(b), (b % 8, u16::from(b)), u16::from(b), b % 16)),
	);
	let (v, s, tree, q, set) = t;
	out.extend(v);
	out.extend(s.bytes());
	out.extend(tree.into_iter().flat_map(|(k, v)| [k, v as u8]));
	out.extend(q.iter().flat_map(|n| n.to_le_bytes()));
	sorted(&set, out);
	let mut t = (BinaryHeap::from([7_u8, 200, 30]), HashMap::from([(1_u8, 1_u8)]));
	t.extend(data.iter().map(|&b| (b, (b % 4, b))));
	out.extend(t.0.into_vec());
	entries(&t.1, out);
}

/// `Extend<&T>` and `Extend<(&K, &V)>`, which store copies of what the items borrow.
fn borrowed(data: &[u8], out: &mut Vec<u8>) {
	let mut points: Vec<Spot> = data
		.iter()
		.map(|&b| Spot {
			x: b,
			y: u16::from(b) << 2,
		})
		.collect();
	let mut t: (Vec<u8>, Vec<Spot>, HashMap<u8, u8>, String) = Default::default();
	let marks = ['z', 'é'];
	t.extend(
		data.iter()
			.zip(&points)
			.map(|(b, p)| (b, p, (b, b), &marks[usize::from(b & 1)])),
	);
	let mut ones = (Vec::new(),);
	ones.extend(points.iter().map(|p| (p,)));
	for p in &mut points {
		p.x = 0xee;
	}
	let (v, copied, map, s) = t;
	out.extend(v);
	out.extend(copied.iter().chain(&ones.0).flat_map(|p| [p.x, p.y as u8]));
	entries(&map, out);
	out.extend(s.bytes());
	out.extend(points.iter().map(|p| p.x));
}

/// Items that drop: a map drops the value an equal key replaces, and a set the key equal
/// to one it holds, as each item's fields extend in order, and what an iterator still
/// holds drops once the tuple extended by it is done.
fn dropped(data: &[u8], out: &mut Vec<u8>) {
	let log = Rc::new(RefCell::new(Vec::new()));
	let logged = |id: u8, key: u8| Logged {
		id,
		key,
		log: log.clone(),
	};
	let (v, map, set): (Vec<Logged>, HashMap<u8, Logged>, BTreeSet<Logged>) = (data.iter().enumerate())
		.map(|(i, &b)| {
			let i = i as u8;
			(logged(i, b), (b % 4, logged(0x40 | i, 0)), logged(0x80 | i, b % 3))
		})
		.collect();
	flush(&log, out);
	out.extend(v.iter().map(|l| l.id));
	let mut held: Vec<(u8, Logged)> = map.into_iter().collect();
	held.sort_by_key(|(k, _)| *k);
	out.extend(held.iter().map(|(k, l)| k ^ l.id));
	out.extend(set.iter().map(|l| l.id));
	drop(v);
	drop(held);
	drop(set);
	flush(&log, out);
	let (tree, keys): (BTreeMap<u8, Logged>, HashSet<Logged>) = (data.iter().enumerate())
		.map(|(i, &b)| ((b % 3, logged(i as u8, 0)), logged(0x80 | i as u8, b % 5)))
		.unzip();
	flush(&log, out);
	let mut held: Vec<&Logged> = keys.iter().collect();
	held.sort_by_key(|l| l.key);
	out.extend(held.iter().map(|l| l.id));
	drop(tree);
	flush(&log, out);
	let source: Vec<(Logged, u8)> = (data.iter().enumerate())
		.map(|(i, &b)| (logged(i as u8, b), b))
		.collect();
	let n = usize::from(data.first().copied().unwrap_or(0) % 8);
	let (taken, bytes): (Vec<Logged>, Vec<u8>) = source.into_iter().take(n).unzip();
	flush(&log, out);
	out.extend(bytes);
	drop(taken);
	flush(&log, out);
	drop(keys);
	log.borrow_mut().clear();
}

/// A panic midway, from what makes the items.
fn panicked(data: &[u8], out: &mut Vec<u8>) {
	let at = usize::from(data.first().copied().unwrap_or(0) % 8);
	let (v, s, w): (Vec<u8>, String, Vec<u16>) = (data.iter().enumerate())
		.map(|(i, &b)| {
			assert!(i != at, "item {i} of {}", data.len());
			(b, char::from(b), u16::from(b))
		})
		.collect();
	out.extend(v);
	out.extend(s.bytes());
	out.extend(w.iter().flat_map(|n| n.to_le_bytes()));
}

/// A size hint past what a collection may hold, which the first to reserve for it panics
/// at, in field order, before any item is made.
fn reserved(data: &[u8], out: &mut Vec<u8>) {
	let items = 0..u32::MAX;
	match data.first().copied().unwrap_or(0) % 6 {
		0 => {
			let ((), v): ((), Vec<u32>) = items.map(|i| ((), i)).collect();
			out.push(v.len() as u8);
		}
		1 => {
			let (set, v): (HashSet<u8>, Vec<u8>) = items.map(|i| (i as u8, i as u8)).collect();
			out.extend([set.len() as u8, v.len() as u8]);
		}
		2 => {
			let (v, set): (Vec<u8>, HashSet<u8>) = items.map(|i| (i as u8, i as u8)).collect();
			out.extend([set.len() as u8, v.len() as u8]);
		}
		3 => {
			let (v, s): (Vec<()>, String) = items.map(|_| ((), 'a')).collect();
			out.extend([v.len() as u8, s.len() as u8]);
		}
		4 => {
			let (tree, q): (BTreeSet<u8>, VecDeque<u16>) = items.map(|i| (i as u8, i as u16)).collect();
			out.extend([tree.len() as u8, q.len() as u8]);
		}
		_ => {
			let (v,): (Vec<u8>,) = items.map(|i| (i as u8,)).collect();
			out.push(v.len() as u8);
		}
	}
}

/// `collect` of `Result`s and `Option`s of pairs, into one of a tuple, which std's docs show.
fn shunted(data: &[u8], out: &mut Vec<u8>) {
	let parsed: Result<(Vec<u8>, String), u8> = data
		.iter()
		.map(|&b| if b == 0 { Err(b) } else { Ok((b, char::from(b))) })
		.collect();
	match parsed {
		Ok((v, s)) => {
			out.extend(v);
			out.extend(s.bytes());
		}
		Err(e) => out.extend([0xfa, e]),
	}
	let found: Option<(Vec<u16>, BTreeSet<u8>)> = data
		.iter()
		.map(|&b| b.checked_sub(4).map(|n| (u16::from(n), b % 8)))
		.collect();
	match found {
		Some((v, tree)) => {
			out.extend(v.iter().flat_map(|n| n.to_le_bytes()));
			out.extend(tree);
		}
		None => out.push(0xf9),
	}
}

thread_local! {
	/// What the crate's own collections and the items they are given log.
	static CALLS: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
	/// The id the next `Tally::default` takes.
	static NEXT: Cell<u8> = const { Cell::new(0) };
}

fn call(entry: &[u8]) {
	CALLS.with_borrow_mut(|calls| calls.extend_from_slice(entry));
}

/// Puts what was logged since the last, after a marker.
fn calls(out: &mut Vec<u8>) {
	out.push(0xfd);
	CALLS.with_borrow_mut(|calls| out.extend(calls.drain(..)));
}

/// The crate's own collection: `default` logs the id it takes, and each `extend` the size
/// hint of the iterator it is given and how many items that held. An item of 0xee panics.
struct Tally {
	id: u8,
	sum: u8,
}

impl Default for Tally {
	fn default() -> Self {
		let id = NEXT.get();
		NEXT.set(id.wrapping_add(1));
		call(&[0xd0, id]);
		Tally { id, sum: 0 }
	}
}

impl Extend<u8> for Tally {
	fn extend<I: IntoIterator<Item = u8>>(&mut self, iter: I) {
		let iter = iter.into_iter();
		let (lower, upper) = iter.size_hint();
		let mut n = 0_u8;
		for b in iter {
			assert!(b != 0xee, "tally {} at its item {n}", self.id);
			self.sum = self.sum.wrapping_add(b);
			n += 1;
		}
		call(&[0xe0, self.id, lower as u8, upper.map_or(0xff, |u| u as u8), n]);
	}
}

/// The crate's own enum, which JS keeps in a place: the last item and how many there were.
#[derive(Default)]
enum Last {
	#[default]
	Empty,
	Seen(char, u8),
}

impl Extend<char> for Last {
	fn extend<I: IntoIterator<Item = char>>(&mut self, iter: I) {
		for c in iter {
			*self = match *self {
				Last::Empty => Last::Seen(c, 1),
				Last::Seen(_, n) => Last::Seen(c, n + 1),
			};
		}
	}
}

impl Last {
	fn put(&self, out: &mut Vec<u8>) {
		match *self {
			Last::Empty => out.push(0xf8),
			Last::Seen(c, n) => out.extend([c as u8, n]),
		}
	}
}

/// Logs each item as it is made, before the collections are given its fields.
fn made(b: &u8) {
	call(&[0xa0, *b]);
}

/// Tuples holding the crate's own collections beside std's, by `collect`, `extend`, and
/// `unzip`: a 1-tuple's by its own `extend` of every item, and a longer one's by its
/// `extend` of `Some` of each item, in field order, after each default in field order.
fn own(data: &[u8], out: &mut Vec<u8>) {
	let data: Vec<u8> = data.iter().map(|&b| if b == 0xee { 0xed } else { b }).collect();
	NEXT.set(0);
	CALLS.with_borrow_mut(Vec::clear);
	let (t,): (Tally,) = data.iter().inspect(|b| made(b)).map(|&b| (b,)).collect();
	out.push(t.sum);
	calls(out);
	let mut one = (Tally::default(),);
	one.extend(data.iter().filter(|&&b| b & 1 == 0).map(|&b| (b,)));
	out.push(one.0.sum);
	calls(out);
	let (v, t): (Vec<u8>, Tally) = data.iter().inspect(|b| made(b)).map(|&b| (b ^ 0x5a, b)).collect();
	out.extend(v);
	out.push(t.sum);
	calls(out);
	let (t, w): (Tally, Vec<u16>) = data.iter().map(|&b| (b, u16::from(b) << 3)).unzip();
	out.push(t.sum);
	out.extend(w.iter().flat_map(|n| n.to_le_bytes()));
	calls(out);
	let (a, s, b): (Tally, String, Tally) = data
		.iter()
		.inspect(|b| made(b))
		.map(|&b| (b, char::from(b), !b))
		.collect();
	out.extend([a.sum, b.sum]);
	out.extend(s.bytes());
	calls(out);
	let (s, last, t): (String, Last, Tally) = data
		.iter()
		.map(|&b| (char::from(b), char::from(b'a' + b % 26), b))
		.collect();
	out.extend(s.bytes());
	last.put(out);
	out.push(t.sum);
	calls(out);
	// A 1-tuple within a pair extends by its collection's `extend_one`.
	let (v, (t,)): (Vec<u8>, (Tally,)) = data.iter().map(|&b| (b, (b,))).collect();
	out.extend(v);
	out.push(t.sum);
	calls(out);
	let (last,): (Last,) = data.iter().map(|&b| (char::from(b),)).collect();
	last.put(out);
	let mut t = (Tally::default(), vec![7_u8], Last::Seen('q', 9));
	t.extend(data.chunks(2).map(|c| (c[0], c[0], char::from(c[0]))));
	out.push(t.0.sum);
	out.extend(t.1);
	t.2.put(out);
	calls(out);
	// What std's collects in place of the items, up to the first `Err` or `None`.
	let parsed: Result<(Tally, Vec<u8>), u8> = data.iter().map(|&b| if b == 0 { Err(b) } else { Ok((b, b)) }).collect();
	match parsed {
		Ok((t, v)) => {
			out.push(t.sum);
			out.extend(v);
		}
		Err(e) => out.extend([0xfa, e]),
	}
	calls(out);
	let found: Option<(Vec<u8>, Tally)> = data.iter().map(|&b| b.checked_sub(4).map(|n| (n, b))).collect();
	match found {
		Some((v, t)) => {
			out.extend(v);
			out.push(t.sum);
		}
		None => out.push(0xf9),
	}
	calls(out);
	// Items that drop, which the iterator still holds past an `Err`.
	let log = Rc::new(RefCell::new(Vec::new()));
	let source: Vec<Result<(Logged, u8), u8>> = (data.iter().enumerate())
		.map(|(i, &b)| match b {
			0x80.. => Err(b),
			_ => Ok((
				Logged {
					id: i as u8,
					key: b,
					log: log.clone(),
				},
				b,
			)),
		})
		.collect();
	let held: Result<(Vec<Logged>, Tally), u8> = source.into_iter().collect();
	flush(&log, out);
	match held {
		Ok((v, t)) => {
			out.extend(v.iter().map(|l| l.id));
			out.push(t.sum);
		}
		Err(e) => out.extend([0xfa, e]),
	}
	flush(&log, out);
	calls(out);
	let alone: Result<(Tally,), u8> = data
		.iter()
		.map(|&b| if b == 0xff { Err(b) } else { Ok((b,)) })
		.collect();
	match alone {
		Ok((t,)) => out.push(t.sum),
		Err(e) => out.extend([0xfa, e]),
	}
	calls(out);
}

/// A panic in the crate's own `extend` at an item partway, which the panic message names.
fn own_panicked(data: &[u8], out: &mut Vec<u8>) {
	let at = usize::from(data.first().copied().unwrap_or(0) % 8);
	let items = data
		.iter()
		.enumerate()
		.map(|(i, &b)| if i == at { 0xee } else { b & 0x7f });
	NEXT.set(0);
	match data.get(1).copied().unwrap_or(0) % 2 {
		0 => {
			let (v, t, s): (Vec<u8>, Tally, String) = items.map(|b| (b, b, char::from(b))).collect();
			out.extend(v);
			out.push(t.sum);
			out.extend(s.bytes());
		}
		_ => {
			let (t,): (Tally,) = items.map(|b| (b,)).collect();
			out.push(t.sum);
		}
	}
}
