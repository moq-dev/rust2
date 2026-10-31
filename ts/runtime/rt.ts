// The runtime of rust2-ts output: what Rust's std does that plain JS does not.
//
// Every operation is its own exported function, named by what it acts on, `sliceRange`
// or `fmtPad`, so a bundler drops every one translated code never calls. Methods are
// left only where a call dispatches on its receiver: an iterator's `next`, a writer's
// `writeStr`, a future's `poll`, a waker's, and `[Symbol.dispose]`.

import type { Ordering } from "./rt/collections.ts";
import type { ParseFloatError } from "./rt/float.ts";
import { type char, type ParseCharError, type ParseIntError, type TryFromIntError, type u32, type usize, u8Debug, usizeDebug, usizeDisplay } from "./rt/int.ts";
import type { Iter, Range, RangeInclusive } from "./rt/iter.ts";
import { drop as dispose, dropAll, type Pending } from "./rt/task.ts";

import { AtIndex } from "./rt/collections.ts";
import { UNPRINTABLE } from "./rt/unicode.ts";
export * from "./rt/ascii.ts";
export * from "./rt/bytes.ts";
export * from "./rt/collections.ts";
export * from "./rt/float.ts";
export * from "./rt/hash.ts";
export * from "./rt/int.ts";
export * from "./rt/iter.ts";
export * from "./rt/rand.ts";
export * from "./rt/str.ts";
export * from "./rt/task.ts";
export * from "./rt/time.ts";

/** A Rust panic. */
export class Panic extends Error {
	override name = "Panic";
}

/** Panic with `message`. */
export function panic(message: string): never {
	throw new Panic(message);
}

/**
 * Panic with a formatted message, which keeps what was written before a value failed to
 * format, as std's does.
 */
export function panicFmt(message: Arguments): never {
	const out = new StringWriter();
	write(out, message);
	throw new Panic(out.s);
}

/** A `match` without arms, on a value Rust proved can't exist, such as a `(Void, u8)`. */
export function unreachable(_value: unknown): never {
	throw new Panic("internal error: entered unreachable code");
}

/** `assert_eq!` or `assert_ne!` failing: `op` is `==` or `!=`. */
export function assertFailed(op: string, left: Fmt, right: Fmt, args?: Arguments): never {
	const failed: Arguments = args === undefined ? [" failed"] : [" failed: ", (f) => fmtWriteFmt(f, args)];
	panicFmt(["assertion `left ", op, " right`", ...failed, "\n  left: ", left, "\n right: ", right]);
}

// Places

/**
 * A `&mut` to a value JS copies, such as a number, which no JS value can point at: where
 * the value is, read and written through `v`.
 */
export interface Place<T> {
	v: T;
}

/** A place read and written by `get` and `set`: a local's, a field's, or an element's. */
class Accessed<T> implements Place<T> {
	private readonly read: () => T;
	private readonly write: (value: T) => void;

	constructor(read: () => T, write: (value: T) => void) {
		this.read = read;
		this.write = write;
	}

	get v(): T {
		return this.read();
	}

	set v(value: T) {
		this.write(value);
	}
}

/**
 * The payload's place of the `Option` at `p`, whose `None` is `none`: `p` itself, as
 * `Some(x)` is `x`, or `undefined` while it is `None`.
 */
export function optionAsMut<T>(p: Place<T | undefined>): Place<T> | undefined;
export function optionAsMut<T>(p: Place<T | None>, none: None): Place<T> | undefined;
export function optionAsMut<T>(p: Place<T | undefined | None>, none?: None): Place<T> | undefined {
	return p.v === none ? undefined : (p as Place<T>);
}

/** The place `get` reads and `set` writes. */
export function place<T>(get: () => T, set: (value: T) => void): Place<T> {
	return new Accessed(get, set);
}

// Raw pointers

/** A null raw pointer, of any type: nothing reads through it. */
export const NULL: never = null as never;

/** A dangling raw pointer, `NonNull::dangling`, of any type: nothing reads through it. */
export const DANGLING: never = /* @__PURE__ */ Object.freeze({}) as never;

/** A dangling pointer to `()`, std's every one: a place that holds only `()`. */
export const DANGLING_UNIT: Place<undefined> = { v: undefined };

/**
 * `ptr::write` of `value` through a pointer to an object: `value` copied into it, one
 * object deep, so every pointer to it sees the write.
 */
export function ptrWrite<T extends object>(p: T, value: T): void {
	if (ArrayBuffer.isView(p)) (p as unknown as Uint8Array).set(value as unknown as Uint8Array);
	else Object.assign(p, value);
}

/**
 * `mem::replace` of what a `&mut` to an object points at, in place: `value`'s fields, or
 * its items for a JS array, moved into `p`, and what `p` held returned as an object of its own.
 */
export function replaceIn<T extends object>(p: T, value: T): T {
	if (Array.isArray(p)) {
		const old = p.slice() as T;
		vecAssign(p, value as unknown[]);
		return old;
	}
	const old = Object.assign(Object.create(Object.getPrototypeOf(p)) as T, p);
	Object.assign(p, value);
	return old;
}

// Bindings

/**
 * `T` if it can be a bound Rust type's value, or else why not: never `undefined` or
 * `Pending`, which `None` and `Poll::Pending` are, an object unless Rust copies it, and
 * disposable exactly when it drops.
 */
export type Bound<T, Copy extends boolean, Drop extends boolean> = [undefined] extends [T]
	? "a bound value can't be undefined, which None is"
	: [typeof Pending] extends [T]
		? "a bound value can't be Pending, which Poll::Pending is"
		: [Copy, T] extends [false, object]
			? Disposes<T, Drop>
			: Copy extends true
				? Disposes<T, Drop>
				: "a bound value Rust doesn't copy must be an object, which a reference changes in place";

/** `T` if it has a `[Symbol.dispose]` exactly when `Drop`, or else why not. */
type Disposes<T, Drop extends boolean> = Drop extends true
	? [T] extends [Disposable]
		? T
		: "a bound value that drops must have a [Symbol.dispose]"
	: [T] extends [{ [Symbol.dispose]: unknown }]
		? "a bound value with a [Symbol.dispose] must bind with drop = true"
		: T;

// Results

/** `Result<T, E>`. */
export type Result<T, E> = { readonly ok: true; readonly value: T } | { readonly ok: false; readonly error: E };

/** `Ok(value)`. */
export function Ok<T, E = never>(value: T): Result<T, E> {
	return { ok: true, value };
}

/** `Err(error)`. */
export function Err<E, T = never>(error: E): Result<T, E> {
	return { ok: false, error };
}

/** `Ok(())`, one frozen object every evaluation shares, as nothing can change it. */
export const OK: Result<undefined, never> = /* @__PURE__ */ Object.freeze({ ok: true, value: undefined });

/** `Err(())`, shared as `OK` is. */
export const ERR: Result<never, undefined> = /* @__PURE__ */ Object.freeze({ ok: false, error: undefined });

/** Panic with `message` and `value`'s `Debug`. */
function unwrapFailed<T>(message: string, value: T, debug: Debug<T>): never {
	panicFmt([message, ": ", (f) => debug(value, f)]);
}

// A function that may drop an operand std's would, rather than return it, takes what
// drops it, and drops it where std's does: as it returns, or after any closure it calls.

/** `unwrap`, which shows the error with `debug`. */
export function resultUnwrap<T, E>(r: Result<T, E>, debug: Debug<E>): T {
	if (!r.ok) unwrapFailed("called `Result::unwrap()` on an `Err` value", r.error, debug);
	return r.value;
}

/** `Result::expect`: the value, or a panic with `message` and the error's `Debug`. */
export function resultExpect<T, E>(r: Result<T, E>, message: string, debug: Debug<E>): T {
	if (!r.ok) unwrapFailed(message, r.error, debug);
	return r.value;
}

/** `Result::unwrap_or_else`: the value, or what `f` makes of the error. */
export function resultUnwrapOrElse<T, E>(r: Result<T, E>, f: (error: E) => T): T {
	return r.ok ? r.value : f(r.error);
}

/** `Result::unwrap_err`: the error, or a panic with the value's `Debug`. */
export function resultUnwrapErr<T, E>(r: Result<T, E>, debug: Debug<T>): E {
	if (r.ok) unwrapFailed("called `Result::unwrap_err()` on an `Ok` value", r.value, debug);
	return r.error;
}

// Options, which are `T | undefined`, or `T | None` where `T` can already be `undefined`.
// A function taking one takes that `None` after its own arguments.

/** `None` of an `Option` whose `Some` can already be `undefined`, as `Option<()>`'s is. */
export const None: unique symbol = /* @__PURE__ */ Symbol("None");

/** The type of `None`. */
export type None = typeof None;

/** `f` of what `o` holds, or `None`: `Option::map` of a payload that is never `undefined`. */
export function optionMap<T, U>(o: T | undefined, f: (value: T) => U): U | undefined {
	return o === undefined ? undefined : f(o);
}

/** `Option::unwrap`: the value, or a panic. */
export function optionUnwrap<T>(o: T | undefined): T;
export function optionUnwrap<T>(o: T | None, none: None): T;
export function optionUnwrap<T>(o: T | undefined | None, none?: None): T {
	if (o === none) panic("called `Option::unwrap()` on a `None` value");
	return o as T;
}

/** `Option::expect`: the value, or a panic with `message`. */
export function optionExpect<T>(o: T | undefined, message: string): T;
export function optionExpect<T>(o: T | None, message: string, none: None): T;
export function optionExpect<T>(o: T | undefined | None, message: string, none?: None): T {
	if (o === none) panic(message);
	return o as T;
}

// Slices: arrays, but for bytes, which are `ByteSlice`s.

/** A bounds-checked index into a slice. */
export function idx(slice: { readonly length: number }, index: number): number {
	if (index >= slice.length) panic(`index out of bounds: the len is ${slice.length} but the index is ${index}`);
	return index;
}

/** A bounds-checked read of a slice. */
export function at<T>(slice: ArrayLike<T>, index: number): T {
	return slice[idx(slice, index)];
}

/** Panic unless `start..end` is in a slice of `length`, as std's range checks do. */
export function rangeCheck(length: number, start: number, end: number): void {
	if (start > length) panic(`range start index ${start} out of range for slice of length ${length}`);
	if (end > length) panic(`range end index ${end} out of range for slice of length ${length}`);
	if (start > end) panic(`slice index starts at ${start} but ends at ${end}`);
}

/** `&s[start..end]`, with `end` defaulting to the length. */
export function sliceRange<T>(s: T[], start: number, end: number = s.length): T[] {
	rangeCheck(s.length, start, end);
	return s.slice(start, end);
}

/**
 * `s[start..end]` lent mutably: an array whose every element is `s`'s, read and written
 * where it is, as a JS array has no views of its own. Its own array, holes of the same
 * length, answers for the rest, such as `length` and the methods. A view of a view reads
 * the array beneath both.
 */
