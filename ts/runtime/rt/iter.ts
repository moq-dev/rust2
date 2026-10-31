// Iterators, for rust2-ts output: std's `Iterator`, its adaptors, and `Range`, lazy as
// Rust's are. `rt.ts` re-exports all of it.
//
// `next` returns an item or `DONE`, never `undefined`, so an iterator of `Option<T>` keeps
// its `None` items apart from its end. Each adaptor calls what it wraps, and the closures
// it was given, in the order and as many times as std's does, and stops where std's
// stops: a `find` or `any` reads no further than its match. Adaptors and consumers are
// functions, `iterMap(it, f)`, so a bundler drops the ones nothing calls; an iterator's
// methods are only what each kind does its own way.
//
// An iterator that owns its items, as a `Vec`'s `into_iter` does, is given what drops
// each, and drops those it still holds as it drops, by `[Symbol.dispose]`; an adaptor
// drops what it wraps. Where std drops an item it skips, rejects, or replaces, the runtime
// function is given what drops it, after its own arguments, and so is a consumer that
// takes the iterator by value, which drops it as std's does. Each kind's `nth`,
// `nth_back`, and `advance_by` are its own, as std's are, reading and dropping what they
// skip in its order. A consumer std folds reads what `folded` returns, which steps
// an inclusive range as its `try_fold` does, and by `nextBack` as its `try_rfold` does,
// as does each adaptor whose std's folds its iterator; `folded(true)` reads as `fold`
// does, which differs only for a zip that reads no more pairs than its size hint. Where
// std's `count` and `last` drop items in an order each kind of iterator has its own, the
// `owned` functions follow it, dispatching on the kind, so a bundle that never drops an
// item carries none of it.

import { ByteSlice, ByteVec, byteVecExtendFromSlice, byteVecPush, Err, type None, Ok, type Place, panic, place, type RangeBound, type Result, sliceView, vecExtendFromSlice } from "../rt.ts";
import { AtIndex, BTreeIter, HashIter, type Ordering } from "./collections.ts";
import { type Hasher, hash8 } from "./hash.ts";
import { charStep, intToBig, type Limbs, lt as wideLt, type Step } from "./int.ts";
import { drop as dispose, dropRange } from "./task.ts";

/** What `next` returns once an iterator is exhausted, which no item is. */
export const DONE: unique symbol = /* @__PURE__ */ Symbol("DONE");

/** The type of `DONE`. */
export type Done = typeof DONE;

/** wasm32's `usize::MAX`, where std's adaptors saturate or split a count. */
const USIZE_MAX = 0xffffffff;

/** What drops a value. */
type Drop<T> = (value: T) => void;

/** `a + b`, saturating at `usize::MAX` as std's size hints do. */
function satAdd(a: number, b: number): number {
	return Math.min(a + b, USIZE_MAX);
}

/** `a + b` of two upper bounds: `None` if either is, or if it passes `usize::MAX`, as std's `checked_add`. */
function addUpper(a: number | undefined, b: number | undefined): number | undefined {
	if (a === undefined || b === undefined) return undefined;
	const sum = a + b;
	return sum > USIZE_MAX ? undefined : sum;
}

/** `Iterator::size_hint`: at least `lower` items, and at most `upper`, if it is known. */
export type SizeHint = [lower: number, upper: number | undefined];

/** The size hint of an iterator that knows it has exactly `n` items. */
export function exactHint(n: number): SizeHint {
	return [n, n];
}

/** std's hint of an adaptor that may yield fewer than `it`, and no more: none at least, and `it`'s upper bound. */
function fewer(it: Iter<unknown>): SizeHint {
	return [0, it.sizeHint()[1]];
}

/**
 * What only the iterators std implements these traits for have, which Rust calls only
 * there: declared, not stubbed, so no bundle carries a stub.
 */
export interface Iter<T> {
	/** `DoubleEndedIterator::next_back`. */
	nextBack(): T | Done;
	/** `ExactSizeIterator::len`. */
	len(): number;
	/** The items it has left, by value, without taking them: what std's `Debug` of it lists. */
	remaining(): Iterable<unknown>;
	/** `Clone::clone`, reading on from where it is, independently of it, by `h`'s clones. */
	clone(h?: Clones): Iter<T>;
}

/**
 * What clones the parts of one of std's iterators that it can't clone itself, as std's
 * `Clone` of each does: `i` and `o` the iterators it wraps, `f` its closure, and `v` an item
 * or another value it holds. A part without one clones as itself.
 */
export interface Clones {
	readonly i?: Clones;
	readonly o?: Clones;
	readonly f?: (f: never) => unknown;
	readonly v?: (value: never) => unknown;
}

/** `value` cloned by `clone`, or itself. */
function cloned<T>(value: T, clone: ((value: never) => unknown) | undefined): T {
	return clone === undefined ? value : (clone as (value: T) => T)(value);
}

/** `Iterator`: `next`, and the methods std's iterators each do their own way. */
export abstract class Iter<T> implements Iterable<T> {
	/** The next item, or `DONE`. */
	abstract next(): T | Done;

	/**
	 * `advance_by`: skip up to `n` items, each dropped by `drop`, and return how many it fell
	 * short. std's folds it, as `folded` steps it.
	 */
	advanceBy(n: number, drop?: Drop<T>): number {
		const folded = this.folded();
		for (let i = 0; i < n; i++) {
			const item = folded.next();
			if (item === DONE) return n - i;
			drop?.(item);
		}
		return 0;
	}

	/** `advance_back_by`, from the back. */
	advanceBackBy(n: number, drop?: Drop<T>): number {
		const folded = this.folded();
		for (let i = 0; i < n; i++) {
			const item = folded.nextBack();
			if (item === DONE) return n - i;
			drop?.(item);
		}
		return 0;
	}

	/** `nth`, as `next` returns it, dropping what it skips by `drop`. */
	nth(n: number, drop?: Drop<T>): T | Done {
		return this.advanceBy(n, drop) === 0 ? this.next() : DONE;
	}

	/** `nth_back`, as `next_back` returns it. */
	nthBack(n: number, drop?: Drop<T>): T | Done {
		return this.advanceBackBy(n, drop) === 0 ? this.nextBack() : DONE;
	}

	/**
	 * What std's `try_fold` reads it by, and `try_rfold` by `nextBack`, or its `fold` and
	 * `rfold` if `fold`: itself, but for an inclusive range, which steps its own way, a zip
	 * whose `fold` stops at its size hint, and an adaptor that folds either.
	 */
	folded(_fold = false): Iter<T> {
		return this;
	}

	/** `size_hint`, as std's of its kind gives it: `(0, None)` unless it knows better. */
	sizeHint(): SizeHint {
		return [0, undefined];
	}

	/**
	 * `fold`, as std's of its kind, which takes the iterator by value and drops what it
	 * holds as std's does. `release` drops what `f` owns, as std drops the closure it was
	 * given: as its fold ends, before the iterator.
	 */
	fold<A>(acc: A, f: (acc: A, item: T) => A, release?: () => void): A {
		return foldBy(this, false, acc, f, release);
	}

	/** `rfold`, from the back, as `fold`. */
	rfold<A>(acc: A, f: (acc: A, item: T) => A, release?: () => void): A {
		return foldBy(this, true, acc, f, release);
	}

	/** Its items, for a JS `for ... of`. */
	*[Symbol.iterator](): Iterator<T> {
		for (let item = this.next(); item !== DONE; item = this.next()) yield item;
	}
}

/**
 * std's default `fold`, or `rfold` if `back`: each item as `folded` steps it, from the
 * back if `back`, and then `release` and `it`.
 */
function foldBy<T, A>(it: Iter<T>, back: boolean, acc: A, f: (acc: A, item: T) => A, release?: () => void): A {
	const folded = it.folded(true);
	for (;;) {
		const item = back ? folded.nextBack() : folded.next();
		if (item === DONE) break;
		acc = f(acc, item);
	}
	ended(it, release);
	return acc;
}

/**
 * The first step of a fold that reads an item by `read` before it folds the rest: whether
 * `it` ended, which drops `release` and then `it` as the fold returns, and the accumulator,
 * then `f` of the item.
 */
function foldFirst<T, A>(it: unknown, read: () => T | Done, acc: A, f: (acc: A, item: T) => A, release?: () => void): [boolean, A] {
	const item = read();
	if (item !== DONE) return [false, f(acc, item)];
	ended(it, release);
	return [true, acc];
}

/** `release`, then `it` dropped, as a fold's closure drops and then its iterator. */
function ended(it: unknown, release?: () => void): void {
	release?.();
	dispose(it);
}

/** What drops `a`'s and then `b`'s, as a closure that captured them in that order drops. */
function then(a: (() => void) | undefined, b: (() => void) | undefined): (() => void) | undefined {
	if (a === undefined) return b;
	if (b === undefined) return a;
	return () => {
		a();
		b();
	};
}

/** An adaptor of `inner`, which drops with it. */
abstract class Adaptor<S, T> extends Iter<T> implements Disposable {
	readonly inner: Iter<S>;

	constructor(inner: Iter<S>) {
		super();
		this.inner = inner;
	}

	[Symbol.dispose](): void {
		dispose(this.inner);
	}
}

/**
 * An adaptor of `inner` that calls `f`, which drops after it, as std's fields do, once:
 * a consumer may drop `inner` before dropping the adaptor that holds it.
 */
abstract class Calling<S, T, F> extends Adaptor<S, T> {
	protected readonly f: F;
	private dropped = false;

	constructor(inner: Iter<S>, f: F) {
		super(inner);
		this.f = f;
	}

	/** What drops `f`, which a fold of `inner` takes from it: dropping it then drops nothing. */
	protected take(): () => void {
		this.dropped = true;
		return () => dispose(this.f);
	}

	override [Symbol.dispose](): void {
		if (this.dropped) return;
		this.dropped = true;
		dispose(this.inner);
		dispose(this.f);
	}
}

/** An adaptor folded, whose `next` and `nextBack` are `front` and `back`, which share its state. */
class Folded<T> extends Iter<T> {
	readonly front: () => T | Done;
	readonly back: () => T | Done;

	constructor(front: () => T | Done, back: () => T | Done) {
		super();
		this.front = front;
		this.back = back;
	}

	next(): T | Done {
		return this.front();
	}

	override nextBack(): T | Done {
		return this.back();
	}
}

/** The item `next` returned, as an `Option`. */
export function iterToOption<T>(item: T | Done): T | undefined;
export function iterToOption<T>(item: T | Done, none: None): T | None;
export function iterToOption<T>(item: T | Done, none?: None): T | undefined | None;
export function iterToOption<T>(item: T | Done, none?: None): T | undefined | None {
	return item === DONE ? none : item;
}

/** An `Option`, as `next` returns it. */
export function iterFromOption<T>(option: T | undefined): T | Done;
export function iterFromOption<T>(option: T | None, none: None): T | Done;
export function iterFromOption<T>(option: T | undefined | None, none?: None): T | Done;
export function iterFromOption<T>(option: T | undefined | None, none?: None): T | Done {
	return option === none ? DONE : (option as T);
}

// Where std's iterators come from.

/** `slice::Iter`, and with what drops each item, what iterating a `Vec` or array by value makes. */
export function iterSlice<T>(s: ArrayLike<T>, drop?: Drop<T>): Iter<T> {
	return new SliceIter(s, drop);
}

/** `slice::IterMut` of elements JS copies: the place of each. */
export function iterSliceMut<T>(s: { [i: number]: T; length: number }): Iter<Place<T>> {
	return new SliceMutIter(s);
}

/** `slice::windows`: each run of `size` elements, panicking on a size of 0 as std's does. */
export function sliceWindows<T>(s: T[], size: number): Iter<T[]> {
	if (size === 0) panic("window size must be non-zero");
	return new PartsIter(s, s.length, size, 1, Math.max(s.length - size + 1, 0), false, sliceCopy);
}

/** `slice::chunks`: each `size` elements, the last fewer, panicking on a size of 0 as std's does. */
export function sliceChunks<T>(s: T[], size: number): Iter<T[]> {
	return chunks(s, s.length, size, false, false, sliceCopy);
}

/** `slice::chunks_exact`: each `size` elements, leaving the rest as its remainder. */
export function sliceChunksExact<T>(s: T[], size: number): Iter<T[]> {
	return chunks(s, s.length, size, true, false, sliceCopy);
}

/** `slice::rchunks`: `chunks` from the back, the last part the front of the slice. */
export function sliceRchunks<T>(s: T[], size: number): Iter<T[]> {
	return chunks(s, s.length, size, false, true, sliceCopy);
}

/** `slice::rchunks_exact`: `chunks_exact` from the back, the remainder the front. */
export function sliceRchunksExact<T>(s: T[], size: number): Iter<T[]> {
	return chunks(s, s.length, size, true, true, sliceCopy);
}

/** `slice::chunks_mut`: as `chunks`, but each part a view of `s` that writes through to it. */
export function sliceChunksMut<T>(s: T[], size: number): Iter<T[]> {
	return chunks(s, s.length, size, false, false, sliceView);
}

/** `slice::chunks_exact_mut`. */
export function sliceChunksExactMut<T>(s: T[], size: number): Iter<T[]> {
	return chunks(s, s.length, size, true, false, sliceView);
}

/** `slice::rchunks_mut`. */
export function sliceRchunksMut<T>(s: T[], size: number): Iter<T[]> {
	return chunks(s, s.length, size, false, true, sliceView);
}

/** `slice::rchunks_exact_mut`. */
export function sliceRchunksExactMut<T>(s: T[], size: number): Iter<T[]> {
	return chunks(s, s.length, size, true, true, sliceView);
}

/** `slice::windows` of bytes. */
export function byteSliceWindows(s: ByteSlice, size: number): Iter<ByteSlice> {
	if (size === 0) panic("window size must be non-zero");
	return new PartsIter(s, s.length, size, 1, Math.max(s.length - size + 1, 0), false, byteSlicePart);
}

/** `slice::chunks` and `chunks_mut` of bytes, whose parts are views already. */
export function byteSliceChunks(s: ByteSlice, size: number): Iter<ByteSlice> {
	return chunks(s, s.length, size, false, false, byteSlicePart);
}

/** `slice::chunks_exact` and `chunks_exact_mut` of bytes. */
export function byteSliceChunksExact(s: ByteSlice, size: number): Iter<ByteSlice> {
	return chunks(s, s.length, size, true, false, byteSlicePart);
}

/** `slice::rchunks` and `rchunks_mut` of bytes. */
export function byteSliceRchunks(s: ByteSlice, size: number): Iter<ByteSlice> {
	return chunks(s, s.length, size, false, true, byteSlicePart);
}

/** `slice::rchunks_exact` and `rchunks_exact_mut` of bytes. */
export function byteSliceRchunksExact(s: ByteSlice, size: number): Iter<ByteSlice> {
	return chunks(s, s.length, size, true, true, byteSlicePart);
}

/**
 * `remainder` of a `ChunksExact` or an `RChunksExact`, and `into_remainder` of their
 * `_mut` forms: what no part holds, however many parts were taken.
 */
export function partsRemainder<S>(parts: Iter<S>): S {
	return (parts as PartsIter<S>).remainder();
}

/** `slice::split`: the parts between the elements `f` is true of, the empty ones too. */
export function sliceSplit<T>(s: T[], f: (item: T) => boolean): Iter<T[]> {
	return new SplitIter(s, s, 0, sliceCopy, f, false, false);
}

/** `slice::split_mut`: as `split`, but each part a view of `s` that writes through to it. */
export function sliceSplitMut<T>(s: T[], f: (item: T) => boolean): Iter<T[]> {
	return new SplitIter(s, s, 0, sliceView, f, false, false);
}

/** `slice::rsplit`: `split` from the back. */
export function sliceRsplit<T>(s: T[], f: (item: T) => boolean): Iter<T[]> {
	return new SplitIter(s, s, 0, sliceCopy, f, true, false);
}

/** `slice::rsplit_mut`. */
export function sliceRsplitMut<T>(s: T[], f: (item: T) => boolean): Iter<T[]> {
	return new SplitIter(s, s, 0, sliceView, f, true, false);
}

/** `slice::split_inclusive`: as `split`, but each part ends with its separator. */
export function sliceSplitInclusive<T>(s: T[], f: (item: T) => boolean): Iter<T[]> {
	return new SplitIter(s, s, 0, sliceCopy, f, false, true);
}

/** `slice::split_inclusive_mut`. */
export function sliceSplitInclusiveMut<T>(s: T[], f: (item: T) => boolean): Iter<T[]> {
	return new SplitIter(s, s, 0, sliceView, f, false, true);
}

/** `slice::splitn`: `split`, but the `n`th part all that is left. */
export function sliceSplitn<T>(s: T[], n: number, f: (item: T) => boolean): Iter<T[]> {
	return new SplitIter(s, s, 0, sliceCopy, f, false, false, n);
}

/** `slice::splitn_mut`. */
export function sliceSplitnMut<T>(s: T[], n: number, f: (item: T) => boolean): Iter<T[]> {
	return new SplitIter(s, s, 0, sliceView, f, false, false, n);
}

/** `slice::rsplitn`: `rsplit`, but the `n`th part all that is left. */
export function sliceRsplitn<T>(s: T[], n: number, f: (item: T) => boolean): Iter<T[]> {
	return new SplitIter(s, s, 0, sliceCopy, f, true, false, n);
}

/** `slice::rsplitn_mut`. */
export function sliceRsplitnMut<T>(s: T[], n: number, f: (item: T) => boolean): Iter<T[]> {
	return new SplitIter(s, s, 0, sliceView, f, true, false, n);
}

/** `slice::split` and `split_mut` of bytes, whose parts are views already. */
export function byteSliceSplit(s: ByteSlice, f: (byte: number) => boolean): Iter<ByteSlice> {
	return new SplitIter(s, s.data, s.start, byteSlicePart, f, false, false);
}

/** `slice::rsplit` and `rsplit_mut` of bytes. */
export function byteSliceRsplit(s: ByteSlice, f: (byte: number) => boolean): Iter<ByteSlice> {
	return new SplitIter(s, s.data, s.start, byteSlicePart, f, true, false);
}

/** `slice::split_inclusive` and `split_inclusive_mut` of bytes. */
export function byteSliceSplitInclusive(s: ByteSlice, f: (byte: number) => boolean): Iter<ByteSlice> {
	return new SplitIter(s, s.data, s.start, byteSlicePart, f, false, true);
}

/** `slice::splitn` and `splitn_mut` of bytes. */
export function byteSliceSplitn(s: ByteSlice, n: number, f: (byte: number) => boolean): Iter<ByteSlice> {
	return new SplitIter(s, s.data, s.start, byteSlicePart, f, false, false, n);
}

/** `slice::rsplitn` and `rsplitn_mut` of bytes. */
export function byteSliceRsplitn(s: ByteSlice, n: number, f: (byte: number) => boolean): Iter<ByteSlice> {
	return new SplitIter(s, s.data, s.start, byteSlicePart, f, true, false, n);
}

/** `slice::Split::as_slice`: what is left to split, or nothing once it is done. */
export function splitAsSlice<S>(parts: Iter<S>): S {
	return (parts as SplitIter<S, unknown>).asSlice();
}

/** `size` elements at a time, all of them or `exact`ly so, from the back if `rev`. */
function chunks<S>(s: S, length: number, size: number, exact: boolean, rev: boolean, part: (s: S, start: number, end: number) => S): Iter<S> {
	if (size === 0) panic("chunk size must be non-zero");
	const count = exact ? Math.floor(length / size) : Math.ceil(length / size);
	return new PartsIter(s, length, size, size, count, rev, part);
}

