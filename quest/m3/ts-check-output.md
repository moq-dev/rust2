# [M] Type-check the output before success

## Goal

[rust2-ts](/ts/README.md) fails translation, at the Rust span the source map names, on
output `tsc` rejects, so a typing gap fails loud where the consumer's crate translates
rather than in the consumer's own `tsc`, and measures what the check costs a moq-net
translation.

## Plan

Found by [the generic pointer fields](https://github.com/moq-dev/rust2/pull/371): a
generic type's `*mut T` field translated, and only `tsc` rejected it. `just ts test`
type-checks the fixtures and the nightly `just ts moq` moq's output, but nothing checks
another consumer's. Translation already runs the flake's `tsc` on bindings, so the
output could go through the same run. A rejection is a translator bug, so the error
should say so beside the span.

Public API: translation fails where it succeeded on ill-typed output.

## Related

- [Waiting: type-check in process with tsrs](/quest/m3/ts-tsrs.md) - would make the check cheaper; this needs no wait
