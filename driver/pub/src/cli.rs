//! The command line: check the workspaces with this binary as rustc, join what it
//! collects, and validate narrowing what no other crate needs.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::analyze::Audit;
use crate::cargo::Workspace;
use crate::report::{self, Scope};
use crate::validate::{self, Run};

const USAGE: &str = "usage: rust2-pub --out <dir> [--manifest-path <Cargo.toml>]... [-- <cargo args>]

Audits every `pub` written in the workspaces' local packages: which another crate names,
which only its own crate reaches, which only tests reach, and which nothing the
workspaces run reaches. It then narrows every `pub` no other crate names to `pub(crate)`
at once, in rustc's reading of the source alone, and checks the workspaces as cargo does,
restoring each `pub` rustc's errors point at, until they check with every name in the
source resolving as before. The source on disk is never changed.

Writes, into <dir>: report.json, every `pub` with what reaches it; report.txt, the
findings; and narrow.patch, the narrowing that checked.

The workspaces are those of each --manifest-path, ./Cargo.toml by default; give several
to audit crates they share as one. Cargo args select what to check, as for
`cargo check`; they default to --workspace --all-targets.

A library's API stays `pub` when crates or tools outside the audit may use it: what a
published library exports, and all of an unpublished library the audit does not run,
which only a package with a binary does, and the unpublished members it builds on. A
translator's fixture is such a library.";

pub fn run(args: &[String]) -> ExitCode {
	match try_run(args) {
		Ok(code) => code,
		Err(err) => {
			eprintln!("rust2-pub: {err}");
			ExitCode::FAILURE
		}
	}
}

struct Args {
	out: PathBuf,
	manifests: Vec<PathBuf>,
	cargo: Vec<String>,
}

fn parse(args: &[String]) -> Result<Option<Args>, String> {
	let mut out = None;
	let mut manifests = Vec::new();
	let mut cargo = None;
	let mut rest = args;
	while let [arg, tail @ ..] = rest {
		rest = tail;
		match arg.as_str() {
			"--out" | "--manifest-path" => {
				let [value, tail @ ..] = rest else {
					return Err(format!("{arg} needs a value\n\n{USAGE}"));
				};
				rest = tail;
				let value = std::path::absolute(value).map_err(|err| format!("resolving {value}: {err}"))?;
				match arg.as_str() {
					"--out" => out = Some(value),
					_ => manifests.push(value),
				}
			}
			"--" => {
				cargo = Some(rest.to_vec());
				break;
			}
			"-h" | "--help" => {
				println!("{USAGE}");
				return Ok(None);
			}
			_ => return Err(format!("unexpected argument {arg}\n\n{USAGE}")),
		}
	}
	let out = out.ok_or_else(|| format!("--out is required\n\n{USAGE}"))?;
	if manifests.is_empty() {
		manifests.push(std::path::absolute("Cargo.toml").map_err(|err| format!("resolving Cargo.toml: {err}"))?);
	}
	let cargo = cargo
		.filter(|c| !c.is_empty())
		.unwrap_or_else(|| vec!["--workspace".into(), "--all-targets".into()]);
	Ok(Some(Args { out, manifests, cargo }))
}

fn try_run(args: &[String]) -> Result<ExitCode, String> {
	let Some(args) = parse(args)? else {
		return Ok(ExitCode::SUCCESS);
	};
	let workspaces = args
		.manifests
		.iter()
		.map(|m| Workspace::load(m, &args.cargo))
		.collect::<Result<Vec<_>, _>>()?;
	std::fs::create_dir_all(&args.out).map_err(|err| format!("creating {}: {err}", args.out.display()))?;
	let local: Vec<&PathBuf> = workspaces.iter().flat_map(|w| w.local.keys()).collect();
	let local_path = args.out.join("local.json");
	std::fs::write(&local_path, serde_json::to_vec(&local).expect("paths serialize"))
		.map_err(|err| format!("writing {}: {err}", local_path.display()))?;
	let run = Run {
		workspaces: &workspaces,
		args: &args.cargo,
		out: &args.out,
		local: &local_path,
	};

	eprintln!("rust2-pub: collecting");
	let (success, diagnostics, original) = run.check("original", None)?;
	if !success {
		let rendered: Vec<&str> = diagnostics.iter().filter_map(|d| d.rendered.as_deref()).collect();
		return Err(format!(
			"the workspaces fail to check as they are:\n{}",
			rendered.join("")
		));
	}
	let audit = Audit::new(&workspaces, &original)?;
	let validation = validate::validate(&run, &audit, &original)?;

	let root = common_root(
		local
			.iter()
			.map(|p| p.as_path())
			.chain(workspaces.iter().map(|w| w.root.as_path())),
	);
	let scope = Scope {
		root: &root,
		workspaces: workspaces.iter().map(|w| w.manifest.as_path()).collect(),
		cargo_args: &args.cargo,
	};
	let summary = report::write(&args.out, &scope, &audit, &validation)?;
	println!("{summary}");
	println!(
		"{} `pub` narrowed together after {} checks; findings in {}, the narrowing in {}, relative to {}",
		validation.narrowed.len(),
		validation.rounds,
		args.out.join("report.txt").display(),
		args.out.join("narrow.patch").display(),
		root.display()
	);
	Ok(ExitCode::SUCCESS)
}

/// The deepest directory holding every path.
fn common_root<'a>(mut paths: impl Iterator<Item = &'a Path>) -> PathBuf {
	let Some(first) = paths.next() else {
		return PathBuf::from("/");
	};
	let mut root = first.to_path_buf();
	for path in paths {
		while !path.starts_with(&root) {
			if !root.pop() {
				break;
			}
		}
	}
	root
}
