//! Byte slices in private fields, which their structs hold as their parts: read, split,
//! written whole and through a `&mut`, taken and swapped, captured, copied, compared, and
//! printed.

/// A cursor through bytes, advanced in place.
#[derive(Clone, Debug, PartialEq)]
struct Scanner<'a> {
	buf: &'a [u8],
	read: usize,
}

impl<'a> Scanner<'a> {
	fn take(&mut self, n: usize) -> Option<&'a [u8]> {
		let (head, rest) = self.buf.split_at_checked(n)?;
		self.buf = rest;
		self.read += n;
		Some(head)
	}

	/// A read undone by a closure that captured the field's `&mut` and the old value.
	fn take_or_rewind(&mut self, n: usize) -> Option<&'a [u8]> {
		let start = self.buf;
		self.take(1)?;
		self.take(n).ok_or(n).inspect_err(|_| self.buf = start).ok()
	}

	fn rest(&mut self) -> &'a [u8] {
		std::mem::take(&mut self.buf)
	}
}

/// Advance a slice through a `&mut` to it.
fn skip_into(buf: &mut &[u8], n: usize) {
	*buf = &buf[n.min(buf.len())..];
}

/// A `Copy` struct of a slice and an array's reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Couple<'a> {
	head: &'a [u8],
	word: &'a [u8; 2],
}

/// A `pub` field, which stays whole.
#[derive(Debug)]
pub struct Exposed<'a> {
	pub buf: &'a [u8],
	at: u8,
}

/// A tuple struct of two slices, one a constant's.
#[derive(Debug, Clone, Copy)]
struct Glued<'a>(&'a [u8], &'a [u8]);

const GLUED: Glued<'static> = Glued(b"ab", &[]);

static SCANNER: Scanner<'static> = Scanner { buf: b"xyz", read: 1 };

/// A generic struct holding a slice beside its parameter.
struct Labeled<'a, T> {
	tag: T,
	bytes: &'a [u8],
}

impl<T: Copy + Into<u32>> Labeled<'_, T> {
	fn sum(&self) -> u32 {
		self.bytes.iter().map(|&b| u32::from(b)).sum::<u32>() + self.tag.into()
	}
}

/// A struct that drops, a class, holding a slice.
struct Sentry<'a, 'b> {
	buf: &'a [u8],
	out: &'b mut Vec<u8>,
}

impl Drop for Sentry<'_, '_> {
	fn drop(&mut self) {
		self.out.push(self.buf.len() as u8);
	}
}

pub fn fields(kind: u8, data: &[u8], out: &mut Vec<u8>) {
	let n = usize::from(kind % 5);
	// Fields read from a struct made once, the one after the slice included.
	let made = Scanner {
		buf: &data[n.min(data.len())..],
		read: usize::from(kind),
	};
	out.push(made.read as u8);
	out.extend_from_slice(&made.buf[..made.buf.len().min(2)]);
	// Reads, splits, and a closure that writes the field back.
	let mut r = Scanner { buf: data, read: 0 };
	while let Some(head) = r.take(n.max(1)) {
		out.extend_from_slice(&head[..head.len().min(1)]);
		if r.read > 12 {
			break;
		}
	}
	let mut r = Scanner { buf: data, read: 0 };
	match r.take_or_rewind(n + 3) {
		Some(got) => out.push(got.len() as u8),
		None => out.push(200),
	}
	out.push(r.buf.len() as u8);
	// A clone that the original's reads don't move, and equality of the two.
	let before = r.clone();
	r.take(1);
	out.push(u8::from(before == r));
	out.push(before.buf.len() as u8);
	// Taken, then replaced and swapped.
	let rest = r.rest();
	out.push(rest.len() as u8);
	out.push(r.buf.len() as u8);
	let mut s = Scanner { buf: data, read: 0 };
	let old = std::mem::replace(&mut s.buf, &data[data.len() / 2..]);
	out.push((old.len() - s.buf.len()) as u8);
	std::mem::swap(&mut r.buf, &mut s.buf);
	out.extend_from_slice(&[r.buf.len() as u8, s.buf.len() as u8]);
	// A `&mut` to the field passed on, and one kept in a variable.
	skip_into(&mut s.buf, n);
	out.push(s.buf.len() as u8);
	let field = &mut r.buf;
	skip_into(field, 1);
	*field = &field[field.len().min(1)..];
	out.push(r.buf.len() as u8);
	// A field whose new value reads it.
	r.buf = &r.buf[r.buf.len() / 2..];
	out.push(r.buf.len() as u8);
	// A `Copy` struct copied, then the original changed.
	if let Some(word) = data.first_chunk::<2>() {
		let mut p = Couple { head: data, word };
		let q = p;
		p.head = &p.head[1..];
		out.extend_from_slice(&[p.head.len() as u8, q.head.len() as u8, u8::from(p == q)]);
		out.extend_from_slice(q.word);
		out.extend_from_slice(format!("{p:?}").as_bytes());
	}
	// Printed, with the `pub` field left whole.
	let open = Exposed {
		buf: &data[..data.len().min(3)],
		at: kind,
	};
	out.extend_from_slice(format!("{open:?}{r:?}").as_bytes());
	out.push(open.at ^ open.buf.len() as u8);
	// A constant's and a static's.
	let two = if kind % 2 == 0 { GLUED } else { Glued(data, GLUED.0) };
	out.extend_from_slice(format!("{two:?}").as_bytes());
	out.push((two.0.len() + two.1.len()) as u8);
	out.extend_from_slice(SCANNER.buf);
	out.push(SCANNER.read as u8);
	// Generic, and a class.
	let tagged = Labeled { tag: kind, bytes: data };
	out.extend_from_slice(&tagged.sum().to_le_bytes());
	let mut g = Sentry { buf: data, out };
	g.buf = &g.buf[g.buf.len().min(2)..];
	g.out.push(g.buf.first().copied().unwrap_or(0));
}
