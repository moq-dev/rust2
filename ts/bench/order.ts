// The `order/` workloads' hand-written route: `bench.rs`'s op 12, as TypeScript orders
// numbers without a library: a typed array's `sort`, a stable `sort` by key, a loop for
// the greatest and least, a binary heap of its own, as JS has none, and a `Map` of the
// last value of each key, its keys then sorted.

/** Run op 12, as `bench::run` does, for the selector and the count `data` starts with. */
export function order(selector: number, data: Uint8Array): Uint8Array {
	const padded = new Uint8Array(8);
	padded.set(data.subarray(0, 8));
	const count = Math.min(Math.max(Number(new DataView(padded.buffer).getBigUint64(0)), 1), 1 << 16);
	let v = Uint32Array.from({ length: count }, (_, i) => (i * 40503) % (1 << 16));
	switch (selector) {
		case 0:
			v.sort();
			break;
		case 1:
			v = Uint32Array.from(Array.from(v).sort((a, b) => (a % 1024) - (b % 1024)));
			break;
		case 2:
		case 3: {
			let hi = 0;
			let lo = 0xffffffff;
			for (const x of v) {
				if (x > hi) hi = x;
				if (x < lo) lo = x;
			}
			v = Uint32Array.of(hi, lo);
			break;
		}
		case 5:
		case 6: {
			const last = new Map<number, number>();
			for (const x of v) last.set(x % 512, x);
			const keys = Uint32Array.from(last.keys()).sort();
			const end = keys[keys.length - 1];
			v = Uint32Array.of(keys[0], keys.length, selector === 5 ? (last.get(end) as number) : end);
			break;
		}
		default: {
			const heap: number[] = [];
			for (const x of v) {
				let i = heap.push(x) - 1;
				while (i > 0 && heap[(i - 1) >> 1] < x) {
					heap[i] = heap[(i - 1) >> 1];
					i = (i - 1) >> 1;
				}
				heap[i] = x;
			}
			for (let n = 0; n < count; n++) {
				v[n] = heap[0];
				const x = heap.pop() as number;
				const len = heap.length;
				if (len === 0) break;
				let i = 0;
				for (;;) {
					let child = 2 * i + 1;
					if (child >= len) break;
					if (child + 1 < len && heap[child + 1] > heap[child]) child++;
					if (heap[child] <= x) break;
					heap[i] = heap[child];
					i = child;
				}
				heap[i] = x;
			}
		}
	}
	const out = new Uint8Array(24);
	const view = new DataView(out.buffer);
	[v[0], v[v.length >> 1], v[v.length - 1]].forEach((x, i) => view.setBigUint64(8 * i, BigInt(x)));
	return out;
}
