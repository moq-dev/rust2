//! rust2-ffi: export a crate's `pub` API as a C ABI and header, with no attributes.
//!
//! The binary is two programs, as rust2-ts is. Run directly, it is the CLI: it runs
//! cargo on the configured package with itself as `RUSTC_WRAPPER`. Run by cargo as that
//! wrapper, it is rustc, which collects the configured library's API after type checking
//! and writes the export.

#![feature(rustc_private)]

extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_infer;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;
extern crate rustc_trait_selection;

mod abi;
mod c;
mod census;
mod cli;
mod collect;
mod config;
mod cpp;
mod driver;
mod glue;
mod ir;
mod kotlin;
mod swift;

use std::process::ExitCode;

fn main() -> ExitCode {
	let args: Vec<String> = std::env::args().collect();
	if rust2_driver::rustc::wrapped(&args) {
		return driver::run(args);
	}
	cli::run(&args[1..])
}
