//! The workspaces to audit, as `cargo metadata` describes them, and the `cargo check` of
//! them with this binary as rustc.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;

use crate::facts::{self, Unit};

/// One workspace, and the cargo arguments that select what of it to check.
pub struct Workspace {
	pub manifest: PathBuf,
	pub root: PathBuf,
	/// Where the audit builds, apart from the workspace's own builds.
	pub target: PathBuf,
	/// Every package with source on disk: its members and their path dependencies.
	pub local: BTreeMap<PathBuf, Package>,
}

/// A package with source on disk.
#[derive(Clone)]
pub struct Package {
	/// Cargo's id for it, which `cargo clean -p` takes.
	pub id: String,
	/// Whether `cargo publish` may publish it.
	pub publishable: bool,
	/// Whether it is a member of the workspace.
	pub member: bool,
	/// Whether it has a binary, which the audit's packages are consumed by running.
	pub bin: bool,
	/// The directories of the local packages that depend on it other than to test it.
	pub dependents: BTreeSet<PathBuf>,
}

/// Every workspace's local packages, each described once: a package two workspaces share
/// is a member of either, and its dependents are those of both.
pub fn packages(workspaces: &[Workspace]) -> BTreeMap<&Path, Package> {
	let mut packages: BTreeMap<&Path, Package> = BTreeMap::new();
	for (dir, package) in workspaces.iter().flat_map(|w| &w.local) {
		match packages.entry(dir.as_path()) {
			std::collections::btree_map::Entry::Vacant(entry) => {
				entry.insert(package.clone());
			}
			std::collections::btree_map::Entry::Occupied(mut entry) => {
				let joined = entry.get_mut();
				joined.member |= package.member;
				joined.bin |= package.bin;
				joined.publishable |= package.publishable;
				joined.dependents.extend(package.dependents.iter().cloned());
			}
		}
	}
	packages
}

#[derive(Deserialize)]
struct Metadata {
	packages: Vec<MetaPackage>,
	workspace_members: Vec<String>,
	workspace_root: PathBuf,
	target_directory: PathBuf,
	resolve: Option<Resolve>,
}

#[derive(Deserialize)]
struct MetaPackage {
	id: String,
	manifest_path: PathBuf,
	source: Option<String>,
	publish: Option<Vec<String>>,
	targets: Vec<Target>,
}

#[derive(Deserialize)]
struct Target {
	kind: Vec<String>,
}

#[derive(Deserialize)]
struct Resolve {
	nodes: Vec<Node>,
}

#[derive(Deserialize)]
struct Node {
	id: String,
	deps: Vec<Dep>,
}

#[derive(Deserialize)]
struct Dep {
	pkg: String,
	dep_kinds: Vec<DepKind>,
}

#[derive(Deserialize)]
struct DepKind {
	/// `dev`, `build`, or none for a normal dependency.
	kind: Option<String>,
}

impl Workspace {
	/// The workspace `manifest` belongs to, resolved as `args` resolve it.
	pub fn load(manifest: &Path, args: &[String]) -> Result<Self, String> {
		let manifest =
			&std::fs::canonicalize(manifest).map_err(|err| format!("resolving {}: {err}", manifest.display()))?;
		let output = cargo()
			// From where the checks run, so cargo reads the same config.
			.current_dir(manifest.parent().expect("a manifest has a directory"))
			.args(["metadata", "--format-version", "1", "--manifest-path"])
			.arg(manifest)
			.args(metadata_args(args))
			.stderr(Stdio::inherit())
			.output()
			.map_err(|err| format!("running cargo metadata: {err}"))?;
		if !output.status.success() {
			return Err(format!("cargo metadata failed for {}", manifest.display()));
		}
		let meta: Metadata =
			serde_json::from_slice(&output.stdout).map_err(|err| format!("parsing cargo metadata: {err}"))?;
		let dir = |p: &MetaPackage| {
			let dir = p.manifest_path.parent().expect("a manifest has a directory");
			std::fs::canonicalize(dir).map_err(|err| format!("resolving {}: {err}", dir.display()))
		};
		let mut local = BTreeMap::new();
		let mut dirs = BTreeMap::new();
		for package in &meta.packages {
			if package.source.is_some() {
				continue;
			}
			let path = dir(package)?;
			dirs.insert(package.id.clone(), path.clone());
			local.insert(
				path,
				Package {
					id: package.id.clone(),
					publishable: package.publish.as_ref().is_none_or(|registries| !registries.is_empty()),
					member: meta.workspace_members.contains(&package.id),
					bin: package.targets.iter().any(|t| t.kind.iter().any(|k| k == "bin")),
					dependents: BTreeSet::new(),
				},
			);
		}
		for node in meta.resolve.iter().flat_map(|r| &r.nodes) {
			let Some(from) = dirs.get(&node.id) else { continue };
			for dep in &node.deps {
				// A library only tests depend on is what they test, such as a fixture,
				// whose API is the point.
				let tests = dep.dep_kinds.iter().all(|k| k.kind.as_deref() == Some("dev"));
				if let Some(to) = dirs.get(&dep.pkg)
					&& to != from
					&& !tests
				{
					local
						.get_mut(to)
						.expect("a local package")
						.dependents
						.insert(from.clone());
				}
			}
		}
		let root = std::fs::canonicalize(&meta.workspace_root)
			.map_err(|err| format!("resolving {}: {err}", meta.workspace_root.display()))?;
		Ok(Self {
			manifest: manifest.to_path_buf(),
			root,
			target: meta.target_directory.join("rust2-pub"),
			local,
		})
	}

