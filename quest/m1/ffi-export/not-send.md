# [L] Types that are not Send export

## Goal

[rust2-ffi](/ffi/README.md) exports an object or future that is not `Send`, bound to the
thread that made it: a call, poll, or drop from another thread never runs Rust code on
that value there. Three of moq's own are refused for it at moq `ffcb7cd9`: `clock::Entered`,
`clock::Rng`, and `catalog::Guard`
([#675](https://github.com/kixel-ai/rust2/pull/675)). At moq's `main` (`a11a584`, measured on 2026-11-14 in
[#802](https://github.com/kixel-ai/rust2/pull/802)), moq-tokio's `server::Listener::accept_with`
is a fourth: its future borrows `&self` of a `Listener` that is not `Sync`, through a
`FuturesUnordered` of boxed futures that are not, so the future is not `Send`. Proven per language by a fixture
type holding an `Rc`, called, awaited, and dropped from a second thread.

## Plan

Decided with the maintainer on 2026-11-09, in the quest audit: ✅ a thread check, with a
shim only where a language has no stable thread. Rejected: a Rust-owned thread for every
such object, which costs two thread handoffs a call even from a single-threaded caller
such as Python's event loop; keeping the refusal, as the
small-shapes quest had it. The maintainer:
an assert is fine "where a language has no concept of something being thread local".

- The handle records its owner thread, and each call compares it: no atomics, so it costs
  less than a `Sync` type's borrow. The wrong thread
  [raises](/quest/m1/ffi-export/checks-raise.md).
- A free from another thread never drops the value there. GC finalizers run elsewhere:
  Kotlin's phantom-reference daemon thread, Go's `runtime.AddCleanup` goroutine, Python's
  collector, Swift's `deinit` on the releasing thread, and Dart's finalizer with no
  current isolate. The free queues to the owner, which drains it at its next call, and a
  queue must not wait on a call that never comes: where the binding has an event loop on
  the owner thread (Python's loop, a C++ executor), queueing a free schedules a drain
  there. Open: what drains it elsewhere, and once the owner thread has exited.
  Recommended: drop the queue from the thread's exit hook (a thread-local's destructor),
  and leak with a logged message only where the thread is already gone, as PyO3's
  `unsendable` does.
- Where the language can say it, its compiler enforces the rule instead: a Swift object
  that is `Send` but not `Sync` is not `Sendable` (today every object is
  `@unchecked Sendable`, `ffi/src/swift.rs`). Confinement is not thread affinity, though:
  a Swift actor's jobs run on any thread of its pool, and a Dart isolate need not keep its
  OS thread, while Rust cannot tell an `Rc` from a guard pinned to its thread, as
  [#675](https://github.com/kixel-ai/rust2/pull/675) calls moq's three. So a `!Send` value needs the check or the owner thread below in
  Swift and Dart too, unless an executor pinned to the owner thread holds it; test a call
  after a suspension, and through Dart's relay.
- Where a language has no stable thread, a Rust-owned thread holds the value and runs
  its calls, two handoffs each: Go, whose goroutines move between OS threads, and a
  `!Send` future awaited on a pool, as Kotlin's dispatchers, Swift's executors, and Go's
  goroutines poll it. The future is spawned on the owner thread and the binding polls a
  `Send` handle to it. Dart's relay (`ffi/runtime/dart.rs`'s `Home`) is the same
  machinery in the other direction.
- wasm32 has one thread, so [rust2-wasm](/quest/m2/wasm/README.md) exports these with no
  check.

Public API: the items that now export, and Swift's `Sendable` conformances (breaking).

## Required

- [Failed checks raise](/quest/m1/ffi-export/checks-raise.md) - the wrong-thread check raises through the same accounting
