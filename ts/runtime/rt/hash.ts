// Hashing, for rust2-ts output: `DefaultHasher`, which is std's SipHash-1-3 byte for byte,
// `RandomState`, which keys it, and the `Hash` of std's types, which write what std's
// write. `rt.ts` re-exports all of it.
//
// A hash collection hashes a key that is a JS object with a faster hasher of its own,
// whose values nothing sees, as std keys a map's hasher at random.

import { ByteSlice, type ByteVec, type None } from "../rt.ts";
import { type I64Limbs, type I128Limbs, type U64Limbs, type U128Limbs, type u64, u64FromHalves } from "./int.ts";
import type { LinkedList } from "./collections.ts";
import type { Arc } from "./task.ts";

/** What `Hash` writes into: `DefaultHasher`, or a hash collection's own. */
export interface Hasher {
	/** Write `word`'s low `n` bytes, little-endian. */
	word(word: number, n: 1 | 2 | 4): void;
	/** Write `str` as `Hasher::write_str` does. */
	str(str: string): void;
	/** `Hasher::finish`, which leaves the hasher as it was. */
	finish(): u64;
}

/**
 * `DefaultHasher`: std's SipHash-1-3, its 64-bit lanes `v0` to `v3` kept as 32-bit halves,
 * then the tail of bytes not yet compressed, how many it holds, and the low byte of how
 * many were written, which is all `finish` reads of it.
 */
class SipHasher implements Hasher {
	private v0h: number;
	private v0l: number;
	private v1h: number;
	private v1l: number;
	private v2h: number;
	private v2l: number;
	private v3h: number;
	private v3l: number;
	private th = 0;
	private tl = 0;
	private ntail = 0;
	private length = 0;

	/** Keyed by `k0` and `k1`, as 32-bit halves. */
	constructor(k: RandomState) {
		this.v0h = k.k0h ^ 0x736f6d65;
		this.v0l = k.k0l ^ 0x70736575;
		this.v1h = k.k1h ^ 0x646f7261;
		this.v1l = k.k1l ^ 0x6e646f6d;
		this.v2h = k.k0h ^ 0x6c796765;
		this.v2l = k.k0l ^ 0x6e657261;
		this.v3h = k.k1h ^ 0x74656462;
		this.v3l = k.k1l ^ 0x79746573;
	}

	word(word: number, n: 1 | 2 | 4): void {
		this.length = (this.length + n) & 0xff;
		const at = this.ntail;
		const free = 8 - at;
		if (n < free) {
			this.put(word, at, n);
			this.ntail = at + n;
			return;
		}
		this.put(word, at, free);
		this.compress(this.th, this.tl);
		this.th = 0;
		this.tl = 0;
		if (n > free) this.put(word >>> (free * 8), 0, n - free);
		this.ntail = n - free;
	}

	str(str: string): void {
		// Its UTF-8 bytes, then 0xff, which no `str` holds, so no write is a prefix of another.
		for (let i = 0; i < str.length; i++) {
			const c = str.codePointAt(i) as number;
			if (c < 0x80) this.word(c, 1);
			else if (c < 0x800) this.word(0x80c0 | (c >> 6) | ((c & 0x3f) << 8), 2);
			else if (c < 0x10000) {
				this.word(0x80e0 | (c >> 12) | (((c >> 6) & 0x3f) << 8), 2);
				this.word(0x80 | (c & 0x3f), 1);
			} else {
				i++;
				this.word(0x808080f0 | (c >> 18) | (((c >> 12) & 0x3f) << 8) | (((c >> 6) & 0x3f) << 16) | ((c & 0x3f) << 24), 4);
			}
		}
		this.word(0xff, 1);
	}

	finish(): u64 {
		const v = LAST;
		v.v0h = this.v0h;
		v.v0l = this.v0l;
		v.v1h = this.v1h;
		v.v1l = this.v1l;
		v.v2h = this.v2h;
		v.v2l = this.v2l;
		v.v3h = this.v3h;
		v.v3l = this.v3l;
		v.compress(this.th | (this.length << 24), this.tl);
		v.v2l ^= 0xff;
		v.rounds(3);
		return u64FromHalves((v.v0h ^ v.v1h ^ v.v2h ^ v.v3h) >>> 0, (v.v0l ^ v.v1l ^ v.v2l ^ v.v3l) >>> 0);
	}

	/** Put `word`'s low `n` bytes into the tail, from its byte `at`. */
	private put(word: number, at: number, n: number): void {
		const bits = n * 8;
		const m = bits === 32 ? word : word & ((1 << bits) - 1);
		const shift = at * 8;
		if (shift >= 32) this.th |= m << (shift - 32);
		else {
			this.tl |= m << shift;
			if (shift + bits > 32) this.th |= m >>> (32 - shift);
		}
	}

	/** Compress the word `hi`:`lo` into the lanes. */
	private compress(hi: number, lo: number): void {
		this.v3h ^= hi;
		this.v3l ^= lo;
		this.rounds(1);
		this.v0h ^= hi;
		this.v0l ^= lo;
	}

