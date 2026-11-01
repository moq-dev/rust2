// A pointer the translation can't follow, such as one a struct's field holds, fails with
// a rust2-ts error where JS keeps apart what Rust's reaches, which native Rust can't show:
// there each is defined, as `repr(C)` fields and a shared subslice's parent are one
// allocation, and a `Vec`'s capacity is past its length.
//
// usage: node pointers.ts

import { strict as assert } from "node:assert";

import * as rt from "../runtime/rt.ts";

const unsure = (f: () => void) => assert.throws(f, (e: Error) => !(e instanceof rt.Panic) && e.message.startsWith("rust2-ts: "));

// Two `&raw const x` of one variable, each its own `rt.Place`.
let x = 5;
const a = rt.place(() => x, (v) => (x = v));
const b = rt.place(() => x, (v) => (x = v));
unsure(() => rt.ptrOffsetFrom(a, b));
assert.equal(rt.ptrOffsetFrom(rt.ptrAdd(a, 1), a), 1);

// A `repr(C)` struct's fields, each a value in no array: reached from the other, and as a slice of both.
const pair = { low: 1, high: 2 };
const low = rt.place(() => pair.low, (v) => (pair.low = v));
const high = rt.place(() => pair.high, (v) => (pair.high = v));
unsure(() => rt.ptrOffsetFrom(high, low));
unsure(() => rt.ptrAdd(high, -1).v);
unsure(() => rt.ptrSlice(low, 2));
unsure(() => rt.ptrCopy(low, high, 2));

// A shared subslice, which is a copy: before it, past it, and against its parent.
const lanes = [1, 2, 3, 4];
const part = rt.sliceRange(lanes, 1, 3);
unsure(() => rt.ptrAdd(rt.sliceAsPtr(part), -1).v);
unsure(() => rt.ptrAdd(rt.sliceAsPtr(part), 2).v);
unsure(() => rt.ptrOffsetFrom(rt.sliceAsPtr(part), rt.sliceAsPtr(lanes)));
unsure(() => rt.ptrSlice(rt.sliceAsPtr(part), 3));
unsure(() => rt.ptrCopy(rt.sliceAsPtr(part), rt.sliceAsPtr(lanes), 3));
assert.equal(rt.ptrAdd(rt.sliceAsPtr(part), 1).v, 3);

// A byte in no array, as a raw or mutable slice, as a shared one past it, and the
// pointer of a shared one, a copy, against the byte's.
let byte = 3;
const lone = rt.place(() => byte, (v) => (byte = v));
unsure(() => rt.ptrByteSlice(lone, 1, false));
unsure(() => rt.ptrByteSlice(lone, 2, true));
const shared = rt.ptrByteSlice(lone, 1, true);
assert.deepEqual(shared.data, Uint8Array.of(3));
unsure(() => rt.ptrOffsetFrom(rt.byteSliceAsPtr(shared), lone));

// A write into a `Vec`'s capacity past its length, which a JS array would grow by.
const vec: number[] = [7];
unsure(() => {
	rt.ptrAdd(rt.sliceAsPtr(vec), 1).v = 8;
});
assert.equal(vec.length, 1);