export function sliceView<T>(s: T[], start: number, end: number): T[] {
	const outer = (s as { [VIEW]?: View<T> })[VIEW];
	const view = outer ? new View(outer.s, outer.start + start, end - start) : new View(s, start, end - start);
	return new Proxy(new Array<T>(end - start), view);
}

/** What a `sliceView` answers to its own `View` by. */
const VIEW = /* @__PURE__ */ Symbol("view");

/** A `sliceView`'s traps: of `length` elements of `s` from `start`. */
class View<T> implements ProxyHandler<T[]> {
	readonly s: T[];
	readonly start: number;
	readonly length: number;

	constructor(s: T[], start: number, length: number) {
		this.s = s;
		this.start = start;
		this.length = length;
	}

	/** Where in `s` the element `key` names is, or -1 if it names none. */
	at(key: string | symbol): number {
		if (typeof key !== "string") return -1;
		const i = Number(key);
		return Number.isInteger(i) && i >= 0 && i < this.length && String(i) === key ? this.start + i : -1;
	}

	get(target: T[], key: string | symbol): unknown {
		if (key === VIEW) return this;
		const i = this.at(key);
		return i < 0 ? Reflect.get(target, key) : this.s[i];
	}

	set(target: T[], key: string | symbol, value: T): boolean {
		const i = this.at(key);
		if (i < 0) return Reflect.set(target, key, value);
		this.s[i] = value;
		return true;
	}

	has(target: T[], key: string | symbol): boolean {
		return this.at(key) >= 0 || Reflect.has(target, key);
	}

	ownKeys(target: T[]): (string | symbol)[] {
		return [...Array.from({ length: this.length }, (_, i) => String(i)), ...Reflect.ownKeys(target)];
	}

	getOwnPropertyDescriptor(target: T[], key: string | symbol): PropertyDescriptor | undefined {
		const i = this.at(key);
		if (i < 0) return Reflect.getOwnPropertyDescriptor(target, key);
		return { value: this.s[i], writable: true, enumerable: true, configurable: true };
	}
}

/** `&mut s[start..end]`: the runtime's view of it, checked as `sliceRange` checks it. */
export function sliceRangeMut<T>(s: T[], start: number, end: number = s.length): T[] {
	rangeCheck(s.length, start, end);
	return sliceView(s, start, end);
}

/** `get` of `start..end`: the part, or `undefined` unless it is within the slice. */
export function sliceGetRange<T>(s: T[], start: number, end: number = s.length): T[] | undefined {
	return start <= end && end <= s.length ? s.slice(start, end) : undefined;
}

/** `get_mut` of `start..end`: the runtime's view of it, or `undefined` as `sliceGetRange`. */
export function sliceGetRangeMut<T>(s: T[], start: number, end: number = s.length): T[] | undefined {
	return start <= end && end <= s.length ? sliceView(s, start, end) : undefined;
}

/** `last_chunk`: the last `n` elements, or `undefined` if fewer. */
export function sliceLastChunk<T>(s: T[], n: number): T[] | undefined {
	return n > s.length ? undefined : s.slice(s.length - n);
}

/** `last_chunk_mut`: a view of the last `n` elements, or `undefined` if fewer. */
export function sliceLastChunkMut<T>(s: T[], n: number): T[] | undefined {
	return n > s.length ? undefined : sliceView(s, s.length - n, s.length);
}

/** `split_last_chunk`: the rest and the last `n` elements, or `undefined` if fewer. */
export function sliceSplitLastChunk<T>(s: T[], n: number): [T[], T[]] | undefined {
	return n > s.length ? undefined : sliceSplitAt(s, s.length - n);
}

/** `split_last_chunk_mut`: views of the rest and the last `n` elements, or `undefined` if fewer. */
export function sliceSplitLastChunkMut<T>(s: T[], n: number): [T[], T[]] | undefined {
	return n > s.length ? undefined : sliceSplitAtMut(s, s.length - n);
}

/** `split_first`: the first element and the rest, or `undefined` if empty. */
export function sliceSplitFirst<T>(s: T[]): [T, T[]] | undefined {
	return s.length === 0 ? undefined : [s[0], s.slice(1)];
}

/** `split_at`, panicking past the length. */
export function sliceSplitAt<T>(s: T[], n: number): [T[], T[]] {
	if (n > s.length) panic("mid > len");
	return [s.slice(0, n), s.slice(n)];
}

/** `split_at_checked`: the halves, or `undefined` past the length. */
export function sliceSplitAtChecked<T>(s: T[], n: number): [T[], T[]] | undefined {
	return n > s.length ? undefined : [s.slice(0, n), s.slice(n)];
}

/** `split_at_mut`: views of either half, panicking past the length. */
export function sliceSplitAtMut<T>(s: T[], n: number): [T[], T[]] {
	if (n > s.length) panic("mid > len");
	return [sliceView(s, 0, n), sliceView(s, n, s.length)];
}

/** `split_at_mut_checked`: views of either half, or `undefined` past the length. */
export function sliceSplitAtMutChecked<T>(s: T[], n: number): [T[], T[]] | undefined {
	return n > s.length ? undefined : [sliceView(s, 0, n), sliceView(s, n, s.length)];
}

/**
 * `split_first_mut`: the first element, as its place if it is one JS copies, and a view
 * of the rest, or `undefined` if empty.
 */
export function sliceSplitFirstMut<T>(s: T[], place: true): [Place<T>, T[]] | undefined;
export function sliceSplitFirstMut<T>(s: T[], place: false): [T, T[]] | undefined;
export function sliceSplitFirstMut<T>(s: T[], place: boolean): [Place<T> | T, T[]] | undefined {
	if (s.length === 0) return undefined;
	return [place ? new AtIndex(s, 0) : s[0], sliceView(s, 1, s.length)];
}

/** `split_last_mut`: as `sliceSplitFirstMut`, the last element and a view of the rest. */
export function sliceSplitLastMut<T>(s: T[], place: true): [Place<T>, T[]] | undefined;
export function sliceSplitLastMut<T>(s: T[], place: false): [T, T[]] | undefined;
export function sliceSplitLastMut<T>(s: T[], place: boolean): [Place<T> | T, T[]] | undefined {
	const n = s.length - 1;
	if (n < 0) return undefined;
	return [place ? new AtIndex(s, n) : s[n], sliceView(s, 0, n)];
}

/** `first`, and `first_mut` of an element that is an object: the first element, or `none` if empty. */
export function sliceFirst<T>(s: ArrayLike<T>): T | undefined;
export function sliceFirst<T>(s: ArrayLike<T>, none: None): T | None;
export function sliceFirst<T>(s: ArrayLike<T>, none?: None): T | undefined | None {
	return s.length === 0 ? none : s[0];
}

/** `get`, and `get_mut` of an element that is an object: the element at `index`, or `none` past the end. */
export function sliceGet<T>(s: ArrayLike<T>, index: number): T | undefined;
export function sliceGet<T>(s: ArrayLike<T>, index: number, none: None): T | None;
export function sliceGet<T>(s: ArrayLike<T>, index: number, none?: None): T | undefined | None {
	return index < s.length ? s[index] : none;
}

/** `core::ops::Bound`, tagged as the crate's own enums are. */
export type RangeBound<T> = { readonly tag: "Included"; readonly _0: T } | { readonly tag: "Excluded"; readonly _0: T } | { readonly tag: "Unbounded" };

/** `Bound::cloned` and `copied`: `b` with `clone` of what it holds. */
export function boundMap<T>(b: RangeBound<T>, clone: (value: T) => T): RangeBound<T> {
	return b.tag === "Unbounded" ? b : { tag: b.tag, _0: clone(b._0) };
}

/** `NonZero::new`: `n`, unless it is 0. */
export function nonZero<T>(n: T): T | undefined {
	return n === 0 ? undefined : n;
}

/** `last`, and `last_mut` of an element that is an object: the last element, or `none` if empty. */
export function sliceLast<T>(s: ArrayLike<T>): T | undefined;
export function sliceLast<T>(s: ArrayLike<T>, none: None): T | None;
export function sliceLast<T>(s: ArrayLike<T>, none?: None): T | undefined | None {
	return s.length === 0 ? none : s[s.length - 1];
}

/** `get_mut` of an element JS copies: its place, or `undefined` past the end. */
export function sliceGetMut<T>(s: { [i: number]: T; length: number }, index: number): Place<T> | undefined {
	return index < s.length ? new AtIndex(s, index) : undefined;
}

/** `first_mut` of an element JS copies. */
export function sliceFirstMut<T>(s: { [i: number]: T; length: number }): Place<T> | undefined {
	return sliceGetMut(s, 0);
}

/** `last_mut` of an element JS copies. */
export function sliceLastMut<T>(s: { [i: number]: T; length: number }): Place<T> | undefined {
	return s.length === 0 ? undefined : new AtIndex(s, s.length - 1);
}

/** `contains`, by `eq` of each element and then `x`, as std's `==` reads them. */
export function sliceContains<T>(s: ArrayLike<T>, x: T, eq: (a: T, b: T) => boolean): boolean {
	for (let i = 0; i < s.length; i++) if (eq(s[i], x)) return true;
	return false;
}

/** `swap`, panicking on either index past the end, `a` first, as std's indexing does. */
export function sliceSwap<T>(s: T[], a: number, b: number): void {
	const x = s[idx(s, a)];
	s[a] = s[idx(s, b)];
	s[b] = x;
}

/**
 * `fill`: a clone of `value` into each slot but the last, each old element dropped once
 * its slot's clone is made, then `value` itself into the last, or `value` dropped if there
 * is none. `clone` is `undefined` where a clone is the value.
 */
export function sliceFill<T>(s: T[], value: T, clone: ((value: T) => T) | undefined, drop?: (item: T) => void): void {
	const last = s.length - 1;
	if (last < 0) return drop?.(value);
	for (let i = 0; i < last; i++) {
		const item = clone === undefined ? value : clone(value);
		drop?.(s[i]);
		s[i] = item;
	}
	drop?.(s[last]);
	s[last] = value;
}

/** `fill_with`: `f()` into each slot in order, each old element dropped once its slot's value is made. */
export function sliceFillWith<T>(s: T[], f: () => T, drop?: (item: T) => void): void {
	for (let i = 0; i < s.length; i++) {
		const item = f();
		drop?.(s[i]);
		s[i] = item;
	}
}

/** `rotate_left`, panicking past the length. */
export function sliceRotateLeft<T>(s: T[], mid: number): void {
	if (mid > s.length) panic("assertion failed: mid <= self.len()");
	rotate(s, mid);
}

/** `rotate_right`, panicking past the length. */
export function sliceRotateRight<T>(s: T[], k: number): void {
	if (k > s.length) panic("assertion failed: k <= self.len()");
	rotate(s, s.length - k);
}

/** The elements from `mid` on, then those before it, in place. */
function rotate<T>(s: T[], mid: number): void {
	if (mid === 0 || mid === s.length) return;
	const head = s.slice(0, mid);
	s.copyWithin(0, mid);
	for (let i = 0; i < mid; i++) s[s.length - mid + i] = head[i];
}

