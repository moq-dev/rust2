// Collections, for rust2-ts output: std's `HashMap`, `HashSet`, `BTreeMap`, `BTreeSet`,
// and `VecDeque`, with `Ordering`, which their keys need. `rt.ts`
// re-exports all of it. Each collection is one type whose functions are free functions
// here, so a bundler keeps only those a caller reaches.
//
// A key compares as Rust's `Eq` or `Ord` says, never by JS identity: a hash collection
// finds a key that is a JS object through its translated `Hash` and `Eq`, and a B-tree
// orders every key by its translated `Ord`. Which of those calls the runtime makes, and
// how often, are its own, not std's.
//
// No key, value, or item is ever `undefined`, which the translator refuses, so
// `undefined` is a missing one.
//
// A function that may drop a key, value, or item std's would, rather than return or keep
// it, takes what drops each that does, as `k`, `v`, or `f`, and drops it where std's does.

import { ByteSlice, ByteVec, byteVecPush, type None, type Place, panic, type RangeBound, rangeCheck, sliceSort } from "../rt.ts";
import type { Hashing } from "./hash.ts";
import { type Limbs, lt } from "./int.ts";
import {
	type Clones,
	DONE,
	type Done,
	exactHint,
	Iter,
	iterChain,
	iterEmpty,
	iterFilter,
	iterMap,
	iterOnce,
	iterSlice,
	type RangeInclusive,
	rangeInclusiveEndBound,
	rangeInclusiveStartBound,
	type SizeHint,
	type Take,
	dequeReserve,
	vecCapacity,
	vecGrow,
	vecPush,
	vecReserve,
	vecReserveExact,
} from "./iter.ts";
import { type Arc, drop as dispose, dropAll, dropRange } from "./task.ts";

// Ordering

/** `core::cmp::Ordering`: its discriminant, as `as i8` reads it. */
export type Ordering = -1 | 0 | 1;

/** `Ord::cmp` of two integers of one type, or of two `bool`s. */
export function cmp<T extends number | boolean | Limbs>(a: T, b: T): Ordering {
	if (typeof a !== "object" && typeof b !== "object") return a < b ? -1 : a > b ? 1 : 0;
	const [x, y] = [a as number | Limbs, b as number | Limbs];
	return lt(x, y) ? -1 : lt(y, x) ? 1 : 0;
}

/**
 * `Ord::cmp` of two strings, by code point as Rust's UTF-8 bytes sort. JS's `<` compares
 * UTF-16 units, which puts a character past U+FFFF, a surrogate pair, below U+E000.
 */
export function cmpStr(a: string, b: string): Ordering {
	if (a === b) return 0;
	const n = Math.min(a.length, b.length);
	for (let i = 0; i < n; i++) {
		let x = a.charCodeAt(i);
		let y = b.charCodeAt(i);
		if (x === y) continue;
		if (x >= 0xd800 && y >= 0xd800) {
			x = x >= 0xe000 ? x - 0x800 : x + 0x2000;
			y = y >= 0xe000 ? y - 0x800 : y + 0x2000;
		}
		return x < y ? -1 : 1;
	}
	return a.length < b.length ? -1 : 1;
}

/** `Ord::cmp` of what holds nothing, as `()` and `PhantomData`: always `Equal`. */
export function cmpNothing(_a: undefined, _b: undefined): Ordering {
	return 0;
}

/**
 * `Ord::cmp` of tuples of type `T`, or `PartialOrd::partial_cmp`, from each field's: the
 * first that is not `Equal`, as `None` is not. A caller passes `T` and `O`, which `tsc`
 * infers from neither a generic field, such as a slice's of `rt.cmp`, nor only total ones.
 */
export function cmpTuple<T extends readonly unknown[], O extends Ordering | undefined>(fields: { readonly [I in keyof T]: (a: T[I], b: T[I]) => O }): (a: T, b: T) => O {
	return (a, b) => {
		for (let i = 0; i < fields.length; i++) {
			const c = (fields[i] as (a: unknown, b: unknown) => O)(a[i], b[i]);
			if (c !== 0) return c;
		}
		return 0 as O;
	};
}

/**
 * `PartialOrd::lt` of tuples, or `le`, `gt`, or `ge`: `test` of their last fields, unless
 * `cmps`, the `partial_cmp`s of the others, find one not `Equal` first, which decides
 * whether it is `order`, as `None` is not.
 */
export function testTuple<T extends readonly unknown[]>(
	cmps: readonly ((a: never, b: never) => Ordering | undefined)[],
	test: (a: never, b: never) => boolean,
	order: Ordering,
): (a: T, b: T) => boolean {
	return (a, b) => {
		for (let i = 0; i < cmps.length; i++) {
			const c = (cmps[i] as (a: unknown, b: unknown) => Ordering | undefined)(a[i], b[i]);
			if (c !== 0) return c === order;
		}
		return (test as (a: unknown, b: unknown) => boolean)(a[cmps.length], b[cmps.length]);
	};
}

/** `PartialOrd::lt` by `cmp`, a `partial_cmp`, as the trait's default tests it: whether it is `Less`. */
export function cmpLt<T>(cmp: (a: T, b: T) => Ordering | undefined): (a: T, b: T) => boolean {
	return (a, b) => cmp(a, b) === -1;
}

/** `PartialOrd::le` by `cmp`, a `partial_cmp`: whether it is `Less` or `Equal`. */
export function cmpLe<T>(cmp: (a: T, b: T) => Ordering | undefined): (a: T, b: T) => boolean {
	return (a, b) => {
		const c = cmp(a, b);
		return c !== undefined && c <= 0;
	};
}

/** `PartialOrd::gt` by `cmp`, a `partial_cmp`: whether it is `Greater`. */
export function cmpGt<T>(cmp: (a: T, b: T) => Ordering | undefined): (a: T, b: T) => boolean {
	return (a, b) => cmp(a, b) === 1;
}

/** `PartialOrd::ge` by `cmp`, a `partial_cmp`: whether it is `Greater` or `Equal`. */
export function cmpGe<T>(cmp: (a: T, b: T) => Ordering | undefined): (a: T, b: T) => boolean {
	return (a, b) => {
		const c = cmp(a, b);
		return c !== undefined && c >= 0;
	};
}

/** `Ord::cmp` of `Option`s, or `PartialOrd::partial_cmp`, from their values': `None` first. */
export function cmpOption<T, O extends Ordering | undefined>(cmp: (a: T, b: T) => O): (a: T | undefined, b: T | undefined) => O;
export function cmpOption<T, O extends Ordering | undefined>(cmp: (a: T, b: T) => O, none: None): (a: T | None, b: T | None) => O;
export function cmpOption<T, O extends Ordering | undefined>(
	cmp: (a: T, b: T) => O,
	none?: None,
): (a: T | undefined | None, b: T | undefined | None) => O {
	return (a, b) => (a === none ? (b === none ? 0 : -1) : b === none ? 1 : cmp(a as T, b as T)) as O;
}

/**
 * `Ord::cmp` of `Reverse`s, or `PartialOrd::partial_cmp` or one of its tests: their
 * values', the other way.
 */
export function cmpReverse<T, O>(cmp: (a: T, b: T) => O): (a: T, b: T) => O {
	return (a, b) => cmp(b, a);
}

/**
 * `Ord::cmp` of slices, or `PartialOrd::partial_cmp`, by their elements' in order, then
 * by length.
 */
export function cmpSlice<T, O extends Ordering | undefined>(cmp: (a: T, b: T) => O): (a: ArrayLike<T>, b: ArrayLike<T>) => O {
	return (a, b) => {
		const n = Math.min(a.length, b.length);
		for (let i = 0; i < n; i++) {
			const c = cmp(a[i], b[i]);
			if (c !== 0) return c;
		}
		return (a.length < b.length ? -1 : a.length > b.length ? 1 : 0) as O;
	};
}

/** `cmpSlice`'s of `VecDeque`s, by their items in order however each one's buffer wraps. */
export function cmpDeque<T, O extends Ordering | undefined>(cmp: (a: T, b: T) => O): (a: VecDeque<T>, b: VecDeque<T>) => O {
	return (a, b) => {
		const n = Math.min(a.n, b.n);
		for (let i = 0; i < n; i++) {
			const c = cmp(a.b[at(a, i)] as T, b.b[at(b, i)] as T);
			if (c !== 0) return c;
		}
		return (a.n < b.n ? -1 : a.n > b.n ? 1 : 0) as O;
	};
}

/**
 * `Ord::cmp` of byte arrays, slices, or vectors, or `PartialOrd::partial_cmp`: by their
 * bytes in order, then by length. Each is a `[u8; N]`, a `&[u8]` or `&[u8; N]`, or a
 * `Vec<u8>`, whichever a key or an operand holds.
 */
export function cmpBytes(a: Uint8Array | ByteSlice | ByteVec, b: Uint8Array | ByteSlice | ByteVec): Ordering {
	const x = a instanceof Uint8Array ? a : a.data;
	const y = b instanceof Uint8Array ? b : b.data;
	const i = a instanceof ByteSlice ? a.start : 0;
	const j = b instanceof ByteSlice ? b.start : 0;
	const n = Math.min(a.length, b.length);
	for (let k = 0; k < n; k++) {
		if (x[i + k] !== y[j + k]) return x[i + k] < y[j + k] ? -1 : 1;
	}
	return a.length < b.length ? -1 : a.length > b.length ? 1 : 0;
}

/**
 * `PartialEq::eq` of byte arrays, slices, or vectors: their lengths', then their bytes in
 * order, of whichever form each is, as `cmpBytes` reads them.
 */
export function eqBytes(a: Uint8Array | ByteSlice | ByteVec, b: Uint8Array | ByteSlice | ByteVec): boolean {
	if (a.length !== b.length) return false;
	const x = a instanceof Uint8Array ? a : a.data;
	const y = b instanceof Uint8Array ? b : b.data;
	const i = a instanceof ByteSlice ? a.start : 0;
	const j = b instanceof ByteSlice ? b.start : 0;
	for (let k = 0; k < a.length; k++) if (x[i + k] !== y[j + k]) return false;
	return true;
}

/** `Ord::cmp` of `Arc`s, or `PartialOrd::partial_cmp` or one of its tests: their values'. */
export function cmpArc<T, O>(cmp: (a: T, b: T) => O): (a: Arc<T>, b: Arc<T>) => O {
	return (a, b) => cmp(a.value, b.value);
}

/**
 * `Ord::clamp`, by `PartialOrd`'s `le`, `lt`, and `gt`: `min` if `a` is less, `max` if
 * greater, or else `a`, once `min <= max` is asserted. The operands it does not return
 * drop, the last first, as std's do.
 */
export function ordClamp<T>(
	a: T,
	min: T,
	max: T,
	le: (a: T, b: T) => boolean,
	lt: (a: T, b: T) => boolean,
	gt: (a: T, b: T) => boolean,
	drop?: (value: T) => void,
): T {
	if (!le(min, max)) panic("assertion failed: min <= max");
	const out = lt(a, min) ? 1 : gt(a, max) ? 2 : 0;
	if (drop !== undefined) dropAll([max, min, a].filter((_, i) => 2 - i !== out), drop);
	return out === 1 ? min : out === 2 ? max : a;
}

/** `Ord::max`, by `lt`: `a` if `b < a`, or else `b`, and the other drops. */
export function ordMax<T>(a: T, b: T, lt: (a: T, b: T) => boolean, drop?: (value: T) => void): T {
	if (lt(b, a)) {
		drop?.(b);
		return a;
	}
	drop?.(a);
	return b;
}

/** `Ord::min`, by `lt`: `b` if `b < a`, or else `a`, and the other drops. */
export function ordMin<T>(a: T, b: T, lt: (a: T, b: T) => boolean, drop?: (value: T) => void): T {
	if (lt(b, a)) {
		drop?.(a);
		return b;
	}
	drop?.(b);
	return a;
}

/** `Ordering::reverse`. */
export function orderingReverse(o: Ordering): Ordering {
	return o === 0 ? 0 : o === 1 ? -1 : 1;
}

/** `Ordering::then`, and with `lazy`, `then_with`, whose `other` is a function. */
export function orderingThen(o: Ordering, other: Ordering | (() => Ordering), lazy = false): Ordering {
	if (o !== 0) return o;
	return lazy ? (other as () => Ordering)() : (other as Ordering);
}

// Equality

/** `PartialEq::eq` of two values JS compares as Rust does: numbers, strings, booleans. */
export function same<T>(a: T, b: T): boolean {
	return a === b;
}

/** `PartialEq::eq` of tuples, from each field's. */
export function eqTuple<T extends readonly unknown[]>(fields: readonly ((a: never, b: never) => boolean)[]): (a: T, b: T) => boolean {
	return (a, b) => {
		for (let i = 0; i < fields.length; i++) if (!(fields[i] as (a: unknown, b: unknown) => boolean)(a[i], b[i])) return false;
		return true;
	};
}

/** `PartialEq::eq` of slices: their lengths', then their elements' in order. */
export function eqSlice<T>(eq: (a: T, b: T) => boolean): (a: ArrayLike<T>, b: ArrayLike<T>) => boolean {
	return (a, b) => {
		if (a.length !== b.length) return false;
		for (let i = 0; i < a.length; i++) if (!eq(a[i], b[i])) return false;
		return true;
	};
}

/** A slice, an array, or a `Vec`, of bytes in any form, or a `VecDeque`. */
type Seq<T> = ArrayLike<T> | ByteSlice | ByteVec | VecDeque<T>;

/** The length of `s` and what reads its element at an index. */
function seqItems<T>(s: Seq<T>): [number, (i: number) => T] {
	if (s instanceof ByteSlice) return [s.length, (i) => s.data[s.start + i] as T];
	if (s instanceof ByteVec) return [s.length, (i) => s.data[i] as T];
	if (!("length" in s)) return [s.n, (i) => s.b[at(s, i)] as T];
	return [s.length, (i) => s[i] as T];
}

/**
 * `PartialEq::eq` of sequences of any form against each other, as std's of slices, arrays,
 * `Vec`s, and `VecDeque`s are: their lengths, then each pair in order by `eq`.
 */
export function eqSeq<T, U>(eq: (a: T, b: U) => boolean): (a: Seq<T>, b: Seq<U>) => boolean {
	return (a, b) => {
		const [n, x] = seqItems(a);
		const [m, y] = seqItems(b);
		if (n !== m) return false;
		for (let i = 0; i < n; i++) if (!eq(x(i), y(i))) return false;
		return true;
	};
}

/** `PartialEq::eq` of `Option`s, from their values'. */
export function eqOption<T>(eq: (a: T, b: T) => boolean): (a: T | undefined, b: T | undefined) => boolean;
export function eqOption<T>(eq: (a: T, b: T) => boolean, none: None): (a: T | None, b: T | None) => boolean;
export function eqOption<T>(
	eq: (a: T, b: T) => boolean,
	none?: None,
): (a: T | undefined | None, b: T | undefined | None) => boolean {
	return (a, b) => (a === none || b === none ? a === b : eq(a as T, b as T));
}

/**
 * `PartialEq::eq` of `Arc`s: their values', after, with `ident`, whether they are one
 * `Arc`, which std checks first of a value that is `Eq`.
 */
export function eqArc<T>(eq: (a: T, b: T) => boolean, ident = false): (a: Arc<T>, b: Arc<T>) => boolean {
	return (a, b) => (ident && a === b) || eq(a.value, b.value);
}

/** A value as its own clone. */
function itself<T>(value: T): T {
	return value;
}

/** The place of index `i` of `a`. */
export class AtIndex<T> implements Place<T> {
	private readonly a: { [i: number]: T };
	private readonly i: number;

	constructor(a: { [i: number]: T }, i: number) {
		this.a = a;
		this.i = i;
	}

	get v(): T {
		return this.a[this.i];
	}

	set v(value: T) {
		this.a[this.i] = value;
	}
}

/** The place of `k`'s value in `m`, which holds it. */
class AtKey<K, V> implements Place<V> {
	private readonly m: Map<K, V>;
	private readonly k: K;

	constructor(m: Map<K, V>, k: K) {
		this.m = m;
		this.k = k;
	}

	get v(): V {
		return this.m.get(this.k) as V;
	}

	set v(value: V) {
		this.m.set(this.k, value);
	}
}

// HashMap and HashSet

/** An entry whose key is a JS object, in the chain of its hash. */
interface Slot<K, V> {
	k: K;
	v: V;
	next: Slot<K, V> | undefined;
}

