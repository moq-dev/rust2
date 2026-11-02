//! rust2-go: translate a Rust subset to pure Go, with no cgo.
//!
//! The binary is two programs, as rust2-ts is. Run directly, it is the CLI: it runs
//! cargo on the configured package with itself as `RUSTC_WRAPPER`. Run by cargo as that
//! wrapper, it is rustc, which translates the configured crate after type and borrow
//! checking and compiles every other crate as rustc would.

#![feature(rustc_private)]

extern crate rustc_abi;
extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_index;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

mod cli;
mod config;
mod driver;
mod lower;

use std::process::ExitCode;

fn main() -> ExitCode {
	let args: Vec<String> = std::env::args().collect();
	if rust2_driver::rustc::wrapped(&args) {
		return driver::run(args);
	}
	cli::run(&args[1..])
}
