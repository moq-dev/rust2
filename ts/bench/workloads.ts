// The workloads every route runs, as inputs to the codec fixture's `bench::run`: moq's own
// benchmarks' cases, from `js/net/bench/varint.ts` and `lite-varint.ts`. Each decode
// reads what its encode wrote, and a split decode reads it as it arrives over a stream.

export interface Workload {
	/** What is coded, such as `varint/quic/2B` or `frames/video/lite-07`, iterated, or converted. */
	name: string;
	op: "encode" | "decode" | "iterate" | "map" | "convert" | "add" | "compare" | "order" | "handoff";
	/** Varints, frame headers, messages, items, keys, timestamps, integers, or handoffs per call, which each time is divided by. */
	items: number;
	input: Uint8Array;
}

/** Each workload, with the encode outputs that make the decode inputs from `encode`. */
export function workloads(encode: (input: Uint8Array) => Uint8Array): Workload[] {
	const out: Workload[] = [];
	const pair = (name: string, items: number, op: number, selector: number, params: bigint[]) => {
		const input = Uint8Array.of(op, selector, ...params.flatMap(u64));
		const wire = encode(input);
		// A varint's name ends in its size, which the encode says.
		name = name.replace("%", `${wire.byteLength / items}B`);
		out.push({ name, op: "encode", items, input });
		out.push({ name, op: "decode", items, input: Uint8Array.of(op + 1, selector, ...wire) });
		return wire;
	};

	// One call codes this many copies, as `varint.ts` decodes them through one Cursor.
	const run = 1024;
	// The largest value of each 1, 2, 4, and 8-byte form that a JS `number` holds.
	const quic = [2 ** 6 - 1, 2 ** 14 - 1, 2 ** 30 - 1, Number.MAX_SAFE_INTEGER];
	const leadingOnes = [2 ** 7 - 1, 2 ** 14 - 1, 2 ** 28 - 1, Number.MAX_SAFE_INTEGER];
	// bench.rs reads the selector through the harness's `form`: QUIC, then draft-17's leading ones.
	for (const v of quic) pair("varint/quic/%", run, 0, 0, [BigInt(run), BigInt(v)]);
	for (const v of leadingOnes) pair("varint/leading-ones/%", run, 0, 1, [BigInt(run), BigInt(v)]);

	// `lite-varint.ts`'s samples, at lite-06 (QUIC varints) and lite-07 (leading ones), which
	// the harness's `lite_version` selects as 5 and 6.
	for (const [version, selector] of [
		["lite-06", 5],
		["lite-07", 6],
	] as const) {
		// One group: 30 fps, a keyframe every 10 frames. Params: count, delta (µs), first
		// size, size, key interval, key size.
		const video = pair(`frames/video/${version}`, 60, 2, selector, [60n, 33_333n, 60_000n, 8_000n, 10n, 17_000n]);
		// The same headers in reads shorter than any of them, so each comes up short at least
		// once and is decoded again: `bench.rs`'s op 7, whose param is the read size.
		const read = 3;
		out.push({
			name: `frames/video/${version}/${read}B-reads`,
			op: "decode",
			items: 60,
			input: Uint8Array.of(7, selector, ...u64(BigInt(read)), ...video),
		});
		pair(`frames/audio/${version}`, 50, 2, selector, [50n, 20_000n, 160n, 160n, 0n, 0n]);
		pair(`group/${version}`, 1, 4, selector, [3n, 1_234n]);
	}

	// Iterator chains over bytes, as `bench.rs`'s op 6 picks them by selector; no byte is
	// 0xff, so `any` reads them all.
	const bytes = Uint8Array.from({ length: 1024 }, (_, i) => (i * 167 + 13) % 255);
	const chains = ["map-filter-count", "any", "filter-collect", "sum", "skip-take-fold", "enumerate-for", "zip-filter-count", "zip-rev-filter-count"];
	chains.forEach((chain, selector) => {
		out.push({ name: `iter/${chain}`, op: "iterate", items: bytes.length, input: Uint8Array.of(6, selector, ...bytes) });
	});

	// A `BTreeMap` of each size, through `bench.rs`'s op 8: each key inserted, looked up, and removed.
	for (const count of [16, 256, 4096]) {
		out.push({ name: `btree/${count}`, op: "map", items: count, input: Uint8Array.of(8, 0, ...u64(BigInt(count))) });
	}

	// Microseconds since moq's 2020 epoch, a frame apart at 30 fps, to 90 kHz and back:
	// each product passes 2^53 and each quotient is back under it. `bench.rs`'s op 9 converts
	// them as moq-net's `Timestamp::convert` does in `u128`, then in `u64`.
	const count = 1024;
	const start = 190_000_000_000_000n;
	const micros = [start, 33_333n, 1_000_000n, 90_000n];
	const ninety = [(start * 90_000n) / 1_000_000n, 3_000n, 90_000n, 1_000_000n];
	for (const [selector, type] of ["u128", "u64"].entries()) {
		for (const [name, params] of [
			["micros-to-90k", micros],
			["90k-to-micros", ninety],
		] as const) {
			const input = Uint8Array.of(9, selector, ...[BigInt(count), ...params].flatMap(u64));
			out.push({ name: `time/${type}/${name}`, op: "convert", items: count, input });
		}
	}

	// The same frames stamped by moq-net's `Timestamp::checked_add` (op 10), and interleaved
	// by its `Ord` (op 11) with an audio track's at 48 kHz, from the same instant: every
	// value is under 2^53, as moq's TypeScript keeps them, and every cross product past it.
	const video = [start, 33_333n, 1_000_000n];
	const audio = [(start * 48_000n) / 1_000_000n, 1_024n, 48_000n];
	const time = (op: number, params: bigint[]) => Uint8Array.of(op, 0, ...[BigInt(count), ...params].flatMap(u64));
	out.push({ name: "time/checked-add/micros", op: "add", items: count, input: time(10, video) });
	out.push({ name: "time/ord/micros-48k", op: "compare", items: count, input: time(11, [...video, ...audio]) });

	// `bench.rs`'s op 12: integers ordered by std's own `Ord`, which the translation tests.
	const orders = ["sort", "sort-by-key", "iter-max-min", "ord-max-min", "binary-heap"];
	orders.forEach((name, selector) => {
		out.push({ name: `order/${name}`, op: "order", items: count, input: Uint8Array.of(12, selector, ...u64(BigInt(count))) });
	});
	return out;
}

/** The async handoff at each await depth: `bench::run` of the task fixture. */
export function handoffs(): Workload[] {
	// One call hands this many values over.
	const count = 1000;
	return [1, 4, 16].map((depth) => ({
		name: `depth ${depth}`,
		op: "handoff",
		items: count,
		input: Uint8Array.of(depth, ...u32(count)),
	}));
}

function u32(v: number): number[] {
	const bytes = new Uint8Array(4);
	new DataView(bytes.buffer).setUint32(0, v);
	return [...bytes];
}

function u64(v: bigint): number[] {
	const bytes = new Uint8Array(8);
	new DataView(bytes.buffer).setBigUint64(0, v);
	return [...bytes];
}