/**
 * `HashMap<K, V>`, and `HashSet<K>` as a map to `undefined`. A key that is a JS primitive,
 * a number, string, or boolean, is its own JS map key, as Rust compares it; a key that
 * is an object goes by its hash, to a chain of the keys `Eq` tells apart. Iteration
 * visits the first kind in insertion order and then the second by hash, an order Rust
 * leaves unspecified.
 */
export interface HashMap<K, V> {
	p: Map<K, V>;
	o: Map<number, Slot<K, V>> | undefined;
	/** How many keys `o` holds. */
	n: number;
	/** How to find a key that is an object, or `undefined` if no key is one. */
	readonly h: Hashing<K> | undefined;
}

/** `HashSet<K>`. */
export type HashSet<K> = HashMap<K, undefined>;

/**
 * `HashMap::new` and `HashSet::new`, with how to find a key that is an object. The type
 * `K` is asserted, not inferred from `h`: a slice's hashing takes any `ArrayLike`.
 */
export function hashNew<K, V>(h?: Hashing<NoInfer<K>>): HashMap<K, V> {
	return { p: new Map(), o: undefined, n: 0, h };
}

/** Whether `key` goes by its hash rather than as its own JS map key. */
function hashed<K, V>(m: HashMap<K, V>, key: K): boolean {
	return m.h !== undefined && (m.h.all || typeof key === "object");
}

/** The slot holding a key equal to `key`, a hashed key whose hash is `hash` if given. */
function slot<K, V>(m: HashMap<K, V>, key: K, hash?: number): Slot<K, V> | undefined {
	const h = m.h as Hashing<K>;
	for (let s = m.o?.get(hash ?? h.hash(key)); s !== undefined; s = s.next) if (h.eq(s.k, key)) return s;
	return undefined;
}

export function hashLen<K, V>(m: HashMap<K, V>): number {
	return m.p.size + m.n;
}

/** `HashMap::get`: the value there, or else `none`, the `None` of an `Option` of a value that can be `undefined`. */
export function hashGet<K, V>(m: HashMap<K, V>, key: K): V | undefined;
export function hashGet<K, V>(m: HashMap<K, V>, key: K, none: None): V | None;
export function hashGet<K, V>(m: HashMap<K, V>, key: K, none?: None): V | None | undefined {
	if (hashed(m, key)) {
		const s = slot(m, key);
		return s === undefined ? none : s.v;
	}
	const v = m.p.get(key);
	return v !== undefined || none === undefined || m.p.has(key) ? v : none;
}

/** `HashMap::get_key_value`: the equal key the map holds, and its value. */
export function hashGetKeyValue<K, V>(m: HashMap<K, V>, key: K): [K, V] | undefined {
	if (hashed(m, key)) {
		const s = slot(m, key);
		return s === undefined ? undefined : [s.k, s.v];
	}
	return m.p.has(key) ? [key, m.p.get(key) as V] : undefined;
}

/** `HashMap::get_mut` of a value JS copies: its place, if the map holds `key`. */
export function hashGetMut<K, V>(m: HashMap<K, V>, key: K): Place<V> | undefined {
	if (hashed(m, key)) return slot(m, key);
	return m.p.has(key) ? new AtKey(m.p, key) : undefined;
}

export function hashContains<K, V>(m: HashMap<K, V>, key: K): boolean {
	return hashed(m, key) ? slot(m, key) !== undefined : m.p.has(key);
}

/** `HashMap::index`: `map[&key]`. */
export function hashIndex<K, V>(m: HashMap<K, V>, key: K): V {
	if (!hashContains(m, key)) panic("no entry found for key");
	return hashGet(m, key) as V;
}

/** Drop `key` by `k` and then `value` by `v`. */
function dropPair<K, V>(key: K, value: V, k?: (key: K) => void, v?: (value: V) => void): void {
	k?.(key);
	v?.(value);
}

/** Drop each key by `k` and its value by `v`, in order. */
function dropPairs<K, V>(keys: readonly K[], values: readonly V[], k?: (key: K) => void, v?: (value: V) => void): void {
	for (let i = 0; i < keys.length; i++) dropPair(keys[i], values[i], k, v);
}

/** `HashMap::insert`: the old value, or else `none`. An equal key already there stays, and `key` drops. */
export function hashInsert<K, V>(m: HashMap<K, V>, key: K, value: V, k?: (key: K) => void): V | undefined;
export function hashInsert<K, V>(m: HashMap<K, V>, key: K, value: V, k: ((key: K) => void) | undefined, none: None): V | None;
export function hashInsert<K, V>(m: HashMap<K, V>, key: K, value: V, k?: (key: K) => void, none?: None): V | None | undefined {
	if (!hashed(m, key)) {
		const had = m.p.has(key);
		const old = m.p.get(key);
		m.p.set(key, value);
		if (!had) return none;
		k?.(key);
		return old;
	}
	// Hashed once, as std's is, into an empty map too.
	const hash = (m.h as Hashing<K>).hash(key);
	const found = slot(m, key, hash);
	if (found !== undefined) {
		const old = found.v;
		found.v = value;
		k?.(key);
		return old;
	}
	add(m, key, value, hash);
	return none;
}

/** Add a hashed key that is not there yet, whose hash is `hash`: its new slot. */
function add<K, V>(m: HashMap<K, V>, key: K, value: V, hash = (m.h as Hashing<K>).hash(key)): Slot<K, V> {
	m.o ??= new Map();
	const s = { k: key, v: value, next: m.o.get(hash) };
	m.o.set(hash, s);
	m.n++;
	return s;
}

/** `HashSet::replace`: the equal key the set held, which `key` takes the place of, or else `none`, with `key` added. */
export function hashReplace<K>(m: HashSet<K>, key: K): K | undefined;
export function hashReplace<K>(m: HashSet<K>, key: K, none: None): K | None;
export function hashReplace<K>(m: HashSet<K>, key: K, none?: None): K | None | undefined {
	if (!hashed(m, key)) {
		if (m.p.has(key)) return key;
		m.p.set(key, undefined);
		return none;
	}
	const hash = (m.h as Hashing<K>).hash(key);
	const s = slot(m, key, hash);
	if (s === undefined) {
		add(m, key, undefined, hash);
		return none;
	}
	const old = s.k;
	s.k = key;
	return old;
}

/** `HashSet::insert`: whether `key` was not there yet. An equal key already there stays, and `key` drops. */
export function hashAdd<K>(m: HashSet<K>, key: K, k?: (key: K) => void): boolean {
	if (!hashed(m, key)) {
		if (m.p.has(key)) {
			k?.(key);
			return false;
		}
		m.p.set(key, undefined);
		return true;
	}
	const hash = (m.h as Hashing<K>).hash(key);
	if (slot(m, key, hash) !== undefined) {
		k?.(key);
		return false;
	}
	add(m, key, undefined, hash);
	return true;
}

/** `HashMap::remove_entry`: the key and value there, if any. */
export function hashRemoveEntry<K, V>(m: HashMap<K, V>, key: K): [K, V] | undefined {
	if (!hashed(m, key)) {
		if (!m.p.has(key)) return undefined;
		const value = m.p.get(key) as V;
		m.p.delete(key);
		return [key, value];
	}
	const hash = (m.h as Hashing<K>).hash(key);
	const s = slot(m, key, hash);
	if (s === undefined) return undefined;
	unlink(m, hash, s);
	return [s.k, s.v];
}

/** Take `s`, a slot whose key's hash is `hash`, out of `m`. */
function unlink<K, V>(m: HashMap<K, V>, hash: number, s: Slot<K, V>): void {
	const o = m.o as Map<number, Slot<K, V>>;
	let prev = o.get(hash) as Slot<K, V>;
	if (prev === s) {
		if (s.next !== undefined) o.set(hash, s.next);
		else o.delete(hash);
	} else {
		while (prev.next !== s) prev = prev.next as Slot<K, V>;
		prev.next = s.next;
	}
	m.n--;
}

/** `HashMap::remove`: the value there, whose key drops, or else `none`. */
export function hashRemove<K, V>(m: HashMap<K, V>, key: K, k?: (key: K) => void): V | undefined;
export function hashRemove<K, V>(m: HashMap<K, V>, key: K, k: ((key: K) => void) | undefined, none: None): V | None;
export function hashRemove<K, V>(m: HashMap<K, V>, key: K, k?: (key: K) => void, none?: None): V | None | undefined {
	if (!hashed(m, key)) {
		if (!m.p.has(key)) return none;
		const old = m.p.get(key);
		m.p.delete(key);
		k?.(key);
		return old;
	}
	const entry = hashRemoveEntry(m, key);
	if (entry === undefined) return none;
	k?.(entry[0]);
	return entry[1];
}

/** `HashSet::remove`: whether `key` was there. The key it held drops. */
export function hashDelete<K>(m: HashSet<K>, key: K, k?: (key: K) => void): boolean {
	if (!hashed(m, key)) {
		if (!m.p.delete(key)) return false;
		k?.(key);
		return true;
	}
	const entry = hashRemoveEntry(m, key);
	if (entry !== undefined) k?.(entry[0]);
	return entry !== undefined;
}

/** `HashSet::get`, and `HashSet::take` if `take`: the equal key the set holds, or else `none`. */
export function hashKey<K>(m: HashSet<K>, key: K, take?: boolean): K | undefined;
export function hashKey<K>(m: HashSet<K>, key: K, take: boolean, none: None): K | None;
export function hashKey<K>(m: HashSet<K>, key: K, take = false, none?: None): K | None | undefined {
	if (hashed(m, key) && take) {
		const entry = hashRemoveEntry(m, key);
		return entry === undefined ? none : entry[0];
	}
	if (hashed(m, key)) {
		const s = slot(m, key);
		return s === undefined ? none : s.k;
	}
	if (!m.p.has(key)) return none;
	if (take) m.p.delete(key);
	return key;
}

/** Drop `m`: each key by `k` and each value by `v`, of those that drop, entry by entry. */
export function hashDrop<K, V>(m: HashMap<K, V>, k: ((key: K) => void) | undefined, v?: (value: V) => void): void {
	for (const [key, value] of m.p) dropPair(key, value, k, v);
	for (const first of m.o?.values() ?? []) {
		for (let s: Slot<K, V> | undefined = first; s !== undefined; s = s.next) dropPair(s.k, s.v, k, v);
	}
}

/** `clear`, which drops each entry and leaves the map empty. */
export function hashClear<K, V>(m: HashMap<K, V>, k?: (key: K) => void, v?: (value: V) => void): void {
	if (k !== undefined || v !== undefined) hashDrop(m, k, v);
	m.p.clear();
	m.o = undefined;
	m.n = 0;
}

/** `retain`, which keeps each entry `f` returns true for. Each entry goes, and drops, as `f` rejects it. */
export function hashRetain<K, V>(m: HashMap<K, V>, f: (key: K, value: V) => boolean, k?: (key: K) => void, v?: (value: V) => void): void {
	retainHash(m, f as (key: K, value: V | Place<V>) => boolean, false, k, v);
}

/** `retain` of values JS copies: `f` takes the place of each. */
export function hashRetainMut<K, V>(m: HashMap<K, V>, f: (key: K, value: Place<V>) => boolean, k?: (key: K) => void, v?: (value: V) => void): void {
	retainHash(m, f as (key: K, value: V | Place<V>) => boolean, true, k, v);
}

/** `retain`, handing `f` each value or, if `lend`, its place. */
function retainHash<K, V>(m: HashMap<K, V>, f: (key: K, value: V | Place<V>) => boolean, lend: boolean, k?: (key: K) => void, v?: (value: V) => void): void {
	for (const [key, value] of m.p) {
		if (f(key, lend ? new AtKey(m.p, key) : value)) continue;
		const now = lend ? (m.p.get(key) as V) : value;
		m.p.delete(key);
		dropPair(key, now, k, v);
	}
	if (m.o === undefined) return;
	for (const [hash, first] of m.o) {
		let prev: Slot<K, V> | undefined;
		for (let s: Slot<K, V> | undefined = first; s !== undefined; s = s.next) {
			if (f(s.k, lend ? s : s.v)) {
				prev = s;
				continue;
			}
			m.n--;
			if (prev !== undefined) prev.next = s.next;
			else if (s.next !== undefined) m.o.set(hash, s.next);
			else m.o.delete(hash);
			dropPair(s.k, s.v, k, v);
		}
	}
}

/**
 * What a collection's iterator yields: its entries as `[key, value]`, keys, values,
 * `[key, place]`, the place of a value JS copies, or that place alone.
 */
type Pick = 0 | 1 | 2 | 3 | 4;
const ENTRIES = 0;
const KEYS = 1;
const VALUES = 2;
const PLACES = 3;
const VALUE_PLACES = 4;

/** What `pick` yields by value: each place its value. */
function byValue(pick: Pick): Pick {
	return pick === PLACES ? ENTRIES : pick === VALUE_PLACES ? VALUES : pick;
}

/**
 * An iterator of a hash collection: `iter`, `keys`, `values`, `drain`, and the rest.
 * One that owns its entries is given what drops each key and value of those that drop,
 * and drops each entry it still holds as it drops.
 */
export class HashIter<K, V, T> extends Iter<T> implements Disposable {
	private readonly h: HashMap<K, V>;
	private readonly m: Map<K, V>;
	private readonly p: Iterator<[K, V]>;
	private readonly o: Iterator<Slot<K, V>> | undefined;
	private at: Slot<K, V> | undefined;
	private readonly pick: Pick;
	left: number;
	private readonly k: ((key: K) => void) | undefined;
	private readonly v: ((value: V) => void) | undefined;
	/** Whether it is a `drain`, whose `count` reads each entry, as an `into_iter`'s does not. */
	readonly drain: boolean;

	constructor(m: HashMap<K, V>, pick: Pick, k?: (key: K) => void, v?: (value: V) => void, drain = false) {
		super();
		this.h = m;
		this.m = m.p;
		this.p = m.p.entries();
		this.o = m.o?.values();
		this.pick = pick;
		this.left = hashLen(m);
		this.k = k;
		this.v = v;
		this.drain = drain;
	}

	/** The next entry, of the `left` it has. */
	private entry(): [K, V] {
		const entry = this.p.next();
		if (!entry.done) return entry.value;
		const s = this.slot();
		return [s.k, s.v];
	}

	/** The next entry of the `left` it has, as its key and the place of its value. */
	private place(): [K, Place<V>] {
		const entry = this.p.next();
		if (!entry.done) return [entry.value[0], new AtKey(this.m, entry.value[0])];
		const s = this.slot();
		return [s.k, s];
	}

	/** The next chained entry, once the primitive keys are done. */
	private slot(): Slot<K, V> {
		if (this.at === undefined) this.at = (this.o as Iterator<Slot<K, V>>).next().value;
		const s = this.at as Slot<K, V>;
		this.at = s.next;
		return s;
	}

	/**
	 * The next item, `into_keys`' and `into_values`' with the other half dropped before it
	 * is returned, as std's `next` drops it.
	 */
	next(): T | Done {
		if (this.left === 0) return DONE;
		this.left--;
		if (this.pick === PLACES) return this.place() as T;
		if (this.pick === VALUE_PLACES) return this.place()[1] as T;
		const entry = this.entry();
		if (this.pick === ENTRIES) return entry as T;
		if (this.pick === KEYS) {
			this.v?.(entry[1]);
			return entry[0] as unknown as T;
		}
		this.k?.(entry[0]);
		return entry[1] as unknown as T;
	}

	/** std's `fold` of `into_keys` and `into_values`: the other half drops after `f` has its item. */
	override fold<A>(acc: A, f: (acc: A, item: T) => A, release?: () => void): A {
		if (this.pick !== KEYS && this.pick !== VALUES) return super.fold(acc, f, release);
		const keys = this.pick === KEYS;
		while (this.left > 0) {
			this.left--;
			const [key, value] = this.entry();
			if (keys) {
				acc = f(acc, key as unknown as T);
				this.v?.(value);
			} else {
				acc = f(acc, value as unknown as T);
				this.k?.(key);
			}
		}
		release?.();
		return acc;
	}

	override remaining(): Iterable<unknown> {
		return this.from(byValue(this.pick));
	}

	/** std's, of an iterator that borrows the map: where it is, independently of it. */
	override clone(): Iter<T> {
		return this.from(this.pick);
	}

