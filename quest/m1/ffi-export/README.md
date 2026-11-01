# Exporter: moq's bindings from its crates' pub API

## Goal

[rust2-ffi](/ffi/README.md) replaces [the UniFFI fork](/uniffi/README.md) for moq: it exports
what moq's bindings need from moq's own crates, with no attributes and no hand-written wrapper
beyond what the census shows is genuinely new API, and generates the languages moq ships on its
C ABI. moq-ffi shrinks to that remainder. The fork stays in maintenance, building moq's bindings
until then.

## Plan

Decided with the maintainer on 2026-10-25, from the spike
([moq-dev/rust2#41](https://github.com/moq-dev/rust2/pull/41)): adopt the exporter, and fold
the fork's paused feature quests into it.

Decided with the maintainer on 2026-10-27: this is an epic, with each item a child ranked by
what the census refuses and what moq waits on first, then the runtime, then the generators.
What stays here is the end-to-end: moq-ffi shrinks to its remainder and takes the stand-in
package's place.

Every generator on the C ABI:

- checks `<prefix>_contract()` against the header's contract when it loads, before any
  other call, and fails loud on a mismatch;
- is measured on the [binding benchmark](/bench/ffi/README.md);
- keeps the C ABI's bulk and zero-copy payloads, with no per-byte loop on either side;
- reports its generated size, raw and gzipped, against the fork's (Python 653 KiB, Kotlin
  914 KiB, Swift 684 KiB for moq-ffi);
- wraps the C ABI's futures in the language's native future. Those futures already take the
  result in the poll that finishes them, so a cancel never loses a frame, cost three crossings
  when ready at once, and reuse a repeated waker. The native future must latch a wake that
  lands between `PENDING` and parking, prove no loss on cancel in a test that forces that
  window, and pick a semantic for concurrent pollers. A hot read still makes a future handle
  per read; measure it before adding a per-handle poll. Decided with the maintainer on
  2026-10-27: async lands with each generator, not as its own quest.
- drives each of moq's `Driver`s from the language's own event loop. Decided with the
  maintainer on 2026-10-27: the C ABI exports the poll, with no generated thread and no
  moq-tokio helpers, which would tie bindings to tokio. A binding holds one `kio::Waiter`
  per driver, made from its waker, since what a poll registers lives only as long as its
  waiter; it polls on each wake and at the deadline the poll returns, and frees the driver
  to cancel it. Decided with the maintainer on 2026-10-27, for Go: a cancel ends only that
  read and leaves the stream usable, as in Rust, since a cancelled C ABI future loses
  nothing; moq's `go/wrapper/doc.go`, which ends the stream, changes with the generator.
  Each driver runs on a goroutine that lives as long as its object, and a call's `ctx`
  bounds only that call.

## Required

- [moq-clock's C names once it moves into kio](/quest/m1/ffi-export/crates.md) - `moq_now` and `moq_origin` become the clock's, after moq moves it into `kio::clock`
- [Sessions on sans-IO QUIC](/quest/m1/ffi-export/session-quic.md) - bindings move UDP datagrams and drive the session's poll themselves, over moq's runtime-free QUIC driver once it lands
- [C++ bindings on MSVC](/quest/m1/ffi-export/cpp.md) - the nightly's MSVC job passes the C++ checks
- [Kotlin bindings on Android in the nightly](/quest/m1/ffi-export/kotlin-android.md) - the nightly's emulator job passes the Kotlin test, shrunk by R8, at API 24 and 35
