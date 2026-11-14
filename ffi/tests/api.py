# The api fixture's Python test: the script fixtures/api/examples/transcript.rs runs
# natively, through the generated Python. tests/fixtures.rs compares the transcripts.
#
# With an argument, it instead checks the futures' semantics, what frees what nobody
# closed, the values only Python's types bound, or how Python calls into the extension,
# or breaks a contract, which must fail.

import array
import asyncio
import datetime
import gc
import io
import math
import os
import struct
import sys
import threading
import weakref

import api

# The transcript's lines end and encode as Rust's do, which Windows' text mode and code
# page would turn into CRLF and the locale's encoding.
sys.stdout.reconfigure(encoding="utf-8", newline="\n")


class PrintLog(api.Log):
	"""The Log callback: keeps even levels."""

	def log(self, level, line):
		print(f"log {level} {line}")
		return level % 2 == 0

	# Rust closes it once it drops the implementation.
	def close(self):
		print("freed")


def bound(b):
	match b.kind:
		case api.Bound.INCLUDED:
			return f"Included({b.value})"
		case api.Bound.EXCLUDED:
			return f"Excluded({b.value})"
		case api.Bound.UNBOUNDED:
			return "Unbounded"


def flag(value):
	"""A bool as Rust prints it."""
	return "true" if value else "false"


def data(view):
	"""The bytes a memoryview lends, releasing it."""
	with view:
		return bytes(view)


def text(view):
	return data(view).decode()


def debugs(values):
	return "[" + ", ".join(repr(v) for v in values) + "]"


def quoted(values):
	return "[" + ", ".join(f'"{v}"' for v in values) + "]"


def escaped(path):
	"""A path's bytes as Rust's `escape_ascii` prints them."""
	out = ""
	for b in os.fsencode(path):
		match b:
			case 0x09:
				out += "\\t"
			case 0x0D:
				out += "\\r"
			case 0x0A:
				out += "\\n"
			case 0x5C | 0x27 | 0x22:
				out += "\\" + chr(b)
			case b if 0x20 <= b < 0x7F:
				out += chr(b)
			case b:
				out += f"\\x{b:02x}"
	return out


def bits(floats, code):
	"""An array of floats' bits, as Rust's `to_bits` gives them."""
	return array.array(code, floats.tobytes()).tolist()


def numbers(what, n):
	"""Print each of `n`'s lists, its floats as their bits, as transcript.rs does."""
	for name, items in [
		("u16", n.u16s),
		("u32", n.u32s),
		("u64", n.u64s),
		("i8", n.i8s),
		("i16", n.i16s),
		("i32", n.i32s),
		("i64", n.i64s),
	]:
		print(f"{what} {name} {items.tolist()}")
	print(f"{what} f32 {bits(n.f32s, 'I')}")
	print(f"{what} f64 {bits(n.f64s, 'Q')}")
	print(f"{what} some {'None' if n.some is None else f'Some({n.some.tolist()})'}")


def check(ok, what):
	if not ok:
		print(f"failed: {what}", file=sys.stderr)
		sys.exit(1)


def step(loop):
	"""Run the loop until what is ready has run."""
	loop.run_until_complete(asyncio.sleep(0))


def until(loop, done):
	"""Run the loop until `done()`."""

	async def wait():
		while not done():
			await asyncio.sleep(0)

	loop.run_until_complete(asyncio.wait_for(wait(), 5))


