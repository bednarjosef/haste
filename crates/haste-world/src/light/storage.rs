//! The light engine's view of the world's light storage, vanilla `LayerLightSectionStorage`:
//! per-cell level reads and writes over both layers, the "storing" test that gates
//! propagation, and the writeback that returns modified sections for re-meshing.
//!
//! Writes go to per-section byte overlays (`Vec<u8>` nibble arrays), materialized from the
//! stored wire arrays (or their absent-section stand-ins) on first write; `finish` writes
//! them back into the chunks, re-applying the uniform-array dedup of `light.rs`. The engine
//! runs only for block changes, so per-run allocations here are fine.
//!
//! Shape-based light occlusion (vanilla `useShapeForLightOcclusion`, the merged-face
//! `getLightDampeningInto` = 16 case) is not modelled: per-state dampening comes from
//! `light_dampening` below.

use ahash::HashMap;
use haste_core::game::DimensionInfo;
use haste_core::view::BlockPhysics;
use haste_core::{BlockPos, BlockStateId, ChunkPos, SectionPos};
use haste_data::GameData;

use crate::chunk::Chunk;
use crate::world::World;

use super::engine::{unpack_x, unpack_y, unpack_z};
use super::{nibble_get, nibble_set, NibbleArray, SkyLight};

/// Which light layer an operation addresses.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Layer {
    Block,
    Sky,
}

/// HashMap key for the overlays: the section position, packed like
/// `SectionBlocksUpdate::pack_section`.
type SectionKey = i64;

fn key(sec: SectionPos) -> SectionKey {
    ((sec.x as i64 & 0x3F_FFFF) << 42)
        | ((sec.z as i64 & 0x3F_FFFF) << 20)
        | (sec.y as i64 & 0xF_FFFF)
}

fn unkey(k: SectionKey) -> SectionPos {
    let s = |v: i64, bits: u32| -> i32 { ((v << (64 - bits)) >> (64 - bits)) as i32 };
    SectionPos::new(s(k >> 42, 22), s(k, 20), s(k >> 20, 22))
}

pub(super) struct Storage<'a> {
    world: &'a mut World,
    /// Writable section overlays by section position, one per layer.
    block: HashMap<SectionKey, Vec<u8>>,
    sky: HashMap<SectionKey, Vec<u8>>,
}

