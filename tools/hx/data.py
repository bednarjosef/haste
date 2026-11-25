"""hx data: what the game's own data says about a name, from the dumps Haste reads at runtime
(the recorder mod's oracle dumps in cache/oracle/26.3 and the data generator's reports in
cache/datagen/26.3/reports). Workers otherwise guess that a model layer, a registry entry or
a size "is not in the data" and skip the work built on it.
"""
import json
import os
from functools import lru_cache

from common import CACHE, ROOT, clip, out, rg

ORACLE = os.path.join(CACHE, "oracle", "26.3")
REPORTS = os.path.join(CACHE, "datagen", "26.3", "reports")


@lru_cache(maxsize=None)
def load(path):
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def loaders(file_name):
    """The Haste source files that read a dump (where to start from in Haste)."""
    files = rg(["-l", "--type", "rust", file_name, "crates"])
    return sorted(files, key=lambda f: ("test" in f, f))[:2]


def part_names(parts, depth=0, acc=None):
    acc = [] if acc is None else acc
    for p in parts or []:
        acc.append(p.get("name", "?"))
        if depth < 2:
            part_names(p.get("children"), depth + 1, acc)
    return acc


def model_layers(name):
    data = load(os.path.join(ORACLE, "entity_models.json"))
    if not data:
        return []
    rows = []
    for layer in data.get("layers", []):
        lid = layer.get("layer", "")
        if name in lid:
            parts = part_names(layer.get("parts"))
            rows.append(f"- {lid}  {layer.get('texture_width')}x{layer.get('texture_height')}  "
                        f"parts: {clip(', '.join(parts), 150)}")
    return rows


def dimensions(name):
    data = load(os.path.join(ORACLE, "entity_dimensions.json"))
    rows = []
    for t in (data or {}).get("types", []):
        if name in t.get("type", ""):
            rest = {k: v for k, v in t.items() if k not in ("type", "id")}
            rows.append(f"- {t['type']} (id {t.get('id')}): {clip(json.dumps(rest), 150)}")
    return rows


def registries(name):
    data = load(os.path.join(REPORTS, "registries.json"))
    rows = []
    for reg, body in (data or {}).items():
        hits = [(k, v.get("protocol_id")) for k, v in body.get("entries", {}).items() if name in k]
        if hits:
            shown = ", ".join(f"{k} ({i})" for k, i in hits[:6])
            more = f" +{len(hits) - 6} more" if len(hits) > 6 else ""
            rows.append(f"- {reg}: {shown}{more}")
    return rows


def blocks(name):
    data = load(os.path.join(REPORTS, "blocks.json"))
    rows = []
    for block, body in (data or {}).items():
        if name in block:
            props = body.get("properties", {})
            desc = ", ".join(f"{k}={'|'.join(map(str, v))}" for k, v in props.items())
            rows.append(f"- {block}: {len(body.get('states', []))} states {clip(desc, 120)}")
    return rows


SECTIONS = (
    ("entity model layers", "entity_models.json", model_layers),
    ("entity dimensions", "entity_dimensions.json", dimensions),
    ("registries (protocol ids)", "registries.json", registries),
    ("block states", "blocks.json", blocks),
)


def is_game_id(name):
    """Whether `name` is an entity type, item or block id (the concepts game data describes)."""
    regs = load(os.path.join(REPORTS, "registries.json")) or {}
    key = f"minecraft:{name}"
    return any(key in regs.get(r, {}).get("entries", {})
               for r in ("minecraft:entity_type", "minecraft:item", "minecraft:block"))


def lookup(name, limit=12):
    """(section title, loader files, rows) for each dump with matches."""
    found = []
    for title, file_name, fn in SECTIONS:
        rows = fn(name)
        if rows:
            found.append((title, file_name, loaders(file_name), rows[:limit], len(rows)))
    return found


def print_lookup(name, found, limit=12):
    for title, file_name, read_by, rows, total in found:
        via = f"; read by {', '.join(read_by)}" if read_by else ""
        out(f"{title} ({file_name}{via}):")
        for r in rows[:limit]:
            out(r)
        if total > limit:
            out(f"  ... {total - limit} more (`hx data {name}` narrows with a longer name)")


def cmd_data(name):
    found = lookup(name)
    if not found:
        out(f"nothing named like {name!r} in {os.path.relpath(ORACLE, ROOT)} or "
            f"{os.path.relpath(REPORTS, ROOT)} (try the registry id's path, e.g. armor_stand)")
        return
    print_lookup(name, found)
