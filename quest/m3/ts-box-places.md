# [S] A &mut into a Box of a value JS copies

## Goal

[rust2-ts](/ts/README.md) translates a `&mut` to what a `Box` holds, where JS copies it (a
number, a `String`, or a `str`), as a write to the local or field holding the box. Today
`*(&mut *b) += 1` of a `Box<u32>`, `push` on a `Box<String>`, and `make_ascii_uppercase`
on a `Box<str>` translate to `b.v` of a value with no `v`, which `tsc` rejects. Proven by
differential tests against native Rust.

## Plan

Found translating `&mut str`'s places. A `Box<T>` is its `T`, so a `&mut` through it is
the box's own place, as `String::as_mut_str` borrows the `String`'s.

Public API: the translator's output.
