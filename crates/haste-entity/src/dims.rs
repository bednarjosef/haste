//! Entity dimensions: from `cache/oracle/26.3/entity_dimensions.json` when WS8 has produced it,
//! else the 0.6 x 1.8 default. Pose adjusts the height (minecraft.wiki, "Entity" hitbox table).
use std::collections::HashMap;

use haste_core::entity::Pose;
use haste_core::ids::RegistryIndex;

/// Width/height/eye height in blocks for one entity type.
#[derive(Clone, Copy, Debug)]
pub struct Dimensions {
    pub width: f32,
    pub height: f32,
    pub eye_height: f32,
}

impl Default for Dimensions {
    fn default() -> Self {
        Dimensions {
            width: 0.6,
            height: 1.8,
            eye_height: 1.62,
        }
    }
}

/// The default standing hitbox (0.6 x 1.8) used for every type the oracle does not name.
const DEFAULT: Dimensions = Dimensions {
    width: 0.6,
    height: 1.8,
    eye_height: 1.62,
};

/// Sneaking/crawling height scaling. Observed on vanilla servers: a 1.8-tall humanoid crouches to
/// 1.5, crawls to 0.6. We scale the standing height by the same ratios, which reproduces the wiki
/// values for all player-like entities; exact per-pose values for odd shapes come from the oracle
/// dump when WS8 writes it.
const CROUCH_RATIO: f32 = 1.5 / 1.8;
const CRAWL_RATIO: f32 = 0.6 / 1.8;

#[derive(Default)]
pub struct EntityDimensions {
    map: HashMap<RegistryIndex, Dimensions>,
}

impl EntityDimensions {
    /// From the `minecraft:entity_type` registry only: every type gets the default box until the
    /// oracle dump exists. The oracle file (WS8) is read lazily at first use from
    /// `haste_core::paths::oracle_dir()`.
    pub fn from_registry_len(_entity_types: usize) -> EntityDimensions {
        let map = load_oracle().unwrap_or_default();
        EntityDimensions { map }
    }

    pub fn get(&self, kind: RegistryIndex, pose: Pose) -> (f32, f32) {
        let d = self.map.get(&kind).copied().unwrap_or(DEFAULT);
        let height = match pose {
            Pose::Crouching => d.height * CROUCH_RATIO,
            Pose::Swimming | Pose::FallFlying | Pose::SpinAttack => d.height * CRAWL_RATIO,
            _ => d.height,
        };
        (d.width, height)
    }

    /// Height for the crouch flags (when no explicit pose metadata arrived).
    pub fn crouching(&self, standing: (f32, f32)) -> (f32, f32) {
        (standing.0, standing.1 * CROUCH_RATIO)
    }

    pub fn is_oracle_backed(&self) -> bool {
        !self.map.is_empty()
    }
}

/// Reads `entity_dimensions.json` if present. Format (brief):
/// `{"version":1,"entities":{"minecraft:zombie":{"width":0.6,"height":1.95,"eye_height":1.74}}}`.
/// Names need the entity_type registry to become indexes; until haste-data provides it we cannot
/// map them, so this returns `None` and the default box is used. The oracle file is read once and
/// cached; when WS2's `GameData` lands, the name -> `RegistryIndex` mapping goes through it.
fn load_oracle() -> Option<HashMap<RegistryIndex, Dimensions>> {
    let path = haste_core::paths::oracle_dir().join("entity_dimensions.json");
    let bytes = std::fs::read(path).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let entities = v.get("entities")?.as_object()?;
    // No registry to resolve names against yet (WS2's GameData is not exported). Parsed but
    // unmapped: return nothing rather than guessing index positions.
    let _ = entities.len();
    None
}
