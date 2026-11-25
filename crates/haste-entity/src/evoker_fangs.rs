//! The evoker fangs' client state (`EvokerFangs`): entity event 4 starts the bite, then
//! `lifeTicks` counts down on the client (`EvokerFangs.tick`) until the server discards the
//! fangs. Before the bite starts the fangs are inert and draw nothing.

use haste_core::ids::EntityId;
use haste_core::{DVec3, RegistryIndex};

use crate::store::EntityStore;

/// `EvokerFangs.ATTACK_DURATION`: the bite lasts 20 ticks.
pub const ATTACK_DURATION: i32 = 20;
/// `EvokerFangs.LIFE_OFFSET`: subtracted from `lifeTicks` before the bite's progress is read.
pub const LIFE_OFFSET: i32 = 2;
/// The `lifeTicks` a fresh `EvokerFangs` starts with (`private int lifeTicks = 22`).
pub const INITIAL_LIFE_TICKS: i32 = 22;
/// `EvokerFangs.tick` on the client: the tick whose `lifeTicks` reaches 14 throws 12 crits.
pub const CRIT_LIFE_TICKS: i32 = 14;
/// How many crit particles that burst throws.
pub const CRIT_COUNT: usize = 12;

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

    /// Whether this tick is the one `EvokerFangs.tick` throws its crit burst on: the bite has
    /// started and `lifeTicks` has just counted down to 14.
    pub fn crit_burst_now(&self) -> bool {
        self.attack_started && self.life_ticks == CRIT_LIFE_TICKS
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

/// One crit of `EvokerFangs.tick`'s burst: the particle's position and velocity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FangsCrit {
    pub pos: DVec3,
    pub vel: DVec3,
}

/// `EvokerFangs.tick`'s burst for fangs at `pos` with box width `width`: [`CRIT_COUNT`]
/// crits spread over the box, one block above a random height in its first block, thrown up
/// and outwards. `next_double` is the entity's random; the draws follow vanilla's order
/// (x, y, z, then the three speeds).
pub fn crit_burst(
    pos: DVec3,
    width: f32,
    mut next_double: impl FnMut() -> f64,
    mut out: impl FnMut(FangsCrit),
) {
    let half = f64::from(width) * 0.5;
    for _ in 0..CRIT_COUNT {
        let x = pos.x + (next_double() * 2.0 - 1.0) * half;
        let y = pos.y + 0.05 + next_double();
        let z = pos.z + (next_double() * 2.0 - 1.0) * half;
        let xd = (next_double() * 2.0 - 1.0) * 0.3;
        let yd = 0.3 + next_double() * 0.3;
        let zd = (next_double() * 2.0 - 1.0) * 0.3;
        out(FangsCrit {
            pos: DVec3::new(x, y + 1.0, z),
            vel: DVec3::new(xd, yd, zd),
        });
    }
}

impl EntityStore {
    /// The fangs whose crit burst falls on this tick (`EvokerFangs.tick`), as their position
    /// and box width, for the client's particle step after the entity tick. No allocation.
    pub fn fangs_crit_bursts(&self) -> impl Iterator<Item = (DVec3, f32)> + '_ {
        self.slots
            .iter()
            .flatten()
            .filter(|slot| slot.fangs.crit_burst_now())
            .map(|slot| (slot.pos, slot.dimensions.0))
    }
}
