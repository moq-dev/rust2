# [M] std's conversions of a reference

## Goal

[rust2-ts](/ts/README.md) translates std's functions that take a reference and convert,
copy, or measure what it points at, for any `T` and for `[u8; N]`, proven by differential
tests against native Rust: `ToOwned::to_owned` and `clone_into`, `Clone::clone_from`,
`slice::binary_search`, `mem::size_of_val` and `align_of_val`, `slice::from_mut`,
`array::from_ref` and `from_mut`, `AsRef`, `AsMut`, and `Borrow` of an array as a slice,
`Cell::from_mut`, and `ptr::from_ref` and `from_mut`.

## Plan

Found by [moq-dev/rust2#354](https://github.com/moq-dev/rust2/pull/354): each is refused as
"unsupported call" for `[u32; 2]` and `u32` as for `[u8; 2]`, having no mapping. A mapping
given a `&[u8; N]` as a `&T` gets an `rt.byteSliceView` of it from `lend::give`, which
shares the bytes, so reads, copies, and writes through it need nothing more. A pointer to
a `[u8; N]` is an `rt.ByteSlice` compared by the buffer and offset of its bytes, so
`ptr::from_ref` of the view is `rt.byteSliceFrom` of it.

No consumer calls them yet; they are planned here as part of std.

Public API: the translator's output.