function sliceCopy<T>(s: T[], start: number, end: number): T[] {
	return s.slice(start, end);
}

function byteSlicePart(s: ByteSlice, start: number, end: number): ByteSlice {
	return new ByteSlice(s.data, s.start + start, end - start);
}

/** `Option::iter`, and with what drops its item, `option::IntoIter`. */
export function iterOption<T>(o: T | undefined, drop?: Drop<T>): Iter<T>;
export function iterOption<T>(o: T | None, drop: Drop<T> | undefined, none: None): Iter<T>;
export function iterOption<T>(o: T | undefined | None, drop?: Drop<T>, none?: None): Iter<T> {
	return new OptionIter(o as T, o !== none, drop);
}

/** `iter::once`. */
export function iterOnce<T>(value: T, drop?: Drop<T>): Iter<T> {
	return new OptionIter<T>(value, true, drop);
}

/** `iter::empty`. */
export function iterEmpty<T>(): Iter<T> {
	return new OptionIter<T>(undefined, false);
}

/** `iter::from_fn`, whose closure returns an `Option`, and drops with it. */
export function iterFromFn<T>(f: () => T | undefined): Iter<T>;
export function iterFromFn<T>(f: () => T | None, none: None): Iter<T>;
export function iterFromFn<T>(f: () => T | undefined | None, none?: None): Iter<T> {
	return new FromNext(f, (f) => iterFromOption(f(), none), dispose);
}

/**
 * `iter::successors`: `first`, then `succ` of each item, called as that item is yielded.
 * The item it holds next drops by `drop`, and then `succ`, as std's fields drop.
 */
export function iterSuccessors<T>(first: T | undefined, succ: (item: T) => T | undefined, drop?: Drop<T>): Iter<T>;
export function iterSuccessors<T>(first: T | None, succ: (item: T) => T | None, drop: Drop<T> | undefined, none: None): Iter<T>;
export function iterSuccessors<T>(first: T | undefined | None, succ: (item: T) => T | undefined | None, drop?: Drop<T>, none?: None): Iter<T> {
	return new SuccessorsIter(first, succ, drop, none);
}

/** An `impl Iterator` of the translated crate, which `next` calls the `next` of, and `drop` drops. */
export function iterFrom<S, T>(source: S, next: (source: S) => T | Done, drop?: Drop<S>): Iter<T> {
	return new FromNext(source, next, drop);
}

/** An iterator borrowed by `&mut`, which std's adaptors take as one: its items, but never it to drop. */
export function iterBorrow<T>(it: Iter<T>): Iter<T> {
	return new Borrowed(it);
}

// How std's `Vec` and `VecDeque` reserve for an iterator they collect or extend by, of
// elements `size` bytes each, which panics past what they may hold: a `TrustedLen` one's
// upper bound first, and else, after each item that finds them full, the lower bound and
// one more; as a tuple's part, the lower bound first. Their capacity is not modeled, so
// one they extend is taken as full, as one holding nothing is, and grows as std's would
// from there.

/** wasm32's `isize::MAX`, the most bytes std's `Layout` describes. */
const ISIZE_MAX = 0x7fffffff;

/** How many `size`-byte elements a `Vec` holds at most: `usize::MAX` of zero-sized ones. */
function vecMax(size: number): number {
	return size === 0 ? USIZE_MAX : Math.floor(ISIZE_MAX / size);
}

/**
 * std's reserve for a `TrustedLen` iterator `it`, in a `Vec` or `VecDeque` holding `len`
 * of `size`-byte elements: its upper bound, which panics without one or past what it may
 * hold.
 */
export function vecReserveExact(it: Iter<unknown>, len: number, size: number): void {
	const upper = it.sizeHint()[1];
	if (upper === undefined || len + upper > vecMax(size)) panic("capacity overflow");
}

/** The capacity of a `Vec` or `VecDeque` holding `len` of `size`-byte elements: full, but for zero-sized ones. */
export function vecCapacity(len: number, size: number): number {
	return size === 0 ? USIZE_MAX : len;
}

/**
 * `join` of slices of `size`-byte elements by the slice `sep`, or `concat` without one:
 * each item's elements, read from it by `slice` unless it is the slice, with `sep`'s
 * between them, each cloned in order by `clone` if given.
 */
export function slicesJoin<V, T>(items: readonly V[], size: number, sep?: ArrayLike<T>, slice?: (item: V) => ArrayLike<T>, clone?: (item: T) => T): T[] {
	const parts = slice === undefined ? (items as unknown as readonly ArrayLike<T>[]) : items.map(slice);
	joinedLength(parts, sep, size);
	const v: T[] = [];
	for (let i = 0; i < parts.length; i++) {
		if (i > 0 && sep !== undefined) vecExtendFromSlice(v, sep, clone);
		vecExtendFromSlice(v, parts[i], clone);
	}
	return v;
}

/**
 * `join` of byte slices by the bytes `sep`, or `concat` without them: each item's bytes,
 * read from it by `slice` unless it is the slice, with `sep` between them, into a
 * `Vec<u8>` of the capacity std's has, their exact length.
 */
export function byteSlicesJoin<V>(items: readonly V[], sep?: ByteSlice, slice?: (item: V) => ByteSlice): ByteVec {
	const parts = slice === undefined ? (items as unknown as readonly ByteSlice[]) : items.map(slice);
	const v = new ByteVec(joinedLength(parts, sep, 1));
	for (let i = 0; i < parts.length; i++) {
		if (i > 0 && sep !== undefined) byteVecExtendFromSlice(v, sep);
		byteVecExtendFromSlice(v, parts[i]);
	}
	return v;
}

/**
 * What `concat` or `join` by `sep` of `parts`, of `size`-byte elements, reserves before it
 * clones any, as std's does: its sum of the parts' lengths panics past `usize::MAX`, the
 * separators' count wraps, and `Vec::with_capacity` of that panics past what the `Vec` may
 * hold. Where the count wraps, the `Vec` grows as it fills, which memory bounds first.
 */
function joinedLength(parts: readonly { length: number }[], sep: { length: number } | undefined, size: number): number {
	let sum = 0;
	for (const part of parts) sum += part.length;
	if (sum > USIZE_MAX) panic("attempt to add with overflow");
	const seps = sep === undefined || parts.length === 0 ? 0 : Math.imul(sep.length, parts.length - 1) >>> 0;
	const length = (sum + seps) % (USIZE_MAX + 1);
	if (length > vecMax(size)) panic("capacity overflow");
	return length;
}

/**
 * The capacity a full `Vec` or `VecDeque` of `cap` `size`-byte elements grows to as it
 * reserves for the next item of `it`, as std's `RawVec` amortizes it, or a panic past what
 * it may hold.
 */
export function vecGrow(it: Iter<unknown>, cap: number, size: number): number {
	return grow(cap, cap + satAdd(it.sizeHint()[0], 1), size);
}

/** The capacity `cap` of `size`-byte elements grows to as std's `RawVec` amortizes it to hold `required`, or a panic past what it may hold. */
function grow(cap: number, required: number, size: number): number {
	const grown = Math.max(cap * 2, required, size === 1 ? 8 : size <= 1024 ? 4 : 1);
	if (grown > vecMax(size)) panic("capacity overflow");
	return grown;
}

/**
 * The capacity of a `Vec` or `VecDeque` of capacity `cap`, or else full, holding `len` of
 * `size`-byte elements, once it reserved `additional` more, growing as `vecGrow` does.
 */
export function vecReserve(len: number, additional: number, size: number, cap = vecCapacity(len, size)): number {
	return additional > cap - len ? grow(cap, len + additional, size) : cap;
}

/** The capacity of one as `vecReserve` takes it once it pushed an item, growing by one as std's `grow_one` does once full. */
export function vecPush(len: number, size: number, cap = vecCapacity(len, size)): number {
	return len === cap ? grow(cap, len + 1, size) : cap;
}

/**
 * The reserve std's `VecDeque` makes at the first item it extends by, holding `len`, for
 * what `it` holds past it, which panics past `usize::MAX` even where its zero-sized
 * elements never fill it.
 */
export function dequeReserve(it: Iter<unknown>, len: number): void {
	if (len + satAdd(it.sizeHint()[0], 1) > USIZE_MAX) panic("capacity overflow");
}

/**
 * `Extend::extend` of a `Vec`, which takes `items` as `take` says, and with elements
 * `size` bytes each, reserves as std's does for a `trusted`, `TrustedLen`, iterator or any
 * other.
 */
export function iterExtend<T>(v: T[], items: Iter<T>, take?: Take, size?: number, trusted = false): T[] {
	if (size !== undefined && !trusted) {
		let cap = vecCapacity(v.length, size);
		for (let item = items.next(); item !== DONE; item = items.next()) {
			if (v.length === cap) cap = vecGrow(items, cap, size);
			v.push(item);
		}
		if (take !== undefined) dispose(items);
		return v;
	}
	if (size !== undefined) vecReserveExact(items, v.length, size);
	if (take === undefined) {
		const folded = items.folded(trusted);
		for (let item = folded.next(); item !== DONE; item = folded.next()) v.push(item);
		return v;
	}
	each(items, (item) => v.push(item), take);
	return v;
}

/** `Extend::extend` of a `Vec<u8>`, reserving as `iterExtend` does. */
export function iterExtendBytes(v: ByteVec, items: Iter<number>, take?: Take, trusted = false): ByteVec {
	if (!trusted) {
		let cap = v.length;
		for (let item = items.next(); item !== DONE; item = items.next()) {
			if (v.length === cap) cap = vecGrow(items, cap, 1);
			byteVecPush(v, item);
		}
		if (take !== undefined) dispose(items);
		return v;
	}
	vecReserveExact(items, v.length, 1);
	if (take === undefined) {
		const folded = items.folded(true);
		for (let item = folded.next(); item !== DONE; item = folded.next()) byteVecPush(v, item);
		return v;
	}
	each(items, (item) => byteVecPush(v, item), take);
	return v;
}

// Adaptors

/** `Iterator::map`. */
export function iterMap<T, U>(it: Iter<T>, f: (item: T) => U): Iter<U> {
	return new MapIter(it, f);
}

/**
 * `it` with each item lent by `f` as translated code reads it, as a `&[u8; N]` of a
 * `Uint8Array` is lent as an `rt.ByteSlice`.
 */
export function iterLend<T, U>(it: Iter<T>, f: (item: T) => U): Iter<U> {
	return new LendIter(it, f);
}

/** `Iterator::filter`, dropping what `f` rejects by `drop`. */
export function iterFilter<T>(it: Iter<T>, f: (item: T) => boolean, drop?: Drop<T>): Iter<T> {
	return new FilterIter(it, f, drop);
}

/** `Iterator::filter_map`. */
export function iterFilterMap<T, U>(it: Iter<T>, f: (item: T) => U | undefined): Iter<U>;
export function iterFilterMap<T, U>(it: Iter<T>, f: (item: T) => U | None, none: None): Iter<U>;
export function iterFilterMap<T, U>(it: Iter<T>, f: (item: T) => U | undefined | None, none?: None): Iter<U> {
	return new FilterMapIter(it, f, none);
}

/** `Iterator::enumerate`. */
export function iterEnumerate<T>(it: Iter<T>): Iter<[number, T]> {
	return new EnumerateIter(it);
}

/** `chain`, with `other` already made an iterator, as std does when it is called. */
export function iterChain<T>(it: Iter<T>, other: Iter<T>): Iter<T> {
	return new ChainIter(it, other);
}

/**
 * How std's `Zip` reads its iterators, by the traits both implement: two
 * `TrustedRandomAccess` ones, as whether each may have side effects (`MAY_HAVE_SIDE_EFFECT`),
 * which it reads by index up to the shorter length and past it only one that may; and
 * `len` of two `TrustedLen` ones, whose `fold` reads as many pairs as its size hint's
 * upper bound, where `next` reads the first once past a shorter second.
 */
export type Trusted = readonly [boolean, boolean] | "len";

/**
 * `zip`, with `other` already made an iterator, as std does when it is called, read as
 * `trusted` says, dropping an item of either it reads without one of the other by `dropA`
 * or `dropB`.
 */
export function iterZip<T, U>(it: Iter<T>, other: Iter<U>, trusted?: Trusted, dropA?: Drop<T>, dropB?: Drop<U>): Iter<[T, U]> {
	if (trusted === undefined || trusted === "len") return new ZipIter(it, other, trusted === "len", dropA, dropB);
	return new TrustedZipIter(it, other, trusted, dropA, dropB);
}

/** `Iterator::rev`. */
export function iterRev<T>(it: Iter<T>): Iter<T> {
	return new RevIter(it);
}

/** `Iterator::skip`, dropping what it skips by `drop`, through its iterator's own `nth`. */
export function iterSkip<T>(it: Iter<T>, n: number, drop?: Drop<T>): Iter<T> {
	return new SkipIter(it, n, drop);
}

/** `Iterator::take`, dropping what `next_back` skips past its limit by `drop`. */
export function iterTake<T>(it: Iter<T>, n: number, drop?: Drop<T>): Iter<T> {
	return new TakeIter(it, n, drop);
}

/** `Iterator::skip_while`, dropping what it skips by `drop`. */
export function iterSkipWhile<T>(it: Iter<T>, f: (item: T) => boolean, drop?: Drop<T>): Iter<T> {
	return new SkipWhileIter(it, f, drop);
}

/** `Iterator::take_while`, dropping the item that ends it by `drop`. */
export function iterTakeWhile<T>(it: Iter<T>, f: (item: T) => boolean, drop?: Drop<T>): Iter<T> {
	return new TakeWhileIter(it, f, drop);
}

/** `Iterator::map_while`. */
export function iterMapWhile<T, U>(it: Iter<T>, f: (item: T) => U | undefined): Iter<U>;
export function iterMapWhile<T, U>(it: Iter<T>, f: (item: T) => U | None, none: None): Iter<U>;
export function iterMapWhile<T, U>(it: Iter<T>, f: (item: T) => U | undefined | None, none?: None): Iter<U> {
	return new MapWhileIter(it, f, none);
}

/** `Iterator::step_by`, panicking on a step of 0, and dropping what it steps over by `drop`. */
export function iterStepBy<T>(it: Iter<T>, step: number, drop?: Drop<T>): Iter<T> {
	return new StepByIter(it, step, drop);
}

/**
 * `Iterator::cycle`: `it`, and then clones of it as it was, by `h`, as std's `Clone` of
 * it clones each part. It holds the first clone as it is made.
 */
export function iterCycle<T>(it: Iter<T>, h?: Clones): Iter<T> {
	return new CycleIter(it.clone(h), it, h);
}

/**
 * `Iterator::copied` of items JS copies by `copy`, which std's skips through: its `nth`
 * and `advance_by` are its iterator's.
 */
export function iterCopied<R, T>(it: Iter<R>, copy: (item: R) => T): Iter<T> {
	return new CopiedIter(it, copy);
}

/** `Iterator::cloned` of items that are JS values, whose `nth` reads by `next`, as std's does. */
export function iterCloned<T>(it: Iter<T>): Iter<T> {
	return new MapIter(it, same);
}

/** `Iterator::inspect`. */
export function iterInspect<T>(it: Iter<T>, f: (item: T) => void): Iter<T> {
	return new InspectIter(it, f);
}

/**
 * `Iterator::fuse`, which drops `it` once it ends, unless it is `fused`, a
 * `FusedIterator`, which std's keeps.
 */
export function iterFuse<T>(it: Iter<T>, fused = false): Iter<T> {
	return new FuseIter(it, fused);
}

/** `Iterator::peekable`, which drops an item it peeked by `drop`. */
export function iterPeekable<T>(it: Iter<T>, drop?: Drop<T>): Peekable<T> {
	return new Peekable(it, drop);
}

/**
 * `flat_map`: `into` makes each of `f`'s results an iterator, as `IntoIterator` does. Its
 * source drops once it ends unless it is `fused`, as `fuse` does. `size` and `oneShot`
 * are as `flatten`'s.
 */
export function iterFlatMap<T, V>(it: Iter<T>, f: (item: T) => Iter<V>): Iter<V>;
export function iterFlatMap<T, U, V>(it: Iter<T>, f: (item: T) => U, into?: (value: U) => Iter<V>, fused?: boolean, size?: number, oneShot?: boolean): Iter<V>;
export function iterFlatMap<T, U, V>(it: Iter<T>, f: (item: T) => U, into?: (value: U) => Iter<V>, fused = false, size?: number, oneShot = false): Iter<V> {
	return new FlattenIter(new MapIter(it, f), into ?? (itself as (value: U) => Iter<V>), fused, size, oneShot);
}

/**
 * `flatten`: `into` makes each item an iterator, as `IntoIterator` does, `fused` is as
 * `flat_map`'s, `size` is each item's length if it is an array, and `oneShot` says each
 * item's iterator has at most one item, as std's size hint specializes on.
 */
export function iterFlatten<V>(it: Iter<Iter<V>>): Iter<V>;
export function iterFlatten<T, V>(it: Iter<T>, into?: (value: T) => Iter<V>, fused?: boolean, size?: number, oneShot?: boolean): Iter<V>;
export function iterFlatten<T, V>(it: Iter<T>, into?: (value: T) => Iter<V>, fused = false, size?: number, oneShot = false): Iter<V> {
	return new FlattenIter(it, into ?? (itself as (value: T) => Iter<V>), fused, size, oneShot);
}

// Consumers. Each that takes an iterator by value that `drops` takes it as std's does: by
// its own `fold` where std's folds, which drops what it holds as it goes, or as `take`
// says, and else by `next`, dropping it once done.

/**
 * How a consumer takes an iterator by value, where std's may either way: by `next`, as
 * std's does one it can't trust the length of, or by its own `fold`, which drops what it
 * holds. Without either, it drops nothing, and std's folds it.
 */
export type Take = "next" | "fold";

/** `it`, taken as `take` says: `f` of each item, and then it dropped. */
function each<T>(it: Iter<T>, f: (item: T) => void, take: Take | undefined): void {
	if (take === "fold") {
		it.fold<void>(undefined, (_, item) => f(item));
		return;
	}
	consumed(it, () => {
		for (let item = it.next(); item !== DONE; item = it.next()) f(item);
	});
}

/** `Iterator::count` of an iterator that drops nothing. */
export function iterCount<T>(it: Iter<T>): number {
	const folded = it.folded(true);
	let n = 0;
	while (folded.next() !== DONE) n++;
	return n;
}

/** `Iterator::last` of an iterator that drops nothing. */
export function iterLast<T>(it: Iter<T>): T | undefined;
export function iterLast<T>(it: Iter<T>, none: None): T | None;
export function iterLast<T>(it: Iter<T>, none?: None): T | undefined | None {
	const folded = it.folded(true);
	let last: T | Done = DONE;
	for (let item = folded.next(); item !== DONE; item = folded.next()) last = item;
	return iterToOption(last, none);
}

/** `Iterator::nth`, dropping what it skips by `drop`, as its own `nth` does. */
export function iterNth<T>(it: Iter<T>, n: number, drop?: Drop<T>): T | undefined;
export function iterNth<T>(it: Iter<T>, n: number, drop: Drop<T> | undefined, none: None): T | None;
export function iterNth<T>(it: Iter<T>, n: number, drop?: Drop<T>, none?: None): T | undefined | None {
	return iterToOption(it.nth(n, drop), none);
}

