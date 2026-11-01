//! `Cell`s and `RefCell`s on one thread: read, written, replaced, and taken, of numbers,
//! tuples, and values that drop; borrowed shared and exclusive, held in bindings and
//! dropped; and borrowed again while held, which panics as wasm32's does. `RefCell`s
//! cloned, written through `get_mut`, taken apart, tried, and borrowed through mapped and
//! cloned guards, each released as it drops.

use std::cell::{Cell, Ref, RefCell};
use std::sync::{Arc, Mutex};

/// Logs its id when it drops.
struct Noisy {
	id: u8,
	log: Arc<Mutex<Vec<u8>>>,
}

impl Drop for Noisy {
	fn drop(&mut self) {
		self.log.lock().unwrap().push(self.id);
	}
}

/// What a shared reference changes, as kio's `Polling` holds it.
#[derive(Default)]
struct Polling {
	current: Cell<Option<(u64, u64)>>,
	held: RefCell<Vec<u8>>,
}

/// Each byte an operation on the cells, its value the operand; each puts what it read and
/// what dropped.
pub fn borrows(data: &[u8], out: &mut Vec<u8>) {
	let log = Arc::new(Mutex::new(Vec::new()));
	let count = Cell::new(0u32);
	let pair = Cell::new((0u64, 0u64));
	let noisy: Cell<Option<Noisy>> = Cell::new(None);
	let polling = Polling::default();
	let shared: RefCell<Vec<u8>> = RefCell::new(Vec::new());
	let owned = RefCell::new(Noisy {
		id: 0xaa,
		log: log.clone(),
	});
	for &op in data {
		match op % 10 {
			0 => count.set(count.get().wrapping_add(u32::from(op))),
			// A copy out, written, leaves the cell as it was.
			1 => {
				let mut copy = pair.get();
				copy.0 += u64::from(op);
				out.push(pair.get().0 as u8);
				pair.set((copy.1, copy.0));
				out.push(pair.get().1 as u8);
			}
			2 => out.push(count.replace(u32::from(op)) as u8),
			// Drops the value it held.
			3 => noisy.set(Some(Noisy {
				id: op,
				log: log.clone(),
			})),
			4 => {
				let taken = noisy.take();
				out.push(taken.as_ref().map_or(0xfd, |n| n.id));
			}
			5 => {
				let outer = polling.current.replace(Some((u64::from(op), 1)));
				polling.held.borrow_mut().push(op);
				out.push(outer.map_or(0xfd, |(a, _)| a as u8));
				out.push(polling.held.borrow().len() as u8);
			}
			// Two shared borrows at once.
			6 => {
				let a = shared.borrow();
				let b = shared.borrow();
				out.push((a.len() + b.len()) as u8);
			}
			7 => {
				let mut w = shared.borrow_mut();
				w.push(op);
				out.push(w.len() as u8);
				drop(w);
				out.push(shared.borrow().len() as u8);
			}
			// Replaced through a borrow, which drops the old value.
			8 => {
				*owned.borrow_mut() = Noisy {
					id: op,
					log: log.clone(),
				};
				out.push(owned.borrow().id);
			}
			// Borrowed while an exclusive borrow is held, or exclusively while any is.
			_ => match op >> 4 {
				0xd => {
					let _r = shared.borrow();
					let _w = shared.borrow_mut();
				}
				0xe => {
					let _w = shared.borrow_mut();
					let _r = shared.borrow();
				}
				0xf => {
					let _w = shared.borrow_mut();
					let _again = shared.borrow_mut();
				}
				_ => {
					drop(shared.borrow_mut());
					out.push(shared.borrow().len() as u8);
				}
			},
		}
		flush(&log, out);
	}
	out.push(count.get() as u8);
	out.push(polling.current.get().map_or(0xfd, |(a, b)| (a + b) as u8));
	drop(noisy);
	drop(owned);
	flush(&log, out);
}

/// Logs its id as it is cloned, with the high bit set, and as it drops; its clone's id
/// is one more.
struct Tagged {
	id: u8,
	n: u32,
	log: Arc<Mutex<Vec<u8>>>,
}

impl Clone for Tagged {
	fn clone(&self) -> Self {
		self.log.lock().unwrap().push(0x80 | self.id);
		Self {
			id: self.id.wrapping_add(1),
			n: self.n,
			log: self.log.clone(),
		}
	}
}

impl Drop for Tagged {
	fn drop(&mut self) {
		self.log.lock().unwrap().push(self.id);
	}
}

