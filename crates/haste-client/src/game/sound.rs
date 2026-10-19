//! The tick's sound steps (vanilla `SoundEngine`): the packet-driven and player-made sound
//! requests played through the output, and the listener tick that follows the camera.

use glam::DVec3;

use super::camera::{third_person_camera, Perspective};
use super::shared::{Shared, SharedInput};
use crate::net::NetDriver;

/// This tick's sound packets, played through the output (when the client set one up).
pub(super) fn play_packet_sounds(driver: &mut NetDriver, shared: &Shared) {
    let requests = &mut driver.session.sound_requests;
    let mut sound = shared.sound.lock();
    if let Some(sounds) = sound.as_mut() {
        // Entity positions for `sound_entity`, from the store the packets just updated.
        let entities = shared.entities.read();
        sounds.apply(requests, |id| {
            entities
                .get(haste_protocol::haste_core::EntityId(id))
                .map(|e| e.pos)
        });
    }
}

/// The player's own block sounds this tick (footsteps, mining, placing), played through
/// the same output the packets went through above.
pub(super) fn play_own_sounds(driver: &mut NetDriver, shared: &Shared) {
    let requests = &mut driver.session.sound_requests;
    let mut sound = shared.sound.lock();
    if let Some(sounds) = sound.as_mut() {
        sounds.apply(requests, |_| None);
    }
}

/// The sound output's tick: listener from the camera (vanilla's
/// `SoundEngine.updateSource(Camera)` — the third-person perspectives move it off the
/// eye), the camera's music volume target (vanilla `Minecraft.getMusicVolume`, resolved
/// even with the output off so a biome crossing shows in the debug log), music situation,
/// then 20 Hz tick.
pub(super) fn tick_listener(
    driver: &mut NetDriver,
    shared: &Shared,
    input: &SharedInput,
    eye: DVec3,
    yaw: f32,
    pitch: f32,
) {
    let listener = if input.perspective == Perspective::FirstPerson {
        (eye, yaw, pitch)
    } else {
        let (world, shapes) = (shared.world.read(), &shared.data.shapes);
        third_person_camera(input.perspective, eye, yaw, pitch, &*world, shapes)
    };
    // The camera's `MUSIC_VOLUME` (see `biomes::audio`): the dimension type's value
    // overridden by the biome at the camera, logged on change (the target itself is
    // instant; the music manager's fade eases the gain toward it).
    let target = {
        let world = shared.world.read();
        crate::app::biomes::audio::camera_music_volume(
            &world,
            &driver.session.biome_audio,
            listener.0,
            driver.session.dimension_music_volume,
        )
    };
    let s = &mut driver.session;
    if target != s.music_volume {
        tracing::info!("music volume target {} -> {}", s.music_volume, target);
        s.music_volume = target;
    }
    let mut sound = shared.sound.lock();
    if let Some(sounds) = sound.as_mut() {
        sounds.set_music_volume(target);
        let situation =
            crate::sound::music_situation(s.player.game_mode, s.player.state.eyes_in_water);
        sounds.tick(listener.0, listener.1, listener.2, situation);
    }
}
