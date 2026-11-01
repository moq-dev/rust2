//! Values that drop inside what std holds them in: an enum's, which has no
//! `[Symbol.dispose]` of its own, in each std type that drops what it holds, and generic
//! types that implement `Drop`. Each step logs every drop, so the log shows the order.

use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet, VecDeque, btree_map, hash_map};
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use kio::Consumer;

use crate::log::Log;
use crate::model::Guard;

/// A `Vec` of the items, pushed in order: `vec!` of values that are not `Copy` moves them
/// through a boxed array, which the translation lacks.
macro_rules! list {
	($($item:expr),* $(,)?) => {{
		let mut v = Vec::new();
		$(v.push($item);)*
		v
	}};
}

mod closures;
mod folds;
mod inside;
mod iters;
mod lists;
mod objects;
mod once;

/// A value of a type that drops, with no `[Symbol.dispose]` of its own.
pub enum Held {
	One(Guard),
	Two(Guard, Guard),
	Empty,
}

/// Logs its own drop before its fields', whatever it holds.
pub struct Wrap<T> {
	id: u32,
	log: Log,
	inner: T,
}

impl<T> Drop for Wrap<T> {
	fn drop(&mut self) {
		self.log.note(format!("w{} drop", self.id));
	}
}

/// A generic type without a `Drop` impl, dropping its fields in order.
pub struct Pair<T> {
	first: T,
	second: Guard,
}

/// Borrowed text or a value that drops, as moq-net's `path::Repr` is: generic over a
/// lifetime alone.
pub enum Text<'a> {
	Borrowed(&'a str),
	Owned(Guard),
}

/// A type generic over a lifetime alone, holding one that is.
pub struct Named<'a>(Text<'a>);

/// Run trait object scenario `arg % 4`: boxed, shared, as `dyn Any`, or as `dyn Fn*`.
pub fn dyns(log: &Log, id: u32, arg: u8) {
	match arg % 4 {
		0 => objects::boxed(log, id),
		1 => objects::shared(log, id),
		2 => objects::anys(log, id),
		_ => objects::fns(log, id),
	}
}

/// Run one-time cells' scenario `arg % 6`.
pub fn cells(log: &Log, id: u32, arg: u8) {
	once::run(log, id, arg);
}

/// Run scenario `arg % 16`: what every value std holds drops, where Rust drops it, and in
/// Rust's order.
pub fn run(log: &Log, id: u32, arg: u8) {
	let guard = |n: u32| Guard::new(id + n, log.clone());
	match arg % 16 {
		15 => elements(log, id),
		14 => closures::iterated(log, id),
		12 => closures::scoped(log, id),
		13 => closures::discarded(log, id),
		8 => iters::looped(log, id),
		9 => iters::collected(log, id),
		10 => iters::drained(log, id),
		11 => iters::extended(log, id),
		4 => ordered(log, id),
		5 => hashed(log, id),
		6 => options(log, id),
		7 => arcs(log, id),
		0 => {
			let held = list![Held::One(guard(1)), Held::Empty, Held::Two(guard(2), guard(3))];
			let arc = Arc::new(Held::One(guard(4)));
			let shared = arc.clone();
			drop(arc);
			log.note(format!("arc {}", Arc::strong_count(&shared)));
			// A borrowed count is the count when it was borrowed, though a clone follows.
			let counted = (&Arc::strong_count(&shared),);
			let count = counted.0;
			let cloned = shared.clone();
			log.note(format!("counted {}", *count));
			drop(cloned);
			let mutex = Mutex::new(list![Held::One(guard(5))]);
			let boxed = Box::new(Held::Two(guard(6), guard(7)));
			let maybe = Some(Box::new(Held::One(guard(8))));
			log.note(format!("held {} {}", held.len(), maybe.is_some()));
			drop(boxed);
			drop(mutex);
		}
		1 => {
			let wrapped = Wrap {
				id: id + 10,
				log: log.clone(),
				inner: Held::One(guard(11)),
			};
			let guarded = Wrap {
				id: id + 20,
				log: log.clone(),
				inner: guard(21),
			};
			let pair = Pair {
				first: Held::Two(guard(31), guard(32)),
				second: guard(33),
			};
			let pairs = list![
				Pair {
					first: guard(41),
					second: guard(42),
				},
				Pair {
					first: guard(43),
					second: guard(44),
				},
			];
			let _borrowed = Named(Text::Borrowed("text"));
			let _owned = Named(Text::Owned(guard(45)));
			let nested = Wrap {
				id: id + 50,
				log: log.clone(),
				inner: list![Wrap {
					id: id + 60,
					log: log.clone(),
					inner: Held::One(guard(61)),
				}],
			};
			drop(pair);
			log.note(format!(
				"wraps {} {} {}",
				wrapped.inner.is_some(),
				guarded.inner.id(),
				pairs[1].first.id() + pairs[0].second.id() + nested.inner.len() as u32
			));
		}
		2 => {
			let shared = Arc::new(Wrap {
				id: id + 70,
				log: log.clone(),
				inner: Pair {
					first: Held::One(guard(71)),
					second: guard(72),
				},
			});
			let clones = list![shared.clone(), shared.clone()];
			drop(shared);
			log.note(format!("clones {}", clones.len()));
			let early = bail(log, id, true);
			let late = bail(log, id + 2, false);
			log.note(format!("bailed {} {}", early.is_err(), late.is_err()));
		}
		_ => {
			let arc = Arc::new(Mutex::new(Held::Two(guard(81), guard(82))));
			let other = Arc::clone(&arc);
			drop(arc);
			let held = other.lock().unwrap().is_some();
			log.note(format!("locked {held}"));
			// The last `Arc` of a borrow drops nothing it points at.
			let lent = guard(83);
			let borrowed = Arc::new(&lent);
			let again = Arc::clone(&borrowed);
			drop(borrowed);
			drop(again);
			log.note(format!("lent {}", lent.id()));
		}
	}
}

