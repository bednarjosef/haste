//! A chunk: sections, heightmaps, block entities, light. Owned by `World`.

use haste_core::chunk::{BlockEntityData, ChunkData, LightData};
use haste_core::{BiomeId, BlockPos, BlockStateId, ChunkPos, SectionPos};

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

    /// Heap bytes of this section's block and biome containers (live allocations).
    pub fn memory_bytes(&self) -> u64 {
        self.blocks.memory_bytes() + self.biomes.memory_bytes()
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
    pub(crate) fn new(section_count: usize) -> Self {
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
    /// Block entities kept raw (NBT decoded lazily by whoever needs them), by position.
    /// Replacing the chunk (a re-sent `level_chunk_with_light`) replaces these wholesale.
    pub block_entities: ahash::HashMap<BlockPos, BlockEntityData>,
    /// Sky/block light per section (same indexing as `sections`).
    pub light: ChunkLight,
    /// Per-column lowest sky source y (vanilla `ChunkSkyLightSources`), lazily built by
    /// the light engine on the first block change that needs it. `i32::MIN` per column
    /// means the column is open to the world's floor.
    pub(crate) sky_sources: Option<Box<[i32; 256]>>,
    /// One dirty bit per section (index = section y - `min_section`), so remesh marking
    /// is a bit-set instead of a hash insert. Owned by the chunk, cleared when the
    /// world drains the dirty sections.
    pub(crate) dirty: Vec<u64>,
}

/// Bit `i % 64` of word `i / 64` set.
fn bit_word(i: usize) -> (usize, u64) {
    (i / 64, 1u64 << (i % 64))
}

impl Chunk {
    /// Decodes a full chunk from `ChunkData` (the `level_chunk_with_light` payload).
    /// `state_table_len` is the local block-state table size: ids at or above it are unknown
    /// (modded server). They are detected once per chunk and logged once per session; the
    /// chunk is kept and the ids read back as the placeholder state.
    pub fn decode(
        data: &ChunkData,
        min_section: i32,
        section_count: usize,
        state_bits: u8,
        biome_bits: u8,
        state_table_len: u32,
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
        let mut largest_unknown = None;
        for s in &sections {
            if let Some(v) = s.blocks.largest_unknown_id(state_table_len) {
                largest_unknown = Some(largest_unknown.map_or(v, |m: u32| m.max(v)));
            }
        }
        if let Some(largest) = largest_unknown {
            crate::palette::warn_unknown_block_state(largest, state_table_len);
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
            block_entities: data
                .block_entities
                .iter()
                .map(|be| (be.pos, be.clone()))
                .collect(),
            light: ChunkLight::new(section_count),
            sky_sources: None,
            dirty: vec![0; section_count.div_ceil(64)],
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

    /// Marks section `idx` (0-based from the dimension's lowest section) as needing a
    /// remesh. Returns true when the chunk had no dirty section before, so the caller
    /// knows to add it to the drain list.
    pub(crate) fn mark_dirty(&mut self, idx: usize) -> bool {
        if idx >= self.sections.len() {
            return false;
        }
        let had_none = self.dirty.iter().all(|&w| w == 0);
        let (w, bit) = bit_word(idx);
        self.dirty[w] |= bit;
        had_none
    }

    /// Marks every section dirty (a freshly loaded or replaced chunk).
    pub(crate) fn mark_all_dirty(&mut self) -> bool {
        let was_clean = self.dirty.iter().all(|&w| w == 0);
        self.dirty.fill(u64::MAX);
        // the last word may hold bits past the section count: those are not sections
        let rem = self.sections.len() % 64;
        if rem != 0 {
            if let Some(last) = self.dirty.last_mut() {
                *last &= (1u64 << rem) - 1;
            }
        }
        was_clean
    }

    /// The dirty bits: bit `i` of word `i / 64` is section `i` (indexed from the
    /// dimension's lowest section). One word covers up to 64 sections.
    pub fn dirty_bits(&self) -> &[u64] {
        &self.dirty
    }

    /// Appends this chunk's dirty sections to `out` (in section order), clearing the
    /// bits as they go. Sections with no blocks are skipped: they mesh to nothing and
    /// their neighbours do not change with them (vanilla's chunk cache drops them the
    /// same way, `LevelChunkSection.hasOnlyAir`).
    pub(crate) fn take_dirty(&mut self, out: &mut Vec<SectionPos>) {
        for (w, word) in self.dirty.iter_mut().enumerate() {
            let mut bits = *word;
            while bits != 0 {
                let b = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let y = self.min_section + (w * 64 + b) as i32;
                if self
                    .sections
                    .get(w * 64 + b)
                    .is_some_and(|s| s.block_count == 0)
                {
                    continue;
                }
                out.push(SectionPos::new(self.pos.x, y, self.pos.z));
            }
            *word = 0;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A chunk with `sections` stone-filled sections (the dirty bitset is what these
    /// tests exercise; the contents only need to be non-empty since `take_dirty`
    /// skips air sections).
    fn chunk_with_sections(sections: usize, min_section: i32) -> Chunk {
        let cd = synthetic_chunk_data(
            ChunkPos::new(1, -2),
            min_section,
            sections,
            |_si, _x, _y, _z| BlockStateId(1),
        );
        Chunk::decode(&cd, min_section, sections, 15, 7, u32::MAX).unwrap()
    }

    #[test]
    fn dirty_bitset_marks_and_takes_sections() {
        let mut c = chunk_with_sections(24, -4);
        assert!(!c.dirty.iter().any(|&w| w != 0));
        // marking twice keeps one bit; a chunk that is dirty already reports "was clean"
        // no more, so the world lists it once
        assert!(c.mark_dirty(0));
        assert!(!c.mark_dirty(0));
        assert!(!c.mark_dirty(23));
        let mut out = Vec::new();
        c.take_dirty(&mut out);
        assert_eq!(
            out,
            vec![SectionPos::new(1, -4, -2), SectionPos::new(1, 19, -2)]
        );
        // drained: nothing left
        assert!(!c.dirty.iter().any(|&w| w != 0));
        out.clear();
        c.take_dirty(&mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn mark_all_dirty_covers_exactly_the_sections() {
        // 24 sections: the single word's upper 40 bits must not become sections
        let mut c = chunk_with_sections(24, -4);
        assert!(c.mark_all_dirty());
        assert!(!c.mark_all_dirty());
        let mut out = Vec::new();
        c.take_dirty(&mut out);
        assert_eq!(out.len(), 24);
        assert_eq!(out.first().copied(), Some(SectionPos::new(1, -4, -2)));
        assert_eq!(out.last().copied(), Some(SectionPos::new(1, 19, -2)));
    }

    #[test]
    fn dirty_bitset_second_word_and_out_of_range() {
        // a taller-than-64-sections world uses more than one word
        let mut c = chunk_with_sections(70, 0);
        assert!(c.mark_dirty(0));
        assert!(!c.mark_dirty(64));
        assert!(!c.mark_dirty(69));
        // past the section count: ignored
        assert!(!c.mark_dirty(70));
        assert!(!c.mark_dirty(1000));
        let mut out = Vec::new();
        c.take_dirty(&mut out);
        assert_eq!(
            out,
            vec![
                SectionPos::new(1, 0, -2),
                SectionPos::new(1, 64, -2),
                SectionPos::new(1, 69, -2),
            ]
        );
    }
}
