# [L] Swift bindings

## Goal

The exporter generates Swift over its C ABI, imported as a clang module, meeting the
[epic's](/quest/m1/ffi-export/README.md) rules for every generator, native futures included.
`tests/api.swift` runs the shared script on Linux in the merge gate and on Apple's toolchain
in the macOS nightly, and `moq/moq.swift` runs a moq session.

## Plan

Decided with the maintainer on 2026-11-01:

- The generated API is the crate's `pub` API in Swift's idiom, as Kotlin's is, not UniFFI's.
  moq's `swift/` package adapts in moq's [switch](https://github.com/moq-dev/moq/blob/main/quest/m2/uniffi-fork.md).
- An object frees its Rust value in `deinit`, and an explicit close frees it sooner.
- Futures are Swift concurrency's `async`, with a cancelled task cancelling its C ABI future.
- Tests run with the Swift from nixpkgs in the merge gate, and on macOS in the nightly, as
  C++ does with Apple Clang. Swift's aarch64-linux gap is
  [its own quest](/quest/m2/uniffi-swift-arm.md).

Public API: a new generator.