def script():
	print(f"limit {api.LIMIT}")

	p = api.Point(1, 2).offset(2, 3)
	print(f"offset {p.x} {p.y}")
	print(f"debug {p!r}")
	print(f"eq {flag(p == api.Point(3, 5))}")
	d = api.Point.default()
	print(f"default {d.x} {d.y}")

	c = api.Counter.new("c", 5)
	print(f"get {c.get()}")
	print(f"name {c.name()}")
	print(f"add ok {c.add(10)}")
	try:
		c.add(2000)
	except api.ErrorException as e:
		print(f"add err {e} {e.error!r}")
	try:
		c.sub(20)
	except api.ErrorException as e:
		print(f"sub err {e} {e.error!r}")
	c.rename("d")
	print(f"name {c.name()}")
	print(f"is d {flag(c.is_('d'))}")
	copy = c.clone()
	copy.add(1)
	print(f"clone {copy.get()} original {c.get()}")
	print(f"debug {c!r}")
	label = c.into_label(p)
	print(f"label {label!r}")
	print(f"note {flag(api.has_text(api.Note('')))} {flag(api.has_text(api.Note(None)))}")

	below, above = api.Counter.new("s", 7).split(5)
	print(f"split {below.name()} {below.get()} {above.name()} {above.get()}")
	(only,) = api.only(api.Counter.new("o", 3))
	print(f"only {only.name()} {only.get()}")
	w8, w16, w32, w64 = api.widths(98305)
	print(f"widths {w8} {w16} {w32} {w64}")
	for b in [2, 0]:
		try:
			q, r = api.divide(7, b)
			print(f"divide {q} {r}")
		except api.ErrorException as e:
			print(f"divide err {e}")
	for n in [98305, 0]:
		spread = api.spread(n)
		print("spread none" if spread is None else f"spread {spread[0]} {spread[1]} {spread[2]} {spread[3]}")
	for needle in [b"lo", b"x"]:
		found = api.find(b"hello", needle)
		print("find none" if found is None else f"find {found[0]} {found[1]}")
	stat_rows = api.rows([api.Point(1, 2), api.Point(3, 3)])
	print("rows " + ", ".join(f"{name} {shape!r} {p.x} {p.y}" for name, shape, p in stat_rows))
	# Each handle in a tuple moves in, as one taken by value does.
	board = api.Board.new([("low", api.Counter.new("l", 1)), ("high", api.Counter.new("h", 9))])
	ranked = board.ranked()
	print("ranked " + ", ".join(f"{name} {counter.name()} {counter.get()}" for name, counter in ranked))
	best = board.best()
	print(f"best {best.name()} {best.get()}")
	# A map crosses as its pairs and a set as its items, which Rust collects in order, each
	# key once, the last kept.
	atlas = api.Atlas([("b", api.Point(1, 1)), ("ab", api.Point(2, 3)), ("b", api.Point(4, 4))], [3, 1, 3])
	atlas = atlas.with_("c", api.Point(5, 6))
	atlas_points = ", ".join(f"{name} {p.x} {p.y}" for name, p in atlas.points)
	print(f"atlas {atlas_points}; ids {' '.join(str(i) for i in atlas.ids)}")
	# Another crate's error crosses as its message.
	for port_text in ["80", "x"]:
		try:
			print(f"port {api.port(port_text)}")
		except api.StringException as e:
			print(f"port err {e.error}")
	span = api.Span((2, 10), ("s", api.Shape.SQUARE))
	lower_half, upper_half = api.halves(span)
	print(f"halves {lower_half[0]} {lower_half[1]} {upper_half[0]} {upper_half[1]} {span!r}")
	swapped_n, swapped_text = api.swapped(("t", 4))
	print(f"swapped {swapped_n} {swapped_text}")
	for value in [None, (None,), (3,)]:
		doubled = api.doubled(value)
		if doubled is None:
			print("doubled None")
		else:
			print(f"doubled Some(({'None' if doubled[0] is None else f'Some({doubled[0]})'},))")
	# A borrowed or boxed tuple of one is a tuple, not its element.
	lent_one = list(data(api.lent_back((bytes([1, 2]),))[0]))
	boxed_one = list(data(api.boxed_tuple(bytes([3]))[0]))
	print(f"lent back {lent_one} boxed {boxed_one}")
	# A borrowed string and an owned one share a tuple struct.
	print(
		f'title Some(("{api.title(True)[0]}",)) {"None" if api.title(False) is None else "Some"} '
		f'named Some(("{api.named("x")[0]}",))'
	)

	# Each lent counter is a new handle to a clone.
	registry = api.Registry.new(api.Counter.new("first", 1))
	print(f"last {flag(registry.last() is not None)}")
	registry.push(api.Counter.new("last", 2))
	first = registry.first_get
	first.add(10)
	print(f"first {registry.first_get.get()} lent {first.get()}")
	last = registry.last()
	last.add(10)
	print(f"last {registry.last().get()} lent {last.get()}")
	# A closure lent for the call edits the first counter through a handle.
	by = 5

	def edit(counter):
		counter.add(by)
		return counter.edited() and counter.get() == 6

	print(f"edit {flag(registry.edit(edit))} {registry.first_get.get()}")
	# A setter moves an object in, as assignment does, closing it.
	registry.first_set(api.Counter.new("second", 3))
	print(f"set first {registry.first_get.get()} {registry.first_get.name()}")
	print(f"walk {api.walk(3, lambda cursor: cursor.at() * 2)}")
	print(f"each {api.each(2, lambda at: print(f'visit {at}'))}")
	print(f"offset {api.offset(lambda: 7, 3)}")
	print(f"shift {api.shift(lambda: 7, 4)}")

	name = api.Name.owned("a")
	joined = name.join(api.Name.owned("b"))
	print(f"joined {joined.as_str()} {joined!r} {flag(joined == name)}")
	joined.rename("c")
	print(f"renamed {joined.as_str()}")
	long = api.longer(joined, api.Name.owned("dd"))
	print(f"longer {long.as_str()}")
	print(f"unnamed [{api.unnamed().as_str()}]")
	lit = api.Name.borrowed("lit")
	shares = [str(lit.shares(api.Name.borrowed(text))).lower() for text in ("lit", "other")]
	print(f"borrowed {lit.as_str()} {' '.join(shares)}")

	print(f"total {api.total(range(2, 5))}")
	print(f"reach {api.reach(2, 5)}")
	scaled = [api.scaled(2**128 - 1, 1), api.scaled(2**64 + 1, 2**63), api.scaled(2**127, 2)]
	print("scaled " + " ".join("none" if v is None else f"{v:032x}" for v in scaled))
	digest = api.Digest.new()
	digest.sum_set(api.fingerprint("deadbeef"))
	try:
		api.fingerprint("zz")
		unparsed = ""
	except api.StringException as e:
		unparsed = e.error
	flipped = digest.flipped(api.ArrayU8x4(bytes([0xFF, 0, 0xFF, 0])))
	print(f"digest {digest.sum_get.hex()} {flipped.hex()} {unparsed}")
	batch, batch2 = api.Batch.new(), api.pair()
	accepted = sum(batch.push(v) for v in range(1, 6))
	paired = sum(batch2.push(v) for v in range(1, 4))
	emptied = api.drain_4(batch)
	print(f"batch {batch.capacity()} {accepted} {emptied} {api.drain_4(batch)} {batch2.capacity()} {paired} {api.drain_2(batch2)}")
	print(f"capacities {api.capacities_4_2(batch, batch2)} {api.capacities_2_2(batch2, batch2)}")
	wide = api.widen(range(2, 5), 1)
	print(f"widen {wide.start} {wide.stop}")
	inc, exc, unb = api.Bound.INCLUDED, api.Bound.EXCLUDED, api.Bound.UNBOUNDED
	counts = [
		api.count(api.BoundsU64(api.BoundU64(start, s), api.BoundU64(end, e)))
		for (start, s), (end, e) in [
			((inc, 2), (exc, 5)),
			((inc, 2), (inc, 5)),
			((inc, 3), (unb, 0)),
			((unb, 0), (unb, 0)),
			((exc, 2), (inc, 4)),
		]
	]
	print("count " + " ".join(str(n) for n in counts))
	print(f"end {bound(api.end(3))} {bound(api.end(0))}")
	ends_start, ends_end = api.ends(3)
	print(f"ends {bound(ends_start)} {bound(ends_end)}")
	for n in [2, 5]:
		try:
			print(f"below {api.below(n, 3)}")
		except api.BoundsU64Exception as e:
			print(f"below err {bound(e.error.start)} {bound(e.error.end)}")
	window = api.Window(range(2, 8), api.end(5))
	print(f"window {flag(window.contains(4))} {flag(window.contains(6))} {window!r}")

	print(api.greet("x", api.Shape.SQUARE))
	print(f"len {api.len(b'abc')}")
	binary = bytes([0xFF, 0, ord("a")])
	print(f"hex {api.hex(binary)}")
	print(f"timed hex {api.timed_hex(binary)}")
	print(f"sample hex {api.sample_hex(binary)}")
	print(f"queued hex {api.queued_hex(binary)}")
	print(f"tick {api.tick_name(api.Label('tick', None, None))}")
	print(f"rooted {api.rooted_name(api.Label('rooted', None, None))}")
	print(f"stored {api.stored_text('stored')}")
	print(f"layer {api.layer_name(api.Label('layer', None, None))}")
	for event in [api.Event.Moved(p), api.Event.Named("n"), api.Event.Done()]:
		print(api.describe(event))

	# A `Copy` handle passed by value is copied, so it stays usable.
	stamp = api.Stamp.from_millis(2500)
	print(f"secs {stamp.secs()}")
	print(f"later {flag(api.Stamp.from_millis(3000).later(stamp))}")
	print(f"secs {stamp.secs()}")
	print(f"whole {stamp.whole()!r}")
	message = api.Message.new(stamp, b"body")
	print(f"payload {text(message.payload_get)}")
	print(f"stamp {message.stamp_get!r}")
	print(f"sent {flag(message.sent())}")
	# Setters assign in place: the bytes are copied in, and a `Copy` handle copied.
	reset = api.Stamp.from_millis(4000)
	message.stamp_set(reset)
	message.payload_set(b"edited")
	print(f"payload {text(message.payload_get)}")
	print(f"stamp {message.stamp_get!r}")
	print(f"secs {reset.secs()}")
	message.stamp_set(None)
	print(f"stamp {flag(message.stamp_get is not None)}")

	# A value past a word, copied across: a closure takes and returns one by value.
	moment = api.Moment.new(1500, 1000)
	print(f"moment {moment.millis()} {flag(api.Moment.new(1, 0) is None)}")
	shifted = api.shifted(moment, lambda m: m.after(500))
	print(f"shifted {shifted.millis()} {flag(shifted == moment.after(500))}")
	print(f"wrapped {moment.after(2**64 - 1).millis()} {flag(moment == shifted)}")
	try:
		api.Moment()
		check(False, "Python makes no opaque value")
	except TypeError:
		pass

	# A padded value crosses with its padding zeroed, and one whose bytes vary is a handle.
	gap = api.Gap.new(7, 3)
	print(f"gap {gap.sum()} {flag(gap == api.Gap.new(7, 3))}")
	berth = api.Berth.new(4)
	print(f"berth Some({berth.at()}) {'None' if api.Berth.new(None).at() is None else 'Some'}")

	# A `Copy` type changed in place is a handle, which a call taking it by value copies.
	score = api.Score.of(2)
	increment = api.Score.of(3)
	score.add(increment)
	score.add(increment)
	print(f"score {score.count()} {increment.doubled().count()} {increment.count()}")
	weighted = api.Weighted(increment, 4)
	print(f"weighted {weighted.total()} {weighted.score.count() + increment.count()}")

	print(f"kept {api.emit(PrintLog(), bytes([1, 2, 3, 4]))}")
	logger = api.Logger.new(PrintLog())
	print(f"write {flag(logger.write('hi'))}")
	del logger
	print("dropped")
	# Rust calls it on another thread, before the call returns.
	threshold = 2

	def keep(level, line):
		print(f"filter {level} {line}")
		return level > threshold

	print(f"filtered {api.filter(bytes([1, 2, 3, 4]), keep)}")

	loop = asyncio.new_event_loop()
	channel = api.Channel.new()
	wakes = 0

	def wake():
		nonlocal wakes
		wakes += 1

	waker = api.Waker(wake, loop)
	# On a loop the script steps, so it sees the receive pend, and its wake resume it.
	got = None

	async def recv():
		nonlocal got
		got = await channel.recv()

	task = loop.create_task(recv())
	step(loop)
	print(f"recv pending {flag(got is None)}")
	channel.send(b"hi")
	check(got is None, "recv never resumes inside the wake")
	loop.run_until_complete(task)
	wakes += 1
	print(f"woken {wakes}")
	print(f"recv {text(got)}")

	async def block():
		channel.send(b"")
		try:
			with await channel.recv_nonempty() as payload:
				print(f"nonempty {payload.nbytes}")
		except api.ErrorException as e:
			print(f"nonempty err {e}")
		filled = await api.fill_4(api.Batch.new(), 9) + await api.fill_2(api.pair(), 1)
		print("fill " + " ".join(map(str, filled)))
		# The send happens at the call: the future is dropped unawaited.
		channel.send_later(b"eager")
		print(f"recv {text(await channel.recv())}")
		# The send happens at the call, and the future borrows the channel and the tag.
		print(f"tagged {await channel.tagged('tag')}")
		print(f"itemized {await channel.itemized(array.array('I', [1, 2, 3]))}")
		print(f"byte_len {await api.byte_len(b'abc')}")
		channel.send(b"sized")
		payload, size = await channel.recv_sized()
		print(f"sized {text(payload)} {size}")

	loop.run_until_complete(block())
	# The waker moves into Rust, which keeps it until the send wakes it.
	channel.notify(waker)
	channel.send(b"rung")
	until(loop, lambda: wakes == 2)
	print(f"woken {wakes}")

	async def polls():
		# A poll with no async twin is a future.
		print(f"take {text(await channel.poll_take())}")
		channel.send(b"measured")
		print(f"measure {await channel.poll_measure(2)}")

	loop.run_until_complete(polls())
	# A context is made of the waker, which Rust keeps until the send wakes it.
	channel.listen(waker)
	channel.send(b"heard")
	until(loop, lambda: wakes == 3)
	print(f"woken {wakes}")
	# A returned waker wakes the waiter's task through Rust until it is dropped.
	# Wakes before the loop gets to them call back once, so it runs between them.
	waiter = api.Waiter.new(waker)
	waiter.waker().wake()
	until(loop, lambda: wakes == 4)
	returned = waiter.context()
	returned.wake()
	until(loop, lambda: wakes == 5)
	print(f"woken {wakes}")
	# Passed back, a returned waker is Rust's own again.
	channel.notify(returned)
	del waiter, returned
	channel.send(b"again")
	until(loop, lambda: wakes == 6)
	channel.clear()
	loop.close()

	nodes = api.Node.new(1, None)
	nodes.push(2)
	print(f"node {nodes.sum()}")

	tally = api.Tally.new()
	tally.bump()
	print(f"tally {tally.bump()} {flag(tally == tally)}")
	print(f"both {api.bump_both(tally, tally)}")
	print(f"peek {api.peek(tally, lambda lent: lent.bump())}")

	area = api.grow(api.Area.new(api.Size(2, 3)), 1)
	size = area.size()
	print(f"area {size.width} {size.height} {area.pixels()}")

	# Generic types, each at the instances it infers, named by their arguments.
	pair = api.PairU32(2, 5)
	swapped = pair.swap()
	print(f"pair {swapped.left} {swapped.right} sum {pair.sum()} {pair!r} {flag(pair == swapped)}")
	print(f"even {flag(pair.even())} {flag(api.PairU32(7, 7).even())}")
	# An instance renamed alone, beside another of `Pair` that keeps its name.
	sized = api.tag(api.Size(1, 2), 3)
	flipped = sized.swap()
	print(f"sized {flipped.left} {flipped.right.width} {flipped!r}")
	beside = api.PairU32(258, 9).beside_size(sized.left)
	print(f"beside {beside.left} {beside!r}")
	# At its default, named as the type itself.
	stack = api.Stack.new()
	for i, n in enumerate(["a", "b"]):
		stack.push(api.Counter.new(n, i + 1))
	top = stack.pop().get()
	print(f"stack {stack.depth()} Some({top}) {flag(stack.pop() is not None)} {flag(stack.pop() is None)}")
	# Generic functions and methods, at each implementation of the trait bounding them, by
	# reference, which leaves Python its object.
	from_point = api.Counter.seeded_point("p", api.Point(-3, 4))
	from_size = api.Counter.seeded_size("s", api.Size(5, 6))
	from_counter = api.Counter.seeded_counter("c", from_point)
	from_polar = api.Counter.seeded_polar_point("q", api.PolarPoint(1, 2))
	print(
		f"seeded {from_point.get()} {from_size.get()} {from_counter.get()} {from_polar.get()} "
		f"past {stack.depth_past_point(api.Point(1, 2))} seeds {api.seeds_point_size(api.Size(2, 3))}"
	)

	# A trait implemented for `Point` and `&Point` apart: one instance.
	print(f"weigh {api.weigh_point(api.Point(-1, 2))}")
	# A function named as the type it returns, and one named as a type it does not.
	polar = api.polar_point(2)
	check(polar is not None, "a point off the origin")
	print(f"polar {polar.distance} {flag(api.polar_point(0) is None)} area {api.area(api.Size(2, 3))}")
	# Two types whose paths join alike, the second kept apart where its path splits.
	print(f"wind {api.wind(api.PolarTurn(2), api.Polar_Turn(3))}")
	# Names that clash in some language, each kept apart there.
	clash_span = api.ClashSpan(4)
	clash_store = api.ClashStore.new(7)
	print(
		f"clash {api.ClashTone(440)!r} {api.ClashTone_(3)!r} {api.Cancelled(2)!r} "
		f"span {clash_span.width_()} {clash_span.to_string()} {clash_span} "
		f"store {clash_store.free()} {clash_store.close()} "
		f"audio {api.clash_audio_encoder_finish()} {api.clash_audio_encoder_finish_()} "
		f"nested {api.clash_nested(((1, 2), 3), ((1, 2, 3),))}"
	)
	# Instances that name each other, and one made only by an impl of it.
	cross = api.cross(api.LeftRightU8(api.RightU8(1)), api.RightLeftU8(api.LeftU8(2)))
	print(f"cross {cross} outer {api.OuterInnerU8.new(4).get()}")
	# A bound that outlives the call takes a value, and an instance only a constant names.
	print(f"kept {api.kept_point(api.Point(1, -2))} boxed {api.BOXED.value}")
	print(f"keep {api.keep_point(api.Point(1, -2))}")
	five = api.Point(5, 2)
	print(f"kept or {api.kept_or_point(five)} {api.kept_or_point(None)} keeper {api.keeper_point(five)}")
	# Inferred instances taking a lent closure and a foreign implementation, and one only
	# its impl of a trait names.
	sides = api.PairU32(2, 5)
	print(f"apply {api.apply_u32(sides, lambda side: side * 3)}")
	print(f"report {api.report_u32(sides, PrintLog())}")
	print(f"wrapped {api.kept_wrap_u32(api.WrapU32(4))}")
	print(f"keeps {api.keeps(PrintLog()).value}")

	# Its Drop runs when its last reference goes, not before.
	token = api.Token.new(7)
	print(f"token {token.id_get} dropped {api.dropped()}")
	del token
	print(f"dropped {api.dropped()}")

	# A field and the method of its name, each its own member.
	retry = api.Retry.new(5)
	limit = retry.limit_get
	print(f"retry {0 if limit is None else 1} {limit or 0} {retry.limit()} {retry.delay()}")
	# The renamed field's setter, named as its getter.
	retry.limit_set(None)
	print(f"retry {retry.limit()}")
	print(f"slot {api.SlotU8.new(200).value()}")

	# Foreign types: a URL parsed from text, socket addresses, OS paths, and `Vec`s.
	base = api.Url.parse("https://example.com/a/b")
	host = api.host(base)
	host = "None" if host is None else f'Some("{host}")'
	print(f"url {base} host {host}")
	url = api.resolve(base.clone(), "c?d")
	print(f"resolve {url} {flag(url == base)}")
	try:
		api.Url.parse("no scheme")
	except api.StringException as e:
		print(f"parse err {e.error}")
	v4 = ("127.0.0.1", 8080)
	v6 = ("::1", 443, 0, 3)
	print(f"ports {api.ports([v4, v6]).tolist()}")
	peer = api.Peer(v6, ["a", "b"]).next("c")
	print(f"peer {peer!r} {quoted(peer.aliases())}")
	file = api.file("dir", "f\\'\"\t")
	print(f"file {escaped(file)}")
	print("parents " + " ".join("none" if p is None else escaped(p) for p in api.parents([file, "/"])))
	words = api.words(" one two  three ")
	print(f"words {len(words)} {quoted(words)}")
	print(f"sentence {api.sentence(words)} {api.sentence(None)}")
	print(f"sentence {api.sentence([])}|")
	print(f"either {api.either('abc')} {api.either(None)}")
	print(f"prefixed {bytes(api.prefixed(b'cd', b'ab')).decode()} {bytes(api.prefixed(b'cd', None)).decode()}")
	print(f"coordinates {api.coordinates(api.Point(2, 3))} {api.coordinates(None)}")
	two, three = api.Counter.new("two", 2), api.Counter.new("three", 3)
	print(f"plus {two.plus(three)} {two.plus(None)}")
	print(f"add_to {str(two.add_to(three)).lower()} {str(two.add_to(None)).lower()} {three.get()}")
	encoded = api.Mut(memoryview(b"0,"))
	api.put_number(1, encoded)
	api.put_number(23, encoded)
	print(f"put_number {bytes(encoded.value).decode()}")
	entry = api.Mut((memoryview(b"ab"), 1))
	stamps = api.push_counted(entry, ord("c"))
	print(f"push_counted {stamps} {bytes(entry.value[0]).decode()} {entry.value[1]}")
	digits = api.Mut(memoryview(b"7x9"))
	first_digit = api.digit(digits)
	try:
		api.digit(digits)
		skipped = False
	except api.ErrorException:
		skipped = True
	print(f"digit {first_digit} {str(skipped).lower()} {api.digit(digits)} {len(digits.value)}")
	nudged = api.Mut(api.Point(1, 2))
	api.nudge(nudged, 3)
	print(f"nudge {nudged.value.x} {nudged.value.y}")
	config = api.secure("h3")
	print(f"secure {api.alpn_lent(config)} {api.alpn(config.clone())}")
	cert = api.certify(b"der")
	print(f"cert {api.cert_len(cert)} {text(api.cert_der(cert))} {api.chain_len([api.certify(b'a'), api.certify(b'bc')])}")
	# Another crate's trait objects: an `Arc`, taken back, and `'static` ones, held by value.
	print(f"resolver {api.resolved(api.resolver('r1'))}")
	codecs = api.Codecs.new()
	print(f"codecs {codecs.ids()}")
	codecs.list_set(list(reversed(codecs.list_get)))
	print(f"codecs reversed {codecs.ids()}")
	# A standard library value no type maps, in a handle.
	failure = api.failure("boom")
	print(f"failure {api.failure_message(failure)} {api.failure_kind(failure)}")
	# Iterators, passed as the arrays of their items.
	print(f"sum_all {api.sum_all(iter([1, 2, 3]))}")
	print(f"weigh_all {api.weigh_all_counter([api.Counter.new('w', 3)])}")
	print(f"offered_len {api.offered_len([b'h3', b'moq-00'])}")
	print(f"join_words {api.join_words(['a', 'b', 'c'])}")
	# Bytes that must be `'static`, interned.
	print(f"der {text(api.Der.from_slice(b'der').bytes())}")
	print(f"der again {text(api.Der.from_slice(b'der').bytes())}")
	session = api.resume("h3", 60)
	print(f"resume {session.alpn()} {session.lifetime().seconds}")
	try:
		api.resume("", 60)
	except api.StringException as e:
		print(f"resume err {e.error}")
	meters = api.Meters.new()
	first = meters.first()
	first.add(2)
	print(f"lent {first.add(3)} {meters.first().count()}")
	meters.clear()
	print(f"cleared {meters.first().count()}")
	lent_last = meters.last_get
	lent_last.add(4)
	print(f"field lent {lent_last.add(1)}")
	# An optional field that is not Clone, lent as `Some` of a handle.
	print(f"spare {str(meters.spare_get is not None).lower()}")
	meters.spare_set(api.Meter.default())
	spare = meters.spare_get
	assert spare is not None
	spare.add(5)
	print(f"spare lent {spare.add(1)}")
	tallies = api.Tallies.new()
	tallies.first().bump()
	print(f"tallied {tallies.first().bump()}")
	print(f"chunks {[list(data(chunk)) for chunk in api.chunks(b'abcde', 2)]}")
	counters = api.counters(["x", "y"])
	print("counters " + ", ".join(f"{c.name()} {c.get()}" for c in counters))
	# Each counter moves in, so Python closes it.
	print(f"worth {api.worth([counters[0]], [counters[1]])} {api.worth(None, [])}")
	for counter in counters:
		try:
			counter.get()
			check(False, "a counter moved in is closed")
		except ValueError:
			pass
	held = [api.held_count((api.Counter.new("h", 4),)), api.held_count(None)]
	print("held " + " ".join("None" if n is None else f"Some({n})" for n in held))
	print(f'boxed Some("{api.boxed_name(api.boxed_counter("b"))}") {api.boxed_name(None)} {str(api.boxed_counter(None) is None).lower()}')
	print(f"less {api.less(5, 2)}")
	rows = [array.array("I", [1, 2]), array.array("I"), memoryview(struct.pack("=I", 3)).cast("I")]
	print(f"sums {api.sums(rows).tolist()}")
	print("least " + " ".join(str(s) for s in api.least([{3, 1}, set(), [2]])))
	numbers("edges", api.Numbers.edges())
	numbers("reversed", api.Numbers.edges().reversed())
	edges = api.Numbers.edges()
	nan = api.Measure.new(math.nan, False)
	equal = [edges == edges, edges == api.Numbers.edges(), api.Numbers.zero(True) == api.Numbers.zero(False), nan == nan, api.Measure.new(-0.0, True) == api.Measure.new(0.0, False)]
	print("float equal " + " ".join(str(e).lower() for e in equal))
	# A record compares a buffer passed in by its numbers, as Rust does, not by its own `==`.
	held = [api.Shadowed(Elementwise("i", items), 1, 2) for items in [[1, 2], [1, 2], [1, 3]]]
	check(held[0] == held[1] and not held[0] == held[2], "a record compares a buffer by its numbers")
	print(f"sum u32 {api.sum_u32(array.array('I'))} {api.sum_u32(array.array('I', [2**32 - 1, 2**32 - 1, 2]))}")
	print(f"doubled i64 {api.doubled_i64(array.array('q', [-(2**63), -1, 0, 2**63 - 1])).tolist()}")
	floats = api.boxed_floats(array.array("d", struct.pack("=dQ", -0.0, 0x7FF8000000000001)))
	print(f"boxed floats {bits(floats, 'Q')}")
	summed = api.summed([api.Point(1, 2), api.Point(-4, 6)])
	none_summed = api.summed([])
	print(f"summed {summed.x} {summed.y} {none_summed.x} {none_summed.y}")
	print(f"longest {api.longest(['a', 'abc'])} {api.longest([])}")
	print(f'first of Some("{api.first_of(["a", "b"])}") {api.first_of([]) or "None"}')
	limits = api.byte_limits(b'a', b'bc')
	waits = api.LimitsDuration(datetime.timedelta(milliseconds=5), datetime.timedelta(seconds=2))
	breadth = api.breadth(waits) // datetime.timedelta(milliseconds=1)
	print(f"limits {bytes(limits.low).decode()} {bytes(limits.high).decode()} breadth {breadth}")
	joined = bytes(api.joined([b'ab', b'', b'c'])).decode()
	print(f"joined {joined} {len(api.joined([]))} read {api.read_len(b'hello')}")
	doc = api.Json.parse('{"b": [1, 2.5, null], "a": "é"}')
	print(f"json {api.encode(doc)}")
	print(f"timed text {api.timed_text(doc)}")
	tagged = api.tagged(doc, "t")
	print(f"tagged {tagged.to_text()}")
	# A JSON object's entries cross as pairs of a key and its document, each moved in.
	entries = api.entries([("b", api.Json.parse("[1]")), ("a", api.Json.parse("2"))], "t")
	print("entries " + ", ".join(f"{key} {value.to_text()}" for key, value in entries))
	try:
		api.Json.parse('{"a":')
	except api.StringException as e:
		print(f"json parse err {e.error}")
	site = api.Site.new(base)
	print(f"site {site.home_get} {site.home_get}")

	# Slices, passed in and returned as clones, and the socket address of each family.
	print(f"shapes {debugs(api.Shape.ALL)} greetings {quoted(api.GREETINGS)}")
	print(f"both {debugs(api.Shape.BOTH)} primes {list(api.PRIMES)}")
	trail = api.Trail.new([api.Point(1, 2), api.Point(3, 4)], ["a", "b"])
	print(f"trail {debugs(trail.points())} {debugs(trail.labels())}")
	# Returned iterators, each collected at the call: drained labels are gone after.
	print(f"iterated {debugs(trail.iter())} {quoted(trail.names())} {list(data(trail.bytes()))}")
	print(f"drained {debugs(trail.drain())} then {debugs(trail.labels())}")
	# A filter, collected at the call.
	print("beyond " + ", ".join(f"{p.x} {p.y}" for p in trail.beyond(1)))
	numbered = api.numbered(["p", "q"])
	print("numbered " + ", ".join(f"{c.name()} {c.get()}" for c in numbered))
	next4, next6 = api.nexts(v4, v6)
	print(f"nexts {next4[0]} {next4[1]} {next6[1]} {next6[3]}")

	# serde_json's map iterator, collected as pairs of a key and its document.
	sections = api.sections([("b", api.Json.parse("[1]")), ("a", api.Json.parse("2"))])
	print("sections " + ", ".join(f"{key} {value.to_text()}" for key, value in sections))
	# An instant only Rust makes, which each call copies out of its handle, and compares.
	ms = datetime.timedelta(milliseconds=1)
	began = api.now()
	ended = api.later(began, datetime.timedelta(milliseconds=1500))
	same = api.later(began, datetime.timedelta())
	print(f"instants {api.since(ended, began) // ms} {api.since(began, ended) // ms} {flag(began == same)} {flag(began == ended)}")
	print(f"has_instant {flag(api.has_instant(began))} {flag(api.has_instant(None))}")
	# tracing's level, and its filter as an optional level whose None is `OFF`.
	verbosity = api.Verbosity.new(api.Level.WARN)
	print(f"verbosity {api.spell(verbosity.filter())} {api.spell(api.louder(verbosity.filter()))}")
	verbosity.level_set(api.Level.TRACE)
	louder = api.spell(api.louder(verbosity.filter()))
	print(
		f"verbosity {flag(verbosity.level_get == api.Level.TRACE)} {api.spell(verbosity.filter())} {louder} {api.spell(api.louder(None))}"
	)

	print(f"made {api.made(3, lambda n: f'{n} times')}")
	# A closure taking and returning `Copy` objects copies them, never running their `Clone`.
	kept = api.Ticket.new(9)
	given = api.reissue(api.Ticket.new(7), lambda t: api.Ticket.new(t.number() + 1)).number()
	held = api.reissue(api.Ticket.new(7), lambda _: kept).number()
	print(f"reissued {given} {held}, cloned {api.Ticket.clones()}")
	# A kept closure, called on another thread after the call that took it returned.
	print(f"spawned {api.Spawned.new(lambda: 7).join()}")
	# A kept closure: called again at the instant it asks for, then once its waiter wakes.
	start = api.now()
	calls = 0

	def drive(now, waiter):
		nonlocal calls
		calls += 1
		match calls:
			case 1:
				return api.later(now, datetime.timedelta(milliseconds=5))
			case 2:
				print(f"drive {api.since(now, start) // ms}")
				waiter.waker().wake()
				return None
			case _:
				raise api.StringException(f"stopped after {calls}")

	print(f"drive {asyncio.run(api.drive(start, drive))}")
	for file in (io.BytesIO(b"hello world"), Broken()):
		try:
			tail, at = api.tail(file, 5)
			print(f"tail {text(tail)} {at}")
		except api.StringException as e:
			print(f"tail error {e}")
	# A seek from the start reaches the reader whole, past `i64::MAX`, as `Cursor` takes one.
	try:
		print(f"sought {api.seek_to(Far(), 2**64 - 1)}")
	except api.StringException as e:
		print(f"sought error {e}")
	print(f"non zeros {api.describe_non_zeros(api.non_zeros())}")
	for hop in [None, 1, 2**64 - 1]:
		next = api.next_hop(hop)
		print("hop None" if next is None else f"hop Some({next})")
	print(f"farther hop {api.farther_hop(api.PairNonZeroU16(3, 0xFFFF))}")


