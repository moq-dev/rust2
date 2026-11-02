# [S] Python's wake on the loop's own thread

## Goal

A future the exporter's [Python](/ffi/README.md) awaits resumes directly when Rust wakes it
on the event loop's own thread, skipping the socket pair, and `just ffi bench` measures the
woken read against [BoltFFI's](/bench/ffi/README.md#boltffi-comparison) 10 µs.

## Plan

Found by [#407](https://github.com/moq-dev/rust2/pull/407): a woken read costs about 21 µs,
about 5 of them the socket pair's write and read. A wake from another thread still goes
through it, and the wake-latch and cancel tests keep passing.

Public API: none.