/** `DoubleEndedIterator::nth_back`, dropping what it skips by `drop`, as its own `nth_back` does. */
export function iterNthBack<T>(it: Iter<T>, n: number, drop?: Drop<T>): T | undefined;
export function iterNthBack<T>(it: Iter<T>, n: number, drop: Drop<T> | undefined, none: None): T | None;
export function iterNthBack<T>(it: Iter<T>, n: number, drop?: Drop<T>, none?: None): T | undefined | None {
	return iterToOption(it.nthBack(n, drop), none);
}

/** `Iterator::any`. */
export function iterAny<T>(it: Iter<T>, f: (item: T) => boolean): boolean {
	const folded = it.folded();
	for (let item = folded.next(); item !== DONE; item = folded.next()) if (f(item)) return true;
	return false;
}

/** `Iterator::all`. */
export function iterAll<T>(it: Iter<T>, f: (item: T) => boolean): boolean {
	const folded = it.folded();
	for (let item = folded.next(); item !== DONE; item = folded.next()) if (!f(item)) return false;
	return true;
}

/** `find`, as `next` returns it: what `f` rejects drops by `drop`. */
function findItem<T>(it: Iter<T>, f: (item: T) => boolean, drop?: Drop<T>): T | Done {
	const folded = it.folded();
	for (let item = folded.next(); item !== DONE; item = folded.next()) {
		if (f(item)) return item;
		drop?.(item);
	}
	return DONE;
}

/** `rfind`, as `next_back` returns it, as `try_rfold` steps it. */
function rfindItem<T>(it: Iter<T>, f: (item: T) => boolean, drop?: Drop<T>): T | Done {
	const folded = it.folded();
	for (let item = folded.nextBack(); item !== DONE; item = folded.nextBack()) {
		if (f(item)) return item;
		drop?.(item);
	}
	return DONE;
}

/** `Iterator::find`, dropping what `f` rejects by `drop`. */
export function iterFind<T>(it: Iter<T>, f: (item: T) => boolean, drop?: Drop<T>): T | undefined;
export function iterFind<T>(it: Iter<T>, f: (item: T) => boolean, drop: Drop<T> | undefined, none: None): T | None;
export function iterFind<T>(it: Iter<T>, f: (item: T) => boolean, drop?: Drop<T>, none?: None): T | undefined | None {
	return iterToOption(findItem(it, f, drop), none);
}

/** `Iterator::find_map`. */
export function iterFindMap<T, U>(it: Iter<T>, f: (item: T) => U | undefined): U | undefined;
export function iterFindMap<T, U>(it: Iter<T>, f: (item: T) => U | None, none: None): U | None;
export function iterFindMap<T, U>(it: Iter<T>, f: (item: T) => U | undefined | None, none?: None): U | undefined | None {
	const folded = it.folded();
	for (let item = folded.next(); item !== DONE; item = folded.next()) {
		const found = f(item);
		if (found !== none) return found;
	}
	return none;
}

/** `DoubleEndedIterator::rfind`, dropping what `f` rejects by `drop`. */
export function iterRfind<T>(it: Iter<T>, f: (item: T) => boolean, drop?: Drop<T>): T | undefined;
export function iterRfind<T>(it: Iter<T>, f: (item: T) => boolean, drop: Drop<T> | undefined, none: None): T | None;
export function iterRfind<T>(it: Iter<T>, f: (item: T) => boolean, drop?: Drop<T>, none?: None): T | undefined | None {
	return iterToOption(rfindItem(it, f, drop), none);
}

/** `Iterator::rposition`: from the back, where its item is counting from the front. */
export function iterRposition<T>(it: Iter<T>, f: (item: T) => boolean): number | undefined {
	let i = it.len();
	const folded = it.folded();
	for (let item = folded.nextBack(); item !== DONE; item = folded.nextBack()) {
		i--;
		if (f(item)) return i;
	}
	return undefined;
}

/** `Iterator::position`. */
export function iterPosition<T>(it: Iter<T>, f: (item: T) => boolean): number | undefined {
	const folded = it.folded();
	let i = 0;
	for (let item = folded.next(); item !== DONE; item = folded.next()) {
		if (f(item)) return i;
		i++;
	}
	return undefined;
}

/** `Iterator::reduce`: the first item by `next`, folded with the rest by `f`. */
export function iterReduce<T>(it: Iter<T>, f: (acc: T, item: T) => T, drops?: boolean): T | undefined;
export function iterReduce<T>(it: Iter<T>, f: (acc: T, item: T) => T, drops: boolean | undefined, none: None): T | None;
export function iterReduce<T>(it: Iter<T>, f: (acc: T, item: T) => T, drops = false, none?: None): T | undefined | None {
	const first = it.next();
	if (first !== DONE) return iterFold(it, first, f, drops);
	if (drops) dispose(it);
	return none;
}

/**
 * `Iterator::fold`. If it `owns` `f`, a closure that drops, `f` drops as the fold ends,
 * before `it`, as std's drops the closure it was given.
 */
export function iterFold<T, A>(it: Iter<T>, init: A, f: (acc: A, item: T) => A, drops = false, owns = false): A {
	if (!drops) {
		const folded = it.folded(true);
		let acc = init;
		for (let item = folded.next(); item !== DONE; item = folded.next()) acc = f(acc, item);
		if (owns) dispose(f);
		return acc;
	}
	return it.fold(init, f, owns ? () => dispose(f) : undefined);
}

/** `Iterator::for_each`, which drops `f` if it `owns` it, as `iterFold` does. */
export function iterForEach<T>(it: Iter<T>, f: (item: T) => void, drops = false, owns = false): void {
	if (!drops) {
		const folded = it.folded(true);
		for (let item = folded.next(); item !== DONE; item = folded.next()) f(item);
		if (owns) dispose(f);
		return;
	}
	it.fold<void>(undefined, (_, item) => f(item), owns ? () => dispose(f) : undefined);
}

/** `sum`, adding with an integer type's checked `add`, so overflow panics as Rust's does. */
export function iterSum<T>(it: Iter<T>, add: (a: T, b: T) => T, zero: T, drops = false): T {
	return iterFold(it, zero, add, drops);
}

/** `product`, multiplying with an integer type's checked `mul`. */
export function iterProduct<T>(it: Iter<T>, mul: (a: T, b: T) => T, one: T, drops = false): T {
	return iterFold(it, one, mul, drops);
}

/** `max`, by `lt`, its items' `PartialOrd::lt`, which `Ord::max` tests: the last of the greatest. */
export function iterMax<T>(it: Iter<T>, lt: Lt<T>, drops?: boolean, drop?: Drop<T>): T | undefined;
export function iterMax<T>(it: Iter<T>, lt: Lt<T>, drops: boolean | undefined, drop: Drop<T> | undefined, none: None): T | None;
export function iterMax<T>(it: Iter<T>, lt: Lt<T>, drops = false, drop?: Drop<T>, none?: None): T | undefined | None {
	return iterToOption(best(it, (item: T) => item, lt, false, drops, drop), none);
}

/** `min`, by `lt`, as `iterMax`: the first of the least. */
export function iterMin<T>(it: Iter<T>, lt: Lt<T>, drops?: boolean, drop?: Drop<T>): T | undefined;
export function iterMin<T>(it: Iter<T>, lt: Lt<T>, drops: boolean | undefined, drop: Drop<T> | undefined, none: None): T | None;
export function iterMin<T>(it: Iter<T>, lt: Lt<T>, drops = false, drop?: Drop<T>, none?: None): T | undefined | None {
	return iterToOption(best(it, (item: T) => item, lt, true, drops, drop), none);
}

/** `max_by_key`, by `lt` of the keys, as `iterMax`, each item's key taken once. */
export function iterMaxByKey<T, K>(it: Iter<T>, f: (item: T) => K, lt: Lt<K>, drops?: boolean, drop?: Drop<T>): T | undefined;
export function iterMaxByKey<T, K>(it: Iter<T>, f: (item: T) => K, lt: Lt<K>, drops: boolean | undefined, drop: Drop<T> | undefined, none: None): T | None;
export function iterMaxByKey<T, K>(it: Iter<T>, f: (item: T) => K, lt: Lt<K>, drops = false, drop?: Drop<T>, none?: None): T | undefined | None {
	return iterToOption(best(it, f, lt, false, drops, drop), none);
}

/** `min_by_key`, by `lt` of the keys, as `iterMin`, each item's key taken once. */
export function iterMinByKey<T, K>(it: Iter<T>, f: (item: T) => K, lt: Lt<K>, drops?: boolean, drop?: Drop<T>): T | undefined;
export function iterMinByKey<T, K>(it: Iter<T>, f: (item: T) => K, lt: Lt<K>, drops: boolean | undefined, drop: Drop<T> | undefined, none: None): T | None;
export function iterMinByKey<T, K>(it: Iter<T>, f: (item: T) => K, lt: Lt<K>, drops = false, drop?: Drop<T>, none?: None): T | undefined | None {
	return iterToOption(best(it, f, lt, true, drops, drop), none);
}

/** `max_by`, by `compare`, as `iterMax`: the last of the greatest. */
export function iterMaxBy<T>(it: Iter<T>, compare: Compare<T>, drops?: boolean, drop?: Drop<T>): T | undefined;
export function iterMaxBy<T>(it: Iter<T>, compare: Compare<T>, drops: boolean | undefined, drop: Drop<T> | undefined, none: None): T | None;
export function iterMaxBy<T>(it: Iter<T>, compare: Compare<T>, drops = false, drop?: Drop<T>, none?: None): T | undefined | None {
	return iterToOption(best(it, (item: T) => item, byCompare(compare), false, drops, drop), none);
}

/** `min_by`, by `compare`, as `iterMin`: the first of the least. */
export function iterMinBy<T>(it: Iter<T>, compare: Compare<T>, drops?: boolean, drop?: Drop<T>): T | undefined;
export function iterMinBy<T>(it: Iter<T>, compare: Compare<T>, drops: boolean | undefined, drop: Drop<T> | undefined, none: None): T | None;
export function iterMinBy<T>(it: Iter<T>, compare: Compare<T>, drops = false, drop?: Drop<T>, none?: None): T | undefined | None {
	return iterToOption(best(it, (item: T) => item, byCompare(compare), true, drops, drop), none);
}

/** A `PartialOrd::lt`. */
type Lt<T> = (a: T, b: T) => boolean;

/** A comparator, as `max_by` and `min_by` call it: of the best so far, and then the next. */
type Compare<T> = (best: T, next: T) => Ordering;

/**
 * `compare` as the `lt` that `best` tests the next and the best by: whether the best is
 * `Greater`, which std's `cmp::max_by` keeps and `min_by` replaces.
 */
function byCompare<T>(compare: Compare<T>): Lt<T> {
	return (next, best) => compare(best, next) > 0;
}

/**
 * The item of the greatest key, the last of equal ones, or of the least if `least`, the
 * first, as std's `reduce` by `Ord::max` or `min` keeps it: the first by `next`, and the
 * rest by `fold`, each tested for `lt` the best's and dropped as it loses.
 */
function best<T, K>(it: Iter<T>, f: (item: T) => K, lt: Lt<K>, least: boolean, drops: boolean, drop?: Drop<T>): T | Done {
	if (!drops && drop === undefined) {
		let best = it.next();
		if (best === DONE) return DONE;
		let key = f(best);
		const folded = it.folded(true);
		for (let item = folded.next(); item !== DONE; item = folded.next()) {
			const k = f(item);
			if (least ? lt(k, key) : !lt(k, key)) [best, key] = [item, k];
		}
		return best;
	}
	const first = it.next();
	if (first === DONE) {
		dispose(it);
		return DONE;
	}
	const [, item] = it.fold<[K, T]>([f(first), first], (best, item) => {
		const k = f(item);
		const wins = least ? lt(k, best[0]) : !lt(k, best[0]);
		drop?.(wins ? best[1] : item);
		return wins ? [k, item] : best;
	});
	return item;
}

/**
 * `is_sorted_by`: whether `compare` holds of each item and the next, read by `next` and
 * then as `all` folds, stopping at the first it fails. Each item drops by `drop` as the
 * next replaces it, or after the one it failed against; then `compare` by `dropCompare`,
 * as `all` drops it, the last item, and `it`, if `drops`.
 */
export function iterIsSortedBy<T>(
	it: Iter<T>,
	compare: (a: T, b: T) => boolean,
	drops = false,
	drop?: Drop<T>,
	dropCompare?: Drop<(a: T, b: T) => boolean>,
): boolean {
	let last = it.next();
	let sorted = true;
	if (last !== DONE) {
		const folded = it.folded();
		for (let item = folded.next(); item !== DONE; item = folded.next()) {
			if (!compare(last, item)) {
				drop?.(item);
				sorted = false;
				break;
			}
			drop?.(last);
			last = item;
		}
	}
	dropCompare?.(compare);
	if (last !== DONE) drop?.(last);
	if (drops) dispose(it);
	return sorted;
}

/**
 * What std's comparisons of iterators drop, after the iterators and the comparator: whether
 * the first iterator drops, whether the second does, which drops before it, and whether
 * the comparator owns values that drop, as a `_by` form's may. Then what drops an item of
 * each that std drops itself: one of the first read past the second's end, and one of the
 * second read past the first's.
 */
type Compared<T, U> = [drops?: boolean, dropsOther?: boolean, owns?: boolean, dropX?: Drop<T>, dropY?: Drop<U>];

/**
 * std's `iter_compare`: each item of `a` as `try_fold` reads it, against the next of `b`,
 * by `f`, which takes both, until it finds them other than `Equal`. If `b` ends first, the
 * item of `a` read past it drops by `dropX`, and `a` is `Greater`. `release` drops what
 * `f` owns once `a`'s fold ends, as std drops the closure there; then, if `a` ended, `b`
 * reads once more: `Less` if it has another, which drops by `dropY`, and else `Equal`.
 */
function iterCompare<T, U, O extends Ordering | undefined>(
	a: Iter<T>,
	b: Iter<U>,
	f: (x: T, y: U) => O,
	release?: () => void,
	dropX?: Drop<T>,
	dropY?: Drop<U>,
): O {
	const folded = a.folded();
	for (let x = folded.next(); x !== DONE; x = folded.next()) {
		const y = b.next();
		if (y === DONE) {
			dropX?.(x);
			release?.();
			return 1 as O;
		}
		const order = f(x, y);
		if (order !== 0) {
			release?.();
			return order;
		}
	}
	release?.();
	const y = b.next();
	if (y === DONE) return 0 as O;
	dropY?.(y);
	return -1 as O;
}

/** `f` as std's own closure of a comparison, which drops what it is given, the second first, once it compares them. */
export function consuming<T, U, R>(f: (x: T, y: U) => R, dropX?: Drop<T>, dropY?: Drop<U>): (x: T, y: U) => R {
	return (x, y) => {
		const result = f(x, y);
		dropY?.(y);
		dropX?.(x);
		return result;
	};
}

/**
 * `Iterator::cmp`, `partial_cmp`, and their `_by` forms: `iter_compare` of `a` and `b`
 * by `cmp`, and then `b` and `a` dropped as `Compared` says.
 */
export function iterCmp<T, U, O extends Ordering | undefined>(
	a: Iter<T>,
	b: Iter<U>,
	cmp: (x: T, y: U) => O,
	...[drops = false, dropsOther = false, owns = false, dropX, dropY]: Compared<T, U>
): O {
	const order = iterCompare(a, b, cmp, owns ? () => dispose(cmp) : undefined, dropX, dropY);
	if (dropsOther) dispose(b);
	if (drops) dispose(a);
	return order;
}

/**
 * `Iterator::eq` and `eq_by`: `iter_compare` of `a` and `b` until `eq` fails. If `trusted`,
 * as std's of two `TrustedLen` iterators, unequal at once, reading neither, unless their
 * size hints' upper bounds are the same or both unknown.
 */
export function iterEq<T, U>(a: Iter<T>, b: Iter<U>, eq: (x: T, y: U) => boolean, trusted = false, ...[drops = false, dropsOther = false, owns = false, dropX, dropY]: Compared<T, U>): boolean {
	const release = owns ? () => dispose(eq) : undefined;
	let equal = false;
	if (trusted && a.sizeHint()[1] !== b.sizeHint()[1]) release?.();
	else equal = iterCompare(a, b, (x: T, y: U) => (eq(x, y) ? 0 : 1), release, dropX, dropY) === 0;
	if (dropsOther) dispose(b);
	if (drops) dispose(a);
	return equal;
}

/** `Iterator::lt`: whether `iterCmp` by `partial_cmp` is `Less`. */
export function iterLt<T, U>(a: Iter<T>, b: Iter<U>, cmp: (x: T, y: U) => Ordering | undefined, ...rest: Compared<T, U>): boolean {
	return iterCmp(a, b, cmp, ...rest) === -1;
}

/** `Iterator::le`: whether `iterCmp` by `partial_cmp` is `Less` or `Equal`. */
export function iterLe<T, U>(a: Iter<T>, b: Iter<U>, cmp: (x: T, y: U) => Ordering | undefined, ...rest: Compared<T, U>): boolean {
	const order = iterCmp(a, b, cmp, ...rest);
	return order !== undefined && order <= 0;
}

/** `Iterator::gt`: whether `iterCmp` by `partial_cmp` is `Greater`. */
export function iterGt<T, U>(a: Iter<T>, b: Iter<U>, cmp: (x: T, y: U) => Ordering | undefined, ...rest: Compared<T, U>): boolean {
	return iterCmp(a, b, cmp, ...rest) === 1;
}

/** `Iterator::ge`: whether `iterCmp` by `partial_cmp` is `Greater` or `Equal`. */
export function iterGe<T, U>(a: Iter<T>, b: Iter<U>, cmp: (x: T, y: U) => Ordering | undefined, ...rest: Compared<T, U>): boolean {
	const order = iterCmp(a, b, cmp, ...rest);
	return order !== undefined && order >= 0;
}

/** `collect` into a `Vec`, as `iterExtend` of an empty one. */
export function iterToArray<T>(it: Iter<T>, take?: Take, size?: number, trusted = false): T[] {
	const out: T[] = [];
	if (size !== undefined && !trusted) {
		let cap = vecCapacity(0, size);
		for (let item = it.next(); item !== DONE; item = it.next()) {
			if (out.length === cap) cap = vecGrow(it, cap, size);
			out.push(item);
		}
		if (take !== undefined) dispose(it);
		return out;
	}
	if (size !== undefined) vecReserveExact(it, 0, size);
	if (take === undefined) {
		const folded = it.folded(trusted);
		for (let item = folded.next(); item !== DONE; item = folded.next()) out.push(item);
		return out;
	}
	each(it, (item) => out.push(item), take);
	return out;
}

/** `collect` into a `Vec<u8>`, as `iterExtendBytes` of an empty one. */
export function iterToByteVec(it: Iter<number>, take?: Take, trusted = false): ByteVec {
	const out = new ByteVec();
	// Its own loops: through `iterExtendBytes`, JavaScriptCore took 1.9x as long.
	if (!trusted) {
		let cap = 0;
		for (let item = it.next(); item !== DONE; item = it.next()) {
			if (out.length === cap) cap = vecGrow(it, cap, 1);
			byteVecPush(out, item);
		}
		if (take !== undefined) dispose(it);
		return out;
	}
	vecReserveExact(it, 0, 1);
	if (take === undefined) {
		const folded = it.folded(true);
		for (let item = folded.next(); item !== DONE; item = folded.next()) byteVecPush(out, item);
		return out;
	}
	each(it, (item) => byteVecPush(out, item), take);
	return out;
}

/**
 * `collect` of `Result`s: the first error, or every value collected by `into`, which drops
 * the iterator. On an error, what it collected drops by `drop`.
 */