	/**
	 * A fresh iterator of the same map yielding `pick`, past what this one took: the map is
	 * unchanged while it is held, and JS can't copy where its own iterator is.
	 */
	private from<U>(pick: Pick): HashIter<K, V, U> {
		const it = new HashIter<K, V, U>(this.h, pick);
		while (it.left > this.left) it.next();
		return it;
	}

	override len(): number {
		return this.left;
	}

	override sizeHint(): SizeHint {
		return exactHint(this.len());
	}

	[Symbol.dispose](): void {
		if (this.k === undefined && this.v === undefined) return;
		const n = this.left;
		this.left = 0;
		for (let i = 0; i < n; i++) {
			const [key, value] = this.entry();
			dropPair(key, value, this.k, this.v);
		}
	}
}

/**
 * `iter` and `iter_mut` of a map, and with what drops each key and value of those that
 * drop, `into_iter`: its entries, as `[key, value]`.
 */
export function hashIter<K, V>(m: HashMap<K, V>, k?: (key: K) => void, v?: (value: V) => void): Iter<[K, V]> {
	return new HashIter<K, V, [K, V]>(m, ENTRIES, k, v);
}

/** `keys` of a map and `iter` of a set, and with what drops each key and value, `into_keys` and `into_iter`. */
export function hashKeys<K, V>(m: HashMap<K, V>, k?: (key: K) => void, v?: (value: V) => void): Iter<K> {
	return new HashIter<K, V, K>(m, KEYS, k, v);
}

/** `values` and `values_mut`, and with what drops each value and key, `into_values`. */
export function hashValues<K, V>(m: HashMap<K, V>, v?: (value: V) => void, k?: (key: K) => void): Iter<V> {
	return new HashIter<K, V, V>(m, VALUES, k, v);
}

/** `iter_mut` of values JS copies: each key and the place of its value, in `hashIter`'s order. */
export function hashIterMut<K, V>(m: HashMap<K, V>): Iter<[K, Place<V>]> {
	return new HashIter<K, V, [K, Place<V>]>(m, PLACES);
}

/** `values_mut` of values JS copies: the place of each. */
export function hashValuesMut<K, V>(m: HashMap<K, V>): Iter<Place<V>> {
	return new HashIter<K, V, Place<V>>(m, VALUE_PLACES);
}

/** `drain` of a map: every entry, taken out at once, which drops as `hashIter`'s. */
export function hashDrain<K, V>(m: HashMap<K, V>, k?: (key: K) => void, v?: (value: V) => void): Iter<[K, V]> {
	return new HashIter<K, V, [K, V]>(taken(m), ENTRIES, k, v, true);
}

/** `drain` of a set: every key, taken out at once. */
export function hashDrainKeys<K, V>(m: HashMap<K, V>, k?: (key: K) => void): Iter<K> {
	return new HashIter<K, V, K>(taken(m), KEYS, k, undefined, true);
}

/** What `m` held, which it no longer does. */
function taken<K, V>(m: HashMap<K, V>): HashMap<K, V> {
	const out: HashMap<K, V> = { p: m.p, o: m.o, n: m.n, h: m.h };
	m.p = new Map();
	hashClear(m);
	return out;
}

/** Extend `c` by `add` of each of `items`, taken as `take` says, as std's `extend` takes the iterator. */
function extending<C, T>(c: C, items: Iter<T>, add: (item: T) => void, take?: Take): C {
	if (take === "fold") items.fold<void>(undefined, (_, item) => add(item));
	else {
		// By `next` where std's reads by it, and else as its `for_each` folds.
		const read = take === "next" ? items : items.folded(true);
		for (let item = read.next(); item !== DONE; item = read.next()) add(item);
	}
	if (take === "next") dispose(items);
	return c;
}

/**
 * `extending` a `Vec` or `VecDeque` holding `len`, which with elements `size` bytes each
 * reserves as `iterExtend` does: a `trusted` iterator up front, and any other by `next`,
 * a `deque`'s at its first item too.
 */
function vecExtending<C, T>(c: C, len: number, items: Iter<T>, add: (item: T) => void, take?: Take, size?: number, trusted = false, deque = false): C {
	if (size === undefined || trusted) {
		if (size !== undefined) vecReserveExact(items, len, size);
		return extending(c, items, add, take);
	}
	let cap = vecCapacity(len, size);
	for (let item = items.next(); item !== DONE; item = items.next()) {
		if (deque) dequeReserve(items, len);
		deque = false;
		if (len === cap) cap = vecGrow(items, cap, size);
		add(item);
		len++;
	}
	if (take !== undefined) dispose(items);
	return c;
}

/**
 * hashbrown's `extend_reserve` of `m` for `additional` more, as its `extend` makes for an
 * iterator's lower bound: all of it, or half rounded up if `m` holds any, which panics past
 * `max` entries, the most its table holds.
 */
function hashReserve<K, V>(m: HashMap<K, V>, additional: number, max: number | undefined): void {
	if (max === undefined) return;
	const n = hashLen(m);
	if (n + (n === 0 ? additional : Math.ceil(additional / 2)) > max) panic("Hash table capacity overflow");
}

/**
 * `extend` of a map with `[key, value]`s, and `collect` into one through `hashNew`, which
 * reserves first as `hashReserve` does. Of equal keys, the first stays and each later one
 * drops, then the value it replaced.
 */
export function hashExtend<K, V>(m: HashMap<K, V>, items: Iter<[K, V]>, k?: (key: K) => void, v?: (value: V) => void, take?: Take, max?: number): HashMap<K, V> {
	hashReserve(m, items.sizeHint()[0], max);
	const add = (item: [K, V]) => {
		const old = hashInsert(m, item[0], item[1], k);
		if (old !== undefined) v?.(old);
	};
	return extending(m, items, add, take);
}

/** `extend` of a set with keys, and `collect` into one, as `hashExtend`: a key already there drops. */
export function hashExtendKeys<K>(m: HashSet<K>, keys: Iter<K>, k?: (key: K) => void, take?: Take, max?: number): HashSet<K> {
	hashReserve(m, keys.sizeHint()[0], max);
	return extending(m, keys, (key) => hashAdd(m, key, k), take);
}

/** `clone`, with each key and value through its `Clone`, or as itself without one. */
export function hashClone<K, V>(m: HashMap<K, V>, key: (k: K) => K = itself, value: (v: V) => V = itself): HashMap<K, V> {
	const out = hashNew<K, V>(m.h);
	for (const [k, v] of m.p) out.p.set(key(k), value(v));
	for (const first of m.o?.values() ?? []) {
		for (let s: Slot<K, V> | undefined = first; s !== undefined; s = s.next) add(out, key(s.k), value(s.v));
	}
	return out;
}

// Set operations of hash sets, as std's: each walks one set's keys in the runtime's
// order, which picks which set's key of two equal ones it yields.

/** `HashSet::union`: each key of the larger, `a` of two as large, then each of the other's that it lacks. */
export function hashUnion<K>(a: HashSet<K>, b: HashSet<K>): Iter<K> {
	const [large, small] = hashLen(a) >= hashLen(b) ? [a, b] : [b, a];
	return iterChain(hashKeys(large), hashDifference(small, large));
}

/** `HashSet::intersection`: each key of the smaller, `a` of two as large, that the other holds. */
export function hashIntersection<K>(a: HashSet<K>, b: HashSet<K>): Iter<K> {
	const [small, large] = hashLen(a) <= hashLen(b) ? [a, b] : [b, a];
	return iterFilter(hashKeys(small), (key) => hashContains(large, key));
}

/** `HashSet::difference`: each key of `a` that `b` lacks. */
export function hashDifference<K>(a: HashSet<K>, b: HashSet<K>): Iter<K> {
	return iterFilter(hashKeys(a), (key) => !hashContains(b, key));
}

/** `HashSet::symmetric_difference`: `a`'s difference from `b`, then `b`'s from `a`. */
export function hashSymmetricDifference<K>(a: HashSet<K>, b: HashSet<K>): Iter<K> {
	return iterChain(hashDifference(a, b), hashDifference(b, a));
}

/** `HashSet::is_subset`: whether `b` holds each key of `a`. */
export function hashIsSubset<K>(a: HashSet<K>, b: HashSet<K>): boolean {
	if (hashLen(a) > hashLen(b)) return false;
	for (const key of hashKeys(a)) if (!hashContains(b, key)) return false;
	return true;
}

/** `HashSet::is_superset`: whether `a` holds each key of `b`. */
export function hashIsSuperset<K>(a: HashSet<K>, b: HashSet<K>): boolean {
	return hashIsSubset(b, a);
}

/** `HashSet::is_disjoint`: whether the larger lacks each key of the smaller, `a` of two as large. */
export function hashIsDisjoint<K>(a: HashSet<K>, b: HashSet<K>): boolean {
	const [small, large] = hashLen(a) <= hashLen(b) ? [a, b] : [b, a];
	for (const key of hashKeys(small)) if (hashContains(large, key)) return false;
	return true;
}

// BTreeMap and BTreeSet

/**
 * The most keys a node holds. A B+ tree's leaves hold its keys and values in order, and
 * its branches each child's lower bound, so every operation but a split or merge shifts
 * at most this many array slots after `log_{NODE/2} n` searches of one node each.
 */
const NODE = 64;
/** The fewest keys a node other than the root holds, half of `NODE`, as a literal a bundler can drop. */
const MIN = 32;

/** A leaf: its keys in order, each key's value at the same index, and the leaves beside it. */
interface Leaf<K, V> {
	k: K[];
	v: V[];
	prev: Leaf<K, V> | undefined;
	next: Leaf<K, V> | undefined;
}

/** A branch: its children, and each one's lower bound, which the first child's is not. */
interface Branch<K, V> {
	k: K[];
	c: Node<K, V>[];
}

type Node<K, V> = Leaf<K, V> | Branch<K, V>;

/** `BTreeMap<K, V>`, and `BTreeSet<K>` as a map to `undefined`: a B+ tree ordered by `Ord`. */
export interface BTreeMap<K, V> {
	/** The root, which is a leaf where `h` is 0. */
	t: Node<K, V>;
	/** The height: how many branches are above each leaf. */
	h: number;
	/** How many keys it holds. */
	n: number;
	/** `Ord::cmp` of the keys. */
	readonly c: (a: K, b: K) => Ordering;
	/**
	 * Whether std's map would have its root node: from its first insert until `clear`.
	 * Only a map with one checks a range's bounds.
	 */
	r: boolean;
}

/** `BTreeSet<K>`. */
export type BTreeSet<K> = BTreeMap<K, undefined>;

function leaf<K, V>(k: K[] = [], v: V[] = []): Leaf<K, V> {
	return { k, v, prev: undefined, next: undefined };
}

/**
 * `BTreeMap::new` and `BTreeSet::new`, ordered by `cmp`. The type `K` is asserted, not
 * inferred from `cmp`: a slice's order takes any `ArrayLike`.
 */
export function btreeNew<K, V>(cmp: (a: NoInfer<K>, b: NoInfer<K>) => Ordering): BTreeMap<K, V> {
	return { t: leaf(), h: 0, n: 0, c: cmp, r: false };
}

/** The index of `key` in `keys` if there, or else the bitwise not of where it would go. */
function search<K>(keys: K[], key: K, cmp: (a: K, b: K) => Ordering): number {
	let lo = 0;
	let hi = keys.length;
	while (lo < hi) {
		const mid = (lo + hi) >>> 1;
		const c = cmp(keys[mid], key);
		if (c === 0) return mid;
		if (c < 0) lo = mid + 1;
		else hi = mid;
	}
	return ~lo;
}

/** The index of the child of `b` whose keys `key` would be among. */
function child<K, V>(b: Branch<K, V>, key: K, cmp: (a: K, b: K) => Ordering): number {
	let lo = 1;
	let hi = b.k.length;
	while (lo < hi) {
		const mid = (lo + hi) >>> 1;
		if (cmp(b.k[mid], key) <= 0) lo = mid + 1;
		else hi = mid;
	}
	return lo - 1;
}

/** The leaf `key` would be in. */
function leafOf<K, V>(m: BTreeMap<K, V>, key: K): Leaf<K, V> {
	let node = m.t;
	for (let h = m.h; h > 0; h--) {
		const b = node as Branch<K, V>;
		node = b.c[child(b, key, m.c)];
	}
	return node as Leaf<K, V>;
}

/** The first leaf, or with `last`, the last. */
function end<K, V>(m: BTreeMap<K, V>, last: boolean): Leaf<K, V> {
	let node = m.t;
	for (let h = m.h; h > 0; h--) {
		const c = (node as Branch<K, V>).c;
		node = c[last ? c.length - 1 : 0];
	}
	return node as Leaf<K, V>;
}

export function btreeLen<K, V>(m: BTreeMap<K, V>): number {
	return m.n;
}

/** `BTreeMap::get`: the value there, or else `none`, the `None` of an `Option` of a value that can be `undefined`. */
export function btreeGet<K, V>(m: BTreeMap<K, V>, key: K): V | undefined;
export function btreeGet<K, V>(m: BTreeMap<K, V>, key: K, none: None): V | None;
export function btreeGet<K, V>(m: BTreeMap<K, V>, key: K, none?: None): V | None | undefined {
	const l = leafOf(m, key);
	const i = search(l.k, key, m.c);
	return i < 0 ? none : l.v[i];
}

/** `BTreeMap::get_mut` of a value JS copies: its place, if the map holds `key`. */
export function btreeGetMut<K, V>(m: BTreeMap<K, V>, key: K): Place<V> | undefined {
	const l = leafOf(m, key);
	const i = search(l.k, key, m.c);
	return i < 0 ? undefined : new AtIndex(l.v, i);
}

/** `BTreeMap::get_key_value`: the equal key the map holds, and its value. */
export function btreeGetKeyValue<K, V>(m: BTreeMap<K, V>, key: K): [K, V] | undefined {
	const l = leafOf(m, key);
	const i = search(l.k, key, m.c);
	return i < 0 ? undefined : [l.k[i], l.v[i]];
}

/** `BTreeSet::get`, and `BTreeSet::take` if `take`: the equal key the set holds, or else `none`. */
export function btreeKey<K, V>(m: BTreeMap<K, V>, key: K, take?: boolean): K | undefined;
export function btreeKey<K, V>(m: BTreeMap<K, V>, key: K, take: boolean, none: None): K | None;
export function btreeKey<K, V>(m: BTreeMap<K, V>, key: K, take = false, none?: None): K | None | undefined {
	const l = leafOf(m, key);
	const i = search(l.k, key, m.c);
	if (i < 0) return none;
	const held = l.k[i];
	if (take) removeAt(m, l, i);
	return held;
}

/** `BTreeSet::replace`: the equal key the set held, which `key` takes the place of, or else `none`, with `key` added. */
export function btreeReplace<K>(m: BTreeSet<K>, key: K): K | undefined;
export function btreeReplace<K>(m: BTreeSet<K>, key: K, none: None): K | None;
export function btreeReplace<K>(m: BTreeSet<K>, key: K, none?: None): K | None | undefined {
	const l = leafOf(m, key);
	const i = search(l.k, key, m.c);
	if (i < 0) {
		insertAt(m, l, ~i, key, undefined);
		return none;
	}
	const old = l.k[i];
	l.k[i] = key;
	if (bounding(l, i)) rebound(m, key, old);
	return old;
}

export function btreeContains<K, V>(m: BTreeMap<K, V>, key: K): boolean {
	return search(leafOf(m, key).k, key, m.c) >= 0;
}

/** `BTreeMap::index`: `map[&key]`. */
export function btreeIndex<K, V>(m: BTreeMap<K, V>, key: K): V {
	const l = leafOf(m, key);
	const i = search(l.k, key, m.c);
	if (i < 0) panic("no entry found for key");
	return l.v[i];
}

/** `BTreeMap::insert`: the old value, or else `none`. An equal key already there stays, and `key` drops. */
export function btreeInsert<K, V>(m: BTreeMap<K, V>, key: K, value: V, k?: (key: K) => void): V | undefined;
export function btreeInsert<K, V>(m: BTreeMap<K, V>, key: K, value: V, k: ((key: K) => void) | undefined, none: None): V | None;
export function btreeInsert<K, V>(m: BTreeMap<K, V>, key: K, value: V, k?: (key: K) => void, none?: None): V | None | undefined {
	const l = leafOf(m, key);
	const i = search(l.k, key, m.c);
	if (i < 0) {
		insertAt(m, l, ~i, key, value);
		return none;
	}
	k?.(key);
	const old = l.v[i];
	l.v[i] = value;
	return old;
}

