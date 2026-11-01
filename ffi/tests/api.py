# The api fixture's Python test: the script fixtures/api/examples/transcript.rs runs
# natively, through the generated Python. tests/fixtures.rs compares the transcripts.
#
# With an argument, it instead checks the futures' semantics, what frees what nobody
# closed, or the values only Python's types bound, or breaks a contract, which must fail.

import asyncio
import datetime
import gc
import os
import sys
import threading

import api


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
	with c.clone() as copy:
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
	with api.Board.new([("low", api.Counter.new("l", 1)), ("high", api.Counter.new("h", 9))]) as board:
		ranked = board.ranked()
		print("ranked " + ", ".join(f"{name} {counter.name()} {counter.get()}" for name, counter in ranked))
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
	first = registry.first
	first.add(10)
	print(f"first {registry.first.get()} lent {first.get()}")
	last = registry.last()
	last.add(10)
	print(f"last {registry.last().get()} lent {last.get()}")
	# A closure lent for the call edits the first counter through a handle.
	by = 5

	def edit(counter):
		counter.add(by)
		return counter.edited() and counter.get() == 6

	print(f"edit {flag(registry.edit(edit))} {registry.first.get()}")
	# A setter moves an object in, as assignment does, closing it.
	registry.set_first(api.Counter.new("second", 3))
	print(f"set first {registry.first.get()} {registry.first.name()}")
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

	print(f"total {api.total(range(2, 5))}")
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
	message = api.Message.new(stamp, b"body")
	print(f"payload {text(message.payload)}")
	print(f"stamp {message.stamp!r}")
	print(f"sent {flag(message.sent())}")
	# Setters assign in place: the bytes are copied in, and a `Copy` handle copied.
	reset = api.Stamp.from_millis(4000)
	message.set_stamp(reset)
	message.set_payload(b"edited")
	print(f"payload {text(message.payload)}")
	print(f"stamp {message.stamp!r}")
	print(f"secs {reset.secs()}")
	message.set_stamp(None)
	print(f"stamp {flag(message.stamp is not None)}")

	print(f"kept {api.emit(PrintLog(), bytes([1, 2, 3, 4]))}")
	with api.Logger.new(PrintLog()) as logger:
		print(f"write {flag(logger.write('hi'))}")
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
		# The send happens at the call: the future is dropped unawaited.
		channel.send_later(b"eager")
		print(f"recv {text(await channel.recv())}")
		channel.send(b"sized")
		payload, size = await channel.recv_sized()
		print(f"sized {text(payload)} {size}")

	loop.run_until_complete(block())
	# The waker moves into Rust, which keeps it until the send wakes it.
	channel.notify(waker)
	channel.send(b"rung")
	until(loop, lambda: wakes == 2)
	print(f"woken {wakes}")
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
	from_size = api.Counter.from_size("s", api.Size(5, 6))
	from_counter = api.Counter.seeded_counter("c", from_point)
	from_polar = api.Counter.seeded_polar_point("q", api.PolarPoint(1, 2))
	print(
		f"seeded {from_point.get()} {from_size.get()} {from_counter.get()} {from_polar.get()} "
		f"past {stack.depth_past_point(api.Point(1, 2))} seeds {api.seeds_point_size(api.Size(2, 3))}"
	)

	# A trait implemented for `Point` and `&Point` apart: one instance.
	print(f"weigh {api.weigh_point(api.Point(-1, 2))}")
	# Instances that name each other, and one made only by an impl of it.
	cross = api.cross(api.LeftRightU8(api.RightU8(1)), api.RightLeftU8(api.LeftU8(2)))
	print(f"cross {cross} outer {api.OuterInnerU8.new(4).get()}")
	# A bound that outlives the call takes a value, and an instance only a constant names.
	print(f"kept {api.kept_point(api.Point(1, -2))} boxed {api.BOXED.value}")
	print(f"keep {api.keep_point(api.Point(1, -2))}")
	# Inferred instances taking a lent closure and a foreign implementation, and one only
	# its impl of a trait names.
	sides = api.PairU32(2, 5)
	print(f"apply {api.apply_u32(sides, lambda side: side * 3)}")
	print(f"report {api.report_u32(sides, PrintLog())}")
	print(f"wrapped {api.kept_wrap_u32(api.WrapU32(4))}")
	print(f"keeps {api.keeps(PrintLog()).value}")

	# Its Drop runs when Python closes it, not before.
	token = api.Token.new(7)
	print(f"token {token.id} dropped {api.dropped()}")
	token.close()
	print(f"dropped {api.dropped()}")

	# A field and the method of its name, each its own member.
	retry = api.Retry.new(5)
	limit = retry.limit_setting
	print(f"retry {0 if limit is None else 1} {limit or 0} {retry.limit()} {retry.delay()}")
	# The renamed field's setter, named as its getter.
	retry.set_limit_setting(None)
	print(f"retry {retry.limit()}")
	print(f"slot {api.SlotU8.new(200).value()}")

	# Foreign types: a URL parsed from text, socket addresses, OS paths, and `Vec`s.
	base = api.Url.parse("https://example.com/a/b")
	host = api.host(base)
	host = "None" if host is None else f'Some("{host}")'
	print(f"url {base} host {host}")
	with api.resolve(base.clone(), "c?d") as url:
		print(f"resolve {url} {flag(url == base)}")
	try:
		api.Url.parse("no scheme").close()
	except api.StringException as e:
		print(f"parse err {e.error}")
	v4 = ("127.0.0.1", 8080)
	v6 = ("::1", 443, 0, 3)
	print(f"ports {api.ports([v4, v6])}")
	peer = api.Peer(v6, ["a", "b"]).next("c")
	print(f"peer {peer!r} {quoted(peer.aliases())}")
	file = api.file("dir", "f\\'\"\t")
	print(f"file {escaped(file)}")
	print("parents " + " ".join("none" if p is None else escaped(p) for p in api.parents([file, "/"])))
	words = api.words(" one two  three ")
	print(f"words {len(words)} {quoted(words)}")
	print(f"sentence {api.sentence(words)} {api.sentence(None)}")
	print(f"sentence {api.sentence([])}|")
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
	print(f"sums {api.sums([[1, 2], [], [3]])}")
	summed = api.summed([api.Point(1, 2), api.Point(-4, 6)])
	none_summed = api.summed([])
	print(f"summed {summed.x} {summed.y} {none_summed.x} {none_summed.y}")
	print(f"longest {api.longest(['a', 'abc'])} {api.longest([])}")
	doc = api.Json.parse('{"b": [1, 2.5, null], "a": "é"}')
	print(f"json {api.encode(doc)}")
	print(f"timed text {api.timed_text(doc)}")
	with api.tagged(doc, "t") as tagged:
		print(f"tagged {tagged.to_text()}")
	try:
		api.Json.parse('{"a":').close()
	except api.StringException as e:
		print(f"json parse err {e.error}")
	site = api.Site.new(base)
	with site.home as a, site.home as b:
		print(f"site {a} {b}")

	# Slices, passed in and returned as clones, and the socket address of each family.
	print(f"shapes {debugs(api.Shape.ALL)} greetings {quoted(api.GREETINGS)}")
	with api.Trail.new([api.Point(1, 2), api.Point(3, 4)], ["a", "b"]) as trail:
		print(f"trail {debugs(trail.points())} {debugs(trail.labels())}")
		# Returned iterators, each collected at the call: drained labels are gone after.
		print(f"iterated {debugs(trail.iter())} {quoted(trail.names())} {list(data(trail.bytes()))}")
		print(f"drained {debugs(trail.drain())} then {debugs(trail.labels())}")
	numbered = api.numbered(["p", "q"])
	print("numbered " + ", ".join(f"{c.name()} {c.get()}" for c in numbered))
	next4, next6 = api.nexts(v4, v6)
	print(f"nexts {next4[0]} {next4[1]} {next6[1]} {next6[3]}")


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

	# A handle closed while a call awaits on it is freed once the call ends.
	other = api.Channel.new()
	got = None

	async def held():
		nonlocal got
		got = text(await other.recv())

	task = loop.create_task(held())
	step(loop)
	sender = other.clone()
	other.close()
	sender.send(b"held")
	loop.run_until_complete(asyncio.wait_for(task, 5))
	check(got == "held", "a closed object stays held by the call awaiting on it")
	sender.close()

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
	loop.close()
	print("futures ok")


