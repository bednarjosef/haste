//! The evoker fangs' bite (vanilla `ClientPacketListener.handleEntityEvent`, status 4, which
//! hands fangs to `EvokerFangs.handleEntityEvent`): the bite starts, and
//! `SoundEvents.EVOKER_FANGS_ATTACK` plays at the fangs (`playLocalSound`, the entity's sound
//! source, pitch `nextFloat * 0.2 + 0.85`). Other entities' status 4 is the iron golem's melee
//! swing and stays with `mark_attack`.

use haste_core::EntityId;

use super::{EntitySink, Session};
use crate::sound::{PlayRequest, SoundPos, SoundRef, SoundRequest};

/// `SoundEvents.EVOKER_FANGS_ATTACK`.
const EVOKER_FANGS_ATTACK: &str = "minecraft:entity.evoker_fangs.attack";
/// `SoundSource.NEUTRAL`: `EvokerFangs` keeps the default `getSoundSource`.
const CATEGORY_NEUTRAL: i32 = 6;

/// Status 4 on an entity: starts a fangs bite and queues its sound, or starts the iron
/// golem's melee swing for anything else.
pub(super) fn start_attack<E: EntitySink + ?Sized>(
    session: &mut Session,
    id: EntityId,
    entities: &mut E,
) {
    let is_fangs = entities
        .entity_type(id)
        .is_some_and(haste_entity::evoker_fangs::is_evoker_fangs);
    if !is_fangs {
        entities.mark_attack(id);
        return;
    }
    entities.mark_fangs_attack(id);
    // The sound source is not silent-aware yet: the store tracks no `isSilent` metadata, so the
    // bite always plays.
    let pitch = session.totem_rng.next_float() * 0.2 + 0.85;
    session.sound_requests.push(SoundRequest::Play(PlayRequest {
        sound: SoundRef::Inline(EVOKER_FANGS_ATTACK.to_owned()),
        category: CATEGORY_NEUTRAL,
        pos: SoundPos::Entity(id.0),
        volume: 1.0,
        pitch,
        seed: session.totem_rng.next_long(),
    }));
}
