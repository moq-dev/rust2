// `bytes`, for rust2-ts output. A `Bytes` is immutable bytes that its clones and slices
// share, as a `&[u8]` shares them, so it is an `rt.ByteSlice`. A `BytesMut` is a window of
// storage that the handles `split_to` makes and the `Bytes` that `freeze` makes share
// without overlapping. `rt.ts` re-exports all of it.

import {
	ByteSlice,
	type ByteVec,
	byteSliceToByteVec,
	bytes,
	byteVecExtendFromSlice,
	byteVecPush,
	copyBytes,
	type FmtResult,
	type Formatter,
	fmtWriteStr,
	type Place,
	panic,
	type RangeInclusive,
} from "../rt.ts";
import type { usize } from "./int.ts";

/** `isize::MAX` on wasm32, the most bytes an allocation holds. */
const ISIZE_MAX = 0x7fffffff;

/**
 * The most bytes, counting those read past, that a `BytesMut` whose storage was shared or
 * taken reaches before `reserve` throws. Below it, `bytes`' storage stays under 1 GiB, so
 * growing it panics nowhere, whichever handles dropped.
 */
const BYTES_MUT_UNSURE_LIMIT = 0x20000000;

/** The furthest a `bytes` vector reads past its front before it becomes shared storage, on wasm32. */
const MAX_VEC_POS = 0x7ffffff;

/** `bytes::Bytes`. */
export type Bytes = ByteSlice;

/** `Bytes::new`. Its bytes are made each time, as a bundler keeps a constant object. */
export function bytesNew(): Bytes {
	return new ByteSlice(new Uint8Array(0), 0, 0);
}

/** `Bytes::copy_from_slice`: a copy, which later writes to `data` don't change. */
export function bytesCopyFromSlice(data: ByteSlice): Bytes {
	return new ByteSlice(data.data.slice(data.start, data.start + data.length), 0, data.length);
}

/** `Bytes::slice` of `begin..end`, `end` defaulting to the length, panicking as `bytes` does. */
export function bytesSlice(b: Bytes, begin: usize, end: usize = b.length as usize): Bytes {
	if (begin > end) panic(`range start must not be greater than end: ${begin} <= ${end}`);
	if (end > b.length) panic(`range end out of bounds: ${end} <= ${b.length}`);
	return new ByteSlice(b.data, b.start + begin, end - begin);
}

/** `Bytes::slice` of `a..=b`; one exhausted by overflowing is empty, ending before its start. */
export function bytesSliceInclusive(b: Bytes, range: RangeInclusive<usize>): Bytes {
	if (range.exhausted) return bytesSlice(b, range.lo, range.lo);
	if (range.hi === 0xffffffff) panic("out of range");
	return bytesSlice(b, range.lo, (range.hi + 1) as usize);
}

/** `Bytes::slice` of `..=end`, which can't end past `usize::MAX`. */
export function bytesSliceToInclusive(b: Bytes, end: usize): Bytes {
	if (end === 0xffffffff) panic("out of range");
	return bytesSlice(b, 0 as usize, (end + 1) as usize);
}

/** `Debug`: a byte string literal, `b"\x00ab"`, written a piece at a time as `bytes` does. */
export function bytesDebug(b: Bytes, f: Formatter): FmtResult {
	let r = fmtWriteStr(f, 'b"');
	for (let i = 0; i < b.length && r.ok; i++) r = fmtWriteStr(f, escape(b.data[b.start + i]));
	return r.ok ? fmtWriteStr(f, '"') : r;
}

function escape(byte: number): string {
	switch (byte) {
		case 0x0a:
			return "\\n";
		case 0x0d:
			return "\\r";
		case 0x09:
			return "\\t";
		case 0x5c:
		case 0x22:
			return `\\${String.fromCharCode(byte)}`;
		case 0:
			return "\\0";
	}
	if (byte >= 0x20 && byte < 0x7f) return String.fromCharCode(byte);
	return `\\x${byte.toString(16).padStart(2, "0")}`;
}

