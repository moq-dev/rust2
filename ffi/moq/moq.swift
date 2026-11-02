// moq.c's script through the generated Swift: a broadcast's track, published and
// subscribed to in one process, with every read an async func, and an origin, with
// moq-pattern, moq-clock, moq-tokio, hang, and moq-mux exported into the same library,
// and the catalog lent to Swift for a call.
//
// The local reads need no runtime: each read that must wait resumes on an executor the
// script drains, after the write that wakes it. A session over QUIC runs on moq-tokio's
// fallback runtime, whose threads resume Swift's.

import Foundation
import Moq

nonisolated(unsafe) var failures = 0

func expect(_ ok: Bool, _ what: String) {
	if !ok {
		FileHandle.standardError.write(Data("failed: \(what)\n".utf8))
		failures += 1
	}
}

func text(_ data: Data?) -> String? {
	data.map { String(decoding: $0, as: UTF8.self) }
}

/// A value a task sets and the script reads, under a lock.
final class Cell<T>: @unchecked Sendable {
	private let lock = NSLock()
	private var stored: T

	init(_ value: T) {
		stored = value
	}

	var value: T {
		get {
			lock.lock()
			defer { lock.unlock() }
			return stored
		}
		set {
			lock.lock()
			defer { lock.unlock() }
			stored = newValue
		}
	}
}

/// The jobs tasks resume with, run when the script drains them.
final class Queue: TaskExecutor, @unchecked Sendable {
	private let lock = NSLock()
	private var jobs: [UnownedJob] = []

	func enqueue(_ job: UnownedJob) {
		lock.lock()
		defer { lock.unlock() }
		jobs.append(job)
	}

	var size: Int {
		lock.lock()
		defer { lock.unlock() }
		return jobs.count
	}

	func drain() {
		while true {
			lock.lock()
			let job = jobs.isEmpty ? nil : jobs.removeFirst()
			lock.unlock()
			guard let job else { return }
			job.runSynchronously(on: asUnownedTaskExecutor())
		}
	}

	func launch(_ body: @escaping @Sendable () async throws -> Void) {
		Task(executorPreference: self, operation: body)
	}
}

/// `body`'s result, waiting on this thread for it to finish on the global executor.
func block<T: Sendable>(_ body: @escaping @Sendable () async throws -> T) throws -> T {
	let done = DispatchSemaphore(value: 0)
	let result = Cell<Result<T, Swift.Error>?>(nil)
	Task {
		do {
			result.value = .success(try await body())
		} catch {
			result.value = .failure(error)
		}
		done.signal()
	}
	done.wait()
	return try result.value!.get()
}

