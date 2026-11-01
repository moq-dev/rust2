//! One-time cells, `OnceCell` and `OnceLock`, of values that are numbers, wide integers,
//! `Option`s whose `None` is `rt.None`, `()`, strings, and vectors: read, set, initialized, taken,
//! cloned, debugged, written through `get_mut`, initialized by a function that may fail,
//! and initialized again from within their own initialization, which panics.

use std::cell::OnceCell;
use std::sync::OnceLock;

macro_rules! cells {
	($($name:ident: $cell:ident<$t:ty>, $make:expr, $put:expr;)*) => {$(
		/// Steps of an operation and an operand on one cell, each putting what it read.
		fn $name(data: &[u8], out: &mut Vec<u8>) {
			let make: fn(u8) -> $t = $make;
			let put: fn(&mut Vec<u8>, &$t) = $put;
			let mut cell: $cell<$t> = $cell::new();
			let mut data = data;
			while let [op, v, rest @ ..] = data {
				data = rest;
				let (op, v) = (*op, *v);
				match op % 13 {
					0 => match cell.get() {
						Some(t) => {
							out.push(1);
							put(out, t);
						}
						None => out.push(0),
					},
					1 => match cell.set(make(v)) {
						Ok(()) => out.push(1),
						Err(t) => {
							out.push(0);
							put(out, &t);
						}
					},
					2 => put(out, cell.get_or_init(|| make(v))),
					// Empty while it initializes.
					3 => {
						let mut seen = 2;
						let t = cell.get_or_init(|| {
							seen = cell.get().is_some() as u8;
							make(v)
						});
						put(out, t);
						out.push(seen);
					}
					4 => match cell.take() {
						Some(t) => {
							out.push(1);
							put(out, &t);
						}
						None => out.push(0),
					},
					// Set from within its own initialization.
					5 => put(
						out,
						cell.get_or_init(|| {
							_ = cell.set(make(v ^ 1));
							make(v)
						}),
					),
					6 => {
						let mut inner = Vec::new();
						let t = cell.get_or_init(|| {
							put(&mut inner, cell.get_or_init(|| make(v ^ 1)));
							make(v)
						});
						put(out, t);
						out.extend_from_slice(&inner);
					}
					7 => put(out, cell.get_or_init(|| panic!("init {v}"))),
					8 => match v % 2 {
						0 => cell = $cell::from(make(v)),
						_ => cell = Default::default(),
					},
					// A clone holds a clone of what it holds, and nothing while it initializes.
					9 => {
						let alias = twin(&&cell);
						let copy = cell.clone();
						let mut inner = None;
						let t = cell.get_or_init(|| {
							inner = Some(cell.clone());
							make(v)
						});
						put(out, t);
						out.push(alias.get().is_some() as u8);
						for copy in [Some(copy), inner].into_iter().flatten() {
							match copy.into_inner() {
								Some(t) => put(out, &t),
								None => out.push(0xee),
							}
						}
					}
					// Debugged, and debugged while it initializes.
					10 => {
						let mut inner = String::new();
						let t = cell.get_or_init(|| {
							inner = format!("{cell:?}");
							make(v)
						});
						put(out, t);
						let text = match v % 2 {
							0 => format!("{inner} {cell:?}"),
							_ => format!("{inner} {cell:#?}"),
						};
						out.extend_from_slice(text.as_bytes());
					}
					11 => match cell.get_mut() {
						Some(t) => {
							*t = make(v);
							out.push(1);
							put(out, cell.get().unwrap());
						}
						None => out.push(0),
					},
					// Fails, or not, and is set from within its own initialization.
					_ => {
						let result = match v % 4 {
							0 | 1 => cell.get_or_try_init(|| made(make, v)),
							2 => cell.get_or_try_init(|| {
								_ = cell.set(make(v ^ 1));
								made(make, v)
							}),
							_ => cell.get_or_try_init(|| {
								let inner = cell.get_or_try_init(|| made(make, v ^ 1)).map(|_| ()).err();
								made(make, inner.unwrap_or(v))
							}),
						};
						match result {
							Ok(t) => {
								out.push(1);
								put(out, t);
							}
							Err(e) => out.extend_from_slice(&[0, e]),
						}
					}
				}
			}
			match cell.into_inner() {
				Some(t) => put(out, &t),
				None => out.push(0xee),
			}
		}
	)*

		/// Steps on the cell and value type `kind` picks.
		pub fn cells(kind: u8, data: &[u8], out: &mut Vec<u8>) {
			let cases: Vec<fn(&[u8], &mut Vec<u8>)> = vec![$($name),*];
			cases[usize::from(kind) % cases.len()](data, out)
		}
	};
}

cells! {
	cell_u8: OnceCell<u8>, |v| v, |out, t| out.push(*t);
	lock_u8: OnceLock<u8>, |v| v, |out, t| out.push(*t);
	cell_u64: OnceCell<u64>, wide, |out, t| out.extend_from_slice(&t.to_be_bytes());
	lock_u64: OnceLock<u64>, wide, |out, t| out.extend_from_slice(&t.to_be_bytes());
	cell_option: OnceCell<Option<u8>>, maybe, put_maybe;
	lock_option: OnceLock<Option<u8>>, maybe, put_maybe;
	cell_unit: OnceCell<()>, |_| (), |out, _| out.push(0xaa);
	lock_unit: OnceLock<()>, |_| (), |out, _| out.push(0xaa);
	cell_string: OnceCell<String>, |v| v.to_string(), |out, t| out.extend_from_slice(t.as_bytes());
	lock_string: OnceLock<String>, |v| v.to_string(), |out, t| out.extend_from_slice(t.as_bytes());
	cell_vec: OnceCell<Vec<u8>>, |v| vec![v; usize::from(v % 4)], |out, t| out.extend_from_slice(t);
	lock_vec: OnceLock<Vec<u8>>, |v| vec![v; usize::from(v % 4)], |out, t| out.extend_from_slice(t);
}

/// A clone of `t`, which for a reference is that reference.
fn twin<T: Clone>(t: &T) -> T {
	t.clone()
}

/// What `make` makes of `v`, or `v` back as an error for every third byte.
fn made<T>(make: fn(u8) -> T, v: u8) -> Result<T, u8> {
	match v % 3 {
		0 => Err(v),
		_ => Ok(make(v)),
	}
}

/// A `u64` past 2^53, from one byte.
fn wide(v: u8) -> u64 {
	u64::from(v) * 0x0101_0101_0101_0101
}

/// `None` for every third byte.
fn maybe(v: u8) -> Option<u8> {
	(v % 3 != 0).then_some(v)
}

fn put_maybe(out: &mut Vec<u8>, t: &Option<u8>) {
	match t {
		Some(v) => out.extend_from_slice(&[1, *v]),
		None => out.push(2),
	}
}
