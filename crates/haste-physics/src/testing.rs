//! Tiny in-memory `BlockView` + `CollisionShapes` + `BlockPhysics` for tests and benchmarks.
//! Not used by the game; `haste-world`/`haste-data` provide the real implementations.
use std::collections::HashMap;

use haste_core::view::{BlockView, CollisionShapes};
use haste_core::{Aabb, BlockPos, BlockStateId, ChunkPos, DVec3};

use crate::{BlockPhysics, FluidInfo, FluidKind};

/// Hand-made block kinds with fixed state ids.
pub mod states {
    use haste_core::BlockStateId;
    pub const AIR: BlockStateId = BlockStateId(0);
    pub const STONE: BlockStateId = BlockStateId(1);
    pub const BOTTOM_SLAB: BlockStateId = BlockStateId(2);
    pub const TOP_SLAB: BlockStateId = BlockStateId(3);
    /// Bottom half plus the north (z < 0.5) top quarter, like a stair facing north.
    pub const STAIR_NORTH: BlockStateId = BlockStateId(4);
    /// 1.5 tall post, 0.375..0.625 horizontally, like a fence post.
    pub const FENCE_POST: BlockStateId = BlockStateId(5);
    pub const ICE: BlockStateId = BlockStateId(6);
    pub const BLUE_ICE: BlockStateId = BlockStateId(7);
    pub const SLIME: BlockStateId = BlockStateId(8);
    /// Soul sand: 0.875 tall, speed factor 0.4.
    pub const SOUL_SAND: BlockStateId = BlockStateId(9);
    pub const WATER_SOURCE: BlockStateId = BlockStateId(10);
    pub const LAVA_SOURCE: BlockStateId = BlockStateId(11);
// MERGE(jump-sprint side):
    /// Block whose top is at 0.3: a step-up candidate height vanilla's loop tries.
    pub const STEP_03: BlockStateId = BlockStateId(12);
    /// Block whose top is exactly 0.6 below a player standing at y = 0: the sneak edge probe
    /// (`Player.canFallAtLeast`, depth step + 1e-7) reaches it.
    pub const LEDGE_0_6: BlockStateId = BlockStateId(13);
    pub const COUNT: usize = 14;
// MERGE(main side):
    /// No collision; sticks entities at (0.25, 0.05, 0.25) (WebBlock.entityInside).
    pub const COBWEB: BlockStateId = BlockStateId(12);
    /// No collision; sticks entities whose in-block state is powder snow at (0.9, 1.5, 0.9).
    pub const POWDER_SNOW: BlockStateId = BlockStateId(13);
    /// No collision; slows living entities at (0.8, 0.75, 0.8).
    pub const SWEET_BERRY_BUSH: BlockStateId = BlockStateId(14);
    /// No collision in these tests; slide slowdown on its sides.
    pub const HONEY: BlockStateId = BlockStateId(15);
    /// Water with a unit east flow vector (for water current tests).
    pub const FLOWING_WATER: BlockStateId = BlockStateId(16);
    /// Water with a small east flow vector (minimum-impulse test).
    pub const FLOWING_WATER_SLOW: BlockStateId = BlockStateId(17);
    pub const COUNT: usize = 18;
// MERGE end
}

/// Shapes and per-block physics properties for the ids in [`states`].
pub struct TestShapes {
    boxes: Vec<Vec<Aabb>>,
}

fn bx(x0: f64, y0: f64, z0: f64, x1: f64, y1: f64, z1: f64) -> Aabb {
    Aabb {
        min: DVec3::new(x0, y0, z0),
        max: DVec3::new(x1, y1, z1),
    }
}

impl Default for TestShapes {
    fn default() -> Self {
        let full = vec![bx(0.0, 0.0, 0.0, 1.0, 1.0, 1.0)];
        let mut boxes = vec![Vec::new(); states::COUNT];
        boxes[states::STONE.0 as usize] = full.clone();
        boxes[states::BOTTOM_SLAB.0 as usize] = vec![bx(0.0, 0.0, 0.0, 1.0, 0.5, 1.0)];
        boxes[states::TOP_SLAB.0 as usize] = vec![bx(0.0, 0.5, 0.0, 1.0, 1.0, 1.0)];
        boxes[states::STAIR_NORTH.0 as usize] = vec![
            bx(0.0, 0.0, 0.0, 1.0, 0.5, 1.0),
            bx(0.0, 0.5, 0.0, 1.0, 1.0, 0.5),
        ];
        boxes[states::FENCE_POST.0 as usize] = vec![bx(0.375, 0.0, 0.375, 0.625, 1.5, 0.625)];
        boxes[states::ICE.0 as usize] = full.clone();
        boxes[states::BLUE_ICE.0 as usize] = full.clone();
        boxes[states::SLIME.0 as usize] = full.clone();
        boxes[states::SOUL_SAND.0 as usize] = vec![bx(0.0, 0.0, 0.0, 1.0, 0.875, 1.0)];
// MERGE(jump-sprint side):
        boxes[states::STEP_03.0 as usize] = vec![bx(0.0, -0.7, 0.0, 1.0, 0.3, 1.0)];
        boxes[states::LEDGE_0_6.0 as usize] = vec![bx(0.0, -1.6, 0.0, 1.0, -0.6, 1.0)];
// MERGE(main side):
        // Cobweb, powder snow, sweet berry bush and honey have no collision in these tests.
// MERGE end
        Self { boxes }
    }
}