def owned():
	"""What frees an object or bytes nobody closed: its last reference going, or the
	cycle collector, and only what closing did not free already. Rust's bytes cross lent,
	until released."""
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
	api.Token.new(1).close()
	api.token_bytes(2, 1025).release()
	check(api.dropped() == before + 2, "closing frees at once")
	unclosed()
	check(api.dropped() == before + 4, "the last reference going frees what nobody closed")
	cycle = [api.Token.new(5), api.token_bytes(6, 1)]
	cycle.append(cycle)
	del cycle
	gc.collect()
	check(api.dropped() == before + 6, "the cycle collector frees what a cycle held")
	print("owned ok")


def unclosed():
	api.Token.new(3)
	api.token_bytes(4, 1025)


def values():
	"""The values Python's types bound: integers checked against their Rust type's range,
	durations truncated to the microsecond up to `timedelta.max`, which stands for longer,
	and on Unix, a path whose bytes are not UTF-8, which crosses as `os.fsdecode` decodes it,
	and back as it came."""

	def raises(error, f, what):
		try:
			f()
			check(False, what)
		except error:
			pass

	raises(OverflowError, lambda: api.widths(2**64), "an int past u64 raises")
	raises(OverflowError, lambda: api.widths(-1), "a negative int for a u64 raises")
	raises(TypeError, lambda: api.title(1), "an int is no bool")

	td = datetime.timedelta
	check(api.doubled_ttl(td(seconds=1, microseconds=5)) == td(seconds=2, microseconds=10), "a duration crosses")
	check(api.doubled_ttl(td(days=600_000_000)) == td.max, "a duration past timedelta's is its max")
	check(api.doubled_ttl(td.max) == td.max, "timedelta's max is Duration::MAX")
	raises(ValueError, lambda: api.doubled_ttl(td(seconds=-1)), "a negative duration raises")

	if os.name == "posix":
		path = api.stray()
		check(os.fsencode(path) == b"\xff", "a path that is not UTF-8 keeps its bytes")
		check(api.parents([path, os.fsdecode(b"\xfe/\xff")])[1] == os.fsdecode(b"\xfe"), "and crosses back")
	print("values ok")


def broken(which):
	"""Each breaks a contract, which must fail with a message naming it."""
	match which:
		case "moved":
			c = api.Counter.new("c", 0)
			c.into_label(None)
			c.get()
		case "boxed moved":
			tail = api.Node.new(1, None)
			api.Node.new(2, tail)
			tail.sum()
		case "lent moved":
			api.Registry.new(api.Counter.new("c", 0)).edit(lambda counter: counter.into_label(None) and False)
		case "lent kept":
			kept = []
			api.Registry.new(api.Counter.new("c", 0)).edit(lambda counter: kept.append(counter) or False)
			kept[0].get()
		# An IPv6 address where Rust takes `SocketAddrV4`.
		case "family":
			api.nexts(("::", 1, 0, 0), ("::", 1, 0, 0))
		case "callback threw":

			class Threw(api.Log):
				def log(self, level, line):
					raise RuntimeError("a log failed")

			api.emit(Threw(), bytes([1]))
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
	case [which]:
		broken(which)
