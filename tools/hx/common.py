"""Shared helpers for hx: the repository root, bounded output."""
import os
import subprocess
import sys

ROOT = os.environ.get("HASTE_ROOT") or subprocess.run(
    ["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True,
    cwd=os.path.dirname(os.path.abspath(__file__))).stdout.strip()



def _cache():
    """$HASTE_CACHE, else the tree's cache/, else the main checkout's (worktrees share it)."""
    if os.environ.get("HASTE_CACHE"):
        return os.environ["HASTE_CACHE"]
    if os.path.isdir(os.path.join(ROOT, "cache")):
        return os.path.join(ROOT, "cache")
    common = subprocess.run(["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
                            capture_output=True, text=True, cwd=ROOT).stdout.strip()
    return os.path.join(os.path.dirname(common), "cache")


CACHE = _cache()

# Every command's output is capped: a worker's context is re-read on every later request.
MAX_LINES = int(os.environ.get("HX_MAX_LINES", "200"))
_printed = 0


def out(line=""):
    global _printed
    _printed += 1
    if _printed == MAX_LINES + 1:
        print(f"... output cut at {MAX_LINES} lines: narrow the query")
    if _printed <= MAX_LINES:
        print(line)


def clip(s, n):
    return s if len(s) <= n else s[: n - 3] + "..."


def rg(args, cwd=ROOT):
    """ripgrep's stdout lines (empty list when nothing matches)."""
    try:
        res = subprocess.run(["rg", *args], capture_output=True, text=True, cwd=cwd)
    except FileNotFoundError:
        sys.exit("hx needs ripgrep (rg) on PATH")
    return [l for l in res.stdout.split("\n") if l]
