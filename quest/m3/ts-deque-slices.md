# [M] A deque's two slices

## Goal

[rust2-ts](/ts/README.md) translates `VecDeque::as_slices` and `as_mut_slices`, and `{:?}`
of `vec_deque::Iter`, `IterMut`, and `Drain`, writing the same text as std, proven by
differential tests against native Rust on wrapped deques.

## Plan

Left by [moq-dev/rust2#317](https://github.com/moq-dev/rust2/pull/317), which translated
the rest of std's iterators' `Debug`.

- `Iter` and `IterMut` write the two slices of the ring buffer as `as_slices` splits it:
  `Iter([3], [1, 2])`, each half as consumed so far, the second taking the first's place
  once it empties (`vec_deque/iter.rs`). The split depends on the buffer's capacity and
  where its head sits.
- The runtime's deque does not split where std's does: it grows to a power of two of at
  least 4 and moves its head to 0, ignores `with_capacity`, and shifts items one at a time
  in `insert` and `remove`. std keeps `RawVec`'s capacity, moves the shorter segment as it
  grows (`handle_capacity_increase`), and moves the shorter side in `insert`, `remove`, and
  the rest. Every function that places items must place them where std's does.
- Open: a deque holds only what std guarantees of its capacity, doubling as it grows
  (decided 2026-11-01 to keep pushes cheap), so splitting where std's does needs its exact
  capacity, at a cost to measure, or the maintainer's call to drop the split.
- `Drain` writes its counts, `Drain(drain_len, idx, tail_len, remaining)`, which the
  runtime's drain does not keep.

Public API: the translator's output.
