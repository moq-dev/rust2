# [M] Borrows refused for a pointer no callee keeps there

## Goal

[rust2-ts](/ts/README.md) refuses a borrow of a variable assigned again only where a
raw pointer taken through that borrow may still be read, so moq's census holds none of
the 194 needless refusals below. Every case in the reject fixture stays refused.

## Plan

Found on 2026-10-31 by the census of
[A pointer a callee takes to a variable that gets another object](https://github.com/moq-dev/rust2/pull/350),
at the pinned moq revision: 184 in moq-net, 8 in the model, and 2 in kio, all of
`a reference to _, through which a raw pointer may be made`. `Writes::taken` in
`ts/src/lower/copy.rs` judges by type, so a type one callee keeps a pointer to is refused
at every borrow of every variable holding it. None of these borrows reaches such a callee:

- 162 hold a `[u8; N]`, such as `Path`'s inline buffer. zlib-rs's `Deflate::compress` and
  `Inflate::decompress` keep `next_in` and `next_out` in the stream past the call, so
  `[u8]` is taken, and arrays share slices' key. zlib-rs writes both again before any
  read.
- 22 hold a `SmallVec`, whose `inline`, `inline_mut`, and `drain` return raw pointers
  used within the borrow they came from.
- 10 are lowered over type parameters, where smallvec's `Drain::next` closure,
  which only `ptr::read`s through its pointer, takes `<T as Array>::Item`, a projection
  any type instantiates. `Trace` counts what `ptr::read` loads as holding the pointer it
  was read through (`Derives::Loaded`), so the closure seems to return it.

Two fixes, which together leave none of these:

1. A value std's reads load through a pointer doesn't hold that pointer, wherever
   `BodyWrites::outlives` asks. Check every other `Trace` caller before changing it there.
2. Record where each taken type's pointer is made, and refuse a borrow only if its
   reference may reach one of those calls, as `Trace::keeps` follows calls already.
   Alternatively, prove a kept pointer is written again before any read on every entry, as
   zlib-rs's stream fields are. Measure both against the census before picking.
