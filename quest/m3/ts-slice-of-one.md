# [S] A slice of one value behind any &mut

## Goal

[rust2-ts](/ts/README.md) translates `slice::from_mut` and `array::from_mut` of every
`&mut`, proven by a differential test against native Rust that writes the element whole,
reads it out with `mem::replace`, and writes it in place: a `&mut u8`, and a `&mut` to an
object, such as a struct or a `[u32; N]`, that names no place in the function, as a
parameter does.

## Plan

Found writing [std's conversions of a reference](https://github.com/moq-dev/rust2/pull/386):
both are refused. A `&mut [u8]` is an `rt.ByteSlice`, which can only view a `Uint8Array`,
not the `rt.Place` of one `u8`. A slice's element write replaces the slot, so a slice of
an object whose place is unknown can't make the caller's variable hold the new value, and
copying the write into the object instead makes `mem::replace` return the object it just
overwrote. Either needs translated code to write such a slice's element in place, or a
`&mut u8` to know the bytes it may point into.

Public API: the translator's output.
