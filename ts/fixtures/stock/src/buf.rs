//! `bytes`' `BytesMut`, `Buf`, and `BufMut`: each case runs ops its input picks, so the
//! differential test reaches their panics and what `split_to` and `freeze` share.

use bytes::{Buf, BufMut, Bytes, BytesMut};

/// A `BytesMut` split, frozen, grown, and written, ending with every piece's bytes: a
/// write to one that reached another would show.
pub fn bytes_mut(data: &[u8], out: &mut Vec<u8>) {
	let mut ops = Ops(data);
	let mut buf = BytesMut::with_capacity(ops.byte() as usize % 24);
	let mut pieces: Vec<BytesMut> = Vec::new();
	let mut frozen: Vec<Bytes> = Vec::new();
	while let Some(op) = ops.next() {
		let n = ops.byte() as usize;
		match op % 18 {
			0 => buf.extend_from_slice(ops.take(n % 12)),
			1 => buf.put_u8(n as u8),
			2 => buf.put_u16(n as u16 * 257),
			3 => pieces.push(buf.split_to(n % (buf.len() + 2))),
			4 => {
				if let Some(piece) = pieces.pop() {
					frozen.push(piece.freeze());
				}
			}
			5 => buf.truncate(n % (buf.len() + 2)),
			6 => buf.resize(n % 40, n as u8),
			// Past what a vector holds, by overflowing `usize` or not, counting what it read
			// past: each panics as `Vec` does. Storage that was shared throws there instead,
			// so these reserve into a vector of their own.
			7 if n % 2 == 0 => {
				let mut fresh = BytesMut::from(ops.take(n % 6));
				let read = n % (fresh.len() + 1);
				fresh.advance(read);
				let len = fresh.len();
				fresh.reserve(match n % 3 {
					0 => usize::MAX - n,
					1 => isize::MAX as usize - len + 1,
					_ => isize::MAX as usize + 1 - read - len,
				});
			}
			7 => buf.reserve(n),
			8 => {
				if let Some(piece) = pieces.pop() {
					buf.unsplit(piece);
				}
			}
			9 => buf.advance(n % (buf.len() + 2)),
			10 => out.extend_from_slice(&read(&mut buf, n).to_be_bytes()),
			11 => buf.clear(),
			12 => pieces.push(buf.split()),
			13 => frozen.push(buf.copy_to_bytes(n % (buf.len() + 2))),
			14 => buf.put_bytes(n as u8, n % 9),
			15 => {
				// Through `DerefMut`, into what this handle alone holds.
				if !buf.is_empty() {
					let at = n % buf.len();
					buf[at] ^= 0xff;
				}
				if let Some(piece) = pieces.last_mut() {
					piece.put_slice(b"xy");
				}
			}
			16 => buf.put(Bytes::copy_from_slice(ops.take(n % 5))),
			_ => {
				let mut source = BytesMut::from(ops.take(n % 6));
				// Its own `Buf`, read into a `BufMut`, then the rest whole.
				let first = source.get_u8_checked();
				buf.put_u8(first);
				buf.put(source);
			}
		}
	}
	let tail = BytesMut::from(&b"tail"[..]);
	out.push((buf == tail) as u8);
	out.push((buf.clone() == buf[..]) as u8);
	out.extend_from_slice(format!("{buf:?} {}\n", buf.len()).as_bytes());
	for piece in &pieces {
		out.extend_from_slice(format!("{piece:?}\n").as_bytes());
	}
	for bytes in &frozen {
		out.extend_from_slice(format!("{bytes:?}\n").as_bytes());
	}
	let vec: Vec<u8> = buf.into();
	out.extend_from_slice(&vec);
}

trait Checked {
	/// The next byte, or 0 if none remains.
	fn get_u8_checked(&mut self) -> u8;
}

impl<B: Buf> Checked for B {
	fn get_u8_checked(&mut self) -> u8 {
		match self.has_remaining() {
			true => self.get_u8(),
			false => 0,
		}
	}
}