	/** `n` of SipHash's rounds over the lanes. */
	private rounds(n: number): void {
		let v0h = this.v0h;
		let v0l = this.v0l;
		let v1h = this.v1h;
		let v1l = this.v1l;
		let v2h = this.v2h;
		let v2l = this.v2l;
		let v3h = this.v3h;
		let v3l = this.v3l;
		let h: number;
		let l: number;
		for (let i = 0; i < n; i++) {
			// v0 += v1; v1 = rotl(v1, 13) ^ v0; v0 = rotl(v0, 32)
			l = (v0l >>> 0) + (v1l >>> 0);
			v0h = (v0h + v1h + (l > 0xffffffff ? 1 : 0)) | 0;
			v0l = l | 0;
			h = v1h;
			v1h = ((h << 13) | (v1l >>> 19)) ^ v0h;
			v1l = ((v1l << 13) | (h >>> 19)) ^ v0l;
			h = v0h;
			v0h = v0l;
			v0l = h;
			// v2 += v3; v3 = rotl(v3, 16) ^ v2
			l = (v2l >>> 0) + (v3l >>> 0);
			v2h = (v2h + v3h + (l > 0xffffffff ? 1 : 0)) | 0;
			v2l = l | 0;
			h = v3h;
			v3h = ((h << 16) | (v3l >>> 16)) ^ v2h;
			v3l = ((v3l << 16) | (h >>> 16)) ^ v2l;
			// v0 += v3; v3 = rotl(v3, 21) ^ v0
			l = (v0l >>> 0) + (v3l >>> 0);
			v0h = (v0h + v3h + (l > 0xffffffff ? 1 : 0)) | 0;
			v0l = l | 0;
			h = v3h;
			v3h = ((h << 21) | (v3l >>> 11)) ^ v0h;
			v3l = ((v3l << 21) | (h >>> 11)) ^ v0l;
			// v2 += v1; v1 = rotl(v1, 17) ^ v2; v2 = rotl(v2, 32)
			l = (v2l >>> 0) + (v1l >>> 0);
			v2h = (v2h + v1h + (l > 0xffffffff ? 1 : 0)) | 0;
			v2l = l | 0;
			h = v1h;
			v1h = ((h << 17) | (v1l >>> 15)) ^ v2h;
			v1l = ((v1l << 17) | (h >>> 15)) ^ v2l;
			h = v2h;
			v2h = v2l;
			v2l = h;
		}
		this.v0h = v0h;
		this.v0l = v0l;
		this.v1h = v1h;
		this.v1l = v1l;
		this.v2h = v2h;
		this.v2l = v2l;
		this.v3h = v3h;
		this.v3l = v3l;
	}
}

/**
 * `RandomState`: the keys `k0` and `k1` of the SipHash-1-3 it builds, as 32-bit halves.
 * Never changed once made, so a clone is itself.
 */
export interface RandomState {
	readonly k0h: number;
	readonly k0l: number;
	readonly k1h: number;
	readonly k1l: number;
}

const UNKEYED: RandomState = { k0h: 0, k0l: 0, k1h: 0, k1l: 0 };

/** Where `finish` runs its last rounds, which leave the hasher as it was. */
const LAST = /* @__PURE__ */ new SipHasher(UNKEYED);

/** `DefaultHasher::new`: SipHash-1-3 keyed by zeros. */
export function hasher(): Hasher {
	return new SipHasher(UNKEYED);
}

/** This thread's keys, `k0` and `k1` as 32-bit halves, high first, once a `RandomState` is made. */
let keys: Int32Array | undefined;

/**
 * `RandomState::new`: this thread's keys, drawn once from `crypto.getRandomValues` where
 * std asks the OS, then `k0` raised by one for each state, as std's is.
 */
export function randomState(): RandomState {
	keys ??= crypto.getRandomValues(new Int32Array(4));
	const state = { k0h: keys[0], k0l: keys[1], k1h: keys[2], k1l: keys[3] };
	keys[1] = (keys[1] + 1) | 0;
	if (keys[1] === 0) keys[0] = (keys[0] + 1) | 0;
	return state;
}

/** `BuildHasher::build_hasher` of a `RandomState`. */
export function buildHasher(state: RandomState): Hasher {
	return new SipHasher(state);
}

/** `BuildHasher::hash_one` of a `RandomState`: `value`'s hash, by its `Hash`, then `value` drops. */
export function hashOne<T>(state: RandomState, value: T, hash: (value: T, s: Hasher) => void, drop?: (value: T) => void): u64 {
	const s = buildHasher(state);
	hash(value, s);
	drop?.(value);
	return s.finish();
}

/** A hash collection's own hasher: a running hash of the words written, as a JS map's key. */
class Mixer implements Hasher {
	h = 0;

	word(word: number): void {
		this.h = Math.imul(((this.h << 5) | (this.h >>> 27)) ^ word, 0x9e3779b1);
	}

	str(str: string): void {
		this.word(str.length);
		for (let i = 0; i < str.length; i++) this.word(str.charCodeAt(i));
	}

	finish(): u64 {
		return this.key() as u64;
	}

