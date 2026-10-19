"""hx check (fmt, clippy, tests and file sizes for the crates a branch touched, errors only) and
hx done (what is still missing before a worker may finish)."""
import hashlib
import os
import re
import subprocess

from common import CACHE, ROOT, clip, out

DIAG = re.compile(r"^(?P<file>[^\s:]+\.rs):(?P<line>\d+):(?P<col>\d+): (?P<sev>error|warning)"
                  r"(?P<msg>.*)$")


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True, cwd=ROOT).stdout.strip()


def base():
    return git("merge-base", "HEAD", os.environ.get("HX_BASE", "main")) or "HEAD"


def changed_files():
    files = set(git("diff", "--name-only", base()).split("\n"))
    files |= set(git("ls-files", "--others", "--exclude-standard").split("\n"))
    return sorted(f for f in files if f)


def changed_crates():
    return sorted({m.group(1) for f in changed_files() if (m := re.match(r"crates/([^/]+)/", f))})


def fingerprint():
    """Identifies the working tree's content (the diff from the branch base plus untracked
    files), so committing an already checked tree keeps its check."""
    h = hashlib.sha1(git("diff", base()).encode())
    for f in git("ls-files", "--others", "--exclude-standard").split("\n"):
        p = os.path.join(ROOT, f)
        if f and os.path.isfile(p):
            h.update(f.encode())
            h.update(open(p, "rb").read())
    return h.hexdigest()


def state_file():
    gd = git("rev-parse", "--absolute-git-dir")
    return os.path.join(gd, "hx-check")


def env():
    e = dict(os.environ)
    e.setdefault("HASTE_ROOT", ROOT)
    e.setdefault("HASTE_CACHE", CACHE)
    jdk = os.path.join(e["HASTE_CACHE"], "jdk")
    if os.path.isdir(jdk):
        e.setdefault("JAVA_HOME", jdk)
        e["PATH"] = os.path.join(jdk, "bin") + os.pathsep + e["PATH"]
    return e


def run(cmd):
    return subprocess.run(cmd, capture_output=True, text=True, cwd=ROOT, env=env())


def diagnostics(text, limit=25):
    """Unique compiler diagnostics in short form, errors first."""
    seen, errs, warns = set(), [], []
    for line in text.split("\n"):
        m = DIAG.match(line.strip())
        if m and line not in seen:
            seen.add(line)
            (errs if m["sev"] == "error" else warns).append(line.strip())
        elif line.startswith("error") and "could not compile" not in line and line not in seen:
            seen.add(line)
            errs.append(line.strip())
    return (errs + warns)[:limit], len(errs) + len(warns)


def test_failures(text, max_fail=5, per_fail=14):
    blocks, cur = [], None
    for line in text.split("\n"):
        if line.startswith("---- ") and line.endswith(" stdout ----"):
            cur = [line[5:-12]]
            blocks.append(cur)
        elif cur is not None:
            if line.startswith(("failures:", "---- ", "test result")):
                cur = None
            elif line.strip() and len(cur) <= per_fail:
                cur.append("    " + clip(line, 160))
    return blocks[:max_fail], len(blocks)


