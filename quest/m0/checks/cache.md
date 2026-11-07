# [XS] A warm gate

## Goal

The land gate's `just ci` builds through mbx, as moq's does, so a run rebuilds only what
changed: measured on nixos2, root clippy falls from 399 to 83 CPU-seconds warm, and uniffi's
test build from 291 to 20.

## Plan

Decided with the maintainer on 2026-11-01: mbx in a per-repository cache mount, as moq's
gate has. PR code can write to it, a risk moq already accepts; mbx keys each compilation on
its compiler, target, and inputs. Local worktrees share it too, where the dev shell already
can.

The dev shell puts mbx's Cargo shim ahead of its toolchains wherever `mbx setup` installed
one. That needs mbx 1.29.2 or later: older releases name a file in a build script's `OUT_DIR`
by mbx's placeholder in compile errors, which fails `uniffi_uitests`' trybuild snapshots
([jdx/mr-boxington#896](https://github.com/jdx/mr-boxington/issues/896)).

What remains is on the host: its mbx pin moves to 1.29.2 or later, and rust2's land instance
takes a per-repository mbx profile, as moq's has (maintainer, 2026-11-02).
[#426](https://github.com/moq-dev/rust2/pull/426)'s `## Host changes` lists both.

Check it by reading a gate run's log for mbx's cache summary.

Public API: none.
