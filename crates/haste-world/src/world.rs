//! The world: chunk map, block get/set, light, dirty tracking, section snapshots.

use std::sync::Arc;

use haste_core::chunk::{BlockEntityData, ChunkData};
use haste_core::game::DimensionInfo;
use haste_core::view::{BlockView, DirtyTracker};
use haste_core::{BlockPos, BlockStateId, ChunkPos, SectionPos};
use haste_data::GameData;

use crate::chunk::{Chunk, Heightmap, Section};
use crate::snapshot::{BorderCells, SectionSnapshot, SnapshotSection};
use crate::world_error::WorldError;

/// Heap bytes of the loaded world, by storage (see [`World::memory_stats`]).
#[derive(Default, Debug, Clone, Copy)]
pub struct WorldMemoryStats {
    /// The chunk map and its `Chunk` structs.
    pub chunks: u64,
    /// Block + biome containers of all sections.
    pub sections: u64,
    /// Decoded heightmaps.
    pub heightmaps: u64,
    /// Sky/block light nibble arrays.
    pub light: u64,
    /// Block entities (NBT payloads plus map entries).
    pub block_entities: u64,
    /// Sections with at least one non-air block.
    pub non_empty_sections: u64,
}

/// The loaded world of one dimension.
pub struct World {
    dim: DimensionInfo,
    data: Arc<GameData>,
    chunks: ahash::HashMap<i64, Chunk>,
    /// Chunk keys (packed positions) with at least one dirty section, in mark order.
    /// A key can appear twice (a chunk replaced while dirty); the second drain reads
    /// bits that were already cleared.
    dirty_chunks: Vec<i64>,
    /// Sections marked dirty whose chunk is not loaded (a `set_block`/`apply_light` on a
    /// border block can reach into a neighbour chunk that has not arrived yet). Rare, so
    /// they live in a plain list that is deduplicated on drain instead of in a hash set.
    dirty_unloaded: Vec<SectionPos>,
    /// Bits for direct-encoded block containers: ceil(log2(state count)), min 4.
    direct_bits: u8,
    /// Bits for direct-encoded biome containers: ceil(log2(synced biome registry size)).
    biome_bits: u8,
    /// Vanilla `BiomeManager.biomeZoomSeed`: the hashed seed of the login/respawn spawn
    /// info, which picks each block's biome cell (`BiomeManager.getBiome`).
    biome_zoom_seed: i64,
}

impl World {
    /// A world whose direct biome containers are 3 bits wide (the old default, right only
    /// for a biome registry of at most 8 entries). For tests; the client uses
    /// [`World::new_with_biomes`] with the synced registry size.
    pub fn new(dim: DimensionInfo, data: Arc<GameData>) -> World {
        Self::new_with_biomes(dim, data, 8)
    }

    /// A world for a server whose synced `minecraft:worldgen/biome` registry has
    /// `biome_registry_len` entries (direct biome containers are ceil(log2(len)) bits wide).
    pub fn new_with_biomes(
        dim: DimensionInfo,
        data: Arc<GameData>,
        biome_registry_len: usize,
    ) -> World {
        let biome_bits = ceil_log2(biome_registry_len);
        let states = data.blocks.len();
        // Local table width only: matching a modded server's wider bit width needs registry
        // sync (a Phase 2 task), not a bigger local table.
        let direct_bits = (u32::BITS - (states as u32).leading_zeros()).max(4) as u8;
        World {
            dim,
            data,
            chunks: ahash::HashMap::with_hasher(ahash::RandomState::default()),
            dirty_chunks: Vec::new(),
            dirty_unloaded: Vec::new(),
            direct_bits,
            biome_bits,
            biome_zoom_seed: 0,
        }
    }

    /// Direct biome container width in bits.
    pub fn biome_bits(&self) -> u8 {
        self.biome_bits
    }

    pub fn dimension(&self) -> &DimensionInfo {
        &self.dim
    }

