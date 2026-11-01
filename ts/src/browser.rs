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
	let packages = tree(manifest, package, "normal", &args)?;
	let picks: Vec<_> = (packages.iter())
		.filter_map(|p| {
			let pick = PICKS
				.iter()
				.find(|(krate, prefix, _)| *krate == p.name && p.version.starts_with(prefix));
			pick.map(|&(_, _, feature)| (p, feature))
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
	// `cargo tree` lists the package first, wherever its directory is.
	let Some(Source::Path(package_dir)) = packages.first().map(|p| &p.source) else {
		return Err(format!(
			"{package} is not a path package, which the browser's randomness can depend on"
		));
	};
	let workspace = rust2_driver::cargo::workspace_root(Some(manifest))?;
	// Cargo cleans only a target directory it made, which it tags as a cache, as it
	// would here had it run first.
	if !dir.exists() {
		std::fs::create_dir_all(dir).map_err(|err| format!("creating {}: {err}", dir.display()))?;
		write(&dir.join("CACHEDIR.TAG"), CACHEDIR_TAG)?;
	}
	let dir = dir.join("browser").join(package);
	std::fs::create_dir_all(&dir).map_err(|err| format!("creating {}: {err}", dir.display()))?;
	if config_overrides_all(manifest, &args)? {
		return Err(format!(
			"cargo's config overrides every package's dev profile with `\"*\"`, which would reach {package} through the root that picks the browser's randomness, as it never reaches a workspace member: set it in the workspace's Cargo.toml instead"
		));
	}
	// What cargo compiles, build scripts' dependencies too, which the profile overrides.
	let built = tree(manifest, package, "normal,build", &args)?;
	let members = members(manifest, &args)?;
	let text = manifest_text(package, package_dir, &workspace, &args, &picks, &built, &members)?;
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
	// getrandom 0.3 before 0.3.4 takes its backend from a cfg beside the feature, which
	// the others ignore.
	if picks
		.iter()
		.any(|(p, _)| p.name == "getrandom" && p.version.starts_with("0.3."))
	{
		let cfg = r#"profile.dev.package.getrandom.rustflags=["--cfg", 'getrandom_backend="wasm_js"']"#;
		cargo.extend(["--config".into(), cfg.into()]);
	}
	Ok(Some(Root {
		manifest: dir.join("Cargo.toml"),
		cargo,
	}))
}

/// A package `cargo tree` lists.
struct Package {
	name: String,
	version: String,
	source: Source,
}

/// Where a package comes from.
enum Source {
	/// crates.io, which a dependency names by its version alone.
	Registry,
	/// This directory.
	Path(PathBuf),
	/// A Git repository at a branch, tag, or revision, if its URL names one.
	Git {
		url: String,
		reference: Option<(String, String)>,
	},
	/// Another, as `cargo tree` prints it.
	Other(String),
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

	/// Each `--config` arg, with its value.
	fn configs(&self) -> Vec<String> {
		let mut args = Vec::new();
		let mut rest = self.rest.iter();
		while let Some(arg) = rest.next() {
			match arg.as_str() {
				"--config" => args.extend([Some(arg), rest.next()].into_iter().flatten().cloned()),
				_ if arg.starts_with("--config=") => args.push(arg.clone()),
				_ => {}
			}
		}
		args
	}

	/// The args that pick how `package` resolves, as `cargo tree` takes them.
	fn resolution(&self) -> Vec<String> {
		let flags = ["--all-features", "--locked", "--frozen", "--offline"];
		let mut args: Vec<String> = (self.rest.iter())
			.filter(|arg| flags.contains(&arg.as_str()))
			.cloned()
			.collect();
		args.extend(self.configs());
		if self.no_default {
			args.push("--no-default-features".into());
		}
		if !self.features.is_empty() {
			args.extend(["--features".into(), self.features.join(",")]);
		}
		args
	}
}

/// Each package `package` builds with for [`TARGET`] along `edges`, itself first.
fn tree(manifest: &Path, package: &str, edges: &str, args: &Args) -> Result<Vec<Package>, String> {
	let tree = ["--package", package, "--edges", edges, "--target", TARGET];
	let output = cargo_tree(manifest, &tree, args)?;
	let mut seen = std::collections::HashSet::new();
	let mut packages = Vec::new();
	for line in output.lines() {
		let line = line.trim_end_matches(" (*)").replace(" (proc-macro)", "");
		// A package is its name, version, and source together.
		if !seen.insert(line.clone()) {
			continue;
		}
		let Some((name, rest)) = line.split_once(' ') else {
			continue;
		};
		let (version, source) = rest.split_once(' ').unwrap_or((rest, ""));
		let version = version.strip_prefix('v').unwrap_or(version);
		let source = match source.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
			None => Source::Registry,
			Some(dir) if Path::new(dir).is_absolute() => Source::Path(dir.into()),
			Some(git) if git.contains("://") => {
				let url = git.split('#').next().unwrap_or(git);
				let (url, query) = url.split_once('?').unwrap_or((url, ""));
				let reference = query.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()));
				Source::Git {
					url: url.to_string(),
					reference,
				}
			}
			Some(other) => Source::Other(other.to_string()),
		};
		packages.push(Package {
			name: name.to_string(),
			version: version.to_string(),
			source,
		});
	}
	Ok(packages)
}

