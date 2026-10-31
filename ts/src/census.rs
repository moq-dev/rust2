//! The census: every construct translation refuses in a crate, ranked, so the next
//! feature to build is picked from numbers.
//!
//! A refusal's construct is its message with each quoted type or path blanked. Those
//! types and paths, without their generic arguments, name the mapping to add, and the
//! type or module each belongs to is its API.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::path::Path;

/// What a census found.
pub struct Census {
	/// Function instances translated or refused.
	pub functions: usize,
	/// Generic functions no instance reached, lowered over their own type parameters.
	pub polymorphic: usize,
	/// Each refusal once.
	pub refusals: Vec<Refusal>,
}

/// A construct translation refuses, where it starts: at the macro call for macro-made
/// code. `file` is empty, and `line` and `col` are 0, for code without a location.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Refusal {
	pub file: String,
	pub line: usize,
	pub col: usize,
	pub message: String,
}

impl Refusal {
	fn at(&self) -> String {
		match self.file.is_empty() {
			true => "no location".to_string(),
			false => format!("{}:{}:{}", self.file, self.line, self.col),
		}
	}
}

type Groups<'a, K> = BTreeMap<K, Vec<&'a Refusal>>;

impl Census {
	/// The census of `krate` from `roots`, or every item where there are none, as Markdown:
	/// refusals ranked by construct, by API, and by type or path, then every span.
	pub fn render(&self, krate: &str, roots: &[String]) -> String {
		let mut refusals = self.refusals.clone();
		// The crate's own code first, so a group's first span is in it when it can be.
		refusals.sort_by(|a, b| (Path::new(&a.file).is_absolute(), a).cmp(&(Path::new(&b.file).is_absolute(), b)));
		let mut constructs: Groups<String> = BTreeMap::new();
		let mut apis: Groups<String> = BTreeMap::new();
		let mut paths: Groups<(String, String)> = BTreeMap::new();
		for refusal in &refusals {
			let (construct, details) = split(&refusal.message);
			let owners: BTreeSet<&str> = details.iter().map(|detail| owner(detail)).collect();
			for owner in owners {
				apis.entry(format!("`{owner}`")).or_default().push(refusal);
			}
			if !details.is_empty() {
				let quoted: Vec<String> = details.iter().map(|detail| format!("`{detail}`")).collect();
				paths
					.entry((construct.clone(), quoted.join(", ")))
					.or_default()
					.push(refusal);
			}
			constructs.entry(construct).or_default().push(refusal);
		}

		let mut out = String::new();
		let _ = writeln!(out, "# Census of `{krate}`\n");
		if !roots.is_empty() {
			let roots: Vec<String> = roots.iter().map(|root| format!("`{root}`")).collect();
			let _ = writeln!(out, "- Roots: {}", roots.join(", "));
		}
		let _ = writeln!(
			out,
			"- Refusals: {}\n- Function instances: {}\n- Generic functions no instance reached, lowered over their type parameters: {}",
			refusals.len(),
			self.functions,
			self.polymorphic,
		);

		let _ = writeln!(out, "\n## By construct\n");
		let _ = writeln!(out, "| Count | Construct | First |\n| ---: | --- | --- |");
		for (construct, list) in ranked(&constructs) {
			let _ = writeln!(out, "| {} | {} | {} |", list.len(), cell(construct), list[0].at());
		}

		let _ = writeln!(out, "\n## By API\n");
		let _ = writeln!(out, "The type or module each refused type or path belongs to.\n");
		let _ = writeln!(out, "| Count | API | First |\n| ---: | --- | --- |");
		for (api, list) in ranked(&apis) {
			let _ = writeln!(out, "| {} | {} | {} |", list.len(), cell(api), list[0].at());
		}

		let _ = writeln!(out, "\n## By type or path\n");
		let _ = writeln!(
			out,
			"| Count | Construct | Type or path | First |\n| ---: | --- | --- | --- |"
		);
		for ((construct, detail), list) in ranked(&paths) {
			let _ = writeln!(
				out,
				"| {} | {} | {} | {} |",
				list.len(),
				cell(construct),
				cell(detail),
				list[0].at()
			);
		}

		let _ = writeln!(out, "\n## Spans");
		for (construct, list) in ranked(&constructs) {
			let _ = writeln!(
				out,
				"\n<details><summary>{} ({})</summary>\n",
				html(construct),
				list.len()
			);
			for refusal in list {
				let _ = writeln!(out, "- {}: {}", refusal.at(), refusal.message);
			}
			let _ = writeln!(out, "\n</details>");
		}
		out
	}
}

