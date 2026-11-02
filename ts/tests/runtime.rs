//! The runtime's own behavior, beyond what translated fixtures exercise.

use std::path::PathBuf;
use std::process::Command;

/// `rt.spawn` yields to a macrotask once a batch spends its poll budget, and a signal cancels it.
#[test]
fn spawn() {
	let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("spawn.ts");
	let output = Command::new("node")
		.arg(script)
		.output()
		.expect("running node, which the dev shell provides");
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

/// A NaN keeps its payload and sign through `f64`'s and `f32`'s bits, which `nan.ts` checks in Node.
#[test]
fn nan_payloads() {
	let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("nan.ts");
	let output = Command::new("node")
		.arg(script)
		.output()
		.expect("running node, which the dev shell provides");
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

/// A recursive `OnceLock` init, or `LazyLock` or `LazyCell` force, panics with std's
/// message, which `once.ts` checks.
#[test]
fn once_recursive() {
	let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("once.ts");
	let output = Command::new("node")
		.arg(script)
		.output()
		.expect("running node, which the dev shell provides");
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

/// A `BytesMut` whose storage was shared throws on a reserve past its limit, which
/// `reserve.ts` checks.
#[test]
fn bytes_mut_reserve() {
	let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.join("tests")
		.join("reserve.ts");
	let output = Command::new("node")
		.arg(script)
		.output()
		.expect("running node, which the dev shell provides");
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

/// A lazy thread-local's first value comes as wasm32's `LazyStorage` makes it, which
/// `local.ts` checks.
#[test]
fn local_lazy() {
	let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("local.ts");
	let output = Command::new("node")
		.arg(script)
		.output()
		.expect("running node, which the dev shell provides");
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

/// The runtime's B-tree runs `btree.ts`'s script of random operations as std's does.
#[test]
fn btree_script() {
	use std::collections::BTreeMap;

	let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("btree.ts");
	let output = Command::new("node")
		.arg(script)
		.output()
		.expect("running node, which the dev shell provides");
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	let got = String::from_utf8(output.stdout).unwrap();

	// xorshift32, as `btree.ts` runs it.
	let mut state: u32 = 0x2545f491;
	let mut rand = move || {
		state ^= state << 13;
		state ^= state >> 17;
		state ^= state << 5;
		state
	};
	fn walk<'a>(
		rand: &mut impl FnMut() -> u32,
		it: &mut impl DoubleEndedIterator<Item = (&'a u32, &'a u32)>,
	) -> String {
		let mut seen = Vec::new();
		for _ in 0..40 {
			let item = if rand() & 1 == 1 { it.next() } else { it.next_back() };
			let Some((k, _)) = item else { break };
			seen.push(k.to_string());
		}
		seen.join(",")
	}
	let show = |v: Option<u32>| v.map_or("-".to_string(), |v| v.to_string());
	let entry = |e: Option<(&u32, &u32)>| e.map_or("-".to_string(), |(k, v)| format!("{k} {v}"));

	let mut want = Vec::new();
	let mut m: BTreeMap<u32, u32> = BTreeMap::new();
	let steps = 120_000;
	for step in 0..steps {
		let keys = 65_536;
		let phase = 3 * step / steps;
		let op = rand() % 10;
		let k = rand() % keys;
		if op < 5 {
			if phase == 0 || (phase == 1 && k % 2 == 0) {
				want.push(format!("insert {k} {}", show(m.insert(k, step))));
			} else {
				want.push(format!("remove {k} {}", show(m.remove(&k))));
			}
		} else if op == 5 {
			want.push(format!("get {k} {}", show(m.get(&k).copied())));
		} else if op == 6 {
			let j = rand() % keys;
			let (lo, hi) = if k < j { (k, j) } else { (j, k) };
			let inclusive = rand() & 1;
			let seen = match inclusive {
				1 => walk(&mut rand, &mut m.range(lo..=hi)),
				_ => walk(&mut rand, &mut m.range(lo..hi)),
			};
			want.push(format!("range {lo} {hi} {inclusive} {seen}"));
		} else if op == 7 {
			let last = rand() & 1 == 1;
			let popped = if last { m.pop_last() } else { m.pop_first() };
			want.push(format!(
				"pop {last} {}",
				popped.map_or("-".to_string(), |(k, v)| format!("{k} {v}"))
			));
		} else if op == 8 {
			want.push(format!("remove {k} {}", show(m.remove(&k))));
		} else if step % 97 == 0 {
			let mut it = m.iter();
			let seen = walk(&mut rand, &mut it);
			want.push(format!("iter {} {} {seen}", m.len(), it.len()));
		} else if step % 1009 == 0 {
			let copy = m.clone();
			m.retain(|key, _| key % 7 != k % 7);
			want.push(format!(
				"retain {} {} {}",
				m.len(),
				entry(m.first_key_value()),
				entry(m.last_key_value())
			));
			if rand() & 1 == 1 {
				m = copy;
			}
			want.push(format!("kept {}", m.len()));
		}
	}
	let keys: Vec<String> = m.keys().map(u32::to_string).collect();
	want.push(format!("end {} {}", m.len(), keys.join(",")));
	let mut drained = Vec::new();
	while !m.is_empty() {
		let popped = if rand() & 1 == 1 { m.pop_last() } else { m.pop_first() };
		drained.push(popped.unwrap().0.to_string());
	}
	want.push(format!("drain {}", drained.join(",")));
	for (got, want) in got.lines().zip(&want) {
		assert_eq!(got, want);
	}
	assert_eq!(got.lines().count(), want.len());
}

/// A value that logs itself when it drops.
struct Logged<'a, T: Copy>(T, &'a std::cell::RefCell<Vec<T>>);

impl<T: Copy> Drop for Logged<'_, T> {
	fn drop(&mut self) {
		self.1.borrow_mut().push(self.0);
	}
}

/// `retain` leaves a B-tree, a deque, and a `Vec` as std's do, and `dedup_by` a `Vec`,
/// each dropping what std's drops, which `retain.ts` prints for every keep mask of up to
/// 6 items.
#[test]
fn retain() {
	use std::collections::{BTreeMap, VecDeque};

	let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.join("tests")
		.join("retain.ts");
	let output = Command::new("node")
		.arg(script)
		.output()
		.expect("running node, which the dev shell provides");
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	let got = String::from_utf8(output.stdout).unwrap();

	let mut want = String::new();
	for n in 0..=6u32 {
		for mask in 0..1u32 << n {
			let keep = |i: u32| (mask >> i) & 1 == 1;
			let mut tree: BTreeMap<u32, u32> = (0..n).map(|i| (i, i * 10)).collect();
			tree.retain(|&k, _| keep(k));
			let mut deque = VecDeque::new();
			for i in (0..n / 2).rev() {
				deque.push_front(i);
			}
			deque.extend(n / 2..n);
			deque.retain(|&i| keep(i));
			let dropped = std::cell::RefCell::new(Vec::new());
			let mut vec: Vec<Logged<u32>> = (0..n).map(|i| Logged(i, &dropped)).collect();
			vec.retain(|i| keep(i.0));
			let vec: Vec<u32> = vec.iter().map(|i| i.0).collect();
			let dropped = std::mem::take(&mut *dropped.borrow_mut());
			// A run ends at each item the mask clears.
			let deduped = std::cell::RefCell::new(Vec::new());
			let mut run = 0;
			let mut runs: Vec<Logged<(u32, u32)>> = (0..n)
				.map(|i| {
					if !keep(i) {
						run += 1;
					}
					Logged((i, run), &deduped)
				})
				.collect();
			runs.dedup_by(|a, b| a.0.1 == b.0.1);
			let ran: Vec<u32> = runs.iter().map(|r| r.0.0).collect();
			let deduped: Vec<u32> = std::mem::take(&mut *deduped.borrow_mut())
				.into_iter()
				.map(|r| r.0)
				.collect();
			let entries: Vec<String> = tree.iter().map(|(k, v)| format!("{k},{v}")).collect();
			let items: Vec<String> = deque.iter().map(u32::to_string).collect();
			let join = |v: &[u32]| v.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
			want += &format!(
				"{n} {mask} btree {} deque {} vec {} dropped {} dedup {} dropped {}\n",
				entries.join(";"),
				items.join(","),
				join(&vec),
				join(&dropped),
				join(&ran),
				join(&deduped),
			);
		}
	}
	for (got, want) in got.lines().zip(want.lines()) {
		assert_eq!(got, want);
	}
	assert_eq!(got.lines().count(), want.lines().count());
}

/// `rt/unicode.ts` holds the chars `Debug` escapes as `\u{..}` by this toolchain's std,
/// whose Unicode version the JS engine's may trail. `UPDATE_EXPECT=1 just ts test --test
/// runtime unicode` rewrites it.
#[test]
fn unicode() {
	let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime/rt/unicode.ts");
	// Alternately how far past the last run's end a run starts, and its length.
	let mut runs = Vec::new();
	let (mut end, mut start) = (0, None);
	for point in 0..=char::MAX as u32 + 1 {
		let escaped = char::from_u32(point).is_some_and(|c| c.escape_debug().to_string().starts_with("\\u"));
		match (escaped, start) {
			(true, None) => start = Some(point),
			(false, Some(s)) => {
				runs.extend([s - end, point - s]);
				(end, start) = (point, None);
			}
			_ => {}
		}
	}
	let mut lines = vec![String::new()];
	for run in runs.iter().map(u32::to_string) {
		if lines.last().unwrap().len() + run.len() + 2 > 90 {
			lines.push(String::new());
		}
		*lines.last_mut().unwrap() += &format!("{run}, ");
	}
	let want = format!(
		"// Generated by `UPDATE_EXPECT=1 just ts test --test runtime unicode` from std. Do not edit.\n\n\
		 /** The chars `Debug` writes as `\\u{{..}}`: alternately how far past the last run's end a run\n \
		 * starts, and its length. */\n\
		 export const UNPRINTABLE: readonly number[] = [\n{}];\n",
		lines
			.iter()
			.map(|l| format!("\t{}\n", l.trim_end()))
			.collect::<String>()
	);
	if std::env::var_os("UPDATE_EXPECT").is_some() {
		std::fs::write(&path, want).unwrap();
		return;
	}
	let got = std::fs::read_to_string(&path).unwrap_or_default();
	assert!(
		got == want,
		"{} differs from std's Unicode; rerun with UPDATE_EXPECT=1",
		path.display()
	);
}

/// A pointer the translation can't follow fails where JS keeps apart what Rust's reaches,
/// which `pointers.ts` checks.
#[test]
fn pointers_apart() {
	let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.join("tests")
		.join("pointers.ts");
	let output = Command::new("node")
		.arg(script)
		.output()
		.expect("running node, which the dev shell provides");
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}
