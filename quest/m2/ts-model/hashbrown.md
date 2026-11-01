# [M] hashbrown as the runtime's map

## Goal

rust2-ts translates `hashbrown`'s `HashMap`, `HashSet`, and `HashTable`, which hashlink
keeps its entries in, as it translates std's: an `rt.HashMap` and its runtime functions,
proven by differential tests against native Rust.

## Plan

Found by [#201](https://github.com/moq-dev/rust2/pull/201) on 2026-10-28: with hashlink's
raw pointers translated, hashbrown is the model's largest source of refusals left
(251 lines of its census), its SIMD groups, raw allocation, and byte reinterpretation
being what JS has no counterpart of.

- Map it as std's map is: std's `HashMap` is hashbrown's, so the same runtime functions
  stand in for both, with no config.
- `capacity` follows hashbrown's `capacity_to_buckets` and `allocation_size` exactly.
  Open: a `Vec`'s follows only std's guarantees and doubles as it grows (decided
  2026-11-01), so whether a map's must be exact is the maintainer's call.
- `HashTable`'s methods take the hash and the equality from the caller, so it needs a
  table keyed by the caller's hash, not the runtime's own.

Public API: the translator's output.