    /// The biome zoom seed (vanilla `BiomeManager.biomeZoomSeed`), 0 until the spawn info sets it.
    pub fn biome_zoom_seed(&self) -> i64 {
        self.biome_zoom_seed
    }

    /// Sets the biome zoom seed from the login or respawn spawn info's hashed seed.
    pub fn set_biome_zoom_seed(&mut self, seed: i64) {
        self.biome_zoom_seed = seed;
    }

    pub fn game_data(&self) -> &Arc<GameData> {
        &self.data
    }

    /// Loads (or replaces) a chunk from `level_chunk_with_light` payload.
    pub fn load_chunk(&mut self, data: &ChunkData) -> Result<(), WorldError> {
        let section_count = self.dim.section_count();
        let mut chunk = Chunk::decode(
            data,
            self.dim.min_section(),
            section_count,
            self.direct_bits,
            self.biome_bits,
            self.data.blocks.len() as u32,
        )
        .map_err(WorldError::Palette)?;
        let key = data.pos.to_packed();
        chunk.mark_all_dirty();
        self.chunks.insert(key, chunk);
        self.dirty_chunks.push(key);
        // a new chunk dirties all its sections and the borders of its neighbours,
        // including the diagonal chunks (their corner columns sample the new chunk)
        for dx in -1..=1 {
            for dz in -1..=1 {
                if dx == 0 && dz == 0 {
                    continue;
                }
                let n = ChunkPos::new(data.pos.x + dx, data.pos.z + dz);
                self.mark_chunk_all_dirty(n);
            }
        }
        Ok(())
    }

    pub fn unload_chunk(&mut self, pos: ChunkPos) {
        self.chunks.remove(&pos.to_packed());
        // neighbour borders now face void: remesh them (diagonal chunks included:
        // their corner columns sampled the removed chunk)
        for dx in -1..=1 {
            for dz in -1..=1 {
                if dx == 0 && dz == 0 {
                    continue;
                }
                let n = ChunkPos::new(pos.x + dx, pos.z + dz);
                self.mark_chunk_all_dirty(n);
            }
        }
    }

    /// Marks every section of the loaded chunk at `pos` dirty (no-op for an unloaded
    /// chunk: nothing of it can be meshed anyway).
    fn mark_chunk_all_dirty(&mut self, pos: ChunkPos) {
        let key = pos.to_packed();
        if let Some(chunk) = self.chunks.get_mut(&key) {
            if chunk.mark_all_dirty() {
                self.dirty_chunks.push(key);
            }
        }
    }

