# [XS] std::iter::zip

## Goal

[rust2-ts](/ts/README.md) translates `std::iter::zip(a, b)`, including of a `&mut` to a
collection of values JS copies, as `a.into_iter().zip(b)` does, proven by a differential
test against native Rust.

## Plan

Found writing [places through an adaptor's argument](https://github.com/moq-dev/rust2/pull/391):
the free function is an "unsupported call", while `Iterator::zip` maps.

Public API: the translator's output.
