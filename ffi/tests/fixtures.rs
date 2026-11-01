//! The api fixture is exported, built as a static library, and driven from C by
//! `tests/api.c`, from C++ by `tests/api.cpp`, from Kotlin by `tests/api.kt` on the JVM
//! and, shrunk by R8, on Android, and from Swift by `tests/api.swift`, whose transcripts must
//! match the fixture's own script run natively.
//! The reject fixture's export must fail at each item it cannot export.

use std::env::consts::EXE_SUFFIX;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

fn manifest_dir() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn scratch() -> PathBuf {
	PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
}

fn cargo() -> Command {
	Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

fn run(cmd: &mut Command) -> Output {
	let output = cmd.output().unwrap_or_else(|err| panic!("running {cmd:?}: {err}"));
	assert!(
		output.status.success(),
		"{cmd:?} failed:\n{}{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
	output
}

/// Export the fixture `name` into `out` with these cargo args, returning what the
/// exporter printed.
fn export(name: &str, out: &Path, cargo: &[&str]) -> Output {
	let fixture = manifest_dir().join("fixtures").join(name);
	Command::new(env!("CARGO_BIN_EXE_rust2-ffi"))
		.arg("--config")
		.arg(fixture.join("ffi.toml"))
		.arg("--out")
		.arg(out)
		.arg("--")
		.args(cargo)
		.env("CARGO_TARGET_DIR", scratch().join(name).join("build"))
		.output()
		.expect("running rust2-ffi")
}

/// The api fixture, exported and built once for each language's test.
struct Api {
	/// The export.
	out: PathBuf,
	/// Where cargo built the static library and the JNI library.
	lib: PathBuf,
	/// What a program linking the static library links besides, as rustc prints it.
	native: Vec<String>,
	/// The fixture's script, run natively.
	transcript: String,
}

impl Api {
	fn staticlib(&self) -> PathBuf {
		self.lib.join(match cfg!(target_env = "msvc") {
			true => "api.lib",
			false => "libapi.a",
		})
	}
}

fn api() -> &'static Api {
	static API: OnceLock<Api> = OnceLock::new();
	API.get_or_init(|| {
		let out = scratch().join("api").join("out");
		// A feature enabled on another export directly, which the C library must build with.
		let features = ["--features", "dep-fixture/doubled"];
		let output = export("api", &out, &features);
		assert!(
			output.status.success(),
			"exporting failed:\n{}",
			String::from_utf8_lossy(&output.stderr)
		);
		// The workspace's lockfile pins what the export depends on.
		std::fs::copy(manifest_dir().join("Cargo.lock"), out.join("Cargo.lock")).unwrap();
		let target = scratch().join("api").join("glue");
		let built = run(cargo()
			.arg("build")
			.arg("--quiet")
			.arg("--manifest-path")
			.arg(out.join("Cargo.toml"))
			.arg("--target-dir")
			.arg(&target)
			.args(["--features", "kotlin"])
			// Glue the exporter writes builds without warnings.
			.env("RUSTFLAGS", "-D warnings --print=native-static-libs"));
		let native = String::from_utf8_lossy(&built.stderr)
			.lines()
			.find_map(|line| line.strip_prefix("note: native-static-libs: "))
			.expect("rustc prints the static library's native libraries")
			.split_whitespace()
			.map(str::to_string)
			.collect();
		let transcript = run(cargo()
			.args([
				"run",
				"--quiet",
				"--locked",
				"--package",
				"api-fixture",
				"--example",
				"transcript",
			])
			.args(features)
			.current_dir(manifest_dir()));
		Api {
			out,
			lib: target.join("debug"),
			native,
			transcript: String::from_utf8_lossy(&transcript.stdout).into_owned(),
		}
	})
}

#[test]
fn api_c() {
	let api = api();
	let exe = scratch().join("api").join("api-test");
	let cc = std::env::var_os("CC").unwrap_or_else(|| "cc".into());
	run(Command::new(cc)
		.args(["-std=c11", "-pedantic", "-Wall", "-Wextra", "-Werror", "-I"])
		.arg(api.out.join("include"))
		.arg(manifest_dir().join("tests").join("api.c"))
		.arg(api.staticlib())
		.args(&api.native)
		.arg("-o")
		.arg(&exe));
	let c = run(&mut Command::new(&exe));
	assert_eq!(
		String::from_utf8_lossy(&c.stdout),
		api.transcript,
		"the C transcript differs from the native one"
	);

	// Each broken contract aborts, naming the function and what broke.
	for (case, message) in [
		("moved", "api_counter_get: a handle's value was moved out"),
		("borrowed", "api_channel_clear: a handle is already borrowed"),
		("utf8", "api_greet: a string is not UTF-8"),
		(
			"finished",
			"api_channel_recv_poll: a future is polled after it finished",
		),
		("enum", "api_greet: an enum value is invalid"),
		("bound", "api_count: a bound's kind is invalid"),
		("socket", "api_ports: an IPv4 socket address has IPv6 bytes or fields"),
		("family", "api_nexts: an IPv6 socket address where IPv4 is taken"),
		("array", "api_vec_u16_free: an array is not as Rust returned it"),
		("lent", "api_registry_edit: a lent handle was freed"),
		(
			"lent moved",
			"api_counter_into_label: a lent handle's value cannot move out",
		),
		("null", "api_registry_edit: a function is null"),
	] {
		let output = Command::new(&exe).arg(case).output().unwrap();
		assert!(!output.status.success(), "{case} did not abort");
		let stderr = String::from_utf8_lossy(&output.stderr);
		assert!(stderr.contains(message), "{case} printed {stderr}");
	}
}

/// The C++ header at each standard, with exceptions and RTTI on and off between them,
/// runs the same script, then checks its futures' semantics.
#[test]
fn api_cpp() {
	let api = api();
	for (standard, plain) in [(17, true), (20, false), (23, true)] {
		let exe = scratch().join("api").join(format!("api-c++{standard}{EXE_SUFFIX}"));
		run(&mut cxx(
			api,
			standard,
			plain,
			&manifest_dir().join("tests").join("api.cpp"),
			&exe,
		));
		let cpp = run(&mut Command::new(&exe));
		assert_eq!(
			String::from_utf8_lossy(&cpp.stdout),
			api.transcript,
			"the C++{standard} transcript differs from the native one"
		);
		run(Command::new(&exe).arg("futures"));
		for (case, message) in [
			("moved", "api_counter_get: a handle's value was moved out"),
			("value", "value() of a result that holds an error"),
			(
				"finished",
				"api_channel_recv_poll: a future is polled after it finished",
			),
			(
				"lent moved",
				"api_counter_into_label: a lent handle's value cannot move out",
			),
			("lent kept", "a lent object was moved out of"),
			("null view", "a view of null bytes has a length"),
		] {
			let output = Command::new(&exe).arg(case).output().unwrap();
			assert!(!output.status.success(), "C++{standard} {case} did not abort");
			let stderr = String::from_utf8_lossy(&output.stderr);
			assert!(stderr.contains(message), "C++{standard} {case} printed {stderr}");
		}
	}

	// A C++ program whose header names another contract than its library's aborts at load.
	let mismatched = scratch().join("api").join("contract.cpp");
	std::fs::write(
		&mismatched,
		"#include \"api.h\"\n#undef API_CONTRACT\n#define API_CONTRACT ((uint64_t)0)\n#include \"api.hpp\"\nint main() {}\n",
	)
	.unwrap();
	let contract = scratch().join("api").join(format!("contract{EXE_SUFFIX}"));
	run(&mut cxx(api, 17, false, &mismatched, &contract));
	let output = Command::new(&contract).output().unwrap();
	assert!(!output.status.success(), "a mismatched contract did not abort");
	let stderr = String::from_utf8_lossy(&output.stderr);
	assert!(
		stderr.contains("api.hpp and the library are from different builds"),
		"a mismatched contract printed {stderr}"
	);
}

/// A build of the C++ `src` against the api library into `exe`, at C++`standard`, with
/// exceptions and RTTI off if `plain`, and warnings as errors: by `CXX` or the platform's
/// compiler, given MSVC's flags where the target is MSVC's.
fn cxx(api: &Api, standard: u8, plain: bool, src: &Path, exe: &Path) -> Command {
	let include = api.out.join("include");
	if cfg!(target_env = "msvc") {
		// cl from a developer environment. Its C++23 is `latest`; its objects go beside `exe`.
		let mut cmd = Command::new(std::env::var_os("CXX").unwrap_or_else(|| "cl".into()));
		let standard = match standard {
			23 => "latest".to_string(),
			n => format!("c++{n}"),
		};
		cmd.args([
			"-nologo",
			&format!("-std:{standard}"),
			"-permissive-",
			"-W4",
			"-WX",
			"-MD",
		]);
		cmd.args(match plain {
			true => &["-EHs-c-", "-D_HAS_EXCEPTIONS=0", "-GR-"][..],
			false => &["-EHsc", "-GR"],
		});
		let mut obj = OsString::from("-Fo");
		obj.push(exe.with_extension("obj"));
		let mut out = OsString::from("-Fe");
		out.push(exe);
		cmd.arg("-I").arg(include).arg(src).arg(obj).arg(out);
		cmd.arg("-link").arg(api.staticlib()).args(&api.native);
		return cmd;
	}
	let mut cmd = Command::new(std::env::var_os("CXX").unwrap_or_else(|| "c++".into()));
	cmd.arg(format!("-std=c++{standard}"));
	if plain {
		cmd.args(["-fno-exceptions", "-fno-rtti"]);
	}
	// -Wshadow stands in for MSVC's warnings that a name hides another.
	cmd.args(["-pedantic", "-Wall", "-Wextra", "-Wshadow", "-Werror", "-I"])
		.arg(include)
		.arg(src)
		.arg(api.staticlib())
		.args(&api.native)
		.arg("-o")
		.arg(exe);
	cmd
}

/// The generated Kotlin, compiled with `tests/api.kt` against the library's JNI
/// functions, runs its script on the JVM, then refuses a library of another contract.
#[test]
fn api_kotlin() {
	let api = api();
	let classes = scratch().join("api").join("kotlin");
	let _ = std::fs::remove_dir_all(&classes);
	run(&mut kotlinc(&api.out, &classes));
	let mut classpath = classes.clone().into_os_string();
	classpath.push(":");
	classpath.push(coroutines());
	let java = |args: &[&str]| {
		let mut cmd = Command::new("kotlin");
		cmd.arg(format!("-Djava.library.path={}", api.lib.display()))
			.arg("-cp")
			.arg(&classpath)
			.arg("ApiKt")
			.args(args);
		cmd
	};
	kotlin(&api.transcript, &java);

	// The same classes, with another contract than the library's in place of theirs,
	// refuse to load it.
	let native_class = classes.join("api").join("Native.class");
	let bytes = std::fs::read(&native_class).unwrap();
	let contract = kotlin_contract(&api.out.join("kotlin").join("api.kt")).to_be_bytes();
	let at: Vec<usize> = (0..bytes.len() - 8).filter(|&i| bytes[i..i + 8] == contract).collect();
	assert_eq!(at.len(), 1, "Native.class holds the contract once");
	let mut mismatched = bytes.clone();
	mismatched[at[0]] ^= 0xff;
	std::fs::write(&native_class, &mismatched).unwrap();
	let output = java(&[]).output().unwrap();
	std::fs::write(&native_class, &bytes).unwrap();
	assert!(!output.status.success(), "a mismatched contract did not fail");
	let stderr = String::from_utf8_lossy(&output.stderr);
	assert!(
		stderr.contains("api.kt and the library are from different builds"),
		"a mismatched contract printed {stderr}"
	);
}

/// The same script on Android: the classes, shrunk by R8 into an app's dex with only the
/// generated rules keeping what JNI looks up, run on ART on the device adb reaches against
/// the library the NDK builds for its ABI, both for API 24, the oldest moq's Android library
/// supports.
#[test]
#[ignore = "needs the Android SDK and NDK, cargo-ndk, and a device: just ffi android"]
fn api_android() {
	const DEVICE: &str = "/data/local/tmp/rust2-ffi";
	let api = api();
	let sdk = PathBuf::from(std::env::var_os("ANDROID_HOME").expect("ANDROID_HOME names the Android SDK"));
	let adb = sdk.join("platform-tools").join("adb");
	let dir = scratch().join("api").join("android");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();

	let abi = run(Command::new(&adb).args(["shell", "getprop", "ro.product.cpu.abi"]));
	let abi = String::from_utf8_lossy(&abi.stdout).trim().to_string();
	run(cargo()
		.args(["ndk", "--target", &abi, "--platform", "24", "--output-dir"])
		.arg(dir.join("lib"))
		.args(["build", "--quiet", "--manifest-path"])
		.arg(api.out.join("Cargo.toml"))
		.arg("--target-dir")
		.arg(scratch().join("api").join("glue"))
		.args(["--features", "kotlin"])
		.env("RUSTFLAGS", "-D warnings"));

	// The app: the script's entry point, kept as an app keeps its own, and everything it
	// reaches, which R8 shrinks and renames, keeping only what the rules keep: the
	// generated ones, and those kotlinx-coroutines' jar holds, which an app's build reads
	// from it. Kotlin's annotations are on an app's classpath, not in its dex.
	let jar = dir.join("app.jar");
	run(kotlinc(&api.out, &jar).arg("-include-runtime"));
	run(Command::new("jar")
		.arg("xf")
		.arg(coroutines())
		.arg("META-INF/com.android.tools/r8/coroutines.pro")
		.current_dir(&dir));
	let app = dir.join("app.pro");
	std::fs::write(
		&app,
		format!(
			"-keep class ApiKt {{ public static void main(java.lang.String[]); }}\n-dontwarn org.jetbrains.annotations.**\n-printusage {}\n",
			dir.join("usage.txt").display()
		),
	)
	.unwrap();
	let dex = dir.join("dex");
	std::fs::create_dir_all(&dex).unwrap();
	run(Command::new("java")
		.arg("-cp")
		.arg(newest(&sdk.join("build-tools")).join("lib").join("d8.jar"))
		.args(["com.android.tools.r8.R8", "--release", "--min-api", "24", "--lib"])
		.arg(newest(&sdk.join("platforms")).join("android.jar"))
		.arg("--pg-conf")
		.arg(api.out.join("kotlin").join("api.pro"))
		.arg("--pg-conf")
		.arg(dir.join("META-INF/com.android.tools/r8/coroutines.pro"))
		.arg("--pg-conf")
		.arg(&app)
		.arg("--output")
		.arg(&dex)
		.arg(&jar)
		.arg(coroutines()));
	// R8 strips the generated Kotlin the script never reaches.
	let usage = std::fs::read_to_string(dir.join("usage.txt")).unwrap();
	assert!(
		usage.lines().any(|line| line.starts_with("api.")),
		"R8 stripped none of the generated Kotlin:\n{usage}"
	);

	run(Command::new(&adb)
		.args(["shell", "rm", "-rf", DEVICE, "&&", "mkdir", "-p"])
		.arg(format!("{DEVICE}/dalvik-cache")));
	run(Command::new(&adb)
		.arg("push")
		.arg(dex.join("classes.dex"))
		.arg(dir.join("lib").join(&abi).join("libapi.so"))
		.arg(DEVICE));
	let art = |args: &[&str]| {
		let args: String = args.iter().map(|arg| format!(" '{arg}'")).collect();
		let mut cmd = Command::new(&adb);
		cmd.arg("shell").arg(format!(
			"ANDROID_DATA={DEVICE} dalvikvm -Djava.library.path={DEVICE} -cp {DEVICE}/classes.dex ApiKt{args}"
		));
		cmd
	};
	kotlin(&api.transcript, &art);
}

/// kotlinc compiling the generated Kotlin with `tests/api.kt` into `dest`.
fn kotlinc(out: &Path, dest: &Path) -> Command {
	let mut cmd = Command::new("kotlinc");
	cmd.args(["-Werror", "-cp"])
		.arg(coroutines())
		.arg(out.join("kotlin").join("api.kt"))
		.arg(manifest_dir().join("tests").join("api.kt"))
		.arg("-d")
		.arg(dest);
	cmd
}

/// kotlinx-coroutines' jar.
fn coroutines() -> OsString {
	std::env::var_os("KOTLINX_COROUTINES")
		.expect("KOTLINX_COROUTINES names kotlinx-coroutines' jar: run this in the dev shell")
}

/// The entry of `dir` of the highest version, as the SDK names its build tools and platforms.
fn newest(dir: &Path) -> PathBuf {
	let version = |path: &PathBuf| -> Vec<u32> {
		let name = path.file_name().unwrap().to_string_lossy().into_owned();
		name.split(|c: char| !c.is_ascii_digit())
			.filter_map(|n| n.parse().ok())
			.collect()
	};
	std::fs::read_dir(dir)
		.unwrap_or_else(|err| panic!("reading {}: {err}", dir.display()))
		.map(|entry| entry.unwrap().path())
		.max_by_key(version)
		.unwrap_or_else(|| panic!("{} is empty", dir.display()))
}

/// `tests/api.kt`'s script, run by `vm` with each argument list, prints the native
/// transcript; then its futures, what the GC frees, and each contract it fails loud on.
fn kotlin(native: &str, vm: &dyn Fn(&[&str]) -> Command) {
	let kt = run(&mut vm(&[]));
	assert_eq!(
		String::from_utf8_lossy(&kt.stdout),
		native,
		"the Kotlin transcript differs from the native one"
	);
	run(&mut vm(&["futures"]));
	run(&mut vm(&["owned"]));
	for (case, message) in [
		("moved", "Counter is closed"),
		("boxed moved", "Node is closed"),
		("lent kept", "Counter is closed"),
		(
			"lent moved",
			"api_counter_into_label: a lent handle's value cannot move out",
		),
		("callback threw", "api_fixture::Log::log: a call into Kotlin threw"),
		("stray", "MalformedInputException"),
		("family", "an IPv6 address has no Rust SocketAddrV4"),
	] {
		let output = vm(&[case]).output().unwrap();
		assert!(!output.status.success(), "Kotlin {case} did not fail");
		let stderr = String::from_utf8_lossy(&output.stderr);
		assert!(stderr.contains(message), "Kotlin {case} printed {stderr}");
	}
}

/// The contract a Kotlin file checks its library against.
fn kotlin_contract(kotlin: &Path) -> u64 {
	let kotlin = std::fs::read_to_string(kotlin).unwrap();
	let at = kotlin
		.find("__contract().toULong() == 0x")
		.expect("the Kotlin checks its contract");
	let hex = &kotlin[at..].split("0x").nth(1).expect("a hex contract")[..16];
	u64::from_str_radix(hex, 16).unwrap()
}

/// The generated Swift, built as the module `Api` with warnings as errors in Swift 6's
/// language mode and type-checked in Swift 5's, which moq's package builds in, runs
/// `tests/api.swift`'s script, its futures, and what releasing frees, then fails loud on
/// each broken contract and on a library of another contract.
#[test]
#[cfg_attr(
	all(target_os = "linux", target_arch = "aarch64"),
	ignore = "nixpkgs builds no Swift for aarch64-linux: quest/m2/uniffi-swift-arm.md"
)]
fn api_swift() {
	let api = api();
	let dir = scratch().join("api").join("swift");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	let module = api.out.join("swift");
	let source = module.join("api.swift");
	run(swiftc(&module).args(["-swift-version", "5", "-typecheck"]).arg(&source));
	run(swiftc(&module)
		.args(["-swift-version", "6", "-parse-as-library", "-module-name", "Api"])
		.args(["-emit-module", "-emit-module-path"])
		.arg(dir.join("Api.swiftmodule"))
		.args(["-emit-library", "-static", "-o"])
		.arg(dir.join("libApi.a"))
		.arg(&source));
	let exe = dir.join("api-test");
	run(swiftc(&module)
		.args(["-parse-as-library", "-I"])
		.arg(&dir)
		.arg(manifest_dir().join("tests").join("api.swift"))
		.arg(dir.join("libApi.a"))
		.arg(api.staticlib())
		.args(&api.native)
		.arg("-o")
		.arg(&exe));
	let swift = run(&mut Command::new(&exe));
	assert_eq!(
		String::from_utf8_lossy(&swift.stdout),
		api.transcript,
		"the Swift transcript differs from the native one"
	);
	run(Command::new(&exe).arg("futures"));
	run(Command::new(&exe).arg("owned"));
	for (case, message) in [
		("moved", "Counter is closed"),
		("boxed moved", "Node is closed"),
		("lent kept", "Counter is closed"),
		(
			"lent moved",
			"api_counter_into_label: a lent handle's value cannot move out",
		),
		("family", "api_nexts: an IPv6 socket address where IPv4 is taken"),
		("stray", "a path Rust returned is not UTF-8"),
	] {
		let output = Command::new(&exe).arg(case).output().unwrap();
		assert!(!output.status.success(), "Swift {case} did not fail");
		let stderr = String::from_utf8_lossy(&output.stderr);
		assert!(stderr.contains(message), "Swift {case} printed {stderr}");
	}

	// The same Swift, with another contract than the library's in place of its own, refuses
	// it at the first call.
	let generated = std::fs::read_to_string(&source).unwrap();
	let at = generated
		.find("api_contract() == 0x")
		.expect("the Swift checks its contract")
		+ "api_contract() == 0x".len();
	let mut mismatched = generated.clone();
	let digit = if &generated[at..at + 1] == "0" { "1" } else { "0" };
	mismatched.replace_range(at..at + 1, digit);
	let contract = dir.join("contract");
	std::fs::create_dir_all(&contract).unwrap();
	std::fs::write(contract.join("api.swift"), mismatched).unwrap();
	std::fs::write(contract.join("main.swift"), "_ = dropped()\n").unwrap();
	let exe = contract.join("contract");
	run(swiftc(&module)
		.args(["-module-name", "Api"])
		.arg(contract.join("api.swift"))
		.arg(contract.join("main.swift"))
		.arg(api.staticlib())
		.args(&api.native)
		.arg("-o")
		.arg(&exe));
	let output = Command::new(&exe).output().unwrap();
	assert!(!output.status.success(), "a mismatched contract did not fail");
	let stderr = String::from_utf8_lossy(&output.stderr);
	assert!(
		stderr.contains("api.swift and the library are from different builds"),
		"a mismatched contract printed {stderr}"
	);
}

