//! A part of the language the driver translates, exercised against native Rust: what
//! each of its modules says.

#[path = "../../harness.rs"]
pub mod harness;
mod pointers;
#[cfg(target_arch = "wasm32")]
#[path = "../../../wasm.rs"]
mod wasm;
#[cfg(target_arch = "wasm32")]
use harness::run as entry;

use harness::Case;

/// Each case's name and what it runs. The first two bytes, big-endian, pick one modulo
/// their count, so a case added at the end moves no other.
fn cases() -> Vec<(&'static str, Case)> {
	vec![
		("pointers::copies", |_, data, out| pointers::copies(data, out)),
		("pointers::lists", |_, data, out| pointers::lists(data, out)),
		("pointers::nested", |_, data, out| pointers::nested(data, out)),
		("pointers::places", |_, data, out| pointers::places(data, out)),
		("pointers::sentinels", |_, data, out| pointers::sentinels(data, out)),
		("pointers::shared", |_, data, out| pointers::shared(data, out)),
		("pointers::unchecked", |_, data, out| pointers::unchecked(data, out)),
		("pointers::cells", |_, data, out| pointers::cells(data, out)),
		("pointers::rebound", |_, data, out| pointers::rebound(data, out)),
		("pointers::writers", |_, data, out| pointers::writers(data, out)),
		("pointers::made", |_, data, out| pointers::made(data, out)),
		("pointers::lapsed", |_, data, out| pointers::lapsed(data, out)),
		("pointers::segments", |_, data, out| pointers::segments(data, out)),
		("pointers::uninit_runs", |_, data, out| pointers::uninit_runs(data, out)),
		("pointers::beside_written", |_, data, out| {
			pointers::beside_written(data, out)
		}),
		("pointers::iterated_runs", |_, data, out| {
			pointers::iterated_runs(data, out)
		}),
		("pointers::waiters", |_, data, out| pointers::waiters(data, out)),
		("pointers::taken", |_, data, out| pointers::taken(data, out)),
		("pointers::bytes", |_, data, out| pointers::bytes(data, out)),
		("pointers::loaded", |_, data, out| pointers::loaded(data, out)),
		("pointers::reattached", |_, data, out| pointers::reattached(data, out)),
		("pointers::part_written", |_, data, out| {
			pointers::part_written(data, out)
		}),
		("pointers::aimed", |_, data, out| pointers::aimed(data, out)),
		("pointers::distinct", |_, data, out| pointers::distinct(data, out)),
		("pointers::raw_arcs", |_, data, out| pointers::raw_arcs(data, out)),
		("pointers::raw_places", |_, data, out| pointers::raw_places(data, out)),
		("pointers::arrays", |_, data, out| pointers::arrays(data, out)),
		("pointers::generic_unions", |_, data, out| {
			pointers::generic_unions(data, out)
		}),
		("pointers::raw_cells", |_, data, out| pointers::raw_cells(data, out)),
		("pointers::cell_writes", |_, data, out| pointers::cell_writes(data, out)),
	]
}

/// `UnsafeCell::get` and `raw_get` of a cell of a slice, which only a caller can give, as
/// translated code can't unsize a cell.
pub fn cell_slice(c: &std::cell::UnsafeCell<[u16]>, at: usize) -> u16 {
	unsafe { (*std::cell::UnsafeCell::raw_get(c))[at] ^ (*c.get())[0] }
}
