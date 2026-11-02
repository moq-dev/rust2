// The runtime every generated Swift file includes, after `Native` names the export's C
// types and functions: the handles objects own, the arena a call lowers its values into,
// bytes and text across the C ABI, and the signal a future parks on.

/// An IP socket address, as Rust's `SocketAddr` holds it.
public struct SocketAddr: Hashable, Sendable {
	/// The IP in network order: 4 bytes for IPv4, or 16 for IPv6.
	public var ip: [UInt8]
	public var port: UInt16
	/// IPv6's flow label, zero for IPv4.
	public var flowinfo: UInt32
	/// IPv6's scope, zero for IPv4.
	public var scopeId: UInt32

	public init(ip: [UInt8], port: UInt16, flowinfo: UInt32 = 0, scopeId: UInt32 = 0) {
		self.ip = ip
		self.port = port
		self.flowinfo = flowinfo
		self.scopeId = scopeId
	}
}

extension Native {
	/// Checked before any other call: a Swift file and a library from different builds
	/// must not mix.
	static let checked: Bool = {
		if !contract() {
			fatalError("\(file) and the library are from different builds")
		}
		return true
	}()

	@inline(__always)
	static func check() {
		_ = checked
	}

	/// A non-null pointer for empty bytes, which C reads as present, unlike a null one.
	nonisolated(unsafe) static let empty: UnsafeMutablePointer<UInt8> = {
		let empty = UnsafeMutablePointer<UInt8>.allocate(capacity: 1)
		empty.initialize(to: 0)
		return empty
	}()

	static var none: Buf { Buf(ptr: nil, len: 0, owner: nil) }

	/// `bytes` as a view C reads during `body`, without a copy.
	@inline(__always)
	static func view<R>(_ bytes: Data, _ body: (Buf) throws -> R) rethrows -> R {
		try bytes.withUnsafeBytes { raw in
			let ptr = raw.baseAddress?.assumingMemoryBound(to: UInt8.self) ?? UnsafePointer(empty)
			return try body(Buf(ptr: ptr, len: raw.count, owner: nil))
		}
	}

	@inline(__always)
	static func view<R>(_ bytes: Data?, _ body: (Buf) throws -> R) rethrows -> R {
		guard let bytes else { return try body(none) }
		return try view(bytes, body)
	}

	/// `text`'s UTF-8 as a view C reads during `body`, without a copy for a native string.
	@inline(__always)
	static func view<R>(_ text: Swift.String, _ body: (Buf) throws -> R) rethrows -> R {
		var text = text
		return try text.withUTF8 { utf8 in
			try body(Buf(ptr: utf8.baseAddress ?? UnsafePointer(empty), len: utf8.count, owner: nil))
		}
	}

	@inline(__always)
	static func view<R>(_ text: Swift.String?, _ body: (Buf) throws -> R) rethrows -> R {
		guard let text else { return try body(none) }
		return try view(text, body)
	}

	/// Bytes Rust returned, lent to `Data` without a copy and freed with it.
	static func data(_ buf: Buf) -> Data {
		guard let ptr = buf.ptr, buf.len > 0 else {
			free(buf)
			return Data()
		}
		return Data(
			bytesNoCopy: UnsafeMutableRawPointer(mutating: ptr), count: buf.len,
			deallocator: .custom { _, _ in free(buf) })
	}

	static func data(option buf: Buf) -> Data? {
		buf.ptr == nil ? nil : data(buf)
	}

	/// Bytes C lends a callback for its call, copied.
	static func data(lent buf: Buf) -> Data {
		buf.len == 0 ? Data() : Data(bytes: buf.ptr!, count: buf.len)
	}

	static func data(lentOption buf: Buf) -> Data? {
		buf.ptr == nil ? nil : data(lent: buf)
	}

	/// Text Rust returned, copied, then freed.
	static func string(_ buf: Buf) -> Swift.String {
		defer { free(buf) }
		return string(lent: buf)
	}

	static func string(option buf: Buf) -> Swift.String? {
		buf.ptr == nil ? nil : string(buf)
	}

	/// Text C lends a callback for its call, which Rust made UTF-8, copied.
	static func string(lent buf: Buf) -> Swift.String {
		Swift.String(decoding: UnsafeBufferPointer(start: buf.ptr, count: buf.len), as: UTF8.self)
	}