	/// Check the workspace with this binary as rustc, writing each local crate's
	/// [`Unit`] into `facts` and reading the source with `edits`, if given, narrowed.
	/// Every local package recompiles: cargo can't see the edits.
	pub fn check(&self, args: &[String], facts: &Path, local: &Path, edits: Option<&Path>) -> Result<Check, String> {
		let exe = std::env::current_exe().map_err(|err| format!("finding this executable: {err}"))?;
		let mut clean = self.cargo("clean");
		clean.args(resolution_args(args));
		for package in self.local.values() {
			clean.arg("--package").arg(&package.id);
		}
		// Show what it says only when it fails, which `--quiet` would hide.
		let output = clean.output().map_err(|err| format!("running cargo clean: {err}"))?;
		if !output.status.success() {
			return Err(format!(
				"cargo clean failed in {}: {}",
				self.root.display(),
				String::from_utf8_lossy(&output.stderr).trim()
			));
		}
		let mut check = self.cargo("check");
		check
			.args(["--message-format=json", "--keep-going"])
			.args(args)
			.env("RUSTC_WRAPPER", &exe)
			.env_remove("RUSTC_WORKSPACE_WRAPPER")
			.env(facts::env::FACTS, facts)
			.env(facts::env::LOCAL, local)
			.stdout(Stdio::piped());
		match edits {
			Some(edits) => check.env(facts::env::EDITS, edits),
			None => check.env_remove(facts::env::EDITS),
		};
		let output = check
			.spawn()
			.and_then(|child| child.wait_with_output())
			.map_err(|err| format!("running cargo check: {err}"))?;
		let ids: BTreeSet<&str> = self.local.values().map(|p| p.id.as_str()).collect();
		let mut diagnostics = Vec::new();
		let mut compiled = 0;
		for line in output.stdout.split(|&b| b == b'\n') {
			let Ok(message) = serde_json::from_slice::<Message>(line) else {
				continue;
			};
			match message.reason.as_str() {
				"compiler-message" => diagnostics.extend(message.message),
				"compiler-artifact" if message.package_id.as_deref().is_some_and(|id| ids.contains(id)) => {
					// Its facts would be missing, or not of the narrowed source.
					if message.fresh == Some(true) {
						return Err(format!(
							"cargo reused a build of {} rather than compile it with this binary as rustc",
							message.package_id.unwrap_or_default()
						));
					}
					compiled += 1;
				}
				_ => {}
			}
		}
		Ok(Check {
			success: output.status.success(),
			diagnostics,
			compiled,
		})
	}

	fn cargo(&self, subcommand: &str) -> Command {
		let mut cmd = cargo();
		// Cargo reads its config from where it runs: run where the manifest is, as
		// `cargo metadata` did.
		cmd.current_dir(self.manifest.parent().expect("a manifest has a directory"));
		// Cargo sets these for build scripts alone, which the wrapper tells its probes by.
		for (key, _) in std::env::vars_os() {
			if key.to_str().is_some_and(|key| key.starts_with("CARGO_CFG_")) {
				cmd.env_remove(key);
			}
		}
		cmd.arg(subcommand)
			.arg("--manifest-path")
			.arg(&self.manifest)
			.arg("--target-dir")
			.arg(&self.target);
		cmd
	}
}

