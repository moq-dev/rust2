//! The binary as a CLI: run cargo on a package with this binary as rustc.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// The command line every driver takes:
/// `--config <file> (--out <dir> | --census <path>) [--manifest-path <Cargo.toml>] [-- <cargo args>]`.
pub struct Args {
	/// The driver's config file, absolute.
	pub config: PathBuf,
	/// What to write.
	pub output: Output,
	/// The package's `Cargo.toml`, absolute: beside the config unless given.
	pub manifest: PathBuf,
	/// Passed on to `cargo check`.
	pub cargo: Vec<String>,
}

/// What a driver writes, absolute.
pub enum Output {
	/// Its output, into this directory.
	Dir(PathBuf),
	/// A census of what it refuses, to this file or directory, as the driver says, instead.
	Census(PathBuf),
}

impl Args {
	/// Parse `args`, without the program name, or `None` after printing `usage` for
	/// `--help`. An error ends with `usage`.
	pub fn parse(args: &[String], usage: &str) -> Result<Option<Self>, String> {
		let mut config = None;
		let mut out = None;
		let mut census = None;
		let mut manifest = None;
		let mut cargo: &[String] = &[];
		let mut rest = args;
		while let [arg, tail @ ..] = rest {
			rest = tail;
			match arg.as_str() {
				"--config" | "--out" | "--census" | "--manifest-path" => {
					let [value, tail @ ..] = rest else {
						return Err(format!("{arg} needs a value\n\n{usage}"));
					};
					rest = tail;
					let slot = match arg.as_str() {
						"--config" => &mut config,
						"--out" => &mut out,
						"--census" => &mut census,
						_ => &mut manifest,
					};
					*slot = Some(absolute(Path::new(value))?);
				}
				"--" => {
					cargo = rest;
					break;
				}
				"-h" | "--help" => {
					println!("{usage}");
					return Ok(None);
				}
				_ => return Err(format!("unexpected argument {arg}\n\n{usage}")),
			}
		}
		let config: PathBuf = config.ok_or_else(|| format!("--config is required\n\n{usage}"))?;
		let output = match (out, census) {
			(Some(out), None) => Output::Dir(out),
			(None, Some(census)) => Output::Census(census),
			_ => return Err(format!("one of --out and --census is required\n\n{usage}")),
		};
		let manifest = match manifest {
			Some(manifest) => manifest,
			None => config
				.parent()
				.expect("an absolute file path has a parent")
				.join("Cargo.toml"),
		};
		Ok(Some(Self {
			config,
			output,
			manifest,
			cargo: cargo.to_vec(),
		}))
	}
}

/// A `cargo check` of one package's library, with this executable as rustc.
pub struct Check<'a> {
	/// The driver's name, which names its target directory.
	pub tool: &'a str,
	/// The package's `Cargo.toml`, absolute.
	pub manifest: &'a Path,
	/// The cargo package whose library the driver handles.
	pub package: &'a str,
	/// The `Cargo.toml` of a package that depends on it, which cargo resolves it through
	/// instead of its own workspace, if any, absolute.
	pub root: Option<&'a Path>,
	/// The target to check for, or `None` for the host.
	pub target: Option<&'a str>,
	/// Passed on to `cargo check`, such as `--features`.
	pub args: &'a [String],
	/// The environment cargo runs in, which the wrapper reads its job from.
	pub env: &'a [(&'a str, &'a OsStr)],
}