export function iterCollectResult<U, E, C>(it: Iter<Result<U, E>>, into: (values: Iter<U>) => C, drop?: Drop<C>): Result<C, E> {
	const shunt = new ResultShunt(it);
	const values = into(shunt);
	if (shunt.error === undefined) return Ok(values);
	const error = Err(shunt.error.value);
	drop?.(values);
	return error;
}

/**
 * `collect` of `Option`s: `None` at the first `None`, or every value collected by `into`,
 * as `iterCollectResult`. An item's `None` is `itemNone`, and the result's `none`, each
 * `rt.None` where `undefined` is a value.
 */
export function iterCollectOption<U, C>(it: Iter<U | undefined>, into: (values: Iter<U>) => C, drop?: Drop<C>): C | undefined;
export function iterCollectOption<U, C>(it: Iter<U | None>, into: (values: Iter<U>) => C, drop: Drop<C> | undefined, itemNone: None): C | undefined;
export function iterCollectOption<U, C>(it: Iter<U | undefined | None>, into: (values: Iter<U>) => C, drop: Drop<C> | undefined, itemNone: None | undefined, none: None): C | None;
export function iterCollectOption<U, C>(it: Iter<U | undefined | None>, into: (values: Iter<U>) => C, drop?: Drop<C>, itemNone?: None, none?: None): C | undefined | None {
	const shunt = new OptionShunt(it, itemNone);
	const values = into(shunt);
	if (!shunt.none) return values;
	drop?.(values);
	return none;
}

// What std's iterators override to count or take the last: `count` and `last`, each of
// which drops items in an order of its own kind's. These follow it, dispatching on the
// kind, rather than as methods every iterator would carry, and the translation calls them
// only where an item or the iterator drops.

/** `Iterator::count` of an iterator that drops what it holds: each item by `drop`, as its own `count` does, and then it. */
export function ownedCount<T>(it: Iter<T>, drop?: Drop<T>): number {
	return count(it, drop);
}

/** `Iterator::last` of an iterator that drops what it holds: each item but the last by `drop`, as its own `last` does, and then it. */
export function ownedLast<T>(it: Iter<T>, drop?: Drop<T>): T | undefined;
export function ownedLast<T>(it: Iter<T>, drop: Drop<T> | undefined, none: None): T | None;
export function ownedLast<T>(it: Iter<T>, drop?: Drop<T>, none?: None): T | undefined | None {
	return iterToOption(last(it, drop), none);
}

/** `count`, dropping each item by `drop`, and then `it`, as std's of its kind does. */
function count<T>(it: Iter<T>, drop: Drop<T> | undefined): number {
	if (it instanceof EnumerateIter) return count(it.inner, items(drop));
	if (it instanceof ChainIter) {
		// std's: each iterator's own `count`, the first's first.
		const { a, b } = it;
		it.a = it.b = undefined;
		const n = a === undefined ? 0 : count(a, drop);
		return n + (b === undefined ? 0 : count(b, drop));
	}
	if (it instanceof SkipIter) {
		// std's: past what it skips by `nth`, and then its iterator's own `count`.
		return skipped(it, drop) ? 0 : count(it.inner, drop);
	}
	if (it instanceof TakeIter) {
		// std's: its iterator advanced through the limit, dropping what it passes.
		const n = it.n;
		it.n = 0;
		return consumed(it.inner, () => (n === 0 ? 0 : n - it.inner.advanceBy(n, drop)));
	}
	if (it instanceof FuseIter) {
		const inner = it.inner;
		it.inner = undefined;
		return inner === undefined ? 0 : count(inner, drop);
	}
	if (it instanceof FlattenIter) {
		// std's: each iterator's own `count` in turn.
		return flattenFold(it as FlattenIter<unknown, T>, false, 0, (n, inner) => n + count(inner, drop));
	}
	if (it instanceof Peekable) {
		// std's: the item it peeked drops after its iterator is counted.
		if (!it.peeked) return count(it.inner, drop);
		const peeked = it.take();
		if (peeked === DONE) {
			dispose(it.inner);
			return 0;
		}
		const n = 1 + count(it.inner, drop);
		drop?.(peeked);
		return n;
	}
	if (it instanceof HashIter && !it.drain) {
		// std's `into_iter`'s: its length, and then each entry drops with it.
		const n = it.left;
		dispose(it);
		return n;
	}
	return it.fold(0, (n, item) => {
		drop?.(item);
		return n + 1;
	});
}

/**
 * `last`, dropping each item but the last by `drop`, and then `it`, as std's of its kind
 * does. By `fold`, each item drops as the next replaces it.
 */
function last<T>(it: Iter<T>, drop: Drop<T> | undefined): T | Done {
	if ((it instanceof SliceIter && drop !== undefined) || (it instanceof BTreeIter && it.back)) {
		// std's, of a `Vec`'s or an array's, and of a B-tree's `into_keys` and
		// `into_values`: the last item, and then the rest drop with it.
		const item = it.nextBack();
		dispose(it);
		return item;
	}
	if (it instanceof ChainIter) {
		// std's: each iterator's last, the first's held while the second is read, and
		// dropped once the second has one.
		const { a, b } = it;
		it.a = it.b = undefined;
		const aLast = a === undefined ? DONE : last(a, drop);
		if (b === undefined) return aLast;
		return or(last(b, drop), aLast, drop);
	}
	if (it instanceof SkipIter) {
		// std's: past what it skips by `nth`, and then its iterator's own `last`.
		return skipped(it, drop) ? DONE : last(it.inner, drop);
	}
	if (it instanceof FuseIter) {
		const inner = it.inner;
		it.inner = undefined;
		return inner === undefined ? DONE : last(inner, drop);
	}
	if (it instanceof FlattenIter) {
		// std's: the last of each iterator in turn, the last so far dropped once a later
		// one has one.
		return flattenFold<unknown, T, T | Done>(it as FlattenIter<unknown, T>, false, DONE, (prev, inner) => or(last(inner, drop), prev, drop));
	}
	if (it instanceof Peekable) {
		// std's: the item it peeked, unless its iterator has a last, which drops it after.
		let peeked: T | Done = DONE;
		if (it.peeked) {
			peeked = it.take();
			if (peeked === DONE) {
				dispose(it.inner);
				return DONE;
			}
		}
		return or(last(it.inner, drop), peeked, drop);
	}
	return it.fold<T | Done>(DONE, (prev, item) => {
		if (prev !== DONE) drop?.(prev);
		return item;
	});
}

/** `item.or(prev)`: `item`, unless that is `DONE`, and then `prev`, which drops by `drop` if not. */
function or<T>(item: T | Done, prev: T | Done, drop: Drop<T> | undefined): T | Done {
	if (item === DONE) return prev;
	if (prev !== DONE) drop?.(prev);
	return item;
}

/** `f()`, a consumer of `it` by value, and then `it` dropped. */
function consumed<T, R>(it: Iter<T>, f: () => R): R {
	const r = f();
	dispose(it);
	return r;
}

/** What drops an `enumerate`'s item, of one its iterator yields: `drop` is of `[number, T]`. */
function items(drop: Drop<never> | undefined): Drop<unknown> | undefined {
	return drop === undefined ? undefined : (item) => drop([0, item] as never);
}

/**
 * Read past what `it` still skips, as `nth` reads, and whether its iterator ended first,
 * which then drops.
 */
function skipped<T>(it: SkipIter<T>, drop: Drop<T> | undefined): boolean {
	if (it.n === 0) return false;
	const n = it.n;
	it.n = 0;
	const item = it.inner.nth(n - 1, drop);
	if (item !== DONE) {
		drop?.(item);
		return false;
	}
	dispose(it.inner);
	return true;
}

/**
 * std's `iter_fold` of a flatten, or `iter_rfold` if `back`: `g` of each iterator it holds
 * in turn, the front, then each of its source's by the source's own fold, and the back.
 * `release` drops what `g` owns as it ends, before what it still holds.
 */
function flattenFold<T, V, A>(it: FlattenIter<T, V>, back: boolean, acc: A, g: (acc: A, inner: Iter<V>) => A, release?: () => void): A {
	const { outer } = it;
	const [first, rest] = back ? [it.back, it.front] : [it.front, it.back];
	it.outer = it.front = it.back = undefined;
	if (first !== undefined) acc = g(acc, first);
	if (outer !== undefined) {
		const each = (acc: A, item: T) => g(acc, it.into(item));
		acc = back ? outer.rfold(acc, each) : outer.fold(acc, each);
	}
	if (rest !== undefined) acc = g(acc, rest);
	release?.();
	return acc;
}

// Iterators. One has `[Symbol.dispose]` if it owns items or wraps an iterator: what holds
// it drops it by `rt.drop`, not knowing its kind.

class SliceIter<T> extends Iter<T> implements Disposable {
	readonly s: ArrayLike<T>;
	front = 0;
	back: number;
	/** What drops each item it still holds, if it owns them. */
	private readonly drop: Drop<T> | undefined;

	constructor(s: ArrayLike<T>, drop?: Drop<T>) {
		super();
		this.s = s;
		this.back = s.length;
		this.drop = drop;
	}

	next(): T | Done {
		return this.front < this.back ? this.s[this.front++] : DONE;
	}

	override nextBack(): T | Done {
		return this.front < this.back ? this.s[--this.back] : DONE;
	}

	override len(): number {
		return this.back - this.front;
	}

	override sizeHint(): SizeHint {
		return exactHint(this.len());
	}

	override remaining(): Iterable<T> {
		return Array.prototype.slice.call(this.s, this.front, this.back);
	}

	/** std's: what it has left, each cloned in order by `h.v` into storage of its own, or else shared. */
	override clone(h?: Clones): Iter<T> {
		if (h?.v !== undefined) {
			const items = Array.prototype.slice.call(this.s, this.front, this.back) as T[];
			for (let i = 0; i < items.length; i++) items[i] = cloned(items[i], h.v);
			return new SliceIter(items, this.drop);
		}
		const it = new SliceIter(this.s, this.drop);
		it.front = this.front;
		it.back = this.back;
		return it;
	}

	[Symbol.dispose](): void {
		if (this.drop === undefined) return;
		const from = this.front;
		this.front = this.back;
		dropRange(this.s, this.drop, from, this.back);
	}

	/** std's, of a `Vec`'s: past them all first, then each dropped, as a slice drops. */
	override advanceBy(n: number, drop?: Drop<T>): number {
		const step = Math.min(n, this.back - this.front);
		const from = this.front;
		this.front += step;
		if (drop !== undefined) dropRange(this.s, drop, from, this.front);
		return n - step;
	}

	/** std's: those it skips drop front to back, as a slice drops. */
	override advanceBackBy(n: number, drop?: Drop<T>): number {
		const step = Math.min(n, this.back - this.front);
		this.back -= step;
		if (drop !== undefined) dropRange(this.s, drop, this.back, this.back + step);
		return n - step;
	}
}

/** The places of a slice's elements, from both ends. */
class SliceMutIter<T> extends Iter<Place<T>> {
	private readonly s: { [i: number]: T; length: number };
	private front = 0;
	private back: number;

	constructor(s: { [i: number]: T; length: number }) {
		super();
		this.s = s;
		this.back = s.length;
	}

	next(): Place<T> | Done {
		return this.front < this.back ? new AtIndex(this.s, this.front++) : DONE;
	}

	override nextBack(): Place<T> | Done {
		return this.front < this.back ? new AtIndex(this.s, --this.back) : DONE;
	}

	override len(): number {
		return this.back - this.front;
	}

	override sizeHint(): SizeHint {
		return exactHint(this.len());
	}

	override remaining(): Iterable<T> {
		return Array.prototype.slice.call(this.s, this.front, this.back);
	}

	override advanceBy(n: number): number {
		const step = Math.min(n, this.back - this.front);
		this.front += step;
		return n - step;
	}

	override advanceBackBy(n: number): number {
		const step = Math.min(n, this.back - this.front);
		this.back -= step;
		return n - step;
	}
}

/**
 * `slice::Windows` and `Chunks` of `s`, of `length` elements: the part from each `step`th
 * element on, `size` long or up to the end, as `part` slices it, while one starts before
 * the end and, for windows, which step by 1, is `size` long.
 */
class PartsIter<S> extends Iter<S> {
	private readonly s: S;
	private readonly length: number;
	private readonly size: number;
	private readonly step: number;
	private readonly count: number;
	/** Whether the first part is at the back of the slice, as `rchunks`' is. */
	private readonly rev: boolean;
	private readonly part: (s: S, start: number, end: number) => S;
	private front = 0;
	/** How many parts there are, less those taken from the back. */
	private back: number;

	constructor(s: S, length: number, size: number, step: number, count: number, rev: boolean, part: (s: S, start: number, end: number) => S) {
		super();
		this.s = s;
		this.length = length;
		this.size = size;
		this.step = step;
		this.count = count;
		this.rev = rev;
		this.part = part;
		this.back = count;
	}

	private at(i: number): S {
		if (this.rev) {
			const end = this.length - i * this.step;
			return this.part(this.s, Math.max(end - this.size, 0), end);
		}
		const start = i * this.step;
		return this.part(this.s, start, Math.min(start + this.size, this.length));
	}

	override clone(): Iter<S> {
		const it = new PartsIter(this.s, this.length, this.size, this.step, this.count, this.rev, this.part);
		it.front = this.front;
		it.back = this.back;
		return it;
	}

	/** What the parts leave over, at the end the parts don't start from. */
	remainder(): S {
		const held = this.count * this.size;
		return this.rev ? this.part(this.s, 0, this.length - held) : this.part(this.s, held, this.length);
	}

	next(): S | Done {
		return this.front < this.back ? this.at(this.front++) : DONE;
	}

	override nextBack(): S | Done {
		return this.front < this.back ? this.at(--this.back) : DONE;
	}

	override len(): number {
		return this.back - this.front;
	}

	override sizeHint(): SizeHint {
		return exactHint(this.len());
	}

	override advanceBy(n: number): number {
		const step = Math.min(n, this.back - this.front);
		this.front += step;
		return n - step;
	}

	override advanceBackBy(n: number): number {
		const step = Math.min(n, this.back - this.front);
		this.back -= step;
		return n - step;
	}
}

/**
 * `slice::Split` and its forms of `s`, whose elements `items` holds from `offset`: the
 * parts between the elements `f` is true of, as `part` slices them, read as std's are.
 * Each read searches what is left from its end, calling `f` on each element in turn up to
 * the first it is true of, and keeps what it passed by in the part. A part found from the
 * back of a `split_inclusive` ends with an element the search leaves out, so a read from
 * the front may call `f` on it again, as std's does.
 */
class SplitIter<S, T> extends Iter<S> implements Disposable {
	readonly #s: S;
	readonly #items: ArrayLike<T>;
	readonly #offset: number;
	readonly #part: (s: S, start: number, end: number) => S;
	readonly #f: (item: T) => boolean;
	/** Whether `next` reads from the back, as `rsplit`'s does. */
	readonly #rev: boolean;
	/** Whether each part ends with its separator, as `split_inclusive`'s does. */
	readonly #inclusive: boolean;
	/** How many more parts a `splitn` yields, the last of them all that is left. */
	#count: number | undefined;
	/** What is left to split: `s[start..end]`. */
	#start = 0;
	#end: number;
	#finished: boolean;
	#dropped = false;

	constructor(s: S, items: ArrayLike<T>, offset: number, part: (s: S, start: number, end: number) => S, f: (item: T) => boolean, rev: boolean, inclusive: boolean, count?: number) {
		super();
		this.#s = s;
		this.#items = items;
		this.#offset = offset;
		this.#part = part;
		this.#f = f;
		this.#rev = rev;
		this.#inclusive = inclusive;
		this.#count = count;
		this.#end = (s as { length: number }).length;
		// An empty slice has no part to include a separator in.
		this.#finished = inclusive && this.#end === 0;
	}

