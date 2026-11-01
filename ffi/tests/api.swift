// The api fixture's Swift test: the script fixtures/api/examples/transcript.rs runs
// natively, through the generated Swift. tests/fixtures.rs compares the transcripts.
//
// With an argument, it instead checks the futures' semantics, what releasing frees, or
// breaks a contract, which must fail.

import Api
import Foundation

// The Log callback: keeps even levels.
final class PrintLog: Log {
	func log(_ level: UInt8, _ line: String) -> Bool {
		print("log \(level) \(line)")
		return level % 2 == 0
	}

	// Swift frees it once Rust drops the implementation.
	deinit {
		print("freed")
	}
}

func bound(_ b: BoundU64) -> String {
	switch b {
	case .included(let n): return "Included(\(n))"
	case .excluded(let n): return "Excluded(\(n))"
	case .unbounded: return "Unbounded"
	}
}

func debug<T>(_ value: T) -> String {
	String(reflecting: value)
}

func text(_ data: Data) -> String {
	String(decoding: data, as: UTF8.self)
}

/// A path's UTF-8 bytes as Rust's `escape_ascii` prints them.
func escaped(_ path: String) -> String {
	var out = ""
	for b in path.utf8 {
		switch b {
		case 0x09: out += "\\t"
		case 0x0d: out += "\\r"
		case 0x0a: out += "\\n"
		case 0x5c, 0x27, 0x22: out += "\\" + String(UnicodeScalar(b))
		case 0x20..<0x7f: out += String(UnicodeScalar(b))
		default: out += "\\x" + String(b, radix: 16).leftPadded(2)
		}
	}
	return out
}

extension String {
	func leftPadded(_ width: Int) -> String {
		String(repeating: "0", count: max(0, width - count)) + self
	}
}

func check(_ ok: Bool, _ what: String) {
	if !ok {
		FileHandle.standardError.write(Data("failed: \(what)\n".utf8))
		exit(1)
	}
}

/// A value a task sets and the test reads, under a lock.
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

/// A queue of jobs that a test runs when it chooses: the executor its tasks prefer, so a
/// call resumes on it.
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

	/// Run `body` as a task that prefers this queue.
	@discardableResult
	func launch(_ body: @escaping @Sendable () async throws -> Void) -> Task<Void, Swift.Error> {
		Task(executorPreference: self, operation: body)
	}
}

/// `body`'s result, waiting on this thread for it to finish on the global executor.
func block<T: Sendable>(_ body: @escaping @Sendable () async throws -> T) -> T {
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
	return try! result.value!.get()
}