/// Whether cargo's config, read as the check reads it, sets the dev profile's `"*"`
/// override.
fn config_overrides_all(manifest: &Path, args: &Args) -> Result<bool, String> {
	let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
		.current_dir(manifest.parent().expect("a manifest path has a parent"))
		.arg("-Zunstable-options")
		.args(args.configs())
		.args(["config", "get", "profile.dev.package"])
		.output()
		.map_err(|err| format!("running cargo config: {err}"))?;
	let stderr = String::from_utf8_lossy(&output.stderr);
	match output.status.success() {
		true => Ok(String::from_utf8_lossy(&output.stdout).contains(".\"*\".")),
		false if stderr.contains("is not set") => Ok(false),
		false => Err(stderr.into_owned()),
	}
}

/// The names of the members of the workspace `manifest` belongs to.
fn members(manifest: &Path, args: &Args) -> Result<Vec<String>, String> {
	let output = cargo_tree(manifest, &["--workspace", "--depth", "0"], args)?;
	Ok((output.lines())
		.filter_map(|line| line.split(' ').next())
		.filter(|name| !name.is_empty())
		.map(String::from)
		.collect())
}

/// What `cargo tree` prints for the workspace `manifest` belongs to with `tree`'s args and
/// the check's resolution: each package as `<name> v<version>`, then its source in
/// parentheses unless it is crates.io, plain, so a repeat's `(*)` reads as one.
fn cargo_tree(manifest: &Path, tree: &[&str], args: &Args) -> Result<String, String> {
	let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
		// Cargo reads its config from where it runs, as the check does.
		.current_dir(manifest.parent().expect("a manifest path has a parent"))
		.args(["tree", "--manifest-path"])
		.arg(manifest)
		.args(tree)
		.args(["--prefix", "none", "--format", "{p}", "--color", "never"])
		.args(args.resolution())
		.output()
		.map_err(|err| format!("running cargo tree: {err}"))?;
	if !output.status.success() {
		return Err(String::from_utf8_lossy(&output.stderr).into_owned());
	}
	String::from_utf8(output.stdout).map_err(|err| format!("reading cargo tree: {err}"))
}