/// Returns early through `?`, if it `fails`, with an enum's guards and a generic `Drop`
/// type live.
fn bail(log: &Log, id: u32, fails: bool) -> Result<u32, u32> {
	let held = list![Held::One(Guard::new(id + 90, log.clone()))];
	let wrap = Wrap {
		id: id + 91,
		log: log.clone(),
		inner: Held::Empty,
	};
	let n = failed(fails)?;
	log.note(format!("not bailed {} {}", held.len(), wrap.inner.is_some()));
	Ok(n)
}

fn failed(fails: bool) -> Result<u32, u32> {
	if fails { Err(0) } else { Ok(1) }
}

/// Holds an enum's guards in std types and a generic `Drop` type across an await,
/// which a cancel drops.
pub async fn held(mut rx: Consumer, guard: Guard) -> u64 {
	let log = guard.log().clone();
	let id = guard.id();
	let held = list![Held::One(Guard::new(id + 500, log.clone())), Held::Empty];
	let wrap = Wrap {
		id: id + 510,
		log: log.clone(),
		inner: Arc::new(Held::One(Guard::new(id + 520, log.clone()))),
	};
	let value = rx.recv().await.unwrap_or(0);
	log.note(format!("g{id} held {} {}", held.len(), wrap.inner.is_some()));
	value
}

impl Held {
	fn is_some(&self) -> bool {
		!matches!(self, Held::Empty)
	}
}

/// A key that logs its own drop, compared, ordered, and hashed by its id alone.
pub struct Key {
	id: u32,
	log: Log,
	/// Whether its `Hash` logs each call.
	counted: bool,
}

impl Key {
	fn new(id: u32, log: Log) -> Self {
		Self {
			id,
			log,
			counted: false,
		}
	}
}

impl Drop for Key {
	fn drop(&mut self) {
		self.log.note(format!("k{} drop", self.id));
		// Nothing reads a dropped key in Rust; a translation that did would find another.
		self.id = u32::MAX;
	}
}

impl PartialEq for Key {
	fn eq(&self, other: &Self) -> bool {
		self.id == other.id
	}
}

impl Eq for Key {}

impl PartialOrd for Key {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		Some(self.cmp(other))
	}
}

impl Ord for Key {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		self.id.cmp(&other.id)
	}
}

impl Hash for Key {
	fn hash<H: Hasher>(&self, state: &mut H) {
		if self.counted {
			self.log.note(format!("k{} hashed", self.id));
		}
		self.id.hash(state);
	}
}

/// The same key, logged.
impl Clone for Key {
	fn clone(&self) -> Self {
		self.log.note(format!("k{} cloned", self.id));
		Self {
			id: self.id,
			log: self.log.clone(),
			counted: self.counted,
		}
	}
}

