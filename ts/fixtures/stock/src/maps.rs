//! `hashbrown`'s tables, which translation maps to the runtime's map as it maps std's, and
//! `hashlink`'s, which translate from their source over a `HashTable`. A hash table's
//! iteration order is unspecified, so only what does not depend on it is written, but a
//! linked map's order is its own, and written as it is.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;

use hashbrown::hash_table::{Entry, HashTable};
use hashbrown::{HashMap, HashSet};
use hashlink::LinkedHashMap;

/// A length, capacity, or value, as four bytes.
fn put(out: &mut Vec<u8>, n: usize) {
	out.extend_from_slice(&(n as u32).to_le_bytes());
}

/// `hashbrown::HashMap` and `HashSet` by its own hasher, of keys that are JS primitives
/// and keys that are not, through each pair of bytes: a step and its key.
pub fn map(data: &[u8], out: &mut Vec<u8>) {
	let mut own: HashMap<u16, u32> = HashMap::new();
	let mut pairs: HashMap<(u8, u8), u32> = HashMap::default();
	let mut set: HashSet<String> = HashSet::with_capacity(data.len() / 4);
	for step in data.chunks(2) {
		let (op, arg) = (step[0], step.get(1).copied().unwrap_or(0));
		let key = u16::from(arg % 16);
		let pair = (arg % 4, arg % 3);
		match op % 10 {
			0 | 1 => {
				let old = own.insert(key, u32::from(op));
				put(out, old.map_or(0x100, |v| v as usize));
				let old = pairs.insert(pair, u32::from(op));
				put(out, old.map_or(0x100, |v| v as usize));
			}
			2 => {
				put(out, own.remove(&key).map_or(0x100, |v| v as usize));
				put(out, pairs.remove(&pair).map_or(0x100, |v| v as usize));
			}
			3 => {
				out.push(own.contains_key(&key) as u8);
				put(out, pairs.get(&pair).map_or(0x100, |&v| v as usize));
			}
			4 => {
				if let Some(v) = own.get_mut(&key) {
					*v += 1;
				}
				*pairs.entry(pair).or_insert(7) += u32::from(arg);
			}
			5 => {
				own.retain(|k, v| (*k + *v as u16) % 3 != 0);
				pairs.retain(|k, _| k.0 != arg % 4);
			}
			6 => {
				out.push(set.insert(format!("{key}")) as u8);
				out.push(set.contains(&format!("{}", key + 1)) as u8);
			}
			7 => out.push(set.remove(&format!("{key}")) as u8),
			8 => {
				let removed = own.remove_entry(&key);
				put(out, removed.map_or(0x100, |(k, v)| usize::from(k) + v as usize));
			}
			_ => {
				let drained: u32 = own.drain().map(|(k, v)| u32::from(k) + v).sum();
				put(out, drained as usize);
			}
		}
		put(out, own.len());
		put(out, pairs.len());
		put(out, set.len());
	}
	let mut own: Vec<(u16, u32)> = own.into_iter().collect();
	own.sort();
	for (k, v) in own {
		put(out, usize::from(k));
		put(out, v as usize);
	}
	let mut pairs: Vec<((u8, u8), u32)> = pairs.iter().map(|(k, v)| (*k, *v)).collect();
	pairs.sort();
	for ((a, b), v) in pairs {
		out.extend_from_slice(&[a, b]);
		put(out, v as usize);
	}
	let mut set: Vec<String> = set.into_iter().collect();
	set.sort();
	out.extend_from_slice(set.join(",").as_bytes());
}

/// The hash of `key` that `kind` picks: one past 2^53 that sets apart a few keys, the
/// same for every key, or each key's own.
fn hash(kind: u8, key: u16) -> u64 {
	match kind % 3 {
		0 => u64::from(key % 5).wrapping_mul(0x9e37_79b9_7f4a_7c15),
		1 => 7,
		_ => u64::from(key),
	}
}

