// The api fixture's Kotlin test: the script fixtures/api/examples/transcript.rs runs
// natively, through the generated Kotlin. tests/fixtures.rs compares the transcripts.
//
// With an argument, it instead checks the futures' semantics, or breaks a contract, which
// must fail.

@file:OptIn(ExperimentalUnsignedTypes::class)

import api.*
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.Executor
import kotlin.concurrent.thread
import kotlin.time.Duration.Companion.milliseconds
import kotlin.time.Duration.Companion.seconds
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout

// What a function marked so holds lives in its own frame: the Android test keeps it from
// R8's inlining, since ART's interpreter keeps every reference a frame's registers last
// held reachable until the frame returns, which would keep it from the GC checks after.
annotation class OwnFrame

// The Log callback: keeps even levels.
class PrintLog : Log, AutoCloseable {
	override fun log(level: UByte, line: String): Boolean {
		println("log $level $line")
		return level.toInt() % 2 == 0
	}

	// Rust closes it once it drops the implementation.
	override fun close() = println("freed")
}

// Print each of `n`'s lists, its floats as their bits, as transcript.rs does.
fun numbers(what: String, n: Numbers) {
	println("$what u16 ${n.u16s.contentToString()}")
	println("$what u32 ${n.u32s.contentToString()}")
	println("$what u64 ${n.u64s.contentToString()}")
	println("$what i8 ${n.i8s.contentToString()}")
	println("$what i16 ${n.i16s.contentToString()}")
	println("$what i32 ${n.i32s.contentToString()}")
	println("$what i64 ${n.i64s.contentToString()}")
	println("$what f32 ${n.f32s.map { it.toRawBits().toUInt() }}")
	println("$what f64 ${n.f64s.map { it.toRawBits().toULong() }}")
	println("$what some ${n.some?.let { "Some(${it.contentToString()})" } ?: "None"}")
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
	val (only) = only(Counter.new("o", 3u))
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
		val found = find(Bytes("hello".encodeToByteArray()), Bytes(needle.encodeToByteArray()))
		println(if (found == null) "find none" else "find ${found.first} ${found.second}")
	}
	val statRows = rows(listOf(Point(1, 2), Point(3, 3)))
	println("rows ${statRows.joinToString(", ") { (name, shape, p) -> "$name ${shape.debug()} ${p.x} ${p.y}" }}")
	// Each handle in a tuple moves in, as one taken by value does.
	Board.new(listOf(Pair("low", Counter.new("l", 1u)), Pair("high", Counter.new("h", 9u)))).use { board ->
		val ranked = board.ranked()
		println("ranked ${ranked.joinToString(", ") { (name, counter) -> "$name ${counter.name()} ${counter.get()}" }}")
		ranked.forEach { it.second.close() }
		board.best().use { println("best ${it.name()} ${it.get()}") }
	}
	// A map crosses as its pairs and a set as its items, which Rust collects in order, each
	// key once, the last kept.
	val atlas = Atlas(listOf(Pair("b", Point(1, 1)), Pair("ab", Point(2, 3)), Pair("b", Point(4, 4))), listOf(3u, 1u, 3u))
		.with("c", Point(5, 6))
	val atlasPoints = atlas.points.joinToString(", ") { (name, p) -> "$name ${p.x} ${p.y}" }
	println("atlas $atlasPoints; ids ${atlas.ids.joinToString(" ")}")
	// Another crate's error crosses as its message.
	for (text in listOf("80", "x")) {
		try {
			println("port ${port(text)}")
		} catch (e: StringException) {
			println("port err ${e.error}")
		}
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
	val lentOne = lentBack(Tuple1(Bytes(byteArrayOf(1, 2)))).first.toByteArray().toList()
	val boxedOne = boxedTuple(Bytes(byteArrayOf(3))).first.toByteArray().toList()
	println("lent back $lentOne boxed $boxedOne")
	// A borrowed string and an owned one share a tuple struct.
	println("title Some((\"${title(true)!!.first}\",)) ${if (title(false) == null) "None" else "Some"} named Some((\"${named("x")!!.first}\",))")

	// Each lent counter is a new handle to a clone.
	val registry = Registry.new(Counter.new("first", 1u))
	println("last ${registry.last() != null}")
	registry.push(Counter.new("last", 2u))
	val first = registry.firstGet
	first.add(10u)
	println("first ${registry.firstGet.get()} lent ${first.get()}")
	val last = registry.last()!!
	last.add(10u)
	println("last ${registry.last()!!.get()} lent ${last.get()}")
	// A closure lent for the call edits the first counter through a handle.
	val by = 5uL
	val edited = registry.edit { counter ->
		counter.add(by)
		counter.edited() && counter.get() == 6uL
	}
	println("edit $edited ${registry.firstGet.get()}")
	// A setter moves an object in, as assignment does, closing its handle.
	registry.firstSet(Counter.new("second", 3u))
	println("set first ${registry.firstGet.get()} ${registry.firstGet.name()}")
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
	val lit = Name.borrowed("lit")
	println("borrowed ${lit.asStr()} ${lit.shares(Name.borrowed("lit"))} ${lit.shares(Name.borrowed("other"))}")

	println("total ${total(RangeU64(2u, 5u))}")
	println("reach ${reach(2u, 5u)}")
	val two = java.math.BigInteger.valueOf(2)
	val scaled = listOf(scaled(two.pow(128) - java.math.BigInteger.ONE, 1u), scaled(two.pow(64) + java.math.BigInteger.ONE, 1uL shl 63), scaled(two.pow(127), 2u))
	println("scaled " + scaled.joinToString(" ") { it?.toString(16)?.padStart(32, '0') ?: "none" })
	for (v in listOf(two.pow(128), java.math.BigInteger.ONE.negate())) {
		check(runCatching { scaled(v, 1u) }.exceptionOrNull() is IllegalArgumentException, "$v is no u128")
	}
	Digest.new().use { digest ->
		digest.sumSet(fingerprint("deadbeef"))
		val unparsed = try {
			fingerprint("zz").toString()
		} catch (e: StringException) {
			e.error
		}
		println("digest ${digest.sumGet} ${digest.flipped(ArrayU8x4(byteArrayOf(-1, 0, -1, 0)))} $unparsed")
		check(runCatching { ArrayU8x4(ByteArray(3)) }.exceptionOrNull() is IllegalArgumentException, "3 bytes are no [u8; 4]")
	}
	Batch.new().use { batch ->
		pair().use { batch2 ->
			val accepted = (1u..5u).count { batch.push(it) }
			val paired = (1u..3u).count { batch2.push(it) }
			val emptied = drain4(batch)
			println("batch ${batch.capacity()} $accepted $emptied ${drain4(batch)} ${batch2.capacity()} $paired ${drain2(batch2)}")
			println("capacities ${capacities42(batch, batch2)} ${capacities22(batch2, batch2)}")
		}
	}
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
	println("len ${len(Bytes("abc".encodeToByteArray()))}")
	val binary = Bytes(byteArrayOf(0xff.toByte(), 0, 'a'.code.toByte()))
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
	println("whole ${stamp.whole().debug()}")
	val message = Message.new(stamp, Bytes("body".encodeToByteArray()))
	println("payload ${message.payloadGet.text()}")
	println("stamp ${message.stampGet!!.debug()}")
	println("sent ${message.sent()}")
	// Setters assign in place: the bytes are copied in, and a `Copy` handle copied.
	val reset = Stamp.fromMillis(4000u)
	message.stampSet(reset)
	message.payloadSet(Bytes("edited".encodeToByteArray()))
	println("payload ${message.payloadGet.text()}")
	println("stamp ${message.stampGet!!.debug()}")
	println("secs ${reset.secs()}")
	message.stampSet(null)
	println("stamp ${message.stampGet != null}")

	// A value past a word, copied across: a closure takes and returns one by value.
	val moment = Moment.new(1500u, 1000u)!!
	println("moment ${moment.millis()} ${Moment.new(1u, 0u) == null}")
	val shifted = shifted(moment) { m -> m.after(500u) }
	println("shifted ${shifted.millis()} ${shifted == moment.after(500u)}")
	println("wrapped ${moment.after(ULong.MAX_VALUE).millis()} ${moment == shifted}")

	// A padded value crosses with its padding zeroed, and one whose bytes vary is a handle.
	val gap = Gap.new(7u, 3u)
	println("gap ${gap.sum()} ${gap == Gap.new(7u, 3u)}")
	val berth = Berth.new(4u)
	println("berth Some(${berth.at()}) ${if (Berth.new(null).at() == null) "None" else "Some"}")

	// A `Copy` type changed in place is a value, which a `&mut self` method returns as the
	// call left it, Rust's own return beside it: a change reaches only the copy it replaces.
	var score = Score.of(2u)
	val step = Score.of(3u)
	score = score.add(step)
	score = score.add(step)
	println("score ${score.count()} ${step.doubled().count()} ${step.count()}")
	var scoreCopy = score
	val (taken, taken0) = scoreCopy.take()
	scoreCopy = taken
	println("taken $taken0 ${scoreCopy.count()} ${score.count()}")
	val otherScore = Mut(step)
	val (absorbed, before0, before1) = scoreCopy.absorb(otherScore)
	scoreCopy = absorbed
	println("absorbed $before0 $before1 ${scoreCopy.count()} ${otherScore.value.count()} ${step.count()}")
	var hits = Hits.of(1u, 2u)
	val keptHits = hits
	hits = hits.add(Hits.of(10u, 20u))
	hits = hits.hitsSet(7u)
	println("hits ${hits.hitsGet} ${hits.missesGet} ${keptHits.hitsGet} ${keptHits.missesGet} ${hits == keptHits}")
	val (counted, countedSame) = keptHits.eq(keptHits)
	println("counted $countedSame ${counted.hitsGet}")
	var extent = Extent.new(5u, 3u)
	val extentStart = extent
	val (slid, old) = extent.slide(10u)
	extent = slid
	extent = extent.startSet(extent.startGet + 1u)
	println("extent $old ${extent.startGet} ${extent.end()} ${extentStart.startGet} ${extentStart.end()}")
	val weighted = Weighted(step, 4u)
	println("weighted ${weighted.total()} ${weighted.score.count() + step.count()}")

	println("kept ${emit(PrintLog(), Bytes(byteArrayOf(1, 2, 3, 4)))}")
	Logger.new(PrintLog()).use { logger -> println("write ${logger.write("hi")}") }
	println("dropped")
	// Rust calls it on another thread, before the call returns.
	val threshold = 2
	val kept = filter(Bytes(byteArrayOf(1, 2, 3, 4))) { level, line ->
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
	channel.send(Bytes("hi".encodeToByteArray()))
	wakes += q.size
	println("woken $wakes")
	q.drain()
	println("recv ${got!!.text()}")
	runBlocking {
		channel.send(Bytes(byteArrayOf()))
		try {
			println("nonempty ${channel.recvNonempty().size}")
		} catch (e: ErrorException) {
			println("nonempty err ${e.message}")
		}
		Batch.new().use { b4 ->
			pair().use { b2 -> println("fill " + (fill4(b4, 9u) + fill2(b2, 1u)).joinToString(" ")) }
		}
		// The send happens at the call: the future is dropped unawaited.
		channel.sendLater(Bytes("eager".encodeToByteArray()))
		println("recv ${channel.recv().text()}")
		// The send happens at the call, and the future borrows the channel and the tag.
		println("tagged ${channel.tagged("tag")}")
		println("itemized ${channel.itemized(uintArrayOf(1u, 2u, 3u))}")
		println("byte_len ${byteLen(Bytes("abc".encodeToByteArray()))}")
		channel.send(Bytes("sized".encodeToByteArray()))
		val (payload, size) = channel.recvSized()
		println("sized ${payload.text()} $size")
	}
	// The waker moves into Rust, which keeps it until the send wakes it.
	channel.notify(waker)
	channel.send(Bytes("rung".encodeToByteArray()))
	println("woken $wakes")
	runBlocking {
		// A poll with no async twin is a future.
		println("take ${channel.pollTake().text()}")
		channel.send(Bytes("measured".encodeToByteArray()))
		println("measure ${channel.pollMeasure(2u)}")
	}
	// A context is made of the waker, which Rust keeps until the send wakes it.
	channel.listen(waker)
	channel.send(Bytes("heard".encodeToByteArray()))
	println("woken $wakes")
	// A returned waker wakes the waiter's task through Rust until the GC drops it.
	Waiter.new(waker).use { waiter ->
		waiter.waker().wake()
		waiter.context().wake()
	}
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
	val sized: PairSizeU8 = tag(Size(1u, 2u), 3u)
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
	val fromSize = Counter.seededSize("s", Size(5u, 6u))
	val fromCounter = Counter.seededCounter("c", fromPoint)
	val fromPolar = Counter.seededPolarPoint("q", PolarPoint(1u, 2u))
	println(
		"seeded ${fromPoint.get()} ${fromSize.get()} ${fromCounter.get()} ${fromPolar.get()} " +
			"past ${stack.depthPastPoint(Point(1, 2))} seeds ${seedsPointSize(Size(2u, 3u))}",
	)

	// A trait implemented for `Point` and `&Point` apart: one instance.
	println("weigh ${weighPoint(Point(-1, 2))}")
	// A function named as the type it returns, and one named as a type it does not.
	println("polar ${polarPoint(2u)!!.distance} ${polarPoint(0u) == null} area ${area(Size(2u, 3u))}")
	// Two types whose paths join alike, the second kept apart where its path splits.
	println("wind ${wind(PolarTurn(2u), Polar_Turn(3u))}")
	// Names that clash in some language, each kept apart there.
	val clashSpan = ClashSpan(4u)
	ClashStore.new(7u).use { clashStore ->
		val clashPair = Pair(Pair(1.toUByte(), 2.toUShort()), 3u)
		val clashTriple = Tuple1(Triple(1.toUByte(), 2.toUShort(), 3u))
		println(
			"clash ${ClashTone(440u).debug()} ${ClashTone_(3u).debug()} ${Cancelled(2u).debug()} " +
				"span ${clashSpan.width()} ${clashSpan.toString_()} $clashSpan " +
				"store ${clashStore.free()} ${clashStore.close_()} " +
				"audio ${clashAudioEncoderFinish()} ${clashAudioEncoderFinish_()} nested ${clashNested(clashPair, clashTriple)}"
		)
	}
	// Instances that name each other, and one made only by an impl of it.
	println("cross ${cross(LeftRightU8(RightU8(1u)), RightLeftU8(LeftU8(2u)))} outer ${OuterInnerU8.new(4u).get()}")
	// A bound that outlives the call takes a value, and an instance only a constant names.
	println("kept ${keptPoint(Point(1, -2))} boxed ${BOXED.value}")
	println("keep ${keepPoint(Point(1, -2))}")
	val five = Point(5, 2)
	println("kept or ${keptOrPoint(five)} ${keptOrPoint(null)} keeper ${keeperPoint(five)}")
	// Inferred instances taking a lent closure and a foreign implementation, and one only
	// its impl of a trait names.
	val sides = PairU32(2u, 5u)
	println("apply ${applyU32(sides) { side -> side * 3u }}")
	println("report ${reportU32(sides, PrintLog())}")
	println("wrapped ${keptWrapU32(WrapU32(4u))}")
	println("keeps ${keeps(PrintLog()).value}")

	// Its Drop runs when Kotlin closes the handle, not before.
	val token = Token.new(7u)
	println("token ${token.idGet} dropped ${dropped()}")
	token.close()
	println("dropped ${dropped()}")

	// A field and the method of its name, each its own member.
	val retry = Retry.new(5u)
	val limit = retry.limitGet
	println("retry ${if (limit != null) 1 else 0} ${limit ?: 0u} ${retry.limit()} ${retry.delay()}")
	// The renamed field's setter, named as its getter.
	retry.limitSet(null)
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
	println("ports ${ports(listOf(v4, v6)).contentToString()}")
	val peer = Peer(v6, listOf("a", "b")).next("c")
	println("peer ${peer.debug()} ${peer.aliases().joinToString(", ", "[", "]") { "\"$it\"" }}")
	val file = file("dir", "f\\'\"\t")
	println("file ${escaped(file)}")
	println("parents ${parents(listOf(file, "/")).joinToString(" ") { it?.let(::escaped) ?: "none" }}")
	val words = words(" one two  three ")
	println("words ${words.size} ${words.joinToString(", ", "[", "]") { "\"$it\"" }}")
	println("sentence ${sentence(words)} ${sentence(null)}")
	println("sentence ${sentence(emptyList())}|")
	println("either ${either("abc")} ${either(null)}")
	val cd = Bytes("cd".encodeToByteArray())
	println("prefixed ${prefixed(cd, Bytes("ab".encodeToByteArray())).toByteArray().decodeToString()} ${prefixed(cd, null).toByteArray().decodeToString()}")
	println("coordinates ${coordinates(Point(2, 3))} ${coordinates(null)}")
	Counter.new("two", 2u).use { two ->
		Counter.new("three", 3u).use { three ->
			println("plus ${two.plus(three)} ${two.plus(null)}")
			println("add_to ${two.addTo(three)} ${two.addTo(null)} ${three.get()}")
			check(two.offset(3u, 4u) == 36uL, "a method's parameters named handle and h are its own")
		}
	}
	check(Counter.new("five", 5u).intoSum(1u) == 6uL, "a consuming method's parameter named close is its own")
	val encoded = Mut(Bytes("0,".encodeToByteArray()))
	putNumber(1u, encoded)
	putNumber(23u, encoded)
	println("put_number ${encoded.value.toByteArray().decodeToString()}")
	val entry = Mut(Pair(Bytes("ab".encodeToByteArray()), 1u))
	val stamps = pushCounted(entry, 'c'.code.toUByte())
	println("push_counted $stamps ${entry.value.first.toByteArray().decodeToString()} ${entry.value.second}")
	val digits = Mut(Bytes("7x9".encodeToByteArray()))
	val firstDigit = digit(digits)
	val skipped = try {
		digit(digits)
		false
	} catch (e: ErrorException) {
		true
	}
	println("digit $firstDigit $skipped ${digit(digits)} ${digits.value.size}")
	val nudged = Mut(Point(1, 2))
	nudge(nudged, 3)
	println("nudge ${nudged.value.x} ${nudged.value.y}")
	secure("h3").use { config -> println("secure ${alpnLent(config)} ${alpn(config.clone())}") }
	val cert = certify(Bytes("der".encodeToByteArray()))
	val certLen = certLen(cert)
	println("cert $certLen ${certDer(cert).text()} ${chainLen(listOf(certify(Bytes("a".encodeToByteArray())), certify(Bytes("bc".encodeToByteArray()))))}")
	resume("h3", 60u).use { session -> println("resume ${session.alpn()} ${session.lifetime().seconds}") }
	try {
		resume("", 60u)
	} catch (e: StringException) {
		println("resume err ${e.error}")
	}
	Meters.new().use { meters ->
		meters.first().use { first ->
			first.add(2uL)
			println("lent ${first.add(3uL)} ${meters.first().use { it.count() }}")
		}
		meters.clear()
		println("cleared ${meters.first().use { it.count() }}")
		meters.lastGet.use { last ->
			last.add(4uL)
			println("field lent ${last.add(1uL)}")
		}
	}
	Tallies.new().use { tallies ->
		tallies.first().use { it.bump() }
		println("tallied ${tallies.first().use { it.bump() }}")
	}
	println("chunks ${chunks(Bytes("abcde".encodeToByteArray()), 2uL).map { it.toByteArray().toList() }}")
	val counters = counters(listOf("x", "y"))
	println("counters ${counters.joinToString(", ") { "${it.name()} ${it.get()}" }}")
	// Each counter moves in, so Kotlin closes it.
	println("worth ${worth(listOf(counters[0]), listOf(counters[1]))} ${worth(null, emptyList())}")
	for (counter in counters) {
		check(runCatching { counter.get() }.exceptionOrNull() is IllegalStateException, "a counter moved in is closed")
	}
	println("held ${listOf(heldCount(Tuple1(Counter.new("h", 4u))), heldCount(null)).joinToString(" ") { if (it == null) "None" else "Some($it)" }}")
	println("boxed ${boxedName(boxedCounter("b"))?.let { "Some(\"$it\")" }} ${boxedName(null) ?: "None"} ${boxedCounter(null) == null}")
	println("less ${less(5u, 2u)}")
	println("sums ${sums(listOf(uintArrayOf(1u, 2u), UIntArray(0), uintArrayOf(3u))).contentToString()}")
	println("least ${least(listOf(listOf(3u, 1u), listOf(), listOf(2u))).joinToString(" ")}")
	check(splitLast(intArrayOf(1, 2, -3)) == splitLast(intArrayOf(1, 2, -3)), "a tuple holding an array compares by contents")
	check(splitLast(intArrayOf(1, 2, -3)).hashCode() == splitLast(intArrayOf(1, 2, -3)).hashCode(), "a tuple holding an array hashes by contents")
	check(Shadowed(intArrayOf(1), 0u, 0u) == Shadowed(intArrayOf(1), 0u, 0u) && Shadowed(intArrayOf(1), 0u, 0u) != Shadowed(intArrayOf(2), 0u, 0u), "a field named other compares as itself")
	check(Builtin.Any(intArrayOf(1)) == Builtin.Any(intArrayOf(1)) && Builtin.Any(intArrayOf(1)).hashCode() == Builtin.Any(intArrayOf(1)).hashCode(), "a variant named Any compares by contents")
	check(Builtin.Native(Double.NaN) != Builtin.Native(Double.NaN) && Builtin.Int(-0.0) == Builtin.Int(0.0) && Builtin.Boolean(1.0) != Builtin.Int(1.0), "variants named as Kotlin's builtins compare as Rust's")
	check(Qualifiers(-0.0, 1.0) == Qualifiers(0.0, 1.0) && Qualifiers(-0.0, 1.0).hashCode() == Qualifiers(0.0, 1.0).hashCode() && Qualifiers(Double.NaN, 1.0) != Qualifiers(Double.NaN, 1.0), "fields named as the package and kotlin compare as Rust's")
	numbers("edges", Numbers.edges())
	numbers("reversed", Numbers.edges().reversed())
	val edges = Numbers.edges()
	val nan = Measure.new(Double.NaN, false)
	println("float equal ${edges == edges} ${edges == Numbers.edges()} ${Numbers.zero(true) == Numbers.zero(false)} ${nan == nan} ${Measure.new(-0.0, true) == Measure.new(0.0, false)}")
	check(Numbers.zero(true).hashCode() == Numbers.zero(false).hashCode(), "a negative zero hashes as zero does")
	check(Measure.new(-0.0, true).hashCode() == Measure.new(0.0, false).hashCode(), "a negative zero hashes as zero does in a nested record")
	val ints = edges.copy(f32s = FloatArray(0), f64s = DoubleArray(0))
	val alike = Numbers.edges().copy(f32s = FloatArray(0), f64s = DoubleArray(0))
	check(ints == alike && ints.hashCode() == alike.hashCode(), "records of arrays compare by contents")
	val back = edges.reversed().reversed()
	check(back.copy(f32s = FloatArray(0), f64s = DoubleArray(0)) == ints, "every integer edge value crosses both ways exactly")
	check(back.f32s.map { it.toRawBits() } == edges.f32s.map { it.toRawBits() } && back.f64s.map { it.toRawBits() } == edges.f64s.map { it.toRawBits() }, "every float edge value crosses both ways exactly")
	check(edges.copy(some = null).reversed().some == null, "a missing array stays missing")
	check(edges.copy(some = IntArray(0)).reversed().some?.size == 0, "an empty array is no missing one")
	check(edges.copy(some = intArrayOf(Int.MIN_VALUE, 7)).reversed().some.contentEquals(intArrayOf(7, Int.MIN_VALUE)), "an array in an option crosses in a record")
	println("sum u32 ${sumU32(UIntArray(0))} ${sumU32(uintArrayOf(UInt.MAX_VALUE, UInt.MAX_VALUE, 2u))}")
	println("doubled i64 ${doubledI64(longArrayOf(Long.MIN_VALUE, -1, 0, Long.MAX_VALUE)).contentToString()}")
	val floats = boxedFloats(doubleArrayOf(-0.0, Double.fromBits(0x7ff8000000000001L)))
	println("boxed floats ${floats.map { it.toRawBits().toULong() }}")
	val summed = summed(listOf(Point(1, 2), Point(-4, 6)))
	val noneSummed = summed(emptyList())
	println("summed ${summed.x} ${summed.y} ${noneSummed.x} ${noneSummed.y}")
	println("longest ${longest(listOf("a", "abc"))} ${longest(emptyList())}")
	println("first of Some(\"${firstOf(listOf("a", "b"))}\") ${firstOf(emptyList()) ?: "None"}")
	val limits = byteLimits(Bytes("a".encodeToByteArray()), Bytes("bc".encodeToByteArray()))
	val waits = breadth(LimitsDuration(5.milliseconds, 2.seconds))
	println("limits ${limits.low.toByteArray().decodeToString()} ${limits.high.toByteArray().decodeToString()} breadth ${waits.inWholeMilliseconds}")
	val parts = joined(listOf(Bytes("ab".encodeToByteArray()), Bytes(ByteArray(0)), Bytes("c".encodeToByteArray()))).toByteArray().decodeToString()
	println("joined $parts ${joined(emptyList()).toByteArray().size} read ${readLen(Bytes("hello".encodeToByteArray()))}")
	val doc = Json.parse("""{"b": [1, 2.5, null], "a": "\u00e9"}""")
	println("json ${encode(doc)}")
	println("timed text ${timedText(doc)}")
	tagged(doc, "t").use { tagged -> println("tagged ${tagged.toText()}") }
	// A JSON object's entries cross as pairs of a key and its document, each moved in.
	val jsonEntries = entries(listOf(Pair("b", Json.parse("[1]")), Pair("a", Json.parse("2"))), "t")
	println("entries ${jsonEntries.joinToString(", ") { (key, value) -> "$key ${value.toText()}" }}")
	jsonEntries.forEach { it.second.close() }
	try {
		Json.parse("{\"a\":").close()
	} catch (e: StringException) {
		println("json parse err ${e.error}")
	}
	val site = Site.new(base)
	site.homeGet.use { a -> site.homeGet.use { b -> println("site $a $b") } }

	// Slices, passed in and returned as clones, and the socket address of each family.
	println("shapes ${Shape.ALL.joinToString(", ", "[", "]") { it.debug() }} greetings ${GREETINGS.joinToString(", ", "[", "]") { "\"$it\"" }}")
	Trail.new(listOf(Point(1, 2), Point(3, 4)), listOf("a", "b")).use { trail ->
		println("trail ${trail.points().joinToString(", ", "[", "]") { it.debug() }} ${trail.labels().joinToString(", ", "[", "]") { it.debug() }}")
		// Returned iterators, each collected at the call: drained labels are gone after.
		println("iterated ${trail.iter().joinToString(", ", "[", "]") { it.debug() }} ${trail.names().joinToString(", ", "[", "]") { "\"$it\"" }} ${trail.bytes().toByteArray().joinToString(", ", "[", "]") { it.toUByte().toString() }}")
		println("drained ${trail.drain().joinToString(", ", "[", "]") { it.debug() }} then ${trail.labels().joinToString(", ", "[", "]") { it.debug() }}")
		// A filter, collected at the call.
		println("beyond ${trail.beyond(1).joinToString(", ") { "${it.x} ${it.y}" }}")
	}
	val numbered = numbered(listOf("p", "q"))
	println("numbered ${numbered.joinToString(", ") { "${it.name()} ${it.get()}" }}")
	numbered.forEach { it.close() }
	val at4 = java.net.InetSocketAddress(java.net.InetAddress.getByAddress(byteArrayOf(127, 0, 0, 1)), 8080)
	val at6 = java.net.InetSocketAddress(java.net.Inet6Address.getByAddress(null, loopback, 3), 443)
	val (next4, next6) = nexts(at4, at6)
	println("nexts ${next4.address.hostAddress} ${next4.port} ${next6.port} ${(next6.address as java.net.Inet6Address).scopeId}")

	// serde_json's map iterator, collected as pairs of a key and its document.
	val jsonSections = sections(listOf(Pair("b", Json.parse("[1]")), Pair("a", Json.parse("2"))))
	println("sections ${jsonSections.joinToString(", ") { (key, value) -> "$key ${value.toText()}" }}")
	jsonSections.forEach { it.second.close() }
	// An instant only Rust makes, which each call copies, and compares.
	val began = now()
	val ended = later(began, 1500.milliseconds)
	val same = later(began, kotlin.time.Duration.ZERO)
	println("instants ${since(ended, began).inWholeMilliseconds} ${since(began, ended).inWholeMilliseconds} ${began == same} ${began == ended}")
	println("has_instant ${hasInstant(began)} ${hasInstant(null)}")
	// tracing's level, and its filter as an optional level whose null is `OFF`.
	Verbosity.new(Level.WARN).use { verbosity ->
		println("verbosity ${spell(verbosity.filter())} ${spell(louder(verbosity.filter()))}")
		verbosity.levelSet(Level.TRACE)
		println("verbosity ${verbosity.levelGet == Level.TRACE} ${spell(verbosity.filter())} ${spell(louder(verbosity.filter()))} ${spell(louder(null))}")
	}

	println("made ${made(3u) { n -> "$n times" }}")
	// A closure taking and returning `Copy` values copies them, never running their `Clone`.
	val nine = Ticket.new(9u)
	val given = reissue(Ticket.new(7u)) { t -> Ticket.new(t.number() + 1u) }.number()
	val held = reissue(Ticket.new(7u)) { nine }.number()
	println("reissued $given $held, cloned ${Ticket.clones()}")
	// A kept closure, called on another thread after the call that took it returned.
	println("spawned ${Spawned.new { 7u }.join()}")
	// A kept closure: called again at the instant it asks for, then once its waiter wakes.
	now().let { start ->
		var calls = 0
		val stopped = runBlocking {
			drive(start) { now, waiter ->
				calls += 1
				when (calls) {
					1 -> later(now, 5.milliseconds)
					2 -> {
						println("drive ${since(now, start).inWholeMilliseconds}")
						waiter.waker().let { waker ->
							waker.wake()
							(waker as AutoCloseable).close()
						}
						null
					}
					else -> throw StringException("stopped after $calls")
				}
			}
		}
		println("drive $stopped")
	}
	for (file in listOf(Memory("hello world".encodeToByteArray()), Broken())) {
		try {
			val (tail, at) = tail(file, 5u)
			println("tail ${tail.text()} $at")
		} catch (e: StringException) {
			println("tail error ${e.error}")
		}
	}
	// A seek from the start reaches the reader whole, past `Long.MAX_VALUE`, as `Cursor` takes one.
	try {
		println("sought ${seekTo(Far(), ULong.MAX_VALUE)}")
	} catch (e: StringException) {
		println("sought error ${e.error}")
	}
	println("non zeros ${describeNonZeros(nonZeros())}")
	for (hop in listOf(null, 1UL, ULong.MAX_VALUE)) {
		println("hop ${nextHop(hop)?.let { "Some($it)" } ?: "None"}")
	}
	println("farther hop ${fartherHop(PairNonZeroU16(3u, UShort.MAX_VALUE))}")
}

