// The api fixture's Kotlin test: the script fixtures/api/examples/transcript.rs runs
// natively, through the generated Kotlin. tests/fixtures.rs compares the transcripts.
//
// With an argument, it instead checks the futures' semantics, or breaks a contract, which
// must fail.

import api.*
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.Executor
import kotlin.concurrent.thread
import kotlin.time.Duration.Companion.seconds
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout

// The Log callback: keeps even levels.
class PrintLog : Log, AutoCloseable {
	override fun log(level: UByte, line: String): Boolean {
		println("log $level $line")
		return level.toInt() % 2 == 0
	}

	// Rust closes it once it drops the implementation.
	override fun close() = println("freed")
}

fun bound(b: BoundU64): String = when (b.kind) {
	Bound.INCLUDED -> "Included(${b.value})"
	Bound.EXCLUDED -> "Excluded(${b.value})"
	Bound.UNBOUNDED -> "Unbounded"
}

fun script() {
	println("limit $LIMIT")

	val p = Point(1, 2).offset(2, 3)
	println("offset ${p.x} ${p.y}")
	println("debug ${p.debug()}")
	println("eq ${p == Point(3, 5)}")
	val d = Point.default()
	println("default ${d.x} ${d.y}")

	val c = Counter.new("c", 5u)
	println("get ${c.get()}")
	println("name ${c.name()}")
	println("add ok ${c.add(10u)}")
	try {
		c.add(2000u)
	} catch (e: ErrorException) {
		println("add err ${e.message} ${e.error.debug()}")
	}
	try {
		c.sub(20u)
	} catch (e: ErrorException) {
		println("sub err ${e.message} ${e.error.debug()}")
	}
	c.rename("d")
	println("name ${c.name()}")
	println("is d ${c.`is`("d")}")
	c.clone().use { copy ->
		copy.add(1u)
		println("clone ${copy.get()} original ${c.get()}")
	}
	println("debug ${c.debug()}")
	val label = c.intoLabel(p)
	println("label ${label.debug()}")
	println("note ${hasText(Note(""))} ${hasText(Note(null))}")

	val (below, above) = Counter.new("s", 7u).split(5u)
	println("split ${below.name()} ${below.get()} ${above.name()} ${above.get()}")
	val only = only(Counter.new("o", 3u))
	println("only ${only.name()} ${only.get()}")
	val (w8, w16, w32, w64) = widths(98305u)
	println("widths $w8 $w16 $w32 $w64")
	for (b in listOf(2uL, 0uL)) {
		try {
			val (q, r) = divide(7u, b)
			println("divide $q $r")
		} catch (e: ErrorException) {
			println("divide err ${e.message}")
		}
	}
	// A tuple inside another type is a Pair, a Triple, or a longer tuple class.
	for (n in listOf(98305uL, 0uL)) {
		val spread = spread(n)
		println(if (spread == null) "spread none" else "spread ${spread.first} ${spread.second} ${spread.third} ${spread.fourth}")
	}
	for (needle in listOf("lo", "x")) {
		val found = find("hello".encodeToByteArray(), needle.encodeToByteArray())
		println(if (found == null) "find none" else "find ${found.first} ${found.second}")
	}
	val statRows = rows(listOf(Point(1, 2), Point(3, 3)))
	println("rows ${statRows.joinToString(", ") { (name, shape, p) -> "$name ${shape.debug()} ${p.x} ${p.y}" }}")
	// Each handle in a tuple moves in, as one taken by value does.
	Board.new(listOf(Pair("low", Counter.new("l", 1u)), Pair("high", Counter.new("h", 9u)))).use { board ->
		val ranked = board.ranked()
		println("ranked ${ranked.joinToString(", ") { (name, counter) -> "$name ${counter.name()} ${counter.get()}" }}")
		ranked.forEach { it.second.close() }
	}
	val span = Span(Pair(2u, 10u), Pair("s", Shape.SQUARE))
	val (lowerHalf, upperHalf) = halves(span)
	println("halves ${lowerHalf.first} ${lowerHalf.second} ${upperHalf.first} ${upperHalf.second} ${span.debug()}")
	val (swappedN, swappedText) = swapped(Pair("t", 4u))
	println("swapped $swappedN $swappedText")
	for (value in listOf(null, Tuple1<UInt?>(null), Tuple1<UInt?>(3u))) {
		val doubled = doubled(value)
		println(if (doubled == null) "doubled None" else "doubled Some((${doubled.first?.let { "Some($it)" } ?: "None"},))")
	}
	// A borrowed or boxed tuple of one is a tuple, not its element.
	val lentOne = lentBack(Tuple1(byteArrayOf(1, 2))).first.use { it.toByteArray().toList() }
	val boxedOne = boxedTuple(byteArrayOf(3)).first.use { it.toByteArray().toList() }
	println("lent back $lentOne boxed $boxedOne")
	// A borrowed string and an owned one share a tuple struct.
	println("title Some((\"${title(true)!!.first}\",)) ${if (title(false) == null) "None" else "Some"} named Some((\"${named("x")!!.first}\",))")

	// Each lent counter is a new handle to a clone.
	val registry = Registry.new(Counter.new("first", 1u))
	println("last ${registry.last() != null}")
	registry.push(Counter.new("last", 2u))
	val first = registry.first
	first.add(10u)
	println("first ${registry.first.get()} lent ${first.get()}")
	val last = registry.last()!!
	last.add(10u)
	println("last ${registry.last()!!.get()} lent ${last.get()}")
	// A closure lent for the call edits the first counter through a handle.
	val by = 5uL
	val edited = registry.edit { counter ->
		counter.add(by)
		counter.edited() && counter.get() == 6uL
	}
	println("edit $edited ${registry.first.get()}")
	// A setter moves an object in, as assignment does, closing its handle.
	registry.setFirst(Counter.new("second", 3u))
	println("set first ${registry.first.get()} ${registry.first.name()}")
	println("walk ${walk(3u) { cursor -> cursor.at() * 2u }}")
	println("each ${each(2u) { at -> println("visit $at") }}")
	println("offset ${offset({ 7u }, 3u)}")
	println("shift ${shift({ 7u }, 4u)}")

	val name = Name.owned("a")
	val joined = name.join(Name.owned("b"))
	println("joined ${joined.asStr()} ${joined.debug()} ${joined == name}")
	joined.rename("c")
	println("renamed ${joined.asStr()}")
	val long = longer(joined, Name.owned("dd"))
	println("longer ${long.asStr()}")
	println("unnamed [${unnamed().asStr()}]")

	println("total ${total(RangeU64(2u, 5u))}")
	val wide = widen(RangeU64(2u, 5u), 1u)
	println("widen ${wide.start} ${wide.end}")
	val counts = listOf(
		BoundsU64(BoundU64(Bound.INCLUDED, 2u), BoundU64(Bound.EXCLUDED, 5u)),
		BoundsU64(BoundU64(Bound.INCLUDED, 2u), BoundU64(Bound.INCLUDED, 5u)),
		BoundsU64(BoundU64(Bound.INCLUDED, 3u), BoundU64(Bound.UNBOUNDED, 0u)),
		BoundsU64(BoundU64(Bound.UNBOUNDED, 0u), BoundU64(Bound.UNBOUNDED, 0u)),
		BoundsU64(BoundU64(Bound.EXCLUDED, 2u), BoundU64(Bound.INCLUDED, 4u)),
	).map { count(it) }
	println("count ${counts.joinToString(" ")}")
	println("end ${bound(end(3u))} ${bound(end(0u))}")
	val (endsStart, endsEnd) = ends(3u)
	println("ends ${bound(endsStart)} ${bound(endsEnd)}")
	for (n in listOf(2uL, 5uL)) {
		try {
			println("below ${below(n, 3u)}")
		} catch (e: BoundsU64Exception) {
			println("below err ${bound(e.error.start)} ${bound(e.error.end)}")
		}
	}
	val window = Window(RangeU64(2u, 8u), end(5u))
	println("window ${window.contains(4u)} ${window.contains(6u)} ${window.debug()}")

	println(greet("x", Shape.SQUARE))
	println("len ${len("abc".encodeToByteArray())}")
	val binary = byteArrayOf(0xff.toByte(), 0, 'a'.code.toByte())
	println("hex ${hex(binary)}")
	println("timed hex ${timedHex(binary)}")
	println("sample hex ${sampleHex(binary)}")
	println("queued hex ${queuedHex(binary)}")
	println("tick ${tickName(Label("tick", null, null))}")
	println("rooted ${rootedName(Label("rooted", null, null))}")
	println("stored ${storedText("stored")}")
	println("layer ${layerName(Label("layer", null, null))}")
	for (event in listOf(Event.Moved(p), Event.Named("n"), Event.Done)) {
		println(describe(event))
	}

	// A `Copy` handle passed by value is copied, so it stays usable.
	val stamp = Stamp.fromMillis(2500u)
	println("secs ${stamp.secs()}")
	println("later ${Stamp.fromMillis(3000u).later(stamp)}")
	println("secs ${stamp.secs()}")
	val message = Message.new(stamp, "body".encodeToByteArray())
	println("payload ${message.payload.text()}")
	println("stamp ${message.stamp!!.debug()}")
	println("sent ${message.sent()}")
	// Setters assign in place: the bytes are copied in, and a `Copy` handle copied.
	val reset = Stamp.fromMillis(4000u)
	message.setStamp(reset)
	message.setPayload("edited".encodeToByteArray())
	println("payload ${message.payload.text()}")
	println("stamp ${message.stamp!!.debug()}")
	println("secs ${reset.secs()}")
	message.setStamp(null)
	println("stamp ${message.stamp != null}")

	println("kept ${emit(PrintLog(), byteArrayOf(1, 2, 3, 4))}")
	Logger.new(PrintLog()).use { logger -> println("write ${logger.write("hi")}") }
	println("dropped")
	// Rust calls it on another thread, before the call returns.
	val threshold = 2
	val kept = filter(byteArrayOf(1, 2, 3, 4)) { level, line ->
		println("filter $level $line")
		level.toInt() > threshold
	}
	println("filtered $kept")

	val channel = Channel.new()
	var wakes = 0
	val waker = Waker { wakes++ }
	// On a dispatcher the script drains, so it sees the receive pend and its wake queue
	// the resume.
	val q = Queue()
	var got: Bytes? = null
	CoroutineScope(q.asCoroutineDispatcher()).launch { got = channel.recv() }
	q.drain()
	println("recv pending ${got == null}")
	channel.send("hi".encodeToByteArray())
	wakes += q.size
	println("woken $wakes")
	q.drain()
	println("recv ${got!!.text()}")
	runBlocking {
		channel.send(byteArrayOf())
		try {
			println("nonempty ${channel.recvNonempty().use { it.buffer.remaining() }}")
		} catch (e: ErrorException) {
			println("nonempty err ${e.message}")
		}
		// The send happens at the call: the future is dropped unawaited.
		channel.sendLater("eager".encodeToByteArray())
		println("recv ${channel.recv().text()}")
		channel.send("sized".encodeToByteArray())
		val (payload, size) = channel.recvSized()
		println("sized ${payload.text()} $size")
	}
	// The waker moves into Rust, which keeps it until the send wakes it.
	channel.notify(waker)
	channel.send("rung".encodeToByteArray())
	println("woken $wakes")

	val list = Node.new(1u, null)
	list.push(2u)
	println("node ${list.sum()}")

	val tally = Tally.new()
	tally.bump()
	println("tally ${tally.bump()} ${tally == tally}")
	println("both ${bumpBoth(tally, tally)}")
	println("peek ${peek(tally) { lent -> lent.bump() }}")

	val area = grow(Area.new(Size(2u, 3u)), 1u)
	val size = area.size()
	println("area ${size.width} ${size.height} ${area.pixels()}")

	// Generic types, each at the instances it infers, named by their arguments.
	val pair = PairU32(2u, 5u)
	val swapped = pair.swap()
	println("pair ${swapped.left} ${swapped.right} sum ${pair.sum()} ${pair.debug()} ${pair == swapped}")
	println("even ${pair.even()} ${PairU32(7u, 7u).even()}")
	// An instance renamed alone, beside another of `Pair` that keeps its name.
	val sized: SizeTag = tag(Size(1u, 2u), 3u)
	val flipped: PairU8Size = sized.swap()
	println("sized ${flipped.left} ${flipped.right.width} ${flipped.debug()}")
	val beside = PairU32(258u, 9u).besideSize(sized.left)
	println("beside ${beside.left} ${beside.debug()}")
	// At its default, named as the type itself.
	val stack = Stack.new()
	for ((i, n) in listOf("a", "b").withIndex()) {
		stack.push(Counter.new(n, i.toULong() + 1u))
	}
	val top = stack.pop()!!.get()
	println("stack ${stack.depth()} Some($top) ${stack.pop() != null} ${stack.pop() == null}")
	// Generic functions and methods, at each implementation of the trait bounding them, by
	// reference, which leaves Kotlin its handle.
	val fromPoint = Counter.seededPoint("p", Point(-3, 4))
	val fromSize = Counter.fromSize("s", Size(5u, 6u))
	val fromCounter = Counter.seededCounter("c", fromPoint)
	val fromPolar = Counter.seededPolarPoint("q", PolarPoint(1u, 2u))
	println(
		"seeded ${fromPoint.get()} ${fromSize.get()} ${fromCounter.get()} ${fromPolar.get()} " +
			"past ${stack.depthPastPoint(Point(1, 2))} seeds ${seedsPointSize(Size(2u, 3u))}",
	)

	// A trait implemented for `Point` and `&Point` apart: one instance.
	println("weigh ${weighPoint(Point(-1, 2))}")
	// Instances that name each other, and one made only by an impl of it.
	println("cross ${cross(LeftRightU8(RightU8(1u)), RightLeftU8(LeftU8(2u)))} outer ${OuterInnerU8.new(4u).get()}")
	// A bound that outlives the call takes a value, and an instance only a constant names.
	println("kept ${keptPoint(Point(1, -2))} boxed ${BOXED.value}")
	println("keep ${keepPoint(Point(1, -2))}")
	// Inferred instances taking a lent closure and a foreign implementation, and one only
	// its impl of a trait names.
	val sides = PairU32(2u, 5u)
	println("apply ${applyU32(sides) { side -> side * 3u }}")
	println("report ${reportU32(sides, PrintLog())}")
	println("wrapped ${keptWrapU32(WrapU32(4u))}")
	println("keeps ${keeps(PrintLog()).value}")

	// Its Drop runs when Kotlin closes the handle, not before.
	val token = Token.new(7u)
	println("token ${token.id} dropped ${dropped()}")
	token.close()
	println("dropped ${dropped()}")

	// A field and the method of its name, each its own member.
	val retry = Retry.new(5u)
	val limit = retry.limitSetting
	println("retry ${if (limit != null) 1 else 0} ${limit ?: 0u} ${retry.limit()} ${retry.delay()}")
	// The renamed field's setter, named as its getter.
	retry.setLimitSetting(null)
	println("retry ${retry.limit()}")
	println("slot ${SlotU8.new(200u).value()}")

	// Foreign types: a URL parsed from text, socket addresses, OS paths, and `Vec`s.
	val base = Url.parse("https://example.com/a/b")
	println("url $base host ${host(base)?.let { "Some(\"$it\")" } ?: "None"}")
	resolve(base.clone(), "c?d").use { url -> println("resolve $url ${url == base}") }
	try {
		Url.parse("no scheme").close()
	} catch (e: StringException) {
		println("parse err ${e.error}")
	}
	val v4 = java.net.InetSocketAddress(java.net.InetAddress.getByAddress(byteArrayOf(127, 0, 0, 1)), 8080)
	val loopback = ByteArray(16).also { it[15] = 1 }
	val v6 = java.net.InetSocketAddress(java.net.Inet6Address.getByAddress(null, loopback, 3), 443)
	println("ports ${ports(listOf(v4, v6))}")
	val peer = Peer(v6, listOf("a", "b")).next("c")
	println("peer ${peer.debug()} ${peer.aliases().joinToString(", ", "[", "]") { "\"$it\"" }}")
	val file = file("dir", "f\\'\"\t")
	println("file ${escaped(file)}")
	println("parents ${parents(listOf(file, "/")).joinToString(" ") { it?.let(::escaped) ?: "none" }}")
	val words = words(" one two  three ")
	println("words ${words.size} ${words.joinToString(", ", "[", "]") { "\"$it\"" }}")
	println("sentence ${sentence(words)} ${sentence(null)}")
	println("sentence ${sentence(emptyList())}|")
	println("chunks ${chunks("abcde".encodeToByteArray(), 2uL).map { it.use { b -> b.toByteArray().toList() } }}")
	val counters = counters(listOf("x", "y"))
	println("counters ${counters.joinToString(", ") { "${it.name()} ${it.get()}" }}")
	// Each counter moves in, so Kotlin closes it.
	println("worth ${worth(listOf(counters[0]), listOf(counters[1]))} ${worth(null, emptyList())}")
	for (counter in counters) {
		check(runCatching { counter.get() }.exceptionOrNull() is IllegalStateException, "a counter moved in is closed")
	}
	println("sums ${sums(listOf(listOf(1u, 2u), emptyList(), listOf(3u)))}")
	val summed = summed(listOf(Point(1, 2), Point(-4, 6)))
	val noneSummed = summed(emptyList())
	println("summed ${summed.x} ${summed.y} ${noneSummed.x} ${noneSummed.y}")
	println("longest ${longest(listOf("a", "abc"))} ${longest(emptyList())}")
	val doc = Json.parse("""{"b": [1, 2.5, null], "a": "\u00e9"}""")
	println("json ${encode(doc)}")
	println("timed text ${timedText(doc)}")
	tagged(doc, "t").use { tagged -> println("tagged ${tagged.toText()}") }
	try {
		Json.parse("{\"a\":").close()
	} catch (e: StringException) {
		println("json parse err ${e.error}")
	}
	val site = Site.new(base)
	site.home.use { a -> site.home.use { b -> println("site $a $b") } }

	// Slices, passed in and returned as clones, and the socket address of each family.
	println("shapes ${Shape.ALL.joinToString(", ", "[", "]") { it.debug() }} greetings ${GREETINGS.joinToString(", ", "[", "]") { "\"$it\"" }}")
	Trail.new(listOf(Point(1, 2), Point(3, 4)), listOf("a", "b")).use { trail ->
		println("trail ${trail.points().joinToString(", ", "[", "]") { it.debug() }} ${trail.labels().joinToString(", ", "[", "]") { it.debug() }}")
		// Returned iterators, each collected at the call: drained labels are gone after.
		println("iterated ${trail.iter().joinToString(", ", "[", "]") { it.debug() }} ${trail.names().joinToString(", ", "[", "]") { "\"$it\"" }} ${trail.bytes().use { it.toByteArray() }.joinToString(", ", "[", "]") { it.toUByte().toString() }}")
		println("drained ${trail.drain().joinToString(", ", "[", "]") { it.debug() }} then ${trail.labels().joinToString(", ", "[", "]") { it.debug() }}")
	}
	val numbered = numbered(listOf("p", "q"))
	println("numbered ${numbered.joinToString(", ") { "${it.name()} ${it.get()}" }}")
	numbered.forEach { it.close() }
	val at4 = java.net.InetSocketAddress(java.net.InetAddress.getByAddress(byteArrayOf(127, 0, 0, 1)), 8080)
	val at6 = java.net.InetSocketAddress(java.net.Inet6Address.getByAddress(null, loopback, 3), 443)
	val (next4, next6) = nexts(at4, at6)
	println("nexts ${next4.address.hostAddress} ${next4.port} ${next6.port} ${(next6.address as java.net.Inet6Address).scopeId}")
}

