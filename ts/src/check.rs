//! The check of a written translation: `tsc` type-checks every translated module, and the
//! check module that holds each binding to what translated code takes it for. Translation
//! fails on any error, at the Rust span the source map names, and the output is removed.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use rustc_middle::ty::TyCtxt;
use rustc_span::{BytePos, Span};

use crate::lower::check_module;
use crate::print::Printed;
use crate::program::{Program, item_sym, module_file};
use crate::ts::Loc;

/// The project `tsc` checks the output in, beside the check module: the runtime's own
/// settings, as `runtime/tsconfig.json` has them, but for the unused names a binding may
/// leave, which are its author's to lint, and the ambient types a caller's `node_modules`
/// may hold. A project of its own, since `tsc` refuses to check a file it names where a
/// caller's `tsconfig.json` is above it, naming only the files translation writes, which
/// a caller may keep others beside.
fn tsconfig(files: &[String]) -> String {
	let files: Vec<String> = files.iter().map(|file| json(file)).collect();
	format!(
		r#"{{
	"compilerOptions": {{
		"strict": true,
		"noEmit": true,
		"pretty": false,
		"target": "es2022",
		"lib": ["es2022", "esnext.disposable", "dom"],
		"module": "esnext",
		"moduleResolution": "bundler",
		"allowImportingTsExtensions": true,
		"verbatimModuleSyntax": true,
		"erasableSyntaxOnly": true,
		"skipLibCheck": true,
		"types": []
	}},
	"files": [{}]
}}
"#,
		files.join(", ")
	)
}

/// Type-check the translation in `out`, whose modules `written` holds as printed,
/// reporting each error at the Rust it maps to, and then removing the translation.
pub fn typecheck(tcx: TyCtxt<'_>, program: &Program, out: &Path, written: &[(PathBuf, Printed)]) {
	if written.is_empty() && program.checks.is_empty() {
		return;
	}
	let failed = |msg: String| {
		tcx.dcx().err(format!("rust2-ts: {msg}"));
		_ = crate::program::clean(out);
	};
	if let Err(err) = tsc_version() {
		return failed(err);
	}
	// `tsc` names each file from its working directory, which the OS resolves through any
	// symlink, so it's given the project by the real path too.
	let real = match out.canonicalize() {
		Ok(real) => real,
		Err(err) => return failed(format!("resolving {}: {err}", out.display())),
	};
	let module = check_module();
	let check_file = module_file(&module);
	let path = real.join(&check_file);
	let mut files: Vec<String> = written
		.iter()
		.map(|(file, _)| format!("../{}", file.to_string_lossy()))
		.collect();
	let checks = match program.checks.is_empty() {
		true => None,
		false => {
			let items: Vec<_> = program.checks.iter().map(|check| &check.item).collect();
			let printed = match crate::print::module(program, &module, &items, &path) {
				Ok(printed) => printed,
				Err(err) => return failed(err),
			};
			if let Err(err) = crate::program::write_file(&path, &printed.code) {
				return failed(err);
			}
			files.push(path.file_name().expect("a file name").to_string_lossy().into_owned());
			Some(printed)
		}
	};
	let project = path.with_extension("json");
	if let Err(err) = crate::program::write_file(&project, &tsconfig(&files)) {
		_ = std::fs::remove_file(&path);
		return failed(err);
	}
	let output = std::process::Command::new("tsc")
		.arg("-p")
		.arg(&project)
		.current_dir(&real)
		.output();
	_ = std::fs::remove_file(&path);
	_ = std::fs::remove_file(&project);
	let output = match output {
		Ok(output) => output,
		Err(err) => return failed(format!("running tsc, which checks the translation: {err}")),
	};
	if output.status.success() {
		return;
	}
	let mut modules: HashMap<&Path, &Printed> = written
		.iter()
		.map(|(file, printed)| (file.as_path(), printed))
		.collect();
	if let Some(checks) = &checks {
		modules.insert(&check_file, checks);
	}
	let by_sym: HashMap<_, _> = program
		.checks
		.iter()
		.map(|check| (item_sym(&check.item), check))
		.collect();
	let text = String::from_utf8_lossy(&output.stdout);
	// Where the output is, which a message naming a module spells out.
	let real_dir = format!("{}/", real.display());
	// Each error, at its Rust span if it has one: those of the bindings and those of the
	// translated modules apart. Each item's instances of a generic function fail alike, so
	// each failure is reported once.
	let mut bound: Vec<(Option<Span>, String)> = Vec::new();
	let mut translated: Vec<(Option<Span>, String)> = Vec::new();
	let mut seen = HashSet::new();
	for diagnostic in diagnostics(&text) {
		let diagnostic = diagnostic.replace(&real_dir, "");
		// `file(line,col): error TS2322: ...`, with a one-based line and column.
		let located = diagnostic.split_once('(').and_then(|(file, rest)| {
			let (at, msg) = rest.split_once("): ")?;
			let (line, col) = at.split_once(',')?;
			let printed = modules.get(Path::new(file))?;
			Some((file, line.parse::<u32>().ok()?, col.parse::<u32>().ok()?, msg, printed))
		});
		let Some((file, line, col, msg, printed)) = located else {
			if seen.insert((None, diagnostic.clone())) {
				bound.push((None, format!("tsc: {diagnostic}")));
			}
			continue;
		};
		let (line, col) = (line - 1, col - 1);
		let item = printed.items.iter().rev().find(|&&(start, _)| start <= line).copied();
		if Path::new(file) == check_file {
			let check = item.and_then(|(_, sym)| by_sym.get(&sym));
			if seen.insert((check.map(|check| check.span), msg.to_string())) {
				bound.push(match check {
					Some(check) => (Some(check.span), format!("{} doesn't type-check: {msg}", check.what)),
					None => (None, format!("tsc: {diagnostic}")),
				});
			}
			continue;
		}
		// The Rust the source map names here, if it maps this item, or else the item's.
		let at = printed.tokens.partition_point(|&(l, c, _)| (l, c) <= (line, col));
		let span = match (at.checked_sub(1).map(|at| printed.tokens[at]), item) {
			(Some((l, _, loc)), Some((start, _))) if l >= start => Some(span(program, loc)),
			(_, Some((_, sym))) => Some(program.syms[sym].span).filter(|span| !span.is_dummy()),
			_ => None,
		};
		if seen.insert((span, msg.to_string())) {
			translated.push((
				span,
				format!(
					"{file}:{}:{} doesn't type-check, which is a bug in rust2-ts: {msg}",
					line + 1,
					col + 1
				),
			));
		}
	}
	// A binding that fails, or whose file does, fails the translated code calling it too,
	// which is no bug of translation's: that is reported once the bindings check.
	let errors = if bound.is_empty() { translated } else { bound };
	if errors.is_empty() {
		tcx.dcx().err(format!(
			"rust2-ts: tsc failed: {text}{}",
			String::from_utf8_lossy(&output.stderr)
		));
	}
	for (span, msg) in errors {
		match span {
			Some(span) => _ = tcx.dcx().span_err(span, format!("rust2-ts: {msg}")),
			None => _ = tcx.dcx().err(format!("rust2-ts: {msg}")),
		}
	}
	_ = crate::program::clean(out);
}

