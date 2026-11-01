# [S] Whole writes into a lazy cell or thread-local key

## Goal

[rust2-ts](/ts/README.md) translates a whole write through a `&mut` to a `LazyCell`, a
`LazyLock`, or a `LocalKey`, as `*r = LazyCell::new(f)` or `mem::swap` of two, keeping
whether each is initialized and what it holds, proven by differential tests against native
Rust that read each after the write.

## Plan

Found by [#386](https://github.com/moq-dev/rust2/pull/386), which refuses these writes with
a span in `Translator::write_in`: their runtime classes keep their state in `#private`
fields, which the field copy a whole write lowers to misses, so the write was silently
lost on `main` before. Copy that state across in the runtime (a method each class owns),
or write the object in place, rather than widening the field copy. `PollFn` is refused the
same way; fold it in if the same method fits.

Public API: the translator's output.
