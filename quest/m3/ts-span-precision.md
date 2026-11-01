# [S] Type errors at the Rust that made them

## Goal

When translated output fails `tsc`, [rust2-ts](/ts/README.md) reports each error at the
Rust expression that made the failing TypeScript, not at a coarser span, proven by reject
fixture cases whose expected spans name the expression.

## Plan

Found by [#376](https://github.com/moq-dev/rust2/pull/376), which fails translation on
output `tsc` rejects at the span the source map names. The map is coarse in places:

- A function's tail `return` maps to its closing `}`, not the tail expression.
- A method call maps to the method's name, not its receiver or the argument that failed.

Give those nodes the span of the MIR statement or operand they come from. Measure that the
finer map does not grow the output or slow a moq-net translation.

Public API: the translator's error spans.
