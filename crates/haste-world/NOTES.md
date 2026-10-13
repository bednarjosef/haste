# haste-world notes

## What was verified vs. assumed (wire format)

- **Verified against the real game:** nothing directly — no packet captures exist yet
  (`cache/oracle/26.3/packets/` is empty; oracles's recorder will fill it). The implementation
  follows the minecraft.wiki Chunk Format page as the brief instructs. protocol should re-run the
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
  arm needs a one-line change. Not verifiable offline; flagged for protocol/oracles.
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
  generic path; the 4..=8 bit fast paths are specialized) plus bounds checks. If physics needs the
  full 5 ns, the next step is a per-section direct block cache (flat [BlockStateId; 4096]
  materialized lazily per section) — deferred; memory cost not justified yet.
- `PalettedContainer::get_index` micro-bench: ~7.7 ns under load (indirect, 4 bits).
- Palette decode: no allocation per entry; palette vec + packed longs only.

## Chunk load benchmark (branch `chunk-load-speed`)

`tests/load_bench.rs`: realistic synthetic overworld chunks (24 sections: single-value stone
below y 0, 2-bit ore belt, 3-bit cave, 3-bit surface belt with water, single-value air above;
2-bit biome containers, 2 heightmaps, 8 block entities, full light — daylight-shaped: all-15
sky / all-0 block light above ground). Counting global allocator (opt-in per run). Run:
`cargo test -q -p haste-world --release -- --ignored --nocapture load_bench`.

Numbers on the shared machine (load 2-8, single thread; noisy — best pass of 5):

- baseline (main): `load_chunk` median 13-16 us, p99 ~21-31 us, ~57-71k chunks/s;
  `Chunk::decode` alone 8.7 us; 75 allocs/chunk steady state, ~22.8 KB allocated/chunk;
  resident ~109 KB/chunk.
- after bulk packed-long reads (`WireReader::take`, one bounds check per array instead of a
  checked `get` + `from_be_bytes` per long): decode 8.7 -> 3.7 us.
- after `dirty_dedup` switched std HashSet -> ahash (216 inserts per loaded chunk):
  `load_chunk` median down to ~7-9 us.
- after uniform-light dedup (all-15 sky stored as absent, all-0 block light stored empty;
  word-wide scan): resident 109 -> 64 KB/chunk (-41%), decode +~1 us (7.9-9.5 us with light
  vs 3-4.5 us without; the scan reads the same bytes the packet brought in anyway, and it
  unpins the packet's `Bytes` — light arrays stop keeping the whole packet buffer alive).
- final: `load_chunk` median 10.7-12.8 us, p99 15-19 us, ~75-92k chunks/s best pass;
  131 allocs/chunk total (75 steady state + one-time world warmup), ~22.8 KB allocated/chunk,
  64 KB resident/chunk. At render distance 32 (1089 chunks): ~70 MB containers.

Behaviour pinned by `benchmark_chunks_decode_stably`: FNV-1a hash over every block state and
biome of the bench chunks (`0xa7e8f48376c8ee92`, `0x5088a771ee4ad23a`), identical before and
after the optimizations.

## Chunk memory: single-value biomes, dirty bitset (branch `chunk-memory-next`)

Follow-up to the two "next steps" above, measured on the shared machine (load 2-9; the
machine's absolute numbers move a lot between passes, so the before/after below was taken
alternating within one session):

- **Single-value biome containers** (vanilla's `Strategy.ZERO_BITS`, verified in the 26.3
  client source: bpe 0 -> one value varint, no data array). `decode` already stored those as
  `Storage::Single`; what was missing was a test pinning it for the 64-entry biome capacity
  and the encoder path: `PalettedContainer::indirect` now collapses a one-entry palette to
  `Single` (a 1-entry palette IS the zero-bits configuration; it used to encode 4 bits of
  data for 64 entries — wrong for biomes anyway, whose indirect range is 1..=3 bits). Tests:
  `single_value_biome_container_has_no_data_array`, `indirect_with_one_entry_is_single_value`,
  `single_value_biome_promotes_on_set`.
- **Dirty tracking is a per-chunk bitset** (`Chunk::dirty`, one bit per section, one word for
  any world up to 64 sections). `World` keeps only `dirty_chunks` (packed keys with marks, in
  mark order — a key can repeat, the second drain reads cleared bits) plus a short
  `dirty_unloaded` list for sections in chunks that have not loaded yet (a `set_block` on a
  border block marks the neighbour chunk's section; those entries were in the old hash set,
  and `set_block_and_border_dirty` pins them, so they stay — deduplicated on drain, no hash).
  `load_chunk`/`unload_chunk` mark whole chunks by bitset instead of 216 hash inserts.
- Bench numbers, best pass of 5, before -> after: `load_chunk` median 10.8-11.7 -> 9.9-10.9 us,
  p99 17.9-26.4 -> 13.1-18.8 us. Same chunks with single-value biome containers on the wire
  (no palette, no packed longs): median 8.7-9.7 us, and `Chunk::decode` alone 7.9 -> 6.5
  us/chunk. Resident: 63992 -> 64000 bytes/chunk for the 2-bit-biome shape (+8 for the dirty
  word) and 63232 for single-value biomes (-768, the 24 sections' 2-bit containers).
  Allocations: 131 -> 132/chunk (+1 for the dirty bitset), which also removes the world-level
  hash set entirely. Pinned hashes unchanged.

Next steps if more is needed: `dirty_unloaded` could be dropped if the renderer stopped
meshing unloaded sections (it captures all-air snapshots for them; the unload path reports
those separately through `gone`); light arrays could be stored as `Arc<[u8; 2048]>` copies to
unpin packet buffers even for non-uniform sections.