	static func string(lentOption buf: Buf) -> Swift.String? {
		buf.ptr == nil ? nil : string(lent: buf)
	}

	/// A path Rust returned, copied, then freed.
	static func path(_ buf: Buf) -> Swift.String {
		defer { free(buf) }
		return path(lent: buf)
	}

	static func path(option buf: Buf) -> Swift.String? {
		buf.ptr == nil ? nil : path(buf)
	}

	/// A path from Rust, copied, which on Unix may hold bytes no `String` holds: those trap.
	static func path(lent buf: Buf) -> Swift.String {
		var bytes = UnsafeBufferPointer(start: buf.ptr, count: buf.len).makeIterator()
		var parser = Unicode.UTF8.ForwardParser()
		while true {
			switch parser.parseScalar(from: &bytes) {
			case .valid: continue
			case .emptyInput: return string(lent: buf)
			case .error: fatalError("a path from Rust is not UTF-8")
			}
		}
	}

	static func path(lentOption buf: Buf) -> Swift.String? {
		buf.ptr == nil ? nil : path(lent: buf)
	}

	static func scalar(_ code: UInt32) -> Unicode.Scalar {
		guard let scalar = Unicode.Scalar(code) else {
			fatalError("a char is not a Unicode scalar")
		}
		return scalar
	}

	/// Rust's longest `Duration`: `u64::MAX` seconds and 999,999,999 nanoseconds.
	static var longest: Swift.Duration {
		.seconds(Int64.max) * 2 + .seconds(1) + .nanoseconds(999_999_999)
	}

	static func duration(_ value: Swift.Duration) -> CDuration {
		if value < .zero {
			fatalError("a negative duration has no Rust Duration")
		}
		if value > longest {
			fatalError("a duration past Rust's Duration::MAX")
		}
		// In halves, since `components` gives Int64's seconds, and Rust's are UInt64's.
		let half = value / 2
		let (secs, attos) = half.components
		let odd = (value - half * 2).components.attoseconds
		var nanos = (attos * 2 + odd) / 1_000_000_000
		var whole = UInt64(secs) * 2
		if nanos >= 1_000_000_000 {
			nanos -= 1_000_000_000
			whole += 1
		}
		return CDuration(secs: whole, nanos: UInt32(nanos))
	}

	static func duration(_ value: CDuration) -> Swift.Duration {
		// Past Int64's seconds, as Rust's may be, in halves.
		let half = Swift.Duration.seconds(Int64(value.secs >> 1))
		return half * 2 + .seconds(Int64(value.secs & 1)) + .nanoseconds(Int64(value.nanos))
	}

	static func socketAddr(_ addr: SocketAddr) -> CAddr {
		let v6: Bool
		switch addr.ip.count {
		case 4: v6 = false
		case 16: v6 = true
		default: fatalError("an IP address is 4 bytes or 16, not \(addr.ip.count)")
		}
		var c = CAddr()
		withUnsafeMutableBytes(of: &c.ip) { ip in ip.copyBytes(from: addr.ip) }
		c.port = addr.port
		c.v6 = v6
		c.flowinfo = addr.flowinfo
		c.scope_id = addr.scopeId
		return c
	}

	static func socketAddr(_ c: CAddr) -> SocketAddr {
		var c = c
		let ip = withUnsafeBytes(of: &c.ip) { ip in Array(ip.prefix(c.v6 ? 16 : 4)) }
		return SocketAddr(ip: ip, port: c.port, flowinfo: c.flowinfo, scopeId: c.scope_id)
	}

	/// The C array Rust returned, as its items each lifted by `lift`, which takes what
	/// each holds; `free` then frees the array alone.
	static func list<C, T>(_ ptr: UnsafePointer<C>?, _ len: Int, _ lift: (C) throws -> T) rethrows -> [T] {
		guard let ptr else { return [] }
		return try UnsafeBufferPointer(start: ptr, count: len).map(lift)
	}

	/// A handle an object owns, or is lent for a call: each call holds it, so `close`
	/// frees it only once no call does, and a call after `close` traps.
	struct Handle {
		let raw: OpaquePointer?
		/// The holds, one for the owner until closed and one per call, shifted past a bit
		/// set once closed.
		let state: UnsafeMutablePointer<Int>
		/// Null for a handle Rust lends, which it frees itself.
		let free: (@convention(c) (OpaquePointer?) -> Void)?
		let type: StaticString

