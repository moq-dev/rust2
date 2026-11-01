# [S] Comparing places

## Goal

[rust2-ts](/ts/README.md) compares `&mut` to values JS copies by the values they lend, as
`a.iter_mut().eq(&mut b)`, `a.iter_mut().lt(&mut b)`, and `a.iter_mut().cmp(&mut b)` do,
proven by a differential test against native Rust.

## Plan

Found writing [places through an adaptor's argument](https://github.com/moq-dev/rust2/pull/391):
an iterator of places compared with another is refused as "comparing `&mut u8` as a key"
or "ordering `&mut u8`", while one of values compared with places already reads each
place's value. `cmp_fn` and `eq_fn` in `ts/src/lower/collections.rs` peel shared
references but not a place.

Public API: the translator's output.