impl CollisionShapes for TestShapes {
    fn collision_boxes(&self, state: BlockStateId) -> &[Aabb] {
        self.boxes
            .get(state.0 as usize)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
    fn outline_boxes(&self, state: BlockStateId) -> &[Aabb] {
        self.collision_boxes(state)
    }
}

/// Values from `cache/oracle/26.3/block_properties.json` for the matching vanilla blocks.
impl BlockPhysics for TestShapes {
    fn friction(&self, s: BlockStateId) -> f32 {
        match s {
            states::ICE => 0.98,
            states::BLUE_ICE => 0.989,
            states::SLIME => 0.8,
            _ => 0.6,
        }
    }
    fn light_emission(&self, s: BlockStateId) -> u8 {
        if s == states::LAVA_SOURCE {
            15
        } else {
            0
        }
    }
    fn is_air(&self, s: BlockStateId) -> bool {
        s == states::AIR
    }
    fn is_opaque_full(&self, s: BlockStateId) -> bool {
        matches!(s, states::STONE | states::ICE | states::BLUE_ICE)
    }
    fn speed_factor(&self, s: BlockStateId) -> f32 {
        match s {
            states::SOUL_SAND => 0.4,
            states::HONEY => 0.4,
            _ => 1.0,
        }
    }
    fn jump_factor(&self, s: BlockStateId) -> f32 {
        if s == states::HONEY {
            0.5
        } else {
            1.0
        }
    }
    fn fluid(&self, s: BlockStateId) -> Option<FluidInfo> {
        match s {
            states::WATER_SOURCE => Some(FluidInfo {
                kind: FluidKind::Water,
                height: 8.0 / 9.0,
            }),
            states::FLOWING_WATER | states::FLOWING_WATER_SLOW => Some(FluidInfo {
                kind: FluidKind::Water,
                height: 8.0 / 9.0,
            }),
            states::LAVA_SOURCE => Some(FluidInfo {
                kind: FluidKind::Lava,
                height: 8.0 / 9.0,
            }),
            _ => None,
        }
    }
    fn is_cobweb(&self, s: BlockStateId) -> bool {
        s == states::COBWEB
    }
    fn is_powder_snow(&self, s: BlockStateId) -> bool {
        s == states::POWDER_SNOW
    }
    fn is_sweet_berry_bush(&self, s: BlockStateId) -> bool {
        s == states::SWEET_BERRY_BUSH
    }
    fn is_slime(&self, s: BlockStateId) -> bool {
        s == states::SLIME
    }
    fn is_honey(&self, s: BlockStateId) -> bool {
        s == states::HONEY
    }
    fn bounce_restitution(&self, s: BlockStateId) -> f32 {
        if s == states::SLIME {
            1.0
        } else {
            0.0
        }
    }
    fn water_flow(
        &self,
        _world: &dyn haste_core::view::BlockView,
        _pos: haste_core::BlockPos,
        s: BlockStateId,
    ) -> Option<DVec3> {
        match s {
            states::FLOWING_WATER => Some(DVec3::new(1.0, 0.0, 0.0)),
            states::FLOWING_WATER_SLOW => Some(DVec3::new(0.5, 0.0, 0.0)),
            _ => None,
        }
    }
}

/// Sparse block map plus an optional infinite floor layer.
#[derive(Default, Clone)]
pub struct TestWorld {
    pub blocks: HashMap<BlockPos, BlockStateId>,
    /// Every block with `y <= floor_y` is `floor_state` (if set).
    pub floor: Option<(i32, BlockStateId)>,
}

impl TestWorld {
    pub fn new() -> Self {
        Self::default()
    }
    /// Infinite flat floor whose top surface is at `top_y` (blocks at y < top_y are `state`).
    pub fn flat(top_y: i32, state: BlockStateId) -> Self {
        Self {
            blocks: HashMap::new(),
            floor: Some((top_y - 1, state)),
        }
    }
    pub fn set(&mut self, x: i32, y: i32, z: i32, s: BlockStateId) -> &mut Self {
        self.blocks.insert(BlockPos::new(x, y, z), s);
        self
    }
    pub fn fill(&mut self, a: (i32, i32, i32), b: (i32, i32, i32), s: BlockStateId) -> &mut Self {
        for x in a.0.min(b.0)..=a.0.max(b.0) {
            for y in a.1.min(b.1)..=a.1.max(b.1) {
                for z in a.2.min(b.2)..=a.2.max(b.2) {
                    self.set(x, y, z, s);
                }
            }
        }
        self
    }
}

impl BlockView for TestWorld {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        if let Some(s) = self.blocks.get(&pos) {
            return *s;
        }
        match self.floor {
            Some((fy, s)) if pos.y <= fy => s,
            _ => BlockStateId::AIR,
        }
    }
    fn is_chunk_loaded(&self, _pos: ChunkPos) -> bool {
        true
    }
    fn sky_light(&self, _pos: BlockPos) -> u8 {
        15
    }
    fn block_light(&self, _pos: BlockPos) -> u8 {
        0
    }
    fn min_y(&self) -> i32 {
        -64
    }
    fn height(&self) -> i32 {
        384
    }
}
