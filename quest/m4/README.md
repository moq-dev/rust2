# m4: long-term research

## Goal

Ideas worth recording that no consumer needs yet, each kept as a quest so its
blockers and evidence are not lost.

## Plan

Decided with the maintainer on 2026-10-25: m4 holds long-term research, and
opens only when the work before it leaves room or new evidence revives an idea.

Declined on 2026-10-25: a drop-in build-cache wrapper with an early cutoff.
The cutoff is unsound on stock rustc (a stale SVH gives E0460, and DefIndex
shifts), mbx and cargo already cover caching and GC, and a sound cutoff holds on
only 7% of moq-net edits
([moq-dev/rust2#62](https://github.com/moq-dev/rust2/pull/62),
[moq-dev/rust2#70](https://github.com/moq-dev/rust2/pull/70)).
[The fork](/quest/m4/fork.md) collects the reasons to change rustc and cargo instead.

## Required

- [A subset compiler](/quest/m4/subset-compiler.md) - whether a compiler for a stricter Rust subset can beat rustc on moq's builds, given the verdict that closed it before
- [Waiting: a rustc and cargo fork](/quest/m4/fork.md) - the evidence ledger for carrying a fork; condition: enough measured reasons to pay for one
- [Spike: moq's codec in pure Go](/quest/m4/go-translator.md) - whether translating moq to Go, with no cgo, can replace the cgo bindings for Go users
