# [M] Borrows refused for a pointer no callee keeps there

## Goal

[rust2-ts](/ts/README.md) refuses a borrow of a variable assigned again only where a
raw pointer taken through that borrow into an `UnsafeCell` may still be read, so moq's
census holds none of the 3 `a reference to _, through which a raw pointer may be made`
refusals left in moq-net at the pinned moq revision. A translated program never silently
reads the old object where Rust defines the read.

## Plan

Decided 2026-10-31: only a part that holds an `UnsafeCell` counts as taken. A caller's
assignment disables a pointer made through a `&` or `&mut` to anything else, so reading
it after is undefined in both Stacked and Tree Borrows, and
[ts-unsafe](/quest/m3/ts-unsafe.md) owes undefined behavior nothing. Miri agreed on each
of #350's six cases and #355's two, which all read a struct with no cell after it was
assigned again: both models report the read as undefined. With cells in their place,
the same eight shapes are defined under Tree Borrows. Comparing such a pointer's address
after is defined, so a type whose pointers the crate compares still counts: each
translation run notes them, and the next judges its borrows by them. That cleared 184 of
the 187 left after [#355](https://github.com/moq-dev/rust2/pull/355).

What remains: `rs/moq-clock/src/timer.rs:290` (`Option<moq_clock::Sleep>`) and the
`kio::Pending` futures at `rs/moq-net/src/ietf/publisher.rs:1860` and
`rs/moq-net/src/ietf/subscriber/namespaces.rs:81`. Each holds a `kio::Park`, whose
`Waiter` holds a `OnceLock` by value, and `Trace` loses the reference into std's
generic code, as #355 measured for the rest. Clearing them needs following that
reference to the callees that make a pointer into the cell, judged per instance, std's
bodies included, or proving each kept pointer is written again before any read.
