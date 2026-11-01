//! Results of byte slices, or of numbers alone, returned by the crate's own functions,
//! which hand back an `Ok`'s parts in a slot their caller reads at once.

#[derive(Debug, Clone, Copy, PartialEq)]
enum Error {
	Short(usize),
	Bad,
}

impl Error {
	fn code(self) -> u8 {
		match self {
			Error::Short(n) => n as u8,
			Error::Bad => 0xff,
		}
	}
}

/// A slice alone, with returns from several places.
fn take(data: &[u8], n: usize) -> Result<&[u8], Error> {
	if n > 200 {
		return Err(Error::Bad);
	}
	match data.split_at_checked(n) {
		Some((head, _)) => Ok(head),
		None => Err(Error::Short(n - data.len())),
	}
}

/// The same slice type through another function, which shares its slot.
fn tail(data: &[u8], n: usize) -> Result<&[u8], Error> {
	(n <= data.len()).then(|| &data[n..]).ok_or(Error::Short(n))
}

/// A number past 2^31 and the rest.
fn wide(data: &[u8]) -> Result<(u64, &[u8]), Error> {
	let (head, rest) = data.split_first_chunk::<8>().ok_or(Error::Short(8))?;
	Ok((u64::from_be_bytes(*head), rest))
}

fn narrow(data: &[u8]) -> Result<(u64, &[u8]), Error> {
	let (&b, rest) = data.split_first().ok_or(Error::Short(1))?;
	match b {
		0xff => Err(Error::Bad),
		_ => Ok((b as u64, rest)),
	}
}

/// What either returns, returned as is.
fn number(data: &[u8], wide_: bool) -> Result<(u64, &[u8]), Error> {
	if wide_ {
		return wide(data);
	}
	narrow(data)
}

/// Each byte before a zero counted, recursively.
fn count(data: &[u8]) -> Result<(u64, &[u8]), Error> {
	match data.split_first() {
		Some((0, rest)) => Ok((0, rest)),
		Some((_, rest)) => {
			let (n, rest) = count(rest)?;
			Ok((n + 1, rest))
		}
		None => Err(Error::Short(1)),
	}
}

/// Two slices, and a reference to an array.
fn halves(data: &[u8]) -> Result<(&[u8], &[u8; 2], &[u8]), u8> {
	let (tag, rest) = data.split_first_chunk::<2>().ok_or(data.len() as u8)?;
	let (a, b) = rest.split_at(rest.len() / 2);
	Ok((a, tag, b))
}

/// Calls whose results are both held while the slot is written again between them.
fn both(data: &[u8]) -> Result<(u64, u64, &[u8]), Error> {
	let (a, rest) = narrow(data)?;
	let (b, rest) = number(rest, a % 2 == 1)?;
	let rest = tail(rest, (a % 3) as usize)?;
	let head = take(rest, (b % 4) as usize)?;
	Ok((a, b ^ head.len() as u64, rest))
}

/// A guard whose drop writes the slot after its function's call did.
struct Guard<'a>(&'a [u8], u8);

impl Drop for Guard<'_> {
	fn drop(&mut self) {
		let _ = narrow(&self.0[self.1 as usize % 2..]);
	}
}

fn guarded<'a>(data: &'a [u8], other: &[u8]) -> Result<(u64, &'a [u8]), Error> {
	let _guard = Guard(other, data.len() as u8);
	narrow(data)
}

/// Errors that are no single value: `()`, and an `Option`.
fn unit_err(data: &[u8]) -> Result<&[u8], ()> {
	data.split_first().map(|(_, rest)| rest).ok_or(())
}

fn none_err(data: &[u8]) -> Result<&[u8], Option<u8>> {
	match data.first() {
		Some(0) => Err(None),
		Some(&b) if b > 0xf0 => Err(Some(b)),
		_ => data.split_first().map(|(_, rest)| rest).ok_or(None),
	}
}

trait Split {
	fn split<'a>(&self, data: &'a [u8]) -> Result<(&'a [u8], &'a [u8]), Error>;
}

struct At(usize);

impl Split for At {
	fn split<'a>(&self, data: &'a [u8]) -> Result<(&'a [u8], &'a [u8]), Error> {
		data.split_at_checked(self.0).ok_or(Error::Short(self.0))
	}
}

// Two instances, of two `Ok` types, so two slots of one function.
fn lead<T: From<u8>>(data: &[u8]) -> Result<(T, &[u8]), Error> {
	let (&first, rest) = data.split_first().ok_or(Error::Short(1))?;
	Ok((T::from(first), rest))
}

fn split_with<'a, S: Split>(s: &S, data: &'a [u8]) -> Result<(&'a [u8], &'a [u8]), Error> {
	s.split(data)
}

fn push(out: &mut Vec<u8>, r: Result<&[u8], Error>) {
	match r {
		Ok(s) => {
			out.push(s.len() as u8);
			out.extend_from_slice(&s[..s.len().min(4)]);
		}
		Err(e) => out.extend_from_slice(&[0xee, e.code()]),
	}
}

