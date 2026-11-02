# rust2-go

A rustc driver that translates a Rust crate to pure Go: no cgo, one Go package, with the
semantics of the Rust source. It is a spike, measuring whether Go users are better served by
a translation than by the C ABI [rust2-ffi](../ffi/README.md) exports, on moq's codec.

```bash
rust2-go --config go.toml --out dir [--manifest-path Cargo.toml] [-- cargo args]
```

```toml
# go.toml: the Go package's name, and the functions it exports by their paths from the
# crate root, `harness::run` as `HarnessRun`.
package = "codec"
roots = ["harness::run"]
```

It runs `cargo check` on the host with itself as `RUSTC_WRAPPER`, through the front end
it shares with [rust2-ts](../ts/README.md) in [`driver/`](../driver), building std from
source so every std function has MIR. From each root it translates every function
instance reached, monomorphized, and every type they name, into one `.go` file per crate
and the runtime in `runtime/`, then runs `gofmt`. What it can't translate is a compile
error at its span, and nothing is written.

## How Rust becomes Go

| Rust | Go |
| --- | --- |
| a function instance | a function, its MIR blocks as labels and `goto`, its locals as variables |
| `u8`..`u64`, `i8`..`i64`, `usize`, `isize` | `uint8`..`uint64`, `int8`..`int64`, `uint`, `int` (64-bit hosts) |
| an overflow check, a bounds check | native Rust's panic message, as a `RustPanic` |
| a struct, a tuple | a struct, generic where Rust's is; `tuple2[A, B]` and so on |
| an enum | a struct of its tag and a field per variant with fields |
| `&T`, `Box<T>` | `*T` |
| `&[T]`, `&str`, `Vec<T>` | `[]T`, `[]uint8`, `[]T` |
| a closure | a struct of its captures |
| `dyn Display`, `dyn Debug` | a `func(*Formatter) fmtResult` |
| a `tracing` event | nothing, as with no subscriber |

std translates from its own MIR, down to bindings for what Go does itself: a `Vec`'s
growth, a slice's iterator, UTF-8 validation, and the formatting machinery, which
`runtime/rt_fmt.go` ports from core so `{:?}` and `{:#?}` print as Rust's do.

A panic is a Go panic of `RustPanic` carrying native Rust's message. As with
`panic = "abort"`, no cleanup runs on the way out, so a caller that recovers one must
drop what the translation held.

## Refused

`fixtures/reject/expected.txt` lists them with their spans: 128-bit integers, floats, a
`Drop` impl, a trait object of any trait but `Display` and `Debug`, a union, and a type
generic over a constant, such as an array's by-value iterator.

## Tests

`just go test` translates each fixture, runs `go vet` on it, and runs it against native
Rust on the same inputs, failing on any difference in output or panic message:
`fixtures/codec` is rust2-ts's harness over moq's codec, and `fixtures/ints` every
integer width's arithmetic, overflow, division by zero, and shifts.

`just go bench` times the codec's workloads translated against the same crate exported by
rust2-ffi and called through cgo.
