//! The command line: run cargo on the configured package with this binary as rustc.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use rust2_driver::cargo::{Args, Check, Output};

use crate::config::{self, Config};

const USAGE: &str = "usage: rust2-ffi --config <ffi.toml> (--out <dir> | --census <dir>) [--manifest-path <Cargo.toml>] [-- <cargo args>]

Exports the pub API of the library of the package that ffi.toml names, and of the other
packages it lists, into one library: a crate in <dir> that builds the C ABI, with its C
header in <dir>/include. The package's Cargo.toml is beside ffi.toml unless
--manifest-path says where. Cargo args, such as --features, are passed to `cargo check`,
on the host.

--census writes a Markdown report per package, <dir>/<package>.md, of what exports and
of every construct the export refuses, instead of exporting.";

pub fn run(args: &[String]) -> ExitCode {
	match try_run(args) {
		Ok(code) => code,
		Err(err) => {
			eprintln!("rust2-ffi: {err}");
			ExitCode::FAILURE
		}
	}
}

fn try_run(args: &[String]) -> Result<ExitCode, String> {
	let Some(args) = Args::parse(args, USAGE)? else {
		return Ok(ExitCode::SUCCESS);
	};
	let config = Config::load(&args.config)?;
	let (crates, patch) = crates(&args.manifest, &config, &args.cargo)?;
	let output = match &args.output {
		Output::Dir(out) => (config::env::OUT, out.as_os_str()),
		Output::Census(census) => (config::env::CENSUS, census.as_os_str()),
	};
	Check {
		tool: "rust2-ffi",
		manifest: &args.manifest,
		package: &config.package,
		root: None,
		// The host's, whatever cargo's config names: a C ABI is the platform's, and the
		// exports are found on it.
		target: Some(rustc_session::config::host_tuple()),
		args: &args.cargo,
		env: &[
			(config::env::CONFIG, args.config.as_os_str()),
			(config::env::CRATES, crates.as_ref()),
			(config::env::PATCH, patch.as_ref()),
			output,
		],
	}
	.run()
}

/// Each export's package, as [`config::env::CRATES`] lists them: the configured package
/// first. Then the patches it resolved with, as [`config::env::PATCH`] has them.
fn crates(manifest: &Path, config: &Config, cargo: &[String]) -> Result<(String, String), String> {
	let picks = Picks::parse(cargo)?;
	// On the host, so the resolved dependencies are the ones the check passes rustc.
	let host = rustc_session::config::host_tuple();
	let output = run_cargo(
		manifest,
		&["metadata", "--format-version", "1", "--filter-platform", host],
		&picks,
	)?;
	let metadata: Metadata = serde_json::from_slice(&output).map_err(|err| format!("reading cargo metadata: {err}"))?;
	let canonical =
		std::fs::canonicalize(manifest).map_err(|err| format!("resolving {}: {err}", manifest.display()))?;
	// Cargo keeps a symlink's spelling of the path.
	let top = (metadata.packages.iter())
		.find(|p| p.name == config.package && std::fs::canonicalize(&p.manifest_path).is_ok_and(|p| p == canonical));
	let Some(top) = top else {
		return Err(format!(
			"{} is not the package of {}",
			config.package,
			manifest.display()
		));
	};
	// What the library builds with on the host, each package as `<name> v<version>`, then
	// its directory in parentheses for a path package. Metadata's graph would also hold
	// what dev-dependencies' features add, and a workspace may hold another version of a
	// package beside it.
	let tree = [
		"tree",
		// The check's package, not the workspace's default members.
		"--package",
		&config.package,
		"--edges",
		"normal",
		"--target",
		rustc_session::config::host_tuple(),
		"--prefix",
		"none",
		"--format",
		"{p}",
		// Plain, whatever cargo's config says, so a repeat's `(*)` reads as one.
		"--color",
		"never",
	];
	let output = run_cargo(manifest, &tree, &picks)?;
	let output = String::from_utf8(output).map_err(|err| format!("reading cargo tree: {err}"))?;
	let reached: BTreeSet<&str> = output.lines().map(|line| line.trim_end_matches(" (*)")).collect();
	let features = picks.of_exports(top, config)?;
	let mut lines = String::new();
	for name in std::iter::once(&config.package).chain(config.crates.keys()) {
		let mut found = reached.iter().filter(|p| p.split(' ').next() == Some(name.as_str()));
		let (Some(&found), None) = (found.next(), found.next()) else {
			return Err(format!(
				"{name} is not exactly one package that {} builds with",
				config.package
			));
		};
		// The glue depends on each by path, so it builds the crate the export checked.
		let package = (metadata.packages.iter()).find(|p| {
			let dir = p.manifest_path.parent().expect("a manifest path has a parent");
			p.source.is_none() && found == format!("{} v{} ({})", p.name, p.version, dir.display())
		});
		let Some(package) = package else {
			return Err(format!("{name} is not a path package, which the export can depend on"));
		};
		let lib = (package.targets.iter()).find(|t| t.kind.iter().any(|k| matches!(k.as_str(), "lib" | "rlib")));
		let Some(lib) = lib else {
			return Err(format!("{name} has no library"));
		};
		let dir = package.manifest_path.parent().expect("a manifest path has a parent");
		// Another export by the name the configured package depends on it by, unique among
		// its dependencies: crates of two packages may share a name.
		let node = metadata.resolve.nodes.iter().find(|n| n.id == top.id);
		let krate = match package.id == top.id {
			true => Some(lib.name.clone()),
			false => (node.into_iter().flat_map(|n| &n.deps))
				.filter(|d| d.pkg == package.id && d.dep_kinds.iter().any(|k| k.kind.is_none()))
				.map(|d| d.name.clone())
				.min(),
		};
		let Some(krate) = krate else {
			return Err(format!(
				"{name} is not a dependency of {}, which names each export",
				config.package
			));
		};
		let package = config::Package {
			name: name.clone(),
			krate: krate.replace('-', "_"),
			dir: dir.into(),
			features: features.get(name.as_str()).cloned().unwrap_or_default(),
		};
		if lines
			.lines()
			.filter_map(|l| l.split('\t').next())
			.any(|n| n.replace('-', "_") == package.root())
		{
			return Err(format!("{name} is another export's name in snake case"));
		}
		lines.push_str(&package.line());
	}
	// Every platform's dependencies, which cargo resolves wherever the glue builds.
	let output = run_cargo(manifest, &["metadata", "--format-version", "1"], &picks)?;
	let resolved: Metadata = serde_json::from_slice(&output).map_err(|err| format!("reading cargo metadata: {err}"))?;
	Ok((lines, overrides(&resolved, top)?))
}

