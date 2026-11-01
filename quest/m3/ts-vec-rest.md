# [M] The rest of a vector's methods

## Goal

[rust2-ts](/ts/README.md) translates the `Vec` and `VecDeque` methods still refused, each
holding what std guarantees of its capacity, proven by differential tests against native
Rust.

## Plan

Found carrying capacity through every vector: these have no mapping, and std's bodies move
elements through raw pointers, so each refuses with its span:

- `Vec::append`, `resize`, `resize_with`, `split_off`, `extend_from_within`, and
  `try_reserve`, and `VecDeque`'s `append`, `resize`, and `split_off`.
- `From<VecDeque<T>>` for `Vec<T>`.

A vector's capacity follows only std's guarantees, at least its length and what
`with_capacity` and `reserve` ask for, and doubles as it grows (decided with the
maintainer on 2026-11-01). No consumer calls them yet; they are planned here as part of
std.

Public API: the translator's output.
