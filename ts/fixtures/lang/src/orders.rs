//! Ordering in std: `cmp`'s `max` and `min` and their `_by` and `_by_key` forms, the
//! `is_sorted` family, and `BinaryHeap`, of a type that logs each comparison std makes of
//! it and each drop, so which tests run, in what order, which of equal values comes out,
//! and when each value drops all show. Then byte arrays, slices, and vectors, by their
//! bytes and then their length, `Result`s, `Poll`s, and deques, by what they hold, and
//! tuples, by their fields. Then B-trees collected of keys whose impls disagree.

use std::cell::RefCell;
use std::cmp::{self, Ordering, Reverse};
use std::collections::binary_heap::PeekMut;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};
use std::marker::PhantomData;
use std::task::Poll;

thread_local! {
	static LOG: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn log(entry: [u8; 3]) {
	LOG.with_borrow_mut(|log| log.extend(entry));
}

/// Puts what was logged since the last flush.
fn flush(out: &mut Vec<u8>) {
	LOG.with_borrow_mut(|log| {
		out.extend_from_slice(log);
		log.clear();
	});
}

/// Ordered by `key` alone, so `tag` tells equal ones apart. Each comparison logs which it
/// is and the two tags, and a drop logs the tag.
struct Item {
	key: u8,
	tag: u8,
}

fn item(tag: usize, key: u8) -> Item {
	Item {
		key: key % 8,
		tag: tag as u8,
	}
}

impl Drop for Item {
	fn drop(&mut self) {
		log([b'd', self.tag, 0]);
	}
}

impl PartialEq for Item {
	fn eq(&self, other: &Self) -> bool {
		self.key == other.key
	}
}

impl Eq for Item {}

impl Ord for Item {
	fn cmp(&self, other: &Self) -> Ordering {
		log([b'c', self.tag, other.tag]);
		self.key.cmp(&other.key)
	}
}

impl PartialOrd for Item {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(other))
	}

	fn lt(&self, other: &Self) -> bool {
		log([b'<', self.tag, other.tag]);
		self.key < other.key
	}

	fn le(&self, other: &Self) -> bool {
		log([b'l', self.tag, other.tag]);
		self.key <= other.key
	}

	fn gt(&self, other: &Self) -> bool {
		log([b'>', self.tag, other.tag]);
		self.key > other.key
	}

	fn ge(&self, other: &Self) -> bool {
		log([b'g', self.tag, other.tag]);
		self.key >= other.key
	}
}

/// Run one case.
pub fn orders(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	match kind % 4 {
		0 => cmps(data, out),
		1 => sorted(data, out),
		2 => heaps(data, out),
		_ => values(data, out),
	}
	flush(out);
}

/// `cmp::max` and `min` and their forms of each byte and the next, as integers, strings,
/// and items, which the one not returned drops, and of references to items.
fn cmps(data: &[u8], out: &mut Vec<u8>) {
	for i in 1..data.len() {
		let (a, b) = (data[i - 1], data[i]);
		out.extend([cmp::max(a, b), cmp::min(a, b), cmp::max(a as i8, b as i8) as u8]);
		let (s, t) = (a.to_string(), (b / 2).to_string());
		out.extend([cmp::max(s.clone(), t.clone()).len() as u8, cmp::min(s, t).len() as u8]);
		let (x, y) = (item(2 * i, a), item(2 * i + 1, b));
		out.push(cmp::max(&x, &y).tag);
		out.push(cmp::min(&x, &y).tag);
		out.push(cmp::max(x, y).tag);
		let pair = || (item(2 * i, a), item(2 * i + 1, b));
		let (x, y) = pair();
		out.push(cmp::min(x, y).tag);
		let (x, y) = pair();
		out.push(cmp::max_by(x, y, |l, r| l.key.cmp(&r.key).reverse()).tag);
		let (x, y) = pair();
		out.push(cmp::min_by(x, y, |l, r| (l.key / 2).cmp(&(r.key / 2))).tag);
		let (x, y) = pair();
		out.push(cmp::max_by_key(x, y, |v| item(v.tag as usize + 100, v.key / 2)).tag);
		let (x, y) = pair();
		out.push(cmp::min_by_key(x, y, |v| Reverse(v.key)).tag);
		flush(out);
	}
}

