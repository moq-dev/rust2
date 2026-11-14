//! A file whose lines end in CRLF.

/// Kept: app calls it.
pub fn spaced() -> u32 {
	9
}

/// Narrowed, unreached: its offset on disk is past every CRLF above.
pub fn after_crlf() -> u32 {
	10
}
