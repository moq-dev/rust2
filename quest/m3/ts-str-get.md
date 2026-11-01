# [S] A str's checked and inclusive parts

## Goal

[rust2-ts](/ts/README.md) translates `str::get`, and indexing a `str` or a `String` by an
inclusive range (`&s[a..=b]`, `&s[..=b]`, and their `&mut` and `get_mut` forms), with
std's `None` and panics. Today each refuses: `get` as an unsupported call, and the
inclusive ranges at `Index::index` or `IndexMut::index_mut`. Proven by differential tests
against native Rust.

## Plan

Found translating `&mut str`'s places, whose `get_mut` and `&mut s[a..b]` take the other
ranges. The runtime's `strSlice` and `strGetMut` check offsets already; an inclusive end
past `usize::MAX` panics as std's does. No consumer calls them yet; planned as part of std.

Public API: the translator's output.
