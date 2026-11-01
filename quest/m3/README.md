# m3: deferred

## Goal

Work that waits on evidence or has no consumer yet: the build-speed tracks,
each a measurement that decides whether its follow-up lands, and the parts of
std the translators cover before a consumer calls them.

## Plan

Decided with the maintainer on 2026-10-27: rust2 aims to support all of std, so a
generic std gap is planned here even when moq never calls it.

## Required

- [Retrial Cranelift on moq's tests](/quest/m3/cranelift-unwind.md) - waits on upstream: moq's tests pass under Cranelift, so its CI can weigh it again
- [Relink without rebuilding](/quest/m3/relink.md) - measure upstream's skip of dependency-only rebuilds on moq's edits once it reaches nightly
- [Unsafe code the translator can model](/quest/m3/ts-unsafe.md) - a dependency using only `unsafe` the translator can model translates from source
- [The capacity of a BytesMut](/quest/m3/ts-bytes-capacity.md) - `BytesMut::capacity` and what reads it translate with `bytes`' values
- [BufMut's provided writes through a &mut](/quest/m3/ts-buf-mut-forward.md) - `put`, `put_bytes`, `put_uint`, and `put_int` of a `&mut Vec<u8>` or `&mut BytesMut` translate
- [Trait objects](/quest/m3/ts-dyn.md) - `dyn Error`, `dyn Iterator`, `dyn Buf`, and `Arc<dyn Debug>` translate, so a consumer neither rewrites nor binds them
- [Placeholders written to a crate's own fmt::Write](/quest/m3/ts-fmt-calls.md) - `write!` with placeholders into a crate's own writer translates, its `write_str` seeing each call std makes
- [Waiting: type-check in process with tsrs](/quest/m3/ts-tsrs.md) - rust2-ts checks bindings through tsrs instead of `tsc` once it is a stable library
- [Drop order in std's folds and iterators](/quest/m3/ts-drop-order.md) - the values that drop still refused in std's consumers, flattens, and collects drop as std's do
- [Fresh writes in a body generic over a type](/quest/m3/ts-fresh-type-params.md) - a whole write into fresh memory holding a type parameter's values fills it, as one generic over a length alone does
- [std's conversions of a reference](/quest/m3/ts-ref-conversions.md) - `to_owned`, `clone_from`, `binary_search`, `size_of_val`, `from_ref`/`from_mut`, and `AsRef`/`Borrow` as a slice translate
- [Waiting: a slice object for a mutable part of other values](/quest/m3/ts-slice-object.md) - waits on a moq profile showing the `Proxy` view's traps matter, then decides on an `rt.ByteSlice`-like form
- [Port the workflow guard to kixelated/quest](/quest/m3/quest-workflow-guard.md) - quest's environment guard parses YAML as this repository's does
- [A deque's two slices](/quest/m3/ts-deque-slices.md) - `VecDeque::as_slices` and `{:?}` of a deque's `Iter`, `IterMut`, and `Drain` translate, split where std's are
- [Places through an adaptor's argument](/quest/m3/ts-adaptor-places.md) - `iter.zip(&mut v)` of values JS copies lends places as `for x in &mut v` does
- [std's sorts call their tests in std's order](/quest/m3/ts-sort-order.md) - `slice::sort` and a B-tree's `collect` call `lt` or `cmp` as std's driftsort does past 16 elements
- [A B-tree's searches call Ord as std's do](/quest/m3/ts-btree-search-order.md) - a B-tree's lookups and inserts call a key's own `Ord` in std's order and number
- [Rc and Arc of a slice](/quest/m3/ts-shared-slices.md) - `Rc<[T]>` and `Arc<[T]>` from a `Vec`, a slice, or an iterator translate
- [The rest of a boxed slice's impls](/quest/m3/ts-boxed-slice-rest.md) - `clone_from`, `AsRef`, `AsMut`, `TryFrom` into a boxed array, and `Box<str>` into bytes translate for boxed slices
- [The rest of a vector's methods](/quest/m3/ts-vec-rest.md) - `append`, `resize`, `split_off`, `try_reserve`, and `Vec::from(VecDeque)` translate
- [Aborts in the differential](/quest/m3/ts-abort-differential.md) - the lang differential compares wasm32's aborts, so an `RwLock` locked again while held is tested
- [A reversed zip's per-pair cost](/quest/m3/ts-zip-rev-overhead.md) - a reversed by-index zip costs per pair what a forward one does, or the gap is explained
- [Whole writes into a lazy cell or thread-local key](/quest/m3/ts-lazy-writes.md) - a whole write through a `&mut` to a `LazyCell`, `LazyLock`, or `LocalKey` keeps its state
- [Type errors at the Rust that made them](/quest/m3/ts-span-precision.md) - a `tsc` error on output names the tail expression or call argument that made it
- [An empty iterator of mutable references](/quest/m3/ts-empty-places.md) - `iter::empty::<&mut T>()` given to `zip` or `chain` translates
- [An Option of numbers in the result slot](/quest/m3/ts-option-slot.md) - `varint_opt`'s `Option` comes back through the slot with no object per call
- [Dropping a crate's generic type inside another](/quest/m3/ts-nested-generic-drop.md) - an `LruCache` holding a `LinkedHashMap` drops as rustc's glue does
- [Fresh writes through a zip, a copy, and a repeat](/quest/m3/ts-fresh-shapes.md) - zipping a `Vec<[T; N]>`, reading a `T` out of `Zip::next`, and `copied` or `vec![x; n]` over a `Copy` type translate
