#!/usr/bin/env bash
# Runs CI locally: the same checks as .github/workflows/ci.yml, limited to what a change can affect.
# usage: scripts/ci-local.sh [BASE]        (BASE defaults to main; the change is BASE..HEAD)
#
# Order, failing fast: the personal-data guard, cargo fmt, clippy (-D warnings, all targets) and
# the tests. Only the crates whose files changed and the workspace crates depending on them are
# linted and tested; a change to the workspace manifest, the lockfile or the toolchain checks the
# whole workspace, and a change that touches no Rust or build file skips cargo entirely. When the
# runtime Java (crates/haste-jvm/java) changed and a test server is up, the pause and language
# feature checks run too: the runtime javac compiles that code only inside the game.
#
# env: CI_JOBS (cargo -j, default 4), CI_GAME_CHECKS=0 to skip the feature checks,
#      CARGO_TARGET_DIR (reuse a warm target directory between runs).
set -uo pipefail
cd "$(dirname "$0")/.."
base="${1:-main}"
jobs="${CI_JOBS:-4}"
step() { printf '== %s\n' "$*"; }
fail() { printf '!! FAILED: %s\n' "$*"; exit 1; }

changed=$(git diff --name-only "$base" HEAD)
[ -n "$changed" ] || { echo "no changes against $base"; exit 0; }

step "personal data"
scripts/check-no-personal.sh >/dev/null || { scripts/check-no-personal.sh; fail "personal data"; }

if grep -qE '\.rs$' <<<"$changed"; then
  step "format"
  cargo fmt --all --check >/dev/null 2>&1 || { cargo fmt --all --check | head -40; fail "cargo fmt"; }
fi

# The crates to check: those owning a changed file plus every workspace crate depending on them.
crates=$(cargo metadata --no-deps --format-version 1 2>/dev/null | python3 -c '
import json, sys, os
meta = json.load(sys.stdin)
changed = [l for l in sys.argv[1].splitlines() if l]
root = meta["workspace_root"]
whole = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml"}
if any(c in whole for c in changed):
    print("--workspace"); sys.exit()
dirs = {p["name"]: os.path.relpath(os.path.dirname(p["manifest_path"]), root) for p in meta["packages"]}
deps = {p["name"]: [d["name"] for d in p["dependencies"] if d.get("path")] for p in meta["packages"]}
hit = {n for n, d in dirs.items() if any(c == d or c.startswith(d + "/") for c in changed)}
while True:
    more = {n for n, ds in deps.items() if n not in hit and any(x in hit for x in ds)}
    if not more: break
    hit |= more
print(" ".join("-p " + n for n in sorted(hit)))
' "$changed")

if [ -n "$crates" ]; then
  step "clippy ($crates)"
  # shellcheck disable=SC2086
  cargo clippy -q -j "$jobs" $crates --all-targets -- -D warnings || fail "clippy"
  step "tests ($crates)"
  # shellcheck disable=SC2086
  out=$(cargo test -q -j "$jobs" $crates 2>&1) || { grep -E 'FAILED|panicked|error(\[|:)' <<<"$out" | head -30; fail "tests"; }
  grep -E '^test result' <<<"$out" | awk '{p += $4; f += $6; i += $8} END {printf "   %d passed, %d failed, %d ignored\n", p, f, i}'
else
  step "no crate changed: cargo skipped"
fi

if [ "${CI_GAME_CHECKS:-1}" != 0 ] && grep -q '^crates/haste-jvm/java/' <<<"$changed"; then
  if (exec 3<>/dev/tcp/127.0.0.1/25691) 2>/dev/null; then
    step "game checks (runtime Java changed): pause language"
    cargo build -q -j "$jobs" --release -p haste-client || fail "release build"
    target="${CARGO_TARGET_DIR:-$PWD/target}"
    out=$(HASTE_BIN="$target/release/haste" bash scripts/checks/run-feature-checks.sh pause language 2>&1)
    rc=$?
    grep -E '^(PASS|FAIL)|passed,' <<<"$out"
    [ "$rc" -eq 0 ] || fail "game checks"
  else
    step "game checks skipped: no test server on 127.0.0.1:25691"
  fi
fi
step "CI passed"