/// The `is_sorted` family of slices and iterators of items, bytes, and strings, as given
/// and sorted: each test std makes, and each item or key it drops.
fn sorted(data: &[u8], out: &mut Vec<u8>) {
	let mut ascending = data.to_vec();
	ascending.sort_by_key(|b| b % 8);
	for bytes in [data, &ascending] {
		let items = || bytes.iter().enumerate().map(|(i, &b)| item(i, b));
		let v: Vec<Item> = items().collect();
		out.push(v.is_sorted() as u8);
		out.push(v.is_sorted_by(|a, b| a.key >= b.key) as u8);
		out.push(v.is_sorted_by_key(|x| item(x.tag as usize + 100, x.key / 2)) as u8);
		out.push(v.iter().is_sorted() as u8);
		out.push(v[..v.len() / 2].iter().is_sorted_by_key(|x| x.key) as u8);
		drop(v);
		flush(out);
		out.push(items().is_sorted() as u8);
		out.push(items().is_sorted_by(|a, b| a.tag > b.tag || a.key <= b.key) as u8);
		out.push(items().is_sorted_by_key(|x| Reverse(x.key)) as u8);
		out.push(items().collect::<Vec<_>>().into_iter().is_sorted() as u8);
		flush(out);
		// A function that owns an item drops as std's `all` drops it: before the last item
		// and the iterator.
		let owned = || item(200, 0);
		let (g, h, k, l) = (owned(), owned(), owned(), owned());
		out.push(items().is_sorted_by(move |a, b| (&g).key <= a.key && a.key <= b.key) as u8);
		let v = items().collect::<Vec<_>>();
		out.push(v.into_iter().is_sorted_by(move |a, b| a.key <= b.key + (&h).key) as u8);
		out.push(items().is_sorted_by_key(move |x| x.key + (&k).key) as u8);
		let key = move |x: Item| x.key + (&l).key;
		out.push(items().collect::<Vec<_>>().into_iter().is_sorted_by_key(&key) as u8);
		out.push(items().collect::<Vec<_>>().into_iter().is_sorted_by_key(Box::new(&key)) as u8);
		flush(out);
		let strings: Vec<String> = bytes.iter().map(|b| (b % 16).to_string()).collect();
		out.extend([
			bytes.is_sorted() as u8,
			bytes.is_sorted_by(|a, b| a / 4 <= b / 4) as u8,
			bytes.is_sorted_by_key(|&b| b % 8) as u8,
			bytes.iter().is_sorted() as u8,
			strings.is_sorted() as u8,
			strings.iter().map(String::len).is_sorted() as u8,
			bytes.iter().map(|&b| b as f64 / 3.0).is_sorted() as u8,
		]);
	}
}

