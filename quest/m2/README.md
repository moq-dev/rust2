# m2: research

## Goal

Research work with a named consumer that is not waiting on it yet: the
TypeScript translator's spikes and features, and the UniFFI fork's gated or
speculative work.

## Plan

Decided with the maintainer on 2026-10-23: the translator measures before it
builds. The benchmark and census come first; each spike's report rescopes or
deletes the quests that follow it.

Decided with the maintainer on 2026-10-24: the translator's output ranks correctness,
then speed, then the size of the consumer's final tree-shaken, minified JS; readability
is not a goal, and types must be solid at the API boundary ([design](/ts/README.md#design)).

Decided with the maintainer on 2026-10-26: the translator targets browsers. A native
runtime such as Node or Bun calls Rust through [rust2-ffi](/ffi/README.md), so Node and
Bun matter only as proxies for V8 and JavaScriptCore, and a speed call that splits them
from the browsers goes to the browsers.

The fork's feature quests folded into the [exporter](/quest/m1/ffi-export/README.md);
[m1](/quest/m1/README.md#plan) records why.

## Required

- [A pointer a callee takes to a part another function replaces](/quest/m2/ts-pointer-part-replaced.md) - a pointer a callee took through a reference to a part, which another function then replaces whole, is refused rather than silently left on the old object
- [Borrows refused for a pointer no callee keeps there](/quest/m2/ts-taken-precision.md) - moq's census drops the 194 borrows refused for a pointer that no callee they reach keeps
- [Byte slices without objects](/quest/m2/ts-byte-views/README.md) - reading through a translated cursor allocates no object per read
- [Async in the TypeScript translator](/quest/m2/ts-async.md) - moq-net's and kio's async code translates with Rust's lazy futures
- [Shared ownership](/quest/m2/ts-shared.md) - `Arc`/`Rc`/`Weak` keep exact counts, and locks and atomics become plain values
- [Translate moq-net's model to TypeScript](/quest/m2/ts-model/README.md) - the model translates with kio, moq-clock, and moq-pattern, for moq to compare against WASM
- [Waiting: Swift 6.4 in nixpkgs-unstable](/quest/m2/uniffi-swift-arm.md) - the nightly UniFFI fixtures move back to arm
- [Waiting: ubrn reads UniFFI 0.32](/quest/m2/ubrn-release.md) - the binding benchmark's browser runner pins a ubrn release, not an open PR
