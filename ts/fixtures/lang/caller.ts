// A hand-written caller of the translation, as moq's TypeScript is: it runs the harness,
// and prints a `Fault` with the `Display` and `Debug` that `printable` brings along,
// which nothing translated calls. `differential.ts` runs it beside native Rust's
// `harness::entry`, from the output directory.

import * as Fault from "./fmt.Fault.ts";
import { cases as named, run as harness, op as pick } from "./harness.ts";
import * as rt from "./rt.ts";

/** The first two bytes, which pick the case named `name`. */
function op(name: string): number[] {
	const at = pick(name);
	return [at >> 8, at & 0xff];
}

/** The bytes that pick each case, through `op`, which panics for one past what two bytes pick, and a `Fault`'s first byte. */
export const ops: Uint8Array[] = [...named().map(([name]) => Uint8Array.from(op(name))), Uint8Array.of(0xff)];

export function run(input: rt.ByteSlice): rt.ByteVec {
	if (rt.byteSliceFirst(input) !== 0xff) return harness(input);
	const fault = Fault.new(rt.byteSliceRange(input, 1));
	const display: rt.Fmt = (f) => Fault.fmt$fmt_Display(fault, f);
	const debug: rt.Fmt = (f) => Fault.fmt$fmt_Debug(fault, f);
	const text = rt.fmtFormat([
		display,
		"|",
		rt.fmtSpec({ align: ">", width: 16 }, display),
		"|",
		debug,
		"|",
		rt.fmtSpec({ alternate: true }, debug),
	]);
	return rt.byteVecFrom(new TextEncoder().encode(text));
}

/** `shapes::loops` with each first byte that one of its cases of `!` takes, with one more byte and without. */
const loops = [0xf1, 0xf3, 0xf4, 0xf5, 0xf6, 0xf9, 0xfa].flatMap((b) => [Uint8Array.of(...op("shapes::loops"), 0, b), Uint8Array.of(...op("shapes::loops"), 0, b, 1)]);

/** A `Duration`'s bytes as `durations` takes them: its seconds, then its nanoseconds. */
function duration(secs: bigint, nanos: number): number[] {
	const bytes = new Uint8Array(12);
	const view = new DataView(bytes.buffer);
	view.setBigUint64(0, secs);
	view.setUint32(8, nanos);
	return [...bytes];
}

/**
 * `durations` where seconds leave a safe integer or reach `u64::MAX`, nanoseconds carry,
 * a sum carries them past it, and a factor makes the nanoseconds' product or a
 * quotient's remainder pass 2^53.
 */
const edges = [0n, 1n, (1n << 53n) - 1n, 1n << 53n, (1n << 64n) - 1n].flatMap((secs) => [0, 1, 999_999_999].map((nanos) => duration(secs, nanos)));
const factors = [0, 1, 3, 1000, 9_007_199, 1_000_000_000, 0xffffffff].map((n) => [n >>> 24, (n >>> 16) & 0xff, (n >>> 8) & 0xff, n & 0xff]);
const durationsOp = op("durations::durations");
const durations = [
	...[0n, 1n, 1n << 32n, (1n << 53n) - 1n, 1n << 53n, (1n << 64n) - 2n, (1n << 64n) - 1n].flatMap((secs) =>
		[0, 1, 999_999_999, 1_000_000_000, 0xffffffff].flatMap((nanos) => [0, 1].map((kind) => Uint8Array.of(...durationsOp, kind, ...duration(secs, nanos)))),
	),
	...edges.flatMap((a) => edges.flatMap((b) => factors.flatMap((n) => [2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 13].map((kind) => Uint8Array.of(...durationsOp, kind, ...a, ...b, ...n))))),
	// Each side of where minutes, hours, and a `u128` of nanoseconds pass `u64::MAX` seconds.
	...[2n ** 64n / 60n, 2n ** 64n / 3600n].flatMap((units) => [units, units + 1n].map((n) => wide(n << 64n))),
	...[(7n << 64n) + 999_999_999n, 2n ** 64n * 1_000_000_000n - 1n, 2n ** 64n * 1_000_000_000n, 2n ** 128n - 1n].map((n) => wide(n)),
];

/** `durations`' larger units, of a `u128` whose high half is also its `u64`. */
function wide(n: bigint): Uint8Array {
	const bytes = new Uint8Array(19);
	bytes.set([...durationsOp, 12]);
	const view = new DataView(bytes.buffer);
	view.setBigUint64(3, n >> 64n);
	view.setBigUint64(11, n & 0xffff_ffff_ffff_ffffn);
	return bytes;
}

/** A double's bits, big-endian, as `floats` takes them. */
function double(x: number | bigint): number[] {
	const bytes = new Uint8Array(8);
	const view = new DataView(bytes.buffer);
	if (typeof x === "bigint") view.setBigUint64(0, x);
	else view.setFloat64(0, x);
	return [...bytes];
}

/**
 * `floats`' operands: each side of a half, of 2^52 and 2^53, where each integer type
 * saturates, of a nanosecond and `u64::MAX` seconds, subnormals, infinities, and NaNs,
 * one of them negative and one with a payload.
 */
const doubles: (number | bigint)[] = [
	0, -0, 0.5, -0.5, 1.5, 2.5, -2.5, 0.49999999999999994, 1, -1, 3, 0.1, 1 / 3, 2 ** 52 - 0.5, 2 ** 52, 2 ** 53, 2 ** 53 + 2,
	255.5, -128.5, 2147483647.5, -2147483648.5, 4294967295.9, 2 ** 63, -(2 ** 63), -(2 ** 63) - 2048, 2 ** 64, 2 ** 64 - 2048,
	2 ** 127, 2 ** 128, -(2 ** 128), 1e-9, 0.5e-9, 1.5e-9, 0.999999999, 0.9999999995, 1.0000000005, 4.656612873077393e-10,
	1e-310, 5e-324, 2.2250738585072014e-308, Number.MAX_VALUE, -Number.MAX_VALUE, Infinity, -Infinity, NaN,
	0xfff8000000000000n, 0x7ff0000000000001n,
];
/** Integers whose `f64` rounds, as `floats`' kind 2 takes them: 16 bytes of a `u128`. */
const wides = [0n, 1n, (1n << 53n) + 1n, (1n << 53n) + 3n, (1n << 54n) + 2n, (1n << 63n) + 1025n, (1n << 64n) - 1n, (1n << 64n) - 1025n, (1n << 64n) + (1n << 11n) + 1n, (1n << 127n) + 1n, (1n << 128n) - 1n, (1n << 128n) - (1n << 74n), -(1n << 100n) - 1n];
const floatsOp = op("floats::floats");
/** How many cases the first two bytes pick among, modulo. */
const count = named().length;
const floats = [
	...Array.from({ length: 10 }, (_, kind) => doubles.flatMap((a) => doubles.map((b) => Uint8Array.of(...floatsOp, kind, ...double(a), ...double(b), 1)))).flat(),
	// `from_secs_f64`, `mul_f64`, and `div_f64` panicking, and `Duration::MAX.mul_f64`.
	...[18, 28, 38].flatMap((kind) => doubles.map((a) => Uint8Array.of(...floatsOp, kind, ...double(a), ...double(a), 3))),
	...wides.map((v) => {
		const bytes = new Uint8Array(16);
		new DataView(bytes.buffer).setBigUint64(0, BigInt.asUintN(64, v >> 64n));
		new DataView(bytes.buffer).setBigUint64(8, BigInt.asUintN(64, v));
		return Uint8Array.of(...floatsOp, 2, ...bytes);
	}),
];