/// Groups by count, most first, then by key.
fn ranked<'a, K: Ord>(groups: &'a Groups<K>) -> Vec<(&'a K, &'a Vec<&'a Refusal>)> {
	let mut ranked: Vec<_> = groups.iter().collect();
	ranked.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(b.0)));
	ranked
}

/// Code that refusal messages quote as written, rather than a type or path they name.
const VERBATIM: &[&str] = &[
	"()",
	"+",
	"-",
	".await",
	"?",
	"Binary",
	"Buf::copy_to_bytes",
	"BufMut::put",
	"Bytes::from_owner",
	"BytesMut",
	"Copy",
	"Debug",
	"Display",
	"Drop",
	"Extend",
	"From",
	"From::from",
	"Hash::hash_slice",
	"LowerExp",
	"MAY_HAVE_SIDE_EFFECT",
	"None",
	"Octal",
	"Option",
	"Ord",
	"Pending",
	"Pointer",
	"Ready",
	"Reborrow",
	"Result",
	"Some",
	"UpperExp",
	"async fn",
	"become",
	"bool",
	"break",
	"clone_from",
	"collect",
	"copy_nonoverlapping",
	"dyn Future",
	"f64",
	"flatten",
	"fmt::Arguments",
	"fmt::Write",
	"fold",
	"for",
	"format_args!",
	"if",
	"if let",
	"let",
	"let ... else",
	"match",
	"next",
	"ref mut",
	"rt.None",
	"rt.OuterPending",
	"rt.Pending",
	"self",
	"skip",
	"undefined",
	"write_str",
	"zip",
	"{:.2}",
	"{:1$}",
	"{:E}",
	"{:b}",
	"{:e}",
	"{:o}",
	"{:p}",
	"{:x?}",
];

/// A message's construct, with each quoted type or path blanked to `_`, and those types
/// and paths without their generic arguments.
fn split(message: &str) -> (String, Vec<String>) {
	let mut construct = String::new();
	let mut details = Vec::new();
	let mut rest = message;
	while let Some((before, after)) = rest.split_once('`') {
		let Some((quoted, after)) = after.split_once('`') else {
			break;
		};
		construct.push_str(before);
		rest = after;
		if VERBATIM.contains(&quoted) {
			construct.push_str(&format!("`{quoted}`"));
			continue;
		}
		construct.push_str("`_`");
		details.push(erase(quoted));
	}
	construct.push_str(rest);
	(construct, details)
}

/// The type or module an erased type or path belongs to, by Rust's naming: a function's
/// owner, or a type itself. `kio::Shared::lock` and `<kio::Shared as Clone>::clone` are
/// `kio::Shared`, and `std::mem::drop` is `std::mem`.
fn owner(path: &str) -> &str {
	let path = path.trim_start_matches('&').trim_start_matches("mut ");
	if let Some((ty, _)) = path.strip_prefix('<').and_then(|rest| rest.split_once(" as ")) {
		return ty;
	}
	match path.rsplit_once("::") {
		Some((owner, last)) if last.starts_with(char::is_lowercase) => owner,
		_ => path,
	}
}

/// A type or path without its generic arguments, and a closure without its location,
/// so that its instances group: `std::sync::Arc::<T>::new` is `std::sync::Arc::new`.
/// `core` and `alloc` are `std`, which re-exports them, since rustc names either.
fn erase(text: &str) -> String {
	let mut out = String::new();
	// How deep in erased `<...>` the text is.
	let mut depth = 0usize;
	let mut prev = ' ';
	let mut chars = text.chars();
	while let Some(c) = chars.next() {
		if depth > 0 {
			match c {
				'<' => depth += 1,
				// The `->` of a function type inside the arguments closes nothing.
				'>' if prev != '-' => depth -= 1,
				_ => {}
			}
			prev = c;
			continue;
		}
		match c {
			// Arguments follow a name or `::`; a `<` anywhere else opens a qualified path.
			'<' if prev.is_alphanumeric() || matches!(prev, '_' | ':') => {
				depth = 1;
				if out.ends_with("::") {
					out.truncate(out.len() - 2);
				}
			}
			// `{closure@src/lib.rs:1:2: 1:5}` is `{closure}`.
			'@' if out.rfind('{').is_some_and(|i| !out[i..].contains('}')) => {
				chars.by_ref().find(|&c| c == '}');
				out.push('}');
			}
			_ => out.push(c),
		}
		prev = c;
	}
	let mut erased = String::new();
	let mut rest = out.as_str();
	while let Some(c) = rest.chars().next() {
		let word_start = !erased.ends_with(|c: char| c.is_alphanumeric() || c == '_');
		match ["core::", "alloc::"].iter().find_map(|krate| rest.strip_prefix(krate)) {
			Some(tail) if word_start => {
				erased.push_str("std::");
				rest = tail;
			}
			_ => {
				erased.push(c);
				rest = &rest[c.len_utf8()..];
			}
		}
	}
	erased
}

