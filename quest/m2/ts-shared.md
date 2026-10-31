# [M] Shared ownership in the TypeScript translator

## Goal

[rust2-ts](/ts/README.md) translates `Arc`, `Rc`, and `Weak` with Rust's exact
counts: `strong_count`, `weak_count`, and `upgrade` match native, and the last
strong drop runs `Drop` where Rust runs it. `Mutex`, `RwLock`, atomics,
`OnceLock`, `OnceCell`, `Cell`, and `RefCell` translate as the single-threaded
values they are in JS, proven by differential tests.

## Plan

Decided with the maintainer on 2026-10-23: deterministic reference counts in
`rt.ts`, not GC-backed `WeakRef`. kio's `WaiterList` frees slots when
`Weak::upgrade` fails and reads `weak_count`, so drop timing is observable.

The async spike ([#16](https://github.com/moq-dev/rust2/pull/16)) landed what its
kio-shaped fixture needs, in `rt/task.ts`: `rt.Arc`, an `Arc` or an `Rc`, counts
strong and weak references and drops its value with the last strong one, `rt.Weak`
upgrades while one is left, and `rt.Mutex` is its own guard, which a lock gives as
`Ok`: the translator aborts on panic, as wasm32 does, so nothing poisons a mutex. A
guard's drop unlocks it, `try_lock` while held gives `WouldBlock`, and a re-entrant
`lock` panics with wasm32's message. `new_cyclic` gives its function a `Weak` that
upgrades once the function returns. `rt.Atomic` holds a `bool` or an integer, and its
operations check their orderings as std does. `rt.OnceCell` and `rt.OnceLock` hold
their `Option<T>`, and a `OnceLock` initialized from within its own initialization panics as wasm32's does;
their `Clone`, `Debug`, `get_mut`, and `get_or_try_init` are still refused.
`rt.Cell` is the place that holds its value. `rt.RefCell`, a `RefCell` or an `RwLock`,
is its own guard and counts its borrows: a conflicting borrow panics, and a conflicting
lock aborts, each with wasm32's message. Their `Clone`, `get_mut`, `into_inner`,
`try_borrow`, `Ref::map`, and `Ref::clone` are still refused.
A `thread_local!` key is an `rt.LocalKey`, a module's value that makes its value on
first use, as wasm32's `LazyStorage` does, and never drops it. A `static` atomic is a
module's value too, an `rt.Atomic` of the value the static's memory starts with.
Left: the refused methods above.

- `Send` and `Sync` bounds have no meaning in JS; accept them.
- The counts cost the async handoff at most 3% on the [TS benchmark](/ts/README.md#bench),
  within its A/A noise. On 2026-10-31, a hand-edited translation without `Weak`'s count
  and without the clone and drop around `WaiterList::wake`'s `upgrade`, which the fixture
  never observes, timed 0.99x the translation in Node and Chrome and 0.97x in Bun and
  Firefox, the geometric mean over 3 depths and 3 runs, whose A/A gaps spanned 0.93-1.12.

Public API: the translator's output for shared and interior-mutable types.