/** `copy_within`: `s[start..end]` copied to index `dest`, panicking out of bounds; `copy` is `undefined` where a copy is the value. */
export function sliceCopyWithin<T>(s: T[], start: number, end: number, dest: number, copy?: (value: T) => T): void {
	copyWithinCheck(s.length, start, end, dest);
	if (copy === undefined) {
		s.copyWithin(dest, start, end);
		return;
	}
	const items = s.slice(start, end);
	for (let i = 0; i < items.length; i++) s[dest + i] = copy(items[i]);
}

/** `copy_within`'s checks of `start..end` and `dest` in `length` elements. */
function copyWithinCheck(length: number, start: number, end: number, dest: number): void {
	// `slice::range` checks the end first, then `range` reports the rest.
	if (end > length) panic(`range end index ${end} out of range for slice of length ${length}`);
	rangeCheck(length, start, end);
	if (dest > length - (end - start)) panic("dest is out of bounds");
}

/** `copy_from_slice`, and `clone_from_slice` of a `TrivialClone`, panicking if the lengths differ; `copy` as `sliceCopyWithin`'s. */
export function sliceCopyFromSlice<T>(dst: T[], src: ArrayLike<T>, copy?: (value: T) => T): void {
	if (dst.length !== src.length) copyLengthFail(dst.length, src.length);
	for (let i = 0; i < src.length; i++) dst[i] = copy === undefined ? src[i] : copy(src[i]);
}

/** `clone_from_slice`: each slot's clone in order, each old element dropped once its slot's clone is made, panicking first if the lengths differ. */
export function sliceCloneFromSlice<T>(dst: T[], src: ArrayLike<T>, clone: ((value: T) => T) | undefined, drop?: (item: T) => void): void {
	if (dst.length !== src.length) panic("destination and source slices have different lengths");
	for (let i = 0; i < src.length; i++) {
		const item = clone === undefined ? src[i] : clone(src[i]);
		drop?.(dst[i]);
		dst[i] = item;
	}
}

function copyLengthFail(dst: number, src: number): never {
	panic(`copy_from_slice: source slice length (${src}) does not match destination slice length (${dst})`);
}

/** `sort_by` and `sort_unstable_by`: by `cmp`, whether it orders `Less`, as `sliceSort` sorts. */
export function sliceSortBy<T>(s: { [i: number]: T; length: number }, cmp: (a: T, b: T) => Ordering): void {
	sliceSort(s, (a, b) => cmp(a, b) === -1);
}

/** `sort_by_key` and `sort_unstable_by_key`: by `lt` of each element's `key`, as `sliceSort` sorts. */
export function sliceSortByKey<T, K>(s: { [i: number]: T; length: number }, key: (item: T) => K, lt: (a: K, b: K) => boolean): void {
	sliceSort(s, (a, b) => lt(key(a), key(b)));
}

/**
 * `sort` and `sort_unstable`, by `less`, the elements' `lt`, which std's sorts test: a
 * stable merge sort over runs that insertion sorts first. The unstable sorts may order
 * equal elements either way and here keep them in order. It calls only `less`, never
 * JS's `sort`, which moves `undefined`, a `None`, to the end without comparing it, and
 * orders its calls differently in each engine.
 */
export function sliceSort<T>(s: { [i: number]: T; length: number }, less: (a: T, b: T) => boolean): void {
	const n = s.length;
	const RUN = 16;
	for (let lo = 0; lo < n; lo += RUN) {
		const hi = Math.min(lo + RUN, n);
		for (let i = lo + 1; i < hi; i++) {
			const x = s[i];
			let j = i;
			while (j > lo && less(x, s[j - 1])) {
				s[j] = s[j - 1];
				j--;
			}
			s[j] = x;
		}
	}
	if (n <= RUN) return;
	const left: T[] = [];
	for (let width = RUN; width < n; width *= 2) {
		for (let lo = 0; lo + width < n; lo += 2 * width) {
			const mid = lo + width;
			const hi = Math.min(mid + width, n);
			if (!less(s[mid], s[mid - 1])) continue;
			// The left run moves aside, and the merge fills `s` from `lo`, never past the
			// right run's next element.
			left.length = 0;
			for (let i = lo; i < mid; i++) left.push(s[i]);
			let i = 0;
			let j = mid;
			let k = lo;
			while (i < left.length && j < hi) s[k++] = less(s[j], left[i]) ? s[j++] : left[i++];
			while (i < left.length) s[k++] = left[i++];
		}
	}
}

/**
 * `is_sorted`, by `le`, the elements' `PartialOrd::le`: as std's, which below 33 elements
 * stops at the first pair out of order, and past that tests every pair of each chunk of 33
 * before it stops, the last element of one the first of the next.
 */
export function sliceIsSorted<T>(s: { [i: number]: T; length: number }, le: (a: T, b: T) => boolean): boolean {
	const CHUNK = 33;
	const n = s.length;
	let i = 0;
	if (n >= CHUNK) {
		for (; i < n - CHUNK; i += CHUNK - 1) {
			let sorted = true;
			for (let j = i + 1; j < i + CHUNK; j++) sorted = le(s[j - 1], s[j]) && sorted;
			if (!sorted) return false;
		}
	}
	for (i++; i < n; i++) if (!le(s[i - 1], s[i])) return false;
	return true;
}

/**
 * Keeps each of the first `n` elements of `a` that `keep` of its index returns true for,
 * in order, and drops each other one once `keep` rejects it: how many it kept.
 */
function retainAt<T>(a: { [i: number]: T }, n: number, keep: (i: number) => boolean, drop?: (item: T) => void): number {
	let write = 0;
	for (let read = 0; read < n; read++) {
		if (keep(read)) a[write++] = a[read];
		else drop?.(a[read]);
	}
	return write;
}

/**
 * Drops each of the first `n` elements of `a` that `same` of its index and the previous
 * kept one's is true for, in order: how many it kept.
 */
function dedupAt<T>(a: { [i: number]: T }, n: number, same: (i: number, kept: number) => boolean, drop?: (item: T) => void): number {
	if (n === 0) return 0;
	let write = 1;
	for (let read = 1; read < n; read++) {
		if (same(read, write - 1)) drop?.(a[read]);
		else a[write++] = a[read];
	}
	return write;
}

/** `Vec::retain`, and `retain_mut` of an element that is an object. */
export function vecRetain<T>(v: T[], f: (item: T) => boolean, drop?: (item: T) => void): void {
	v.length = retainAt(v, v.length, (i) => f(v[i]), drop);
}

/** `Vec::retain_mut` of an element JS copies, which `f` takes the place of. */
export function vecRetainMut<T>(v: T[], f: (item: Place<T>) => boolean, drop?: (item: T) => void): void {
	v.length = retainAt(v, v.length, (i) => f(new AtIndex(v, i)), drop);
}

/** `Vec::dedup_by`: drops each element `same(element, previous kept)` is true for, in order. */
export function vecDedupBy<T>(v: T[], same: (a: T, b: T) => boolean, drop?: (item: T) => void): void {
	v.length = dedupAt(v, v.length, (i, j) => same(v[i], v[j]), drop);
}

/** `Vec::dedup_by` of an element JS copies, whose places `same` takes. */
export function vecDedupByMut<T>(v: T[], same: (a: Place<T>, b: Place<T>) => boolean, drop?: (item: T) => void): void {
	v.length = dedupAt(v, v.length, (i, j) => same(new AtIndex(v, i), new AtIndex(v, j)), drop);
}

/** `Vec::dedup_by_key`: as `vecDedupBy`, by `eq` of `key` of each element, then of the previous kept. */
export function vecDedupByKey<T, K>(v: T[], key: (item: T) => K, eq: (a: K, b: K) => boolean, drop?: (item: T) => void): void {
	vecDedupBy(v, (a, b) => eq(key(a), key(b)), drop);
}

/** `Vec::dedup_by_key` of an element JS copies, whose places `key` takes. */
export function vecDedupByKeyMut<T, K>(v: T[], key: (item: Place<T>) => K, eq: (a: K, b: K) => boolean, drop?: (item: T) => void): void {
	vecDedupByMut(v, (a, b) => eq(key(a), key(b)), drop);
}

/** `*v = value` through a `&mut` to a `Vec<T>` other than of bytes, or to a boxed slice: `value`'s items, in place. */
export function vecAssign<T>(v: T[], value: T[]): void {
	v.length = value.length;
	for (let i = 0; i < value.length; i++) v[i] = value[i]!;
}

/** `Vec::insert`: `item` at `i`, with those from `i` on moved up. */
export function vecInsert<T>(v: T[], i: number, item: T): void {
	if (i > v.length) panic(`insertion index (is ${i}) should be <= len (is ${v.length})`);
	v.splice(i, 0, item);
}

/** `Vec::pop` of an element that can be `undefined`: the last element, or `none` if empty. */
export function vecPop<T>(v: T[], none: None): T | None {
	return v.length === 0 ? none : (v.pop() as T);
}

/**
 * `Vec::pop_if`: the last element if `f` takes it, given the element, or else `none`. An
 * empty `v` drops `f` by `dropF` uncalled.
 */
export function vecPopIf<T>(v: T[], f: (item: T) => boolean, dropF?: (f: (item: T) => boolean) => void): T | undefined;
export function vecPopIf<T>(v: T[], f: (item: T) => boolean, dropF: ((f: (item: T) => boolean) => void) | undefined, none: None): T | None;
export function vecPopIf<T>(v: T[], f: (item: T) => boolean, dropF?: (f: (item: T) => boolean) => void, none?: None): T | None | undefined {
	if (v.length === 0) {
		dropF?.(f);
		return none;
	}
	return f(v[v.length - 1]) ? (v.pop() as T) : none;
}

/** `Vec::pop_if` of an element JS copies, which `f` takes the place of. */
export function vecPopIfMut<T>(v: T[], f: (item: Place<T>) => boolean, dropF?: (f: (item: Place<T>) => boolean) => void): T | undefined;
export function vecPopIfMut<T>(
	v: T[],
	f: (item: Place<T>) => boolean,
	dropF: ((f: (item: Place<T>) => boolean) => void) | undefined,
	none: None,
): T | None;
export function vecPopIfMut<T>(
	v: T[],
	f: (item: Place<T>) => boolean,
	dropF?: (f: (item: Place<T>) => boolean) => void,
	none?: None,
): T | None | undefined {
	if (v.length === 0) {
		dropF?.(f);
		return none;
	}
	return f(new AtIndex(v, v.length - 1)) ? (v.pop() as T) : none;
}

/** `Vec::remove`: the element at `i`, with those after it moved down. */
export function vecRemove<T>(v: T[], i: number): T {
	if (i >= v.length) panic(`removal index (is ${i}) should be < len (is ${v.length})`);
	return v.splice(i, 1)[0];
}