class Elementwise(array.array):
	"""A buffer whose `==` compares item by item into an object of no truth, as numpy's does.
	A native buffer, since CPython before 3.12 takes no `__buffer__` written in Python."""

	def __eq__(self, other):
		return NoTruth()


class NoTruth:
	def __bool__(self):
		raise ValueError("the truth value of an array is ambiguous")


class Broken:
	"""A reader whose reads fail, as the transcript's: each seek lands at 0."""

	def readinto(self, buffer):
		raise OSError("broken")

	def seek(self, offset, whence=0):
		return 0


class Far:
	"""A reader of nothing, which seeks anywhere from its start, as Rust's `Cursor` does."""

	def readinto(self, buffer: memoryview, /) -> int:
		return 0

	def seek(self, offset: int, whence: int = 0, /) -> int:
		return offset


def futures():
	"""The futures' semantics beyond the transcript, each checked by an assertion."""
	channel = api.Channel.new()

	# The send lands from another thread while the receive waits.
	async def threaded():
		sender = threading.Thread(target=lambda: channel.send(b"blocked"))
		sender.start()
		payload = await asyncio.wait_for(channel.recv(), 5)
		sender.join()
		check(text(payload) == "blocked", "recv returns the payload another thread sent")

	asyncio.run(threaded())

	loop = asyncio.new_event_loop()

	# Resumed on its loop, never inside the wake.
	got = None

	async def recv():
		nonlocal got
		got = text(await channel.recv())

	task = loop.create_task(recv())
	step(loop)
	check(got is None, "recv waits for a send")
	channel.send(b"then")
	check(got is None, "recv never polls inside the wake")
	loop.run_until_complete(asyncio.wait_for(task, 5))
	check(got == "then", "recv resumes with the payload")

	# A wake that lands while the future polls, before it returns pending, is kept: the
	# future polls again rather than waiting for a wake that already happened.
	check(loop.run_until_complete(asyncio.wait_for(api.yielded(7), 5)) == 7, "a wake during the poll polls again")
	check(loop.run_until_complete(api.corner_later()) == (1, 1), "a future's borrowed tuple returns whole")

	# Cancelled between its wake and its poll, a receive loses nothing: the payload stays
	# for the next.
	called = False

	async def cancelled():
		nonlocal called
		await channel.recv()
		called = True

	task = loop.create_task(cancelled())
	step(loop)
	channel.send(b"kept")
	task.cancel()
	until(loop, task.done)
	check(not called and task.cancelled(), "a cancelled receive never resumes")
	payload = loop.run_until_complete(asyncio.wait_for(channel.recv(), 5))
	check(text(payload) == "kept", "a cancelled receive loses nothing")

	# An object whose last reference goes while a call awaits on it is freed once the
	# call ends.
	other = api.Channel.new()
	sender = other.clone()
	task = loop.create_task(other.recv())
	step(loop)
	del other
	sender.send(b"held")
	got = loop.run_until_complete(asyncio.wait_for(task, 5))
	check(text(got) == "held", "the call awaiting on an object holds it")

	# An awaitable from Rust is awaited once, as a coroutine is.
	later = channel.send_later(b"once")

	async def wait(awaitable):
		return await awaitable

	loop.run_until_complete(asyncio.wait_for(wait(later), 5))
	try:
		loop.run_until_complete(wait(later))
		check(False, "a second await raises")
	except RuntimeError:
		pass
	check(text(loop.run_until_complete(asyncio.wait_for(channel.recv(), 5))) == "once", "the send landed once")

	# Python run while a closing future is freed, as by its drop's callback, finds it ended.
	class Polls(api.Log):
		def log(self, level, line):
			return True

		def close(self):
			try:
				later.poll(api._n.Queue(loop))
				check(False, "a future polled while it is freed raises")
			except ValueError:
				pass

	later = api.log_later(Polls())
	later.close()

	# A waker woken from another thread calls back on its loop, once for wakes that land
	# before the loop gets to them.
	wakes = []
	waker = api.Waker(lambda: wakes.append(threading.get_ident()), loop)
	channel.notify(waker)
	sender = threading.Thread(target=lambda: channel.send(b"rung"))
	sender.start()
	sender.join()
	until(loop, lambda: wakes)
	check(wakes == [threading.get_ident()], "a waker calls back on its loop's thread")
	loop.run_until_complete(asyncio.wait_for(on_loop(), 5))
	loop.run_until_complete(asyncio.wait_for(foreign(), 5))
	loop.run_until_complete(asyncio.wait_for(polled(), 5))
	loop.run_until_complete(asyncio.wait_for(kept(), 5))
	loop.close()
	readers()
	print("futures ok")


