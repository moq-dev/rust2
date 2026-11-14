//! The audit's output: a JSON report of every `pub` token, a text report of the
//! findings, and the validated narrowing as a patch.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::analyze::{Audit, Category, Evidence, Place, Token, Verdict};
use crate::validate::{Restored, Validation};

/// What the report says of each token.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
enum Outcome {
	/// Narrowed with every other, and the workspaces checked alike.
	Narrowed,
	/// Proposed, then restored by what the check found.
	Restored,
	/// The API of a library crates or tools outside the audit may use.
	External,
	/// Named by another crate.
	CrossCrate,
	/// A symbol code outside Rust links to.
	Symbol,
	/// Not in a local package's source.
	Generated,
	/// In a file a crate embeds as bytes.
	Embedded,
}

#[derive(Serialize)]
struct Report<'a> {
	/// The directory every path is relative to.
	root: &'a Path,
	workspaces: Vec<&'a Path>,
	cargo_args: &'a [String],
	/// What the checks did not see.
	unchecked: &'static str,
	rounds: usize,
	summary: BTreeMap<String, usize>,
	tokens: Vec<TokenReport>,
}

#[derive(Serialize)]
struct TokenReport {
	file: PathBuf,
	line: usize,
	column: usize,
	offset: u32,
	outcome: Outcome,
	category: Category,
	#[serde(skip_serializing_if = "Option::is_none")]
	restored: Option<String>,
	items: Vec<ItemReport>,
}

#[derive(Serialize)]
struct ItemReport {
	path: String,
	kind: String,
	#[serde(rename = "crate")]
	crate_name: String,
	category: Category,
	/// Whether a crate depending on its library can name it.
	exported: bool,
	external: bool,
	/// Where another crate names it.
	#[serde(skip_serializing_if = "Option::is_none")]
	named_by_other_crate: Option<String>,
	/// Where its own crate names it.
	#[serde(skip_serializing_if = "Option::is_none")]
	named_by_own_crate: Option<String>,
}

pub const UNCHECKED: &str = "Only the cargo targets, features, and host platform checked here: \
	doctests, other features and platforms, and crates outside the audited workspaces are unseen.";

/// The audit's inputs, for the report.
pub struct Scope<'a> {
	pub root: &'a Path,
	pub workspaces: Vec<&'a Path>,
	pub cargo_args: &'a [String],
}

/// Write `report.json`, `report.txt`, and `narrow.patch` into `out`, and return the
/// summary.
pub fn write(out: &Path, scope: &Scope, audit: &Audit, validation: &Validation) -> Result<String, String> {
	let mut lines = Lines::default();
	let mut tokens = Vec::new();
	for (t, token) in audit.tokens.iter().enumerate() {
		let outcome = match token.verdict {
			Verdict::Narrow if validation.narrowed.contains(&t) => Outcome::Narrowed,
			Verdict::Narrow => Outcome::Restored,
			Verdict::External => Outcome::External,
			Verdict::CrossCrate => Outcome::CrossCrate,
			Verdict::Symbol => Outcome::Symbol,
			Verdict::Generated => Outcome::Generated,
			Verdict::Embedded => Outcome::Embedded,
		};
		let restored = validation.restored.get(&t).map(|why| match why {
			Restored::Diagnostic(Some(code), message) => format!("rustc {code}: {message}"),
			Restored::Diagnostic(None, message) => format!("rustc: {message}"),
			Restored::Resolution(place) => {
				format!("a name resolves differently at {}", lines.at(scope.root, place))
			}
			Restored::Expanded(path) => format!("a macro it reaches makes {path} differently"),
			Restored::Embedded(file) => format!("the narrowed build embeds {}", relative(scope.root, file).display()),
		});
		let items = token
			.defs
			.iter()
			.map(|&d| {
				let def = &audit.defs[d];
				let evidence = |e: &Option<Evidence>, lines: &mut Lines| {
					e.as_ref().map(|e| {
						let from = &audit.defs[e.from].path;
						match &e.site {
							Some(site) => format!("{from} at {}", lines.at(scope.root, site)),
							None => from.clone(),
						}
					})
				};
				ItemReport {
					path: def.path.clone(),
					kind: def.kind.clone(),
					crate_name: def.crate_name.clone(),
					category: def.category,
					exported: def.exported,
					external: def.external,
					named_by_other_crate: evidence(&def.cross, &mut lines),
					named_by_own_crate: evidence(&def.local, &mut lines),
				}
			})
			.collect();
		let (line, column) = lines.line_col(&token.place);
		tokens.push(TokenReport {
			file: relative(scope.root, &token.place.file),
			line,
			column,
			offset: token.place.lo,
			outcome,
			category: category(audit, token),
			restored,
			items,
		});
	}
	let mut summary = BTreeMap::new();
	for token in &tokens {
		let outcome = serde_json::to_value(token.outcome).expect("serializes");
		let category = serde_json::to_value(token.category).expect("serializes");
		let key = format!(
			"{} {}",
			outcome.as_str().unwrap_or_default(),
			category.as_str().unwrap_or_default()
		);
		*summary.entry(key).or_default() += 1;
	}
	let report = Report {
		root: scope.root,
		workspaces: scope.workspaces.clone(),
		cargo_args: scope.cargo_args,
		unchecked: UNCHECKED,
		rounds: validation.rounds,
		summary: summary.clone(),
		tokens,
	};
	let json = serde_json::to_string_pretty(&report).expect("report serializes");
	write_file(&out.join("report.json"), &json)?;
	write_file(&out.join("report.txt"), &text(&report))?;
	write_file(&out.join("narrow.patch"), &patch(scope.root, audit, validation)?)?;
	let mut text = String::new();
	for (key, count) in &summary {
		let _ = writeln!(text, "{count:>7} {key}");
	}
	Ok(text)
}

