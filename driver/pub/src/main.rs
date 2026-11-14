//! rust2-pub: audit which `pub` items another crate needs, and validate narrowing the
//! rest to `pub(crate)`, without editing the source.
//!
//! The binary is two programs, as the other drivers are. Run directly, it is the CLI: it
//! runs cargo on the workspaces with itself as `RUSTC_WRAPPER`. Run by cargo as that
//! wrapper, it is rustc, which writes each local crate's definitions and references
//! after type checking.

#![feature(rustc_private)]

extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_hir_pretty;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_span;
extern crate rustc_structures;

mod analyze;
mod cargo;
mod cli;
mod collect;
mod facts;
mod report;
mod validate;

use std::process::ExitCode;

fn main() -> ExitCode {
	let args: Vec<String> = std::env::args().collect();
	if rust2_driver::rustc::wrapped(&args) {
		return collect::run(args);
	}
	cli::run(&args[1..])
}
