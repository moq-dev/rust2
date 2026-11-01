//! The choice a crate leaves to the wasm32 binary that depends on it, which translation
//! makes as a browser's binary would: the source of randomness. getrandom and uuid refuse
//! to build for wasm32-unknown-unknown until the binary enables the feature that picks
//! `crypto.getRandomValues`, and only a package that depends on them can, so translation
//! checks the package through a root package of its own that does.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::TARGET;

/// The feature that picks `crypto.getRandomValues`, by crate and the version prefix it is
/// documented for.
const PICKS: [(&str, &str, &str); 4] = [
	("getrandom", "0.2.", "js"),
	("getrandom", "0.3.", "wasm_js"),
	("getrandom", "0.4.", "wasm_js"),
	("uuid", "1.", "js"),
];

/// A cache directory's tag, as <https://bford.info/cachedir/> specifies it.
const CACHEDIR_TAG: &str = "Signature: 8a477f597d28d172789f06886806bc55
# This file is a cache directory tag created by rust2-ts.
# For information about cache directory tags see https://bford.info/cachedir/
";

/// The root package that checks `package` with the browser's picks.
pub struct Root {
	/// Its `Cargo.toml`, absolute.
	pub manifest: PathBuf,
	/// The cargo args for the check through it: the package's features move into the
	/// root's dependency on it, since cargo takes none on the command line for a package
	/// outside the workspace.
	pub cargo: Vec<String>,
}

/// The root package for `package` of `manifest`, written in `dir`, if what it builds with
/// for [`TARGET`] holds a crate that leaves the browser's randomness to the binary.
pub fn root(manifest: &Path, package: &str, dir: &Path, cargo: &[String]) -> Result<Option<Root>, String> {
	let args = Args::parse(cargo);
	let packages = tree(manifest, package, &args)?;
	let picks: Vec<_> = (packages.iter())
		.filter_map(|(name, version)| {
			let pick = PICKS
				.iter()
				.find(|(krate, prefix, _)| krate == name && version.starts_with(prefix));
			pick.map(|&(_, _, feature)| (name.as_str(), version.as_str(), feature))
		})
		.collect();
	if picks.is_empty() {
		return Ok(None);
	}
	if args.rest.iter().any(|arg| arg == "--all-features") {
		return Err(format!(
			"--all-features: the browser's randomness checks {package} as a dependency, which takes only the features --features names"
		));
	}
	let workspace = rust2_driver::cargo::workspace_root(Some(manifest))?;
	// Cargo cleans only a target directory it made, which it tags as a cache, as it
	// would here had it run first.
	if !dir.exists() {
		std::fs::create_dir_all(dir).map_err(|err| format!("creating {}: {err}", dir.display()))?;
		write(&dir.join("CACHEDIR.TAG"), CACHEDIR_TAG)?;
	}
	let dir = dir.join("browser").join(package);
	std::fs::create_dir_all(&dir).map_err(|err| format!("creating {}: {err}", dir.display()))?;
	let text = manifest_text(manifest, package, &workspace, &args, &picks, &packages)?;
	write(&dir.join("Cargo.toml"), &text)?;
	write(&dir.join("lib.rs"), "")?;
	// The workspace's own versions, wherever the root needs no other.
	let lock = workspace.join("Cargo.lock");
	if lock.exists() {
		std::fs::copy(&lock, dir.join("Cargo.lock")).map_err(|err| format!("copying {}: {err}", lock.display()))?;
	}
	// The lockfile gains the root and what its picks build with, so `--locked` holds the
	// workspace's own lockfile to account in `cargo tree` above, not the root's.
	let mut cargo: Vec<String> = (args.rest.into_iter())
		.filter(|arg| arg != "--locked")
		.map(|arg| match arg.as_str() {
			"--frozen" => "--offline".to_string(),
			_ => arg,
		})
		.collect();
	// The workspace's paths, as rustc reads its members': relative to its root, so
	// `file!()` and the census read as they would there.
	let remap = toml::Value::from(format!("--remap-path-prefix={}/=", workspace.display()));
	cargo.extend(["--config".into(), format!("profile.dev.rustflags=[{remap}]")]);
	Ok(Some(Root {
		manifest: dir.join("Cargo.toml"),
		cargo,
	}))
}