impl<'a> Storage<'a> {
    pub(super) fn new(world: &'a mut World) -> Storage<'a> {
        Storage {
            world,
            block: HashMap::with_hasher(ahash::RandomState::default()),
            sky: HashMap::with_hasher(ahash::RandomState::default()),
        }
    }

    /// The section's index inside its chunk, or None when out of the world's section span.
    fn section_index(&self, chunk: &Chunk, sec_y: i32) -> Option<usize> {
        let idx = sec_y - chunk.min_section;
        if idx < 0 || idx as usize >= chunk.sections.len() {
            None
        } else {
            Some(idx as usize)
        }
    }

    /// Vanilla `storingLightForSection`: whether the section has a light layer. Layers exist
    /// for sections that hold blocks (the client marks every non-empty section on chunk
    /// arrival, `ClientChunkCache.refreshEmptySections`, and on the emptiness flip in
    /// `LevelChunk.setBlockState`) and for sections that arrived with a stored array.
    pub(super) fn storing(&self, layer: Layer, pos: u64) -> bool {
        let (x, y, z) = (unpack_x(pos), unpack_y(pos), unpack_z(pos));
        let Some(chunk) = self.world.chunk(ChunkPos::new(x >> 4, z >> 4)) else {
            return false;
        };
        let Some(idx) = self.section_index(chunk, y >> 4) else {
            return false;
        };
        match layer {
            Layer::Block => {
                !chunk.light.block[idx].is_empty() || chunk.sections[idx].block_count > 0
            }
            Layer::Sky => chunk.light.sky[idx].is_storing() || chunk.sections[idx].block_count > 0,
        }
    }

    /// The stored level of a cell, vanilla `LayerLightSectionStorage.getStoredLevel`. For a
    /// sky section whose array is absent (all-15 dedup or never sent) but storing (it holds
    /// blocks), vanilla reads the layer it materialized from the walk-up; the absent read
    /// below is that same walk-up. Callers only read storing sections.
    pub(super) fn stored_level(&self, layer: Layer, pos: u64) -> u8 {
        let (x, y, z) = (unpack_x(pos), unpack_y(pos), unpack_z(pos));
        let lx = (x & 15) as u8;
        let ly = (y & 15) as u8;
        let lz = (z & 15) as u8;
        let sec = key(SectionPos::new(x >> 4, y >> 4, z >> 4));
        let overlay = match layer {
            Layer::Block => &self.block,
            Layer::Sky => &self.sky,
        };
        if let Some(arr) = overlay.get(&sec) {
            return nibble_get(arr, lx, ly, lz);
        }
        let Some(chunk) = self.world.chunk(ChunkPos::new(x >> 4, z >> 4)) else {
            return 0;
        };
        let Some(idx) = self.section_index(chunk, y >> 4) else {
            return 0;
        };
        match layer {
            Layer::Block => chunk.light.block[idx].get(lx, ly, lz),
            Layer::Sky => match chunk.light.sky[idx].value(lx, ly, lz) {
                Some(v) => v,
                None => self.walk_up_sky(chunk, idx as i32 + 1, lx, lz),
            },
        }
    }

    /// Sky light read through absent sections: the bottom row of the nearest section above
    /// `from_idx` that has an array (overlay or stored), or 15 when none does
    /// (vanilla `SkyLightSectionStorage.getLightValue`).
    fn walk_up_sky(&self, chunk: &Chunk, from_idx: i32, lx: u8, lz: u8) -> u8 {
        let mut idx = from_idx;
        while (idx as usize) < chunk.light.sky.len() {
            let sec = SectionPos::new(chunk.pos.x, chunk.min_section + idx, chunk.pos.z);
            if let Some(arr) = self.sky.get(&key(sec)) {
                return nibble_get(arr, lx, 0, lz);
            }
            if let Some(v) = chunk.light.sky[idx as usize].row_value(lx, lz) {
                return v;
            }
            idx += 1;
        }
        15
    }

    /// Writes a cell, materializing the section's overlay on first write (vanilla
    /// `getDataLayerToWrite`'s copy-on-write). Only storing sections are written;
    /// callers guarantee that.
    pub(super) fn set_stored_level(&mut self, layer: Layer, pos: u64, level: u8) {
        let (x, y, z) = (unpack_x(pos), unpack_y(pos), unpack_z(pos));
        let sec = SectionPos::new(x >> 4, y >> 4, z >> 4);
        let k = key(sec);
        let have = match layer {
            Layer::Block => self.block.contains_key(&k),
            Layer::Sky => self.sky.contains_key(&k),
        };
        if !have {
            let arr = materialize(&self.sky, self.world, layer, sec);
            match layer {
                Layer::Block => self.block.insert(k, arr),
                Layer::Sky => self.sky.insert(k, arr),
            };
        }
        let arr = match layer {
            Layer::Block => &mut self.block,
            Layer::Sky => &mut self.sky,
        }
        .get_mut(&k)
        .expect("just inserted");
        nibble_set(arr, (x & 15) as u8, (y & 15) as u8, (z & 15) as u8, level);
    }

    /// Vanilla `LightEngine.getOpacity`: at least 1. Unloaded chunks read as fully opaque
    /// (vanilla's `getState` returns bedrock there).
    pub(super) fn opacity(&self, pos: u64) -> u8 {
        let (x, y, z) = (unpack_x(pos), unpack_y(pos), unpack_z(pos));
        match self.block_state(BlockPos::new(x, y, z)) {
            Some(state) => light_dampening(self.world.game_data(), state).max(1),
            None => 15,
        }
    }

    /// Vanilla `BlockLightEngine.getEmission` (always "light on": loaded chunks have light).
    pub(super) fn emission(&self, pos: u64) -> u8 {
        let (x, y, z) = (unpack_x(pos), unpack_y(pos), unpack_z(pos));
        self.block_state(BlockPos::new(x, y, z))
            .map_or(0, |s| self.world.game_data().light_emission(s))
    }

    pub(super) fn block_state(&self, pos: BlockPos) -> Option<BlockStateId> {
        let chunk = self.world.chunk(pos.chunk())?;
        let section = chunk.section(pos.y >> 4)?;
        Some(section.get((pos.x & 15) as u8, (pos.y & 15) as u8, (pos.z & 15) as u8))
    }

    /// The world, for the layer logic that needs more than cell access.
    pub(super) fn world(&mut self) -> &mut World {
        self.world
    }

    /// Whether the dimension has skylight (the sky layer's "light on" gate).
    pub(super) fn has_skylight(&self) -> bool {
        self.world.dimension().has_skylight
    }

    /// The world's dimension info.
    pub(super) fn dimension(&self) -> &DimensionInfo {
        self.world.dimension()
    }

    /// Writes the overlays back, re-applying the uniform-section dedup (`light.rs`), and
    /// returns every section whose cells were written (the caller re-meshes their 3x3x3
    /// neighbourhoods, like vanilla `setStoredLevel`'s `aroundAndAtBlockPos` collection in
    /// `swapSectionMap`).
    pub(super) fn finish(self) -> Vec<SectionPos> {
        let Storage { world, block, sky } = self;
        let mut written = Vec::with_capacity(block.len() + sky.len());
        for (k, arr) in block {
            let sec = unkey(k);
            if let Some(chunk) = world.chunk_mut(sec.chunk()) {
                let idx = sec.y - chunk.min_section;
                if idx >= 0 && (idx as usize) < chunk.sections.len() {
                    chunk.light.block[idx as usize] = if arr.iter().all(|&b| b == 0) {
                        NibbleArray::default()
                    } else {
                        NibbleArray::from_vec(arr)
                    };
                }
            }
            written.push(sec);
        }
        for (k, arr) in sky {
            let sec = unkey(k);
            if let Some(chunk) = world.chunk_mut(sec.chunk()) {
                let idx = sec.y - chunk.min_section;
                if idx >= 0 && (idx as usize) < chunk.sections.len() {
                    chunk.light.sky[idx as usize] = SkyLight::from_packed(&arr);
                }
            }
            written.push(sec);
        }
        written
    }
}

/// The section's bytes to edit, vanilla `LayerLightSectionStorage.createDataLayer`: a copy
/// of the stored array, or its absent stand-in (the walk-up repeat for sky sections below
/// the column's top, all-15 above it, zeros for block light).
fn materialize(
    sky: &HashMap<SectionKey, Vec<u8>>,
    world: &World,
    layer: Layer,
    sec: SectionPos,
) -> Vec<u8> {
    let Some(chunk) = world.chunk(sec.chunk()) else {
        return vec![0u8; 2048];
    };
    let idx = sec.y - chunk.min_section;
    if idx < 0 || idx as usize >= chunk.sections.len() {
        return vec![0u8; 2048];
    }
    match layer {
        Layer::Block => chunk.light.block[idx as usize].to_vec(),
        Layer::Sky => {
            match &chunk.light.sky[idx as usize] {
                // a real array: copy it
                SkyLight::Array(a) => return a.to_vec(),
                // a uniform section (vanilla's null-data `DataLayer`): its one value
                SkyLight::Uniform(v) => return vec![v | (v << 4); 2048],
                SkyLight::Absent => {}
            }
            // absent: repeat the nearest array above's bottom row down the whole
            // section (the absent-section flattening, crate::sky_light's rule)
            let mut arr = vec![0u8; 2048];
            let mut above = idx + 1;
            while (above as usize) < chunk.light.sky.len() {
                let asec = SectionPos::new(chunk.pos.x, chunk.min_section + above, chunk.pos.z);
                let read = |x: u8, z: u8| -> Option<u8> {
                    if let Some(a) = sky.get(&key(asec)) {
                        return Some(nibble_get(a, x, 0, z));
                    }
                    chunk.light.sky[above as usize].row_value(x, z)
                };
                if read(0, 0).is_some() {
                    for z in 0..16u8 {
                        for x in 0..16u8 {
                            let v = read(x, z).unwrap_or(0);
                            for y in 0..16u8 {
                                nibble_set(&mut arr, x, y, z, v);
                            }
                        }
                    }
                    return arr;
                }
                above += 1;
            }
            // nothing above with data: full sky
            arr.fill(0xFF);
            arr
        }
    }
}

/// Vanilla `BlockBehaviour.getLightDampening` per state, computed from the game data the
/// oracle already carries: `isSolidRender` (the dump's `opaque`) means 15; otherwise
/// `propagatesSkylightDown` (not a full collision cube and not a fluid) means 0, else 1.
/// Two overrides are matched by block path: the glass family forces skylight through
/// (`TransparentBlock`) and tinted glass blocks it entirely (15). Leaves land on 1 through
/// their full cube, like vanilla's own override.
pub(crate) fn light_dampening(data: &GameData, state: BlockStateId) -> u8 {
    if state == BlockStateId::AIR {
        return 0;
    }
    if data.is_opaque_full(state) {
        return 15;
    }
    let path = data.blocks.block_of(state).path();
    if path == "tinted_glass" {
        return 15;
    }
    if data.fluid(state).is_some() {
        return 1;
    }
    if !data.shapes.is_collision_full(state) {
        return 0;
    }
    if path == "glass" || path.ends_with("_glass") {
        return 0;
    }
    1
}