/** `Bytes::split_to`: the first `at` bytes, leaving the rest. */
export function bytesSplitTo(p: Place<Bytes>, at: usize): Bytes {
	const b = p.v;
	if (at > b.length) panic(`split_to out of bounds: ${at} <= ${b.length}`);
	p.v = new ByteSlice(b.data, b.start + at, b.length - at);
	return new ByteSlice(b.data, b.start, at);
}

/** `Bytes::split_off`: the bytes from `at` on, leaving the first `at`. */
export function bytesSplitOff(p: Place<Bytes>, at: usize): Bytes {
	const b = p.v;
	if (at > b.length) panic(`split_off out of bounds: ${at} <= ${b.length}`);
	p.v = new ByteSlice(b.data, b.start, at);
	return new ByteSlice(b.data, b.start + at, b.length - at);
}

/** `Bytes::truncate`, and `clear` of 0. */
export function bytesTruncate(p: Place<Bytes>, len: usize): void {
	if (len < p.v.length) p.v = new ByteSlice(p.v.data, p.v.start, len);
}

/**
 * `Bytes::slice_ref`: the bytes of `b` that `subset` is, by where they are in memory. Its
 * panic names no addresses, which JS has none of. Bytes of other storage throw instead:
 * Rust may share what JS copied, such as a `&'static str`'s bytes, which `as_bytes`
 * encodes anew each time.
 */
export function bytesSliceRef(b: Bytes, subset: ByteSlice): Bytes {
	if (subset.length === 0) return bytesNew();
	if (subset.data.buffer !== b.data.buffer) {
		throw new Error("rust2-ts: `Bytes::slice_ref` of bytes from other storage, which Rust may share where JS copied");
	}
	const at = subset.data.byteOffset + subset.start;
	const from = b.data.byteOffset + b.start;
	if (at < from) panic("subset pointer is smaller than self pointer");
	if (at + subset.length > from + b.length) panic("subset is out of bounds");
	return new ByteSlice(b.data, b.start + (at - from), subset.length);
}

// Buf

/** `bytes`' panic for reading or advancing past what remains. */
function panicAdvance(requested: number, available: number): never {
	panic(`advance out of bounds: the len is ${available} but advancing by ${requested}`);
}

/** The `Buf` of `Bytes` or `BytesMut` advancing past what remains, which asserts. */
function panicRemaining(cnt: number, remaining: number): never {
	panic(`cannot advance past \`remaining\`: ${cnt} <= ${remaining}`);
}

/** `nbytes` read into an integer of `size` bytes, which holds at most that many. */
function checkFits(size: number, nbytes: number): void {
	if (nbytes > size) panic(`size too large: the integer type can fit ${size} bytes, but nbytes is ${nbytes}`);
}

/** `n` bytes from the front of `s`, the other way around if `le`. */
function readOf(s: ByteSlice, n: number, le: boolean): Uint8Array {
	const view = s.data.subarray(s.start, s.start + n);
	return le ? view.slice().reverse() : view;
}

/** The unsigned number of the `n` big-endian bytes of `s` from `i`, `n` at most 4. */
function wordOf(s: ByteSlice, n: number, le: boolean): number {
	let v = 0;
	for (let i = 0; i < n; i++) v = (v << 8) | s.data[s.start + (le ? n - 1 - i : i)];
	return n === 4 ? v >>> 0 : v;
}

/** `<Bytes as Buf>::advance`. */
export function bytesAdvance(p: Place<Bytes>, cnt: usize): void {
	const b = p.v;
	if (cnt > b.length) panicRemaining(cnt, b.length);
	p.v = new ByteSlice(b.data, b.start + cnt, b.length - cnt);
}

/** `<&[u8] as Buf>::advance`, which panics as reads past the end do. */
export function byteSliceAdvance(p: Place<ByteSlice>, cnt: usize): void {
	const b = p.v;
	if (cnt > b.length) panicAdvance(cnt, b.length);
	p.v = new ByteSlice(b.data, b.start + cnt, b.length - cnt);
}

