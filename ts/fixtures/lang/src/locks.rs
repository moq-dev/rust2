//! Mutexes on one thread, where a panic aborts and so never poisons one: made by
//! `Default`, read and written through a guard, recovered from a `PoisonError` that never
//! comes, read and written by `get_mut` without locking, taken apart, tried while held and free, and locked again
//! while held, which panics as wasm32's does; and read-write locks, read and written,
//! tried while read and while written, written through `get_mut`, taken apart, read
//! and written through guards mapped and filtered to parts of their values, and locked
//! again while a guard holds them, which aborts as wasm32's does.

use std::fmt;
use std::sync::{
	Arc, MappedRwLockReadGuard, MappedRwLockWriteGuard, Mutex, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard,
	TryLockError,
};

/// A value a mutex holds, which `Default` makes.
#[derive(Default)]
struct Tally {
	n: u64,
	seen: Vec<u8>,
}

/// A mutex that debugs through `try_lock`, as kio's `Lock` does.
struct Lock(Mutex<u32>);

impl fmt::Debug for Lock {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self.0.try_lock() {
			Ok(guard) => f.debug_tuple("Lock").field(&*guard).finish(),
			Err(_) => f.debug_tuple("Lock").field(&"<locked>").finish(),
		}
	}
}

/// Each byte an operation on the mutexes, its value the operand; each puts what it read.
pub fn locks(data: &[u8], out: &mut Vec<u8>) {
	let shared: Arc<Mutex<Tally>> = Arc::default();
	let other = shared.clone();
	let mut own: Mutex<Vec<u8>> = Mutex::default();
	let mut number: Mutex<u32> = Mutex::default();
	let mut slot: Mutex<Option<u8>> = Mutex::default();
	let lock = Lock(Mutex::new(7));
	for &op in data {
		match op % 8 {
			0 => {
				let mut tally = shared.lock().unwrap_or_else(PoisonError::into_inner);
				tally.n += u64::from(op);
				tally.seen.push(op);
			}
			1 => {
				let tally = other.lock().expect("never poisoned");
				out.push(tally.n as u8);
				out.push(tally.seen.len() as u8);
			}
			// A value JS copies, written in place, as moq's `Content` takes its own.
			2 => {
				own.get_mut().unwrap().push(op);
				*number.get_mut().unwrap() += u32::from(op);
				let held = slot.get_mut().unwrap_or_else(PoisonError::into_inner);
				out.push(held.take().unwrap_or(0xfd));
				*held = Some(op);
			}
			3 => {
				*number.lock().unwrap() += u32::from(op);
				out.push(*number.lock().unwrap() as u8);
			}
			// Tried while another guard holds it, and again once that guard drops.
			4 => {
				let held = shared.lock().unwrap();
				out.push(matches!(other.try_lock(), Err(TryLockError::WouldBlock)) as u8);
				drop(held);
				match other.try_lock() {
					Ok(tally) => out.push(tally.seen.len() as u8),
					Err(TryLockError::WouldBlock) => out.push(0xfe),
					Err(TryLockError::Poisoned(err)) => out.push(err.into_inner().n as u8),
				}
			}
			// Debugged free and held, and a guard kept in a binding until the block ends.
			5 => {
				*lock.0.lock().unwrap() += u32::from(op);
				let free = format!("{lock:?}");
				let guard = lock.0.try_lock();
				let held = format!("{lock:?}");
				out.extend_from_slice(format!("{free} {held} {}", guard.is_ok()).as_bytes());
			}
			6 => out.push(std::thread::panicking() as u8),
			// Matched, as a caller that tells the two apart does.
			_ => match shared.lock() {
				Ok(tally) => out.push(tally.seen.len() as u8),
				Err(err) => out.push(err.into_inner().n as u8 ^ 0xff),
			},
		}
	}
	out.extend_from_slice(own.get_mut().unwrap_or_else(PoisonError::into_inner));
	out.push(Arc::strong_count(&shared) as u8);
	out.push(number.into_inner().unwrap() as u8);
	out.push(slot.into_inner().unwrap().unwrap_or(0xfd));
}

