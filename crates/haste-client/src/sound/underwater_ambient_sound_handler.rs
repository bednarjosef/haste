//! Vanilla `UnderwaterAmbientSoundHandler`: while the player is under water, each tick
//! one roll of the level's random picks an addition to the underwater loop — the
//! ultra-rare one below 0.0001, the rare one below 0.001, the common bubbles below 0.01 —
//! played as an `UnderLiquidSubSound` that is cut off when the player surfaces.

use haste_audio::random::LegacyRandom;

use super::ambient_sound_handlers::AmbientPlayer;
use super::sound_manager::SoundManager;
use super::ticking_sounds::{TickableInstance, TickingSounds};
use super::under_liquid_sub_sound::UnderLiquidSubSound;

/// Vanilla `UnderwaterAmbientSoundHandler.CHANCE_PER_TICK`.
pub const CHANCE_PER_TICK: f32 = 0.01;
/// Vanilla `UnderwaterAmbientSoundHandler.RARE_CHANCE_PER_TICK`.
pub const RARE_CHANCE_PER_TICK: f32 = 0.001;
/// Vanilla `UnderwaterAmbientSoundHandler.ULTRA_RARE_CHANCE_PER_TICK`.
pub const ULTRA_RARE_CHANCE_PER_TICK: f32 = 1.0E-4;
/// Vanilla `UnderwaterAmbientSoundHandler.MINIMUM_TICK_DELAY`: the delay after a play.
pub const MINIMUM_TICK_DELAY: i32 = 0;

/// Vanilla `SoundEvents.AMBIENT_UNDERWATER_LOOP_ADDITIONS`.
pub const ADDITIONS: &str = "minecraft:ambient.underwater.loop.additions";
/// Vanilla `SoundEvents.AMBIENT_UNDERWATER_LOOP_ADDITIONS_RARE`.
pub const ADDITIONS_RARE: &str = "minecraft:ambient.underwater.loop.additions.rare";
/// Vanilla `SoundEvents.AMBIENT_UNDERWATER_LOOP_ADDITIONS_ULTRA_RARE`.
pub const ADDITIONS_ULTRA_RARE: &str = "minecraft:ambient.underwater.loop.additions.ultra_rare";

/// The addition one roll picks, the rarest first; `None` above the common chance.
pub fn pick_addition(roll: f32) -> Option<&'static str> {
    if roll < ULTRA_RARE_CHANCE_PER_TICK {
        Some(ADDITIONS_ULTRA_RARE)
    } else if roll < RARE_CHANCE_PER_TICK {
        Some(ADDITIONS_RARE)
    } else if roll < CHANCE_PER_TICK {
        Some(ADDITIONS)
    } else {
        None
    }
}

/// Vanilla `UnderwaterAmbientSoundHandler`.
#[derive(Debug, Default)]
pub struct UnderwaterAmbientSoundHandler {
    tick_delay: i32,
}

impl UnderwaterAmbientSoundHandler {
    /// `UnderwaterAmbientSoundHandler.tick`: count the delay down; once it is out and the
    /// player is under water, roll the level's random for an addition (for the `player`
    /// generation the sub sound follows).
    pub fn tick(
        &mut self,
        player_id: u32,
        player: &AmbientPlayer,
        random: &mut LegacyRandom,
        ticking: &mut TickingSounds,
        out: &mut dyn SoundManager,
    ) {
        // Java's int arithmetic: the delay keeps counting down, wrapping like vanilla's.
        self.tick_delay = self.tick_delay.wrapping_sub(1);
        if self.tick_delay > 0 || !player.under_water {
            return;
        }
        if let Some(event) = pick_addition(random.next_float()) {
            self.tick_delay = MINIMUM_TICK_DELAY;
            let sound = UnderLiquidSubSound::new(player_id, event);
            ticking.play(TickableInstance::UnderLiquidSub(sound), false, out);
        }
    }
}

#[cfg(test)]
mod tests;
