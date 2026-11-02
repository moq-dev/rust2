//! rust2-ts: translate a documented Rust subset to readable TypeScript.
//!
//! The binary is two programs. Run directly, it is the CLI: it runs cargo on the
//! configured package with itself as `RUSTC_WRAPPER`. Run by cargo as that wrapper, it
//! is rustc, which translates the configured crate after type and borrow checking and
//! compiles every other crate as rustc would.

#![feature(rustc_private)]

extern crate rustc_abi;
extern crate rustc_ast;
extern crate rustc_attr_ir;
extern crate rustc_const_eval;
extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_hashes;
extern crate rustc_hir;
extern crate rustc_index;
extern crate rustc_infer;
extern crate rustc_interface;
extern crate rustc_metadata;
extern crate rustc_middle;
extern crate rustc_mir_dataflow;
extern crate rustc_span;
extern crate rustc_target;
extern crate rustc_trait_selection;

mod browser;
mod census;
mod check;
mod cli;
mod config;
mod driver;
mod lower;
mod print;
mod program;
mod ts;

use std::process::ExitCode;

fn main() -> ExitCode {
	let args: Vec<String> = std::env::args().collect();
	if rust2_driver::rustc::wrapped(&args) {
		return driver::run(args);
	}
	cli::run(&args[1..])
}
