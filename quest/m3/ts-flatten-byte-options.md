# [S] Arrays of bytes a flattened Option reference lends

## Goal

[rust2-ts](/ts/README.md) translates `options.iter().flatten()` over a `&[Option<[u8; N]>]`
to an iterator of `rt.ByteSlice`s, as translated code reads each `&[u8; N]`, proven by a
differential test against native Rust.

## Plan

Found by [moq-dev/rust2#354](https://github.com/moq-dev/rust2/pull/354): the output types
the iterator as `rt.Iter<rt.ByteSlice>` but yields the `Uint8Array`s the slice holds, which
`tsc` refuses. `lend::lends` reaches an iterator's items through std's `next`, but
`Flatten`'s inner iterator here is `option::Iter`, made of each `&Option<[u8; N]>` by
`IntoIterator`, which the lend does not follow.

Public API: the translator's output.
