# [L] Go bindings

## Goal

The exporter generates Go over its C ABI through cgo, meeting the
[epic's](/quest/m1/ffi-export/README.md) rules for every generator, native futures included.
`tests/api.go` runs the shared script, and `moq/moq.go` runs a moq session.

## Plan

Decided with the maintainer on 2026-11-01:

- The generated API is the crate's `pub` API in Go's idiom, as Kotlin's is, not UniFFI's.
  moq's `go/` module adapts in moq's [switch](https://github.com/moq-dev/moq/blob/main/quest/m2/uniffi-fork.md).
- An object frees its Rust value in `Close`, with `runtime.AddCleanup` as the backstop for
  one never closed.
- The epic settles a cancel's semantics and each driver's goroutine.

Public API: a new generator.
