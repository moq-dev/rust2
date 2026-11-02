# [S] Stable names on a small fixture

## Goal

`stable_names` checks that output names stay stable on a small fixture of its own that still
has generic instances and trait impls, instead of translating all of `lang`.

## Plan

Decided with the maintainer on 2026-11-01: it caught 3 failures nothing else did, so it
stays in the gate, but on a fixture that costs a fraction of `lang`'s (about 300
CPU-seconds saved after [the reuse](/quest/m0/checks/differential.md)). Cover each naming
rule it relies on in `lang` today.

Public API: none.
