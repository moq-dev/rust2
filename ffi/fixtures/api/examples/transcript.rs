//! The script `tests/api.c` runs through the C ABI, run natively: the two transcripts
//! must match line for line.

use std::future::Future;
use std::ops::Bound;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use api::{
	Channel, Counter, Event, LIMIT, Label, Log, Logger, Message, Name, Node, Pair, Point, Registry, Shape, Size, Stack,
	Stamp, Tally, Waiter, Window,
};
use bytes::Bytes;

/// Keeps even levels, as `tests/api.c`'s implementation does.
struct Print;

impl Log for Print {
	fn log(&self, level: u8, line: &str) -> bool {
		println!("log {level} {line}");
		level % 2 == 0
	}
}

impl Drop for Print {
	fn drop(&mut self) {
		println!("freed");
	}
}

struct Count(AtomicUsize);

impl Wake for Count {
	fn wake(self: Arc<Self>) {
		self.0.fetch_add(1, Ordering::Relaxed);
	}
}

fn main() {
	println!("limit {LIMIT}");

	let p = Point { x: 1, y: 2 }.offset(2, 3);
	println!("offset {} {}", p.x, p.y);
	println!("debug {p:?}");
	println!("eq {}", p == Point { x: 3, y: 5 });
	let d = Point::default();
	println!("default {} {}", d.x, d.y);

	let mut c = Counter::new("c", 5);
	println!("get {}", c.get());
	println!("name {}", c.name());
	match c.add(10) {
		Ok(v) => println!("add ok {v}"),
		Err(e) => println!("add err {e}"),
	}
	match c.add(2000) {
		Ok(v) => println!("add ok {v}"),
		Err(e) => println!("add err {e} {e:?}"),
	}
	match c.sub(20) {
		Ok(v) => println!("sub ok {v}"),
		Err(e) => println!("sub err {e} {e:?}"),
	}
	c.rename("d");
	println!("name {}", c.name());
	println!("is d {}", c.is("d"));
	let mut copy = c.clone();
	let _ = copy.add(1);
	println!("clone {} original {}", copy.get(), c.get());
	println!("debug {c:?}");
	let label = c.into_label(Some(p));
	println!("label {label:?}");
	let empty = api::Note {
		text: Some(String::new()),
	};
	println!("note {} {}", api::has_text(empty), api::has_text(api::Note::default()));

	let (below, above) = Counter::new("s", 7).split(5);
	println!(
		"split {} {} {} {}",
		below.name(),
		below.get(),
		above.name(),
		above.get()
	);
	let (only,) = api::only(Counter::new("o", 3));
	println!("only {} {}", only.name(), only.get());
	let (a, b, c, d) = api::widths(98305);
	println!("widths {a} {b} {c} {d}");
	for b in [2, 0] {
		match api::divide(7, b) {
			Ok((q, r)) => println!("divide {q} {r}"),
			Err(e) => println!("divide err {e}"),
		}
	}
	for n in [98305, 0] {
		match api::spread(n) {
			Some((a, b, c, d)) => println!("spread {a} {b} {c} {d}"),
			None => println!("spread none"),
		}
	}
	for needle in [&b"lo"[..], b"x"] {
		match api::find(b"hello", needle) {
			Some((at, len)) => println!("find {at} {len}"),
			None => println!("find none"),
		}
	}
	let rows = api::rows(&[Point { x: 1, y: 2 }, Point { x: 3, y: 3 }]);
	let rows: Vec<String> = (rows.iter())
		.map(|(name, shape, p)| format!("{name} {shape:?} {} {}", p.x, p.y))
		.collect();
	println!("rows {}", rows.join(", "));
	let board = api::Board::new(vec![
		("low".to_string(), Counter::new("l", 1)),
		("high".to_string(), Counter::new("h", 9)),
	]);
	// The counters a board returns build another, each handle moved back in.
	let board = api::Board::new(board.into_counters());
	let ranked: Vec<String> = (board.ranked())
		.map(|(name, counter)| format!("{name} {} {}", counter.name(), counter.get()))
		.collect();
	println!("ranked {}", ranked.join(", "));
	let best: Counter = board.best().into();
	println!("best {} {}", best.name(), best.get());
	let atlas = api::Atlas {
		points: [("b", 1, 1), ("ab", 2, 3), ("b", 4, 4)]
			.into_iter()
			.map(|(name, x, y)| (name.to_string(), Point { x, y }))
			.collect(),
		ids: [3, 1, 3].into_iter().collect(),
	}
	.with("c", Point { x: 5, y: 6 });
	let points: Vec<String> = (atlas.points.iter())
		.map(|(name, p)| format!("{name} {} {}", p.x, p.y))
		.collect();
	let ids: Vec<String> = atlas.ids.iter().map(u32::to_string).collect();
	println!("atlas {}; ids {}", points.join(", "), ids.join(" "));
	for text in ["80", "x"] {
		match api::port(text) {
			Ok(port) => println!("port {port}"),
			Err(e) => println!("port err {e}"),
		}
	}
	let span = api::Span {
		bounds: (2, 10),
		named: Some(("s".to_string(), Shape::Square)),
	};
	let ((a, b), (c, d)) = api::halves(&span);
	println!("halves {a} {b} {c} {d} {span:?}");
	let (n, text) = api::swapped(("t".to_string(), 4));
	println!("swapped {n} {text}");
	for value in [None, Some((None,)), Some((Some(3),))] {
		println!("doubled {:?}", api::doubled(value));
	}
	let lent = (vec![1, 2],);
	let (lent,) = api::lent_back(&lent);
	let (boxed,) = *api::boxed_tuple(vec![3]);
	println!("lent back {lent:?} boxed {boxed:?}");
	println!(
		"title {:?} {:?} named {:?}",
		api::title(true),
		api::title(false),
		api::named("x")
	);

	// Each lent counter is a new handle to a clone.
	let mut registry = Registry::new(Counter::new("first", 1));
	println!("last {}", registry.last().is_some());
	registry.push(Counter::new("last", 2));
	let mut first = registry.first.clone();
	let _ = first.add(10);
	println!("first {} lent {}", registry.first.get(), first.get());
	let mut last = registry.last().unwrap().clone();
	let _ = last.add(10);
	println!("last {} lent {}", registry.last().unwrap().get(), last.get());
	// A closure lent for the call edits the first counter through a handle.
	let edited = registry.edit(|counter| {
		let _ = counter.add(5);
		counter.edited() && counter.get() == 6
	});
	println!("edit {edited} {}", registry.first.get());
	// A setter moves an object in, as assignment does.
	registry.first = Counter::new("second", 3);
	println!("set first {} {}", registry.first.get(), registry.first.name());
	println!("walk {}", api::walk(3, |cursor| cursor.at() * 2));
	println!("each {}", api::each(2, |at| println!("visit {at}")));
	println!("offset {}", api::offset(|| 7, 3));
	println!("shift {}", api::shift(|| 7, 4));

	let name = Name::owned("a".to_string());
	let mut joined = name.join(&Name::owned("b".to_string()));
	println!("joined {} {joined:?} {}", joined.as_str(), joined == name);
	joined.rename("c".to_string());
	println!("renamed {}", joined.as_str());
	let long = api::longer(joined, &Name::owned("dd".to_string()));
	println!("longer {}", long.as_str());
	println!("unnamed [{}]", api::unnamed().as_str());
	// One static, so Rust lends one string, as interning lends a binding's equal ones.
	static LIT: &str = "lit";
	let (lit, again) = (Name::borrowed(LIT), Name::borrowed(LIT));
	let other = Name::borrowed("other");
	println!(
		"borrowed {} {} {}",
		lit.as_str(),
		lit.shares(&again),
		lit.shares(&other)
	);

	println!("total {}", api::total(2..5));
	println!("reach {}", api::reach(2, (), 5));
	let scaled = |v, by| api::scaled(v, by).map_or("none".to_string(), |v| format!("{v:032x}"));
	println!(
		"scaled {} {} {}",
		scaled(u128::MAX, 1),
		scaled((1 << 64) + 1, 1 << 63),
		scaled(1 << 127, 2)
	);
	let hex = |sum: [u8; 4]| sum.iter().map(|b| format!("{b:02x}")).collect::<String>();
	let mut digest = api::Digest::new();
	digest.sum = api::fingerprint("deadbeef").unwrap();
	println!(
		"digest {} {} {}",
		hex(digest.sum),
		hex(digest.flipped([0xff, 0, 0xff, 0])),
		api::fingerprint("zz").unwrap_err()
	);
	let mut batch: api::Batch = api::Batch::new();
	let accepted = (1..=5).filter(|&v| batch.push(v)).count();
	let mut batch2 = api::pair();
	let paired = (1..=3).filter(|&v| batch2.push(v)).count();
	let emptied = api::drain(&mut batch);
	println!(
		"batch {} {accepted} {emptied} {} {} {paired} {}",
		batch.capacity(),
		api::drain(&mut batch),
		batch2.capacity(),
		api::drain(&mut batch2)
	);
	println!(
		"capacities {} {}",
		api::capacities(&batch, &batch2),
		api::capacities(&batch2, &batch2)
	);
	let wide = api::widen(2..5, 1);
	println!("widen {} {}", wide.start, wide.end);
	println!(
		"count {} {} {} {} {}",
		api::count(2..5),
		api::count(2..=5),
		api::count(3..),
		api::count(..),
		api::count((Bound::Excluded(2), Bound::Included(4)))
	);
	println!("end {:?} {:?}", api::end(3), api::end(0));
	let (start, end) = api::ends(3);
	println!("ends {start:?} {end:?}");
	for n in [2, 5] {
		match api::below(n, 3) {
			Ok(n) => println!("below {n}"),
			Err((start, end)) => println!("below err {start:?} {end:?}"),
		}
	}
	let window = Window {
		range: 2..8,
		end: api::end(5),
	};
	println!("window {} {} {window:?}", window.contains(4), window.contains(6));

	println!("{}", api::greet("x", Shape::Square));
	println!("len {}", api::len(b"abc"));
	println!("hex {}", api::hex(Bytes::from_static(b"\xff\x00a")));
	println!("timed hex {}", api::timed_hex(Bytes::from_static(b"\xff\x00a")));
	println!("sample hex {}", api::sample_hex(&b"\xff\x00a"[..]));
	println!("queued hex {}", api::queued_hex(b"\xff\x00a".to_vec()));
	let tick = Label {
		name: "tick".to_string(),
		at: None,
		ttl: None,
	};
	println!("tick {}", api::tick_name(tick));
	let rooted = Label {
		name: "rooted".to_string(),
		at: None,
		ttl: None,
	};
	println!("rooted {}", api::rooted_name(rooted));
	println!("stored {}", api::stored_text("stored"));
	let layer = Label {
		name: "layer".to_string(),
		at: None,
		ttl: None,
	};
	println!("layer {}", api::layer_name(layer));
	for event in [Event::Moved(p), Event::Named { name: "n".to_string() }, Event::Done] {
		println!("{}", api::describe(&event));
	}

	let stamp = Stamp::from_millis(2500);
	println!("secs {}", stamp.secs());
	println!("later {}", Stamp::from_millis(3000).later(stamp));
	println!("secs {}", stamp.secs());
	let whole: Stamp = stamp.whole().into();
	println!("whole {whole:?}");
	let mut message = Message::new(Some(stamp), Bytes::from_static(b"body"));
	println!("payload {}", String::from_utf8_lossy(&message.payload));
	println!("stamp {:?}", message.stamp.unwrap());
	println!("sent {}", message.sent());
	// Setters assign in place: the bytes are copied in, and a `Copy` value copied.
	let reset = Stamp::from_millis(4000);
	message.stamp = Some(reset);
	message.payload = Bytes::from_static(b"edited");
	println!("payload {}", String::from_utf8_lossy(&message.payload));
	println!("stamp {:?}", message.stamp.unwrap());
	println!("secs {}", reset.secs());
	message.stamp = None;
	println!("stamp {}", message.stamp.is_some());

	// A value past a word, copied across: a closure takes and returns one by value.
	let moment = api::Moment::new(1500, 1000).unwrap();
	println!("moment {} {}", moment.millis(), api::Moment::new(1, 0).is_none());
	let shifted = api::shifted(moment, |m| m.after(500));
	println!("shifted {} {}", shifted.millis(), shifted == moment.after(500));
	println!("wrapped {} {}", moment.after(u64::MAX).millis(), moment == shifted);

	// A padded value crosses with its padding zeroed, and one whose bytes vary is a handle.
	let gap = api::Gap::new(7, 3);
	println!("gap {} {}", gap.sum(), gap == api::Gap::new(7, 3));
	let berth = api::Berth::new(Some(4));
	println!("berth {:?} {:?}", berth.at(), api::Berth::new(None).at());

	// A `Copy` type changed in place is a value, whose change reaches only the copy changed.
	let mut score = api::Score::of(2);
	let step = api::Score::of(3);
	score.add(step);
	score.add(step);
	println!("score {} {} {}", score.count(), step.doubled().count(), step.count());
	let mut copy = score;
	let taken = copy.take();
	println!("taken {taken} {} {}", copy.count(), score.count());
	let mut other = step;
	let before = copy.absorb(&mut other);
	println!(
		"absorbed {} {} {} {} {}",
		before.0,
		before.1,
		copy.count(),
		other.count(),
		step.count()
	);
	let mut hits = api::Hits::of(1, 2);
	let kept = hits;
	hits.add(api::Hits::of(10, 20));
	hits.hits = 7;
	println!(
		"hits {} {} {} {} {}",
		hits.hits,
		hits.misses,
		kept.hits,
		kept.misses,
		hits == kept
	);
	let mut counted = kept;
	let same = api::Hits::eq(&mut counted, kept);
	println!("counted {same} {}", counted.hits);
	let mut extent = api::Extent::new(5, 3);
	let start = extent;
	let old = extent.slide(10);
	extent.start += 1;
	println!(
		"extent {old} {} {} {} {}",
		extent.start,
		extent.end(),
		start.start,
		start.end()
	);
	let weighted = api::Weighted { score: step, weight: 4 };
	println!(
		"weighted {} {}",
		weighted.total(),
		weighted.score.count() + step.count()
	);

	println!("kept {}", api::emit(Print, vec![1, 2, 3, 4]));
	let logger = Logger::new(Box::new(Print));
	println!("write {}", logger.write("hi"));
	drop(logger);
	println!("dropped");
	let kept = api::filter(vec![1, 2, 3, 4], |level, line| {
		println!("filter {level} {line}");
		level > 2
	});
	println!("filtered {kept}");

	let channel = Channel::new();
	let count = Arc::new(Count(AtomicUsize::new(0)));
	let waker = Waker::from(count.clone());
	let mut cx = Context::from_waker(&waker);
	{
		let mut recv = pin!(channel.recv());
		let first = recv.as_mut().poll(&mut cx);
		println!("recv pending {}", first.is_pending());
		channel.send(Bytes::from_static(b"hi"));
		println!("woken {}", count.0.load(Ordering::Relaxed));
		if let Poll::Ready(payload) = recv.as_mut().poll(&mut cx) {
			println!("recv {}", String::from_utf8_lossy(&payload));
		}
	}
	channel.send(Bytes::new());
	let mut recv = pin!(channel.recv_nonempty());
	match recv.as_mut().poll(&mut cx) {
		Poll::Ready(Ok(payload)) => println!("nonempty {}", payload.len()),
		Poll::Ready(Err(e)) => println!("nonempty err {e}"),
		Poll::Pending => println!("nonempty pending"),
	}
	let mut filled: Vec<String> = Vec::new();
	let mut fill4: api::Batch = api::Batch::new();
	if let Poll::Ready(v) = pin!(api::fill(&mut fill4, 9)).poll(&mut cx) {
		filled.extend(v.iter().map(u32::to_string));
	}
	let mut fill2 = api::pair();
	if let Poll::Ready(v) = pin!(api::fill(&mut fill2, 1)).poll(&mut cx) {
		filled.extend(v.iter().map(u32::to_string));
	}
	println!("fill {}", filled.join(" "));

	// The send happens at the call: the future is dropped unpolled.
	drop(channel.send_later(Bytes::from_static(b"eager")));
	let mut recv = pin!(channel.recv());
	if let Poll::Ready(payload) = recv.as_mut().poll(&mut cx) {
		println!("recv {}", String::from_utf8_lossy(&payload));
	}
	// The send happens at the call, and the future borrows the channel and the tag.
	let mut tagged = pin!(channel.tagged("tag"));
	if let Poll::Ready(text) = tagged.as_mut().poll(&mut cx) {
		println!("tagged {text}");
	}
	// So does a future borrowing an optional slice, which the export owns while it is pending.
	let items = [1u32, 2, 3];
	let mut itemized = pin!(channel.itemized(Some(&items)));
	if let Poll::Ready(count) = itemized.as_mut().poll(&mut cx) {
		println!("itemized {count}");
	}
	// Another crate's future borrowing its bytes, which the export owns while it is pending.
	let bytes = *b"abc";
	if let Poll::Ready(len) = pin!(api::byte_len(&bytes)).poll(&mut cx) {
		println!("byte_len {len}");
	}
	channel.send(Bytes::from_static(b"sized"));
	let mut sized = pin!(channel.recv_sized());
	if let Poll::Ready((payload, len)) = sized.as_mut().poll(&mut cx) {
		println!("sized {} {len}", String::from_utf8_lossy(&payload));
	}
	channel.notify(waker.clone());
	channel.send(Bytes::from_static(b"rung"));
	println!("woken {}", count.0.load(Ordering::Relaxed));
	if let Poll::Ready(payload) = channel.poll_take(&Waiter::new(waker.clone())) {
		println!("take {}", String::from_utf8_lossy(&payload));
	}
	channel.send(Bytes::from_static(b"measured"));
	if let Poll::Ready(Ok(len)) = channel.clone().poll_measure(&mut cx, 2) {
		println!("measure {len}");
	}
	channel.listen((), &cx);
	channel.send(Bytes::from_static(b"heard"));
	println!("woken {}", count.0.load(Ordering::Relaxed));
	let waiter = Waiter::new(waker.clone());
	waiter.waker().clone().wake();
	waiter.context().waker().clone().wake();
	println!("woken {}", count.0.load(Ordering::Relaxed));

	let list = Node {
		value: 1,
		next: Some(Box::new(Node { value: 2, next: None })),
	};
	println!("node {}", list.sum());

	let tally = Tally::new();
	tally.bump();
	println!("tally {} {}", tally.bump(), tally == tally);
	println!("both {}", api::bump_both(&tally, &tally));
	println!("peek {}", api::peek(&tally, |tally| tally.bump()));

	let area = api::grow(&dep::Area::new(Size { width: 2, height: 3 }), 1);
	let size = area.size();
	println!("area {} {} {}", size.width, size.height, area.pixels());

	let pair = Pair { left: 2u32, right: 5 };
	let swapped = pair.swap();
	println!(
		"pair {} {} sum {} {pair:?} {}",
		swapped.left,
		swapped.right,
		pair.sum(),
		pair == swapped
	);
	println!("even {} {}", pair.even(), Pair { left: 7u32, right: 7 }.even());
	let sized = api::tag(Size { width: 1, height: 2 }, 3);
	let flipped = sized.swap();
	println!("sized {} {} {flipped:?}", flipped.left, flipped.right.width);
	let beside = Pair { left: 258u32, right: 9 }.beside(sized.left);
	println!("beside {} {beside:?}", beside.left);
	let mut stack = Stack::new();
	for (i, name) in ["a", "b"].into_iter().enumerate() {
		stack.push(Counter::new(name, i as u64 + 1));
	}
	let top = stack.pop().map(|c| c.get());
	println!(
		"stack {} {top:?} {} {}",
		stack.depth(),
		stack.pop().is_some(),
		stack.pop().is_none()
	);
	let from_point = Counter::seeded("p", Point { x: -3, y: 4 });
	let from_size = Counter::seeded("s", Size { width: 5, height: 6 });
	let from_counter = Counter::seeded("c", &from_point);
	let from_polar = Counter::seeded("q", api::polar::Point { turns: 1, distance: 2 });
	println!(
		"seeded {} {} {} {} past {} seeds {}",
		from_point.get(),
		from_size.get(),
		from_counter.get(),
		from_polar.get(),
		stack.depth_past(&Point { x: 1, y: 2 }),
		api::seeds::<Point>(&Size { width: 2, height: 3 })
	);

	println!("weigh {}", api::weigh(Point { x: -1, y: 2 }));
	// A function named as the type it returns, and one named as a type it does not.
	let polar = api::polar::point(2).expect("a point off the origin");
	println!(
		"polar {} {} area {}",
		polar.distance,
		api::polar::point(0).is_none(),
		api::area(Size { width: 2, height: 3 })
	);
	println!(
		"wind {}",
		api::wind(api::PolarTurn { whole: 2 }, api::polar::Turn { part: 3 })
	);
	// Names that clash in some language, each kept apart there.
	let clash_span = api::clash::Span { width: 4 };
	let clash_store = api::clash::Store::new(7);
	println!(
		"clash {:?} {:?} {:?} span {} {} {clash_span} store {} {} audio {} {} nested {}",
		api::clash::Tone { hz: 440 },
		api::clash::TONE { db: 3 },
		api::Cancelled { step: 2 },
		clash_span.width(),
		clash_span.to_string(),
		clash_store.free(),
		clash_store.close(),
		api::clash::audio::encoder_finish(),
		api::clash::audio_encoder::finish(),
		api::clash::nested(Some(((1, 2), 3)), Some(((1, 2, 3),)))
	);
	let left = api::Left {
		value: api::Right { value: 1u8 },
	};
	let right = api::Right {
		value: api::Left { value: 2u8 },
	};
	println!("cross {} outer {}", api::cross(left, right), api::Outer::new(4).get());
	println!("kept {} boxed {}", api::kept(Point { x: 1, y: -2 }), api::BOXED.value);
	println!("keep {}", api::keep(Point { x: 1, y: -2 }));
	let five = Point { x: 5, y: 2 };
	println!(
		"kept or {} {} keeper {}",
		api::kept_or(five),
		api::kept_or::<Point>(None),
		api::keeper(five)
	);
	let sides = Pair { left: 2u32, right: 5 };
	println!("apply {}", api::apply(sides, |side| side * 3));
	println!("report {}", api::report(sides, Print));
	println!("wrapped {}", api::kept(api::Wrap { value: 4u32 }));
	println!("keeps {}", api::keeps(Print).value);

	let token = api::Token::new(7);
	println!("token {} dropped {}", token.id, api::dropped());
	drop(token);
	println!("dropped {}", api::dropped());

	let mut retry = api::Retry::new(Some(5));
	let limit = retry.limit;
	println!(
		"retry {} {} {} {}",
		u8::from(limit.is_some()),
		limit.unwrap_or(0),
		retry.limit(),
		retry.delay()
	);
	// The renamed field's setter, named as its getter.
	retry.limit = None;
	println!("retry {}", retry.limit());
	println!("slot {}", api::Slot::<u8>::new(200).value());

	// Foreign types: a URL parsed from text, socket addresses, OS paths, and `Vec`s.
	let base = url::Url::parse("https://example.com/a/b").unwrap();
	println!("url {base} host {:?}", api::host(&base));
	match api::resolve(base.clone(), "c?d") {
		Ok(url) => println!("resolve {url} {}", url == base),
		Err(err) => println!("resolve err {err}"),
	}
	match url::Url::parse("no scheme") {
		Ok(url) => println!("parse {url}"),
		Err(err) => println!("parse err {err}"),
	}
	let v4: std::net::SocketAddr = "127.0.0.1:8080".parse().unwrap();
	let v6: std::net::SocketAddr = "[::1%3]:443".parse().unwrap();
	println!("ports {:?}", api::ports(vec![v4, v6]));
	let peer = api::Peer {
		addr: v6,
		names: vec!["a".to_string(), "b".to_string()],
	}
	.next("c");
	println!("peer {peer:?} {:?}", peer.aliases());
	let file = api::file(std::path::Path::new("dir"), "f\\'\"\t");
	println!("file {}", file.as_os_str().as_encoded_bytes().escape_ascii());
	let parents = api::parents(vec![file, std::path::PathBuf::from("/")]);
	let parents: Vec<String> = (parents.iter())
		.map(|p| match p {
			Some(p) => p.as_os_str().as_encoded_bytes().escape_ascii().to_string(),
			None => "none".to_string(),
		})
		.collect();
	println!("parents {}", parents.join(" "));
	let words = api::words(" one two  three ");
	println!("words {} {words:?}", words.len());
	println!("sentence {} {}", api::sentence(Some(words)), api::sentence(None));
	println!("sentence {}|", api::sentence(Some(Vec::new())));
	println!("either {} {}", api::either(Some("abc")), api::either(None));
	let prefixed = |prefix: Option<&[u8]>| String::from_utf8(api::prefixed(b"cd", prefix)).unwrap();
	println!("prefixed {} {}", prefixed(Some(b"ab")), prefixed(None));
	println!(
		"coordinates {} {}",
		api::coordinates(Some(&Point { x: 2, y: 3 })),
		api::coordinates(None)
	);
	let (two, mut three) = (Counter::new("two", 2), Counter::new("three", 3));
	println!("plus {} {}", two.plus(Some(&three)), two.plus(None));
	println!(
		"add_to {} {} {}",
		two.add_to(Some(&mut three)),
		two.add_to(None),
		three.get()
	);
	let mut buf = b"0,".to_vec();
	api::put_number(1, &mut buf);
	api::put_number(23, &mut buf);
	println!("put_number {}", String::from_utf8(buf).unwrap());
	let mut entry = (b"ab".to_vec(), 1);
	let count = api::push_counted(&mut entry, b'c');
	println!(
		"push_counted {count} {} {}",
		String::from_utf8(entry.0).unwrap(),
		entry.1
	);
	let mut buf = bytes::Bytes::from_static(b"7x9");
	let first = api::digit(&mut buf).unwrap();
	let skipped = api::digit(&mut buf).is_err();
	println!(
		"digit {first} {skipped} {} {}",
		api::digit(&mut buf).unwrap(),
		buf.len()
	);
	let mut point = Point { x: 1, y: 2 };
	api::nudge(&mut point, 3);
	println!("nudge {} {}", point.x, point.y);
	let config = api::secure("h3");
	println!("secure {} {}", api::alpn_lent(&config), api::alpn(config.clone()));
	let cert = api::certify(b"der");
	let chain = vec![api::certify(b"a"), api::certify(b"bc")];
	println!(
		"cert {} {} {}",
		api::cert_len(&cert),
		String::from_utf8_lossy(&api::cert_der(cert)),
		api::chain_len(chain)
	);
	let session = api::resume("h3", 60).unwrap();
	println!("resume {} {}", session.alpn(), session.lifetime().seconds);
	println!("resume err {}", api::resume("", 60).err().unwrap());
	let mut meters = api::Meters::new();
	let first = meters.first();
	first.add(2);
	println!("lent {} {}", first.add(3), meters.first().count());
	meters.clear();
	println!("cleared {}", meters.first().count());
	meters.last.add(4);
	println!("field lent {}", meters.last.add(1));
	let tallies = api::Tallies::new();
	tallies.first().bump();
	println!("tallied {}", tallies.first().bump());
	println!("chunks {:?}", api::chunks(b"abcde", 2));
	let mut counters = api::counters(vec!["x".to_string(), "y".to_string()]);
	let names: Vec<String> = counters.iter().map(|c| format!("{} {}", c.name(), c.get())).collect();
	println!("counters {}", names.join(", "));
	let boxed = counters.split_off(1);
	println!(
		"worth {} {}",
		api::worth(Some(counters), Box::new(boxed)),
		api::worth(None, Box::default())
	);
	println!(
		"held {:?} {:?}",
		api::held_count(Some((Counter::new("h", 4),))),
		api::held_count(None)
	);
	let boxed = api::boxed_counter(Box::new(Some("b".to_string())));
	println!(
		"boxed {:?} {:?} {}",
		api::boxed_name(boxed),
		api::boxed_name(Box::new(None)),
		api::boxed_counter(Box::new(None)).is_none()
	);
	println!("less {}", api::less(5, 2));
	println!("sums {:?}", api::sums(vec![vec![1, 2], Vec::new(), vec![3]]));
	let least = api::least(vec![[3, 1].into(), Default::default(), [2].into()]);
	println!(
		"least {}",
		least
			.iter()
			.map(|set| format!("{:?}", set.iter().collect::<Vec<_>>()))
			.collect::<Vec<_>>()
			.join(" ")
	);
	numbers("edges", &api::Numbers::edges());
	numbers("reversed", &api::Numbers::edges().reversed());
	let edges = api::Numbers::edges();
	let nan = api::Measure::new(f64::NAN, false);
	println!(
		"float equal {} {} {} {} {}",
		// Each value against itself, which a NaN it holds makes unequal.
		PartialEq::eq(&edges, &edges),
		edges == api::Numbers::edges(),
		api::Numbers::zero(true) == api::Numbers::zero(false),
		PartialEq::eq(&nan, &nan),
		api::Measure::new(-0.0, true) == api::Measure::new(0.0, false)
	);
	println!(
		"sum u32 {} {}",
		api::sum_u32(&[]),
		api::sum_u32(&[u32::MAX, u32::MAX, 2])
	);
	println!("doubled i64 {:?}", api::doubled_i64(&[i64::MIN, -1, 0, i64::MAX]));
	let floats = api::boxed_floats(Box::new([-0.0, f64::from_bits(0x7ff8_0000_0000_0001)]));
	println!(
		"boxed floats {:?}",
		floats.iter().map(|f| f.to_bits()).collect::<Vec<_>>()
	);
	let summed = api::summed([Point { x: 1, y: 2 }, Point { x: -4, y: 6 }]);
	let none = api::summed([]);
	println!("summed {} {} {} {}", summed.x, summed.y, none.x, none.y);
	println!(
		"longest {} {}",
		api::longest(["a".to_string(), "abc".to_string()]),
		api::longest([])
	);
	let words = ["a".to_string(), "b".to_string()];
	println!("first of {:?} {:?}", api::first_of(&words), api::first_of(&[]));
	let limits = api::byte_limits(Bytes::from_static(b"a"), Bytes::from_static(b"bc"));
	let waits = api::Limits {
		low: std::time::Duration::from_millis(5),
		high: std::time::Duration::from_secs(2),
	};
	println!(
		"limits {} {} breadth {}",
		String::from_utf8_lossy(&limits.low),
		String::from_utf8_lossy(&limits.high),
		api::breadth(waits).as_millis()
	);
	let parts = [Bytes::from_static(b"ab"), Bytes::new(), Bytes::from_static(b"c")];
	println!(
		"joined {} {} read {}",
		String::from_utf8_lossy(&api::joined(&parts)),
		api::joined(&[]).len(),
		api::read_len(&b"hello"[..])
	);
	let doc: serde_json::Value = r#"{"b": [1, 2.5, null], "a": "\u00e9"}"#.parse().expect("a JSON document");
	println!("json {}", api::encode(&doc));
	println!("timed text {}", api::timed_text(&doc));
	println!("tagged {}", api::tagged(doc, "t"));
	let entries = api::entries(
		[("b", "[1]"), ("a", "2")]
			.into_iter()
			.map(|(key, text)| (key.to_string(), text.parse().expect("a JSON document")))
			.collect(),
		"t",
	);
	let entries: Vec<String> = entries.iter().map(|(key, value)| format!("{key} {value}")).collect();
	println!("entries {}", entries.join(", "));
	match "{\"a\":".parse::<serde_json::Value>() {
		Ok(doc) => println!("json parse {doc}"),
		Err(err) => println!("json parse err {err}"),
	}
	let site = api::Site::new(base);
	println!("site {} {}", site.home, site.home);

	// Slices, passed in and returned as clones, and the socket address of each family.
	println!("shapes {:?} greetings {:?}", api::Shape::ALL, api::GREETINGS);
	let trail = api::Trail::new(
		&[api::Point { x: 1, y: 2 }, api::Point { x: 3, y: 4 }],
		&["a".to_string(), "b".to_string()],
	);
	println!("trail {:?} {:?}", trail.points(), trail.labels());
	// Returned iterators, each collected at the call: drained labels are gone after.
	let mut trail = trail;
	println!(
		"iterated {:?} {:?} {:?}",
		trail.iter().collect::<Vec<_>>(),
		trail.names().collect::<Vec<_>>(),
		trail.bytes().collect::<Vec<_>>()
	);
	println!(
		"drained {:?} then {:?}",
		trail.drain().collect::<Vec<_>>(),
		trail.labels()
	);
	// A filter, collected at the call.
	let beyond: Vec<String> = trail.beyond(1).map(|p| format!("{} {}", p.x, p.y)).collect();
	println!("beyond {}", beyond.join(", "));
	let numbered: Vec<String> = api::numbered(vec!["p".to_string(), "q".to_string()])
		.map(|c| format!("{} {}", c.name(), c.get()))
		.collect();
	println!("numbered {}", numbered.join(", "));
	let (v4, v6) = api::nexts("127.0.0.1:8080".parse().unwrap(), "[::1%3]:443".parse().unwrap());
	println!("nexts {} {} {} {}", v4.ip(), v4.port(), v6.port(), v6.scope_id());

	// Standard and common types: serde_json's map iterator, std's instant, and tracing's
	// level and its filter.
	let sections: Vec<String> = api::sections(
		[("b", "[1]"), ("a", "2")]
			.into_iter()
			.map(|(key, text)| (key.to_string(), text.parse().expect("a JSON document")))
			.collect(),
	)
	.map(|(key, value)| format!("{key} {value}"))
	.collect();
	println!("sections {}", sections.join(", "));
	let start = api::now();
	let end = api::later(start, std::time::Duration::from_millis(1500));
	println!(
		"instants {} {} {} {}",
		api::since(end, start).as_millis(),
		api::since(start, end).as_millis(),
		start == api::later(start, std::time::Duration::ZERO),
		start == end
	);
	// An instant lent inside an `Option`, which takes no handle.
	println!(
		"has_instant {} {}",
		api::has_instant(&Some(start)),
		api::has_instant(&None)
	);
	let mut verbosity = api::Verbosity::new(tracing::Level::WARN);
	println!(
		"verbosity {} {}",
		api::spell(verbosity.filter()),
		api::spell(api::louder(verbosity.filter()))
	);
	verbosity.level = tracing::Level::TRACE;
	println!(
		"verbosity {} {} {} {}",
		verbosity.level == tracing::Level::TRACE,
		api::spell(verbosity.filter()),
		api::spell(api::louder(verbosity.filter())),
		api::spell(api::louder(tracing::level_filters::LevelFilter::OFF))
	);

	println!("made {}", api::made(3, |n| format!("{n} times")));
	// A closure taking and returning `Copy` objects copies them, never running their `Clone`.
	let kept = api::Ticket::new(9);
	let given = api::reissue(api::Ticket::new(7), |t| api::Ticket::new(t.number() + 1)).number();
	let held = api::reissue(api::Ticket::new(7), |_| kept).number();
	println!("reissued {given} {held}, cloned {}", api::Ticket::clones());
	// A kept closure, called on another thread after the call that took it returned.
	println!("spawned {}", api::Spawned::new(|| 7).join());
	// A kept closure: called again at the instant it asks for, then once its waiter wakes.
	let start = api::now();
	let mut calls = 0;
	let mut drive = pin!(api::drive(start, |now, waiter: &Waiter| {
		calls += 1;
		match calls {
			1 => Ok(Some(api::later(now, std::time::Duration::from_millis(5)))),
			2 => {
				println!("drive {}", api::since(now, start).as_millis());
				waiter.waker().wake_by_ref();
				Ok(None)
			}
			_ => Err(format!("stopped after {calls}")),
		}
	}));
	let waker = Waker::from(Arc::new(Count(AtomicUsize::new(0))));
	let mut cx = Context::from_waker(&waker);
	let stopped = loop {
		if let Poll::Ready(stopped) = drive.as_mut().poll(&mut cx) {
			break stopped;
		}
	};
	println!("drive {stopped}");
	match api::tail(&mut std::io::Cursor::new(b"hello world"), 5) {
		Ok((tail, at)) => println!("tail {} {at}", String::from_utf8_lossy(&tail)),
		Err(err) => println!("tail error {err}"),
	}
	match api::tail(&mut Broken, 5) {
		Ok((tail, at)) => println!("tail {} {at}", String::from_utf8_lossy(&tail)),
		Err(err) => println!("tail error {err}"),
	}
	// A seek from the start reaches the reader whole, past `i64::MAX`, as `Cursor` takes one.
	match api::seek_to(&mut std::io::Cursor::new(Vec::<u8>::new()), u64::MAX) {
		Ok(at) => println!("sought {at}"),
		Err(err) => println!("sought error {err}"),
	}
	println!("non zeros {}", api::describe_non_zeros(api::non_zeros()));
	for hop in [None, Some(1), Some(u64::MAX)] {
		println!(
			"hop {:?}",
			api::next_hop(hop.and_then(std::num::NonZero::new)).map(|h| h.get())
		);
	}
	let near = std::num::NonZero::new(3u16).unwrap();
	let far = std::num::NonZero::new(u16::MAX).unwrap();
	println!(
		"farther hop {}",
		api::farther_hop(Pair { left: near, right: far }).get()
	);
}

/// Print each of `numbers`' lists, its floats as their bits.
fn numbers(what: &str, numbers: &api::Numbers) {
	println!("{what} u16 {:?}", numbers.u16s);
	println!("{what} u32 {:?}", numbers.u32s);
	println!("{what} u64 {:?}", numbers.u64s);
	println!("{what} i8 {:?}", numbers.i8s);
	println!("{what} i16 {:?}", numbers.i16s);
	println!("{what} i32 {:?}", numbers.i32s);
	println!("{what} i64 {:?}", numbers.i64s);
	println!(
		"{what} f32 {:?}",
		numbers.f32s.iter().map(|f| f.to_bits()).collect::<Vec<_>>()
	);
	println!(
		"{what} f64 {:?}",
		numbers.f64s.iter().map(|f| f.to_bits()).collect::<Vec<_>>()
	);
	println!("{what} some {:?}", numbers.some);
}

/// A reader whose reads fail, as `tests/api.c`'s broken one does: each seek lands at 0.
struct Broken;

impl std::io::Read for Broken {
	fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
		Err(std::io::Error::other("broken"))
	}
}

impl std::io::Seek for Broken {
	fn seek(&mut self, _: std::io::SeekFrom) -> std::io::Result<u64> {
		Ok(0)
	}
}
