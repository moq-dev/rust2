//! The crate's `entry` as a wasm32 module: its harness for the native side of the
//! differential test, or the benchmark's workloads for its WASM route.
//!
//! The host writes a case into `input`, calls `run`, and reads `output`. A panic traps,
//! after its message is saved for `message`, and an abort traps without one, as std's
//! `rtabort!` runs no panic hook; either way the host starts a fresh instance.

use std::sync::{Mutex, Once};

static INPUT: Mutex<Vec<u8>> = Mutex::new(Vec::new());
static OUTPUT: Mutex<Vec<u8>> = Mutex::new(Vec::new());
static MESSAGE: Mutex<Option<String>> = Mutex::new(None);
static HOOK: Once = Once::new();

/// Size the input buffer to `len` bytes and return where to write them.
#[unsafe(no_mangle)]
pub extern "C" fn input(len: usize) -> *mut u8 {
	// The default hook's message, without its location, which translation can't match.
	HOOK.call_once(|| {
		std::panic::set_hook(Box::new(|info| {
			*MESSAGE.lock().unwrap() = Some(info.payload_as_str().unwrap_or("Box<dyn Any>").to_string());
		}))
	});
	let mut input = INPUT.lock().unwrap();
	input.clear();
	input.resize(len, 0);
	input.as_mut_ptr()
}

/// Run the entry on the input, returning the output's length.
#[unsafe(no_mangle)]
pub extern "C" fn run() -> usize {
	let output = crate::entry(&INPUT.lock().unwrap());
	let len = output.len();
	*OUTPUT.lock().unwrap() = output;
	len
}

/// Where the last run's output is.
#[unsafe(no_mangle)]
pub extern "C" fn output() -> *const u8 {
	OUTPUT.lock().unwrap().as_ptr()
}

/// Where the panic's message is, as UTF-8, or null if no panic ran the hook, as after an
/// abort; its length is `message_len`.
#[unsafe(no_mangle)]
pub extern "C" fn message() -> *const u8 {
	MESSAGE.lock().unwrap().as_ref().map_or(std::ptr::null(), |message| message.as_ptr())
}

/// The length of the panic's message.
#[unsafe(no_mangle)]
pub extern "C" fn message_len() -> usize {
	MESSAGE.lock().unwrap().as_ref().map_or(0, String::len)
}
