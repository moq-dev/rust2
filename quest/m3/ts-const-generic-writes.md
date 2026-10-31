# [S] Judging writes in a body generic over a const

## Goal

[rust2-ts](/ts/README.md) translates a function generic over a `const N: usize` whose
calls take a value of a type holding `N`, such as
`out.extend(a.iter().chain(v.iter().flatten()))` over `v: Vec<[u8; N]>`, instead of
panicking in rustc's trait solver.

## Plan

Found by [moq-dev/rust2#354](https://github.com/moq-dev/rust2/pull/354): `Translator::stores`
calls `is_freeze` with the translator's typing environment, not the body's, so rustc
cannot find `N`'s type and panics (`cannot find !BoundConst in param-env`), reached from
`Trace::fresh` through `copy::BodyWrites`. Type parameters pass only because the solver
needs no bound for them. Either judge in the body's environment or treat a type holding a
parameter as not `Freeze`; measure which keeps more writes fresh in the lang fixture.

Public API: the translator's output.
