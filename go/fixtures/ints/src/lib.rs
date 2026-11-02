//! Every integer width Go has one for, its arithmetic one operation per case so a panic
//! hides nothing: overflow, division by zero, and shifts past the width panic as native
//! Rust's do, with its messages.

/// What a case runs: an operation, picked by the second byte, on the rest.
type Case = fn(u8, &[u8], &mut Vec<u8>);

/// Run the case the first byte picks: a width, then an operation on the rest.
pub fn run(input: &[u8]) -> Vec<u8> {
	let mut out = Vec::new();
	let Some((&pick, data)) = input.split_first() else {
		return out;
	};
	let cases: [Case; 15] = [
		u8_op, u16_op, u32_op, u64_op, usize_op, i8_op, i16_op, i32_op, i64_op, isize_op, i8_neg, i16_neg, i32_neg,
		i64_neg, isize_neg,
	];
	let (op, data) = data.split_first().map_or((0, &[][..]), |(&op, data)| (op, data));
	cases[usize::from(pick) % cases.len()](op, data, &mut out);
	out
}

/// Take an integer's big-endian bytes from the front of `data`, zero-padded.
macro_rules! take {
	($t:ty, $data:expr) => {{
		let data: &[u8] = $data;
		let mut buf = [0u8; core::mem::size_of::<$t>()];
		let n = data.len().min(buf.len());
		buf[..n].copy_from_slice(&data[..n]);
		(<$t>::from_be_bytes(buf), &data[n..])
	}};
}

macro_rules! ops {
	($name:ident, $t:ty, $wide:ty) => {
		/// Apply operation `op` to operands taken from `data`.
		fn $name(op: u8, data: &[u8], out: &mut Vec<u8>) {
			let (a, data) = take!($t, data);
			let (b, data) = take!($t, data);
			let (c, _) = take!($t, data);
			let shift = data.first().copied().unwrap_or(0) as u32 % (2 * <$t>::BITS);
			let put = |out: &mut Vec<u8>, v: $t| out.extend_from_slice(&(v as $wide).to_be_bytes());
			let put_opt = |out: &mut Vec<u8>, v: Option<$t>| match v {
				Some(v) => {
					out.push(1);
					put(out, v);
				}
				None => out.push(0),
			};
			match op % 32 {
				0 => put(out, a + b),
				1 => put(out, a - b),
				2 => put(out, a * b),
				3 => put(out, a / b),
				4 => put(out, a % b),
				5 => put(out, a & b),
				6 => put(out, a | b),
				7 => put(out, a ^ b),
				8 => put(out, a << shift),
				9 => put(out, a >> shift),
				10 => put(out, !a),
				11 => put_opt(out, a.checked_add(b)),
				12 => put_opt(out, a.checked_sub(b)),
				13 => put_opt(out, a.checked_mul(b)),
				14 => put(out, a.saturating_add(b)),
				15 => put(out, a.saturating_sub(b)),
				16 => put(out, a.wrapping_add(b)),
				17 => put(out, a.wrapping_sub(b)),
				18 => out.extend_from_slice(&a.leading_ones().to_be_bytes()),
				19 => out.extend_from_slice(&a.leading_zeros().to_be_bytes()),
				20 => put(out, a.max(b)),
				21 => put(out, a.min(b)),
				22 => {
					out.push((a == b) as u8);
					out.push((a < b) as u8);
					out.push((a >= b) as u8);
				}
				23 => {
					// Products nothing reads still overflow, of a sum that does too.
					let x = a + b;
					let _ = x * b;
					let _ = (x - b) * a;
					out.push(1);
				}
				24 => put(out, a.saturating_mul(b)),
				25 => put(out, a.wrapping_mul(b)),
				// Panics unless `b <= c`.
				26 => put(out, a.clamp(b, c)),
				27 => put(out, a.rotate_left(shift)),
				28 => put(out, a.swap_bytes()),
				29 => {
					let (le, be) = (a.to_le_bytes(), a.to_be_bytes());
					put(out, <$t>::from_le_bytes(le));
					put(out, <$t>::from_le_bytes(be));
					out.extend_from_slice(&le);
				}
				30 => {
					for &v in &[0, 1, <$t>::MAX, <$t>::MIN, a, a << shift % <$t>::BITS] {
						out.extend_from_slice(&v.trailing_zeros().to_be_bytes());
						out.extend_from_slice(&v.count_ones().to_be_bytes());
					}
				}
				_ => {
					// Compound assignment, with constants folded beside variables.
					let mut x = a;
					x += b;
					x -= 1;
					x *= 2;
					x >>= 1;
					x <<= 1;
					x |= 1 << 3;
					x ^= b;
					x &= !(1 << 2);
					// By a constant, whose overflow check a signed type folds.
					x %= 7;
					x /= 3;
					put(out, x);
				}
			}
		}
	};
}

ops!(u8_op, u8, u64);
ops!(u16_op, u16, u64);
ops!(u32_op, u32, u64);
ops!(u64_op, u64, u64);
ops!(usize_op, usize, u64);
ops!(i8_op, i8, i64);
ops!(i16_op, i16, i64);
ops!(i32_op, i32, i64);
ops!(i64_op, i64, i64);
ops!(isize_op, isize, i64);

macro_rules! neg {
	($name:ident, $t:ty) => {
		/// Negate, which overflows at the minimum.
		fn $name(_: u8, data: &[u8], out: &mut Vec<u8>) {
			let (a, _) = take!($t, data);
			out.extend_from_slice(&((-a) as i64).to_be_bytes());
		}
	};
}

neg!(i8_neg, i8);
neg!(i16_neg, i16);
neg!(i32_neg, i32);
neg!(i64_neg, i64);
neg!(isize_neg, isize);
