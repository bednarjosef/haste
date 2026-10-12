# haste-world notes

## What was verified vs. assumed (wire format)

- **Verified against the real game:** nothing directly — no packet captures exist yet
  (`cache/oracle/26.3/packets/` is empty; WS8's recorder will fill it). The implementation
  follows the minecraft.wiki Chunk Format page as the brief instructs. WS1 should re-run the
  round-trip tests against a real captured `level_chunk_with_light` when captures exist.
- **Section payload layout (assumed from the wiki, 1.21.5+):** per section: block_count (varint,
  i16 range), blocks container, biomes container; sections ordered from the dimension's lowest.
  Bits-per-entry 0 = single value with NO data array (length prefix removed in 1.21.5);
  indirect = palette length + palette varints + packed longs with NO data-array length prefix;
  the long count is implicit: ceil(4096 * bits / 64) with entries never spanning longs.
- **Direct format (the one real uncertainty):** for bits-per-entry > 8 on the wire I read a
  direct container at `direct_bits` = ceil(log2(global state count)) = 15 for 26.3 (35723
  states), passed in by `World` (it knows the state count from GameData). If a capture shows
  vanilla sends the direct width itself in the bits-per-entry field, `decode_with_bpe`'s direct
  arm needs a one-line change. Not verifiable offline; flagged for WS1/WS8.
- **Biome containers:** 64 entries (4x4x4 cells per section), indirect 1..=3 bits; I pass 3 as
  the "direct" width so a 4-bit biome container would decode as direct rather than error.
- **Light masks** index sections from min_section - 1 to max_section + 1 (one extra each end);
  I confirmed the empty-mask-clears semantics against the interface docs. Unset in both masks =
  keep existing (partial `light_update`).
- Pumpkin's palette code (Bedrock) packs entries into longs differently (left-aligned per
  64/bits entries, no span check) — consistent with my `PackedArray`; it is Bedrock so only the
  *packing shape* was cross-checked, not the Java wire details. Pomme was not consulted (its
  physics/player code is off-limits; the world code was not needed).

## Decisions

- `PalettedContainer<T>` is generic over a local `WireId` trait (impl'd for `BlockStateId` and
  `BiomeId`) because haste-core's ids deliberately have no `From<u32>`. No haste-core change
  requested.
- `set_index` promotes Single -> Indirect (4..=8 bits) -> Direct, rewriting the packed data
  (O(4096) per promotion, rare). Bug found by test: the indirect branch wrote the palette
  VALUE where the palette INDEX was required — a real corruption bug the round-trip tests
  caught; fixed and covered.
- `World` keeps an `UnsafeCell<Option<(key, *const Chunk)>>` memo of the last chunk hit by a
  BlockView call: `BlockView` takes `&self` but hot loops hit the same chunk repeatedly.
  Invalidated on load/unload. Send/Sync unsafe impls documented. This took the hot-path
  `block_state` from ~80 ns to ~18 ns/call.
- `SectionSnapshot` carries the section (cloned containers) plus six 16x16 border faces and its
  light, exactly per the interface; `get(x,y,z)` accepts -1..=16. Border index order is
  documented in `border_face_index`.
- `BlockView` returns AIR for unloaded chunks/out-of-range y, sky light 15 when absent
  (nether/no data), block light 0 — as the interface states.
- Heightmaps: stored per wire type id at 9 bits/entry (26.3 max value 385 fits); `height_at`
  falls back to a linear scan when the chunk carried none. **Approximate:** which type id maps
  to world_surface vs motion_blocking is not determined from the reports (the ids are wire
  varints, not in blocks.json/registries.json); the consumer picks by id.

## Performance

- Hot-loop `World::block_state`: ~18 ns/call release (measured on the shared 16-core machine
  under load 3-18 from parallel workstream builds; ~8 ns idle). Brief target was < 5 ns/call —
  NOT met on this machine; the bench asserts < 30 ns as a regression bound (losing the chunk
  memo gives ~80 ns). The remaining cost is the packed-array read (runtime div/mod in the
  generic path; the 4..=8 bit fast paths are specialized) plus bounds checks. If WS5 needs the
  full 5 ns, the next step is a per-section direct block cache (flat [BlockStateId; 4096]
  materialized lazily per section) — deferred; memory cost not justified yet.
- `PalettedContainer::get_index` micro-bench: ~7.7 ns under load (indirect, 4 bits).
- Palette decode: no allocation per entry; palette vec + packed longs only.