    /// Sets a block. Marks every section whose one-block border touches the block dirty: the
    /// mesher samples AO and smooth light from the 3x3x3 block neighbourhood, so edge and
    /// corner blocks also affect the diagonally adjacent sections.
    pub fn set_block(&mut self, pos: BlockPos, state: BlockStateId) {
        let chunk_pos = pos.chunk();
        let sec_y = pos.y >> 4;
        let direct_bits = self.direct_bits;
        let blocks = &self.data.blocks;
        let Some(chunk) = self.chunks.get_mut(&chunk_pos.to_packed()) else {
            return;
        };
        let Some(section) = chunk.section_mut(sec_y) else {
            return;
        };
        let lx = (pos.x & 15) as u8;
        let ly = (pos.y & 15) as u8;
        let lz = (pos.z & 15) as u8;
        let old = section.get(lx, ly, lz);
        let was_empty = section.block_count == 0;
        section.set(lx, ly, lz, state, direct_bits);
        let now_empty = section.block_count == 0;
        // A replaced block drops its block entity (the server resends one if the new block
        // has one); a state change of the same block keeps it (vanilla
        // `LevelChunk.setBlockState`: removed only when `!oldState.is(newBlock)`).
        if blocks.block_of(old) != blocks.block_of(state) {
            chunk.block_entities.remove(&pos);
        }
        // vanilla `LevelChunk.setBlockState`: a section that flipped between empty and
        // non-empty gains or loses its light layers (`updateSectionStatus`). Gaining them
        // needs no work here (the engine materializes on write, and the absent stand-ins
        // read the same values); losing them drops the arrays so reads flatten through to
        // the sections above, and re-meshes the neighbourhood
        // (`markSectionAndNeighborsAsAffected`).
        if !was_empty && now_empty {
            let idx = (sec_y - chunk.min_section) as usize;
            chunk.light.sky[idx] = crate::light::SkyLight::Absent;
            chunk.light.block[idx] = crate::light::NibbleArray::default();
            for dy in -1..=1i32 {
                for dx in -1..=1i32 {
                    for dz in -1..=1i32 {
                        self.mark_dirty(SectionPos::new(
                            chunk_pos.x + dx,
                            sec_y + dy,
                            chunk_pos.z + dz,
                        ));
                    }
                }
            }
        }
        // vanilla recomputes light for the changed block in the same frame
        // (`LevelChunk.setBlockState` -> `checkBlock`, `ClientLevel.update` ->
        // `runLightUpdates`); the server only sends border-chunk `light_update`s, so the
        // client must do this itself
        if crate::light::light_properties_differ(&self.data, old, state) {
            crate::light::run_for_block(self, pos);
        }
        // per axis, the section offsets of the block's 3x3x3 neighbourhood: {0} plus {-1}
        // when on the low border, {+1} when on the high border (at most 2 per axis)
        let offs = |l: u8| -> &[i32] {
            if l == 0 {
                &[-1, 0]
            } else if l == 15 {
                &[0, 1]
            } else {
                &[0]
            }
        };
        let xs = offs(lx);
        let ys = offs(ly);
        let zs = offs(lz);
        // sections outside the world's vertical range are skipped: the dimension knows its
        // section span, and the mesher can never read them (out-of-bounds y is air)
        let min_sec = self.dim.min_section();
        let max_sec = min_sec + self.dim.section_count() as i32;
        for &dx in xs {
            for &dy in ys {
                for &dz in zs {
                    let ty = sec_y + dy;
                    if ty < min_sec || ty >= max_sec {
                        continue;
                    }
                    self.mark_dirty(SectionPos::new(chunk_pos.x + dx, ty, chunk_pos.z + dz));
                }
            }
        }
    }

    /// The block entity at `pos` (signs, chests, ...), if that chunk is loaded.
    pub fn block_entity(&self, pos: BlockPos) -> Option<&BlockEntityData> {
        self.chunk_of(pos).and_then(|c| c.block_entities.get(&pos))
    }