async def polled():
	"""A poll of a waiter, made of each poll's waker, of an awaitable Rust owns and lends it
	pinned: closed once it finishes, or once the future is dropped first."""
	gate = asyncio.get_running_loop().create_future()
	ran = []

	async def lent():
		await gate
		ran.append("lent")

	task = asyncio.ensure_future(api.Waiter.poll_future(lent()))
	for _ in range(3):
		await asyncio.sleep(0)
	check(not task.done() and not ran, "a polled future waits on the awaitable it owns")
	gate.set_result(None)
	await task
	check(ran == ["lent"], "a polled future finishes once its awaitable does")

	cancelled = []

	async def never():
		try:
			await asyncio.get_running_loop().create_future()
		except asyncio.CancelledError:
			cancelled.append(True)
			raise

	try:
		await asyncio.wait_for(api.Waiter.poll_future(never()), 0.05)
		check(False, "a polled future that never ends times out")
	except asyncio.TimeoutError:
		pass
	for _ in range(3):
		await asyncio.sleep(0)
	check(cancelled == [True], "dropping a polled future cancels the awaitable it owns")


class Drive:
	"""drive's closure, which only ever waits, or stops at once."""

	def __init__(self, stop):
		self.stop = stop

	def __call__(self, now, waiter):
		if self.stop:
			raise api.StringException("stopped")
		return None


