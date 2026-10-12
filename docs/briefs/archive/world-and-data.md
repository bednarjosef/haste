# world-and-data: haste-data + haste-world

You own `crates/haste-data` and `crates/haste-world`. Both are consumed by nearly everyone, so the
order matters: `haste-data` first (others block on it), then `haste-world`.

## Task 1 (first, small): haste-data per docs/INTERFACES.md
- `GameData::load()` / `load_from()`: parse `blocks.json` (block -> properties -> states with
  `id`, `default`, property values), `registries.json` (registry -> entries -> protocol_id).
  Build dense tables indexed by `BlockStateId`: block index, property list, plus lookup maps.
  Verify at load: state ids are dense 0..n with `minecraft:air` = 0; error otherwise.
- `BlockShapes`: load `cache/oracle/26.3/block_shapes.json` if present (format: see
  `crates/haste-testkit/FORMATS.md` once oracles writes it; until then assume
  `{"version":1,"states":[{"id":0,"collision":[[x1,y1,z1,x2,y2,z2],...],"outline":[...]}]}`),
  else the documented fallback heuristic from INTERFACES.md. `is_oracle_backed()` says which.
- Tests: unit tests on a tiny hand-written blocks.json/registries.json fixture in the crate
  (a few blocks); an `#[ignore]` test loading the real reports: counts > 20000 states,
  `minecraft:stone` default state resolves, `minecraft:oak_stairs` with facing=east resolves,
  `Registries::index("minecraft:entity_type", "minecraft:player")` is Some.
- Measure `GameData::load()` time on the real reports and put it in NOTES.md (target < 150 ms release).

## Task 2: haste-world per docs/INTERFACES.md
- `PalettedContainer<T>`: decode vanilla's wire format (bits-per-entry 0 = single value; 4..=8
  indirect palette for blocks (1..=3 for biomes); direct above). Packed longs: entries never span
  longs (1.16+ layout). Read values with no allocation. Provide `get(x,y,z)` and `set` (promote
  palette when needed). The 26.3 chunk payload: per section `block_count: i16`, blocks container,
  biomes container; sections ordered from the dimension's lowest. Note: since 1.21.5 the per-section
  palette length is implicit for the direct format and the data-array length prefix was removed in
  1.21.5 (`Chunk Format` page on minecraft.wiki documents the current layout; verify against the
  26.3 page, and write what you verified in NOTES.md). Follow the wiki, not decompiled code.
- `World`: chunk map keyed by `ChunkPos`, `load_chunk` from `haste_core::chunk::ChunkData`
  (decode sections, heightmaps packed longs, block entities kept raw, light), `unload_chunk`,
  `set_block` (marks the section and neighbouring sections dirty when on a border),
  `apply_light` (light_update packet semantics: masks and arrays, empty masks clear),
  `BlockView` and `DirtyTracker` impls, `snapshot_section` returning an `Arc<SectionSnapshot>`
  that carries the section plus a 1-block border from the six neighbours and sky/block light,
  for lock-free meshing on other threads.
- Tests: paletted container decode/encode round trips for every bits-per-entry; a synthetic
  chunk payload built by your own encoder decodes back; `set_block` across a section border dirties
  both; light nibble get/set; `BlockView` returns AIR outside loaded chunks and outside y range.
  Benchmark (`cargo bench` or a timed test) of `get` in a hot loop: target < 5 ns/call.
- Give protocol what it needs: a small `haste_world::wire` module with `decode_sections(bytes,
  section_count) -> Result<Vec<Section>>` that the protocol crate can call in its own tests.

## Task 3 (after join milestone lands): block updates from `section_blocks_update` (packed positions),
world time/weather state holders, chunk unload radius helper, and performance work the lead asks for.

Do not touch: haste-protocol, haste-render, haste-entity, haste-physics, haste-client.
