# [M] Python's call cost

## Goal

A call on an object through the exporter's [Python](/ffi/README.md) costs what
[BoltFFI's](/bench/ffi/README.md#boltffi-comparison) does, 63 ns, or `just ffi bench`
explains the gap.

## Plan

Found by [#407](https://github.com/moq-dev/rust2/pull/407), which measured about 125 ns on a
loaded host. Measure on a quiet one first, then profile where the extension spends the rest:
argument parsing, the handle's lookup, and the contract or GIL work per call.

Public API: none.
