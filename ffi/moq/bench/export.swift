// bench/export.cpp's frame writes and reads through the generated Swift, so the two differ
// by what the Swift wrapper adds.

import Foundation
import Moq

let groups = 10000
let frames = 100

/// Nanoseconds per frame since `start`.
func perFrame(_ start: UInt64) -> Double {
	Double(DispatchTime.now().uptimeNanoseconds - start) / Double(groups * frames)
}

/// A value a task sets and this thread reads once the task signals.
final class Cell<T>: @unchecked Sendable {
	var value: T?
}

/// `body`'s result, waiting on this thread for it to finish on the global executor.
func block<T: Sendable>(_ body: @escaping @Sendable () async throws -> T) -> T {
	let done = DispatchSemaphore(value: 0)
	let result = Cell<T>()
	Task {
		result.value = try! await body()
		done.signal()
	}
	done.wait()
	return result.value!
}

func round() throws -> String {
	let broadcast = BroadcastInfo.new().produce()
	let track = try broadcast.createTrack("bench", TrackInfo.default().withTimescale(Timescale.MICRO))

	// A frame of 100 zero bytes, as bench/export.c writes.
	let frame = Data(count: 100)
	var micros: UInt64 = 0

	// Each frame stamped through a fresh timestamp handle, as a caller holding a u64 must.
	var start = DispatchTime.now().uptimeNanoseconds
	for _ in 0..<groups {
		let group = try track.appendGroup()
		for _ in 0..<frames {
			let at = try Timestamp.fromMicros(micros)
			micros += 1
			try group.writeFrame(at, frame)
			at.close()
		}
		try group.finish()
		group.close()
	}
	let stamped = perFrame(start)

	// The same, reusing one timestamp handle: what the handle costs.
	let at = try Timestamp.fromMicros(micros)
	start = DispatchTime.now().uptimeNanoseconds
	for _ in 0..<groups {
		let group = try track.appendGroup()
		for _ in 0..<frames {
			try group.writeFrame(at, frame)
		}
		try group.finish()
		group.close()
	}
	let reused = perFrame(start)

	// The floor: a hold of a handle and a bool back.
	start = DispatchTime.now().uptimeNanoseconds
	var closed = 0
	for _ in 0..<groups * frames where track.isClosed() {
		closed += 1
	}
	let call = perFrame(start)

	// Reads: each frame written, then read through an async func whose future is ready at
	// its first poll, so it never suspends.
	let (read, last) = block {
		let subscriber = try await track.consume().subscribe(nil)
		let start = DispatchTime.now().uptimeNanoseconds
		var last: FrameFrame?
		for _ in 0..<groups {
			let group = try track.appendGroup()
			let reader = try await subscriber.recvGroup()!
			for _ in 0..<frames {
				try group.writeFrame(at, frame)
				let read = try await reader.readFrame()!
				last?.close()
				last = read
			}
			reader.close()
			try group.finish()
			group.close()
		}
		subscriber.close()
		return (perFrame(start), last!)
	}

	// Payload reads: the last frame's 100 bytes, lent without a copy, then freed.
	start = DispatchTime.now().uptimeNanoseconds
	for _ in 0..<groups * frames {
		closed += Int(last.payload[0])
	}
	let lent = perFrame(start)
	last.close()
	at.close()
	track.close()
	broadcast.close()

	let bang = closed != 0 ? "!" : ""
	return String(
		format: "export (Swift): %.0f ns per stamped 100 B frame, %.0f reusing the timestamp, %.1f ns per handle call, %.0f ns per frame written and read, %.0f ns per payload read%@",
		stamped, reused, call, read, lent, bang)
}

print(try round())
