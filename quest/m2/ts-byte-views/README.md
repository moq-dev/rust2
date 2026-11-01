# Byte slices without objects

## Goal

Reading through a translated cursor allocates no object per read, so the codec's decode on
the [TS benchmark](/ts/README.md#bench) moves toward moq's hand-written cursor, with every
differential test still passing.

## Plan

`&[u8]` and `&[u8; N]` are an `rt.ByteSlice`, a range of a `Uint8Array`, so each split, the
tuples, `Option`s, and `Result`s around it, and each cursor update make objects: 10 per
multi-byte varint, none of which V8 removes. A hand edit of the codec's translation that
kept them as `(data, start, length)` made Chrome's varint decode 2.3-2.9x and frame decode
2.0-2.2x faster, with Firefox, Node, and Bun in step and encode unchanged (spiked on
2026-10-27, local branch `spike/ts-scalar-slices`).

Decided with the maintainer on 2026-10-27: scalar-replace values built of byte slices,
scalars, tuples, `Option`, `Result`, and `ControlFlow`, materializing an object only where
one escapes, in pieces that each land and measure alone (the children below).
The precedents are `stepped` iterators, `range_nexts`, and `place_opts`, which a general
pass could later absorb.

Left here: iterating a byte slice makes one view, which `rt.iterSlice` and stepping in
place read.

Measure Chrome, Firefox, Node, and Bun; Chrome decides.

Public API: the translator's output for byte slices.

## Required

- [Byte slices passed and returned as parts](/quest/m2/ts-byte-views/returns.md) - a function takes a slice's parts and returns a split's parts without an object
- [Byte slices in fields](/quest/m2/ts-byte-views/fields.md) - a private slice field is three fields
