# [S] The rest of a guard's maps

## Goal

[rust2-ts](/ts/README.md) translates `RefMut::map`, `Ref::filter_map`, `Ref::map_split`,
`RefMut::filter_map`, `RefMut::map_split`, and the `map` of an `RwLock`'s guards, each
releasing its cell's borrow as the last guard drops, as std's do. Today they are refused.
Proven by differential tests against native Rust.

## Plan

`Ref::map` makes an `rt.RefMap`, which holds the part it lends and counts its borrow in
the cell. A mapped `RefMut` lends a `&mut`, which is an `rt.Place` where JS copies the
part, so writes through it must reach that place rather than a copy in the guard.

Public API: the translator's output.