/**
 * `floats::formats`' doubles: ties at each precision, which std rounds half to even on the
 * exact value, carries through nines, each side of where `Debug` takes an exponent, the
 * shortest digits that are not the nearest ones, and the extremes.
 */
const formatted: (number | bigint)[] = [
	...doubles,
	0.125, 0.375, 0.625, 0.875, 0.0625, 3.5, 4.5, 9.5, 99.5, 999.5, 0.95, 9.995, 1.25, 1.35, 2.675, 1e-4, 9.999999999999999e-5,
	1e16, 9999999999999998, 1e21, 1e22, 1e23, 5e-324, 2.225073858507201e-308, 123456789.125, 1.5e300, -1.5e-300, 2 ** -1022 * 1.5,
	0.1 + 0.2, 100, 1e15, 1e-7, 0.000123, -9.5, -0.5,
];
const formatsOp = op("floats::formats");
const formats = [
	...formatted.flatMap((a) => Array.from({ length: 22 }, (_, p) => Uint8Array.of(...formatsOp, 0, ...double(a), p))),
	...formatted.flatMap((a) => [1, 3, 4].map((kind) => Uint8Array.of(...formatsOp, kind, ...double(a), 0))),
	// Seeds for `parses`' decimals, a signaling NaN's bits among them, which a seed keeps.
	...doubles.map((a) => Uint8Array.of(...op("floats::parses"), 2, ...double(a))),
	...formatted.flatMap((a) => [[1, 2], [2, 1], [NaN, 1], [1, NaN], [-0, 0], [0, -0]].map(([b, c]) => Uint8Array.of(...formatsOp, 2, ...double(a), ...double(b), ...double(c)))),
];

/**
 * `floats::parses`' texts: std's grammar at each edge, past each end of a double's range,
 * and digits that round half to even, or just off it past 19 digits.
 */
const parsed = [
	"", "+", "-", ".", "+.", "-.", "e5", ".e5", "1e", "1e+", "1e-", "1.e5", ".5", "5.", "-.5e-3", "+1", "++1", "+-1", "1_000", "0x10", " 1",
	"1 ", "1,5", "1.5.5", "1e5e5", "1E+05", "1e400", "-1e400", "1e-400", "1e99999999999999999999", "0e99999999999", "-0", "+0.0e-5",
	"0000.0000", "inf", "-inf", "+inf", "infinity", "INFINITY", "InFiNiTy", "infin", "in", "infinityy", "nan", "NaN", "-nan", "+NAN",
	"nan1", "Infinity", "i\u0307nf", "\u0131nf", "2.2250738585072011e-308", "2.2250738585072012e-308", "4.9406564584124654e-324",
	"2.4703282292062327e-324", "2.4703282292062328e-324", "1.7976931348623157e308", "1.7976931348623158e308", "1.7976931348623159e308",
	"9007199254740993", "9007199254740993.0000000000000000001", "9007199254740992.9999999999999999999", "123456789012345678901234567890",
	"0.1000000000000000055511151231257827021181583404541015625", "1e23", "8.98846567431158e307", `0.${"0".repeat(400)}1e400`,
	`1${"0".repeat(800)}e-800`, `${"9".repeat(800)}e-1100`, "1e65535", `1e-65536`, `0.${"0".repeat(70000)}1e70000`,
].map((text) => Uint8Array.of(...op("floats::parses"), 0, ...new TextEncoder().encode(text)));

/** A single's bits, big-endian, as `singles` takes them. */
function single(x: number | bigint): number[] {
	const bytes = new Uint8Array(4);
	const view = new DataView(bytes.buffer);
	if (typeof x === "bigint") view.setUint32(0, Number(x));
	else view.setFloat32(0, x);
	return [...bytes];
}

/**
 * `singles`' operands: each side of a half, of 2^23 and 2^24, where each integer type
 * saturates, where a single's range and its normals end, the bounds of `Debug`'s
 * exponent, infinities, and NaNs, one negative, one with a payload, and one signaling.
 */
const singlesValues: (number | bigint)[] = [
	0, -0, 0.5, -0.5, 1.5, 2.5, -2.5, 0.49999997, 1, -1, 3, 0.1, 1 / 3, 2 ** 23 - 0.5, 2 ** 23, 2 ** 24, 2 ** 24 + 2, 255.5, -128.5,
	2147483520, 2 ** 31, -(2 ** 31), 4294967040, 2 ** 32, 2 ** 63, -(2 ** 63), 2 ** 64, 2 ** 127, 1e-9, 0.99999994, 1.0000001, 1e-4, 1e16,
	1e-40, 2 ** -149, 2 ** -126, 3.4028234663852886e38, -3.4028234663852886e38, Infinity, -Infinity, NaN, 0xffc00000n, 0x7fc00123n, 0x7f800001n,
];
const singlesOp = op("singles::singles");
const singlesCases = [
	...Array.from({ length: 10 }, (_, kind) => singlesValues.flatMap((a) => singlesValues.map((b) => Uint8Array.of(...singlesOp, kind, ...single(a), ...single(b), 1)))).flat(),
	// `from_secs_f32`, `mul_f32`, and `div_f32` panicking, and `Duration::MAX.mul_f32`.
	...[18, 28, 38].flatMap((kind) => singlesValues.map((a) => Uint8Array.of(...singlesOp, kind, ...single(a), ...single(a), 3))),
	// Integers whose `f32` rounds once where through an `f64` it would round twice: past
	// 2^53, a bit just past halfway between two singles.
	...wides.concat([(1n << 60n) + (1n << 36n) + 1n, (1n << 64n) - (1n << 39n) - 1n, (1n << 127n) + (1n << 103n) + 1n, (1n << 24n) + 1n]).map((v) => {
		const bytes = new Uint8Array(16);
		new DataView(bytes.buffer).setBigUint64(0, BigInt.asUintN(64, v >> 64n));
		new DataView(bytes.buffer).setBigUint64(8, BigInt.asUintN(64, v));
		return Uint8Array.of(...singlesOp, 2, ...bytes);
	}),
];

/** `singles::formats`' singles: ties at each precision, carries through nines, and the shortest digits at each end. */
const singlesFormatted: (number | bigint)[] = [
	...singlesValues,
	0.125, 0.375, 3.5, 9.5, 99.5, 0.95, 9.995, 1.25, 1.35, 2.675, 9.999999e-5, 1e15, 9999999e9, 1e-7, 123456.79, 16777215, 7.006492e-46, 1.17549421e-38, 8388609,
];
const singlesFormatsOp = op("singles::formats");
const singlesParsesOp = op("singles::parses");
const singlesFormats = [
	...singlesFormatted.flatMap((a) => Array.from({ length: 12 }, (_, p) => Uint8Array.of(...singlesFormatsOp, 0, ...single(a), p))),
	...singlesFormatted.flatMap((a) => [1, 3, 4].map((kind) => Uint8Array.of(...singlesFormatsOp, kind, ...single(a), 0))),
	...singlesFormatted.flatMap((a) => [[1, 2], [2, 1], [NaN, 1], [1, NaN], [-0, 0], [0, -0]].map(([b, c]) => Uint8Array.of(...singlesFormatsOp, 2, ...single(a), ...single(b), ...single(c)))),
	...singlesValues.map((a) => Uint8Array.of(...singlesParsesOp, 2, ...single(a))),
];

/**
 * `singles::parses`' texts: std's grammar, past each end of a single's range, and
 * decimals within a hair of halfway between two singles, which rounding through the
 * nearest double would round twice.
 */
