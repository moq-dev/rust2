//! Each item says what the audit must find of it: narrowed, or kept public and why.

use std::sync::atomic::{AtomicU32, Ordering};

static DROPS: AtomicU32 = AtomicU32::new(0);

/// Kept: app calls it generically and through `dyn`.
pub trait Action {
	/// Kept: an item of a kept trait.
	fn run(&self) -> u32;
}

/// Kept: app builds one.
pub struct Live {
	/// Kept: app sets it.
	pub value: u32,
	/// Narrowed: only this crate reads it.
	pub hidden: u32,
}

impl Live {
	/// Kept: app calls it.
	pub fn new(value: u32) -> Self {
		Self { value, hidden: helper() }
	}
}

impl Action for Live {
	fn run(&self) -> u32 {
		self.value + self.hidden
	}
}

impl Drop for Live {
	fn drop(&mut self) {
		on_drop();
	}
}

/// Narrowed: only `Live`'s drop calls it.
pub fn on_drop() {
	DROPS.fetch_add(1, Ordering::Relaxed);
}

/// Kept: app reads how many dropped.
pub fn drops() -> u32 {
	DROPS.load(Ordering::Relaxed)
}

/// Narrowed, crate-only: only `Live::new` calls it.
pub fn helper() -> u32 {
	0
}

/// Kept: app calls it.
pub fn generic<T: Action>(x: &T) -> u32 {
	x.run()
}

/// Kept: app calls it.
pub fn dynamic(x: &dyn Action) -> u32 {
	x.run()
}

/// Narrowed, test-only: only this crate's unit test calls it.
pub fn unit_tested() -> u32 {
	11
}

/// Kept, test-only: an integration test, another crate, calls it.
pub fn integration_tested() -> u32 {
	12
}

/// Narrowed, unreached: a dead cycle.
pub fn cycle_a() {
	cycle_b();
}

/// Narrowed, unreached: a dead cycle.
pub fn cycle_b() {
	cycle_a();
}

macro_rules! define {
	($name:ident, $value:expr) => {
		/// Written once in the macro, for every expansion.
		pub fn $name() -> u32 {
			$value
		}
	};
}

// Kept: the token is shared, and app calls `expanded_used`.
define!(expanded_used, 5);
define!(expanded_dead, 6);

mod inner {
	/// Kept: app names it through the re-export.
	pub fn reexported() -> u32 {
		7
	}

	/// Narrowed: re-exported, but nothing names it.
	pub fn reexported_dead() -> u32 {
		8
	}

	/// Kept: app calls it through `prelude::*`.
	pub fn through_glob() -> u32 {
		22
	}

	/// Narrowed: `prelude::*` supplies it, but nothing names it.
	pub fn past_glob() -> u32 {
		23
	}

	/// Kept, unreached: `prelude::*` supplies it to app's dead code alone.
	pub fn past_live_glob() -> u32 {
		25
	}
}

/// Kept: app imports `reexported` through it.
pub use inner::reexported;
/// Narrowed: nothing names it.
pub use inner::reexported_dead;

/// Kept: app imports `globbed` through `prelude::*`.
pub mod prelude {
	/// Kept: app imports it.
	pub fn globbed() -> u32 {
		9
	}

	/// Kept: app calls it through the glob.
	pub use crate::inner::through_glob;
	/// Narrowed: app's glob supplies it, but app never names it.
	pub use crate::inner::past_glob;
	/// Kept, unreached: app's glob supplies it to dead code alone.
	pub use crate::inner::past_live_glob;
	/// Kept: app's glob supplies it, from outside the audit, to main.
	pub use std::mem::size_of as measure;
	/// Kept: app's glob supplies `via_nested_glob` through it.
	pub use crate::deeper::*;
}

mod deeper {
	/// Kept: app calls it through two globs.
	pub fn via_nested_glob() -> u32 {
		28
	}
}

/// Kept: app drains one generically.
pub trait Source {
	/// Kept: a kept trait's.
	type Item;
	/// Kept: a kept trait's.
	fn next(&mut self) -> Self::Item;
}

/// Kept: app constructs it.
pub struct Counter(pub u32);

