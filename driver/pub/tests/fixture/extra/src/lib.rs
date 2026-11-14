//! A library app uses, which facade exposes whole with `pub extern crate`.

/// Kept: app calls it.
pub fn used() -> u32 {
	7
}

/// External: facade exposes this crate, so crates outside the audit may call it.
pub fn exposed() -> u32 {
	8
}

/// External: an exported macro, whose `pub` crates outside the audit get wherever they call
/// it, though in the audit only app's binary does.
#[macro_export]
macro_rules! make {
	($name:ident) => {
		pub fn $name() -> u32 {
			14
		}
	};
}
