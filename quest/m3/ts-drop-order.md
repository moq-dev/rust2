# [M] Drop order in std's folds and iterators

## Goal

[rust2-ts](/ts/README.md) translates the values that drop which it still refuses
inside std's iterators, each dropping where std's does, in std's order,
proven by the task fixture's differential tests against native Rust. None appears in moq's census at the pinned revision; rust2 aims to support
all of std.

## Plan

Moved from the m2 drop quest on 2026-10-27, once moq-net, its model, and kio had no
drop refusal left ([#139](https://github.com/moq-dev/rust2/pull/139)). `DebugList::entries`,
ranges of bounds that drop, a drain or a B-tree's `range` of a range that drops, a hash
map's `into_keys` and `into_values`, and the closures `for_each` and `fold` own landed in
[#307](https://github.com/moq-dev/rust2/pull/307), with the task fixture's `inside` step.
Each left is refused with a span where std's order is one JS's can't follow yet:

- A closure that owns a value that drops given to a std function that takes another
  value that drops by value, other than `for_each` and `fold` (`iter::releases`), such as
  `reduce`, `max_by_key`, or `partition` of items that drop; one that keeps it other
  than in an adaptor; or one that calls it through `FnOnce` other than where `discards`
  in `library.rs` says, as `or_insert_with` does (`called_closures`). Each consumer whose
  std folds takes a `release` the way `rt.iterFold` does. The reject fixture's
  `called_closure` reproduces `or_insert_with`.
- A `skip` of a `zip` of two `TrustedRandomAccess` iterators of items that drop, as the
  reject fixture's `skipped_zip`: std's `Skip` read by index, as a zip by index or `take`'s
  fold reads it, makes and drops each skipped pair whole, where its `next` skips through
  the zip's `nth`, a side at a time. `rt.Iter` would need a reader by index beside `folded`.
- A `collect` in place of a zip std reads by index that holds values that drop, as the
  reject fixture's `zip_in_place`: std reads it by index, then drops it whole, inner
  closures before outer ones, where the runtime's bounded read is its `fold`, which drops
  each closure as its fold ends.
- A `flatten` or `flat_map` whose inner iterator holds items once ended, as
  `v.into_iter().map(|v| v.into_iter().take(1)).flatten()` of a `Vec<Vec<T>>` whose `T`
  drops. std's `try_fold` keeps an ended inner iterator until the next one replaces it, or
  drops it after the source once the source ends, where `next` drops it as it ends. Every
  adaptor whose `try_fold` is its source's, such as `map`, `filter`'s `next`, and `rev`'s
  through `try_rfold`, reads it that way, so `rt.Iter::folded` would follow through each,
  a hot path to measure on the [TS benchmark](/ts/README.md#bench).
- A B-tree's `collect` of entries that drop, sorted and deduplicated by the crate's own
  impls, as `v.into_iter().collect::<BTreeSet<O>>()` of an `O` whose `Ord`, `PartialOrd`,
  or `PartialEq` is its own: one that panics or disagrees with itself leaves equal keys
  in the order [std's sort](/quest/m3/ts-sort-order.md) does, which decides which of them
  drop.

Each `rt.Iter` has a `fold` and `rfold` that drop what it holds where std's do, and the
by-value consumers std folds go through it: build on those rather than `rt.drop`.

Public API: the translator's output for values that drop in std's iterators.