    /// Every block entity in chunk `pos`, in arbitrary order.
    pub fn block_entities_in(
        &self,
        pos: ChunkPos,
    ) -> impl Iterator<Item = (BlockPos, &BlockEntityData)> + '_ {
        self.chunk(pos)
            .into_iter()
            .flat_map(|c| c.block_entities.iter().map(|(p, be)| (*p, be)))
    }

    /// Replaces (or adds) the block entity at `pos` of `data` (the `block_entity_data`
    /// packet). Ignored if that chunk is not loaded; marks the section dirty (the change is
    /// visible in the mesh or UI).
    pub fn set_block_entity(&mut self, pos: BlockPos, data: BlockEntityData) {
        let chunk_pos = pos.chunk();
        let Some(chunk) = self.chunks.get_mut(&chunk_pos.to_packed()) else {
            return;
        };
        chunk.block_entities.insert(pos, data);
        self.mark_dirty(pos.section());
    }

    /// Applies a `light_update` to the chunk at `pos`. Marks dirty the 3x3x3 section
    /// neighbourhood of every changed light section, like `set_block` does for blocks: the
    /// mesher's smooth light samples the 3x3x3 block neighbourhood of each block, so a
    /// section's border blocks read light from the adjacent sections (diagonal ones too, in
    /// neighbouring chunks).
    ///
    /// The server only sends these for chunks on the *border* of a player's tracked area
    /// (vanilla `ChunkHolder.broadcastChanges` sends `ClientboundLightUpdatePacket` to
    /// `getPlayers(pos, true)` — `isChunkOnTrackedBorder`): block changes in interior chunks
    /// arrive with no light update at all, because the vanilla client recomputes light
    /// itself on every block change (`LevelChunk.setBlockState` -> `LevelLightEngine
    /// .checkBlock`, run from `ClientLevel.update`). Haste does the same from `set_block`
    /// (see `crate::light`); this path only matters for border chunks, where the packet's
    /// values overwrite the locally computed (identical) ones.
    pub fn apply_light(&mut self, pos: ChunkPos, light: &haste_core::chunk::LightData) {
        if let Some(chunk) = self.chunks.get_mut(&pos.to_packed()) {
            chunk.apply_light(light);
            // light sections are indexed from one below the world: light index i is
            // section y `min_section - 1 + i`, `section_count + 2` of them in total
            let min_sec = self.dim.min_section();
            let max_sec = min_sec + self.dim.section_count() as i32;
            let changed = |i: i32| -> bool {
                let set =
                    |mask: &[u64]| mask.get(i as usize / 64).map_or(0, |w| w >> (i % 64) & 1) != 0;
                set(&light.sky_mask)
                    || set(&light.block_mask)
                    || set(&light.empty_sky_mask)
                    || set(&light.empty_block_mask)
            };
            for i in 0..self.dim.section_count() as i32 + 2 {
                if !changed(i) {
                    continue;
                }
                // the light sections outside the world's span only change light for the
                // sections next to them: sections outside the span are skipped (out-of-bounds
                // y is air for the mesher)
                let sy = min_sec - 1 + i;
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        for dz in -1..=1 {
                            let ty = sy + dy;
                            if ty < min_sec || ty >= max_sec {
                                continue;
                            }
                            self.mark_dirty(SectionPos::new(pos.x + dx, ty, pos.z + dz));
                        }
                    }
                }
            }
        }
    }

    /// Replaces every section's biome container of the loaded chunk at `pos` with the
    /// containers decoded from `data` (the `chunks_biomes` payload), the way vanilla's
    /// `LevelChunk.replaceBiomes` does. Marks the 3x3 chunk columns dirty (biome tint).
    /// No-op for an unloaded chunk.
    pub fn replace_biomes(&mut self, pos: ChunkPos, data: &[u8]) {
        use crate::palette::{BiomeContainer, WireReader, BIOMES_PER_SECTION};
        let key = pos.to_packed();
        let Some(chunk) = self.chunks.get_mut(&key) else {
            return;
        };
        let biome_bits = self.biome_bits;
        let section_count = chunk.sections.len();
        let mut r = WireReader::new(data);
        for section in chunk.sections.iter_mut().take(section_count) {
            match BiomeContainer::decode(&mut r, BIOMES_PER_SECTION, biome_bits) {
                Ok(b) => section.biomes = b,
                Err(_) => break,
            }
        }
        for dx in -1..=1i32 {
            for dz in -1..=1i32 {
                self.mark_chunk_all_dirty(ChunkPos::new(pos.x + dx, pos.z + dz));
            }
        }
    }

    pub fn chunk(&self, pos: ChunkPos) -> Option<&Chunk> {
        self.chunks.get(&pos.to_packed())
    }

    /// The chunk containing `pos`, mutably. Unloaded -> None.
    pub(crate) fn chunk_mut(&mut self, pos: ChunkPos) -> Option<&mut Chunk> {
        self.chunks.get_mut(&pos.to_packed())
    }

    pub fn loaded_chunks(&self) -> impl Iterator<Item = ChunkPos> + '_ {
        self.chunks.values().map(|c| c.pos)
    }

    /// Resident-world memory diagnostics (bytes): the chunk map's heap footprint split
    /// into sections (blocks+biomes containers), heightmaps, light and block entities,
    /// plus the number of non-empty sections. Counts live allocations (capacity of the
    /// backing vectors), not allocator slack.
    pub fn memory_stats(&self) -> WorldMemoryStats {
        let mut stats = WorldMemoryStats {
            chunks: self.chunks.len() as u64 * (std::mem::size_of::<Chunk>() as u64 + 16),
            ..Default::default()
        };
        for chunk in self.chunks.values() {
            stats.heightmaps += chunk
                .heightmaps
                .iter()
                .map(|(_, h)| {
                    h.longs.capacity() as u64 * 8 + std::mem::size_of::<Heightmap>() as u64
                })
                .sum::<u64>();
            stats.block_entities += chunk
                .block_entities
                .values()
                .map(|be| be.nbt.len() as u64 + std::mem::size_of::<BlockEntityData>() as u64 + 24)
                .sum::<u64>();
            for s in &chunk.sections {
                stats.sections += s.memory_bytes();
                if !s.is_empty() {
                    stats.non_empty_sections += 1;
                }
            }
            // Light: the layout's own accounting (see `ChunkLight::heap_bytes`).
            stats.light += chunk.light.heap_bytes();
        }
        stats
    }

    /// Immutable copy of a section (plus 1-block border from neighbours and its light) for
    /// meshing threads. `None` if the section's chunk is not loaded.
    pub fn snapshot_section(&self, pos: SectionPos) -> Option<Arc<SectionSnapshot>> {
        let chunk = self.chunks.get(&pos.chunk().to_packed())?;
        let idx = pos.y - chunk.min_section;
        if idx < 0 || idx as usize >= chunk.sections.len() {
            return None;
        }
        let section = &chunk.sections[idx as usize];

        // Border blocks: neighbours in the six face directions. `None` = unloaded/void (treated
        // as air by the mesher through `SectionView`).
        let mut border = BorderCells::default();
        let gather = |wx: i32, wy: i32, wz: i32| -> Option<BlockStateId> {
            let cp = BlockPos::new(wx, wy, wz).chunk();
            let c = self.chunks.get(&cp.to_packed())?;
            let sy = wy >> 4;
            let s = c.section(sy)?;
            Some(s.get((wx & 15) as u8, (wy & 15) as u8, (wz & 15) as u8))
        };
        let base = pos.block_min();
        // -x face (16 y * 16 z)
        for y in 0..16 {
            for z in 0..16 {
                border
                    .neg_x
                    .push(gather(base.x - 1, base.y + y, base.z + z).unwrap_or(BlockStateId::AIR));
            }
        }
        for y in 0..16 {
            for z in 0..16 {
                border
                    .pos_x
                    .push(gather(base.x + 16, base.y + y, base.z + z).unwrap_or(BlockStateId::AIR));
            }
        }
        for y in 0..16 {
            for x in 0..16 {
                border
                    .neg_z
                    .push(gather(base.x + x, base.y + y, base.z - 1).unwrap_or(BlockStateId::AIR));
            }
        }
        for y in 0..16 {
            for x in 0..16 {
                border
                    .pos_z
                    .push(gather(base.x + x, base.y + y, base.z + 16).unwrap_or(BlockStateId::AIR));
            }
        }
        for z in 0..16 {
            for x in 0..16 {
                border
                    .neg_y
                    .push(gather(base.x + x, base.y - 1, base.z + z).unwrap_or(BlockStateId::AIR));
            }
        }
        for z in 0..16 {
            for x in 0..16 {
                border
                    .pos_y
                    .push(gather(base.x + x, base.y + 16, base.z + z).unwrap_or(BlockStateId::AIR));
            }
        }

        // An absent section is not all 15: it reads the bottom row of the nearest section
        // above with data (vanilla: SkyLightSectionStorage.getLightValue flattens y), so
        // the snapshot gets a stand-in repeating that row (one value when it is uniform).
        let sky = match chunk.light.sky.get(idx as usize) {
            Some(sky) if sky.is_storing() => sky.clone(),
            _ => crate::sky_light::absent_section_light(chunk, idx as usize),
        };
        let block_light = chunk
            .light
            .block
            .get(idx as usize)
            .cloned()
            .unwrap_or_default();

        Some(Arc::new(SectionSnapshot {
            pos,
            section: SnapshotSection {
                block_count: section.block_count,
                fluid_count: section.fluid_count,
                blocks: section.blocks.clone(),
                biomes: section.biomes.clone(),
            },
            border,
            sky,
            block: block_light,
        }))
    }

    /// Marks one section dirty. A section whose chunk is not loaded goes to the
    /// world's unloaded list instead (a `set_block` on a border block reaches into a
    /// neighbour chunk that may not have arrived yet).
    fn mark_dirty(&mut self, pos: SectionPos) {
        let key = pos.chunk().to_packed();
        if let Some(chunk) = self.chunks.get_mut(&key) {
            let idx = pos.y - chunk.min_section;
            if idx >= 0 && chunk.mark_dirty(idx as usize) {
                self.dirty_chunks.push(key);
            }
        } else {
            self.dirty_unloaded.push(pos);
        }
    }

    /// `mark_dirty` for the light engine (crate-internal).
    pub(crate) fn mark_dirty_section(&mut self, pos: SectionPos) {
        self.mark_dirty(pos)
    }

    // -- BlockView helpers --

    /// The chunk containing `pos`. Unloaded -> None.
    #[inline]
    fn chunk_of(&self, pos: BlockPos) -> Option<&Chunk> {
        self.chunks.get(&pos.chunk().to_packed())
    }

    fn chunk_and_section(&self, pos: BlockPos) -> Option<(&Chunk, &Section)> {
        let chunk = self.chunk_of(pos)?;
        let section = chunk.section(pos.y >> 4)?;
        Some((chunk, section))
    }
}