func script() {
	print("limit \(LIMIT)")

	let p = Point(x: 1, y: 2).offset(2, 3)
	print("offset \(p.x) \(p.y)")
	print("debug \(debug(p))")
	print("eq \(p == Point(x: 3, y: 5))")
	let d = Point.default()
	print("default \(d.x) \(d.y)")

	let c = Counter.new("c", 5)
	print("get \(c.get())")
	print("name \(c.name())")
	print("add ok \(try! c.add(10))")
	do {
		_ = try c.add(2000)
	} catch let e as Api.Error {
		print("add err \(e) \(debug(e))")
	} catch {
		check(false, "add throws an Error")
	}
	do {
		_ = try c.sub(20)
	} catch let e as Api.Error {
		print("sub err \(e) \(debug(e))")
	} catch {
		check(false, "sub throws an Error")
	}
	c.rename("d")
	print("name \(c.name())")
	print("is d \(c.`is`("d"))")
	let copy = c.clone()
	_ = try! copy.add(1)
	print("clone \(copy.get()) original \(c.get())")
	copy.close()
	print("debug \(debug(c))")
	let label = c.intoLabel(p)
	print("label \(debug(label))")
	print("note \(hasText(Note(text: ""))) \(hasText(Note(text: nil)))")

	let (below, above) = Counter.new("s", 7).split(5)
	print("split \(below.name()) \(below.get()) \(above.name()) \(above.get())")
	let one = only(Counter.new("o", 3))
	print("only \(one.name()) \(one.get())")
	let (w8, w16, w32, w64) = widths(98305)
	print("widths \(w8) \(w16) \(w32) \(w64)")
	for b: UInt64 in [2, 0] {
		do {
			let (q, r) = try divide(7, b)
			print("divide \(q) \(r)")
		} catch {
			print("divide err \(error)")
		}
	}
	for n: UInt64 in [98305, 0] {
		if let s = spread(n) {
			print("spread \(s.0) \(s.1) \(s.2) \(s.3)")
		} else {
			print("spread none")
		}
	}
	for needle in ["lo", "x"] {
		if let found = find(Data("hello".utf8), Data(needle.utf8)) {
			print("find \(found.0) \(found.1)")
		} else {
			print("find none")
		}
	}
	let statRows = rows([Point(x: 1, y: 2), Point(x: 3, y: 3)])
	print("rows \(statRows.map { "\($0.0) \(debug($0.1)) \($0.2.x) \($0.2.y)" }.joined(separator: ", "))")
	// Each handle in a tuple moves in, as one taken by value does.
	let board = Board.new([("low", Counter.new("l", 1)), ("high", Counter.new("h", 9))])
	let ranked = board.ranked()
	print("ranked \(ranked.map { "\($0.0) \($0.1.name()) \($0.1.get())" }.joined(separator: ", "))")
	board.close()
	let span = Span(bounds: (2, 10), named: ("s", .square))
	let (lowerHalf, upperHalf) = halves(span)
	print("halves \(lowerHalf.0) \(lowerHalf.1) \(upperHalf.0) \(upperHalf.1) \(debug(span))")
	let (swappedN, swappedText) = swapped(("t", 4))
	print("swapped \(swappedN) \(swappedText)")
	// A tuple of one is its element, so `Option<(Option<u32>,)>` nests two optionals.
	for value: UInt32?? in [.none, .some(nil), .some(3)] {
		switch doubled(value) {
		case .none: print("doubled None")
		case .some(let inner): print("doubled Some((\(inner.map { "Some(\($0))" } ?? "None"),))")
		}
	}
	print("lent back \(Array(lentBack(Data([1, 2])))) boxed \(Array(boxedTuple(Data([3]))))")
	// A borrowed string and an owned one share a tuple struct.
	print("title Some((\"\(title(true)!)\",)) \(title(false) == nil ? "None" : "Some") named Some((\"\(named("x")!)\",))")

	// Each lent counter is a new handle to a clone.
	let registry = Registry.new(Counter.new("first", 1))
	print("last \(registry.last() != nil)")
	registry.push(Counter.new("last", 2))
	let first = registry.first
	_ = try! first.add(10)
	print("first \(registry.first.get()) lent \(first.get())")
	let last = registry.last()!
	_ = try! last.add(10)
	print("last \(registry.last()!.get()) lent \(last.get())")
	// A closure lent for the call edits the first counter through a handle.
	let by: UInt64 = 5
	let edited = registry.edit { counter in
		_ = try! counter.add(by)
		return counter.edited() && counter.get() == 6
	}
	print("edit \(edited) \(registry.first.get())")
	// A setter moves an object in, as assignment does, closing its handle.
	registry.setFirst(Counter.new("second", 3))
	print("set first \(registry.first.get()) \(registry.first.name())")
	print("walk \(walk(3) { cursor in cursor.at() * 2 })")
	print("each \(each(2) { at in print("visit \(at)") })")
	print("offset \(offset({ 7 }, 3))")
	print("shift \(shift({ 7 }, 4))")

	let name = Name.owned("a")
	let joined = name.join(Name.owned("b"))
	print("joined \(joined.asStr()) \(debug(joined)) \(joined == name)")
	joined.rename("c")
	print("renamed \(joined.asStr())")
	let long = longer(joined, Name.owned("dd"))
	print("longer \(long.asStr())")
	print("unnamed [\(unnamed().asStr())]")

	print("total \(total(RangeU64(start: 2, end: 5)))")
	let wide = widen(RangeU64(start: 2, end: 5), 1)
	print("widen \(wide.start) \(wide.end)")
	let counts = [
		BoundsU64(start: .included(2), end: .excluded(5)),
		BoundsU64(start: .included(2), end: .included(5)),
		BoundsU64(start: .included(3), end: .unbounded),
		BoundsU64(start: .unbounded, end: .unbounded),
		BoundsU64(start: .excluded(2), end: .included(4)),
	].map { count($0) }
	print("count \(counts.map(String.init).joined(separator: " "))")
	print("end \(bound(end(3))) \(bound(end(0)))")
	let (endsStart, endsEnd) = ends(3)
	print("ends \(bound(endsStart)) \(bound(endsEnd))")
	for n: UInt64 in [2, 5] {
		do {
			print("below \(try Api.below(n, 3))")
		} catch let e as BoundsU64Error {
			print("below err \(bound(e.error.start)) \(bound(e.error.end))")
		} catch {
			check(false, "below throws its bounds")
		}
	}
	let window = Window(range: RangeU64(start: 2, end: 8), end: end(5))
	print("window \(window.contains(4)) \(window.contains(6)) \(debug(window))")

	print(greet("x", .square))
	print("len \(len(Data("abc".utf8)))")
	let binary = Data([0xff, 0, 0x61])
	print("hex \(hex(binary))")
	print("timed hex \(timedHex(binary))")
	print("sample hex \(sampleHex(binary))")
	print("queued hex \(queuedHex(binary))")
	print("tick \(tickName(Label(name: "tick", at: nil, ttl: nil)))")
	print("rooted \(rootedName(Label(name: "rooted", at: nil, ttl: nil)))")
	print("stored \(storedText("stored"))")
	print("layer \(layerName(Label(name: "layer", at: nil, ttl: nil)))")
	for event: Event in [.moved(p), .named(name: "n"), .done] {
		print(describe(event))
	}
	check(advance(.init_(1)) == .init_(0) && advance(.init_(0)) == .done, "a variant named init crosses")

	// A `Copy` handle passed by value is copied, so it stays usable.
	let stamp = Stamp.fromMillis(2500)
	print("secs \(stamp.secs())")
	print("later \(Stamp.fromMillis(3000).later(stamp))")
	print("secs \(stamp.secs())")
	let message = Message.new(stamp, Data("body".utf8))
	print("payload \(text(message.payload))")
	print("stamp \(debug(message.stamp!))")
	print("sent \(message.sent())")
	// Setters assign in place: the bytes are copied in, and a `Copy` handle copied.
	let reset = Stamp.fromMillis(4000)
	message.setStamp(reset)
	message.setPayload(Data("edited".utf8))
	print("payload \(text(message.payload))")
	print("stamp \(debug(message.stamp!))")
	print("secs \(reset.secs())")
	message.setStamp(nil)
	print("stamp \(message.stamp != nil)")

	// Bound first, so Swift's reference is gone before the print.
	let kept = emit(PrintLog(), Data([1, 2, 3, 4]))
	print("kept \(kept)")
	let logger = Logger.new(PrintLog())
	print("write \(logger.write("hi"))")
	logger.close()
	print("dropped")
	// Rust calls it on another thread, before the call returns.
	let threshold: UInt8 = 2
	let filtered = filter(Data([1, 2, 3, 4])) { level, line in
		print("filter \(level) \(line)")
		return level > threshold
	}
	print("filtered \(filtered)")

	let channel = Channel.new()
	let wakes = Cell(0)
	// On an executor the script drains, so it sees the receive wait and its wake queue
	// the resume.
	let q = Queue()
	let got = Cell<Data?>(nil)
	q.launch { got.value = try await channel.recv() }
	q.drain()
	print("recv pending \(got.value == nil)")
	channel.send(Data("hi".utf8))
	wakes.value += q.size
	print("woken \(wakes.value)")
	q.drain()
	print("recv \(text(got.value!))")
	block {
		channel.send(Data())
		do {
			print("nonempty \(try await channel.recvNonempty().count)")
		} catch {
			print("nonempty err \(error)")
		}
		// The send happens at the call, before the future is first polled.
		try await channel.sendLater(Data("eager".utf8))
		print("recv \(text(try await channel.recv()))")
		channel.send(Data("sized".utf8))
		let (payload, size) = try await channel.recvSized()
		print("sized \(text(payload)) \(size)")
	}
	// The waker moves into Rust, which keeps it until the send wakes it.
	channel.notify { wakes.value += 1 }
	channel.send(Data("rung".utf8))
	print("woken \(wakes.value)")

	let list = Node.new(1, nil)
	list.push(2)
	print("node \(list.sum())")

	let tally = Tally.new()
	_ = tally.bump()
	print("tally \(tally.bump()) \(tally == tally)")
	print("both \(bumpBoth(tally, tally))")
	print("peek \(peek(tally) { lent in lent.bump() })")

	let area = grow(Area.new(Size(width: 2, height: 3)), 1)
	let size = area.size()
	print("area \(size.width) \(size.height) \(area.pixels())")

	// Generic types, each at the instances it infers, named by their arguments.
	let pair = PairU32(left: 2, right: 5)
	let swappedPair = pair.swap()
	print("pair \(swappedPair.left) \(swappedPair.right) sum \(pair.sum()) \(debug(pair)) \(pair == swappedPair)")
	print("even \(pair.even()) \(PairU32(left: 7, right: 7).even())")
	// An instance renamed alone, beside another of `Pair` that keeps its name.
	let sized: SizeTag = tag(Size(width: 1, height: 2), 3)
	let flipped: PairU8Size = sized.swap()
	print("sized \(flipped.left) \(flipped.right.width) \(debug(flipped))")
	let beside = PairU32(left: 258, right: 9).besideSize(sized.left)
	print("beside \(beside.left) \(debug(beside))")
	// At its default, named as the type itself.
	let stack = Stack.new()
	for (i, n) in ["a", "b"].enumerated() {
		stack.push(Counter.new(n, UInt64(i) + 1))
	}
	let top = stack.pop()!.get()
	print("stack \(stack.depth()) Some(\(top)) \(stack.pop() != nil) \(stack.pop() == nil)")
	// Generic functions and methods, at each implementation of the trait bounding them, by
	// reference, which leaves Swift its handle.
	let fromPoint = Counter.seededPoint("p", Point(x: -3, y: 4))
	let fromSize = Counter.fromSize("s", Size(width: 5, height: 6))
	let fromCounter = Counter.seededCounter("c", fromPoint)
	let fromPolar = Counter.seededPolarPoint("q", PolarPoint(turns: 1, distance: 2))
	print(
		"seeded \(fromPoint.get()) \(fromSize.get()) \(fromCounter.get()) \(fromPolar.get()) "
			+ "past \(stack.depthPastPoint(Point(x: 1, y: 2))) seeds \(seedsPointSize(Size(width: 2, height: 3)))")

	// A trait implemented for `Point` and `&Point` apart: one instance.
	print("weigh \(weighPoint(Point(x: -1, y: 2)))")
	// Instances that name each other, and one made only by an impl of it.
	let crossed = cross(LeftRightU8(value: RightU8(value: 1)), RightLeftU8(value: LeftU8(value: 2)))
	print("cross \(crossed) outer \(OuterInnerU8.new(4).get())")
	// A bound that outlives the call takes a value, and an instance only a constant names.
	print("kept \(keptPoint(Point(x: 1, y: -2))) boxed \(BOXED.value)")
	print("keep \(keepPoint(Point(x: 1, y: -2)))")
	// Inferred instances taking a lent closure and a foreign implementation, and one only
	// its impl of a trait names.
	let sides = PairU32(left: 2, right: 5)
	print("apply \(applyU32(sides) { side in side * 3 })")
	let report = reportU32(sides, PrintLog())
	print("report \(report)")
	print("wrapped \(keptWrapU32(WrapU32(value: 4)))")
	let keptLog = keeps(PrintLog())
	print("keeps \(keptLog.value)")

	// Its Drop runs when Swift closes the handle, not before.
	let token = Token.new(7)
	print("token \(token.id) dropped \(dropped())")
	token.close()
	print("dropped \(dropped())")

	// A field and the method of its name, each its own member.
	let retry = Retry.new(5)
	let limit = retry.limitSetting
	print("retry \(limit != nil ? 1 : 0) \(limit ?? 0) \(retry.limit()) \(retry.delay())")
	// The renamed field's setter, named as its getter.
	retry.setLimitSetting(nil)
	print("retry \(retry.limit())")
	print("slot \(SlotU8.new(200).value())")

	// Foreign types: a URL parsed from text, socket addresses, OS paths, and `Vec`s.
	let base = try! Url.parse("https://example.com/a/b")
	print("url \(base) host \(host(base).map { "Some(\"\($0)\")" } ?? "None")")
	let url = try! resolve(base.clone(), "c?d")
	print("resolve \(url) \(url == base)")
	url.close()
	do {
		try Url.parse("no scheme").close()
	} catch let e as StringError {
		print("parse err \(e.error)")
	} catch {
		check(false, "parse throws its message")
	}
	let v4 = SocketAddr(ip: [127, 0, 0, 1], port: 8080)
	let loopback: [UInt8] = Array(repeating: 0, count: 15) + [1]
	let v6 = SocketAddr(ip: loopback, port: 443, scopeId: 3)
	print("ports \(ports([v4, v6]))")
	let peer = Peer(addr: v6, names: ["a", "b"]).next("c")
	print("peer \(debug(peer)) \(peer.aliases())")
	let file = file("dir", "f\\'\"\t")
	print("file \(escaped(file))")
	print("parents \(parents([file, "/"]).map { $0.map(escaped) ?? "none" }.joined(separator: " "))")
	let words = words(" one two  three ")
	print("words \(words.count) \(words)")
	print("sentence \(sentence(words)) \(sentence(nil))")
	print("sentence \(sentence([]))|")
	print("chunks \(chunks(Data("abcde".utf8), 2).map { Array($0) })")
	let counters = counters(["x", "y"])
	print("counters \(counters.map { "\($0.name()) \($0.get())" }.joined(separator: ", "))")
	// Each counter moves in, so Swift closes it.
	print("worth \(worth([counters[0]], [counters[1]])) \(worth(nil, []))")
	print("sums \(sums([[1, 2], [], [3]]))")
	let sum = summed([Point(x: 1, y: 2), Point(x: -4, y: 6)])
	let noneSum = summed([])
	print("summed \(sum.x) \(sum.y) \(noneSum.x) \(noneSum.y)")
	print("longest \(longest(["a", "abc"])) \(longest([]))")
	let doc = try! Json.parse(#"{"b": [1, 2.5, null], "a": "é"}"#)
	print("json \(encode(doc))")
	print("timed text \(timedText(doc))")
	let tagged = tagged(doc, "t")
	print("tagged \(tagged.toText())")
	tagged.close()
	do {
		try Json.parse("{\"a\":").close()
	} catch let e as StringError {
		print("json parse err \(e.error)")
	} catch {
		check(false, "parse throws its message")
	}
	let site = Site.new(base)
	print("site \(site.home) \(site.home)")

	// Slices, passed in and returned as clones, and the socket address of each family.
	print("shapes \(Shape.ALL) greetings \(GREETINGS)")
	let trail = Trail.new([Point(x: 1, y: 2), Point(x: 3, y: 4)], ["a", "b"])
	print("trail \(trail.points()) \(trail.labels())")
	// Returned iterators, each collected at the call: drained labels are gone after.
	print("iterated \(trail.iter()) \(trail.names()) \(Array(trail.bytes()))")
	print("drained \(trail.drain()) then \(trail.labels())")
	trail.close()
	let numbered = numbered(["p", "q"])
	print("numbered \(numbered.map { "\($0.name()) \($0.get())" }.joined(separator: ", "))")
	let (next4, next6) = nexts(SocketAddr(ip: [127, 0, 0, 1], port: 8080), v6)
	print("nexts \(next4.ip.map(String.init).joined(separator: ".")) \(next4.port) \(next6.port) \(next6.scopeId)")
}

// The futures' semantics beyond the transcript, each checked by an assertion.
func futures() {
	let channel = Channel.new()

	// The send lands from another thread while the receive waits.
	let payload = block {
		Thread.detachNewThread { channel.send(Data("blocked".utf8)) }
		return try await channel.recv()
	}
	check(text(payload) == "blocked", "recv returns the payload another thread sent")

	let q = Queue()

	// Resumed on its executor, never inside the wake.
	do {
		let got = Cell<String?>(nil)
		q.launch { got.value = text(try await channel.recv()) }
		q.drain()
		check(got.value == nil, "recv waits for a send")
		channel.send(Data("then".utf8))
		check(got.value == nil, "recv never resumes inside the wake")
		q.drain()
		check(got.value == "then", "recv resumes with the payload")
	}

	// A wake that lands while the future polls, before it returns pending, is kept: the
	// future polls again rather than waiting for a wake that already happened.
	do {
		let got = Cell<UInt32>(0)
		q.launch { got.value = try await yielded(7) }
		q.drain()
		check(got.value == 7, "a wake during the poll polls again")
	}

	// A future whose output is `None` is ready, not pending.
	do {
		let got = Cell<UInt32??>(nil)
		q.launch { got.value = .some(try await halved(3)) }
		q.drain()
		check(got.value == .some(nil), "a future ready with None returns nil")
		check(block { try await halved(4) } == 2, "a future ready with Some returns its value")
	}

	// Cancelled between its wake and its poll, a receive loses nothing: the payload stays
	// for the next.
	do {
		let called = Cell(false)
		let threw = Cell(false)
		let task = q.launch {
			do {
				_ = try await channel.recv()
				called.value = true
			} catch is CancellationError {
				threw.value = true
			}
		}
		q.drain()
		channel.send(Data("kept".utf8))
		task.cancel()
		q.drain()
		check(!called.value && threw.value, "a cancelled receive throws CancellationError")
		let payload = block { try await channel.recv() }
		check(text(payload) == "kept", "a cancelled receive loses nothing")
	}

	// Cancelling a task that waits for no wake resumes it.
	do {
		let threw = Cell(false)
		let task = q.launch {
			do {
				_ = try await channel.recv()
			} catch is CancellationError {
				threw.value = true
			}
		}
		q.drain()
		check(!threw.value, "recv waits for a send")
		task.cancel()
		q.drain()
		check(threw.value, "cancelling a waiting receive resumes it, throwing")
	}

	// A handle closed while a call awaits on it stays alive until the call ends.
	do {
		let other = Channel.new()
		let got = Cell<String?>(nil)
		q.launch { got.value = text(try await other.recv()) }
		q.drain()
		let sender = other.clone()
		other.close()
		sender.send(Data("held".utf8))
		q.drain()
		check(got.value == "held", "a closed handle stays alive in the call awaiting on it")
		sender.close()
	}
	print("futures ok")
}

// What frees an object or bytes nobody closed: Swift's release of the last reference, at
// once. Returned bytes lend Rust's buffer until released.
func owned() {
	for n in 1023...1025 {
		let payload = Data((0..<n).map { UInt8(truncatingIfNeeded: $0) })
		let packet = nextPacket(Packet(seq: 1, payload: payload))
		check(packet.seq == 2 && packet.payload == payload, "a record's \(n) bytes cross both ways")
	}
	let before = dropped()
	do {
		let bytes = tokenBytes(1, 1025)
		check(dropped() == before, "returned bytes lend Rust's buffer")
		check(bytes == Data(repeating: 1, count: 1025), "returned bytes read back")
	}
	check(dropped() == before + 1, "releasing returned bytes frees Rust's")
	do {
		let token = Token.new(2)
		check(token.id == 2, "a token reads back")
	}
	check(dropped() == before + 2, "releasing an object frees its handle")
	closed()
	check(dropped() == before + 3, "releasing a closed object frees nothing again")
	print("owned ok")
}

func closed() {
	let before = dropped()
	let token = Token.new(3)
	token.close()
	check(dropped() == before + 1, "closing frees at once")
	token.close()
	check(dropped() == before + 1, "closing again frees nothing")
}

/// Holds a lent counter past the call it was lent for.
final class Kept: @unchecked Sendable {
	var counter: Counter?
}

// Each breaks a contract, which must fail with a message naming it.
func broken(_ which: String) {
	switch which {
	case "moved":
		let c = Counter.new("c", 0)
		_ = c.intoLabel(nil)
		_ = c.get()
	case "boxed moved":
		let tail = Node.new(1, nil)
		_ = Node.new(2, tail)
		_ = tail.sum()
	case "lent moved":
		_ = Registry.new(Counter.new("c", 0)).edit { counter in
			_ = counter.intoLabel(nil)
			return false
		}
	case "lent kept":
		let kept = Kept()
		_ = Registry.new(Counter.new("c", 0)).edit { counter in
			kept.counter = counter
			return false
		}
		_ = kept.counter!.get()
	// An IPv6 address where Rust takes `SocketAddrV4`.
	case "family":
		let any = SocketAddr(ip: Array(repeating: 0, count: 16), port: 1)
		_ = nexts(any, any)
	// No String holds a path that is not UTF-8.
	case "stray":
		_ = stray()
	default:
		check(false, "no case \(which)")
	}
}

@main
struct Main {
	static func main() {
		let args = CommandLine.arguments.dropFirst()
		switch args.first {
		case nil: script()
		case "futures": futures()
		case "owned": owned()
		case let which?: broken(which)
		}
	}
}
