set shell := ["bash", "-euo", "pipefail", "-c"]

# The TypeScript driver, its own workspace on its own nightly: `just ts <recipe>`.
mod ts

# The C exporter, its own workspace on the TypeScript driver's nightly: `just ffi <recipe>`.
mod ffi

# The Go translator, its own workspace on the TypeScript driver's nightly: `just go <recipe>`.
mod go

# The drivers' shared front end and rust2-lint, its own workspace on the TypeScript driver's nightly: `just driver <recipe>`.
mod driver

# The UniFFI fork, in the root workspace: `just uniffi <recipe>`.
mod uniffi

# List the available development commands.
default:
    @just --list

# Lint the repository tooling, the quest tree, the root workspace, and every project.
check:
    quest check
    actionlint
    shellcheck .claude/hooks/*.sh .codex/hooks/*.sh bench/*/*.sh
    shfmt -d .claude/hooks/*.sh .codex/hooks/*.sh bench/*/*.sh
    taplo format --check
    nixfmt --check flake.nix bench/build/nix.nix
    cargo fmt --all --check
    cargo clippy --locked --workspace --all-targets -- -D warnings
    just ts check
    just ffi check
    just go check
    just driver check
    just uniffi check

# Run what CI's Check runs: the lints, then the tests.
ci:
    just check
    just test

# Run the tests.
test:
    bash .claude/hooks/direnv.test.sh
    just ts test
    just ffi test
    just go test
    just driver test
    just uniffi test

# Measure Rust build time on the pinned corpus; see bench/build/README.md.
bench *args:
    bench/build/run.sh {{ args }}

# Measure moq's Nix release builds in CPU time, built each way; see bench/build/README.md.
bench-nix *args:
    bench/build/nix.sh {{ args }}

# Measure what moq's binding wrappers add over Rust, against the UniFFI fork; see bench/ffi/README.md.
bench-ffi *args:
    bench/ffi/run.sh {{ args }}

# Apply formatters without changing program behavior.
fix:
    shfmt -w .claude/hooks/*.sh .codex/hooks/*.sh bench/*/*.sh
    taplo format
    nixfmt flake.nix bench/build/nix.nix
    cargo fmt --all
    just ts fix
    just ffi fix
    just go fix
    just driver fix