// A channel whose reads fail, which seeks anywhere from its start, as Rust's `Cursor` does.
class Far : Broken(), StartSeekable {
	override fun seekStart(offset: ULong): ULong = offset
}

// A channel over bytes in memory, as Rust's `Cursor` reads them.
class Memory(private val bytes: ByteArray) : java.nio.channels.SeekableByteChannel {
	private var at = 0L

	override fun read(dst: java.nio.ByteBuffer): Int {
		if (at >= bytes.size) return -1
		val n = minOf(dst.remaining().toLong(), bytes.size - at).toInt()
		dst.put(bytes, at.toInt(), n)
		at += n
		return n
	}

	override fun write(src: java.nio.ByteBuffer): Int = throw java.nio.channels.NonWritableChannelException()

	override fun position(): Long = at

	override fun position(newPosition: Long) = apply { at = newPosition }

	override fun size(): Long = bytes.size.toLong()

	override fun truncate(size: Long) = throw java.nio.channels.NonWritableChannelException()

	override fun isOpen() = true

	override fun close() {}
}

// A channel whose reads fail, as the native script's broken reader's do: each seek lands
// at 0.
open class Broken : java.nio.channels.SeekableByteChannel {
	override fun read(dst: java.nio.ByteBuffer): Int = throw java.io.IOException("broken")