/// A heap of items changed by each pair of bytes, an operation and its operand: each
/// comparison its sifts make, which of equal items it gives back, and when each drops.
fn heaps(data: &[u8], out: &mut Vec<u8>) {
	let mut heap = BinaryHeap::new();
	let mut other = BinaryHeap::with_capacity(4);
	let mut ops = data.iter().copied().enumerate();
	while let Some((i, op)) = ops.next() {
		let v = ops.next().map_or(0, |(_, v)| v);
		match op % 12 {
			0..=2 => heap.push(item(i, v)),
			3 => out.push(heap.pop().map_or(0xff, |x| x.tag)),
			// `DerefMut` sifts the top once it drops, but of a heap of one, and forgotten,
			// leaves the top alone in the heap.
			4 => {
				if let Some(mut top) = heap.peek_mut() {
					top.key = v % 8;
					if v & 0x10 != 0 {
						std::mem::forget(top);
					}
				}
			}
			// `Deref` alone sifts nothing, and `PeekMut::pop` pops.
			5 => {
				if let Some(top) = heap.peek_mut() {
					match v % 3 {
						0 => out.push(PeekMut::pop(top).tag),
						1 => out.push(top.tag),
						_ => {
							let mut top = top;
							top.tag ^= 0x40;
							out.push(PeekMut::pop(top).tag);
						}
					}
				}
			}
			6 => other.push(item(i, v)),
			7 => heap.append(&mut other),
			8 => heap.retain(|x| x.key != v % 8),
			9 => heap.extend(data.iter().take(v as usize % 6).map(|&b| item(i + 50, b))),
			10 => {
				let sorted = std::mem::take(&mut heap).into_sorted_vec();
				out.extend(sorted.iter().map(|x| x.tag));
				flush(out);
				heap = BinaryHeap::from(sorted);
			}
			_ => {
				out.extend(heap.iter().map(|x| x.tag));
				out.extend([
					heap.peek().map_or(0xff, |x| x.tag),
					heap.len() as u8,
					heap.is_empty() as u8,
				]);
			}
		}
		flush(out);
	}
	let mut drain = heap.drain();
	out.push(drain.next().map_or(0xff, |x| x.tag));
	drop(drain);
	flush(out);
	let mut collected: BinaryHeap<Item> = data.iter().enumerate().map(|(i, &b)| item(i, b)).collect();
	out.push(collected.pop().map_or(0xff, |x| x.tag));
	let mut rest = collected.into_iter();
	out.push(rest.next().map_or(0xff, |x| x.tag));
	drop(rest);
	flush(out);
	other.clear();
	flush(out);
}

/// Heaps of values with no comparison of their own: bytes, a min-heap of them, strings,
/// and tuples, built every way, and their vectors, slices, clones, and debug form.
fn values(data: &[u8], out: &mut Vec<u8>) {
	let mut bytes = BinaryHeap::from(data.to_vec());
	bytes.push(data.len() as u8);
	out.extend(bytes.as_slice());
	out.extend(bytes.clone().into_sorted_vec());
	out.extend(format!("{bytes:?}").bytes());
	let v: Vec<u8> = bytes.into();
	out.extend(v);
	let mut least: BinaryHeap<Reverse<u8>> = data.iter().map(|&b| Reverse(b)).collect();
	least.extend(data.iter().take(3).map(|&b| Reverse(b / 2)));
	while let Some(Reverse(b)) = least.pop() {
		out.push(b);
	}
	let strings: BinaryHeap<String> =
		BinaryHeap::from([String::from("pear"), String::from("apple"), data.len().to_string()]);
	let mut copy = strings.clone();
	copy.extend(data.iter().map(|b| (b % 10).to_string()));
	copy.retain(|s| s.len() > 1 || s.as_str() < "5");
	out.extend(copy.into_vec().iter().flat_map(|s| s.bytes()));
	out.extend(strings.into_iter().map(|s| s.len() as u8));
	let mut pairs: BinaryHeap<(u8, u8)> = BinaryHeap::new();
	pairs.extend(data.iter().map(|&b| (b % 4, b / 64)));
	pairs.extend(&[(data.len() as u8 % 4, 9)]);
	if let Some(mut top) = pairs.peek_mut() {
		top.1 = top.1.wrapping_add(1);
	}
	if let Some(mut top) = pairs.peek_mut() {
		*top = (0, 0);
	}
	out.extend(pairs.into_sorted_vec().into_iter().flat_map(|(a, b)| [a, b]));
}