/// The `[patch]` the glue's workspace resolves the exports by as the check did, wherever
/// it builds, as TOML with each path absolute.
fn overrides(metadata: &Metadata, top: &Package) -> Result<String, String> {
	let patch = patch(metadata, top)?;
	let mut overrides = toml::Table::new();
	if !patch.is_empty() {
		overrides.insert("patch".to_string(), patch.into());
	}
	toml::to_string(&overrides).map_err(|err| format!("writing the patches: {err}"))
}

/// Each patch cargo applied in resolving what `top` builds with on any platform, wherever
/// it was declared: the manifest, cargo's config, `--config`, or the environment. A
/// dependency resolved to a package of another source than the one it names was patched,
/// or `[replace]`d by a package of the same version, which a patch resolves to alike; it
/// is keyed by the URL of the source it names, which reads the same without that config.
fn patch(metadata: &Metadata, top: &Package) -> Result<toml::Table, String> {
	let packages: BTreeMap<&str, &Package> = metadata.packages.iter().map(|p| (p.id.as_str(), p)).collect();
	let nodes: BTreeMap<&str, &Node> = metadata.resolve.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
	let mut patch = toml::Table::new();
	let mut seen = BTreeSet::new();
	let mut queue = vec![top.id.as_str()];
	while let Some(id) = queue.pop() {
		let (true, Some(package), Some(node)) = (seen.insert(id), packages.get(id), nodes.get(id)) else {
			continue;
		};
		// What the library builds, not what tests do.
		let built = |kind: &Option<String>| kind.as_deref() != Some("dev");
		for dep in node.deps.iter().filter(|d| d.dep_kinds.iter().any(|k| built(&k.kind))) {
			queue.push(&dep.pkg);
			let Some(resolved) = packages.get(dep.pkg.as_str()) else {
				continue;
			};
			// Unrenamed, the edge is named by the library's crate name.
			let lib = (resolved.targets.iter()).find(|t| t.kind.iter().any(|k| k.contains("lib") || k == "proc-macro"));
			let named = (package.dependencies.iter()).find(|d| {
				let name = d.rename.as_deref().or(lib.map(|t| t.name.as_str()));
				built(&d.kind) && d.name == resolved.name && name.map(|n| n.replace('-', "_")) == Some(dep.name.clone())
			});
			// A path dependency names no source to patch.
			let Some(named) = named.and_then(|d| d.source.as_deref()) else {
				continue;
			};
			// A git package's source adds the commit it resolved to.
			let source = resolved.source.as_deref().map(|s| s.split('#').next().unwrap_or(s));
			if source == Some(named) {
				continue;
			}
			let key = patch_key(named)?;
			let replacement = match resolved.source.as_deref() {
				None => {
					let dir = resolved.manifest_path.parent().expect("a manifest path has a parent");
					toml::toml! { path = (dir.display().to_string()) }
				}
				Some(git) if git.starts_with("git+") => {
					let (url, rev) = git["git+".len()..].split_once('#').unwrap_or((git, ""));
					let url = url.split('?').next().unwrap_or(url);
					// Only a `[replace]` resolves a package to another commit of the source
					// the dependency names; cargo refuses a patch from that source.
					if url == key {
						return Err(format!(
							"{} {} is replaced from its own source, which a `[patch]` of the export's library cannot carry",
							resolved.name, resolved.version
						));
					}
					// A repository may hold several versions of the package.
					let version = format!("={}", resolved.version);
					toml::toml! { git = url rev = rev version = version }
				}
				Some(other) => {
					return Err(format!(
						"{} {} is patched in from {other}, which the export's library cannot patch in",
						resolved.name, resolved.version
					));
				}
			};
			let into = patch.entry(key).or_insert_with(|| toml::Table::new().into());
			let into = into.as_table_mut().expect("a patch's source is a table");
			match into.get(&resolved.name) {
				Some(toml::Value::Table(existing)) if *existing == replacement => {}
				// Two versions of one package patched in from one source.
				Some(_) => {
					let mut renamed = replacement;
					renamed.insert("package".to_string(), resolved.name.clone().into());
					into.insert(format!("{}-{}", resolved.name, resolved.version), renamed.into());
				}
				None => {
					into.insert(resolved.name.clone(), replacement.into());
				}
			}
		}
	}
	Ok(patch)
}

