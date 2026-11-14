//! Narrows every proposed token at once, checks the workspaces as cargo does, and
//! restores the tokens rustc's errors point at until the whole set checks, and every
//! name in the source still resolves to what it did.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::analyze::{Audit, Place};
use crate::cargo::{Diagnostic, DiagnosticSpan, Workspace};
use crate::facts::{Edits, Hash, Span, Unit};

/// Why a proposed token stays `pub`.
pub enum Restored {
	/// rustc's diagnostic pointed at it: its code, if any, and message.
	Diagnostic(Option<String>, String),
	/// Narrowing it changed what a name resolves to, at this place.
	Resolution(Place),
	/// Narrowing it changed what a macro it reached makes: this item, by path.
	Expanded(String),
	/// The narrowed build embeds its file as bytes, which applying the narrowing changes.
	Embedded(PathBuf),
}

/// What the validation ends with.
pub struct Validation {
	/// The tokens narrowed together, by index in [`Audit::tokens`].
	pub narrowed: BTreeSet<usize>,
	pub restored: BTreeMap<usize, Restored>,
	/// How many checks it took.
	pub rounds: usize,
}

/// The audit's run: where it writes, and what it checks.
pub struct Run<'a> {
	pub workspaces: &'a [Workspace],
	pub args: &'a [String],
	pub out: &'a Path,
	pub local: &'a Path,
}

impl Run<'_> {
	/// Check every workspace, narrowed by `edits` if given, into `facts`.
	pub fn check(&self, name: &str, edits: Option<&Edits>) -> Result<(bool, Vec<Diagnostic>, Vec<Unit>), String> {
		let facts = self.out.join("facts").join(name);
		if facts.exists() {
			std::fs::remove_dir_all(&facts).map_err(|err| format!("clearing {}: {err}", facts.display()))?;
		}
		std::fs::create_dir_all(&facts).map_err(|err| format!("creating {}: {err}", facts.display()))?;
		let edits_path = match edits {
			Some(edits) => {
				let path = self.out.join("facts").join(format!("{name}-edits.json"));
				let json = serde_json::to_vec(edits).expect("edits serialize");
				std::fs::write(&path, json).map_err(|err| format!("writing {}: {err}", path.display()))?;
				Some(path)
			}
			None => None,
		};
		let mut success = true;
		let mut diagnostics = Vec::new();
		let mut compiled = 0;
		for workspace in self.workspaces {
			let check = workspace.check(self.args, &facts, self.local, edits_path.as_deref())?;
			success &= check.success;
			compiled += check.compiled;
			for mut diagnostic in check.diagnostics {
				absolute(&mut diagnostic, &workspace.root);
				diagnostics.push(diagnostic);
			}
		}
		let units = crate::cargo::units(&facts)?;
		// A crate cargo compiled without this binary, such as through a build cache,
		// leaves the graph without its references.
		if success && units.len() != compiled {
			return Err(format!(
				"cargo compiled {compiled} local crates, but {} ran through this binary; does a cache or wrapper bypass RUSTC_WRAPPER?",
				units.len()
			));
		}
		Ok((success, diagnostics, units))
	}
}