/// `hashbrown::HashTable` of `(key, value)`, found by the hash `data`'s first byte picks
/// and the key's equality, through each pair of bytes after it: a step and its key.
pub fn table(data: &[u8], out: &mut Vec<u8>) {
	let Some((&kind, steps)) = data.split_first() else {
		return;
	};
	let mut t: HashTable<(u16, u32)> = HashTable::new();
	let rehash = |e: &(u16, u32)| hash(kind, e.0);
	let mut n: HashTable<u32> = HashTable::with_capacity(usize::from(kind % 8));
	for step in steps.chunks(2) {
		let (op, arg) = (step[0], step.get(1).copied().unwrap_or(0));
		let key = u16::from(arg % 16);
		let h = hash(kind, key);
		numbers(&mut n, kind, op, key, out);
		match op % 9 {
			0 | 1 => match t.find_entry(h, |e| e.0 == key) {
				Ok(mut e) => {
					e.get_mut().1 += u32::from(op);
					put(out, e.get().1 as usize);
				}
				Err(absent) => {
					let e = absent.into_table().insert_unique(h, (key, u32::from(op)), rehash);
					put(out, e.get().1 as usize);
				}
			},
			2 => match t.find_entry(h, |e| e.0 == key) {
				Ok(e) => {
					let ((k, v), vacant) = e.remove();
					put(out, usize::from(k) + v as usize);
					put(out, vacant.into_table().len());
				}
				Err(_) => put(out, 0x100),
			},
			3 => put(out, t.find(h, |e| e.0 == key).map_or(0x100, |e| e.1 as usize)),
			4 => {
				if let Some(e) = t.find_mut(h, |e| e.0 == key) {
					e.1 *= 2;
				}
			}
			5 => match t.entry(h, |e| e.0 == key, rehash) {
				Entry::Occupied(e) => put(out, e.into_mut().1 as usize),
				Entry::Vacant(e) => put(out, e.insert((key, 1)).into_mut().1 as usize),
			},
			6 => {
				let e = t.entry(h, |e| e.0 == key, rehash).or_insert((key, 3));
				put(out, e.get().1 as usize);
			}
			7 => t.retain(|e| {
				e.1 += 1;
				e.0 % 3 != u16::from(arg % 3)
			}),
			_ => {
				let sum: u32 = t.iter().map(|e| u32::from(e.0) + e.1).sum();
				put(out, sum as usize);
				let copy = t.clone();
				let mut drained: Vec<(u16, u32)> = t.drain().collect();
				drained.sort();
				for (k, v) in drained {
					put(out, usize::from(k));
					put(out, v as usize);
				}
				// The clone finds each item under the hash its original was given.
				put(out, copy.find(h, |e| e.0 == key).map_or(0x100, |e| e.1 as usize));
				put(out, copy.len());
				if arg & 1 == 1 {
					t = copy;
				}
			}
		}
		put(out, t.len());
		out.push(t.is_empty() as u8);
	}
	let mut left: Vec<(u16, u32)> = t.into_iter().collect();
	left.sort();
	for (k, v) in left {
		put(out, usize::from(k));
		put(out, v as usize);
	}
}

/// `HashTable<u32>`, whose items JS copies, so a `&mut` to one is its place: each item's
/// key in its high half and a count in its low half, changed by step `op` of `key`.
fn numbers(n: &mut HashTable<u32>, kind: u8, op: u8, key: u16, out: &mut Vec<u8>) {
	let h = hash(kind, key);
	let ours = |e: &u32| *e >> 16 == u32::from(key);
	let rehash = |e: &u32| hash(kind, (*e >> 16) as u16);
	match op % 6 {
		0 => {
			let e = n.entry(h, ours, rehash).and_modify(|e| *e += 1);
			put(out, *e.or_insert(u32::from(key) << 16).get() as usize);
		}
		1 => match n.find_mut(h, ours) {
			Some(e) => *e += 2,
			None => put(out, *n.insert_unique(h, u32::from(key) << 16, rehash).into_mut() as usize),
		},
		2 => {
			if let Ok(mut e) = n.find_entry(h, ours) {
				*e.get_mut() += 3;
				let e = e.into_mut();
				*e += 4;
			}
		}
		3 => n.retain(|e| {
			*e += 5;
			*e & 0xff < 200
		}),
		4 => {
			for e in n.iter_mut() {
				*e ^= 1;
			}
			for e in &mut *n {
				*e += 1;
			}
		}
		_ => {
			if let Entry::Occupied(mut e) = n.entry(h, ours, rehash) {
				*e.get_mut() += 6;
			}
		}
	}
	let mut items: Vec<u32> = n.iter().copied().collect();
	items.sort();
	for item in items {
		put(out, item as usize);
	}
}

/// What std's guarantees fix of a hash collection's capacity, `cap`, holding `len`: at
/// least `len` and `floor`, the most an operation just asked for.
fn holds(out: &mut Vec<u8>, len: usize, cap: usize, floor: usize) {
	put(out, len);
	out.push(u8::from(cap >= len.max(floor)));
}