/** Puts `key` and `value` at index `i` of `l`, the leaf it goes in, and splits what overflows. */
function insertAt<K, V>(m: BTreeMap<K, V>, l: Leaf<K, V>, i: number, key: K, value: V): void {
	m.r = true;
	m.n++;
	put(l.k, i, key);
	put(l.v, i, value);
	if (l.k.length > NODE) settle(m, key);
}

/** Puts `item` at index `i` of `a`, after moving the rest up: in a node, faster than `splice`. */
function put<T>(a: T[], i: number, item: T): void {
	let j = a.length;
	a.push(item);
	for (; j > i; j--) a[j] = a[j - 1];
	a[i] = item;
}

/** Takes index `i` out of `a`, moving the rest down. */
function take<T>(a: T[], i: number): void {
	const n = a.length - 1;
	for (let j = i; j < n; j++) a[j] = a[j + 1];
	a.pop();
}

/** `BTreeSet::insert`: whether `key` was not there yet. An equal key already there stays, and `key` drops. */
export function btreeAdd<K>(m: BTreeSet<K>, key: K, k?: (key: K) => void): boolean {
	const l = leafOf(m, key);
	const i = search(l.k, key, m.c);
	if (i >= 0) {
		k?.(key);
		return false;
	}
	insertAt(m, l, ~i, key, undefined);
	return true;
}

/** Takes the key and value at index `i` of `l` out, and merges the leaf if too small. */
function removeAt<K, V>(m: BTreeMap<K, V>, l: Leaf<K, V>, i: number): void {
	const key = l.k[i];
	const bound = bounding(l, i);
	m.n--;
	take(l.k, i);
	take(l.v, i);
	if (l.k.length < MIN && m.h > 0) settle(m, key);
	if (bound) rebound(m, key, key);
}

/**
 * Whether index `i` of `l` may hold a key a branch bounds a child by: a leaf's first,
 * but for the first leaf's, whose bounds no lookup reads, as `pop_first` takes.
 */
function bounding<K, V>(l: Leaf<K, V>, i: number): boolean {
	return i === 0 && l.prev !== undefined;
}

/**
 * Bounds each child by its first key where `old`, a key the tree gave back, which may
 * change as it drops, bounds it: on the path to `key`, which `old` is equal to.
 */
function rebound<K, V>(m: BTreeMap<K, V>, key: K, old: K): void {
	let node = m.t;
	for (let h = m.h; h > 0; h--) {
		const b = node as Branch<K, V>;
		const j = child(b, key, m.c);
		node = b.c[j];
		if (b.k[j] !== old) continue;
		let first = node;
		for (let d = h - 1; d > 0; d--) first = (first as Branch<K, V>).c[0];
		b.k[j] = first.k[0];
	}
}

/** `BTreeMap::remove_entry`. */
export function btreeRemoveEntry<K, V>(m: BTreeMap<K, V>, key: K): [K, V] | undefined {
	const l = leafOf(m, key);
	const i = search(l.k, key, m.c);
	if (i < 0) return undefined;
	const entry: [K, V] = [l.k[i], l.v[i]];
	removeAt(m, l, i);
	return entry;
}

/** `BTreeMap::remove`: the value there, whose key drops, or else `none`. */
export function btreeRemove<K, V>(m: BTreeMap<K, V>, key: K, k?: (key: K) => void): V | undefined;
export function btreeRemove<K, V>(m: BTreeMap<K, V>, key: K, k: ((key: K) => void) | undefined, none: None): V | None;
export function btreeRemove<K, V>(m: BTreeMap<K, V>, key: K, k?: (key: K) => void, none?: None): V | None | undefined {
	const l = leafOf(m, key);
	const i = search(l.k, key, m.c);
	if (i < 0) return none;
	const held = l.k[i];
	const value = l.v[i];
	removeAt(m, l, i);
	k?.(held);
	return value;
}

/** `BTreeSet::remove`: whether `key` was there. The key it held drops. */
export function btreeDelete<K>(m: BTreeSet<K>, key: K, k?: (key: K) => void): boolean {
	const l = leafOf(m, key);
	const i = search(l.k, key, m.c);
	if (i < 0) return false;
	const held = l.k[i];
	removeAt(m, l, i);
	k?.(held);
	return true;
}

/**
 * Splits or merges the node on `key`'s path that a change left too large or too small,
 * and each above it that this leaves so in turn, finding them again from the root.
 */
function settle<K, V>(m: BTreeMap<K, V>, key: K): void {
	if (m.h > 0) settleBelow(m, m.t as Branch<K, V>, m.h, key);
	const t = m.t;
	if (t.k.length > NODE) {
		const right = split(t, m.h === 0);
		m.t = { k: [t.k[0], right.k[0]], c: [t, right] };
		m.h++;
	} else if (m.h > 0 && (t as Branch<K, V>).c.length === 1) {
		m.t = (t as Branch<K, V>).c[0];
		m.h--;
	}
}

/** `settle` below `b`, a branch at height `h`. */
function settleBelow<K, V>(m: BTreeMap<K, V>, b: Branch<K, V>, h: number, key: K): void {
	const i = child(b, key, m.c);
	const c = b.c[i];
	if (h > 1) settleBelow(m, c as Branch<K, V>, h - 1, key);
	if (c.k.length > NODE) {
		const right = split(c, h === 1);
		b.k.splice(i + 1, 0, right.k[0]);
		b.c.splice(i + 1, 0, right);
	} else if (c.k.length < MIN) {
		const j = i > 0 ? i - 1 : i;
		balance(b, j, h === 1);
	}
}

/** Takes the second half of `node` into a new node beside it, and returns that. */
function split<K, V>(node: Node<K, V>, isLeaf: boolean): Node<K, V> {
	const mid = node.k.length >> 1;
	if (!isLeaf) {
		const b = node as Branch<K, V>;
		return { k: b.k.splice(mid), c: b.c.splice(mid) };
	}
	const l = node as Leaf<K, V>;
	const right = leaf(l.k.splice(mid), l.v.splice(mid));
	right.prev = l;
	right.next = l.next;
	if (l.next !== undefined) l.next.prev = right;
	l.next = right;
	return right;
}

/** Evens out children `j` and `j + 1` of `b`, one of which is too small, or merges them. */
function balance<K, V>(b: Branch<K, V>, j: number, isLeaf: boolean): void {
	const left = b.c[j];
	const right = b.c[j + 1];
	const total = left.k.length + right.k.length;
	if (total <= NODE) {
		left.k.push(...right.k);
		if (isLeaf) {
			const l = left as Leaf<K, V>;
			const r = right as Leaf<K, V>;
			l.v.push(...r.v);
			l.next = r.next;
			if (r.next !== undefined) r.next.prev = l;
		} else {
			(left as Branch<K, V>).c.push(...(right as Branch<K, V>).c);
		}
		b.k.splice(j + 1, 1);
		b.c.splice(j + 1, 1);
		return;
	}
	const half = total >> 1;
	if (left.k.length < half) {
		const n = half - left.k.length;
		left.k.push(...right.k.splice(0, n));
		if (isLeaf) (left as Leaf<K, V>).v.push(...(right as Leaf<K, V>).v.splice(0, n));
		else (left as Branch<K, V>).c.push(...(right as Branch<K, V>).c.splice(0, n));
	} else {
		const n = left.k.length - half;
		right.k.unshift(...left.k.splice(half, n));
		if (isLeaf) (right as Leaf<K, V>).v.unshift(...(left as Leaf<K, V>).v.splice(half, n));
		else (right as Branch<K, V>).c.unshift(...(left as Branch<K, V>).c.splice(half, n));
	}
	b.k[j + 1] = right.k[0];
}

/** The tree of `keys` and their `values`, in order and without equal keys, built whole. */
function build<K, V>(m: BTreeMap<K, V>, keys: K[], values: V[]): void {
	m.n = keys.length;
	// As few nodes per level as hold them, evened out, so each holds at least half.
	const chunks = (n: number) => {
		const count = Math.max(1, Math.ceil(n / NODE));
		return Array.from({ length: count }, (_, i) => [Math.floor((i * n) / count), Math.floor(((i + 1) * n) / count)]);
	};
	let level: Node<K, V>[] = [];
	let prev: Leaf<K, V> | undefined;
	for (const [from, to] of chunks(keys.length)) {
		const l = leaf(keys.slice(from, to), values.slice(from, to));
		l.prev = prev;
		if (prev !== undefined) prev.next = l;
		prev = l;
		level.push(l);
	}
	let h = 0;
	while (level.length > 1) {
		level = chunks(level.length).map(([from, to]) => {
			const c = level.slice(from, to);
			return { k: c.map((node) => node.k[0]), c };
		});
		h++;
	}
	m.t = level[0];
	m.h = h;
}

/** Each key in order, and each one's value. */
function flatten<K, V>(m: BTreeMap<K, V>): [K[], V[]] {
	const keys: K[] = [];
	const values: V[] = [];
	for (let l: Leaf<K, V> | undefined = end(m, false); l !== undefined; l = l.next) {
		keys.push(...l.k);
		values.push(...l.v);
	}
	return [keys, values];
}

/** `first_key_value`, and with `last`, `last_key_value`. */
export function btreeFirst<K, V>(m: BTreeMap<K, V>, last = false): [K, V] | undefined {
	if (m.n === 0) return undefined;
	const l = end(m, last);
	const i = last ? l.k.length - 1 : 0;
	return [l.k[i], l.v[i]];
}

/** `BTreeSet::first`, and with `last`, `BTreeSet::last`, or else `none`. */
export function btreeFirstKey<K, V>(m: BTreeMap<K, V>, last?: boolean): K | undefined;
export function btreeFirstKey<K, V>(m: BTreeMap<K, V>, last: boolean, none: None): K | None;
export function btreeFirstKey<K, V>(m: BTreeMap<K, V>, last = false, none?: None): K | None | undefined {
	if (m.n === 0) return none;
	const l = end(m, last);
	return l.k[last ? l.k.length - 1 : 0];
}

/** `pop_first`, and with `last`, `pop_last`. */
export function btreePop<K, V>(m: BTreeMap<K, V>, last = false): [K, V] | undefined {
	if (m.n === 0) return undefined;
	const l = end(m, last);
	const i = last ? l.k.length - 1 : 0;
	const entry: [K, V] = [l.k[i], l.v[i]];
	removeAt(m, l, i);
	return entry;
}

/** `BTreeSet::pop_first`, and with `last`, `pop_last`, or else `none`. */
export function btreePopKey<K, V>(m: BTreeMap<K, V>, last?: boolean): K | undefined;
export function btreePopKey<K, V>(m: BTreeMap<K, V>, last: boolean, none: None): K | None;
export function btreePopKey<K, V>(m: BTreeMap<K, V>, last = false, none?: None): K | None | undefined {
	const entry = btreePop(m, last);
	return entry === undefined ? none : entry[0];
}

/** Drop `m`: each key by `k` and each value by `v`, of those that drop, in key order. */
export function btreeDrop<K, V>(m: BTreeMap<K, V>, k: ((key: K) => void) | undefined, v?: (value: V) => void): void {
	const [keys, values] = flatten(m);
	dropPairs(keys, values, k, v);
}

/** `clear`, which leaves the map empty and then drops each entry in key order. */
export function btreeClear<K, V>(m: BTreeMap<K, V>, k?: (key: K) => void, v?: (value: V) => void): void {
	if (k !== undefined || v !== undefined) {
		const old: BTreeMap<K, V> = { t: m.t, h: m.h, n: m.n, c: m.c, r: m.r };
		btreeClear(m);
		btreeDrop(old, k, v);
		return;
	}
	m.t = leaf();
	m.h = 0;
	m.n = 0;
	m.r = false;
}

/**
 * `split_off` of a map or set: the entries from `key` on, moved in order into a tree of
 * their own. Like std's, the new tree has a root unless `m` was empty.
 */
export function btreeSplitOff<K, V>(m: BTreeMap<K, V>, key: K): BTreeMap<K, V> {
	const out = btreeNew<K, V>(m.c);
	if (m.n === 0) return out;
	const [keys, values] = flatten(m);
	const i = search(keys, key, m.c);
	const at = i < 0 ? ~i : i;
	build(out, keys.slice(at), values.slice(at));
	build(m, keys.slice(0, at), values.slice(0, at));
	out.r = true;
	return out;
}

/**
 * `retain`, which visits the keys in order and keeps each entry `f` returns true for,
 * dropping each other one as `f` rejects it.
 */
export function btreeRetain<K, V>(m: BTreeMap<K, V>, f: (key: K, value: V) => boolean, k?: (key: K) => void, v?: (value: V) => void): void {
	retainBTree(m, f as (key: K, value: V | Place<V>) => boolean, false, k, v);
}

/** `retain` of values JS copies: `f` takes the place of each. */
export function btreeRetainMut<K, V>(m: BTreeMap<K, V>, f: (key: K, value: Place<V>) => boolean, k?: (key: K) => void, v?: (value: V) => void): void {
	retainBTree(m, f as (key: K, value: V | Place<V>) => boolean, true, k, v);
}

/** `retain`, handing `f` each value or, if `lend`, its place, which the map is built from again. */
function retainBTree<K, V>(m: BTreeMap<K, V>, f: (key: K, value: V | Place<V>) => boolean, lend: boolean, k?: (key: K) => void, v?: (value: V) => void): void {
	const [keys, values] = flatten(m);
	let kept = 0;
	for (let i = 0; i < keys.length; i++) {
		const key = keys[i];
		const keep = f(key, lend ? new AtIndex(values, i) : values[i]);
		const value = values[i];
		if (!keep) {
			dropPair(key, value, k, v);
			continue;
		}
		keys[kept] = key;
		values[kept] = value;
		kept++;
	}
	if (kept < keys.length || lend) {
		keys.length = kept;
		values.length = kept;
		build(m, keys, values);
	}
}

/**
 * An iterator of a B-tree's entries, keys, or values, from index `fi` of leaf `fl` up to
 * before index `bi` of leaf `bl`. The front moves to the next leaf as it passes a leaf's
 * last key, and the back to the previous leaf as it passes a leaf's first key. One that
 * owns its entries drops as `HashIter` does.
 */
export class BTreeIter<K, V, T> extends Iter<T> implements Disposable {
	fl: Leaf<K, V>;
	fi: number;
	bl: Leaf<K, V>;
	bi: number;
	/** How many items are left, where it is known: an iterator of the whole map. */
	private n: number;
	private readonly pick: Pick;
	private readonly k: ((key: K) => void) | undefined;
	private readonly v: ((value: V) => void) | undefined;
	/** Whether its `last` reads from the back, as a map's `into_keys`' and `into_values`' do. */
	readonly back: boolean;

	constructor(pick: Pick, fl: Leaf<K, V>, fi: number, bl: Leaf<K, V>, bi: number, n = -1, k?: (key: K) => void, v?: (value: V) => void, back = false) {
		super();
		this.pick = pick;
		if (fi === fl.k.length && fl.next !== undefined) {
			fl = fl.next;
			fi = 0;
		}
		if (bi === 0 && bl.prev !== undefined) {
			bl = bl.prev;
			bi = bl.k.length;
		}
		this.fl = fl;
		this.fi = fi;
		this.bl = bl;
		this.bi = bi;
		this.n = n;
		this.k = k;
		this.v = v;
		this.back = back;
	}

	next(): T | Done {
		if (done(this)) return DONE;
		const l = this.fl;
		const i = this.fi++;
		if (this.fi === l.k.length && l.next !== undefined) {
			this.fl = l.next;
			this.fi = 0;
		}
		this.n--;
		return this.picked(l, i);
	}

	override nextBack(): T | Done {
		if (done(this)) return DONE;
		const l = this.bl;
		const i = --this.bi;
		if (this.bi === 0 && l.prev !== undefined) {
			this.bl = l.prev;
			this.bi = this.bl.k.length;
		}
		this.n--;
		return this.picked(l, i);
	}

	/**
	 * What it yields of index `i` of leaf `l`: of `into_keys` or `into_values`, the other
	 * half drops first.
	 */
	private picked(l: Leaf<K, V>, i: number): T {
		if (this.pick === ENTRIES) return [l.k[i], l.v[i]] as T;
		if (this.pick === PLACES) return [l.k[i], new AtIndex(l.v, i)] as T;
		if (this.pick === VALUE_PLACES) return new AtIndex(l.v, i) as T;
		if (this.pick === KEYS) {
			this.v?.(l.v[i]);
			return l.k[i] as unknown as T;
		}
		this.k?.(l.k[i]);
		return l.v[i] as unknown as T;
	}

