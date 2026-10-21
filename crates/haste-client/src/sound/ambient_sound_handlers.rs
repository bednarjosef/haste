//! The local player's ambient sound handlers (vanilla `LocalPlayer.ambientSoundHandlers`,
//! each an `AmbientSoundHandler`), and the underwater enter/exit sounds of
//! `LocalPlayer.updateIsUnderwater` that start the underwater loop.
//!
//! Vanilla's order is kept: `updateIsUnderwater` runs in the player's `baseTick`, then
//! `LocalPlayer.tick` ends by ticking the handlers in the order the constructor adds them
//! (underwater, bubble column, biome). The handlers draw from the level's random; the
//! instances they start tick on the sound tick ([`super::ticking_sounds`]). A respawn
//! builds a new `LocalPlayer`, so [`AmbientSoundHandlers::respawn`] starts them over.
//!
//! `BubbleColumnAmbientSoundHandler` is not ported here: it plays a one-shot entity sound
//! on entering a column and changes none of the loops; its seam is the slot between the
//! two handlers in [`AmbientSoundHandlers::tick`].

use glam::DVec3;
use haste_audio::engine::SoundSource;
use haste_audio::output::InstancePlay;
use haste_audio::random::LegacyRandom;
use haste_core::view::BlockView;
use haste_core::BlockPos;

use super::biome_ambient_sounds_handler::BiomeAmbientSoundsHandler;
use super::sound_manager::SoundManager;
use super::ticking_sounds::{TickableInstance, TickingPlayer, TickingSounds};
use super::under_liquid_ambient_sound_instance::UnderLiquidAmbientSoundInstance;
use super::underwater_ambient_sound_handler::UnderwaterAmbientSoundHandler;
use crate::app::biomes::ambient_sounds::AmbientSounds;

/// Vanilla `SoundEvents.AMBIENT_UNDERWATER_ENTER`.
pub const AMBIENT_UNDERWATER_ENTER: &str = "minecraft:ambient.underwater.enter";
/// Vanilla `SoundEvents.AMBIENT_UNDERWATER_EXIT`.
pub const AMBIENT_UNDERWATER_EXIT: &str = "minecraft:ambient.underwater.exit";

/// What the handlers read off the local player this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AmbientPlayer {
    /// `Entity.position()` (the feet).
    pub pos: DVec3,
    /// `Entity.getEyeY()`: the feet plus the pose's eye height (not the camera's
    /// smoothed one).
    pub eye_y: f64,
    /// `LocalPlayer.isUnderWater` (the eyes in water, as of this tick's `baseTick`).
    pub under_water: bool,
    pub spectator: bool,
}

/// Vanilla `LightLayer`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightLayer {
    Sky,
    Block,
}

/// Vanilla `Level.getBrightness(LightLayer, BlockPos)`: the stored light, 0..=15.
pub trait LevelBrightness {
    fn brightness(&self, layer: LightLayer, pos: BlockPos) -> u8;
}

/// The client world's light. A dimension without skylight reads 0 sky light everywhere,
/// as vanilla's dummy sky light engine answers (the world's own fallback of 15 there is
/// for rendering).
pub struct WorldBrightness<'a> {
    pub world: &'a dyn BlockView,
    pub has_skylight: bool,
}

impl LevelBrightness for WorldBrightness<'_> {
    fn brightness(&self, layer: LightLayer, pos: BlockPos) -> u8 {
        match layer {
            LightLayer::Sky if !self.has_skylight => 0,
            LightLayer::Sky => self.world.sky_light(pos),
            LightLayer::Block => self.world.block_light(pos),
        }
    }
}

/// What the handlers read off the level this tick.
pub struct AmbientLevel<'a> {
    /// `EnvironmentAttributes.AMBIENT_SOUNDS` at the player's position.
    pub sounds: &'a AmbientSounds,
    pub light: &'a dyn LevelBrightness,
}