async def kept():
	"""Closures Rust keeps past the call, which it lets go of once it drops them."""
	start = api.now()
	stopping = Drive(True)
	ref = weakref.ref(stopping)
	check(await api.drive(start, stopping) == "stopped", "a kept closure's error ends its future")
	del stopping
	gc.collect()
	check(ref() is None, "a finished future lets go of its closure")

	# Dropped before it finishes, it lets go of the closure it kept. Cancelled without
	# awaiting it, as a traceback through its frame would keep the closure.
	idle = Drive(False)
	ref = weakref.ref(idle)
	task = asyncio.ensure_future(api.drive(start, idle))
	for _ in range(3):
		await asyncio.sleep(0)
	check(not task.done(), "a drive that only waits pends")
	task.cancel()
	for _ in range(3):
		await asyncio.sleep(0)
	check(task.cancelled(), "a drive is cancelled")
	del idle, task
	gc.collect()
	check(ref() is None, "a dropped future lets go of its closure")

	# Refused at the call, before Rust takes it.
	try:
		await api.drive(start, 42)  # type: ignore[arg-type]
		check(False, "a value that is not callable is refused")
	except TypeError:
		pass

	# Called on Rust's own thread, after the call that took it returned.
	threads = []

	class Task:
		def __call__(self):
			threads.append(threading.get_ident())
			return 9

	task = Task()
	ref = weakref.ref(task)
	spawned = api.Spawned.new(task)
	del task
	check(spawned.join() == 9, "a spawned closure returns to Rust")
	gc.collect()
	check(threads != [threading.get_ident()] and ref() is None, "a spawned closure runs elsewhere, then is let go")

	# What a closure returns, Rust owns: a copy of a string, and a clone of an object.
	later = api.later(start, datetime.timedelta(milliseconds=7))
	calls = 0

	def ask(now, waiter):
		nonlocal calls
		calls += 1
		if calls == 1:
			return later
		check(api.since(now, later) == datetime.timedelta(), "Rust's copy of a returned object is its value")
		raise api.StringException("done")

	check(await api.drive(start, ask) == "done", "a closure's returned object is Rust's own copy")
	check(api.since(later, start) == datetime.timedelta(milliseconds=7), "a returned object stays the caller's")


