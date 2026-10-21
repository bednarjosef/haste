//! Vanilla `UnderLiquidSubSound`: one of the underwater additions
//! (`UnderwaterAmbientSoundHandler` picks which), played once at volume 1 relative to the
//! listener and cut off the tick its player leaves the water (or is gone).

use super::ticking_sounds::TickingPlayer;

/// One underwater addition.
#[derive(Clone, Debug, PartialEq)]
pub struct UnderLiquidSubSound {
    /// The local player it follows.
    player: u32,
    event: &'static str,
    stopped: bool,
}

impl UnderLiquidSubSound {
    /// The addition `event` for `player`: not looping, no delay, volume 1, relative.
    pub fn new(player: u32, event: &'static str) -> Self {
        Self {
            player,
            event,
            stopped: false,
        }
    }

    /// The event it plays.
    pub fn event(&self) -> &'static str {
        self.event
    }

    /// The instance volume: constant 1.
    pub fn volume(&self) -> f32 {
        1.0
    }

    /// `AbstractTickableSoundInstance.isStopped`.
    pub fn is_stopped(&self) -> bool {
        self.stopped
    }

    /// `UnderLiquidSubSound.tick`: stops once the player is gone or no longer under water.
    pub fn tick(&mut self, player: &TickingPlayer) {
        if player.is_removed(self.player) || !player.under_water {
            self.stopped = true;
        }
    }
}
