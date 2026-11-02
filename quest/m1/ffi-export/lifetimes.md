# [M] Lifetimes with no undefined behavior

## Goal

No generated binding in a garbage-collected or reference-counted language lets a program
reach undefined behavior through an object or its bytes: Swift and Python have no `close()`,
Kotlin's and Go's `close()` leave every later call an error, and returned bytes are owned by
the language's memory manager, so no view outlives its buffer. C++ owns handles through the
standard library's smart pointers. Each rule is proven by a test that uses an object after
its free, races a free against a call, and keeps a view past its owner.

## Plan

Decided with the maintainer on 2026-11-01, from an audit of each generator: avoid an
explicit `close()` where the language frees deterministically, since without Rust's
`close(self)` a program can keep a reference to a closed object. A `close()` stays only
where the language's GC can't free promptly, and then every call after it returns or throws
an error, never a crash or undefined behavior.

- **Swift**: drop `close()`. ARC frees in `deinit`, so no closed state exists; today a call
  after `close()` traps with `fatalError`. Its `Data(bytesNoCopy:)` bytes are already safe.
- **Python**: drop `close()` and `with`; CPython's refcount frees in `tp_dealloc`. Its bytes,
  a `memoryview` over a refcounted owner, are already safe. Lands after
  [#407](https://github.com/moq-dev/rust2/pull/407).
- **Kotlin**: objects keep `close()`, which already throws `IllegalStateException` after, and
  a racing close waits for the call. Its bytes over 1 KiB, a direct `ByteBuffer` that
  `Bytes.close()` or the GC can free under a live slice, become owned by the GC as Go's are:
  no `close()`, freed by a cleaner on the buffer that its slices and views keep alive. Verify
  that on ART at API 24 and 35, or else read them through methods with no buffer escaping.
- **Go**: settled in [#411](https://github.com/moq-dev/rust2/pull/411): objects keep `Close`,
  with every later call returning an error, and bytes are a GC-owned `Bytes` with no raw
  slice, read through methods and copied by `Copy()`.
- **C++**: an object owns its handle through `std::unique_ptr` with a deleter calling
  `<obj>_free`, and a `Clone` type through `std::shared_ptr`, so a copy shares one Rust handle
  without crossing into Rust. Sharing must not differ from the crate's `clone`: a type whose
  `clone` makes an independent value, which a `&mut self` method or interior mutability can
  then tell apart, keeps an explicit `clone()` that crosses. The header keeps C++17; where C++20's `std::span` is available,
  `view` converts to `std::span<const std::uint8_t>`, the standard's non-owning view of
  contiguous bytes. A dangling reference stays C++'s own undefined behavior.
- **C**: out of scope; its `_free` follows C's norms.

Public API: breaking for Swift, Python, Kotlin bytes, and C++ ownership; each gets a
`MIGRATE.md` entry.