/// Kept: the type app receives from `Counter::next`, and its field app reads, though app
/// never names it.
pub struct Ticket(pub u32);

impl Source for Counter {
	type Item = Ticket;

	fn next(&mut self) -> Ticket {
		self.0 += 1;
		Ticket(self.0)
	}
}

/// Kept: app calls `Outer::len`, which narrowing would redirect to the slice's `len`
/// through `Deref`, printing 3 rather than 99, though it would still compile.
pub struct Outer(Vec<u8>);

impl Outer {
	/// Kept: app calls it.
	pub fn new() -> Self {
		Self(vec![1, 2, 3])
	}

	/// Kept: app calls it, and only resolution, not compilation, shows that it must stay.
	pub fn len(&self) -> usize {
		99
	}

	/// Narrowed: nothing calls it.
	pub fn is_empty(&self) -> bool {
		false
	}
}

impl Default for Outer {
	fn default() -> Self {
		Self::new()
	}
}

impl std::ops::Deref for Outer {
	type Target = [u8];

	fn deref(&self) -> &[u8] {
		&self.0
	}
}

/// Restored: a public trait's associated type in an impl for a public type must be public,
/// which rustc enforces when this narrows.
pub struct Leaked;

/// Kept: app uses `Box<dyn Holder>`.
pub trait Holder {
	/// Kept: a kept trait's.
	type Held;
	/// Kept: a kept trait's.
	fn held(&self) -> u32;
}

/// Kept: app builds one.
pub struct Holds;

impl Holder for Holds {
	type Held = Leaked;

	fn held(&self) -> u32 {
		10
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn unit() {
		assert_eq!(super::unit_tested(), 11);
	}
}

/// External: facade re-exports it.
pub struct Exposed {
	/// External: a field of a type facade's API exposes.
	pub field: u32,
}

impl Exposed {
	/// External: a method of a type facade's API exposes, though nothing calls it.
	pub fn method(&self) -> u32 {
		self.field
	}
}

/// External: facade's `unused` returns it.
#[derive(Default)]
pub struct Returned {
	/// External: a field of a type facade's API exposes.
	pub field: u32,
}

/// Kept: app builds one from a default.
#[derive(Default)]
pub struct Settings {
	/// Kept: app sets it.
	pub named: u32,
	/// Kept: app's `..Default::default()` moves it, which it must see to.
	pub defaulted: u32,
}

/// External: facade re-exports it.
pub trait Shape {
	/// External: an item of an external trait, whose signature is that API too.
	fn area(&self) -> Area;
}

/// External: an external trait's method returns it.
#[derive(Default)]
pub struct Area {
	/// External: a field of a type an external trait exposes.
	pub value: u32,
}

impl Area {
	/// External: a method of a type an external trait exposes.
	pub fn doubled(&self) -> u32 {
		self.value * 2
	}
}

/// Kept: app names it. Its file's lines end in CRLF, which rustc reads as LF.
pub mod crlf;

/// Narrowed, unreached: a derive and its helper attribute make the item again,
/// unchanged, so its token is the one written here.
#[derive(Clone, Debug, Default)]
pub enum Derived {
	/// Narrowed, unreached.
	#[default]
	Unit,
	Field(u32),
}

mod hidden {
	/// Kept: app imports it under another name.
	pub mod m {
		/// Kept: app calls it through the alias.
		pub fn through_alias() -> u32 {
			19
		}
	}
}

/// Kept, cross-crate: app imports it, and calls through it.
pub use hidden::m as alias;

/// Kept, unreached: only app's dead code calls it, beside an import of a live function.
pub fn only_dead() -> u32 {
	21
}

mod outer {
	mod parts {
		/// External: facade re-exports it, through the glob below.
		pub struct Part;

		/// Narrowed: the glob supplies it to `size` alone, whose body is not API.
		pub const SIZE: u32 = 2;
	}

	/// External: facade's re-export of `Part` goes through it.
	pub use parts::*;
}

/// External: facade re-exports it.
pub use outer::Part;

/// Narrowed: nothing calls it.
pub fn size() -> u32 {
	outer::SIZE
}
