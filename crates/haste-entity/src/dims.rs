//! Entity dimensions: from `cache/oracle/26.3/entity_dimensions.json` when oracles has produced it,
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
/// dump when oracles writes it.
const CROUCH_RATIO: f32 = 1.5 / 1.8;
const CRAWL_RATIO: f32 = 0.6 / 1.8;

#[derive(Default)]
pub struct EntityDimensions {
    map: HashMap<RegistryIndex, Dimensions>,
}

impl EntityDimensions {
    /// From the `minecraft:entity_type` registry only: every type gets the default box until the
    /// oracle dump exists. The oracle file (oracles) is read lazily at first use from
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

/// Reads `entity_dimensions.json` if present. Format (tools/recorder-mod `ServerDumps`):
/// `{"version":1,"types":[{"id":<entity_type protocol id>,"type":"minecraft:oak_boat","width":1.375,
/// "height":0.5625,"eye_height":...,"fixed":bool}],"player":{...}}`. Every entry already carries
/// the registry protocol id, so no name -> index mapping is needed; `id` values the synced
/// registry does not carry (a mod's) resolve to themselves and simply never hit. The file is read
/// once and cached.
fn load_oracle() -> Option<HashMap<RegistryIndex, Dimensions>> {
    let path = haste_core::paths::oracle_dir().join("entity_dimensions.json");
    let bytes = std::fs::read(path).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let types = v.get("types")?.as_array()?;
    let mut map = HashMap::with_capacity(types.len());
    for t in types {
        let (Some(id), Some(width), Some(height), Some(eye_height)) = (
            t.get("id").and_then(serde_json::Value::as_u64),
            t.get("width").and_then(serde_json::Value::as_f64),
            t.get("height").and_then(serde_json::Value::as_f64),
            t.get("eye_height").and_then(serde_json::Value::as_f64),
        ) else {
            continue;
        };
        map.insert(
            RegistryIndex(id as u32),
            Dimensions {
                width: width as f32,
                height: height as f32,
                eye_height: eye_height as f32,
            },
        );
    }
    Some(map)
}
