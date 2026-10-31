# [M] A pointer a callee takes to a part another function replaces

## Goal

[rust2-ts](/ts/README.md) refuses, with its span, a raw pointer to a whole value that a
callee took through a reference to a part of a variable, where another function then
puts another object in that part, as `mem::replace(&mut w.part, ..)` does. A translated
program never silently reads the old object.

## Plan

Left over from [A pointer a callee takes to a variable that gets another object](https://github.com/moq-dev/rust2/pull/350),
which followed the maintainer's pick of 2026-10-31: judge by type, record only pointers
that may outlive the call, and refuse at the caller's borrow when the caller assigns the
variable. A part replaced in another function is no assignment the caller makes, and
the callee's pointer has no projection past its last `Deref` for `out_of_step` in
`ts/src/lower/mir/place.rs` to match against `Writes::holders`:

```rust
struct Whole { cell: UnsafeCell<u16> }
fn whole(c: &UnsafeCell<u16>) -> *mut u16 { c.get() }
fn reset(w: &mut Whole) { drop(std::mem::replace(&mut w.cell, UnsafeCell::new(0))) }
let mut w = Whole { cell: UnsafeCell::new(n) };
let p = whole(&w.cell);
reset(&mut w);
unsafe { *p } // `n` in JS; `0` natively, if the read is defined
```

Reproduce first with a differential test against native wasm32, and check the native
read under Miri's Tree Borrows. A pointer through a `&mut` or a `&` to a type without an
`UnsafeCell` is disabled by `reset`'s write, so only `rt.Cell`s may be defined; if the read
is undefined for those too, delete this quest.