/// Byte arrays, slices, and vectors, of the same length and of others, ordered as std
/// orders them, by their bytes and then by length: directly, through `Ord`'s defaults, as
/// keys and items, and in tuples, owned and borrowed.
pub fn bytes(data: &[u8], out: &mut Vec<u8>) {
	for w in data.windows(2) {
		let (a, b) = ([w[0] % 4, w[1] % 4], [w[1] % 4, w[0] % 2]);
		out.extend([(a < b) as u8, (a <= b) as u8, (a > b) as u8, (a >= b) as u8]);
		out.extend([a.cmp(&b) as u8, a.partial_cmp(&b).unwrap() as u8]);
		out.extend([cmp::max(a, b)[0], a.min(b)[1], a.clamp([1, 1], [2, 2])[1]]);
		let pair = (a, b[0]);
		out.push((pair < (b, a[1])) as u8);
	}
	// Each prefix against each suffix: a shorter one that is a prefix of the other is less.
	let cut = data.len().min(6);
	for i in 0..=cut {
		for j in 0..=cut {
			let (s, t) = (&data[..i], &data[j..cut]);
			out.extend([s.cmp(t) as u8, (s < t) as u8, (s >= t) as u8]);
			let (v, u) = (s.to_vec(), t.to_vec());
			out.extend([v.cmp(&u) as u8, (v > u) as u8, (v <= u) as u8]);
		}
	}
	let mut arrays: Vec<[u8; 2]> = data
		.chunks(2)
		.map(|c| [c[0] % 4, c.get(1).map_or(0, |b| b % 4)])
		.collect();
	out.extend(arrays.iter().max().map_or([9, 9], |a| *a));
	out.extend(arrays.iter().min().map_or([9, 9], |a| *a));
	let mut heap: BinaryHeap<[u8; 2]> = arrays.iter().copied().collect();
	while let Some(a) = heap.pop() {
		out.extend(a);
	}
	let keys: BTreeMap<[u8; 2], usize> = arrays.iter().copied().enumerate().map(|(i, a)| (a, i)).collect();
	out.extend(keys.into_iter().flat_map(|(k, i)| [k[0], k[1], i as u8]));
	arrays.sort();
	out.extend(arrays.iter().flatten());
	let mut slices: Vec<&[u8]> = data.chunks(3).collect();
	slices.sort_by(|a, b| b.cmp(a));
	out.extend(slices.iter().map(|s| s.len() as u8));
	let mut vecs: Vec<(Vec<u8>, usize)> = data.chunks(3).enumerate().map(|(i, c)| (c.to_vec(), i)).collect();
	vecs.sort();
	out.extend(vecs.iter().map(|(_, i)| *i as u8));
	let mut refs: Vec<(&[u8; 2], u8)> = Vec::new();
	for w in data.windows(2) {
		if let Some((a, _)) = w.split_first_chunk::<2>() {
			refs.push((a, w[0] / 64));
		}
	}
	refs.sort();
	out.extend(refs.iter().map(|(a, n)| a[0] ^ n));
}

/// Ordered by `PartialOrd` by its key, but not at all where either key is 7, and by `Ord`
/// the other way, so which of the two std calls shows. Each comparison logs which it is and
/// the two tags.
struct Part {
	key: u8,
	tag: u8,
}

fn part(tag: usize, key: u8) -> Part {
	Part {
		key: key % 8,
		tag: tag as u8,
	}
}

impl PartialEq for Part {
	fn eq(&self, other: &Self) -> bool {
		log([b'=', self.tag, other.tag]);
		self.key == other.key
	}
}

impl Eq for Part {}

impl PartialOrd for Part {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		log([b'p', self.tag, other.tag]);
		match self.key == 7 || other.key == 7 {
			true => None,
			false => self.key.partial_cmp(&other.key),
		}
	}
}

impl Ord for Part {
	fn cmp(&self, other: &Self) -> Ordering {
		log([b'c', self.tag, other.tag]);
		other.key.cmp(&self.key)
	}
}

