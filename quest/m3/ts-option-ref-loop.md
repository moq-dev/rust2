# [S] An Option a loop borrows, tested as one

## Goal

[rust2-ts](/ts/README.md) translates `if let Some(x) = item` of an `&Option<T>` a `for`
loop yields, as `for a in options { if let Some(a) = a { .. } }` over a `&[Option<T>]`
does, testing the item's `None`, proven by a differential test against native Rust.

## Plan

Found by [moq-dev/rust2#354](https://github.com/moq-dev/rust2/pull/354): the output tests
`_5 !== rt.DONE`, the iterator protocol's end, where it should test the `Option`'s `None`,
for `Option<u32>` and `Option<[u8; N]>` alike. `tsc` refuses it today (TS2367); an
`Option` whose `None` is `rt.None` may not be caught. The loop's `next` folding likely
takes the item's own `Option` test for the protocol's.

Public API: the translator's output.
