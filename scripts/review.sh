#!/usr/bin/env bash
# Lead's review helper: shows what a workstream branch changed and runs its checks in its worktree.
# usage: scripts/review.sh <ws-branch> [crate...]
set -uo pipefail
WS=$1; shift
W=$HASTE_WT/$WS
cd "$W" || exit 1
echo "== $WS: $(git log --oneline main..HEAD | wc -l) commits ahead of main"
git log --oneline main..HEAD | head -40
echo "== files changed"
git diff --stat main...HEAD | tail -40
echo "== forbidden paths touched?"
git diff --name-only main...HEAD | grep -E '^(crates/haste-core/|docs/INTERFACES.md|Cargo.toml$|crates/haste-client/src/main.rs)' && echo "!! touched lead-owned files" || echo "none"
echo "== cache files committed?"
git diff --name-only main...HEAD | grep -E '^cache/|\.jar$|blocks\.json|registries\.json' && echo "!! derived data committed" || echo "none"
for c in "$@"; do
  echo "== cargo test -p $c"; cargo test -p "$c" 2>&1 | grep -E '^(test result|error|warning: unused|failures)' | head -20
  echo "== cargo clippy -p $c"; cargo clippy -p "$c" -- -D warnings 2>&1 | grep -E '^(error|warning)' | head -20; echo "clippy exit: ${PIPESTATUS[0]}"
done
