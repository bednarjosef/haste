//! Game thread (client-app): the 20 Hz tick that drains the connection into the world, the
//! entity store and the session, and sends this tick's packets. The render thread reads the
//! same state through [`Shared`].
//!
//! Locking: the world and the entity store each sit behind a `parking_lot::RwLock`. The game
//! thread takes the write locks once per tick for the packet drain (never per packet); the
//! render thread takes read locks only to capture dirty sections for the mesher and to copy the
//! entity render instances, never across `Renderer::render`. Everything else the render thread
//! needs per frame is a small `Copy` value in [`Shared::view`].

use std::sync::Arc;
#[cfg(test)]
use std::time::Duration;
use std::time::Instant;

use glam::DVec3;
#[cfg(test)]
use haste_protocol::haste_core::game::DimensionInfo;
use haste_protocol::haste_core::game::TICK_SECONDS;
#[cfg(test)]
use haste_protocol::haste_core::RegistryIndex;
use haste_protocol::haste_core::SectionPos;
#[cfg(test)]
use haste_protocol::haste_core::{BlockPos, BlockStateId, ChunkPos};

mod block_entities;
mod block_entity_ticks;
mod camera;
mod hud;
mod input;
pub(crate) mod item_uses;
mod level;
mod maps;
mod particles;
pub(crate) mod pickup;
mod player_model;
mod run;
mod settle_diet;
mod shared;
mod sound;
mod tracked_world;
pub(crate) mod totem_emitter;
mod view;
mod weather;

use crate::net::NetDriver;
#[cfg(test)]
use crate::net::WorldSink;

pub mod skins;

pub use camera::{third_person_camera, Perspective};
pub use player_model::push_local_instance;
pub use run::run;
pub use shared::{PackReload, Shared};
#[cfg(test)]
pub use view::death_time;
pub use view::{FirstPersonView, ViewState};

#[cfg(test)]
use camera::THIRD_PERSON_DISTANCE;
#[cfg(test)]
use haste_core::view::DirtyTracker;
#[cfg(test)]
use haste_world::World;
#[cfg(test)]
use run::take_slot;
use run::{drain_packets, HudSlot};
#[cfg(test)]
use tracked_world::TrackedWorld;

/// Fraction of the current tick that has elapsed at `now`, in `0..=1`.
pub fn partial_tick(tick_at: Instant, now: Instant) -> f32 {
    let dt = now.saturating_duration_since(tick_at).as_secs_f64();
    (dt / TICK_SECONDS).clamp(0.0, 1.0) as f32
}

/// Position between the last two ticks (vanilla's `lerp(partialTick, old, new)`).
pub fn interpolate(prev: DVec3, cur: DVec3, partial: f32) -> DVec3 {
    prev.lerp(cur, partial as f64)
}

