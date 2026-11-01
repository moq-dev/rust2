# [S] An Option of numbers in the result slot

## Goal

A translated function that returns an `Option` of numbers alone, as `Decoder::varint_opt`'s
`Result<Option<u64>, E>`, makes no object per call, with every differential test passing
and no workload on `just ts bench` meaningfully slower in any engine.

## Plan

Left by [#385](https://github.com/moq-dev/rust2/pull/385), which returns a `Result` of
numbers alone through #160's slot: the slot can't carry an `Option`, so `varint_opt` still
makes an `rt.Ok` per call. moq-net calls it 8 times against 249 for `varint`, so this is
low value; take it when a profile or a new caller shows it matters, or with other work on
the slot.

Public API: the translator's output.