/** `Vec::truncate`, and `clear` to 0: the first `len` elements, if there are more, and then each other drops in order. */
export function vecTruncate<T>(v: T[], len: number, drop?: (item: T) => void): void {
	if (len >= v.length) return;
	const cut = v.splice(len);
	if (drop !== undefined) dropAll(cut, drop);
}

/** `Vec::swap_remove`: the element at `i`, with the last moved into its place. */
export function vecSwapRemove<T>(v: T[], i: number): T {
	if (i >= v.length) panic(`swap_remove index (is ${i}) should be < len (is ${v.length})`);
	const item = v[i];
	v[i] = v[v.length - 1];
	v.pop();
	return item;
}

/** `Vec::extend_from_slice`: a clone of each element of `s`, pushed in order, as far as they clone. */
export function vecExtendFromSlice<T>(v: T[], s: ArrayLike<T>, clone?: (item: T) => T): void {
	for (let i = 0; i < s.length; i++) v.push(clone === undefined ? s[i] : clone(s[i]));
}

/** `Clone` of a tuple, by `clone` of each field in order, `undefined` for one whose value is itself. */
export function tupleClone<T extends readonly unknown[]>(t: T, clones: readonly (((value: never) => unknown) | undefined)[]): T {
	const out: unknown[] = [];
	for (let i = 0; i < clones.length; i++) {
		const clone = clones[i] as ((value: unknown) => unknown) | undefined;
		out.push(clone === undefined ? t[i] : clone(t[i]));
	}
	return out as unknown as T;
}

/**
 * `&[u8]` and `&mut [u8]`, and `&[u8; N]`: `length` bytes of `data` from `start`. It never changes, so a
 * split makes another, which costs an object where a `Uint8Array` view costs several
 * times as much.
 */
export class ByteSlice {
	readonly data: Uint8Array;
	readonly start: number;
	readonly length: number;

	constructor(data: Uint8Array, start: number, length: number) {
		this.data = data;
		this.start = start;
		this.length = length;
	}
}

/** All of `bytes` as a slice, as `&[u8; N]` coerces to `&[u8]`. */
export function byteSliceFrom(bytes: Uint8Array): ByteSlice {
	return new ByteSlice(bytes, 0, bytes.length);
}

/** A view of the bytes, for what reads a `Uint8Array`. */
export function byteSliceView(s: ByteSlice): Uint8Array {
	return s.data.subarray(s.start, s.start + s.length);
}

/** `&s[start..end]`, with `end` defaulting to the length. */
export function byteSliceRange(s: ByteSlice, start: number, end: number = s.length): ByteSlice {
	rangeCheck(s.length, start, end);
	return new ByteSlice(s.data, s.start + start, end - start);
}

/** `get` of `start..end`: the part, or `undefined` unless it is within the slice. */
export function byteSliceGetRange(s: ByteSlice, start: number, end: number = s.length): ByteSlice | undefined {
	return start <= end && end <= s.length ? new ByteSlice(s.data, s.start + start, end - start) : undefined;
}

/** `last_chunk`: the last `n` bytes, or `undefined` if fewer. */
export function byteSliceLastChunk(s: ByteSlice, n: number): ByteSlice | undefined {
	return n > s.length ? undefined : new ByteSlice(s.data, s.start + s.length - n, n);
}

/** `split_last_chunk`: the rest and the last `n` bytes, or `undefined` if fewer. */
export function byteSliceSplitLastChunk(s: ByteSlice, n: number): [ByteSlice, ByteSlice] | undefined {
	return n > s.length ? undefined : byteSliceSplitAt(s, s.length - n);
}

/** `split_first`: the first byte and the rest, or `undefined` if empty. */
export function byteSliceSplitFirst(s: ByteSlice): [number, ByteSlice] | undefined {
	return s.length === 0 ? undefined : [s.data[s.start], new ByteSlice(s.data, s.start + 1, s.length - 1)];
}

/** `split_first_mut`: the first byte's place and the rest, or `undefined` if empty. */
export function byteSliceSplitFirstMut(s: ByteSlice): [Place<number>, ByteSlice] | undefined {
	return s.length === 0 ? undefined : [new AtIndex(s.data, s.start), new ByteSlice(s.data, s.start + 1, s.length - 1)];
}

/** `split_last_mut`: the last byte's place and the rest, or `undefined` if empty. */
export function byteSliceSplitLastMut(s: ByteSlice): [Place<number>, ByteSlice] | undefined {
	const n = s.length - 1;
	return n < 0 ? undefined : [new AtIndex(s.data, s.start + n), new ByteSlice(s.data, s.start, n)];
}

/**
 * `ptr::eq` of pointers to `[u8; N]`s: the same null or dangling pointer, or slices of the
 * same bytes, which separate views of one buffer may hold.
 */
export function byteSliceSame(a: ByteSlice, b: ByteSlice): boolean {
	if (!(a instanceof ByteSlice && b instanceof ByteSlice)) return a === b;
	return a.data.buffer === b.data.buffer && a.data.byteOffset + a.start === b.data.byteOffset + b.start;
}

/** `ptr::write` of `bytes` through a pointer to a `[u8; N]`: copied into its slice. */
export function byteSliceSet(s: ByteSlice, bytes: Uint8Array): void {
	s.data.set(bytes, s.start);
}

/** `*r` of a `&[u8; N]`: a copy of its bytes. */
export function byteSliceToArray(s: ByteSlice): Uint8Array {
	return s.data.slice(s.start, s.start + s.length);
}

/** `split_at`, panicking past the length. */
export function byteSliceSplitAt(s: ByteSlice, n: number): [ByteSlice, ByteSlice] {
	if (n > s.length) panic("mid > len");
	return [new ByteSlice(s.data, s.start, n), new ByteSlice(s.data, s.start + n, s.length - n)];
}

/** `split_at_checked`: the halves, or `undefined` past the length. */
export function byteSliceSplitAtChecked(s: ByteSlice, n: number): [ByteSlice, ByteSlice] | undefined {
	if (n > s.length) return undefined;
	return [new ByteSlice(s.data, s.start, n), new ByteSlice(s.data, s.start + n, s.length - n)];
}

/** `first`: the first byte, or `undefined` if empty. */
export function byteSliceFirst(s: ByteSlice): number | undefined {
	return s.length === 0 ? undefined : s.data[s.start];
}

/** `last`: the last byte, or `undefined` if empty. */
export function byteSliceLast(s: ByteSlice): number | undefined {
	return s.length === 0 ? undefined : s.data[s.start + s.length - 1];
}

/** `swap`, panicking on either index past the end, `a` first, as std's indexing does. */
export function byteSliceSwap(s: ByteSlice, a: number, b: number): void {
	const x = s.data[s.start + idx(s, a)];
	s.data[s.start + a] = s.data[s.start + idx(s, b)];
	s.data[s.start + b] = x;
}

/** `get`: the byte at `index`, or `undefined` past the end. */
export function byteSliceGet(s: ByteSlice, index: number): number | undefined {
	return index < s.length ? s.data[s.start + index] : undefined;
}

/** `==` of byte slices: the same length and bytes. */
export function byteSliceEq(a: ByteSlice, b: ByteSlice): boolean {
	if (a.length !== b.length) return false;
	for (let i = 0; i < a.length; i++) if (a.data[a.start + i] !== b.data[b.start + i]) return false;
	return true;
}

/** Copy `src` into `dst` at `at`; a few bytes copy faster than a view of them is made. */
export function copyBytes(dst: Uint8Array, at: number, src: Uint8Array, start: number, length: number): void {
	if (length <= 16) {
		for (let i = 0; i < length; i++) dst[at + i] = src[start + i];
	} else {
		dst.set(src.subarray(start, start + length), at);
	}
}

/** `copy_from_slice`, panicking if the lengths differ. */
export function byteSliceCopyFromSlice(dst: ByteSlice, src: ByteSlice): void {
	if (dst.length !== src.length) copyLengthFail(dst.length, src.length);
	copyBytes(dst.data, dst.start, src.data, src.start, src.length);
}

/** `copy_within`: `s[start..end]` copied to index `dest`, panicking out of bounds. */
export function byteSliceCopyWithin(s: ByteSlice, start: number, end: number, dest: number): void {
	copyWithinCheck(s.length, start, end, dest);
	s.data.copyWithin(s.start + dest, s.start + start, s.start + end);
}

/** `fill` of bytes. */
export function byteSliceFill(s: ByteSlice, value: number): void {
	s.data.fill(value, s.start, s.start + s.length);
}

/** `fill_with` of bytes: `f()` into each in order. */
export function byteSliceFillWith(s: ByteSlice, f: () => number): void {
	for (let i = 0; i < s.length; i++) s.data[s.start + i] = f();
}

/** `rotate_left` of bytes, panicking past the length. */
export function byteSliceRotateLeft(s: ByteSlice, mid: number): void {
	if (mid > s.length) panic("assertion failed: mid <= self.len()");
	byteRotate(s, mid);
}

/** `rotate_right` of bytes, panicking past the length. */
export function byteSliceRotateRight(s: ByteSlice, k: number): void {
	if (k > s.length) panic("assertion failed: k <= self.len()");
	byteRotate(s, s.length - k);
}

/** The bytes from `mid` on, then those before it, in place. */
function byteRotate(s: ByteSlice, mid: number): void {
	const head = s.data.slice(s.start, s.start + mid);
	s.data.copyWithin(s.start, s.start + mid, s.start + s.length);
	s.data.set(head, s.start + s.length - mid);
}

/** `to_vec` of bytes. */
export function byteSliceToByteVec(s: ByteSlice): ByteVec {
	const v = new ByteVec(s.length);
	copyBytes(v.data, 0, s.data, s.start, s.length);
	v.length = s.length;
	return v;
}

/** `vec![value; n]`: `n - 1` clones of `value`, in order, then `value` itself; `clone` is `undefined` where a clone is the value. */
export function vecFromElem<T>(value: T, n: number, clone: ((value: T) => T) | undefined): T[] {
	const v: T[] = [];
	for (let i = 1; i < n; i++) v.push(clone === undefined ? value : clone(value));
	if (n > 0) v.push(value);
	return v;
}

const SLAB = 8192;
let slab = /* @__PURE__ */ new ArrayBuffer(SLAB);
let slabUsed = 0;

/**
 * `n` zeroed bytes. Up to 1 KiB, they share an `ArrayBuffer`, as Node's `Buffer` pool
 * does: a lone array's own buffer costs V8 about a microsecond, made when the array is
 * past 64 bytes or else the first time something views it. No byte is handed out twice.
 */
export function bytes(n: number): Uint8Array {
	if (n > SLAB / 8) return new Uint8Array(n);
	if (slabUsed + n > SLAB) {
		slab = new ArrayBuffer(SLAB);
		slabUsed = 0;
	}
	const out = new Uint8Array(slab, slabUsed, n);
	// Aligned, as Node aligns its pool.
	slabUsed += (n + 7) & ~7;
	return out;
}

/** `array::from_fn`: `f` of each index, called in order. */
export function arrayFromFn<T>(n: number, f: (i: number) => T): T[] {
	const out: T[] = [];
	for (let i = 0; i < n; i++) out.push(f(i));
	return out;
}

