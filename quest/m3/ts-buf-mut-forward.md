# [S] BufMut's provided writes through a &mut

## Goal

[rust2-ts](/ts/README.md) translates every `BufMut` method of a `&mut Vec<u8>` and a
`&mut BytesMut`, including the provided ones `&mut T` does not forward, such as `put`,
`put_bytes`, `put_uint`, and `put_int`, proven by differential tests against native Rust.

## Plan

Found by [#241](https://github.com/moq-dev/rust2/pull/241) on 2026-10-29: `bytes`' impl for
`&mut T` forwards the required methods and most `put_*`, but not these, so their provided
bodies lower from `bytes`' MIR and reach `chunk_mut`'s `UninitSlice`, which is refused.
moq-net's census reaches none of them, so this is m3 under the all-of-std decision.

- Mapping them as the buffer's own methods is the likely shape, but the provided bodies
  grow through `chunk_mut`, which reserves 64 bytes when full, where `Vec<u8>`'s and
  `BytesMut`'s overrides reserve what they write. Match the provided body's capacity once
  `BytesMut`'s capacity is exact ([`BytesMut`'s](/quest/m3/ts-bytes-capacity.md)), not the
  override's.

Public API: the translator's output.
