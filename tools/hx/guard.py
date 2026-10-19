"""Claude Code hooks for headless workers (tools/worker/settings.json).

pre:  PreToolUse on Bash. Refuses what wastes a worker's context or ends a headless session:
      background commands, unbounded searches of the decompiled source, whole-file `cat` of big
      files, workspace-wide cargo runs. Each refusal names the cheaper command to use instead.
stop: Stop. While `hx done` lists something missing, the worker may not end its turn (at most
      HX_STOP_BLOCKS times per session, so a truly blocked worker can still finish).
"""
import json
import os
import re
import sys

import checks
from common import ROOT


def deny(reason):
    print(json.dumps({"hookSpecificOutput": {
        "hookEventName": "PreToolUse", "permissionDecision": "deny",
        "permissionDecisionReason": reason}}))
    sys.exit(0)


def big_file(path, limit=250):
    p = path if os.path.isabs(path) else os.path.join(os.getcwd(), path)
    try:
        with open(p, "rb") as f:
            return sum(1 for _ in f) > limit
    except OSError:
        return False


def pre(event):
    if event.get("tool_name") != "Bash":
        return
    inp = event.get("tool_input") or {}
    cmd = inp.get("command", "")
    if inp.get("run_in_background"):
        deny("This session is headless: a background command's completion notice never arrives, "
             "and ending your turn to wait ends the session. Run it in the foreground with a "
             "timeout of up to 600000 ms.")
    # Bounded: piped into head/tail/wc, or a flag that prints only names, counts or a few hits
    # (-l and -c also inside a bundle like -rln, -m N, the long forms).
    # The permission checker refuses what it cannot read statically; say so precisely, before
    # its generic refusal (which suggests workarounds and wastes turns).
    bare = re.sub(r"'[^']*'", "''", cmd)
    if re.search(r"(^|[;&|]\s*)(for|while|until)\s", bare) or re.search(r"\$\(|`|\$\{?[A-Za-z_]", bare) \
            or re.search(r"<<-?\s*['\"]?\w+", cmd):
        deny("The permission checker refuses shell loops, $(...) and backticks, $VARIABLE "
             "expansion and heredocs. Spell paths and values out, run one command per item "
             "(several commands joined with ; or && in one call are fine), and create or edit "
             "files with the Write and Edit tools.")
    bounded = re.search(r"\|\s*(head|tail|wc|sort -u|grep -c)\b|\s-[a-km-zA-Z]*[lc][a-zA-Z]*\b"
                        r"|\s-m\s*\d+|--(count|files-with-matches|max-count)", cmd)
    if re.search(r"\b(grep|rg)\b", cmd) and "cache/source" in cmd and not bounded:
        deny("Unbounded search of the decompiled source. Use `hx vgrep PATTERN [package/dir]` "
             "(capped, every hit labelled Class.method:line) or add -l / | head -40.")
    if re.search(r"\bfind\b[^|]*cache/source", cmd) and not bounded:
        deny("Use `hx vanilla ClassName` to locate a vanilla class, or `hx vgrep -l PATTERN`.")
    m = re.fullmatch(r"\s*cat\s+([^\s|;&<>]+)\s*", cmd)
    if m and big_file(m.group(1)):
        deny(f"{m.group(1)} is over 250 lines. Run `hx outline {m.group(1)}` for its items with "
             "line ranges, then Read only the range you need (offset/limit).")
    if re.search(r"\bcargo\s+(test|clippy|build|check)\b", cmd) and not re.search(
            r"(\s-p\s|--package|--manifest-path)", cmd):
        deny("A workspace-wide cargo run is slow and floods the context. Use `hx check` (fmt, "
             "clippy and tests of the crates you changed, errors only) or pass -p <crate>.")


def stop(event):
    limit = int(os.environ.get("HX_STOP_BLOCKS", "3"))
    marker = os.path.join(os.path.dirname(checks.state_file()),
                          f"hx-stop-{event.get('session_id', 'x')}")
    count = int(open(marker).read()) if os.path.exists(marker) else 0
    if count >= limit:
        return
    todo = checks.missing()
    if not todo:
        return
    with open(marker, "w") as f:
        f.write(str(count + 1))
    sys.stderr.write(
        "You are not done (`hx done`):\n" + "\n".join("- " + t for t in todo)
        + "\nFinish these in the foreground, then give your final report. If one is truly "
        "blocked, say why in the report and end your turn again"
        f" ({limit - count - 1} more stops will be held).\n")
    sys.exit(2)


def main(which):
    os.chdir(ROOT)
    event = json.load(sys.stdin)
    (pre if which == "pre" else stop)(event)
