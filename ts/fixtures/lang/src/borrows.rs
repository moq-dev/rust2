//! `Cell`s and `RefCell`s on one thread: read, written, replaced, and taken, of numbers,
//! tuples, and values that drop; borrowed shared and exclusive, held in bindings and
//! dropped; and borrowed again while held, which panics as wasm32's does. `RefCell`s
//! cloned, written through `get_mut`, taken apart, tried, and borrowed through mapped and
//! cloned guards, each released as it drops; guards mapped, filtered, and split to parts
//! JS copies and parts it doesn't, written through, released as the last drops; and
//! guards tried on their parts, given back beside an error, and printed.

use std::cell::{Cell, Ref, RefCell, RefMut};
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
			// A clone borrows what it clones, which panics while it is mutably borrowed, and a
			// clone of a reference to it, or to an `Arc`, is that reference.
			1 => {
				let alias = twin(&&number);
				*alias.borrow_mut() += 1;
				out.push(*number.borrow() as u8);
				out.push(Arc::strong_count(twin(&&log)) as u8);
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

/// What mapped guards lend parts of: a number and a `String`, which JS copies, a `Vec` and
/// a value that logs as it drops, which it doesn't, and bytes, which a reference views.
struct Parts {
	n: u32,
	text: String,
	bytes: Vec<u8>,
	tagged: Tagged,
	maybe: Option<u16>,
	quad: [u8; 4],
}

/// Each byte an operation on `RefCell`s through guards mapped, filtered, and split to their
/// parts, its value the operand; each puts what it read through them, what the cell holds
/// after they drop, whether it is still borrowed, and what dropped.
pub fn maps(data: &[u8], out: &mut Vec<u8>) {
	let log = Arc::new(Mutex::new(Vec::new()));
	let tag = |id| Tagged {
		id,
		n: 0,
		log: log.clone(),
	};
	let parts = RefCell::new(Parts {
		n: 0,
		text: String::new(),
		bytes: Vec::new(),
		tagged: tag(1),
		maybe: None,
		quad: [0; 4],
	});
	let number = RefCell::new(0u32);
	let bytes4 = RefCell::new([1u8, 2, 3, 4]);
	let slot: RefCell<Option<Tagged>> = RefCell::new(None);
	for &op in data {
		match op % 8 {
			// A number written through a guard mapped to it, and through that guard's own map.
			0 => {
				let mut n = RefMut::map(parts.borrow_mut(), |p| &mut p.n);
				*n += u32::from(op);
				out.push(borrowed(&parts));
				let mut again = RefMut::map(n, |n| n);
				*again += 1;
				drop(again);
				out.extend_from_slice(&[borrowed(&parts), parts.borrow().n as u8]);
				let mut whole = RefMut::map(number.borrow_mut(), |n| n);
				*whole = whole.wrapping_mul(3) + u32::from(op);
				drop(whole);
				out.push(*number.borrow() as u8);
			}
			// A `String` pushed to and written whole.
			1 => {
				let mut text = RefMut::map(parts.borrow_mut(), |p| &mut p.text);
				text.push(char::from(b'a' + op % 26));
				if op >> 4 == 0xf {
					*text = String::from("z");
				}
				out.push(text.len() as u8);
				drop(text);
				out.extend_from_slice(parts.borrow().text.as_bytes());
			}
			// A `Vec` and a value that drops written whole, which drops the old one there.
			2 => {
				let mut bytes = RefMut::map(parts.borrow_mut(), |p| &mut p.bytes);
				bytes.push(op);
				if op >> 4 == 0xf {
					*bytes = vec![op, op];
				}
				drop(bytes);
				out.extend_from_slice(&parts.borrow().bytes);
				let mut tagged = RefMut::map(parts.borrow_mut(), |p| &mut p.tagged);
				tagged.n += 1;
				*tagged = Tagged {
					id: op,
					n: tagged.n,
					log: log.clone(),
				};
				flush(&log, out);
				drop(tagged);
				out.extend_from_slice(&[parts.borrow().tagged.id, parts.borrow().tagged.n as u8]);
			}
			// Filtered, which gives the guard back on `None`.
			3 => {
				match RefMut::filter_map(parts.borrow_mut(), |p| p.maybe.as_mut()) {
					Ok(mut m) => {
						*m += 1;
						out.push(*m as u8);
					}
					Err(mut whole) => {
						whole.maybe = Some(u16::from(op));
						out.push(borrowed(&parts));
					}
				}
				out.push(borrowed(&parts));
				match Ref::filter_map(parts.borrow(), |p| p.bytes.last()) {
					Ok(last) => out.extend_from_slice(&[*last, borrowed(&parts)]),
					Err(whole) => out.push(0xf0 | whole.bytes.len() as u8),
				}
				out.push(borrowed(&parts));
				match RefMut::filter_map(slot.borrow_mut(), |s| s.as_mut()) {
					Ok(mut t) => *t = tag(op),
					Err(mut s) => *s = Some(tag(op)),
				}
				flush(&log, out);
				out.push(slot.borrow().as_ref().map_or(0xfd, |t| t.id));
			}
			// Split shared, released as the last part drops.
			4 => {
				let (n, bytes) = Ref::map_split(parts.borrow(), |p| (&p.n, &p.bytes));
				out.extend_from_slice(&[*n as u8, bytes.len() as u8, borrowed(&parts)]);
				drop(n);
				out.push(borrowed(&parts));
				drop(bytes);
				out.push(borrowed(&parts));
			}
			// Split exclusive, released as the last part drops in either order.
			5 => {
				let (mut n, mut text) = RefMut::map_split(parts.borrow_mut(), |p| (&mut p.n, &mut p.text));
				*n += 1;
				text.push('s');
				out.push(borrowed(&parts));
				if op & 0x10 == 0 {
					drop(n);
					out.push(borrowed(&parts));
					drop(text);
				} else {
					drop(text);
					out.push(borrowed(&parts));
					drop(n);
				}
				out.extend_from_slice(&[borrowed(&parts), parts.borrow().n as u8]);
				let (mut low, mut high) = RefMut::map_split(parts.borrow_mut(), |p| {
					let half = p.bytes.len() / 2;
					p.bytes.split_at_mut(half)
				});
				if let Some(b) = low.first_mut() {
					*b = b.wrapping_add(op);
				}
				if let Some(b) = high.last_mut() {
					*b ^= 0xff;
				}
				drop((low, high));
				out.extend_from_slice(&parts.borrow().bytes);
			}
			// Borrowed again while a mapped, filtered, or split guard holds it, which panics.
			6 => match op >> 4 {
				0xc => {
					let _n = RefMut::map(parts.borrow_mut(), |p| &mut p.n);
					let _r = parts.borrow();
				}
				0xd => {
					let (a, _b) = RefMut::map_split(parts.borrow_mut(), |p| (&mut p.n, &mut p.bytes));
					drop(a);
					let _w = parts.borrow_mut();
				}
				0xe => {
					let _f = Ref::filter_map(parts.borrow(), |p| Some(&p.n));
					let _w = parts.borrow_mut();
				}
				0xf => {
					let (_a, b) = Ref::map_split(parts.borrow(), |p| (&p.n, &p.text));
					drop(b);
					let _w = parts.borrow_mut();
				}
				_ => out.push(borrowed(&parts)),
			},
			// Read through mapped guards as what they lend, shared and exclusive.
			_ => {
				let text = Ref::map(parts.borrow(), |p| p.text.as_str());
				let n = RefMut::map(number.borrow_mut(), |n| n);
				out.extend_from_slice(format!("{} {}", &*text, *n + 1).as_bytes());
				out.extend_from_slice(&[borrowed(&parts), borrowed(&number)]);
				drop((text, n));
				let mut quad = RefMut::map(parts.borrow_mut(), |p| &mut p.quad);
				quad[usize::from(op >> 4 & 3)] ^= op;
				if op & 0x40 != 0 {
					*quad = [op; 4];
				}
				drop(quad);
				let quad = Ref::map(parts.borrow(), |p| &p.quad);
				out.extend_from_slice(&*quad);
				drop(quad);
				let mut byte = RefMut::map(bytes4.borrow_mut(), |b| &mut b[usize::from(op & 3)]);
				*byte = byte.wrapping_add(op);
				drop(byte);
				let (low, high) = Ref::map_split(bytes4.borrow(), |b| b.split_at(2));
				out.extend_from_slice(&[low[0] ^ high[1], borrowed(&bytes4)]);
			}
		}
		flush(&log, out);
	}
	let last = parts.into_inner();
	out.extend_from_slice(&[last.n as u8, last.tagged.id, last.maybe.map_or(0xfd, |m| m as u8)]);
	out.extend_from_slice(last.text.as_bytes());
	out.extend_from_slice(&last.bytes);
	drop(last);
	drop(slot);
	flush(&log, out);
}

/// Each byte an operation on `RefCell`s through guards tried on their parts, its value the
/// operand, and printed by `Debug` and `Display`; each puts what it read and printed
/// through them, what the cell holds after they drop, whether it is still borrowed, and
/// what dropped.
pub fn tries(data: &[u8], out: &mut Vec<u8>) {
	let log = Arc::new(Mutex::new(Vec::new()));
	let parts = RefCell::new(Parts {
		n: 0,
		text: String::new(),
		bytes: Vec::new(),
		tagged: Tagged {
			id: 1,
			n: 0,
			log: log.clone(),
		},
		maybe: None,
		quad: [1, 2, 3, 4],
	});
	let number = RefCell::new(0u32);
	for &op in data {
		match op % 4 {
			// Tried exclusive, to a number JS copies, with an error that drops, and to a
			// `Vec` with one that doesn't, each giving the guard back beside its error.
			0 => {
				let tried = RefMut::try_map(parts.borrow_mut(), |p| match p.maybe.as_mut() {
					Some(m) => Ok(m),
					None => Err(Noisy {
						id: op,
						log: log.clone(),
					}),
				});
				match tried {
					Ok(mut m) => {
						*m = m.wrapping_add(u16::from(op));
						out.push(borrowed(&parts));
					}
					Err((mut whole, noisy)) => {
						whole.maybe = Some(u16::from(noisy.id));
						out.push(borrowed(&parts));
						drop(noisy);
						flush(&log, out);
						out.push(borrowed(&parts));
					}
				}
				out.extend_from_slice(&[borrowed(&parts), parts.borrow().maybe.map_or(0xfd, |m| m as u8)]);
				let tried = RefMut::try_map(parts.borrow_mut(), |p| match op & 0x10 {
					0 => Ok(&mut p.bytes),
					_ => Err(()),
				});
				match tried {
					Ok(mut bytes) => {
						bytes.push(op);
						if op >> 5 == 7 {
							*bytes = vec![op; 2];
						}
					}
					Err((whole, ())) => out.push(0xf0 | whole.bytes.len() as u8),
				}
				out.push(borrowed(&parts));
				out.extend_from_slice(&parts.borrow().bytes);
				let mut n = RefMut::try_map(number.borrow_mut(), |n| Ok::<_, u8>(n)).unwrap();
				*n += u32::from(op);
				drop(n);
				out.push(*number.borrow() as u8);
			}
			// Tried shared, and again on what that lends, and its guard back while another
			// shares the cell.
			1 => {
				let other = parts.borrow();
				match Ref::try_map(parts.borrow(), |p| p.bytes.last().ok_or(p.bytes.len())) {
					Ok(last) => {
						let tried = Ref::try_map(last, |b| if *b & 1 == 0 { Ok(b) } else { Err(*b) });
						match tried {
							Ok(b) => out.extend_from_slice(&[*b, borrowed(&parts)]),
							Err((b, e)) => out.extend_from_slice(&[*b ^ e, borrowed(&parts)]),
						}
					}
					Err((whole, len)) => out.extend_from_slice(&[whole.n as u8, len as u8]),
				}
				out.push(borrowed(&parts));
				drop(other);
				out.push(borrowed(&parts));
			}
			// Printed through guards, plain, mapped, and tried, by each trait and with options.
			2 => {
				parts.borrow_mut().text.push(char::from(b'a' + op % 26));
				let text = Ref::map(parts.borrow(), |p| &p.text);
				let quad = Ref::map(parts.borrow(), |p| &p.quad);
				let n = number.borrow();
				let printed = format!("{text} {text:?} [{text:>4}] {quad:?} {n:#?} {n:?} {n:>5}");
				out.extend_from_slice(printed.as_bytes());
				drop((text, quad, n));
				let maybe = RefMut::map(parts.borrow_mut(), |p| &mut p.maybe);
				out.extend_from_slice(format!("{maybe:?} {maybe:#?}").as_bytes());
				drop(maybe);
				let bytes = RefMut::map(parts.borrow_mut(), |p| &mut p.bytes);
				out.extend_from_slice(format!("{bytes:?} {:?}", number.borrow_mut()).as_bytes());
				drop(bytes);
				let pair = RefCell::new((op, "pair"));
				out.extend_from_slice(format!("{:?} {:#?}", pair.borrow(), pair.borrow_mut()).as_bytes());
				let tried = Ref::try_map(pair.borrow(), |p| if p.0 & 1 == 0 { Ok(&p.1) } else { Err(p.0) });
				out.extend_from_slice(format!("{tried:?}").as_bytes());
			}
			// Tried, as it borrows again while the guard it gave back holds it, which panics.
			_ => match op >> 4 {
				0xd => {
					let (_whole, ()) = RefMut::try_map(parts.borrow_mut(), |_| Err::<&mut u32, _>(())).unwrap_err();
					let _r = parts.borrow();
				}
				0xe => {
					let _n = Ref::try_map(parts.borrow(), |p| Ok::<_, ()>(&p.n));
					let _w = parts.borrow_mut();
				}
				0xf => {
					let _n = RefMut::try_map(parts.borrow_mut(), |p| Ok::<_, ()>(&mut p.n));
					let _r = parts.borrow();
				}
				_ => out.push(borrowed(&parts)),
			},
		}
		flush(&log, out);
	}
	let last = parts.into_inner();
	out.extend_from_slice(&[last.n as u8, last.maybe.map_or(0xfd, |m| m as u8)]);
	out.extend_from_slice(last.text.as_bytes());
	out.extend_from_slice(&last.bytes);
	drop(last);
	flush(&log, out);
}

/// Whether `cell` is borrowed exclusively, as 2, shared, as 1, or not at all.
fn borrowed<T>(cell: &RefCell<T>) -> u8 {
	match (cell.try_borrow().is_err(), cell.try_borrow_mut().is_err()) {
		(true, _) => 2,
		(false, true) => 1,
		(false, false) => 0,
	}
}

/// A clone of `t`, which for a reference is that reference.
fn twin<T: Clone>(t: &T) -> T {
	t.clone()
}

/// Puts what dropped since the last flush.
fn flush(log: &Mutex<Vec<u8>>, out: &mut Vec<u8>) {
	let mut log = log.lock().unwrap();
	out.extend_from_slice(&log);
	log.clear();
}
