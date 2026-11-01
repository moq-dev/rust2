# [S] Raw pointers to a type parameter in a struct

## Goal

[rust2-ts](/ts/README.md) types a `*mut T` or `*const T` field of a generic struct as
each instance stores it, or refuses the struct with a span, proven by a differential
test against native Rust of `struct Window<T> { at: *mut T }` instantiated with a
struct and with a `u8`.

## Plan

Found by lang's `pointers::reattached` case: the
interface declares `at: rt.Place<T>`, because `pointee` of a type parameter outside an
instance is a `Place`, but `Window<Reattached>`'s code stores the object itself, so
`tsc` rejects the assignment and `(*w.at).0`. Either type the field by a conditional
type that picks `T` for an object and `rt.Place<T>` otherwise, or refuse a raw pointer
to a type parameter in a field until [Unsafe code the translator can model](/quest/m3/ts-unsafe.md)
types pointers.

Public API: the translator's output.
