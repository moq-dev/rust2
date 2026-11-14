# [M] Opaque padding from the rustc the glue builds with

## Goal

A [rust2-ffi](/ffi/README.md) export zeroes an opaque value's padding from a table only
when the table describes the layout the glue is compiled with. Today `collect.rs` reads
each target's padding from the exporter's own nightly `rustc`, while the glue builds with
the `rustc` cargo resolved, and Rust promises no `repr(Rust)` layout across compilers. A
compiler that orders a type's fields differently makes `lower_<T>` zero a live field on
every write, silently. That mismatch fails the glue's build instead.

## Plan

- The maintainer settled on 2026-11-13: ✅ keep the quest (rejected: drop). How the table
  is tied to the glue's layout: ✅ decide later (rejected for now: pin the glue's `rustc`;
  padding-free types only). The decision stays under Waiting.
- Measured on 2026-11-14, at moq's pin `ffcb7cd9` on `x86_64-unknown-linux-gnu`
  ([#811](https://github.com/kixel-ai/rust2/pull/811)): 3 of moq's 28 opaque values have
  padding, `moq_pattern::Specificity` (48 bytes, 7 at 41), `moq_tokio::cli::Duration` (16,
  4 at 12) and `std::time::Instant` (16, 4 at 12). Each is off moq's hot path: a pattern's
  rank, a config duration, and `clock::Instant::into_std` and `from_std`. The hot
  `clock::Instant` is a padding-free `u64`. The four stats types
  [#808](https://github.com/kixel-ai/rust2/pull/808) makes values are all `u64` fields.
- The hazard is real: `lower_<Specificity>` with today's table, built with
  `-Zrandomize-layout`, puts `exact` at 0 or 16 and zeroes 7 bytes of a live `usize`, while
  the glue's size and alignment assert passes. The dev shell's stable 1.98.1 happens to
  lay it out as the exporter's nightly 1.102 does.
- A struct with an empty table is safe on any `rustc`: its leaf fields fill its size, so
  no field order leaves a gap once the glue asserts that size. Only an enum breaks this,
  since its tag width is a layout choice. moq's one enum value, `Version`, has a 1-byte
  tag and a 1-byte payload in its 2 bytes, so no field order leaves a gap there either.
  Only the 3 non-empty tables are at risk today.
- Pinning to rust2's nightly would make every consumer build the glue with it, which
  today only the exporter needs.
- The glue can't name a private field, so it can't check an offset itself.
- If the pin is picked: the export records the commit hash of the `rustc` cargo resolved
  beside each table, and the glue's build script refuses any other `rustc` with an error
  naming both. The table comes from that `rustc`'s layout, from `-Zprint-type-sizes`
  under `RUSTC_BOOTSTRAP`, which on stable 1.98.1 prints each type's fields in memory order
  with its gaps. Prove it with a test that builds the glue with another `rustc` and
  expects the refusal, and the api fixture's `Gap` test still passing with the matching
  one.
- If handles are picked: a type is a value only when, on every variant, its leaf sizes,
  plus the smallest tag that counts its variants, add up to its size. Each other type is
  an object. `Record::padding`, the glue's `PADDING` tables, and their merge are deleted.
  The api fixture's `Gap` becomes a handle, and a test that builds a value's glue with
  `-Zrandomize-layout` round-trips every field.
- The API lock leaves padding out: `merge::bare_record` clears it, so a change of it is no
  break.

## Waiting

- Maintainer: how the table is tied to the glue's layout. Options: cross as raw bytes
  only types whose leaf sizes, plus the smallest tag on an enum, fill their size, so no
  field order leaves padding, and cross the rest as objects (handles) (recommended: sound
  on any `rustc` with no table, no pin, and no unstable `rustc` output, and it deletes the
  per-target tables; moq pays one handle on each of 3 types off its hot path);
  pin the glue's `rustc` to the one whose layout the export read (exact, and fails loudly
  at build, but moq's glue then fails on each toolchain bump until it exports again, and
  the table parses `-Zprint-type-sizes`, an unstable text format, under
  `RUSTC_BOOTSTRAP`); compare only each target's size and alignment exactly (rejected:
  misses a reorder of the same size, as the `-Zrandomize-layout` check shows); stop zeroing
  padding (rejected: stale Rust bytes reach languages that serialize a value). An agent
  cannot pick, since the pin constrains every consumer's toolchain and handles change
  moq's API for 3 types.
