//! The tick's level step (vanilla `Minecraft.tick` -> `ClientLevel.tick`): the screen-effects
//! probe, the entities' and the player's tick (`end_tick`), the weather effects and the dirty
//! sections. Returns the horizontal distance the player moved this tick, for the hand's walk
//! bob.

use std::sync::atomic::Ordering;

use haste_core::view::DirtyTracker;

use super::shared::{Shared, SharedInput};
use super::weather;
use crate::net::NetDriver;

pub(super) fn tick(
    driver: &mut NetDriver,
    shared: &Shared,
    input: &SharedInput,
) -> anyhow::Result<f32> {
    let tick_start = driver.session.player.state.pos;
    // Write lock: the player's breaking predicts block changes into the world.
    let mut world = shared.world.write();
    let mut entities = shared.entities.write();
    // The world probe for the screen effects (vanilla's
    // `NetherPortalBlock.entityInside` and `LevelExtractor.getViewBlockingState`): the
    // portal under our feet feeds the portal effect's tick below, and the block the
    // camera is inside plus the eye's light go into this tick's view.
    {
        let p = &driver.session.player;
        let probe = crate::screen_effects::Probe {
            world: &*world,
            shapes: &shared.data.shapes,
            blocks: &shared.data.blocks,
            tags: p.int.block_tags(),
            eye: p.camera(1.0).0,
            feet: p.state.pos,
            bb_width: p.attrs.box_width(),
            bb_height: p.attrs.height_for(p.state.pose),
            scale: f64::from(p.attrs.scale),
            spectator: p.game_mode == haste_core::game::GameMode::Spectator,
        }
        .sample();
        driver.session.screen.set_inside_portal(probe.in_portal);
        // The local player's freeze ticks are client-predicted (the server does not send the
        // owner their own metadata): powder snow adds one, fire clears them.
        driver.session.player.own_flags.frozen_ticks = crate::screen_effects::tick_frozen(
            driver.session.player.own_flags.frozen_ticks,
            probe.in_powder_snow,
            probe.in_fire,
        );
        driver.session.screen_probe = probe;
    }
    let ui = shared.ui.lock().clone();
    driver.end_tick(&input.state, &mut *world, &mut *entities, ui.as_ref())?;
    // The remote entities' use effects (vanilla's entities tick before the level's
    // particles spawn, `ClientLevel.tick`): the use steps the entity tick just recorded,
    // run through the same consume path the local player's own use takes.
    driver
        .session
        .apply_remote_use(entities.use_ticks(), &shared.data);
    weather::tick(driver, &world, shared);
    let mut dirty = world.take_dirty_sections();
    if !dirty.is_empty() {
        shared.dirty.lock().append(&mut dirty);
        shared.world_version.fetch_add(1, Ordering::Relaxed);
    }
    let moved = driver.session.player.state.pos - tick_start;
    Ok(((moved.x * moved.x + moved.z * moved.z).sqrt()) as f32)
}