const singlesParsed = [
	"", "+", "-", ".", "e5", "1e", "1.e5", ".5", "+1", "1_000", " 1", "1e40", "-1e40", "1e-50", "-0", "inf", "-infinity", "NaN", "nan1",
	"3.4028235e38", "3.4028236e38", "3.40282357e38", "3.4028235677973366e38", "3.4028235677973367e38", "1.4e-45", "7e-46",
	"7.006492321624085354618647916449580656401e-46", "7.006492321624085354618647916449580656402e-46", "1.1754942e-38", "1.17549435e-38",
	"16777217", "16777217.000000000001", "16777218.99999999999", "16777219", "1.00000005960464477539062499", "1.000000059604644775390625",
	"1.00000005960464477539062501", "1.0000000596046448", "0.1", "1e10", "1234567e-10", "12345678e-10", "9007199254740993",
	`1${"0".repeat(60)}e-60`, `0.${"0".repeat(60)}1e61`, "1e65535", "1e-65536",
].map((text) => Uint8Array.of(...singlesParsesOp, 0, ...new TextEncoder().encode(text)));

/**
 * `mul_adds`' operands at each width: signed zeros, a product that underflows to either
 * zero, infinities and NaN, a product past the type's range whose sum is in it, subnormal
 * sums and ties, products whose error alone survives, which rounding twice loses, and a
 * product of half `c`'s last place, which only its error moves off a tie; then
 * seeds for its sweeps.
 */
const mulAdds = (max: number, tiny: number, eps: number, op: number[], bytes: (x: number) => number[]) => [
	...[
		[0, 1, -0], [-0, 1, -0], [0, -1, 0], [-0, -0, -0], [-0, 0, -0], [1, -1, 1], [-1, 1, 1],
		[tiny, -(2 ** -30), 0], [tiny, 2 ** -30, -0], [-tiny, 2 ** -30, -0], [tiny, 0.5, 0], [tiny, 0.5, -0], [-tiny, 0.5, 0],
		[tiny, 1.5, 0], [tiny, 0.5, tiny], [tiny, -0.5, tiny], [tiny, 2 ** 52, tiny],
		[Infinity, 0, 1], [0, -Infinity, 1], [Infinity, 1, -Infinity], [Infinity, -1, -Infinity], [1, 1, Infinity], [max, 2, -Infinity], [max, -2, Infinity],
		[NaN, 1, 1], [1, NaN, 1], [1, 1, NaN], [Infinity, 1, NaN],
		[max, 2, -max], [max, -2, max], [max, 1 + eps, -max], [max, 1 + eps, 0], [max * 0.5, 2, max * eps * 0.25],
		[1 + 2 ** -27, 1 - 2 ** -27, -1], [1 + eps, 1 + eps, -1], [1 + eps, 1 - eps, -1], [0.1, 10, -1], [3, 1 / 3, -1], [1 + eps, 1 + eps, -(1 + 2 * eps)],
		[1 + 2 ** -12, 1 + 2 ** -12, tiny], [1 + 2 ** -12, 1 + 2 ** -12, -tiny], [1 + eps, 1 + eps / 2, 0], [eps / 2 + eps * eps / 2, 1 - eps / 2, 1], [eps / 2 + eps * eps / 2, 1 - eps / 2, -1],
	].map((xs) => Uint8Array.of(...op, 0, ...xs.flatMap(bytes))),
	...Array.from({ length: 400 }, (_, i) => Uint8Array.of(...op, 1, ...double(BigInt(i + 1) * 0x9e3779b97f4a7c15n & 0xffff_ffff_ffff_ffffn))),
];
const floatsMulAdds = mulAdds(Number.MAX_VALUE, 5e-324, 2 ** -52, op("floats::mul_adds"), double);
const singlesMulAdds = mulAdds(3.4028234663852886e38, 2 ** -149, 2 ** -23, op("singles::mul_adds"), single);

const [floatsAt, singlesAt] = [pick("floats::floats"), pick("singles::singles")];

/** How many ULPs from native Rust's each float of `floats`' and `singles`' kind 7 may be, which Rust leaves to libm. */
const ULPS = 2n;

export function within(input: Uint8Array, native: Uint8Array, translated: Uint8Array): boolean {
	const at = ((input[0] << 8) | input[1]) % count;
	const width = at === floatsAt ? 8 : at === singlesAt ? 4 : 0;
	if (width === 0 || input[2] % 10 !== 7 || native.length !== translated.length) return false;
	const [a, b] = [native, translated].map((v) => new DataView(v.buffer, v.byteOffset, v.length));
	const read = (v: DataView, i: number) => (width === 8 ? v.getBigInt64(i) : BigInt(v.getInt32(i)));
	// Bits as a line through zero, so the distance between two is how many floats apart they are.
	const magnitude = (1n << BigInt(width * 8 - 1)) - 1n;
	const line = (v: bigint) => (v < 0n ? -(v & magnitude) : v);
	for (let i = 0; i < native.length; i += width) {
		const d = line(read(a, i)) - line(read(b, i));
		if (d > ULPS || d < -ULPS) return false;
	}
	return true;
}

/**
 * The 64- and 128-bit cases over operands that random bytes rarely make: every pair of
 * values next to a power of two.
 */