fn push_number(out: &mut Vec<u8>, r: Result<(u64, &[u8]), Error>) {
	match r {
		Ok((n, rest)) => {
			out.extend_from_slice(&n.to_be_bytes());
			out.push(rest.len() as u8);
		}
		Err(e) => out.extend_from_slice(&[0xee, e.code()]),
	}
}

/// A slice that takes no argument.
#[inline(never)]
fn fixed() -> Result<&'static [u8], Error> {
	Ok(b"abc")
}

/// Takes `fixed`'s result apart, so it is held as parts.
fn after_fixed(data: &[u8]) -> Result<&[u8], Error> {
	let s = fixed()?;
	Ok(&s[..data.len().min(s.len())])
}

/// An error converted from a residual of a slice and a byte.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Wrapped(u8, u8);

impl From<(&[u8], u8)> for Wrapped {
	fn from((s, n): (&[u8], u8)) -> Self {
		Wrapped(s.len() as u8 ^ s.first().copied().unwrap_or(0x5a), n)
	}
}

/// `?` on a residual whose error is a slice and a byte, converted.
fn wrapped(data: &[u8]) -> Result<(u8, &[u8]), Wrapped> {
	let r: Result<u8, (&[u8], u8)> = match data.split_first() {
		Some((&b, _)) if b % 2 == 0 => Ok(b),
		Some((&b, rest)) => Err((rest, b)),
		None => Err((data, 0)),
	};
	let v = r?;
	Ok((v, data))
}

/// A number alone, past 2^53 from eight bytes.
fn big(data: &[u8]) -> Result<u64, Error> {
	match data.split_first_chunk::<8>() {
		Some((head, _)) => Ok(u64::from_be_bytes(*head)),
		None if data.first() == Some(&0xff) => Err(Error::Bad),
		None => Err(Error::Short(8 - data.len())),
	}
}

/// A signed number and an unsigned one, each through `?`, at an offset.
fn pair(data: &[u8], at: usize) -> Result<(i64, u64), Error> {
	let rest = data.get(at..).ok_or(Error::Short(at))?;
	let a = big(rest)? as i64;
	Ok((a.wrapping_neg(), big(&rest[rest.len().min(3)..])? >> 3))
}

/// A flag, which is no number to return, so it goes in the slot.
fn odd(data: &[u8]) -> Result<bool, Error> {
	let n = big(data)?;
	if n == 7 {
		return Err(Error::Short(7));
	}
	Ok(n % 2 == 1)
}

/// A number whose error is a number too.
fn byte(data: &[u8], i: usize) -> Result<u8, u8> {
	match data.get(i) {
		Some(&b) if b < 0x80 => Ok(b),
		Some(&b) => Err(b),
		None => Err(i as u8),
	}
}

/// A float and a char.
fn mixed(data: &[u8]) -> Result<(f64, char), u8> {
	let b = byte(data, 0)?;
	let c = char::from_u32(byte(data, 1)? as u32 * 37 + 0xd7f0).ok_or(b)?;
	Ok((-(b as f64) / 3.0, c))
}

/// Each byte before a zero counted, recursively, without the rest.
fn depth(data: &[u8]) -> Result<u32, Error> {
	match data.split_first() {
		Some((0, _)) => Ok(0),
		Some((_, rest)) => Ok(depth(rest)? + 1),
		None => Err(Error::Short(1)),
	}
}

/// What either returns, returned as is.
fn either(data: &[u8], first: bool) -> Result<u64, Error> {
	if first {
		return big(data);
	}
	depth(data).map(u64::from)
}

/// A guard whose drop writes the slot after its function's call did.
struct Late<'a>(&'a [u8]);

impl Drop for Late<'_> {
	fn drop(&mut self) {
		let _ = big(self.0);
	}
}

fn late(data: &[u8], other: &[u8]) -> Result<u64, Error> {
	let _late = Late(other);
	big(data)
}

fn late_self(data: &[u8]) -> Result<u64, Error> {
	late(data, data)
}

fn push_big(out: &mut Vec<u8>, r: Result<u64, Error>) {
	match r {
		Ok(n) => out.extend_from_slice(&n.to_be_bytes()),
		Err(e) => out.extend_from_slice(&[0xee, e.code()]),
	}
}