/// Text for a Markdown table cell.
fn cell(text: &str) -> String {
	text.replace('|', "\\|")
}

/// Text for HTML, where Markdown is not rendered.
fn html(text: &str) -> String {
	text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn erases_generic_arguments() {
		let cases = [
			("std::sync::Arc::<T>::new", "std::sync::Arc::new"),
			("std::option::Option<std::sync::Arc<u8>>", "std::option::Option"),
			(
				"<std::iter::Filter<std::slice::Iter<'_, u8>, {closure@src/lib.rs:40:18: 40:21}> as std::iter::Iterator>::count",
				"<std::iter::Filter as std::iter::Iterator>::count",
			),
			("core::slice::<impl [u8]>::iter", "std::slice::iter"),
			("alloc::vec::Vec<my_core::X>", "std::vec::Vec"),
			("my_core::X", "my_core::X"),
			("std::boxed::Box<dyn std::ops::Fn(u8) -> u8>", "std::boxed::Box"),
			("dyn std::ops::Fn(u8) -> u8", "dyn std::ops::Fn(u8) -> u8"),
			("{closure@src/lib.rs:1:2: 1:5}", "{closure}"),
			("&mut [u32; 4]", "&mut [u32; 4]"),
		];
		for (text, erased) in cases {
			assert_eq!(erase(text), erased, "{text}");
		}
	}

	#[test]
	fn splits_construct_from_details() {
		let cases: [(&str, &str, &[&str]); 4] = [
			(
				"unsupported call to `std::sync::Arc::<T>::new`",
				"unsupported call to `_`",
				&["std::sync::Arc::new"],
			),
			(
				"`&mut u32`: a JS value of `u32` is a copy",
				"`_`: a JS value of `_` is a copy",
				&["&mut u32", "u32"],
			),
			(
				"`Guard` implements `Drop`, which is not supported yet",
				"`_` implements `Drop`, which is not supported yet",
				&["Guard"],
			),
			(
				"unsupported expression: StaticRef",
				"unsupported expression: StaticRef",
				&[],
			),
		];
		for (message, construct, details) in cases {
			assert_eq!(
				split(message),
				(construct.to_string(), details.iter().map(|d| d.to_string()).collect())
			);
		}
	}

	#[test]
	fn owns_by_naming() {
		let cases = [
			("kio::Shared::lock", "kio::Shared"),
			("<kio::Shared as std::clone::Clone>::clone", "kio::Shared"),
			("std::mem::drop", "std::mem"),
			("std::task::Poll", "std::task::Poll"),
			("&mut u32", "u32"),
			("f64", "f64"),
		];
		for (path, api) in cases {
			assert_eq!(owner(path), api, "{path}");
		}
	}

	/// Each code word that a message in `src/lower` quotes as written is verbatim, so it
	/// stays in the construct.
	#[test]
	fn verbatim_covers_messages() {
		let mut dirs = vec![
			std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
				.join("src")
				.join("lower"),
		];
		let mut files = Vec::new();
		while let Some(dir) = dirs.pop() {
			for entry in std::fs::read_dir(dir).unwrap() {
				let path = entry.unwrap().path();
				match path.is_dir() {
					true => dirs.push(path),
					false => files.push(path),
				}
			}
		}
		for path in files {
			let text = std::fs::read_to_string(&path).unwrap();
			let code = text.lines().filter(|line| !line.trim_start().starts_with("//"));
			// Every other piece between quotes is a string literal.
			for literal in code.flat_map(|line| line.split('"').skip(1).step_by(2)) {
				for quoted in literal.split('`').skip(1).step_by(2) {
					let formatted = quoted.replace("{{", "").contains('{');
					assert!(
						formatted || VERBATIM.contains(&quoted),
						"{}: add `{quoted}` to VERBATIM",
						path.display()
					);
				}
			}
		}
	}
}