function ints(): Uint8Array[] {
	const [u64, u128, i64, i128] = ["ints::u64_op", "ints::u128_op", "ints::i64_op", "ints::i128_op"].map(op);
	const [neg, cast, convert, fmtInts] = ["ints::neg", "ints::cast", "ints::convert", "fmt::ints"].map(op);
	const cases: Uint8Array[] = [];
	// 2^k - 1, 2^k, and 2^k + 1 for each k where a limb, a safe number, or a sign ends,
	// and values without that structure, whose quotients and remainders are not round.
	const edges = new Set<bigint>([
		1_000_000n,
		0x9e3779b97f4a7c15n,
		0x0123456789abcdefn,
		0xfedcba9876543210n,
		// Safe integers whose halves carry when multiplied, and a negative one.
		0x1d3c5b7a9f8e6dn,
		-0x1b2a3c4d5e6f71n & 0xffffffffffffffffn,
	]);
	for (const k of [0n, 1n, 16n, 21n, 31n, 32n, 33n, 52n, 53n, 54n, 63n, 64n]) {
		for (const delta of [-1n, 0n, 1n]) edges.add(((1n << k) + delta) & 0xffffffffffffffffn);
	}
	// Small negative numbers, -2^32, and where negative safe numbers end, as signed values.
	for (const k of [1n, 32n, 53n]) {
		for (const delta of [-1n, 0n, 1n]) edges.add((-(1n << k) + delta) & 0xffffffffffffffffn);
	}
	const values = [...edges];
	const be = (v: bigint) => {
		const bytes = new Uint8Array(8);
		new DataView(bytes.buffer).setBigUint64(0, v);
		return [...bytes];
	};
	// Shift amounts in and past each limb, and past the width, which panics.
	const shifts = [0, 1, 7, 31, 32, 33, 63, 64, 65, 95, 96, 127, 128, 200];
	let i = 0;
	for (const x of values) {
		for (const y of values) {
			// A 64-bit case's operands are `x` and `y`. A 128-bit case's are `x:y` and `u:v`,
			// whose high halves are equal a third of the time, so the low half decides.
			const u = i % 3 === 0 ? x : values[(i * 7) % values.length];
			const v = values[(i * 13 + 5) % values.length];
			const shift = shifts[i++ % shifts.length];
			for (let kind = 0; kind < 31; kind++) {
				for (const o of [u64, i64]) cases.push(Uint8Array.of(...o, kind, ...be(x), ...be(y), shift));
				for (const o of [u128, i128]) cases.push(Uint8Array.of(...o, kind, ...be(x), ...be(y), ...be(u), ...be(v), shift));
			}
			// `clamp` between two values of the sweep, whose third operand follows the shift's place.
			for (const o of [u64, i64]) cases.push(Uint8Array.of(...o, 26, ...be(x), ...be(y), ...be(u)));
			for (const o of [u128, i128]) cases.push(Uint8Array.of(...o, 26, ...be(x), ...be(y), ...be(u), ...be(v), ...be(y), ...be(x)));
			// Negating an `i64` and an `i128`.
			cases.push(Uint8Array.of(...neg, 3, ...be(x)), Uint8Array.of(...neg, 4, ...be(x), ...be(y)));
			// `x * y`, `checked_mul`, `saturating_mul`, and `wrapping_mul` at 128 bits, extended in
			// each type's sign: a product of two numbers, past 2^53, multiplies their halves.
			const signs = (v: bigint) => (v >> 63n === 1n ? 0xffffffffffffffffn : 0n);
			for (const kind of [2, 13, 24, 25]) {
				cases.push(Uint8Array.of(...u128, kind, ...be(0n), ...be(x), ...be(0n), ...be(y)));
				cases.push(Uint8Array.of(...i128, kind, ...be(signs(x)), ...be(x), ...be(signs(y)), ...be(y)));
			}
			// `x:y / d`, `x:y % d`, `div_ceil`, and `is_multiple_of` at 128 bits by a divisor
			// below 2^32 in magnitude, which divides limbs by a number, extended in each type's
			// sign: -1 overflows `i128::MIN`.
			for (const d of [3n, 1_000_000n, 0xffffffffn, -1n, -0xffffffffn]) {
				for (const kind of [3, 4, 27, 28]) {
					for (const o of [u128, i128]) cases.push(Uint8Array.of(...o, kind, ...be(x), ...be(y), ...be(signs(d & 0xffffffffffffffffn)), ...be(d & 0xffffffffffffffffn)));
				}
			}
		}
		// Conversions, and halves joined back.
		cases.push(Uint8Array.of(...convert, 0, ...be(x), ...be(x)));
		for (const y of [0n, 1n, 1n << 32n, 1n << 63n, 0xffffffffffffffffn]) {
			for (const from of [3, 4, 9, 10]) {
				// A cast to every type, and every format spec.
				for (let to = 0; to < 12; to++) cases.push(Uint8Array.of(...cast, from, to, ...be(x), ...be(y)));
				cases.push(Uint8Array.of(...fmtInts, from, ...be(x), ...be(y)));
			}
		}
	}
	return cases;
}

/**
 * `utf8::decodes` of raw bytes: a byte order mark, which std keeps, and the sequences at
 * each edge of what it takes: surrogates, overlongs, past U+10FFFF, and cut short at the
 * end or by another byte.
 */
const utf8 = [
	[0xef, 0xbb, 0xbf, 0x61],
	[0xed, 0x9f, 0xbf, 0xed, 0xa0, 0x80, 0xed, 0xbf, 0xbf],
	[0xc0, 0x80, 0xc1, 0xbf, 0xc2, 0x80],
	[0xe0, 0x80, 0x80, 0xe0, 0x9f, 0xbf, 0xe0, 0xa0, 0x80],
	[0xf0, 0x80, 0x80, 0x80, 0xf0, 0x8f, 0xbf, 0xbf, 0xf0, 0x90, 0x80, 0x80],
	[0xf4, 0x8f, 0xbf, 0xbf, 0xf4, 0x90, 0x80, 0x80, 0xf5, 0x80, 0x80, 0x80],
	[0xf0, 0x9f, 0x98, 0x80, 0xf0, 0x9f, 0x98, 0x41, 0xe2, 0x82, 0x41, 0xf0, 0x9f, 0x98],
	[0xe2, 0x82],
	[0xc2],
	[0x80, 0xf8, 0xff],
].map((bytes) => Uint8Array.of(...op("utf8::decodes"), 0, ...bytes));

/**
 * `dyns::dyns` too short to run, with a `Tri` of each variant, at the ends of each side and
 * count, and boxing each kind of shape.
 */
const dyns = [[], [1, 2], [3, 0, 7], [255, 1, 255, 0, 1, 2, 3, 4, 5], [0, 3, 2, 9, 255, 6, 128]].map((bytes) =>
	Uint8Array.of(...op("dyns::dyns"), 0, ...bytes),
);

/**
 * `cells` scripts of (op, operand) pairs on every kind, so each cell and value type: reads
 * and sets, a `None` and a `()` held, initializing from within initialization and
 * panicking in it, empty and full, and a cell replaced; clones, `Debug`, and `get_mut`,
 * empty and full; and `get_or_try_init` failing, succeeding, set, and set or initialized
 * again from within, failing after or not.
 */
const cellsOp = op("cells::cells");
const cellScripts = [
	[0, 0, 1, 5, 0, 0, 1, 6, 2, 7, 4, 0, 0, 0, 4, 0, 2, 9, 0, 0],
	[1, 3, 0, 0, 1, 4, 4, 0, 1, 6, 2, 3],
	[3, 4, 3, 5],
	[5, 3],
	[6, 3],
	[7, 1],
	[2, 1, 5, 3, 6, 3, 7, 1],
	[8, 4, 0, 0, 8, 5, 0, 0, 1, 3, 8, 6],
	[9, 0, 9, 1, 0, 0],
	[10, 0, 10, 1, 11, 2, 0, 0],
	[11, 1, 2, 1, 11, 4, 0, 0, 4, 0, 11, 5],
	[12, 0, 12, 6, 0, 0, 12, 1],
	[12, 1, 12, 2],
	[12, 2],
	[12, 3, 4, 0, 12, 7],
	[12, 11],
	[12, 7, 12, 4],
	[],
];
const cells = Array.from({ length: 256 }, (_, kind) => cellScripts.map((script) => Uint8Array.of(...cellsOp, kind, ...script))).flat();

/**
 * `dyns::shared` too short to run, and with `Arc`s of each type, the last count of each at
 * zero and at the top of its range.
 */
const shared = [[], [1], [0, 0], [255, 255, 0, 1, 2, 3, 4, 5], [7, 0xff, 255, 254, 253]].map((bytes) =>
	Uint8Array.of(...op("dyns::shared"), 0, ...bytes),
);

/**
 * `locks` scripts: writes through guards and reads, `get_mut`, and a lock matched.
 */
const locks = [[0, 1, 5, 6, 2, 7, 3, 8, 3, 0, 1], [4, 0, 4, 1], [2, 9, 2, 10, 4, 1], [3, 3, 4, 0]].map(
	(bytes) => Uint8Array.of(...op("locks::locks"), 0, ...bytes),
);

/**
 * `borrows::guards` scripts: each operation on cells empty and not, cloned while borrowed
 * shared and exclusive, and borrowed exclusively while a mapped or cloned guard holds it.
 */
const guards = [
	[0x04, 0x05, 0x06, 0x03, 0x07, 0x02, 0x01],
	[0x10, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0b, 0x0c, 0x0d],
	[0x08, 0xe1, 0xf1],
	[0x08, 0xe6],
	[0xf6],
].map((bytes) => Uint8Array.of(...op("borrows::guards"), 0, ...bytes));