/** `<&[u8] as Buf>::copy_to_bytes`: a copy of its first `len` bytes, read past. */
export function byteSliceCopyToBytes(p: Place<ByteSlice>, len: usize): Bytes {
	const b = p.v;
	if (len > b.length) panicAdvance(len, b.length);
	p.v = new ByteSlice(b.data, b.start + len, b.length - len);
	return bytesCopyFromSlice(new ByteSlice(b.data, b.start, len));
}

/**
 * `Buf::copy_to_bytes` as `bytes` provides it, of a buffer read through its own methods:
 * `BytesMut::put` of a `Take` of `len` of it into a new `BytesMut` of capacity `len`, which
 * isn't 0 once anything remains, so it reserves and copies chunk by chunk, then freezes.
 */
export function bufCopyToBytes<B>(
	src: B,
	len: usize,
	remaining: (src: B) => usize,
	chunk: (src: B) => ByteSlice,
	advance: (src: B, cnt: usize) => void,
): Bytes {
	if (remaining(src) < len) panicAdvance(len, remaining(src));
	const out = bytesMutWithCapacity(len);
	let limit: number = len;
	// The `Take`'s `remaining`, which reads the buffer's each time.
	const left = () => Math.min(remaining(src), limit);
	if (left() === 0) return bytesMutFreeze(out);
	bytesMutReserve(out, left() as usize);
	while (left() !== 0) {
		const c = chunk(src);
		const s = c.length > limit ? new ByteSlice(c.data, c.start, limit) : c;
		bytesMutExtend(out, s);
		advance(src, s.length as usize);
		limit -= s.length;
	}
	return bytesMutFreeze(out);
}

/** `Buf::get_*` of `Bytes`: its first `n` bytes, the other way around if `le`, read past. */
export function bytesRead(p: Place<Bytes>, n: usize, le: boolean): Uint8Array {
	const b = p.v;
	if (b.length < n) panicAdvance(n, b.length);
	p.v = new ByteSlice(b.data, b.start + n, b.length - n);
	return readOf(b, n, le);
}

/** `Buf::get_*` of `Bytes` of at most 4 bytes, as an unsigned number. */
export function bytesReadWord(p: Place<Bytes>, n: usize, le: boolean): number {
	const b = p.v;
	if (b.length < n) panicAdvance(n, b.length);
	p.v = new ByteSlice(b.data, b.start + n, b.length - n);
	return wordOf(b, n, le);
}

/**
 * `Buf::get_uint` or `get_int` of `Bytes`: `nbytes` as the big-endian bytes of an integer of
 * `size` bytes, sign-extended if `signed`.
 */
export function bytesReadInt(p: Place<Bytes>, size: usize, nbytes: usize, signed: boolean, le: boolean): Uint8Array {
	checkFits(size, nbytes);
	return widen(bytesRead(p, nbytes, le), size, signed);
}

/** `Buf::copy_to_slice` of `Bytes`. */
export function bytesCopyToSlice(p: Place<Bytes>, dst: ByteSlice): void {
	const b = p.v;
	if (b.length < dst.length) panicAdvance(dst.length, b.length);
	copyBytes(dst.data, dst.start, b.data, b.start, dst.length);
	p.v = new ByteSlice(b.data, b.start + dst.length, b.length - dst.length);
}

/** Big-endian bytes as the low bytes of `size`, the rest zero, or copies of the sign if `signed`. */
function widen(read: Uint8Array, size: number, signed: boolean): Uint8Array {
	const out = new Uint8Array(size);
	if (signed && read.length > 0 && read[0] >= 0x80) out.fill(0xff);
	out.set(read, size - read.length);
	return out;
}

// BytesMut

/**
 * `bytes::BytesMut`: `length` bytes of `data` from `start`, with room up to `cap` before
 * the bytes another handle or a frozen `Bytes` holds. Once its storage was shared or
 * taken from a `Bytes`, its capacity isn't `bytes`', which depends on whether other
 * handles of that storage dropped, so translation refuses what reads it.
 */
