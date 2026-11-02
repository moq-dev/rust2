# [S] A box's clone_from, and a vector's AsRef and AsMut

## Goal

[rust2-ts](/ts/README.md) translates `Clone::clone_from` of a `Box<T>` whose `T` needs a
drop, such as `Box<String>`, dropping the old value where std's does, and `AsRef<[T]>` and
`AsMut<[T]>` of a `Vec<T>`, proven by differential tests against native Rust.

## Plan

Found by [#387](https://github.com/moq-dev/rust2/pull/387) after
[#386](https://github.com/moq-dev/rust2/pull/386) added a general `clone_from` rule, which
refuses a `Box<T>` whose `T` needs a drop, and `AsRef` and `AsMut` as a slice for arrays only.
No consumer calls them yet; they are planned here as part of std.

Public API: the translator's output.