func script() throws {
	let q = Queue()

	let broadcast = BroadcastInfo.new().produce()
	let track = try broadcast.createTrack("video", nil)
	expect(track.name() == "video", "a track has its name")

	// A second track of the same name is an error, printed as Rust prints it.
	do {
		_ = try broadcast.createTrack("video", nil)
		expect(false, "a duplicate track is an error")
	} catch let e as Moq.Error {
		expect("\(e)" == "duplicate", "a duplicate track's error is Rust's")
	}

	let subscriber = try block { try await track.consume().subscribe(nil) }

	// A group's arrival wakes the subscriber's pending receive.
	let group = Cell<GroupConsumer?>(nil)
	q.launch { group.value = try await subscriber.recvGroup() }
	q.drain()
	expect(group.value == nil, "a receive waits for a group")
	let writer = try track.appendGroup()
	expect(q.size == 1, "a group's arrival wakes the receive")
	q.drain()
	let first = group.value!
	expect(first.index() == 0, "the first group is 0")

	// So does a frame's.
	let frame = Cell<FrameFrame?>(nil)
	q.launch { frame.value = try await first.readFrame() }
	q.drain()
	expect(frame.value == nil, "a read waits for a frame")
	try writer.writeFrame(nil, Data("hello".utf8))
	expect(q.size == 1, "a frame's arrival wakes the read")
	q.drain()
	let hello = frame.value!
	expect(text(hello.payload) == "hello", "a frame has its payload")
	expect(hello.timestamp == nil, "an untimed frame has no timestamp")
	// When it arrived: moq-clock's `Instant`, from moq-clock's own export.
	expect(now().checkedDurationSince(hello.arrived()) != nil, "a frame arrived before now")

	// A timestamp is a `Copy` value behind a handle: passing it copies it out. The track is
	// untimed, so a timestamped frame is refused.
	let at = try Timestamp.fromMillis(1500)
	expect(at.asSecs() == 1, "1500 ms is 1 s")
	do {
		try writer.writeFrame(at, Data("late".utf8))
		expect(false, "a timestamped frame on an untimed track is an error")
	} catch let e as Moq.Error {
		expect("\(e)" == "frame timestamp doesn't fit its track", "the late frame's error is Rust's")
	}
	expect(at.asSecs() == 1, "a copied timestamp stays usable")

	// The last frame, bytes that are not UTF-8, then the group's end, which reads as nil.
	let world = Data([0xff]) + Data("world".utf8)
	try writer.writeFrame(nil, world)
	try writer.finish()
	try block {
		expect(try await first.readFrame()?.payload == world, "the last frame reads")
		expect(try await first.readFrame() == nil, "a finished group reads nil")
	}

	// An origin and the driver that runs it, a pair. Its root, which it lends, is returned
	// as a new handle to a clone.
	let (origin, driver) = OriginProducer.new(OriginConfig.default())
	expect(origin.root().asStr() == "", "an origin's root is empty")

	// Scoped beneath a root to patterns, which moq-pattern's own export builds.
	let patterns = Patterns.new()
	expect(try patterns.insert(Pattern.subtree("alice")), "a pattern inserts")
	let scoped = try origin.scope("room", patterns)
	expect(scoped.root().asStr() == "room", "a scope has its root")
	expect(try scoped.absolute("alice").asStr() == "room/alice", "a scope's paths are absolute")
	let allowed = scoped.allowed()
	expect(allowed.matches("alice/cam"), "a scope allows its patterns")
	expect(!allowed.matches("bob"), "a scope allows nothing else")
	let consumer = scoped.consume()
	expect(try consumer.absolute("bob").asStr() == "room/bob", "a scope's consumer shares its root")
	consumer.close()
	scoped.close()
	patterns.close()

	// A broadcast the origin creates is stamped with its path, which its info lends. Each
	// handle to it is closed, since an open one keeps it, and so the origin, alive.
	do {
		let publication = try origin.createBroadcast("room/alice")
		defer { publication.close() }
		let created = publication.broadcast()
		let path = created.info().path
		created.close()
		expect(path.asStr() == "room/alice", "a broadcast has its path")

		// Paths own their segments, so Swift builds them, and relative ones, from a string.
		do {
			_ = try Path.new("/cam/")
			expect(false, "a path with an empty segment is an error")
		} catch let e as PathInvalid {
			expect(e == .emptySegment, "an empty segment is its own error")
		}
		let cam = try Path.new("cam")
		expect(cam.asStr() == "cam", "a path builds from its segments")
		let joined = try path.join(cam)
		expect(joined.asStr() == "room/alice/cam", "paths join")
		expect(joined.stripPrefix(path) == cam, "a path strips its prefix")
		expect(path.stripPrefix(joined) == nil, "a path strips no longer prefix")
		expect(try joined.resolve(Relative.new("../bob")).asStr() == "room/bob", "a relative path resolves")
	}

	// The driver, polled as a language's event loop would: at the time, and with a waiter
	// made from the waker and held across polls, since what a poll registers lives only as
	// long as its waiter. It runs until every owner of the origin is gone.
	let wakes = Cell(0)
	let waiter = Waiter.new { wakes.value += 1 }
	_ = try driver.poll(now(), waiter)
	// A client publishes the origin through moq-net's generic `with_publisher`, at each
	// type that implements its bound, by reference, so Swift keeps its handle.
	Client.new().withPublisherProducer(origin).close()
	let before = wakes.value
	origin.close()
	// The last owner's close wakes the driver, which then finishes, cleanly.
	expect(wakes.value > before, "the last owner's close wakes the driver")
	do {
		_ = try driver.poll(now(), waiter)
		expect(false, "a driver whose origin is gone finishes")
	} catch let e as Moq.Error {
		expect("\(e)" == "closed", "a finished driver's error is Rust's")
	}

	// moq-mux reads an OpusHead into hang's catalog config: 44.1 kHz in, stereo, which the
	// catalog lists at the decoder's 48 kHz, keeping the head as its description.
	let head = Data("OpusHead".utf8) + Data([1, 2, 0x38, 0x01, 0x44, 0xac, 0, 0, 0, 0, 0])
	let audio = try codecOpusAudioConfig(head)
	expect(audio.sampleRate == 48000, "Opus decodes at 48 kHz")
	expect(audio.channelCount == 2, "the head is stereo")
	expect(audio.codec == .opus, "the codec is Opus")
	expect(audio.description == head, "the head is the description")
	// A truncated head is moq-mux's error.
	do {
		_ = try codecOpusAudioConfig(head.prefix(8))
		expect(false, "a truncated head is an error")
	} catch let e as MuxError {
		expect(!"\(e)".isEmpty, "a truncated head's error has a message")
	}

	// A subscription's ranges, from any iterable of them: an array.
	let groups = TrackRange.groups(BoundsU64(start: .included(2), end: .excluded(6)))
	let ranged = TrackSubscription.default().withRanges([TrackRange.live(), groups]).ranges
	expect(ranged == [TrackRange.live(), groups], "a subscription keeps its ranges")
	expect(groups.start == .absolute(2), "groups start at their first")
	expect(groups.end == .absolute(6), "groups end before their end")

	// A catalog at moq-mux's untyped extension, from the config Swift builds, whose generic
	// `is_claimed` is named by the last segment of its argument, and whose `mutate` lends
	// the catalog to Swift.
	let publisher = BroadcastInfo.new().produce()
	let extra = CatalogConfig.default().withCatalogExtra(HangCatalogExtra.default())
	let catalog = try CatalogProducerExtra.new(publisher, extra)
	expect(!catalog.isClaimedBinaryConfig("state"), "an unclaimed name is not claimed")
	let read = Cell(0)
	try catalog.mutate { lent in
		if try! lent.toJson().hasPrefix("{") {
			read.value += 1
		}
	}
	expect(read.value == 1, "mutate lends the catalog once")

	// A JSON track the catalog lists, published through moq-mux's generic `json_snapshot` at
	// a JSON document, and a section Swift parses from text, set in the lent catalog and
	// read back as text.
	let status = try catalog.jsonSnapshotConfig(
		try publisher.createTrack("status", nil), JsonConfig.default(), JsonOptions.default())
	expect(status.name() == "status", "a JSON snapshot has its track's name")
	expect(catalog.isClaimedJsonConfig("status"), "the JSON snapshot claims its name")
	// A value Swift parses is published as the track's latest, which a subscriber reads
	// back as JSON text.
	let on = try Json.parse(#"{"on": true}"#)
	try status.update(on)
	on.close()
	try block {
		let frame = try await status.consume().recvGroup()?.readFrame()
		expect(text(frame?.payload) == #"{"on":true}"#, "a subscriber reads the latest value")
	}
	try catalog.mutate { lent in
		let sections = lent.ext
		try! sections.set("app", Json.parse(#"{"v": [1, true]}"#))
		lent.setExt(sections)
	}
	let app = Cell<String?>(nil)
	try catalog.mutate { lent in
		expect(try! lent.toJson().contains("\"status\""), "the catalog lists the JSON track")
		app.value = lent.ext.get("app")?.toText()
	}
	expect(app.value == #"{"v":[1,true]}"#, "the catalog reads its section back as text")
	// Malformed text fails to parse, and a reserved section is moq-mux's error.
	do {
		try Json.parse(#"{"v":"#).close()
		expect(false, "malformed JSON fails to parse")
	} catch let e as StringError {
		expect(e.error == "EOF while parsing a value at line 1 column 5", "a parse error is serde_json's")
	}
	do {
		try CatalogHangExtra.default().set("json", Json.parse("1"))
		expect(false, "a reserved section is an error")
	} catch let e as MuxError {
		expect("\(e)" == "reserved catalog section: json", "a reserved section is moq-mux's error")
	}

	// moq-tokio's terminal connect errors, which moq-net's `Error` shares no names with.
	expect(ConnectError.unauthorized.isAuth(), "unauthorized is an auth error")
	expect("\(ConnectError.unauthorized)" == "unauthorized", "a connect error prints as Rust's")

	// Session config's foreign types. A URL is parsed from text, failing with why.
	do {
		try Url.parse("relay.example").close()
		expect(false, "a relative URL is an error")
	} catch let e as StringError {
		expect(e.error == "relative URL without a base", "a URL's parse error is Rust's")
	}
	// A session's config, its URL and pinned fingerprints set as Rust assigns them.
	let connect = ConnectConfig.default()
	let tls = connect.tls
	tls.setFingerprint(["ab:cd"])
	connect.setTls(tls)
	tls.close()
	connect.setUrl(try Url.parse("https://relay.example:4443/demo"))
	expect(connect.tls.fingerprint == ["ab:cd"], "a TLS config keeps its fingerprints")
	let url = connect.url!
	connect.close()
	let versions = Versions.default()
	let tokens = AuthTokens.new([])
	let dial = tokens.dial(url, versions)
	let dialed = dial.url
	expect(dialed == url, "a dial keeps its URL")
	expect("\(dialed)" == "https://relay.example:4443/demo", "a URL prints as Rust's")
	dialed.close()
	dial.close()
	tokens.close()
	url.close()
	// A `Vec<String>`, as an array.
	expect(!ALPNS.isEmpty, "a constant slice crosses as an array")
	let alpns = versions.alpns()
	expect(!alpns.isEmpty && alpns.allSatisfy { !$0.isEmpty }, "each version has an ALPN")
	versions.close()
	// A socket address, crossing in a listener's bind.
	let loopback: [UInt8] = Array(repeating: 0, count: 15) + [1]
	let bind = ListenBind.addr(SocketAddr(ip: loopback, port: 4443))
	expect("\(bind)" == "[::1]:4443", "an IPv6 bind prints as Rust's")
	let v4 = SocketAddr(ip: [127, 0, 0, 1], port: 0)
	expect("\(ListenBind.addr(v4))" == "127.0.0.1:0", "an IPv4 bind prints as Rust's")
	let listen = ListenConfig.default()
	listen.setBind(bind)
	// The preferred address of each family, which takes only that family.
	listen.setPreferredV4(v4)
	expect(listen.preferredV4 == v4 && listen.preferredV6 == nil, "a listener keeps its preferred IPv4 address")
	expect(listen.bind == bind, "a listener's config keeps its bind")
	listen.close()
	// TLS files, as OS paths, and fingerprints, as strings: none by default.
	let defaults = TlsConnect.default()
	expect(defaults.root.isEmpty && defaults.fingerprint.isEmpty, "a default TLS config lists none")
	defaults.close()

	// A session, as moq.c runs it: a server bound to port 0 reads back the port it got and
	// serves with one call, in a task of its own, and a client dials it with one call and
	// reads a frame back, each async func resuming once moq-tokio's fallback runtime wakes
	// it from its threads. A stall fails the script.
	DispatchQueue.global().asyncAfter(deadline: .now() + 30) {
		FileHandle.standardError.write(Data("failed: the session stalled\n".utf8))
		exit(1)
	}
	try block {
		let served = originSpawn()
		let live = try served.createBroadcast("live")
		try live.announce(OriginRoute.default())
		let liveTrack = try live.broadcast().createTrack("video", nil)
		let liveGroup = try liveTrack.appendGroup()
		try liveGroup.writeFrame(nil, Data("hello".utf8))
		try liveGroup.finish()

		let serverConfig = ServerConfig.default()
		let listen = serverConfig.listen
		listen.setBind(.addr(SocketAddr(ip: Array(repeating: 0, count: 16), port: 0)))
		let tls = listen.tls
		tls.setGenerate(["localhost"])
		listen.setTls(tls)
		serverConfig.setListen(listen)
		let server = try serverConfig.init_()
		let local = try server.localAddr()
		expect(local.port != 0, "a server bound to port 0 reads back the port it got")
		let serving = Task { try await server.servePublish(served.consume()) }

		let connect = ConnectConfig.default()
		connect.setUrl(try Url.parse("moqt://localhost:\(local.port)"))
		let insecure = connect.tls
		insecure.setInsecure(true)
		connect.setTls(insecure)
		let clientConfig = ClientConfig.default()
		clientConfig.setConnect(connect)
		let client = try clientConfig.init_()
		let received = originSpawn()
		let remote = received.consume()
		let connection = client.clone().withReconnect(false).consume(received)
		expect(connection != nil, "a client with a URL dials it")
		let established = try await connection!.established()

		let subscriber = try await remote.requestBroadcast("live", nil).track("video").subscribe(nil)
		let frame = try await subscriber.recvGroup()?.readFrame()
		expect(text(frame?.payload) == "hello", "a session reads the frame the server serves")

		try await established.close_()
		try await client.close_()
		serving.cancel()
	}
}

@main
struct Main {
	static func main() {
		do {
			try script()
		} catch {
			expect(false, "the script threw \(error)")
		}
		if failures == 0 {
			print("ok")
		}
		exit(failures == 0 ? 0 : 1)
	}
}
