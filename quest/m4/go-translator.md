# [L] Spike: moq's codec in pure Go

## Goal

A spike translates moq's codec from its MIR to pure Go, with no cgo, as
[rust2-ts](/ts/README.md) began, and records whether a Go translator should follow:
differential tests against native Rust pass, and a Go benchmark measures the translation
against the [exporter's cgo bindings](/ffi/README.md#go) on the same workloads.

## Plan

Decided with the maintainer on 2026-11-01: Go users avoid cgo, which breaks `go get`'s
plain install and cross-compiling, so a pure-Go moq is worth researching. It is research
in m4; the cgo exporter stays moq's Go route until a translation matches it.

- The translation is held to rust2-ts's bar: integer widths and overflow, drop order,
  panics, and error paths match native, and an unsupported construct is refused with its
  span.
- Go has every Rust integer width and wraps natively, so overflow checks are the only
  arithmetic it adds; MIR's monomorphized instances avoid Go's limited generics.
- Whether Go shares rust2-ts's front end (MIR lowering, std's mapping, the census) is the
  spike's to measure: report how much of `ts/src` is language-neutral, and propose a shared
  crate only if Go needs one.
- Report go or no-go with the evidence: what translated, what was refused and why, the
  benchmark, and the generated size against the cgo bindings'.

Public API: none until a translator lands.
