// `std::alloc`, for rust2-ts output: a `Layout`, and memory from `alloc` that a cast of its
// pointer to one to `T` makes a JS array of `T`s, as many as fit. `rt.ts` re-exports all of it.
//
// JS keeps values, not bytes, so the memory is untyped until that cast, and a read or
// write of its bytes fails. `realloc` keeps the values in a new array, and `dealloc` lets
// the array go; a pointer into it after either reads what Rust leaves undefined.

import { Err, type FmtResult, type Formatter, fmtDebugStructFields, Ok, panic, type Place, type Result } from "../rt.ts";
import { AtIndex } from "./collections.ts";
import type { usize } from "./int.ts";

/** `core::alloc::Layout`: a size, and an alignment that is a power of two. */
export interface Layout {
	readonly size: usize;
	readonly align: usize;
}

/** `core::alloc::LayoutError`, which holds nothing. */
export class LayoutError {}

/** The one `LayoutError`. */
export const LAYOUT_ERROR = /* @__PURE__ */ new LayoutError();

/** The largest size a layout of `align` may have: wasm32's `isize::MAX`, less the padding up to `align`. */
function maxSize(align: number): number {
	return 0x7fffffff - (align - 1);
}

/** `Layout::from_size_align`: an error unless `align` is a power of two and `size` rounded up to it fits in `isize`. */
export function layoutFromSizeAlign(size: usize, align: usize): Result<Layout, LayoutError> {
	if (align === 0 || (align & (align - 1)) !== 0 || size > maxSize(align)) return Err(LAYOUT_ERROR);
	return Ok({ size, align });
}

/** `Layout::array::<T>(n)`, of `T`s of `size` bytes aligned to `align`. */
export function layoutArray(n: usize, size: usize, align: usize): Result<Layout, LayoutError> {
	if (size !== 0 && n > Math.floor(maxSize(align) / size)) return Err(LAYOUT_ERROR);
	return Ok({ size: size * n, align });
}

/** `Debug` of a `Layout`, as std's: its alignment also as a power of two. */
export function fmtLayoutDebug(l: Layout, f: Formatter): FmtResult {
	return fmtDebugStructFields(
		f,
		"Layout",
		["size", "align"],
		[(f) => f.out.writeStr(String(l.size)), (f) => f.out.writeStr(`${l.align} (1 << ${31 - Math.clz32(l.align)})`)],
	);
}

/** `Debug` of a `LayoutError`. */
export function fmtLayoutErrorDebug(_: LayoutError, f: Formatter): FmtResult {
	return f.out.writeStr("LayoutError");
}

/** `Display` of a `LayoutError`. */
export function fmtLayoutErrorDisplay(_: LayoutError, f: Formatter): FmtResult {
	return f.out.writeStr("invalid parameters to Layout::from_size_align");
}

/**
 * Memory from `alloc`: `size` bytes, which hold the values of type `type`, `elem` bytes
 * each, in `array` once a cast of a pointer to it types it. A pointer to its bytes is the block.
 */
export class Block implements Place<number> {
	readonly size: number;
	array: unknown[] | undefined;
	elem = 0;
	type = "";

	constructor(size: number) {
		this.size = size;
	}

	get v(): number {
		return bytes();
	}

	set v(_: number) {
		bytes();
	}
}

function bytes(): never {
	throw new Error("rust2-ts: a read or write of the bytes of memory from `alloc`, which JS keeps as values");
}

/** The block each array a cast of one's pointer made is in. */
const BLOCKS = /* @__PURE__ */ new WeakMap<unknown[], Block>();

/** `alloc::alloc`: `layout`'s bytes, untyped until a cast of the pointer to them. */
export function alloc(layout: Layout): Block {
	return new Block(layout.size);
}

/**
 * The pointer `p` to the bytes of a block, cast to one to the values of Rust type `type`,
 * `elem` bytes each, it holds: the first of a new array of as many as fit, or of the array
 * it holds already, of values of that type.
 */
export function allocAs<T>(p: Place<number>, elem: number, type: string): Place<T> {
	if (!(p instanceof Block)) {
		throw new Error("rust2-ts: a pointer cast from one to bytes to one to values, which isn't memory from `alloc`");
	}
	// As many as fit, past which bytes nothing can read are left over.
	if (!p.array) {
		p.array = new Array(Math.floor(p.size / elem));
		p.elem = elem;
		p.type = type;
		BLOCKS.set(p.array, p);
	}
	if (p.type !== type) throw new Error(`rust2-ts: memory from \`alloc\` holding \`${p.type}\`s, as \`${type}\`s`);
	return new AtIndex(p.array as T[], 0);
}

/**
 * The pointer `p` to the first of the values of Rust type `type`, `elem` bytes each, that
 * memory from `alloc`, or a `Vec`, holds, cast to one to its bytes: the block, which only
 * `dealloc` and `realloc` take.
 */
export function allocBytes(p: Place<unknown>, elem: number, type: string): Block {
	if (!(p instanceof AtIndex) || p.index !== 0 || !Array.isArray(p.array)) {
		throw new Error("rust2-ts: a pointer cast to one to bytes, which isn't to the start of memory from `alloc` or a `Vec`'s");
	}
	const array = p.array as unknown[];
	let block = BLOCKS.get(array);
	// A `Vec`'s, which `Layout::array` laid out as one block's is.
	if (!block) {
		block = new Block(array.length * elem);
		block.array = array;
		block.elem = elem;
		block.type = type;
		BLOCKS.set(array, block);
	}
	if (block.type !== type) throw new Error(`rust2-ts: memory from \`alloc\` holding \`${block.type}\`s, as \`${type}\`s`);
	return block;
}

/** The block the pointer `p` to bytes, which `dealloc` or `realloc` takes, is. */
function blockOf(p: Place<number>): Block {
	if (!(p instanceof Block)) throw new Error("rust2-ts: memory freed or grown that isn't from `alloc`");
	return p;
}

/** `alloc::dealloc` of the block `p`, whose values JS frees once nothing holds them. */
export function dealloc(p: Place<number>): void {
	const block = blockOf(p);
	if (block.array) BLOCKS.delete(block.array);
	block.array = undefined;
}

/**
 * `alloc::realloc` of the block `p` to `size` bytes: a new block holding the values the old one
 * did, as many as fit, and holes past them, which Rust leaves uninitialized.
 */
export function realloc(p: Place<number>, size: usize): Block {
	const block = blockOf(p);
	const next = new Block(size);
	const array = block.array;
	if (array) {
		const n = Math.floor(size / block.elem);
		next.array = array.slice(0, n);
		next.array.length = n;
		next.elem = block.elem;
		next.type = block.type;
		BLOCKS.set(next.array, next);
		dealloc(block);
	}
	return next;
}

/** `alloc::handle_alloc_error`, which wasm32's std aborts by, as a panic does there. */
export function handleAllocError(layout: Layout): never {
	panic(`memory allocation of ${layout.size} bytes failed`);
}
