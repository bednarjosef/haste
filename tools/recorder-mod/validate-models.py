#!/usr/bin/env python3
"""Validation checks over cache/oracle/26.3/entity_models.json (model-dump brief).

Checks: the dump has a plausible layer count; the pig, zombie, villager, player, chest, sign,
bed, banner and shulker_box layers exist; every part and cube has numeric fields where the
format requires them. Run from the repo root: tools/recorder-mod/validate-models.py
"""
import json
import sys
from pathlib import Path

root = Path(__file__).resolve().parent.parent.parent
dump = json.loads((root / "cache/oracle/26.3/entity_models.json").read_text())

layers = dump["layers"]
by_id = {layer["layer"]: layer for layer in layers}
print(f"entity_models.json: {len(layers)} layers, version {dump['version']}")

failures = []

if len(layers) < 300:
    failures.append(f"only {len(layers)} layers (expected 300+ for 26.3)")

# Entities the brief names, plus the block-entity layers whose models are model layers too.
# Signs and beds are NOT model layers in 26.3 (ModelLayers has no sign/bed entries; they render
# as baked block models), so they cannot appear here — see NOTES in the brief report.
expected = [
    "minecraft:pig#main", "minecraft:zombie#main", "minecraft:villager#main",
    "minecraft:player#main", "minecraft:chest#main", "minecraft:double_chest_left#main",
    "minecraft:standing_banner#main", "minecraft:shulker_box#main",
    "minecraft:bell#main", "minecraft:decorated_pot_base#main",
]
for layer_id in expected:
    if layer_id not in by_id:
        failures.append(f"missing layer {layer_id}")


def check_numbers(value, what, where):
    if not isinstance(value, list) or len(value) != 3:
        failures.append(f"{where}: {what} is not a 3-number list: {value!r}")
        return
    for v in value:
        if not isinstance(v, (int, float)):
            failures.append(f"{where}: {what} has a non-numeric entry: {v!r}")


def check_part(part, where):
    if not isinstance(part.get("name"), str) or not part["name"]:
        failures.append(f"{where}: part has no name")
    check_numbers(part.get("pivot"), "pivot", where)
    check_numbers(part.get("rotation"), "rotation", where)
    check_numbers(part.get("scale"), "scale", where)
    for cube in part.get("cubes", []):
        check_numbers(cube.get("origin"), "cube origin", where)
        check_numbers(cube.get("size"), "cube size", where)
        for field in ("u", "v"):
            if not isinstance(cube.get(field), (int, float)):
                failures.append(f"{where}: cube {field} is not numeric: {cube.get(field)!r}")
        if not isinstance(cube.get("mirror"), bool):
            failures.append(f"{where}: cube mirror is not a bool: {cube.get('mirror')!r}")
        check_numbers(cube.get("inflate"), "cube inflate", where)
    for child in part.get("children", []):
        check_part(child, where)


parts_checked = 0
for layer in layers:
    where = layer.get("layer", "<unnamed>")
    if not isinstance(layer.get("texture_width"), int) or not isinstance(layer.get("texture_height"), int):
        failures.append(f"{where}: texture size is not int pair")
    for part in layer.get("parts", []):
        check_part(part, where)
        parts_checked += 1

print(f"parts checked: {parts_checked}")

if failures:
    for f in failures:
        print(f"FAIL: {f}", file=sys.stderr)
    sys.exit(1)
print(f"entity_models.json: OK ({len(layers)} layers, {parts_checked} parts)")