/// What ordered collections drop of what they are given or hold, in order: a key
/// `insert` and `entry` find there already, the key `remove` takes out, the value
/// `or_insert` is given, and what `retain`, `truncate`, and `clear` discard.
fn ordered(log: &Log, id: u32) {
	let key = |n: u32| Key::new(id + n, log.clone());
	let guard = |n: u32| Guard::new(id + n, log.clone());
	let mut map = BTreeMap::new();
	map.insert(key(1), guard(1));
	map.insert(key(2), guard(2));
	map.insert(key(3), guard(3));
	let old = map.insert(key(1), guard(4));
	log.note(format!("replaced {}", old.is_some()));
	drop(old);
	let removed = map.remove(&key(2));
	log.note(format!("removed {}", removed.is_some()));
	match map.entry(key(3)) {
		btree_map::Entry::Occupied(e) => log.note(format!("occupied {}", e.remove().id())),
		btree_map::Entry::Vacant(_) => log.note(format!("vacant")),
	}
	if let btree_map::Entry::Vacant(e) = map.entry(key(5)) {
		log.note(format!("left {}", e.key().id));
	}
	let kept = map.entry(key(1)).or_insert(guard(6)).id();
	let added = map.entry(key(7)).or_insert(guard(7)).id();
	log.note(format!("or_insert {kept} {added}"));
	map.insert(key(8), guard(8));
	map.retain(|k, _| k.id % 2 == 0);
	let mut set = BTreeSet::new();
	set.insert(key(10));
	set.insert(key(11));
	log.note(format!("again {}", set.insert(key(10))));
	log.note(format!("gone {}", set.remove(&key(11))));
	set.insert(key(12));
	let mut queue = VecDeque::new();
	for n in 20..26 {
		queue.push_back(guard(n));
	}
	queue.push_front(guard(19));
	queue.retain(|g| g.id() % 3 != 0);
	queue.truncate(2);
	log.note(format!("queue {}", queue.len()));
	map.clear();
	set.clear();
	log.note(format!("left {} {} {}", map.len(), set.len(), queue.len()));
	// Past one node, whose branches bound their children by keys: a key `replace`, `take`,
	// or `remove` gives back drops, which a lookup through a branch still holding it would
	// misread.
	let sink = Log::new();
	let mut many = BTreeSet::new();
	for n in 0..400 {
		many.insert(Key::new(n, sink.clone()));
	}
	let found = |many: &BTreeSet<Key>| (0..400).filter(|&n| many.contains(&Key::new(n, sink.clone()))).count();
	for n in (0..400).step_by(3) {
		drop(many.replace(Key::new(n, sink.clone())));
	}
	log.note(format!("replaced {} {}", found(&many), many.len()));
	for n in (0..400).step_by(7) {
		drop(many.take(&Key::new(n, sink.clone())));
	}
	log.note(format!("taken {} {}", found(&many), many.len()));
	for n in (0..400).step_by(5) {
		many.remove(&Key::new(n, sink.clone()));
	}
	log.note(format!("removed {} {}", found(&many), many.len()));
}

/// The same of hash collections, whose order of drops std leaves unspecified: each key
/// and value logs to a sink, and the log reads how much it holds, in an order it ignores.
fn hashed(log: &Log, id: u32) {
	let sink = Log::new();
	let key = |n: u32| Key::new(id + n, sink.clone());
	let guard = |n: u32| Guard::new(id + n, sink.clone());
	let tally = |what: &str| {
		let text = sink.text();
		let sum: u32 = text.iter().map(|&b| u32::from(b)).sum();
		log.note(format!("{what}: {} {sum}", text.len()));
	};
	let mut map = HashMap::new();
	map.insert(key(1), guard(1));
	map.insert(key(2), guard(2));
	map.insert(key(3), guard(3));
	let old = map.insert(key(1), guard(4));
	tally("insert");
	drop(old);
	let removed = map.remove(&key(2));
	tally("remove");
	drop(removed);
	match map.entry(key(3)) {
		hash_map::Entry::Occupied(e) => drop(e.remove()),
		hash_map::Entry::Vacant(_) => log.note(format!("vacant")),
	}
	if let hash_map::Entry::Vacant(e) = map.entry(key(5)) {
		log.note(format!("left {}", e.key().id));
	}
	tally("entry");
	let kept = map.entry(key(1)).or_insert(guard(6)).id();
	let added = map.entry(key(7)).or_insert(guard(7)).id();
	log.note(format!("or_insert {kept} {added}"));
	tally("or_insert");
	map.insert(key(8), guard(8));
	map.retain(|k, _| k.id % 2 == 0);
	tally("retain");
	let mut set = HashSet::new();
	set.insert(key(10));
	set.insert(key(11));
	log.note(format!("again {}", set.insert(key(10))));
	log.note(format!("gone {}", set.remove(&key(11))));
	set.insert(key(12));
	set.retain(|k| k.id % 2 == 1);
	tally("set");
	// An entry hashes its key once, and what consumes it hashes nothing: two keys, so the
	// table never grows, which hashes each again.
	let counted = |n: u32| Key {
		id: id + n,
		log: sink.clone(),
		counted: true,
	};
	let mut counts = HashMap::new();
	counts.insert(counted(20), guard(20));
	drop(counts.entry(counted(20)).or_insert(guard(21)));
	drop(counts.entry(counted(22)).or_insert_with(|| guard(22)));
	drop(counts.entry(counted(20)).and_modify(|_| ()).or_insert(guard(23)));
	if let hash_map::Entry::Occupied(e) = counts.entry(counted(22)) {
		drop(e.remove());
	}
	tally("counted");
	map.clear();
	tally("clear");
	drop(set);
	drop(map);
	tally("end");
}