/// swiftc, with warnings as errors, finding the C header's module in `module`.
fn swiftc(module: &Path) -> Command {
	let mut cmd = Command::new("swiftc");
	cmd.args(["-warnings-as-errors", "-I"]).arg(module);
	cmd
}

/// A relative `CARGO_TARGET_DIR` is the caller's, wherever cargo runs.
#[test]
fn relative_target() {
	let dir = scratch().join("relative");
	std::fs::create_dir_all(&dir).unwrap();
	let census = dir.join("census");
	let output = Command::new(env!("CARGO_BIN_EXE_rust2-ffi"))
		.arg("--config")
		.arg(manifest_dir().join("fixtures/api/ffi.toml"))
		.arg("--census")
		.arg(&census)
		.current_dir(&dir)
		.env("CARGO_TARGET_DIR", "build")
		.output()
		.expect("running rust2-ffi");
	assert!(
		output.status.success(),
		"the census failed:\n{}",
		String::from_utf8_lossy(&output.stderr)
	);
	// One census per export.
	assert!(census.join("api-fixture.md").exists() && census.join("dep-fixture.md").exists());
	assert!(dir.join("build").exists());
}

#[test]
fn reject() {
	let out = scratch().join("reject").join("out");
	let output = export("reject", &out, &[]);
	assert!(!output.status.success(), "exporting reject succeeded");
	let stderr = String::from_utf8_lossy(&output.stderr);
	// Each error at its span, then each of its notes at its own.
	let lines: Vec<&str> = stderr.lines().collect();
	let at = |i: usize| {
		lines[i + 1..]
			.iter()
			.take_while(|l| !l.starts_with("error") && !l.starts_with("warning") && !l.starts_with("note: "))
			.find_map(|l| l.trim_start().strip_prefix("--> "))
			.unwrap_or("no span")
	};
	let mut errors: Vec<String> = Vec::new();
	let mut ours = false;
	for (i, line) in lines.iter().enumerate() {
		if let Some(error) = line.strip_prefix("error: rust2-ffi:") {
			errors.push(format!("{}: rust2-ffi:{error}", at(i)));
			ours = true;
		} else if line.starts_with("error") || line.starts_with("warning") {
			ours = false;
		} else if let Some(note) = line.strip_prefix("note: ")
			&& ours
		{
			let error = errors.last_mut().expect("a note follows its error");
			*error += &format!("\n\t{}: note: {note}", at(i));
		}
	}
	errors.sort();
	let expected = std::fs::read_to_string(manifest_dir().join("fixtures/reject/expected.txt")).unwrap();
	if std::env::var_os("UPDATE_EXPECT").is_some() {
		std::fs::write(
			manifest_dir().join("fixtures/reject/expected.txt"),
			errors.join("\n") + "\n",
		)
		.unwrap();
		return;
	}
	assert_eq!(errors.join("\n") + "\n", expected);
}

