// The benchmark's hand-written route: each workload in the codec fixture's `bench.rs`,
// written with moq's own TypeScript codec (`js/net`) the way its benchmarks and session
// code call it. Varints are sync, as `bench/varint.ts` times them; a frame header decodes
// through the Reader's sync `tryDecode`, as the subscriber's read loop does, and so does
// each read's worth of a split one; every other encode and decode goes through moq's
// async Writer and Reader, as `lite-varint.ts`.
//
// `moq/` imports resolve to the moq checkout when `main.ts` bundles this file.

import { Group } from "moq/js/net/src/lite/group.ts";
import { Version as Lite } from "moq/js/net/src/lite/version.ts";
import { Cursor, Reader, type StreamVersion, Writer } from "moq/js/net/src/stream.ts";
import * as Varint from "moq/js/net/src/varint.ts";

import { iterate } from "./iter.ts";
import { order } from "./order.ts";
import { push } from "./push.ts";
import { rescale } from "./rescale.ts";
import { ordered } from "./sorted.ts";
import { interleave, stamp } from "./timestamp.ts";

type Version = (typeof Lite)[keyof typeof Lite];

// bench.rs reads a selector through the harness's `form` and `lite_version`; these are the
// ones the workloads use. A form is the varint encoding of a version moq still speaks:
// Lite 06 is the last with QUIC varints, and Lite 07 the first with leading ones.
const FORMS: Record<number, { version: StreamVersion; encode: (dst: ArrayBuffer, v: number) => Uint8Array }> = {
	0: { version: Lite.DRAFT_06, encode: Varint.encodeTo },
	1: { version: Lite.DRAFT_07, encode: Varint.encodeLeadingOnesTo },
};
const LITE: Record<number, Version> = { 5: Lite.DRAFT_06, 6: Lite.DRAFT_07 };

const scratch = new ArrayBuffer(9);

/** Run one workload, as `bench::run` does. */
export function run(input: Uint8Array): Uint8Array | Promise<Uint8Array> {
	const [op, selector] = input;
	const data = input.subarray(2);
	switch (op) {
		case 0:
			return encodeVarints(data, selector);
		case 1:
			return decodeVarints(data, selector);
		case 2:
			return encodeFrames(data, lite(selector));
		case 3:
			return decodeFrames(data, lite(selector));
		case 4:
			return encodeGroup(data, lite(selector));
		case 5:
			return decodeGroup(data, lite(selector));
		case 6:
			return iterate(selector, data);
		case 7:
			return decodeSplit(data, lite(selector));
		case 8:
			return ordered(data);
		case 9:
			return rescale(selector, data);
		case 10:
			return stamp(data);
		case 11:
			return interleave(data);
		case 12:
			return order(selector, data);
		case 13:
			return push(data);
		default:
			throw new Error(`unknown workload ${op}`);
	}
}

function form(selector: number) {
	const form = FORMS[selector];
	if (!form) throw new Error(`unknown varint form ${selector}`);
	return form;
}

function lite(selector: number): Version {
	const version = LITE[selector];
	if (version === undefined) throw new Error(`unknown lite version ${selector}`);
	return version;
}

/** The `i`th big-endian u64 of `data`, zero-padded as `take_u64` pads it. */
function u64(data: Uint8Array, i: number): bigint {
	const bytes = new Uint8Array(8);
	bytes.set(data.subarray(i * 8, i * 8 + 8));
	return new DataView(bytes.buffer).getBigUint64(0);
}

function u64s(...values: (number | bigint)[]): Uint8Array {
	const out = new Uint8Array(values.length * 8);
	const view = new DataView(out.buffer);
	values.forEach((v, i) => view.setBigUint64(i * 8, BigInt.asUintN(64, BigInt(v))));
	return out;
}

function encodeVarints(data: Uint8Array, selector: number): Uint8Array {
	const { encode } = form(selector);
	const count = Number(u64(data, 0));
	const value = Number(u64(data, 1));
	const out = new Uint8Array(count * 9);
	let offset = 0;
	for (let i = 0; i < count; i++) {
		const varint = encode(scratch, value);
		out.set(varint, offset);
		offset += varint.byteLength;
	}
	return out.subarray(0, offset);
}

function decodeVarints(wire: Uint8Array, selector: number): Uint8Array {
	const cursor = new Cursor(wire, form(selector).version);
	let count = 0;
	let last = 0;
	while (cursor.remaining > 0) {
		last = cursor.u53();
		count++;
	}
	return u64s(count, last);
}

// moq's `lite/publisher.ts` and `lite/group.ts`, which keep these private.
const zigzag = (d: bigint) => (d << 1n) ^ (d >> 63n);
const unzigzag = (v: bigint) => (v >> 1n) ^ -(v & 1n);

function encodeFrames(data: Uint8Array, version: Version): Promise<Uint8Array> {
	const count = Number(u64(data, 0));
	const delta = BigInt.asIntN(64, u64(data, 1));
	const first = Number(u64(data, 2));
	const size = Number(u64(data, 3));
	const keyEvery = Number(u64(data, 4));
	const keySize = Number(u64(data, 5));
	return Writer.collect(version, async (w) => {
		for (let n = 0; n < count; n++) {
			if (n === 0) {
				await w.u62(zigzag(0n));
				await w.u53(first);
			} else {
				await w.u62(zigzag(delta));
				await w.u53(keyEvery !== 0 && n % keyEvery === 0 ? keySize : size);
			}
		}
	});
}

function decodeFrames(wire: Uint8Array, version: Version): Uint8Array {
	const reader = new Reader(undefined, wire, version);
	let count = 0;
	let total = 0;
	let last = 0n;
	const header = (c: Cursor) => {
		last = unzigzag(c.u62());
		total += c.u53();
		return true;
	};
	while (reader.tryDecode(header) !== undefined) count++;
	return u64s(count, total, last);
}

// The headers as they arrive a read at a time. The Reader over the bytes so far stands in
// for one that a stream fills, which awaits each read: its `tryDecode` drains every
// complete header, and a cut one comes up short and is decoded again on the next read.
function decodeSplit(data: Uint8Array, version: Version): Uint8Array {
	const read = Math.max(1, Number(u64(data, 0)));
	const wire = data.subarray(8);
	let start = 0;
	let count = 0;
	let total = 0;
	let last = 0n;
	let short = 0;
	// Nothing is kept from a header until its last read, which a retry runs again.
	const header = (c: Cursor) => {
		const delta = c.u62();
		const size = c.u53();
		last = unzigzag(delta);
		total += size;
		start += c.offset;
		return true;
	};
	for (let end = 0; end < wire.byteLength; ) {
		end = Math.min(wire.byteLength, end + read);
		const reader = new Reader(undefined, wire.subarray(start, end), version);
		while (reader.tryDecode(header) !== undefined) count++;
		if (start < end) short++;
	}
	if (start < wire.byteLength) throw new Error("the wire ends mid-header");
	return u64s(count, total, last, short);
}

function encodeGroup(data: Uint8Array, version: Version): Promise<Uint8Array> {
	const group = new Group({ subscribe: u64(data, 0), sequence: Number(u64(data, 1)) });
	return Writer.collect(version, (w) => group.encode(w, version));
}

async function decodeGroup(wire: Uint8Array, version: Version): Promise<Uint8Array> {
	const group = await Group.decode(new Reader(undefined, wire, version), version);
	// The wire is one message, which Group.decode reads whole.
	return u64s(group.subscribe, group.sequence, group.frameStart, wire.byteLength);
}
