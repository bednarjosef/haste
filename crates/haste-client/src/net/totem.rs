//! The totem of undying's pop (vanilla `ClientPacketListener.handleEntityEvent`, status 35):
//! a `TrackingEmitter` of `ParticleTypes.TOTEM_OF_UNDYING` starts on the entity, and
//! `SoundEvents.TOTEM_USE` plays at it (`playLocalSound`, volume 1, pitch 1, the entity's
//! sound source). The particles are ticked by the game thread (`game::particles`).

use haste_core::EntityId;

use super::{EntitySink, Session};
use crate::sound::{PlayRequest, SoundPos, SoundRef, SoundRequest};

/// `SoundEvents.TOTEM_USE`.
const TOTEM_USE: &str = "minecraft:item.totem.use";

/// Status 35 on an entity: starts its totem emitter and queues the pop sound. Our own
/// entity is not in the store, so its sound plays at the local player's position.
pub(super) fn start_totem<E: EntitySink + ?Sized>(
    session: &mut Session,
    id: EntityId,
    entities: &E,
) {
    session
        .totem_emitters
        .push(crate::game::totem_emitter::TotemEmitter::new(id));
    let own = id == session.entity_id;
    // The category follows the entity's type, as the hurt sounds do (our own is a player).
    let name: Option<String> = if own {
        Some("minecraft:player".to_owned())
    } else {
        entities
            .entity_type(id)
            .and_then(|k| session.entity_type_names.get(k.0 as usize))
            .map(|n| n.to_string())
    };
    let category = super::damage::hurt_category(name.as_deref());
    let pos = if own {
        SoundPos::Fixed(session.player.state.pos)
    } else {
        SoundPos::Entity(id.0)
    };
    session.sound_requests.push(SoundRequest::Play(PlayRequest {
        sound: SoundRef::Inline(TOTEM_USE.to_owned()),
        category,
        pos,
        volume: 1.0,
        pitch: 1.0,
        seed: session.totem_rng.next_long(),
    }));
}
