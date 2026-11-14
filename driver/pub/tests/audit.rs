//! Audit a copy of the fixture workspace, then check what it found against what each
//! item's comment says, and that the narrowed copy behaves as the original: its binary
//! prints the same, and its tests pass.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixture");

#[test]
fn fixture() {
	let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("audit-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	let original = dir.join("original");
	copy(Path::new(FIXTURE), &original);
	let out = dir.join("out");
	let status = Command::new(env!("CARGO_BIN_EXE_rust2-pub"))
		.arg("--out")
		.arg(&out)
		.current_dir(&original)
		.status()
		.unwrap();
	assert!(status.success());

	let report: Value = serde_json::from_slice(&std::fs::read(out.join("report.json")).unwrap()).unwrap();
	let mut found = BTreeMap::new();
	for token in report["tokens"].as_array().unwrap() {
		for item in token["items"].as_array().unwrap() {
			let path = item["path"].as_str().unwrap().to_string();
			found.insert(
				path,
				(
					token["outcome"].as_str().unwrap().to_string(),
					item["category"].as_str().unwrap().to_string(),
				),
			);
		}
	}
	let expect = |path: &str, outcome: &str, category: &str| {
		assert_eq!(
			found.get(path).map(|(o, c)| (o.as_str(), c.as_str())),
			Some((outcome, category)),
			"{path}"
		);
	};
	expect("Live::hidden", "narrowed", "crate-only");
	expect("on_drop", "narrowed", "crate-only");
	expect("helper", "narrowed", "crate-only");
	expect("unit_tested", "narrowed", "test-only");
	expect("cycle_a", "narrowed", "unreached");
	expect("cycle_b", "narrowed", "unreached");
	expect("inner::reexported_dead", "narrowed", "unreached");
	expect("crate::{use inner::reexported_dead}", "narrowed", "unreached");
	expect("Outer::is_empty", "narrowed", "unreached");
	expect("Leaked", "restored", "crate-only");
	expect("integration_tested", "cross-crate", "test-only");
	// facade is a library nothing in the workspace depends on: its API, and what that API
	// exposes of api, stays public.
	expect("unused", "external", "unreached");
	expect("crate::{use api::Exposed}", "external", "unreached");
	expect("Exposed::field", "external", "unreached");
	expect("Exposed::method", "external", "unreached");
	expect("Returned::field", "external", "unreached");
	// An external trait's items are its API, with what they expose.
	expect("Area", "external", "unreached");
	expect("Area::value", "external", "unreached");
	expect("Area::doubled", "external", "unreached");
	// A glob that is external API, as facade's re-export through it makes it, exposes
	// what its module exports, not all it supplies.
	expect("outer::{use parts}", "cross-crate", "unreached");
	expect("outer::parts::Part", "cross-crate", "unreached");
	expect("outer::parts::SIZE", "narrowed", "unreached");
	// `pub extern crate` exposes the whole crate.
	expect("exposed", "external", "unreached");
	expect("extra", "external", "unreached");
	// A macro that reads a token it is given makes its output differently, so the token
	// stays, however deep the macro: `stringify!`, a derive, a macro a derive calls, and an
	// attribute.
	expect("label", "restored", "crate-only");
	expect("Labeled", "restored", "crate-only");
	expect("NestedLabeled", "restored", "crate-only");
	expect("tagged", "restored", "crate-only");
	// One that only passes it on as the visibility makes the same, so the token narrows.
	expect("passed_through", "narrowed", "crate-only");
	expect("kept_through", "narrowed", "crate-only");
	expect("Derived", "narrowed", "unreached");
	// Narrowing shifts the columns after it on its line, but no line.
	expect("at_column", "restored", "crate-only");
	expect("at_line", "narrowed", "crate-only");
	// A macro that changes only which binding a name picks.
	expect("shadowed", "restored", "crate-only");
	// A file a crate embeds as bytes changes with any narrowing in it.
	expect("embedded::embedded", "embedded", "crate-only");
	// A file only the narrowed build embeds keeps every `pub` in it.
	expect("payload::payload", "restored", "crate-only");
	expect("payload_text", "narrowed", "crate-only");
	// An import is as live as what it imports, with the re-export it goes through, module
	// or not, but makes nothing it is in live.
	expect("crate::{use inner::reexported}", "cross-crate", "cross-crate");
	expect("crate::{use hidden::m}", "cross-crate", "cross-crate");
	expect("hidden::m::through_alias", "cross-crate", "cross-crate");
	expect("only_dead", "cross-crate", "unreached");
	// A glob supplies only what is named through it.
	expect(
		"prelude::{use crate::inner::through_glob}",
		"cross-crate",
		"cross-crate",
	);
	expect("inner::through_glob", "cross-crate", "cross-crate");
	expect("prelude::{use crate::inner::past_glob}", "narrowed", "unreached");
	expect("inner::past_glob", "narrowed", "unreached");
	// What a glob supplies from outside the audit, or through another crate's glob.
	expect("prelude::{use std::mem::size_of}", "cross-crate", "cross-crate");
	expect("prelude::{use crate::deeper}", "cross-crate", "cross-crate");
	expect("deeper::via_nested_glob", "cross-crate", "cross-crate");
	// An impl of an imported trait from outside the audit needs only its type.
	expect("shown_part", "narrowed", "crate-only");
	// So does a glob of a module or enum of its own crate.
	expect("tools::tool", "narrowed", "crate-only");
	expect("crate::{use tools}", "narrowed", "crate-only");
	expect("crate::{use Mode}", "narrowed", "crate-only");
	// One live name it supplies revives no other.
	expect("inner::past_live_glob", "cross-crate", "unreached");
	expect(
		"prelude::{use crate::inner::past_live_glob}",
		"cross-crate",
		"unreached",
	);
	// A list holds its paths' `pub`, live once one of theirs is.
	expect("crate::{use listed}", "narrowed", "crate-only");
	expect("listed::listed_dead", "narrowed", "unreached");
	// A token in an exported macro goes wherever its callers outside the audit expand it.
	expect("made", "external", "crate-only");
	// Offsets on disk, past CRLFs rustc reads as LF.
	expect("crlf::after_crlf", "narrowed", "unreached");
	for kept in [
		"Action",
		"Live",
		"Live::value",
		"Live::new",
		"drops",
		"generic",
		"dynamic",
		"expanded_used",
		"expanded_dead",
		"inner::reexported",
		"crate::{use inner::reexported}",
		"prelude",
		"prelude::globbed",
		"Source",
		"Counter",
		"Counter::0",
		"Ticket",
		"Ticket::0",
		"Outer",
		"Outer::new",
		"Outer::len",
		"Holder",
		"Holds",
		"Exposed",
		"Returned",
		"Settings",
		"Settings::named",
		"Settings::defaulted",
		"used",
		"Shape",
		"crlf",
		"crlf::spaced",
	] {
		assert_eq!(found[kept].0, "cross-crate", "{kept}");
	}

	// The narrowed copy prints what the original does, and passes its tests.
	let narrowed = dir.join("narrowed");
	copy(Path::new(FIXTURE), &narrowed);
	apply(&narrowed, &out.join("narrow.patch"));
	let before = run(&original);
	assert_eq!(run(&narrowed), before);
	assert!(cargo(&narrowed).arg("test").status().unwrap().success());

	// Narrowing `Outer::len` too still compiles, but calls the slice's `len` instead:
	// what keeps it public is that app's call resolves to it, not a compile error.
	let lib = narrowed.join("api/src/lib.rs");
	let text = std::fs::read_to_string(&lib).unwrap();
	std::fs::write(&lib, text.replace("pub fn len(", "pub(crate) fn len(")).unwrap();
	assert_eq!(run(&narrowed), before.replace("len 99", "len 3"));
}

fn cargo(dir: &Path) -> Command {
	let mut cmd = Command::new(env!("CARGO"));
	cmd.current_dir(dir).env("CARGO_TARGET_DIR", dir.join("target"));
	cmd
}

fn run(dir: &Path) -> String {
	let output = cargo(dir).args(["run", "--quiet", "-p", "app"]).output().unwrap();
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	String::from_utf8(output.stdout).unwrap()
}

fn apply(dir: &Path, patch: &Path) {
	let status = Command::new("git")
		.arg("apply")
		.arg(patch)
		.current_dir(dir)
		.status()
		.unwrap();
	assert!(status.success());
}

fn copy(from: &Path, to: &Path) {
	std::fs::create_dir_all(to).unwrap();
	for entry in std::fs::read_dir(from).unwrap() {
		let entry = entry.unwrap();
		let target = to.join(entry.file_name());
		if entry.file_type().unwrap().is_dir() {
			if entry.file_name() != "target" {
				copy(&entry.path(), &target);
			}
		} else {
			std::fs::copy(entry.path(), target).unwrap();
		}
	}
}
