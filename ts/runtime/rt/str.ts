// `str`'s slices, iterators, and searches, for rust2-ts output. `rt.ts` re-exports all of
// it.
//
// A JS string is UTF-16 where a `str` is UTF-8: these step by code point, and an offset
// they take or return counts UTF-8 bytes, as Rust's does. A string from translated code is
// well formed, so a pattern's text matches where its UTF-8 would, never across a
// surrogate pair.

import { fmtCharDebug, type Place, panic, panicFmt, stringLen } from "../rt.ts";
import { type Bounds, boundsRange, type Part } from "./collections.ts";
import type { char, u8, usize } from "./int.ts";
import { type Clones, DONE, type Done, Iter, iterForEach, iterSlice, type RangeInclusive, type SizeHint } from "./iter.ts";
import { drop as dispose } from "./task.ts";

const encoder = /* @__PURE__ */ new TextEncoder();

/** A pattern: a `char` or a `&str`, as its text, or a `FnMut(char) -> bool`. */
export type Pattern = string | ((c: char) => boolean);

/** The UTF-8 length of the code units of `s` from `from` to `to`. */
function utf8Len(s: string, from: number, to: number): number {
	let n = to - from;
	for (let i = from; i < to; i++) {
		const u = s.charCodeAt(i);
		// Each half of a surrogate pair is two of its four bytes.
		if (u >= 0x80) n += u < 0x800 || (u >= 0xd800 && u <= 0xdfff) ? 1 : 2;
	}
	return n;
}

/** Where the char that ends at `end` starts. */
function before(s: string, end: number): number {
	const u = s.charCodeAt(end - 1);
	return u >= 0xdc00 && u <= 0xdfff ? end - 2 : end - 1;
}

/** A char's UTF-8 length. */
function charLen(c: number): number {
	return c < 0x80 ? 1 : c < 0x800 ? 2 : c < 0x10000 ? 3 : 4;
}

/**
 * The code unit of `s` where byte `to` starts a char, reading on from code unit `i`, which
 * is byte `from`; -1 if `to` is inside a char, past the end, or before `from`.
 */
function unitAt(s: string, i: number, from: number, to: number): number {
	while (from < to && i < s.length) {
		const u = s.charCodeAt(i);
		if (u >= 0xd800 && u <= 0xdbff) {
			// A surrogate pair is one char of four bytes.
			from += 4;
			i += 2;
		} else {
			from += u < 0x80 ? 1 : u < 0x800 ? 2 : 3;
			i++;
		}
	}
	return from === to ? i : -1;
}

/** `&s[start..end]` by UTF-8 byte offsets, with `end` defaulting to the length. */
export function strSlice(s: string, start: number, end?: number): string {
	return s.slice(...units(s, start, end));
}

/** `str::get` by a range's UTF-8 byte offsets, with `end` defaulting to the length. */
export function strGet(s: string, start: number, end?: number): string | undefined {
	const at = unitsChecked(s, start, end);
	return at && s.slice(...at);
}

/**
 * The byte offsets `&s[r]` reads for an `a..=b` or a `..=b`, panicking as std's does where
 * `b` is not before the end; the part is checked as `a..b + 1`'s is.
 */
export function strInclusive(s: string, r: RangeInclusive<usize> | { readonly end: usize; readonly inclusive: true }): [number, number] {
	const [lo, hi] = "lo" in r ? [r.lo, r.hi] : [0, r.end];
	if (hi >= stringLen(s)) sliceFailed(s, lo, hi);
	// An `a..=b` iterated to its end starts past it, at `b + 1`, which JS doesn't overflow.
	return ["exhausted" in r && r.exhausted ? hi + 1 : lo, hi + 1];
}

/**
 * The code units of `s` from byte `start` to byte `end`, or to the end, or `undefined` if
 * either is out of bounds or inside a char, or they are backwards.
 */
function unitsChecked(s: string, start: number, end?: number): [number, number] | undefined {
	const from = unitAt(s, 0, 0, start);
	if (from < 0) return undefined;
	const to = end === undefined ? s.length : unitAt(s, from, start, end);
	return to < 0 ? undefined : [from, to];
}

