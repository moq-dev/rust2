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
- [Type-check the output before success](/quest/m3/ts-check-output.md) - translation fails at a Rust span on output `tsc` rejects, rather than at the consumer's `tsc`
- [Waiting: type-check in process with tsrs](/quest/m3/ts-tsrs.md) - rust2-ts checks bindings through tsrs instead of `tsc` once it is a stable library
- [Drop order in std's folds and iterators](/quest/m3/ts-drop-order.md) - the values that drop still refused in std's consumers, flattens, and collects drop as std's do
- [Fresh writes in a body generic over a type](/quest/m3/ts-fresh-type-params.md) - a whole write into fresh memory holding a type parameter's values fills it, as one generic over a length alone does
- [Result's iterators](/quest/m3/ts-result-iters.md) - `Result::iter`, `iter_mut`, `into_iter`, and `flatten` over `Result`s translate
- [std's conversions of a reference](/quest/m3/ts-ref-conversions.md) - `to_owned`, `clone_from`, `binary_search`, `size_of_val`, `from_ref`/`from_mut`, and `AsRef`/`Borrow` as a slice translate
- [Waiting: a slice object for a mutable part of other values](/quest/m3/ts-slice-object.md) - waits on a moq profile showing the `Proxy` view's traps matter, then decides on an `rt.ByteSlice`-like form
- [Port the workflow guard to kixelated/quest](/quest/m3/quest-workflow-guard.md) - quest's environment guard parses YAML as this repository's does
- [A deque's two slices](/quest/m3/ts-deque-slices.md) - `VecDeque::as_slices` and `{:?}` of a deque's `Iter`, `IterMut`, and `Drain` translate, split where std's are
- [== of an Option of NonNull](/quest/m3/ts-option-pointer-eq.md) - `==` of `Option<NonNull<T>>` translates
- [ptr::replace whose result nothing reads](/quest/m3/ts-ptr-replace-unread.md) - `ptr::replace` with its result discarded type-checks
- [std's sorts call their tests in std's order](/quest/m3/ts-sort-order.md) - `slice::sort` and a B-tree's `collect` call `lt` or `cmp` as std's driftsort does past 16 elements
- [A B-tree's searches call Ord as std's do](/quest/m3/ts-btree-search-order.md) - a B-tree's lookups and inserts call a key's own `Ord` in std's order and number
- [A &mut into a Box of a value JS copies](/quest/m3/ts-box-places.md) - a write through `&mut *b` of a `Box<u32>`, a `Box<String>`, or a `Box<str>` reaches the box's place
- [A str's checked and inclusive parts](/quest/m3/ts-str-get.md) - `str::get` and indexing a `str` by `a..=b` or `..=b` translate
- [Rc and Arc of a slice](/quest/m3/ts-shared-slices.md) - `Rc<[T]>` and `Arc<[T]>` from a `Vec`, a slice, or an iterator translate
- [The rest of a boxed slice's impls](/quest/m3/ts-boxed-slice-rest.md) - `clone_from`, `AsRef`, `AsMut`, `TryFrom` into a boxed array, and `Box<str>` into bytes translate for boxed slices
- [A reversed zip's per-pair cost](/quest/m3/ts-zip-rev-overhead.md) - a reversed by-index zip costs per pair what a forward one does, or the gap is explained
