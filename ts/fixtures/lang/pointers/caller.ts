// The inputs `pointers`' cases name, which run with the generated ones.

import { op } from "./lang.ts";

export { ops, run } from "./lang.ts";

/**
 * `pointers::cells` scripts: each operation on every cell, a number wrapping past its top, a
 * struct assigned through `get` and written over, and cells replaced or made and taken out.
 */
const unsafeCells = [
	[0, 248, 1, 9, 2, 10, 3, 11, 4, 12, 5, 13, 6, 14, 7, 15],
	[4, 12, 4, 5, 253, 7, 255, 0],
	[3, 251, 6, 3, 2, 2, 254],
	[],
].map((bytes) => Uint8Array.of(...op("pointers::cells"), 0, ...bytes));

/**
 * `pointers::raw_cells` scripts: each operation, then each replace before and after writes
 * through pointers to where the cells are kept, through cells of what a `&mut` borrows, and
 * of pointers to no cell beside cells of a `Vec` and a `String`.
 */
const rawCells = [
	[0, 8, 9, 2, 3, 4, 5, 6, 7],
	[1, 249, 3, 251, 0, 11, 4, 252, 2, 3],
	[2, 10, 18, 4, 12, 20, 3, 1, 0],
	[5, 13, 21, 0, 3, 5, 253, 4],
	[6, 14, 22, 30, 254],
	[],
].map((bytes) => Uint8Array.of(...op("pointers::raw_cells"), 0, ...bytes));

/**
 * `pointers::cell_writes` scripts: each write, then each again with a wrapping operand, and
 * cells of what a `&mut` borrows between writes to the others.
 */
const cellWrites = [
	[0, 1, 2, 3, 4, 5, 6, 7],
	[8, 249, 250, 3, 252, 13, 1, 0],
	[5, 2, 4, 253, 0, 5, 3],
	[],
].map((bytes) => Uint8Array.of(...op("pointers::cell_writes"), 0, ...bytes));

export const cases: Uint8Array[] = [...unsafeCells, ...rawCells, ...cellWrites];