/// The root's `Cargo.toml`: a library on `package`, in `dir`, and on each pick, in a
/// workspace that resolves and compiles the `built` packages as `workspace`, the package's
/// own, does.
fn manifest_text(
	package: &str,
	dir: &Path,
	workspace: &Path,
	args: &Args,
	picks: &[(&Package, &str)],
	built: &[Package],
	members: &[String],
) -> Result<String, String> {
	let mut dependencies = toml::Table::new();
	// A feature named as the package's own, `<package>/<feature>`, by its name there.
	let own = format!("{package}/");
	let features: Vec<&str> = (args.features.iter())
		.map(|f| f.strip_prefix(&own).unwrap_or(f))
		.collect();
	// Cargo takes none of a dependency's own dependencies' features.
	if let Some(feature) = features.iter().find(|f| f.contains('/')) {
		return Err(format!(
			"--features {feature}: the browser's randomness checks {package} as a dependency, which takes only its own features"
		));
	}
	let dependency = toml::toml! {
		path = (dir.display().to_string())
		default-features = (!args.no_default)
		features = features
	};
	dependencies.insert(package.to_string(), dependency.into());
	for (i, (pick, feature)) in picks.iter().enumerate() {
		// Its feature alone, so the crate builds as the package has it otherwise, from the
		// source it resolved to, which a version alone would name another of.
		let mut dependency = toml::toml! {
			package = (pick.name.clone())
			version = (format!("={}", pick.version))
			default-features = false
			features = (vec![feature.to_string()])
		};
		match &pick.source {
			Source::Registry => {}
			// Another registry, which the root would have to name by the key cargo's config
			// gives it.
			Source::Other(source) => {
				return Err(format!(
					"{} {} is from {source}, which the browser's randomness can't depend on",
					pick.name, pick.version
				));
			}
			Source::Path(dir) => {
				dependency.insert("path".into(), dir.display().to_string().into());
			}
			Source::Git { url, reference } => {
				dependency.insert("git".into(), url.clone().into());
				if let Some((key, value)) = reference {
					dependency.insert(key.clone(), value.clone().into());
				}
			}
		}
		// Unique, since a graph may hold one version of a crate from two sources.
		let key = format!("{}-{}-{i}", pick.name, pick.version.replace(['.', '+'], "_"));
		dependencies.insert(key, dependency.into());
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
	let dev = theirs
		.get_mut("profile")
		.and_then(|p| p.as_table_mut())
		.and_then(|p| p.remove("dev"));
	if let Some(toml::Value::Table(dev)) = dev {
		let dev = dev_profile(dev, built, members);
		root.insert("profile".into(), toml::toml! { dev = dev }.into());
	}
	toml::to_string(&root).map_err(|err| format!("writing the root package: {err}"))
}

/// The workspace's dev profile `dev` as the root's, for the `built` packages, of which
/// `members` were the workspace's own: `"*"` overrides every package but a member, which
/// outside the workspace it would override too, so it becomes each other package's
/// override, under the one that already matches it. An override of a package never built
/// goes, which cargo would warn of.
fn dev_profile(mut dev: toml::Table, built: &[Package], members: &[String]) -> toml::Table {
	let Some(toml::Value::Table(mut overrides)) = dev.remove("package") else {
		return dev;
	};
	let all = overrides.remove("*");
	let matches = |spec: &str, p: &Package| {
		spec_parts(spec).is_some_and(|(name, version)| {
			p.name == name && version.is_none_or(|v| p.version == v || p.version.starts_with(&format!("{v}.")))
		})
	};
	// A spec of another form, such as a URL, may match one.
	overrides.retain(|spec, _| spec_parts(spec).is_none() || built.iter().any(|p| matches(spec, p)));
	if let Some(toml::Value::Table(all)) = all {
		for package in built.iter().filter(|p| !members.contains(&p.name)) {
			let spec = (overrides.keys())
				.find(|spec| matches(spec, package))
				.cloned()
				.unwrap_or_else(|| format!("{}@{}", package.name, package.version));
			if let toml::Value::Table(named) = overrides.entry(spec).or_insert_with(|| toml::Table::new().into()) {
				for (key, value) in &all {
					named.entry(key.clone()).or_insert_with(|| value.clone());
				}
			}
		}
	}
	dev.insert("package".into(), overrides.into());
	dev
}

/// The name and version, if any, of a package spec of the form `name`, `name@version`, or
/// `name:version`.
fn spec_parts(spec: &str) -> Option<(&str, Option<&str>)> {
	// A URL's scheme reads as a name before a `:`.
	if spec.contains('/') {
		return None;
	}
	let (name, version) = match spec.split_once(['@', ':']) {
		Some((name, version)) => (name, Some(version)),
		None => (spec, None),
	};
	let crate_name = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
	crate_name.then_some((name, version))
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

	#[test]
	fn keeps_star_off_the_members() {
		let package = |name: &str| Package {
			name: name.into(),
			version: "1.0.0".into(),
			source: Source::Registry,
		};
		let packages = [
			package("member"),
			package("dep"),
			package("named"),
			package("versioned"),
		];
		let dev: toml::Table = toml::from_str(
			r#"
			debug = 1
			[package."*"]
			debug-assertions = false
			opt-level = 1
			[package.named]
			opt-level = 3
			[package.unbuilt]
			opt-level = 3
			[package."versioned@1"]
			opt-level = 2
			[package."https://example.com/index#other@1.0.0"]
			opt-level = 2
			"#,
		)
		.unwrap();
		let expected: toml::Table = toml::from_str(
			r#"
			debug = 1
			[package."dep@1.0.0"]
			debug-assertions = false
			opt-level = 1
			[package.named]
			debug-assertions = false
			opt-level = 3
			[package."versioned@1"]
			debug-assertions = false
			opt-level = 2
			[package."https://example.com/index#other@1.0.0"]
			opt-level = 2
			"#,
		)
		.unwrap();
		assert_eq!(dev_profile(dev, &packages, &["member".into()]), expected);
	}
}