/// Tuples by their fields in order, of `Part`s beside integers, vectors, and floats, which
/// order as nothing at NaN: of one field, two, and twelve, through a generic function, as
/// an array's, a vector's, an `Option`'s, a `Result`'s, and a `Reverse`'s, by `max`,
/// `min`, `clamp`, and their `_by` forms, and as `sort_by` orders them, whose comparisons
/// are the runtime's own.
pub fn tuples(data: &[u8], out: &mut Vec<u8>) {
	fn ranked<T: Ord>(a: &(&Part, T), b: &(&Part, T)) -> Ordering {
		a.cmp(b)
	}
	let float = |b: u8| if b % 5 == 0 { f64::NAN } else { f64::from(b % 4) };
	let parts: Vec<Part> = data.iter().enumerate().map(|(i, &b)| part(i, b)).collect();
	for (i, w) in data.windows(2).enumerate() {
		let (a, b) = (&parts[i], &parts[i + 1]);
		let (x, y) = (w[0] % 3, w[1] % 3);
		let orders = [
			(a,).cmp(&(b,)),
			(a, x).cmp(&(b, y)),
			(x, a).cmp(&(y, b)),
			ranked(&(a, x), &(b, y)),
			ranked(&(a, vec![x]), &(b, vec![y])),
		];
		out.extend(orders.map(|o| o as i8 as u8));
		flush(out);
		let s = (x, 0u8, 1u8, 2u8, 3u8, 4u8, 5u8, 6u8, 7u8, 8u8, 9u8, a);
		let t = (y, 0u8, 1u8, 2u8, 3u8, 4u8, 5u8, 6u8, 7u8, 8u8, 9u8, b);
		out.extend([s.cmp(&t) as i8 as u8, order(s.partial_cmp(&t)), u8::from(s < t)]);
		flush(out);
		let (fx, fy) = (float(w[0]), float(w[1]));
		out.extend([
			order((a, fx).partial_cmp(&(b, fy))),
			order((fx, a).partial_cmp(&(fy, b))),
			order((x, fx).partial_cmp(&(y, fy))),
		]);
		out.extend([(a, fx) < (b, fy), (fx, x) >= (fy, y)].map(u8::from));
		flush(out);
		let orders = [
			[(a, x)].cmp(&[(b, y)]),
			vec![(x, a)].cmp(&vec![(y, b)]),
			Some((a, x)).cmp(&Some((b, y))),
			Ok::<_, u8>((a,)).cmp(&Ok((b,))),
			Reverse((a, x)).cmp(&Reverse((b, y))),
		];
		out.extend(orders.map(|o| o as i8 as u8));
		flush(out);
		let (max, min) = ((a, x).max((b, y)), (x, a).min((y, b)));
		let clamped = (x, a).clamp((0, b), (2, b));
		out.extend([max.0.tag, max.1, min.0, min.1.tag, clamped.0, clamped.1.tag]);
		flush(out);
		let total = |p: &(f64, u8), q: &(f64, u8)| p.partial_cmp(q).unwrap_or(Ordering::Less);
		let (p, q) = ((fx, x), (fy, y));
		let by = [cmp::max_by(p, q, total), cmp::min_by(p, q, total)];
		out.extend(by.iter().flat_map(|&(f, n)| [if f.is_nan() { 9 } else { f as u8 }, n]));
	}
	let mut pairs: Vec<(u8, &Part)> = parts.iter().map(|p| (p.tag % 3, p)).collect();
	pairs.sort_by(|p, q| (p.0, p.1).cmp(&(q.0, q.1)));
	out.extend(pairs.iter().map(|p| p.1.tag));
	LOG.with_borrow_mut(Vec::clear);
	let mut floats: Vec<(f64, u8)> = data.iter().map(|&b| (f64::from(b % 4), b)).collect();
	floats.sort_by(|p, q| p.partial_cmp(q).expect("no NaN"));
	out.extend(floats.iter().map(|p| p.1));
}

fn order(o: Option<Ordering>) -> u8 {
	o.map_or(9, |o| o as i8 as u8)
}

/// A deque of `f` of each byte and its index: straight, or with its first half pushed to the
/// front in reverse, so it wraps around its buffer.
fn deque<T>(s: &[u8], wraps: bool, f: impl Fn(usize, u8) -> T) -> VecDeque<T> {
	let at = if wraps { s.len() / 2 } else { 0 };
	let (front, back) = s.split_at(at);
	let mut d: VecDeque<T> = back.iter().enumerate().map(|(i, &b)| f(i + at, b)).collect();
	for (i, &b) in front.iter().enumerate().rev() {
		d.push_front(f(i, b));
	}
	d
}

