#!/usr/bin/env python3
"""Validation checks over cache/oracle/26.3/block_sounds.json (block-sound-dump brief).

Checks: the dump covers a plausible number of blocks; stone, grass_block, sand, oak_planks,
glass, wool and a per-state block (the decorated pot) are present with plausible values — the
right sound event ids (block.<material>.break/step/place), volume and pitch — and every entry
in the dedupe table has the five sound ids and numeric volume/pitch. Run from the repo root:
tools/recorder-mod/validate-block-sounds.py
"""
import json
import sys
from pathlib import Path

root = Path(__file__).resolve().parent.parent.parent
dump = json.loads((root / "cache/oracle/26.3/block_sounds.json").read_text())

blocks = dump["blocks"]
types = dump["sound_types"]
print(f"block_sounds.json: {len(blocks)} blocks, {len(types)} sound types, "
      f"version {dump['version']}")


def entry(block_id):
    block = blocks.get(block_id)
    if block is None:
        return None, None
    return block, types[block["sound"]]


failures = []
if len(blocks) < 1000:
    failures.append(f"only {len(blocks)} blocks (expected 1000+ for 26.3)")

# block id -> (expected break id, expected step id); volume/pitch 1.0/1.0 for all of these.
# Sound event ids are written with the minecraft: prefix (Identifier.toString()).
expected = {
    "minecraft:stone": ("minecraft:block.stone.break", "minecraft:block.stone.step"),
    "minecraft:grass_block": ("minecraft:block.grass.break", "minecraft:block.grass.step"),
    "minecraft:sand": ("minecraft:block.sand.break", "minecraft:block.sand.step"),
    "minecraft:oak_planks": ("minecraft:block.wood.break", "minecraft:block.wood.step"),
    "minecraft:glass": ("minecraft:block.glass.break", "minecraft:block.glass.step"),
    "minecraft:white_wool": ("minecraft:block.wool.break", "minecraft:block.wool.step"),
}
for block_id, (break_id, step_id) in expected.items():
    block, sound = entry(block_id)
    if block is None:
        failures.append(f"missing block {block_id}")
        continue
    if sound.get("break") != break_id:
        failures.append(f"{block_id}: break is {sound.get('break')!r}, expected {break_id!r}")
    if sound.get("step") != step_id:
        failures.append(f"{block_id}: step is {sound.get('step')!r}, expected {step_id!r}")
    if sound.get("volume") != 1.0 or sound.get("pitch") != 1.0:
        failures.append(f"{block_id}: volume/pitch {sound.get('volume')}/{sound.get('pitch')}, "
                         "expected 1.0/1.0")

# A non-default per-state sound: the decorated pot's uncracked states override the block's
# default (cracked) sound type — the only per-state SoundType override in 26.3's BlockBehaviour
# hierarchy. Check both the default and the override are present and differ.
pot = blocks.get("minecraft:decorated_pot")
if pot is None:
    failures.append("missing block minecraft:decorated_pot")
else:
    states = pot.get("states", [])
    uncracked = [s for s in states if s.get("properties", {}).get("cracked") == "false"]
    if not uncracked:
        failures.append("decorated_pot has no cracked=false state in its per-state list")
    else:
        override = types[uncracked[0]["sound"]]
        default = types[pot["sound"]]
        if uncracked[0]["sound"] == pot["sound"]:
            failures.append("decorated_pot uncracked state has the same sound index as the default")
        if not default.get("break", "").endswith("decorated_pot.shatter"):
            failures.append(f"decorated_pot default break is {default.get('break')!r}")
        if not override.get("break", "").endswith("decorated_pot.break"):
            failures.append(f"decorated_pot uncracked break is {override.get('break')!r}")
    if not any(s.get("properties", {}).get("waterlogged") == "true" for s in states):
        failures.append("decorated_pot has no waterlogged=true state in its per-state list")

# A waterlogged/non-default state exists somewhere: any block listing per-state overrides at all
# exercises the states path; sanity-check its shape.
states_listed = 0
for block_id, block in blocks.items():
    for field in ("sound",):
        if not isinstance(block.get(field), int) or not (0 <= block[field] < len(types)):
            failures.append(f"{block_id}: sound index {block.get(field)!r} out of range")
    for state in block.get("states", []):
        states_listed += 1
        props = state.get("properties")
        if not isinstance(props, dict) or not props:
            failures.append(f"{block_id}: per-state entry has no properties: {state!r}")
        idx = state.get("sound")
        if not isinstance(idx, int) or not (0 <= idx < len(types)):
            failures.append(f"{block_id}: per-state sound index {idx!r} out of range")

# Every dedupe table entry is well-formed.
for i, sound in enumerate(types):
    for field in ("break", "step", "place", "hit", "fall"):
        if not isinstance(sound.get(field), str) or not sound[field]:
            failures.append(f"sound_types[{i}]: {field} is {sound.get(field)!r}")
    for field in ("volume", "pitch"):
        v = sound.get(field)
        if not isinstance(v, (int, float)) or not (0.0 < v <= 4.0):
            failures.append(f"sound_types[{i}]: {field} is {v!r}, not a plausible number")

print(f"per-state entries listed: {states_listed}")

if failures:
    for f in failures:
        print(f"FAIL: {f}", file=sys.stderr)
    sys.exit(1)
print(f"block_sounds.json: OK ({len(blocks)} blocks, {len(types)} sound types, "
      f"{states_listed} per-state entries)")
