//! Byte slices passed to the crate's own functions, which take them as their parts unless
//! their bodies make them whole all the same.

/// A use that needs the slice itself.
#[inline(never)]
fn opaque(s: &[u8]) -> u8 {
	s.iter().fold(s.len() as u8, |a, b| (a ^ (a >> 3)).wrapping_add(*b))
}

/// A parameter reassigned from its own tail.
fn sum(mut data: &[u8]) -> u8 {
	let mut sum = 0u8;
	while let Some((&b, rest)) = data.split_first() {
		sum = sum.wrapping_add(b);
		data = rest;
	}
	sum
}

/// A parameter only measured.
fn len(data: &[u8]) -> usize {
	data.len()
}

/// A parameter borrowed, which takes it whole.
fn by_ref(data: &[u8]) -> usize {
	let r = &data;
	len_of(r)
}

fn len_of(data: &&[u8]) -> usize {
	data.len()
}

/// A parameter made whole in a loop, which takes it whole.
fn kept(data: &[u8], n: u8) -> Vec<&[u8]> {
	let mut all = Vec::new();
	for _ in 0..n % 4 {
		all.push(data);
	}
	all
}

/// A reference to an array of bytes.
fn word(w: &[u8; 4]) -> u32 {
	u32::from_be_bytes(*w)
}

/// Two slices, perhaps the same.
fn both(a: &[u8], b: &[u8]) -> u8 {
	opaque(a) ^ len(b) as u8
}

/// A recursive count of the bytes before a zero.
fn depth(data: &[u8]) -> u8 {
	match data.split_first() {
		Some((&b, rest)) if b != 0 => 1 + depth(rest),
		_ => 0,
	}
}

/// Passed along to one that takes it whole, so taken whole too.
fn relay(data: &[u8]) -> u8 {
	opaque(data)
}

/// Passed along to one that takes its parts, so taken as parts too.
fn relay_sum(data: &[u8]) -> u8 {
	sum(data)
}

/// Recursive, and made whole at the bottom, so taken whole all the way down.
fn last_run(data: &[u8]) -> u8 {
	match data.split_first() {
		Some((&b, rest)) if b != 0 => last_run(rest),
		_ => opaque(data),
	}
}

/// Mutually recursive, each reading bytes, so both take parts.
fn evens(data: &[u8]) -> u8 {
	match data.split_first() {
		Some((&b, rest)) => b.wrapping_add(odds(rest)),
		None => 0,
	}
}

fn odds(data: &[u8]) -> u8 {
	match data.split_first() {
		Some((_, rest)) => evens(rest),
		None => 0,
	}
}

/// Mutually recursive, where `ping` makes its slice whole and `pong` only splits its own.
fn ping(data: &[u8]) -> u8 {
	match data.split_first() {
		Some((&b, rest)) if b % 3 != 0 => pong(rest),
		_ => opaque(data),
	}
}

fn pong(data: &[u8]) -> u8 {
	match data.split_first() {
		Some((&b, rest)) => b ^ ping(rest),
		None => 0,
	}
}

/// A slice chosen by a call that does something.
fn pick<'a>(out: &mut Vec<u8>, data: &'a [u8]) -> &'a [u8] {
	out.push(0xbb);
	&data[data.len() / 2..]
}

fn code(out: &mut Vec<u8>, n: u8) -> u8 {
	out.push(0xee);
	n
}

fn pair(n: u8, s: &[u8]) -> u8 {
	n ^ opaque(s)
}

trait Weigh {
	fn weigh(&self, data: &[u8]) -> u8;
}

struct Scale(u8);

impl Weigh for Scale {
	fn weigh(&self, data: &[u8]) -> u8 {
		data.iter().fold(self.0, |a, b| a.wrapping_add(*b ^ 0x5a))
	}
}

fn weigh<W: Weigh>(w: &W, data: &[u8]) -> u8 {
	w.weigh(data)
}

pub fn args(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	match kind % 8 {
		0 => {
			out.push(sum(data));
			out.push(len(data) as u8);
			out.push(by_ref(data) as u8);
			out.push(kept(data, kind).iter().map(|s| opaque(s)).fold(0, u8::wrapping_add));
		}
		1 => {
			if let Some((w, rest)) = data.split_first_chunk::<4>() {
				out.extend_from_slice(&word(w).wrapping_add(1).to_be_bytes());
				out.push(both(rest, rest));
				out.push(both(w, rest));
			}
			out.push(depth(&data[..data.len().min(64)]));
		}
		2 => {
			// As values: a function item, and a pointer to it.
			out.extend(data.split_first().map(|(_, rest)| rest).map(opaque));
			let f: fn(&[u8]) -> u8 = if data.len() % 2 == 0 { opaque } else { sum };
			out.push(f(data));
			out.push(data.split_first().map(|(_, rest)| rest).map_or(0, len) as u8);
		}
		3 => {
			// The earlier argument runs first.
			let n = pair(code(out, 1), pick(out, data));
			out.push(n);
		}
		4 => {
			// Captured by a closure.
			let mut seen = Vec::new();
			let mut note = |b: u8| seen.push(b ^ opaque(data));
			for &b in data.iter().take(3) {
				note(b);
			}
			out.extend_from_slice(&seen);
		}
		6 => {
			let data = &data[..data.len().min(64)];
			out.push(relay(data));
			out.push(relay_sum(data));
			out.push(last_run(data));
			out.push(evens(data));
			out.push(ping(data));
			out.push(pong(data));
		}
		7 => {
			// Callers holding the slice whole, and ones holding its parts, of one callee.
			let halves = [data.split_at(data.len() / 2).0, data.split_at(data.len() / 2).1];
			for half in halves.iter() {
				out.push(sum(half));
				out.push(relay(half));
			}
			out.push(relay_sum(halves[1]));
			out.push(sum(data));
		}
		_ => {
			out.push(weigh(&Scale(kind), data));
			out.push(Scale(3).weigh(&data[data.len().min(2)..]));
		}
	}
}