		static let closed = 1
		static let held = 2

		init(_ raw: OpaquePointer?, _ free: (@convention(c) (OpaquePointer?) -> Void)?, _ type: StaticString) {
			self.raw = raw
			self.free = free
			self.type = type
			state = UnsafeMutablePointer<Int>.allocate(capacity: 1)
			state.initialize(to: Handle.held)
		}

		func hold() -> OpaquePointer? {
			var n = load(state)
			while true {
				if n & Handle.closed != 0 {
					fatalError("\(type) is closed")
				}
				let seen = swap(state, n, n + Handle.held)
				if seen == n {
					return raw
				}
				n = seen
			}
		}

		func release() {
			if add(state, -Handle.held) < Handle.held {
				free?(raw)
			}
		}

		/// Run `body` with the raw handle, held until it returns.
		@inline(__always)
		func with<R>(_ body: (OpaquePointer?) throws -> R) rethrows -> R {
			let raw = hold()
			defer { release() }
			return try body(raw)
		}

		func close() {
			var n = load(state)
			while n & Handle.closed == 0 {
				let seen = swap(state, n, (n | Handle.closed) - Handle.held)
				if seen == n {
					if (n | Handle.closed) - Handle.held < Handle.held {
						free?(raw)
					}
					return
				}
				n = seen
			}
		}

		/// Close it, for its owner's `deinit`, when no call can hold it.
		func drop() {
			close()
			state.deallocate()
		}
	}

	/// Run `body` with the raw handle of `handle`, or null for none, held until it returns.
	@inline(__always)
	static func with<R>(_ handle: Handle?, _ body: (OpaquePointer?) throws -> R) rethrows -> R {
		guard let handle else { return try body(nil) }
		return try handle.with(body)
	}

	/// What a call lowers nested values into, kept until it returns: copied bytes, arrays,
	/// and the handles it holds.
	final class Arena {
		private var blocks: [UnsafeMutableRawPointer] = []
		private var held: [Handle] = []

		func copy(_ bytes: UnsafeRawBufferPointer) -> Buf {
			guard bytes.count > 0 else {
				return Buf(ptr: UnsafePointer(empty), len: 0, owner: nil)
			}
			let block = UnsafeMutableRawPointer.allocate(byteCount: bytes.count, alignment: 1)
			block.copyMemory(from: bytes.baseAddress!, byteCount: bytes.count)
			blocks.append(block)
			return Buf(ptr: block.assumingMemoryBound(to: UInt8.self), len: bytes.count, owner: nil)
		}

		func buf(_ bytes: Data) -> Buf {
			bytes.withUnsafeBytes { copy($0) }
		}

		func buf(_ text: Swift.String) -> Buf {
			var text = text
			return text.withUTF8 { copy(UnsafeRawBufferPointer($0)) }
		}

		func buf(_ bytes: Data?) -> Buf {
			bytes.map { buf($0) } ?? none
		}

		func buf(_ text: Swift.String?) -> Buf {
			text.map { buf($0) } ?? none
		}

		func hold(_ handle: Handle) -> OpaquePointer? {
			let raw = handle.hold()
			held.append(handle)
			return raw
		}

		func hold(_ handle: Handle?) -> OpaquePointer? {
			handle.map { hold($0) } ?? nil
		}

		/// `items` as a C array, each lowered by `lower`: never null, so an empty one is
		/// told apart from a missing one.
		func list<T, C>(_ items: [T], _ lower: (T) throws -> C) rethrows -> (UnsafePointer<C>, Int) {
			let array = UnsafeMutablePointer<C>.allocate(capacity: Swift.max(items.count, 1))
			blocks.append(UnsafeMutableRawPointer(array))
			for (i, item) in items.enumerated() {
				(array + i).initialize(to: try lower(item))
			}
			return (UnsafePointer(array), items.count)
		}

		/// Release what the call held, once it returns.
		func finish() {
			for handle in held {
				handle.release()
			}
			for block in blocks {
				block.deallocate()
			}
		}
	}