/** A path's UTF-8 bytes as Rust's `escape_ascii` prints them. */
fun escaped(path: String): String = path.encodeToByteArray().joinToString("") {
	when (val b = it.toInt() and 0xff) {
		'\t'.code -> "\\t"
		'\r'.code -> "\\r"
		'\n'.code -> "\\n"
		'\\'.code, '\''.code, '"'.code -> "\\${b.toChar()}"
		in 0x20 until 0x7f -> b.toChar().toString()
		else -> "\\x%02x".format(b)
	}
}

/** The bytes as text, closing them. */
fun Bytes.text(): String = use { it.toByteArray().decodeToString() }

fun check(ok: Boolean, what: String) {
	if (!ok) {
		System.err.println("failed: $what")
		kotlin.system.exitProcess(1)
	}
}

// A queue of jobs that a test runs when it chooses: the dispatcher coroutines resume on.
class Queue : Executor {
	private val jobs = ConcurrentLinkedQueue<Runnable>()

	override fun execute(job: Runnable) {
		jobs.add(job)
	}

	val size: Int
		get() = jobs.size

	fun drain() {
		while (true) {
			(jobs.poll() ?: return).run()
		}
	}
}

// The futures' semantics beyond the transcript, each checked by an assertion.
fun futures() {
	val channel = Channel.new()

	// The send lands from another thread while the receive waits.
	runBlocking {
		val sender = thread { channel.send("blocked".encodeToByteArray()) }
		val payload = channel.recv()
		sender.join()
		check(payload.text() == "blocked", "recv returns the payload another thread sent")
	}

	val q = Queue()
	val scope = CoroutineScope(q.asCoroutineDispatcher())

	// Resumed on its dispatcher, never inside the wake.
	run {
		var got: String? = null
		scope.launch { got = channel.recv().text() }
		q.drain()
		check(got == null, "recv waits for a send")
		channel.send("then".encodeToByteArray())
		check(got == null, "recv never polls inside the wake")
		q.drain()
		check(got == "then", "recv resumes with the payload")
	}

	// On a dispatcher that resumes in place, too: the send wakes while it holds the lock the
	// receive's poll takes.
	runBlocking {
		var got: String? = null
		val job = launch(Dispatchers.Unconfined) { got = channel.recv().text() }
		channel.send("unconfined".encodeToByteArray())
		job.join()
		check(got == "unconfined", "an unconfined recv resumes outside the wake")
	}

	// A wake that lands while the future polls, before it returns pending, is kept: the
	// future polls again rather than waiting for a wake that already happened.
	run {
		var got = 0u
		scope.launch { got = yielded(7u) }
		q.drain()
		check(got == 7u, "a wake during the poll polls again")
	}

	// Cancelled between its wake and its poll, a receive loses nothing: the payload stays
	// for the next.
	run {
		var called = false
		val job = scope.launch {
			channel.recv()
			called = true
		}
		q.drain()
		channel.send("kept".encodeToByteArray())
		job.cancel()
		q.drain()
		check(!called && job.isCancelled, "a cancelled receive never resumes")
		val payload = runBlocking { withTimeout(5.seconds) { channel.recv() } }
		check(payload.text() == "kept", "a cancelled receive loses nothing")
	}

	// A handle closed while a call awaits on it is freed once the call ends.
	run {
		val other = Channel.new()
		var got: String? = null
		scope.launch { got = other.recv().text() }
		q.drain()
		val sender = other.clone()
		other.close()
		sender.send("held".encodeToByteArray())
		q.drain()
		check(got == "held", "a closed handle stays held by the call awaiting on it")
		sender.close()
	}
	println("futures ok")
}

