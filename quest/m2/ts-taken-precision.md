# [M] Borrows refused for a pointer no callee keeps there

## Goal

[rust2-ts](/ts/README.md) refuses a borrow of a variable assigned again only where a
raw pointer taken through that borrow may still be read, so moq's census holds none of
the 187 needless `a reference to _, through which a raw pointer may be made` refusals
left at the pinned moq revision: 178 in moq-net, 8 in the model, and 1 in kio. A
translated program never silently reads the old object where Rust defines the read.

## Plan

[PR #355](https://github.com/moq-dev/rust2/pull/355) stopped counting what std's reads
load through a taken pointer as holding it, which cleared smallvec's `Drain::next`
closure over type parameters (7 of the first 194). Measured there on 2026-10-31:

- Every remaining borrow is refused because `Trace` loses its reference: into std's
  generic code over the crate's types (`mem::take`, `Option::is_none`), callees read
  over their own type parameters, closures, and drops. Refusing only where the
  reference may reach a pointer's maker, followed as `Trace::keeps` follows calls,
  clears none of them.
- 170 in moq-net hold a `[u8; N]`, and appear only because its whole-crate census binds
  no flate (the model's config does). zlib-rs's `Deflate::compress_uninit` nulls
  `next_in` and `next_out` before it returns, but `Inflate::decompress` copies them into
  its heap state (`bit_reader`, `writer`), which keeps them until the next call writes
  them again. arrayvec's `ArrayVecImpl::truncate` takes `[u8]` too, through
  `drop_in_place` over a type parameter that `ArrayVec<u8, _>` instantiates.
- 17 hold a `SmallVec`: `inline`, `inline_mut`, and `drain` return pointers their
  callers use within the borrow, and its `Drop` gives `&mut self[..]` to `drop_in_place`
  over a type parameter. Clearing them needs judging a callee per instance, std's
  bodies included.

Decision for the maintainer. Under Miri, reading such a pointer after the caller's
reassignment is undefined in Stacked Borrows, and in Tree Borrows unless the part read is
in an `UnsafeCell`: the reassignment disables a pointer made through a `&` or `&mut` to
anything else. [ts-unsafe](/quest/m3/ts-unsafe.md) says undefined behavior need not be
reproduced, and [ts-pointer-part-replaced](/quest/m2/ts-pointer-part-replaced.md) already
judges by Tree Borrows.

1. Recommended: take only parts that hold an `UnsafeCell`. Measured: 3 refusals left in
   moq-net, none in the model or kio, and no other construct moves. Six of #350's reject
   cases translate (`Owned`, `Stored`, `Passed`, `Counter`, `Core`, `Picked`), all
   undefined in both models, so their reject cases move to an `UnsafeCell`.
2. Keep refusing undefined reads too, and make `Trace` judge callees per instance and
   prove zlib-rs's kept pointers written before any read across calls.
