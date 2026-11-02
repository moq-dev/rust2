# [L] Match cgo on the codec

## Goal

Every `just go bench` workload, translated, runs within 2x of the same crate called
through cgo, and the differential tests still pass.

## Plan

The spike measured why it is 5 to 120 times slower, decoding a QUIC varint in 150 ns where
cgo takes 1.2 ns, with 2 allocations per 1,024 varints:

- **Every MIR local is a Go variable** declared at the top: 240 of `read_quic`'s 981
  instructions zero 16 bytes each, as Go keeps a struct too large for registers in memory.
- **An enum holds every variant's fields** side by side, where Rust overlaps them:
  `Result<(u64, &[u8]), DecodeError>` is 136 bytes in Go, copied at every return.
- **Blocks are labels and `goto`**, which Go's inliner costs past its budget: a
  one-line `Try::branch` costs 81 against 80. PGO recovers little.

Fold each temporary assigned once and read once into its reader, as rust2-ts does, and
lower the blocks to structured control flow. rust2-ts's `cfg` structuring
(`ts/src/lower/mir/cfg.rs`) builds TypeScript statements only at its edges: make it
generic over the statement and condition types in a crate both translators share, as the
first piece of a shared front end. Measure each step on the benchmark before the next.
