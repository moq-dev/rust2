#!/usr/bin/env bash
# Run `just check`, `just test`, or `just ci` over the modules the branch changed
# since BASE: the queue's explicit argument, the fixer's QUEST_LAND_BASE, or origin/main.
# --all runs every module. --query prints the selected modules instead.
set -euo pipefail

usage="usage: sh/dispatch.sh check|test|ci [BASE|--all] [--query]"
action=${1:-}
base=${2:-}
query=${3:-}
# BASE is optional before --query.
if [[ "$base" == --query && $# -eq 2 ]]; then
    base='' query=--query
fi
if [[ $# -gt 3 || ! "$action" =~ ^(check|test|ci)$ || (-n "$query" && "$query" != --query) ]]; then
    echo "$usage" >&2
    exit 2
fi

cd "$(git rev-parse --show-toplevel)"

# The impact map: a module runs when a changed path matches its pattern. An
# empty pattern runs on every diff.
declare -A scope=(
    [lint]=''
    [quest]='^quest/'
    [sh]='^sh/dispatch\.test\.py$'
    [uniffi]='^uniffi/'
    # The driver and ffi link ts's toolchain, and both of the others build on
    # the driver's front end.
    [driver]='^(driver/|ts/rust-toolchain\.toml$)'
    [ffi]='^(ffi/|driver/|ts/rust-toolchain\.toml$)'
    # go's codec fixture includes ts's codec sources.
    [go]='^(go/|driver/|ts/rust-toolchain\.toml$|ts/fixtures/codec/)'
    [ts]='^(ts|driver)/'
)
# Cheapest first, so a lint fails before the builds start.
modules=(lint quest sh uniffi driver ffi go ts)

# Paths that only the repository-wide lints read.
lints='^(bench/|\.agents/|\.claude/|\.codex/|[^/]+\.md$|LICENSE-|\.(editorconfig|gitattributes|gitignore|taplo\.toml)$)'

selected=()
if [[ "$base" == --all ]]; then
    selected=("${modules[@]}")
else
    base=${base:-${QUEST_LAND_BASE:-origin/main}}
    merge_base=$(git merge-base "$base" HEAD) || {
        echo "error: cannot resolve a merge base with $base; pass BASE, or run \`just gate\`" >&2
        exit 1
    }
    echo "$action: changes since $base" >&2
    # Untracked files count, so a new file selects its module, and a rename
    # lists both sides, since a reader of the old path is affected too.
    changed=$({
        git diff --name-only --no-renames "$merge_base"
        git ls-files --others --exclude-standard
    } | sort -u)

    # A path no module claims is a shared input, such as the flake, a
    # toolchain, the root Cargo.* and justfile, .github, or this map, so it
    # selects everything. A new project selects everything until it is mapped.
    claimed=$lints
    for module in "${modules[@]}"; do
        [[ -z "${scope[$module]}" ]] || claimed+="|${scope[$module]}"
    done
    shared=$(grep -vE "$claimed" <<<"$changed" | grep . || true)
    if [[ -n "$shared" ]]; then
        echo "$action: running everything for $(paste -sd ' ' <<<"$shared")" >&2
        selected=("${modules[@]}")
    else
        for module in "${modules[@]}"; do
            pattern=${scope[$module]}
            if [[ -z "$pattern" ]] || grep -qE "$pattern" <<<"$changed"; then
                selected+=("$module")
            fi
        done
    fi
fi

if [[ -n "$query" ]]; then
    printf '%s\n' "${selected[@]}"
    exit
fi

scripts=(bench/*/*.sh sh/*.sh)

# One function per module and phase; a module skips a phase it has none for.
check_lint() {
    actionlint
    shellcheck "${scripts[@]}"
    shfmt -d "${scripts[@]}"
    taplo format --check
    nixfmt --check flake.nix bench/build/nix.nix
}
check_quest() { quest check; }
# The root workspace holds only the UniFFI fork.
check_uniffi() {
    cargo fmt --all --check
    cargo clippy --locked --workspace --all-targets -- -D warnings
    just uniffi check
}
check_driver() { just driver check; }
check_ffi() { just ffi check; }
check_go() { just go check; }
check_ts() { just ts check; }
test_sh() { python3 -I sh/dispatch.test.py; }
test_uniffi() { just uniffi test; }
test_driver() { just driver test; }
test_ffi() { just ffi test; }
test_go() { just go test; }
# moq-net's translation at the pin, its first consumer, before the slower test build.
test_ts() {
    just ts moq
    just ts test
}

# Every check runs before any test, so a lint fails before the test builds.
case "$action" in
    check | test) phases=("$action") ;;
    ci) phases=(check test) ;;
esac
for phase in "${phases[@]}"; do
    for module in "${selected[@]}"; do
        declare -F "${phase}_$module" >/dev/null || continue
        echo "==> $phase $module" >&2
        "${phase}_$module"
    done
done