	override fun write(src: java.nio.ByteBuffer): Int = throw java.nio.channels.NonWritableChannelException()

	override fun position(): Long = 0

	override fun position(newPosition: Long) = this

	override fun size(): Long = 0

	override fun truncate(size: Long) = throw java.nio.channels.NonWritableChannelException()

	override fun isOpen() = true

	override fun close() {}
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

/** The bytes as text. */
fun Bytes.text(): String = toByteArray().decodeToString()

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
		val sender = thread { channel.send(Bytes("blocked".encodeToByteArray())) }
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
		channel.send(Bytes("then".encodeToByteArray()))
		check(got == null, "recv never polls inside the wake")
		q.drain()
		check(got == "then", "recv resumes with the payload")
	}

	// On a dispatcher that resumes in place, too: the send wakes while it holds the lock the
	// receive's poll takes.
	runBlocking {
		var got: String? = null
		val job = launch(Dispatchers.Unconfined) { got = channel.recv().text() }
		channel.send(Bytes("unconfined".encodeToByteArray()))
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
		channel.send(Bytes("kept".encodeToByteArray()))
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
		sender.send(Bytes("held".encodeToByteArray()))
		q.drain()
		check(got == "held", "a closed handle stays held by the call awaiting on it")
		sender.close()
	}

	// Suspending lambdas Rust polls as futures, each launched in the caller's context.
	run {
		val ran = mutableListOf<String>()
		var got = 0u
		scope.launch { got = after({ ran += "ready" }, 4u) }
		q.drain()
		check(got == 8u && ran == listOf("ready"), "Rust awaits a suspending lambda")
	}
	run {
		val gate = CompletableDeferred<Unit>()
		var got = 0u
		scope.launch { got = after({ gate.await() }, 5u) }
		q.drain()
		check(got == 0u, "Rust waits on a lambda that suspends")
		gate.complete(Unit)
		q.drain()
		check(got == 10u, "a lambda's end wakes Rust")
	}
	run {
		var error: ErrorException? = null
		var ran = false
		scope.launch {
			try {
				within(0u) { ran = true }
			} catch (e: ErrorException) {
				error = e
			}
		}
		q.drain()
		check(error != null && !ran, "Rust gives up on a lambda before it starts, which never runs")
	}
	run {
		var cancelled = false
		val job = scope.launch {
			within(1000u) {
				try {
					awaitCancellation()
				} finally {
					cancelled = true
				}
			}
		}
		q.drain()
		check(!cancelled, "a lambda Rust polls runs until Rust drops it")
		job.cancel()
		q.drain()
		check(cancelled, "cancelling the call cancels the lambda Rust polls")
	}
	run {
		// A lambda that returns once the caller cancelled ends unfinished: the call
		// reports the cancel, never Rust's success.
		var result: String? = null
		scope.launch {
			val caller = coroutineContext[kotlinx.coroutines.Job]!!
			result = try {
				"returned ${after({ caller.cancel() }, 1u)}"
			} catch (e: kotlinx.coroutines.CancellationException) {
				"cancelled"
			}
		}
		q.drain()
		check(result == "cancelled", "a caller's cancel the lambda sees reports the cancel")
	}

	// A lambda a poll borrows, which the future it makes owns: polled until it ends, and
	// cancelled with the call.
	run {
		val gate = CompletableDeferred<Unit>()
		var done = false
		scope.launch {
			Waiter.pollFuture { gate.await() }
			done = true
		}
		q.drain()
		check(!done, "a poll of a lambda that suspends waits")
		gate.complete(Unit)
		q.drain()
		check(done, "a borrowed lambda's end finishes the poll")
	}
	run {
		var cancelled = false
		val job = scope.launch {
			Waiter.pollFuture {
				try {
					awaitCancellation()
				} finally {
					cancelled = true
				}
			}
		}
		q.drain()
		check(!cancelled, "a lambda a poll borrows runs until Rust drops it")
		job.cancel()
		q.drain()
		check(cancelled, "cancelling the poll cancels the lambda it borrows")
	}

	// Rust lets go of a closure it keeps once it drops it, done or not, so the GC collects it.
	released(spawned(9u), "a spawned closure is released once it ran")
	run {
		val (job, weak) = idle(scope)
		q.drain()
		check(held(weak), "a closure a pending future keeps stays held")
		job.cancel()
		q.drain()
		released(weak, "a closure a cancelled future kept is released")
	}
	println("futures ok")
}

