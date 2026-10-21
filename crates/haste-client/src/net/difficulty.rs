// The level's difficulty (vanilla `ClientPacketListener.handleChangeDifficulty` ->
// `ClientLevel.ClientLevelData.setDifficulty` and `setDifficultyLocked`). A login makes a new
// level data at `Difficulty.NORMAL`, unlocked (`handleLogin`); a respawn keeps the difficulty.

use haste_protocol::packets::play::clientbound as cb;

use super::Session;

/// `Difficulty`: peaceful, easy, normal and hard, by their ids 0 to 3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Difficulty {
    Peaceful,
    Easy,
    Normal,
    Hard,
}

impl Difficulty {
    /// `Difficulty.byId`: the id wraps around the four values (`ByIdMap.OutOfBoundsStrategy.WRAP`),
    /// so every id has a difficulty.
    pub fn from_id(id: u8) -> Self {
        match id % 4 {
            0 => Difficulty::Peaceful,
            1 => Difficulty::Easy,
            2 => Difficulty::Normal,
            _ => Difficulty::Hard,
        }
    }
}

/// `ClientLevel.ClientLevelData`'s difficulty and its lock (the lock only greys out the option).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LevelDifficulty {
    /// The level's difficulty.
    pub difficulty: Difficulty,
    /// Whether the server has locked the difficulty.
    pub locked: bool,
}

impl Default for LevelDifficulty {
    /// The difficulty a login starts with (`handleLogin` passes `Difficulty.NORMAL`), unlocked.
    fn default() -> Self {
        LevelDifficulty {
            difficulty: Difficulty::Normal,
            locked: false,
        }
    }
}

impl Session {
    /// `handleChangeDifficulty`: the packet's difficulty and lock replace the stored ones.
    pub(super) fn set_difficulty(&mut self, c: &cb::ChangeDifficulty) {
        self.level_difficulty = LevelDifficulty {
            difficulty: Difficulty::from_id(c.difficulty),
            locked: c.locked,
        };
    }
}

#[cfg(test)]
mod tests;