/// Narrow every token the audit proposes, restoring until the workspaces check alike.
pub fn validate(run: &Run, audit: &Audit, original: &[Unit]) -> Result<Validation, String> {
	let mut active: BTreeSet<usize> = (0..audit.tokens.len())
		.filter(|&t| audit.tokens[t].verdict == crate::analyze::Verdict::Narrow)
		.collect();
	let mut restored = BTreeMap::new();
	let expected = sites(original);
	let mut rounds = 0;
	while !active.is_empty() {
		rounds += 1;
		let edits = edits(audit, &active);
		eprintln!("rust2-pub: checking {} narrowed tokens, round {rounds}", active.len());
		let (success, diagnostics, units) = run.check(&format!("round-{rounds}"), Some(&edits))?;
		let mut restore = BTreeMap::new();
		if !success {
			let mut unattributed = Vec::new();
			for diagnostic in diagnostics.iter().filter(|d| fails(d)) {
				let blamed = blame(audit, &active, &edits, diagnostic);
				if blamed.is_empty() && diagnostic.level == "error" {
					unattributed.push(diagnostic);
				}
				for token in blamed {
					restore.entry(token).or_insert_with(|| restoring(diagnostic));
				}
			}
			if restore.is_empty() {
				let rendered: Vec<&str> = unattributed.iter().filter_map(|d| d.rendered.as_deref()).collect();
				return Err(format!(
					"the narrowed workspaces fail to check with errors no narrowed `pub` explains:\n{}",
					rendered.join("")
				));
			}
		} else {
			// What no narrowed token explains fails the run, once nothing else is left to
			// restore that might.
			let mut unexplained = Vec::new();
			let actual = sites(&units);
			for (place, before, after) in differences(&expected, &actual, &edits)? {
				let mut blamed = BTreeSet::new();
				for hash in before.iter().chain(&after) {
					if let Some(&def) = audit.hashes.get(hash) {
						blamed.extend(active.iter().copied().filter(|&t| audit.tokens[t].defs.contains(&def)));
					}
				}
				if blamed.is_empty() {
					unexplained.push(format!(
						"{}: byte {}: narrowing changed what this name resolves to",
						place.file.display(),
						place.lo
					));
				}
				for token in blamed {
					restore
						.entry(token)
						.or_insert_with(|| Restored::Resolution(place.clone()));
				}
			}
			// Printing leaves visibility out, so an item prints differently only where a
			// macro read a narrowed token: as its input, as `stringify!` does, or by
			// position, as `column!` does.
			for (hash, blamed) in printed(audit, &active, &edits, original, &units) {
				let path = audit
					.hashes
					.get(&hash)
					.map_or_else(|| format!("an item hashed {hash:x?}"), |&d| audit.defs[d].path.clone());
				if blamed.is_empty() {
					unexplained.push(format!("narrowing changed what macros make of {path}"));
				}
				for token in blamed {
					restore.entry(token).or_insert_with(|| Restored::Expanded(path.clone()));
				}
			}
			// A file the narrowed build first embeds, as a macro's narrowed expansion may,
			// would embed the narrowing once applied.
			for unit in &units {
				for file in &unit.embedded {
					for &t in active.iter().filter(|&&t| &audit.tokens[t].place.file == file) {
						restore.entry(t).or_insert_with(|| Restored::Embedded(file.clone()));
					}
				}
			}
			// Narrowing must not add warnings either: the lints that make public API
			// name a private type warn, rather than fail, but `-D warnings` fails them.
			for diagnostic in diagnostics.iter().filter(|d| fails(d)) {
				for token in blame(audit, &active, &edits, diagnostic) {
					restore.entry(token).or_insert_with(|| restoring(diagnostic));
				}
			}
			if restore.is_empty() {
				if !unexplained.is_empty() {
					return Err(format!(
						"narrowing changed the build in ways no narrowed `pub` explains:\n{}",
						unexplained.join("\n")
					));
				}
				break;
			}
		}
		for (token, why) in restore {
			active.remove(&token);
			restored.insert(token, why);
		}
	}
	Ok(Validation {
		narrowed: active,
		restored,
		rounds,
	})
}

