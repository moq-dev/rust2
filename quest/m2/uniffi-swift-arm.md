# [XS] Waiting: Swift 6.4 in nixpkgs-unstable

## Goal

The nightly `uniffi-fixtures` job runs on `ubuntu-24.04-arm` again, and
`nix develop .#uniffi` evaluates on aarch64-linux. Check: the latest
`nixpkgs-unstable` carries Swift 6.4.0
([NixOS/nixpkgs#574002](https://github.com/NixOS/nixpkgs/pull/574002), merged to
`staging`), and `nix build --dry-run .#devShells.aarch64-linux.uniffi` succeeds
after a bump.

## Plan

Decided with the maintainer on 2026-10-26, from
[#93](https://github.com/moq-dev/rust2/pull/93): swiftc 6.3.3 is marked broken on
aarch64-linux in the pinned nixpkgs, where its gold link fails, so the job moved to
x86_64. Then bump the `nixpkgs` input in its own change, move the job back, and
drop the workflow comment. The default shell then has Swift on aarch64-linux too, so
drop rust2-ffi's skips there as well: `api_swift`'s `ignore`, and the Swift's in
`just ffi moq` and `just ffi bench`. Never allow the broken package.

Public API: none.