export class BytesMut {
	data: Uint8Array;
	/** How far into `data` it starts, which is at least as far as `bytes`' handle reads past. */
	start: number;
	length: number;
	cap: number;
	/**
	 * Whether it is a vector that was never shared nor taken, whose `start` and `cap` are
	 * `bytes`' own, so it grows and panics as `Vec` does.
	 */
	exact: boolean;

	constructor(data: Uint8Array, start: number, length: number, cap: number, exact: boolean) {
		this.data = data;
		this.start = start;
		this.length = length;
		this.cap = cap;
		this.exact = exact;
	}
}

/** `BytesMut::with_capacity`, `new` of 0. */
export function bytesMutWithCapacity(capacity: usize): BytesMut {
	if (capacity > ISIZE_MAX) panic("capacity overflow");
	return new BytesMut(bytes(capacity), 0, 0, capacity, true);
}

/** `BytesMut::zeroed`. */
export function bytesMutZeroed(len: usize): BytesMut {
	const b = bytesMutWithCapacity(len);
	b.length = len;
	return b;
}

/** A `BytesMut` of a copy of `s`: `From<&[u8]>`, `From<&str>`, and `Clone`. */
export function bytesMutFrom(s: ByteSlice): BytesMut {
	const b = bytesMutWithCapacity(s.length);
	copyBytes(b.data, 0, s.data, s.start, s.length);
	b.length = s.length;
	return b;
}

/**
 * `BytesMut::from` a `Bytes`, whose storage `bytes` takes where no other handle holds it,
 * reading past as much of it as the `Bytes` did.
 */
export function bytesMutFromBytes(s: Bytes): BytesMut {
	const b = new BytesMut(bytes(s.start + s.length), s.start, s.length, s.length, false);
	copyBytes(b.data, s.start, s.data, s.start, s.length);
	return b;
}

/** Its bytes, as `Deref`, `DerefMut`, `AsRef`, `AsMut`, and `Buf::chunk` lend them. */
export function bytesMutView(b: BytesMut): ByteSlice {
	return new ByteSlice(b.data, b.start, b.length);
}

/** `BytesMut::freeze`: a `Bytes` of its bytes, which nothing writes again. */
export function bytesMutFreeze(b: BytesMut): Bytes {
	return new ByteSlice(b.data, b.start, b.length);
}

/** `From<BytesMut> for Vec<u8>`. */
export function bytesMutToByteVec(b: BytesMut): ByteVec {
	return byteSliceToByteVec(bytesMutView(b));
}

/**
 * `BytesMut::reserve`: room for `additional` more bytes. A vector grows and panics as `Vec`
 * does, counting the bytes it read past. Storage that was shared or taken throws past
 * `BYTES_MUT_UNSURE_LIMIT`, where `bytes`' panic depends on which handles dropped.
 */
export function bytesMutReserve(b: BytesMut, additional: usize): void {
	if (!b.exact && b.start + b.length + additional > BYTES_MUT_UNSURE_LIMIT) {
		throw new Error(
			`rust2-ts: \`BytesMut::reserve\` past ${BYTES_MUT_UNSURE_LIMIT} bytes of storage that was shared, whose panic depends on whether other handles of it dropped`,
		);
	}
	if (additional <= b.cap - b.length) return;
	const needed = b.length + additional;
	if (!b.exact) {
		// Never before `start`, which stays as far as `bytes`' handle reads past or further.
		const cap = Math.max(needed, b.cap * 2, 8);
		const data = bytes(b.start + cap);
		copyBytes(data, b.start, b.data, b.start, b.length);
		b.data = data;
		b.cap = cap;
		return;
	}
	// Its bytes move to the front where that alone makes the room.
	if (b.start >= b.length && b.cap - b.length + b.start >= additional) {
		b.data.copyWithin(0, b.start, b.start + b.length);
		b.cap += b.start;
		b.start = 0;
		return;
	}
	// `Vec::reserve` of the vector from the front of the storage.
	const required = b.start + needed;
	const total = Math.max((b.start + b.cap) * 2, required, 8);
	if (required > ISIZE_MAX || total > ISIZE_MAX) panic("capacity overflow");
	const data = bytes(total);
	copyBytes(data, b.start, b.data, b.start, b.length);
	b.data = data;
	b.cap = total - b.start;
}