/// The `[patch]` key of a source as `cargo metadata` spells it.
fn patch_key(source: &str) -> Result<String, String> {
	if source == "registry+https://github.com/rust-lang/crates.io-index" || source == "sparse+https://index.crates.io/"
	{
		return Ok("crates-io".to_string());
	}
	if let Some(url) = source.strip_prefix("registry+") {
		return Ok(url.to_string());
	}
	if source.starts_with("sparse+") {
		return Ok(source.to_string());
	}
	if let Some(url) = source.strip_prefix("git+") {
		return Ok(url.split(['?', '#']).next().unwrap_or(url).to_string());
	}
	Err(format!("a patch of {source}, which the export's library cannot patch"))
}

/// What a cargo subcommand prints for the package, run as the check resolves: from the
/// package's directory, where cargo reads its config, with the check's picks.
fn run_cargo(manifest: &Path, subcommand: &[&str], picks: &Picks<'_>) -> Result<Vec<u8>, String> {
	let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
		.current_dir(manifest.parent().expect("a manifest path has a parent"))
		.args(subcommand)
		.arg("--manifest-path")
		.arg(manifest)
		.args(picks.args())
		.output()
		.map_err(|err| format!("running cargo {}: {err}", subcommand[0]))?;
	match output.status.success() {
		true => Ok(output.stdout),
		false => Err(String::from_utf8_lossy(&output.stderr).into_owned()),
	}
}

/// What the cargo args pick for how the check resolves its dependencies.
struct Picks<'a> {
	/// Each feature `--features` and `-F` name.
	named: Vec<&'a str>,
	/// The other args that pick a resolution: feature, lockfile, network, and `--config`.
	flags: Vec<&'a str>,
}

impl<'a> Picks<'a> {
	fn parse(cargo: &'a [String]) -> Result<Self, String> {
		let mut picks = Self {
			named: Vec::new(),
			flags: Vec::new(),
		};
		let mut args = cargo.iter().map(String::as_str);
		while let Some(arg) = args.next() {
			let value = match arg {
				"--all-features" | "--no-default-features" | "--locked" | "--frozen" | "--offline" => {
					picks.flags.push(arg);
					continue;
				}
				"--target" => return Err("--target: the export builds for the host".to_string()),
				_ if arg.starts_with("--target=") => return Err(format!("{arg}: the export builds for the host")),
				"--config" => {
					picks.flags.extend([Some(arg), args.next()].into_iter().flatten());
					continue;
				}
				_ if arg.starts_with("--config=") => {
					picks.flags.push(arg);
					continue;
				}
				"--features" | "-F" => args.next(),
				_ => (arg.strip_prefix("--features="))
					.or_else(|| arg.strip_prefix("-F").map(|v| v.strip_prefix('=').unwrap_or(v))),
			};
			let named = value.into_iter().flat_map(|v| v.split([',', ' ']));
			picks.named.extend(named.filter(|f| !f.is_empty()));
		}
		Ok(picks)
	}

