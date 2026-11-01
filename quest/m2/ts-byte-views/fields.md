# [M] Byte slices in fields

## Goal

A private byte slice field, such as `Decoder::buf`, is three JS fields, so a cursor that
advances through it makes no object. Chrome's varint and frame decode on `just ts bench`
are faster than with [returns](/quest/m2/ts-byte-views/returns.md), with every
differential test passing.

## Plan

The [epic's](/quest/m2/ts-byte-views/README.md) hand edit with all three pieces measured
Chrome 2.3-2.9x on varint decode and 2.0-2.2x on frames against [#120](https://github.com/moq-dev/rust2/pull/120), Firefox 2.6-3.5x,
Node and Bun 1.9-3.4x.

Decided with the maintainer on 2026-10-27: private fields split even on root types such as
`Decoder`, since they are not Rust API.

- `mem::take(&mut self.buf)` lowers on the three fields. Any other `&mut` to the field
  leaves that field an object crate-wide, which needs a pass over every reachable body
  before structs are emitted.
- `Debug` and `Clone` rebuild the slice from the fields.

Public API: the translator's output; a private slice field of an exported class becomes
three `#private` fields.

## Required

- [Byte slices passed and returned as parts](/quest/m2/ts-byte-views/returns.md) - the measurement this one builds on
