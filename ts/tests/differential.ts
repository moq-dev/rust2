// Runs a fixture's harness both ways on the same inputs and fails on any difference:
// natively, as the fixture's wasm32 build, and as its TypeScript translation.
//
// usage: node differential.ts <fixture.wasm> <harness.ts> [cases]
//
// A harness, or a hand-written caller wrapping one, exports `run(input: rt.ByteSlice):
// Vec<u8>`, and may export `cases`, inputs it names, which run with the generated ones,
// `ops`, the bytes that pick each of its cases, if not the first byte alone,
// and `within(input, native, translated)`, which accepts outputs that differ only where
// Rust leaves the result to the platform, as libm's are.
// The wasm build exports `input`, `run`, and `output` around the crate's entry, which
// `wasm.ts` calls. A panic traps natively and throws `Panic` in
// TypeScript, and the two agree only if their messages do. An abort traps natively
// without a panic message, as wasm32 has nowhere to print std's, and throws `Abort` in
// TypeScript, so the two agree that it aborted but not on why.

import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";

import { type Outcome, Wasm } from "./wasm.ts";

const [wasmPath, harnessPath, countArg] = process.argv.slice(2);
if (!wasmPath || !harnessPath) throw new Error("usage: differential.ts <fixture.wasm> <harness.ts> [cases]");
const count = Number(countArg ?? "20000");

const harness = await import(pathToFileURL(harnessPath).href);
// The translation's runtime, beside the harness, which makes the input its slice type.
const rt = await import(pathToFileURL(join(dirname(harnessPath), "rt.ts")).href);
const wasm = new Wasm(await WebAssembly.compile(await readFile(wasmPath)));

function translated(input: Uint8Array): Outcome {
	try {
		const out = harness.run(rt.byteSliceFrom(input));
		return out.data.slice(0, out.length);
	} catch (err) {
		if (err instanceof Error && err.name === "Abort") return "abort";
		if (err instanceof Error && err.name === "Panic") return { panic: err.message };
		throw err;
	}
}

// xorshift64*: the same cases on every run.
let state = 0x9e3779b97f4a7c15n;
function next(): bigint {
	state ^= state >> 12n;
	state ^= (state << 25n) & 0xffffffffffffffffn;
	state ^= state >> 27n;
	return (state * 0x2545f4914f6cdd1dn) & 0xffffffffffffffffn;
}
function byte(): number {
	return Number(next() & 0xffn);
}

/** Bytes biased toward the values codecs care about: zero, all ones, and boundaries. */
function interesting(): number {
	const pick = Number(next() % 8n);
	switch (pick) {
		case 0:
			return 0;
		case 1:
			return 0xff;
		case 2:
			return [0x3f, 0x40, 0x7f, 0x80, 0xbf, 0xc0, 0xdf, 0xe0, 0xef, 0xf0, 0xf7, 0xf8, 0xfb, 0xfc, 0xfd, 0xfe][Number(next() % 16n)];
		default:
			return byte();
	}
}

function show(outcome: Outcome): string {
	if (outcome === "abort") return outcome;
	return outcome instanceof Uint8Array ? Buffer.from(outcome).toString("hex") : `panic ${JSON.stringify(outcome.panic)}`;
}

/** An outcome to read: as text if it is lines of printable ASCII, as a log is. */
function display(outcome: Outcome): string {
	if (outcome instanceof Uint8Array && outcome.every((b) => b === 0x0a || (b >= 0x20 && b < 0x7f))) {
		return `\n${Buffer.from(outcome).toString()}`;
	}
	return show(outcome);
}

const cases: Uint8Array[] = [new Uint8Array(0)];
const ops: Uint8Array[] = harness.ops ?? Array.from({ length: 256 }, (_, op) => Uint8Array.of(op));
if (new Set(ops.map((op) => Buffer.from(op).toString("hex"))).size !== ops.length) throw new Error("two ops are the same bytes, so one of them never runs");
// Every op with every short tail, so each path's edges are covered exhaustively.
for (const op of ops) {
	cases.push(op);
	for (let b = 0; b < 256; b += 17) cases.push(Uint8Array.of(...op, b));
}
// Full-width values through the first ops: each u64 boundary 2^k - 1, 2^k, 2^k + 1.
for (const op of ops.slice(0, 8)) {
	for (let k = 0n; k <= 64n; k++) {
		for (const delta of [-1n, 0n, 1n]) {
			const value = ((1n << k) + delta) & 0xffffffffffffffffn;
			const bytes = new Uint8Array(8);
			new DataView(bytes.buffer).setBigUint64(0, value);
			for (const selector of [0, 1, 2, 3, 4, 5, 6]) {
				cases.push(Uint8Array.of(...op, selector, ...bytes, ...bytes, ...bytes));
			}
		}
	}
}
// A harness's own cases, which name what they cover.
if (Array.isArray(harness.cases)) for (const input of harness.cases) cases.push(input);
for (let i = 0; i < count; i++) {
	const len = Number(next() % 48n);
	const input = new Uint8Array(len + 1);
	input[0] = byte();
	for (let j = 1; j <= len; j++) input[j] = interesting();
	cases.push(input);
}

let failures = 0;
let panics = 0;
let aborts = 0;
for (const input of cases) {
	const want = wasm.call(input);
	let got: Outcome;
	try {
		got = translated(input);
	} catch (err) {
		console.error(`input ${show(input)}: the translation threw ${err}`);
		failures++;
		continue;
	}
	if (want === "abort") aborts++;
	else if (!(want instanceof Uint8Array)) panics++;
	const close = want instanceof Uint8Array && got instanceof Uint8Array && harness.within?.(input, want, got) === true;
	if (show(want) !== show(got) && !close) {
		failures++;
		if (failures <= 20) {
			console.error(`input  ${show(input)}\nnative ${display(want)}\nts     ${display(got)}\n`);
		}
	}
}

console.log(`${cases.length} cases, ${panics} panics, ${aborts} aborts, ${failures} differences`);
if (failures > 0) process.exit(1);