/// The C library builds a dependency with what the check enabled in it, so a feature
/// enabled directly in one that is no export fails loud.
#[test]
fn feature_of_non_export() {
	let out = scratch().join("feature").join("out");
	let output = export("api", &out, &["--features", "bytes/std"]);
	assert!(!output.status.success(), "exporting with bytes/std succeeded");
	let stderr = String::from_utf8_lossy(&output.stderr);
	assert!(
		stderr
			.contains("--features bytes/std: the export builds bytes with only the features api-fixture enables in it"),
		"{stderr}"
	);
}

/// A one-function package named `name` in its own workspace, written into scratch.
fn package(name: &str) -> PathBuf {
	let package = scratch().join(name);
	std::fs::create_dir_all(package.join("src")).unwrap();
	let manifest = format!("[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[workspace]\n");
	std::fs::write(package.join("Cargo.toml"), manifest).unwrap();
	std::fs::write(package.join("src/lib.rs"), "pub fn one() -> u32 {\n\t1\n}\n").unwrap();
	std::fs::write(package.join("ffi.toml"), format!("package = \"{name}\"\n")).unwrap();
	package
}

/// A census of the package whose config is `config` with these cargo args, to run.
fn census(config: &Path, cargo: &[&str]) -> Command {
	let dir = config.parent().unwrap();
	let mut cmd = Command::new(env!("CARGO_BIN_EXE_rust2-ffi"));
	cmd.arg("--config")
		.arg(config)
		.arg("--census")
		.arg(dir.join("census"))
		.arg("--")
		.args(cargo)
		.env("CARGO_TARGET_DIR", dir.join("build"));
	cmd
}

