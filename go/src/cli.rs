//! The command line: run cargo on the configured package with this binary as rustc.

use std::process::ExitCode;

use rust2_driver::cargo::{Args, Check, Output};

use crate::config::{self, Config};

const USAGE: &str = "usage: rust2-go --config <go.toml> --out <dir> [--manifest-path <Cargo.toml>] [-- <cargo args>]

Translates the library of the package that go.toml names to a Go package in <dir>. The
package's Cargo.toml is beside go.toml unless --manifest-path says where. Cargo args,
such as --features, are passed to `cargo check`, which checks with the dev profile on the
host, with std checked from source.";

pub fn run(args: &[String]) -> ExitCode {
	match try_run(args) {
		Ok(code) => code,
		Err(err) => {
			eprintln!("rust2-go: {err}");
			ExitCode::FAILURE
		}
	}
}

fn try_run(args: &[String]) -> Result<ExitCode, String> {
	let Some(args) = Args::parse(args, USAGE)? else {
		return Ok(ExitCode::SUCCESS);
	};
	let config = Config::load(&args.config)?;
	let Output::Dir(out) = &args.output else {
		return Err(format!("--census is not supported\n\n{USAGE}"));
	};
	let mut cargo = profile_args();
	// The profile `PROFILE` sets, which cargo refuses to combine with one the cargo args pick.
	cargo.extend(["--profile".into(), "dev".into()]);
	cargo.extend(args.cargo.iter().cloned());
	Check {
		tool: "rust2-go",
		manifest: &args.manifest,
		package: &config.package,
		root: None,
		// The host's: Go's `int` and `uint` are 64 bits on every platform the output
		// targets, as the host's `usize` is.
		target: Some(rustc_session::config::host_tuple()),
		args: &cargo,
		env: &[
			(config::env::CONFIG, args.config.as_os_str()),
			(config::env::OUT, out.as_os_str()),
			("CARGO_UNSTABLE_PROFILE_RUSTFLAGS", "true".as_ref()),
		],
	}
	.run()
}

/// The cargo config of the profile every crate compiles with, one `--config` per line.
const PROFILE: &str = include_str!("profile.toml");

/// Cargo args that set [`PROFILE`] and check std from source.
fn profile_args() -> Vec<String> {
	let mut args = vec!["-Zbuild-std=core,alloc,std,panic_abort".to_string()];
	for line in PROFILE
		.lines()
		.filter(|line| !line.is_empty() && !line.starts_with('#'))
	{
		args.extend(["--config".to_string(), line.to_string()]);
	}
	args
}