/// What std's functions on `Option`, `Result`, and `bool` drop of what they are given
/// rather than return: an unused error or fallback, after any closure they call.
fn options(log: &Log, id: u32) {
	let guard = |n: u32| Guard::new(id + n, log.clone());
	let some = Some(guard(1)).ok_or(guard(2));
	let none = None::<Guard>.ok_or(guard(3));
	log.note(format!("ok_or {} {}", some.is_ok(), none.is_ok()));
	let ok = Ok::<Guard, Guard>(guard(4)).ok();
	let err = Err::<Guard, Guard>(guard(5)).ok();
	let flipped = Ok::<Guard, Guard>(guard(6)).err();
	log.note(format!("ok {} {} {}", ok.is_some(), err.is_some(), flipped.is_some()));
	let kept = Some(guard(7)).unwrap_or(guard(8));
	let fallback = None.unwrap_or(guard(9));
	let value = Ok::<Guard, Guard>(guard(10)).unwrap_or(guard(11));
	let error = Err::<Guard, Guard>(guard(12)).unwrap_or(guard(13));
	log.note(format!(
		"unwrap_or {} {} {} {}",
		kept.id(),
		fallback.id(),
		value.id(),
		error.id()
	));
	let mapped = Some(id).map_or(guard(14), |n| {
		log.note(format!("mapped"));
		Guard::new(n + 15, log.clone())
	});
	let unmapped = None.map_or(guard(16), |n: u32| Guard::new(n, log.clone()));
	log.note(format!("map_or {} {}", mapped.id(), unmapped.id()));
	let then = some.is_ok().then_some(guard(17));
	let never = false.then_some(guard(18));
	log.note(format!("then_some {} {}", then.is_some(), never.is_some()));
	// Lowered from core's MIR.
	let kept = Some(guard(19)).filter(|g| g.id() % 2 == 0);
	let dropped = Some(guard(20)).filter(|_| false);
	let zipped = Some(guard(21)).zip(Some(guard(22)));
	let other = None.or_else(|| Some(guard(23)));
	let flag = Some(guard(24)).is_some_and(|g| g.id() > id);
	log.note(format!(
		"filter {} {} zip {} or_else {} is_some_and {flag}",
		kept.is_some(),
		dropped.is_some(),
		zipped.is_some(),
		other.is_some()
	));
	let chained = Ok::<Guard, Guard>(guard(25)).and_then(Err::<Guard, Guard>);
	let error = Err::<Guard, Guard>(guard(26)).map_or_else(|e| e.id(), |v| v.id());
	let cloned = Some(Arc::new(guard(27))).clone();
	log.note(format!(
		"and_then {} map_or_else {error} clone {}",
		chained.is_ok(),
		cloned.is_some()
	));
	// What a `&mut Option`'s methods drop: the value `get_or_insert` doesn't insert, the
	// closure `get_or_insert_with` doesn't call, and what `insert` replaces.
	let mut slot = None;
	slot.get_or_insert(guard(28));
	slot.get_or_insert(guard(29));
	let owned = guard(30);
	slot.get_or_insert_with(move || owned);
	let owned = guard(31);
	let mut empty = None;
	let made = empty.get_or_insert_with(move || owned).id();
	let replaced = slot.insert(guard(32)).id();
	log.note(format!("get_or_insert {made} {replaced}"));
	// Of a payload JS copies, which the methods lend as its place, as moq's `answered` is.
	let mut answered: Option<Result<Guard, u8>> = None;
	answered.get_or_insert(Ok(guard(33)));
	let kept = answered.get_or_insert(Ok(guard(34))).as_ref().map_or(0, Guard::id);
	*answered.insert(Err(3)) = Err(4);
	log.note(format!("answered {kept} {}", matches!(answered, Some(Err(4)))));
}

