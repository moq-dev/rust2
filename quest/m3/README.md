# m3: deferred

## Goal

Work that waits on evidence or has no consumer yet: two waits on upstream's build-speed
levers ([relink](/quest/m3/relink.md) and [TPDE](/quest/m3/tpde.md)) and
[the benchmark on bare metal](/quest/m3/bench-metal.md), the translator's features that
wait on [the gate](/quest/m2/wasm/README.md), the binding targets planned ahead of a
consumer, what waits on moq, [crabx's GitHub App](/quest/m3/crabx-app.md), and
[narrowing rust2's own `pub`](/quest/m3/pub-apply.md) with rust2-pub.

## Plan

Decided with the maintainer on 2026-11-09, in the quest audit:

- ✅ Until [the gate](/quest/m2/wasm/README.md) reports, translator work is only what the
  gate needs. The quests here that no consumer calls wait on it:
  [no `ts.toml`](/quest/m3/ts-configless.md),
  [browser APIs through wasm-bindgen](/quest/m3/ts-wasm-bindgen/README.md), and
  [type-checking with tsrs](/quest/m3/ts-tsrs.md), which the 2026-11-12 audit found
  ready with no link to the gate. This
  supersedes, until then, the 2026-10-27 decision that rust2 aims to support all of std,
  so a generic std gap was planned here even when moq never calls it. Rejected:
  continuing as before; freezing the model's work too.
- ✅ C# and Unity, a Node-API addon, React Native, and a WASI target are planned here
  before any consumer asks, since they are the targets a media SDK's users expect: LiveKit ships Python, Node,
  Unity, and React Native over its Rust core. Each waits on
  [the exporter epic](/quest/m1/ffi-export/README.md), so it copies the settled C ABI,
  types, and checks. Rejected: none until a consumer asks. Not planned: Java without
  Kotlin (the JNI library serves it, and Android has no FFM API), Ruby, PHP, Elixir, Lua, and game engines.

Decided with the maintainer on 2026-11-12, in the quest audit:

- ✅ [TPDE](/quest/m3/tpde.md) moves here from m4: it is an upstream build-speed lever the
  benchmark measures, which is what m3 holds, and the nearest of them to clearing.
  Rejected: m4.
- ✅ The Cranelift retrial is deleted. `bench/build/README.md` already holds its verdict:
  12% less CPU cold and 4% warm, on CI that is usually warm, and enabling unwinding can
  only shrink that. Its two upstream waits
  ([rustc_codegen_cranelift#1567](https://github.com/rust-lang/rustc_codegen_cranelift/issues/1567)
  and [#1712](https://github.com/rust-lang/rustc_codegen_cranelift/issues/1712)) were
  still open. Rejected: keeping it with the ceiling stated.
- ✅ [The dead `pub` spike](https://github.com/kixel-ai/rust2/pull/763) joins with no consumer waiting. Considered and left out: a build doctor
  that ranks why an edit rebuilt slowly, which conflicts with build speed keeping only
  what costs nothing; a cancel-safety report for exported futures, likely mostly false
  positives; a general semver checker, which cargo-semver-checks is taking into cargo; a
  Rust generator over the C ABI as a rebuild firewall; and driver-generated derives,
  since proc macros measured 0.2% of a clean build
  ([#64](https://github.com/kixel-ai/rust2/pull/64)).

The [dead `pub` spike](https://github.com/kixel-ai/rust2/pull/763) finished on
2026-11-13 with a go for a compiler-validated audit that preserves original resolved
references, which [rust2-pub](/driver/pub/README.md) now is. The raw
graph still fails moq publicity checks. A compiler-only narrowing loop can silently
redirect method dispatch: the native fixture changes 99 to 3, while the graph
retains the original method. Neither alone is an automatic cleanup tool, and
neither establishes the absence of external API consumers. The PR holds the census,
collective mutation checks and comparison.

Decided with the maintainer on 2026-11-13, for
[rust2-pub](https://github.com/kixel-ai/rust2/pull/785):

- ✅ Validation fingerprints every item as rustc's HIR printer prints it after macro
  expansion, which leaves visibility out, and traces a changed item back through the
  macro calls it came from to the narrowed `pub` they read. It replaces the derive
  fingerprint and the rule that kept every `pub` passed to a macro. Rejected: adding a
  check per macro kind, where each review round found one more.
- ✅ What a published crate exports, and all of an unpublished library the audit never
  runs, stays public. Rejected: also reporting a published crate's unused exports as
  breaking narrowings.
- ✅ Applying the findings is follow-up work: [rust2's](/quest/m3/pub-apply.md) here,
  and moq's in moq's own quest tree.

Decided with the maintainer on 2026-11-07, the unsafe policy, to stop one-off fixes for
ever rarer `unsafe`:

1. ✅ Safe Rust is the contract, and where translation effort goes.
2. ✅ `unsafe` inside std and common crates is wrapped at their safe API, as std,
   `hashbrown`, `bytes`, [tokio's channels](/ts/src/lower/tokio.rs), and
   [smallvec](/ts/src/lower/smallvec.rs) are, rather than translated.
3. ✅ Other `unsafe` translates only where it fits the translator's existing pointer forms
   (a place, an array element, a byte in a buffer). A construct needing a new pointer form
   or pointer metadata is refused with its span, unless a consumer needs it and no library
   wrapper covers it.
4. ✅ An `unsafe`-heavy crate a consumer needs is compiled to WASM behind its public API
   rather than fixed construct by construct, through
   [rust2-wasm](/quest/m2/wasm/README.md) once it exists. Mixing a translated crate with
   one compiled so is not planned until a consumer needs it.
5. ✅ Where translation can't see where an `unsafe` pointer came from, a loud run-time
   check stands in for the refusal, the exception AGENTS.md names: `ptrUnerase`,
   `offset_from` across arrays, `AtIndex.reach`, and raw slices of a field's byte
   (decided 2026-11-07 in [#661](https://github.com/kixel-ai/rust2/pull/661) and
   [#621](https://github.com/kixel-ai/rust2/pull/621)). Safe code gets no such check: it
   translates exactly or is refused.

Rejected: an unsafe std gap with no consumer becoming its own quest, as
`MaybeUninit::zeroed`, `Rc::new_uninit`, and `ptr::write_bytes` would have
([#647](https://github.com/kixel-ai/rust2/pull/647)), or as `Vec::from_raw_parts`'s adopted
storage did ([#635](https://github.com/kixel-ai/rust2/pull/635)); and a second, byte-backed
memory model for values whose address is reinterpreted, without a consumer.
