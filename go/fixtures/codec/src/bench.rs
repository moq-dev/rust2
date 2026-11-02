//! The workloads of rust2-ts's codec benchmark that code moq's wire format, copied from
//! `ts/fixtures/codec/src/bench.rs` with its ops: varints, frame headers, and groups,
//! encoded and decoded. Its other ops keep collections, which rust2-go does not translate.

use crate::coding::{Decode, DecodeError, Decoder, Encode, EncodeError, Encoder, Form, varint};
use crate::harness::{decode_error, encode_error, form, lite_version, put_u64, put_usize, take_u64};
use crate::lite;

/// Run one workload.
pub fn run(input: &[u8]) -> Vec<u8> {
	let mut out = Vec::new();
	let Some((&op, data)) = input.split_first() else {
		return out;
	};
	let Some((&selector, data)) = data.split_first() else {
		return out;
	};

	match op {
		0 => encoded(encode_varints(data, form(selector), &mut out), &mut out),
		1 => decoded(decode_varints(data, form(selector), &mut out), &mut out),
		2 => encoded(encode_frames(data, lite_version(selector), &mut out), &mut out),
		3 => decoded(decode_frames(data, lite_version(selector), &mut out), &mut out),
		4 => encoded(encode_group(data, lite_version(selector), &mut out), &mut out),
		7 => decoded(decode_split(data, lite_version(selector), &mut out), &mut out),
		_ => decoded(decode_group(data, lite_version(selector), &mut out), &mut out),
	}
	out
}

fn encoded(result: Result<(), EncodeError>, out: &mut Vec<u8>) {
	if let Err(err) = result {
		encode_error(out, &err);
	}
}

fn decoded(result: Result<(), DecodeError>, out: &mut Vec<u8>) {
	if let Err(err) = result {
		decode_error(out, &err);
	}
}

/// The same value, `count` times.
fn encode_varints(data: &[u8], form: Form, out: &mut Vec<u8>) -> Result<(), EncodeError> {
	let (count, data) = take_u64(data);
	let (value, _) = take_u64(data);
	let mut w = Encoder::new(out, form);
	for _ in 0..count as usize {
		w.varint(value)?;
	}
	Ok(())
}

/// Every varint on the wire: how many, and the last.
fn decode_varints(wire: &[u8], form: Form, out: &mut Vec<u8>) -> Result<(), DecodeError> {
	let mut r = Decoder::new(wire, form);
	let mut count = 0;
	let mut last = 0;
	while !r.is_empty() {
		last = r.varint()?;
		count += 1;
	}
	put_usize(out, count);
	put_u64(out, last);
	Ok(())
}

/// A group's FRAME headers without their payloads, as a lite publisher writes them: the
/// zigzag timestamp delta, then the size. The first frame has no delta and its own size,
/// and every `key_every`th frame after it is `key_size`, as moq's benchmark samples are.
fn encode_frames(data: &[u8], version: lite::Version, out: &mut Vec<u8>) -> Result<(), EncodeError> {
	let (count, data) = take_u64(data);
	let (delta, data) = take_u64(data);
	let (first, data) = take_u64(data);
	let (size, data) = take_u64(data);
	let (key_every, data) = take_u64(data);
	let (key_size, _) = take_u64(data);

	let (delta, key_every) = (delta as i64, key_every as usize);

	let mut w = Encoder::new(out, version.into());
	for n in 0..count as usize {
		let (delta, size) = match n {
			0 => (0, first),
			n if key_every != 0 && n % key_every == 0 => (delta, key_size),
			_ => (delta, size),
		};
		w.varint(varint::zigzag(delta))?;
		w.varint(size)?;
	}
	Ok(())
}

/// Every FRAME header on the wire: how many, their total size, and the last delta.
fn decode_frames(wire: &[u8], version: lite::Version, out: &mut Vec<u8>) -> Result<(), DecodeError> {
	let mut r = Decoder::new(wire, version.into());
	let mut count = 0;
	let mut total = 0;
	let mut last = 0;
	while !r.is_empty() {
		last = varint::unzigzag(r.varint()?);
		total += r.varint()?;
		count += 1;
	}
	put_usize(out, count);
	put_u64(out, total);
	put_u64(out, last as u64);
	Ok(())
}

/// The FRAME headers of [`decode_frames`] as a subscriber reads them: the wire after the
/// read size arrives that many bytes at a time, and each read decodes every header it
/// completes. A header cut by the end of a read comes up [`DecodeError::Short`] and is
/// decoded again from its start on the next. How many, their total size, the last delta,
/// and how many decodes came up short.
fn decode_split(data: &[u8], version: lite::Version, out: &mut Vec<u8>) -> Result<(), DecodeError> {
	let (read, wire) = take_u64(data);
	let read = (read as usize).max(1);
	let mut start = 0;
	let mut end = 0;
	let mut count = 0;
	let mut total = 0;
	let mut last = 0;
	let mut short = 0;
	while end < wire.len() {
		end = wire.len().min(end + read);
		while start < end {
			let mut r = Decoder::new(&wire[start..end], version.into());
			match frame_header(&mut r) {
				Ok((delta, size)) => {
					start = end - r.remaining();
					last = delta;
					total += size;
					count += 1;
				}
				Err(DecodeError::Short) => {
					short += 1;
					break;
				}
				Err(err) => return Err(err),
			}
		}
	}
	// The wire ended mid-header.
	if start < end {
		return Err(DecodeError::Short);
	}
	put_usize(out, count);
	put_u64(out, total);
	put_u64(out, last as u64);
	put_usize(out, short);
	Ok(())
}

/// One FRAME header: the zigzag timestamp delta, then the size.
fn frame_header(r: &mut Decoder<'_>) -> Result<(i64, u64), DecodeError> {
	let delta = varint::unzigzag(r.varint()?);
	Ok((delta, r.varint()?))
}

/// One size-prefixed GROUP header, starting at the first frame.
fn encode_group(data: &[u8], version: lite::Version, out: &mut Vec<u8>) -> Result<(), EncodeError> {
	let (subscribe, data) = take_u64(data);
	let (sequence, _) = take_u64(data);
	let group = lite::Group {
		subscribe,
		sequence,
		frame_start: 0,
	};
	group.encode(&mut Encoder::new(out, version.into()), version)
}

fn decode_group(wire: &[u8], version: lite::Version, out: &mut Vec<u8>) -> Result<(), DecodeError> {
	let (group, used) = lite::Group::decode_slice(wire, version)?;
	put_u64(out, group.subscribe);
	put_u64(out, group.sequence);
	put_u64(out, group.frame_start);
	put_usize(out, used);
	Ok(())
}
