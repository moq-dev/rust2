# [S] Kotlin and Swift: boxed options and parameters apart

## Goal

The api fixture exports these, and every generator's test calls them:

- `held_count(held: Option<(Counter,)>) -> Option<u64>`;
- `boxed_count(boxed: Box<Option<Counter>>) -> Option<u64>`;
- `less(value: u32, _value: u32) -> u32`.

The Go generator handles them already. The generated Kotlin and Swift do not compile:

- Each reads a field through an object's option behind a `Box` or a one-tuple without
  unwrapping it.
- Each declares both of `less`'s parameters as `value`.

## Plan

- Match an option through `Type::base()` and one-tuples where the generator picks the
  nullable-handle path, as `ffi/src/go.rs` does for parameters.
- Give parameters distinct names after conversion, as Go's `locals` does.
- Add the three functions to `ffi/fixtures/api/src/lib.rs`, with checks in `tests/api.kt`,
  `tests/api.swift`, and `tests/api.go`.