/**
 * `borrows::maps` scripts: each operation through mapped, filtered, and split guards, with
 * parts written whole and split parts dropped in either order, and each borrow again
 * while one holds the cell, which panics.
 */
const maps = [
	[0x00, 0x01, 0x02, 0x03, 0x03, 0x04, 0x05, 0x15, 0x07, 0xf1, 0xf2, 0x0b, 0x06, 0x13, 0x0c],
	[0x02, 0x22, 0x0d, 0xc6],
	[0x0a, 0xd6],
	[0xe6],
	[0x02, 0xf6],
].map((bytes) => Uint8Array.of(...op("borrows::maps"), 0, ...bytes));

/** `locks::mapped` scripts: writes and reads through mapped and filtered guards. */
const mappedLocks = [[0x04, 0x01, 0x02, 0x86, 0x05, 0x0a, 0x03, 0x02, 0x80, 0x01, 0x03]].map((bytes) =>
	Uint8Array.of(...op("locks::mapped"), 0, ...bytes),
);

/**
 * `locks::relock` with each pick: a mutex locked again while a guard holds it, which
 * panics, and a read-write lock read or written again while a guard, mapped or not, holds
 * it, which aborts but for a read under a read.
 */
const relocks = Array.from({ length: 16 }, (_, pick) => Uint8Array.of(...op("locks::relock"), 0, pick));

/**
 * `borrows::tries` scripts: each operation through tried guards, given back beside each
 * error, printed, and each borrow again while a tried guard holds the cell, which panics.
 */
const tries = [
	[0x00, 0x00, 0x10, 0xf0, 0xe0, 0x01, 0x02, 0x05, 0x20, 0x21, 0x06, 0x03],
	[0x01, 0x10, 0x01, 0x12, 0xe2],
	[0xd3],
	[0xe3],
	[0x00, 0xf3],
].map((bytes) => Uint8Array.of(...op("borrows::tries"), 0, ...bytes));

/** `locks::mutex_mapped` scripts: writes and reads through mapped and filtered mutex guards. */
const mutexMaps = [[0x04, 0x01, 0x02, 0x86, 0xf5, 0x0a, 0x03, 0x02, 0xe1, 0x80, 0x07, 0x02]].map((bytes) =>
	Uint8Array.of(...op("locks::mutex_mapped"), 0, ...bytes),
);

/** `locks::printed` scripts: each lock's guards printed, empty and not. */
const printed = [[], [0, 1, 25, 200]].map((bytes) => Uint8Array.of(...op("locks::printed"), 0, ...bytes));

/** `locks::downgraded` scripts: write guards downgraded, each lock's released in either order, and whole byte arrays mapped. */
const downgrades = [[0x00, 0x01, 0x11, 0x02, 0xf2, 0xe2, 0x03, 0x04, 0x13, 0x05]].map((bytes) =>
	Uint8Array.of(...op("locks::downgraded"), 0, ...bytes),
);

/**
 * `borrows::leaks` scripts: guards leaked and undone, whole byte arrays mapped, and each
 * borrow again while a leak holds the cell, which panics.
 */
const leaks = [[0x00, 0x01, 0x02, 0xf1, 0xf2, 0x04, 0x05, 0x03, 0x06], [0xe3], [0x00, 0xf3]].map((bytes) =>
	Uint8Array.of(...op("borrows::leaks"), 0, ...bytes),
);

/** `locks::rwlocks` scripts: `get_mut` and `into_inner` among writes and reads. */
const rwlocks = [[6, 7, 0, 6, 2, 14, 15, 1]].map((bytes) => Uint8Array.of(...op("locks::rwlocks"), 0, ...bytes));

/**
 * `statics::onces` scripts: a `Once` forced, called, and forced again, which drops the
 * function it never runs, each static's, each `call_once` and `call_once_force` run again
 * from within itself, and crossbeam-epoch's `OnceLock` of each kind.
 */
const onces = [[1, 0, 1, 7], [0, 6, 1, 2, 3, 4, 5, 11], [2, 14, 8], [9], [3, 15], [4, 17, 23, 29, 4]].map((bytes) => Uint8Array.of(...op("statics::onces"), 0, ...bytes));

/**
 * `bounds` with each kind of bound at each end, over every pair of values around and past
 * four items, so ranges are empty, backwards, equal, and out of bounds, and of no items.
 */
const boundsOp = op("bounds::bounds");
const bounds = Array.from({ length: 6 * 9 * 36 }, (_, n) => {
	const [kind, ends, at] = [n % 6, Math.floor(n / 6) % 9, Math.floor(n / 54)];
	const [a, b] = [at % 6, Math.floor(at / 6)];
	return [Uint8Array.of(...boundsOp, kind, ends % 3, Math.floor(ends / 3), a, b, 1, 3, 2, 5), Uint8Array.of(...boundsOp, kind, ends % 3, Math.floor(ends / 3), a, b)];
}).flat();

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
 * `strs::edits` of each edit on `aé\u{800}😀`, a char of each UTF-8 width, at every byte
 * offset into it and up to two past its end, each made a raw offset by its byte's bits.
 */
const editsOp = op("strs::edits");
const edits = Array.from({ length: 14 * 16 * 16 }, (_, i) => Uint8Array.of(...editsOp, 4, 2, 7, 9, 14, i >> 8, (i >> 4) & 15, i & 15));

/**
 * `subslices` of each kind over zero to four items, at every pair of bounds up to one past
 * the end, so parts are empty, at the end, backwards, and out of bounds.
 */
const subslicesOp = op("slices::subslices");
const subslices = [0, 1, 2, 3, 4].flatMap((n) =>
	Array.from({ length: 8 * (n + 2) ** 2 }, (_, i) => {
		const [kind, a, b] = [i % 8, Math.floor(i / 8) % (n + 2), Math.floor(i / 8 / (n + 2))];
		return Uint8Array.of(...subslicesOp, kind, a, b, ...[0x31, 0xc2, 0x07, 0xfe].slice(0, n));
	}),
);

/**
 * `nested::consumers`, whose first byte is also `nth`'s index: of no items, one `Some` or
 * `None`, `None` before `Some`, `nth` past the end, only `None`s, keys all tied, and
 * `find` stopping early.
 */
const consumers = [[], [6], [5], [1, 2], [7, 2], [1, 3, 5], [2, 6, 10, 14], [4, 3, 0x90, 0x92, 5]].map((bytes) => Uint8Array.of(...op("nested::nested"), 3, ...bytes));

/**
 * `nested::closures`, whose bytes `pick` each `None` (0 mod 3), `Some(None)` (1), or
 * `Some(Some(_))` (2): of no bytes, only `None`s, only `Some(None)`s, `Some(None)` before
 * a `None` and after it, a seed of each, and a `successors` whose size hint reserves past
 * `isize::MAX`.
 */
const closures = [[], [3], [0, 3], [1], [4, 7, 1], [1, 2, 0, 4, 5], [4, 0, 1, 3, 5], [2, 1, 4, 3, 8], [5, 4], [0, 1, 2], [0xf0], [0xf1]].map((bytes) => Uint8Array.of(...op("nested::nested"), 5, ...bytes));

/**
 * `nested::elements`, whose first byte is also the index and whose odd bytes are each a
 * `None` element: of no elements, one of each, only `None`s, and an index in range and
 * past the end of each mix.
 */