/// Finding the exports keeps to the check's `--locked`: it neither writes a lockfile the
/// check then accepts, nor passes.
#[test]
fn locked() {
	let package = package("locked");
	let _ = std::fs::remove_file(package.join("Cargo.lock"));
	let output = census(&package.join("ffi.toml"), &["--locked"]).output().unwrap();
	assert!(!output.status.success(), "a census without a lockfile passed --locked");
	assert!(!package.join("Cargo.lock").exists(), "the lockfile was written");
}

/// A package reached through a symlinked directory is the same package, though cargo
/// keeps the link in its paths. On Unix: making a symlink on Windows takes a privilege.
#[cfg(unix)]
#[test]
fn symlinked() {
	let package = package("symlinked");
	let link = scratch().join("symlinked-link");
	if !link.exists() {
		std::os::unix::fs::symlink(&package, &link).unwrap();
	}
	let output = census(&link.join("ffi.toml"), &[]).output().unwrap();
	assert!(
		output.status.success(),
		"the census failed:\n{}",
		String::from_utf8_lossy(&output.stderr)
	);
}

/// Another export is found among what the check builds on the host, whatever target
/// cargo's config names: a version only another platform depends on is no rival.
#[test]
fn other_platform() {
	let package = package("platform");
	for (dir, version) in [("unix", "1.0.0"), ("windows", "2.0.0")] {
		let dep = package.join(dir);
		std::fs::create_dir_all(dep.join("src")).unwrap();
		let manifest = format!("[package]\nname = \"platform-dep\"\nversion = \"{version}\"\nedition = \"2024\"\n");
		std::fs::write(dep.join("Cargo.toml"), manifest).unwrap();
		std::fs::write(dep.join("src/lib.rs"), "pub struct Size {\n\tpub width: u32,\n}\n").unwrap();
	}
	let manifest = "[package]\nname = \"platform\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
		[target.'cfg(unix)'.dependencies]\nunix = { package = \"platform-dep\", path = \"unix\" }\n\n\
		[target.'cfg(windows)'.dependencies]\nwindows = { package = \"platform-dep\", path = \"windows\" }\n\n\
		[workspace]\nexclude = [\"unix\", \"windows\"]\n";
	std::fs::write(package.join("Cargo.toml"), manifest).unwrap();
	std::fs::write(
		package.join("src/lib.rs"),
		"#[cfg(unix)]\nuse unix as dep;\n#[cfg(windows)]\nuse windows as dep;\n\npub fn width(size: dep::Size) -> u32 {\n\tsize.width\n}\n",
	)
	.unwrap();
	std::fs::write(
		package.join("ffi.toml"),
		"package = \"platform\"\n\n[crates.platform-dep]\n",
	)
	.unwrap();
	let output = census(&package.join("ffi.toml"), &[])
		.env("CARGO_BUILD_TARGET", "x86_64-pc-windows-msvc")
		.output()
		.unwrap();
	assert!(
		output.status.success(),
		"the census failed:\n{}",
		String::from_utf8_lossy(&output.stderr)
	);
}

/// Another export is found among what the library builds with: a version only a
/// dev-dependency's features pull in is no rival.
#[test]
fn dev_features() {
	let package = package("dev");
	let write = |path: &str, text: &str| {
		let path = package.join(path);
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, text).unwrap();
	};
	for (dir, version) in [("exp1", "1.0.0"), ("exp2", "2.0.0")] {
		write(
			&format!("{dir}/Cargo.toml"),
			&format!("[package]\nname = \"dev-exp\"\nversion = \"{version}\"\nedition = \"2024\"\n"),
		);
		write(
			&format!("{dir}/src/lib.rs"),
			"pub struct Size {\n\tpub width: u32,\n}\n",
		);
	}
	write(
		"route/Cargo.toml",
		"[package]\nname = \"dev-route\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
		[dependencies]\nexp2 = { package = \"dev-exp\", path = \"../exp2\", optional = true }\n\n\
		[features]\nextra = [\"dep:exp2\"]\n",
	);
	write("route/src/lib.rs", "");
	write(
		"Cargo.toml",
		"[package]\nname = \"dev\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
		[dependencies]\nexp = { package = \"dev-exp\", path = \"exp1\" }\nroute = { package = \"dev-route\", path = \"route\" }\n\n\
		[dev-dependencies]\nroute = { package = \"dev-route\", path = \"route\", features = [\"extra\"] }\n\n\
		[workspace]\nexclude = [\"exp1\", \"exp2\", \"route\"]\n",
	);
	write(
		"src/lib.rs",
		"pub fn width(size: exp::Size) -> u32 {\n\tsize.width\n}\n",
	);
	write("ffi.toml", "package = \"dev\"\n\n[crates.dev-exp]\n");
	let output = census(&package.join("ffi.toml"), &[]).output().unwrap();
	assert!(
		output.status.success(),
		"the census failed:\n{}",
		String::from_utf8_lossy(&output.stderr)
	);
}

/// Another export is found in the checked package's own graph, read plainly: a root
/// package that the workspace's default members leave out, under forced color, reaching
/// an export twice.
#[test]
fn diamond() {
	let package = package("diamond");
	let write = |path: &str, text: &str| {
		let path = package.join(path);
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, text).unwrap();
	};
	// `left` and `right` both depend on the export `shared`, which depends on `leaf`.
	for (name, deps) in [
		("app", ""),
		(
			"left",
			"shared = { package = \"diamond-shared\", path = \"../shared\" }\n",
		),
		(
			"right",
			"shared = { package = \"diamond-shared\", path = \"../shared\" }\n",
		),
		("shared", "leaf = { package = \"diamond-leaf\", path = \"../leaf\" }\n"),
		("leaf", ""),
	] {
		write(
			&format!("{name}/Cargo.toml"),
			&format!(
				"[package]\nname = \"diamond-{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[dependencies]\n{deps}"
			),
		);
		write(
			&format!("{name}/src/lib.rs"),
			"pub struct Size {\n\tpub width: u32,\n}\n",
		);
	}
	write(
		"Cargo.toml",
		"[package]\nname = \"diamond\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
		[dependencies]\nleft = { package = \"diamond-left\", path = \"left\" }\n\
		right = { package = \"diamond-right\", path = \"right\" }\n\
		shared = { package = \"diamond-shared\", path = \"shared\" }\n\n\
		[workspace]\nmembers = [\"app\", \"left\", \"right\", \"shared\", \"leaf\"]\ndefault-members = [\"app\"]\n",
	);
	write(
		"src/lib.rs",
		"pub fn width(size: shared::Size) -> u32 {\n\tsize.width\n}\n",
	);
	write("ffi.toml", "package = \"diamond\"\n\n[crates.diamond-shared]\n");
	let output = census(&package.join("ffi.toml"), &[])
		.env("CARGO_TERM_COLOR", "always")
		.output()
		.unwrap();
	assert!(
		output.status.success(),
		"the census failed:\n{}",
		String::from_utf8_lossy(&output.stderr)
	);
}

/// Another export is bound by the configured package's dependency on it, not its crate
/// name or source: a used dependency whose crate shares an unused export's name stands
/// in for none, and two exports of one crate name export, by names the configured package
/// calls them that are its own crate name and `bytes`, which the glue names too, with
/// their sources remapped.
#[test]
fn lookalike() {
	let package = package("lookalike");
	let write = |path: &str, text: &str| {
		let path = package.join(path);
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, text).unwrap();
	};
	// Both libraries are crate `shared`.
	for name in ["exp", "other"] {
		write(
			&format!("{name}/Cargo.toml"),
			&format!(
				"[package]\nname = \"lookalike-{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[lib]\nname = \"shared\"\n"
			),
		);
		write(
			&format!("{name}/src/lib.rs"),
			"pub struct Size {\n\tpub width: u32,\n}\n\nimpl Size {\n\tpub fn new(width: u32) -> Self {\n\t\tSelf { width }\n\t}\n}\n",
		);
	}
	write(
		"Cargo.toml",
		"[package]\nname = \"lookalike\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[lib]\nname = \"shared\"\n\n\
		[dependencies]\nshared = { package = \"lookalike-exp\", path = \"exp\" }\n\
		bytes = { package = \"lookalike-other\", path = \"other\" }\n\n\
		[workspace]\nexclude = [\"exp\", \"other\"]\n",
	);
	write(
		"src/lib.rs",
		"pub fn width(size: bytes::Size) -> u32 {\n\tsize.width\n}\n",
	);
	write("ffi.toml", "package = \"lookalike\"\n\n[crates.lookalike-exp]\n");
	let output = census(&package.join("ffi.toml"), &[]).output().unwrap();
	assert!(!output.status.success(), "a lookalike stood in for the export");
	let stderr = String::from_utf8_lossy(&output.stderr);
	assert!(
		stderr.contains(
			"[crates.lookalike-exp] needs `lookalike` to use its dependency `shared`, and it uses 0 crates of it"
		),
		"{stderr}"
	);

	write(
		"src/lib.rs",
		"pub fn total(a: shared::Size, b: bytes::Size) -> u32 {\n\ta.width + b.width\n}\n",
	);
	write(
		"ffi.toml",
		"package = \"lookalike\"\n\n[crates.lookalike-exp]\n\n\
		[crates.lookalike-exp.rename]\n\"Size\" = \"exp_size\"\n\n[crates.lookalike-other]\n",
	);
	let out = package.join("out");
	let remap = format!("--remap-path-prefix={}=/src", package.display());
	let output = Command::new(env!("CARGO_BIN_EXE_rust2-ffi"))
		.arg("--config")
		.arg(package.join("ffi.toml"))
		.arg("--out")
		.arg(&out)
		.env("CARGO_TARGET_DIR", package.join("build"))
		.env("RUSTFLAGS", remap)
		.output()
		.unwrap();
	assert!(
		output.status.success(),
		"the export failed:\n{}",
		String::from_utf8_lossy(&output.stderr)
	);
	run(cargo()
		.args(["build", "--quiet", "--manifest-path"])
		.arg(out.join("Cargo.toml"))
		.arg("--target-dir")
		.arg(package.join("glue")));
}

/// Writes each `(path, text)` into `dir`.
fn write_all(dir: &Path, files: &[(&str, &str)]) {
	for (path, text) in files {
		let path = dir.join(path);
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, text).unwrap();
	}
}