/// The span where `loc` is.
fn span(program: &Program, loc: Loc) -> Span {
	let source = &program.sources[loc.file as usize];
	let line: usize = source
		.text
		.split_inclusive('\n')
		.take(loc.line as usize)
		.map(str::len)
		.sum();
	let mut units = 0;
	let col: usize = source.text[line..]
		.chars()
		.take_while(|c| {
			units += c.len_utf16() as u32;
			units <= loc.col
		})
		.map(char::len_utf8)
		.sum();
	let lo = source.start + BytePos((line + col) as u32);
	Span::with_root_ctxt(lo, lo)
}

/// `text` as a JSON string.
fn json(text: &str) -> String {
	let mut out = String::from('"');
	for c in text.chars() {
		match c {
			'"' | '\\' => {
				out.push('\\');
				out.push(c);
			}
			c if c.is_control() => write!(out, "\\u{:04x}", c as u32).unwrap(),
			c => out.push(c),
		}
	}
	out.push('"');
	out
}

/// The oldest `tsc` the check project is written for.
const TSC_MAJOR: u32 = 7;

/// Refuse a `tsc` on `PATH` older than [`TSC_MAJOR`], which would fail the check project's
/// options rather than the translation.
fn tsc_version() -> Result<(), String> {
	let output = std::process::Command::new("tsc")
		.arg("--version")
		.output()
		.map_err(|err| format!("running tsc, which checks the translation: {err}"))?;
	let text = String::from_utf8_lossy(&output.stdout);
	// `Version 7.0.2`.
	let major = text
		.split_whitespace()
		.last()
		.and_then(|version| version.split('.').next())
		.and_then(|major| major.parse::<u32>().ok());
	match major {
		Some(major) if major >= TSC_MAJOR => Ok(()),
		_ => Err(format!(
			"tsc {}, which checks the translation, must be {TSC_MAJOR}.0 or later",
			text.trim()
		)),
	}
}

/// Each of `tsc`'s diagnostics, with the lines that go on with it.
fn diagnostics(text: &str) -> Vec<String> {
	let mut all: Vec<String> = Vec::new();
	for line in text.lines() {
		match all.last_mut() {
			Some(last) if line.starts_with(char::is_whitespace) => {
				last.push('\n');
				last.push_str(line);
			}
			_ if line.trim().is_empty() => {}
			_ => all.push(line.to_string()),
		}
	}
	all
}
