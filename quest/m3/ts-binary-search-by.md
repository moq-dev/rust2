# [XS] A slice's searches by a closure

## Goal

[rust2-ts](/ts/README.md) translates `slice::binary_search_by`, `binary_search_by_key`,
and `partition_point`, calling the closure on the elements std's search probes, in its
order, proven by a differential test against native Rust over runs of equal elements.

## Plan

Found writing [std's conversions of a reference](https://github.com/moq-dev/rust2/pull/386),
which maps `binary_search` to the runtime's `sliceBinarySearch`, std's probe sequence. The
others are std's same search by another comparison, so they can share it. They are refused
as "unsupported call" today.

Public API: the translator's output.
