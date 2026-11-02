# [S] A warm gate

## Goal

The land gate's `just ci` builds through mbx, as moq's does, so a run rebuilds only what
changed: measured on nixos2, root clippy falls from 399 to 83 CPU-seconds warm, and uniffi's
test build from 291 to 20.

## Plan

Decided with the maintainer on 2026-11-01: mbx in a per-repository cache mount, as moq's
gate has. PR code can write to it, a risk moq already accepts; mbx keys each compilation on
its compiler, target, and inputs. Local worktrees share it too, where the dev shell already
can.

Public API: none.
