//! A chunk: sections, heightmaps, block entities, light. Owned by `World`.

use haste_core::chunk::{BlockEntityData, ChunkData, LightData};
use haste_core::{BiomeId, BlockStateId, ChunkPos};

use crate::light::NibbleArray;
use crate::palette::{BiomeContainer, BlockContainer, DecodedSection, PaletteError};

/// One 16x16x16 section of a chunk.
pub struct Section {
    /// Number of non-air blocks (vanilla's `block_count`); used to skip empty sections.
    pub block_count: i16,
    /// Number of fluid blocks as the server sent it (not maintained by `set`); 0 lets the
    /// mesher skip fluid work for the section.
    pub fluid_count: i16,
    pub blocks: BlockContainer,
    pub biomes: BiomeContainer,
}

impl Section {
    pub fn get(&self, x: u8, y: u8, z: u8) -> BlockStateId {
        self.blocks.get_block(x, y, z)
    }

    pub fn set(&mut self, x: u8, y: u8, z: u8, state: BlockStateId, direct_bits: u8) {
        let old = self.blocks.get_block(x, y, z);
        self.blocks
            .set_index(crate::palette::flat_index(x, y, z), state.0, direct_bits);
        let now_air = state == BlockStateId::AIR;
        let was_air = old == BlockStateId::AIR;
        if was_air && !now_air {
            self.block_count += 1;
        } else if !was_air && now_air && self.block_count > 0 {
            self.block_count -= 1;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.block_count == 0
    }

    /// Biome at cell coordinates (0..4 within the section).
    pub fn biome(&self, x: u8, y: u8, z: u8) -> BiomeId {
        self.biomes.get_biome(x, y, z)
    }
}

/// A heightmap decoded from packed longs. `bits` is derived from the dimension height
/// (9 for a 384-tall world: values 0..=385).
#[derive(Clone)]
pub struct Heightmap {
    pub bits: u8,
    pub longs: Vec<i64>,
}

impl Heightmap {
    pub fn get(&self, x: u8, z: u8) -> i32 {
        let i = (z as usize) * 16 + x as usize;
        let bits = self.bits as usize;
        let per_long = 64 / bits;
        let long = i / per_long;
        let off = (i % per_long) * bits;
        (((self.longs[long] as u64) >> off) as i32) & ((1 << bits) - 1)
    }
}

/// Sky and block light per section, parallel to `Chunk::sections`.
#[derive(Default)]
pub struct ChunkLight {
    pub sky: Vec<Option<NibbleArray>>,
    pub block: Vec<NibbleArray>,
}

impl ChunkLight {
    fn new(section_count: usize) -> Self {
        Self {
            sky: vec![None; section_count],
            block: vec![NibbleArray::default(); section_count],
        }
    }
}

/// A decoded chunk. Sections are ordered from the dimension's lowest section upward.
pub struct Chunk {
    pub pos: ChunkPos,
    /// Y (in section units) of `sections[0]` — the dimension's `min_y >> 4`.
    pub min_section: i32,
    pub sections: Vec<Section>,
    /// Heightmaps by wire type id, as sent.
    pub heightmaps: Vec<(i32, Heightmap)>,
    /// Block entities kept raw (NBT decoded lazily by whoever needs them).
    pub block_entities: Vec<BlockEntityData>,
    /// Sky/block light per section (same indexing as `sections`).
    pub light: ChunkLight,
}

impl Chunk {
    /// Decodes a full chunk from `ChunkData` (the `level_chunk_with_light` payload).
    pub fn decode(
        data: &ChunkData,
        min_section: i32,
        section_count: usize,
        state_bits: u8,
        biome_bits: u8,
    ) -> Result<Chunk, PaletteError> {
        let decoded: Vec<DecodedSection<BlockStateId>> =
            BlockContainer::decode_sections(&data.sections, section_count, state_bits, biome_bits)?;
        let mut sections = Vec::with_capacity(decoded.len());
        for d in decoded {
            sections.push(Section {
                block_count: d.block_count,
                fluid_count: d.fluid_count,
                blocks: d.blocks,
                biomes: d.biomes.remap::<BiomeId>(),
            });
        }
        let heightmaps = data
            .heightmaps
            .iter()
            .map(|(ty, longs)| {
                (
                    *ty,
                    Heightmap {
                        bits: 9,
                        longs: longs.clone(),
                    },
                )
            })
            .collect();
        let mut chunk = Chunk {
            pos: data.pos,
            min_section,
            sections,
            heightmaps,
            block_entities: data.block_entities.clone(),
            light: ChunkLight::new(section_count),
        };
        chunk.apply_light(&data.light);
        Ok(chunk)
    }

    /// Applies light (`level_chunk_with_light` or `light_update` semantics).
    pub fn apply_light(&mut self, light: &LightData) {
        crate::light::apply_to_chunk(self, light);
    }

    pub fn section(&self, y: i32) -> Option<&Section> {
        let idx = y - self.min_section;
        if idx < 0 {
            return None;
        }
        self.sections.get(idx as usize)
    }

    pub fn section_mut(&mut self, y: i32) -> Option<&mut Section> {
        let idx = y - self.min_section;
        if idx < 0 {
            return None;
        }
        self.sections.get_mut(idx as usize)
    }

    /// First heightmap of the given wire type id, if present.
    pub fn heightmap(&self, ty: i32) -> Option<&Heightmap> {
        self.heightmaps
            .iter()
            .find(|(t, _)| *t == ty)
            .map(|(_, h)| h)
    }

    /// Highest non-air y at column (x, z) using the first heightmap (value = top y + 1,
    /// vanilla's heightmap convention), or a linear scan when no heightmap was sent.
    pub fn height_at(&self, x: u8, z: u8) -> Option<i32> {
        if let Some((_, h)) = self.heightmaps.first() {
            return Some(h.get(x, z) - 1);
        }
        let mut si = self.sections.len() as i32 - 1;
        while si >= 0 {
            let s = &self.sections[si as usize];
            if !s.is_empty() {
                for yy in (0..16).rev() {
                    if s.get(x, yy, z) != BlockStateId::AIR {
                        return Some(self.min_section * 16 + si * 16 + yy as i32);
                    }
                }
            }
            si -= 1;
        }
        None
    }
}

/// Test/bench helper: build a synthetic `ChunkData` with our own encoder.
pub fn synthetic_chunk_data(
    pos: ChunkPos,
    min_section: i32,
    section_count: usize,
    fill: impl Fn(usize, u8, u8, u8) -> BlockStateId,
) -> ChunkData {
    crate::wire::encode_synthetic_chunk(pos, min_section, section_count, fill)
}