const elements = [[], [1], [2], [1, 3], [3, 1, 5], [0, 1, 2], [2, 6, 1, 8], [5, 4, 3, 0x10, 7, 0x40], [7, 0x22, 9, 0x22, 1, 0x80, 3, 6]].map((bytes) => Uint8Array.of(...op("nested::nested"), 6, ...bytes));

/**
 * `ranges` of each kind, and each kind of `Bound` at the end, over zero to four items, at
 * every pair of bounds up to one past the end and at `usize::MAX`.
 */
const rangesOp = op("slices::ranges");
const ranges = [0, 1, 2, 3, 4].flatMap((n) => {
	const ends = [...Array(n + 2).keys(), 0xff];
	return Array.from({ length: 24 * ends.length ** 2 }, (_, i) => {
		const [kind, a, b] = [i % 24, ends[Math.floor(i / 24) % ends.length], ends[Math.floor(i / 24 / ends.length)]];
		return Uint8Array.of(...rangesOp, kind, a, b, ...[0x31, 0xc2, 0x07, 0xfe].slice(0, n));
	});
});

/**
 * `iters::reducers` of no items, one, one repeated, which `next_if_eq` takes, and keys
 * that all tie.
 */
const reducers = [[], [9], [9, 9], [8, 4, 12, 0x40, 0x80]].map((bytes) => Uint8Array.of(...op("iters::reducers"), 0, ...bytes));

/**
 * `iters::peeked_others` of no items, one, a run that matches the first throughout, and
 * items that each match one comparison and miss the next.
 */
const peekedOthers = [[], [3], [4, 4, 4, 4], [0, 1, 2, 3, 4, 5, 6], [1, 0, 0, 2, 5, 9]].map((bytes) =>
	Uint8Array.of(...op("iters::peeked_others"), 0, ...bytes),
);

/**
 * `edits` of each kind over zero to four items, at every three bounds up to one past the
 * end, so parts are empty, at the end, backwards, and out of bounds.
 */
const editsSlicesOp = op("slices::edits");
const sliceEdits = [0, 1, 2, 3, 4].flatMap((n) =>
	Array.from({ length: 8 * (n + 2) ** 3 }, (_, i) => {
		const [kind, a, b, c] = [i % 8, Math.floor(i / 8) % (n + 2), Math.floor(i / 8 / (n + 2)) % (n + 2), Math.floor(i / 8 / (n + 2) ** 2)];
		return Uint8Array.of(...editsSlicesOp, kind, a, b, c, ...[0x31, 0xc2, 0x07, 0xfe].slice(0, n));
	}),
);

/**
 * `strs::drains` of each range and way to end the drain on `aé\u{800}😀`, at every byte
 * offset into it and up to two past its end, reading none, two from either end, or three
 * from the back.
 */
const drainsOp = op("strs::drains");
const drains = Array.from({ length: 32 * 13 * 13 }, (_, i) => {
	const [form, a, b] = [i % 32, Math.floor(i / 32) % 13, Math.floor(i / 416)];
	return Uint8Array.of(...drainsOp, 4, 2, 7, 9, 14, form, a, b, [0, 0x0a, 0x1f][(form + a + b) % 3]);
});

/** `strs::iters` of each kind, on no text, text of each UTF-8 width, and parts between `/`s, empty ones too. */
const iters = Array.from({ length: 12 }, (_, kind) => [[], [2, 7, 9, 14], [0, 2, 0, 0, 7, 9], [14, 0]].map((data) => Uint8Array.of(...op("strs::iters"), kind, ...data))).flat();

/** `strs::hinted` of each adaptor and each pair of range lengths, each way, on `aé` alone. */
const hintedOp = op("strs::hinted");
const hinted = Array.from({ length: 10 * 7 * 21 }, (_, i) => Uint8Array.of(...hintedOp, i % 10, Math.floor(i / 10) % 7, Math.floor(i / 70)));

/**
 * `hints::hints` of each iterator, each pair of lengths, and each of its arms, read three
 * times each way, with as many bytes as the first length's index.
 */
const hintsOp = op("hints::hints");
const hints = Array.from({ length: 24 * 8 * 8 * 8 * 2 }, (_, i) => {
	const [kind, n, m, arm, back] = [i % 24, Math.floor(i / 24) % 8, Math.floor(i / 192) % 8, Math.floor(i / 1536) % 8, Math.floor(i / 12288)];
	const steps = (arm << 5) | ((back ? 0b101 : 0b010) << 2) | 3;
	return Uint8Array.of(...hintsOp, kind, n, m, steps, ...Array.from({ length: n }, (_, b) => b * 37));
});

/** `hints::reserves` of each collection and element type, each shape, length, and limit on what it reads. */
const reservesOp = op("hints::reserves");
const reserves = Array.from({ length: 13 * 6 * 5 * 4 * 5 }, (_, i) =>
	Uint8Array.of(...reservesOp, i % 78, Math.floor(i / 78) % 5, Math.floor(i / 390) % 4, Math.floor(i / 1560)),
);

/**
 * `collections::set_ops` of one key against 30, which std's B-tree looks up in the larger
 * rather than merging, of each key type: a key below the rest, at either end, between, and
 * above. Then `Nibble`s equal to another set's but for their high bits, of hash sets as
 * large as each other and one larger, where std's picks which set's to yield by size.
 */
const setOpsOp = op("collections::set_ops");
const setOps = [
	...[15, 16, 17, 18, 19].flatMap((kind) => [0x20, 0x21, 0x25, 0x3e, 0x3f].map((single) => Uint8Array.of(...setOpsOp, kind, single, ...Array.from({ length: 30 }, (_, i) => 0x41 + i)))),
	...[
		[0x01, 0x02, 0x13, 0x21, 0x32, 0x44],
		[0x01, 0x02, 0x13, 0x21, 0x32, 0x44, 0x55],
		[0x01, 0x02, 0x13, 0x55, 0x21, 0x32, 0x44],
	].map((data) => Uint8Array.of(...setOpsOp, 3, ...data)),
];

/** `zeros::zeros` of each case, on up to three bytes. */
const zerosOp = op("zeros::zeros");
const zeros = Array.from({ length: 7 * 4 }, (_, i) => Uint8Array.of(...zerosOp, i % 7, ...Array.from({ length: Math.floor(i / 7) }, (_, b) => b)));

/** `zeros::writes` of each case, on up to three bytes: the last past the slice it swaps in. */
const writesOp = op("zeros::writes");
const writes = Array.from({ length: 5 * 4 }, (_, i) => Uint8Array.of(...writesOp, i % 5, ...Array.from({ length: Math.floor(i / 5) }, (_, b) => b)));

/**
 * `collections::unlike_eqs` of every pair of positions where `eq` and `ne` disagree and each
 * length the right side may lose, over no items, one, two, a wrapped deque's four, and nine,
 * whose positions are all within it.
 */
const unlikeEqsOp = op("collections::unlike_eqs");
const unlikeEqs = [[], [5], [1, 2], [0, 7, 2, 9], [3, 3, 3, 3, 3, 3, 3, 3, 3]].flatMap((bytes) =>
	Array.from({ length: 128 }, (_, kind) => Uint8Array.of(...unlikeEqsOp, kind, ...bytes)),
);

/** `collections::own_nes` of the same positions, lengths, and items as `unlike_eqs`. */
const ownNesOp = op("collections::own_nes");
const ownNes = unlikeEqs.map((input) => Uint8Array.of(...ownNesOp, ...input.subarray(2)));

/** `collections::unlike_compares` of the same positions, lengths, and items as `unlike_eqs`. */
const unlikeComparesOp = op("collections::unlike_compares");
const unlikeCompares = unlikeEqs.map((input) => Uint8Array.of(...unlikeComparesOp, ...input.subarray(2)));

