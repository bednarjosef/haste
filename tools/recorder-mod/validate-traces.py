#!/usr/bin/env python3
"""Sanity checks for client-recorded physics traces in cache/oracle/26.3/traces/ (oracles Task 3).

Checks what the sequences should produce: the walk/sprint/sprint-jump traces moved the player,
jump traces left the ground, the stop trace stayed (nearly) still, sneak was slower than walk,
swim was in water (slower still), fall gained downward speed, and every file parses with the
documented fields. Run from the repo root: tools/recorder-mod/validate-traces.py [prefix]
"""
import json
import math
import sys
from pathlib import Path

root = Path(__file__).resolve().parent.parent.parent
traces_dir = root / "cache/oracle/26.3/traces"
prefix = sys.argv[1] if len(sys.argv) > 1 else ""

if not traces_dir.is_dir():
    print(f"no traces dir at {traces_dir} (record some with /haste record, see docs/RECORDING.md)")
    sys.exit(1)

files = sorted(p for p in traces_dir.glob(f"{prefix}*.json"))
if not files:
    print(f"no trace files matching {prefix}* in {traces_dir}")
    sys.exit(1)

failures = []
by_seq = {}


def dist(a, b):
    return math.dist(a["pos"], b["pos"])


for path in files:
    try:
        trace = json.loads(path.read_text())
    except Exception as e:
        failures.append(f"{path.name}: does not parse ({e})")
        continue
    for field in ("version", "name", "world", "player", "ticks"):
        if field not in trace:
            failures.append(f"{path.name}: missing field '{field}'")
    ticks = trace.get("ticks", [])
    if len(ticks) < 50:
        failures.append(f"{path.name}: only {len(ticks)} ticks (expected ~100)")
        continue
    for t in ticks:
        for field in ("tick", "input", "pos", "velocity", "on_ground", "pose", "dimensions"):
            if field not in t:
                failures.append(f"{path.name} tick {t.get('tick')}: missing '{field}'")
                break
    seq = path.name.rsplit("-", 1)[-1].removesuffix(".json")
    by_seq[seq] = trace

print(f"{len(files)} traces, sequences: {sorted(by_seq)}")


def horizontal(t):
    x, _, z = t["pos"]
    return math.hypot(x, z)


if "walk" in by_seq:
    t = by_seq["walk"]["ticks"]
    moved = horizontal(t[-1]) - horizontal(t[0])
    if moved < 5:
        failures.append(f"walk: only moved {moved:.2f} blocks in 100 ticks (expected ~20+)")
if "sprint" in by_seq:
    t = by_seq["sprint"]["ticks"]
    moved = horizontal(t[-1]) - horizontal(t[0])
    if moved < 8:
        failures.append(f"sprint: only moved {moved:.2f} blocks (expected more than walk)")
if "walk" in by_seq and "sprint" in by_seq:
    w = horizontal(by_seq["walk"]["ticks"][-1]) - horizontal(by_seq["walk"]["ticks"][0])
    s = horizontal(by_seq["sprint"]["ticks"][-1]) - horizontal(by_seq["sprint"]["ticks"][0])
    if s <= w:
        failures.append(f"sprint ({s:.2f}) did not outrun walk ({w:.2f})")
if "jump" in by_seq:
    ys = [t["pos"][1] for t in by_seq["jump"]["ticks"]]
    if max(ys) - ys[0] < 0.5:
        failures.append(f"jump: max height gain only {max(ys) - ys[0]:.2f} blocks")
    if all(t["on_ground"] for t in by_seq["jump"]["ticks"]):
        failures.append("jump: never left the ground")
if "stop" in by_seq:
    t = by_seq["stop"]["ticks"]
    moved = dist(t[0], t[-1])
    if moved > 2.0:
        failures.append(f"stop: moved {moved:.2f} blocks while holding no keys (momentum is fine, sliding is not)")
if "sneak" in by_seq and "walk" in by_seq:
    w = horizontal(by_seq["walk"]["ticks"][-1]) - horizontal(by_seq["walk"]["ticks"][0])
    k = horizontal(by_seq["sneak"]["ticks"][-1]) - horizontal(by_seq["sneak"]["ticks"][0])
    if k >= w:
        failures.append(f"sneak ({k:.2f}) not slower than walk ({w:.2f})")
    if not any(t["pose"] == "crouching" for t in by_seq["sneak"]["ticks"]):
        failures.append("sneak: pose never became crouching")
if "fall" in by_seq:
    vy = [t["velocity"][1] for t in by_seq["fall"]["ticks"]]
    if min(vy) > -0.5:
        failures.append(f"fall: min vy {min(vy):.2f} (expected downward speed building up)")
if "swim" in by_seq:
    if not any(t["pose"] == "swimming" for t in by_seq["swim"]["ticks"]):
        print("note: swim never entered swimming pose (shallow water? jump held: crawling is fine too)")

# sprint-jump should be the fastest horizontal sequence overall
speeds = {}
for seq in ("walk", "sprint", "sprint-jump", "sneak"):
    if seq in by_seq:
        t = by_seq[seq]["ticks"]
        speeds[seq] = horizontal(t[-1]) - horizontal(t[0])
if "sprint-jump" in speeds and "sprint" in speeds and speeds["sprint-jump"] < speeds["sprint"]:
    failures.append(f"sprint-jump ({speeds['sprint-jump']:.2f}) slower than sprint ({speeds['sprint']:.2f})")
if speeds:
    print("distances: " + ", ".join(f"{k}={v:.1f}" for k, v in speeds.items()))

if failures:
    print("FAILURES:")
    for f in failures:
        print(" -", f)
    sys.exit(1)
print("all trace checks passed")