/// Each byte an operation on `RefCell`s, its value the operand: cloned, written through
/// `get_mut`, taken apart, tried, and borrowed through mapped and cloned guards; each puts
/// what it read and what dropped.
pub fn guards(data: &[u8], out: &mut Vec<u8>) {
	let log = Arc::new(Mutex::new(Vec::new()));
	let tag = |id| Tagged {
		id,
		n: 0,
		log: log.clone(),
	};
	let mut number = RefCell::new(0u32);
	let mut bytes: RefCell<Vec<u8>> = RefCell::new(Vec::new());
	let mut tagged = RefCell::new(tag(0));
	for &op in data {
		match op % 8 {
			0 => {
				*number.get_mut() += u32::from(op);
				bytes.get_mut().push(op);
				tagged.get_mut().n += 1;
				out.push(*number.borrow() as u8);
			}
			// A clone borrows what it clones, which panics while it is mutably borrowed.
			1 => {
				let held = (op >> 4 == 0xe).then(|| tagged.borrow());
				let held_mut = (op >> 4 == 0xf).then(|| tagged.borrow_mut());
				let copy = tagged.clone();
				out.push(copy.borrow().id);
				drop((held, held_mut));
				out.extend_from_slice(&bytes.clone().into_inner());
				out.push(number.clone().into_inner() as u8);
			}
			// Taken apart, which drops nothing until what it held does.
			2 => {
				let inner = RefCell::new(tag(op)).into_inner();
				out.push(inner.id);
				flush(&log, out);
				drop(inner);
			}
			// Tried while borrowed shared, then exclusive, then free.
			3 => {
				let shared = bytes.borrow();
				let read = bytes.try_borrow();
				out.push(read.is_ok() as u8);
				match bytes.try_borrow_mut() {
					Ok(_) => out.push(0xfe),
					Err(e) => out.extend_from_slice(format!("{e} {e:?}").as_bytes()),
				}
				drop((read, shared));
				let write = bytes.try_borrow_mut();
				out.push(write.is_ok() as u8);
				match bytes.try_borrow() {
					Ok(_) => out.push(0xfe),
					Err(e) => out.extend_from_slice(format!("{e} {e:?}").as_bytes()),
				}
				drop(write);
				let pushed = bytes.try_borrow_mut().map(|mut b| {
					b.push(op);
					b.len() as u8
				});
				out.push(pushed.unwrap_or(0xfd));
			}
			// Mapped to a part, and that to a part of its own, released as each drops.
			4 => {
				let n = Ref::map(tagged.borrow(), |t| &t.n);
				let id = Ref::map(tagged.borrow(), |t| &t.id);
				out.extend_from_slice(&[*n as u8, *id]);
				out.push(tagged.try_borrow_mut().is_err() as u8);
				drop(n);
				out.push(tagged.try_borrow_mut().is_err() as u8);
				drop(id);
				out.push(tagged.try_borrow_mut().is_err() as u8);
				let half = Ref::map(bytes.borrow(), |b| &b[b.len() / 2..]);
				out.push(half.len() as u8);
				let last = Ref::map(half, |s| s.last().unwrap_or(&0xfd));
				out.push(*last);
				out.push(bytes.try_borrow_mut().is_err() as u8);
				drop(last);
				out.push(bytes.try_borrow_mut().is_err() as u8);
			}
			// Cloned, each clone a borrow of its own, mapped or not.
			5 => {
				let a = bytes.borrow();
				let b = Ref::clone(&a);
				drop(a);
				out.push(bytes.try_borrow_mut().is_err() as u8);
				let c = Ref::map(Ref::clone(&b), |v| v.first().unwrap_or(&0xfd));
				let d = Ref::clone(&c);
				drop((b, c));
				out.push(*d);
				out.push(bytes.try_borrow_mut().is_err() as u8);
				drop(d);
				out.push(bytes.try_borrow_mut().is_err() as u8);
			}
			// Borrowed exclusively while a mapped or cloned guard holds it, which panics.
			6 => match op >> 4 {
				0xe => {
					let _n = Ref::map(tagged.borrow(), |t| &t.n);
					let _w = tagged.borrow_mut();
				}
				0xf => {
					let a = tagged.borrow();
					let _b = Ref::clone(&a);
					drop(a);
					let _w = tagged.borrow_mut();
				}
				_ => out.push(tagged.borrow_mut().id),
			},
			// The cell replaced, and the one it was taken apart.
			_ => {
				let old = std::mem::replace(&mut tagged, RefCell::new(tag(op)));
				out.push(old.into_inner().id);
			}
		}
		flush(&log, out);
	}
	out.push(number.into_inner() as u8);
	out.extend_from_slice(&bytes.into_inner());
	let last = tagged.into_inner();
	out.push(last.id);
	flush(&log, out);
	drop(last);
	flush(&log, out);
}

/// Puts what dropped since the last flush.
fn flush(log: &Mutex<Vec<u8>>, out: &mut Vec<u8>) {
	let mut log = log.lock().unwrap();
	out.extend_from_slice(&log);
	log.clear();
}
