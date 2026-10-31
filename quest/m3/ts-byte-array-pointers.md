# [S] Raw pointers to arrays of bytes

## Goal

[rust2-ts](/ts/README.md) translates raw pointers to a `[u8; N]` made from a reference to
it with std's semantics, or refuses them with a span: `ptr::eq` of two pointers to one
array is `true`, and `ptr::swap`, `read`, and `write` move its bytes, proven by
differential tests against native Rust.

## Plan

Found by [moq-dev/rust2#354](https://github.com/moq-dev/rust2/pull/354): a `&[u8; N]` is an
`rt.ByteSlice`, and each reference to one array is its own object, so `ptr::eq(&a, &a)`
translates to `a === b` of two objects, `false`. `ptr::swap` copies through `.slice()`,
which an `rt.ByteSlice` lacks. Compare by `data` and `start`, and copy by
`rt.byteSliceToArray`, or refuse until [Unsafe code the translator can model](/quest/m3/ts-unsafe.md)
types pointers.

Public API: the translator's output.

## Related

- [Unsafe code the translator can model](/quest/m3/ts-unsafe.md) - the typed pointers these would be
