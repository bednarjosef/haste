use serde::{Deserialize, Serialize};

use crate::Identifier;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum GameMode { Survival = 0, Creative = 1, Adventure = 2, Spectator = 3 }
impl GameMode {
    pub fn from_id(i: i32) -> Option<Self> {
        Some(match i { 0 => Self::Survival, 1 => Self::Creative, 2 => Self::Adventure, 3 => Self::Spectator, _ => return None })
    }
}

/// What the Login packet tells us about the dimension we are in.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct DimensionInfo {
    pub name: Identifier,
    pub dimension_type: Identifier,
    pub min_y: i32,
    pub height: i32,
    pub has_skylight: bool,
}
impl DimensionInfo {
    pub fn overworld() -> Self {
        Self { name: Identifier::minecraft("overworld"), dimension_type: Identifier::minecraft("overworld"), min_y: -64, height: 384, has_skylight: true }
    }
    pub fn section_count(&self) -> usize { (self.height / 16) as usize }
    pub fn min_section(&self) -> i32 { self.min_y >> 4 }
}

/// Fixed tick rate of the game loop.
pub const TICKS_PER_SECOND: u32 = 20;
pub const TICK_SECONDS: f64 = 1.0 / TICKS_PER_SECOND as f64;
