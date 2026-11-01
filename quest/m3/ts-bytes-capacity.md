# [M] The capacity of a BytesMut

## Goal

[rust2-ts](/ts/README.md) translates `BytesMut::capacity`, `split_off`, `try_reclaim`,
`try_unsplit`, and `spare_capacity_mut` with `bytes`' values, and `rt.BytesMut` drops its
512 MiB limit on `reserve` for storage that was shared, proven by differential tests
against native Rust.

## Plan

Found by [#222](https://github.com/moq-dev/rust2/pull/222) on 2026-10-29: `bytes`
reclaims a `BytesMut`'s storage in `reserve` once no other handle of it is alive, so its
capacity depends on whether the `Bytes` it froze and the handles it split off dropped.
`rt.ByteSlice`, which a `Bytes` is, is a value JS copies and counts no drops, so #222
refuses these. moq calls none of them. For the same reason, once its storage was shared
or taken from a `Bytes`, `reserve` throws a `rust2-ts:` error past 512 MiB, counting what
the handle read past, where `bytes`' panic, or its growing instead, depends on which
handles dropped. Exact capacity removes that limit.

- Count the live handles of a storage, as `rt.Arc` counts
  an `Arc`'s, which makes a `Bytes` a value that drops.
- Measure the count's cost on the codec's [TS benchmark](/ts/README.md#bench).

Public API: the translator's output.
