use api::prelude::*;
use api::{Action, Holder, Source, alias, reexported};

mod embedded;
mod payload;

/// Its file's text, which narrowing the `pub` in it would change.
const EMBEDDED: &str = include_str!("embedded.rs");

fn drain<S: Source>(source: &mut S) -> S::Item {
	source.next()
}

macro_rules! labeled {
	($v:vis $name:ident) => {
		$v fn $name() -> &'static str {
			stringify!($v)
		}
	};
}

// Restored: the macro stringifies the token it is passed, so narrowing it would change
// what `label` returns.
labeled!(pub label);

macro_rules! visible {
	($v:vis fn $name:ident() -> $ret:ty $body:block) => {
		$v fn $name() -> $ret $body
	};
}

// Narrowed: the macro uses the token it is passed as the visibility alone.
visible!(pub fn passed_through() -> u32 { 15 });

/// Restored: its derive prints its visibility, which narrowing would change.
#[derive(derives::Label)]
pub struct Labeled;

macro_rules! make_label {
	($name:ident, $label:literal) => {
		impl $name {
			fn nested() -> &'static str {
				$label
			}
		}
	};
}

/// Restored: its derive hands its visibility to another macro, `make_label!`.
#[derive(derives::NestedLabel)]
pub struct NestedLabeled;

/// Restored: its attribute adds a function returning its visibility.
#[derives::tag]
pub fn tagged() -> u32 {
	16
}

/// Narrowed: its attribute returns it unchanged.
#[derives::keep]
pub fn kept_through() -> u32 {
	17
}

/// Restored: `column!` reads a column that narrowing the `pub` before it shifts.
pub fn at_column() -> u32 { column!() }

/// Narrowed: narrowing shifts no line.
pub fn at_line() -> u32 {
	line!()
}

macro_rules! hygienic {
	(($p:tt) fn $name:ident($x:ident)) => {
		$p fn $name() -> u32 {
			let $x = 1;
			{
				let x = 2;
				$x
			}
		}
	};
	(($($v:tt)*) fn $name:ident($x:ident)) => {
		$($v)* fn $name() -> u32 {
			let $x = 1;
			{
				let $x = 2;
				$x
			}
		}
	};
}

// Restored: narrowing picks the other arm, which prints alike but binds the caller's `x`
// both times, so `shadowed` would return 2 rather than 1.
hygienic!((pub) fn shadowed(x));

macro_rules! payload {
	(($p:tt) fn $name:ident) => {
		$p fn $name() -> &'static str {
			"pub fn payload() -> u32 {\n\t20\n}\n"
		}
	};
	(($($v:tt)*) fn $name:ident) => {
		$($v)* fn $name() -> &'static str {
			include_str!("payload.rs")
		}
	};
}

// Narrowed: narrowing picks the arm that embeds payload.rs, whose text is the other arm's
// literal, so long as payload.rs itself stays as it is.
payload!((pub) fn payload_text);

#[allow(dead_code, unused_imports)]
fn dead_import() -> u32 {
	use api::drops;
	api::only_dead() + past_live_glob()
}

// External: the token is in an exported macro of extra, whose callers outside the audit
// get it too.
extra::make!(made);

mod tools {
	/// Narrowed, crate-only: main calls it through a glob of its module.
	pub fn tool() -> u32 {
		24
	}
}

/// Narrowed, crate-only: main calls `tool` through it.
pub use tools::*;

mod listed {
	/// Narrowed, crate-only: main calls it through the list below.
	pub fn listed() -> u32 {
		26
	}

	/// Narrowed, unreached: listed beside it, but nothing calls it.
	pub fn listed_dead() -> u32 {
		27
	}
}

/// Narrowed, crate-only: main calls `listed` through it.
pub use listed::{listed, listed_dead};

/// Narrowed, crate-only: main names a variant through a glob of it.
pub enum Mode {
	On,
	Off,
}

/// Narrowed, crate-only: main names `On` through it.
pub use Mode::*;

use std::fmt::Display;

struct Shown;

impl Display for Shown {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", shown_part())
	}
}

/// Narrowed, crate-only: only `Shown`'s impl of an imported std trait calls it.
pub fn shown_part() -> u32 {
	29
}

fn main() {
	{
		let live = api::Live::new(42);
		println!("value {}", live.value);
		println!("generic {}", api::generic(&live));
		println!("dynamic {}", api::dynamic(&live));
		let built = api::Live::new(1);
		println!("run {}", (&built as &dyn Action).run());
	}
	println!("drops {}", api::drops());
	println!("expanded {}", api::expanded_used());
	println!("reexported {}", reexported());
	println!("globbed {}", globbed());
	println!("through glob {}", through_glob());
	println!("tool {} {}", tool(), matches!(On, Mode::Off));
	println!("listed {}", listed());
	println!("measure {} {}", measure::<u16>(), via_nested_glob());
	println!("shown {Shown}");
	let mut counter = api::Counter(0);
	println!("ticket {}", drain(&mut counter).0);
	println!("len {}", api::Outer::new().len());
	println!("held {}", api::Holds.held());
	let settings = api::Settings {
		named: 1,
		..Default::default()
	};
	println!("settings {}", settings.named);
	println!("label {}", label());
	println!("labeled {}", Labeled::label());
	println!("passed through {}", passed_through());
	println!("nested {}", NestedLabeled::nested());
	println!("tagged {} {}", tagged(), tagged_tag());
	println!("kept through {}", kept_through());
	println!("column {}", at_column());
	println!("line {}", at_line());
	println!("made {}", made());
	println!("shadowed {}", shadowed());
	println!("alias {}", alias::through_alias());
	println!("payload {} {:?}", payload::payload(), payload_text());
	println!("embedded {} {}", embedded::embedded(), EMBEDDED.len());
	println!("extra {}", extra::used());
	println!("crlf {}", api::crlf::spaced());
}