/// Each item the narrowed `units` print differently from the `original`, make, or no
/// longer make, with the active tokens a macro it came from read: those on its call's line
/// up to its end, and an attribute's or derive's input item's. Failing those, the tokens on
/// its own lines, which a `column!` there reads.
fn printed(
	audit: &Audit,
	active: &BTreeSet<usize>,
	edits: &Edits,
	original: &[Unit],
	units: &[Unit],
) -> BTreeMap<Hash, BTreeSet<usize>> {
	// Each item's outputs, and where each was printed, by unit and index: a crate shared
	// by workspaces compiles once in each.
	type Outputs = HashMap<Hash, (Vec<u64>, Vec<(usize, usize)>)>;
	fn outputs(units: &[Unit]) -> Outputs {
		let mut outputs = Outputs::new();
		for (u, unit) in units.iter().enumerate() {
			for (p, printed) in unit.printed.iter().enumerate() {
				let entry = outputs.entry(printed.hash).or_default();
				entry.0.push(printed.output);
				entry.1.push((u, p));
			}
		}
		for (list, _) in outputs.values_mut() {
			list.sort();
		}
		outputs
	}
	let before = outputs(original);
	let after = outputs(units);
	let mut by_def: HashMap<usize, Vec<usize>> = HashMap::new();
	for &t in active {
		for &d in &audit.tokens[t].defs {
			by_def.entry(d).or_default().push(t);
		}
	}
	let mut changed = BTreeMap::new();
	let hashes: BTreeSet<&Hash> = before.keys().chain(after.keys()).collect();
	for hash in hashes {
		let (b, a) = (before.get(hash), after.get(hash));
		if b.map(|b| &b.0) == a.map(|a| &a.0) {
			continue;
		}
		let mut blamed = BTreeSet::new();
		let printed = [(original, b, None), (units, a, Some(edits))];
		for &(units, printed, narrowed) in &printed {
			for &(u, p) in printed.map(|p| p.1.as_slice()).unwrap_or_default() {
				read(
					audit,
					active,
					&by_def,
					&units[u],
					&units[u].printed[p].expansions,
					narrowed,
					&mut blamed,
				);
			}
		}
		if blamed.is_empty() {
			for &(units, printed, narrowed) in &printed {
				for &(u, p) in printed.map(|p| p.1.as_slice()).unwrap_or_default() {
					if let Some(lines) = units[u].printed[p].lines {
						within(audit, active, &units[u], lines, narrowed, &mut blamed);
					}
				}
			}
		}
		changed.insert(*hash, blamed);
	}
	changed
}

/// Add to `blamed` the active tokens in `unit`'s `span`; `edits` narrowed its offsets, if
/// given.
fn within(
	audit: &Audit,
	active: &BTreeSet<usize>,
	unit: &Unit,
	span: Span,
	edits: Option<&Edits>,
	blamed: &mut BTreeSet<usize>,
) {
	let file = &unit.files[span.file as usize];
	let (lo, hi) = match edits {
		Some(edits) => (original(edits, file, span.lo), original(edits, file, span.hi)),
		None => (span.lo, span.hi),
	};
	blamed.extend(active.iter().copied().filter(|&t| {
		let place = &audit.tokens[t].place;
		&place.file == file && lo <= place.lo && place.lo < hi
	}));
}

/// Add to `blamed` the active tokens the `expansions` of `unit`, or those they came from,
/// read; `edits` narrowed the unit's offsets, if given.
fn read(
	audit: &Audit,
	active: &BTreeSet<usize>,
	by_def: &HashMap<usize, Vec<usize>>,
	unit: &Unit,
	expansions: &[u32],
	edits: Option<&Edits>,
	blamed: &mut BTreeSet<usize>,
) {
	let mut seen = BTreeSet::new();
	let mut inputs = BTreeSet::new();
	let mut stack = expansions.to_vec();
	while let Some(e) = stack.pop() {
		if !seen.insert(e) {
			continue;
		}
		let expansion = &unit.expansions[e as usize];
		stack.extend(expansion.parent);
		inputs.extend(expansion.input);
		if let Some(call) = expansion.call {
			within(audit, active, unit, call, edits, blamed);
		}
	}
	if inputs.is_empty() {
		return;
	}
	// The input item's tokens are those of what the expansion carrying it made, and what
	// macros called from that made in turn.
	let carried = |mut at: Option<u32>| {
		while let Some(e) = at {
			if inputs.contains(&e) {
				return true;
			}
			at = unit.expansions[e as usize].parent;
		}
		false
	};
	for def in &unit.defs {
		if carried(def.expansion)
			&& let Some(d) = audit.hashes.get(&def.hash)
			&& let Some(tokens) = by_def.get(d)
		{
			blamed.extend(tokens);
		}
	}
}