/// Locks a lock again while a guard of it is held, as the first byte picks, then once more
/// after the guard drops: a mutex, locked or tried, which panics as wasm32's does, or a
/// read-write lock read or written through a guard, mapped or not, then read if the low
/// bit is clear and otherwise written, which aborts as wasm32's does unless both read.
pub fn relock(data: &[u8], out: &mut Vec<u8>) {
	let Some(&pick) = data.first() else {
		return;
	};
	let mutex = Mutex::new(pick);
	let lock = RwLock::new(Tally {
		n: u64::from(pick),
		seen: vec![pick],
	});
	let again = |out: &mut Vec<u8>| match pick & 1 {
		0 => out.push(lock.read().unwrap().n as u8),
		_ => out.push(lock.write().unwrap().seen.len() as u8),
	};
	match (pick >> 1) % 8 {
		0 => {
			let guard = mutex.lock().unwrap();
			out.push(*guard);
			out.push(*mutex.lock().unwrap());
		}
		1 => {
			let Ok(guard) = mutex.try_lock() else {
				unreachable!("nothing holds the mutex");
			};
			out.push(*guard);
			out.push(*mutex.lock().unwrap());
		}
		2 => {
			let read = lock.read().unwrap();
			out.push(read.n as u8);
			again(out);
		}
		3 => {
			let write = lock.write().unwrap();
			out.push(write.n as u8);
			again(out);
		}
		4 => {
			let seen = RwLockReadGuard::map(lock.read().unwrap(), |t| &t.seen);
			out.push(seen.len() as u8);
			again(out);
		}
		5 => {
			let n = RwLockWriteGuard::map(lock.write().unwrap(), |t| &mut t.n);
			out.push(*n as u8);
			again(out);
		}
		// A mapped guard's own maps, which hold the lock as the guard they came from did.
		6 => {
			let Ok(first) = RwLockReadGuard::filter_map(lock.read().unwrap(), |t| t.seen.first()) else {
				unreachable!("seen holds the pick");
			};
			let first = MappedRwLockReadGuard::map(first, |b| b);
			out.push(*first);
			again(out);
		}
		_ => {
			let n = RwLockWriteGuard::map(lock.write().unwrap(), |t| &mut t.n);
			let n = MappedRwLockWriteGuard::map(n, |n| n);
			out.push(*n as u8);
			again(out);
		}
	}
	out.push(*mutex.lock().unwrap());
	again(out);
}

/// Each byte an operation on read-write locks, its value the operand; each puts what it
/// read.
pub fn rwlocks(data: &[u8], out: &mut Vec<u8>) {
	let lock: Arc<RwLock<Tally>> = Arc::default();
	let mut number = RwLock::new(0u32);
	let mut owned: RwLock<Vec<u8>> = RwLock::default();
	for &op in data {
		match op % 8 {
			0 => {
				let mut tally = lock.write().unwrap();
				tally.n += u64::from(op);
				tally.seen.push(op);
			}
			// Two reads at once.
			1 => {
				let a = lock.read().unwrap();
				let b = lock.read().expect("never poisoned");
				out.push(a.seen.len() as u8);
				out.push(b.n as u8);
			}
			2 => {
				*number.write().unwrap() += u32::from(op);
				out.push(*number.read().unwrap() as u8);
			}
			// Tried while a read is held, then while a write is.
			3 => {
				let read = lock.read().unwrap();
				out.push(lock.try_read().is_ok() as u8);
				out.push(matches!(lock.try_write(), Err(TryLockError::WouldBlock)) as u8);
				drop(read);
				let write = lock.try_write();
				out.push(write.is_ok() as u8);
				out.push(matches!(lock.try_read(), Err(TryLockError::WouldBlock)) as u8);
			}
			4 => match lock.try_write() {
				Ok(tally) => out.push(tally.seen.len() as u8),
				Err(TryLockError::WouldBlock) => out.push(0xfe),
				Err(TryLockError::Poisoned(err)) => out.push(err.into_inner().n as u8),
			},
			5 => {
				let made: RwLock<Vec<u8>> = RwLock::default();
				made.write().unwrap().push(op);
				out.extend_from_slice(&made.read().unwrap());
			}
			6 => {
				*number.get_mut().unwrap() += u32::from(op);
				owned.get_mut().unwrap_or_else(PoisonError::into_inner).push(op);
				out.push(*number.read().unwrap() as u8);
			}
			_ => {
				let made = RwLock::new(Tally {
					n: u64::from(op),
					seen: vec![op],
				});
				let tally = made.into_inner().unwrap();
				out.push(tally.n as u8);
				out.extend_from_slice(&tally.seen);
			}
		}
	}
	out.push(lock.read().unwrap().n as u8);
	out.push(number.into_inner().unwrap() as u8);
	out.extend_from_slice(&owned.into_inner().unwrap_or_else(PoisonError::into_inner));
}