/// Results of numbers alone: held as parts, kept whole, and read after the slot was
/// written again.
fn numbers(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	push_big(out, big(data));
	match pair(data, kind as usize % 4) {
		Ok((a, b)) => {
			out.extend_from_slice(&a.to_be_bytes());
			out.extend_from_slice(&b.to_be_bytes());
		}
		Err(e) => out.push(e.code()),
	}
	match odd(data) {
		Ok(b) => out.push(b as u8),
		Err(e) => out.push(e.code()),
	}
	// Both held while the slot is written again between them.
	let (x, y) = (big(data), big(&data[data.len().min(1)..]));
	match (x, y) {
		(Ok(x), Ok(y)) => out.extend_from_slice(&(x ^ y).to_be_bytes()),
		(x, y) => out.extend_from_slice(&[x.is_ok() as u8, y.is_ok() as u8]),
	}
	// Kept whole: in a collection, and through a function as a value.
	let kept: Vec<Result<u64, Error>> = (0..3).map(|i| big(&data[data.len().min(i)..])).collect();
	for r in kept {
		push_big(out, r);
	}
	let f: fn(&[u8]) -> Result<u64, Error> = if kind % 2 == 0 { big } else { late_self };
	push_big(out, f(data));
	push_big(out, either(data, kind % 3 == 0));
	push_big(out, late(data, &data[data.len().min(2)..]));
	match mixed(data) {
		Ok((f, c)) => {
			out.extend_from_slice(&f.to_bits().to_be_bytes());
			out.extend_from_slice(&(c as u32).to_be_bytes());
		}
		Err(b) => out.extend_from_slice(&[0xee, b]),
	}
	// A loop that calls again while the last result is still read.
	let mut rest = data;
	let mut sum = 0u64;
	while let Ok(n) = depth(rest) {
		sum = sum.wrapping_mul(31).wrapping_add(n as u64);
		rest = &rest[n as usize + 1..];
	}
	out.extend_from_slice(&sum.to_be_bytes());
}

pub fn returns(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	if kind % 8 == 7 {
		return numbers(kind, data, out);
	}
	match kind % 7 {
		0 => {
			push(out, take(data, kind as usize % 7));
			push(out, take(data, 201));
			push(out, tail(data, 3));
			push_number(out, number(data, kind % 2 == 0));
			push_number(out, count(data));
		}
		1 => {
			// Kept whole: in a collection, and read later, after other calls wrote the slot.
			let kept: Vec<Result<&[u8], Error>> = (0..4).map(|n| take(data, n * 3)).collect();
			let last = tail(data, 1);
			for r in kept {
				push(out, r);
			}
			push(out, last);
			match both(data) {
				Ok((a, b, rest)) => {
					out.extend_from_slice(&a.to_be_bytes());
					out.extend_from_slice(&b.to_be_bytes());
					out.push(rest.len() as u8);
				}
				Err(e) => out.push(e.code()),
			}
		}
		2 => {
			// As values: a function item, and a pointer to it.
			let f: fn(&[u8]) -> Result<(u64, &[u8]), Error> = if data.len() % 2 == 0 { wide } else { narrow };
			push_number(out, f(data));
			let all: Vec<_> = [data, &data[data.len().min(3)..]].into_iter().map(count).collect();
			for r in all {
				push_number(out, r);
			}
			out.extend(
				data.split_first()
					.map(|(_, rest)| rest)
					.map(|s| tail(s, 2).map_or(0, <[u8]>::len) as u8),
			);
		}
		3 => {
			match halves(data) {
				Ok((a, tag, b)) => {
					out.extend_from_slice(tag);
					out.push(a.len() as u8);
					out.extend_from_slice(b);
				}
				Err(n) => out.push(n),
			}
			let other = [0xff, 1, 2];
			push_number(out, guarded(data, &other));
			push_number(out, guarded(data, &other[1..]));
		}
		4 => {
			match unit_err(data) {
				Ok(s) => out.push(s.len() as u8),
				Err(()) => out.push(0xee),
			}
			match none_err(data) {
				Ok(s) => out.push(s.len() as u8),
				Err(e) => out.extend_from_slice(&[0xee, e.unwrap_or(0)]),
			}
			match (lead::<u16>(data), lead::<u32>(&data[data.len().min(1)..])) {
				(Ok((a, _)), Ok((b, rest))) => {
					out.extend_from_slice(&a.to_be_bytes());
					out.extend_from_slice(&b.to_be_bytes());
					out.push(rest.len() as u8);
				}
				(a, b) => out.extend_from_slice(&[a.is_ok() as u8, b.is_ok() as u8]),
			}
		}
		5 => {
			match after_fixed(data) {
				Ok(s) => out.extend_from_slice(s),
				Err(e) => out.push(e.code()),
			}
			match wrapped(data) {
				Ok((v, rest)) => out.extend_from_slice(&[v, rest.len() as u8]),
				Err(Wrapped(a, b)) => out.extend_from_slice(&[0xee, a, b]),
			}
		}
		_ => {
			let at = At(kind as usize % 5);
			match split_with(&at, data) {
				Ok((a, b)) => out.extend_from_slice(&[a.len() as u8, b.len() as u8]),
				Err(e) => out.push(e.code()),
			}
			// Through a trait object.
			let dynamic: &dyn Split = &at;
			match dynamic.split(data) {
				Ok((a, b)) => out.extend_from_slice(&[b.len() as u8, a.len() as u8]),
				Err(e) => out.push(e.code()),
			}
			match at.split(data).and_then(|(a, b)| take(b, a.len())) {
				Ok(s) => out.push(s.len() as u8),
				Err(e) => out.push(e.code()),
			}
			// A loop that calls again while the last result is still read.
			let mut rest = data;
			let mut sum = 0u64;
			while let Ok((n, more)) = narrow(rest) {
				sum = sum.wrapping_mul(31).wrapping_add(n);
				rest = more;
			}
			out.extend_from_slice(&sum.to_be_bytes());
		}
	}
}