// A closure Spawned keeps, which returns [n], once Spawned ran it.
@OwnFrame
fun spawned(n: UInt): java.lang.ref.WeakReference<Any> {
	val task = { n }
	check(Spawned.new(task).join() == n, "a kept closure runs on Rust's thread")
	return java.lang.ref.WeakReference(task)
}

// A drive whose closure waits for a wake that never comes, launched on [scope].
@OwnFrame
fun idle(scope: CoroutineScope): Pair<kotlinx.coroutines.Job, java.lang.ref.WeakReference<Any>> {
	val start = now()
	val poll = { _: Instant, _: Waiter -> start.let { null } }
	val job = scope.launch { drive(start, poll) }
	return Pair(job, java.lang.ref.WeakReference(poll))
}

// Whether [weak] still reaches its value, which never reaches the caller's frame.
@OwnFrame
fun held(weak: java.lang.ref.WeakReference<*>): Boolean = weak.get() != null

// Collect until [weak] is gone, as once Rust lets go of the closure it held.
fun released(weak: java.lang.ref.WeakReference<*>, what: String) {
	val deadline = System.nanoTime() + 10.seconds.inWholeNanoseconds
	while (held(weak)) {
		check(System.nanoTime() < deadline, what)
		Runtime.getRuntime().gc()
		Thread.sleep(1)
	}
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
		"outlived" -> Meters.new().let { meters ->
			val first = meters.first()
			meters.clear()
			first.count()
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
		// Any exception but a closure's own error, and a reader's IOException, aborts.
		"closure threw" -> made(1u) { throw IllegalStateException("a make failed") }
		"closure failed" -> now().let { start ->
			runBlocking { drive(start) { _, _ -> throw IllegalStateException("a poll failed") } }
		}
		"reader threw" -> tail(
			object : Broken() {
				override fun read(dst: java.nio.ByteBuffer): Int = throw IllegalStateException("a read failed")
			},
			1u,
		)
		"callback threw" -> emit(
			object : Log {
				override fun log(level: UByte, line: String): Boolean = throw IllegalStateException("a log failed")
			},
			Bytes(byteArrayOf(1)),
		)
	}
}