	override remaining(): Iterable<unknown> {
		return new BTreeIter<K, V, unknown>(byValue(this.pick), this.fl, this.fi, this.bl, this.bi);
	}

	/** std's, of an iterator that borrows the map. */
	override clone(): Iter<T> {
		return new BTreeIter<K, V, T>(this.pick, this.fl, this.fi, this.bl, this.bi, this.n);
	}

	override len(): number {
		return this.n;
	}

	override sizeHint(): SizeHint {
		// A range's count is unknown, and std's hint of one is `(0, None)`.
		return this.n < 0 ? [0, undefined] : exactHint(this.n);
	}

	[Symbol.dispose](): void {
		if (this.k === undefined && this.v === undefined) return;
		const keys: K[] = [];
		const values: V[] = [];
		while (!done(this)) {
			const l = this.fl;
			keys.push(l.k[this.fi]);
			values.push(l.v[this.fi]);
			this.fi++;
			if (this.fi === l.k.length && l.next !== undefined) {
				this.fl = l.next;
				this.fi = 0;
			}
		}
		this.n = 0;
		dropPairs(keys, values, this.k, this.v);
	}
}

/** Whether `it`'s front has met its back. */
function done<K, V, T>(it: BTreeIter<K, V, T>): boolean {
	return it.fl === it.bl ? it.fi >= it.bi : it.fi === 0 && it.bl.next === it.fl && it.bi === it.bl.k.length;
}

function whole<K, V, T>(m: BTreeMap<K, V>, pick: Pick, k?: (key: K) => void, v?: (value: V) => void, back = false): Iter<T> {
	const last = end(m, true);
	return new BTreeIter<K, V, T>(pick, end(m, false), 0, last, last.k.length, m.n, k, v, back);
}

/**
 * `iter` and `iter_mut` of a map, and with what drops each key and value of those that
 * drop, `into_iter`: its entries in key order.
 */
export function btreeIter<K, V>(m: BTreeMap<K, V>, k?: (key: K) => void, v?: (value: V) => void): Iter<[K, V]> {
	return whole(m, ENTRIES, k, v);
}

/**
 * `keys` of a map and `iter` of a set, and with droppers as `btreeIter`'s, `into_iter` of
 * a set, or with `back`, `into_keys`.
 */
export function btreeKeys<K, V>(m: BTreeMap<K, V>, k?: (key: K) => void, v?: (value: V) => void, back = false): Iter<K> {
	return whole(m, KEYS, k, v, back);
}

/** `iter_mut` of values JS copies: each key and the place of its value, in key order. */
export function btreeIterMut<K, V>(m: BTreeMap<K, V>): Iter<[K, Place<V>]> {
	return whole(m, PLACES);
}

/** `values_mut` of values JS copies: the place of each. */
export function btreeValuesMut<K, V>(m: BTreeMap<K, V>): Iter<Place<V>> {
	return whole(m, VALUE_PLACES);
}

/** `values` and `values_mut`, and with droppers as `btreeIter`'s, `into_values`. */
export function btreeValues<K, V>(m: BTreeMap<K, V>, k?: (key: K) => void, v?: (value: V) => void): Iter<V> {
	return whole(m, VALUES, k, v, k !== undefined || v !== undefined);
}

/**
 * A range of keys or indices, as its value is: `a..b`, `a..=b`, `a..` as `{ start }`, `..b`
 * as `{ end }`, `..` as `undefined`, and `(Bound<K>, Bound<K>)` as its pair. Any other
 * `RangeBounds` is a function returning its `start_bound` and `end_bound`, called where
 * std's calls them.
 */
export type Bounds<K> =
	| { readonly start: K; readonly end: K }
	| RangeInclusive<K>
	| { readonly start: K }
	| { readonly end: K; readonly inclusive?: true }
	| BoundPair<K>
	| (() => BoundPair<K>)
	| undefined;

type BoundPair<K> = readonly [RangeBound<K>, RangeBound<K>];

const UNBOUNDED: RangeBound<never> = { tag: "Unbounded" };

/** Another `RangeBounds`, `r`, as `Bounds` reads it: by `start` and then `end`, once read. */
export function boundsOf<R, K>(r: R, start: (r: R) => RangeBound<K>, end: (r: R) => RangeBound<K>): () => BoundPair<K> {
	return () => [start(r), end(r)];
}

/** `RangeBounds`: `start_bound`, then `end_bound`. */
function bounds<K>(r: Bounds<K>): BoundPair<K> {
	if (r === undefined) return [UNBOUNDED, UNBOUNDED];
	if (typeof r === "function") return r();
	if (Array.isArray(r)) return r as BoundPair<K>;
	if (inclusive(r)) return [rangeInclusiveStartBound(r), rangeInclusiveEndBound(r)];
	const start: RangeBound<K> = "start" in r ? { tag: "Included", _0: r.start } : UNBOUNDED;
	if (!("end" in r)) return [start, UNBOUNDED];
	return [start, { tag: "inclusive" in r ? "Included" : "Excluded", _0: r.end }];
}

/**
 * `BTreeMap::range` and `range_mut`: the entries within `r`. A key of the crate's own gives
 * its `PartialEq::eq` and `PartialOrd::gt`, which std checks the ends by.
 */
export function btreeRange<K, V>(m: BTreeMap<K, V>, r: Bounds<K>, eq?: Test<K>, gt?: Test<K>): Iter<[K, V]> {
	return range(m, r, ENTRIES, "BTreeMap", eq, gt);
}

/** `range_mut` of values JS copies: each key within `r` and the place of its value. */
export function btreeRangeMut<K, V>(m: BTreeMap<K, V>, r: Bounds<K>, eq?: Test<K>, gt?: Test<K>): Iter<[K, Place<V>]> {
	return range(m, r, PLACES, "BTreeMap", eq, gt);
}

/** `BTreeSet::range`: the keys within `r`. */
export function btreeRangeKeys<K, V>(m: BTreeMap<K, V>, r: Bounds<K>, eq?: Test<K>, gt?: Test<K>): Iter<K> {
	return range(m, r, KEYS, "BTreeSet", eq, gt);
}

/** The items of `m` within `r`, checked as std checks it: by `eq` and `gt`, or by `m`'s order. */
function range<K, V, T>(m: BTreeMap<K, V>, r: Bounds<K>, pick: Pick, name: string, eq?: Test<K>, gt?: Test<K>): Iter<T> {
	if (!m.r) return new BTreeIter<K, V, T>(pick, m.t as Leaf<K, V>, 0, m.t as Leaf<K, V>, 0);
	const [start, stop] = bounds(r);
	if (start.tag !== "Unbounded" && stop.tag !== "Unbounded") {
		const s = start._0;
		const e = stop._0;
		if (start.tag === "Excluded" && stop.tag === "Excluded" && (eq === undefined ? m.c(s, e) === 0 : eq(s, e))) {
			panic(`range start and end are equal and excluded in ${name}`);
		}
		if (gt === undefined ? m.c(s, e) > 0 : gt(s, e)) panic(`range start is greater than range end in ${name}`);
	}
	const [fl, fi] = start.tag === "Unbounded" ? [end(m, false), 0] : edge(m, start._0, start.tag === "Excluded");
	const last = end(m, true);
	const [bl, bi] = stop.tag === "Unbounded" ? [last, last.k.length] : edge(m, stop._0, stop.tag === "Included");
	return new BTreeIter<K, V, T>(pick, fl, fi, bl, bi);
}

/** Where the first key past `key` is if `past`, or else the first key not below it. */
function edge<K, V>(m: BTreeMap<K, V>, key: K, past: boolean): [Leaf<K, V>, number] {
	const l = leafOf(m, key);
	const i = search(l.k, key, m.c);
	return [l, i < 0 ? ~i : past ? i + 1 : i];
}

/**
 * `extend` of a map with `[key, value]`s, which keeps an equal key already there and drops
 * the one given, then the value it replaced, and takes `items` as `take` says. With
 * `last`, `collect` and `from` of entries that drop nothing into an empty one, which keep
 * the last of equal keys, as `btreeCollect` does, and reserve as the `Vec` of `size`-byte
 * entries std's collects into first does, as `iterExtend`.
 */
export function btreeExtend<K, V>(
	m: BTreeMap<K, V>,
	items: Iter<[K, V]>,
	last = false,
	k?: (key: K) => void,
	v?: (value: V) => void,
	take?: Take,
	size?: number,
	trusted = false,
): BTreeMap<K, V> {
	const add = (item: [K, V]) => {
		if (!last) {
			const old = btreeInsert(m, item[0], item[1], k);
			if (old !== undefined) v?.(old);
			return;
		}
		const l = leafOf(m, item[0]);
		const i = search(l.k, item[0], m.c);
		if (i < 0) insertAt(m, l, ~i, item[0], item[1]);
		else {
			l.k[i] = item[0];
			l.v[i] = item[1];
		}
	};
	return vecExtending(m, 0, items, add, take, size, trusted);
}

/** `extend` of a set with keys, and with `last`, `collect` and `from`, as `btreeExtend`. */
export function btreeExtendKeys<K>(m: BTreeSet<K>, keys: Iter<K>, last = false, k?: (key: K) => void, take?: Take, size?: number, trusted = false): BTreeSet<K> {
	const add = (key: K) => {
		if (!last) {
			btreeAdd(m, key, k);
			return;
		}
		const l = leafOf(m, key);
		const i = search(l.k, key, m.c);
		if (i < 0) insertAt(m, l, ~i, key, undefined);
		else l.k[i] = key;
	};
	return vecExtending(m, 0, keys, add, take, size, trusted);
}

/**
 * `collect` and `from` into `m`, an empty map, of `entries`, which std's collects into a
 * `Vec` first. It sorts them stably by `Ord::cmp` of their keys if `sorts`, as std's sort
 * does all but zero-sized ones, and builds the tree of each but those whose key `same`,
 * `!ne`, finds equal to the next one's, which drop in order. Without `same`, the keys'
 * impls agree, so JS's sort and `Ord` alone build the same tree faster.
 */
export function btreeCollect<K, V>(m: BTreeMap<K, V>, entries: [K, V][], sorts = false, same?: Test<K>, k?: (key: K) => void, v?: (value: V) => void): BTreeMap<K, V> {
	if (same === undefined) entries.sort((a, b) => m.c(a[0], b[0]));
	else if (sorts) sliceSort(entries, (a, b) => m.c(a[0], b[0]) === -1);
	const keys: K[] = [];
	const values: V[] = [];
	const n = entries.length;
	for (let i = 0; i < n; i++) {
		const [key, value] = entries[i];
		if (i + 1 === n || (same === undefined ? m.c(key, entries[i + 1][0]) !== 0 : !same(key, entries[i + 1][0]))) {
			keys.push(key);
			values.push(value);
		} else dropPair(key, value, k, v);
	}
	build(m, keys, values);
	// std's makes no root of none.
	m.r = n > 0;
	return m;
}

/**
 * `collect` and `from` into `m`, an empty set, of `keys`, as `btreeCollect`, but sorted by
 * `less`, their `PartialOrd::lt`, or not at all if zero-sized.
 */
export function btreeCollectKeys<K>(m: BTreeSet<K>, keys: K[], less?: Test<K>, same?: Test<K>, k?: (key: K) => void): BTreeSet<K> {
	if (same === undefined) {
		return btreeCollect(
			m,
			keys.map((key) => [key, undefined]),
			true,
			undefined,
			k,
		);
	}
	if (less !== undefined) sliceSort(keys, less);
	btreeFromSorted(m, iterSlice(keys), same, k);
	m.r = keys.length > 0;
	return m;
}

/** `clone`, with each key and value through its `Clone`, or as itself without one. */
export function btreeClone<K, V>(m: BTreeMap<K, V>, key: (k: K) => K = itself, value: (v: V) => V = itself): BTreeMap<K, V> {
	const keys: K[] = [];
	const values: V[] = [];
	// Each key, then its value, as std's clones them.
	for (let l: Leaf<K, V> | undefined = end(m, false); l !== undefined; l = l.next) {
		for (let i = 0; i < l.k.length; i++) {
			keys.push(key(l.k[i]));
			values.push(value(l.v[i]));
		}
	}
	const out = btreeNew<K, V>(m.c);
	build(out, keys, values);
	out.r = m.n > 0;
	return out;
}

// Set operations of B-trees, as std's: each merges both sets' keys in order, or of sets
// whose sizes differ by `TIPPING` times or more, looks up each of the smaller's in the
// larger, which picks which set's key of two equal ones it yields.

/** std's `ITER_PERFORMANCE_TIPPING_SIZE_DIFF`. */
const TIPPING = 16;

/** The first and last keys of `m`, which holds some. */
function ends<K>(m: BTreeSet<K>): [K, K] {
	return [btreeFirstKey(m) as K, btreeFirstKey(m, true) as K];
}

/**
 * std's `MergeIterInner`: the keys of `a` and `b` in order, a step at a time, holding back
 * the greater of two that differ.
 */
abstract class Merge<K> extends Iter<K> {
	private readonly a: Iter<K>;
	private readonly b: Iter<K>;
	private readonly c: (a: K, b: K) => Ordering;
	/** The key held back, and whether it is `a`'s. */
	private held: K | Done = DONE;
	private heldA = false;

	constructor(a: Iter<K>, b: Iter<K>, c: (a: K, b: K) => Ordering) {
		super();
		this.a = a;
		this.b = b;
		this.c = c;
	}

	/** std's: each of its iterators, and the key it holds back. */
	override clone(): Iter<K> {
		const kind = this.constructor as new (a: Iter<K>, b: Iter<K>, c: (a: K, b: K) => Ordering) => Merge<K>;
		const it = new kind(this.a.clone(), this.b.clone(), this.c);
		it.held = this.held;
		it.heldA = this.heldA;
		return it;
	}

	/** The next key of each, or `DONE` for one that ended or is held back. */
	protected nexts(): [K | Done, K | Done] {
		const held = this.held;
		this.held = DONE;
		const x = held !== DONE && this.heldA ? held : this.a.next();
		const y = held !== DONE && !this.heldA ? held : this.b.next();
		if (x === DONE || y === DONE) return [x, y];
		const order = this.c(x, y);
		if (order === 0) return [x, y];
		this.held = order < 0 ? y : x;
		this.heldA = order > 0;
		return order < 0 ? [x, DONE] : [DONE, y];
	}

	/** How many keys each has left, the one held back included. */
	protected lens(): [number, number] {
		const held = this.held === DONE ? 0 : 1;
		return [this.a.len() + (this.heldA ? held : 0), this.b.len() + (this.heldA ? 0 : held)];
	}
}

class Union<K> extends Merge<K> {
	next(): K | Done {
		const [x, y] = this.nexts();
		return x !== DONE ? x : y;
	}

	override sizeHint(): SizeHint {
		const [a, b] = this.lens();
		return [Math.max(a, b), a + b];
	}
}

class SymmetricDifference<K> extends Merge<K> {
	next(): K | Done {
		for (;;) {
			const [x, y] = this.nexts();
			if (x === DONE || y === DONE) return x !== DONE ? x : y;
		}
	}

	override sizeHint(): SizeHint {
		const [a, b] = this.lens();
		return [0, a + b];
	}
}

/** `BTreeSet::union`: each key of either in order, `a`'s of two equal ones. */
export function btreeUnion<K>(a: BTreeSet<K>, b: BTreeSet<K>): Iter<K> {
	return new Union(btreeKeys(a), btreeKeys(b), a.c);
}

/** `BTreeSet::symmetric_difference`: each key of one that the other lacks, in order. */
export function btreeSymmetricDifference<K>(a: BTreeSet<K>, b: BTreeSet<K>): Iter<K> {
	return new SymmetricDifference(btreeKeys(a), btreeKeys(b), a.c);
}

/**
 * Each key of `a` that the other set lacks: looked up in it, or with no set to look in,
 * found by stepping through its keys by `b`, peeked a key ahead.
 */
class Difference<K> extends Iter<K> {
	private readonly a: Iter<K>;
	private readonly set: BTreeSet<K> | undefined;
	private readonly b: Iter<K>;
	private readonly c: (a: K, b: K) => Ordering;
	/** `b`'s next key, once `peeked` says it was read. */
	private peek: K | Done = DONE;
	private peeked = false;

	constructor(a: Iter<K>, set: BTreeSet<K> | undefined, b: Iter<K>, c: (a: K, b: K) => Ordering) {
		super();
		this.a = a;
		this.set = set;
		this.b = b;
		this.c = c;
	}