/** `array::from_fn` of bytes. */
export function bytesFromFn(n: number, f: (i: number) => number): Uint8Array {
	const out = bytes(n);
	for (let i = 0; i < n; i++) out[i] = f(i);
	return out;
}

/** `<[T; N]>::map`: `f` of each element, called in order. */
export function arrayMap<T, U>(a: ArrayLike<T>, f: (x: T) => U): U[] {
	const out: U[] = [];
	for (let i = 0; i < a.length; i++) out.push(f(a[i]));
	return out;
}

/** `<[T; N]>::map` to bytes. */
export function bytesMap<T>(a: ArrayLike<T>, f: (x: T) => number): Uint8Array {
	const out = bytes(a.length);
	for (let i = 0; i < a.length; i++) out[i] = f(a[i]);
	return out;
}

/** `<[T; N]>::each_ref` and `each_mut` of elements lent as themselves. */
export function arrayFrom<T>(a: ArrayLike<T>): T[] {
	return Array.from(a);
}

/** `<[T; N]>::each_mut` of elements JS copies: the place of each. */
export function arrayPlaces<T>(a: { [i: number]: T; length: number }): Place<T>[] {
	const out: Place<T>[] = [];
	for (let i = 0; i < a.length; i++) out.push(new AtIndex(a, i));
	return out;
}

/** `Vec<u8>`: a growable byte buffer. */
export class ByteVec {
	/** The storage; only the first `length` bytes are the vector. */
	data: Uint8Array;
	length = 0;

	constructor(capacity = 0) {
		this.data = bytes(capacity);
	}
}

/** A `Vec<u8>` of a copy of `bytes`. */
export function byteVecFrom(bytes: Uint8Array): ByteVec {
	const v = new ByteVec(bytes.length);
	v.data.set(bytes);
	v.length = bytes.length;
	return v;
}

/** `vec![value; n]` of bytes. */
export function byteVecOf(value: number, n: number): ByteVec {
	const v = new ByteVec(n);
	v.data.fill(value);
	v.length = n;
	return v;
}

/** Make room for `additional` more bytes. */
function reserve(v: ByteVec, additional: number): void {
	const needed = v.length + additional;
	if (needed <= v.data.length) return;
	const grown = bytes(Math.max(needed, v.data.length * 2, 8));
	// All of `data`, which costs less than making a view of its first `length` bytes.
	grown.set(v.data);
	v.data = grown;
}

/** The bytes, as a view that a later push may detach. */
export function byteVecView(v: ByteVec): Uint8Array {
	return v.data.subarray(0, v.length);
}

/** The bytes, as `Vec`'s `Deref` borrows them, which a later push may detach. */
export function byteVecSlice(v: ByteVec): ByteSlice {
	return new ByteSlice(v.data, 0, v.length);
}

/** `Vec::push`. */
export function byteVecPush(v: ByteVec, byte: number): void {
	reserve(v, 1);
	v.data[v.length++] = byte;
}

/** `Vec::pop`: the last byte, or `undefined` if empty. */
export function byteVecPop(v: ByteVec): number | undefined {
	return v.length === 0 ? undefined : v.data[--v.length];
}

/** `Vec::pop_if`, which `f` takes the last byte's place in. An empty `v` drops `f` by `dropF` uncalled. */
export function byteVecPopIf(
	v: ByteVec,
	f: (byte: Place<number>) => boolean,
	dropF?: (f: (byte: Place<number>) => boolean) => void,
): number | undefined {
	if (v.length === 0) {
		dropF?.(f);
		return undefined;
	}
	return f(new AtIndex(v.data, v.length - 1)) ? v.data[--v.length] : undefined;
}

/** `Vec::extend_from_slice`. */
export function byteVecExtendFromSlice(v: ByteVec, s: ByteSlice): void {
	reserve(v, s.length);
	copyBytes(v.data, v.length, s.data, s.start, s.length);
	v.length += s.length;
}

/** `Vec::truncate`. */
export function byteVecTruncate(v: ByteVec, length: number): void {
	if (length < v.length) v.length = length;
}

/** `Vec::clear`. */
export function byteVecClear(v: ByteVec): void {
	v.length = 0;
}

/** `Vec::insert`: `byte` at `i`, with those from `i` on moved up. */
export function byteVecInsert(v: ByteVec, i: number, byte: number): void {
	if (i > v.length) panic(`insertion index (is ${i}) should be <= len (is ${v.length})`);
	reserve(v, 1);
	v.data.copyWithin(i + 1, i, v.length);
	v.data[i] = byte;
	v.length++;
}

/** `Vec::remove`: the byte at `i`, with those after it moved down. */
export function byteVecRemove(v: ByteVec, i: number): number {
	if (i >= v.length) panic(`removal index (is ${i}) should be < len (is ${v.length})`);
	const byte = v.data[i];
	v.data.copyWithin(i, i + 1, v.length);
	v.length--;
	return byte;
}

/** `Vec::swap_remove`: the byte at `i`, with the last moved into its place. */
export function byteVecSwapRemove(v: ByteVec, i: number): number {
	if (i >= v.length) panic(`swap_remove index (is ${i}) should be < len (is ${v.length})`);
	const byte = v.data[i];
	v.data[i] = v.data[--v.length];
	return byte;
}

/** `Vec::retain`. */
export function byteVecRetain(v: ByteVec, f: (byte: number) => boolean): void {
	v.length = retainAt(v.data, v.length, (i) => f(v.data[i]));
}

/** `Vec::retain_mut`, which `f` takes each byte's place in. */
export function byteVecRetainMut(v: ByteVec, f: (byte: Place<number>) => boolean): void {
	v.length = retainAt(v.data, v.length, (i) => f(new AtIndex(v.data, i)));
}

/** `Vec::dedup`. */
export function byteVecDedup(v: ByteVec): void {
	v.length = dedupAt(v.data, v.length, (i, j) => v.data[i] === v.data[j]);
}

/** `Vec::dedup_by`, whose places `same` takes. */
export function byteVecDedupByMut(v: ByteVec, same: (a: Place<number>, b: Place<number>) => boolean): void {
	v.length = dedupAt(v.data, v.length, (i, j) => same(new AtIndex(v.data, i), new AtIndex(v.data, j)));
}

/** `Vec::dedup_by_key`, whose places `key` takes. */
export function byteVecDedupByKeyMut<K>(v: ByteVec, key: (byte: Place<number>) => K, eq: (a: K, b: K) => boolean): void {
	byteVecDedupByMut(v, (a, b) => eq(key(a), key(b)));
}

// Cows of slices. An owned one is its `Vec` and a borrowed one its slice, told apart by
// class: a `Cow<[u8]>` is an `rt.ByteVec` or an `rt.ByteSlice`, and any other is an array
// or the `rt.Borrowed` wrapping the array it borrows.

/**
 * The slice a `Cow<[T]>` borrows, which it clones before it changes. A `&mut` to the
 * borrow writes `slice`, so no two `Cow`s share one.
 */
export class Borrowed<T> {
	slice: T[];

	constructor(slice: T[]) {
		this.slice = slice;
	}
}

/** `Cow<[T]>`: the `Vec` it owns, or the slice it borrows. */
export type Cow<T> = T[] | Borrowed<T>;
/** `Cow<[u8]>`: the `Vec<u8>` it owns, or the bytes it borrows. */
export type ByteCow = ByteVec | ByteSlice;

/** `Deref` of a `Cow<[T]>`: its slice. */
export function cowSlice<T>(c: Cow<T>): T[] {
	return c instanceof Borrowed ? c.slice : c;
}

/** `Deref` of a `Cow<[u8]>`: its bytes. */
export function byteCowSlice(c: ByteCow): ByteSlice {
	return c instanceof ByteVec ? byteVecSlice(c) : c;
}

/** `Cow::into_owned` of a `Cow<[T]>`: its `Vec`, or a clone of what it borrows, each item by `clone` if given. */
export function cowIntoOwned<T>(c: Cow<T>, clone?: (item: T) => T): T[] {
	if (!(c instanceof Borrowed)) return c;
	return clone === undefined ? c.slice.slice() : c.slice.map(clone);
}

/** `Cow::into_owned` of a `Cow<[u8]>`. */
export function byteCowIntoOwned(c: ByteCow): ByteVec {
	return c instanceof ByteVec ? c : byteSliceToByteVec(c);
}

/** `Clone` of a `Cow<[T]>`: a borrow of the same slice, or a clone of its `Vec`. */
export function cowClone<T>(c: Cow<T>, clone?: (item: T) => T): Cow<T> {
	if (c instanceof Borrowed) return new Borrowed(c.slice);
	return clone === undefined ? c.slice() : c.map(clone);
}

/** `Clone` of a `Cow<[u8]>`. */
export function byteCowClone(c: ByteCow): ByteCow {
	return c instanceof ByteVec ? byteSliceToByteVec(byteVecSlice(c)) : c;
}

/** Drop a `Cow<[T]>`: the items it owns, by `f` if given. */
export function cowDrop<T>(c: Cow<T>, f?: (item: T) => void): void {
	if (!(c instanceof Borrowed)) dropAll(c, f);
}

/** `f` of the slices two `Cow<[T]>`s deref to, as their `PartialEq` and `Ord` compare them. */
export function cowBy<T, R>(f: (a: T[], b: T[]) => R): (a: Cow<T>, b: Cow<T>) => R {
	return (a, b) => f(cowSlice(a), cowSlice(b));
}

/** `Hash::hash` of a `Cow<[T]>`: its slice's. */
export function hashCow<T, S>(hash: (value: T[], s: S) => void): (value: Cow<T>, s: S) => void {
	return (value, s) => hash(cowSlice(value), s);
}

// Strings

const encoder = /* @__PURE__ */ new TextEncoder();
// `ignoreBOM` keeps a leading U+FEFF, as Rust does.
const decoder = /* @__PURE__ */ new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });

/** `core::str::Utf8Error`: where bytes stop being UTF-8. */
export type Utf8Error = {
	/** `Utf8Error::valid_up_to`. */
	readonly validUpTo: usize;
	/** `Utf8Error::error_len`: `undefined` if the input ended mid-character. */
	readonly errorLen: usize | undefined;
};

/** `alloc::string::FromUtf8Error`: the bytes that were not UTF-8, and where they stop being. */
export class FromUtf8Error {
	readonly bytes: ByteVec;
	readonly error: Utf8Error;

	constructor(bytes: ByteVec) {
		this.bytes = bytes;
		this.error = utf8Error(byteVecView(bytes));
	}
}

/** `Clone` of a `FromUtf8Error`: its bytes, copied. */
export function fromUtf8ErrorClone(e: FromUtf8Error): FromUtf8Error {
	return new FromUtf8Error(byteVecFrom(byteVecView(e.bytes)));
}

