# moq in pure Go

## Goal

[rust2-go](/go/README.md) translates moq-net to pure Go, with no cgo, close enough to the
exporter's [cgo bindings](/quest/m1/ffi-export/go.md) in speed that Go users can take it
in their place: `go get` installs it and it cross-compiles.

## Plan

Decided with the maintainer on 2026-11-01: Go users avoid cgo, which breaks `go get`'s
plain install and cross-compiling, so a pure-Go moq is worth researching. The cgo
exporter stays moq's Go route until a translation matches it.

The spike ([moq-dev/rust2#410](https://github.com/moq-dev/rust2/pull/410)) translated
moq's codec held to rust2-ts's bar: 35,273 codec cases and 117,953 integer cases match
native Rust, panics included. It runs 5 to 120 times slower than cgo on batched loops,
and beats it only on a single short call, where cgo's crossing dominates. Stripped, it
adds 324 KB to a Go binary where cgo adds 1.15 MB and a libc dependency.

## Required

- [Match cgo on the codec](/quest/m4/go/speed.md) - the translated codec within 2x of cgo on `just go bench`
- [moq-net beyond the codec](/quest/m4/go/coverage.md) - `Drop`, floats, 128-bit integers, trait objects, and collections, so moq-net translates whole