	/** std's: each of its iterators, and the key it peeked. */
	override clone(): Iter<K> {
		const it = new Difference(this.a.clone(), this.set, this.b.clone(), this.c);
		it.peek = this.peek;
		it.peeked = this.peeked;
		return it;
	}

	next(): K | Done {
		let x = this.a.next();
		if (x === DONE) return DONE;
		if (this.set !== undefined) {
			while (btreeContains(this.set, x)) {
				x = this.a.next();
				if (x === DONE) return DONE;
			}
			return x;
		}
		for (;;) {
			if (!this.peeked) {
				this.peek = this.b.next();
				this.peeked = true;
			}
			const order = this.peek === DONE ? -1 : this.c(x, this.peek);
			if (order < 0) return x;
			if (order === 0) {
				x = this.a.next();
				if (x === DONE) return DONE;
			}
			this.peeked = false;
		}
	}

	override sizeHint(): SizeHint {
		const n = this.a.len();
		const other = this.set !== undefined ? this.set.n : this.b.len() + (this.peeked && this.peek !== DONE ? 1 : 0);
		return [Math.max(n - other, 0), n];
	}
}

/** `BTreeSet::difference`: each key of `a` that `b` lacks, in order. */
export function btreeDifference<K>(a: BTreeSet<K>, b: BTreeSet<K>): Iter<K> {
	const keys = btreeKeys(a);
	if (a.n === 0 || b.n === 0) return keys;
	const [aMin, aMax] = ends(a);
	const [bMin, bMax] = ends(b);
	const low = a.c(aMin, bMax);
	const high = a.c(aMax, bMin);
	if (low > 0 || high < 0) return keys;
	if (low === 0) keys.next();
	else if (high === 0) keys.nextBack();
	else if (a.n <= Math.floor(b.n / TIPPING)) return new Difference(keys, b, iterEmpty(), b.c);
	else return new Difference(keys, undefined, btreeKeys(b), b.c);
	return keys;
}

/** An intersection that merges: each key of `a` that `b` holds. */
class Intersection<K> extends Iter<K> {
	private readonly a: Iter<K>;
	private readonly b: Iter<K>;
	private readonly c: (a: K, b: K) => Ordering;

	constructor(a: Iter<K>, b: Iter<K>, c: (a: K, b: K) => Ordering) {
		super();
		this.a = a;
		this.b = b;
		this.c = c;
	}

	override clone(): Iter<K> {
		return new Intersection(this.a.clone(), this.b.clone(), this.c);
	}

	next(): K | Done {
		let x = this.a.next();
		if (x === DONE) return DONE;
		let y = this.b.next();
		if (y === DONE) return DONE;
		for (;;) {
			const order = this.c(x, y);
			if (order === 0) return x;
			if (order < 0) x = this.a.next();
			else y = this.b.next();
			if (x === DONE || y === DONE) return DONE;
		}
	}

	override sizeHint(): SizeHint {
		return [0, Math.min(this.a.len(), this.b.len())];
	}
}

/**
 * `BTreeSet::intersection`: each key of both in order, `a`'s of two equal ones, unless
 * `b` is the smaller by `TIPPING` times, which has its own looked up in `a`.
 */
export function btreeIntersection<K>(a: BTreeSet<K>, b: BTreeSet<K>): Iter<K> {
	if (a.n === 0 || b.n === 0) return iterEmpty();
	const [aMin, aMax] = ends(a);
	const [bMin, bMax] = ends(b);
	const low = a.c(aMin, bMax);
	const high = a.c(aMax, bMin);
	if (low > 0 || high < 0) return iterEmpty();
	if (low === 0) return iterOnce(aMin);
	if (high === 0) return iterOnce(aMax);
	if (a.n <= Math.floor(b.n / TIPPING)) return iterFilter(btreeKeys(a), (key) => btreeContains(b, key));
	if (b.n <= Math.floor(a.n / TIPPING)) return iterFilter(btreeKeys(b), (key) => btreeContains(a, key));
	return new Intersection(btreeKeys(a), btreeKeys(b), a.c);
}

/** `BTreeSet::is_subset`: whether `b` holds each key of `a`. */
export function btreeIsSubset<K>(a: BTreeSet<K>, b: BTreeSet<K>): boolean {
	if (a.n > b.n) return false;
	if (a.n === 0) return true;
	if (b.n === 0) return false;
	const [aMin, aMax] = ends(a);
	const [bMin, bMax] = ends(b);
	const keys = btreeKeys(a);
	const low = a.c(aMin, bMin);
	if (low < 0) return false;
	if (low === 0) keys.next();
	const high = a.c(aMax, bMax);
	if (high > 0) return false;
	if (high === 0) keys.nextBack();
	if (keys.len() <= Math.floor(b.n / TIPPING)) {
		for (const key of keys) if (!btreeContains(b, key)) return false;
		return true;
	}
	// Each of `a`'s found by stepping through `b`'s between their ends, which `a`'s left lack.
	const others = btreeKeys(b);
	others.next();
	others.nextBack();
	for (const key of keys) {
		let order: Ordering = 1;
		for (let other = others.next(); other !== DONE; other = others.next()) {
			order = a.c(other, key);
			if (order >= 0) break;
		}
		if (order !== 0) return false;
	}
	return true;
}

/** `BTreeSet::is_superset`: whether `a` holds each key of `b`. */
export function btreeIsSuperset<K>(a: BTreeSet<K>, b: BTreeSet<K>): boolean {
	return btreeIsSubset(b, a);
}

/** `BTreeSet::is_disjoint`: whether their intersection yields nothing. */
export function btreeIsDisjoint<K>(a: BTreeSet<K>, b: BTreeSet<K>): boolean {
	return btreeIntersection(a, b).next() === DONE;
}

/**
 * std's sorted bulk build of `m`, an empty set, which its operators make: of `keys`, in
 * order, but for each that `same`, `!ne`, finds equal to the next, which drops. It reads
 * the next key before it tests one, as std's peeks. Without `same`, the keys are distinct.
 */
export function btreeFromSorted<K>(m: BTreeSet<K>, keys: Iter<K>, same?: Test<K>, k?: (key: K) => void): BTreeSet<K> {
	let held: K[];
	if (same === undefined) held = [...keys];
	else {
		held = [];
		for (let key = keys.next(); key !== DONE; ) {
			const next = keys.next();
			if (next === DONE || !same(key, next)) held.push(key);
			else k?.(key);
			key = next;
		}
	}
	build(
		m,
		held,
		held.map(() => undefined),
	);
	// std's bulk build makes the root even of no keys.
	m.r = true;
	return m;
}

// Entries

/**
 * `hash_map::Entry` and `btree_map::Entry`, and their `OccupiedEntry` and `VacantEntry`:
 * the map, the key, and where `entry` found it. The map stays as it was until the entry is
 * consumed, so nothing compares or hashes the key again, as nothing does in Rust.
 */
export interface Entry<K, V> {
	readonly m: HashMap<K, V> | BTreeMap<K, V>;
	/** The key it was given, or the map's own if it is occupied. */
	readonly k: K;
	/** Whether it is occupied: whether the map held the key. */
	readonly o: boolean;
	/** A hashed key's slot if it is occupied, and its hash. */
	readonly s: Slot<K, V> | undefined;
	readonly h: number | undefined;
	/** A B-tree's leaf, and the key's index in it, or the complement of where it goes. */
	readonly l: Leaf<K, V> | undefined;
	readonly i: number;
}

/**
 * `entry`: `key` drops at once if the map holds an equal one. A hashed key is hashed into
 * an empty map too, as std's is.
 */
export function entry<K, V>(m: HashMap<K, V> | BTreeMap<K, V>, key: K, k?: (key: K) => void): Entry<K, V> {
	const e = locate(m, key);
	if (e.o) k?.(key);
	return e;
}

function locate<K, V>(m: HashMap<K, V> | BTreeMap<K, V>, key: K): Entry<K, V> {
	if (!isHash(m)) {
		const l = leafOf(m, key);
		const i = search(l.k, key, m.c);
		return { m, k: i < 0 ? key : l.k[i], o: i >= 0, s: undefined, h: undefined, l, i };
	}
	if (!hashed(m, key)) return { m, k: key, o: m.p.has(key), s: undefined, h: undefined, l: undefined, i: 0 };
	const h = (m.h as Hashing<K>).hash(key);
	const s = slot(m, key, h);
	return { m, k: s === undefined ? key : s.k, o: s !== undefined, s, h, l: undefined, i: 0 };
}

/** Insert `value` where `e`, a vacant entry, goes: its place. */
function fill<K, V>(e: Entry<K, V>, value: V): Place<V> {
	const m = e.m;
	if (isHash(m)) {
		if (e.h !== undefined) return add(m, e.k, value, e.h);
		m.p.set(e.k, value);
		return new AtKey(m.p, e.k);
	}
	const l = e.l as Leaf<K, V>;
	const i = ~e.i;
	insertAt(m, l, i, e.k, value);
	// A split keeps the leaf's first half and moves the rest to the next leaf.
	return i < l.k.length ? new AtIndex(l.v, i) : new AtIndex((l.next as Leaf<K, V>).v, i - l.k.length);
}

/** Drop `e`, an `Entry`: a vacant one holds the key it was given. */
export function entryDrop<K, V>(e: Entry<K, V>, k: (key: K) => void): void {
	if (!e.o) k(e.k);
}

function isHash<K, V>(m: HashMap<K, V> | BTreeMap<K, V>): m is HashMap<K, V> {
	return "p" in m;
}

/** `Entry::or_insert_with`: the value there, or else `f()`'s, inserted. */
export function entryOrInsertWith<K, V>(e: Entry<K, V>, f: (key: K) => V): V {
	if (e.o) return entryGet(e);
	const value = f(e.k);
	fill(e, value);
	return value;
}

/** `Entry::or_insert`: the value there, and `value` drops, or else `value`, inserted. */
export function entryOrInsert<K, V>(e: Entry<K, V>, value: V, v?: (value: V) => void): V {
	if (!e.o) {
		fill(e, value);
		return value;
	}
	v?.(value);
	return entryGet(e);
}

/** Whether `e` is `Entry::Occupied`: whether the map holds its key. */
export function entryOccupied<K, V>(e: Entry<K, V>): boolean {
	return e.o;
}

/** `Entry::and_modify` of a value JS copies: `f` of its place, if any. */
export function entryAndModifyMut<K, V>(e: Entry<K, V>, f: (value: Place<V>) => void): Entry<K, V> {
	if (e.o) f(entryGetMut(e));
	return e;
}

/** `Entry::and_modify`: `f` of the value there, if any. */
export function entryAndModify<K, V>(e: Entry<K, V>, f: (value: V) => void): Entry<K, V> {
	if (e.o) f(entryGet(e));
	return e;
}

/** `Entry::key` and `OccupiedEntry::key`: the key the map holds, or else the one looked up. */
export function entryKey<K, V>(e: Entry<K, V>): K {
	return e.k;
}

/** `OccupiedEntry::get` and `get_mut`. */
export function entryGet<K, V>(e: Entry<K, V>): V {
	if (e.s !== undefined) return e.s.v;
	if (e.l !== undefined) return e.l.v[e.i];
	return (e.m as HashMap<K, V>).p.get(e.k) as V;
}

/** `OccupiedEntry::get_mut` and `into_mut` of a value JS copies: its place. */
export function entryGetMut<K, V>(e: Entry<K, V>): Place<V> {
	if (e.s !== undefined) return e.s;
	if (e.l !== undefined) return new AtIndex(e.l.v, e.i);
	return new AtKey((e.m as HashMap<K, V>).p, e.k);
}

/**
 * `Entry::or_insert_with` and `or_insert_with_key` of a value JS copies, and
 * `or_default`: its place, after inserting `f`'s value if the map lacks the key.
 */
export function entryOrInsertWithMut<K, V>(e: Entry<K, V>, f: (key: K) => V): Place<V> {
	return e.o ? entryGetMut(e) : fill(e, f(e.k));
}

/** `Entry::or_insert` and `VacantEntry::insert` of a value JS copies: its place, and `value` drops if one was there. */
export function entryOrInsertMut<K, V>(e: Entry<K, V>, value: V, v?: (value: V) => void): Place<V> {
	if (!e.o) return fill(e, value);
	const place = entryGetMut(e);
	v?.(value);
	return place;
}

/** `OccupiedEntry::insert`: the old value. */
export function entryInsert<K, V>(e: Entry<K, V>, value: V): V {
	const place = entryGetMut(e);
	const old = place.v;
	place.v = value;
	return old;
}

/** `OccupiedEntry::remove_entry`. */
export function entryRemoveEntry<K, V>(e: Entry<K, V>): [K, V] {
	const m = e.m;
	const value = entryGet(e);
	if (!isHash(m)) removeAt(m, e.l as Leaf<K, V>, e.i);
	else if (e.s !== undefined) unlink(m, e.h as number, e.s);
	else m.p.delete(e.k);
	return [e.k, value];
}

/** `OccupiedEntry::remove`, whose key drops. */
export function entryRemove<K, V>(e: Entry<K, V>, k?: (key: K) => void): V {
	const [key, value] = entryRemoveEntry(e);
	k?.(key);
	return value;
}

/** `first_entry`, and with `last`, `last_entry`: an occupied entry, if the map has one. */
export function btreeFirstEntry<K, V>(m: BTreeMap<K, V>, last = false): Entry<K, V> | undefined {
	if (m.n === 0) return undefined;
	const l = end(m, last);
	const i = last ? l.k.length - 1 : 0;
	return { m, k: l.k[i], o: true, s: undefined, h: undefined, l, i };
}

// VecDeque

/**
 * `VecDeque<T>`: a ring buffer of `b.length` slots, a power of two, holding `n` items
 * from `h` on, wrapping past the end.
 */
export interface VecDeque<T> {
	b: (T | undefined)[];
	h: number;
	n: number;
}

/** `VecDeque::new` and `with_capacity`. */
export function dequeNew<T>(): VecDeque<T> {
	return { b: [], h: 0, n: 0 };
}

/** The slot of item `i`. */
function at<T>(q: VecDeque<T>, i: number): number {
	return (q.h + i) & (q.b.length - 1);
}

/** Room for one more item. */
function grow<T>(q: VecDeque<T>): void {
	if (q.n < q.b.length) return;
	const b = new Array(Math.max(4, q.b.length * 2));
	for (let i = 0; i < q.n; i++) b[i] = q.b[at(q, i)];
	q.b = b;
	q.h = 0;
}

export function dequePushBack<T>(q: VecDeque<T>, item: T): void {
	grow(q);
	q.b[at(q, q.n++)] = item;
}

export function dequePushFront<T>(q: VecDeque<T>, item: T): void {
	grow(q);
	q.h = (q.h - 1) & (q.b.length - 1);
	q.b[q.h] = item;
	q.n++;
}

/** `pop_front`: the first item, or else `none`. */
export function dequePopFront<T>(q: VecDeque<T>): T | undefined;
export function dequePopFront<T>(q: VecDeque<T>, none: None): T | None;
export function dequePopFront<T>(q: VecDeque<T>, none?: None): T | None | undefined {
	if (q.n === 0) return none;
	const item = q.b[q.h];
	q.b[q.h] = undefined;
	q.h = at(q, 1);
	q.n--;
	return item;
}

/** `pop_back`: the last item, or else `none`. */
export function dequePopBack<T>(q: VecDeque<T>): T | undefined;
export function dequePopBack<T>(q: VecDeque<T>, none: None): T | None;
export function dequePopBack<T>(q: VecDeque<T>, none?: None): T | None | undefined {
	if (q.n === 0) return none;
	const i = at(q, --q.n);
	const item = q.b[i];
	q.b[i] = undefined;
	return item;
}

/** `get` and `get_mut`: the item at `i`, or else `none`. */
export function dequeGet<T>(q: VecDeque<T>, i: number): T | undefined;
export function dequeGet<T>(q: VecDeque<T>, i: number, none: None): T | None;
export function dequeGet<T>(q: VecDeque<T>, i: number, none?: None): T | None | undefined {
	return i < q.n ? (q.b[at(q, i)] as T) : none;
}

/** `front`, and with `back`, `back`, or else `none`. */
export function dequeEnd<T>(q: VecDeque<T>, back?: boolean): T | undefined;
export function dequeEnd<T>(q: VecDeque<T>, back: boolean, none: None): T | None;
export function dequeEnd<T>(q: VecDeque<T>, back = false, none?: None): T | None | undefined {
	return q.n === 0 ? none : (q.b[at(q, back ? q.n - 1 : 0)] as T);
}

