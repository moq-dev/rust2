# [M] std's sorts call their tests in std's order

## Goal

[rust2-ts](/ts/README.md)'s `slice::sort` and its `_by` and `_by_key` forms, and the sort
a `BTreeSet` or `BTreeMap` `collect` or `from` makes of keys whose impls are the crate's
own, call `lt` or the comparator in the order and number std's do at any length, proven
by differential tests against native Rust with logging impls.

## Plan

The runtime's `sliceSort` insertion sorts runs of 16, then merges them: the same calls as
std's for at most 16 elements, and the same result for any strict weak order. std's
stable sort insertion sorts at most 20 elements, and past that runs driftsort, whose small
sort, run detection, and merges depend on the element's size and whether it is `Freeze`,
which translation knows. Port it as std's `core/src/slice/sort/stable` reads at the pinned
toolchain, and the unstable sorts' ipnsort likewise, measured on the TS bench's
`order/sort` workloads. [Drop order](/quest/m3/ts-drop-order.md)'s B-tree `collect` of
entries that drop sorted by a crate's own `Ord` follows once this lands.