/** Where `v`, which is not UTF-8, stops being so, and how many bytes are invalid there, as std reports it. */
function utf8Error(v: Uint8Array): Utf8Error {
	const tail = (b: number | undefined) => b !== undefined && b >= 0x80 && b <= 0xbf;
	for (let i = 0; i < v.length; ) {
		const first = v[i];
		const width = first < 0x80 ? 1 : first >= 0xc2 && first <= 0xdf ? 2 : first >= 0xe0 && first <= 0xef ? 3 : first >= 0xf0 && first <= 0xf4 ? 4 : 0;
		if (width === 1) {
			i++;
			continue;
		}
		if (width === 0) return { validUpTo: i, errorLen: 1 };
		const second = v[i + 1];
		if (second === undefined) return { validUpTo: i, errorLen: undefined };
		const [lo, hi] =
			first === 0xe0 ? [0xa0, 0xbf] : first === 0xed ? [0x80, 0x9f] : first === 0xf0 ? [0x90, 0xbf] : first === 0xf4 ? [0x80, 0x8f] : [0x80, 0xbf];
		if (second < lo || second > hi) return { validUpTo: i, errorLen: 1 };
		for (let k = 2; k < width; k++) {
			if (i + k >= v.length) return { validUpTo: i, errorLen: undefined };
			if (!tail(v[i + k])) return { validUpTo: i, errorLen: k };
		}
		i += width;
	}
	throw new Error("utf8Error: the bytes are valid UTF-8");
}

/** `str::from_utf8`. */
export function strFromUtf8(bytes: ByteSlice): Result<string, Utf8Error> {
	const v = byteSliceView(bytes);
	try {
		return Ok(decoder.decode(v));
	} catch {
		return Err(utf8Error(v));
	}
}

/** `String::from_utf8`. */
export function stringFromUtf8(bytes: ByteVec): Result<string, FromUtf8Error> {
	try {
		return Ok(decoder.decode(byteVecView(bytes)));
	} catch {
		return Err(new FromUtf8Error(bytes));
	}
}

/** `str::as_bytes`: its UTF-8 bytes. */
export function stringAsBytes(s: string): ByteSlice {
	return byteSliceFrom(encoder.encode(s));
}

/** `str::len`: its length in UTF-8 bytes. */
export function stringLen(s: string): number {
	return encoder.encode(s).length;
}

/** `char::from_u32`: `n`, unless it is a surrogate or past the last char. */
export function charFromU32(n: u32): char | undefined {
	return n > 0x10ffff || (n >= 0xd800 && n <= 0xdfff) ? undefined : (n as char);
}

// Formatting, as `core::fmt` does it: the same output byte for byte.

/** `core::fmt::Error`. */
export class FmtError {}

/** `core::str::ParseBoolError`. */
export class ParseBoolError {}

/** `fmt::Result`. */
export type FmtResult = Result<undefined, FmtError>;

/** A value ready to write: `&dyn Display`, `&dyn Debug`, or one placeholder's argument. */
export type Fmt = (f: Formatter) => FmtResult;

/** A type's `Debug` or `Display`, as a function of its values. */
export type Debug<T> = (value: T, f: Formatter) => FmtResult;

/** `fmt::Arguments`: literal pieces, and the values written between them. */
export type Arguments = readonly (string | Fmt)[];

/** One placeholder's options: `{:*^9}` is `{ fill: "*", align: "^", width: 9 }`. */
export interface Spec {
	readonly fill?: string;
	readonly align?: "<" | "^" | ">";
	/** The `+` flag: a sign on a nonnegative number too. */
	readonly plus?: boolean;
	/** The `#` flag: `{:#?}` and `{:#x}`. */
	readonly alternate?: boolean;
	/** The `0` flag: zeros between the sign and the digits. */
	readonly zero?: boolean;
	/** `{:x?}` or `{:X?}`: an integer's `Debug` in hex. */
	readonly debugHex?: "x" | "X";
	readonly width?: number;
	/** The `.N` of `{:.N}`: the most chars `pad` writes, or digits past a number's point. */
	readonly precision?: number;
}

/** A width or precision from an argument, as in `{:1$}`: a `fmt::rt::Argument` only a spec reads. */
export interface Count extends Fmt {
	readonly count: number;
}

/** `Argument::from_usize`: `n` as a width or precision, panicking past what one holds. */
export function fmtCount(n: number): Count {
	if (n > 0xffff) panic("Formatting argument out of range");
	const unreachable = (): never => {
		throw new Error("rust2-ts: a width or precision written as a placeholder");
	};
	return Object.assign(unreachable, { count: n });
}

/** The width or precision `argument`, a `Count`, holds. */
export function fmtCountOf(argument: Fmt): number {
	return (argument as Count).count;
}

/** `dyn fmt::Write`: where a `Formatter` writes. */
export interface Write {
	writeStr(s: string): FmtResult;
}

class StringWriter implements Write {
	s = "";

	writeStr(s: string): FmtResult {
		this.s += s;
		return OK;
	}
}

/** Indents each line it writes, for `{:#?}`'s nesting. */
class PadAdapter implements Write {
	private onNewline = true;
	private readonly out: Write;

	constructor(out: Write) {
		this.out = out;
	}

	writeStr(s: string): FmtResult {
		for (const line of s.split(/(?<=\n)/)) {
			if (line === "") continue;
			if (this.onNewline) {
				const r = this.out.writeStr("    ");
				if (!r.ok) return r;
			}
			this.onNewline = line.endsWith("\n");
			const r = this.out.writeStr(line);
			if (!r.ok) return r;
		}
		return OK;
	}
}

/** `core::fmt::Formatter`: where it writes, and its placeholder's options. */
export interface Formatter {
	readonly out: Write;
	readonly spec: Spec;
}

/** `fmt::write`: each placeholder gets a fresh `Formatter` with only its own options. */
function write(out: Write, args: Arguments): FmtResult {
	for (const part of args) {
		const r = typeof part === "string" ? out.writeStr(part) : part({ out, spec: {} });
		if (!r.ok) return r;
	}
	return OK;
}

/** The number of chars in `s`, as Rust counts them for padding. */
function chars(s: string): number {
	let n = 0;
	for (const _ of s) n++;
	return n;
}

/** `Formatter::write_str`. */
export function fmtWriteStr(f: Formatter, s: string): FmtResult {
	return f.out.writeStr(s);
}

/** `Formatter::write_fmt`. */
export function fmtWriteFmt(f: Formatter, args: Arguments): FmtResult {
	return write(f.out, args);
}

/** `write!` into a `String`, `s`: `s` with what was written, even before an error, and the result. */
export function fmtWriteString(s: string, args: Arguments): { s: string; result: FmtResult } {
	const out = new StringWriter();
	out.s = s;
	const result = write(out, args);
	return { s: out.s, result };
}

/** `Formatter::alternate`: whether the spec has `#`. */
export function fmtAlternate(f: Formatter): boolean {
	return f.spec.alternate === true;
}

/** `Formatter::width`: the spec's width, or `undefined`. */
export function fmtWidth(f: Formatter): number | undefined {
	return f.spec.width;
}

/** Write `s`, cut to the precision in chars, padded to the width. */
export function fmtPad(f: Formatter, s: string): FmtResult {
	const { width, precision } = f.spec;
	if (width === undefined && precision === undefined) return f.out.writeStr(s);
	let count = 0;
	if (precision === undefined) {
		count = chars(s);
	} else {
		let end = 0;
		for (const c of s) {
			if (count === precision) break;
			end += c.length;
			count++;
		}
		s = s.slice(0, end);
	}
	if (width === undefined || count >= width) return f.out.writeStr(s);
	return padded(f, width - count, f.spec.align ?? "<", f.spec.fill ?? " ", s);
}

/** Write an integer's sign, `prefix` if alternate, and `digits`, padded to the width. */
export function fmtPadIntegral(f: Formatter, nonnegative: boolean, prefix: string, digits: string): FmtResult {
	return fmtPadFormatted(f, (nonnegative ? (f.spec.plus ? "+" : "") : "-") + (fmtAlternate(f) ? prefix : ""), digits);
}

/** `Formatter::pad_formatted_parts`: `head` and `digits`, ASCII, padded to the width, the `0` flag's zeros between them. */
export function fmtPadFormatted(f: Formatter, head: string, digits: string): FmtResult {
	const length = head.length + digits.length;
	const width = f.spec.width ?? 0;
	if (length >= width) return f.out.writeStr(head + digits);
	if (f.spec.zero) {
		const r = f.out.writeStr(head);
		return r.ok ? padded(f, width - length, ">", "0", digits) : r;
	}
	return padded(f, width - length, f.spec.align ?? ">", f.spec.fill ?? " ", head + digits);
}

/** Write `s` with `padding` fills around it. */
function padded(f: Formatter, padding: number, align: "<" | "^" | ">", fill: string, s: string): FmtResult {
	const left = align === "<" ? 0 : align === ">" ? padding : Math.floor(padding / 2);
	const r = f.out.writeStr(fill.repeat(left) + s);
	return r.ok ? f.out.writeStr(fill.repeat(padding - left)) : r;
}

/** `Formatter::debug_struct`. */
export function fmtDebugStruct(f: Formatter, name: string): DebugStruct {
	return { fmt: f, result: f.out.writeStr(name), hasFields: false };
}

/** `Formatter::debug_tuple`. */
export function fmtDebugTuple(f: Formatter, name: string): DebugTuple {
	return { fmt: f, result: f.out.writeStr(name), fields: 0, emptyName: name === "" };
}

/** `Formatter::debug_list`. */
export function fmtDebugList(f: Formatter): DebugList {
	return { fmt: f, result: f.out.writeStr("["), hasFields: false };
}

/** `debug_struct_fields_finish`, which derived `Debug` calls past five fields. */
export function fmtDebugStructFields(f: Formatter, name: string, names: readonly string[], values: readonly Fmt[]): FmtResult {
	if (names.length !== values.length) {
		assertFailed(
			"==",
			(f) => usizeDisplay(names.length, f),
			(f) => usizeDisplay(values.length, f),
		);
	}
	const builder = fmtDebugStruct(f, name);
	names.forEach((field, i) => debugStructField(builder, field, values[i]));
	return debugStructFinish(builder);
}

/** `debug_tuple_fields_finish`, which derived `Debug` calls past five fields. */
export function fmtDebugTupleFields(f: Formatter, name: string, values: readonly Fmt[]): FmtResult {
	const builder = fmtDebugTuple(f, name);
	for (const value of values) debugTupleField(builder, value);
	return debugTupleFinish(builder);
}

/** `fmt::DebugStruct`. */
export interface DebugStruct {
	readonly fmt: Formatter;
	result: FmtResult;
	hasFields: boolean;
}

/** `DebugStruct::field`. */
export function debugStructField(b: DebugStruct, name: string, value: Fmt): DebugStruct {
	if (b.result.ok) {
		if (fmtAlternate(b.fmt)) {
			const head = b.hasFields ? OK : b.fmt.out.writeStr(" {\n");
			b.result = head.ok ? pretty(b.fmt, [name, ": ", value, ",\n"]) : head;
		} else {
			b.result = b.fmt.out.writeStr((b.hasFields ? ", " : " { ") + name + ": ");
			if (b.result.ok) b.result = value(b.fmt);
		}
	}
	b.hasFields = true;
	return b;
}