/// The cargo args, with the package's features apart.
struct Args {
	/// Each feature `--features` and `-F` name.
	features: Vec<String>,
	/// Whether `--no-default-features` is given.
	no_default: bool,
	/// The rest.
	rest: Vec<String>,
}

impl Args {
	fn parse(cargo: &[String]) -> Self {
		let mut parsed = Self {
			features: Vec::new(),
			no_default: false,
			rest: Vec::new(),
		};
		let mut args = cargo.iter();
		while let Some(arg) = args.next() {
			let value = match arg.as_str() {
				"--no-default-features" => {
					parsed.no_default = true;
					continue;
				}
				"--features" | "-F" => args.next().map(String::as_str),
				_ => match arg.strip_prefix("--features=").or_else(|| arg.strip_prefix("-F")) {
					Some(value) => Some(value.strip_prefix('=').unwrap_or(value)),
					None => {
						parsed.rest.push(arg.clone());
						continue;
					}
				},
			};
			let named = value.into_iter().flat_map(|v| v.split([',', ' ']));
			parsed
				.features
				.extend(named.filter(|f| !f.is_empty()).map(String::from));
		}
		parsed
	}

	/// The args that pick how `package` resolves, as `cargo tree` takes them.
	fn resolution(&self) -> Vec<String> {
		let mut args = Vec::new();
		let mut rest = self.rest.iter();
		while let Some(arg) = rest.next() {
			match arg.as_str() {
				"--all-features" | "--locked" | "--frozen" | "--offline" => args.push(arg.clone()),
				"--config" => args.extend([Some(arg), rest.next()].into_iter().flatten().cloned()),
				_ if arg.starts_with("--config=") => args.push(arg.clone()),
				_ => {}
			}
		}
		if self.no_default {
			args.push("--no-default-features".into());
		}
		if !self.features.is_empty() {
			args.extend(["--features".into(), self.features.join(",")]);
		}
		args
	}
}

/// Each package `package` builds with for [`TARGET`], by name and version.
fn tree(manifest: &Path, package: &str, args: &Args) -> Result<Vec<(String, String)>, String> {
	let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
		// Cargo reads its config from where it runs, as the check does.
		.current_dir(manifest.parent().expect("a manifest path has a parent"))
		.args(["tree", "--manifest-path"])
		.arg(manifest)
		.args(["--package", package, "--edges", "normal", "--target", TARGET])
		// Each package as `<name> v<version>`, plain, so a repeat's `(*)` reads as one.
		.args(["--prefix", "none", "--format", "{p}", "--color", "never"])
		.args(args.resolution())
		.output()
		.map_err(|err| format!("running cargo tree: {err}"))?;
	if !output.status.success() {
		return Err(String::from_utf8_lossy(&output.stderr).into_owned());
	}
	let output = String::from_utf8(output.stdout).map_err(|err| format!("reading cargo tree: {err}"))?;
	let mut packages = Vec::new();
	for line in output.lines() {
		let mut words = line.split(' ');
		if let (Some(name), Some(version)) = (words.next(), words.next().and_then(|v| v.strip_prefix('v'))) {
			let package = (name.to_string(), version.to_string());
			if !packages.contains(&package) {
				packages.push(package);
			}
		}
	}
	Ok(packages)
}

