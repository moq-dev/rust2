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
- [An Option a loop borrows, tested as one](/quest/m3/ts-option-ref-loop.md) - `if let Some(x)` of the `&Option<T>` a `for` loop yields tests its `None`, not the iterator's end
- [Judging writes in a body generic over a const](/quest/m3/ts-const-generic-writes.md) - a function generic over `const N` whose calls take a type holding `N` translates instead of panicking
- [Arrays of bytes a flattened Option reference lends](/quest/m3/ts-flatten-byte-options.md) - `iter().flatten()` over `&[Option<[u8; N]>]` yields `rt.ByteSlice`s
- [std's conversions of a reference](/quest/m3/ts-ref-conversions.md) - `to_owned`, `clone_from`, `binary_search`, `size_of_val`, `from_ref`/`from_mut`, and `AsRef`/`Borrow` as a slice translate
- [Waiting: a slice object for a mutable part of other values](/quest/m3/ts-slice-object.md) - waits on a moq profile showing the `Proxy` view's traps matter, then decides on an `rt.ByteSlice`-like form
- [Port the workflow guard to kixelated/quest](/quest/m3/quest-workflow-guard.md) - quest's environment guard parses YAML as this repository's does
- [A deque's two slices](/quest/m3/ts-deque-slices.md) - `VecDeque::as_slices` and `{:?}` of a deque's `Iter`, `IterMut`, and `Drain` translate, split where std's are
- [ptr::replace whose result nothing reads](/quest/m3/ts-ptr-replace-unread.md) - `ptr::replace` with its result discarded type-checks
- [LinkedList](/quest/m3/ts-linked-list.md) - `LinkedList` and its methods translate
- [std's sorts call their tests in std's order](/quest/m3/ts-sort-order.md) - `slice::sort` and a B-tree's `collect` call `lt` or `cmp` as std's driftsort does past 16 elements
- [A B-tree's searches call Ord as std's do](/quest/m3/ts-btree-search-order.md) - a B-tree's lookups and inserts call a key's own `Ord` in std's order and number
- [A &mut str to any String](/quest/m3/ts-str-places.md) - `make_ascii_*` of a `String` a tuple, an array, or a `Vec` holds, and of a part of one, translate
- [The rest of a boxed slice's impls](/quest/m3/ts-boxed-slice-rest.md) - `clone_from`, `AsRef`, `AsMut`, `TryFrom` into a boxed array, and `Box<str>` into bytes translate for boxed slices
- [A crate's own Extend in a tuple](/quest/m3/ts-tuple-own-extend.md) - `collect` and `extend` into a tuple holding a crate's `Extend` type translate
- [moq-pin without a network when the pin is present](/quest/m3/ts-moq-pin-offline.md) - `just ts moq-pin` skips its fetch when the pinned commit is already there
- [A reversed zip's per-pair cost](/quest/m3/ts-zip-rev-overhead.md) - a reversed by-index zip costs per pair what a forward one does, or the gap is explained