/// The capacity of `hashbrown::HashMap`, `HashSet`, and `HashTable`, which follows std's
/// guarantees only, as each step of `data` grows, shrinks, or remakes them: at least
/// their length and what `with_capacity` and `reserve` asked for, no less past `shrink_to`
/// than it asked for of what there was, doubled as inserts grow it, none until one
/// allocates, and an allocation while it holds any.
pub fn capacity(data: &[u8], out: &mut Vec<u8>) {
	let Some((&first, steps)) = data.split_first() else {
		return;
	};
	let made = usize::from(first >> 2);
	let (mut m, mut s, mut t): (HashMap<u16, u64, RandomState>, HashSet<u8>, HashTable<u32>) = match first & 3 {
		0 => (HashMap::default(), HashSet::new(), HashTable::new()),
		_ => (
			HashMap::with_capacity_and_hasher(made, RandomState::new()),
			HashSet::with_capacity(made),
			HashTable::with_capacity(made),
		),
	};
	let floor = if first & 3 == 0 { 0 } else { made };
	for (cap, len) in [(m.capacity(), m.len()), (s.capacity(), s.len()), (t.capacity(), t.len())] {
		holds(out, len, cap, floor);
		out.push(u8::from(cap == 0 || first & 3 != 0));
	}
	let hasher = RandomState::new();
	let rehash = |v: &u32| hasher.hash_one(v);
	// Whether nothing was removed since each last grew, so an insert that grows it doubles it.
	let mut kept = [true; 3];
	for step in steps.chunks(2) {
		let (op, arg) = (step[0], step.get(1).copied().unwrap_or(0));
		let before = [m.capacity(), s.capacity(), t.capacity()];
		// What `reserve` asked for past the length, or what `shrink_to` kept.
		let (mut additional, mut kept_min) = (None, None);
		match op % 7 {
			0..=2 => {
				m.insert(u16::from(arg), u64::from(op));
				s.insert(arg);
				let v = u32::from(arg);
				if t.find(hasher.hash_one(v), |&o| o == v).is_none() {
					t.insert_unique(hasher.hash_one(v), v, rehash);
				}
			}
			3 => {
				m.remove(&u16::from(arg));
				s.remove(&arg);
				let v = u32::from(arg);
				if let Ok(e) = t.find_entry(hasher.hash_one(v), |&o| o == v) {
					e.remove();
				}
				kept = [false; 3];
			}
			4 => {
				let n = usize::from(arg % 40);
				m.reserve(n);
				s.reserve(n);
				t.reserve(n, rehash);
				additional = Some(n);
			}
			5 => {
				let min = usize::from(arg % 20);
				match arg & 0x80 {
					0 => {
						m.shrink_to(min);
						s.shrink_to(min);
						t.shrink_to(min, rehash);
						kept_min = Some(min);
					}
					_ => {
						m.shrink_to_fit();
						s.shrink_to_fit();
						t.shrink_to_fit(rehash);
					}
				}
				kept = [true; 3];
			}
			_ => {
				// Clearing keeps what it holds room for.
				m.clear();
				s.clear();
				t.clear();
				out.push(u8::from(m.capacity() >= before[0] && s.capacity() >= before[1] && t.capacity() >= before[2]));
			}
		}
		let lens = [m.len(), s.len(), t.len()];
		let caps = [m.capacity(), s.capacity(), t.capacity()];
		let sizes = [m.allocation_size(), s.allocation_size(), t.allocation_size()];
		for i in 0..3 {
			let floor = match (additional, kept_min) {
				(Some(n), _) => lens[i] + n,
				(_, Some(min)) => before[i].min(lens[i].max(min)),
				_ => 0,
			};
			holds(out, lens[i], caps[i], floor);
			// An insert that grows it doubles it, at least.
			let grew = op % 7 <= 2 && caps[i] > before[i];
			out.push(u8::from(!grew || !kept[i] || before[i] == 0 || caps[i] >= 2 * before[i]));
			if grew {
				kept[i] = true;
			}
			out.push(u8::from(lens[i] == 0 || sizes[i] > 0));
		}
	}
}

/// `hashlink::LinkedHashMap`, by std's hasher, whose order is its own: each step of
/// `data` and then every entry, from the front.
pub fn linked(data: &[u8], out: &mut Vec<u8>) {
	let mut m: LinkedHashMap<u16, u32, RandomState> = LinkedHashMap::with_hasher(RandomState::new());
	for step in data.chunks(2) {
		let (op, arg) = (step[0], step.get(1).copied().unwrap_or(0));
		let key = u16::from(arg % 12);
		match op % 8 {
			0 | 1 => put(out, m.insert(key, u32::from(op)).map_or(0x100, |v| v as usize)),
			2 => put(out, m.remove(&key).map_or(0x100, |v| v as usize)),
			3 => put(out, m.to_back(&key).map_or(0x100, |v| *v as usize)),
			4 => match arg & 1 {
				0 => put(out, m.pop_front().map_or(0x100, |(k, v)| usize::from(k) * 0x1000 + v as usize)),
				_ => put(out, m.pop_back().map_or(0x100, |(k, v)| usize::from(k) * 0x1000 + v as usize)),
			},
			5 => {
				put(out, m.front().map_or(0x100, |(k, _)| usize::from(*k)));
				put(out, m.back().map_or(0x100, |(k, _)| usize::from(*k)));
			}
			6 => put(out, m.insert_front(key, u32::from(op)).map_or(0x100, |v| v as usize)),
			_ => m.retain(|k, v| {
				*v += 1;
				*k % 3 != u16::from(arg % 3)
			}),
		}
		put(out, m.len());
		out.push(u8::from(m.capacity() >= m.len()));
	}
	while let Some((k, v)) = m.pop_front() {
		put(out, usize::from(k));
		put(out, v as usize);
	}
}

