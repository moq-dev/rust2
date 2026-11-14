# rust2

Rust to other places.

- **TypeScript**: an own rustc driver that translates a [documented Rust subset](ts/README.md) to TypeScript, without WASM.
- **C, then any language**: an [exporter](ffi/README.md) that turns a crate's `pub` API into a C ABI and header with no attributes, sharing the TypeScript driver's front end.
- **Public-visibility audit**: [rust2-pub](driver/pub/README.md), on the drivers' front end, finds the `pub` items no other crate needs and validates narrowing them.
- **Faster builds**: a reproducible [build-speed benchmark](bench/build/README.md) of upstream's levers on moq.

Everything here is early; the [quest tree](quest/README.md) holds the plan.

## Development

```bash
nix develop   # or direnv allow
just ci       # just check, then just test, as the merge queue runs them
```

## License

Licensed under either [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
