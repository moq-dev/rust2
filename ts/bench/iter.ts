// The iterator workloads' hand-written route: each chain in the codec fixture's
// `bench.rs` op 6, written with ES iterator helpers (`Iterator.prototype.map` and the
// rest), as TypeScript would write it without the translation's `rt.Iter`. Helpers
// without an ES counterpart, `enumerate` and `zip` here, are a `for ... of` with a counter.

/** Run one chain, as `bench::run`'s op 6 does, over `data`. */
export function iterate(chain: number, data: Uint8Array): Uint8Array {
	switch (chain) {
		case 0:
			return u64(
				data
					.values()
					.map((b) => b >> 1)
					.filter((b) => (b & 1) === 0)
					.reduce((n) => n + 1, 0),
			);
		case 1:
			return Uint8Array.of(data.values().some((b) => b === 0xff) ? 1 : 0);
		case 2:
			return u64(
				data
					.values()
					.filter((b) => (b & 1) === 1)
					.toArray().length,
			);
		case 3:
			return u64(data.values().reduce((sum, b) => sum + b, 0));
		case 4:
			return u64(
				data
					.values()
					.drop(16)
					.take(512)
					.reduce((acc, b) => acc ^ b, 0),
			);
		case 6: {
			// No ES helper zips: the back read by index beside the front's `for ... of`.
			let n = 0;
			let back = data.length;
			for (const a of data.values()) if (a < data[--back]) n++;
			return u64(n);
		}
		case 7: {
			// The pairs below the bytes' length, the range's excess past them never read.
			let n = 0;
			for (let i = data.length - 1; i >= 0; i--) if ((i & 0xff) < data[i]) n++;
			return u64(n);
		}
		default: {
			let last = 0;
			let i = 0;
			for (const b of data.values()) {
				if (b === 0x80) last = i;
				i++;
			}
			return u64(last);
		}
	}
}

function u64(value: number): Uint8Array {
	const out = new Uint8Array(8);
	new DataView(out.buffer).setBigUint64(0, BigInt(value));
	return out;
}
