# [S] Places through an adaptor's argument

## Goal

[rust2-ts](/ts/README.md) translates an adaptor given a `&mut` to a collection of values
JS copies, as `iter.zip(&mut v)` or `iter.chain(&mut res)` where `iter` yields no places,
proven by a differential test against native Rust that writes through each.

## Plan

Found writing [Result's iterators](https://github.com/moq-dev/rust2/pull/381): such a
call is refused as "a mutable reference to a value JS copies, which its mapping would
copy", while `for x in &mut v` and `v.iter_mut().zip(&mut w)` translate. `std_places` in
`ts/src/lower/mir/call.rs` accepts places only from a listed mapping or from what the
adaptor's receiver yields. Accepting every generic `IntoIterator` argument is too broad:
`iter.eq(&mut v)` would compare places, not values, so check each adaptor's mapping.

Public API: the translator's output.