	/** A 30-bit integer, which V8 keeps unboxed. */
	key(): number {
		return (this.h ^ (this.h >>> 15)) & 0x3fffffff;
	}
}

/** `Hash::hash` of a `u8` or `i8`, or of a `bool`, which writes a byte. */
export function hash8(value: number | boolean, s: Hasher): void {
	s.word(+value, 1);
}

/** `Hash::hash` of a `u16` or `i16`. */
export function hash16(value: number, s: Hasher): void {
	s.word(value, 2);
}

/** `Hash::hash` of a 32-bit integer, `usize` and `isize` among them, or of a `char`. */
export function hash32(value: number, s: Hasher): void {
	s.word(value, 4);
}

/** `Hash::hash` of a `u64` or `i64`. */
export function hash64(value: number | U64Limbs | I64Limbs, s: Hasher): void {
	if (typeof value === "number") {
		s.word(value >>> 0, 4);
		s.word(Math.floor(value / 4294967296) | 0, 4);
	} else {
		s.word(value.lo, 4);
		s.word(value.hi, 4);
	}
}

/** `Hash::hash` of a `u128` or `i128`. */
export function hash128(value: number | U128Limbs | I128Limbs, s: Hasher): void {
	if (typeof value === "number") {
		const fill = value < 0 ? -1 : 0;
		s.word(value >>> 0, 4);
		s.word(Math.floor(value / 4294967296) | 0, 4);
		s.word(fill, 4);
		s.word(fill, 4);
	} else {
		s.word(value.lo, 4);
		s.word(value.w1, 4);
		s.word(value.w2, 4);
		s.word(value.hi, 4);
	}
}

/** `Hash::hash` of a string. */
export function hashStr(str: string, s: Hasher): void {
	s.str(str);
}

/** `Hash::hash` of what holds nothing, as `()` and `PhantomData`: nothing written. */
export function hashNothing(_value: undefined, _s: Hasher): void {}

/** `Hash::hash` of tuples, from each field's. */
export function hashTuple<T extends readonly unknown[]>(fields: readonly ((value: never, s: Hasher) => void)[]): (value: T, s: Hasher) => void {
	return (value, s) => {
		for (let i = 0; i < fields.length; i++) (fields[i] as (value: unknown, s: Hasher) => void)(value[i], s);
	};
}

/** `Hash::hash` of a slice: its length as a `usize`, then its elements' in order. */
export function hashSlice<T>(hash: (value: T, s: Hasher) => void): (value: ArrayLike<T>, s: Hasher) => void {
	return (value, s) => {
		s.word(value.length, 4);
		for (let i = 0; i < value.length; i++) hash(value[i], s);
	};
}

/** `Hash::hash` of a `LinkedList`: its length as a `usize`, then its items' in order. */
export function hashList<T>(hash: (value: T, s: Hasher) => void): (value: LinkedList<T>, s: Hasher) => void {
	return (value, s) => {
		s.word(value.n, 4);
		for (let node = value.h; node !== undefined; node = node.x) hash(node.v, s);
	};
}

/**
 * `Hash::hash` of a byte array, slice, or vector, which is alike in each form: its length
 * as a `usize`, then its bytes.
 */
export function hashBytes(value: Uint8Array | ByteSlice | ByteVec, s: Hasher): void {
	const data = value instanceof Uint8Array ? value : value.data;
	const start = value instanceof ByteSlice ? value.start : 0;
	s.word(value.length, 4);
	for (let i = 0; i < value.length; i++) s.word(data[start + i], 1);
}

/** `Hash::hash` of an `Option`: its discriminant as an `isize`, then its value's. */
export function hashOption<T>(hash: (value: T, s: Hasher) => void): (value: T | undefined, s: Hasher) => void;
export function hashOption<T>(hash: (value: T, s: Hasher) => void, none: None): (value: T | None, s: Hasher) => void;
export function hashOption<T>(
	hash: (value: T, s: Hasher) => void,
	none?: None,
): (value: T | undefined | None, s: Hasher) => void {
	return (value, s) => {
		s.word(value === none ? 0 : 1, 4);
		if (value !== none) hash(value as T, s);
	};
}

/** `Hash::hash` of an `Arc`: its value's. */
export function hashArc<T>(hash: (value: T, s: Hasher) => void): (value: Arc<T>, s: Hasher) => void {
	return (value, s) => hash(value.value, s);
}

/** `Hasher::finish`. */
export function hashFinish(s: Hasher): u64 {
	return s.finish();
}

/** How a hash collection finds a key that is a JS object: its `Hash` and `Eq`. */
export interface Hashing<K> {
	readonly hash: (key: K) => number;
	readonly eq: (a: K, b: K) => boolean;
	/** Whether a key that is a JS primitive goes by them too: an enum with its own `Eq`. */
	readonly all: boolean;
}

/** The `Hashing` of a key type, from its `Hash::hash` and `PartialEq::eq`. */
export function hashing<K>(hash: (key: K, s: Hasher) => void, eq: (a: K, b: K) => boolean, all = false): Hashing<K> {
	return {
		hash: (key) => {
			const s = new Mixer();
			hash(key, s);
			return s.key();
		},
		eq,
		all,
	};
}
