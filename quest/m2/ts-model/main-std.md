# [S] The std calls moq's main added to the model

## Goal

rust2-ts translates the std calls that moq-net's model reaches at moq's `main` and refused
there first, proven by differential tests against native Rust: `VecDeque::range`, and
`str::rsplit_once` and `str::split_terminator` by a `char`.

## Plan

Found moving the translator's moq pin past moq's WASM removal: each is one refusal in the
census of `ts/moq/model.toml` (`model/group.rs`, `model/origin.rs`, `path/mod.rs`) that the
previous pin's census did not have. Translate each as std's others of its kind are, with
no config. The model's new `dyn DoubleEndedIterator` in `model/track.rs` is
[trait objects](/quest/m3/ts-dyn.md)' work.

Public API: the translator's output.
