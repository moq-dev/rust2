//! The command line: run cargo on the configured package with this binary as rustc.

use std::path::Path;
use std::process::ExitCode;

use rust2_driver::cargo::{Args, Check, Output};

use crate::browser;
use crate::config::{self, Config};

/// Translation type-checks for this target: its 32-bit `usize` is what a JS number
/// holds exactly, and its `cfg`s are the browser's.
pub const TARGET: &str = "wasm32-unknown-unknown";

const USAGE: &str = "usage: rust2-ts --config <ts.toml> (--out <dir> | --census <file>) [--manifest-path <Cargo.toml>] [-- <cargo args>]

Translates the library of the package that ts.toml names to TypeScript in <dir>. The
package's Cargo.toml is beside ts.toml unless --manifest-path says where. Cargo args,
such as --features, are passed to `cargo check`, which checks with the dev profile, and
picks getrandom's and uuid's browser randomness for wasm32 as a binary would.

--census writes a Markdown report of every construct translation refuses in the library,
reached from each of its items as if each were a root, instead of translating it.";

pub fn run(args: &[String]) -> ExitCode {
	match try_run(args) {
		Ok(code) => code,
		Err(err) => {
			eprintln!("rust2-ts: {err}");
			ExitCode::FAILURE
		}
	}
}

fn try_run(args: &[String]) -> Result<ExitCode, String> {
	let Some(args) = Args::parse(args, USAGE)? else {
		return Ok(ExitCode::SUCCESS);
	};
	let config = Config::load(&args.config)?;
	let output = match &args.output {
		Output::Dir(out) => (config::env::OUT, out.as_os_str()),
		Output::Census(census) => (config::env::CENSUS, census.as_os_str()),
	};
	let sysroot = sysroot()?;
	// `cargo clean` reads the profiles too. The sysroot is absolute for the wrapper, which
	// cargo runs elsewhere, and empty for none. The wrapper's version names it, so cargo
	// must not cache that version by the executables alone, and must hash all of it into
	// `-C metadata`, which on a nightly it does only with `-Zseparate-nightlies`.
	let env = [
		(config::env::CONFIG, args.config.as_os_str()),
		output,
		("CARGO_UNSTABLE_PROFILE_RUSTFLAGS", "true".as_ref()),
		(config::env::SYSROOT, sysroot.as_deref().unwrap_or("").as_ref()),
		("CARGO_CACHE_RUSTC_INFO", "0".as_ref()),
		("CARGO_UNSTABLE_SEPARATE_NIGHTLIES", "true".as_ref()),
	];
	let target_dir = rust2_driver::cargo::target_dir("rust2-ts", Some(&args.manifest))?;
	let root = browser::root(&args.manifest, &config.package, &target_dir, &args.cargo)?;
	let mut cargo = profile_args(sysroot.is_none());
	// The profile `PROFILE` sets, which cargo refuses to combine with one the cargo args
	// pick.
	cargo.extend(["--profile".into(), "dev".into()]);
	cargo.extend(root.as_ref().map_or(args.cargo, |root| root.cargo.clone()));
	Check {
		tool: "rust2-ts",
		manifest: &args.manifest,
		package: &config.package,
		root: root.as_ref().map(|root| root.manifest.as_path()),
		target: Some(TARGET),
		args: &cargo,
		env: &env,
	}
	.run()
}

/// The cargo config of the profile every crate compiles with, one `--config` per line.
const PROFILE: &str = include_str!("profile.toml");

/// The std crates `-Zbuild-std` checks, which flake.nix's sysroot holds too.
const STD: &str = "-Zbuild-std=core,alloc,std,panic_abort";

/// Cargo args that set [`PROFILE`], and check std from source if `std`.
fn profile_args(std: bool) -> Vec<String> {
	let mut args = Vec::new();
	if std {
		args.push(STD.to_string());
	}
	for line in PROFILE
		.lines()
		.filter(|line| !line.is_empty() && !line.starts_with('#'))
	{
		args.extend(["--config".to_string(), line.to_string()]);
	}
	args
}

/// The sysroot of std checked with [`PROFILE`] for [`TARGET`], which rustc takes for the
/// target's crates instead of a `-Zbuild-std` in every target directory, if set:
/// absolute.
pub fn sysroot() -> Result<Option<String>, String> {
	let dir = match std::env::var(config::env::SYSROOT) {
		Ok(dir) if !dir.is_empty() => dir,
		Err(std::env::VarError::NotUnicode(_)) => return Err(format!("{} is not UTF-8", config::env::SYSROOT)),
		_ => return Ok(None),
	};
	let dir = std::path::absolute(&dir)
		.map_err(|err| format!("resolving {}: {err}", config::env::SYSROOT))?
		.into_os_string()
		.into_string()
		.expect("a UTF-8 path resolves to a UTF-8 path");
	let lib = Path::new(&dir).join("lib/rustlib").join(TARGET).join("lib");
	if !lib.is_dir() {
		return Err(format!(
			"{} is {dir}, which holds no std for {TARGET} in {}",
			config::env::SYSROOT,
			lib.display()
		));
	}
	Ok(Some(dir))
}
