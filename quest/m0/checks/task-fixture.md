# [S] The task fixture outside the gate

## Goal

The ts `task` differential passes on every host `just ci` runs on, as it does in the land
gate's container, so an agent's local `just ci` on `main` is green.

## Plan

Found by the 2026-11-01 checks audit: on `main` at `8e6a7219`, `task` fails on nixos2 and in
the fixer's container, alone or in parallel, with "35417 cases, 3059 panics, 628
differences", and passes in the gate. It made the fixer hand back #387, #393, #394, and #397.
Reproduce it, find what differs between the hosts (Node's version, threads, timers, the
environment), and fix the cause, never by loosening the comparison.

Public API: none.
