//! Mutexes on one thread, where a panic aborts and so never poisons one: made by
//! `Default`, read and written through a guard, recovered from a `PoisonError` that never
//! comes, read and written by `get_mut` without locking, taken apart, tried while held and free, and locked again
//! while held, which panics as wasm32's does; and read-write locks, read and written,
//! tried while read and while written, written through `get_mut`, taken apart, and read
//! and written through guards mapped and filtered to parts of their values, a mutex's
//! included; write guards downgraded, which keep the lock read; guards mapped to the whole
//! of the bytes a lock holds; and each lock's guards printed.

use std::fmt;
use std::sync::{
	Arc, MappedMutexGuard, MappedRwLockReadGuard, MappedRwLockWriteGuard, Mutex, MutexGuard, PoisonError, RwLock,
	RwLockReadGuard, RwLockWriteGuard, TryLockError,
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

/// Locks a mutex while a guard of it is held, if the first byte is odd, which panics.
pub fn relock(data: &[u8], out: &mut Vec<u8>) {
	let mutex = Mutex::new(data.len() as u8);
	let guard = mutex.lock().unwrap();
	out.push(*guard);
	if data.first().is_some_and(|b| b % 2 == 1) {
		out.push(*mutex.lock().unwrap());
	}
	drop(guard);
	out.push(*mutex.lock().unwrap());
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
/// after they drop, and whether it is still locked. Locking it again while one is held
/// aborts on wasm32, with no panic message to compare.
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

/// Each byte an operation on mutexes through guards mapped and filtered to parts of their
/// values, its value the operand; each puts what it read through them, what the mutex
/// holds after they drop, and whether it is still locked. Locking it again while one is
/// held aborts on wasm32, with no panic message to compare.
pub fn mutex_mapped(data: &[u8], out: &mut Vec<u8>) {
	let mutex = Mutex::new(Tally::default());
	let number = Mutex::new(0u32);
	let locked = |out: &mut Vec<u8>| {
		let tally = matches!(mutex.try_lock(), Err(TryLockError::WouldBlock));
		let number = matches!(number.try_lock(), Err(TryLockError::WouldBlock));
		out.push(u8::from(tally) << 1 | u8::from(number));
	};
	for &op in data {
		match op % 4 {
			// A number written through a guard mapped to it, and through that one's map.
			0 => {
				let mut n = MutexGuard::map(mutex.lock().unwrap(), |t| &mut t.n);
				*n += u64::from(op);
				locked(out);
				let mut again = MappedMutexGuard::map(n, |n| n);
				*again = again.wrapping_mul(3);
				drop(again);
				locked(out);
				out.push(mutex.lock().unwrap().n as u8);
				let mut whole = MutexGuard::map(number.lock().unwrap(), |n| n);
				*whole = whole.wrapping_mul(5) + u32::from(op);
				locked(out);
				drop(whole);
				out.push(*number.lock().unwrap() as u8);
			}
			// A `Vec`, which JS doesn't copy, pushed to and written whole, and the whole value
			// written through a guard mapped to it.
			1 => {
				let mut seen = MutexGuard::map(mutex.lock().unwrap(), |t| &mut t.seen);
				seen.push(op);
				if op >> 4 == 0xf {
					*seen = vec![op, op];
				}
				locked(out);
				drop(seen);
				out.extend_from_slice(&mutex.lock().unwrap().seen);
				if op >> 4 == 0xe {
					let mut whole = MutexGuard::map(mutex.lock().unwrap(), |t| t);
					*whole = Tally {
						n: u64::from(op),
						seen: vec![op],
					};
					drop(whole);
					out.push(mutex.lock().unwrap().n as u8);
				}
			}
			// Filtered, which gives the guard back where there is no part.
			2 => {
				match MutexGuard::filter_map(mutex.lock().unwrap(), |t| t.seen.last_mut()) {
					Ok(mut last) => *last = last.wrapping_add(op),
					Err(mut whole) => whole.seen.push(op),
				}
				locked(out);
				let n = MutexGuard::filter_map(mutex.lock().unwrap(), |t| Some(&mut t.n));
				match n.map(|n| MappedMutexGuard::filter_map(n, |n| (*n & 1 == 0).then_some(n))) {
					Ok(Ok(mut n)) => *n += 1,
					Ok(Err(mut n)) => {
						locked(out);
						*n += 2;
					}
					Err(_) => out.push(0xfe),
				}
				locked(out);
				out.push(mutex.lock().unwrap().n as u8);
				out.extend_from_slice(&mutex.lock().unwrap().seen);
			}
			// Tried while a mapped guard holds it, and once that guard drops.
			_ => {
				let n = MutexGuard::map(mutex.lock().unwrap(), |t| &mut t.n);
				locked(out);
				out.push(mutex.try_lock().is_err() as u8);
				drop(n);
				out.push(mutex.try_lock().map_or(0xfe, |t| t.seen.len() as u8));
				locked(out);
			}
		}
	}
	let last = mutex.into_inner().unwrap();
	out.push(last.n as u8);
	out.extend_from_slice(&last.seen);
	out.push(number.into_inner().unwrap() as u8);
}

/// Each lock's guards printed, plain and mapped, by `Debug` and `Display` and with options,
/// as std writes what each lends.
pub fn printed(data: &[u8], out: &mut Vec<u8>) {
	let text: String = data.iter().map(|b| char::from(b'a' + b % 26)).collect();
	let mutex = Mutex::new((data.len() as u8, text.clone()));
	let lock = RwLock::new((data.to_vec(), text));
	{
		let guard = mutex.lock().unwrap();
		out.extend_from_slice(format!("{guard:?} {guard:#?}").as_bytes());
		let n = MutexGuard::map(guard, |t| &mut t.0);
		out.extend_from_slice(format!("{n} {n:?} {n:>4} {n:04}").as_bytes());
		let n = MappedMutexGuard::map(n, |n| n);
		out.extend_from_slice(format!("[{n:<3}]").as_bytes());
	}
	let text = MutexGuard::map(mutex.lock().unwrap(), |t| &mut t.1);
	out.extend_from_slice(format!("{text} {text:?} [{text:^9}]").as_bytes());
	drop(text);
	{
		let read = lock.read().unwrap();
		out.extend_from_slice(format!("{read:?}").as_bytes());
		let bytes = RwLockReadGuard::map(read, |t| &t.0);
		out.extend_from_slice(format!("{bytes:?} {bytes:#?}").as_bytes());
		let text = RwLockReadGuard::map(lock.read().unwrap(), |t| &t.1);
		let first = MappedRwLockReadGuard::map(text, |t| t.get(..1).unwrap_or("-"));
		out.extend_from_slice(format!("{first} {first:?} {first:>3}").as_bytes());
	}
	let write = lock.write().unwrap();
	out.extend_from_slice(format!("{write:?}").as_bytes());
	let mut text = RwLockWriteGuard::map(write, |t| &mut t.1);
	text.push('!');
	out.extend_from_slice(format!("{text} {text:?}").as_bytes());
	let text = MappedRwLockWriteGuard::map(text, |t| t);
	out.extend_from_slice(format!("{text:>12}").as_bytes());
}

/// Each byte an operation on read-write locks through write guards downgraded, which keep
/// the lock read, and guards mapped to the whole of the bytes a lock holds, its value the
/// operand; each puts what it read through them and whether each lock is still held.
pub fn downgraded(data: &[u8], out: &mut Vec<u8>) {
	let lock = RwLock::new(Tally::default());
	let number = RwLock::new(0u32);
	let quad = RwLock::new([1u8, 2, 3, 4]);
	let mutex = Mutex::new([5u8, 6, 7, 8]);
	for &op in data {
		match op % 4 {
			// Written, then downgraded, so a write still fails while a read shares it.
			0 => {
				let mut write = lock.write().unwrap();
				write.n += u64::from(op);
				write.seen.push(op);
				let read = RwLockWriteGuard::downgrade(write);
				out.push(held(&lock));
				out.push(lock.try_read().map_or(0xfe, |t| t.seen.len() as u8));
				out.push(read.n as u8);
				drop(read);
				out.push(held(&lock));
			}
			// A number JS copies, downgraded while another read waits, and released as the
			// last read drops in either order.
			1 => {
				let mut write = number.write().unwrap();
				*write += u32::from(op);
				let read = RwLockWriteGuard::downgrade(write);
				let other = number.read().unwrap();
				out.extend_from_slice(&[*read as u8 ^ *other as u8, held(&number)]);
				if op & 0x10 == 0 {
					drop(read);
					out.push(held(&number));
					drop(other);
				} else {
					drop(other);
					out.push(held(&number));
					drop(read);
				}
				out.push(held(&number));
			}
			// The whole of the bytes a lock holds, written through a write guard mapped to
			// them and that one's map, written whole, and read through a read guard's map.
			2 => {
				let mut bytes = RwLockWriteGuard::map(quad.write().unwrap(), |q| q);
				bytes[usize::from(op & 3)] ^= op;
				out.push(held(&quad));
				let mut again = MappedRwLockWriteGuard::map(bytes, |q| q);
				if op >> 4 == 0xf {
					*again = [op; 4];
				}
				drop(again);
				out.push(held(&quad));
				let read = RwLockReadGuard::map(quad.read().unwrap(), |q| q);
				out.extend_from_slice(&*read);
				out.push(held(&quad));
				drop(read);
				let mut bytes = MutexGuard::map(mutex.lock().unwrap(), |q| q);
				bytes.rotate_left(1);
				if op >> 4 == 0xe {
					*bytes = [op; 4];
				}
				out.push(mutex.try_lock().is_err() as u8);
				drop(bytes);
				out.extend_from_slice(&*mutex.lock().unwrap());
			}
			// Downgraded, then mapped to a part, whose drop releases the lock.
			_ => {
				let write = lock.write().unwrap();
				let seen = RwLockReadGuard::map(RwLockWriteGuard::downgrade(write), |t| &t.seen);
				out.push(held(&lock));
				out.push(seen.len() as u8);
				drop(seen);
				out.push(held(&lock));
				let mut write = quad.write().unwrap();
				write[0] = write[0].wrapping_add(op);
				let read = RwLockWriteGuard::downgrade(write);
				out.extend_from_slice(&[read[0], held(&quad)]);
			}
		}
	}
	out.push(held(&lock));
	let last = lock.into_inner().unwrap();
	out.push(last.n as u8);
	out.extend_from_slice(&last.seen);
	out.push(number.into_inner().unwrap() as u8);
	out.extend_from_slice(&quad.into_inner().unwrap());
	out.extend_from_slice(&mutex.into_inner().unwrap());
}

/// Whether a read of `lock` would block, as 2, and whether a write would, as 1.
fn held<T>(lock: &RwLock<T>) -> u8 {
	let read = matches!(lock.try_read(), Err(TryLockError::WouldBlock));
	let write = matches!(lock.try_write(), Err(TryLockError::WouldBlock));
	u8::from(read) << 1 | u8::from(write)
}
