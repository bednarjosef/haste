// The minecart track packet (vanilla `ClientPacketListener.handleMinecartAlongTrack`) and the
// level feature that decides which behaviour a minecart runs
// (`AbstractMinecart.useExperimentalMovement`). The walk itself lives in `haste-entity`'s
// `new_minecart_behavior`.

use haste_protocol::haste_core::EntityId;
use haste_protocol::packets::play::clientbound::MoveMinecartAlongTrack;
use haste_protocol::SyncedRegistries;

use super::{EntitySink, Session};

/// `FeatureFlags.MINECART_IMPROVEMENTS`'s id (`minecraft:minecart_improvements`).
const MINECART_IMPROVEMENTS: (&str, &str) = ("minecraft", "minecart_improvements");

/// `AbstractMinecart.useExperimentalMovement`: the level's enabled features (the server's
/// `update_enabled_features` during configuration) include `minecart_improvements`, so a
/// minecart spawned now runs `NewMinecartBehavior`. Compared in place: no allocation per spawn.
pub(super) fn use_experimental_movement(regs: &SyncedRegistries) -> bool {
    regs.features
        .iter()
        .any(|f| (f.namespace(), f.path()) == MINECART_IMPROVEMENTS)
}

impl Session {
    /// `ClientPacketListener.handleMinecartAlongTrack`: a minecart on the new behaviour adds
    /// the packet's steps to the ones its next window walks; anything else (an old-behaviour
    /// cart, another type, an unknown id) ignores them.
    pub(super) fn handle_minecart_along_track<E: EntitySink + ?Sized>(
        packet: &MoveMinecartAlongTrack,
        entities: &mut E,
    ) {
        entities.handle_minecart_along_track(EntityId(packet.entity_id), &packet.steps);
    }
}

#[cfg(test)]
mod tests;