/// The local player's handlers and the state `updateIsUnderwater` compares against.
#[derive(Debug)]
pub struct AmbientSoundHandlers {
    /// Which local player these belong to; a respawn makes the next one.
    player: u32,
    /// `wasUnderwater` as of the last tick (a new player starts out of water).
    was_under_water: bool,
    /// The level's random (`player.level().getRandom()`), shared by the handlers and the
    /// enter/exit sounds' seeds.
    level_random: LegacyRandom,
    underwater: UnderwaterAmbientSoundHandler,
    biome: BiomeAmbientSoundsHandler,
}

impl Default for AmbientSoundHandlers {
    fn default() -> Self {
        Self::new(super::own_block::unseeded())
    }
}

impl AmbientSoundHandlers {
    /// The handlers of a new player, the level's random seeded with `seed`.
    pub fn new(seed: i64) -> Self {
        Self {
            player: 0,
            was_under_water: false,
            level_random: LegacyRandom::new(seed),
            underwater: UnderwaterAmbientSoundHandler::default(),
            biome: BiomeAmbientSoundsHandler::default(),
        }
    }

    /// What the sound tick's instances read off the player.
    pub fn ticking_player(&self, under_water: bool) -> TickingPlayer {
        TickingPlayer {
            player: self.player,
            under_water,
        }
    }

    /// `LocalPlayer.getCurrentMood` (the seam the debug overlay's mood line reads; no
    /// Haste overlay shows it yet).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn moodiness(&self) -> f32 {
        self.biome.moodiness()
    }

    /// A respawn's new `LocalPlayer`: fresh handlers and underwater state. The old
    /// player's underwater sounds stop on their next tick (their player is removed); the
    /// old biome handler's loops are released and keep playing as they were, as vanilla's
    /// do when their handler goes. A dimension change also stops every sound
    /// (`Minecraft.updateLevelInEngines`), on the next tick that has the output.
    pub fn respawn(&mut self, ticking: &mut TickingSounds, dimension_changed: bool) {
        self.biome.release_loops(ticking);
        self.player = self.player.wrapping_add(1);
        self.was_under_water = false;
        self.underwater = UnderwaterAmbientSoundHandler::default();
        self.biome = BiomeAmbientSoundsHandler::default();
        if dimension_changed {
            ticking.request_stop_all();
        }
    }

    /// The player's ambient steps for this tick, in vanilla's order: the underwater
    /// transition sounds (`updateIsUnderwater`, in `baseTick`), then each handler.
    pub fn tick(
        &mut self,
        player: &AmbientPlayer,
        level: &AmbientLevel<'_>,
        ticking: &mut TickingSounds,
        out: &mut dyn SoundManager,
    ) {
        ticking.flush_stop_all(out);
        self.update_is_underwater(player, ticking, out);
        self.underwater
            .tick(self.player, player, &mut self.level_random, ticking, out);
        // `BubbleColumnAmbientSoundHandler.tick` goes here (not ported, see the module).
        self.biome
            .tick(player, level, &mut self.level_random, ticking, out);
    }

    /// `LocalPlayer.updateIsUnderwater`'s sounds: going under plays the enter sound at
    /// the feet and starts the underwater loop; coming up plays the exit sound. A
    /// spectator's state follows along silently.
    fn update_is_underwater(
        &mut self,
        player: &AmbientPlayer,
        ticking: &mut TickingSounds,
        out: &mut dyn SoundManager,
    ) {
        let was = std::mem::replace(&mut self.was_under_water, player.under_water);
        if player.spectator || was == player.under_water {
            return;
        }
        // `ClientLevel.playLocalSound(x, y, z, ...)`: ambient, volume and pitch 1, at the
        // feet, seeded from the level's random.
        let event = if player.under_water {
            AMBIENT_UNDERWATER_ENTER
        } else {
            AMBIENT_UNDERWATER_EXIT
        };
        out.play(&InstancePlay {
            event,
            source: SoundSource::Ambient,
            pos: Some(player.pos),
            volume: 1.0,
            pitch: 1.0,
            seed: self.level_random.next_long(),
            looping: false,
        });
        if player.under_water {
            let instance = UnderLiquidAmbientSoundInstance::underwater(self.player);
            ticking.play(TickableInstance::UnderLiquidAmbient(instance), false, out);
        }
    }
}

#[cfg(test)]
mod tests;