fn restoring(diagnostic: &Diagnostic) -> Restored {
	Restored::Diagnostic(
		diagnostic.code.as_ref().map(|c| c.code.clone()),
		diagnostic.message.clone(),
	)
}

/// Whether narrowing must not cause `diagnostic`: an error, or a lint about a private
/// type in a public interface.
fn fails(diagnostic: &Diagnostic) -> bool {
	diagnostic.level == "error"
		|| diagnostic
			.code
			.as_ref()
			.is_some_and(|c| matches!(c.code.as_str(), "private_interfaces" | "private_bounds"))
}

/// The tokens' offsets, by file.
pub fn edits(audit: &Audit, active: &BTreeSet<usize>) -> Edits {
	let mut edits = Edits::new();
	for &t in active {
		let place = &audit.tokens[t].place;
		edits.entry(place.file.clone()).or_default().push(place.lo);
	}
	for offsets in edits.values_mut() {
		offsets.sort();
	}
	edits
}

/// Where `pos`, in `file` read narrowed by `edits`, is in the original.
fn original(edits: &Edits, file: &Path, pos: u32) -> u32 {
	let Some(offsets) = edits.get(file) else { return pos };
	let mut shift = 0;
	for &offset in offsets {
		let narrowed = offset + shift;
		if pos < narrowed {
			break;
		}
		if pos < narrowed + 10 {
			return offset;
		}
		shift += 7;
	}
	pos - shift
}

/// The active tokens a diagnostic's spans point at: for each span, the narrowest of the
/// tokens whose definition contains it.
fn blame(audit: &Audit, active: &BTreeSet<usize>, edits: &Edits, diagnostic: &Diagnostic) -> BTreeSet<usize> {
	let mut spans = Vec::new();
	collect_spans(diagnostic, &mut spans);
	let mut blamed = BTreeSet::new();
	for span in spans {
		let lo = original(edits, &span.file_name, span.byte_start);
		let hi = original(edits, &span.file_name, span.byte_end).max(lo);
		let mut best: Option<(u32, usize)> = None;
		for &t in active {
			let token = &audit.tokens[t];
			let ranges = token
				.defs
				.iter()
				.filter_map(|&d| audit.defs[d].span.as_ref())
				.chain([&token.place]);
			for range in ranges {
				if range.file == span.file_name && range.lo <= hi && lo <= range.hi.max(range.lo + 3) {
					let width = range.hi - range.lo;
					if best.is_none_or(|(w, _)| width < w) {
						best = Some((width, t));
					}
				}
			}
		}
		blamed.extend(best.map(|(_, t)| t));
	}
	blamed
}

fn collect_spans<'a>(diagnostic: &'a Diagnostic, out: &mut Vec<&'a DiagnosticSpan>) {
	for span in &diagnostic.spans {
		let mut span = span;
		out.push(span);
		while let Some(expansion) = &span.expansion {
			out.push(&expansion.def_site_span);
			span = &expansion.span;
			out.push(span);
		}
	}
	for child in &diagnostic.children {
		collect_spans(child, out);
	}
}

/// Make the diagnostic's file names absolute: rustc names a workspace member's files
/// from the workspace's root.
fn absolute(diagnostic: &mut Diagnostic, root: &Path) {
	fn absolute_span(span: &mut DiagnosticSpan, root: &Path) {
		let path = root.join(&span.file_name);
		span.file_name = std::fs::canonicalize(&path).unwrap_or(path);
		if let Some(expansion) = &mut span.expansion {
			absolute_span(&mut expansion.span, root);
			absolute_span(&mut expansion.def_site_span, root);
		}
	}
	for s in &mut diagnostic.spans {
		absolute_span(s, root);
	}
	for child in &mut diagnostic.children {
		absolute(child, root);
	}
}

/// Each compilation's resolved names: by crate, every place and what it resolves to.
type Sites = HashMap<u64, Vec<(PathBuf, u32, u32, Hash)>>;

