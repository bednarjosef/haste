//! Protocol-independent carrier for chunk data so `haste-world` never depends on packet structs.
//! `haste-protocol` fills these from `level_chunk_with_light`; `haste-world` decodes the payload.
use bytes::Bytes;

use crate::{BlockPos, ChunkPos};

#[derive(Clone, Debug)]
pub struct ChunkData {
    pub pos: ChunkPos,
    /// Heightmaps as sent: (heightmap type id, packed longs).
    pub heightmaps: Vec<(i32, Vec<i64>)>,
    /// The raw section payload: for each section, block count (i16) + block paletted container +
    /// biome paletted container, in vanilla's wire layout. Decoding is `haste-world`'s job.
    pub sections: Bytes,
    pub block_entities: Vec<BlockEntityData>,
    pub light: LightData,
}

#[derive(Clone, Debug)]
pub struct BlockEntityData {
    pub pos: BlockPos,
    /// Index into the `minecraft:block_entity_type` registry.
    pub kind: u32,
    /// NBT payload as raw bytes (network NBT, unnamed root).
    pub nbt: Bytes,
}

/// Light arrays as sent in `level_chunk_with_light` and `light_update`. Bit sets index sections
/// from `min_section - 1` to `max_section + 1` (one extra on each end).
#[derive(Clone, Debug, Default)]
pub struct LightData {
    pub sky_mask: Vec<u64>,
    pub block_mask: Vec<u64>,
    pub empty_sky_mask: Vec<u64>,
    pub empty_block_mask: Vec<u64>,
    /// 2048-byte nibble arrays, one per set bit of `sky_mask`, in order.
    pub sky_arrays: Vec<Bytes>,
    pub block_arrays: Vec<Bytes>,
}

/// One block change from `block_update` or `section_blocks_update`.
#[derive(Clone, Copy, Debug)]
pub struct BlockChange {
    pub pos: BlockPos,
    pub state: crate::BlockStateId,
}
