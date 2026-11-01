# [XS] An empty iterator of mutable references

## Goal

[rust2-ts](/ts/README.md) translates `std::iter::empty::<&mut T>()` for a `T` JS copies,
as given to `zip` or `chain`, proven by a differential test against native Rust.

## Plan

Found by [#391](https://github.com/moq-dev/rust2/pull/391): the check that lets an
adaptor's argument lend places refuses `iter::empty`, though it yields nothing to write
through. Accept it in that check, so an empty iterator is a place source like a
`&mut` collection.

Public API: the translator's output.