/**
 * `BufMut::put` of a `Bytes` or a `BytesMut`. At zero capacity `bytes` takes its storage,
 * reading past as much of it as it did.
 */
export function bytesMutPutStorage(b: BytesMut, s: Bytes): void {
	if (s.length === 0) return;
	if (b.exact ? b.cap === 0 : b.length === 0) {
		const start = Math.max(b.start, s.start);
		b.data = bytes(start + s.length);
		b.start = start;
		b.cap = s.length;
		b.exact = false;
	}
	bytesMutExtend(b, s);
}

/** `BytesMut::extend_from_slice`, and `BufMut::put_slice`. */
export function bytesMutExtend(b: BytesMut, s: ByteSlice): void {
	bytesMutReserve(b, s.length);
	copyBytes(b.data, b.start + b.length, s.data, s.start, s.length);
	b.length += s.length;
}

/** `BufMut::put_u8` and the other writes of a narrow integer: its low `n` bytes, big-endian unless `le`. */
export function bytesMutPutWord(b: BytesMut, v: number, n: number, le: boolean): void {
	bytesMutReserve(b, n);
	const at = b.start + b.length;
	for (let i = 0; i < n; i++) b.data[at + (le ? i : n - 1 - i)] = v >>> (8 * i);
	b.length += n;
}

/** `BufMut::put_*` of the big-endian bytes of a wide integer or float, reversed if `le`. */
export function bytesMutPutBytes(b: BytesMut, be: Uint8Array, le: boolean): void {
	bytesMutReserve(b, be.length);
	const at = b.start + b.length;
	for (let i = 0; i < be.length; i++) b.data[at + i] = be[le ? be.length - 1 - i : i];
	b.length += be.length;
}

/** `BufMut::put_uint` or `put_int`: the low `nbytes` of the big-endian bytes, which `bytes` checks against `size` the other way around. */
export function bytesMutPutInt(b: BytesMut, be: Uint8Array, nbytes: usize, le: boolean): void {
	if (nbytes > be.length) panic(`size too large: the integer type can fit ${nbytes} bytes, but nbytes is ${be.length}`);
	bytesMutPutBytes(b, be.subarray(be.length - nbytes), le);
}

/** `BufMut::put_bytes`: `cnt` copies of `val`. */
export function bytesMutPutFill(b: BytesMut, val: number, cnt: usize): void {
	bytesMutReserve(b, cnt);
	b.data.fill(val, b.start + b.length, b.start + b.length + cnt);
	b.length += cnt;
}

/** `BytesMut::truncate`, and `clear` of 0. */
export function bytesMutTruncate(b: BytesMut, len: usize): void {
	if (len <= b.length) b.length = len;
}

/** `BytesMut::resize`. */
export function bytesMutResize(b: BytesMut, len: usize, value: number): void {
	if (len <= b.length) {
		b.length = len;
		return;
	}
	bytesMutPutFill(b, value, len - b.length);
}

/** `BytesMut::split_to`: the first `at` bytes, sharing the storage, leaving the rest. */
export function bytesMutSplitTo(b: BytesMut, at: usize): BytesMut {
	if (at > b.length) panic(`split_to out of bounds: ${at} <= ${b.length}`);
	const head = new BytesMut(b.data, b.start, at, at, false);
	b.start += at;
	b.length -= at;
	b.cap -= at;
	b.exact = false;
	return head;
}

/** `BytesMut::unsplit`: `other`'s bytes after these, in place where it follows them in their storage. */
export function bytesMutUnsplit(b: BytesMut, other: BytesMut): void {
	if (b.length === 0) {
		b.data = other.data;
		b.start = other.start;
		b.length = other.length;
		b.cap = other.cap;
		b.exact = other.exact;
		return;
	}
	if (other.cap === 0) return;
	if (!b.exact && !other.exact && b.data === other.data && b.start + b.length === other.start) {
		b.length += other.length;
		b.cap += other.cap;
		return;
	}
	bytesMutExtend(b, bytesMutView(other));
}