/// `Result`s and `Poll`s, by variant, `Ok` before `Err` and `Ready` before `Pending`, and
/// then by what each holds, and deques as sequences of their items, however each one's
/// ring buffer wraps: of parts, which log where std stops, and of floats, whose NaN orders
/// as nothing. Then each as a B-tree key.
pub fn held(data: &[u8], out: &mut Vec<u8>) {
	let result = |i: usize, b: u8| match b & 0x80 {
		0 => Ok(part(i, b)),
		_ => Err(part(i, b >> 3)),
	};
	let float = |b: u8| {
		let x = if b % 5 == 0 { f64::NAN } else { f64::from(b % 4) };
		if b & 1 == 0 { Ok(x) } else { Err(x) }
	};
	let poll = |b: u8| if b % 3 == 0 { Poll::Pending } else { Poll::Ready(b % 4) };
	for (i, w) in data.windows(2).enumerate() {
		let (a, b) = (result(2 * i, w[0]), result(2 * i + 1, w[1]));
		out.extend([order(a.partial_cmp(&b)), a.cmp(&b) as i8 as u8]);
		out.extend([a < b, a <= b, a > b, a >= b, a == b].map(u8::from));
		out.extend([order(Some(&a).partial_cmp(&Some(&b))), (&a,).cmp(&(&b,)) as i8 as u8]);
		out.extend([[&a, &b] < [&b, &a], vec![&a] >= vec![&b]].map(u8::from));
		flush(out);
		let (x, y) = (float(w[0]), float(w[1]));
		out.extend([order(x.partial_cmp(&y)), order(Some(x).partial_cmp(&Some(y)))]);
		out.extend([x < y, x <= y, x > y, x >= y, x == y, (x, 1) < (y, 0)].map(u8::from));
		let (p, q) = (poll(w[0]), poll(w[1]));
		out.extend([
			p.cmp(&q) as i8 as u8,
			order(p.partial_cmp(&q)),
			p.cmp(&Poll::Pending) as i8 as u8,
		]);
		out.extend([p < q, p >= q, p == q, q > Poll::Ready(2), Some(p) != Some(q)].map(u8::from));
		out.push(order(Poll::Ready(x).partial_cmp(&Poll::Ready(y))));
	}
	// Each prefix, straight, against each suffix, wrapped: a shorter one that is a prefix of
	// the other is less.
	let cut = data.len().min(6);
	for i in 0..=cut {
		for j in 0..=cut {
			let (d, e) = (
				deque(&data[..i], false, part),
				deque(&data[j..cut], true, |i, b| part(i + 50, b)),
			);
			out.extend([
				order(d.partial_cmp(&e)),
				d.cmp(&e) as i8 as u8,
				order(e.partial_cmp(&d)),
			]);
			out.extend([d < e, d >= e, e > d, d == e].map(u8::from));
			out.extend([
				order(Some(&d).partial_cmp(&Some(&e))),
				vec![&e].cmp(&vec![&d]) as i8 as u8,
			]);
			let floats = |d: &VecDeque<Part>| -> VecDeque<f64> {
				d.iter()
					.map(|p| if p.key == 7 { f64::NAN } else { f64::from(p.key) })
					.collect()
			};
			out.push(order(floats(&d).partial_cmp(&floats(&e))));
			let (r, s) = (Ok::<_, u8>(d), Err::<VecDeque<Part>, _>(e.len() as u8));
			out.extend([order(r.partial_cmp(&s)), (r < s) as u8]);
			flush(out);
		}
	}
	// Keys whose comparisons log nothing, since how often a B-tree calls `Ord` is the runtime's.
	let results: BTreeMap<Result<u8, i8>, usize> = data
		.iter()
		.enumerate()
		.map(|(i, &b)| (if b & 1 == 0 { Ok(b / 2 % 8) } else { Err(b as i8 >> 4) }, i))
		.collect();
	for (k, &i) in &results {
		out.extend(match k {
			Ok(v) => [0, *v, i as u8],
			Err(e) => [1, *e as u8, i as u8],
		});
	}
	out.extend(results.get(&Ok(3)).map(|&i| i as u8));
	out.extend(results.get(&Err(-1)).map(|&i| i as u8));
	let mut deques = BTreeMap::new();
	for (i, c) in data.chunks(3).enumerate() {
		let d = deque(c, i % 2 == 1, |_, b| b % 4);
		let key = if c[0] & 1 == 0 { Ok(d) } else { Err(poll(c[0])) };
		out.extend(deques.insert(key, i).map(|i| i as u8));
	}
	out.extend(deques.values().map(|&i| i as u8));
	let probe = deque(&data[..data.len().min(3)], true, |_, b| b % 4);
	out.push(deques.contains_key(&Ok(probe)) as u8);
	let polls: BTreeSet<Poll<u8>> = data.iter().map(|&b| poll(b)).collect();
	out.extend(polls.iter().map(|p| match p {
		Poll::Ready(v) => *v,
		Poll::Pending => 9,
	}));
	let mut sorted: Vec<Result<u8, i8>> = results.into_keys().rev().collect();
	sorted.sort();
	out.push(sorted.is_sorted() as u8);
	out.extend(sorted.iter().max().map(|r| r.is_ok() as u8));
}

