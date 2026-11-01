//! The iterators std makes of a `Result`: `iter`, `iter_mut`, and `into_iter`, by value and
//! through a reference, alone and beside others, and `flatten` over `Result`s, read from
//! both ends and written through. Of payloads JS copies, holds as objects, holds as
//! `undefined`, nests in an `Option`, and lends as a slice, with each `Ok` and `Err` dropped
//! where std drops it.

use std::cell::RefCell;
use std::rc::Rc;

/// The iterators of `Result`s of each payload `kind` picks.
pub fn results(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	match kind % 7 {
		0 => copied(data, out),
		1 => owned(data, out),
		2 => units(data, out),
		3 => nested(data, out),
		4 => arrays::<0>(data, out),
		5 => arrays::<1>(data, out),
		_ => arrays::<5>(data, out),
	}
}

/// `Ok` for a byte not divisible by 3, else `Err`.
fn split(b: u8) -> Result<u8, u8> {
	if b % 3 == 0 { Err(b) } else { Ok(b) }
}

/// Of bytes, which JS copies, so a `&mut` lends each as its place.
fn copied(data: &[u8], out: &mut Vec<u8>) {
	let mut results: Vec<Result<u8, u8>> = data.iter().map(|&b| split(b)).collect();
	for r in &results {
		out.extend(r.iter());
		out.push(r.iter().len() as u8);
		let (lo, hi) = r.iter().size_hint();
		out.extend([lo as u8, hi.map_or(0xee, |h| h as u8)]);
	}
	for r in results.iter_mut() {
		for x in r.iter_mut() {
			*x = x.wrapping_add(1);
		}
	}
	for r in &mut results {
		for x in r {
			*x ^= 0x10;
		}
	}
	out.extend(results.iter().flatten());
	for x in results.iter_mut().flatten() {
		*x = x.wrapping_mul(3);
	}
	out.extend(results.iter().copied().flatten());
	out.extend(results.iter().flatten().rev());
	// From both ends at once, until they meet.
	let mut it = results.iter().flatten();
	while let (Some(a), b) = (it.next(), it.next_back()) {
		out.extend([*a, b.copied().unwrap_or(0xaa)]);
	}
	// Beside another iterator, through `IntoIterator` of a reference to a `Result`.
	let mut first = results.first().copied().unwrap_or(Err(0));
	out.extend(data.iter().chain(&first).chain(first.iter().rev()));
	for x in data.iter().take(1).zip(&mut first) {
		*x.1 = x.1.wrapping_sub(*x.0);
	}
	out.extend(first.into_iter().chain(first));
	// One read item by item, and a clone, which reads on independently.
	let mut it = first.iter();
	let cloned = it.clone();
	out.push(it.next().copied().unwrap_or(0xee));
	out.push(it.next().is_none() as u8);
	out.push(cloned.count() as u8);
	let mut into = first.into_iter();
	let mut back = into.clone();
	out.extend([into.next(), back.next_back(), back.next()].map(|x| x.unwrap_or(0xdd)));
	let lent: Vec<&u8> = Vec::from_iter(&first);
	out.extend(lent.into_iter().copied());
}

/// Logs its id as it drops.
struct Logged {
	id: u8,
	log: Rc<RefCell<Vec<u8>>>,
}

impl Drop for Logged {
	fn drop(&mut self) {
		self.log.borrow_mut().push(self.id);
	}
}

/// Puts what was logged since the last flush, after a marker.
fn flush(log: &RefCell<Vec<u8>>, out: &mut Vec<u8>) {
	out.push(0xfd);
	out.extend(log.borrow_mut().drain(..));
}

