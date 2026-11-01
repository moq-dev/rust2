# [S] A guard's downgrade, leak, and whole byte-array map

## Goal

[rust2-ts](/ts/README.md) translates `RwLockWriteGuard::downgrade`, `Ref::leak` and
`RefMut::leak`, and a guard's `map(|p| p)` that lends a whole `[u8; N]`, each holding or
releasing its lock or borrow as std's does, proven by differential tests against native Rust.

## Plan

Found by [#392](https://github.com/moq-dev/rust2/pull/392), which translated the rest of a
guard's maps and `Debug`:

- `RwLockWriteGuard::map(guard, |p| p)` over a whole `[u8; N]` is refused with "a reference
  to an array of bytes it is generic over". The refusal predates #392; check whether
  `RefMut::map` over a whole byte array fails the same way, and fix both.
- `downgrade` (stable since 1.92) turns the write lock into a read lock without releasing
  it, so a `try_write` between fails and a `try_read` succeeds.
- `leak` keeps the cell borrowed for the rest of the program, so a later `borrow_mut`
  panics with wasm32's message.

Reproduce each refusal first; any that already translate drop from this quest.

Public API: the translator's output.