/// A scalar read off the front of a buffer by `n`'s pick, as a `u128`'s bytes.
fn read<B: Buf>(buf: &mut B, n: usize) -> u128 {
	match n % 15 {
		0 => buf.get_u8().into(),
		1 => buf.get_i8() as u128,
		2 => buf.get_u16().into(),
		3 => buf.get_i16_le() as u128,
		4 => buf.get_u32_le().into(),
		5 => buf.get_i32() as u128,
		6 => buf.get_u64().into(),
		7 => buf.get_i64_le() as u128,
		8 => buf.get_u128_le(),
		9 => buf.get_i128() as u128,
		10 => buf.get_uint(n % 10).into(),
		11 => buf.get_int_le(n % 10) as u128,
		12 => buf.get_f64().to_bits().into(),
		13 => buf.get_f32_le().to_bits().into(),
		_ => {
			let mut dst = [0; 3];
			buf.copy_to_slice(&mut dst[..n % 4]);
			u32::from_be_bytes([0, dst[0], dst[1], dst[2]]).into()
		}
	}
}

/// `reserve` at the edges of `rt.BytesMut`'s limit for storage that was shared, 512 MiB
/// counting what it read past: a vector of its own grows as `Vec` does on either side of
/// it, and shared storage grows up to it. Only the inputs `caller.ts` names reach these,
/// as each allocates that much.
pub fn reserves(data: &[u8], out: &mut Vec<u8>) {
	const LIMIT: usize = 1 << 29;
	let &[b'e', b'd', b'g', b'e', kind, read] = data else {
		return;
	};
	let mut buf = BytesMut::from(&b"edges"[..]);
	let read = usize::from(read) % 5;
	buf.advance(read);
	let head = (kind % 2 == 1).then(|| buf.split_to(0));
	let past = usize::from(kind / 2 % 2);
	buf.reserve(LIMIT + past - read - buf.len());
	buf.put_slice(b"!");
	out.extend_from_slice(&buf);
	out.push(head.is_some() as u8);
}

/// A buffer of the crate's own, in pieces: `Buf`'s provided methods read across them.
struct Pieces {
	/// None empty, so `chunk` is empty only once nothing remains, as `Buf` requires.
	parts: Vec<Vec<u8>>,
	/// How far into the first part reads are.
	at: usize,
}

impl Pieces {
	/// `input` in two parts, cut at `cut`.
	fn new(input: &[u8], cut: usize) -> Self {
		let (a, b) = input.split_at(cut);
		let mut parts = vec![a.to_vec(), b.to_vec()];
		parts.retain(|part| !part.is_empty());
		Self { parts, at: 0 }
	}
}

impl Buf for Pieces {
	fn remaining(&self) -> usize {
		self.parts.iter().map(Vec::len).sum::<usize>() - self.at
	}

	fn chunk(&self) -> &[u8] {
		match self.parts.first() {
			Some(part) => &part[self.at..],
			None => &[],
		}
	}

	fn advance(&mut self, mut cnt: usize) {
		while cnt > 0 {
			let left = self.parts[0].len() - self.at;
			if cnt < left {
				self.at += cnt;
				return;
			}
			cnt -= left;
			self.parts.remove(0);
			self.at = 0;
		}
	}

	// Its own, as moq's buffers have: the provided one reads through a `Take<&mut Self>`.
	fn copy_to_bytes(&mut self, len: usize) -> Bytes {
		let mut out = BytesMut::with_capacity(len);
		while out.len() < len {
			let take = self.chunk().len().min(len - out.len());
			out.extend_from_slice(&self.chunk()[..take]);
			self.advance(take);
		}
		out.freeze()
	}
}

/// A buffer in pieces with only `Buf`'s required methods, so `copy_to_bytes` is the
/// provided one, which reads it through a `Take<&mut Self>`.
struct Provided(Pieces);

impl Buf for Provided {
	fn remaining(&self) -> usize {
		self.0.remaining()
	}

	fn chunk(&self) -> &[u8] {
		self.0.chunk()
	}

	fn advance(&mut self, cnt: usize) {
		self.0.advance(cnt);
	}
}

