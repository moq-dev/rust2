//! Calls into `bytes`, `hashbrown`, `rand`, and `tracing`, which translation maps itself,
//! and into `hashlink`, which translates over `hashbrown`, so this config binds and leaves
//! out nothing.
//!
//! `harness::run` is the entry point both sides of the differential test run.

mod buf;
pub mod harness;
mod maps;
#[cfg(target_arch = "wasm32")]
#[path = "../../wasm.rs"]
mod wasm;
#[cfg(target_arch = "wasm32")]
use harness::run as entry;

/// The native side's source for `rand::rng()` on wasm32, which has no OS to seed from:
/// xorshift64*, so a failure reproduces. The translated side reads `crypto` instead.
///
/// # Safety
///
/// `dest` is valid for writes of `len` bytes, as getrandom's custom backend promises.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
unsafe extern "Rust" fn __getrandom_v03_custom(dest: *mut u8, len: usize) -> Result<(), getrandom::Error> {
	use std::sync::atomic::{AtomicU64, Ordering};
	static STATE: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);
	for i in 0..len {
		let mut x = STATE.load(Ordering::Relaxed);
		x ^= x >> 12;
		x ^= x << 25;
		x ^= x >> 27;
		STATE.store(x, Ordering::Relaxed);
		// SAFETY: `i < len`, within what the caller gave.
		unsafe { dest.add(i).write((x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 56) as u8) };
	}
	Ok(())
}