	/// An array C passes, lowered into `arena`.
	@inline(__always)
	static func vec<C, V>(
		_ list: (UnsafePointer<C>, Int), _ make: (UnsafePointer<C>?, Int, UnsafeMutableRawPointer?) -> V
	) -> V {
		make(list.0, list.1, nil)
	}

	/// What Rust holds of a Swift value: a callback's implementation, or a waker's closure.
	final class Box<T> {
		let value: T

		init(_ value: T) {
			self.value = value
		}

		static func unbox(_ data: UnsafeMutableRawPointer?) -> T {
			Unmanaged<Box<T>>.fromOpaque(data!).takeUnretainedValue().value
		}

		static func release(_ data: UnsafeMutableRawPointer?) {
			Unmanaged<Box<T>>.fromOpaque(data!).release()
		}

		/// `value`, retained until Rust releases it.
		static func give(_ value: T) -> UnsafeMutableRawPointer {
			Unmanaged.passRetained(Box(value)).toOpaque()
		}
	}

	/// `wake` as a waker Rust holds until it drops it, calling it from any thread.
	static func waker(_ wake: @escaping @Sendable () -> Void) -> CWaker {
		CWaker(
			wake: { data in Box<@Sendable () -> Void>.unbox(data)() },
			drop: { data in Box<@Sendable () -> Void>.release(data) },
			data: Box.give(wake))
	}

	/// The waker of a future's first poll, which wakes nothing: a poll that pends polls
	/// again with its signal's.
	static var idle: CWaker { CWaker(wake: nil, drop: nil, data: nil) }

	/// What a future parks on: it latches a wake that lands between a poll that pends and
	/// the park, so the next poll sees it.
	final class Signal: @unchecked Sendable {
		static let idle = 0
		static let woken = 1
		static let parked = 2

		/// Which of those it is, changed only atomically.
		private let state = UnsafeMutablePointer<Swift.Int>.allocate(capacity: 1)
		/// The parked task, written before `state` is `parked` and read after.
		private var parked: UnsafeContinuation<Void, Never>?

		init() {
			state.initialize(to: Signal.idle)
		}

		deinit {
			state.deallocate()
		}

		/// This as a waker: given, holding a reference Rust drops, the first time; lent,
		/// which Rust reuses as the same waker, after.
		func waker(given: Bool) -> CWaker {
			let data = given ? Unmanaged.passRetained(self).toOpaque() : Unmanaged.passUnretained(self).toOpaque()
			return CWaker(
				wake: { data in Unmanaged<Signal>.fromOpaque(data!).takeUnretainedValue().wake() },
				drop: { data in Unmanaged<Signal>.fromOpaque(data!).release() },
				data: data)
		}

		/// Resume the parked task, or else latch the wake for the next park. The task
		/// resumes on its own executor, never inside the wake.
		func wake() {
			if exchange(state, Signal.woken) == Signal.parked {
				parked!.resume()
			}
		}

		/// Forget any wake before the next poll, which sees whatever it was for.
		func reset() {
			store(state, Signal.idle)
		}

		/// Suspend until a wake since `reset`, at once if one already landed.
		func park() async {
			await withUnsafeContinuation { (continuation: UnsafeContinuation<Void, Never>) in
				parked = continuation
				if swap(state, Signal.idle, Signal.parked) != Signal.idle {
					continuation.resume()
				}
			}
		}
	}

	/// Await the future `raw`: poll it until `poll` returns its output, then free it. A
	/// cancelled task frees it unfinished, which cancels it in Rust and loses nothing, and
	/// throws `CancellationError`. Its first poll makes no signal, which only a future
	/// that pends needs.
	static func run<T>(
		_ raw: OpaquePointer?, _ free: @convention(c) (OpaquePointer?) -> Void,
		_ poll: (OpaquePointer?, CWaker) throws -> T?
	) async throws -> T {
		defer { free(raw) }
		// A task cancelled already takes nothing, even what is ready.
		try Task.checkCancellation()
		if let output = try poll(raw, idle) {
			return output
		}
		let signal = Signal()
		var given = false
		return try await withTaskCancellationHandler {
			while true {
				signal.reset()
				try Task.checkCancellation()
				if let output = try poll(raw, signal.waker(given: !given)) {
					return output
				}
				given = true
				await signal.park()
			}
		} onCancel: {
			signal.wake()
		}
	}
}