/// A key whose `Ord` orders by `key`, and whose `PartialOrd` and `PartialEq` judge by
/// what `mode` picks: all three agree, or `lt` reverses `cmp`, or `==` tells apart keys
/// `cmp` finds equal, or `lt` and `==` see only half the key, or each judges its own way.
/// Each call logs which it is and the two tags.
#[derive(Clone, Copy)]
struct Torn {
	key: u8,
	tag: u8,
	mode: u8,
}

fn torn(tag: usize, key: u8, mode: u8) -> Torn {
	Torn {
		key: key % 8,
		tag: tag as u8,
		mode,
	}
}

impl Torn {
	/// What `PartialOrd` orders by.
	fn rank(&self) -> i16 {
		match self.mode {
			1 => -i16::from(self.key),
			3 | 4 => i16::from(self.key / 2),
			_ => i16::from(self.key),
		}
	}

	/// What `PartialEq` compares.
	fn class(&self) -> u16 {
		match self.mode {
			2 => u16::from(self.key) << 8 | u16::from(self.tag),
			3 => u16::from(self.key / 2),
			4 => u16::from(self.tag % 2),
			_ => u16::from(self.key),
		}
	}
}

impl PartialEq for Torn {
	fn eq(&self, other: &Self) -> bool {
		log([b'=', self.tag, other.tag]);
		self.class() == other.class()
	}

	fn ne(&self, other: &Self) -> bool {
		log([b'!', self.tag, other.tag]);
		self.class() != other.class()
	}
}

impl Eq for Torn {}

impl PartialOrd for Torn {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		log([b'p', self.tag, other.tag]);
		Some(self.rank().cmp(&other.rank()))
	}

	fn lt(&self, other: &Self) -> bool {
		log([b'<', self.tag, other.tag]);
		self.rank() < other.rank()
	}
}

impl Ord for Torn {
	fn cmp(&self, other: &Self) -> Ordering {
		log([b'c', self.tag, other.tag]);
		self.key.cmp(&other.key)
	}
}

/// Zero-sized, equal by `cmp` and never by `==`.
#[derive(Clone, Copy)]
struct Apart;

impl PartialEq for Apart {
	fn eq(&self, _: &Self) -> bool {
		log([b'=', 0, 0]);
		false
	}
}

impl Eq for Apart {}

impl PartialOrd for Apart {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(other))
	}

	fn lt(&self, _: &Self) -> bool {
		log([b'<', 0, 0]);
		false
	}
}

impl Ord for Apart {
	fn cmp(&self, _: &Self) -> Ordering {
		log([b'c', 0, 0]);
		Ordering::Equal
	}
}

