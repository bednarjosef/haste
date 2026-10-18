#!/usr/bin/env python3
"""Validation checks over cache/oracle/26.3 server-side oracle dumps (oracles brief, Task 1).

Checks: minecraft:stone is a full cube; oak_slab[type=bottom] is a half cube; torch has empty
collision; water has empty collision. Run from the repo root: tools/recorder-mod/validate-dumps.py
"""
import json
import sys
from pathlib import Path

root = Path(__file__).resolve().parent.parent.parent
shapes = json.loads((root / "cache/oracle/26.3/block_shapes.json").read_text())

states = shapes["states"]
by_id = {s["id"]: s for s in states}
assert [s["id"] for s in states] == list(range(len(states))), "state ids not contiguous"
print(f"block_shapes.json: {len(states)} states, version {shapes['version']}")


def find(block, **props):
    out = [
        s for s in states
        if s["block"] == block and all(s["properties"].get(k) == v for k, v in props.items())
    ]
    assert out, f"no state for {block} {props}"
    return out


def boxes_of(block, **props):
    s = find(block, **props)[0]
    return s["collision"], s["outline"]


FULL = [[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]
HALF_BOTTOM = [[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]

failures = []

# stone: full cube, opaque
c, o = boxes_of("minecraft:stone")
if c != FULL or o != FULL:
    failures.append(f"stone not a full cube: collision={c} outline={o}")
s = find("minecraft:stone")[0]
if not s["opaque"]:
    failures.append("stone not opaque")
if s["air"]:
    failures.append("stone marked air")

# oak_slab[type=bottom]: half cube
c, o = boxes_of("minecraft:oak_slab", type="bottom")
if c != HALF_BOTTOM or o != HALF_BOTTOM:
    failures.append(f"oak_slab[bottom] not half: collision={c} outline={o}")

# oak_slab[type=top]: top half
HALF_TOP = [[0.0, 0.5, 0.0, 1.0, 1.0, 1.0]]
c, o = boxes_of("minecraft:oak_slab", type="top")
if c != HALF_TOP or o != HALF_TOP:
    failures.append(f"oak_slab[top] not top half: collision={c} outline={o}")

# torch: empty collision
c, o = boxes_of("minecraft:torch")
if c != []:
    failures.append(f"torch collision not empty: {c}")
if o == []:
    print("note: torch outline empty with empty level (expected: model-defined outline)")

# water: empty collision, fluid present
c, o = boxes_of("minecraft:water", level="0")
if c != []:
    failures.append(f"water collision not empty: {c}")
s = find("minecraft:water", level="0")[0]
if s.get("fluid") != "minecraft:water":
    failures.append(f"water fluid tag wrong: {s.get('fluid')}")
if s.get("opaque"):
    failures.append("water marked opaque")

# sanity: light emission for glowstone
s = find("minecraft:glowstone")[0]
if s["light"] != 15:
    failures.append(f"glowstone light != 15: {s['light']}")

# render_shape: barrier/water invisible, stone a model
for block, props, want in [
    ("minecraft:barrier", {}, "invisible"),
    ("minecraft:water", {"level": "0"}, "invisible"),
    ("minecraft:stone", {}, "model"),
]:
    s = find(block, **props)[0]
    if s.get("render_shape") != want:
        failures.append(f"{block} render_shape {s.get('render_shape')} != {want}")

# suffocation (vanilla LocalPlayer.suffocatesAt -> BlockState.isSuffocating):
# stone yes, glass no (tag excludes it), torch no (not a full cube).
for block, want in [
    ("minecraft:stone", True),
    ("minecraft:glass", False),
    ("minecraft:torch", False),
]:
    s = find(block)[0]
    if s.get("suffocating") != want:
        failures.append(f"{block} suffocating {s.get('suffocating')} != {want}")

# entity dimensions: player standing/swimming exist
edims = json.loads((root / "cache/oracle/26.3/entity_dimensions.json").read_text())
print(f"entity_dimensions.json: {len(edims['types'])} types, player poses: {list(edims['player'].keys())}")
standing = edims["player"].get("standing")
if not standing or standing["height"] != 1.8 or standing["width"] != 0.6:
    failures.append(f"player standing dims wrong: {standing}")
if standing["eye_height"] != 1.62:
    failures.append(f"player standing eye height wrong: {standing['eye_height']}")

# block properties: stone friction 0.6, ice 0.98, slime 0.8
bp = json.loads((root / "cache/oracle/26.3/block_properties.json").read_text())
print(f"block_properties.json: {len(bp['blocks'])} blocks")
for name, fr in [("minecraft:stone", 0.6), ("minecraft:ice", 0.98), ("minecraft:slime_block", 0.8), ("minecraft:blue_ice", 0.989)]:
    got = bp["blocks"].get(name, {}).get("friction")
    if got != fr:
        failures.append(f"{name} friction {got} != {fr}")

# block entity types: chest -> chest, oak_sign -> sign, spawner -> mob_spawner, stone none
bet = json.loads((root / "cache/oracle/26.3/block_entity_types.json").read_text())
print(f"block_entity_types.json: {len(bet['blocks'])} blocks, {len(bet['types'])} types")
if bet.get("states"):
    print(f"note: {len(bet['states'])} per-state entries (blocks whose states disagree)")
for block, want in [
    ("minecraft:chest", "minecraft:chest"),
    ("minecraft:trapped_chest", "minecraft:trapped_chest"),
    ("minecraft:exposed_copper_chest", "minecraft:chest"),
    ("minecraft:oak_sign", "minecraft:sign"),
    ("minecraft:oak_hanging_sign", "minecraft:hanging_sign"),
    ("minecraft:white_banner", "minecraft:banner"),
    ("minecraft:player_head", "minecraft:skull"),
    ("minecraft:shulker_box", "minecraft:shulker_box"),
    ("minecraft:poplar_shelf", "minecraft:shelf"),
    ("minecraft:spawner", "minecraft:mob_spawner"),
    ("minecraft:moving_piston", "minecraft:piston"),
]:
    got = bet["blocks"].get(block)
    if got != want:
        failures.append(f"{block} block entity type {got} != {want}")
if "minecraft:stone" in bet["blocks"]:
    failures.append("stone has a block entity type")

if failures:
    print("FAILURES:")
    for f in failures:
        print(" -", f)
    sys.exit(1)
print("all validation checks passed")