// Each breaks a contract, which must fail with a message naming it.
fun broken(which: String) {
	when (which) {
		"moved" -> {
			val c = Counter.new("c", 0u)
			c.intoLabel(null)
			c.get()
		}
		"boxed moved" -> {
			val tail = Node.new(1u, null)
			Node.new(2u, tail)
			tail.sum()
		}
		"lent moved" -> Registry.new(Counter.new("c", 0u)).edit { counter ->
			counter.intoLabel(null)
			false
		}
		"lent kept" -> {
			var kept: Counter? = null
			Registry.new(Counter.new("c", 0u)).edit { counter ->
				kept = counter
				false
			}
			kept!!.get()
		}
		// An IPv6 address where Rust takes `SocketAddrV4`.
		"family" -> nexts(
			java.net.InetSocketAddress(java.net.Inet6Address.getByAddress(null, ByteArray(16), null as java.net.NetworkInterface?), 1),
			java.net.InetSocketAddress(java.net.Inet6Address.getByAddress(null, ByteArray(16), null as java.net.NetworkInterface?), 1),
		)
		// No String holds a path that is not UTF-8.
		"stray" -> stray()
		"callback threw" -> emit(
			object : Log {
				override fun log(level: UByte, line: String): Boolean = throw IllegalStateException("a log failed")
			},
			byteArrayOf(1),
		)
	}
}

