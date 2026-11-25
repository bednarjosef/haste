#!/usr/bin/env bash
# Removes worktrees under ../haste-wt whose branch is merged into the local main, to keep disk use
# down (each worktree's target/ is several GB). A worktree is kept when:
#   - its branch is not merged (or it is detached), or it never had a commit of its own (new),
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
  # The local main, not origin/main: merges land locally first (local-merge.sh) and origin can
  # lag far behind while GitHub is unreachable, which kept every worktree and filled the disk.
  git merge-base --is-ancestor "$branch" main 2>/dev/null || { kept=$((kept+1)); continue; }
  # A freshly created branch with no commits of its own is also "contained in main": keep it until
  # it has had a commit (its reflog has more than the creation entry), so a worker's new worktree
  # is never removed before its first commit.
  [ "$(git reflog show --format=%gs "refs/heads/$branch" 2>/dev/null | wc -l)" -le 1 ] && { kept=$((kept+1)); continue; }
  # Uncommitted changes to tracked files keep the worktree; untracked files do not: they are the
  # scratch a worker's sandbox could not delete (dumps, .tmp copies, helper scripts), and keeping
  # every such worktree held ~190 GB of target/ dirs on 2026-10-19.
  [ -n "$(git -C "$wt" status --porcelain --untracked-files=no 2>/dev/null)" ] && { kept=$((kept+1)); continue; }
  if grep -qxF -e "$wt" <<<"$busy" || grep -q "^$wt/" <<<"$busy"; then kept=$((kept+1)); continue; fi
  if [ "$DRY" = "--dry-run" ]; then echo "would remove $wt ($branch)"; else
    git worktree remove --force "$wt" && removed=$((removed+1)) && echo "removed $wt ($branch)"; fi
done < <(git worktree list --porcelain | awk '/^worktree /{w=$2; b="-"} /^branch /{b=$2; sub("refs/heads/","",b)} /^$/{print w" "b}')
git worktree prune
echo "prune-worktrees: removed $removed, kept $kept"
