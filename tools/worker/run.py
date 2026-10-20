"""Launch a headless Claude Code worker on a Haste brief, set up for small contexts:

- only the six tools a worker needs (Bash, Read, Edit, Write, Grep, Glob) and no settings or
  CLAUDE.md from disk: tools/worker/RULES.md (the worker's condensed AGENTS.md and the `hx`
  cheat sheet) is appended to the system prompt instead, which halves the fixed prompt;
- the `hx` tools on PATH, and the orientation pack (`hx context BRIEF`) after the brief;
- hooks (tools/worker/settings.json): wasteful commands are refused with the cheaper one named,
  and the worker cannot finish until `hx done` passes;
- capped file reads and shell output, auto-compaction before Haiku's 100K price tier.

  python3 tools/worker/run.py BRIEF --branch NAME [--model haiku|sonnet|opus] [--base REF]
                              [--user NAME] [--budget USD] [--effort medium] [--dry-run]
  python3 tools/worker/run.py --resume NAME     continue a run that stopped early

Events go to $HASTE_WT/runs/NAME.jsonl (resumed segments to NAME.r<N>.jsonl); summarise them
with tools/worker/costs.py.
"""
import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
# The main checkout (this file may run from a worktree): worktrees go next to it in haste-wt/.
MAIN = Path(subprocess.run(["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
                           cwd=HERE, capture_output=True, text=True).stdout.strip()).parent
WT = Path(os.environ.get("HASTE_WT", MAIN.parent / "haste-wt"))
RUNS = WT / "runs"

MODELS = {"haiku": "claude-haiku-5-5", "sonnet": "claude-sonnet-5-5", "opus": "claude-opus-5-5"}
BUDGET = {"haiku": 5, "sonnet": 15, "opus": 30}
TOOLS = ["Bash", "Read", "Edit", "Write", "Grep", "Glob"]
ALLOW = TOOLS[1:] + [f"Bash({p})" for p in (
    "hx *", "cargo *", "rustc *", "bash -n *", "./target/*", "target/*", "scripts/*", "./scripts/*",
    "bash scripts/*", "source scripts/env.sh*", "git status*", "git diff*", "git log*", "git show*",
    "git add *", "git commit *", "git restore *", "git stash*", "git blame *", "git grep *",
    "git rev-parse *", "git branch --show-current", "ls*", "cat *", "head *", "tail *", "grep *",
    "rg *", "find *", "wc *", "sort *", "uniq *", "cut *", "tr *", "diff *", "file *", "stat *",
    "du *", "sed -n *", "awk *", "jq *", "echo*", "printf *", "true", "false", "test *", "[ *",
    "pwd", "which *", "sleep *", "date*", "mkdir -p *", "touch *", "timeout * cargo *",
    "timeout * target/*", "timeout * ./target/*", "timeout * scripts/*", "timeout -s INT * target/*",
    "nproc", "free *", "pgrep *", "ps *", "ss *", "sed -i *", "rm crates/*", "rm target/*",
    "rm -f crates/*", "rm -f target/*", "git rm *", "git mv *")]
NUDGE = ("You are running headless: no notification will ever arrive, and ending your turn ends "
         "the session for good. The shell is not banned: plain commands run from the worktree "
         "root (`git status`, `git add -A`, `git commit -m MSG`, `hx check`, `cargo test -p "
         "CRATE`) are allowed; only loops, $(...), $VARIABLES, heredocs and `git -C` are refused. "
         "Continue the task now from where you are, in the foreground, and "
         "do not end your turn until `hx done` passes or something is truly blocked. Then give "
         "your full report as your final message.")


def header(branch, wt, user):
    return f"""# Setup for this run
You work alone and unattended in the git worktree {wt}, on branch `{branch}` (already checked
out; ignore any branch name in the brief). Commit your work on this branch (git add / git
commit, never push) and finish with your report as your final message.
The test server 127.0.0.1:25691 is already running. Use the username `{user}` for every client
and RCON command.

"""


def worker_env(wt, base, user=None):
    env = dict(os.environ)
    env.update({
        # haste-wt/bin holds the machine-local `cargo` that lets one Haste build run at a time.
        "PATH": os.pathsep.join([str(WT / "bin"), str(REPO / "scripts"), env["PATH"]]),
        "HASTE_ROOT": str(wt),
        "HASTE_CACHE": str(MAIN / "cache"),
        "HX_BASE": base,
        # scripts/vanilla-client.sh logs in as this player (an env prefix would be refused).
        "HASTE_VANILLA_USER": user or "",
        "CARGO_BUILD_JOBS": env.get("CARGO_BUILD_JOBS", "8"),
        "CLAUDE_CODE_FILE_READ_MAX_OUTPUT_TOKENS": "6000",
        "BASH_MAX_OUTPUT_LENGTH": "12000",
        "CLAUDE_CODE_AUTO_COMPACT_WINDOW": "110000",
    })
    return env


def claude_cmd(prompt, model, effort, budget, extra=()):
    return ["nice", "-n", "10", "claude", "-p", prompt, *extra,
            "--model", model, "--effort", effort,
            "--output-format", "stream-json", "--verbose",
            "--permission-mode", "dontAsk", "--tools", *TOOLS, "--allowedTools", *ALLOW,
            "--setting-sources", "", "--settings", str(HERE / "settings.json"),
            # The cache (vanilla source, jars) is a symlink out of the worktree: without this,
            # every shell read under cache/ is refused in dontAsk mode.
            "--add-dir", str(MAIN / "cache"),
            "--strict-mcp-config", "--max-budget-usd", str(budget),
            "--append-system-prompt-file", str(HERE / "RULES.md")]


def git(*args, cwd=REPO):
    return subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, check=True
                          ).stdout.strip()