impl BlockView for World {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        match self.chunk_and_section(pos) {
            Some((_, s)) => s.get((pos.x & 15) as u8, (pos.y & 15) as u8, (pos.z & 15) as u8),
            None => BlockStateId::AIR,
        }
    }

    fn is_chunk_loaded(&self, pos: ChunkPos) -> bool {
        self.chunks.contains_key(&pos.to_packed())
    }

    fn sky_light(&self, pos: BlockPos) -> u8 {
        match self.chunk_and_section(pos) {
            Some((c, _)) => {
                let idx = ((pos.y >> 4) - c.min_section) as usize;
                let (lx, ly, lz) = ((pos.x & 15) as u8, (pos.y & 15) as u8, (pos.z & 15) as u8);
                match c.light.sky.get(idx).and_then(|s| s.value(lx, ly, lz)) {
                    Some(v) => v,
                    // vanilla: SkyLightSectionStorage.getLightValue — a section with no
                    // data layer reads the bottom row of the nearest layer above
                    None => crate::sky_light::absent_value(c, idx, lx, lz),
                }
            }
            None => 15,
        }
    }

    fn block_light(&self, pos: BlockPos) -> u8 {
        match self.chunk_and_section(pos) {
            Some((c, _)) => {
                let idx = (pos.y >> 4) - c.min_section;
                c.light
                    .block
                    .get(idx as usize)
                    .map(|n| n.get((pos.x & 15) as u8, (pos.y & 15) as u8, (pos.z & 15) as u8))
                    .unwrap_or(0)
            }
            None => 0,
        }
    }

    fn min_y(&self) -> i32 {
        self.dim.min_y
    }

    fn height(&self) -> i32 {
        self.dim.height
    }
}

impl DirtyTracker for World {
    fn take_dirty_sections(&mut self) -> Vec<SectionPos> {
        let mut out = std::mem::take(&mut self.dirty_unloaded);
        if out.len() > 1 {
            // the same unloaded section can be marked repeatedly before a drain
            out.sort_unstable_by_key(|p| (p.x, p.y, p.z));
            out.dedup_by_key(|p| (p.x, p.y, p.z));
        }
        for key in self.dirty_chunks.drain(..) {
            if let Some(chunk) = self.chunks.get_mut(&key) {
                chunk.take_dirty(&mut out);
            }
        }
        out
    }
}

/// ceil(log2(n)), at least 1 (bit width of an id in `0..n`).
fn ceil_log2(n: usize) -> u8 {
    ((usize::BITS - n.saturating_sub(1).leading_zeros()) as u8).max(1)
}