// What frees what: an object's close, or the GC once it is collected if never closed, and
// Rust's bytes only the GC, once no Bytes reaching them is reachable. Rust's bytes up to
// 1 KiB cross copied, freed at the call, and longer ones lent.
// Kotlin's text and bytes, which Rust copies or encodes once from where Kotlin keeps them:
// each reads back as Kotlin's own encoder has it, at a call's top level and in a record.
fun encoded() {
	val texts = listOf(
		"", "a\u0000b", "é\u4e2d\uD83D\uDE00", "\uD800x", "x\uDC00", "end\uD83D", "\uDE00\uD83D",
		"a".repeat(1023) + "\uD83D\uDE00", "a".repeat(1023) + "\uD83Dz", "\u4e2d\uD83D\uDE00".repeat(1500),
	)
	for (text in texts) {
		val expected = text.encodeToByteArray().decodeToString()
		check(greet(text, Shape.SQUARE) == "hello $expected, a Square", "text crosses as its UTF-8")
		check(tickName(Label(text, null, null)) == expected, "a record's text crosses as its UTF-8")
	}
	val body = ByteArray(300) { it.toByte() }
	val sliced = nextPacket(Packet(1u, Bytes(body).slice(7, 290))).payload
	check(sliced.toByteArray().contentEquals(body.copyOfRange(7, 290)), "a slice of Kotlin's bytes in a record crosses")
}