/// What std's accessors of elements that are each an `Option` of a value that drops give
/// and drop: `None` for no element, apart from the `Some(None)` of a `None` one, and each
/// value once, where Rust drops it.
fn elements(log: &Log, id: u32) {
	let guard = |n: u32| Guard::new(id + n, log.clone());
	let key = |n: u32| Key::new(id + n, log.clone());
	let ids = |o: &Option<Guard>| o.as_ref().map(Guard::id);
	let mut v = list![Some(guard(1)), None, Some(guard(2)), None];
	log.note(format!("vec {:?} {:?}", v.first().map(ids), v.last().map(ids)));
	log.note(format!("get {:?} {:?}", v.get(1).map(ids), v.get(4).map(ids)));
	// Over a `None`, which drops nothing, and over a guard, which drops it.
	if let Some(o) = v.last_mut() {
		*o = Some(guard(3));
	}
	if let Some(o) = v.first_mut() {
		*o = None;
	}
	// What the test changes in place it keeps, and what it doesn't take stays.
	let kept = v.pop_if(|o| {
		*o = None;
		false
	});
	let taken = v.pop_if(|o| {
		*o = Some(guard(12));
		true
	});
	log.note(format!("pop_if {} {:?}", kept.is_some(), taken.as_ref().map(ids)));
	drop(taken);
	while let Some(o) = v.pop() {
		log.note(format!("pop {:?}", ids(&o)));
	}
	log.note(format!("popped {:?}", v.pop().map(|o| ids(&o))));
	// A test that owns a guard drops it uncalled without an element, and else once called.
	let owned = guard(14);
	log.note(format!("uncalled {}", v.pop_if(move |_| owned.id() > 0).is_some()));
	v.push(None);
	let owned = guard(15);
	let called = v.pop_if(move |o| {
		log.note(format!("test {}", owned.id()));
		o.is_none()
	});
	log.note(format!("called {:?}", called.map(|o| ids(&o))));
	let mut q = VecDeque::new();
	q.push_back(None);
	q.push_back(Some(guard(4)));
	q.push_front(Some(guard(5)));
	q.push_back(None);
	log.note(format!(
		"deque {:?} {:?} {:?}",
		q.front().map(ids),
		q.back().map(ids),
		q.get(4).map(ids)
	));
	if let Some(o) = q.front_mut() {
		*o = None;
	}
	let removed = q.remove(2);
	log.note(format!(
		"remove {:?} {:?}",
		removed.as_ref().map(ids),
		q.remove(3).map(|o| ids(&o))
	));
	drop(removed);
	q.push_back(Some(guard(6)));
	q.push_back(Some(guard(13)));
	q.push_front(None);
	let back = q.swap_remove_back(0);
	log.note(format!(
		"swap_remove {:?} {:?} {:?}",
		back.as_ref().map(ids),
		q.swap_remove_front(9).map(|o| ids(&o)),
		q.swap_remove_front(1).map(|o| ids(&o))
	));
	drop(back);
	let front = q.pop_front();
	log.note(format!("front {:?}", front.as_ref().map(ids)));
	while let Some(o) = q.pop_back() {
		log.note(format!("back {:?}", ids(&o)));
	}
	log.note(format!(
		"emptied {:?} {:?}",
		q.pop_front().map(|o| ids(&o)),
		front.map(|o| ids(&o))
	));
	let keys = |o: &Option<Key>| o.as_ref().map(|k| k.id);
	let mut heap = BinaryHeap::new();
	heap.push(Some(key(7)));
	heap.push(None);
	heap.push(Some(key(8)));
	log.note(format!("heap {:?}", heap.peek().map(keys)));
	let top = heap.pop();
	log.note(format!("top {:?}", top.as_ref().map(keys)));
	drop(top);
	let mut sorted = BTreeSet::new();
	sorted.insert(Some(key(9)));
	sorted.insert(None);
	sorted.insert(Some(key(10)));
	log.note(format!(
		"set {:?} {:?}",
		sorted.first().map(keys),
		sorted.last().map(keys)
	));
	// The key `replace` gives back drops when its caller drops it, and the one it keeps
	// with the set; `get` and `take` find either by an equal key.
	let replaced = sorted.replace(Some(key(9)));
	log.note(format!(
		"replace {:?} {:?}",
		replaced.as_ref().map(keys),
		sorted.replace(None).map(|o| keys(&o))
	));
	drop(replaced);
	log.note(format!("get {:?}", sorted.get(&Some(key(10))).map(keys)));
	let taken = sorted.take(&Some(key(10)));
	log.note(format!(
		"take {:?} {:?}",
		taken.as_ref().map(keys),
		sorted.take(&Some(key(14))).is_some()
	));
	drop(taken);
	sorted.insert(Some(key(10)));
	let least = sorted.pop_first();
	let most = sorted.pop_last();
	log.note(format!(
		"popped {:?} {:?}",
		least.map(|o| keys(&o)),
		most.map(|o| keys(&o))
	));
	// One key that drops, whose order of drops beside others std leaves unspecified.
	let mut hashed = HashSet::new();
	hashed.insert(Some(key(11)));
	log.note(format!("hashed {:?}", hashed.get(&None).map(keys)));
	hashed.insert(None);
	let taken = hashed.take(&None);
	log.note(format!(
		"taken {:?} {:?}",
		taken.map(|o| keys(&o)),
		hashed.take(&None).map(|o| keys(&o))
	));
	let replaced = hashed.replace(Some(key(11)));
	log.note(format!("replaced {:?}", replaced.as_ref().map(keys)));
	drop(replaced);
	let held = hashed.take(&Some(key(11)));
	log.note(format!("held {:?}", held.map(|o| keys(&o))));
	log.note(format!("left {} {} {}", heap.len(), sorted.len(), hashed.len()));
}

