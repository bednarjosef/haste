//! The creative-flight block of vanilla `LocalPlayer.aiStep`: the jump double tap that toggles
//! flight, spectator's forced flight, and flight dropped when `mayfly` is off.

use haste_core::view::{BlockPhysics, BlockView};

use super::{spectator, GameMode, LocalPlayer, JUMP_DOUBLE_TAP_TICKS};

impl LocalPlayer {
    /// Runs the block for this tick's fresh jump press; returns whether it just toggled creative
    /// flight (the glide start below it skips that press).
    pub(super) fn creative_flight_step(
        &mut self,
        pressed: bool,
        world: &dyn BlockView,
        blocks: &dyn BlockPhysics,
    ) -> bool {
        // Vanilla's double tap: while the window is armed a fresh press of jump (or the input
        // layer's own double-tap signal) toggles flight; without it a press arms the window for
        // 7 ticks. Vanilla keeps the window while the player swims, and skips the toggle on a
        // vehicle the player cannot jump (haste has no vehicles).
        let armed = self.jump_trigger_time > 0;
        let double_tap = std::mem::take(&mut self.toggle_flight);
        let mut toggled = false;
        // Vanilla `LocalPlayer.aiStep`'s creative-flight block: the double-tap toggle runs
        // only outside spectator; in spectator `mayfly` keeps flight on instead.
        if self.abilities.may_fly
            && self.game_mode != GameMode::Spectator
            && (pressed || double_tap)
        {
            if armed {
                if !self.state.swimming {
                    self.jump_trigger_time = 0;
                    self.state.flying = !self.state.flying;
                    toggled = true;
                    if self.state.flying && self.state.on_ground {
                        self.jump_from_ground(world, blocks);
                    }
                }
            } else {
                self.jump_trigger_time = JUMP_DOUBLE_TAP_TICKS;
            }
        }
        if spectator::ai_step_forces_flight(
            self.abilities.may_fly,
            self.game_mode == GameMode::Spectator,
            self.state.flying,
        ) {
            self.state.flying = true;
            toggled = true;
        }
        if !self.abilities.may_fly {
            self.state.flying = false;
        }
        toggled
    }
}
