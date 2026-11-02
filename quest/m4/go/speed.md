# [L] Match cgo on the codec

## Goal

Every `just go bench` workload, translated, runs within 2x of the same crate called
through cgo, and the differential tests still pass. If it cannot, the translator is
dropped, as [the epic](/quest/m4/go/README.md) records.

## Plan

The spike measured the translation 2 to 113 times slower, decoding a 2-byte QUIC varint in
143 to 185 ns where cgo takes 1.0 to 1.6 ns. Neither the heap nor the collector explains
it:

- **No allocation in the loop:** a decode of 1,024 varints makes 2 allocations, 24 bytes,
  to cgo's 1. `go build -gcflags=-m=2` finds nothing on the decode path escaping but the
  constant `"unreachable"` panic strings, which are static, and the output's `append`.
- **No collector:** `mallocgc`, the mark workers, and write barriers take 2% of the
  profile.

The time is the translated code moving memory. In the decode path's own time, 58% is
copies of large structs and 23% is zeroing stack slots, leaving 19% for the work and the
calls:

- **An enum holds every variant's fields** side by side, where Rust overlaps them:
  `DecodeError` is 96 bytes and `Result<(u64, &[u8]), DecodeError>` 136. Each
  `Option`, `Result`, and `ControlFlow` on the path carries a `DecodeError`, and each is
  copied in and out of every call by value.
- **Every MIR local is a Go variable** declared at the top. Go zeroes each one that
  holds a pointer at entry, so `read_quic` reserves a 1,488-byte frame. 240 of its 980
  instructions zero 16 bytes each. A varint passes about 3.7 KB of frames.
- **Blocks are labels and `goto`**, which Go's inliner costs past its budget: a
  one-line `Try::branch` costs 81 against 80, so each `?` is a call. PGO recovers 0 to
  15%.

Measure each step on the benchmark before the next:

1. Fold each temporary assigned once and read once into its reader, as rust2-ts does.
2. Lay an enum's variants out so a value is about Rust's size, e.g. a pointer to a cold
   variant's fields.
3. Lower the blocks to structured control flow. rust2-ts's `cfg` structuring
   (`ts/src/lower/mir/cfg.rs`) builds TypeScript statements only at its edges: make it
   generic over the statement and condition types, in a crate both translators share, as
   the first piece of a shared front end.

Public API: none.
