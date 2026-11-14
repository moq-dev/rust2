# [S] Delete the exporter's config

## Goal

`just ffi census` lists no refusals at the moq pin, and [rust2-ffi](/ffi/README.md) reads
no config: an item it cannot export fails the export with a span.

Deleted: `ffi/moq/ffi.toml`, the `--config` flag, `RUST2_FFI_CONFIG`, and `config.rs`'s
`Config` and `exclude`, with the fixtures' `ffi/fixtures/api/ffi.toml` and
`ffi/fixtures/reject/ffi.toml` and the exclude tests in `ffi/tests/fixtures.rs`;
`ffi/README.md` and the recipes stop naming them. `config.rs`'s `Package` and environment
constants stay. Also deleted: the `--config` parsing rust2-ffi takes from
`driver/src/cargo.rs`, which already has `parse_configless`; `go/bench/ffi.toml` and the
`--config` in `go/justfile`'s bench recipe, whose `package` and `prefix` keys `config.rs`
already rejects; and "exclude it in the config" in `ffi/src/driver.rs`'s refusal notes and
`ffi/fixtures/reject/expected.txt`.

## Plan

[The epic](/quest/m1/ffi-refusals/README.md) holds the decisions: configless since
2026-11-04, and no exclude stands in for a rule. This is its last child, split out by the
2026-11-12 audit so the epic needs no `Required` of its own.

One deletion needs no clean census and may land in any sibling's PR first:
`go/bench/ffi.toml` with the `--config` in `go/justfile`'s bench recipe.

Public API: the `--config` flag and `RUST2_FFI_CONFIG` are removed (breaking for a caller
that passes them).

## Required

- [The moq pin bump](/quest/m1/ffi-refusals/moq-pin.md) - the pin is at moq's `main`, where the census is taken
- [`IpAddr`](/quest/m1/ffi-refusals/ip-addr.md) - exports `quic::demux::Datagram::local`, the last item moq's main refuses beyond not-send's
- [Not Send](/quest/m1/ffi-export/not-send.md) - exports the three types the census still refuses for not being `Send`
