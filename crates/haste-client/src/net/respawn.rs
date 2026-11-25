// The default spawn point (vanilla `ClientPacketListener.handleSetSpawn` -> `ClientLevel.setRespawnData`):
// the dimension, block and facing the server sent for the player's respawn, which the compass
// points at.

use haste_protocol::haste_core::{BlockPos, Identifier};
use haste_protocol::packets::play::clientbound as cb;

use super::Session;

/// The overworld's registry name (vanilla `Level.OVERWORLD`).
const OVERWORLD: &str = "overworld";

/// A respawn point (vanilla `LevelData.RespawnData`): the dimension, the block and the yaw and
/// pitch the player faces there.
#[derive(Clone, Debug, PartialEq)]
pub struct RespawnData {
    /// The dimension's id (`minecraft:overworld`, ...).
    pub dimension: Identifier,
    /// The block the player respawns on.
    pub pos: BlockPos,
    /// The yaw the player faces there.
    pub yaw: f32,
    /// The pitch the player faces there.
    pub pitch: f32,
}

impl Default for RespawnData {
    /// Vanilla `LevelData.RespawnData.DEFAULT`: the overworld at the origin, facing `0, 0`.
    fn default() -> Self {
        RespawnData {
            dimension: Identifier::minecraft(OVERWORLD),
            pos: BlockPos::default(),
            yaw: 0.0,
            pitch: 0.0,
        }
    }
}

impl Session {
    /// `ClientPacketListener.handleSetSpawn`: the packet's respawn data replaces the stored one.
    /// Vanilla then moves it inside the world border (`Level.getWorldBorderAdjustedRespawnData`);
    /// that needs the world's motion-blocking heightmap, which the packet step has no access to,
    /// so the position is kept as sent.
    pub(super) fn set_respawn_data(&mut self, s: &cb::SetDefaultSpawnPosition) {
        self.respawn = RespawnData {
            dimension: s.dimension.clone(),
            pos: s.pos,
            yaw: s.yaw,
            pitch: s.pitch,
        };
    }
}

#[cfg(test)]
mod tests;