	/// The same picks, as cargo args.
	fn args(&self) -> Vec<String> {
		let mut args: Vec<String> = self.flags.iter().map(|f| f.to_string()).collect();
		if !self.named.is_empty() {
			args.extend(["--features".to_string(), self.named.join(",")]);
		}
		args
	}

	/// The features named in another export directly, as `<dependency>/<feature>`, by
	/// package. Cargo unifies in the rest through the configured package's own.
	fn of_exports(&self, top: &'a Package, config: &Config) -> Result<BTreeMap<&'a str, Vec<String>>, String> {
		let mut features: BTreeMap<&str, Vec<String>> = BTreeMap::new();
		for feature in &self.named {
			let Some((key, name)) = feature.split_once('/') else {
				continue;
			};
			let key = key.strip_suffix('?').unwrap_or(key);
			let dep =
				(top.dependencies.iter()).find(|d| d.kind.is_none() && d.rename.as_deref().unwrap_or(&d.name) == key);
			// Cargo refuses a feature of no dependency.
			let Some(dep) = dep else {
				continue;
			};
			if !config.crates.contains_key(&dep.name) {
				return Err(format!(
					"--features {feature}: the export builds {} with only the features {} enables in it, unless it is an export too",
					dep.name, config.package
				));
			}
			features.entry(&dep.name).or_default().push(name.to_string());
		}
		Ok(features)
	}
}

/// What `cargo metadata` says of the packages, and how they depend on each other.
#[derive(serde::Deserialize)]
struct Metadata {
	packages: Vec<Package>,
	resolve: Resolve,
}

#[derive(serde::Deserialize)]
struct Resolve {
	nodes: Vec<Node>,
}

#[derive(serde::Deserialize)]
struct Node {
	id: String,
	deps: Vec<Dep>,
}

/// A resolved dependency.
#[derive(serde::Deserialize)]
struct Dep {
	/// The name the dependent calls its library by.
	name: String,
	pkg: String,
	dep_kinds: Vec<DepKind>,
}

#[derive(serde::Deserialize)]
struct DepKind {
	/// `dev` or `build`, or none for a normal dependency.
	kind: Option<String>,
}

#[derive(serde::Deserialize)]
struct Package {
	id: String,
	name: String,
	version: String,
	source: Option<String>,
	manifest_path: PathBuf,
	targets: Vec<Target>,
	dependencies: Vec<Dependency>,
}

#[derive(serde::Deserialize)]
struct Dependency {
	/// The package's name.
	name: String,
	/// The name the dependent calls it by, when not the package's.
	rename: Option<String>,
	/// `dev` or `build`, or none for a normal dependency.
	kind: Option<String>,
	/// The source it names, such as `registry+<url>`, or none for a path dependency.
	source: Option<String>,
}

#[derive(serde::Deserialize)]
struct Target {
	name: String,
	kind: Vec<String>,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reads_what_picks_a_resolution() {
		for spelling in [
			&["--features", "a,dep/b"][..],
			&["--features=a dep/b"],
			&["-F", "a", "-F", "dep/b"],
			&["-Fa,dep/b"],
			&["-F=a", "--features", "dep/b"],
		] {
			let cargo: Vec<String> = spelling.iter().map(|s| s.to_string()).collect();
			assert_eq!(
				Picks::parse(&cargo).unwrap().args(),
				["--features", "a,dep/b"],
				"{spelling:?}"
			);
		}
		let cargo = [
			"--all-features",
			"--locked",
			"--config",
			"a=1",
			"--config=b=2",
			"--quiet",
		];
		let cargo: Vec<String> = cargo.iter().map(|s| s.to_string()).collect();
		let flags = ["--all-features", "--locked", "--config", "a=1", "--config=b=2"];
		assert_eq!(Picks::parse(&cargo).unwrap().args(), flags);
		for target in [
			&["--target", "wasm32-unknown-unknown"][..],
			&["--target=wasm32-unknown-unknown"],
		] {
			let cargo: Vec<String> = target.iter().map(|s| s.to_string()).collect();
			assert!(Picks::parse(&cargo).is_err(), "{target:?}");
		}
	}
}
