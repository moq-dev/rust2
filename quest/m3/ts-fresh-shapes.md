# [M] Fresh writes through a zip, a copy, and a repeat

## Goal

[rust2-ts](/ts/README.md) translates three shapes it refuses for concrete types and type
parameters alike, proven by differential tests against native Rust:

- zipping a slot with a `Vec<[T; N]>`, as `[u8; N]`;
- reading a `T` back out of `Zip::next`'s result;
- `iter().copied()` and `vec![x; n]` over a `Copy` type, a crate's own or a `T: Copy`.

## Plan

Found by [#394](https://github.com/moq-dev/rust2/pull/394), which made a whole write into
fresh memory count as fresh in a function generic over a type. These three stay refused for
reasons that have nothing to do with type parameters:

- `vec::IntoIter` points at a type the slot holds in place, so zipping with it refuses.
- Reading a `T` out of `Zip::next`'s result makes it a holder. A concrete control
  (`Vec<Option<u8>>`) is refused as well.
- `Copy`'s supertrait `Clone` selects a parameter candidate, so `copied` and `vec![x; n]`
  count as running the crate's `Clone`. The two differ: `copied` copies and calls no
  `clone`, while `vec![x; n]` calls `clone` `n - 1` times, and a `Copy` type's hand-written
  `Clone` may have effects or panic. Treat `copied` as running no crate code; keep
  `vec![x; n]` calling the crate's `clone`, judged fresh only where that `clone` writes
  nothing the slot holds. Cover both with differential tests whose `Clone` logs and panics.

Reproduce each first and fix the one that is a single cause before the others.

Public API: the translator's output.
