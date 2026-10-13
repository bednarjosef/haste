#!/usr/bin/env bash
# Removes worktrees under ../haste-wt whose branch is merged into origin/main, to keep disk use
# down (each worktree's target/ is several GB). A worktree is kept when:
#   - its branch is not merged (or it is detached),
#   - it has uncommitted changes or untracked files (`git status --porcelain` not empty),
#   - any process is working in it (a worker's shell, cargo, an agent: /proc/*/cwd inside it).
# The branches themselves stay. Safe to run any time, e.g. after each merge or from a timer.
#   scripts/prune-worktrees.sh [--dry-run]
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/env.sh"
DRY=${1:-}
cd "$HASTE_ROOT"
# The worktree directory next to the main checkout (env.sh's default is wrong when this runs
# from inside a worktree).
MAIN=$(git worktree list --porcelain | awk 'NR==1{print $2}')
WT_DIR=$(dirname "$MAIN")/haste-wt
git fetch -q origin || true
busy=$(for p in /proc/[0-9]*; do readlink "$p/cwd" 2>/dev/null || true; done | sort -u)
removed=0; kept=0
while read -r wt branch; do
  case "$wt" in "$WT_DIR"/*) ;; *) continue ;; esac
  [ "$branch" = "-" ] && { kept=$((kept+1)); continue; }
  git merge-base --is-ancestor "$branch" origin/main 2>/dev/null || { kept=$((kept+1)); continue; }
  [ -n "$(git -C "$wt" status --porcelain 2>/dev/null)" ] && { kept=$((kept+1)); continue; }
  if grep -qxF -e "$wt" <<<"$busy" || grep -q "^$wt/" <<<"$busy"; then kept=$((kept+1)); continue; fi
  if [ "$DRY" = "--dry-run" ]; then echo "would remove $wt ($branch)"; else
    git worktree remove "$wt" && removed=$((removed+1)) && echo "removed $wt ($branch)"; fi
done < <(git worktree list --porcelain | awk '/^worktree /{w=$2; b="-"} /^branch /{b=$2; sub("refs/heads/","",b)} /^$/{print w" "b}')
git worktree prune
echo "prune-worktrees: removed $removed, kept $kept"
