# [L] Python bindings

## Goal

The exporter generates a CPython extension for the crate, built into the same library, meeting
the [epic's](/quest/m1/ffi-export/README.md) rules for every generator, native futures
included. `tests/api.py` runs the shared script, and `moq/moq.py` runs a moq session.

## Plan

Decided with the maintainer on 2026-11-01:

- Python skips the C ABI, as Kotlin does through JNI: generated Rust glue on the CPython API,
  built with `--features python`. moq's
  [bench](/bench/ffi/README.md#boltffi-comparison) measured `noop()` at 834 ns through UniFFI's
  `ctypes` and 21.6 ns through BoltFFI's CPython extension. Not `ctypes` or `cffi`.
- The glue uses [pyo3-ffi](https://crates.io/crates/pyo3-ffi) and the stable ABI (`abi3`), one
  build for every supported CPython, unless `just ffi bench` measures the version-specific API
  faster by enough to pay for a build per version.
- The generated API is the crate's `pub` API in Python's idiom, as Kotlin's is, not UniFFI's.
  moq's `py/` packages adapt in moq's [switch](https://github.com/moq-dev/moq/blob/main/quest/m2/uniffi-fork.md).
- An object frees its Rust value when its last reference goes, and an explicit close or
  context manager frees it sooner.
- Futures are `asyncio` awaitables.

Public API: a new generator.
