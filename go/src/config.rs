//! The `go.toml` that says what to translate.

use std::path::Path;

use serde::Deserialize;

/// What to translate from one crate.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Config {
	/// The cargo package whose library is translated.
	pub package: String,
	/// Paths from the crate root to the functions to translate, which Go callers reach as
	/// exported functions. Everything they reach comes along.
	pub roots: Vec<String>,
}

impl Config {
	pub fn load(path: &Path) -> Result<Self, String> {
		let text = std::fs::read_to_string(path).map_err(|err| format!("reading {}: {err}", path.display()))?;
		toml::from_str(&text).map_err(|err| format!("parsing {}: {err}", path.display()))
	}
}

/// Environment variables the CLI hands the wrapper.
pub mod env {
	/// The `go.toml` path, absolute.
	pub const CONFIG: &str = "RUST2_GO_CONFIG";
	/// The output directory, absolute.
	pub const OUT: &str = "RUST2_GO_OUT";
}