def cmd_check(crates=None, skip_tests=False, filt=None):
    crates = crates or changed_crates()
    ok = True
    fmt = run(["cargo", "fmt", "--all", "--", "-l"])
    fixed = [l for l in fmt.stdout.split("\n") if l.strip()]
    out("fmt: " + ("reformatted " + ", ".join(os.path.relpath(f, ROOT) for f in fixed)
                   if fixed else "clean"))
    sizes = run(["bash", "scripts/check-file-sizes.sh"])
    if sizes.returncode:
        ok = False
        out("file sizes: FAIL")
        for l in (sizes.stdout + sizes.stderr).strip().split("\n")[:10]:
            out("  " + l)
    else:
        out("file sizes: ok")
    if not crates:
        out("no crate changed since the branch base: nothing to build")
        record(ok)
        return ok
    pk = [a for c in crates for a in ("-p", c)]
    clippy = run(["cargo", "clippy", *pk, "--all-targets", "--message-format", "short", "--",
                  "-D", "warnings"])
    diags, n = diagnostics(clippy.stdout + clippy.stderr)
    if clippy.returncode:
        ok = False
        out(f"clippy ({' '.join(crates)}): FAIL, {n} diagnostics")
        for d in diags:
            out("  " + clip(d, 220))
    else:
        out(f"clippy ({' '.join(crates)}): clean")
    if skip_tests or clippy.returncode:
        if clippy.returncode:
            out("tests: not run (fix clippy first)")
        record(False if skip_tests else ok)
        return ok
    tcmd = ["cargo", "test", "-q", *pk, "--message-format", "short"]
    if filt:
        tcmd += ["--", filt]
    test = run(tcmd)
    text = test.stdout + test.stderr
    passed = sum(int(x) for x in re.findall(r"(\d+) passed", text))
    failed = sum(int(x) for x in re.findall(r"(\d+) failed", text))
    ignored = sum(int(x) for x in re.findall(r"(\d+) ignored", text))
    if test.returncode:
        ok = False
        diags, n = diagnostics(text)
        fails, nf = test_failures(text)
        out(f"tests: FAIL ({passed} passed, {failed} failed, {ignored} ignored)")
        for d in diags if not fails else []:
            out("  " + clip(d, 220))
        for b in fails:
            out(f"  {b[0]}:")
            for l in b[1:]:
                out(l)
        if nf > len(fails):
            out(f"  ... {nf - len(fails)} more failing tests")
    else:
        out(f"tests: ok ({passed} passed, {ignored} ignored)")
    record(ok and not filt)
    return ok


def record(ok):
    with open(state_file(), "w") as f:
        f.write(f"{'pass' if ok else 'fail'} {fingerprint()}\n")


def missing():
    """What a worker still owes before it may finish (empty when done)."""
    todo = []
    b = base()
    if os.environ.get("HX_DONE_COMMITS", "1") != "0" and git("rev-list", "--count", f"{b}..HEAD") == "0":
        todo.append("no commit on this branch yet: commit your work (git add + git commit)")
    dirty = [l for l in git("status", "--porcelain").split("\n") if l and "target/" not in l]
    if dirty:
        todo.append("uncommitted changes: " + ", ".join(l[3:] for l in dirty[:8])
                    + (" ..." if len(dirty) > 8 else "") + " (commit them, or drop stray files)")
    if changed_crates():
        try:
            status, fp = open(state_file()).read().split()
        except (OSError, ValueError):
            status, fp = "none", ""
        if status != "pass" or fp != fingerprint():
            why = {"none": "never ran", "fail": "last run failed"}.get(status, "")
            if status == "pass":
                why = "the code changed since"
            todo.append(f"hx check has not passed on the current tree ({why}): run `hx check`")
    return checklist_missing() + todo


def checklist_path():
    return os.path.join(ROOT, "target", "hx-checklist.md")


def checklist_missing():
    """The deliverables still open in target/hx-checklist.md (written at the start of a task:
    one `- [ ]` line per deliverable of the brief, ticked `[x]` or marked `[blocked: why]`)."""
    if os.environ.get("HX_CHECKLIST", "1") == "0":
        return []
    path = checklist_path()
    if not os.path.isfile(path):
        return ["no checklist: write target/hx-checklist.md with one `- [ ] item` line per "
                "deliverable of the brief (what to port, tests, checks, measurements, report "
                "items), then tick each `- [x]` when done or mark it `- [blocked: why]`"]
    open_items = [l.strip() for l in open(path) if re.match(r"\s*[-*] \[ \]", l)]
    if not open_items:
        return []
    return [f"{len(open_items)} checklist items still open in target/hx-checklist.md: "
            + "; ".join(clip(l[6:], 70) for l in open_items[:6])
            + ". Do them, or mark one `- [blocked: why]` only when it truly cannot be done here "
            "(look for existing Haste code first: hx map, hx ported, hx refs)"]


def cmd_done():
    todo = missing()
    if not todo:
        out("done: committed, clean tree, hx check passed on this tree")
        return True
    for t in todo:
        out("missing: " + t)
    return False
