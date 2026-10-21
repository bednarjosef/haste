// A firework rocket's burst (vanilla `FireworkRocketEntity.handleEntityEvent` status 17 ->
// `ClientLevel.createFireworks`): the packet carries no explosions, so the rocket's synched
// stack (its `fireworks` component) is read here and queued for the particle tick, which holds
// the component tables that decode it.

use glam::DVec3;
use haste_protocol::haste_core::entity::ItemStackData;
use haste_protocol::haste_core::EntityId;

use super::{EntitySink, Session};

/// One rocket's burst, waiting for the particle tick.
#[derive(Clone, Debug, Default)]
pub struct FireworkBurst {
    /// The rocket's position and movement (`createFireworks`' `x..z` and `xd..zd`).
    pub pos: DVec3,
    pub velocity: DVec3,
    /// The rocket's synched stack; its `fireworks` component holds the explosions. Empty when
    /// the rocket sent none (vanilla's default rocket, which bursts into POOF).
    pub stack: ItemStackData,
}

impl Session {
    /// Entity status 17 on a rocket we track: the burst is queued for the particle tick. Other
    /// entities' status 17 is not ours to draw.
    pub(super) fn firework_status<E: EntitySink + ?Sized>(&mut self, id: EntityId, entities: &E) {
        if let Some(burst) = entities.firework_burst(id) {
            self.firework_bursts.push(burst);
        }
    }
}
