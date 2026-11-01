# [L] Dart bindings

## Goal

The exporter generates Dart over its C ABI through `dart:ffi`, meeting the
[epic's](/quest/m1/ffi-export/README.md) rules for every generator, native futures included.
`tests/api.dart` runs the shared script, and `moq/moq.dart` runs a moq session.

## Plan

Decided with the maintainer on 2026-11-01:

- The generated API is the crate's `pub` API in Dart's idiom, as Kotlin's is, not UniFFI's.
  moq's `dart/` packages adapt in moq's [switch](https://github.com/moq-dev/moq/blob/main/quest/m2/uniffi-fork.md).
- An object frees its Rust value in an explicit close, with a `NativeFinalizer` as the
  backstop for one never closed.
- Futures are Dart `Future`s, completed on the isolate that made them.

Public API: a new generator.