/// Of objects that drop, each `Ok` and `Err` alike: `into_iter` drops an `Err` as it makes
/// the iterator, and the iterator the `Ok` it never yielded.
fn owned(data: &[u8], out: &mut Vec<u8>) {
	let log = Rc::new(RefCell::new(Vec::new()));
	let made = |b: u8| -> Result<Logged, Logged> {
		let logged = Logged {
			id: b,
			log: log.clone(),
		};
		if b % 3 == 0 { Err(logged) } else { Ok(logged) }
	};
	for &b in data {
		let it = made(b).into_iter();
		log.borrow_mut().push(0xfe);
		drop(it);
		for l in made(b) {
			out.push(l.id);
		}
		flush(&log, out);
	}
	let mut results: Vec<Result<Logged, Logged>> = data.iter().map(|&b| made(b)).collect();
	for r in &results {
		out.extend(r.iter().map(|l| l.id));
	}
	for r in results.iter_mut() {
		for l in r {
			l.id ^= 0x40;
		}
	}
	for l in results.iter_mut().flatten().rev() {
		l.id = l.id.wrapping_add(1);
	}
	out.extend(results.iter().flatten().map(|l| l.id));
	flush(&log, out);
	// What a `take` leaves drops with the rest of the source.
	for l in results.into_iter().flatten().take(2) {
		out.push(l.id);
		flush(&log, out);
	}
	flush(&log, out);
	let rest: Vec<Logged> = data.iter().rev().map(|&b| made(b)).flatten().collect();
	flush(&log, out);
	out.extend(rest.iter().map(|l| l.id));
	drop(rest);
	flush(&log, out);
}

/// Of `()`, which JS holds as `undefined`, so an `Option` of one is `rt.None`.
fn units(data: &[u8], out: &mut Vec<u8>) {
	let mut results: Vec<Result<(), u8>> = data.iter().map(|&b| split(b).map(|_| ())).collect();
	out.push(results.iter().flatten().count() as u8);
	for r in &mut results {
		let mut it = r.iter_mut();
		out.push(it.next().is_some() as u8);
		out.push(it.next().is_some() as u8);
	}
	for r in &results {
		out.push(r.iter().next().map_or(0xa0, |()| 0xa1));
		out.push(r.into_iter().next_back().is_some() as u8);
	}
	out.push(results.into_iter().flatten().count() as u8);
}

/// Of an `Option`, which `flatten` reads into in turn.
fn nested(data: &[u8], out: &mut Vec<u8>) {
	let mut results: Vec<Result<Option<u8>, u8>> = data
		.iter()
		.map(|&b| match b % 4 {
			0 => Err(b),
			1 => Ok(None),
			_ => Ok(Some(b)),
		})
		.collect();
	for r in &results {
		out.push(match r.iter().next() {
			Some(Some(b)) => *b,
			Some(None) => 0xa3,
			None => 0xa4,
		});
	}
	for o in results.iter_mut().flatten() {
		match o {
			Some(b) => *b = b.wrapping_add(1),
			None => *o = Some(0xa5),
		}
	}
	out.extend(results.iter().flatten().flatten());
	out.extend(results.into_iter().flatten().flatten().rev());
}

/// `N` bytes of `data` from `at`, past its end each its index.
fn bytes_at<const N: usize>(data: &[u8], at: usize) -> [u8; N] {
	std::array::from_fn(|i| data.get(at + i).copied().unwrap_or((at + i) as u8))
}

/// Of arrays of bytes, each lent as a slice of it: of no bytes, one, and many.
fn arrays<const N: usize>(data: &[u8], out: &mut Vec<u8>) {
	// An `Err` in the middle and at the end.
	let mut results: Vec<Result<[u8; N], u8>> = (0..5)
		.map(|k| match k {
			2 | 4 => Err(k as u8),
			_ => Ok(bytes_at(data, k * N)),
		})
		.collect();
	for a in results.iter().flatten() {
		out.extend_from_slice(a);
	}
	for a in results.iter().flatten().rev() {
		out.extend_from_slice(a);
	}
	for a in results.iter_mut().flatten() {
		a.reverse();
		if let Some(x) = a.last_mut() {
			*x ^= 0x11;
		}
	}
	for r in &mut results {
		for a in r.iter_mut() {
			if let Some(x) = a.first_mut() {
				*x = x.wrapping_add(3);
			}
		}
	}
	let mut first = results[0];
	let other = results[1];
	for a in first.iter().chain(&other).chain(results.iter().flatten()) {
		out.extend_from_slice(a);
	}
	for a in first.iter_mut().chain(&mut results[3]) {
		a.fill(0x5a);
	}
	for a in &mut first {
		a.reverse();
	}
	for a in &first {
		out.extend_from_slice(a);
	}
	for r in &results {
		for a in r {
			out.extend_from_slice(a);
		}
	}
	let copied: Vec<[u8; N]> = results.into_iter().flatten().chain(first).collect();
	for a in &copied {
		out.extend_from_slice(a);
	}
}
