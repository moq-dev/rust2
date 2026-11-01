# [M] Borrows refused for a pointer no callee keeps there

## Goal

[rust2-ts](/ts/README.md) refuses a borrow only where a raw pointer taken through it
into an `UnsafeCell` may still be read after the place it points at gets another object,
so moq's census at the pinned moq revision holds none of the 3 `a reference to _, through
which a raw pointer may be made` refusals left in moq-net, nor the 10 `a reference
through which a raw pointer may be made into _ ...` ones. A translated program never
silently reads the old object where Rust defines the read.

## Plan

Decided 2026-10-31: only a part that holds an `UnsafeCell` counts as taken. A caller's
assignment disables a pointer made through a `&` or `&mut` to anything else, so reading
it after is undefined in both Stacked and Tree Borrows, and
[ts-unsafe](/quest/m3/ts-unsafe.md) owes undefined behavior nothing. Miri agreed on each
of #350's six cases and #355's two, which all read a struct with no cell after it was
assigned again: both models report the read as undefined. With cells in their place,
the same eight shapes are defined under Tree Borrows. Comparing such a pointer's address
after is defined, so a type still counts if the crate compares pointers to it or to what
it holds: each translation run notes them, and the next judges its borrows by them. That
cleared 184 of the 187 left after [#355](https://github.com/moq-dev/rust2/pull/355).

What remains: `rs/moq-clock/src/timer.rs:290` (`Option<moq_clock::Sleep>`) and the
`kio::Pending` futures at `rs/moq-net/src/ietf/publisher.rs:1860` and
`rs/moq-net/src/ietf/subscriber/namespaces.rs:81`. Each holds a `kio::Park`, whose
`Waiter` holds a `OnceLock` by value, and `Trace` loses the reference into std's
generic code, as #355 measured for the rest. Clearing them needs following that
reference to the callees that make a pointer into the cell, judged per instance, std's
bodies included, or proving each kept pointer is written again before any read.

[ts-pointer-part-replaced](https://github.com/moq-dev/rust2/pull/365) added the second
construct: a borrow of a part that the crate may give another object, here or in another
body. Its 10 in the census are false positives, none a real hazard:

- 8 borrow a `kio::Park` held by a future or task over a type parameter: the futures of
  `rs/moq-net/src/transport.rs` (`OpenBi`, `SendClosed`, `Write`, `WriteBuf`,
  `WriteChunk`, `ReadChunk`), and kio's `Pending<P>` (`rs/kio/src/pending.rs:56`) and
  `Occupant<T>` (`rs/kio/src/task.rs:405`). Each `Park` is pinned, and its drop
  unregisters its waiter before any whole write of what holds it.
- 1 borrows the `[AtomicU64; 16]` of kio's `task::Chunk` behind an `Arc`, which nothing
  writes (`rs/kio/src/task.rs:367`).
- 1 borrows a `Follow` in an `Option`'s payload that nothing writes again
  (`rs/moq-net/src/model/origin.rs:6455`), which the rule refuses for any payload.

Two causes besides the taken `OnceLock`: a census lowers these functions over their type
parameters, where any type with one, or any type a generic body's whole write of a `T`
may be, counts as replaced; and a payload is refused outright rather than judged by
whole writes of its enum.