/// What a `cargo check` printed.
pub struct Check {
	pub success: bool,
	pub diagnostics: Vec<Diagnostic>,
	/// How many local crates cargo says it compiled, each of which writes a [`Unit`].
	pub compiled: usize,
}

#[derive(Deserialize)]
struct Message {
	reason: String,
	message: Option<Diagnostic>,
	package_id: Option<String>,
	fresh: Option<bool>,
}

/// A rustc diagnostic, as its JSON gives it.
#[derive(Deserialize, Clone)]
pub struct Diagnostic {
	pub message: String,
	pub level: String,
	pub code: Option<Code>,
	pub spans: Vec<DiagnosticSpan>,
	pub children: Vec<Diagnostic>,
	pub rendered: Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct Code {
	pub code: String,
}

#[derive(Deserialize, Clone)]
pub struct DiagnosticSpan {
	pub file_name: PathBuf,
	pub byte_start: u32,
	pub byte_end: u32,
	pub expansion: Option<Box<Expansion>>,
}

#[derive(Deserialize, Clone)]
pub struct Expansion {
	pub span: DiagnosticSpan,
	pub def_site_span: DiagnosticSpan,
}

/// Every [`Unit`] in `dir`.
pub fn units(dir: &Path) -> Result<Vec<Unit>, String> {
	let mut units = Vec::new();
	let entries = std::fs::read_dir(dir).map_err(|err| format!("reading {}: {err}", dir.display()))?;
	for entry in entries {
		let path = entry.map_err(|err| format!("reading {}: {err}", dir.display()))?.path();
		let text = std::fs::read(&path).map_err(|err| format!("reading {}: {err}", path.display()))?;
		units.push(serde_json::from_slice(&text).map_err(|err| format!("parsing {}: {err}", path.display()))?);
	}
	Ok(units)
}

fn cargo() -> Command {
	Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

/// The cargo args that pick how the workspace resolves: `--config`, and the lockfile and
/// network flags.
fn resolution_args(args: &[String]) -> Vec<&str> {
	pick(args, &["--locked", "--frozen", "--offline"], &["--config"])
}

/// The cargo args `cargo metadata` also takes: those that resolve, and the features.
fn metadata_args(args: &[String]) -> Vec<&str> {
	pick(
		args,
		&[
			"--locked",
			"--frozen",
			"--offline",
			"--all-features",
			"--no-default-features",
		],
		&["--config", "--features", "-F"],
	)
}

/// The flags of `args` in `flags`, and those in `valued` with their values.
fn pick<'a>(args: &'a [String], flags: &[&str], valued: &[&str]) -> Vec<&'a str> {
	let mut picked = Vec::new();
	let mut args = args.iter().map(String::as_str);
	while let Some(arg) = args.next() {
		if flags.contains(&arg) {
			picked.push(arg);
		} else if valued.contains(&arg) {
			picked.extend([Some(arg), args.next()].into_iter().flatten());
		} else if valued.iter().any(|v| arg.starts_with(&format!("{v}="))) {
			picked.push(arg);
		}
	}
	picked
}

#[cfg(test)]
mod tests {
	use std::collections::{BTreeMap, BTreeSet};
	use std::path::{Path, PathBuf};

	use super::{Package, Workspace, packages};

	fn workspace(member: bool, dependents: &[&str]) -> Workspace {
		let package = Package {
			id: "shared".into(),
			publishable: false,
			member,
			bin: false,
			dependents: dependents.iter().map(PathBuf::from).collect(),
		};
		Workspace {
			manifest: PathBuf::new(),
			root: PathBuf::new(),
			target: PathBuf::new(),
			local: BTreeMap::from([(PathBuf::from("/shared"), package)]),
		}
	}

	#[test]
	fn a_shared_package_joins_alike_in_either_order() {
		let own = workspace(true, &["/tool"]);
		let other = workspace(false, &["/translator"]);
		for order in [[&own, &other], [&other, &own]] {
			let workspaces = order.map(|w| Workspace {
				local: w.local.clone(),
				..workspace(false, &[])
			});
			let joined = &packages(&workspaces)[Path::new("/shared")];
			assert!(joined.member);
			assert_eq!(
				joined.dependents,
				BTreeSet::from([PathBuf::from("/tool"), PathBuf::from("/translator")])
			);
		}
	}
}
