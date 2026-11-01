// The `push/` workloads' hand-written route: `bench.rs`'s op 13, as TypeScript grows a
// list: `push` onto an array, for a `VecDeque` too, as JS has no deque.

/** Run op 13, as `bench::run` does, for the count `data` starts with. */
export function push(data: Uint8Array): Uint8Array {
	const padded = new Uint8Array(8);
	padded.set(data.subarray(0, 8));
	const count = Math.min(Number(new DataView(padded.buffer).getBigUint64(0)), 1 << 16);
	const v: number[] = [];
	for (let i = 0; i < count; i++) v.push(i);
	const out = new Uint8Array(16);
	const view = new DataView(out.buffer);
	view.setBigUint64(0, BigInt(v.length));
	view.setBigUint64(8, BigInt(v[v.length >> 1] ?? 0));
	return out;
}
