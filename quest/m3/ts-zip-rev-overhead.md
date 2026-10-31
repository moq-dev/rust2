# [S] A reversed zip's per-pair cost

## Goal

`iter/zip-rev-filter-count` in the TS bench costs about as much per pair as
`iter/zip-filter-count`, its forward twin, or the PR that closes this says what
makes the back read cost more.

## Plan

On the 2026-10-31 run, a reversed zip of a range and bytes still ran at 57-121 ns per byte
translated, 40-80x the hand-written route. The forward zip in the same run cost 19-38 ns per byte.
The excess is no longer read, so the cost is in the pairs: `rev().filter().count()` reaches
`rfold` through `Iter`'s default, and each `TrustedZipIter.nextBack` reads both sides' `len()`
twice. Profile before changing either.