/// The root's `Cargo.toml`: a library on `package` and each pick, in a workspace that
/// resolves and compiles `packages` as `workspace`, the package's own, does.
fn manifest_text(
	manifest: &Path,
	package: &str,
	workspace: &Path,
	args: &Args,
	picks: &[(&str, &str, &str)],
	packages: &[(String, String)],
) -> Result<String, String> {
	let dir = manifest.parent().expect("a manifest path has a parent");
	let mut dependencies = toml::Table::new();
	// A feature named as the package's own, `<package>/<feature>`, by its name there.
	let own = format!("{package}/");
	let features: Vec<&str> = (args.features.iter())
		.map(|f| f.strip_prefix(&own).unwrap_or(f))
		.collect();
	let dependency = toml::toml! {
		path = (dir.display().to_string())
		default-features = (!args.no_default)
		features = features
	};
	dependencies.insert(package.to_string(), dependency.into());
	for (name, version, feature) in picks {
		// Its feature alone, so the crate builds as the package has it otherwise.
		let dependency = toml::toml! {
			package = (name.to_string())
			version = (format!("={version}"))
			default-features = false
			features = (vec![feature.to_string()])
		};
		dependencies.insert(format!("{name}-{}", version.replace('.', "_")), dependency.into());
	}
	let mut root = toml::toml! {
		[package]
		name = "rust2-ts-browser"
		version = "0.0.0"
		edition = "2024"
		publish = false

		[lib]
		path = "lib.rs"
	};
	root.insert("dependencies".into(), dependencies.into());
	let path = workspace.join("Cargo.toml");
	let text = std::fs::read_to_string(&path).map_err(|err| format!("reading {}: {err}", path.display()))?;
	let mut theirs: toml::Table = toml::from_str(&text).map_err(|err| format!("parsing {}: {err}", path.display()))?;
	root.insert(
		"workspace".into(),
		toml::toml! { resolver = (resolver(&theirs)) }.into(),
	);
	// What else picks how the workspace resolves, each path absolute.
	for key in ["patch", "replace"] {
		if let Some(mut value) = theirs.remove(key) {
			absolute(&mut value, workspace);
			root.insert(key.into(), value);
		}
	}
	// The profile the check compiles with, but its overrides of packages it never builds,
	// which cargo would warn of.
	let dev = theirs
		.get_mut("profile")
		.and_then(|p| p.as_table_mut())
		.and_then(|p| p.remove("dev"));
	if let Some(mut dev) = dev {
		if let Some(overrides) = dev.get_mut("package").and_then(toml::Value::as_table_mut) {
			overrides.retain(|spec, _| {
				let name = spec.split('@').next().unwrap_or(spec);
				spec == "*" || packages.iter().any(|(built, _)| built == name)
			});
		}
		root.insert("profile".into(), toml::toml! { dev = dev }.into());
	}
	toml::to_string(&root).map_err(|err| format!("writing the root package: {err}"))
}

/// The feature resolver a workspace's root manifest picks: its own, or its package's
/// edition's, or a virtual manifest's 1.
fn resolver(root: &toml::Table) -> String {
	let field = |table: &str, key: &str| root.get(table).and_then(|t| t.get(key)).and_then(toml::Value::as_str);
	if let Some(resolver) = field("workspace", "resolver").or_else(|| field("package", "resolver")) {
		return resolver.to_string();
	}
	// An edition the package inherits is the workspace's.
	let inherited = || root.get("workspace")?.get("package")?.get("edition")?.as_str();
	let edition = root.get("package").and(field("package", "edition").or_else(inherited));
	match edition {
		Some("2024") => "3",
		Some("2021") => "2",
		_ => "1",
	}
	.to_string()
}

/// `value` with each `path` in it absolute, relative to `dir` where it was not.
fn absolute(value: &mut toml::Value, dir: &Path) {
	let Some(table) = value.as_table_mut() else {
		return;
	};
	for (key, value) in table.iter_mut() {
		match value {
			toml::Value::String(path) if key == "path" => *path = dir.join(&*path).display().to_string(),
			_ => absolute(value, dir),
		}
	}
}

fn write(path: &Path, text: &str) -> Result<(), String> {
	std::fs::write(path, text).map_err(|err| format!("writing {}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn takes_the_features_apart() {
		for spelling in [
			&["--features", "a,dep/b"][..],
			&["--features=a dep/b"],
			&["-F", "a", "-F", "dep/b"],
			&["-Fa,dep/b"],
			&["-F=a", "--features", "dep/b"],
		] {
			let cargo: Vec<String> = spelling.iter().map(|s| s.to_string()).collect();
			let args = Args::parse(&cargo);
			assert_eq!(
				(args.features, args.rest),
				(vec!["a".into(), "dep/b".into()], vec![]),
				"{spelling:?}"
			);
		}
		let cargo = ["--no-default-features", "--locked", "--config", "a=1", "--quiet"];
		let args = Args::parse(&cargo.map(String::from));
		assert!(args.no_default);
		assert_eq!(args.rest, ["--locked", "--config", "a=1", "--quiet"]);
		assert_eq!(
			args.resolution(),
			["--locked", "--config", "a=1", "--no-default-features"]
		);
	}
}
