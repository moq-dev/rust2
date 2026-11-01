# [S] The rest of a guard's methods

## Goal

[rust2-ts](/ts/README.md) translates what is still refused of std's guards: `Debug` of a
`Ref`, a `RefMut`, and an `RwLock`'s guards, mapped or not; `MutexGuard::map` and
`filter_map` and the `MappedMutexGuard` they make; and `Ref::try_map` and
`RefMut::try_map`. Each releases its lock or cell as std's does, proven by differential
tests against native Rust.

## Plan

The maps reuse what a `RefCell` guard's map makes: an `rt.RefMap` of where its part is,
so a write through a mapped exclusive guard reaches the value. A `MutexGuard` is its
mutex, so a mapped one needs the mutex to unlock beside the part it lends.