/** `<BytesMut as Buf>::advance`. */
export function bytesMutAdvance(b: BytesMut, cnt: usize): void {
	if (cnt > b.length) panicRemaining(cnt, b.length);
	b.start += cnt;
	b.length -= cnt;
	b.cap -= cnt;
	if (b.start > MAX_VEC_POS) b.exact = false;
}

/** `Buf::get_*` of `BytesMut`: its first `n` bytes, the other way around if `le`, read past. */
export function bytesMutRead(b: BytesMut, n: usize, le: boolean): Uint8Array {
	if (b.length < n) panicAdvance(n, b.length);
	const read = readOf(bytesMutView(b), n, le);
	bytesMutAdvance(b, n);
	return read;
}

/** `Buf::get_*` of `BytesMut` of at most 4 bytes, as an unsigned number. */
export function bytesMutReadWord(b: BytesMut, n: usize, le: boolean): number {
	if (b.length < n) panicAdvance(n, b.length);
	const v = wordOf(bytesMutView(b), n, le);
	bytesMutAdvance(b, n);
	return v;
}

/** `Buf::get_uint` or `get_int` of `BytesMut`, as `bytesReadInt`. */
export function bytesMutReadInt(b: BytesMut, size: usize, nbytes: usize, signed: boolean, le: boolean): Uint8Array {
	checkFits(size, nbytes);
	return widen(bytesMutRead(b, nbytes, le), size, signed);
}

/** `Buf::copy_to_slice` of `BytesMut`. */
export function bytesMutCopyToSlice(b: BytesMut, dst: ByteSlice): void {
	if (b.length < dst.length) panicAdvance(dst.length, b.length);
	copyBytes(dst.data, dst.start, b.data, b.start, dst.length);
	bytesMutAdvance(b, dst.length);
}

// BufMut of Vec<u8>

/** `BufMut::put` into a `Vec<u8>` of a `Buf` the crate implements, each chunk in turn. */
export function byteVecPutBuf<B>(
	v: ByteVec,
	src: B,
	hasRemaining: (src: B) => boolean,
	remaining: (src: B) => usize,
	chunk: (src: B) => ByteSlice,
	advance: (src: B, cnt: usize) => void,
): void {
	if (v.length + remaining(src) > ISIZE_MAX) panic("capacity overflow");
	while (hasRemaining(src)) {
		const s = chunk(src);
		byteVecExtendFromSlice(v, s);
		advance(src, s.length as usize);
	}
}

/** `BufMut::put_u8` and the other writes of a narrow integer to a `Vec<u8>`, as `bytesMutPutWord`. */
export function byteVecPutWord(v: ByteVec, value: number, n: number, le: boolean): void {
	for (let i = 0; i < n; i++) byteVecPush(v, value >>> (8 * (le ? i : n - 1 - i)));
}

/** `BufMut::put_*` of big-endian bytes to a `Vec<u8>`, reversed if `le`. */
export function byteVecPutBytes(v: ByteVec, be: Uint8Array, le: boolean): void {
	for (let i = 0; i < be.length; i++) byteVecPush(v, be[le ? be.length - 1 - i : i]);
}

/** `BufMut::put_uint` or `put_int` to a `Vec<u8>`, as `bytesMutPutInt`. */
export function byteVecPutInt(v: ByteVec, be: Uint8Array, nbytes: usize, le: boolean): void {
	if (nbytes > be.length) panic(`size too large: the integer type can fit ${nbytes} bytes, but nbytes is ${be.length}`);
	byteVecPutBytes(v, be.subarray(be.length - nbytes), le);
}

/** `BufMut::put_bytes` to a `Vec<u8>`: its `resize`, which panics past what a vector holds. */
export function byteVecPutFill(v: ByteVec, val: number, cnt: usize): void {
	if (v.length + cnt > ISIZE_MAX) panic("capacity overflow");
	for (let i = 0; i < cnt; i++) byteVecPush(v, val);
}
