# [M] Fixtures that merge

## Goal

`lang` splits into fixtures by its modules, sharing one wasm target dir, and `reject`'s
snapshots key each refusal by its item rather than its line number, so two PRs adding cases
rarely conflict and a targeted run builds only its module.

## Plan

Decided with the maintainer on 2026-11-01. Of 71 merge-conflict ejections in four days, 43
were in `lang` (31.7k lines, 108k lines of TS) and 28 in `reject`, and gate failure #374 was
`reject`'s line numbers shifting inside a batch. Each conflict costs a fixer run, a local
`just ci`, and another gate run.

Public API: none.