/// Export the package in `dir` with cargo `args`, then build its glue offline from
/// here, with the lockfile the check resolved.
fn export_and_build(dir: &Path, args: &[&str]) {
	let out = dir.join("out");
	let output = Command::new(env!("CARGO_BIN_EXE_rust2-ffi"))
		.arg("--config")
		.arg(dir.join("ffi.toml"))
		.arg("--out")
		.arg(&out)
		.arg("--")
		.args(args)
		.env("CARGO_TARGET_DIR", dir.join("build"))
		.output()
		.unwrap();
	assert!(
		output.status.success(),
		"the export failed:\n{}",
		String::from_utf8_lossy(&output.stderr)
	);
	std::fs::copy(dir.join("Cargo.lock"), out.join("Cargo.lock")).unwrap();
	build_offline(&out, &dir.join("glue"));
}

/// Build the export in `out` offline from here, into `target`, once a fetch has brought
/// the registry crates its manifest names, such as `bytes` and `jni-sys`, as any first
/// build would: offline, the build shows it needs none of the config the check read.
fn build_offline(out: &Path, target: &Path) {
	run(cargo()
		.args(["fetch", "--quiet", "--manifest-path"])
		.arg(out.join("Cargo.toml")));
	run(cargo()
		.args(["build", "--quiet", "--offline", "--manifest-path"])
		.arg(out.join("Cargo.toml"))
		.arg("--target-dir")
		.arg(target));
}