/// The most a token's definitions are reached.
fn category(audit: &Audit, token: &Token) -> Category {
	token
		.defs
		.iter()
		.map(|&d| audit.defs[d].category)
		.max()
		.unwrap_or(Category::Unreached)
}

/// A section of the text report: its title, and which tokens it lists.
type Section = (&'static str, fn(&TokenReport) -> bool);

fn text(report: &Report) -> String {
	let mut text = String::new();
	let _ = writeln!(
		text,
		"rust2-pub: {} `pub` tokens under {}",
		report.tokens.len(),
		report.root.display()
	);
	let _ = writeln!(text, "{}", report.unchecked);
	let sections: [Section; 5] = [
		(
			"Narrowed to pub(crate), all together, checked in the selected targets with every name resolving alike (narrow.patch)",
			|t| t.outcome == Outcome::Narrowed,
		),
		("Proposed, but the check restored", |t| t.outcome == Outcome::Restored),
		(
			"The API of a library crates outside the audit may use, which nothing in the audit runs",
			|t| t.outcome == Outcome::External && t.category != Category::CrossCrate,
		),
		("Named by another crate only from tests or unreached code", |t| {
			t.outcome == Outcome::CrossCrate && t.category != Category::CrossCrate
		}),
		(
			"In a file a crate embeds as bytes, with `include_bytes!` or `include_str!`, which narrowing would change",
			|t| t.outcome == Outcome::Embedded,
		),
	];
	for (title, filter) in sections {
		let tokens: Vec<&TokenReport> = report.tokens.iter().filter(|t| filter(t)).collect();
		let _ = writeln!(text, "\n## {title}: {}\n", tokens.len());
		for token in tokens {
			let category = serde_json::to_value(token.category).expect("serializes");
			let _ = write!(
				text,
				"{}:{}:{}: {}",
				token.file.display(),
				token.line,
				token.column,
				category.as_str().unwrap_or_default()
			);
			if let Some(why) = &token.restored {
				let _ = write!(text, " (restored: {why})");
			}
			let _ = writeln!(text);
			for item in &token.items {
				let _ = write!(text, "    {} {}", item.kind, item.path);
				if let Some(by) = &item.named_by_other_crate {
					let _ = write!(text, "; another crate: {by}");
				}
				if let Some(by) = &item.named_by_own_crate {
					let _ = write!(text, "; its crate: {by}");
				}
				let _ = writeln!(text);
			}
		}
	}
	text
}

/// The narrowed tokens as a unified diff, relative to `root`.
fn patch(root: &Path, audit: &Audit, validation: &Validation) -> Result<String, String> {
	let mut by_file: BTreeMap<&Path, Vec<u32>> = BTreeMap::new();
	for &t in &validation.narrowed {
		let place = &audit.tokens[t].place;
		by_file.entry(&place.file).or_default().push(place.lo);
	}
	let mut patch = String::new();
	for (file, mut offsets) in by_file {
		offsets.sort();
		let text = std::fs::read_to_string(file).map_err(|err| format!("reading {}: {err}", file.display()))?;
		let mut narrowed = text.clone();
		for &offset in offsets.iter().rev() {
			narrowed.replace_range(offset as usize..offset as usize + 3, "pub(crate)");
		}
		let name = relative(root, file);
		let _ = writeln!(patch, "--- a/{}\n+++ b/{}", name.display(), name.display());
		patch.push_str(&hunks(&text, &narrowed));
	}
	Ok(patch)
}

/// Unified diff hunks of two texts with the same lines but some changed in place.
fn hunks(before: &str, after: &str) -> String {
	let before: Vec<&str> = before.split_inclusive('\n').collect();
	let after: Vec<&str> = after.split_inclusive('\n').collect();
	assert_eq!(before.len(), after.len(), "narrowing keeps every line");
	let changed: Vec<usize> = (0..before.len()).filter(|&i| before[i] != after[i]).collect();
	let mut out = String::new();
	let mut i = 0;
	while i < changed.len() {
		let start = changed[i].saturating_sub(3);
		let mut end = changed[i];
		while i < changed.len() && changed[i] <= end + 6 {
			end = changed[i];
			i += 1;
		}
		let end = (end + 4).min(before.len());
		let count = end - start;
		let _ = writeln!(out, "@@ -{},{count} +{},{count} @@", start + 1, start + 1);
		for line in start..end {
			let newline = |l: &str| {
				if l.ends_with('\n') {
					""
				} else {
					"\n\\ No newline at end of file\n"
				}
			};
			if before[line] == after[line] {
				let _ = write!(out, " {}{}", before[line], newline(before[line]));
			} else {
				let _ = write!(out, "-{}{}", before[line], newline(before[line]));
				let _ = write!(out, "+{}{}", after[line], newline(after[line]));
			}
		}
	}
	// Removed and added lines come in pairs per line; reorder each run into the
	// removals, then the additions, as a diff lists them.
	reorder(&out)
}

/// Group each run of alternating `-`/`+` lines into its `-` lines, then its `+` lines.
fn reorder(diff: &str) -> String {
	let mut out = String::new();
	let mut minus = String::new();
	let mut plus = String::new();
	let mut last_sign = ' ';
	for line in diff.split_inclusive('\n') {
		let sign = line.chars().next().unwrap_or(' ');
		match sign {
			'-' => {
				minus.push_str(line);
				last_sign = '-';
			}
			'+' => {
				plus.push_str(line);
				last_sign = '+';
			}
			'\\' => {
				if last_sign == '-' { &mut minus } else { &mut plus }.push_str(line);
			}
			_ => {
				out.push_str(&minus);
				out.push_str(&plus);
				minus.clear();
				plus.clear();
				out.push_str(line);
			}
		}
	}
	out.push_str(&minus);
	out.push_str(&plus);
	out
}

/// Each file's line starts, read once.
#[derive(Default)]
struct Lines(HashMap<PathBuf, Vec<u32>>);

impl Lines {
	fn line_col(&mut self, place: &Place) -> (usize, usize) {
		let starts = self.0.entry(place.file.clone()).or_insert_with(|| {
			let text = std::fs::read(&place.file).unwrap_or_default();
			std::iter::once(0)
				.chain(
					text.iter()
						.enumerate()
						.filter(|(_, b)| **b == b'\n')
						.map(|(i, _)| i as u32 + 1),
				)
				.collect()
		});
		let line = starts.partition_point(|&s| s <= place.lo);
		(line, (place.lo - starts[line - 1]) as usize + 1)
	}

	fn at(&mut self, root: &Path, place: &Place) -> String {
		let (line, column) = self.line_col(place);
		format!("{}:{line}:{column}", relative(root, &place.file).display())
	}
}

fn relative(root: &Path, file: &Path) -> PathBuf {
	file.strip_prefix(root).unwrap_or(file).to_path_buf()
}

fn write_file(path: &Path, text: &str) -> Result<(), String> {
	std::fs::write(path, text).map_err(|err| format!("writing {}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
	use super::hunks;

	#[test]
	fn hunks_merge_nearby_lines_and_keep_context() {
		let before = "a\npub fn x() {}\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\npub fn y() {}";
		let after = before.replace("pub fn", "pub(crate) fn");
		let diff = hunks(before, &after);
		assert_eq!(
			diff,
			"@@ -1,5 +1,5 @@\n a\n-pub fn x() {}\n+pub(crate) fn x() {}\n b\n c\n d\n\
			 @@ -11,4 +11,4 @@\n j\n k\n l\n-pub fn y() {}\n\\ No newline at end of file\n+pub(crate) fn y() {}\n\\ No newline at end of file\n"
		);
	}
}
