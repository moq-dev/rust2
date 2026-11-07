# Unsafe code the translator can model

## Goal

[rust2-ts](/ts/README.md) translates every `unsafe` construct it can give exact semantics
in JS, each proven by differential tests against native Rust, so a stock crate's `unsafe`
never needs a hand binding or a rewrite in its consumer.

## Plan

Decided with the maintainer on 2026-10-27: a dependency containing `unsafe` is bound by
the consumer [at first](/ts/README.md#bindings), a stopgap, since a stock crate should
translate with no annotations or config; this epic is what retires it. Decided on 2026-11-07
in [the model epic](/quest/m2/ts-model/README.md): moq writes no `cfg(rust2_ts)` twins of
its own `unsafe`.

Decided with the maintainer on 2026-10-28, typed pointers: a raw pointer is what its
reference is, the object, an `rs.Place`, or `rs.NULL`, and a pointer into a slice or `Vec`
is `{ array, index }`. Byte reinterpretation, pointer-to-integer casts, and raw allocation
are refused with a span. [#201](https://github.com/moq-dev/rust2/pull/201) landed `Box`
to raw and back, `NonNull`, `MaybeUninit`, raw reads and writes, `get_unchecked`, and
unions whose overlapping fields share a type, which took hashlink from 375 refusals to
none of its own `unsafe`.

- An `unsafe` operation outside the modeled set is refused with its span.
- Undefined behavior in Rust need not be reproduced, but a translated program never
  silently differs from native where Rust's behavior is defined.
- Element pointers into arrays of values JS copies are `rs.AtIndex`, `{ array, index }`,
  which `add`, `offset`, `offset_from`, `ptr::copy`, and `slice::from_raw_parts` act on.
  Decided with the maintainer on 2026-11-01: where one body shows a pointer crossing what
  JS keeps apart (a value in no array), translation refuses it with a span. A pointer to
  an element JS keeps as an object, such as a tuple or the crate's struct, is that object,
  which knows no array it is in, so arithmetic on one stays refused: hashbrown's buckets
  and kio's `log.rs` reach it.
- Decided with the maintainer on 2026-11-01, typed allocation: memory from `alloc` for a
  `Layout::array::<T>(n)`, or an equal layout of `T`'s size and alignment, cast to `*mut T`
  is a fresh JS array of `T`s, which `realloc` copies and `dealloc` frees. Untyped and
  byte allocation stay refused. smallvec spills to it, and
  [`Vec::from_raw_parts` of it](/quest/m3/ts-unsafe/vec-from-raw-parts.md) is left.
- Decided with the maintainer on 2026-11-02: a raw slice pointer is `rs.RawSlice`, its
  first element's pointer and its length, read through only where Rust reads, and an empty
  slice from a pointer into no array keeps that pointer through every part of it.

Decided with the maintainer on 2026-11-06, splitting this quest into one child per
construct still open at `b3df54b6c`, all in m3:

- ✅ Goal: every `unsafe` construct rust2-ts can make exact, one quest each.
  Rejected: only the constructs moq's dependencies hit.
- ✅ All children in m3. Rejected: spare capacity in m2; smallvec's constructs in m2.
  No child blocks moq: the model's only hits are smallvec's (7 of its 10 refusals), and
  moq drops or binds smallvec.
- `Vec::from_raw_parts` of a typed allocation is its own child, restored on review of
  [#618](https://github.com/moq-dev/rust2/pull/618): the spare-capacity child fills an
  existing `Vec`, not one adopting an allocation.
- The 2026-11-01 clauses that let a pointer from elsewhere, a `Vec`'s spare capacity, and
  a byte pointer past its view fail loudly at run time are superseded by
  [Throw only where Rust panics](/quest/m3/ts-runtime-throws.md) (2026-11-04, #597): where
  Rust defines the access it translates exactly or is refused with a span, and only a
  throw guarding UB stays, as a precondition abort.
- Done and dropped: a byte pointer past its own `Uint8Array` view. Byte pointers index the
  whole buffer since [#400](https://github.com/moq-dev/rust2/pull/400) and
  [#576](https://github.com/moq-dev/rust2/pull/576), and a byte `Vec` keeps its
  capacity since [#555](https://github.com/moq-dev/rust2/pull/555) and
  [#568](https://github.com/moq-dev/rust2/pull/568). `bytes`' `Buf::get_u16` and wider
  map to the runtime since [#447](https://github.com/moq-dev/rust2/pull/447).

Census at the moq pin on 2026-11-07: model 45, kio 72, moq-net 347 refusals.

Public API: the translator's output.

## Required

- [A Vec's spare capacity](/quest/m3/ts-unsafe/vec-spare-capacity.md) - writes through `as_mut_ptr` past the length, committed by `set_len`, translate for any `Vec<T>`
- [Vec::from_raw_parts of a typed allocation](/quest/m3/ts-unsafe/vec-from-raw-parts.md) - a `Vec` adopts an `alloc`ed array of `T`s, with its length, capacity, and drops
- [Unions over a type parameter](/quest/m3/ts-unsafe/generic-unions.md) - each instance of a generic union is laid out with its own offsets
- [Vec::drain from std's MIR](/quest/m3/ts-unsafe/vec-drain-mir.md) - std's `Drain` translates, replacing the runtime's port
- [BinaryHeap from std's MIR](/quest/m3/ts-unsafe/binary-heap-mir.md) - std's `BinaryHeap` and `PeekMut` translate, replacing the runtime's port
- [A slice's pointer cast to an array's](/quest/m3/ts-unsafe/array-casts.md) - `*(s as *const [u8] as *const [u8; N])` in a crate's own code, and `IoSlice`
- [Unchecked integer operations](/quest/m3/ts-unsafe/unchecked-ops.md) - `unchecked_*` and `NonZero::new_unchecked` translate, aborting on a violated precondition
- [UnsafeCell's raw accessors](/quest/m3/ts-unsafe/unsafecell.md) - `UnsafeCell::raw_get`, `from_mut`, and `replace` translate
- [A Box of one uninitialized value](/quest/m3/ts-unsafe/box-new-uninit.md) - `Box::new_uninit` and `new_zeroed` for one value translate, through `write` and `assume_init`