def readers():
	"""Readers Rust reads and seeks during the call."""
	data = bytes(range(256)) * 64
	file = io.BytesIO(data)
	tail, at = api.tail(file, 1000)
	check(bytes(tail) == data[-1000:] and at == len(data) - 1000, "a long tail reads in several reads")
	check(file.tell() == at, "Rust's seeks move the reader")

	with open(__file__, "rb") as opened:
		size = os.fstat(opened.fileno()).st_size
		tail, at = api.tail(opened, 10)
		check(len(tail) == 10 and at == size - 10, "a file opened for reading is a reader")

	class Seeks(io.BytesIO):
		def seek(self, offset, whence=0, /):
			if whence == os.SEEK_END:
				raise OSError("no end")
			return super().seek(offset, whence)

	try:
		api.tail(Seeks(b"abc"), 1)
		check(False, "a failed seek fails the call")
	except api.StringException as e:
		check(str(e) == "no end", "a failed seek's message crosses")

	try:
		api.tail(object(), 1)  # type: ignore[arg-type]
		check(False, "a value with no readinto is refused")
	except TypeError:
		pass


async def foreign():
	"""Awaitables Rust polls as futures, each run as a task on the loop it was passed on."""
	ran = []

	async def ready():
		ran.append("ready")

	check(await api.after(ready(), 4) == 8 and ran == ["ready"], "Rust awaits a coroutine")

	# Rust waits on a coroutine that waits, which wakes it once it ends.
	gate = asyncio.get_running_loop().create_future()

	async def later():
		await gate
		ran.append("later")

	task = asyncio.ensure_future(api.after(later(), 5))
	for _ in range(3):
		await asyncio.sleep(0)
	check(not task.done() and ran == ["ready"], "Rust waits on a coroutine that waits")
	gate.set_result(None)
	check(await task == 10 and ran == ["ready", "later"], "a coroutine's end wakes Rust")

	# Given up on before it starts, a coroutine is closed, never run.
	try:
		await api.within(0, ready())
		check(False, "within gives up on a coroutine")
	except api.ErrorException as e:
		check(str(e) == "overflow", "within gives up on a coroutine")
	for _ in range(3):
		await asyncio.sleep(0)
	check(ran == ["ready", "later"], "a coroutine Rust gave up on before it started never runs")

	# Dropped while it runs, its task is cancelled.
	cancelled = []

	async def never():
		try:
			await asyncio.get_running_loop().create_future()
		except asyncio.CancelledError:
			cancelled.append(True)
			raise

	try:
		await asyncio.wait_for(api.within(1, never()), 0.05)
		check(False, "a coroutine that never ends times out")
	except asyncio.TimeoutError:
		pass
	for _ in range(3):
		await asyncio.sleep(0)
	check(cancelled == [True], "dropping a future Rust polls cancels its task")

	# Refused at the call, before Rust takes it: what is not awaitable, and a future of
	# another loop, which Rust's first poll could not start.
	try:
		await api.after(42, 1)  # type: ignore[arg-type]
		check(False, "a value that is not awaitable is refused")
	except TypeError:
		pass
	other = asyncio.new_event_loop()
	try:
		await api.after(other.create_future(), 1)
		check(False, "a future of another loop is refused")
	except ValueError:
		pass
	other.close()



