#!/usr/bin/env python3
"""Check sh/dispatch.sh's selection against real diffs in a scratch repository."""

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

DISPATCH = Path(__file__).with_name("dispatch.sh").resolve()
EVERYTHING = ["lint", "quest", "sh", "uniffi", "driver", "ffi", "go", "ts"]


class SelectTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)
        (self.repo / "sh").mkdir()
        shutil.copy(DISPATCH, self.repo / "sh" / "dispatch.sh")
        self.env = {
            "PATH": os.environ["PATH"],
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_CONFIG_GLOBAL": os.devnull,
        }
        self.git("init", "-q", "-b", "main")
        self.git("config", "user.name", "test")
        self.git("config", "user.email", "test@example.com")
        self.git("add", ".")
        self.git("commit", "-qm", "base")
        self.git("update-ref", "refs/remotes/origin/main", "HEAD")

    def git(self, *args):
        return subprocess.run(
            ["git", *args], cwd=self.repo, env=self.env, text=True, capture_output=True, check=True
        ).stdout.strip()

    def change(self, *paths):
        for path in paths:
            file = self.repo / path
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text(file.read_text() + "\n" if file.exists() else "changed\n")

    def commit(self, *paths):
        self.change(*paths)
        self.git("add", ".")
        self.git("commit", "-qm", "change")
        return self.git("rev-parse", "HEAD")

    def query(self, *base):
        return subprocess.run(
            ["bash", "sh/dispatch.sh", "ci", *base, "--query"],
            cwd=self.repo,
            env=self.env,
            text=True,
            capture_output=True,
        )

    def select(self, *base):
        result = self.query(*base)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.split()

    def test_projects_select_themselves_and_the_lints(self):
        for paths, expected in (
            (["ts/src/lib.rs"], ["lint", "ts"]),
            (["ffi/src/lib.rs"], ["lint", "ffi"]),
            (["go/src/lib.rs"], ["lint", "go"]),
            (["ts/fixtures/codec/src/lib.rs"], ["lint", "go", "ts"]),
            (["uniffi/uniffi_core/src/lib.rs"], ["lint", "uniffi"]),
            (["quest/m0/example.md"], ["lint", "quest"]),
            (["sh/dispatch.test.py"], ["lint", "sh"]),
            (["ts/src/lib.rs", "ffi/src/lib.rs"], ["lint", "ffi", "ts"]),
        ):
            with self.subTest(paths=paths):
                self.change(*paths)
                self.assertEqual(self.select(), expected)
                self.git("clean", "-fdq")

    def test_the_driver_selects_its_dependents(self):
        self.change("driver/src/lib.rs")
        self.assertEqual(self.select(), ["lint", "driver", "ffi", "go", "ts"])

    def test_the_shared_nightly_selects_each_project_on_it(self):
        self.change("ts/rust-toolchain.toml")
        self.assertEqual(self.select(), ["lint", "driver", "ffi", "go", "ts"])

    def test_lint_only_paths_select_only_the_lints(self):
        for path in ("bench/ffi/run.sh", "README.md", ".claude/settings.json"):
            with self.subTest(path=path):
                self.change(path)
                self.assertEqual(self.select(), ["lint"])
                self.git("clean", "-fdq")

    def test_empty_diff_keeps_the_lints(self):
        self.assertEqual(self.select(), ["lint"])
        self.assertEqual(self.select(""), ["lint"])

    def test_shared_inputs_select_everything(self):
        for path in (
            "flake.nix",
            "flake.lock",
            "rust-toolchain.toml",
            "Cargo.toml",
            "Cargo.lock",
            "justfile",
            ".github/workflows/nightly.yml",
            "sh/dispatch.sh",
            # An unmapped project runs everything until the map claims it.
            "next/src/lib.rs",
        ):
            with self.subTest(path=path):
                self.change(path)
                self.assertEqual(self.select(), EVERYTHING)
                self.git("checkout", "-q", ".")
                self.git("clean", "-fdq")

    def test_all_selects_everything(self):
        self.assertEqual(self.select("--all"), EVERYTHING)

    def test_deletions_count(self):
        self.commit("ts/src/lib.rs")
        self.git("update-ref", "refs/remotes/origin/main", "HEAD")
        (self.repo / "ts/src/lib.rs").unlink()
        self.assertEqual(self.select(), ["lint", "ts"])

    def test_base_changes_since_the_branch_do_not_count(self):
        branch = self.git("rev-parse", "HEAD")
        self.commit("uniffi/uniffi_core/src/lib.rs")
        self.git("update-ref", "refs/remotes/origin/main", "HEAD")
        self.git("checkout", "-q", "-b", "work", branch)
        self.commit("ts/src/lib.rs")
        self.assertEqual(self.select(), ["lint", "ts"])

    def test_land_gate_base_comes_before_origin_main(self):
        base = self.git("rev-parse", "HEAD")
        self.commit("ts/src/lib.rs")
        # The gate's clone has no remote; origin/main standing at HEAD selects nothing.
        self.git("update-ref", "refs/remotes/origin/main", "HEAD")
        self.assertEqual(self.select(), ["lint"])
        self.env["QUEST_LAND_BASE"] = base
        self.assertEqual(self.select(), ["lint", "ts"])
        # An explicit base still wins.
        self.assertEqual(self.select("HEAD"), ["lint"])

    def test_an_unknown_base_fails(self):
        result = self.query("no-such-ref")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no-such-ref", result.stderr)

    def test_usage_errors(self):
        for args in (["deploy"], ["ci", "origin/main", "--all"], ["ci", "a", "--query", "extra"]):
            with self.subTest(args=args):
                result = subprocess.run(
                    ["bash", "sh/dispatch.sh", *args], cwd=self.repo, env=self.env, text=True, capture_output=True
                )
                self.assertEqual(result.returncode, 2, result.stderr)


if __name__ == "__main__":
    unittest.main()