/** A record's scalars cross apart from its other fields, each exactly, both ways. */
fun readings() {
	check(reading(0u) == null && readingDebug(0u) == "", "no reading crosses as none")
	for (n in listOf(1u, 2u, 3u, 127u)) {
		val r = reading(n)!!
		check(r.debug() == readingDebug(n), "reading $n crosses as Rust holds it: ${r.debug()}")
		check(r.count == ULong.MAX_VALUE - n && r.len == ULong.MAX_VALUE - n && r.tag == (UShort.MAX_VALUE - n.toUShort()).toUShort(), "reading $n's unsigned scalars")
		check(r.at.x == Int.MIN_VALUE + n.toInt() && r.offset == (Byte.MIN_VALUE + n.toInt()).toByte(), "reading $n's signed scalars")
		check(r.level == -n.toFloat() / 2 && r.ratio == Double.MAX_VALUE / n.toDouble() && r.mark == 0x10FFFF - n.toInt(), "reading $n's floats and char")
		check(r.ok == (n % 2u == 1u) && r.shape == (if (n % 2u == 1u) Shape.CIRCLE else Shape.SQUARE) && r.note == "reading $n", "reading $n's bool, enum, and text")
		check(readingEcho(r) == r, "reading $n crosses back")
		check(r.next() == reading(n + 1u), "reading $n's receiver crosses")
	}
}

/** A call near the JVM's 255 parameter slots, which a record's flattening must not pass. */
fun crowding() {
	val r = reading(1u)!!
	val sum = crowded(r, Stamp.fromMillis(2uL), Stamp.fromMillis(3uL), 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL, 1uL)
	check(sum == r.count + 2uL + 3uL + 119uL, "a crowded call crosses every parameter: $sum")
}

fun owned() {
	encoded()
	readings()
	crowding()
	budgeted()
	for (n in 1023..1025) {
		val payload = ByteArray(n) { it.toByte() }
		val packet = nextPacket(Packet(1u, Bytes(payload)))
		check(packet.seq == 2u && packet.payload.toByteArray().contentEquals(payload), "a record's $n bytes cross both ways")
		val sliced = nextPacket(Packet(1u, packet.payload.slice(1, n - 1))).payload
		check(sliced.toByteArray().contentEquals(payload.copyOfRange(1, n - 1)), "a slice of a record's $n bytes crosses as its range")
	}
	collected(lentAround() + 1u, "the GC frees lent bytes nothing reaches")
	reads(Bytes(ByteArray(1025) { 9 }), 9, "Kotlin's bytes")

	// A call after close throws, and close frees at once, once.
	val before = dropped()
	val token = Token.new(1u)
	token.close()
	check(dropped() == before + 1u, "closing frees at once")
	token.close()
	check(dropped() == before + 1u, "closing again frees nothing")
	check(runCatching { token.idGet }.exceptionOrNull()?.message == "Token is closed", "a call after close throws")

	// The file serves wasm32 too: what only wasm32 has throws on the host's library.
	check(pointerBits(platform()) == 64u && pointerBits(Platform.Native(64u)) == 64u, "the host is native")
	check(isFamily(Family.NATIVE), "and of the native family")
	for ((what, call) in listOf<Pair<String, () -> Unit>>(
		"api_wasm_only" to { wasmOnly() },
		"api_Platform.Wasm" to { pointerBits(Platform.Wasm(32u)) },
		"api_Family.Wasm" to { isFamily(Family.WASM) },
	)) {
		val thrown = runCatching(call).exceptionOrNull()
		check(thrown is UnsupportedOperationException && thrown.message == "$what: it is not on this library's target", "$what throws: $thrown")
	}
	Counter.new("k", 7u).use { kept ->
		check(runCatching { wasmTake(kept) }.exceptionOrNull() is UnsupportedOperationException, "api_wasm_take throws")
		check(kept.get() == 7uL, "a refused call leaves its moved argument open")
	}

	// Bytes outlive the object that returned them and the record that held them.
	val body = ByteArray(2048) { 'b'.code.toByte() }
	val message = Message.new(Stamp.fromMillis(1u), Bytes(body))
	val payload = message.payloadGet
	message.close()
	val carried = nextPacket(Packet(1u, Bytes(body))).payload
	uncollected(before + 1u, "the GC frees nothing still reachable")
	check(payload.toByteArray().contentEquals(body), "bytes outlive the object that returned them")
	check(carried.toByteArray().contentEquals(body), "bytes outlive the record that held them")

	// Never closed, each is freed once collected, but a slice keeps its bytes.
	unclosed()
	collected(before + 3u, "the GC frees what nobody closed")
	keptBySlice(before + 3u)
	collected(before + 4u, "the GC frees bytes once no slice reaches them")

	raced()
	compared()
	forwarded()
}

