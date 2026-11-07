//! A part of the language the driver translates, exercised against native Rust: what
//! each of its modules says.

mod fmt;
mod generics;
#[path = "../../harness.rs"]
pub mod harness;
mod shapes;
#[cfg(target_arch = "wasm32")]
#[path = "../../../wasm.rs"]
mod wasm;

use harness::Case;

/// The native side's entry: [`harness::run`], except that a case starting with 0xff prints
/// a [`fmt::Fault`], which the translated side prints from TypeScript in `caller.ts`.
pub fn entry(input: &[u8]) -> Vec<u8> {
	match input.split_first() {
		Some((0xff, data)) => fmt::printed(data),
		_ => harness::run(input),
	}
}

/// Each case's name and what it runs. The first two bytes, big-endian, pick one modulo
/// their count, so a case added at the end moves no other.
fn cases() -> Vec<(&'static str, Case)> {
	vec![
		("fmt::ints", fmt::ints),
		("fmt::strings", |_, data, out| fmt::strings(data, out)),
		("fmt::derived", |_, data, out| fmt::derived(data, out)),
		("fmt::written", |_, data, out| fmt::written(data, out)),
		("fmt::panics", fmt::panics),
		("fmt::counts", |_, data, out| fmt::counts(data, out)),
		("fmt::rounding", |_, _, out| fmt::rounding(out)),
		("fmt::wrapped", |_, data, out| fmt::wrapped(data, out)),
		("fmt::writes", |_, data, out| fmt::writes(data, out)),
		("fmt::collections", |_, data, out| fmt::collections(data, out)),
		("fmt::iters", |_, data, out| fmt::iters(data, out)),
		("fmt::enums", |_, data, out| fmt::enums(data, out)),
		("fmt::objects", |_, data, out| fmt::objects(data, out)),
		("generics::methods", |_, data, out| generics::methods(data, out)),
		("generics::runs", |_, data, out| generics::runs(data, out)),
		("shapes::copies", |_, data, out| shapes::copies(data, out)),
		("shapes::loops", |_, data, out| shapes::loops(data, out)),
		("shapes::patterns", |_, data, out| shapes::patterns(data, out)),
		("shapes::closures", |_, data, out| shapes::closures(data, out)),
		("shapes::discriminants", |_, data, out| shapes::discriminants(data, out)),
		("shapes::library", |_, data, out| shapes::library(data, out)),
		("shapes::order", |_, data, out| shapes::order(data, out)),
		("shapes::flows", |_, data, out| shapes::flows(data, out)),
		("shapes::uninhabited", |_, data, out| shapes::uninhabited(data, out)),
		("fmt::adaptors", |_, data, out| fmt::adaptors(data, out)),
		("fmt::sources", |_, data, out| fmt::sources(data, out)),
		("fmt::sets", |_, data, out| fmt::sets(data, out)),
		("fmt::calls", |_, data, out| fmt::calls(data, out)),
		("fmt::cells", |_, data, out| fmt::cells(data, out)),
		("fmt::searches", |_, data, out| fmt::searches(data, out)),
		("generics::units", |_, data, out| generics::units(data, out)),
		("generics::projections", |_, data, out| generics::projections(data, out)),
		("generics::projected_cycles", |_, data, out| {
			generics::projected_cycles(data, out)
		}),
	]
}