/// Another export patched in for a registry dependency is bound by its resolved package,
/// though the dependency names no path, and the glue builds it patched in too, though
/// the dependency goes by its library's crate name. So is a dependency of no platform's,
/// which cargo resolves all the same.
#[test]
fn patched() {
	let package = package("patched");
	write_all(
		&package,
		&[
			(
				"dep/Cargo.toml",
				"[package]\nname = \"rust2-ffi-patched-dep\"\nversion = \"1.0.0\"\nedition = \"2024\"\n\n\
			[lib]\nname = \"rust2_ffi_patched_core\"\n",
			),
			("dep/src/lib.rs", "pub struct Size {\n\tpub width: u32,\n}\n"),
			(
				"inactive/Cargo.toml",
				"[package]\nname = \"rust2-ffi-patched-inactive\"\nversion = \"1.0.0\"\nedition = \"2024\"\n",
			),
			("inactive/src/lib.rs", ""),
			(
				"Cargo.toml",
				"[package]\nname = \"patched\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
			[dependencies]\nrust2-ffi-patched-dep = \"1\"\n\n\
			[target.'cfg(any())'.dependencies]\nrust2-ffi-patched-inactive = \"1\"\n\n\
			[patch.crates-io]\nrust2-ffi-patched-dep = { path = \"dep\" }\n\
			rust2-ffi-patched-inactive = { path = \"inactive\" }\n\n\
			[workspace]\nexclude = [\"dep\", \"inactive\"]\n",
			),
			(
				"src/lib.rs",
				"pub fn width(size: rust2_ffi_patched_core::Size) -> u32 {\n\tsize.width\n}\n",
			),
			("ffi.toml", "package = \"patched\"\n\n[crates.rust2-ffi-patched-dep]\n"),
		],
	);
	export_and_build(&package, &["--offline"]);
}

