//! Read-only views over world state. `haste-world` implements them; meshing (`haste-render`),
//! physics (`haste-physics`) and entities consume them. Implementations must be cheap to call in
//! tight loops: no allocation per call, no locking per block if avoidable (take a section snapshot).
use crate::{Aabb, BlockPos, BlockStateId, ChunkPos, SectionPos};

pub trait BlockView {
    /// `BlockStateId::AIR` for unloaded chunks and out-of-range y.
    fn block_state(&self, pos: BlockPos) -> BlockStateId;
    fn is_chunk_loaded(&self, pos: ChunkPos) -> bool;
    /// 0..=15. Sky light is 15 where there is no skylight data (e.g. nether).
    fn sky_light(&self, pos: BlockPos) -> u8;
    fn block_light(&self, pos: BlockPos) -> u8;
    fn min_y(&self) -> i32;
    fn height(&self) -> i32;
}

/// Collision geometry per block state. Backed by the oracle dump `block_shapes.json` in the cache
/// (recorded from the real game by the recorder mod), with a conservative fallback when absent.
pub trait CollisionShapes {
    /// Boxes in block-local coordinates (0..=1). Empty for air/non-colliding blocks.
    fn collision_boxes(&self, state: BlockStateId) -> &[Aabb];
    /// Outline/selection boxes (for the block highlight), same convention.
    fn outline_boxes(&self, state: BlockStateId) -> &[Aabb];
}

/// Which sections changed since the last frame, for incremental remeshing.
pub trait DirtyTracker {
    fn take_dirty_sections(&mut self) -> Vec<SectionPos>;
}
