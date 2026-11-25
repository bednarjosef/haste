// This file is derived from Pomme (https://github.com/josefbednar/pomme),
// pomme-client/src/renderer/chunk/mesher.rs, reused under GPL-3.0 and reworked
// for Haste. See docs/THIRD_PARTY.md.

use std::sync::Arc;

use haste_core::BlockStateId;

/// Per-state ambient-occlusion occluder flag (vanilla `getShadeBrightness <=
/// 0.2`): whether the state's collision shape is a full cube, minus the glass
/// family whose class overrides the brightness to 1.0. Also carries the raw
/// collision-full flag (vanilla `isCollisionShapeFullBlock`), which the
/// mesher's faceCubic test reads (glass is collision-full but bright). Built
/// from `haste_data::BlockShapes`; the fallback treats every state as a full
/// cube except air and fluids (approximate like all fallback shapes).
#[derive(Clone)]
pub struct AoTable {
    solid: Vec<bool>,
    /// Raw full-collision-cube flag (no glass override).
    cubic: Vec<bool>,
}

impl AoTable {
    /// From haste-data's shape data (oracle-backed when the dump exists).
    pub fn from_shapes(
        states: &dyn haste_core::view::BlockStateTable,
        shapes: &haste_data::BlockShapes,
    ) -> AoTable {
        let mut solid = Vec::with_capacity(states.len());
        let mut cubic = Vec::with_capacity(states.len());
        for id in 0..states.len() as u32 {
            solid.push(shapes.is_ao_solid(BlockStateId(id)));
            cubic.push(shapes.is_collision_full(BlockStateId(id)));
        }
        AoTable { solid, cubic }
    }

    /// Fallback with no shape data (tests, before the game data is known):
    /// every in-range state occludes (stone-like), ids past the table read as
    /// non-occluding.
    pub fn all_solid(n: usize) -> AoTable {
        AoTable {
            solid: vec![true; n],
            cubic: vec![true; n],
        }
    }

    /// Whether `state` darkens its neighbours' AO. Out-of-table ids (modded
    /// servers) read as non-occluding rather than panicking; the placeholder
    /// resolution the other tables do needs the state table, which tests
    /// without one cannot provide, and an unknown block that renders should
    /// not darken either (vanilla resolves it to stone, but the difference is
    /// one AO corner on an unseen block).
    pub fn is_solid(&self, state: BlockStateId) -> bool {
        self.solid.get(state.0 as usize).copied().unwrap_or(false)
    }

    /// Raw full-collision-cube flag; out-of-table ids read as non-cubic (the
    /// faceCubic test then falls back to the quad-geometry half alone).
    pub fn is_cubic(&self, state: BlockStateId) -> bool {
        self.cubic.get(state.0 as usize).copied().unwrap_or(false)
    }
}

/// Per-block-state and per-biome tables the mesher reads (shared with the workers).
#[derive(Clone)]
pub struct MeshTables {
    pub tints: Arc<crate::tint::TintTable>,
    pub colors: Arc<crate::tint::BiomeColors>,
    pub fluids: Arc<crate::fluid::FluidTable>,
    /// AO occluder test per state (vanilla collision-shape rule).
    pub ao: Arc<AoTable>,
    /// Fluid face-occlusion shapes per state (vanilla occlusion shape).
    pub fluid_occluders: Arc<crate::fluid::FluidOccluders>,
    /// Terrain face-occlusion shapes per state (vanilla `Block.shouldRenderFace`'s
    /// shape comparison; `None` per state keeps the model rule in the mesher).
    pub face_occluders: Arc<crate::mesher::face_occlusion::FaceOccluders>,
    /// Face-skip rules per state (vanilla `BlockBehaviour.skipRendering`).
    pub skip: Arc<crate::mesher::skip::SkipTable>,
    /// Set identity for the mesher's per-state cache: a fresh value per
    /// constructed set (monotonic counter), so a new Renderer's tables
    /// never alias a previous set's pointer identities after a reconnect.
    pub generation: u64,
}

/// Monotonic source of `MeshTables::generation`: every construction takes
/// the next value, so two sets are never the same cache generation even when
/// the allocator reuses a freed set's addresses.
static TABLE_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl MeshTables {
    pub fn new(
        tints: Arc<crate::tint::TintTable>,
        colors: Arc<crate::tint::BiomeColors>,
        fluids: Arc<crate::fluid::FluidTable>,
        ao: Arc<AoTable>,
        fluid_occluders: Arc<crate::fluid::FluidOccluders>,
        face_occluders: Arc<crate::mesher::face_occlusion::FaceOccluders>,
        skip: Arc<crate::mesher::skip::SkipTable>,
    ) -> MeshTables {
        MeshTables {
            tints,
            colors,
            fluids,
            ao,
            fluid_occluders,
            face_occluders,
            skip,
            generation: TABLE_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }
}

impl Default for MeshTables {
    fn default() -> Self {
        MeshTables::new(
            Arc::new(crate::tint::TintTable::empty()),
            Arc::new(crate::tint::BiomeColors::default()),
            Arc::new(crate::fluid::FluidTable::empty()),
            // No shape data known: no state occludes AO (flat lighting).
            Arc::new(AoTable::all_solid(0)),
            // No shape data known: fluid faces fall back to full-if-opaque.
            Arc::new(crate::fluid::FluidOccluders::absent()),
            // No shape data known: terrain faces fall back to the model rule.
            Arc::new(crate::mesher::face_occlusion::FaceOccluders::absent()),
            // No class dump known: no face skips (the mesher draws every face).
            Arc::new(crate::mesher::skip::SkipTable::absent()),
        )
    }
}
