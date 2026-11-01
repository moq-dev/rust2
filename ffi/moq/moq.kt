// moq.c's script through the generated Kotlin: a broadcast's track, published and
// subscribed to in one process, with every read a suspend fun, and an origin, with
// moq-pattern, kio, moq-tokio, hang, and moq-mux exported into the same library,
// and the catalog lent to Kotlin for a call.
//
// The local reads need no runtime: each read that must wait resumes on a dispatcher the
// script drains, after the write that wakes it. A session over QUIC runs on moq-tokio's
// fallback runtime, whose threads resume Kotlin's.

import java.net.InetAddress
import java.net.InetSocketAddress
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.Executor
import kotlin.time.Duration.Companion.seconds
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import moq.*

var failures = 0

fun expect(ok: Boolean, what: String) {
	if (!ok) {
		System.err.println("failed: $what")
		failures++
	}
}

// The jobs coroutines resume with, run when the script drains them.
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

fun main() {
	val q = Queue()
	val scope = CoroutineScope(q.asCoroutineDispatcher())

	val broadcast = BroadcastInfo.new().produce()
	val track = broadcast.createTrack("video", null)
	expect(track.name() == "video", "a track has its name")

	// A second track of the same name is an error, printed as Rust prints it.
	try {
		broadcast.createTrack("video", null)
		expect(false, "a duplicate track is an error")
	} catch (e: ErrorException) {
		expect(e.message == "duplicate", "a duplicate track's error is Rust's")
	}

	val subscriber = runBlocking { track.consume().subscribe(null) }

	// A group's arrival wakes the subscriber's pending receive.
	var group: GroupConsumer? = null
	scope.launch { group = subscriber.recvGroup() }
	q.drain()
	expect(group == null, "a receive waits for a group")
	val writer = track.appendGroup()
	expect(q.size == 1, "a group's arrival wakes the receive")
	q.drain()
	val first = group!!
	expect(first.index() == 0uL, "the first group is 0")

	// So does a frame's.
	var frame: FrameFrame? = null
	scope.launch { frame = first.readFrame() }
	q.drain()
	expect(frame == null, "a read waits for a frame")
	writer.writeFrame(null, "hello".encodeToByteArray())
	expect(q.size == 1, "a frame's arrival wakes the read")
	q.drain()
	val hello = frame!!
	expect(hello.payload.use { it.toByteArray().decodeToString() } == "hello", "a frame has its payload")
	expect(hello.timestamp == null, "an untimed frame has no timestamp")
	// When it arrived: kio's clock's `Instant`, from kio's own export.
	expect(clockNow().checkedDurationSince(hello.arrived()) != null, "a frame arrived before now")

	// A timestamp is a `Copy` value behind a handle: passing it copies it out. The track is
	// untimed, so a timestamped frame is refused.
	val at = Timestamp.fromMillis(1500u)
	expect(at.asSecs() == 1uL, "1500 ms is 1 s")
	try {
		writer.writeFrame(at, "late".encodeToByteArray())
		expect(false, "a timestamped frame on an untimed track is an error")
	} catch (e: ErrorException) {
		expect(e.message == "frame timestamp doesn't fit its track", "the late frame's error is Rust's")
	}
	expect(at.asSecs() == 1uL, "a copied timestamp stays usable")

	// The last frame, bytes that are not UTF-8, then the group's end, which reads as null.
	val world = byteArrayOf(0xff.toByte()) + "world".encodeToByteArray()
	writer.writeFrame(null, world)
	writer.finish()
	runBlocking {
		expect(first.readFrame()?.payload?.use { it.toByteArray().contentEquals(world) } == true, "the last frame reads")
		expect(first.readFrame() == null, "a finished group reads null")
	}

	// An origin and the driver that runs it, a pair. Its root, which it lends, is returned
	// as a new handle to a clone.
	val (origin, driver) = OriginProducer.new(OriginConfig.default())
	expect(origin.root().asStr() == "", "an origin's root is empty")

	// Scoped beneath a root to patterns, which moq-pattern's own export builds.
	Patterns.new().use { patterns ->
		expect(patterns.insert(Pattern.subtree("alice")), "a pattern inserts")
		origin.scope("room", patterns).use { scoped ->
			expect(scoped.root().asStr() == "room", "a scope has its root")
			expect(scoped.absolute("alice").asStr() == "room/alice", "a scope's paths are absolute")
			val allowed = scoped.allowed()
			expect(allowed.matches("alice/cam"), "a scope allows its patterns")
			expect(!allowed.matches("bob"), "a scope allows nothing else")
			scoped.consume().use { consumer ->
				expect(consumer.absolute("bob").asStr() == "room/bob", "a scope's consumer shares its root")
			}
		}
	}

	// A broadcast the origin creates is stamped with its path, which its info lends. Each
	// handle to it is closed, since an open one keeps it, and so the origin, alive.
	origin.createBroadcast("room/alice").use { publication ->
		val path = publication.broadcast().use { it.info().path }
		expect(path.asStr() == "room/alice", "a broadcast has its path")

		// Paths own their segments, so Kotlin builds them, and relative ones, from a string.
		try {
			Path.new("/cam/")
			expect(false, "a path with an empty segment is an error")
		} catch (e: PathErrorException) {
			expect(e.error == PathError.EMPTY_SEGMENT, "an empty segment is its own error")
		}
		val cam = Path.new("cam")
		expect(cam.asStr() == "cam", "a path builds from its segments")
		val joined = path.join(cam)
		expect(joined.asStr() == "room/alice/cam", "paths join")
		expect(joined.stripPrefix(path) == cam, "a path strips its prefix")
		expect(path.stripPrefix(joined) == null, "a path strips no longer prefix")
		expect(joined.resolve(Relative.new("../bob")).asStr() == "room/bob", "a relative path resolves")
	}

	// The driver, polled as a language's event loop would: at the time, and with a waiter
	// made from the waker and held across polls, since what a poll registers lives only as
	// long as its waiter. It runs until every owner of the origin is gone.
	var wakes = 0
	val waiter = Waiter.new(Waker { wakes++ })
	driver.poll(clockNow(), waiter)
	// A client publishes the origin through moq-net's generic `with_publisher`, at each
	// type that implements its bound, by reference, so Kotlin keeps its handle.
	Client.new().withPublisherProducer(origin).close()
	val before = wakes
	origin.close()
	// The last owner's close wakes the driver, which then finishes, cleanly.
	expect(wakes > before, "the last owner's close wakes the driver")
	try {
		driver.poll(clockNow(), waiter)
		expect(false, "a driver whose origin is gone finishes")
	} catch (e: ErrorException) {
		expect(e.message == "closed", "a finished driver's error is Rust's")
	}

	// moq-mux reads an OpusHead into hang's catalog config: 44.1 kHz in, stereo, which the
	// catalog lists at the decoder's 48 kHz, keeping the head as its description.
	val head = "OpusHead".encodeToByteArray() + byteArrayOf(1, 2, 0x38, 0x01, 0x44, 0xac.toByte(), 0, 0, 0, 0, 0)
	val audio = codecOpusAudioConfig(head)
	expect(audio.sampleRate == 48000u, "Opus decodes at 48 kHz")
	expect(audio.channelCount == 2u, "the head is stereo")
	expect(audio.codec == CatalogAudioCodec.Opus, "the codec is Opus")
	expect(audio.description?.use { it.toByteArray().contentEquals(head) } == true, "the head is the description")
	// A truncated head is moq-mux's error.
	try {
		codecOpusAudioConfig(head.copyOf(8))
		expect(false, "a truncated head is an error")
	} catch (e: MuxErrorException) {
		expect(e.message.isNotEmpty(), "a truncated head's error has a message")
	}

	// A subscription's ranges, from any iterable of them: a list.
	val groups = TrackRange.groups(BoundsU64(BoundU64(Bound.INCLUDED, 2u), BoundU64(Bound.EXCLUDED, 6u)))
	val ranged = TrackSubscription.default().withRanges(listOf(TrackRange.live(), groups)).ranges
	expect(ranged == listOf(TrackRange.live(), groups), "a subscription keeps its ranges")
	expect(groups.start == TrackAnchor.Absolute(2u), "groups start at their first")
	expect(groups.end == TrackAnchor.Absolute(6u), "groups end before their end")

	// A catalog at moq-mux's untyped extension, from the config Kotlin builds, whose generic
	// `is_claimed` is named by the last segment of its argument, and whose `mutate` lends
	// the catalog to Kotlin.
	val publisher = BroadcastInfo.new().produce()
	val extra = CatalogConfig.default().withCatalogExtra(HangCatalogExtra.default())
	val catalog = CatalogProducerExtra.new(publisher, extra)
	expect(!catalog.isClaimedBinaryConfig("state"), "an unclaimed name is not claimed")
	var read = 0
	catalog.mutate { lent ->
		if (lent.toJson().startsWith("{")) read++
	}
	expect(read == 1, "mutate lends the catalog once")

	// A JSON track the catalog lists, published through moq-mux's generic `json_snapshot` at
	// a JSON document, and a section Kotlin parses from text, set in the lent catalog and
	// read back as text.
	val status = catalog.jsonSnapshotConfig(publisher.createTrack("status", null), JsonConfig.default(), JsonOptions.default())
	expect(status.name() == "status", "a JSON snapshot has its track's name")
	expect(catalog.isClaimedJsonConfig("status"), "the JSON snapshot claims its name")
	// A value Kotlin parses is published as the track's latest, which a subscriber reads
	// back as JSON text.
	Json.parse("""{"on": true}""").use { status.update(it) }
	runBlocking {
		val frame = status.consume().recvGroup()?.readFrame()
		expect(frame?.payload?.use { it.toByteArray().decodeToString() } == """{"on":true}""", "a subscriber reads the latest value")
	}
	catalog.mutate { lent ->
		val sections = lent.ext
		sections.set("app", Json.parse("""{"v": [1, true]}"""))
		lent.setExt(sections)
	}
	var app: String? = null
	catalog.mutate { lent ->
		expect(lent.toJson().contains("\"status\""), "the catalog lists the JSON track")
		app = lent.ext.get("app")?.use { it.toText() }
	}
	expect(app == """{"v":[1,true]}""", "the catalog reads its section back as text")
	// Malformed text fails to parse, and a reserved section is moq-mux's error.
	try {
		Json.parse("""{"v":""").close()
		expect(false, "malformed JSON fails to parse")
	} catch (e: StringException) {
		expect(e.error == "EOF while parsing a value at line 1 column 5", "a parse error is serde_json's")
	}
	try {
		CatalogHangExtra.default().use { it.set("json", Json.parse("1")) }
		expect(false, "a reserved section is an error")
	} catch (e: MuxErrorException) {
		expect(e.message == "reserved catalog section: json", "a reserved section is moq-mux's error")
	}

	// moq-tokio's terminal connect errors, which moq-net's `Error` shares no names with.
	expect(ConnectError.UNAUTHORIZED.isAuth(), "unauthorized is an auth error")
	expect(ConnectError.UNAUTHORIZED.toString() == "unauthorized", "a connect error prints as Rust's")

	// Session config's foreign types. A URL is parsed from text, failing with why.
	try {
		Url.parse("relay.example").close()
		expect(false, "a relative URL is an error")
	} catch (e: StringException) {
		expect(e.message == "relative URL without a base", "a URL's parse error is Rust's")
	}
	// A session's config, its URL and pinned fingerprints set as Rust assigns them.
	val connect = ConnectConfig.default()
	connect.tls.use { tls ->
		tls.setFingerprint(listOf("ab:cd"))
		connect.setTls(tls)
	}
	connect.setUrl(Url.parse("https://relay.example:4443/demo"))
	connect.tls.use { tls -> expect(tls.fingerprint == listOf("ab:cd"), "a TLS config keeps its fingerprints") }
	val url = connect.url!!
	connect.close()
	val versions = Versions.default()
	AuthTokens.new(emptyList()).use { tokens ->
		tokens.dial(url, versions).use { dial ->
			dial.url.use { dialed ->
				expect(dialed == url, "a dial keeps its URL")
				expect(dialed.toString() == "https://relay.example:4443/demo", "a URL prints as Rust's")
			}
		}
	}
	url.close()
	// A `Vec<String>`, as a List.
	expect(ALPNS.isNotEmpty(), "a constant slice crosses as a List")
	val alpns = versions.alpns()
	expect(alpns.isNotEmpty() && alpns.all { it.isNotEmpty() }, "each version has an ALPN")
	versions.close()
	// A socket address, crossing in a listener's bind.
	val loopback = ByteArray(16).also { it[15] = 1 }
	val bind = ListenBind.Addr(java.net.InetSocketAddress(java.net.Inet6Address.getByAddress(null, loopback, null as java.net.NetworkInterface?), 4443))
	expect(bind.toString() == "[::1]:4443", "an IPv6 bind prints as Rust's")
	val v4 = ListenBind.Addr(java.net.InetSocketAddress(java.net.InetAddress.getByAddress(byteArrayOf(127, 0, 0, 1)), 0))
	expect(v4.toString() == "127.0.0.1:0", "an IPv4 bind prints as Rust's")
	ListenConfig.default().use { listen ->
		listen.setBind(bind)
		// The preferred address of each family, which takes only that family.
		listen.setPreferredV4(v4._0)
		expect(listen.preferredV4 == v4._0 && listen.preferredV6 == null, "a listener keeps its preferred IPv4 address")
		expect(listen.bind == bind, "a listener's config keeps its bind")
	}
	// TLS files, as OS paths, and fingerprints, as strings: none by default.
	TlsConnect.default().use { tls ->
		expect(tls.root.isEmpty() && tls.fingerprint.isEmpty(), "a default TLS config lists none")
	}

	// A session, as moq.c runs it: a server bound to port 0 reads back the port it got and
	// serves with one call, in a coroutine of its own, and a client dials it with one call
	// and reads a frame back, each suspend fun resuming once moq-tokio's fallback runtime
	// wakes it from its threads. A stall fails with the timeout's exception.
	runBlocking {
		withTimeout(30.seconds) {
			val served = originSpawn()
			val live = served.createBroadcast("live")
			live.announce(OriginRoute.default())
			val liveTrack = live.broadcast().createTrack("video", null)
			val liveGroup = liveTrack.appendGroup()
			liveGroup.writeFrame(null, "hello".encodeToByteArray())
			liveGroup.finish()

			val serverConfig = ServerConfig.default()
			val listen = serverConfig.listen
			listen.setBind(ListenBind.Addr(InetSocketAddress(InetAddress.getByName("::"), 0)))
			val tls = listen.tls
			tls.setGenerate(listOf("localhost"))
			listen.setTls(tls)
			serverConfig.setListen(listen)
			val server = serverConfig.init()
			val local = server.localAddr()
			expect(local.port != 0, "a server bound to port 0 reads back the port it got")
			val serving = launch { server.servePublish(served.consume()) }

			val connect = ConnectConfig.default()
			connect.setUrl(Url.parse("moqt://localhost:${local.port}"))
			val insecure = connect.tls
			insecure.setInsecure(true)
			connect.setTls(insecure)
			val clientConfig = ClientConfig.default()
			clientConfig.setConnect(connect)
			val client = clientConfig.init()
			val received = originSpawn()
			val remote = received.consume()
			val connection = client.clone().withReconnect(false).consume(received)
			expect(connection != null, "a client with a URL dials it")
			val established = connection!!.established()

			val subscriber = remote.request("live", null).track("video").subscribe(null)
			val frame = subscriber.recvGroup()?.readFrame()
			expect(frame?.payload?.use { it.toByteArray().decodeToString() } == "hello", "a session reads the frame the server serves")

			established.close_()
			client.close_()
			serving.cancel()
		}
	}

	if (failures == 0) println("ok")
	kotlin.system.exitProcess(if (failures == 0) 0 else 1)
}