/// `collect` and `from` into B-trees of `Torn` keys whose impls disagree as `kind` picks:
/// a set's sort tests `lt`, a map's `cmp`, and each keeps the last of each run `!=` finds
/// equal. Of at most 16 keys, of every byte, and of one key over and over: which keys and
/// values each keeps, in its order, and each call std makes but of more than 16, which
/// the runtime's sort makes in an order of its own. Then `extend`, which inserts each, and
/// the set operators, which build as `collect` does from what they yield, of sets that
/// `insert` built. Then zero-sized keys, which std's sort leaves as they are.
pub fn btrees(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let mode = kind % 5;
	let keys = |bytes: &[u8]| -> Vec<Torn> { bytes.iter().enumerate().map(|(i, &b)| torn(i, b, mode)).collect() };
	let short = &data[..data.len().min(16)];
	let runs = [
		keys(short),
		keys(data),
		short.iter().enumerate().map(|(i, _)| torn(i, 5, mode)).collect(),
	];
	for keys in runs {
		let logged = keys.len() <= 16;
		let set: BTreeSet<Torn> = keys.iter().copied().collect();
		out.push(set.len() as u8);
		out.extend(set.iter().map(|k| k.tag));
		let map: BTreeMap<Torn, u8> = keys.iter().map(|k| (*k, k.tag ^ 0x80)).collect();
		out.push(map.len() as u8);
		out.extend(map.iter().flat_map(|(k, &v)| [k.tag, v]));
		if let [a, b, c, ..] = *keys.as_slice() {
			out.extend(BTreeSet::from([a, b, c]).iter().map(|k| k.tag));
			out.extend(
				BTreeMap::from([(a, 1), (b, 2), (c, 3)])
					.iter()
					.flat_map(|(k, &v)| [k.tag, v]),
			);
			out.extend(BTreeSet::from([a]).iter().map(|k| k.tag));
		}
		out.push(BTreeSet::<Torn>::from([]).len() as u8);
		// std's own types order and compare as what they hold does.
		let chunks: BTreeSet<Vec<Torn>> = keys.chunks(2).map(<[Torn]>::to_vec).collect();
		out.extend(chunks.iter().flat_map(|c| c.iter().map(|k| k.tag)));
		if logged {
			flush(out);
		}
		// `insert` of each, whose searches call `cmp` in the order of the runtime's tree.
		let mut extended = BTreeSet::new();
		extended.extend(keys.iter().copied());
		out.extend(extended.iter().map(|k| k.tag));
		let mut entries = BTreeMap::new();
		entries.extend(keys.iter().map(|k| (*k, k.tag)));
		out.extend(entries.iter().flat_map(|(k, &v)| [k.tag, v]));
		let (mut left, mut right) = (BTreeSet::new(), BTreeSet::new());
		let (l, r) = keys.split_at(keys.len() / 2);
		left.extend(l.iter().copied());
		right.extend(r.iter().copied());
		LOG.with_borrow_mut(Vec::clear);
		out.extend((&left | &right).iter().map(|k| k.tag));
		out.extend((&left ^ &right).iter().map(|k| k.tag));
		if logged {
			flush(out);
		}
		out.extend((&left & &right).iter().map(|k| k.tag));
		out.extend((&left - &right).iter().map(|k| k.tag));
		LOG.with_borrow_mut(Vec::clear);
	}
	let apart = vec![Apart; short.len()];
	let set: BTreeSet<Apart> = apart.iter().copied().collect();
	let units: BTreeMap<Apart, ()> = apart.iter().map(|&k| (k, ())).collect();
	let map: BTreeMap<Apart, u8> = apart.iter().enumerate().map(|(i, &k)| (k, i as u8)).collect();
	out.extend([set.len() as u8, units.len() as u8, map.len() as u8]);
	out.extend(map.values());
	flush(out);
	// `PhantomData` compares nothing, so keys holding one agree whatever it names, and
	// each value but the last of equal keys drops.
	let phantoms: BTreeMap<Option<PhantomData<Torn>>, Item> = data
		.iter()
		.enumerate()
		.map(|(i, &b)| ((b % 2 == 0).then_some(PhantomData), item(i, b)))
		.collect();
	out.extend(phantoms.values().map(|x| x.tag));
	drop(phantoms);
	flush(out);
}
