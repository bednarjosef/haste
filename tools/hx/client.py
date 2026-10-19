"""hx client: run the Haste client against the test server for an on-screen or headless check,
with RCON commands before the run and at given seconds into it, and print a short digest of
the log (script marks, warnings, errors, screenshots) instead of the whole log.

A worker's shell cannot run the client and RCON at once, nor set HASTE_WALK_SCRIPT in front of
a command (the allow-list refuses both), so this does it for them.
"""
import os
import re
import subprocess
import threading
import time

from common import ROOT, clip, out

DIGEST = re.compile(r"\b(mark|WARN|ERROR|panick|summary|screenshot|script|joined|disconnect|"
                    r"kicked|died|death)\b", re.I)
ANSI = re.compile(r"\x1b\[[0-9;]*m")


def rcon(port, command):
    res = subprocess.run(["bash", os.path.join(ROOT, "scripts", "vanilla-rcon.sh"), "--port",
                          str(port), command], capture_output=True, text=True, cwd=ROOT)
    return (res.stdout + res.stderr).strip()


def cmd_client(user, script=None, walk=None, seconds=60, headless=False, before=(), at=(),
               server="127.0.0.1:25691", extra=()):
    binary = os.path.join(ROOT, "target", "release", "haste")
    if not os.path.isfile(binary):
        out("no target/release/haste: build it first with `cargo build --release -p haste-client`")
        return False
    port = int(os.environ.get("HX_RCON_PORT", "25691"))
    for c in before:
        out(f"rcon before: {c} -> {clip(rcon(port, c), 160)}")
    env = dict(os.environ)
    if walk:
        env["HASTE_WALK_SCRIPT"] = os.path.abspath(walk)
    args = [binary, "--server", server, "--username", user, "--seconds", str(seconds), *extra]
    if headless:
        args.append("--headless")
    if script:
        args += ["--script", script]
    log_path = os.path.join(ROOT, "target", f"hx-client-{user}.log")
    log = open(log_path, "w")
    t0 = time.time()
    proc = subprocess.Popen(args, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)

    def fire(delay, command):
        time.sleep(delay)
        if proc.poll() is None:
            log.write(f"[hx] rcon at {delay:.0f}s: {command} -> {rcon(port, command)}\n")
            log.flush()

    timers = []
    for spec in at:
        delay, _, command = spec.partition(":")
        th = threading.Thread(target=fire, args=(float(delay), command), daemon=True)
        th.start()
        timers.append(th)
    try:
        code = proc.wait(timeout=seconds + 90)
    except subprocess.TimeoutExpired:
        proc.kill()
        code = "killed after timeout"
    log.close()
    lines = [ANSI.sub("", l.rstrip()) for l in open(log_path, errors="replace")]
    out(f"client exited ({code}) after {time.time() - t0:.0f}s; full log: "
        f"{os.path.relpath(log_path, ROOT)} ({len(lines)} lines, grep it for more)")
    picked = [l for l in lines if DIGEST.search(l) or l.startswith("[hx]")]
    for l in picked[:60]:
        out("  " + clip(l, 200))
    if len(picked) > 60:
        out(f"  ... {len(picked) - 60} more matching lines in the log")
    out("last lines:")
    for l in lines[-8:]:
        out("  " + clip(l, 200))
    shots = os.path.join(ROOT, "target", "shots")
    if os.path.isdir(shots):
        new = sorted(f for f in os.listdir(shots) if os.path.getmtime(os.path.join(shots, f)) >= t0)
        if new:
            out("new screenshots (open one with Read): " + ", ".join(f"target/shots/{f}" for f in new))
    return code == 0