/** `unitsChecked`, panicking as std's indexing does where it gives `undefined`. */
function units(s: string, start: number, end?: number): [number, number] {
	return unitsChecked(s, start, end) ?? sliceFailed(s, start, end ?? stringLen(s));
}

/**
 * `&mut p[from..to]` by code units: a part of the string at `p`, whose writes splice the
 * string anew. A write through a `&mut str` keeps each char's length, so the units hold.
 */
class StrPart implements Place<string> {
	readonly #whole: Place<string>;
	readonly #from: number;
	readonly #to: number;

	constructor(whole: Place<string>, [from, to]: [number, number]) {
		this.#whole = whole;
		this.#from = from;
		this.#to = to;
	}

	get v(): string {
		return this.#whole.v.slice(this.#from, this.#to);
	}

	set v(value: string) {
		const s = this.#whole.v;
		this.#whole.v = s.slice(0, this.#from) + value + s.slice(this.#to);
	}
}

/** `&mut p[start..end]` by UTF-8 byte offsets, with `end` defaulting to the length. */
export function strPart(p: Place<string>, start: number, end?: number): Place<string> {
	return new StrPart(p, units(p.v, start, end));
}

/** `str::get_mut` by a range's UTF-8 byte offsets, with `end` defaulting to the length. */
export function strGetMut(p: Place<string>, start: number, end?: number): Place<string> | undefined {
	const at = unitsChecked(p.v, start, end);
	return at && new StrPart(p, at);
}

/** `str::split_at_mut`. */
export function strSplitAtMut(p: Place<string>, mid: number): [Place<string>, Place<string>] {
	const [, at] = units(p.v, 0, mid);
	return [new StrPart(p, [0, at]), new StrPart(p, [at, p.v.length])];
}

/** `str::split_at_mut_checked`. */
export function strSplitAtMutChecked(p: Place<string>, mid: number): [Place<string>, Place<string>] | undefined {
	const at = unitsChecked(p.v, 0, mid);
	return at && [new StrPart(p, at), new StrPart(p, [at[1], p.v.length])];
}

/**
 * std's panic for a slice of `s` that is out of bounds, backwards, or inside a char, or that
 * ends inclusively at the length.
 */
function sliceFailed(s: string, start: number, end: number): never {
	const len = stringLen(s);
	if (start > len) panic(`start byte index ${start} is out of bounds for string of length ${len}`);
	if (end > len) panic(`end byte index ${end} is out of bounds for string of length ${len}`);
	if (start > end) panic(`byte range starts at ${start} but ends at ${end}`);
	// Neither inside a char: an inclusive end at the length, which is past the last byte.
	const [which, at] =
		unitAt(s, 0, 0, start) < 0
			? ["start", start]
			: unitAt(s, 0, 0, end) < 0
				? ["end", end]
				: panic(`end byte index ${end} is out of bounds for string of length ${len}`);
	// The char `at` is inside, and where its bytes start.
	let floor = 0;
	let c = 0;
	for (const ch of s) {
		c = ch.codePointAt(0) as number;
		if (floor + charLen(c) > at) break;
		floor += charLen(c);
	}
	panicFmt([
		`${which} byte index ${at} is not a char boundary; it is inside `,
		(f) => fmtCharDebug(c as char, f),
		` (bytes ${floor}..${floor + charLen(c)} of string)`,
	]);
}

/** `str::chars`. */
export function strChars(s: string): Iter<char> {
	return new Chars(s);
}

class Chars extends Iter<char> {
	readonly #s: string;
	#front = 0;
	#back: number;

	constructor(s: string) {
		super();
		this.#s = s;
		this.#back = s.length;
	}

