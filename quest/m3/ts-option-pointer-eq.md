# [XS] == of an Option of NonNull

## Goal

[rust2-ts](/ts/README.md) translates `==` of an `Option<NonNull<T>>`, proven by a
differential test against native Rust for pointers to an object and to a `[u8; N]`,
equal and apart, and `None`.

## Plan

Found writing [the generic pointer fields](https://github.com/moq-dev/rust2/pull/371)'
fixture: `a.maybe == Some(a.held)` is refused as an unsupported call to `Option`'s
`PartialEq::eq`, while `matches!(a.maybe, Some(p) if p == a.held)` translates. Pointers
to a value JS copies stay refused, as their `==` is.

Public API: the translator's output.