// A payload Rust lent passed back to Rust: it crosses as Rust's own, never copied, so the
// Bytes it went through keep nothing of their own, and its one buffer drops once, after
// the last of them.
fun forwarded() {
	val before = dropped()
	forwardedRead(before)
	collected(before + 1u, "a forwarded payload's buffer drops with the last Bytes reaching it")
}

// In its own frame, so nothing on the caller's keeps the last Bytes reachable.
@OwnFrame
fun forwardedRead(before: UInt) {
	val last = forward()
	uncollected(before, "a payload forwarded through Rust stays the buffer Rust made")
	check(last.size == 2046 && last.toByteArray().all { it == 6.toByte() }, "a forwarded payload reads as Rust made it")
}

// Rust's bytes sent, received, sliced, carried in a record, and sent again.
fun forward(): Bytes = runBlocking {
	Channel.new().use { channel ->
		channel.send(tokenBytes(6u, 2048u))
		val carried = nextPacket(Packet(1u, channel.recv().slice(1, 2047))).payload
		channel.send(carried)
		channel.recv()
	}
}

// Every read of [bytes], each of which is [value].
fun reads(bytes: Bytes, value: Byte, what: String) {
	val n = bytes.size
	val all = bytes.toByteArray()
	check(all.size == n && all.all { it == value }, "$what read back")
	check(bytes[0] == value && bytes[n - 1] == value, "$what read by index")
	val into = bytes.copyInto(ByteArray(n + 1) { 7 }, 1, 1, n)
	check(into[0] == 7.toByte() && (1 until n).all { into[it] == value } && into[n] == 7.toByte(), "$what copied into a range")
	val slice = bytes.slice(2, n - 1).slice(1)
	check(slice.size == n - 4 && slice.toByteArray().all { it == value } && slice[n - 5] == value, "$what sliced twice")
	check(bytes.slice(n).size == 0 && bytes.slice(0, 0).toByteArray().isEmpty(), "$what sliced empty")
	val outOfRange = listOf<() -> Any>(
		{ bytes[n] },
		{ bytes[-1] },
		{ slice[slice.size] },
		{ bytes.slice(2, 1) },
		{ bytes.slice(0, n + 1) },
		{ slice.slice(0, slice.size + 1) },
		{ bytes.copyInto(ByteArray(n - 1)) },
		{ bytes.copyInto(ByteArray(n), 1) },
		{ bytes.copyInto(ByteArray(n), -1, 0, 1) },
	)
	for (read in outOfRange) {
		check(runCatching(read).exceptionOrNull() is IndexOutOfBoundsException, "$what refuse a read out of range")
	}
}

// Never closed, and long enough to be lent.
@OwnFrame
fun unclosed() {
	Token.new(3u)
	tokenBytes(4u, 1025u)
}

// A slice of Rust's bytes, made in a frame of its own so nothing keeps the Bytes it came
// from, which keeps Rust's buffer while it reads.
@OwnFrame
fun keptBySlice(freed: UInt) {
	val view = sliced(5u)
	uncollected(freed, "a slice keeps Rust's bytes")
	check(view.size == 10 && view.toByteArray().all { it == 5.toByte() }, "a slice reads Rust's bytes once the Bytes it came from are gone")
}

@OwnFrame
fun sliced(id: UInt): Bytes = tokenBytes(id, 1025u).slice(1015)

// Frees racing calls: a close racing calls on other threads, and the GC freeing what a
// call reads while it runs.
fun raced() {
	// Each call either runs, holding the handle so Rust's value outlives it, or throws.
	var before = dropped()
	val token = Token.new(5u)
	val ran = java.util.concurrent.atomic.AtomicInteger()
	val failed = java.util.concurrent.atomic.AtomicInteger()
	val callers = List(8) {
		thread {
			while (true) {
				val id = try {
					token.idGet
				} catch (e: IllegalStateException) {
					check(e.message == "Token is closed", "a call racing close throws")
					failed.incrementAndGet()
					return@thread
				}
				check(id == 5u, "a call racing close reads the live value")
				ran.incrementAndGet()
			}
		}
	}
	while (ran.get() < 100) Thread.yield()
	token.close()
	callers.forEach { it.join() }
	check(failed.get() == 8 && dropped() == before + 1u, "close racing calls drops Rust's value once")

	// Calls on values nothing else reaches, while another thread collects: each reads what
	// it was given, never a buffer freed under it and reused by the next, which holds
	// another id.
	before = dropped()
	val collecting = java.util.concurrent.atomic.AtomicBoolean(true)
	val cycles = java.util.concurrent.atomic.AtomicInteger()
	val collector = thread {
		while (collecting.get()) {
			Runtime.getRuntime().gc()
			cycles.incrementAndGet()
		}
	}
	var rounds = 0u
	while (cycles.get() < 50) {
		racedRound(rounds % 100u + 1u)
		rounds++
	}
	collecting.set(false)
	collector.join()
	collected(before + 3u * rounds, "the GC frees what each call raced")
}

// Calls on values nothing else reaches once it returns, holding [id], racing the GC.
@OwnFrame
fun racedRound(id: UInt) {
	check(tokenBytes(id, 2048u)[2047] == id.toByte(), "a read by index races the GC")
	check(tokenBytes(id, 2048u).slice(1024).toByteArray().all { it == id.toByte() }, "a slice's copy races the GC")
	check(Token.new(id).idGet == id, "a call on an object races the GC")
}

// At most 64 MiB of Rust's bytes are lent at once, freed or not, since the GC sees none of
// them: past that, bytes cross copied, freed at the call, until the GC frees what was lent.
// First, so no bytes an earlier step lent count against it.
fun budgeted() {
	val before = dropped()
	lentToBudget(before)
	collected(before + 65u, "the GC frees bytes lent up to the budget")
	lentAgain(before + 65u)
	collected(before + 66u, "the GC frees bytes lent once the budget is free")
}

// Bytes lent once the GC freed those that filled the budget, with Rust's drops still [freed].
@OwnFrame
fun lentAgain(freed: UInt) {
	check(tokenBytes(3u, 2048u)[2047] == 3.toByte() && dropped() == freed, "bytes cross lent again once the GC frees them")
}

// Rust's bytes either side of the copy limit, returning the drops once the call has freed
// those it copied.
@OwnFrame
fun lentAround(): UInt {
	var freed = dropped()
	for (n in 1023..1025) {
		val bytes = tokenBytes(n.toUInt(), n.toUInt())
		if (n <= 1024) freed++
		check(dropped() == freed, "$n bytes are freed at the call only if copied")
		reads(bytes, n.toByte(), "$n bytes")
	}
	return freed
}

@OwnFrame
fun lentToBudget(before: UInt) {
	val mib = 1 shl 20
	val held = List(64) { tokenBytes(1u, mib.toUInt()) }
	check(dropped() == before, "64 MiB of Rust's bytes cross lent")
	val copied = tokenBytes(2u, mib.toUInt())
	check(dropped() == before + 1u, "bytes past 64 MiB lent cross copied, freed at the call")
	check(copied.size == mib && copied[0] == 2.toByte() && copied[mib - 1] == 2.toByte() && held[63][mib - 1] == 1.toByte(), "bytes past the budget read as lent ones do")
	val carried = nextPacket(Packet(1u, Bytes(ByteArray(2048) { 4 }))).payload
	check(carried.size == 2048 && carried.toByteArray().all { it == 4.toByte() }, "a record's bytes past the budget cross copied")
}