/**
 * `iters::range_froms` of each case, each way its zips read the range, from each distance
 * below its type's last value and from 250, on up to nine bytes: each length its zips
 * panic at.
 */
const rangeFromsOp = op("iters::range_froms");
const rangeFroms = Array.from({ length: 16 * 4 * 4 * 10 }, (_, i) => {
	const kind = (i % 16) | ((Math.floor(i / 16) % 4) << 6);
	const a = [0, 1, 3, 250][Math.floor(i / 64) % 4];
	const data = Array.from({ length: Math.floor(i / 256) }, (_, j) => (j === 0 ? a : (j * 37) & 0xff));
	return Uint8Array.of(...rangeFromsOp, kind, ...data);
});

/** `iters::range_methods` of each type's table, at each pair of its first eight values. */
const rangeMethodsOp = op("iters::range_methods");
const rangeMethods = Array.from({ length: 19 * 64 }, (_, i) => Uint8Array.of(...rangeMethodsOp, i % 19, Math.floor(i / 19) % 8, Math.floor(i / 152)));

/**
 * `iters::compares` of each comparison, shape, and pair, over no items, one, a run up to
 * 255, which `start..` panics past and a sum overflows at, NaNs, and repeats.
 */
const comparesOp = op("iters::compares");
const compares = [[], [5], [1, 2, 3], [250, 251, 252, 253, 254, 255], [0xff, 3, 0xff], [0, 1, 2, 3, 4, 5], [7, 7, 7, 7, 7, 7, 7, 7, 7]].flatMap((data) =>
	Array.from({ length: 256 }, (_, kind) => Uint8Array.of(...comparesOp, kind, ...data)),
);

/**
 * `collections::ref_orders` of no items, one, repeats, and runs whose remainders by 2, 3,
 * 4, and 5 agree and disagree.
 */
const refOrdersOp = op("collections::ref_orders");
const refOrders = [[], [5], [3, 3, 3], [1, 2], [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11], [7, 2, 9, 4, 4, 11, 15, 6], [12, 6, 0, 30, 60]].map((data) => Uint8Array.of(...refOrdersOp, 0, ...data));

/**
 * `separated::separated` of every form and order of reads, over data whose bytes its bounds
 * make separators of alone, in runs, and at either end, and over one element and none.
 */
const separatedOp = op("separated::separated");
const separated = [[], [0], [0x90], [0, 0, 0], [5, 0x90, 0, 0x7f, 0, 0xff, 3], [0x80, 1, 2, 0], [0xff, 0x7f, 0x80, 0x81]].flatMap((data) =>
	Array.from({ length: 256 }, (_, kind) => Uint8Array.of(...separatedOp, kind, ...data)),
);

/**
 * `wraps::*` of every kind at every width, over operands at the edges of its range: 0, 1,
 * either end, the sign bit's neighbors, which make `MIN / -1`, and at 128 bits values past
 * 2^64. Shift amounts are at, past, and many times past the width, which std masks.
 */
function wraps(): Uint8Array[] {
	const widths: [string, number][] = [["u8s", 8], ["u16s", 16], ["u32s", 32], ["u64s", 64], ["u128s", 128], ["usizes", 32], ["i8s", 8], ["i16s", 16], ["i32s", 32], ["i64s", 64], ["i128s", 128], ["isizes", 32]];
	const cases: Uint8Array[] = [];
	for (const [name, bits] of widths) {
		const o = op(`wraps::${name}`);
		const width = BigInt(bits);
		const mask = (1n << width) - 1n;
		const sign = 1n << (width - 1n);
		const edges = [0n, 1n, 3n, mask, mask - 1n, sign, sign - 1n, sign + 1n];
		if (bits === 128) edges.push(1n << 64n, (1n << 64n) + 3n, 0x9e3779b97f4a7c150123456789abcdefn);
		const be = (v: bigint) => Array.from({ length: bits / 8 }, (_, i) => Number((v >> BigInt(bits - 8 - 8 * i)) & 0xffn));
		// The amount's low byte and its high byte, which reads as `high << 24`.
		const shifts = [[0, 0], [1, 0], [bits - 1, 0], [bits, 0], [bits + 1, 0], [2 * bits + 3, 0], [5, 1]];
		let i = 0;
		for (const a of edges) {
			for (const b of edges) {
				const c = edges[(i * 5 + 3) % edges.length];
				const [low, high] = shifts[i++ % shifts.length];
				for (let kind = 0; kind < 24; kind++) cases.push(Uint8Array.of(...o, kind, ...be(a), ...be(b), ...be(c), low, high));
			}
		}
	}
	return cases;
}

/**
 * `saturates::*` of every kind at every width, over operands at the edges of its range and
 * across them: 0, 1, either end, the sign bit's neighbors, which make `MIN / -1` and
 * `MIN % -1`, and at 128 bits values past 2^64.
 */
function saturates(): Uint8Array[] {
	const widths: [string, number][] = [["u8s", 8], ["u16s", 16], ["u32s", 32], ["u64s", 64], ["u128s", 128], ["usizes", 32], ["i8s", 8], ["i16s", 16], ["i32s", 32], ["i64s", 64], ["i128s", 128], ["isizes", 32]];
	const cases: Uint8Array[] = [];
	for (const [name, bits] of widths) {
		const o = op(`saturates::${name}`);
		const width = BigInt(bits);
		const mask = (1n << width) - 1n;
		const sign = 1n << (width - 1n);
		const half = sign >> 1n;
		const edges = [0n, 1n, 3n, mask, mask - 1n, sign, sign - 1n, sign + 1n, half, half + 1n, mask - half];
		if (bits === 128) edges.push(1n << 64n, (1n << 64n) + 3n, 0x9e3779b97f4a7c150123456789abcdefn);
		const be = (v: bigint) => Array.from({ length: bits / 8 }, (_, i) => Number((v >> BigInt(bits - 8 - 8 * i)) & 0xffn));
		let i = 0;
		for (const a of edges) {
			for (const b of edges) {
				const c = edges[(i++ * 5 + 3) % edges.length];
				for (let kind = 0; kind < 24; kind++) cases.push(Uint8Array.of(...o, kind, ...be(a), ...be(b), ...be(c)));
			}
		}
	}
	return cases;
}

/**
 * `methods::*` of every kind at every width, over operands at the edges of its range:
 * both ends and their neighbors, the sign bit's, small values, and where they reach,
 * either side of 2^53 and 2^64 and their negations. Each meets 0, 1, -1, 2, 10, and
 * either end, which make `MIN / -1`, `MIN % -1`, and a zero divisor or base, and
 * exponents and shifts in, at, and past the width.
 */
