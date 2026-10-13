#!/usr/bin/env bash
# Lead's review helper: shows what a workstream branch changed and runs its checks in its worktree.
# usage: scripts/review.sh <ws-branch> [crate...]
#   REVIEW_SERVER=host:port adds a normal-world screenshot step (needs a running server).
set -uo pipefail
WS=$1; shift
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
W=${HASTE_WT:-$(dirname "$ROOT")/haste-wt}/$WS
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
if [[ -n "${REVIEW_SERVER:-}" ]]; then
  echo "== screenshot (server $REVIEW_SERVER)"
  cargo build -q -p haste-client || echo "!! screenshot failed"
  mkdir -p target/review
  png=target/review/$WS.png; log=target/review/$WS.log
  rm -f "$png"
  target/debug/haste --server "$REVIEW_SERVER" --username review --screenshot "$png" \
    --yaw 180 --pitch 25 --seconds 60 >"$log" 2>&1
  status=$?
  if [[ $status -eq 0 && -f $png ]]; then
    echo "$png"
    tail -5 "$log"
  else
    echo "!! screenshot failed (haste exit $status)"
    tail -5 "$log" 2>/dev/null
  fi
else
  echo "== screenshot: skipped (set REVIEW_SERVER=host:port)"
fi
