//! Attacking (player workstream): `Minecraft.startAttack` / `Minecraft.continueAttack` —
//! the one-click attack (entity: `MultiPlayerGameMode.attack`; block: start breaking; nothing:
//! a miss) and the held-button repeat that keeps breaking.

use haste_core::game::GameMode;
use haste_core::view::{BlockPhysics, BlockView};
use haste_core::Direction;
use haste_protocol::packets::play::serverbound as sb;
use haste_protocol::ServerboundPlay;

use super::super::LocalPlayer;
use super::{action, InteractEnv, Target, DESTROY_DELAY, MISS_TIME};

impl LocalPlayer {
    /// `Minecraft.startAttack`: one left click. Returns vanilla's `endAttack` — the block broke
    /// at once (creative, or a survival block with progress >= 1 in one tick), so
    /// `continueAttack` is skipped for this tick.
    pub(super) fn start_attack(
        &mut self,
        world: &dyn BlockView,
        env: &InteractEnv<'_>,
        out: &mut Vec<ServerboundPlay>,
    ) -> bool {
        if self.int.miss_time > 0 {
            return false;
        }
        // `LocalPlayer.isHandsBusy` — rowing (a movement key held while controlling a boat;
        // `rideTick` sets it): the click is dropped, no swing. Item use is gated separately,
        // in handleKeybinds' using-item branch.
        if self.hands_busy {
            return false;
        }
        // `Minecraft.startAttack`'s spectator branch comes before the swing: an entity under
        // the crosshair is teleported to (`MultiPlayerGameMode.spectate`, entity id + 1),
        // anything else is the explicit no-action (`spectatorNoAction`, id 0). No swing, no
        // missTime, and `endAttack` (true) skips continueAttack this tick.
        if self.game_mode == GameMode::Spectator {
            let target = match self.int.target {
                Some(Target::Entity { id, .. }) => id.0 + 1,
                _ => 0,
            };
            out.push(ServerboundPlay::SpectatorAction(sb::SpectatorAction {
                entity: target,
            }));
            return true;
        }
        // `Player.cannotAttackWithItem` (any item, not just spears): an attack-strength
        // requirement from `minecraft:minimum_attack_charge` the player has not reached yet
        // blocks the click outright — no swing, no missTime.
        let minimum_charge = self.held_minimum_attack_charge();
        if minimum_charge > 0.0 && self.hand.attack_strength_scale(0.0) < minimum_charge {
            return false;
        }
        // A piercing weapon (spear) stabs instead of attacking or breaking
        // (`MultiPlayerGameMode.piercingAttack`): STAB with a zeroed pos and DOWN face, no
        // sequence, `Player.onAttack` (the attack ticker's own half), the swing and the
        // weapon's draw sound; no block or entity packet. `endAttack` (true) skips
        // continueAttack this tick.
        if let Some(sound) = self.held_piercing().map(|s| s.map(str::to_owned)) {
            out.push(ServerboundPlay::PlayerAction(sb::PlayerAction {
                status: action::STAB,
                pos: haste_core::BlockPos::new(0, 0, 0),
                face: Direction::Down as u8,
                sequence: 0,
            }));
            self.hand.reset_only_attack_strength_ticker();
            self.hand.swing();
            if let Some(sound) = sound {
                self.piercing_sound(&sound);
            }
            return true;
        }
        // `Minecraft.startAttack` swings on every attack, whatever the crosshair is on.
        self.hand.swing();
        match self.int.target {
            Some(Target::Entity { id, .. }) => {
                // `MultiPlayerGameMode.attack`: the packet, `Player.attack` (`onAttack` resets
                // the attack ticker), `resetAttackStrengthTicker` (both tickers) and
                // `destroyDelay` 5 in creative. No missTime.
                out.push(ServerboundPlay::Attack(sb::Attack { entity_id: id.0 }));
                self.hand.reset_attack_strength_ticker();
                if self.game_mode == GameMode::Creative {
                    self.int.destroy_delay = DESTROY_DELAY;
                }
            }
            Some(Target::Block(h)) if !env.data.is_air(world.block_state(h.pos)) => {
                // startDestroyBlock, then endAttack when the block is air afterwards.
                let gone = self.start_destroy(h, world, env, out);
                out.push(ServerboundPlay::Punch(sb::Punch {}));
                return gone;
            }
            // A miss, or a block hit on an already-air block: vanilla's switch has no `break`
            // on the air branch, so it falls through to the MISS case — missTime 10 outside
            // creative (gameMode.hasMissTime) and an attack-strength reset.
            _ => {
                if self.game_mode != GameMode::Creative {
                    self.int.miss_time = MISS_TIME;
                }
                self.hand.reset_attack_strength_ticker();
            }
        }
        out.push(ServerboundPlay::Punch(sb::Punch {}));
        false
    }

    /// `Minecraft.continueAttack` with `down = keyAttack.isDown()` (this tick's `instantAttack`
    /// aside): a held block target keeps breaking (swinging every tick), anything else stops
    /// the break; releasing the button clears `missTime`; after a miss nothing happens for
    /// `missTime` ticks.
    pub(super) fn continue_attack(
        &mut self,
        down: bool,
        world: &dyn BlockView,
        env: &InteractEnv<'_>,
        out: &mut Vec<ServerboundPlay>,
    ) {
        if !down {
            self.int.miss_time = 0;
        }
        if self.int.miss_time > 0 || self.int.is_using_item {
            return;
        }
        // `Minecraft.continueAttack` does nothing at all for a piercing weapon: no breaking,
        // no swing, not even the release's stopDestroyBlock.
        if self.held_piercing().is_some() {
            return;
        }
        if !down {
            self.stop_destroy(out);
            return;
        }
        match self.int.target {
            Some(Target::Block(h)) if !env.data.is_air(world.block_state(h.pos)) => {
                if self.continue_destroy(h, world, env, out) {
                    // `Minecraft.continueAttack` swings each tick the breaking continues.
                    self.hand.swing();
                    out.push(ServerboundPlay::Punch(sb::Punch {}));
                }
            }
            // An entity, nothing, or an already-air block: stop breaking.
            _ => self.stop_destroy(out),
        }
    }
}
