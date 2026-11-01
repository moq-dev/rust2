# [M] The rest of a vector's methods

## Goal

[rust2-ts](/ts/README.md) translates the `Vec`, `VecDeque`, and `BinaryHeap` methods still
refused, each holding std's capacity, proven by differential tests against native Rust.

## Plan

Found carrying std's capacity through every vector: these have no mapping, and std's
bodies move elements through raw pointers, so each refuses with its span:

- `Clone::clone_from` of a vector, which keeps its capacity where that holds the clones,
  and so `slice::fill`, `clone_from_slice`, and `LinkedList::clone_from` of vectors,
  which call it on each slot.
- `Vec::append`, `resize`, `resize_with`, `split_off`, `extend_from_within`, and
  `try_reserve`, and `VecDeque`'s `append`, `resize`, and `split_off`.
- `From<VecDeque<T>>` for `Vec<T>`, which keeps the deque's capacity.

No consumer calls them yet; they are planned here as part of std.

Public API: the translator's output.