function methods(): Uint8Array[] {
	const widths: [string, number, boolean][] = [["u8s", 8, false], ["u16s", 16, false], ["u32s", 32, false], ["u64s", 64, false], ["u128s", 128, false], ["usizes", 32, false], ["i8s", 8, true], ["i16s", 16, true], ["i32s", 32, true], ["i64s", 64, true], ["i128s", 128, true], ["isizes", 32, true]];
	// The kinds that read the `u32`, and the others, which read `b`.
	const exponents = new Set([3, 4, 7, 8, 21, 22, 23, 38, 39, 40]);
	const cases: Uint8Array[] = [];
	for (const [name, bits, signed] of widths) {
		const o = op(`methods::${name}`);
		const width = BigInt(bits);
		const min = signed ? -(1n << (width - 1n)) : 0n;
		const max = (1n << (signed ? width - 1n : width)) - 1n;
		const sign = 1n << (width - 1n);
		const values = [min, min + 1n, min + 2n, -2n, -1n, 0n, 1n, 2n, 3n, 10n, 99n, max - 1n, max, sign - 1n, sign, sign + 1n];
		for (const k of [53n, 64n]) {
			if (k < width) values.push(...[-1n, 0n, 1n].flatMap((d) => [(1n << k) + d, -(1n << k) + d]));
		}
		const edges = [...new Set(values.filter((v) => v >= min && v <= max))];
		const others = [0n, 1n, -1n, 2n, 10n, min, max].filter((v) => v >= min && v <= max);
		const shifts = [0, 1, 2, bits - 1, bits, bits + 1, 2 * bits, 3 * bits - 1, 0x80000000, 0xffffffff];
		const be = (v: bigint) => Array.from({ length: bits / 8 }, (_, i) => Number((v >> BigInt(bits - 8 - 8 * i)) & 0xffn));
		const u32 = (n: number) => [n >>> 24, (n >>> 16) & 0xff, (n >>> 8) & 0xff, n & 0xff];
		let i = 0;
		for (let kind = 0; kind < 42; kind++) {
			for (const a of edges) {
				if (exponents.has(kind)) {
					for (const n of shifts) cases.push(Uint8Array.of(...o, kind, ...be(a), ...be(others[i++ % others.length]), ...u32(n)));
				} else {
					for (const b of others) cases.push(Uint8Array.of(...o, kind, ...be(a), ...be(b), ...u32(shifts[i++ % shifts.length])));
				}
			}
		}
	}
	return cases;
}

/**
 * `ascii` cases of each kind: bytes and slices on each prefix of text padded by ASCII
 * whitespace, vertical tab among it, holding a byte of each escape and class, so escapes
 * are read from both ends; slices on text of each char `ascii.rs` tries; chars on the
 * scalars either side of the surrogates and past the last.
 */
const asciiText = [0x20, 0x09, 0x0b, 0x48, 0x7a, 0x22, 0x5c, 0x27, 0x7f, 0x80, 0xff, 0x0a, 0x0c, 0x0d, 0x20];
const asciiCases = [
	...["ascii::bytes", "ascii::slices"].flatMap((name) =>
		Array.from({ length: 4 * (asciiText.length + 1) }, (_, i) => Uint8Array.of(...op(name), i % 4, ...asciiText.slice(0, Math.floor(i / 4)))),
	),
	// Text of every char `ascii.rs` tries, each once.
	...[0, 1, 2, 3].map((kind) => Uint8Array.of(...op("ascii::slices"), kind, ...Array.from({ length: 41 }, (_, i) => i))),
	Uint8Array.of(...op("ascii::chars"), 0, ...[0xd7ff, 0xd800, 0xdfff, 0xe000, 0x1f600, 0x10ffff, 0x110000].flatMap((n) => [n >>> 24, (n >>> 16) & 0xff, (n >>> 8) & 0xff, n & 0xff])),
	// `parts` of each kind at every pair of byte offsets up to two past the end of its
	// 14 bytes of text, and of no text.
	...Array.from({ length: 8 * 17 * 17 }, (_, i) => Uint8Array.of(...op("ascii::parts"), i % 8, Math.floor(i / 8) % 17, Math.floor(i / 8 / 17))),
	...Array.from({ length: 8 * 3 * 3 }, (_, i) => Uint8Array.of(...op("ascii::parts"), 0x10 | (i % 8), Math.floor(i / 8) % 3, Math.floor(i / 24))),
];

/**
 * `strs::gets` of each kind at every pair of byte offsets up to two past the end of its 13
 * bytes of text, and of no text, and at `usize::MAX`.
 */
const getsOp = op("strs::gets");
const getsAt = (n: number) => [...Array.from({ length: n + 3 }, (_, i) => i), 0xff];
const gets = [0, 0x10].flatMap((text) => {
	const at = getsAt(text === 0 ? 13 : 0);
	return Array.from({ length: 16 * at.length * at.length }, (_, i) =>
		Uint8Array.of(...getsOp, text | (i % 16), at[Math.floor(i / 16) % at.length], at[Math.floor(i / 16 / at.length)]),
	);
});

/**
 * `capacity` cases of each element type: each step of a vector, a deque, and a heap, by a
 * few, past what it may hold, and within eight of `usize::MAX`, once it grew past its
 * least capacity, made by `new` and by `with_capacity`; each step after a `reserve` or
 * `reserve_exact` that needed no room, whose floor it keeps; a run of pushes that doubles
 * it from each; and each way `collects` makes one, of each length it tries.
 */
const capacityCases = [
	...[["capacity::vecs", 25], ["capacity::deques", 21], ["capacity::heaps", 13]].flatMap(([name, steps]) =>
		Array.from({ length: 5 * 2 * (steps as number) * 4 }, (_, i) => {
			const [kind, first, step, arg] = [i % 5, (Math.floor(i / 5) % 2) * 0x11, Math.floor(i / 10) % (steps as number), [3, 0xc1, 0xe3, 0x16][Math.floor(i / (10 * (steps as number)))]];
			return Uint8Array.of(...op(name as string), kind, first, ...[0, 1, 0, 2, 0, 3, 0, 4, 0, 5].map((b, j) => (j % 2 === 0 ? b : b * 41)), step, arg, step, arg ^ 1);
		}),
	),
	...[["capacity::vecs", 25], ["capacity::deques", 21], ["capacity::heaps", 13]].flatMap(([name, steps]) =>
		Array.from({ length: 5 * 2 * (steps as number) }, (_, i) => Uint8Array.of(...op(name as string), i % 5, 0, 0, 1, 3 + (Math.floor(i / 5) % 2), 3, Math.floor(i / 10), 0)),
	),
	...["capacity::vecs", "capacity::deques", "capacity::heaps"].flatMap((name) =>
		Array.from({ length: 5 * 2 }, (_, i) => Uint8Array.of(...op(name), i % 5, Math.floor(i / 5) * 0x11, ...Array.from({ length: 40 }, (_, p) => [0, p]).flat())),
	),
	...Array.from({ length: 8 * 6 }, (_, i) => Uint8Array.of(...op("capacity::collects"), i % 8, [0, 1, 2, 5, 9, 23][Math.floor(i / 8)], 1, 2, 3, 4, 5, 6, 7, 8, 9, 10)),
];

export const cases: Uint8Array[] = [...loops, ...durations, ...ints(), ...utf8, ...floats, ...formats, ...parsed, ...singlesCases, ...singlesFormats, ...singlesParsed, ...dyns, ...cells, ...shared, ...locks, ...guards, ...maps, ...mappedLocks, ...relocks, ...tries, ...mutexMaps, ...printed, ...downgrades, ...leaks, ...rwlocks, ...onces, ...bounds, ...unsafeCells, ...edits, ...subslices, ...consumers, ...closures, ...elements, ...ranges, ...reducers, ...sliceEdits, ...drains, ...iters, ...hinted, ...peekedOthers, ...hints, ...reserves, ...setOps, ...zeros, ...writes, ...floatsMulAdds, ...singlesMulAdds, ...unlikeEqs, ...ownNes, ...rangeFroms, ...compares, ...unlikeCompares, ...rangeMethods, ...wraps(), ...refOrders, ...saturates(), ...methods(), ...separated, ...asciiCases, ...gets, ...capacityCases];
