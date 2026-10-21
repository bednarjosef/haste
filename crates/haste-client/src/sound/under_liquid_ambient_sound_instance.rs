//! Vanilla `UnderLiquidAmbientSoundInstance`: the looping underwater ambience
//! (`ambient.underwater.loop`) that `LocalPlayer.updateIsUnderwater` starts when the eyes
//! go under water. It fades in over 40 ticks while the player stays under the liquid and
//! out twice as fast once they leave it, and stops itself once faded out (or once its
//! player is gone); re-entering before it has faded out fades the same loop back in.

use super::ticking_sounds::TickingPlayer;

/// Vanilla `UnderLiquidAmbientSoundInstance.FADE_DURATION`, in ticks.
pub const FADE_DURATION: i32 = 40;

/// The underwater loop event (vanilla `SoundEvents.AMBIENT_UNDERWATER_LOOP`).
pub const AMBIENT_UNDERWATER_LOOP: &str = "minecraft:ambient.underwater.loop";

/// One underwater ambience loop.
#[derive(Clone, Debug, PartialEq)]
pub struct UnderLiquidAmbientSoundInstance {
    /// The local player it follows (a respawn makes a new player and this one "removed").
    player: u32,
    /// Ticks of fade, 0..=40; below 0 the instance stops.
    fade: i32,
    volume: f32,
    stopped: bool,
}

impl UnderLiquidAmbientSoundInstance {
    /// `UnderLiquidAmbientSoundInstance.underwater(player)`: looping, no delay, volume 1,
    /// relative to the listener, following the player's `isUnderWater`.
    pub fn underwater(player: u32) -> Self {
        Self {
            player,
            fade: 0,
            volume: 1.0,
            stopped: false,
        }
    }

    /// The event it plays.
    pub fn event(&self) -> &'static str {
        AMBIENT_UNDERWATER_LOOP
    }

    /// The instance volume (`AbstractSoundInstance.volume`).
    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// `AbstractTickableSoundInstance.isStopped`.
    pub fn is_stopped(&self) -> bool {
        self.stopped
    }

    /// `UnderLiquidAmbientSoundInstance.tick`: while its player is there and it has not
    /// faded out, one tick of fade in (under the liquid) or a double tick of fade out,
    /// capped at the full duration, and the volume from it; otherwise it stops.
    pub fn tick(&mut self, player: &TickingPlayer) {
        if player.is_removed(self.player) || self.fade < 0 {
            self.stopped = true;
            return;
        }
        if player.under_water {
            self.fade += 1;
        } else {
            self.fade -= 2;
        }
        self.fade = self.fade.min(FADE_DURATION);
        self.volume = (self.fade as f32 / FADE_DURATION as f32).clamp(0.0, 1.0);
    }
}
