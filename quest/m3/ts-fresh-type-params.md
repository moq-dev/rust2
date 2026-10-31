# [M] Fresh writes in a body generic over a type

## Goal

[rust2-ts](/ts/README.md) judges a whole write fresh in a function generic over a type
parameter as it does in one generic over a length alone: `slot.write(..)` through
`iter_mut` over `[MaybeUninit<([T; N], (u8, u8))>; 3]` fills the slot, so a later
`&raw mut` to a part of it translates instead of being refused.

## Plan

Found while landing [Judging writes in a body generic over a const](https://github.com/moq-dev/rust2/pull/359):
the lang fixture's `lengths::filled` stays fresh only over `[u8; N]`, `[u32; N]`, and
`[[u8; N]; N]`. `Trace::runs_own_code` counts every `ty::Param` as code std's iterators may
run, so a std call given a type holding `T` keeps what it is given. A parameter whose
bounds call nothing, as `T: Copy`, runs none of the crate's code; decide from the bounds
in the body's typing environment, as `Trace::inert` does for std's own impls.

Public API: the translator's output.
