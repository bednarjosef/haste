# world-and-data: block entities, and proof that chunk decoding is exact

Worker: GLM. Worktree `$HASTE_WT/world-and-data`, branch `world-and-data`, test server port
**25617**. Read `docs/briefs/COMMON.md` first.

You own `crates/haste-world` and `crates/haste-data`.

## Where it stands
`haste-world` decodes chunk packets into paletted sections, stores light, tracks dirty sections
and makes snapshots for the mesher. It has unit tests but has never been checked against real
chunks from a real server (docs/TASKS.md: "unverified against a real chunk capture"). The client is
being switched from a stand-in world to `haste_world::World` right now (client-app workstream).

## Tasks, in order (commit after each)
1. **Exactness oracle from region files.** The server saves the same chunks it sends us into
   Anvil region files (`cache/servers/world-and-data/world/region/*.mca`; format: minecraft.wiki
   "Region file format", "Chunk format": NBT, `sections[].block_states.palette/data`,
   `biomes`, `BlockLight`/`SkyLight`, `block_entities`). Write an `#[ignore]` integration test
   (a test-only module or `tests/region_oracle.rs`) that reads those region files (Pumpkin has an
   Anvil reader and NBT you may adapt, `cache/pumpkin/pumpkin-world/src/chunk/format/anvil.rs`
   and its NBT crate; or `haste-protocol`'s vendored NBT), and compares, block by block, every
   section of chunks our `World` loaded from the network with what the region file says.
   How to get the network side: use the testkit packet capture
   (`crates/haste-testkit`, `packets/<name>.bin` format in `FORMATS.md`; find how its proxy
   records) or decode `level_chunk_with_light` packets from a headless join yourself.
   Use a normal world (`scripts/test-server.sh world-and-data 25617 3600 minecraft:normal` after
   deleting the copy's `world/`), save-all on the server before reading regions. Every mismatch
   is a bug in our decoding: fix it, keep a regression test with a minimal byte sample you
   construct yourself (never commit captured bytes: they are derived from Mojang's world gen).
   Compare biomes and light too (light: what the server sent vs what it saved can differ by
   timing; report rather than chase light mismatches at chunk edges).
2. **Block entities in the world.** Store `haste_core::chunk::BlockEntityData` from chunk packets,
   apply `block_entity_data` updates, drop them when `set_block` replaces the block or the chunk
   unloads. API: `World::block_entity(&self, pos: BlockPos) -> Option<&BlockEntityData>`,
   `World::block_entities_in(&self, pos: ChunkPos) -> impl Iterator<Item = (BlockPos, &BlockEntityData)>`,
   `World::set_block_entity(&mut self, pos: BlockPos, data: BlockEntityData)`. Mark the section
   dirty when a block entity changes (signs and banners will be meshed or drawn from it).
3. **Section updates and light updates.** `section_blocks_update` (multi-block change),
   `block_update`, `light_update`, `forget_level_chunk`, `chunk_batch_start/finished` and
   `set_chunk_cache_center/radius` go through `World` correctly: test each with constructed
   packets (unit tests on `World` methods, no network), including blocks on section borders
   marking the neighbour section dirty (the mesher reads neighbour faces).
4. **Speed.** Measure `load_chunk` per chunk (median, p99) on the captured chunks and the memory
   per loaded chunk; no per-block allocations. Report the numbers.

## Done
`cargo test -p haste-world -p haste-data` (plus `--ignored` with your server's region files) and
clippy clean; the oracle test passes on at least a 21x21-chunk area of a normal world in the
overworld and a few chunks in the nether (`/execute in minecraft:the_nether run tp ...` from the
server console). Report mismatches found and fixed.
