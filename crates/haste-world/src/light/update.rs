//! The client light update for one changed block, vanilla `LevelLightEngine.checkBlock` +
//! `runLightUpdates` (which the vanilla client runs from `ClientLevel.update` every frame
//! after `LevelChunk.setBlockState` queued the block). Haste runs it inline from
//! `World::set_block`, so the light is right in the same frame; the server's `light_update`
//! for border chunks later overwrites the same values.

use haste_core::view::BlockPhysics;
use haste_core::{BlockPos, BlockStateId, SectionPos};
use haste_data::GameData;

use crate::world::World;

use super::block_light;
use super::engine::{self, Queues};
use super::sky_light_engine;
use super::sky_sources;
use super::storage::{light_dampening, Layer, Storage};

/// Vanilla `LightEngine.hasDifferentLightProperties`: whether a state change can change
/// light at all (dampening or emission differ, or either state's shape can occlude light —
/// the flag is `None` on old dumps, which keep the pre-shape approximation).
pub(crate) fn light_properties_differ(
    data: &GameData,
    old: BlockStateId,
    new: BlockStateId,
) -> bool {
    new != old
        && (light_dampening(data, old) != light_dampening(data, new)
            || data.light_emission(old) != data.light_emission(new)
            || data.shapes.use_shape_for_light_occlusion(new) == Some(true)
            || data.shapes.use_shape_for_light_occlusion(old) == Some(true))
}

/// Recomputes the light for a changed block (the world already holds the new state) and
/// re-meshes every section whose light changed, plus their 3x3x3 neighbourhoods (vanilla
/// marks each written cell's surrounding sections, `setStoredLevel`'s
/// `aroundAndAtBlockPos` collected in `swapSectionMap` -> `onLightUpdate`).
pub(crate) fn run_for_block(world: &mut World, pos: BlockPos) {
    // vanilla LevelChunk.setBlockState updates the column's sky sources before checking
    sky_sources::init(world, pos.chunk());
    sky_sources::update_column(world, pos);
    let node = engine::pack(pos.x, pos.y, pos.z);
    let mut store = Storage::new(world);
    let mut block = Queues::new();
    let mut sky = Queues::new();
    block_light::check_node(&mut store, &mut block, node);
    if store.has_skylight() {
        sky_light_engine::check_node(&mut store, &mut sky, node);
    }
    engine::run(&mut store, &mut block, Layer::Block);
    if store.has_skylight() {
        engine::run(&mut store, &mut sky, Layer::Sky);
    }
    let written = store.finish();
    for sec in written {
        for dy in -1..=1i32 {
            for dx in -1..=1i32 {
                for dz in -1..=1i32 {
                    world.mark_dirty_section(SectionPos::new(sec.x + dx, sec.y + dy, sec.z + dz));
                }
            }
        }
    }
}