/// The same reads off a `Bytes`, a `BytesMut`, a `&[u8]`, buffers in pieces, and each
/// through the impl forwarding through a `&mut`: each scalar, and what remains.
pub fn reads(data: &[u8], out: &mut Vec<u8>) {
	let [kind, cut, ops @ ..] = data else {
		return;
	};
	let (ops, input) = ops.split_at(ops.len() / 2);
	let cut = *cut as usize % (input.len() + 1);
	let rest = match kind % 8 {
		0 => read_all(&mut Bytes::copy_from_slice(input), ops, read, out),
		1 => read_all(&mut BytesMut::from(input), ops, read, out),
		2 => read_all(&mut Pieces::new(input, cut), ops, read_pieces, out),
		3 => read_all(&mut &input[..], ops, read, out),
		4 => read_all(&mut Provided(Pieces::new(input, cut)), ops, read_pieces, out),
		5 => read_all(&mut &mut Bytes::copy_from_slice(input), ops, read, out),
		6 => read_all(&mut &mut &input[..], ops, read, out),
		_ => read_all(&mut &mut Pieces::new(input, cut), ops, read_pieces, out),
	};
	out.extend_from_slice(&rest);
	// Put whole, chunk by chunk.
	let mut vec = Vec::new();
	vec.put(Pieces::new(input, cut));
	out.extend_from_slice(&vec);
	let slice: &[u8] = input;
	out.push((slice.remaining() == input.len()) as u8);
	out.push((slice.chunk() == input) as u8);
	out.push(slice.has_remaining() as u8);
}

/// The reads of `Buf`'s provided methods that translate for a buffer of the crate's own:
/// those whose MIR reads no bytes as another type.
fn read_pieces<B: Buf>(buf: &mut B, n: usize) -> u128 {
	match n % 2 {
		0 => buf.get_u8().into(),
		_ => {
			let mut dst = [0; 3];
			buf.copy_to_slice(&mut dst[..n % 4]);
			u32::from_be_bytes([0, dst[0], dst[1], dst[2]]).into()
		}
	}
}

/// `read`s by each of `ops`, then the rest, as `copy_to_bytes` takes it.
fn read_all<B: Buf>(buf: &mut B, ops: &[u8], read: fn(&mut B, usize) -> u128, out: &mut Vec<u8>) -> Vec<u8> {
	for &op in ops {
		if op == 0xff {
			buf.advance(1);
			continue;
		}
		out.extend_from_slice(&read(buf, op as usize).to_be_bytes());
		out.push(buf.has_remaining() as u8);
	}
	let n = buf.remaining();
	buf.copy_to_bytes(n).to_vec()
}

/// The same writes into a `Vec<u8>` and a `BytesMut`, which must match, and through the
/// impls forwarding through a `&mut` and a `Box`, whose provided `put` and `put_bytes`
/// grow by `chunk_mut` instead: the bytes, and each vector's capacity.
pub fn writes(data: &[u8], out: &mut Vec<u8>) {
	let [first, cap, data @ ..] = data else {
		return;
	};
	let cap = usize::from(*cap % 24);
	let (mut vec, mut buf) = (Vec::with_capacity(cap), BytesMut::with_capacity(cap));
	let (mut through_vec, mut through_buf) = (Vec::with_capacity(cap), BytesMut::with_capacity(cap));
	let (mut boxed_vec, mut boxed_buf) = (Box::new(Vec::with_capacity(cap)), Box::new(BytesMut::with_capacity(cap)));
	// Each first in turn, so each one's panic is reached.
	for i in 0..6 {
		match first.wrapping_add(i) % 6 {
			0 => write_all(&mut vec, data),
			1 => write_all(&mut buf, data),
			2 => write_all(&mut &mut through_vec, data),
			3 => write_all(&mut &mut through_buf, data),
			4 => write_all(&mut boxed_vec, data),
			_ => write_all(&mut boxed_buf, data),
		}
	}
	// A buffer of the crate's own, in chunks as long as the room `chunk_mut` gives.
	vec.put(Pieces::new(data, data.len() / 2));
	BufMut::put(&mut &mut through_vec, Pieces::new(data, data.len() / 2));
	BufMut::put(&mut boxed_vec, Pieces::new(data, data.len() / 2));
	for vec in [&vec, &through_vec, &boxed_vec] {
		out.extend_from_slice(&(vec.capacity() as u32).to_le_bytes());
	}
	out.extend_from_slice(&through_vec);
	out.extend_from_slice(&through_buf);
	out.push((*boxed_vec == through_vec) as u8);
	out.push((*boxed_buf == through_buf) as u8);
	out.push((vec == buf) as u8);
	out.push((vec.remaining_mut() == buf.remaining_mut()) as u8);
	out.push(buf.has_remaining_mut() as u8);
	out.extend_from_slice(&vec);
}

