//! Vanilla `PlayerMenuItem`: one player in a teleport list.

use haste_protocol::packets::play::serverbound as sb;
use haste_protocol::ServerboundPlay;
use uuid::Uuid;

use super::menu::MenuEffects;

/// A player the menu can teleport to: the profile id and name, snapshotted when the menu
/// (or its category) was built, like vanilla's `PlayerInfo` reference.
#[derive(Clone, Debug)]
pub struct PlayerItem {
    pub uuid: Uuid,
    /// The profile name (`Component.literal(playerInfo.getProfile().name())`).
    pub name: String,
}

impl PlayerItem {
    /// `selectItem`: the teleport request, `ServerboundTeleportToEntityPacket` with the
    /// profile id.
    pub fn select(&self, out: &mut MenuEffects) {
        out.packets
            .push(ServerboundPlay::TeleportToEntity(sb::TeleportToEntity {
                target: self.uuid,
            }));
    }
}
