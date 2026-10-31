# [L] A B-tree's searches call Ord as std's do

## Goal

[rust2-ts](/ts/README.md)'s `BTreeMap` and `BTreeSet` call a key's own `Ord::cmp` in the
order and number std's do in `insert`, `get`, `remove`, `extend`, `range`, and the set
operators that search, and answer the same where the tree is out of `Ord`'s order, as a
`collect` sorted by a disagreeing `PartialOrd::lt` leaves it, proven by differential tests
against native Rust with a logging `Ord`.

## Plan

The runtime keeps a B+ tree of 64-key nodes searched by bisection, where std's keeps
nodes of at most 11 keys, searched linearly, with values beside keys in every node, so
their calls differ for any key but the first. Matching them means std's node layout and
search, which the TS bench's `btree/` workloads must weigh before it lands; keys whose
impls are derived or std's could keep the faster tree.