/** `get_mut` of an item JS copies: its place, or `undefined` past the end. */
export function dequeGetMut<T>(q: VecDeque<T>, i: number): Place<T> | undefined {
	return i < q.n ? new AtIndex(q.b as T[], at(q, i)) : undefined;
}

/** `front_mut` of an item JS copies, and with `back`, `back_mut`. */
export function dequeEndMut<T>(q: VecDeque<T>, back = false): Place<T> | undefined {
	return q.n === 0 ? undefined : new AtIndex(q.b as T[], at(q, back ? q.n - 1 : 0));
}

/** `index` and `index_mut`: `q[i]`. */
export function dequeIndex<T>(q: VecDeque<T>, i: number): T {
	if (i >= q.n) panic("out of bounds access");
	return q.b[at(q, i)] as T;
}

/** `q[i] = item`, through `index_mut`. */
export function dequeSet<T>(q: VecDeque<T>, i: number, item: T): void {
	if (i >= q.n) panic("out of bounds access");
	q.b[at(q, i)] = item;
}

/** Drop `q`: each item by `f`, front to back. */
export function dequeDrop<T>(q: VecDeque<T>, f: (item: T) => void): void {
	const items: T[] = [];
	for (let i = 0; i < q.n; i++) items.push(q.b[at(q, i)] as T);
	dropAll(items, f);
}

/** `clear`, which leaves the deque empty and then drops each item, front to back. */
export function dequeClear<T>(q: VecDeque<T>, f?: (item: T) => void): void {
	const old: VecDeque<T> = { b: q.b, h: q.h, n: q.n };
	q.b = [];
	q.h = 0;
	q.n = 0;
	if (f !== undefined) dequeDrop(old, f);
}

/** `truncate`: the first `len` items, if there are more, and then each other drops in order. */
export function dequeTruncate<T>(q: VecDeque<T>, len: number, f?: (item: T) => void): void {
	if (f === undefined || q.n <= len) {
		while (q.n > len) dequePopBack(q);
		return;
	}
	const cut: T[] = [];
	for (let i = len; i < q.n; i++) cut.push(q.b[at(q, i)] as T);
	while (q.n > len) dequePopBack(q);
	dropAll(cut, f);
}

/**
 * `retain`, which visits the items in order and keeps each `f` returns true for. It swaps
 * each kept item into place, as std's does.
 */
export function dequeRetain<T>(q: VecDeque<T>, f: (item: T) => boolean, drop?: (item: T) => void): void {
	retainDeque(q, f as (item: T | Place<T>) => boolean, false, drop);
}

/** `retain_mut` of items JS copies: `f` takes the place of each. */
export function dequeRetainMut<T>(q: VecDeque<T>, f: (item: Place<T>) => boolean, drop?: (item: T) => void): void {
	retainDeque(q, f as (item: T | Place<T>) => boolean, true, drop);
}

/** `retain`, handing `f` each item or, if `lend`, its place. */
function retainDeque<T>(q: VecDeque<T>, f: (item: T | Place<T>) => boolean, lend: boolean, drop?: (item: T) => void): void {
	let kept = 0;
	for (let i = 0; i < q.n; i++) {
		const keep = f(lend ? new AtIndex(q.b as T[], at(q, i)) : (q.b[at(q, i)] as T));
		const item = q.b[at(q, i)] as T;
		if (!keep) continue;
		q.b[at(q, i)] = q.b[at(q, kept)];
		q.b[at(q, kept++)] = item;
	}
	dequeTruncate(q, kept, drop);
}

/** `contains`, by `eq`. */
export function dequeContains<T>(q: VecDeque<T>, item: T, eq: (a: T, b: T) => boolean): boolean {
	for (let i = 0; i < q.n; i++) if (eq(q.b[at(q, i)] as T, item)) return true;
	return false;
}

/**
 * `swap_remove_front`, and with `back`, `swap_remove_back`: the item at `i`, with the
 * front or back item moved into its place, or else `none`.
 */
export function dequeSwapRemove<T>(q: VecDeque<T>, i: number, back?: boolean): T | undefined;
export function dequeSwapRemove<T>(q: VecDeque<T>, i: number, back: boolean, none: None): T | None;
export function dequeSwapRemove<T>(q: VecDeque<T>, i: number, back = false, none?: None): T | None | undefined {
	if (i >= q.n) return none;
	const [j, end] = [at(q, i), at(q, back ? q.n - 1 : 0)];
	[q.b[j], q.b[end]] = [q.b[end], q.b[j]];
	return (back ? dequePopBack(q) : dequePopFront(q)) as T;
}

/** `remove`: the item at `i`, with those after it moved up, or else `none`. */
export function dequeRemove<T>(q: VecDeque<T>, i: number): T | undefined;
export function dequeRemove<T>(q: VecDeque<T>, i: number, none: None): T | None;
export function dequeRemove<T>(q: VecDeque<T>, i: number, none?: None): T | None | undefined {
	if (i >= q.n) return none;
	const item = q.b[at(q, i)] as T;
	for (let j = i; j + 1 < q.n; j++) q.b[at(q, j)] = q.b[at(q, j + 1)];
	dequePopBack(q);
	return item;
}

/** `insert`: `item` at `i`, with those from `i` on moved down. */
export function dequeInsert<T>(q: VecDeque<T>, i: number, item: T): void {
	if (i > q.n) panic("index out of bounds");
	dequePushBack(q, item);
	for (let j = q.n - 1; j > i; j--) q.b[at(q, j)] = q.b[at(q, j - 1)];
	q.b[at(q, i)] = item;
}

/**
 * An iterator of a deque's items from index `front` up to `back`, which drops those it
 * still holds by `drop` as it drops, if it owns them, as a `drain` does.
 */
class DequeIter<T> extends Iter<T> implements Disposable {
	private readonly q: VecDeque<T>;
	private front: number;
	private back: number;
	private readonly drop: ((item: T) => void) | undefined;

	constructor(q: VecDeque<T>, front: number, back: number, drop?: (item: T) => void) {
		super();
		this.q = q;
		this.front = front;
		this.back = back;
		this.drop = drop;
	}

	next(): T | Done {
		return this.front < this.back ? (this.q.b[at(this.q, this.front++)] as T) : DONE;
	}

	override remaining(): Iterable<T> {
		return new DequeIter(this.q, this.front, this.back);
	}

	/** std's: where it is, or of `into_iter`, what it has left, each cloned in order by `h.v`. */
	override clone(h?: Clones): Iter<T> {
		if (h?.v === undefined) return new DequeIter(this.q, this.front, this.back, this.drop);
		const clone = h.v as (item: T) => T;
		const items: T[] = [];
		for (let i = this.front; i < this.back; i++) items.push(clone(this.q.b[at(this.q, i)] as T));
		return iterSlice(items, this.drop);
	}

	override nextBack(): T | Done {
		return this.front < this.back ? (this.q.b[at(this.q, --this.back)] as T) : DONE;
	}

	override len(): number {
		return this.back - this.front;
	}

	override sizeHint(): SizeHint {
		return exactHint(this.len());
	}

	[Symbol.dispose](): void {
		if (this.drop === undefined) return;
		const items: T[] = [];
		while (this.front < this.back) items.push(this.q.b[at(this.q, this.front++)] as T);
		dropAll(items, this.drop);
	}
}

/**
 * `iter` and `iter_mut`, and with what drops each item, `into_iter`, whose `last`,
 * `count`, and skips are a `Vec`'s.
 */
export function dequeIter<T>(q: VecDeque<T>, drop?: (item: T) => void): Iter<T> {
	if (drop === undefined) return new DequeIter(q, 0, q.n);
	const items: T[] = [];
	for (let i = 0; i < q.n; i++) items.push(q.b[at(q, i)] as T);
	return iterSlice(items, drop);
}

/** `iter_mut` of items JS copies: the place of each. */
export function dequeIterMut<T>(q: VecDeque<T>): Iter<Place<T>> {
	return new DequePlaces(q);
}

/** A deque's `iter_mut` of items JS copies, which lends each as its place. */
class DequePlaces<T> extends Iter<Place<T>> {
	private readonly q: VecDeque<T>;
	private front = 0;
	private back: number;

	constructor(q: VecDeque<T>) {
		super();
		this.q = q;
		this.back = q.n;
	}

	next(): Place<T> | Done {
		return this.front < this.back ? new AtIndex(this.q.b as T[], at(this.q, this.front++)) : DONE;
	}

	override nextBack(): Place<T> | Done {
		return this.front < this.back ? new AtIndex(this.q.b as T[], at(this.q, --this.back)) : DONE;
	}

	override len(): number {
		return this.back - this.front;
	}

	override sizeHint(): SizeHint {
		return exactHint(this.len());
	}
}

/** `drain`: the items within `r`, taken out at once, which drop by `drop` as `dequeIter`'s. */
export function dequeDrain<T>(q: VecDeque<T>, r: Bounds<number>, drop?: (item: T) => void): Iter<T> {
	const [start, end] = boundsRange(r, q.n);
	const taken: VecDeque<T> = { b: [], h: 0, n: 0 };
	for (let i = start; i < end; i++) dequePushBack(taken, q.b[at(q, i)] as T);
	for (let i = end; i < q.n; i++) q.b[at(q, i - (end - start))] = q.b[at(q, i)];
	dequeTruncate(q, q.n - (end - start));
	return new DequeIter(taken, 0, taken.n, drop);
}

/** `slice::range`: the start and end of `r` within `n` elements, panicking as std's does. */
export function boundsRange(r: Bounds<number>, n: number): [number, number] {
	const [lo, hi] = bounds(r);
	let end = n;
	if (hi.tag !== "Unbounded") {
		end = hi._0;
		if (hi.tag === "Included" ? end >= n : end > n) sliceIndexFail(0, end, n);
		if (hi.tag === "Included") end++;
	}
	let start = 0;
	if (lo.tag !== "Unbounded") {
		start = lo._0;
		if (lo.tag === "Excluded" ? start >= end : start > end) sliceIndexFail(start, end, n);
		if (lo.tag === "Excluded") start++;
	}
	return [start, end];
}

/** `slice::range` itself: `r` within `n` elements, as `{ start, end }`, panicking as std's does. */
export function boundsChecked(r: Bounds<number>, n: number): { start: number; end: number } {
	const [start, end] = boundsRange(r, n);
	return { start, end };
}

/**
 * A slice's index `r` other than `a..b`, `a..`, `..b`, and `..`, as the start and end of
 * the part it reads within `n` elements, panicking as std's `index` does.
 */
export function sliceIndexBounds(r: Bounds<number>, n: number): [number, number] {
	if (!inclusive(r)) return boundsRange(r, n);
	// std checks `a..=b` by its own fields, not by the bounds it gives.
	if (r.hi >= n) sliceIndexFail(r.lo, r.hi, n);
	const bounds = sliceGetBounds(r, n);
	if (bounds[0] > bounds[1]) sliceIndexFail(bounds[0], bounds[1], n);
	return bounds;
}

/**
 * A slice's index `r`, as `sliceIndexBounds` reads it, unchecked: what `get` reads, which
 * gives `None` unless the start is at most the end and the end at most `n`.
 */
export function sliceGetBounds(r: Bounds<number>, n: number): [number, number] {
	// An `a..=b` iterated to its end starts past it, at `b + 1`, which JS doesn't overflow.
	if (inclusive(r)) return [r.exhausted ? r.hi + 1 : r.lo, r.hi + 1];
	const [lo, hi] = bounds(r);
	const start = lo.tag === "Unbounded" ? 0 : lo.tag === "Included" ? lo._0 : lo._0 + 1;
	const end = hi.tag === "Unbounded" ? n : hi.tag === "Included" ? hi._0 + 1 : hi._0;
	return [start, end];
}

/** Whether `r` is an `a..=b`: a field test, not `instanceof`, so a bundle without one leaves the class out. */
function inclusive<K>(r: Bounds<K>): r is RangeInclusive<K> {
	return typeof r === "object" && "exhausted" in r;
}

/** std's `slice_index_fail`, which names the end where neither bound is out of order. */
function sliceIndexFail(start: number, end: number, n: number): never {
	rangeCheck(n, start, end);
	return panic(`range end index ${end} out of range for slice of length ${n}`);
}

// `Vec::drain`, which leaves the vector only what comes before the range until the drain
// drops and moves the tail back, so a drain that never drops, as `mem::forget` leaves
// it, leaks the rest, as std's does.

/**
 * `Vec::drain`: the elements within `r`. Those it still holds as it drops each drop by
 * `drop`, front to back, before the tail moves back.
 */
export function vecDrain<T>(v: T[], r: Bounds<number>, drop?: (item: T) => void): Iter<T> {
	const [start, end] = boundsRange(r, v.length);
	return new VecDrain(v, v.splice(start), end - start, drop);
}

/** A `Vec`'s drain: of `rest`, what followed the range's start, the range, then the tail. */
class VecDrain<T> extends Iter<T> implements Disposable {
	private readonly v: T[];
	private readonly rest: T[];
	private front = 0;
	private back: number;
	/** Where the tail starts in `rest`. */
	private readonly tail: number;
	private readonly drop: ((item: T) => void) | undefined;

	constructor(v: T[], rest: T[], tail: number, drop?: (item: T) => void) {
		super();
		this.v = v;
		this.rest = rest;
		this.back = tail;
		this.tail = tail;
		this.drop = drop;
	}

	next(): T | Done {
		return this.front < this.back ? this.rest[this.front++] : DONE;
	}

	override remaining(): Iterable<T> {
		return this.rest.slice(this.front, this.back);
	}

	override nextBack(): T | Done {
		return this.front < this.back ? this.rest[--this.back] : DONE;
	}

	override len(): number {
		return this.back - this.front;
	}

	override sizeHint(): SizeHint {
		return exactHint(this.len());
	}

	[Symbol.dispose](): void {
		const from = this.front;
		this.front = this.back;
		if (this.drop !== undefined) dropRange(this.rest, this.drop, from, this.back);
		for (let i = this.tail; i < this.rest.length; i++) this.v.push(this.rest[i]);
		this.rest.length = this.tail;
	}
}

/** `Vec::drain` of bytes. */
export function byteVecDrain(v: ByteVec, r: Bounds<number>): Iter<number> {
	const [start, end] = boundsRange(r, v.length);
	const length = v.length;
	v.length = start;
	return new ByteVecDrain(v, start, end, length);
}

/** A `Vec<u8>`'s drain, which reads the range from the vector's storage in place. */
class ByteVecDrain extends Iter<number> implements Disposable {
	private readonly v: ByteVec;
	private readonly start: number;
	private front: number;
	private back: number;
	private readonly end: number;
	/** The vector's length before the drain, or -1 once the tail moved back. */
	private length: number;

	constructor(v: ByteVec, start: number, end: number, length: number) {
		super();
		this.v = v;
		this.start = start;
		this.front = start;
		this.back = end;
		this.end = end;
		this.length = length;
	}

	next(): number | Done {
		return this.front < this.back ? this.v.data[this.front++] : DONE;
	}

	override remaining(): Iterable<number> {
		return this.v.data.slice(this.front, this.back);
	}

	override nextBack(): number | Done {
		return this.front < this.back ? this.v.data[--this.back] : DONE;
	}

	override len(): number {
		return this.back - this.front;
	}

	override sizeHint(): SizeHint {
		return exactHint(this.len());
	}

	[Symbol.dispose](): void {
		if (this.length < 0) return;
		this.v.data.copyWithin(this.start, this.end, this.length);
		this.v.length = this.start + this.length - this.end;
		this.length = -1;
	}
}

/**
 * `extend`, and `collect` into a deque through `dequeNew`, which takes `items` as `take`
 * says and reserves as `iterExtend` does.
 */
export function dequeExtend<T>(q: VecDeque<T>, items: Iter<T>, take?: Take, size?: number, trusted = false): VecDeque<T> {
	return vecExtending(q, q.n, items, (item) => dequePushBack(q, item), take, size, trusted, true);
}

/** `clone`, with each item through its `Clone`, or as itself without one. */
export function dequeClone<T>(q: VecDeque<T>, item: (value: T) => T = itself): VecDeque<T> {
	const out = dequeNew<T>();
	for (let i = 0; i < q.n; i++) dequePushBack(out, item(q.b[at(q, i)] as T));
	return out;
}