	override clone(h?: Clones): Iter<S> {
		const it = new SplitIter(this.#s, this.#items, this.#offset, this.#part, cloned(this.#f, h?.f), this.#rev, this.#inclusive, this.#count);
		it.#start = this.#start;
		it.#end = this.#end;
		it.#finished = this.#finished;
		return it;
	}

	next(): S | Done {
		if (this.#count !== undefined) {
			if (this.#count === 0) return DONE;
			if (this.#count-- === 1) return this.#finish();
		}
		return this.#rev ? this.#back() : this.#front();
	}

	override nextBack(): S | Done {
		return this.#rev ? this.#front() : this.#back();
	}

	override sizeHint(): SizeHint {
		const left = this.#end - this.#start;
		// No separator yields one part, and all of them one more than there are elements.
		const upper = this.#inclusive ? Math.max(1, left) : left + 1;
		const [lower, most]: SizeHint = this.#finished ? [0, 0] : [1, upper];
		const n = this.#count;
		return n === undefined ? [lower, most] : [Math.min(n, lower), Math.min(n, most as number)];
	}

	/** What is left to split, or nothing once the last part is taken. */
	asSlice(): S {
		return this.#part(this.#s, this.#start, this.#finished ? this.#start : this.#end);
	}

	/** The rest, as the last part. */
	#finish(): S | Done {
		if (this.#finished) return DONE;
		this.#finished = true;
		return this.#part(this.#s, this.#start, this.#end);
	}

	#front(): S | Done {
		if (this.#finished) return DONE;
		const start = this.#start;
		let i = start;
		while (i < this.#end && !this.#f(this.#items[this.#offset + i])) i++;
		if (this.#inclusive) {
			const end = Math.min(i + 1, this.#end);
			if (end === this.#end) this.#finished = true;
			this.#start = end;
			return this.#part(this.#s, start, end);
		}
		if (i === this.#end) return this.#finish();
		this.#start = i + 1;
		return this.#part(this.#s, start, i);
	}

	#back(): S | Done {
		if (this.#finished) return DONE;
		const end = this.#end;
		// The last element ends an inclusive part whether it is a separator or not.
		let i = this.#inclusive ? Math.max(end - 1, this.#start) : end;
		while (i > this.#start && !this.#f(this.#items[this.#offset + i - 1])) i--;
		if (this.#inclusive) {
			if (i === this.#start) this.#finished = true;
			this.#end = i;
			return this.#part(this.#s, i, end);
		}
		if (i === this.#start) return this.#finish();
		this.#end = i - 1;
		return this.#part(this.#s, i, end);
	}

	[Symbol.dispose](): void {
		if (this.#dropped) return;
		this.#dropped = true;
		dispose(this.#f);
	}
}

/** `option::IntoIter`, `Once`, and `Empty`: at most one item. */
class OptionIter<T> extends Iter<T> implements Disposable {
	private item: T | undefined;
	private some: boolean;
	private readonly drop: Drop<T> | undefined;

	constructor(item: T | undefined, some: boolean, drop?: Drop<T>) {
		super();
		this.item = item;
		this.some = some;
		this.drop = drop;
	}

	next(): T | Done {
		if (!this.some) return DONE;
		this.some = false;
		const item = this.item as T;
		this.item = undefined;
		return item;
	}

	override nextBack(): T | Done {
		return this.next();
	}

	override len(): number {
		return this.some ? 1 : 0;
	}

	override clone(h?: Clones): Iter<T> {
		return new OptionIter(this.some ? cloned(this.item as T, h?.v) : undefined, this.some, this.drop);
	}

	override sizeHint(): SizeHint {
		return exactHint(this.len());
	}

	[Symbol.dispose](): void {
		if (this.drop === undefined) return;
		const item = this.next();
		if (item !== DONE) this.drop(item);
	}
}

/** An iterator whose `next` is a function of its source, which `drop` drops. */
class FromNext<S, T> extends Iter<T> implements Disposable {
	private readonly source: S;
	private readonly step: (source: S) => T | Done;
	private drop: Drop<S> | undefined;

	constructor(source: S, step: (source: S) => T | Done, drop?: Drop<S>) {
		super();
		this.source = source;
		this.step = step;
		this.drop = drop;
	}

	next(): T | Done {
		return this.step(this.source);
	}

	/** Its source cloned by `h.v`, as the crate's own `Clone` of it. */
	override clone(h?: Clones): Iter<T> {
		return new FromNext(cloned(this.source, h?.v), this.step, this.drop);
	}

	[Symbol.dispose](): void {
		const drop = this.drop;
		this.drop = undefined;
		drop?.(this.source);
	}
}

/** `Successors`, holding the item it yields next, which drops once, as `Calling` does. */
class SuccessorsIter<T> extends Iter<T> implements Disposable {
	private item: T | undefined | None;
	private readonly succ: (item: T) => T | undefined | None;
	private readonly drop: Drop<T> | undefined;
	/** The `Option`'s `None`. */
	private readonly none: None | undefined;
	private dropped = false;

	constructor(first: T | undefined | None, succ: (item: T) => T | undefined | None, drop?: Drop<T>, none?: None) {
		super();
		this.item = first;
		this.succ = succ;
		this.drop = drop;
		this.none = none;
	}

	next(): T | Done {
		const item = this.item;
		if (item === this.none) return DONE;
		this.item = this.succ(item as T);
		return item as T;
	}

	override sizeHint(): SizeHint {
		return this.item === this.none ? [0, 0] : [1, undefined];
	}

	/** std's: the item it holds next by `h.v`, then its closure by `h.f`. */
	override clone(h?: Clones): Iter<T> {
		const item = this.item === this.none ? this.item : cloned(this.item as T, h?.v);
		return new SuccessorsIter(item, cloned(this.succ, h?.f), this.drop, this.none);
	}

	[Symbol.dispose](): void {
		if (this.dropped) return;
		this.dropped = true;
		const item = this.item;
		this.item = this.none;
		if (item !== this.none) this.drop?.(item as T);
		dispose(this.succ);
	}
}

/**
 * An iterator borrowed by `&mut`, which forwards only what std's `impl Iterator for &mut
 * I` does, and which dropping drops nothing.
 */
class Borrowed<T> extends Iter<T> {
	readonly inner: Iter<T>;

	constructor(inner: Iter<T>) {
		super();
		this.inner = inner;
	}

	override sizeHint(): SizeHint {
		return this.inner.sizeHint();
	}

	next(): T | Done {
		return this.inner.next();
	}

	override folded(): Iter<T> {
		return this.inner.folded();
	}

	override nextBack(): T | Done {
		return this.inner.nextBack();
	}

	override len(): number {
		return this.inner.len();
	}

	override advanceBy(n: number, drop?: Drop<T>): number {
		return this.inner.advanceBy(n, drop);
	}

	override advanceBackBy(n: number, drop?: Drop<T>): number {
		return this.inner.advanceBackBy(n, drop);
	}

	override nth(n: number, drop?: Drop<T>): T | Done {
		return this.inner.nth(n, drop);
	}

	override nthBack(n: number, drop?: Drop<T>): T | Done {
		return this.inner.nthBack(n, drop);
	}
}

class MapIter<T, U> extends Calling<T, U, (item: T) => U> {

	constructor(inner: Iter<T>, f: (item: T) => U) {
		super(inner, f);
	}

	override sizeHint(): SizeHint {
		return this.inner.sizeHint();
	}

	next(): U | Done {
		const item = this.inner.next();
		return item === DONE ? DONE : this.f(item);
	}

	override nextBack(): U | Done {
		const item = this.inner.nextBack();
		return item === DONE ? DONE : this.f(item);
	}

	override len(): number {
		return this.inner.len();
	}

	override folded(fold = false): Iter<U> {
		const inner = this.inner.folded(fold);
		if (inner === this.inner) return this;
		return new MapIter(inner, this.f);
	}

	/** Its iterator by `h.i`, then `f` by `h.f`, as each kind that maps does. */
	override clone(h?: Clones): Iter<U> {
		const kind = this.constructor as new (inner: Iter<T>, f: (item: T) => U) => MapIter<T, U>;
		return new kind(this.inner.clone(h?.i), cloned(this.f, h?.f));
	}

	/** std's: `inner`'s, by a closure that drops `release`'s and then `f`. */
	override fold<A>(acc: A, g: (acc: A, item: U) => A, release?: () => void): A {
		return this.inner.fold(acc, mapFold(this.f, g), then(release, this.take()));
	}

	override rfold<A>(acc: A, g: (acc: A, item: U) => A, release?: () => void): A {
		return this.inner.rfold(acc, mapFold(this.f, g), then(release, this.take()));
	}
}

/** What `iterLend` makes, which has left the items `inner` has, as it holds them. */
class LendIter<T, U> extends MapIter<T, U> {
	override remaining(): Iterable<unknown> {
		return this.inner.remaining();
	}
}

/** std's `map_fold`: `g` of what `f` makes of each item. */
function mapFold<T, U, A>(f: (item: T) => U, g: (acc: A, item: U) => A): (acc: A, item: T) => A {
	return (acc, item) => g(acc, f(item));
}

/** `Copied` of items JS copies by a function: a `map` whose `nth` and `advance_by` are its iterator's. */
class CopiedIter<R, T> extends MapIter<R, T> {
	override advanceBy(n: number): number {
		return this.inner.advanceBy(n);
	}

	override advanceBackBy(n: number): number {
		return this.inner.advanceBackBy(n);
	}

	override nth(n: number): T | Done {
		const item = this.inner.nth(n);
		return item === DONE ? DONE : this.f(item);
	}
}

class FilterIter<T> extends Calling<T, T, (item: T) => boolean> {
	private readonly drop: Drop<T> | undefined;

	constructor(inner: Iter<T>, f: (item: T) => boolean, drop?: Drop<T>) {
		super(inner, f);
		this.drop = drop;
	}

	next(): T | Done {
		return findItem(this.inner, this.f, this.drop);
	}

	override sizeHint(): SizeHint {
		return fewer(this.inner);
	}

	override nextBack(): T | Done {
		return rfindItem(this.inner, this.f, this.drop);
	}

	/** std's `fold` folds its iterator, where `next` reads it as `try_fold` does. */
	override folded(fold = false): Iter<T> {
		const inner = fold ? this.inner.folded(true) : this.inner;
		if (inner === this.inner) return this;
		return new FilterIter(inner, this.f, this.drop);
	}

	override clone(h?: Clones): Iter<T> {
		return new FilterIter(this.inner.clone(h?.i), cloned(this.f, h?.f), this.drop);
	}

	/** std's: `inner`'s, by a closure that drops `f` and then `release`'s. */
	override fold<A>(acc: A, g: (acc: A, item: T) => A, release?: () => void): A {
		return this.inner.fold(acc, this.filterFold(g), then(this.take(), release));
	}

	override rfold<A>(acc: A, g: (acc: A, item: T) => A, release?: () => void): A {
		return this.inner.rfold(acc, this.filterFold(g), then(this.take(), release));
	}

	/** std's `filter_fold`: `g` of each item `f` keeps. */
	private filterFold<A>(g: (acc: A, item: T) => A): (acc: A, item: T) => A {
		const { f, drop } = this;
		return (acc, item) => {
			if (f(item)) return g(acc, item);
			drop?.(item);
			return acc;
		};
	}
}

class FilterMapIter<T, U> extends Calling<T, U, (item: T) => U | undefined | None> {
	/** The `Option`'s `None` that `f` returns. */
	private readonly none: None | undefined;

	constructor(inner: Iter<T>, f: (item: T) => U | undefined | None, none?: None) {
		super(inner, f);
		this.none = none;
	}

	/** std's, by `find_map`, which folds its iterator. */
	next(): U | Done {
		const inner = this.inner.folded();
		for (let item = inner.next(); item !== DONE; item = inner.next()) {
			const mapped = this.f(item);
			if (mapped !== this.none) return mapped as U;
		}
		return DONE;
	}

	override sizeHint(): SizeHint {
		return fewer(this.inner);
	}

	override nextBack(): U | Done {
		const inner = this.inner.folded();
		for (let item = inner.nextBack(); item !== DONE; item = inner.nextBack()) {
			const mapped = this.f(item);
			if (mapped !== this.none) return mapped as U;
		}
		return DONE;
	}

	/** std's `fold` folds its iterator, where `next` reads it as `try_fold` does. */
	override folded(fold = false): Iter<U> {
		const inner = fold ? this.inner.folded(true) : this.inner;
		if (inner === this.inner) return this;
		return new FilterMapIter(inner, this.f, this.none);
	}

	override clone(h?: Clones): Iter<U> {
		return new FilterMapIter(this.inner.clone(h?.i), cloned(this.f, h?.f), this.none);
	}

	/** std's: `inner`'s, by a closure that drops `f` and then `release`'s. */
	override fold<A>(acc: A, g: (acc: A, item: U) => A, release?: () => void): A {
		return this.inner.fold(acc, this.filterMapFold(g), then(this.take(), release));
	}

	override rfold<A>(acc: A, g: (acc: A, item: U) => A, release?: () => void): A {
		return this.inner.rfold(acc, this.filterMapFold(g), then(this.take(), release));
	}

	/** std's `filter_map_fold`: `g` of each value `f` makes. */
	private filterMapFold<A>(g: (acc: A, item: U) => A): (acc: A, item: T) => A {
		const [f, none] = [this.f, this.none];
		return mapFold(f, (acc, mapped: U | undefined | None) => (mapped === none ? acc : g(acc, mapped as U)));
	}
}

class EnumerateIter<T> extends Adaptor<T, [number, T]> {
	index = 0;

	override clone(h?: Clones): Iter<[number, T]> {
		const it = new EnumerateIter(this.inner.clone(h?.i));
		it.index = this.index;
		return it;
	}

	override sizeHint(): SizeHint {
		return this.inner.sizeHint();
	}

	next(): [number, T] | Done {
		const item = this.inner.next();
		return item === DONE ? DONE : [this.index++, item];
	}

	override nextBack(): [number, T] | Done {
		const item = this.inner.nextBack();
		return item === DONE ? DONE : [this.index + this.inner.len(), item];
	}

	override len(): number {
		return this.inner.len();
	}

	/** std's, which counts what its iterator skipped. */
	override advanceBy(n: number, drop?: Drop<[number, T]>): number {
		const left = this.inner.advanceBy(n, items(drop));
		this.index += n - left;
		return left;
	}

	override advanceBackBy(n: number, drop?: Drop<[number, T]>): number {
		return this.inner.advanceBackBy(n, items(drop));
	}

	/** std's, which counts only when it finds an item. */
	override nth(n: number, drop?: Drop<[number, T]>): [number, T] | Done {
		const item = this.inner.nth(n, items(drop));
		if (item === DONE) return DONE;
		const i = this.index + n;
		this.index = i + 1;
		return [i, item];
	}

	override nthBack(n: number, drop?: Drop<[number, T]>): [number, T] | Done {
		const item = this.inner.nthBack(n, items(drop));
		return item === DONE ? DONE : [this.index + this.inner.len(), item];
	}

	/** std's: its iterator folded, counting each item as it goes, and from the back from its length. */
	override folded(fold = false): Iter<[number, T]> {
		const inner = this.inner.folded(fold);
		if (inner === this.inner) return this;
		const front = (): [number, T] | Done => {
			const item = inner.next();
			return item === DONE ? DONE : [this.index++, item];
		};
		const back = (): [number, T] | Done => {
			const item = inner.nextBack();
			return item === DONE ? DONE : [this.index + this.inner.len(), item];
		};
		return new Folded(front, back);
	}

	override fold<A>(acc: A, g: (acc: A, item: [number, T]) => A, release?: () => void): A {
		let i = this.index;
		return this.inner.fold(acc, (acc, item) => g(acc, [i++, item]), release);
	}

	override rfold<A>(acc: A, g: (acc: A, item: [number, T]) => A, release?: () => void): A {
		let i = this.index + this.inner.len();
		return this.inner.rfold(acc, (acc, item) => g(acc, [--i, item]), release);
	}
}

/**
 * `Chain`, which drops its first iterator once it ends, but not its second, as std's
 * stops reading it.
 */
class ChainIter<T> extends Iter<T> implements Disposable {
	a: Iter<T> | undefined;
	b: Iter<T> | undefined;

	constructor(a: Iter<T> | undefined, b: Iter<T> | undefined) {
		super();
		this.a = a;
		this.b = b;
	}

	/** std's: each half it still holds, the first by `h.i` and the second by `h.o`. */
	override clone(h?: Clones): Iter<T> {
		return new ChainIter(this.a?.clone(h?.i), this.b?.clone(h?.o));
	}

	/** Drop `a`, which ended. */
	endA(): void {
		const a = this.a;
		this.a = undefined;
		dispose(a);
	}

	/** Drop `b`, which ended from the back. */
	endB(): void {
		const b = this.b;
		this.b = undefined;
		dispose(b);
	}

	override sizeHint(): SizeHint {
		const { a, b } = this;
		if (a === undefined || b === undefined) return a?.sizeHint() ?? b?.sizeHint() ?? [0, 0];
		const [aLower, aUpper] = a.sizeHint();
		const [bLower, bUpper] = b.sizeHint();
		return [satAdd(aLower, bLower), addUpper(aUpper, bUpper)];
	}

	next(): T | Done {
		if (this.a !== undefined) {
			const item = this.a.next();
			if (item !== DONE) return item;
			this.endA();
		}
		return this.b === undefined ? DONE : this.b.next();
	}

	override nextBack(): T | Done {
		if (this.b !== undefined) {
			const item = this.b.nextBack();
			if (item !== DONE) return item;
			this.endB();
		}
		return this.a === undefined ? DONE : this.a.nextBack();
	}

	[Symbol.dispose](): void {
		this.endA();
		this.endB();
	}

	/** std's: each folded in turn, the first dropped once it ends. */
	override folded(fold = false): Iter<T> {
		const a = this.a?.folded(fold);
		const b = this.b?.folded(fold);
		if (a === this.a && b === this.b) return this;
		const front = (): T | Done => {
			if (this.a !== undefined) {
				const item = a!.next();
				if (item !== DONE) return item;
				this.endA();
			}
			return this.b === undefined ? DONE : b!.next();
		};
		const back = (): T | Done => {
			if (this.b !== undefined) {
				const item = b!.nextBack();
				if (item !== DONE) return item;
				this.endB();
			}
			return this.a === undefined ? DONE : a!.nextBack();
		};
		return new Folded(front, back);
	}

	override advanceBy(n: number, drop?: Drop<T>): number {
		if (this.a !== undefined) {
			n = this.a.advanceBy(n, drop);
			if (n === 0) return 0;
			this.endA();
		}
		return this.b === undefined ? n : this.b.advanceBy(n, drop);
	}

	override advanceBackBy(n: number, drop?: Drop<T>): number {
		if (this.b !== undefined) {
			n = this.b.advanceBackBy(n, drop);
			if (n === 0) return 0;
			this.endB();
		}
		return this.a === undefined ? n : this.a.advanceBackBy(n, drop);
	}

	override nth(n: number, drop?: Drop<T>): T | Done {
		if (this.a !== undefined) {
			n = this.a.advanceBy(n, drop);
			if (n === 0) {
				const item = this.a.next();
				if (item !== DONE) return item;
			}
			this.endA();
		}
		return this.b === undefined ? DONE : this.b.nth(n, drop);
	}

	override nthBack(n: number, drop?: Drop<T>): T | Done {
		if (this.b !== undefined) {
			n = this.b.advanceBackBy(n, drop);
			if (n === 0) {
				const item = this.b.nextBack();
				if (item !== DONE) return item;
			}
			this.endB();
		}
		return this.a === undefined ? DONE : this.a.nthBack(n, drop);
	}

	/** std's: the first's fold and then the second's, which drops `release`'s as it ends. */
	override fold<A>(acc: A, f: (acc: A, item: T) => A, release?: () => void): A {
		const { a, b } = this;
		this.a = this.b = undefined;
		if (a !== undefined) acc = a.fold(acc, f);
		if (b !== undefined) return b.fold(acc, f, release);
		release?.();
		return acc;
	}

	override rfold<A>(acc: A, f: (acc: A, item: T) => A, release?: () => void): A {
		const { a, b } = this;
		this.a = this.b = undefined;
		if (b !== undefined) acc = b.rfold(acc, f);
		if (a !== undefined) return a.rfold(acc, f, release);
		release?.();
		return acc;
	}
}

class ZipIter<A, B> extends Iter<[A, B]> implements Disposable {
	readonly a: Iter<A>;
	readonly b: Iter<B>;
	/** Whether both are `TrustedLen`, so `fold` stops at its size hint. */
	private readonly bounded: boolean;
	protected readonly dropA: Drop<A> | undefined;
	protected readonly dropB: Drop<B> | undefined;

	constructor(a: Iter<A>, b: Iter<B>, bounded: boolean, dropA?: Drop<A>, dropB?: Drop<B>) {
		super();
		this.a = a;
		this.b = b;
		this.bounded = bounded;
		this.dropA = dropA;
		this.dropB = dropB;
	}

	override clone(h?: Clones): Iter<[A, B]> {
		return new ZipIter(this.a.clone(h?.i), this.b.clone(h?.o), this.bounded, this.dropA, this.dropB);
	}

	/** std's `SpecFold` of two `TrustedLen` iterators, which `try_fold` doesn't share. */
	override folded(fold = false): Iter<[A, B]> {
		return fold && this.bounded ? new ZipFold(this) : this;
	}

	/** An item of `a` it read, dropped if `b` ends. */
	next(): [A, B] | Done {
		const a = this.a.next();
		if (a === DONE) return DONE;
		const b = this.b.next();
		if (b !== DONE) return [a, b];
		this.dropA?.(a);
		return DONE;
	}

	/** std's: the longer side drops its extra items from the back first. */
	override nextBack(): [A, B] | Done {
		const aLen = this.a.len();
		const bLen = this.b.len();
		for (let i = bLen; i < aLen; i++) {
			const item = this.a.nextBack();
			if (item !== DONE) this.dropA?.(item);
		}
		for (let i = aLen; i < bLen; i++) {
			const item = this.b.nextBack();
			if (item !== DONE) this.dropB?.(item);
		}
		const a = this.a.nextBack();
		const b = this.b.nextBack();
		if (a === DONE && b === DONE) return DONE;
		if (a === DONE || b === DONE) panic("internal error: entered unreachable code");
		return [a, b];
	}

	override len(): number {
		return Math.min(this.a.len(), this.b.len());
	}

	/** std's: the shorter side's, where either knows its upper bound. */
	override sizeHint(): SizeHint {
		const [aLower, aUpper] = this.a.sizeHint();
		const [bLower, bUpper] = this.b.sizeHint();
		const upper = aUpper === undefined || bUpper === undefined ? (aUpper ?? bUpper) : Math.min(aUpper, bUpper);
		return [Math.min(aLower, bLower), upper];
	}

	[Symbol.dispose](): void {
		dispose(this.a);
		dispose(this.b);
	}
}

/**
 * A zip of two `TrustedLen` iterators as std's `fold` reads it: as many pairs as its size
 * hint's upper bound when it starts, which `TrustedLen` promises each holds, and from the
 * back by `next_back`, as `rfold` reads it.
 */
class ZipFold<A, B> extends Iter<[A, B]> {
	private readonly zip: ZipIter<A, B>;
	/** The pairs left, once it starts, or -1. */
	private left = -1;

	constructor(zip: ZipIter<A, B>) {
		super();
		this.zip = zip;
	}

	next(): [A, B] | Done {
		if (this.left < 0) this.left = this.zip.sizeHint()[1] ?? Infinity;
		if (this.left === 0) return DONE;
		this.left--;
		const a = this.zip.a.next();
		const b = this.zip.b.next();
		if (a === DONE || b === DONE) panic("internal error: entered unreachable code");
		return [a, b];
	}

	override nextBack(): [A, B] | Done {
		return this.zip.nextBack();
	}
}

/**
 * std's `Zip` of two `TrustedRandomAccess` iterators, which counts the pairs it reads, and
 * which `fold` reads as `next` does. It reads an item it never yields only from a side
 * that may have side effects, and skips the rest of a side that has none in one step.
 */
class TrustedZipIter<A, B> extends ZipIter<A, B> {
	private index = 0;
	private end: number;
	/** Whether each side may have side effects. */
	private readonly effects: readonly [boolean, boolean];

	constructor(a: Iter<A>, b: Iter<B>, effects: readonly [boolean, boolean], dropA?: Drop<A>, dropB?: Drop<B>) {
		super(a, b, false, dropA, dropB);
		this.end = Math.min(a.len(), b.len());
		this.effects = effects;
	}

	/** std's: its count of pairs read, with its iterators, which it reads by index as they are. */
	override clone(h?: Clones): Iter<[A, B]> {
		const it = new TrustedZipIter(this.a.clone(h?.i), this.b.clone(h?.o), this.effects, this.dropA, this.dropB);
		it.index = this.index;
		it.end = this.end;
		return it;
	}

	override next(): [A, B] | Done {
		if (this.index >= this.end) return DONE;
		this.index++;
		return super.next();
	}

	/**
	 * std's: each pair it skips made and dropped one side at a time, of only the sides that
	 * may have side effects, then `next`.
	 */
	override nth(n: number): [A, B] | Done {
		const skip = Math.min(n, this.end - this.index);
		const [effectA, effectB] = this.effects;
		if (!effectA) this.a.advanceBy(skip, this.dropA);
		if (!effectB) this.b.advanceBy(skip, this.dropB);
		if (!effectA && !effectB) this.index += skip;
		else
			for (let i = 0; i < skip; i++) {
				this.index++;
				if (effectA) {
					const a = this.a.next();
					if (a !== DONE) this.dropA?.(a);
				}
				if (effectB) {
					const b = this.b.next();
					if (b !== DONE) this.dropB?.(b);
				}
			}
		return this.next();
	}

	/**
	 * std's: the longer side trimmed to the pairs left, as only the first pair it reads
	 * from the back needs, reading each item past them back to front from a side that may
	 * have side effects, and none from one that has none.
	 */
	override nextBack(): [A, B] | Done {
		if (this.index >= this.end) return DONE;
		const left = this.end - this.index;
		trim(this.a, this.a.len() - left, this.effects[0], this.dropA);
		trim(this.b, this.b.len() - left, this.effects[1], this.dropB);
		this.end--;
		return super.nextBack();
	}

	/** `advance_by`, by `next`, in one step where neither side has side effects. */
	override advanceBy(n: number, drop?: Drop<[A, B]>): number {
		if (this.effects[0] || this.effects[1]) return super.advanceBy(n, drop);
		const skip = Math.min(n, this.len());
		this.index += skip;
		this.a.advanceBy(skip, this.dropA);
		this.b.advanceBy(skip, this.dropB);
		return n - skip;
	}

	/** `advance_back_by`, by `nextBack`, in one step where neither side has side effects. */
	override advanceBackBy(n: number, drop?: Drop<[A, B]>): number {
		if (this.effects[0] || this.effects[1]) return super.advanceBackBy(n, drop);
		const skip = Math.min(n, this.len());
		const left = this.len() - skip;
		trim(this.a, this.a.len() - left, false, this.dropA);
		trim(this.b, this.b.len() - left, false, this.dropB);
		this.end -= skip;
		return n - skip;
	}

	override len(): number {
		return this.end - this.index;
	}

	override sizeHint(): SizeHint {
		return exactHint(this.len());
	}
}

/** Drop `n` items from the back of `it`, read one at a time if it may have side effects. */
function trim<T>(it: Iter<T>, n: number, effect: boolean, drop?: Drop<T>): void {
	if (!effect) {
		it.advanceBackBy(n, drop);
		return;
	}
	for (let i = 0; i < n; i++) {
		const item = it.nextBack();
		if (item !== DONE) drop?.(item);
	}
}

class RevIter<T> extends Adaptor<T, T> {
	override sizeHint(): SizeHint {
		return this.inner.sizeHint();
	}

	next(): T | Done {
		return this.inner.nextBack();
	}

	override nextBack(): T | Done {
		return this.inner.next();
	}

	override len(): number {
		return this.inner.len();
	}

	override advanceBy(n: number, drop?: Drop<T>): number {
		return this.inner.advanceBackBy(n, drop);
	}

	override advanceBackBy(n: number, drop?: Drop<T>): number {
		return this.inner.advanceBy(n, drop);
	}

	override nth(n: number, drop?: Drop<T>): T | Done {
		return this.inner.nthBack(n, drop);
	}

	override nthBack(n: number, drop?: Drop<T>): T | Done {
		return this.inner.nth(n, drop);
	}

	override folded(fold = false): Iter<T> {
		const inner = this.inner.folded(fold);
		if (inner === this.inner) return this;
		return new RevIter(inner);
	}

	override clone(h?: Clones): Iter<T> {
		return new RevIter(this.inner.clone(h?.i));
	}

	override fold<A>(acc: A, f: (acc: A, item: T) => A, release?: () => void): A {
		return this.inner.rfold(acc, f, release);
	}

	override rfold<A>(acc: A, f: (acc: A, item: T) => A, release?: () => void): A {
		return this.inner.fold(acc, f, release);
	}
}

/** `Skip`, whose items drop by `drop` as its iterator's own `nth` skips them. */
class SkipIter<T> extends Adaptor<T, T> {
	n: number;
	private readonly drop: Drop<T> | undefined;

	constructor(inner: Iter<T>, n: number, drop?: Drop<T>) {
		super(inner);
		this.n = n;
		this.drop = drop;
	}

	override clone(h?: Clones): Iter<T> {
		return new SkipIter(this.inner.clone(h?.i), this.n, this.drop);
	}

	override sizeHint(): SizeHint {
		const [lower, upper] = this.inner.sizeHint();
		return [Math.max(lower - this.n, 0), upper === undefined ? undefined : Math.max(upper - this.n, 0)];
	}

	next(): T | Done {
		if (this.n === 0) return this.inner.next();
		const n = this.n;
		this.n = 0;
		return this.inner.nth(n, this.drop);
	}

	override nextBack(): T | Done {
		return this.len() > 0 ? this.inner.nextBack() : DONE;
	}

	override len(): number {
		return Math.max(this.inner.len() - this.n, 0);
	}

	/** std's: what it skips and `n` more in one step, and the rest of `n` if that saturated a `usize`. */
	override advanceBy(n: number, drop?: Drop<T>): number {
		const skip = this.n;
		const both = Math.min(skip + n, USIZE_MAX);
		const left = this.inner.advanceBy(both, drop);
		const advanced = both - left;
		n -= Math.max(advanced - skip, 0);
		this.n = Math.max(skip - advanced, 0);
		if (left === 0 && n > 0) n = this.inner.advanceBy(n, drop);
		return n;
	}

	/** std's: its iterator advanced from the back in one step, short of what it skips. */
	override advanceBackBy(n: number, drop?: Drop<T>): number {
		const min = Math.min(this.len(), n);
		this.inner.advanceBackBy(min, drop);
		return n - min;
	}

	/**
	 * std's: what it still skips and then `n` more, in one step, or past a `usize`'s
	 * greatest, what it skips in one and `n` in another.
	 */
	override nth(n: number, drop?: Drop<T>): T | Done {
		const skip = this.n;
		this.n = 0;
		if (skip + n <= USIZE_MAX) return this.inner.nth(skip + n, drop);
		const item = this.inner.nth(skip - 1, drop);
		if (item === DONE) return DONE;
		drop?.(item);
		return this.inner.nth(n, drop);
	}

	/**
	 * std's: past what it skips by `nth`, one short of where `next` reads, then its iterator
	 * folded, and from the back, its iterator folded down to what it skips.
	 */
	override folded(fold = false): Iter<T> {
		const inner = this.inner.folded(fold);
		if (inner === this.inner) return this;
		const front = (): T | Done => {
			if (this.n > 0) {
				const n = this.n;
				this.n = 0;
				const item = this.inner.nth(n - 1, this.drop);
				if (item === DONE) return DONE;
				this.drop?.(item);
			}
			return inner.next();
		};
		return new Folded(front, () => (this.len() > 0 ? inner.nextBack() : DONE));
	}

	/** std's: past what it skips by `nth`, and then its iterator's own. */
	override fold<A>(acc: A, f: (acc: A, item: T) => A, release?: () => void): A {
		if (this.n > 0) {
			const item = this.inner.nth(this.n - 1, this.drop);
			if (item === DONE) {
				ended(this.inner, release);
				return acc;
			}
			this.drop?.(item);
		}
		return this.inner.fold(acc, f, release);
	}

	/** std's: past what it doesn't skip, its iterator read from the back through what it does. */
	override nthBack(n: number, drop?: Drop<T>): T | Done {
		const len = this.len();
		if (n < len) return this.inner.nthBack(n, drop);
		if (len > 0) {
			const item = this.inner.nthBack(len - 1, drop);
			if (item !== DONE) drop?.(item);
		}
		return DONE;
	}
}

/** `Take`, of which what its iterator holds past the limit drops by `drop` as `next_back` reads through it. */
class TakeIter<T> extends Adaptor<T, T> {
	n: number;
	private readonly drop: Drop<T> | undefined;

	constructor(inner: Iter<T>, n: number, drop?: Drop<T>) {
		super(inner);
		this.n = n;
		this.drop = drop;
	}

	override clone(h?: Clones): Iter<T> {
		return new TakeIter(this.inner.clone(h?.i), this.n, this.drop);
	}

	override sizeHint(): SizeHint {
		if (this.n === 0) return [0, 0];
		const [lower, upper] = this.inner.sizeHint();
		return [Math.min(lower, this.n), Math.min(upper ?? this.n, this.n)];
	}

	next(): T | Done {
		if (this.n === 0) return DONE;
		this.n--;
		return this.inner.next();
	}

	/** std's: what its iterator holds past the limit is read through from the back. */
	override nextBack(): T | Done {
		if (this.n === 0) return DONE;
		const n = this.n--;
		return this.inner.nthBack(Math.max(this.inner.len() - n, 0), this.drop);
	}

	override len(): number {
		return Math.min(this.inner.len(), this.n);
	}

	override advanceBy(n: number, drop?: Drop<T>): number {
		const min = Math.min(this.n, n);
		const advanced = min - this.inner.advanceBy(min, drop);
		this.n -= advanced;
		return n - advanced;
	}

	/**
	 * std's: its iterator folded, counting each item it reads against the limit, and from
	 * the back, past what it holds beyond the limit by `nth_back`, folded without counting.
	 */
	override folded(): Iter<T> {
		const inner = this.inner.folded();
		if (inner === this.inner) return this;
		const front = (): T | Done => {
			if (this.n === 0) return DONE;
			const item = inner.next();
			if (item !== DONE) this.n--;
			return item;
		};
		const back = (): T | Done => {
			if (this.n === 0) return DONE;
			const len = this.inner.len();
			if (len > this.n) {
				const item = this.inner.nthBack(len - this.n - 1, this.drop);
				if (item === DONE) return DONE;
				this.drop?.(item);
			}
			return inner.nextBack();
		};
		return new Folded(front, back);
	}

	/** std's: its iterator advanced from the back past the limit and `n` more, in one step. */
	override advanceBackBy(n: number, drop?: Drop<T>): number {
		const trim = Math.max(this.inner.len() - this.n, 0);
		const advanced = trim + n - this.inner.advanceBackBy(trim + n, drop) - trim;
		this.n -= advanced;
		return n - advanced;
	}

	/** std's: past the limit, it reads the rest of the limit through, drops it, and ends. */
	override nth(n: number, drop?: Drop<T>): T | Done {
		if (this.n > n) {
			this.n -= n + 1;
			return this.inner.nth(n, drop);
		}
		if (this.n > 0) {
			const item = this.inner.nth(this.n - 1, drop);
			if (item !== DONE) drop?.(item);
			this.n = 0;
		}
		return DONE;
	}

	/** std's: past what its iterator holds beyond the limit by `nth_back`, and then its iterator's own. */
	override rfold<A>(acc: A, f: (acc: A, item: T) => A, release?: () => void): A {
		const len = this.n === 0 ? 0 : this.inner.len();
		if (this.n === 0 || len > this.n) {
			const item = this.n > 0 ? this.inner.nthBack(len - this.n - 1, this.drop) : DONE;
			if (item === DONE) {
				ended(this.inner, release);
				return acc;
			}
			this.drop?.(item);
		}
		return this.inner.rfold(acc, f, release);
	}

	/** std's: past the limit, it reads the whole rest from the back, even at a limit of 0. */
	override nthBack(n: number, drop?: Drop<T>): T | Done {
		const len = this.inner.len();
		if (this.n > n) {
			const m = Math.max(len - this.n, 0) + n;
			this.n -= n + 1;
			return this.inner.nthBack(m, drop);
		}
		if (len > 0) {
			const item = this.inner.nthBack(len - 1, drop);
			if (item !== DONE) drop?.(item);
		}
		return DONE;
	}
}

class SkipWhileIter<T> extends Calling<T, T, (item: T) => boolean> {
	private readonly drop: Drop<T> | undefined;
	private flag = false;

	constructor(inner: Iter<T>, f: (item: T) => boolean, drop?: Drop<T>) {
		super(inner, f);
		this.drop = drop;
	}

	override clone(h?: Clones): Iter<T> {
		const it = new SkipWhileIter(this.inner.clone(h?.i), cloned(this.f, h?.f), this.drop);
		it.flag = this.flag;
		return it;
	}

	next(): T | Done {
		return findItem(
			this.inner,
			(item) => {
				if (this.flag || !this.f(item)) {
					this.flag = true;
					return true;
				}
				return false;
			},
			this.drop,
		);
	}

	override sizeHint(): SizeHint {
		return fewer(this.inner);
	}

	/** std's `fold`: its first item by `next`, and then its iterator folded. */
	override folded(fold = false): Iter<T> {
		const inner = fold ? this.inner.folded(true) : this.inner;
		if (inner === this.inner) return this;
		return new FromNext(inner, (inner) => (this.flag ? inner.next() : this.next()));
	}

	/** std's: its first item by `next`, and then its iterator's fold, after which `f` drops. */
	override fold<A>(acc: A, f: (acc: A, item: T) => A, release?: () => void): A {
		if (!this.flag) {
			let done: boolean;
			[done, acc] = foldFirst(this, () => this.next(), acc, f, release);
			if (done) return acc;
		}
		const pred = this.take();
		acc = this.inner.fold(acc, f, release);
		pred();
		return acc;
	}
}

class TakeWhileIter<T> extends Calling<T, T, (item: T) => boolean> {
	private readonly drop: Drop<T> | undefined;
	private flag = false;

	constructor(inner: Iter<T>, f: (item: T) => boolean, drop?: Drop<T>) {
		super(inner, f);
		this.drop = drop;
	}

	override clone(h?: Clones): Iter<T> {
		const it = new TakeWhileIter(this.inner.clone(h?.i), cloned(this.f, h?.f), this.drop);
		it.flag = this.flag;
		return it;
	}

	next(): T | Done {
		return this.from(this.inner);
	}

	override folded(): Iter<T> {
		const inner = this.inner.folded();
		if (inner === this.inner) return this;
		return new FromNext(inner, (inner) => this.from(inner));
	}

	/** The next item of `inner`, its iterator or that folded, while `f` holds of each. */
	private from(inner: Iter<T>): T | Done {
		if (this.flag) return DONE;
		const item = inner.next();
		if (item === DONE) return DONE;
		if (this.f(item)) return item;
		this.flag = true;
		this.drop?.(item);
		return DONE;
	}

	override sizeHint(): SizeHint {
		return this.flag ? [0, 0] : fewer(this.inner);
	}
}

/** `MapWhile`, which reads on after `f` returns `None` if called again, as std's does. */
class MapWhileIter<T, U> extends Calling<T, U, (item: T) => U | undefined | None> {
	/** The `Option`'s `None` that `f` returns. */
	private readonly none: None | undefined;

	constructor(inner: Iter<T>, f: (item: T) => U | undefined | None, none?: None) {
		super(inner, f);
		this.none = none;
	}

	next(): U | Done {
		const item = this.inner.next();
		return item === DONE ? DONE : iterFromOption(this.f(item), this.none);
	}

	override folded(): Iter<U> {
		const inner = this.inner.folded();
		if (inner === this.inner) return this;
		return new MapWhileIter(inner, this.f, this.none);
	}

	override clone(h?: Clones): Iter<U> {
		return new MapWhileIter(this.inner.clone(h?.i), cloned(this.f, h?.f), this.none);
	}

	override sizeHint(): SizeHint {
		return fewer(this.inner);
	}
}

/** `StepBy`, whose items drop by `drop` as its iterator's own `nth` and `nth_back` step over them. */
class StepByIter<T> extends Adaptor<T, T> {
	readonly step: number;
	firstTake = true;
	private readonly drop: Drop<T> | undefined;

	constructor(inner: Iter<T>, step: number, drop?: Drop<T>) {
		super(inner);
		if (step === 0) panic("assertion failed: step != 0");
		this.step = step - 1;
		this.drop = drop;
	}

	override clone(h?: Clones): Iter<T> {
		const it = new StepByIter(this.inner.clone(h?.i), this.step + 1, this.drop);
		it.firstTake = this.firstTake;
		return it;
	}

	next(): T | Done {
		const n = this.firstTake ? 0 : this.step;
		this.firstTake = false;
		return this.inner.nth(n, this.drop);
	}

	override nextBack(): T | Done {
		return this.inner.nthBack(this.backIndex(), this.drop);
	}

	/** How far from the back its last item is. */
	backIndex(): number {
		const rem = this.inner.len() % (this.step + 1);
		return this.firstTake ? (rem === 0 ? this.step : rem - 1) : rem;
	}

	override len(): number {
		return this.size(this.inner.len());
	}

	override sizeHint(): SizeHint {
		const [lower, upper] = this.inner.sizeHint();
		return [this.size(lower), upper === undefined ? undefined : this.size(upper)];
	}

	/** How many of `n` items it yields. */
	private size(n: number): number {
		if (this.firstTake) return n === 0 ? 0 : 1 + Math.floor((n - 1) / (this.step + 1));
		return Math.floor(n / (this.step + 1));
	}

	/**
	 * std's: the first item, dropped unless it is the one, then every step in one `nth`,
	 * as many as a `usize` counts at a time.
	 */
	override nth(n: number, drop?: Drop<T>): T | Done {
		if (this.firstTake) {
			this.firstTake = false;
			const first = this.inner.next();
			if (first === DONE || n === 0) return first;
			drop?.(first);
			n -= 1;
		}
		let step = this.step + 1;
		if (n === USIZE_MAX) {
			const item = this.inner.nth(step - 1, drop);
			if (item === DONE) return DONE;
			drop?.(item);
		} else {
			n += 1;
		}
		for (;;) {
			if (n * step <= USIZE_MAX) return this.inner.nth(n * step - 1, drop);
			const byN = Math.floor(USIZE_MAX / n) * n;
			const byStep = Math.floor(USIZE_MAX / step) * step;
			if (byN > byStep) step -= Math.floor(USIZE_MAX / n);
			else n -= Math.floor(USIZE_MAX / step);
			const item = this.inner.nth(Math.max(byN, byStep) - 1, drop);
			if (item === DONE) return DONE;
			drop?.(item);
		}
	}

	/** std's: every step from the back in one, saturating at a `usize`'s greatest. */
	override nthBack(n: number, drop?: Drop<T>): T | Done {
		const steps = Math.min(n * (this.step + 1), USIZE_MAX);
		return this.inner.nthBack(Math.min(steps + this.backIndex(), USIZE_MAX), drop);
	}

	/** std's: the first item by `next`, then each step by `nth`, as `next` reads past it. */
	override fold<A>(acc: A, f: (acc: A, item: T) => A, release?: () => void): A {
		if (this.firstTake) {
			this.firstTake = false;
			let done: boolean;
			[done, acc] = foldFirst(this.inner, () => this.inner.next(), acc, f, release);
			if (done) return acc;
		}
		return foldBy(this, false, acc, f, release);
	}

	/** std's: the last item by `next_back`, then each step back by `nth_back`. */
	override rfold<A>(acc: A, f: (acc: A, item: T) => A, release?: () => void): A {
		let done: boolean;
		[done, acc] = foldFirst(this.inner, () => this.nextBack(), acc, f, release);
		if (done) return acc;
		const steps = new FromNext(this.inner, (inner) => inner.nthBack(this.step, this.drop));
		acc = foldBy(steps, false, acc, f, release);
		dispose(this.inner);
		return acc;
	}
}

/**
 * std's `Cycle`: `iter`, read, and `orig`, the iterator as it was, which it clones by `h`
 * in place of `iter` each time that ends. It drops `orig` and then `iter`, as std's fields
 * order them.
 */
class CycleIter<T> extends Iter<T> implements Disposable {
	readonly orig: Iter<T>;
	iter: Iter<T>;
	private readonly h: Clones | undefined;

	constructor(orig: Iter<T>, iter: Iter<T>, h: Clones | undefined) {
		super();
		this.orig = orig;
		this.iter = iter;
		this.h = h;
	}

	/** A clone of `orig` in place of `iter`, which drops once the clone is made. */
	private restart(): void {
		const ended = this.iter;
		this.iter = this.orig.clone(this.h);
		dispose(ended);
	}

	next(): T | Done {
		const item = this.iter.next();
		if (item !== DONE) return item;
		this.restart();
		return this.iter.next();
	}

	/** std's: by `orig`'s, either empty or endless. */
	override sizeHint(): SizeHint {
		const [lower, upper] = this.orig.sizeHint();
		if (lower > 0) return [USIZE_MAX, undefined];
		return upper === 0 ? [0, 0] : [0, undefined];
	}

	/** std's: `orig` by `h.i`, then `iter`. */
	override clone(h?: Clones): Iter<T> {
		return new CycleIter(this.orig.clone(h?.i), this.iter.clone(h?.i), this.h);
	}

	/** std's: `iter` advanced, then clones, until one advances no further than it had left. */
	override advanceBy(n: number, drop?: Drop<T>): number {
		n = this.iter.advanceBy(n, drop);
		while (n > 0) {
			this.restart();
			const left = this.iter.advanceBy(n, drop);
			if (left === n) return n;
			n = left;
		}
		return 0;
	}

	/**
	 * std's `try_fold`: the rest of `iter` folded, then a clone, which ends it if that was
	 * empty, and then clones without end. `fold` is std's default, by `next`.
	 */
	override folded(fold = false): Iter<T> {
		if (fold) return this;
		let reading = this.iter.folded();
		// Which clone it reads: 0 for none yet, 1 for the first, and 2 for one after.
		let clones = 0;
		let empty = true;
		return new FromNext(this, (cycle) => {
			for (;;) {
				const item = reading.next();
				if (item !== DONE) {
					if (clones === 1) empty = false;
					return item;
				}
				if (clones === 1 && empty) return DONE;
				clones = Math.min(clones + 1, 2);
				cycle.restart();
				reading = cycle.iter.folded();
			}
		});
	}

	[Symbol.dispose](): void {
		dispose(this.orig);
		dispose(this.iter);
	}
}

class InspectIter<T> extends Calling<T, T, (item: T) => void> {
	override sizeHint(): SizeHint {
		return this.inner.sizeHint();
	}

	next(): T | Done {
		return this.inspect(this.inner.next());
	}

	override nextBack(): T | Done {
		return this.inspect(this.inner.nextBack());
	}

	/** `f` of `item`. */
	private inspect(item: T | Done): T | Done {
		if (item !== DONE) this.f(item);
		return item;
	}

	override len(): number {
		return this.inner.len();
	}

	override folded(fold = false): Iter<T> {
		const inner = this.inner.folded(fold);
		if (inner === this.inner) return this;
		return new InspectIter(inner, this.f);
	}

	override clone(h?: Clones): Iter<T> {
		return new InspectIter(this.inner.clone(h?.i), cloned(this.f, h?.f));
	}

	/** std's: `inner`'s, by a closure that drops `f` and then `release`'s. */
	override fold<A>(acc: A, g: (acc: A, item: T) => A, release?: () => void): A {
		return this.inner.fold(acc, this.inspectFold(g), then(this.take(), release));
	}

	override rfold<A>(acc: A, g: (acc: A, item: T) => A, release?: () => void): A {
		return this.inner.rfold(acc, this.inspectFold(g), then(this.take(), release));
	}

	/** std's `inspect_fold`: `f` of each item, then `g`. */
	private inspectFold<A>(g: (acc: A, item: T) => A): (acc: A, item: T) => A {
		const f = this.f;
		return (acc, item) => {
			f(item);
			return g(acc, item);
		};
	}
}

class FuseIter<T> extends Iter<T> implements Disposable {
	inner: Iter<T> | undefined;
	/** Whether its iterator is a `FusedIterator`, which std's keeps once it ends. */
	private readonly fused: boolean;

	constructor(inner: Iter<T> | undefined, fused: boolean) {
		super();
		this.inner = inner;
		this.fused = fused;
	}

	/** std's: its iterator, while it holds it. */
	override clone(h?: Clones): Iter<T> {
		return new FuseIter(this.inner?.clone(h?.i), this.fused);
	}

	override sizeHint(): SizeHint {
		return this.inner?.sizeHint() ?? [0, 0];
	}

	/** `item`, dropping its iterator first if it ended there and is not fused. */
	clear(item: T | Done): T | Done {
		if (item === DONE && !this.fused) {
			const inner = this.inner;
			this.inner = undefined;
			dispose(inner);
		}
		return item;
	}

	next(): T | Done {
		return this.inner === undefined ? DONE : this.clear(this.inner.next());
	}

	override nextBack(): T | Done {
		return this.inner === undefined ? DONE : this.clear(this.inner.nextBack());
	}

	override len(): number {
		return this.inner === undefined ? 0 : this.inner.len();
	}

	[Symbol.dispose](): void {
		const inner = this.inner;
		this.inner = undefined;
		dispose(inner);
	}

	override nth(n: number, drop?: Drop<T>): T | Done {
		return this.inner === undefined ? DONE : this.clear(this.inner.nth(n, drop));
	}

	/** std's: its iterator folded, dropped once it ends unless it is fused. */
	override folded(fold = false): Iter<T> {
		const inner = this.inner?.folded(fold);
		if (inner === this.inner) return this;
		const front = () => (this.inner === undefined ? DONE : this.clear(inner!.next()));
		const back = () => (this.inner === undefined ? DONE : this.clear(inner!.nextBack()));
		return new Folded(front, back);
	}

	override nthBack(n: number, drop?: Drop<T>): T | Done {
		return this.inner === undefined ? DONE : this.clear(this.inner.nthBack(n, drop));
	}

	override fold<A>(acc: A, f: (acc: A, item: T) => A, release?: () => void): A {
		const inner = this.inner;
		this.inner = undefined;
		if (inner !== undefined) return inner.fold(acc, f, release);
		release?.();
		return acc;
	}

	override rfold<A>(acc: A, f: (acc: A, item: T) => A, release?: () => void): A {
		const inner = this.inner;
		this.inner = undefined;
		if (inner !== undefined) return inner.rfold(acc, f, release);
		release?.();
		return acc;
	}
}

/**
 * `Flatten` and `FlatMap`: the iterator of each item it reads, which drops once it ends,
 * as does its source unless it is `fused`.
 */
class FlattenIter<T, V> extends Iter<V> implements Disposable {
	outer: Iter<T> | undefined;
	readonly into: (value: T) => Iter<V>;
	readonly fused: boolean;
	/** How many items each of its source's has, if each is an array. */
	readonly size: number | undefined;
	/** Whether each of its source's items makes an iterator of at most one item. */
	readonly oneShot: boolean;
	front: Iter<V> | undefined;
	back: Iter<V> | undefined;

	constructor(outer: Iter<T> | undefined, into: (value: T) => Iter<V>, fused: boolean, size?: number, oneShot = false) {
		super();
		this.outer = outer;
		this.into = into;
		this.fused = fused;
		this.size = size;
		this.oneShot = oneShot;
	}

	/** std's: its source by `h.i`, while it holds it, then its front and back by `h.o`. */
	override clone(h?: Clones): Iter<V> {
		const it = new FlattenIter(this.outer?.clone(h?.i), this.into, this.fused, this.size, this.oneShot);
		it.front = this.front?.clone(h?.o);
		it.back = this.back?.clone(h?.o);
		return it;
	}

	/**
	 * std's: what its front and back hold, and its source's items, if it knows how many
	 * each has. Of items that hold at most one, only its source's, which std never holds
	 * the iterator of.
	 */
	override sizeHint(): SizeHint {
		const [lower, upper] = this.outer?.sizeHint() ?? [0, 0];
		if (this.oneShot) {
			if (this.size === 0) return [0, 0];
			return this.size === 1 ? [lower, upper] : [0, upper];
		}
		const [frontLower, frontUpper] = this.front?.sizeHint() ?? [0, 0];
		const [backLower, backUpper] = this.back?.sizeHint() ?? [0, 0];
		const held = satAdd(frontLower, backLower);
		const heldUpper = addUpper(frontUpper, backUpper);
		if (this.size !== undefined) {
			const all = upper === undefined || upper * this.size > USIZE_MAX ? undefined : upper * this.size;
			return [satAdd(lower * this.size, held), addUpper(heldUpper, all)];
		}
		return [held, lower === 0 && upper === 0 ? heldUpper : undefined];
	}

	/** The source's next item, or `DONE`, which drops it unless it is fused. */
	private nextOuter(back: boolean): T | Done {
		const outer = this.outer;
		if (outer === undefined) return DONE;
		const item = back ? outer.nextBack() : outer.next();
		if (item === DONE && !this.fused) {
			this.outer = undefined;
			dispose(outer);
		}
		return item;
	}

	/** `it`'s next item, from the back if `back`, dropping it if it ended. */
	private static step<V>(it: Iter<V>, back: boolean): V | Done {
		const item = back ? it.nextBack() : it.next();
		if (item === DONE) dispose(it);
		return item;
	}

	next(): V | Done {
		for (;;) {
			if (this.front !== undefined) {
				const item = FlattenIter.step(this.front, false);
				if (item !== DONE) return item;
				this.front = undefined;
			}
			const inner = this.nextOuter(false);
			if (inner === DONE) {
				if (this.back === undefined) return DONE;
				const item = FlattenIter.step(this.back, false);
				if (item === DONE) this.back = undefined;
				return item;
			}
			this.front = this.into(inner);
		}
	}

	override nextBack(): V | Done {
		for (;;) {
			if (this.back !== undefined) {
				const item = FlattenIter.step(this.back, true);
				if (item !== DONE) return item;
				this.back = undefined;
			}
			const inner = this.nextOuter(true);
			if (inner === DONE) {
				if (this.front === undefined) return DONE;
				const item = FlattenIter.step(this.front, true);
				if (item === DONE) this.front = undefined;
				return item;
			}
			this.back = this.into(inner);
		}
	}

	/** Hold `it` as the front iterator, dropping the one it held, as std's `insert` does. */
	setFront(it: Iter<V> | undefined): void {
		const old = this.front;
		this.front = it;
		dispose(old);
	}

	setBack(it: Iter<V> | undefined): void {
		const old = this.back;
		this.back = it;
		dispose(old);
	}

	[Symbol.dispose](): void {
		const outer = this.outer;
		const front = this.front;
		const back = this.back;
		this.outer = undefined;
		this.front = undefined;
		this.back = undefined;
		dispose(outer);
		dispose(front);
		dispose(back);
	}

	/** Its fold's view, which each fold starts anew. */
	private view: FlattenFold<T, V> | undefined;

	override folded(fold = false): Iter<V> {
		this.view ??= new FlattenFold(this);
		return this.view.reset(fold);
	}

	override fold<A>(acc: A, f: (acc: A, item: V) => A, release?: () => void): A {
		return flattenFold(this, false, acc, (acc, inner) => inner.fold(acc, f), release);
	}

	override rfold<A>(acc: A, f: (acc: A, item: V) => A, release?: () => void): A {
		return flattenFold(this, true, acc, (acc, inner) => inner.rfold(acc, f), release);
	}

	/**
	 * std's, by `try_fold`: each iterator in turn advanced by its own `advance_by`, of its
	 * source folded, one that falls short dropped only as the next replaces it, and the
	 * source dropped once it ends unless it is fused.
	 */
	override advanceBy(n: number, drop?: Drop<V>): number {
		if (this.front !== undefined) {
			n = this.front.advanceBy(n, drop);
			if (n === 0) return 0;
		}
		this.setFront(undefined);
		const outer = this.outer;
		if (outer !== undefined) {
			const source = outer.folded();
			for (let inner = source.next(); inner !== DONE; inner = source.next()) {
				const front = this.into(inner);
				this.setFront(front);
				n = front.advanceBy(n, drop);
				if (n === 0) return 0;
			}
			if (!this.fused) {
				this.outer = undefined;
				dispose(outer);
			}
		}
		this.setFront(undefined);
		if (this.back !== undefined) {
			n = this.back.advanceBy(n, drop);
			if (n === 0) return 0;
		}
		this.setBack(undefined);
		return n;
	}

	override advanceBackBy(n: number, drop?: Drop<V>): number {
		if (this.back !== undefined) {
			n = this.back.advanceBackBy(n, drop);
			if (n === 0) return 0;
		}
		this.setBack(undefined);
		const outer = this.outer;
		if (outer !== undefined) {
			const source = outer.folded();
			for (let inner = source.nextBack(); inner !== DONE; inner = source.nextBack()) {
				const back = this.into(inner);
				this.setBack(back);
				n = back.advanceBackBy(n, drop);
				if (n === 0) return 0;
			}
			if (!this.fused) {
				this.outer = undefined;
				dispose(outer);
			}
		}
		this.setBack(undefined);
		if (this.front !== undefined) {
			n = this.front.advanceBackBy(n, drop);
			if (n === 0) return 0;
		}
		this.setFront(undefined);
		return n;
	}
}

/**
 * A flatten as std's `try_fold` reads it, or `try_rfold` from the back: the iterator it
 * holds at that end folded, which then drops, then its source folded, each iterator it
 * makes held and folded in turn, dropping the one before, and then the other end's, which
 * drops once it ends. The source drops once it ends unless it is fused.
 */
class FlattenFold<T, V> extends Iter<V> {
	readonly of: FlattenIter<T, V>;
	/** Whether it reads as `fold` does, which folds its source and each iterator. */
	private whole = false;
	/** At its own end, then its source's, then the other end. */
	private phase = 0;
	private source: Iter<T> | undefined;
	/** The iterator it reads, and that folded. */
	private held: Iter<V> | undefined;
	private reading: Iter<V> | undefined;

	constructor(of: FlattenIter<T, V>) {
		super();
		this.of = of;
	}

	/** Itself, at the start of a fold, or a `try_fold` unless `fold`. */
	reset(fold: boolean): this {
		this.whole = fold;
		this.phase = 0;
		this.source = this.of.outer?.folded(fold);
		this.held = this.reading = undefined;
		return this;
	}

	next(): V | Done {
		return this.step(false);
	}

	override nextBack(): V | Done {
		return this.step(true);
	}

	private step(back: boolean): V | Done {
		const of = this.of;
		for (;;) {
			const it = back !== (this.phase === 2) ? of.back : of.front;
			if (it !== undefined) {
				if (it !== this.held) [this.held, this.reading] = [it, it.folded(this.whole)];
				const item = back ? this.reading!.nextBack() : this.reading!.next();
				if (item !== DONE) return item;
			}
			if (this.phase === 2) {
				this.hold(back, undefined, true);
				return DONE;
			}
			if (this.phase === 1) {
				const outer = of.outer;
				const value = outer === undefined ? DONE : back ? this.source!.nextBack() : this.source!.next();
				if (value !== DONE) {
					this.hold(back, of.into(value), false);
					continue;
				}
				if (outer !== undefined && !of.fused) {
					of.outer = undefined;
					dispose(outer);
				}
			}
			this.hold(back, undefined, false);
			this.phase++;
		}
	}

	/** Hold `it` at the end it folds from, from the back if `back`, or at the other. */
	private hold(back: boolean, it: Iter<V> | undefined, other: boolean): void {
		if (back !== other) this.of.setBack(it);
		else this.of.setFront(it);
	}
}

/** `Peekable`, which drops the item it peeked by `drop`. */
export class Peekable<T> extends Adaptor<T, T> {
	peeked = false;
	item: T | Done = DONE;
	readonly drop: Drop<T> | undefined;

	constructor(inner: Iter<T>, drop?: Drop<T>) {
		super(inner);
		this.drop = drop;
	}

	/** std's: its iterator by `h.i`, then the item it peeked by `h.v`. */
	override clone(h?: Clones): Peekable<T> {
		const it = new Peekable(this.inner.clone(h?.i), this.drop);
		it.peeked = this.peeked;
		it.item = this.item === DONE ? DONE : cloned(this.item, h?.v);
		return it;
	}

	/** The item it peeked, if any, which it no longer holds. */
	take(): T | Done {
		this.peeked = false;
		const item = this.item;
		this.item = DONE;
		return item;
	}

	/** std's: none past a peeked end, and one more than its iterator's for a peeked item. */
	override sizeHint(): SizeHint {
		if (!this.peeked) return this.inner.sizeHint();
		if (this.item === DONE) return [0, 0];
		const [lower, upper] = this.inner.sizeHint();
		return [satAdd(lower, 1), addUpper(upper, 1)];
	}

	next(): T | Done {
		if (!this.peeked) return this.inner.next();
		return this.take();
	}

	override nextBack(): T | Done {
		if (!this.peeked) return this.inner.nextBack();
		if (this.item === DONE) return DONE;
		const item = this.inner.nextBack();
		if (item !== DONE) return item;
		// The peeked item is the last: taking it leaves the end peeked.
		const peeked = this.item;
		this.item = DONE;
		return peeked;
	}

	override len(): number {
		if (!this.peeked) return this.inner.len();
		return this.item === DONE ? 0 : this.inner.len() + 1;
	}

	override [Symbol.dispose](): void {
		dispose(this.inner);
		const item = this.take();
		if (item !== DONE) this.drop?.(item);
	}

	/**
	 * std's: the item it peeked, or none past a peeked end, and then its iterator folded; from
	 * the back, its iterator folded and then the item it peeked, which it holds until then.
	 */
	override folded(fold = false): Iter<T> {
		const inner = this.inner.folded(fold);
		if (inner === this.inner) return this;
		const front = () => (this.peeked ? this.take() : inner.next());
		const back = (): T | Done => {
			if (!this.peeked) return inner.nextBack();
			if (this.item === DONE) return this.take();
			const item = inner.nextBack();
			return item !== DONE ? item : this.take();
		};
		return new Folded(front, back);
	}

	/** std's: the item it peeked, and then its iterator's fold. */
	override fold<A>(acc: A, f: (acc: A, item: T) => A, release?: () => void): A {
		if (this.peeked) {
			let done: boolean;
			[done, acc] = foldFirst(this.inner, () => this.take(), acc, f, release);
			if (done) return acc;
		}
		return this.inner.fold(acc, f, release);
	}

	/** std's: its iterator's from the back, and then the item it peeked. */
	override rfold<A>(acc: A, f: (acc: A, item: T) => A, release?: () => void): A {
		if (!this.peeked) return this.inner.rfold(acc, f, release);
		const peeked = this.take();
		if (peeked === DONE) {
			ended(this.inner, release);
			return acc;
		}
		acc = f(this.inner.rfold(acc, f), peeked);
		release?.();
		return acc;
	}

	/** std's: the item it peeked drops after what its iterator skips. */
	override nth(n: number, drop?: Drop<T>): T | Done {
		if (!this.peeked) return this.inner.nth(n, drop);
		const peeked = this.take();
		if (peeked === DONE) return DONE;
		if (n === 0) return peeked;
		const item = this.inner.nth(n - 1, drop);
		drop?.(peeked);
		return item;
	}
}

/** The item `p` peeked, peeking it first if it has not. */
function peeked<T>(p: Peekable<T>): T | Done {
	if (!p.peeked) {
		p.item = p.inner.next();
		p.peeked = true;
	}
	return p.item;
}

/** `Peekable::peek`. */
export function peekablePeek<T>(p: Peekable<T>): T | undefined;
export function peekablePeek<T>(p: Peekable<T>, none: None): T | None;
export function peekablePeek<T>(p: Peekable<T>, none?: None): T | undefined | None {
	return iterToOption(peeked(p), none);
}

/** `Peekable::peek_mut`: the item it peeked, as its place if it is one JS copies. */
export function peekablePeekMut<T>(p: Peekable<T>, lend: true): Place<T> | undefined;
export function peekablePeekMut<T>(p: Peekable<T>, lend: false): T | undefined;
export function peekablePeekMut<T>(p: Peekable<T>, lend: boolean): Place<T> | T | undefined {
	const item = peeked(p);
	if (item === DONE) return undefined;
	return lend ? place(() => p.item as T, (value: T) => (p.item = value)) : item;
}

/** `Peekable::next_if`: an item `f` rejects is peeked again. */
export function peekableNextIf<T>(p: Peekable<T>, f: (item: T) => boolean): T | undefined;
export function peekableNextIf<T>(p: Peekable<T>, f: (item: T) => boolean, none: None): T | None;
export function peekableNextIf<T>(p: Peekable<T>, f: (item: T) => boolean, none?: None): T | undefined | None;
export function peekableNextIf<T>(p: Peekable<T>, f: (item: T) => boolean, none?: None): T | undefined | None {
	const item = p.next();
	if (item !== DONE && f(item)) return item;
	p.peeked = true;
	p.item = item;
	return none;
}

/** `Peekable::next_if_eq`: `next_if` by `eq` of the next item and `expected`. */
export function peekableNextIfEq<T, U>(p: Peekable<T>, expected: U, eq: (a: T, b: U) => boolean): T | undefined;
export function peekableNextIfEq<T, U>(p: Peekable<T>, expected: U, eq: (a: T, b: U) => boolean, none: None): T | None;
export function peekableNextIfEq<T, U>(p: Peekable<T>, expected: U, eq: (a: T, b: U) => boolean, none?: None): T | undefined | None {
	return peekableNextIf(p, (item: T) => eq(item, expected), none);
}

/** What `iterCollectResult` collects from: values until the first error, which it keeps. */
class ResultShunt<U, E> extends Adaptor<Result<U, E>, U> {
	error: { value: E } | undefined;

	/** std's, by `try_for_each`, which folds its iterator. */
	next(): U | Done {
		if (this.error !== undefined) return DONE;
		const item = this.inner.folded().next();
		if (item === DONE) return DONE;
		if (item.ok) return item.value;
		this.error = { value: item.error };
		return DONE;
	}

	override sizeHint(): SizeHint {
		return this.error === undefined ? fewer(this.inner) : [0, 0];
	}
}

/** What `iterCollectOption` collects from: values until the first `None`, which is `itemNone`. */
class OptionShunt<U> extends Adaptor<U | undefined | None, U> {
	none = false;
	private readonly itemNone: None | undefined;

	constructor(inner: Iter<U | undefined | None>, itemNone: None | undefined) {
		super(inner);
		this.itemNone = itemNone;
	}

	next(): U | Done {
		if (this.none) return DONE;
		const item = this.inner.folded().next();
		if (item === DONE) return DONE;
		if (item !== this.itemNone) return item as U;
		this.none = true;
		return DONE;
	}

	override sizeHint(): SizeHint {
		return this.none ? [0, 0] : fewer(this.inner);
	}
}

/** `a < b` of two integers, `char`s, or floats of one type, or two booleans. */
function lt<T>(a: T, b: T): boolean {
	if (typeof a === "object" || typeof b === "object") return wideLt(a as number | Limbs, b as number | Limbs);
	return (a as number) < (b as number);
}

/** `a <= b` of two of what `lt` compares: never of a NaN. */
function le<T>(a: T, b: T): boolean {
	if (typeof a === "object" || typeof b === "object") return !wideLt(b as number | Limbs, a as number | Limbs);
	return (a as number) <= (b as number);
}

/**
 * `n + by` in the integer type `int` steps, or `undefined` past its range, which a range
 * stepping past its end only needs to stay past. A narrow integer's sum is an exact
 * number whatever its type; a `char`'s skips the surrogates.
 */
function step<T>(n: T, by: number, int: Step<T> | undefined): T | undefined {
	if (typeof n === "number" && int !== (charStep as unknown) && Number.isSafeInteger(n + by)) return (n + by) as T;
	if (int === undefined) throw new Error("rust2-ts: stepping a wide range without its type");
	const [sum, over] = int(n, by);
	return over ? undefined : sum;
}

/**
 * std's `Step::steps_between` of `a` and `b`, at most `b`, in the type `int` steps:
 * `(usize::MAX, None)` past `usize::MAX`, and less the surrogates `char`s skip.
 */
function stepsBetween<T>(a: T, b: T, int: Step<T> | undefined): SizeHint {
	if (int === (charStep as unknown)) {
		const n = (b as number) - (a as number);
		return exactHint((a as number) < 0xd800 && (b as number) >= 0xe000 ? n - 0x800 : n);
	}
	const n = typeof a === "number" && typeof b === "number" ? b - a : intToBig(b as number | Limbs) - intToBig(a as number | Limbs);
	return n <= USIZE_MAX ? exactHint(Number(n)) : [USIZE_MAX, undefined];
}

/**
 * `ops::Range`: `start..end`, which iterates its integers by moving `start`. A 64- or
 * 128-bit range takes its type's `Step`, `rt.u64Step`, to step past 2^53, and a range
 * of `char`s `rt.charStep`, to step past the surrogates.
 */
export class Range<T> extends Iter<T> {
	start: T;
	end: T;
	readonly int: Step<T> | undefined;

	constructor(start: T, end: T, int?: Step<T>) {
		super();
		this.start = start;
		this.end = end;
		this.int = int;
	}

	next(): T | Done {
		const start = this.start;
		if (!lt(start, this.end)) return DONE;
		this.start = step(start, 1, this.int)!;
		return start;
	}

	override nextBack(): T | Done {
		if (!lt(this.start, this.end)) return DONE;
		this.end = step(this.end, -1, this.int)!;
		return this.end;
	}

	/** std's `nth`, in one step: past the end, the range ends. */
	override nth(n: number): T | Done {
		const item = step(this.start, n, this.int);
		if (item !== undefined && lt(item, this.end)) {
			this.start = step(item, 1, this.int)!;
			return item;
		}
		this.start = this.end;
		return DONE;
	}

	/** std's `advance_by`, in one step. */
	override advanceBy(n: number): number {
		const [len] = this.sizeHint();
		const skip = Math.min(n, len);
		this.start = step(this.start, skip, this.int)!;
		return n - skip;
	}

	/** std's `advance_back_by`, in one step. */
	override advanceBackBy(n: number): number {
		const [len] = this.sizeHint();
		const skip = Math.min(n, len);
		this.end = step(this.end, -skip, this.int)!;
		return n - skip;
	}

	/** std's `nth_back`, in one step. */
	override nthBack(n: number): T | Done {
		const item = step(this.end, -n, this.int);
		if (item !== undefined && lt(this.start, item)) {
			this.end = step(item, -1, this.int)!;
			return this.end;
		}
		this.end = this.start;
		return DONE;
	}

	override len(): number {
		return lt(this.start, this.end) ? Number(this.end) - Number(this.start) : 0;
	}

	override sizeHint(): SizeHint {
		return lt(this.start, this.end) ? stepsBetween(this.start, this.end, this.int) : [0, 0];
	}

	override clone(): Range<T> {
		return new Range(this.start, this.end, this.int);
	}
}

/** `Range::is_empty`. */
export function rangeIsEmpty<T>(r: Range<T>): boolean {
	return !lt(r.start, r.end);
}

/** `Range::contains`: `start <= item < end`, so nothing past a NaN. */
export function rangeContains<T>(r: Range<T>, item: T): boolean {
	return le(r.start, item) && lt(item, r.end);
}

/**
 * `ops::RangeFrom`: `start..`, which iterates by moving `start` and never ends. It takes
 * its type's `Step`, `rt.u8Step` or `rt.charStep`, past whose last value it panics, as
 * std's does in a debug build.
 */
export class RangeFrom<T> extends Iter<T> {
	start: T;
	readonly int: Step<T> | undefined;

	constructor(start: T, int?: Step<T>) {
		super();
		this.start = start;
		this.int = int;
	}

	/** std's: the next `start` is found before the item is given, so the last value panics. */
	next(): T | Done {
		const start = this.start;
		this.start = forward(start, 1, this.int);
		return start;
	}

	override nth(n: number): T | Done {
		const item = forward(this.start, n, this.int);
		this.start = forward(item, 1, this.int);
		return item;
	}

	override sizeHint(): SizeHint {
		return [USIZE_MAX, undefined];
	}

	override clone(): RangeFrom<T> {
		return new RangeFrom(this.start, this.int);
	}
}

/** `RangeFrom::contains`. */
export function rangeFromContains<T>(r: RangeFrom<T>, item: T): boolean {
	return le(r.start, item);
}

/** `RangeTo::contains` and `RangeToInclusive::contains`. */
export function rangeToContains<T>(r: { readonly end: T; readonly inclusive?: true }, item: T): boolean {
	return r.inclusive ? le(item, r.end) : lt(item, r.end);
}

/** `RangeBounds::contains` of a pair of `Bound`s. */
export function boundsContains<T>([start, end]: readonly [RangeBound<T>, RangeBound<T>], item: T): boolean {
	const above = start.tag === "Unbounded" || (start.tag === "Included" ? le(start._0, item) : lt(start._0, item));
	return above && (end.tag === "Unbounded" || (end.tag === "Included" ? le(item, end._0) : lt(item, end._0)));
}

/** std's `Step::forward` in a debug build: `n + by`, panicking past `int`'s last value. */
function forward<T>(n: T, by: number, int: Step<T> | undefined): T {
	if (int === undefined) throw new Error("rust2-ts: stepping a range of no integer type");
	const [sum, over] = int(n, by);
	if (over) panic(int === (charStep as unknown) ? "overflow in `Step::forward`" : "attempt to add with overflow");
	return sum;
}

/** `ops::RangeInclusive`: `start..=end`, which remembers once it has yielded `end`. */
export class RangeInclusive<T> extends Iter<T> {
	lo: T;
	hi: T;
	exhausted = false;
	/** The integer type, which `nth` wraps within as std's does. */
	readonly int: Step<T> | undefined;

	constructor(start: T, end: T, int?: Step<T>) {
		super();
		this.lo = start;
		this.hi = end;
		this.int = int;
	}

	/** std's: `start` steps past the item, so it passes `end`, and only wrapping exhausts it. */
	next(): T | Done {
		if (rangeInclusiveIsEmpty(this)) return DONE;
		const lo = this.lo;
		[this.lo, this.exhausted] = stepOverflowing(lo, 1, this.int);
		return lo;
	}

	override folded(): Iter<T> {
		return new RangeInclusiveFold(this);
	}

	override clone(): RangeInclusive<T> {
		const range = new RangeInclusive(this.lo, this.hi, this.int);
		range.exhausted = this.exhausted;
		return range;
	}

	override nextBack(): T | Done {
		if (rangeInclusiveIsEmpty(this)) return DONE;
		const hi = this.hi;
		[this.hi, this.exhausted] = stepOverflowing(hi, -1, this.int);
		return hi;
	}

	override len(): number {
		return rangeInclusiveIsEmpty(this) ? 0 : Number(this.hi) - Number(this.lo) + 1;
	}

	override sizeHint(): SizeHint {
		if (rangeInclusiveIsEmpty(this)) return [0, 0];
		const [lower, upper] = stepsBetween(this.lo, this.hi, this.int);
		return [satAdd(lower, 1), addUpper(upper, 1)];
	}

	/** std's `nth`: `start` moves past the item, wrapping if it overflows, which ends the range. */
	override nth(n: number): T | Done {
		if (rangeInclusiveIsEmpty(this)) return DONE;
		const [item, over] = overflowing(this.lo, n, this.int);
		const [after, overAfter] = overflowing(item, 1, this.int);
		this.lo = after;
		this.exhausted = over || overAfter;
		return !over && !lt(this.hi, item) ? item : DONE;
	}

	/** std's `nth_back`, from the end. */
	override nthBack(n: number): T | Done {
		if (rangeInclusiveIsEmpty(this)) return DONE;
		const [item, over] = overflowing(this.hi, -n, this.int);
		const [before, overBefore] = overflowing(item, -1, this.int);
		this.hi = before;
		this.exhausted = over || overBefore;
		return !over && !lt(item, this.lo) ? item : DONE;
	}
}

/**
 * An inclusive range as std's `try_fold` steps it, and `try_rfold` from the back: `start`
 * or `end` stops at the other, which exhausts it, so it never wraps.
 */
class RangeInclusiveFold<T> extends Iter<T> {
	readonly range: RangeInclusive<T>;

	constructor(range: RangeInclusive<T>) {
		super();
		this.range = range;
	}

	next(): T | Done {
		const r = this.range;
		if (rangeInclusiveIsEmpty(r)) return DONE;
		const lo = r.lo;
		if (lt(lo, r.hi)) r.lo = step(lo, 1, r.int)!;
		else r.exhausted = true;
		return lo;
	}

	override nextBack(): T | Done {
		const r = this.range;
		if (rangeInclusiveIsEmpty(r)) return DONE;
		const hi = r.hi;
		if (lt(r.lo, hi)) r.hi = step(hi, -1, r.int)!;
		else r.exhausted = true;
		return hi;
	}

	override sizeHint(): SizeHint {
		return this.range.sizeHint();
	}
}

/** `RangeInclusive::start`. */
export function rangeInclusiveStart<T>(r: RangeInclusive<T>): T {
	return r.lo;
}

/** `RangeInclusive::end`. */
export function rangeInclusiveEnd<T>(r: RangeInclusive<T>): T {
	return r.hi;
}

/** `RangeInclusive::into_inner`: its bounds as they are, even exhausted. */
export function rangeInclusiveIntoInner<T>(r: RangeInclusive<T>): [T, T] {
	return [r.lo, r.hi];
}

/** `RangeInclusive::is_empty`: exhausted, or not `start <= end`, as of a NaN. */
export function rangeInclusiveIsEmpty<T>(r: RangeInclusive<T>): boolean {
	return r.exhausted || !le(r.lo, r.hi);
}

/** `RangeInclusive::contains`. */
export function rangeInclusiveContains<T>(r: RangeInclusive<T>, item: T): boolean {
	// Exhausted, it may have wrapped past its end, and contains nothing, as std's bounds say.
	return !r.exhausted && le(r.lo, item) && le(item, r.hi);
}

/** What `==` and `Hash` read of any of std's ranges that holds a bound, whatever steps it. */
type Bounded<T> =
	| { readonly lo: T; readonly hi: T; readonly exhausted: boolean }
	| { readonly start: T; readonly end: T }
	| { readonly start: T }
	| { readonly end: T };

/**
 * `PartialEq::eq` of std's ranges of one kind, from their bounds' `eq`: each bound's, then
 * whether an inclusive one is exhausted, as std's derived one compares its fields.
 */
export function rangeEq<T>(eq: (a: T, b: T) => boolean): (a: Bounded<T>, b: Bounded<T>) => boolean {
	return (a, b) => {
		if ("exhausted" in a) {
			const c = b as typeof a;
			return eq(a.lo, c.lo) && eq(a.hi, c.hi) && a.exhausted === c.exhausted;
		}
		const c = b as { readonly start: T; readonly end: T };
		return (!("start" in a) || eq(a.start, c.start)) && (!("end" in a) || eq(a.end, c.end));
	};
}

/** `Hash::hash` of std's ranges, from their bounds': each bound's, then whether an inclusive one is exhausted. */
export function rangeHash<T>(hash: (value: T, s: Hasher) => void): (value: Bounded<T>, s: Hasher) => void {
	return (r, s) => {
		if ("exhausted" in r) {
			hash(r.lo, s);
			hash(r.hi, s);
			hash8(r.exhausted, s);
			return;
		}
		if ("start" in r) hash(r.start, s);
		if ("end" in r) hash(r.end, s);
	};
}

/** `RangeInclusive::start_bound`: its start, included. */
export function rangeInclusiveStartBound<T>(r: RangeInclusive<T>): RangeBound<T> {
	return { tag: "Included", _0: r.lo };
}

/** `RangeInclusive::end_bound`: its end, included, until wrapping exhausts it and it excludes its start, as std's. */
export function rangeInclusiveEndBound<T>(r: RangeInclusive<T>): RangeBound<T> {
	return r.exhausted ? { tag: "Excluded", _0: r.lo } : { tag: "Included", _0: r.hi };
}

/** `Step::forward_overflowing` and `backward_overflowing`: `n + by`, wrapped within `int`, and whether it wrapped. */
/** `Step::forward_overflowing` by `by`, of an integer type's step or a safe number's. */
function stepOverflowing<T>(n: T, by: number, int: Step<T> | undefined): [T, boolean] {
	if (int !== undefined) return int(n, by);
	return [step(n, by, int)!, false];
}

function overflowing<T>(n: T, by: number, int: Step<T> | undefined): [T, boolean] {
	if (int === undefined) throw new Error("rust2-ts: stepping a range of no integer type");
	return int(n, by);
}

/** An iterator as itself: what `into_iter` makes of one. */
function itself<T>(iter: Iter<T>): Iter<T> {
	return iter;
}

/** A value as itself: what `cloned` makes of a JS value. */
function same<T>(value: T): T {
	return value;
}
