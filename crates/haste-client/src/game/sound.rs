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

/// The local player's ambient sound handlers (vanilla `LocalPlayer.tick` ends by ticking
/// them, after `baseTick`'s `updateIsUnderwater`), run once the player's tick ran: only
/// while the player ticks at all (`hasClientLoaded`). They read the player's position,
/// eye and water state this tick, the `audio/ambient_sounds` attribute at the feet and
/// the light around the eye; what they start plays through the output (nothing without
/// one, the state still advances).
pub(super) fn tick_ambient_handlers(driver: &mut NetDriver, shared: &Shared) {
    let s = &mut driver.session;
    let mut sound = shared.sound.lock();
    let mut silent = crate::sound::NoSoundManager;
    let out: &mut dyn crate::sound::SoundManager = match sound.as_mut() {
        Some(sounds) => sounds,
        None => &mut silent,
    };
    if !(s.player.loaded && s.player.has_position) {
        // The player does not tick; a pending level-change stop still goes out.
        s.ticking_sounds.flush_stop_all(out);
        return;
    }
    let p = &s.player;
    let player = crate::sound::AmbientPlayer {
        pos: p.state.pos,
        eye_y: p.state.pos.y + p.eye_height(),
        under_water: p.state.eyes_in_water,
        spectator: p.game_mode == haste_core::game::GameMode::Spectator,
    };
    let world = shared.world.read();
    let light = crate::sound::WorldBrightness {
        world: &*world,
        has_skylight: world.dimension().has_skylight,
    };
    let level = crate::sound::AmbientLevel {
        sounds: s.ambient_sounds_layers.at(&world, player.pos),
        light: &light,
    };
    s.ambient_sound_handlers
        .tick(&player, &level, &mut s.ticking_sounds, out);
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
    // The ticking sound instances (vanilla `SoundEngine.tickInGameSound`): fades,
    // self-stops and played-through sounds, with the player's water state this tick.
    let ticking_player = s
        .ambient_sound_handlers
        .ticking_player(s.player.state.eyes_in_water);
    let mut sound = shared.sound.lock();
    match sound.as_mut() {
        Some(sounds) => s.ticking_sounds.tick(&ticking_player, sounds),
        None => s
            .ticking_sounds
            .tick(&ticking_player, &mut crate::sound::NoSoundManager),
    }
    if let Some(sounds) = sound.as_mut() {
        sounds.set_music_volume(target);
        let situation =
            crate::sound::music_situation(s.player.game_mode, s.player.state.eyes_in_water);
        sounds.tick(listener.0, listener.1, listener.2, situation);
        // The subtitled sounds this tick played, to the subtitle overlay (vanilla
        // `SoundEngine.notifyListeners`, batched per tick).
        sounds.publish_played_sounds();
    }
}
