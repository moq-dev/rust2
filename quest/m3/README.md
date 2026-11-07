# m3: deferred

## Goal

Work that waits on evidence or has no consumer yet: the build-speed tracks,
each a measurement that decides whether its follow-up lands, and the parts of
std the translators cover before a consumer calls them.

## Plan

Decided with the maintainer on 2026-10-27: rust2 aims to support all of std, so a
generic std gap is planned here even when moq never calls it.

Decided with the maintainer on 2026-11-07: ✅ little time goes to unsafe Rust. A
translator may refuse unsafe code with its span, requiring safe Rust, and a common
library's unsafe internals may be replaced by a runtime mapping over its public API, as
`hashbrown`'s and tokio's channels are (rejected: planning each unsafe std gap that no
consumer reaches, such as `MaybeUninit::zeroed`, `Rc::new_uninit`, and
`ptr::write_bytes`).

Decided with the maintainer on 2026-11-07, the unsafe policy, to stop one-off fixes for
ever rarer `unsafe`:

1. ✅ Safe Rust is the contract, and where translation effort goes.
2. ✅ `unsafe` inside std and common crates is wrapped at their safe API, as std,
   `hashbrown`, `bytes`, and [tokio's channels](/quest/m3/ts-wasm-bindgen/tokio-sync.md)
   are, rather than translated.
3. ✅ Other `unsafe` translates only where it fits the translator's existing pointer forms
   (a place, an array element, a byte in a buffer). A construct needing a new pointer form
   or pointer metadata is refused with its span, unless a consumer needs it and no library
   wrapper covers it.
4. ✅ An `unsafe`-heavy crate a consumer needs is compiled to WASM behind its public API
   rather than fixed construct by construct. Not planned until a consumer needs it.

Rejected: an unsafe std gap with no consumer becoming its own quest, as
`MaybeUninit::zeroed`, `Rc::new_uninit`, and `ptr::write_bytes` would have
([#647](https://github.com/moq-dev/rust2/pull/647)), or as `Vec::from_raw_parts`'s adopted
storage did ([#635](https://github.com/moq-dev/rust2/pull/635)); and a second, byte-backed
memory model for values whose address is reinterpreted, without a consumer.

## Required

- [Retrial Cranelift on moq's tests](/quest/m3/cranelift-unwind.md) - waits on upstream: moq's tests pass under Cranelift, so its CI can weigh it again
- [Relink without rebuilding](/quest/m3/relink.md) - measure upstream's skip of dependency-only rebuilds on moq's edits once it reaches nightly
- [Unsafe code the translator can model](/quest/m3/ts-unsafe/README.md) - a dependency using only `unsafe` the translator can model translates from source
- [Waiting: drop the lang fixture's mul_add allowance](/quest/m3/ts-fma-allowance.md) - the differential fails on any `mul_add` difference again once wasm32's `fma` rounds exactly
- [Waiting: type-check in process with tsrs](/quest/m3/ts-tsrs.md) - rust2-ts checks bindings through tsrs instead of `tsc` once it is a stable library
- [Parts of arrays without arrayView](/quest/m3/ts-slice-object.md) - a `&mut [T; N]` to part of an array is taken where it is, with no `Proxy`
- [Writes through iter_mut without a Place per item](/quest/m3/ts-iter-mut-places.md) - a loop over `iter_mut()` of copy values writes the slice's array by index
- [Throw only where Rust panics](/quest/m3/ts-runtime-throws.md) - no runtime `rust2-ts:` error where native Rust goes on: an exact slower path, or a refusal at translation
- [rust2-ts with no ts.toml](/quest/m3/ts-configless.md) - a crate's whole `pub` API translates with no config and no bound items
- [A slice of one byte's place](/quest/m3/ts-byte-slice-place.md) - `slice::from_mut` of a `&mut u8` writes the byte where the caller holds it
- [Browser APIs through wasm-bindgen](/quest/m3/ts-wasm-bindgen/README.md) - web-sys and js-sys imports become direct JS calls, so idiomatic crates such as web-rs's web-codecs translate as stock crates
