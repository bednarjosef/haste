// The `animate` packet (vanilla `ClientPacketListener.handleAnimate`): a critical hit or a magic
// critical hit on an entity starts a tracking emitter on it. The leave-bed action (0) is not
// ported yet: Haste keeps no sleeping state for players.

use haste_core::{EntityId, Identifier};
use haste_protocol::packets::play::clientbound as cb;
use haste_protocol::ClientboundPlay;

use super::{EntitySink, Session};

impl Session {
    /// The `animate` packet; returns the packet when it is not one.
    pub(super) fn apply_animate<E: EntitySink + ?Sized>(
        &mut self,
        p: ClientboundPlay,
        entities: &E,
    ) -> Option<ClientboundPlay> {
        use ClientboundPlay as P;
        match p {
            P::Animate(a) => self.animate(&a, entities),
            _ => return Some(p),
        }
        None
    }

    /// `handleAnimate`'s critical hit actions: `ParticleTypes.CRIT` (action 1) and
    /// `ParticleTypes.ENCHANTED_HIT` (action 2) on the entity. Other actions are not drawn.
    fn animate<E: EntitySink + ?Sized>(&mut self, a: &cb::Animate, entities: &E) {
        let kind = match a.animation {
            1 => "crit",
            2 => "enchanted_hit",
            _ => return,
        };
        self.create_tracking_emitter(EntityId(a.entity_id), Identifier::minecraft(kind), entities);
    }
}
