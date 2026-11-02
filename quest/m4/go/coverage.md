# [XL] moq-net beyond the codec

## Goal

rust2-go translates moq-net's model, as rust2-ts does, and its differential tests cover
each construct below against native Rust.

## Plan

The spike refuses each of these with its span (`go/fixtures/reject/expected.txt`):

- **`Drop`:** a `Drop` impl runs where Rust drops the value, in drop order, with drop
  flags where a value moves on some paths only. Translation now skips cleanup blocks, so
  a panic runs no drops, as `panic = "abort"` does.
- **Floats:** `f32` and `f64`, whose arithmetic Go matches; their formatting does not.
- **128-bit integers:** Go has none, so a runtime struct of two `uint64`s.
- **Trait objects** of traits other than `Display` and `Debug`, and unions.
- **Const generics,** such as an array's by-value iterator.
- **Pointers:** a wide pointer's `==`, which compares address and length, and any
  pointer's ordering, which Go lacks; and a constant allocation referenced at two offsets
  or types, which needs one Go variable and references derived into it by its layout.
- **`Vec::capacity`,** which Go's growth would make differ from Rust's.
- **std's collections:** `HashMap`, `BTreeMap`, `VecDeque`, translated from std's MIR or
  bound to the runtime as rust2-ts's are.

The translation targets the host, 64-bit `usize`; a 32-bit Go target needs its own
translation.