/** `DebugStruct::finish_non_exhaustive`. */
export function debugStructFinishNonExhaustive(b: DebugStruct): FmtResult {
	if (b.result.ok) {
		if (!b.hasFields) b.result = b.fmt.out.writeStr(" { .. }");
		else if (!fmtAlternate(b.fmt)) b.result = b.fmt.out.writeStr(", .. }");
		else b.result = closePretty(b.fmt, "}");
	}
	return b.result;
}

/** `DebugStruct::finish`. */
export function debugStructFinish(b: DebugStruct): FmtResult {
	if (b.hasFields && b.result.ok) b.result = b.fmt.out.writeStr(fmtAlternate(b.fmt) ? "}" : " }");
	return b.result;
}

/** `fmt::DebugTuple`. */
export interface DebugTuple {
	readonly fmt: Formatter;
	result: FmtResult;
	fields: number;
	readonly emptyName: boolean;
}

/** `DebugTuple::field`. */
export function debugTupleField(b: DebugTuple, value: Fmt): DebugTuple {
	if (b.result.ok) {
		if (fmtAlternate(b.fmt)) {
			const head = b.fields > 0 ? OK : b.fmt.out.writeStr("(\n");
			b.result = head.ok ? pretty(b.fmt, [value, ",\n"]) : head;
		} else {
			b.result = b.fmt.out.writeStr(b.fields === 0 ? "(" : ", ");
			if (b.result.ok) b.result = value(b.fmt);
		}
	}
	b.fields++;
	return b;
}

/** `DebugTuple::finish_non_exhaustive`. */
export function debugTupleFinishNonExhaustive(b: DebugTuple): FmtResult {
	if (b.result.ok) {
		if (b.fields === 0) b.result = b.fmt.out.writeStr("(..)");
		else if (!fmtAlternate(b.fmt)) b.result = b.fmt.out.writeStr(", ..)");
		else b.result = closePretty(b.fmt, ")");
	}
	return b.result;
}

/** `DebugTuple::finish`. */
export function debugTupleFinish(b: DebugTuple): FmtResult {
	if (b.fields > 0 && b.result.ok) {
		const comma = b.fields === 1 && b.emptyName && !fmtAlternate(b.fmt);
		b.result = b.fmt.out.writeStr(comma ? ",)" : ")");
	}
	return b.result;
}

/** `fmt::DebugList`. */
export interface DebugList {
	readonly fmt: Formatter;
	result: FmtResult;
	hasFields: boolean;
}

/** `DebugList::entry`. */
export function debugListEntry(b: DebugList, value: Fmt): DebugList {
	if (b.result.ok) {
		if (fmtAlternate(b.fmt)) {
			const head = b.hasFields ? OK : b.fmt.out.writeStr("\n");
			b.result = head.ok ? pretty(b.fmt, [value, ",\n"]) : head;
		} else {
			b.result = b.hasFields ? b.fmt.out.writeStr(", ") : OK;
			if (b.result.ok) b.result = value(b.fmt);
		}
	}
	b.hasFields = true;
	return b;
}

/** `DebugList::entries`: each item written and then dropped by `drop`, and then `items` dropped, as std's `for` does. */
export function debugListEntries<T>(b: DebugList, items: Iterable<T>, debug: Debug<T>, drop?: (value: T) => void): DebugList {
	for (const item of items) {
		debugListEntry(b, (f) => debug(item, f));
		drop?.(item);
	}
	dispose(items);
	return b;
}

/** `DebugList::finish_non_exhaustive`. */
export function debugListFinishNonExhaustive(b: DebugList): FmtResult {
	if (!b.result.ok) return b.result;
	if (!b.hasFields) return b.fmt.out.writeStr("..]");
	if (!fmtAlternate(b.fmt)) return b.fmt.out.writeStr(", ..]");
	return closePretty(b.fmt, "]");
}

/** `DebugList::finish`. */
export function debugListFinish(b: DebugList): FmtResult {
	if (b.result.ok) b.result = b.fmt.out.writeStr("]");
	return b.result;
}

/** Write `parts` indented, as a pretty builder writes one field: values keep `f`'s options. */
function pretty(f: Formatter, parts: Arguments): FmtResult {
	const writer: Formatter = { out: new PadAdapter(f.out), spec: f.spec };
	for (const part of parts) {
		const r = typeof part === "string" ? writer.out.writeStr(part) : part(writer);
		if (!r.ok) return r;
	}
	return OK;
}

/** A pretty builder's `finish_non_exhaustive`: an indented `..`, then `close`. */
function closePretty(f: Formatter, close: string): FmtResult {
	const r = pretty(f, ["..\n"]);
	return r.ok ? f.out.writeStr(close) : r;
}

// `Debug` and `Display` of what std defines them for. A string's `Display` is `fmtPad`.

/** Where each run of `UNPRINTABLE` starts and ends, in order, once one is looked up. */
let unprintableBounds: Uint32Array | undefined;

/** Whether `Debug` writes `point` as `\u{..}`, by std's Unicode rather than the engine's. */
function unprintable(point: number): boolean {
	if (unprintableBounds === undefined) {
		unprintableBounds = new Uint32Array(UNPRINTABLE.length);
		let at = 0;
		for (let i = 0; i < UNPRINTABLE.length; i++) unprintableBounds[i] = at += UNPRINTABLE[i];
	}
	// How many bounds are at most `point`: odd inside a run.
	let lo = 0;
	let hi = unprintableBounds.length;
	while (lo < hi) {
		const mid = (lo + hi) >>> 1;
		if (unprintableBounds[mid] <= point) lo = mid + 1;
		else hi = mid;
	}
	return (lo & 1) === 1;
}

/** One char of `Debug` of a string, which escapes `"`, or of a char, which escapes `'`. */
function escapeDebug(c: string, quote: '"' | "'"): string {
	switch (c) {
		case quote:
			return `\\${quote}`;
		case "\\":
			return "\\\\";
		case "\n":
			return "\\n";
		case "\t":
			return "\\t";
		case "\r":
			return "\\r";
		case "\0":
			return "\\0";
	}
	const point = c.codePointAt(0)!;
	return unprintable(point) ? `\\u{${point.toString(16)}}` : c;
}

/** `format!`, from `alloc::fmt::format`. */
export function fmtFormat(args: Arguments): string {
	const out = new StringWriter();
	if (!write(out, args).ok) {
		panic("a formatting trait implementation returned an error when the underlying stream did not: Error");
	}
	return out.s;
}

/** `ToString::to_string` of a `Display` value. */
export function fmtToString(value: Fmt): string {
	const out = new StringWriter();
	if (!value({ out, spec: {} }).ok) panic("a Display implementation returned an error unexpectedly: Error");
	return out.s;
}

/** A placeholder with options, which replace the defaults its value is written with. */
export function fmtSpec(spec: Spec, value: Fmt): Fmt {
	return (f) => value({ out: f.out, spec });
}

/** `Display` and `Debug` of a `bool`. */
export function fmtBool(value: boolean, f: Formatter): FmtResult {
	return fmtPad(f, value ? "true" : "false");
}

/** `Debug` of a string. */
export function fmtStr(s: string, f: Formatter): FmtResult {
	let out = '"';
	for (const c of s) out += escapeDebug(c, '"');
	return f.out.writeStr(out + '"');
}

/** `Display` of a `char`. */
export function fmtCharDisplay(c: char, f: Formatter): FmtResult {
	return fmtPad(f, String.fromCodePoint(c));
}

/** `Debug` of a `char`, which no width pads. */
export function fmtCharDebug(c: char, f: Formatter): FmtResult {
	return f.out.writeStr(`'${escapeDebug(String.fromCodePoint(c), "'")}'`);
}

/** `Debug` of `()`. */
export function fmtUnit(_: undefined, f: Formatter): FmtResult {
	return fmtPad(f, "()");
}

/** `Debug` of an `Option`, through its value's. */
export function fmtOption<T>(o: T | undefined, f: Formatter, debug: Debug<T>): FmtResult;
export function fmtOption<T>(o: T | None, f: Formatter, debug: Debug<T>, none: None): FmtResult;
export function fmtOption<T>(o: T | undefined | None, f: Formatter, debug: Debug<T>, none?: None): FmtResult {
	if (o === none) return f.out.writeStr("None");
	return debugTupleFinish(debugTupleField(fmtDebugTuple(f, "Some"), (f) => debug(o as T, f)));
}

/** `Debug` of a `Result`, through its value's and error's. */
export function fmtResult<T, E>(r: Result<T, E>, f: Formatter, ok: Debug<T>, err: Debug<E>): FmtResult {
	const field: Fmt = r.ok ? (f) => ok(r.value, f) : (f) => err(r.error, f);
	return debugTupleFinish(debugTupleField(fmtDebugTuple(f, r.ok ? "Ok" : "Err"), field));
}

/** `Debug` of a slice, array, `Vec`, `VecDeque`, or `BinaryHeap`. */
export function fmtList<T>(items: Iterable<T>, f: Formatter, debug: Debug<T>): FmtResult {
	return debugListFinish(debugListEntries(fmtDebugList(f), items, debug));
}

/** `Debug` of a set: std's `DebugSet` is its `DebugList` in braces. */
export function fmtSet<T>(items: Iterable<T>, f: Formatter, debug: Debug<T>): FmtResult {
	const b = debugListEntries({ fmt: f, result: f.out.writeStr("{"), hasFields: false }, items, debug);
	if (b.result.ok) b.result = f.out.writeStr("}");
	return b.result;
}

/** `Debug` of a map: a set of entries, each its key and value apart by `": "` in one indented field. */
export function fmtMap<K, V>(entries: Iterable<[K, V]>, f: Formatter, key: Debug<K>, value: Debug<V>): FmtResult {
	return fmtSet(entries, f, ([k, v], f) => {
		let r = key(k, f);
		if (r.ok) r = f.out.writeStr(": ");
		return r.ok ? value(v, f) : r;
	});
}

/** What one of std's iterators writes around its list: a tuple struct's name, or a struct's name and its one field's. */
export type IterWrap = string | readonly [name: string, field: string];

/** `Debug` of one of std's iterators: the items `it` has left, as a list inside `wraps`, outermost first. */
export function fmtIter<T>(it: Iter<unknown>, f: Formatter, debug: Debug<T>, wraps: readonly IterWrap[] = []): FmtResult {
	return wrapIn(f, wraps, (f) => fmtList(it.remaining() as Iterable<T>, f, debug));
}

function wrapIn(f: Formatter, wraps: readonly IterWrap[], inner: Fmt): FmtResult {
	if (wraps.length === 0) return inner(f);
	const [wrap, ...rest] = wraps;
	const field: Fmt = (f) => wrapIn(f, rest, inner);
	if (typeof wrap === "string") return fmtDebugTupleFields(f, wrap, [field]);
	return fmtDebugStructFields(f, wrap[0], [wrap[1]], [field]);
}