async def on_loop():
	"""Wakes during a call on the running loop's own thread, which write no byte to its
	socket and dispatch once the call returns, beside wakes from other threads, which do."""
	channel = api.Channel.new()
	got = []

	async def recv():
		got.append(text(await channel.recv()))

	async def parked():
		task = asyncio.ensure_future(recv())
		await asyncio.sleep(0)
		return task

	def signaled():
		return wakeup.queue.signaled()

	async def settle():
		for _ in range(4):
			await asyncio.sleep(0)

	task = await parked()
	wakeup = api._wakeups[asyncio.get_running_loop()]
	channel.send(b"near")
	check(not got, "a wake on the loop's thread never resumes inside the call")
	check(not signaled(), "a wake on the loop's thread writes no byte")
	await task
	check(got == ["near"], "a wake on the loop's thread resumes the receive")

	task = await parked()
	sender = threading.Thread(target=lambda: channel.send(b"far"))
	sender.start()
	sender.join()
	check(signaled(), "a wake from another thread writes a byte")
	await task
	check(got == ["near", "far"], "a wake from another thread resumes the receive")

	# Before the first await, the call's poll with no waker leaves nothing to wake.
	early = channel.recv()
	channel.send(b"early")
	check(text(await early) == "early", "a send before the first await is received")

	# Cancelled after its wake, or before it, in the same step, a receive never resumes and
	# leaves the payload.
	for send_first in (True, False):
		task = await parked()
		if send_first:
			channel.send(b"kept")
			task.cancel()
		else:
			task.cancel()
			channel.send(b"kept")
		await settle()
		check(task.cancelled() and len(got) == 2, "a cancelled receive never resumes")
		check(text(await channel.recv()) == "kept", "a cancelled receive loses nothing")
	check(not signaled(), "wakes on the loop's thread wrote no byte")

	# Wakes that land before the loop gets to them call the waker once.
	wakes = []
	waker = api.Waker(lambda: wakes.append(1))
	for _ in range(2):
		channel.notify(waker)
		channel.send(b"rung")
	check(not wakes, "a waker is never called inside the call")
	check(not signaled(), "a waker's wakes on the loop's thread, with nothing parked, write no byte")
	await settle()
	check(wakes == [1], "wakes on the loop's thread call the waker once")
	await channel.recv()

	# Many receives, each woken once from the loop's thread or another, while both wake.
	channels = [api.Channel.new() for _ in range(64)]
	for _ in range(20):
		tasks = [asyncio.ensure_future(c.recv()) for c in channels]
		await asyncio.sleep(0)
		sender = threading.Thread(target=lambda: [c.send(b"far") for c in channels[1::2]])
		sender.start()
		for c in channels[::2]:
			c.send(b"near")
		payloads = await asyncio.gather(*tasks)
		sender.join()
		expected = ["near", "far"] * (len(channels) // 2)
		check([text(p) for p in payloads] == expected, "every receive resumes once with its payload")


def owned():
	"""What frees an object or bytes: its last reference going, or the cycle collector.
	Rust's bytes cross lent, until they and every view of them are released."""
	for n in range(1023, 1026):
		payload = bytes(i % 256 for i in range(n))
		packet = api.next_packet(api.Packet(1, payload))
		check(packet.seq == 2 and data(packet.payload) == payload, f"a record's {n} bytes cross both ways")
	freed = api.dropped()
	for n in range(1023, 1026):
		view = api.token_bytes(n, n)
		check(api.dropped() == freed, f"{n} bytes are lent, not copied")
		check(bytes(view) == bytes([n % 256]) * n, f"{n} bytes read back")
		view.release()
		freed += 1
		check(api.dropped() == freed, f"releasing {n} bytes frees them")
		view.release()
		check(api.dropped() == freed, f"releasing {n} bytes again frees nothing")
		try:
			bytes(view)
			check(False, f"released {n} bytes refuse a read")
		except ValueError:
			pass

	before = api.dropped()
	api.Token.new(3)
	api.token_bytes(4, 1025)
	check(api.dropped() == before + 2, "the last reference going frees at once")

	# A view kept past what it was read from keeps Rust's buffer.
	part = api.token_bytes(7, 1025)[1:]
	packet = api.next_packet(api.Packet(1, bytes(range(256)) * 5))
	payload = packet.payload
	del packet
	check(api.dropped() == before + 2, "a view keeps Rust's buffer")
	check(bytes(part) == bytes([7]) * 1024, "a slice outlives the bytes it was cut from")
	check(bytes(payload) == bytes(range(256)) * 5, "bytes outlive their record")
	del part
	check(api.dropped() == before + 3, "the last view going frees Rust's buffer")

	cycle = [api.Token.new(5), api.token_bytes(6, 1)]
	cycle.append(cycle)
	del cycle
	gc.collect()
	check(api.dropped() == before + 5, "the cycle collector frees what a cycle held")
	forwarding()
	released_mid_call()


def released_mid_call():
	"""Rust's bytes a call reads in place stay Rust's until it returns, though the view
	passed is released while the arguments are still being read."""
	kept = []

	def parts():
		before = api.dropped()
		with api.token_bytes(9, 2048) as view:
			yield view
		kept.append(api.dropped() == before)
		yield b"x"

	before = api.dropped()
	joined = bytes(api.joined(parts()))
	check(kept == [True], "a call keeps Rust's bytes it was passed a released view of")
	check(joined == bytes([9]) * 2048 + b"x", "a released view's bytes read back")
	check(api.dropped() == before + 1, "Rust's bytes drop once the call returns")
	print("owned ok")


def forwarding():
	"""Rust's bytes, read and passed back, cross in place: whole, sliced, kept while passed
	on, and in a record. The token's drop shows no copy stood in for them: it comes once,
	only when the last view of the bytes Rust put it in goes."""
	loop = asyncio.new_event_loop()
	channel = api.Channel.new()
	before = api.dropped()
	payload = api.token_bytes(3, 64)
	channel.send(payload)
	del payload
	payload = loop.run_until_complete(channel.recv())
	channel.send(payload[1:63])
	del payload
	payload = loop.run_until_complete(channel.recv())
	check(bytes(payload) == bytes([3]) * 62, "a slice of Rust's bytes crosses as itself")
	channel.send(payload)
	kept = loop.run_until_complete(channel.recv())
	del payload
	packet = api.next_packet(api.Packet(1, kept))
	del kept
	check(api.dropped() == before and len(packet.payload) == 62, "Rust's bytes cross back without a copy")
	del packet
	check(api.dropped() == before + 1, "Rust's bytes drop once, with their last view")
	# Bytes of Python's own cross copied once, and a view of Rust's only read stays theirs.
	packet = api.next_packet(api.Packet(1, bytearray(b"abc")))
	check(bytes(packet.payload) == b"abc", "a bytearray crosses copied")
	loop.close()


def values():
	"""The values Python's types bound: integers checked against their Rust type's range,
	durations truncated to the microsecond up to `timedelta.max`, which stands for longer,
	and on Unix, a path whose bytes are not UTF-8, which crosses as `os.fsdecode` decodes it,
	and back as it came. Also a borrowed tuple, returned whole."""

	def raises(error, f, what):
		try:
			f()
			check(False, what)
		except error:
			pass

	raises(ValueError, lambda: api.ArrayU8x4(b"abc"), "3 bytes are no [u8; 4]")
	raises(OverflowError, lambda: api.widths(2**64), "an int past u64 raises")
	raises(OverflowError, lambda: api.widths(-1), "a negative int for a u64 raises")
	raises(TypeError, lambda: api.title(1), "an int is no bool")
	raises(OverflowError, lambda: api.scaled(2**128, 1), "an int past u128 raises")
	raises(OverflowError, lambda: api.scaled(-1, 1), "a negative int for a u128 raises")

	# An object reclassed in Python is still what Rust made: its methods refuse it.
	stamp = api.Stamp.from_millis(5)
	stamp.__class__ = api.Moment
	raises(TypeError, lambda: stamp.millis(), "a Stamp reclassed as a Moment is no Moment")
	counter = api.Counter.new("r", 1)
	counter.__class__ = api.Berth
	raises(TypeError, lambda: counter.at(), "a Counter reclassed as a Berth is no Berth")

	# A list of numbers is any buffer of exactly its numbers, one dimension and contiguous,
	# and returns as an array of them.
	edges = api.Numbers.edges()
	check(type(edges.u32s) is array.array and edges.u32s.typecode == "I", "a u32 list is an array of 'I'")
	check(bits(edges.reversed().reversed().f64s, "Q") == bits(edges.f64s, "Q"), "floats cross back exactly")
	check(edges.reversed().reversed().i64s == edges.i64s, "integers cross back exactly")
	check(bits(api.boxed_floats(edges.f64s), "Q") == bits(edges.f64s, "Q"), "a NaN keeps its payload")
	raises(TypeError, lambda: api.sum_u32(array.array("i", [1])), "an i32 buffer is no u32 list")
	raises(TypeError, lambda: api.sum_u32(array.array("H", [1])), "a u16 buffer is no u32 list")
	raises(TypeError, lambda: api.sum_u32(b"\0\0\0\0"), "bytes are no u32 list")
	raises(TypeError, lambda: api.sum_u32([1]), "a list is no buffer")
	square = memoryview(struct.pack("=4I", 1, 2, 3, 4)).cast("B").cast("I", [2, 2])
	raises(TypeError, lambda: api.sum_u32(square), "a 2D buffer is no list")
	raises(BufferError, lambda: api.sum_u32(memoryview(array.array("I", [1, 2, 3, 4]))[::2]), "a strided buffer")
	check(api.sum_u32(memoryview(struct.pack("=2I", 5, 6)).cast("I")) == 11, "a memoryview cast to u32s is one")
	# The same numbers under another code of their width and kind, as numpy's may be.
	code = "L" if struct.calcsize("L") == 4 else "I"
	check(api.sum_u32(memoryview(struct.pack(f"={2}I", 5, 6)).cast(code)) == 11, "a buffer of another u32 code")
	n = api.Numbers.edges()
	n.some = None
	check(n.reversed().some is None, "a missing list is None")
	n.some = array.array("i")
	check(n.reversed().some == array.array("i"), "an empty list is not None")

	# A borrowed tuple is one C value, not one out per element.
	check(api.corner(True) == (1, 1), "a borrowed tuple returns whole")
	raises(api.ErrorException, lambda: api.corner(False), "a borrowed tuple's error raises")

	td = datetime.timedelta
	check(api.doubled_ttl(td(seconds=1, microseconds=5)) == td(seconds=2, microseconds=10), "a duration crosses")
	check(api.doubled_ttl(td(days=600_000_000)) == td.max, "a duration past timedelta's is its max")
	check(api.doubled_ttl(td.max) == td.max, "timedelta's max is Duration::MAX")
	raises(ValueError, lambda: api.doubled_ttl(td(seconds=-1)), "a negative duration raises")

	# The module serves wasm32 too: what only wasm32 has raises on the host's library.
	check(api.pointer_bits(api.platform()) == api.pointer_bits(api.Platform.Native(64)) == 64, "the host is native")
	check(api.is_family(api.Family.NATIVE), "and of the native family")
	for f, what in [
		(api.wasm_only, "api_wasm_only"),
		(lambda: api.pointer_bits(api.Platform.Wasm(32)), "api_Platform.Wasm"),
		(lambda: api.is_family(api.Family.WASM), "api_Family.Wasm"),
	]:
		try:
			f()
			check(False, f"{what} raises")
		except NotImplementedError as e:
			check(str(e) == f"{what}: it is not on this library's target", repr(e))

	if os.name == "posix":
		path = api.stray()
		check(os.fsencode(path) == b"\xff", "a path that is not UTF-8 keeps its bytes")
		check(api.parents([path, os.fsdecode(b"\xfe/\xff")])[1] == os.fsdecode(b"\xfe"), "and crosses back")
	print("values ok")


def calls():
	"""Calls Python makes straight into the extension, with no Python frame between, as a
	`def` of the same signature takes them: by position or keyword, raising its errors,
	with its docstring and the signature `inspect` reads."""
	import inspect

	def raises(f, message):
		try:
			f()
			check(False, message)
		except TypeError as e:
			check(str(e) == message, f"{e!r} is {message!r}")

	kinds = {
		(api.Counter, "get"): "method_descriptor",
		(api.Counter, "new"): "staticmethod",
		(api.Counter, "__repr__"): "method_descriptor",
		(api.Registry, "first_get"): "getset_descriptor",
		(api.Point, "offset"): "method_descriptor",
		(api.Channel, "send_later"): "method_descriptor",
		# An `async def` stays a coroutine function.
		(api.Gauge, "later"): "function",
	}
	for (cls, name), kind in kinds.items():
		got = type(inspect.getattr_static(cls, name)).__name__
		check(got == kind, f"{cls.__name__}.{name} is a {got}")
	check(type(api.has_text).__name__ == "builtin_function_or_method", "a function is the extension's")

	c = api.Counter.new(start=2, name="c")
	check(c.add(n=3) == 5 and c.sub(1) == 4, "a method takes its arguments by keyword")
	check(api.Point(1, 2).offset(dy=3, dx=4) == api.Point(5, 5), "a record's method takes keywords")
	check(api.has_text(note=api.Note.default()) is False, "a function takes keywords")
	check(api.Registry.new(first=c).edit(edit=lambda counter: True), "a closure passes by keyword")
	loop = asyncio.new_event_loop()
	check(loop.run_until_complete(api.halved(n=4)) == 2, "an async def takes keywords")
	channel = api.Channel.new()
	loop.run_until_complete(channel.send_later(payload=b"kw"))
	check(bytes(loop.run_until_complete(channel.recv())) == b"kw", "an eager future takes keywords")
	check(loop.run_until_complete(api.either_later("ab")) == 2, "a future owns an optional borrow")
	check(loop.run_until_complete(api.either_later(None)) == 0, "a future owns a None borrow")
	two, three = api.Counter.new("two", 2), api.Counter.new("three", 3)
	check(loop.run_until_complete(two.plus_later(three)) == 5, "a future holds an optional handle")
	check(loop.run_until_complete(two.plus_later(None)) == 2, "a future holds a None handle")
	loop.close()

	# C callers may pass no argument array at all for no arguments.
	import ctypes

	no_args = ctypes.pythonapi.PyObject_CallNoArgs
	no_args.restype, no_args.argtypes = ctypes.py_object, [ctypes.py_object]
	check(no_args(api.Note.default) == api.Note.default(), "a call with no argument array")

	raises(lambda: c.add(), "Counter.add() missing 1 required positional argument: 'n'")
	raises(lambda: c.add(1, 2), "Counter.add() takes 2 positional arguments but 3 were given")
	raises(lambda: c.add(1, n=2), "Counter.add() got multiple values for argument 'n'")
	raises(lambda: c.add(m=1), "Counter.add() got an unexpected keyword argument 'm'")
	raises(lambda: c.get(1), "Counter.get() takes 1 positional argument but 2 were given")
	raises(lambda: api.Counter.new(), "Counter.new() missing 2 required positional arguments: 'name' and 'start'")
	raises(lambda: api.has_text(1, 2), "has_text() takes 1 positional argument but 2 were given")

	check(str(inspect.signature(c.add)) == "(n)", "a bound method's signature")
	check(str(inspect.signature(api.Counter.new)) == "(name, start)", "a static method's signature")
	check(api.has_text.__doc__ == "Whether `note` has text, even empty.", "a function's docstring")
	check(api.Gauge.set.__doc__ == "Set the level each clone reads.", "a method's docstring")
	check(api.Counter.get.__doc__ is None, "a method without docs has no docstring")
	check(api.Registry.first_get.__doc__ is None, "a property without docs has no docstring")
	print("calls ok")


def broken(which):
	"""Each breaks a contract, which must fail with a message naming it."""
	if sys.platform == "linux":
		# Non-dumpable, an expected abort writes no core and never starts the host's crash
		# handler. PR_SET_DUMPABLE is 4.
		import ctypes

		if ctypes.CDLL(None, use_errno=True).prctl(4, 0, 0, 0, 0) != 0:
			raise OSError(ctypes.get_errno(), "prctl")
	match which:
		case "moved":
			c = api.Counter.new("c", 0)
			c.into_label(None)
			c.get()
		case "boxed moved":
			tail = api.Node.new(1, None)
			api.Node.new(2, tail)
			tail.sum()
		case "outlived":
			meters = api.Meters.new()
			first = meters.first()
			meters.clear()
			first.count()
		case "lent moved":
			api.Registry.new(api.Counter.new("c", 0)).edit(lambda counter: counter.into_label(None) and False)
		case "lent kept":
			kept = []
			api.Registry.new(api.Counter.new("c", 0)).edit(lambda counter: kept.append(counter) or False)
			kept[0].get()
		# A thread holds the lent counter while converting an argument, which waits, the GIL
		# released, until the lend has ended; the call would then reach a freed handle.
		case "lent held":
			converting, ended = threading.Event(), threading.Event()

			class Slow:
				def __index__(self):
					converting.set()
					ended.wait()
					return 1

			def edit(counter):
				edit.thread = threading.Thread(target=counter.add, args=(Slow(),))
				edit.thread.start()
				converting.wait()
				return False

			api.Registry.new(api.Counter.new("c", 0)).edit(edit)
			ended.set()
			edit.thread.join()
		# An IPv6 address where Rust takes `SocketAddrV4`.
		case "family":
			api.nexts(("::", 1, 0, 0), ("::", 1, 0, 0))
		# A future Rust polls, cancelled by its owner rather than by Rust.
		case "foreign cancelled":

			async def cancel():
				gate = asyncio.get_running_loop().create_future()
				task = asyncio.ensure_future(api.after(gate, 1))
				await asyncio.sleep(0)
				gate.cancel()
				await task

			asyncio.run(cancel())
		case "callback threw":

			class Threw(api.Log):
				def log(self, level, line):
					raise RuntimeError("a log failed")

			api.emit(Threw(), bytes([1]))
		# A kept closure raising what is not its error, on Rust's own thread.
		case "kept threw":
			api.Spawned.new(lambda: 1 // 0).join()
		# A closure returning what Rust does not take.
		case "returned":
			api.made(1, lambda n: n)  # type: ignore[arg-type, return-value]
		# A reader keeping a slice of Rust's buffer, which would write it once it is gone.
		case "reader kept":

			class Keeps(io.BytesIO):
				def readinto(self, buffer, /):
					self.kept = buffer[1:]
					return super().readinto(buffer)

			api.tail(Keeps(b"abc"), 1)
		case _:
			raise ValueError(f"no case {which}")


match sys.argv[1:]:
	case []:
		script()
	case ["futures"]:
		futures()
	case ["owned"]:
		owned()
	case ["values"]:
		values()
	case ["calls"]:
		calls()
	case [which]:
		broken(which)
