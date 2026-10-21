//! The critical hit of the local player's own attack (`Player.attack`: `criticalAttack`, then
//! `attackVisualEffects` -> `LocalPlayer.crit`). The attacker's client starts the crit emitter
//! itself, since the server does not send the attacker an animate packet for its own hit. The
//! target is queued here and drained by the session, which starts the emitter only on a living
//! target (`instanceof LivingEntity`, decided by the entity store).

use haste_core::view::BlockView;
use haste_core::EntityId;

use super::super::climbable::SyncedClimbables;
use super::super::LocalPlayer;
use super::InteractEnv;

/// `Player.attack`'s `fullStrengthAttack`: `getAttackStrengthScale(0.5F) > 0.9F`.
pub(crate) fn full_strength(scale_half: f32) -> bool {
    scale_half > 0.9
}

/// What `Player.canCriticalAttack` reads from the attacker, besides the target (which the session
/// checks: it must be a `LivingEntity`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CritInputs {
    pub fall_distance: f32,
    pub on_ground: bool,
    /// `Player.onClimbable`: false while flying, otherwise `LivingEntity.onClimbable`.
    pub on_climbable: bool,
    pub in_water: bool,
    /// `Player.isMobilityRestricted`: the blindness effect.
    pub blind: bool,
    /// `Player.isPassenger`.
    pub passenger: bool,
    pub sprinting: bool,
}

/// `Player.canCriticalAttack` without the `LivingEntity` test: a falling attacker that is not on
/// the ground, a climbable, water, blindness, riding or sprinting all rule the crit out.
pub(crate) fn can_critical_attack(c: &CritInputs) -> bool {
    c.fall_distance > 0.0
        && !c.on_ground
        && !c.on_climbable
        && !c.in_water
        && !c.blind
        && !c.passenger
        && !c.sprinting
}

impl LocalPlayer {
    /// The crit half of `Player.attack`, run before the attack ticker resets (`onAttack`).
    /// Queues `id` when the attack is a critical hit.
    pub(super) fn queue_critical_hit(
        &mut self,
        id: EntityId,
        world: &dyn BlockView,
        env: &InteractEnv<'_>,
    ) {
        if !full_strength(self.hand.attack_strength_scale(0.5)) {
            return;
        }
        let climb = SyncedClimbables::new(self.int.block_tags(), Some(&env.data.blocks));
        let inputs = CritInputs {
            fall_distance: self.state.fall_distance,
            on_ground: self.state.on_ground,
            on_climbable: !self.state.flying
                && haste_physics::on_climbable(&self.state, world, &climb),
            in_water: self.state.in_water,
            blind: self.int.is_blind(),
            passenger: self.vehicle.is_some(),
            sprinting: self.state.sprinting,
        };
        if can_critical_attack(&inputs) {
            self.int.crit_hits.push(id);
        }
    }

    /// The critical hits queued since the last drain, for the session's emitters.
    pub(crate) fn take_own_crit_hits(&mut self) -> Vec<EntityId> {
        std::mem::take(&mut self.int.crit_hits)
    }
}

#[cfg(test)]
mod tests;
