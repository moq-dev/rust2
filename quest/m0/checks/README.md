# Checks at moq's cost

## Goal

The land gate and agents' local checks cost what moq's do: `just ci` runs only what a
change touches, builds warm, and runs the differential fixtures optimized, while a nightly
`just gate` runs the full suite. Measured against the 2026-11-01 audit's baseline: about
33 CPU-hours a day in the gate (65 CPU-minutes a run) and 100 to 130 locally.

## Plan

Decided with the maintainer on 2026-11-01, from an audit of the gate's and agents' checks
(nixos2 and the host's systemd accounting): copy how moq works. `just ci` before every push
stays the rule, and costs less once it selects by the diff; CONTRIBUTING.md and the skills
keep their rules. All of it is m0, since every agent pays the cost now.

Each child reports its CPU saved, cold and warm, against `origin/main`.

## Required

- [A cheaper differential](/quest/m0/checks/differential.md) - fixtures' wasm builds at opt-level 1 under single-threaded Node, and `stable_names` reuses `lang`'s translation
- [Checks by the diff](/quest/m0/checks/dispatch.md) - `just ci` runs only the projects a change touches, as moq's `sh/dispatch.sh` does, with a nightly `just gate`
- [A warm gate](/quest/m0/checks/cache.md) - the land gate builds through mbx, as moq's does
- [Stable names on a small fixture](/quest/m0/checks/stable-names.md) - `stable_names` runs on a small fixture with generic instances and trait impls
- [Fixtures that merge](/quest/m0/checks/fixture-layout.md) - `lang` splits by module, and `reject`'s snapshots key by item, not line
