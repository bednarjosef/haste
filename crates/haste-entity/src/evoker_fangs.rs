//! The evoker fangs' client state (`EvokerFangs`): entity event 4 starts the bite, then
//! `lifeTicks` counts down on the client (`EvokerFangs.tick`) until the server discards the
//! fangs. Before the bite starts the fangs are inert and draw nothing.

use haste_core::ids::EntityId;
use haste_core::RegistryIndex;

use crate::store::EntityStore;

/// `EvokerFangs.ATTACK_DURATION`: the bite lasts 20 ticks.
pub const ATTACK_DURATION: i32 = 20;
/// `EvokerFangs.LIFE_OFFSET`: subtracted from `lifeTicks` before the bite's progress is read.
pub const LIFE_OFFSET: i32 = 2;
/// The `lifeTicks` a fresh `EvokerFangs` starts with (`private int lifeTicks = 22`).
pub const INITIAL_LIFE_TICKS: i32 = 22;

/// The fangs' client-side state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FangsState {
    /// `clientSideAttackStarted`: set by entity event 4 (`EvokerFangs.handleEntityEvent`).
    pub attack_started: bool,
    /// `lifeTicks`: counts down once the attack has started.
    pub life_ticks: i32,
}

impl Default for FangsState {
    fn default() -> Self {
        Self {
            attack_started: false,
            life_ticks: INITIAL_LIFE_TICKS,
        }
    }
}

impl FangsState {
    /// `EvokerFangs.tick` on the client: `lifeTicks` counts down only once the attack has
    /// started. Other entities keep the flag clear, so this is a no-op for them.
    pub(crate) fn tick(&mut self) {
        if self.attack_started {
            self.life_ticks -= 1;
        }
    }

    /// `EvokerFangs.getAnimationProgress(partial)`: 0 before the bite, 1 once the bite is
    /// over, and in between the bite's progress through its 20 ticks at `partial`.
    pub fn animation_progress(&self, partial: f32) -> f32 {
        if !self.attack_started {
            return 0.0;
        }
        let remaining_life = self.life_ticks - LIFE_OFFSET;
        if remaining_life <= 0 {
            1.0
        } else {
            1.0 - (remaining_life as f32 - partial) / ATTACK_DURATION as f32
        }
    }
}

/// Whether a tracked entity is a `minecraft:evoker_fangs` (its status 4 starts a bite, where
/// the iron golem's status 4 starts a melee swing).
pub fn is_evoker_fangs(kind: RegistryIndex) -> bool {
    crate::registry::entity_type_name(kind) == Some("minecraft:evoker_fangs")
}

impl EntityStore {
    /// Entity event 4 on an `EvokerFangs` (`EvokerFangs.handleEntityEvent`): the bite starts.
    /// The bite sound is queued by the client, which knows the sound source and the position.
    pub fn mark_fangs_attack(&mut self, id: EntityId) {
        if let Some(e) = self.get_mut(id) {
            e.fangs.attack_started = true;
        }
    }
}