/// A write by each pair of `data`'s bytes.
fn write_all<B: BufMut>(buf: &mut B, data: &[u8]) {
	let mut data = data;
	while let [op, n, rest @ ..] = data {
		data = rest;
		let (op, n) = (*op, *n);
		let v = u64::from(n) * 0x0101_0101_0101_0101;
		match op % 20 {
			0 => buf.put_u8(n),
			1 => buf.put_i8(n as i8),
			2 => buf.put_u16(v as u16),
			3 => buf.put_i16_le(v as i16),
			4 => buf.put_u32(v as u32),
			5 => buf.put_i32_le(v as i32),
			6 => buf.put_u64_le(v),
			7 => buf.put_i64(v as i64),
			8 => buf.put_u128_le(u128::from(v) << 40),
			9 => buf.put_uint(v, n as usize % 10),
			10 => buf.put_int_le(v as i64, n as usize % 10),
			11 => buf.put_slice(&[op, n]),
			// Past what a vector holds, which panics as `Vec` does directly, and as
			// `remaining_mut` says through a `&mut`.
			12 if n == 0xff => buf.put_bytes(n, usize::MAX),
			12 => buf.put_bytes(n, n as usize % 100),
			13 => buf.put(Bytes::from_static(b"put")),
			14 => buf.put(&[op; 70][..usize::from(n) % 71]),
			15 => buf.put(BytesMut::from(&[n; 3][..])),
			16 => buf.put_f32(f32::from(n) / 3.0),
			17 => buf.put_int(v as i64, n as usize % 10),
			18 => buf.put_uint_le(v, n as usize % 10),
			_ => buf.put_f64(f64::from(n) / 3.0),
		}
		buf.put_u8(buf.has_remaining_mut() as u8 + (buf.remaining_mut() % 7) as u8);
	}
}

/// `Bytes`' own edits, through a `&mut`, and `slice_ref` of a part of it.
pub fn edits(data: &[u8], out: &mut Vec<u8>) {
	let [kind, a, rest @ ..] = data else {
		return;
	};
	let mut bytes = Bytes::copy_from_slice(rest);
	let a = *a as usize % (rest.len() + 2);
	let part = match kind % 6 {
		0 => bytes.split_to(a),
		1 => bytes.split_off(a),
		2 => {
			bytes.truncate(a);
			bytes.clone()
		}
		3 => {
			bytes.clear();
			Bytes::new()
		}
		4 => {
			let sub = &bytes[a.min(rest.len())..];
			bytes.slice_ref(sub)
		}
		_ => {
			bytes.advance(a);
			bytes.copy_to_bytes(a / 2)
		}
	};
	out.extend_from_slice(format!("{part:?} {bytes:?}").as_bytes());
}

/// The bytes of a case's input, taken from the front.
struct Ops<'a>(&'a [u8]);

impl<'a> Ops<'a> {
	fn next(&mut self) -> Option<u8> {
		let (&first, rest) = self.0.split_first()?;
		self.0 = rest;
		Some(first)
	}

	fn byte(&mut self) -> u8 {
		self.next().unwrap_or(0)
	}

	fn take(&mut self, n: usize) -> &'a [u8] {
		let (taken, rest) = self.0.split_at(n.min(self.0.len()));
		self.0 = rest;
		taken
	}
}
