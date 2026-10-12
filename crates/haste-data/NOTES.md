# haste-data notes

## Decisions

- **State-id layout (verified against the 26.3 report, all 35723 states):** within a block, ids
  run `first .. first + product(len(values))` in mixed-radix order over the *name-sorted*
  properties, the last property varying fastest. `blocks.json` lists property definitions sorted
  by name for all but 12 blocks (chests, `moving_piston`, `piston_head` — their definitions are
  listed `type, facing, waterlogged`); those 12 match the radix order once sorted. Because of
  this layout, `state()` and `with_property()` are O(#properties) arithmetic with no map lookups
  over states. The parser still stores each state's properties explicitly (needed for
  `properties()` and as a safety check) and validates contiguity + density + `minecraft:air` = 0
  at load; a malformed file is an error, not a fallback.
- **Interning:** property names (93 distinct) and values (153 distinct) are interned into one
  pool; a state's properties are `&[(u32, u32)]` slices into flattened storage. Block names are
  kept as `Identifier`s per block; `ns:path` string keys in the lookup maps are `Box<str>`.
- **`properties()` returns `Vec<(String, String)>`** (allocated, name-sorted) because the
  interface in docs/INTERFACES.md says `&[(String, String)]`, which cannot be returned borrowed
  from interned storage without per-call allocation anyway. Hot paths use the borrowed
  `property_value(state, name) -> Option<&str>`. Interface deviation, flagged to the lead.
- **Registries:** protocol ids are validated dense 0..len per registry at load (26.3 report is
  clean). Unknown registry in `len()` returns 0; in `index`/`name` returns None — callers treat
  missing data as absent rather than erroring.
- **Oracle file:** assumes `{"version":1,"states":[{"id",...,"collision":[[x1,y1,z1,x2,y2,z2]],"outline":[...]}]}`.
  The real WS8 dump (produced 2026-10-12) matches: extra fields `block`, `properties`, `light`,
  `air`, `opaque` are ignored by serde. States absent from the file get empty shapes.
  Unknown `version` is an error.

## Performance (measured)

- `GameData::load()` on the real 26.3 reports + oracle dump, release build, otherwise-idle
  machine: **~150-300 ms** depending on machine load (the shared builder machine runs other
  workstreams' compiles; worst observed ~300 ms under load 10+, ~150 ms idle). Breakdown:
  blocks.json typed parse ~74 ms (7 MB), registries ~12 ms, oracle block_shapes.json ~180 ms
  (30 MB, 903k f64 floats).
- The per-state JSON map was initially a `BTreeMap` (35k allocations); replaced by a custom
  `ordered_pairs` deserializer into `Vec<(&str, &str)>` — cut blocks.json parse from ~190 ms to
  ~74 ms.
- The oracle parse double-buffers (serde Vec<[f64;6]> then converted to Vec<Aabb>). The dump has
  only **922 distinct shape pairs across 35723 states**; a dedup/tape-based decode would cut
  memory and time substantially — deferred until someone needs it (startup-only cost).
- Target in the brief was < 150 ms release for the reports alone; reports alone (blocks +
  registries, no oracle) measure ~85-110 ms, within target. The oracle dump adds ~180 ms;
  asked the lead whether to optimize now or at M2 (report sent).

## Approximate / not verified

- Fallback shapes (no oracle file): documented heuristic from docs/INTERFACES.md. Air
  variants/water/lava/bubble_column empty; `snow` = layers/8 height; `_slab` half cube by `type`
  (top/bottom/double — 26.3 uses `type`, not `half`); `_stairs` approximated as a half cube by
  `half` (real stairs have two boxes, e.g. oak_stairs east/bottom/straight = base slab + quarter
  column); `pointed_dripstone` half cube by `vertical_direction` (real: taper by `thickness`);
  keyword list (grass, fern, flower, ...) = no collision. All replaced by the real oracle dump,
  which is present in the cache now; the fallback only fires without it.
- Registries with no `protocol_id` (none in 26.3) would fail to deserialize (field is
  non-optional); not handled because not present.