// `BinaryHeap`: std's max-heap in an array, sifted as std's sifts it, so equal elements
// leave in std's order and each comparison std makes runs. Every function that sifts is
// given the elements' `PartialOrd` tests std's sifts make: `le` up, and `le`, `ge`, and
// `lt` down.

/** A `PartialOrd` test. */
type Test<T> = (a: T, b: T) => boolean;

/** `BinaryHeap<T>`: its elements in `d`, in std's order. */
export interface BinaryHeap<T> {
	d: T[];
}

/** `BinaryHeap::new`, `with_capacity`, and `default`. */
export function heapNew<T>(): BinaryHeap<T> {
	return { d: [] };
}

/** `From<Vec<T>>`: the vector's elements, rebuilt into a heap in place, as std's are. */
export function heapFrom<T>(d: T[], le: Test<T>, ge: Test<T>, lt: Test<T>): BinaryHeap<T> {
	rebuild(d, le, ge, lt);
	return { d };
}

/** `push`: `item` sifted up from the end. */
export function heapPush<T>(h: BinaryHeap<T>, item: T, le: Test<T>): void {
	h.d.push(item);
	siftUp(h.d, 0, h.d.length - 1, le);
}

/** `peek` of an element that can be `undefined`: the greatest element, or `none` if empty. */
export function heapPeek<T>(h: BinaryHeap<T>, none: None): T | None {
	return h.d.length === 0 ? none : h.d[0];
}

/** `pop`: the greatest element, or `none` if empty, the last element sifted into its place to the bottom and back up. */
export function heapPop<T>(h: BinaryHeap<T>, le: Test<T>): T | undefined;
export function heapPop<T>(h: BinaryHeap<T>, le: Test<T>, none: None): T | None;
export function heapPop<T>(h: BinaryHeap<T>, le: Test<T>, none?: None): T | None | undefined {
	const d = h.d;
	if (d.length === 0) return none;
	let item = d.pop() as T;
	if (d.length > 0) {
		[item, d[0]] = [d[0], item];
		siftDownToBottom(d, 0, le);
	}
	return item;
}

/** `into_sorted_vec`: the elements in ascending order, each greatest swapped to the end and the rest sifted. */
export function heapIntoSortedVec<T>(h: BinaryHeap<T>, le: Test<T>, ge: Test<T>, lt: Test<T>): T[] {
	const d = h.d;
	for (let end = d.length - 1; end > 0; end--) {
		[d[0], d[end]] = [d[end], d[0]];
		siftDownRange(d, 0, end, le, ge, lt);
	}
	return d;
}

/** `append`: `other`'s elements added to the larger of the two, which `h` then holds, and `other` left empty. */
export function heapAppend<T>(h: BinaryHeap<T>, other: BinaryHeap<T>, le: Test<T>, ge: Test<T>, lt: Test<T>): void {
	if (h.d.length < other.d.length) [h.d, other.d] = [other.d, h.d];
	const start = h.d.length;
	for (const item of other.d) h.d.push(item);
	other.d = [];
	rebuildTail(h.d, start, le, ge, lt);
}

/**
 * `extend`, taking `items` as `take` says and reserving as its `Vec`'s `iterExtend` does,
 * and then the heap rebuilt past what it held.
 */
export function heapExtend<T>(h: BinaryHeap<T>, items: Iter<T>, le: Test<T>, ge: Test<T>, lt: Test<T>, take?: Take, size?: number, trusted = false): BinaryHeap<T> {
	const start = h.d.length;
	vecExtending(h, start, items, (item) => h.d.push(item), take, size, trusted);
	rebuildTail(h.d, start, le, ge, lt);
	return h;
}

/**
 * `retain`: the elements `f` keeps, in order, each other dropped by `drop` as `f` rejects
 * it, and then the heap rebuilt from the first it rejected.
 */
export function heapRetain<T>(h: BinaryHeap<T>, f: (item: T) => boolean, le: Test<T>, ge: Test<T>, lt: Test<T>, drop?: (item: T) => void): void {
	const d = h.d;
	const n = d.length;
	let from = n;
	let kept = 0;
	for (let i = 0; i < n; i++) {
		const item = d[i];
		if (f(item)) d[kept++] = item;
		else {
			if (from === n) from = i;
			drop?.(item);
		}
	}
	d.length = kept;
	rebuildTail(d, from, le, ge, lt);
}

/** `clear`, which leaves the heap empty and then drops each element in order. */
export function heapClear<T>(h: BinaryHeap<T>, drop?: (item: T) => void): void {
	const d = h.d;
	h.d = [];
	dropAll(d, drop);
}

/** `drain`: an iterator of the elements in order, which leaves the heap empty and drops those it still holds by `drop`. */
export function heapDrain<T>(h: BinaryHeap<T>, drop?: (item: T) => void): Iter<T> {
	const d = h.d;
	h.d = [];
	return iterSlice(d, drop);
}

/** `iter` and `into_iter`: the elements in order, any not read dropped by `drop` as the iterator drops. */
export function heapIter<T>(h: BinaryHeap<T>, drop?: (item: T) => void): Iter<T> {
	return iterSlice(h.d, drop);
}

/** The heap's drop: each element in order. */
export function heapDrop<T>(h: BinaryHeap<T>, drop: (item: T) => void): void {
	dropAll(h.d, drop);
}

/** `clone`, with each element through its `Clone`, or as itself without one. */
export function heapClone<T>(h: BinaryHeap<T>, item: (value: T) => T = itself): BinaryHeap<T> {
	return { d: h.d.map((value) => item(value)) };
}

/**
 * `PeekMut`, the greatest element lent: a place, which writes it, of a heap with at least
 * one. Once `DerefMut` lends it of a heap of more, its drop sifts it down.
 */
export class PeekMut<T> implements Place<T> {
	readonly h: BinaryHeap<T>;
	readonly le: Test<T>;
	readonly ge: Test<T>;
	readonly lt: Test<T>;
	/** The heap's elements `DerefMut` hid behind a heap of the top alone, as std's `set_len(1)` does, so a forgotten guard leaks them as std's does. */
	t: T[] | undefined;

	constructor(h: BinaryHeap<T>, le: Test<T>, ge: Test<T>, lt: Test<T>) {
		this.h = h;
		this.le = le;
		this.ge = ge;
		this.lt = lt;
	}

	get v(): T {
		return this.h.d[0];
	}

	set v(value: T) {
		this.h.d[0] = value;
	}
}

/** `peek_mut`: the greatest element lent, or `undefined` if empty. */
export function heapPeekMut<T>(h: BinaryHeap<T>, le: Test<T>, ge: Test<T>, lt: Test<T>): PeekMut<T> | undefined {
	return h.d.length === 0 ? undefined : new PeekMut(h, le, ge, lt);
}

/** `PeekMut`'s `DerefMut`, which hides the rest of a heap of more than the one element until the guard drops. */
export function peekMutDerefMut<T>(p: PeekMut<T>): void {
	const d = p.h.d;
	if (d.length > 1) {
		p.t = d;
		p.h.d = [d[0]];
	}
}

/** The elements `DerefMut` hid back in the heap, with the top as written; whether there were any. */
function peekMutRestore<T>(p: PeekMut<T>): boolean {
	const t = p.t;
	if (t === undefined) return false;
	t[0] = p.h.d[0];
	p.h.d = t;
	p.t = undefined;
	return true;
}

/** `PeekMut`'s drop: the element sifted down from the top, if `DerefMut` lent it. */
export function peekMutDrop<T>(p: PeekMut<T>): void {
	if (peekMutRestore(p)) siftDownRange(p.h.d, 0, p.h.d.length, p.le, p.ge, p.lt);
}

/** `PeekMut::pop`: the heap's `pop`, which leaves the drop nothing to sift. */
export function peekMutPop<T>(p: PeekMut<T>): T {
	peekMutRestore(p);
	return heapPop(p.h, p.le) as T;
}

/** std's `sift_up`: the element at `pos` moved up past each parent it is not `le`, but not above `start`. */
function siftUp<T>(d: T[], start: number, pos: number, le: Test<T>): number {
	const item = d[pos];
	while (pos > start) {
		const parent = (pos - 1) >>> 1;
		if (le(item, d[parent])) break;
		d[pos] = d[parent];
		pos = parent;
	}
	d[pos] = item;
	return pos;
}

/** std's `sift_down_range`: the element at `pos` moved down past each greater child, within the first `end`. */
function siftDownRange<T>(d: T[], pos: number, end: number, le: Test<T>, ge: Test<T>, lt: Test<T>): number {
	const item = d[pos];
	let child = 2 * pos + 1;
	while (child <= Math.max(end - 2, 0)) {
		if (le(d[child], d[child + 1])) child++;
		if (ge(item, d[child])) {
			d[pos] = item;
			return pos;
		}
		d[pos] = d[child];
		pos = child;
		child = 2 * pos + 1;
	}
	if (child === end - 1 && lt(item, d[child])) {
		d[pos] = d[child];
		pos = child;
	}
	d[pos] = item;
	return pos;
}

/** std's `sift_down_to_bottom`: the element at `pos` moved down to a leaf past the greater child at each level, then sifted up. */
function siftDownToBottom<T>(d: T[], pos: number, le: Test<T>): void {
	const end = d.length;
	const start = pos;
	const item = d[pos];
	let child = 2 * pos + 1;
	while (child <= Math.max(end - 2, 0)) {
		if (le(d[child], d[child + 1])) child++;
		d[pos] = d[child];
		pos = child;
		child = 2 * pos + 1;
	}
	if (child === end - 1) {
		d[pos] = d[child];
		pos = child;
	}
	d[pos] = item;
	siftUp(d, start, pos, le);
}

/** std's `rebuild_tail`: the elements from `start` on added to the heap before them, by a rebuild or by sifting each up, as std's estimate picks. */
function rebuildTail<T>(d: T[], start: number, le: Test<T>, ge: Test<T>, lt: Test<T>): void {
	if (start === d.length) return;
	const tail = d.length - start;
	const better = start < tail || (d.length <= 2048 ? 2 * d.length < tail * (31 - Math.clz32(start)) : 2 * d.length < tail * 11);
	if (better) rebuild(d, le, ge, lt);
	else for (let i = start; i < d.length; i++) siftUp(d, 0, i, le);
}

/** std's `rebuild`: each parent sifted down, the last first. */
function rebuild<T>(d: T[], le: Test<T>, ge: Test<T>, lt: Test<T>): void {
	for (let n = d.length >>> 1; n > 0; ) {
		n--;
		siftDownRange(d, n, d.length, le, ge, lt);
	}
}

// Tuples of collections, which std's `Extend` of 2- through 12-tuples extends by each
// collection's `extend_reserve` and `extend_one` (core/src/iter/traits/collect.rs). A
// `Part` stands in for one collection's.

/**
 * A collection of type `C` as a tuple's `Extend` adds to it: `reserve` is its
 * `extend_reserve`, if it reserves, and `one` its `extend_one` of an item, giving the
 * collection back, which for a `String` is a new one.
 */
export interface Part<C, T> {
	reserve?(c: C, additional: number): void;
	one(c: C, item: T): C;
}

/**
 * `Extend::extend` of `t`, a tuple of collections, by `items`, and `collect` into the
 * tuple of their defaults: each reserves the lower bound of the size hint, if any, in
 * order, and then each item's fields extend them in order, by `for_each` as `take` says.
 */
export function tupleExtend<T extends unknown[]>(t: T, items: Iter<unknown[]>, parts: Part<unknown, unknown>[], take?: Take): T {
	tupleReserve(t, parts, items.sizeHint()[0]);
	return extending(t, items, (item) => tupleOne(t, parts, item), take);
}

/** A tuple's `extend_reserve` of `additional` more in each of its collections, where the size hint has a lower bound. */
function tupleReserve(t: unknown[], parts: Part<unknown, unknown>[], additional: number): void {
	if (additional === 0) return;
	for (let i = 0; i < parts.length; i++) parts[i].reserve?.(t[i], additional);
}

/** A tuple's `extend_one`: each field of `item` into its collection, in order. */
function tupleOne(t: unknown[], parts: Part<unknown, unknown>[], item: unknown[]): void {
	for (let i = 0; i < parts.length; i++) t[i] = parts[i].one(t[i], item[i]);
}

/** A tuple of collections within a tuple, which reserves and extends each of its own. */
export function tuplePart(parts: Part<unknown, unknown>[]): Part<unknown[], unknown[]> {
	return {
		reserve: (t, additional) => tupleReserve(t, parts, additional),
		one: (t, item) => {
			tupleOne(t, parts, item);
			return t;
		},
	};
}

/** A 1-tuple's `Extend::extend` of `t`, which is its collection's own `extend`, by `extend`, of each item's field. */
export function tupleExtendOne<C, T>(t: [C], items: Iter<[T]>, extend: (c: C, items: Iter<T>) => C): [C] {
	t[0] = extend(t[0], iterMap(items, (item) => item[0]));
	return t;
}

/** `part` of a borrowed `Copy` item, which it adds a `copy` of. */
export function copiedPart<C, T>(part: Part<C, T>, copy: (item: T) => T): Part<C, T> {
	return { reserve: part.reserve, one: (c, item) => part.one(c, copy(item)) };
}

/**
 * A collection kept in a vector, which holds `length(c)` of `size`-byte elements and
 * `push` adds to: it reserves as `vecReserve`, and from what it reserved, grows by one as
 * `vecPush` does, panicking past what it may hold.
 */
function vectorPart<C, T>(length: (c: C) => number, push: (c: C, item: T) => void, size?: number): Part<C, T> {
	let cap: number | undefined;
	return {
		reserve:
			size === undefined
				? undefined
				: (c, additional) => {
						cap = vecReserve(length(c), additional, size, cap);
					},
		one: (c, item) => {
			if (size !== undefined) cap = vecPush(length(c), size, cap);
			push(c, item);
			return c;
		},
	};
}

/** A `Vec` of `size`-byte elements, which pushes each. */
export function vecPart<T>(size?: number): Part<T[], T> {
	return vectorPart(
		(v) => v.length,
		(v, item) => v.push(item),
		size,
	);
}

/** A `Vec<u8>`, as `vecPart`. */
export function byteVecPart(): Part<ByteVec, number> {
	return vectorPart((v) => v.length, byteVecPush, 1);
}

/** A `VecDeque` of `size`-byte elements, which pushes each at the back. */
export function dequePart<T>(size?: number): Part<VecDeque<T>, T> {
	return vectorPart((q) => q.n, dequePushBack, size);
}

/** A `BinaryHeap` of `size`-byte elements, which pushes each, sifting it up by `le`. */
export function heapPart<T>(le: Test<T>, size?: number): Part<BinaryHeap<T>, T> {
	return vectorPart(
		(h) => h.d.length,
		(h, item) => heapPush(h, item, le),
		size,
	);
}

/** A `HashMap`, which inserts each entry as `hashExtend` does, and reserves as `hashReserve`. */
export function hashPart<K, V>(k?: (key: K) => void, v?: (value: V) => void, max?: number): Part<HashMap<K, V>, [K, V]> {
	return {
		reserve: (m, additional) => hashReserve(m, additional, max),
		one: (m, item) => {
			const old = hashInsert(m, item[0], item[1], k);
			if (old !== undefined) v?.(old);
			return m;
		},
	};
}

/** A `HashSet`, which adds each key as `hashExtendKeys` does. */
export function hashKeysPart<K>(k?: (key: K) => void, max?: number): Part<HashSet<K>, K> {
	return {
		reserve: (m, additional) => hashReserve(m, additional, max),
		one: (m, key) => {
			hashAdd(m, key, k);
			return m;
		},
	};
}

/** A `BTreeMap`, which inserts each entry as `btreeExtend` does, and never reserves. */
export function btreePart<K, V>(k?: (key: K) => void, v?: (value: V) => void): Part<BTreeMap<K, V>, [K, V]> {
	return {
		one: (m, item) => {
			const old = btreeInsert(m, item[0], item[1], k);
			if (old !== undefined) v?.(old);
			return m;
		},
	};
}

/** A `BTreeSet`, which adds each key as `btreeExtendKeys` does. */
export function btreeKeysPart<K>(k?: (key: K) => void): Part<BTreeSet<K>, K> {
	return {
		one: (m, key) => {
			btreeAdd(m, key, k);
			return m;
		},
	};
}

/** `()`, which extends by `()`s. */
export const UNIT_PART: Part<undefined, undefined> = { one: () => undefined };