// What frees an object or bytes nobody closed: the GC, once each is collected, and only
// what closing did not free already. Rust's bytes up to 1 KiB cross copied, freed at the
// call, and longer ones lent until closed or collected.
fun owned() {
	for (n in 1023..1025) {
		val payload = ByteArray(n) { it.toByte() }
		val packet = nextPacket(Packet(1u, Bytes(payload)))
		check(packet.seq == 2u && packet.payload.use { it.toByteArray() }.contentEquals(payload), "a record's $n bytes cross both ways")
	}
	var freed = dropped()
	for (n in 1023u..1025u) {
		val bytes = tokenBytes(n, n)
		if (n <= 1024u) freed++
		check(dropped() == freed, "$n bytes are freed at the call only if copied")
		check(bytes.toByteArray().contentEquals(ByteArray(n.toInt()) { n.toByte() }), "$n bytes read back")
		bytes.close()
		if (n > 1024u) freed++
		check(dropped() == freed, "closing $n bytes frees them")
		bytes.close()
		check(dropped() == freed, "closing $n bytes again frees nothing")
		check(runCatching { bytes.toByteArray() }.isFailure, "closed $n bytes refuse a read")
	}

	val before = dropped()
	Token.new(1u).close()
	tokenBytes(2u, 1025u).close()
	check(dropped() == before + 2u, "closing frees at once")
	unclosed()
	val deadline = System.nanoTime() + 10.seconds.inWholeNanoseconds
	while (dropped() < before + 4u) {
		check(System.nanoTime() < deadline, "the GC frees what nobody closed")
		// Not System.gc(), which ART skips unless finalization ran since its last call.
		Runtime.getRuntime().gc()
		Thread.sleep(1)
	}
	// The closed ones were collected by the same collections, and not freed again.
	check(dropped() == before + 4u, "the GC frees each once")
}

// In its own frame, so nothing on the caller's keeps them reachable. The bytes are long
// enough to be lent.
fun unclosed() {
	Token.new(3u)
	tokenBytes(4u, 1025u)
}

fun main(args: Array<String>) {
	when {
		args.isEmpty() -> script()
		args[0] == "futures" -> futures()
		args[0] == "owned" -> owned()
		else -> broken(args[0])
	}
}
