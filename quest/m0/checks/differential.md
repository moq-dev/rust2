# [S] A cheaper differential

## Goal

The ts differential fixtures run their native wasm32 side built at opt-level 1, under
`node --single-threaded`, and `stable_names` reuses the `lang` test's translation as its
base, with every case still compared.

## Plan

Measured by the 2026-11-01 checks audit on nixos2: `lang`'s differential spends about 590 of
its 872 CPU-seconds running a 212 MB debug wasm in V8. At opt-level 1 it takes 164, with 0
differences across 709k cases. Overflow checks and debug assertions stay on, so panics and
overflow still match. Apply it to every differential fixture. `stable_names`'s base config
equals `lang`'s roots, so its first translation repeats the `lang` test's (about 150
CPU-seconds).

Public API: none.
