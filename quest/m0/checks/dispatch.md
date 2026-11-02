# [M] Checks by the diff

## Goal

`just ci` runs only what a change touches, against the base the land gate passes as
`QUEST_LAND_BASE` or `origin/main` locally, as moq's `sh/dispatch.sh` does, and `just gate`
runs the full suite nightly on `main`. A quest-only change runs `quest check` and the lints.

## Plan

Decided with the maintainer on 2026-11-01: copy moq's dispatch. Since the gate started,
147 of 201 PRs touched only ts and quest files, 25 only quest files, 2 uniffi, and 16 ffi;
uniffi has never failed in the gate. Shared inputs (the flake, toolchains, the root
`Cargo.*` and justfile, `.github`) select everything. Add `just ci-select` to show the
selection, and a nightly job running `just gate`, which the nightly lacks today. Update
CONTRIBUTING.md's description of the gate to say what moq's does.

Public API: none.