impl Check<'_> {
	/// Run it, after cleaning the package: cargo can't tell the driver's output is
	/// stale, so the package always recompiles.
	pub fn run(&self) -> Result<ExitCode, String> {
		let exe = std::env::current_exe().map_err(|err| format!("finding this executable: {err}"))?;
		let target_dir = target_dir(self.tool, Some(self.manifest))?;
		let package_dir = self.manifest.parent().expect("a manifest path has a parent");
		let cargo = |subcommand: &str| {
			let mut cmd = cargo();
			// Cargo reads its config from where it runs, so run where the package is.
			cmd.current_dir(package_dir).envs(self.env.iter().copied());
			cmd.arg(subcommand)
				.arg("--manifest-path")
				.arg(self.root.unwrap_or(self.manifest))
				.arg("--package")
				.arg(self.package)
				.arg("--target-dir")
				.arg(&target_dir);
			if let Some(target) = self.target {
				cmd.arg("--target").arg(target);
			}
			cmd
		};
		run_cargo(cargo("clean").arg("--quiet").args(resolution(self.args)))?;
		// Its own, since runs may share a target directory.
		let ran = target_dir.join(format!("{}-{}.ran", self.tool, std::process::id()));
		match std::fs::remove_file(&ran) {
			Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
				return Err(format!("removing {}: {err}", ran.display()));
			}
			_ => {}
		}
		let status = cargo("check")
			.arg("--lib")
			.args(self.args)
			.env("RUSTC_WRAPPER", &exe)
			.env(crate::rustc::RAN, &ran)
			.status()
			.map_err(|err| format!("running cargo: {err}"))?;
		if !status.success() {
			return Ok(ExitCode::FAILURE);
		}
		// A library the wrapper never recognized would otherwise succeed with nothing written.
		let compiled = ran.exists();
		let _ = std::fs::remove_file(&ran);
		if !compiled {
			return Err(format!("cargo compiled no library of package {}", self.package));
		}
		Ok(ExitCode::SUCCESS)
	}
}

/// Where `tool` builds, absolute: `CARGO_TARGET_DIR`, or its own directory in the target
/// directory of the workspace `manifest`, or the current directory, belongs to.
pub fn target_dir(tool: &str, manifest: Option<&Path>) -> Result<PathBuf, String> {
	match std::env::var_os("CARGO_TARGET_DIR") {
		// Absolute, as the caller meant it: cargo runs in the package's directory.
		Some(dir) => absolute(Path::new(&dir)),
		// Apart from the workspace's own builds, which a different rustc would invalidate.
		None => Ok(workspace_root(manifest)?.join("target").join(tool)),
	}
}

fn absolute(path: &Path) -> Result<PathBuf, String> {
	std::path::absolute(path).map_err(|err| format!("resolving {}: {err}", path.display()))
}

fn cargo() -> Command {
	Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

fn run_cargo(cmd: &mut Command) -> Result<(), String> {
	let status = cmd.status().map_err(|err| format!("running cargo: {err}"))?;
	match status.success() {
		true => Ok(()),
		false => Err(format!("{cmd:?} failed")),
	}
}

/// The cargo args that pick how the workspace resolves, which `cargo clean` resolves it
/// by too: `--config`, and the lockfile and network flags.
fn resolution(args: &[String]) -> Vec<&str> {
	let mut picked = Vec::new();
	let mut args = args.iter().map(String::as_str);
	while let Some(arg) = args.next() {
		match arg {
			"--locked" | "--frozen" | "--offline" => picked.push(arg),
			"--config" => picked.extend([Some(arg), args.next()].into_iter().flatten()),
			_ if arg.starts_with("--config=") => picked.push(arg),
			_ => {}
		}
	}
	picked
}

/// The directory of the workspace `manifest`, or the current directory, belongs to.
pub fn workspace_root(manifest: Option<&Path>) -> Result<PathBuf, String> {
	let mut cmd = cargo();
	cmd.args(["locate-project", "--workspace", "--message-format", "plain"]);
	if let Some(manifest) = manifest {
		cmd.arg("--manifest-path").arg(manifest);
	}
	let output = cmd.output().map_err(|err| format!("running cargo: {err}"))?;
	if !output.status.success() {
		return Err(String::from_utf8_lossy(&output.stderr).into_owned());
	}
	let root = String::from_utf8(output.stdout).map_err(|_| "cargo printed a non-UTF-8 path".to_string())?;
	Ok(Path::new(root.trim())
		.parent()
		.expect("a manifest path has a parent")
		.to_path_buf())
}