fn sites(units: &[Unit]) -> Sites {
	let mut sites: Sites = HashMap::new();
	for unit in units {
		let list = sites.entry(unit.crate_id).or_default();
		list.extend(
			unit.sites
				.iter()
				.map(|s| (unit.files[s.span.file as usize].clone(), s.span.lo, s.span.hi, s.to)),
		);
	}
	sites
}

/// Each place whose resolutions differ between the original and the narrowed build: what
/// it resolved to before, and after.
/// A place, and what it resolved to before narrowing and after.
type Difference = (Place, Vec<Hash>, Vec<Hash>);

fn differences(expected: &Sites, actual: &Sites, edits: &Edits) -> Result<Vec<Difference>, String> {
	let mut differences = Vec::new();
	if expected.len() != actual.len() || expected.keys().any(|k| !actual.contains_key(k)) {
		return Err("the narrowed check compiled other crates than the original; is the workspace changing?".into());
	}
	for (crate_id, before) in expected {
		let mut before_by: BTreeMap<Place, Vec<Hash>> = BTreeMap::new();
		for (file, lo, hi, to) in before {
			let place = Place {
				file: file.clone(),
				lo: *lo,
				hi: *hi,
			};
			before_by.entry(place).or_default().push(*to);
		}
		let mut after_by: BTreeMap<Place, Vec<Hash>> = BTreeMap::new();
		for (file, lo, hi, to) in &actual[crate_id] {
			let place = Place {
				file: file.clone(),
				lo: original(edits, file, *lo),
				hi: original(edits, file, *hi),
			};
			after_by.entry(place).or_default().push(*to);
		}
		let places: BTreeSet<&Place> = before_by.keys().chain(after_by.keys()).collect();
		for place in places {
			let mut b = before_by.get(place).cloned().unwrap_or_default();
			let mut a = after_by.get(place).cloned().unwrap_or_default();
			b.sort();
			a.sort();
			if a != b {
				differences.push((place.clone(), b, a));
			}
		}
	}
	Ok(differences)
}

#[cfg(test)]
mod tests {
	use std::path::PathBuf;

	use super::{Edits, Sites, differences, original};

	#[test]
	fn a_name_resolving_elsewhere_differs_and_a_shifted_one_does_not() {
		let file = PathBuf::from("/x.rs");
		// `pub fn len` narrowed at 0: the call at 40 moves to 47 and still resolves to
		// [1, 1]; the call at 60 moves to 67 and now resolves to [2, 2].
		let edits = Edits::from([(file.clone(), vec![0])]);
		let expected = Sites::from([(7, vec![(file.clone(), 40, 43, [1, 1]), (file.clone(), 60, 63, [1, 1])])]);
		let actual = Sites::from([(7, vec![(file.clone(), 47, 50, [1, 1]), (file.clone(), 67, 70, [2, 2])])]);
		let found = differences(&expected, &actual, &edits).unwrap();
		assert_eq!(found.len(), 1);
		let (place, before, after) = &found[0];
		assert_eq!((place.lo, place.hi), (60, 63));
		assert_eq!((before.as_slice(), after.as_slice()), (&[[1, 1]][..], &[[2, 2]][..]));
		// A crate the narrowed check did not compile is an error, not a pass.
		assert!(differences(&expected, &Sites::new(), &edits).is_err());
	}

	#[test]
	fn narrowed_offsets_map_back() {
		// "pub fn a(); pub fn b();" narrowed at 0 and 12.
		let file = PathBuf::from("/x.rs");
		let edits = Edits::from([(file.clone(), vec![0, 12])]);
		// Inside the first `pub(crate)`, after it, inside the second, and after both.
		assert_eq!(original(&edits, &file, 4), 0);
		assert_eq!(original(&edits, &file, 11), 4);
		assert_eq!(original(&edits, &file, 19), 12);
		assert_eq!(original(&edits, &file, 30), 16);
		assert_eq!(original(&edits, &PathBuf::from("/y.rs"), 30), 30);
	}
}