fn tick_once(
    driver: &mut NetDriver,
    shared: &Arc<Shared>,
    gone: &mut Vec<SectionPos>,
    jvm: &mut HudSlot,
    server_packs_version: &mut u64,
    extractor: &mut crate::app::block_entity_extract::BlockEntityExtractor,
) -> anyhow::Result<()> {
    // Once the JVM arrives (the session joined before the game thread started): build the
    // `Ui` and give the JVM the registries, tags and the local player. Menu mode's UI arrives
    // pre-built (its JVM opened the title screen); only `joined` runs for it.
    hud::take_arrival(driver, shared, jvm);
    let shared: &Shared = shared;
    // This tick's UI output (packets vanilla code sent, chat and commands typed).
    let mut ui_out = Vec::new();
    drain_packets(driver, shared, gone, server_packs_version)?;
    // This tick's sound packets, played through the output (when the client set one up).
    // This runs again after `end_tick` below to pick up the player's own block sounds
    // (footsteps, mining, placing) that the player's tick just produced.
    sound::play_packet_sounds(driver, shared);
    // This tick's input, sampled from winit by the render thread.
    let input = input::sample(driver, shared);
    // The horizontal distance travelled this tick, for the hand's walk bob: vanilla's
    // `LocalPlayer.move` adds `length(dx, dz) * 0.6` to `ClientAvatarState.walkDist`.
    let tick_moved = level::tick(driver, shared, &input)?;
    // The pickup particles the tick produced or advanced, published to the render thread.
    pickup::publish(driver, shared);
    // The maps' data, published when a map's packets changed it.
    maps::publish(driver, shared);
    // The player's own block sounds this tick (footsteps, mining, placing), played through
    // the same output the packets went through above.
    sound::play_own_sounds(driver, shared);
    {
        let mut breaking = shared.breaking.lock();
        breaking.clear();
        driver.session.breaking_stages(&mut breaking);
    }
    // The lids' open targets (vanilla routes `block_event` to the block entity).
    block_entities::tick_lids(driver, shared);
    // The skulls' animation counters (vanilla ticks each `SkullBlockEntity.animation`:
    // while the block state is powered the counter grows, powering the jaw/ear swing).
    block_entities::tick_skulls_and_model_anims(driver, shared);
    // The vaults' and spawners' spins (vanilla `VaultClientData.updateDisplayItemSpin` and
    // the spawners' client tick): every vault's displayed item turns 10 degrees a tick, a
    // spawner's display mob turns at the rate its state's spinning mob speed and its remaining
    // spawn delay give (a plain mob spawner only while a player is near it).
    block_entities::tick_spins(driver, shared);
    // Particles: spawn what the packets brought, then one 20 Hz simulation step. The camera
    // for `ClientLevel.doAddParticle`'s 32-block limiter is the player's eye.
    particles::tick(driver, shared);
    hud::tick_ui(driver, shared, &mut ui_out);
    // The per-tick camera effects (FOV modifier smoothing and the hurt and death counters;
    // the render thread interpolates them per frame). The view bobbing's walk distance and
    // bob are the hand's own, ticked below (vanilla's one `ClientAvatarState` serves both).
    driver.session.tick_camera(
        Some(shared.data.as_ref()),
        input.perspective == Perspective::FirstPerson,
    );
    // The lightmap's per-tick state (vanilla `GameRenderer.tick` + the effect durations).
    driver.session.lightmap.tick();
    // The first-person hand's tick, in vanilla's order (`LivingEntity.baseTick`, then
    // `Player.tick`, then `FirstPersonHandsAndItems.tick`, with the walk bob in `aiStep` before
    // them); the interaction step above ran the attacks and uses that started the swings. In
    // spectator the hand is not drawn at all (vanilla hides it there), so it does not advance.
    let first_person = view::tick_hand(driver, shared, tick_moved);
    hud::publish_attack_indicator(driver, shared);
    hud::publish_item_cooldowns(driver);
    let s = &driver.session;
    let (prev_eye, prev_yaw, prev_pitch) = s.player.camera(0.0);
    let (eye, yaw, pitch) = s.player.camera(1.0);
    // The local player's own model state, ticked from this tick's end state before the view
    // publishes it (the same animation path a remote player's takes).
    player_model::tick(s, shared, (prev_yaw, prev_pitch), (yaw, pitch));
    // The sound output's tick: listener from the camera (vanilla's
    // `SoundEngine.updateSource(Camera)` — the third-person perspectives move it off the
    // eye), music situation, then 20 Hz tick.
    sound::tick_listener(driver, shared, &input, eye, yaw, pitch);
    // The view update, the tick's last step: everything the render thread needs per frame is
    // copied into the view state here, bracketed around the tick.
    let feet = view::publish(
        driver,
        shared,
        first_person,
        (prev_eye, prev_yaw, prev_pitch),
        (eye, yaw, pitch),
    );
    // The block entities' render data for the next tick's frames (vanilla's per-tick
    // `extractRenderState`): everything that changes only per tick (block states, lights,
    // payloads, the anims' tick ends) is extracted here, off the render thread; the render
    // thread only folds the partial tick into the animatable fields. Runs after the view
    // update so the camera chunk, view radius and world age are this tick's.
    extractor.extract(
        shared,
        feet,
        shared.view.lock().view_distance,
        driver.session.world_age,
    );
    Ok(())
}

#[cfg(test)]
mod tests;
