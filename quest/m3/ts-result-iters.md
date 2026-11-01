# [S] Result's iterators

## Goal

[rust2-ts](/ts/README.md) translates `Result::iter`, `iter_mut`, and `into_iter`, by value
and through a reference, and `flatten` over `Result`s, for every `T`, `[u8; N]` lent as an
`rt.ByteSlice` as an `Option`'s is, proven by a differential test against native Rust.

## Plan

Found by [moq-dev/rust2#366](https://github.com/moq-dev/rust2/pull/366): `res.iter()` is
refused as "constructing `std::result::Iter` from another crate", and
`results.iter().flatten()` and `iter.chain(&res)` as unsupported calls. `iter::into_iter`
and `iter::call` map an `Option`'s iterators but not a `Result`'s.

Public API: the translator's output.