/// Each byte an operation on a read-write lock through guards mapped and filtered to its
/// parts, its value the operand; each puts what it read through them, what the lock holds
/// after they drop, and whether it is still locked.
pub fn mapped(data: &[u8], out: &mut Vec<u8>) {
	let lock = RwLock::new(Tally::default());
	let locked = |out: &mut Vec<u8>| {
		let read = matches!(lock.try_read(), Err(TryLockError::WouldBlock));
		let write = matches!(lock.try_write(), Err(TryLockError::WouldBlock));
		out.push(u8::from(read) << 1 | u8::from(write));
	};
	for &op in data {
		match op % 4 {
			// A number written through a write guard mapped to it, and through that one's map.
			0 => {
				let mut n = RwLockWriteGuard::map(lock.write().unwrap(), |t| &mut t.n);
				*n += u64::from(op);
				locked(out);
				let mut again = MappedRwLockWriteGuard::map(n, |n| n);
				*again = again.wrapping_mul(3);
				drop(again);
				locked(out);
				out.push(lock.read().unwrap().n as u8);
			}
			// Read through a read guard mapped to a part, and that one's map to a part of it.
			1 => {
				let seen = RwLockReadGuard::map(lock.read().unwrap(), |t| &t.seen);
				out.push(seen.len() as u8);
				let last = MappedRwLockReadGuard::map(seen, |s| s.last().unwrap_or(&0xfd));
				out.push(*last);
				locked(out);
				drop(last);
				locked(out);
			}
			// Filtered, which gives the guard back where there is no part.
			2 => {
				match RwLockWriteGuard::filter_map(lock.write().unwrap(), |t| t.seen.last_mut()) {
					Ok(mut last) => *last = last.wrapping_add(op),
					Err(mut whole) => whole.seen.push(op),
				}
				locked(out);
				match RwLockReadGuard::filter_map(lock.read().unwrap(), |t| t.seen.first()) {
					Ok(first) => match MappedRwLockReadGuard::filter_map(first, |b| (*b > 0x80).then_some(b)) {
						Ok(b) => out.push(*b),
						Err(first) => out.push(!*first),
					},
					Err(whole) => out.push(0xf0 | whole.seen.len() as u8),
				}
				locked(out);
				let n = RwLockWriteGuard::filter_map(lock.write().unwrap(), |t| Some(&mut t.n));
				match n.map(|n| MappedRwLockWriteGuard::filter_map(n, |n| (*n & 1 == 0).then_some(n))) {
					Ok(Ok(mut n)) => *n += 1,
					Ok(Err(mut n)) => *n += 2,
					Err(_) => out.push(0xfe),
				}
				out.push(lock.read().unwrap().n as u8);
				out.extend_from_slice(&lock.read().unwrap().seen);
			}
			// Tried while a mapped write guard holds it, then while a mapped read guard does.
			_ => {
				let n = RwLockWriteGuard::map(lock.write().unwrap(), |t| &mut t.n);
				locked(out);
				drop(n);
				let n = RwLockReadGuard::map(lock.read().unwrap(), |t| &t.n);
				locked(out);
				out.push(*n as u8);
			}
		}
	}
	let last = lock.into_inner().unwrap();
	out.push(last.n as u8);
	out.extend_from_slice(&last.seen);
}