/// What maps keyed by `Arc<str>` drop of the keys they are given or hold, and `max` and
/// `min` of the operand they do not return, read by the keys' counts; what `or_insert`
/// drops of a value JS copies, which it returns a place for; and what `into_inner` gives.
fn arcs(log: &Log, id: u32) {
	let count = |what: &str, key: &Arc<str>| log.note(format!("{what} {}", Arc::strong_count(key)));
	let name: Arc<str> = Arc::from("name");
	let other: Arc<str> = Arc::from("other");
	let mut map = HashMap::new();
	map.insert(name.clone(), 1u8);
	map.insert(name.clone(), 2);
	map.insert(other.clone(), 3);
	count("insert", &name);
	let mut tree = BTreeMap::new();
	tree.insert(other.clone(), 1u8);
	tree.insert(other.clone(), 2);
	count("tree", &other);
	*map.entry(name.clone()).or_insert(0) += 1;
	count("entry", &name);
	map.remove(&name);
	count("remove", &name);
	tree.remove("other");
	count("removed", &other);
	let max = name.clone().max(other.clone());
	let min = name.clone().min(other.clone());
	log.note(format!("max {} min {}", &*max, &*min));
	count("name", &name);
	count("other", &other);
	drop((max, min));
	let mut tags = BTreeMap::new();
	tags.insert(1u8, Held::One(Guard::new(id + 1, log.clone())));
	if let Held::One(guard) = tags.entry(1).or_insert(Held::One(Guard::new(id + 2, log.clone()))) {
		log.note(format!("tag {}", guard.id()));
	}
	tags.entry(2).or_insert(Held::Two(
		Guard::new(id + 3, log.clone()),
		Guard::new(id + 4, log.clone()),
	));
	log.note(format!("tags {}", tags.len()));
	// `into_inner` gives the value from the last strong handle alone, after which a `Weak`
	// no longer upgrades; another's only drops its share.
	let shared = Arc::new(Guard::new(id + 5, log.clone()));
	let weak = Arc::downgrade(&shared);
	let clone = Arc::clone(&shared);
	log.note(format!("into_inner {}", Arc::into_inner(shared).is_some()));
	let inner = Arc::into_inner(clone);
	log.note(format!(
		"into_inner {:?} {}",
		inner.as_ref().map(Guard::id),
		weak.upgrade().is_some()
	));
	drop(inner);
	let held = Rc::new(Some(Guard::new(id + 6, log.clone())));
	let again = Rc::clone(&held);
	drop(Rc::into_inner(held));
	let inner = Rc::into_inner(again);
	log.note(format!("rc {:?}", inner.as_ref().map(|o| o.as_ref().map(Guard::id))));
	drop(inner);
	drop(map);
	count("dropped", &name);
	count("dropped", &other);
}

