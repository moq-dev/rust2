//! Each fixture under `fixtures/` is translated, vetted with `go vet`, and run on the
//! same inputs as its native build, failing on any difference.

use std::cell::RefCell;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn manifest_dir() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn scratch() -> PathBuf {
	PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
}

fn fixture(name: &str) -> PathBuf {
	manifest_dir().join("fixtures").join(name)
}

/// Translate a fixture into a Go module named after it, returning the module's directory.
fn translated(name: &str) -> PathBuf {
	let module = scratch().join(name);
	let out = module.join("out");
	let output = Command::new(env!("CARGO_BIN_EXE_rust2-go"))
		.arg("--config")
		.arg(fixture(name).join("go.toml"))
		.arg("--manifest-path")
		.arg(fixture(name).join("Cargo.toml"))
		.arg("--out")
		.arg(&out)
		.env("CARGO_TARGET_DIR", scratch().join("build"))
		.output()
		.expect("running rust2-go");
	assert!(
		output.status.success(),
		"translating {name} failed:\n{}",
		String::from_utf8_lossy(&output.stderr)
	);
	std::fs::write(out.join("go.mod"), "module fixture\n\ngo 1.24\n").unwrap();
	out
}

/// Run `go` in `dir`, failing with what it printed.
fn go(dir: &Path, args: &[&str]) {
	let output = Command::new("go")
		.args(args)
		.current_dir(dir)
		.output()
		.expect("running go, which the dev shell provides");
	assert!(
		output.status.success(),
		"go {} failed:\n{}{}",
		args.join(" "),
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
}

#[derive(PartialEq, Debug)]
enum Outcome {
	Output(Vec<u8>),
	Panic(String),
}

thread_local! {
	/// The message of this thread's last panic, which the hook records.
	static PANIC: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn native(run: fn(&[u8]) -> Vec<u8>, input: &[u8]) -> Outcome {
	static HOOK: std::sync::Once = std::sync::Once::new();
	HOOK.call_once(|| {
		std::panic::set_hook(Box::new(|info| {
			let msg = info
				.payload_as_str()
				.unwrap_or("a panic of a payload other than a message");
			PANIC.set(Some(msg.to_string()));
		}))
	});
	match std::panic::catch_unwind(|| run(input)) {
		Ok(out) => Outcome::Output(out),
		Err(_) => Outcome::Panic(PANIC.take().expect("a panic message")),
	}
}

/// Run the translated harness, the root exported as `entry`, on every input in one process.
fn translation(out: &Path, entry: &str, cases: &[Vec<u8>]) -> Vec<Outcome> {
	let main = out.join("differential");
	std::fs::create_dir_all(&main).unwrap();
	let text = std::fs::read_to_string(manifest_dir().join("tests/differential/main.go")).unwrap();
	std::fs::write(
		main.join("main.go"),
		text.replace("fixture.Run(", &format!("fixture.{entry}(")),
	)
	.unwrap();
	go(
		out,
		&["build", "-buildvcs=false", "-o", "differential/run", "./differential"],
	);
	let mut child = Command::new(main.join("run"))
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.spawn()
		.expect("running the translated harness");
	let mut stdin = child.stdin.take().unwrap();
	let mut input = Vec::new();
	for case in cases {
		input.extend_from_slice(&(case.len() as u32).to_le_bytes());
		input.extend_from_slice(case);
	}
	let writer = std::thread::spawn(move || {
		// The harness may exit early, which its status reports below.
		let _ = stdin.write_all(&input);
	});
	let output = child.wait_with_output().unwrap();
	writer.join().unwrap();
	assert!(
		output.status.success(),
		"the translated harness failed:\n{}",
		String::from_utf8_lossy(&output.stderr)
	);
	let mut rest = &output.stdout[..];
	let mut outcomes = Vec::new();
	while let Some((&kind, tail)) = rest.split_first() {
		let len = u32::from_le_bytes(tail[..4].try_into().unwrap()) as usize;
		let body = tail[4..4 + len].to_vec();
		rest = &tail[4 + len..];
		outcomes.push(match kind {
			0 => Outcome::Output(body),
			_ => Outcome::Panic(String::from_utf8(body).unwrap()),
		});
	}
	outcomes
}

/// xorshift64*: the same cases on every run.
struct Rng(u64);

impl Rng {
	fn next(&mut self) -> u64 {
		self.0 ^= self.0 >> 12;
		self.0 ^= self.0 << 25;
		self.0 ^= self.0 >> 27;
		self.0.wrapping_mul(0x2545f4914f6cdd1d)
	}

	fn byte(&mut self) -> u8 {
		self.next() as u8
	}

	/// A byte biased toward the values codecs care about: zero, all ones, and boundaries.
	fn interesting(&mut self) -> u8 {
		const EDGES: [u8; 16] = [
			0x3f, 0x40, 0x7f, 0x80, 0xbf, 0xc0, 0xdf, 0xe0, 0xef, 0xf0, 0xf7, 0xf8, 0xfb, 0xfc, 0xfd, 0xfe,
		];
		match self.next() % 8 {
			0 => 0,
			1 => 0xff,
			2 => EDGES[(self.next() % 16) as usize],
			_ => self.byte(),
		}
	}
}

/// The inputs rust2-ts's differential test runs: every op with every short tail, each
/// u64 boundary through the first `ops` ops with each of `selectors` second bytes, and
/// `count` random ones.
fn cases(ops: u8, selectors: u8, count: usize) -> Vec<Vec<u8>> {
	let mut cases = vec![Vec::new()];
	for op in 0..=255u8 {
		cases.push(vec![op]);
		for b in (0..=255u8).step_by(17) {
			cases.push(vec![op, b]);
		}
	}
	for op in 0..ops {
		for k in 0..=64u32 {
			for delta in [-1i64, 0, 1] {
				let value = 1u64
					.checked_shl(k)
					.unwrap_or(0)
					.wrapping_add_signed(delta)
					.to_be_bytes();
				for selector in 0..selectors {
					let mut case = vec![op, selector];
					for _ in 0..3 {
						case.extend_from_slice(&value);
					}
					cases.push(case);
				}
			}
		}
	}
	let mut rng = Rng(0x9e3779b97f4a7c15);
	for _ in 0..count {
		let len = (rng.next() % 48) as usize;
		let mut case = vec![rng.byte()];
		case.extend((0..len).map(|_| rng.interesting()));
		cases.push(case);
	}
	cases
}

fn show(outcome: &Outcome) -> String {
	match outcome {
		Outcome::Output(out) => out.iter().map(|b| format!("{b:02x}")).collect(),
		Outcome::Panic(msg) => format!("panic {msg:?}"),
	}
}

/// Run a fixture's harness natively, as `run`, and translated, as `entry`, on the same
/// inputs, failing on any difference.
fn differential(name: &str, run: fn(&[u8]) -> Vec<u8>, entry: &str, cases: Vec<Vec<u8>>) {
	let out = translated(name);
	go(&out, &["vet", "."]);
	let want: Vec<Outcome> = cases.iter().map(|case| native(run, case)).collect();
	let got = translation(&out, entry, &cases);
	assert_eq!(got.len(), cases.len(), "the translated harness stopped early");
	let mut failures = Vec::new();
	for ((input, want), got) in cases.iter().zip(&want).zip(&got) {
		if want != got {
			failures.push(format!(
				"input  {}\nnative {}\ngo     {}\n",
				show(&Outcome::Output(input.clone())),
				show(want),
				show(got)
			));
		}
	}
	let panics = want.iter().filter(|o| matches!(o, Outcome::Panic(_))).count();
	println!(
		"{name}: {} cases, {panics} panics, {} differences",
		cases.len(),
		failures.len()
	);
	assert!(
		failures.is_empty(),
		"{name}: {} cases, {panics} panics, {} differences:\n{}",
		cases.len(),
		failures.len(),
		failures[..failures.len().min(20)].join("\n")
	);
}

/// moq's codec, through rust2-ts's harness of it.
#[test]
fn codec() {
	differential("codec", codec::harness::run, "HarnessRun", cases(8, 7, 20_000));
}

/// Every integer width's arithmetic, which overflows, divides by zero, and shifts past
/// its width where native Rust panics.
#[test]
fn ints() {
	differential("ints", ints::run, "Run", cases(15, 32, 20_000));
}

/// Constructs outside the subset each fail translation with a span, as
/// `fixtures/reject/expected.txt` lists them, and no output is written.
#[test]
fn reject() {
	let name = "reject";
	let out = scratch().join(name).join("out");
	let output = Command::new(env!("CARGO_BIN_EXE_rust2-go"))
		.arg("--config")
		.arg(fixture(name).join("go.toml"))
		.arg("--manifest-path")
		.arg(fixture(name).join("Cargo.toml"))
		.arg("--out")
		.arg(&out)
		.env("CARGO_TARGET_DIR", scratch().join("build"))
		.output()
		.expect("running rust2-go");
	assert!(!output.status.success(), "translating {name} succeeded");
	assert!(!out.exists(), "a failed translation wrote output");
	// Each error and where it points, with std's source from wherever the toolchain keeps it.
	let stderr = String::from_utf8_lossy(&output.stderr);
	let mut got = String::new();
	let mut lines = stderr.lines().peekable();
	while let Some(line) = lines.next() {
		let Some(error) = line.strip_prefix("error: rust2-go: ") else {
			continue;
		};
		got.push_str(error);
		if let Some(at) = lines.peek().and_then(|l| l.trim_start().strip_prefix("--> ")) {
			let at = at.split_once("/rustlib/src/rust/").map_or(at, |(_, path)| path);
			got.push_str(&format!(" at {at}"));
		}
		got.push('\n');
	}
	let expected = fixture(name).join("expected.txt");
	if std::env::var_os("UPDATE_EXPECT").is_some() {
		std::fs::write(&expected, &got).unwrap();
		return;
	}
	let want = std::fs::read_to_string(&expected).unwrap();
	assert!(
		got == want,
		"{name}: the errors differ from expected.txt; rerun with UPDATE_EXPECT=1\n{}",
		diff(&want, &got)
	);
}

/// The first lines where two texts differ.
fn diff(want: &str, got: &str) -> String {
	let (want, got): (Vec<&str>, Vec<&str>) = (want.lines().collect(), got.lines().collect());
	let start = want.iter().zip(&got).take_while(|(a, b)| a == b).count();
	let mut out = String::new();
	for line in want.iter().skip(start).take(8) {
		out.push_str(&format!("- {line}\n"));
	}
	for line in got.iter().skip(start).take(8) {
		out.push_str(&format!("+ {line}\n"));
	}
	out
}
