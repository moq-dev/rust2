# [M] Narrow rust2's own pub items with rust2-pub

## Goal

rust2's driver, ffi, ts, and go workspaces carry no `pub` that only their own crate uses:
[rust2-pub](/driver/pub/README.md)'s validated `narrow.patch` for rust2 is applied, every
item the narrowing exposes to `dead_code` is deleted or kept with its reason, and every
test passes under `-D warnings`.

## Plan

Decided with the maintainer on 2026-11-13, when [kixel-ai/rust2#785](https://github.com/kixel-ai/rust2/pull/785)
added rust2-pub: ✅ keep this quest.

- Run the audit fresh on `main` inside the dev shell, as the PR's Validation did:
  `just driver pub --out <dir> --manifest-path driver/Cargo.toml --manifest-path ffi/Cargo.toml --manifest-path ts/Cargo.toml --manifest-path go/Cargo.toml -- --workspace --all-targets --locked`.
  At that PR it narrowed 1,468 tokens after one check, none restored.
- Apply the patch, then delete each item `dead_code` flags, or keep it with a comment
  saying who uses it, such as a test or a planned consumer.
- `ffi`'s `shapes::ir_covered` test reads `ffi/src/ir.rs` as text for `pub enum Type {`,
  which the narrowing changes. Make it read the enum's variants from what the compiler
  sees, or match the declaration whatever its visibility, rather than restoring the `pub`.
- What every published crate exports, and all of an unpublished library the audit
  never runs, stays public: the audit already keeps those.