/// A commit of all of `dir`'s files.
const COMMIT: [&str; 8] = [
	"-c",
	"user.name=t",
	"-c",
	"user.email=t@t",
	"commit",
	"--quiet",
	"-am",
	"commit",
];

/// The URL of a new git repository in `dir`, whose branch `main` holds `files`.
fn repository(dir: &Path, files: &[(&str, &str)]) -> String {
	let _ = std::fs::remove_dir_all(dir);
	write_all(dir, files);
	for args in [
		&["init", "--quiet", "--initial-branch", "main"][..],
		&["add", "."],
		&COMMIT,
	] {
		run(Command::new("git").args(args).current_dir(dir));
	}
	format!("file://{}", dir.display())
}

/// A dependency patched in from a git repository that holds two versions of it, the patch
/// picking one by version, is carried with the version it resolved to.
#[test]
fn git_patched() {
	let package = package("git_patched");
	let manifest =
		|version| format!("[package]\nname = \"rust2-ffi-git-dep\"\nversion = \"{version}\"\nedition = \"2024\"\n");
	let stale = repository(
		&package.join("stale"),
		&[("Cargo.toml", &manifest("1.0.0")), ("src/lib.rs", "")],
	);
	let fresh = repository(
		&package.join("fresh"),
		&[
			("one/Cargo.toml", &manifest("1.0.0")),
			("one/src/lib.rs", ""),
			("two/Cargo.toml", &manifest("2.0.0")),
			("two/src/lib.rs", ""),
		],
	);
	let text = format!(
		"[package]\nname = \"git_patched\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
		[dependencies]\nrust2-ffi-git-dep = {{ git = \"{stale}\" }}\n\n\
		[patch.\"{stale}\"]\nrust2-ffi-git-dep = {{ git = \"{fresh}\", version = \"1\" }}\n\n\
		[workspace]\nexclude = [\"stale\", \"fresh\"]\n"
	);
	write_all(&package, &[("Cargo.toml", &text)]);
	export_and_build(&package, &[]);
}

/// Another export the workspace `[replace]`s, by a spec naming no package, is carried as
/// a `[patch]` of it, which resolves alike since a replacement keeps its version. A
/// replacement from the source it replaces is an error, since no patch can carry it.
#[test]
fn replaced() {
	let package = package("replaced");
	// A git repository the dependency names, whose default branch lacks the field the
	// package reads, named for the package so the spec names it by its URL.
	let manifest = "[package]\nname = \"rust2-ffi-replaced-dep\"\nversion = \"1.0.0\"\nedition = \"2024\"\n";
	let git = package.join("rust2-ffi-replaced-dep");
	let url = repository(
		&git,
		&[
			("Cargo.toml", manifest),
			("src/lib.rs", "pub struct Size {\n\tpub height: u32,\n}\n"),
		],
	);
	run(Command::new("git")
		.args(["checkout", "--quiet", "-b", "fixed"])
		.current_dir(&git));
	write_all(&git, &[("src/lib.rs", "pub struct Size {\n\tpub width: u32,\n}\n")]);
	run(Command::new("git").args(COMMIT).current_dir(&git));
	let files = |replacement: &str| {
		write_all(
			&package,
			&[
				("dep/Cargo.toml", manifest),
				("dep/src/lib.rs", "pub struct Size {\n\tpub width: u32,\n}\n"),
				(
					"Cargo.toml",
					&format!(
						"[package]\nname = \"replaced\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
						[dependencies]\nrust2-ffi-replaced-dep = {{ git = \"{url}\", branch = \"main\" }}\n\n\
						[replace]\n\"{url}#1.0.0\" = {replacement}\n\n\
						[workspace]\nexclude = [\"dep\", \"rust2-ffi-replaced-dep\"]\n"
					),
				),
				(
					"src/lib.rs",
					"pub fn width(size: rust2_ffi_replaced_dep::Size) -> u32 {\n\tsize.width\n}\n",
				),
				(
					"ffi.toml",
					"package = \"replaced\"\n\n[crates.rust2-ffi-replaced-dep]\n",
				),
			],
		);
	};
	files("{ path = \"dep\" }");
	export_and_build(&package, &[]);
	// Not an export, which is a path package, but what the package builds with.
	files(&format!("{{ git = \"{url}\", branch = \"fixed\" }}"));
	write_all(
		&package,
		&[("ffi.toml", "package = \"replaced\"\nexclude = [\"width\"]\n")],
	);
	let output = census(&package.join("ffi.toml"), &[]).output().unwrap();
	assert!(!output.status.success(), "a replacement from its own source passed");
	let stderr = String::from_utf8_lossy(&output.stderr);
	assert!(
		stderr.contains("rust2-ffi-replaced-dep 1.0.0 is replaced from its own source"),
		"{stderr}"
	);
}

/// Patches in cargo's config, which cargo reads only where the check runs, reach the glue
/// as the check resolved them: an ancestor's file through its `include`, with paths from
/// the directory holding its `.cargo`, outranking the manifest's patch of the same
/// dependency under another spelling of its source, and a `--config` value's `include`,
/// from the package's directory. Both name that source by a registry only the config
/// defines, whose index the environment overrides.
#[test]
fn config_patched() {
	let root = scratch().join("config-patched");
	let write = |path: &str, text: &str| {
		let path = root.join(path);
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, text).unwrap();
	};
	let dep = |dir: &str, name: &str, item: &str| {
		write(
			&format!("{dir}/Cargo.toml"),
			&format!("[package]\nname = \"{name}\"\nversion = \"1.0.0\"\nedition = \"2024\"\n"),
		);
		write(&format!("{dir}/src/lib.rs"), item);
	};
	dep(
		"pkg/dep",
		"rust2-ffi-config-dep",
		"pub struct Size {\n\tpub width: u32,\n}\n",
	);
	// What the manifest patches in, which lacks the field the package reads.
	dep(
		"pkg/stale",
		"rust2-ffi-config-dep",
		"pub struct Size {\n\tpub height: u32,\n}\n",
	);
	dep(
		"pkg/cli",
		"rust2-ffi-config-cli",
		"pub struct Margin {\n\tpub width: u32,\n}\n",
	);
	write(".cargo/config.toml", "include = [\"patch.toml\"]\n");
	write(
		".cargo/patch.toml",
		"[registries.index]\nindex = \"https://example.com/index\"\n\n\
		[patch.index]\nrust2-ffi-config-dep = { path = \"pkg/dep\" }\n",
	);
	write(
		"pkg/.cargo/cli.toml",
		"[patch.index]\nrust2-ffi-config-cli = { path = \"cli\" }\n",
	);
	write(
		"pkg/Cargo.toml",
		"[package]\nname = \"config-patched\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
		[dependencies]\nrust2-ffi-config-cli = \"1\"\nrust2-ffi-config-dep = \"1\"\n\n\
		[patch.'https://github.com/rust-lang/crates.io-index']\nrust2-ffi-config-dep = { path = \"stale\" }\n\n\
		[workspace]\nexclude = [\"cli\", \"dep\", \"stale\"]\n",
	);
	write(
		"pkg/src/lib.rs",
		"pub fn width(size: rust2_ffi_config_dep::Size, cli: rust2_ffi_config_cli::Margin) -> u32 {\n\
		\tsize.width + cli.width\n}\n",
	);
	write(
		"pkg/ffi.toml",
		"package = \"config-patched\"\n\n[crates.rust2-ffi-config-cli]\n\n[crates.rust2-ffi-config-dep]\n",
	);
	let out = root.join("out");
	let output = Command::new(env!("CARGO_BIN_EXE_rust2-ffi"))
		.arg("--config")
		.arg(root.join("pkg/ffi.toml"))
		.arg("--out")
		.arg(&out)
		.args(["--", "--offline", "--config", "include = [\".cargo/cli.toml\"]"])
		.env("CARGO_TARGET_DIR", root.join("build"))
		// What the check resolves `index` by, outranking its config.
		.env(
			"CARGO_REGISTRIES_INDEX_INDEX",
			"https://github.com/rust-lang/crates.io-index",
		)
		.output()
		.unwrap();
	assert!(
		output.status.success(),
		"the export failed:\n{}",
		String::from_utf8_lossy(&output.stderr)
	);
	// Built from here, where none of that config is read.
	build_offline(&out, &root.join("glue"));
}