	next(): char | Done {
		if (this.#front >= this.#back) return DONE;
		const c = this.#s.codePointAt(this.#front) as number;
		this.#front += c > 0xffff ? 2 : 1;
		return c as char;
	}

	override nextBack(): char | Done {
		if (this.#front >= this.#back) return DONE;
		this.#back = before(this.#s, this.#back);
		return this.#s.codePointAt(this.#back) as char;
	}

	/** std's: a char per four bytes of what is left, and at most one per byte. */
	override sizeHint(): SizeHint {
		const bytes = utf8Len(this.#s, this.#front, this.#back);
		return [Math.ceil(bytes / 4), bytes];
	}

	/** `as_str`: what is left. */
	rest(): string {
		return this.#s.slice(this.#front, this.#back);
	}

	override clone(): Iter<char> {
		const it = new Chars(this.#s);
		it.#front = this.#front;
		it.#back = this.#back;
		return it;
	}

	/** std's `Debug` lists a clone. */
	override remaining(): Iterable<char> {
		return this.clone();
	}
}

/**
 * std's searcher of a pattern in `s` between `front` and `back`, in code units. Each step
 * finds the next match from one end, `start` to `end`, and moves that end past it. A
 * `&str` pattern searches from one end only, as std's can.
 */
class Searcher {
	readonly s: string;
	readonly #p: Pattern;
	#front = 0;
	#back: number;
	// An empty pattern matches before each char and after the last, as std's does.
	#matched = false;
	start = 0;
	end = 0;

	constructor(s: string, p: Pattern) {
		this.s = s;
		this.#p = p;
		this.#back = s.length;
	}

	/** std's: where it is, with its pattern cloned by `h.f`, a closure's as Rust clones it. */
	clone(h: Clones | undefined): Searcher {
		const m = new Searcher(this.s, h?.f === undefined ? this.#p : (h.f as (p: Pattern) => Pattern)(this.#p));
		m.#front = this.#front;
		m.#back = this.#back;
		m.#matched = this.#matched;
		m.start = this.start;
		m.end = this.end;
		return m;
	}

	nextMatch(): boolean {
		const { s } = this;
		const p = this.#p;
		if (p === "") {
			if (this.#matched) {
				if (this.#front >= this.#back) return false;
				this.#front += (s.codePointAt(this.#front) as number) > 0xffff ? 2 : 1;
			}
			this.#matched = true;
			this.start = this.end = this.#front;
			return true;
		}
		if (typeof p === "string") {
			const i = s.indexOf(p, this.#front);
			if (i < 0 || i + p.length > this.#back) {
				this.#front = this.#back;
				return false;
			}
			this.start = i;
			this.end = this.#front = i + p.length;
			return true;
		}
		while (this.#front < this.#back) {
			const i = this.#front;
			const c = s.codePointAt(i) as number;
			this.#front += c > 0xffff ? 2 : 1;
			if (p(c as char)) {
				this.start = i;
				this.end = this.#front;
				return true;
			}
		}
		return false;
	}

	nextMatchBack(): boolean {
		const { s } = this;
		const p = this.#p;
		if (typeof p === "string") {
			const from = this.#back - p.length;
			const i = from < this.#front ? -1 : s.lastIndexOf(p, from);
			if (i < this.#front) {
				this.#back = this.#front;
				return false;
			}
			this.start = this.#back = i;
			this.end = i + p.length;
			return true;
		}
		while (this.#back > this.#front) {
			const end = this.#back;
			this.#back = before(s, end);
			if (p(s.codePointAt(this.#back) as char)) {
				this.start = this.#back;
				this.end = end;
				return true;
			}
		}
		return false;
	}
}

/** `str::split`. */
export function strSplit(s: string, p: Pattern): Iter<string> {
	return new Split(new Searcher(s, p));
}

class Split extends Iter<string> {
	readonly #m: Searcher;
	#start = 0;
	#end: number;
	#finished = false;

	constructor(m: Searcher) {
		super();
		this.#m = m;
		this.#end = m.s.length;
	}

	override clone(h?: Clones): Iter<string> {
		const it = new Split(this.#m.clone(h));
		it.#start = this.#start;
		it.#end = this.#end;
		it.#finished = this.#finished;
		return it;
	}

	next(): string | Done {
		if (this.#finished) return DONE;
		const m = this.#m;
		if (m.nextMatch()) {
			const part = m.s.slice(this.#start, m.start);
			this.#start = m.end;
			return part;
		}
		this.#finished = true;
		return m.s.slice(this.#start, this.#end);
	}

	override nextBack(): string | Done {
		if (this.#finished) return DONE;
		const m = this.#m;
		if (m.nextMatchBack()) {
			const part = m.s.slice(m.end, this.#end);
			this.#end = m.start;
			return part;
		}
		this.#finished = true;
		return m.s.slice(this.#start, this.#end);
	}
}

/** `str::match_indices`: each match, after its offset in UTF-8 bytes. */
export function strMatchIndices(s: string, p: Pattern): Iter<[usize, string]> {
	return new MatchIndices(new Searcher(s, p));
}

class MatchIndices extends Iter<[usize, string]> {
	readonly #m: Searcher;
	// Where the last match from each end starts, in code units and in bytes. The back's
	// bytes are counted once it first moves.
	#front = 0;
	#frontBytes = 0;
	#back = -1;
	#backBytes = 0;

	constructor(m: Searcher) {
		super();
		this.#m = m;
	}

	override clone(h?: Clones): Iter<[usize, string]> {
		const it = new MatchIndices(this.#m.clone(h));
		it.#front = this.#front;
		it.#frontBytes = this.#frontBytes;
		it.#back = this.#back;
		it.#backBytes = this.#backBytes;
		return it;
	}

	next(): [usize, string] | Done {
		const m = this.#m;
		if (!m.nextMatch()) return DONE;
		this.#frontBytes += utf8Len(m.s, this.#front, m.start);
		this.#front = m.start;
		return [this.#frontBytes as usize, m.s.slice(m.start, m.end)];
	}

	override nextBack(): [usize, string] | Done {
		const m = this.#m;
		if (!m.nextMatchBack()) return DONE;
		if (this.#back < 0) {
			this.#back = m.s.length;
			this.#backBytes = this.#frontBytes + utf8Len(m.s, this.#front, this.#back);
		}
		this.#backBytes -= utf8Len(m.s, m.start, this.#back);
		this.#back = m.start;
		return [this.#backBytes as usize, m.s.slice(m.start, m.end)];
	}
}

/** `str::contains`. */
export function strContains(s: string, p: Pattern): boolean {
	return typeof p === "string" ? s.includes(p) : new Searcher(s, p).nextMatch();
}

/** `str::starts_with`. */
export function strStartsWith(s: string, p: Pattern): boolean {
	if (typeof p === "string") return s.startsWith(p);
	return s.length > 0 && p(s.codePointAt(0) as char);
}

/** `str::trim_start_matches`, which an empty pattern leaves as it is. */
export function strTrimStartMatches(s: string, p: Pattern): string {
	let i = 0;
	if (typeof p === "string") {
		if (p === "") return s;
		while (s.startsWith(p, i)) i += p.length;
	} else {
		while (i < s.length) {
			const c = s.codePointAt(i) as number;
			if (!p(c as char)) break;
			i += c > 0xffff ? 2 : 1;
		}
	}
	return s.slice(i);
}

/** `str::trim_end_matches`, which an empty pattern leaves as it is. */
export function strTrimEndMatches(s: string, p: Pattern): string {
	let end = s.length;
	if (typeof p === "string") {
		if (p === "") return s;
		while (end >= p.length && s.startsWith(p, end - p.length)) end -= p.length;
	} else {
		while (end > 0) {
			const i = before(s, end);
			if (!p(s.codePointAt(i) as char)) break;
			end = i;
		}
	}
	return s.slice(0, end);
}

/** `str::rfind`: the last match's offset in UTF-8 bytes. */
export function strRfind(s: string, p: Pattern): usize | undefined {
	const m = new Searcher(s, p);
	return m.nextMatchBack() ? (utf8Len(s, 0, m.start) as usize) : undefined;
}

/** A pattern of `[char; N]` or `&[char]`: whether a char is any of `cs`. */
export function patternChars(cs: ArrayLike<char>): (c: char) => boolean {
	return (c) => {
		for (let i = 0; i < cs.length; i++) if (cs[i] === c) return true;
		return false;
	};
}

/** `str::ends_with`. */
export function strEndsWith(s: string, p: Pattern): boolean {
	if (typeof p === "string") return s.endsWith(p);
	return s.length > 0 && p(s.codePointAt(before(s, s.length)) as char);
}

/** `str::split_once`: the text before the first match and after it. */
export function strSplitOnce(s: string, p: Pattern): [string, string] | undefined {
	const m = new Searcher(s, p);
	return m.nextMatch() ? [s.slice(0, m.start), s.slice(m.end)] : undefined;
}

/** `str::is_char_boundary`: whether byte `i` starts a char, or is the end. */
export function strIsCharBoundary(s: string, i: number): boolean {
	return unitAt(s, 0, 0, i) >= 0;
}

/** `str::bytes`: its UTF-8 bytes. */
export function strBytes(s: string): Iter<u8> {
	return iterSlice(encoder.encode(s));
}

// `String`'s edits, each giving the edited string, which the caller writes where the
// `&mut String` points, and what the edit returns beside it.

/** `isize::MAX` on wasm32, past which std refuses a capacity. */
const ISIZE_MAX = 0x7fffffff;

/**
 * `concat` and `join` of strings by `sep`, which panic as std's do where the bytes they
 * reserve pass `usize::MAX` or what a `String` may hold.
 */
export function strsJoin(parts: readonly string[], sep: string): string {
	const seps = Math.max(parts.length - 1, 0);
	let units = sep.length * seps;
	for (const part of parts) units += part.length;
	// A code unit is one to three bytes, so only a long result needs its bytes counted, each
	// distinct part once, as such a result mostly repeats a few.
	if (units * 3 > ISIZE_MAX) {
		const counted = new Map<string, number>();
		const count = (s: string): number => {
			let n = counted.get(s);
			if (n === undefined) counted.set(s, (n = stringLen(s)));
			return n;
		};
		let len = count(sep) * seps;
		for (const part of parts) len += count(part);
		if (len > 0xffffffff) panic("attempt to join into collection with len > usize::MAX");
		if (len > ISIZE_MAX) panic("capacity overflow");
	}
	return parts.join(sep);
}

/** `String::with_capacity`: the empty string, which keeps no capacity. */
export function stringWithCapacity(capacity: usize): string {
	if (capacity > ISIZE_MAX) panic("capacity overflow");
	return "";
}

/** `String::reserve` and `reserve_exact`, which panic as std's do past `isize::MAX`. */
export function stringReserve(s: string, additional: usize): void {
	// A code unit is at most three bytes, so only a large `additional` needs the length.
	if (additional > ISIZE_MAX - 3 * s.length && additional > ISIZE_MAX - utf8Len(s, 0, s.length)) {
		panic("capacity overflow");
	}
}

/** The code unit where byte `i` of `s` starts a char, or panic with `message`. */
function boundary(s: string, i: number, message: string): number {
	const at = unitAt(s, 0, 0, i);
	if (at < 0) panic(message);
	return at;
}

/** `String::truncate`: `s` to `len` bytes, or as it is if no longer. */
export function stringTruncate(s: string, len: usize): string {
	const at = unitAt(s, 0, 0, len);
	if (at >= 0) return s.slice(0, at);
	if (len > utf8Len(s, 0, s.length)) return s;
	panic("assertion failed: self.is_char_boundary(new_len)");
}

/** `String::insert_str`, and `insert` of a char's text. */
export function stringInsertStr(s: string, i: usize, text: string): string {
	const at = boundary(s, i, "assertion failed: self.is_char_boundary(idx)");
	return s.slice(0, at) + text + s.slice(at);
}

/** `String::retain`: the chars `f` keeps, calling it on each in order. */
export function stringRetain(s: string, f: (c: char) => boolean): string {
	let kept = "";
	for (const ch of s) if (f(ch.codePointAt(0) as char)) kept += ch;
	return kept;
}

/** `String::replace_range`. */
export function stringReplaceRange(s: string, r: Bounds<number>, text: string): string {
	const [start, end] = boundsRange(r, utf8Len(s, 0, s.length));
	const from = boundary(s, start, "start of range should be a character boundary");
	const to = boundary(s, end, "end of range should be a character boundary");
	return s.slice(0, from) + text + s.slice(to);
}

/** `String::extend_from_within`. */
export function stringExtendFromWithin(s: string, r: Bounds<number>): string {
	const [start, end] = boundsRange(r, utf8Len(s, 0, s.length));
	const from = boundary(s, start, "assertion failed: self.is_char_boundary(start)");
	const to = boundary(s, end, "assertion failed: self.is_char_boundary(end)");
	return s + s.slice(from, to);
}

/** `String::pop`: the rest, and the last char if any. */
export function stringPop(s: string): [string, char | undefined] {
	if (s.length === 0) return [s, undefined];
	const at = before(s, s.length);
	return [s.slice(0, at), s.codePointAt(at) as char];
}

/** `String::remove`: the rest, and the char at byte `i`. */
export function stringRemove(s: string, i: usize): [string, char] {
	const at = unitAt(s, 0, 0, i);
	if (at < 0) sliceFailed(s, i, utf8Len(s, 0, s.length));
	if (at === s.length) panic("cannot remove a char from the end of a string");
	const c = s.codePointAt(at) as number;
	return [s.slice(0, at) + s.slice(at + (c > 0xffff ? 2 : 1)), c as char];
}

/** `String::split_off`: what comes before byte `i`, and after it. */
export function stringSplitOff(s: string, i: usize): [string, string] {
	const at = boundary(s, i, "assertion failed: self.is_char_boundary(at)");
	return [s.slice(0, at), s.slice(at)];
}

// `String` from and into iterators. Each reads the iterator by `for_each`, as std's does,
// which `drops` makes its own `fold`.

/**
 * `Extend<char>` and `Extend<&char>` of `s`, and `collect` of chars from `""`: `s` and each
 * char, once it reserved the iterator's lower bound, which panics past `isize::MAX`.
 */
export function stringExtendChars(s: string, it: Iter<char>, drops = false): string {
	stringReserve(s, it.sizeHint()[0]);
	iterForEach(
		it,
		(c) => {
			s += String.fromCodePoint(c);
		},
		drops,
	);
	return s;
}

/** `String` as a tuple's part, of chars, which reserves as `stringReserve`. */
export const STRING_CHARS_PART: Part<string, char> = {
	reserve: stringReserve,
	one: (s, c) => s + String.fromCodePoint(c),
};

/** `String` as a tuple's part, of strings, which never reserves. */
export const STRING_PART: Part<string, string> = { one: (s, text) => s + text };

/** `Extend` of `s` by strings, `&str`, `String`, or `Cow<str>`, and `collect` of `&str`s from `""`. */
export function stringExtend(s: string, it: Iter<string>, drops = false): string {
	iterForEach(
		it,
		(text) => {
			s += text;
		},
		drops,
	);
	return s;
}

/** `collect` of owned strings, a `String` or `Cow<str>`: the first by `next`, then the rest extend it. */
export function stringFromStrings(it: Iter<string>, drops = false): string {
	const first = it.next();
	if (first !== DONE) return stringExtend(first, it, drops);
	if (drops) dispose(it);
	return "";
}

/**
 * `String::drain`: the chars within the byte range `r` of the string at `s`, which leaves
 * the string without them as it drops, and as it was if it never does, as `mem::forget`
 * leaves it.
 */
export function stringDrain(s: Place<string>, r: Bounds<number>): Iter<char> {
	const text = s.v;
	const [start, end] = boundsRange(r, utf8Len(text, 0, text.length));
	const from = boundary(text, start, "assertion failed: self.is_char_boundary(start)");
	const to = boundary(text, end, "assertion failed: self.is_char_boundary(end)");
	return new StringDrain(s, text, from, to);
}

/** `Drain::as_str`: the chars `drain` has yet to yield. */
export function stringDrainAsStr(drain: Iter<char>): string {
	return (drain as Chars).rest();
}

class StringDrain extends Chars implements Disposable {
	readonly #s: Place<string>;
	readonly #text: string;
	readonly #from: number;
	readonly #to: number;

	constructor(s: Place<string>, text: string, from: number, to: number) {
		super(text.slice(from, to));
		this.#s = s;
		this.#text = text;
		this.#from = from;
		this.#to = to;
	}

	[Symbol.dispose](): void {
		this.#s.v = this.#text.slice(0, this.#from) + this.#text.slice(this.#to);
	}
}