def make_worktree(branch, base):
    wt = WT / branch
    if not wt.exists():
        git("worktree", "add", "-q", "-b", branch, str(wt), base)
    cache = wt / "cache"
    if not cache.exists() and (MAIN / "cache").exists():
        cache.symlink_to(MAIN / "cache")
    return wt


def start(cmd, wt, env, log):
    RUNS.mkdir(parents=True, exist_ok=True)
    p = subprocess.Popen(cmd, cwd=wt, env=env, stdin=subprocess.DEVNULL, start_new_session=True,
                         stdout=open(log, "w"), stderr=open(log.with_suffix(".err"), "w"))
    return p.pid


def save_meta(branch, meta):
    path = RUNS / f"{branch}.meta.json"
    old = json.loads(path.read_text()) if path.exists() else {}
    old.update(meta)
    path.write_text(json.dumps(old, indent=1))
    return old


def launch(a):
    model_key = a.model if a.model in MODELS else "custom"
    model = MODELS.get(a.model, a.model)
    base = git("rev-parse", a.base)
    wt = make_worktree(a.branch, base)
    user = a.user or a.branch[:16]
    env = worker_env(wt, base, user)
    brief = Path(a.brief).read_text()
    pack = subprocess.run([str(REPO / "scripts" / "hx"), "context", a.brief], env=env,
                          capture_output=True, text=True).stdout
    prompt = header(a.branch, wt, user) + brief + "\n\n" + pack
    # Checklist items the brief spells out (`- [ ]` lines) start the worker's checklist.
    items = [l.strip() for l in brief.splitlines() if l.strip().startswith("- [ ]")]
    if items:
        (wt / "target").mkdir(exist_ok=True)
        (wt / "target" / "hx-checklist.md").write_text("\n".join(items) + "\n")
    budget = a.budget or BUDGET.get(model_key, 10)
    cmd = claude_cmd(prompt, model, a.effort, budget)
    if a.dry_run:
        print(prompt)
        print(f"\n[{len(prompt)} chars] would run in {wt}: claude -p ... --model {model}")
        return
    pid = start(cmd, wt, env, RUNS / f"{a.branch}.jsonl")
    save_meta(a.branch, {"pid": pid, "start": time.time(), "model": model, "base": base,
                         "effort": a.effort, "budget": budget, "brief": str(a.brief),
                         "user": user})
    print(f"started {a.branch} ({model}) pid {pid}; events in {RUNS / (a.branch + '.jsonl')}")


def resume(branch):
    meta = json.loads((RUNS / f"{branch}.meta.json").read_text())
    first = RUNS / f"{branch}.jsonl"
    session = next(json.loads(l)["session_id"] for l in open(first)
                   if '"subtype":"init"' in l.replace(" ", ""))
    n = 1
    while (RUNS / f"{branch}.r{n}.jsonl").exists():
        n += 1
    wt = WT / branch
    cmd = claude_cmd(NUDGE, meta["model"], meta["effort"], meta["budget"],
                     extra=("--resume", session))
    pid = start(cmd, wt, worker_env(wt, meta["base"], meta.get("user")),
                RUNS / f"{branch}.r{n}.jsonl")
    save_meta(branch, {"resumes": meta.get("resumes", []) + [{"pid": pid, "start": time.time()}]})
    print(f"resumed {branch} as segment r{n}, pid {pid}")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("brief", nargs="?")
    ap.add_argument("--branch")
    ap.add_argument("--model", default="haiku")
    ap.add_argument("--base", default="main")
    ap.add_argument("--user")
    ap.add_argument("--budget", type=float)
    ap.add_argument("--effort", default="medium")
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--resume", metavar="BRANCH")
    a = ap.parse_args()
    if a.resume:
        resume(a.resume)
    elif a.brief and a.branch:
        launch(a)
    else:
        ap.print_help()
        sys.exit(2)


if __name__ == "__main__":
    main()
