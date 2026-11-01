# [M] Translate moq-net without moq-wasm

## Goal

`just ts moq` and `just ts census` translate moq-net from moq's `main`, and the translator
and the exporter share one moq pin again ([#390](https://github.com/moq-dev/rust2/pull/390)
holds until then). moq dropped WASM in
[moq-dev/moq#7475](https://github.com/moq-dev/moq/pull/7475), and with it moq-wasm, which
picked the browser's randomness for moq-net's `rand` and `uuid` on wasm32. Without it,
getrandom and uuid refuse to build there, so the translator's moq pin cannot move past it.

## Plan

The browser's randomness is a platform boundary, so the driver may pick it for a stock
crate, as a wasm32 binary would. Prefer what getrandom and uuid document for wasm32 over a
stand-in crate.

`just ts bench`'s `moq-wasm package` row has no source past that pin either: decide with
the maintainer whether it goes, or stays on the last revision with moq-wasm. The
[model's](/quest/m2/ts-model/README.md) comparison against moq's WASM build may change
with it.
