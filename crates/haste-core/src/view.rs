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

/// Per-block-state physics properties from the oracle dump `block_properties.json`
/// (friction, speed and jump factors) and the fluid a state carries. Implemented by `haste-data`,
/// consumed by `haste-physics`.
pub trait BlockPhysics {
    /// Vanilla "slipperiness": 0.6 for most blocks, 0.98 ice, 0.989 blue ice, 0.8 slime.
    fn friction(&self, state: BlockStateId) -> f32;
    fn speed_factor(&self, state: BlockStateId) -> f32;
    fn jump_factor(&self, state: BlockStateId) -> f32;
    fn fluid(&self, state: BlockStateId) -> Option<FluidInfo>;
    /// Light emitted by the state, 0..=15.
    fn light_emission(&self, state: BlockStateId) -> u8;
    fn is_air(&self, state: BlockStateId) -> bool;
    /// Opaque full cube (vanilla `isSolidRender`): occludes neighbours' faces and light.
    fn is_opaque_full(&self, state: BlockStateId) -> bool;
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FluidKind {
    Water,
    Lava,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FluidInfo {
    pub kind: FluidKind,
    /// Fluid surface height within the block, 0..=1 (level 0 source = 8/9 like vanilla).
    pub height: f32,
}

/// The block-state table as `haste-data` loads it from `blocks.json`; consumed by `haste-assets`
/// (model baking) and anything else that needs names without depending on `haste-data`.
pub trait BlockStateTable {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The placeholder state for ids outside the table: vanilla has no "missing block",
    /// so tables resolve out-of-range ids (modded servers) to this — haste-data uses
    /// `minecraft:stone`'s default state. Every lookup by state id must be total.
    fn placeholder_state(&self) -> BlockStateId;
    fn block_of(&self, state: BlockStateId) -> &crate::Identifier;
    /// Property name/value pairs, sorted by name (owned: implementations intern names and values).
    /// Startup/baking use only; hot paths use `property_value`.
    fn properties(&self, state: BlockStateId) -> Vec<(String, String)>;
    fn property_value(&self, state: BlockStateId, name: &str) -> Option<&str>;
}
