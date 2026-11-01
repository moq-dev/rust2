# [S] Waiting: type-check in process with tsrs

## Goal

Condition: [tsrs](https://blog.cloudflare.com/typescript-rust-port/), the Rust port of
TypeScript, publishes a stable release usable as a Rust library. Then rust2-ts
type-checks its output and bindings in process through it, with no `tsc` on `PATH`,
failing at the Rust span as today, and measures the translation's time against running `tsc`.
Check: tsrs's releases, and whether it ships a crate (it is moving into Oxc).

## Plan

Decided with the maintainer on 2026-10-27, on
[the bindings PR](https://github.com/moq-dev/rust2/pull/127): translation runs the
flake's `tsc` 7 for now. tsrs was experimental then, and its reported speed over
TypeScript 7 is the vendor's own.

- Its checks must agree with `tsc`'s on every fixture, or the switch waits.

Public API: none, unless the translator drops its `tsc` requirement.
