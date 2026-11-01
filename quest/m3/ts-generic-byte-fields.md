# [S] A generic type's bytes at u8

## Goal

[rust2-ts](/ts/README.md) types a `[T; N]`, `Vec<T>`, or `&[T]` field of a generic type as
its `u8` instance holds it, or refuses that instance with a span, proven by a differential
test against native Rust of one such type instantiated at `u8` and at another element.

## Plan

Found beside [the generic pointer fields](https://github.com/moq-dev/rust2/pull/371): the
definition types each field `T[]`, but an instance at `u8` stores a `Uint8Array`, an
`rt.ByteVec`, or an `rt.ByteSlice`, so `tsc` rejects the instance's code while
translation succeeds. A field whose definition types a pointer as an `rt.Ptr` is
already read as its instance's type through a cast; a union of the element array and
the byte form, cast on read the same way, may fit, as may refusing the `u8` instance as
`generic_erasure` refuses an `Option` of a type parameter that can be `undefined`.

Public API: the translator's output.