// Bytes compare and hash by their contents wherever each keeps them, Kotlin's or Rust's,
// copied or lent, whole or sliced, and so do records holding them, as map keys too.
fun compared() {
	val body = ByteArray(2050) { (it * 7).toByte() }
	// Rust's whole are lent, its 1000 copied; the middle two differ by a shift.
	val ranges = listOf(0 to 2050, 1 to 1001, 2 to 1002, 5 to 5)
	val forms = ranges.map { (from, until) ->
		listOf(
			Bytes(body.copyOfRange(from, until)),
			Bytes(body).slice(from, until),
			nextPacket(Packet(1u, Bytes(body.copyOfRange(from, until)))).payload,
			nextPacket(Packet(1u, Bytes(body))).payload.slice(from, until),
		)
	}
	val keys = HashMap<Packet, Int>()
	forms.forEachIndexed { i, same -> keys[Packet(1u, same[0])] = i }
	val all = forms.flatMapIndexed { i, same -> same.map { i to it } }
	for ((i, a) in all) {
		for ((j, b) in all) {
			check((a == b) == (i == j) && (Packet(1u, a) == Packet(1u, b)) == (i == j), "bytes ${ranges[i]} and ${ranges[j]} compare by content")
			if (i == j) check(a.hashCode() == b.hashCode() && Packet(1u, a).hashCode() == Packet(1u, b).hashCode(), "bytes ${ranges[i]} hash by content")
		}
		check(keys[Packet(1u, a)] == i, "a record holding bytes ${ranges[i]} finds its key")
	}
	check(all.map { it.second }.toSet().size == ranges.size, "a set holds bytes once per content")
	val last = Bytes(body.copyOf().also { it[body.size - 1]++ })
	check(last != forms[0][3] && forms[0][3] != last, "bytes differing in their last byte differ")
	check(!forms[0][3].equals(body), "bytes differ from a ByteArray")
}

// Collect until Rust's values dropped count [n], and none past it.
fun collected(n: UInt, what: String) {
	val deadline = System.nanoTime() + 10.seconds.inWholeNanoseconds
	while (dropped() < n) {
		check(System.nanoTime() < deadline, what)
		// Not System.gc(), which ART skips unless finalization ran since its last call.
		Runtime.getRuntime().gc()
		Thread.sleep(1)
	}
	uncollected(n, "$what, each once")
}

// Collect a few times, checking that Rust's values dropped still count [n].
fun uncollected(n: UInt, what: String) {
	repeat(5) {
		Runtime.getRuntime().gc()
		Thread.sleep(1)
	}
	check(dropped() == n, what)
}

/** An enum whose variants are named as types its sealed class names, each round-tripped. */
fun clash() {
	val p = Point(1, 2)
	val all = listOf(
		Clash.Point(p),
		Clash.Shape(Shape.CIRCLE),
		Clash.Event(Event.Named("n")),
		Clash.Double(1.5),
		Clash.Float(0.5f),
		Clash.String("s"),
		Clash.Int(-3),
		Clash.Boolean(true),
		Clash.Native(7u),
		Clash.Bytes(Bytes("ab".encodeToByteArray())),
		Clash.Any,
		Clash.Unit,
		Clash.Waker,
		Clash.Mut,
		Clash.Pair(1u, 2u),
		Clash.Stamp(5L),
		Clash.Moment(Moment.new(3u, 1000u)!!),
	)
	for (c in all) {
		check(c.debug().isNotEmpty(), "clash $c debug")
	}
	check(Clash.Double(1.5).debug() == "Double(1.5)", "clash Double debug")
	check(Clash.String("s").debug() == "String(\"s\")", "clash String debug")
	check(Clash.Native(7u).debug() == "Native(7)", "clash Native debug")
	check(all[0].point() == p && all[3].point() == null, "clash point")
	check(Clash.of(2.5) == Clash.Double(2.5), "clash of")
	check(Clash.at(p) == Clash.Point(p), "clash at")
	check(Clash.Unit.reading(3u) == readingEcho(reading(3u)!!), "clash reading")
	val len = Mut(9u)
	check(Clash.listed(uintArrayOf(1u, 2u), len).map { it.toList() } == listOf(listOf(1u, 2u, 9u)) && len.value == 2u, "clash listed")
	check(runBlocking { Clash.after({}, 0.25) } == Clash.Double(0.25), "clash after")
	check(Packaged(3u).point() == Point(3, 3) && Packaged(3u).shape() == Shape.CIRCLE, "a field named as the package")
	val b = { t: String -> Bytes(t.encodeToByteArray()) }
	check(Clash.Unit.joined(b("a"), b("b"), b("c")).toByteArray().decodeToString() == "abc", "clash joined")
	check(Clash.Unit.joined(b("a"), b("b"), null).toByteArray().decodeToString() == "ab", "clash joined none")
	check(runBlocking { Clash.Int(1).tagged("t") } == "t Int(1)", "clash tagged")
	check(Clash.Unit.stamp(2500u).secs() == 2uL && Clash.Unit.stamp(5u) == Stamp.fromMillis(5u), "clash stamp")
	check(all[16].moment(9u) == Moment.new(3u, 1000u) && Clash.Unit.moment(4000u).millis() == 4000uL, "clash moment")
	check(Clash.String("hi").raw().toByteArray().decodeToString() == "hi", "clash bytes")
	val out = Mut(0u)
	check(Clash.Int(4).describe("t", 3u, Shape.SQUARE, out) == "t Int(4) 3 Square" && out.value == 6u, "clash describe")
	check(Handle.new(5u).use { it.count() } == 5u && runBlocking { Handle.later(6u) }.use { it.count() } == 6u, "an object named as a former runtime class")
}

fun returns() {
	val saved = returnValues(Mut(Point(0, 0)), false)
	val completed = java.util.concurrent.atomic.AtomicInteger()
	List(4) { thread {
		repeat(64) { returnValues(Mut(Point(it, -it)), false); Counter.new("thread", it.toULong()).close(); completed.incrementAndGet() }
	} }.forEach { it.join() }
	check(completed.get() == 256, "all concurrent return calls completed")
	check(saved.first == "hé🙂\u0000" && saved.third[1][9999].toUByte() == 15u.toUByte(), "a return owns its data after concurrent scratch reuse")
	val point = Mut(Point(3, -5))
	for (fail in listOf(false, true, false)) {
		try {
			val (text, widths, rows) = returnValues(point, fail)
			val (a, b, c, d) = widths!!
			println("values $text $a $b $c $d")
			println("rows ${rows[0].size} ${rows[0][0].toUByte()} ${rows[0][1].toUByte()} ${rows[0][2].toUByte()} ${rows[1].size} ${rows[1][9999].toUByte()}")
		} catch (e: ErrorException) { println("values error") }
		println("point ${point.value.x} ${point.value.y}")
	}
	for (fail in listOf(false, true)) {
		val counter = Counter.new("moved", ULong.MAX_VALUE)
		try {
			returnMoved(counter, fail).use { println("moved ${it.name()} ${it.get()}") }
		} catch (e: ErrorException) { println("moved error") }
		val usable = try { counter.get(); true } catch (_: IllegalStateException) { false }
		check(!usable) { "a moved counter was usable" }
	}
	for (hex in listOf("ff0080fe", "bad")) {
		try {
			val value = fingerprint(hex)
			println("array ${value[0].toUByte()} ${value[1].toUByte()} ${value[2].toUByte()} ${value[3].toUByte()}")
		} catch (e: StringException) { println("array error") }
	}
	for (unit in listOf(true, false)) {
		try { val (a, b) = corner(unit); println("corner $a $b") }
		catch (e: ErrorException) { println("corner error") }
	}
}

fun main(args: Array<String>) {
	when {
		args.isEmpty() -> script()
		args[0] == "returns" -> returns()
		args[0] == "futures" -> futures()
		args[0] == "owned" -> owned()
		args[0] == "clash" -> clash()
		else -> broken(args[0])
	}
}
