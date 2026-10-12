//! Wire helpers shared with `haste-protocol` (WS1) and our own tests.
//!
//! `decode_sections` is the entry point WS1 calls in its tests; `encode_synthetic_chunk` builds
//! a valid 26.3 section payload so `haste-world`'s round-trip tests do not depend on the cache.

use haste_core::chunk::ChunkData;
use haste_core::{BlockStateId, ChunkPos};

use crate::palette::{BiomeContainer, BlockContainer, PalettedContainer, WireWriter, BLOCKS_PER_SECTION};

/// Decodes a raw `level_chunk_with_light` section payload into per-section containers.
/// `section_count` is the dimension's; sections are ordered from the dimension's lowest.
/// `state_bits` = ceil(log2(global block state count)) for direct-encoded containers.
pub fn decode_sections(
    bytes: &[u8],
    section_count: usize,
    state_bits: u8,
) -> Result<Vec<crate::palette::DecodedSection<BlockStateId>>, crate::palette::PaletteError> {
    BlockContainer::decode_sections(bytes, section_count, state_bits)
}

/// Encodes one section payload: block count, blocks container, biomes container.
pub fn encode_section(
    w: &mut WireWriter,
    block_count: i16,
    blocks: &BlockContainer,
    biomes: &BiomeContainer,
    direct_bits: u8,
) {
    w.varint(block_count as u32);
    blocks.encode(w, direct_bits);
    biomes.encode(w, 3);
}

/// Builds a complete synthetic `ChunkData` whose payload is produced by our own encoder, so
/// decode tests never need the cache. `fill(section_index, x, y, z)` picks each block; uniform
/// sections encode as single-value, others as indirect palettes.
pub fn encode_synthetic_chunk(
    pos: ChunkPos,
    _min_section: i32,
    section_count: usize,
    fill: impl Fn(usize, u8, u8, u8) -> BlockStateId,
) -> ChunkData {
    let mut w = WireWriter::new();
    for si in 0..section_count {
        let mut states: Vec<u32> = Vec::with_capacity(BLOCKS_PER_SECTION);
        for y in 0..16u8 {
            for z in 0..16u8 {
                for x in 0..16u8 {
                    states.push(fill(si, x, y, z).0);
                }
            }
        }
        let non_air = states.iter().filter(|&&s| s != BlockStateId::AIR.0).count();
        let mut palette: Vec<u32> = Vec::new();
        for s in &states {
            if !palette.contains(s) {
                palette.push(*s);
            }
        }
        let blocks = if palette.len() == 1 {
            PalettedContainer::single(palette[0])
        } else {
            PalettedContainer::indirect(palette, &states)
        };
        let biomes = PalettedContainer::single_biome(0);
        encode_section(&mut w, non_air as i16, &blocks, &biomes, 16);
    }
    ChunkData {
        pos,
        heightmaps: Vec::new(),
        sections: bytes::Bytes::from(w.data),
        block_entities: Vec::new(),
        light: Default::default(),
    }
}
