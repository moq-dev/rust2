// The `hash/` workloads' hand-written route: `bench.rs`'s op 14 over a `Map`, as TypeScript
// keeps a map of numbers.

/** Run op 14, as `bench::run` does, for the count `data` starts with: with `selector` 1, inserts alone. */
export function hashed(selector: number, data: Uint8Array): Uint8Array {
	const padded = new Uint8Array(8);
	padded.set(data.subarray(0, 8));
	const count = Math.min(Number(new DataView(padded.buffer).getBigUint64(0)), 1 << 16);
	const key = (i: number) => (i * 40503) % (1 << 16);
	const map = new Map<number, number>();
	for (let i = 0; i < count; i++) map.set(key(i), i);
	let sum = 0;
	if (selector !== 1) {
		for (let i = 0; i < count; i++) sum += map.get(key(count - 1 - i)) ?? 0;
		for (let i = 0; i < count; i++) map.delete(key(i));
	}
	const out = new Uint8Array(16);
	const view = new DataView(out.buffer);
	view.setBigUint64(0, BigInt(sum));
	view.setBigUint64(8, BigInt(map.size));
	return out;
}
