# [S] Rc and Arc of a slice

## Goal

[rust2-ts](/ts/README.md) translates an `Rc<[T]>` and an `Arc<[T]>` made from a `Vec`, a
slice, or an iterator, read as the slice, cloned, and dropped, proven by a differential
test against native Rust at `u8`, at another element, and at one that drops.

## Plan

Found writing [a generic type's byte fields](https://github.com/moq-dev/rust2/pull/375)'
fixture: `Rc::from(vec)` and `Rc::from(slice)` are refused as unsupported calls to
`From`. A shared slice is what a generic type's `[T]` already is, so the form of a
`u8` instance is the open question: an `rt.ByteSlice` as a `Box<[u8]>` is, or one that
counts its owners as `rt.Arc` does.

Public API: the translator's output.