/** `Debug` of `str::Chars`: its name around the list of chars it has left, which a field's indent would not be. */
export function fmtChars(it: Iter<char>, f: Formatter): FmtResult {
	let r = f.out.writeStr("Chars(");
	if (r.ok) r = fmtList(it.remaining() as Iterable<char>, f, fmtCharDebug);
	return r.ok ? f.out.writeStr(")") : r;
}

/** `Debug` of a key as a `BTreeSet`'s `range` and `into_iter` list it: its map's entry, the key and a `SetValZST`. */
export function fmtSetVal<K>(debug: Debug<K>): Debug<K> {
	return (key, f) => fmtTuple([(f) => debug(key, f), (f) => f.out.writeStr("SetValZST")], f);
}

/** `Debug` of a range: each bound present, written with the formatter's options, around `dots`, which none pads. */
function fmtBounds(f: Formatter, start: Fmt | undefined, dots: string, end: Fmt | undefined): FmtResult {
	if (start !== undefined) {
		const r = start(f);
		if (!r.ok) return r;
	}
	const r = f.out.writeStr(dots);
	return r.ok && end !== undefined ? end(f) : r;
}

/** `Debug` of `start..end`. */
export function fmtRange<T>(r: Range<T>, f: Formatter, debug: Debug<T>): FmtResult {
	return fmtBounds(f, (f) => debug(r.start, f), "..", (f) => debug(r.end, f));
}

/** `Debug` of `start..=end`, which notes once iteration has exhausted it. */
export function fmtRangeInclusive<T>(r: RangeInclusive<T>, f: Formatter, debug: Debug<T>): FmtResult {
	const result = fmtBounds(f, (f) => debug(r.lo, f), "..=", (f) => debug(r.hi, f));
	return result.ok && r.exhausted ? f.out.writeStr(" (exhausted)") : result;
}

/** `Debug` of `start..`. */
export function fmtRangeFrom<T>(r: { start: T }, f: Formatter, debug: Debug<T>): FmtResult {
	return fmtBounds(f, (f) => debug(r.start, f), "..", undefined);
}

/** `Debug` of `..end`. */
export function fmtRangeTo<T>(r: { end: T }, f: Formatter, debug: Debug<T>): FmtResult {
	return fmtBounds(f, undefined, "..", (f) => debug(r.end, f));
}

/** `Debug` of `..=end`. */
export function fmtRangeToInclusive<T>(r: { end: T }, f: Formatter, debug: Debug<T>): FmtResult {
	return fmtBounds(f, undefined, "..=", (f) => debug(r.end, f));
}

/** `Debug` of `..`. */
export function fmtRangeFull(_: undefined, f: Formatter): FmtResult {
	return f.out.writeStr("..");
}

/** `Debug` of a `Bound`: its variant, around what it holds. */
export function fmtBound<T>(b: RangeBound<T>, f: Formatter, debug: Debug<T>): FmtResult {
	if (b.tag === "Unbounded") return f.out.writeStr("Unbounded");
	return fmtDebugTupleFields(f, b.tag, [(f) => debug(b._0, f)]);
}

/** `Debug` of an `Ordering`, unpadded. */
export function fmtOrdering(o: Ordering, f: Formatter): FmtResult {
	return f.out.writeStr(o < 0 ? "Less" : o > 0 ? "Greater" : "Equal");
}

/** `Debug` of a `ControlFlow` held as a `Result`, `Ok` to continue: `stop` writes what it breaks with, the `Err` itself with `residual`, as `?` breaks with it. */
export function fmtControlFlow<C, B>(r: Result<C, B>, f: Formatter, go: Debug<C>, stop: Debug<B>): FmtResult;
export function fmtControlFlow<C, E>(r: Result<C, E>, f: Formatter, go: Debug<C>, stop: Debug<Result<never, E>>, residual: true): FmtResult;
export function fmtControlFlow<C, B>(r: Result<C, B>, f: Formatter, go: Debug<C>, stop: Debug<B> | Debug<Result<never, B>>, residual = false): FmtResult {
	if (r.ok) return fmtDebugTupleFields(f, "Continue", [(f) => go(r.value, f)]);
	const held: Fmt = residual ? (f) => (stop as Debug<Result<never, B>>)(r, f) : (f) => (stop as Debug<B>)(r.error, f);
	return fmtDebugTupleFields(f, "Break", [held]);
}

/** `Debug` of a `ControlFlow` held as the `Option` `?` was applied to, which breaks with `None`. */
export function fmtControlFlowOption<C>(o: C | undefined, f: Formatter, go: Debug<C>, stop: Debug<undefined>): FmtResult;
export function fmtControlFlowOption<C>(o: C | None, f: Formatter, go: Debug<C>, stop: Debug<undefined>, none: None): FmtResult;
export function fmtControlFlowOption<C>(o: C | undefined | None, f: Formatter, go: Debug<C>, stop: Debug<undefined>, none?: None): FmtResult {
	if (o === none) return fmtDebugTupleFields(f, "Break", [(f) => stop(undefined, f)]);
	return fmtDebugTupleFields(f, "Continue", [(f) => go(o as C, f)]);
}

/** `Debug` of a tuple, given each element's. */
export function fmtTuple(fields: readonly Fmt[], f: Formatter): FmtResult {
	const builder = fmtDebugTuple(f, "");
	for (const field of fields) debugTupleField(builder, field);
	return debugTupleFinish(builder);
}

/** `Display` of `fmt::Arguments`, which ignores the formatter's options. */
export function fmtArguments(args: Arguments, f: Formatter): FmtResult {
	return fmtWriteFmt(f, args);
}

/** `Debug` of `fmt::Error`. */
export function fmtErrorDebug(_: FmtError, f: Formatter): FmtResult {
	return f.out.writeStr("Error");
}

/** `Display` of `fmt::Error`. */
export function fmtErrorDisplay(_: FmtError, f: Formatter): FmtResult {
	return fmtPad(f, "an error occurred when formatting an argument");
}

/** `Debug` of `TryFromIntError`. */
export function fmtTryFromIntErrorDebug(e: TryFromIntError, f: Formatter): FmtResult {
	return debugTupleFinish(debugTupleField(fmtDebugTuple(f, "TryFromIntError"), (f) => f.out.writeStr(e.kind)));
}

/** `Debug` of `ParseIntError`. */
export function fmtParseIntErrorDebug(e: ParseIntError, f: Formatter): FmtResult {
	return fmtDebugStructFields(f, "ParseIntError", ["kind"], [(f) => f.out.writeStr(e.kind)]);
}

/** `Display` of `ParseIntError`. */
export function fmtParseIntErrorDisplay(e: ParseIntError, f: Formatter): FmtResult {
	const messages = {
		Empty: "cannot parse integer from empty string",
		InvalidDigit: "invalid digit found in string",
		PosOverflow: "number too large to fit in target type",
		NegOverflow: "number too small to fit in target type",
		Zero: "number would be zero for non-zero type",
		NotAPowerOfTwo: "number is not a power of two",
	};
	return fmtPad(f, messages[e.kind]);
}

/** `Debug` of `ParseBoolError`. */
export function fmtParseBoolErrorDebug(_: ParseBoolError, f: Formatter): FmtResult {
	return f.out.writeStr("ParseBoolError");
}

/** `Display` of `ParseBoolError`. */
export function fmtParseBoolErrorDisplay(_: ParseBoolError, f: Formatter): FmtResult {
	return fmtPad(f, "provided string was not `true` or `false`");
}

/** `Debug` of `ParseCharError`. */
export function fmtParseCharErrorDebug(e: ParseCharError, f: Formatter): FmtResult {
	return fmtDebugStructFields(f, "ParseCharError", ["kind"], [(f) => f.out.writeStr(e.kind)]);
}

/** `Display` of `ParseCharError`. */
export function fmtParseCharErrorDisplay(e: ParseCharError, f: Formatter): FmtResult {
	return fmtPad(f, e.kind === "EmptyString" ? "cannot parse char from empty string" : "too many characters in string");
}

/** `Debug` of `ParseFloatError`. */
export function fmtParseFloatErrorDebug(e: ParseFloatError, f: Formatter): FmtResult {
	return fmtDebugStructFields(f, "ParseFloatError", ["kind"], [(f) => f.out.writeStr(e.kind)]);
}

/** `Display` of `ParseFloatError`. */
export function fmtParseFloatErrorDisplay(e: ParseFloatError, f: Formatter): FmtResult {
	return fmtPad(f, e.kind === "Empty" ? "cannot parse float from empty string" : "invalid float literal");
}

/** `Debug` of `PoisonError`. */
export function fmtPoisonErrorDebug(_: unknown, f: Formatter): FmtResult {
	return f.out.writeStr("PoisonError { .. }");
}

/** `Display` of `PoisonError`. */
export function fmtPoisonErrorDisplay(_: unknown, f: Formatter): FmtResult {
	return fmtPad(f, "poisoned lock: another task failed inside");
}

/** `Display` of `TryFromIntError`. */
export function fmtTryFromIntErrorDisplay(e: TryFromIntError, f: Formatter): FmtResult {
	const positive = e.kind === "PosOverflow";
	return fmtPad(f, `number too ${positive ? "large" : "small"} to fit in target type`);
}

/** `Debug` of `Utf8Error`. */
export function fmtUtf8ErrorDebug(e: Utf8Error, f: Formatter): FmtResult {
	return debugStructFinish(
		debugStructField(
			debugStructField(fmtDebugStruct(f, "Utf8Error"), "valid_up_to", (f) => usizeDebug(e.validUpTo, f)),
			"error_len",
			(f) => fmtOption(e.errorLen, f, usizeDebug),
		),
	);
}

/** `Display` of `Utf8Error`, which `write!`s and so ignores the formatter's options. */
export function fmtUtf8ErrorDisplay(e: Utf8Error, f: Formatter): FmtResult {
	if (e.errorLen === undefined) return f.out.writeStr(`incomplete utf-8 byte sequence from index ${e.validUpTo}`);
	return f.out.writeStr(`invalid utf-8 sequence of ${e.errorLen} bytes from index ${e.validUpTo}`);
}

/** `Debug` of `FromUtf8Error`. */
export function fmtFromUtf8ErrorDebug(e: FromUtf8Error, f: Formatter): FmtResult {
	return debugStructFinish(
		debugStructField(
			debugStructField(fmtDebugStruct(f, "FromUtf8Error"), "bytes", (f) => fmtList(byteVecView(e.bytes), f, u8Debug)),
			"error",
			(f) => fmtUtf8ErrorDebug(e.error, f),
		),
	);
}

/** `Display` of `FromUtf8Error`: its `Utf8Error`'s. */
export function fmtFromUtf8ErrorDisplay(e: FromUtf8Error, f: Formatter): FmtResult {
	return fmtUtf8ErrorDisplay(e.error, f);
}

