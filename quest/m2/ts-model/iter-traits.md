# [M] A crate's own double-ended, exact-size, and fused iterators

## Goal

[rust2-ts](/ts/README.md) translates a crate's own iterator that implements
`DoubleEndedIterator`, `ExactSizeIterator`, or `FusedIterator`, or overrides `size_hint`, as
hashlink's do, so std's adaptors and collectors call them where native's does, proven by
differential tests against native Rust.

## Plan

Found by [#398](https://github.com/moq-dev/rust2/pull/398): once hashbrown translated, 39 of
the model's census refusals were in these impls on hashlink's iterators, such as
`LinkedHashMap`'s `Iter`, `Keys`, and `Drain`. They predate #398.

- Read the refusals from `just ts census` on `ts/moq/model.toml` first, and close each kind.
- `next_back`, `len`, and an overridden `size_hint` are the crate's methods, called wherever
  std's adaptors and collectors call them, `rev`, `last`, a `collect`'s reserve, and the rest,
  and nowhere they don't.
- `FusedIterator` is only a marker, but `fuse` skips its guard for one, which a differential
  test with an iterator that yields again after `None` shows.

Public API: the translator's output.
