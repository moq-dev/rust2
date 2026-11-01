# rust2 questline

## Goal

Keep the repository's living work organized as visible, versioned quests,
grouped into milestones ordered by priority.

## Plan

m0 holds urgent fixes. m1 is the work with a consumer waiting on it now:
the UniFFI fork moq switches to, the exporter spike that may replace it, and
the fork's perf work. m2 is research: the TypeScript translator, which moq's
browser package adopts once it matches the handwritten one, and the fork's gated or
speculative work. m3
holds the build-speed tracks, which the
[build-speed benchmark](/bench/build/README.md) rescopes before they start,
and speculative work. m4 holds long-term research.

Decided with the maintainer on 2026-10-23, when rust2 was planned from moq
([moq-dev/moq#6578](https://github.com/moq-dev/moq/pull/6578)):

- One repository with three projects and one setup, copied from moq's reusable
  parts: the nix flake and direnv, just, the quest tree, and agent docs and
  hooks.
- moq is the first consumer of each project, and pins rust2 as a flake input.
- The first quest that adds a crate also adds the Cargo workspace (moq's
  workspace lints, including a reason on every suppression) and the cargo
  steps of `just check`.

## Required

- [m0: immediate priorities](/quest/m0/README.md) - urgent fixes
- [m1: consumers waiting](/quest/m1/README.md) - the UniFFI fork moq switches to, the exporter spike, and the fork's perf work
- [m2: research](/quest/m2/README.md) - the TypeScript translator's spikes and features toward moq-net's model, and the fork's gated work
- [m3: deferred](/quest/m3/README.md) - the pure-Rust profile and upstream's build-speed levers, measured by the benchmark
- [m4: long-term research](/quest/m4/README.md) - ideas no consumer needs yet, such as a subset compiler