/// Run scenario `arg % 42` of what std's by-value consumers drop of adaptors that hold
/// closures that drop, and of zips of items that drop.
pub fn folded(log: &Log, id: u32, arg: u8) {
	folds::folded(log, id, arg);
}

/// Run scenario `arg % 9` of what folds back to front, and through what wraps a closure
/// beside another.
pub fn rfolded(log: &Log, id: u32, arg: u8) {
	folds::rfolded(log, id, arg);
}

/// Run scenario `arg % 10` of where std drops values inside its iterators and formatting.
/// Three panic, each with std's message: in a `for_each`, a drain past the end, and a
/// range that panics as `String`'s drop it.
pub fn inside(log: &Log, id: u32, arg: u8) {
	inside::inside(log, id, arg);
}

/// Run scenario `arg % 9` of what std's comparisons of iterators drop.
pub fn compared(log: &Log, id: u32, arg: u8) {
	iters::compared(log, id, arg);
}

/// Run scenario `arg % 8` of what a `LinkedList` drops, and when.
pub fn lists(log: &Log, id: u32, arg: u8) {
	lists::run(log, id, arg);
}

/// Run scenario `arg % 9` of what a `Vec`'s edits, its, a tuple's, and a box's `clone`,
/// `Ord`'s defaults, and an iterator's `max` and `min` drop. One ends in an index past the
/// end, which panics with std's message.
pub fn edited(log: &Log, id: u32, arg: u8) {
	let key = |n: u32| Key::new(id + n, log.clone());
	let ids = |v: &[Key]| v.iter().map(|k| (k.id - id).to_string()).collect::<Vec<_>>().join(",");
	match arg % 9 {
		0 => {
			let mut v = list![key(1), key(2), key(3), key(4)];
			v.retain(|k| (k.id - id) % 2 == 1);
			log.note(format!("retained {}", ids(&v)));
			v.retain_mut(|k| k.id != id + 3);
			log.note(format!("retained {}", ids(&v)));
		}
		1 => {
			let mut v = list![key(1), key(1), key(2), key(2), key(3)];
			v.dedup();
			log.note(format!("deduped {}", ids(&v)));
			v.dedup_by_key(|k| k.id / 2);
			log.note(format!("deduped {}", ids(&v)));
			v.dedup_by(|a, b| a.id > b.id);
			log.note(format!("deduped {}", ids(&v)));
		}
		2 => {
			let mut v = list![key(1), key(2), key(3)];
			drop(v.swap_remove(0));
			log.note(format!("swapped {}", ids(&v)));
			drop(v.swap_remove(3));
		}
		3 => {
			let v = list![key(1), key(2)];
			log.note(format!("cloned {}", ids(&v.clone())));
		}
		4 => {
			let pair = (key(1), key(2));
			drop(pair.clone());
		}
		5 => {
			let mut v = list![key(1), key(2), key(3)];
			v.insert(1, key(4));
			log.note(format!("inserted {}", ids(&v)));
			drop(v.remove(0));
			log.note(format!("removed {}", ids(&v)));
			v.truncate(1);
			log.note(format!("truncated {}", ids(&v)));
			v.clear();
		}
		6 => {
			// Each loser drops as it loses.
			let best = list![key(1), key(3), key(5)]
				.into_iter()
				.max_by_key(|k| (k.id - id) % 3);
			log.note(format!("max {}", best.map_or(0, |k| k.id - id)));
			let least = list![key(3), key(1), key(2)].into_iter().min();
			log.note(format!("min {}", least.map_or(0, |k| k.id - id)));
		}
		7 => {
			let boxed = Box::new(key(1));
			log.note(format!("boxed {}", boxed.clone().id - id));
		}
		_ => {
			log.note(format!("max {}", key(1).max(key(2)).id - id));
			log.note(format!("min {}", key(5).min(key(4)).id - id));
			log.note(format!("clamped {}", key(2).clamp(key(1), key(3)).id - id));
			log.note(format!("clamped {}", key(0).clamp(key(1), key(3)).id - id));
		}
	}
}
