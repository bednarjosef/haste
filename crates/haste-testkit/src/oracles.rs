//! Loaders for the server-side oracle dumps (see `tools/recorder-mod` and FORMATS.md).
//!
//! `block_shapes.json` etc. live in `cache/oracle/26.3/`. Loading is best-effort per file: each
//! `load_*` function reads one file and validates its `version`; missing files are an error (the
//! dump script exists for that), but `BlockShapes::load_dir` callers decide how to fall back
//! (haste-data has its own fallback when the file is absent).

use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use haste_core::{Aabb, BlockStateId, DVec3};
use serde::Deserialize;

const KNOWN_VERSION: u32 = 1;

fn check_version(v: u32, what: &str) -> Result<()> {
    if v != KNOWN_VERSION {
        bail!("{what}: unsupported format version {v} (expected {KNOWN_VERSION})");
    }
    Ok(())
}

// ---------------------------------------------------------------- block_shapes.json

#[derive(Deserialize)]
struct BlockShapesFile {
    version: u32,
    #[serde(default)]
    #[allow(dead_code)] // written by the mod for traceability; not needed once loaded
    minecraft: Option<String>,
    states: Vec<BlockShapeStateJson>,
}

#[derive(Deserialize)]
struct BlockShapeStateJson {
    id: u32,
    block: String,
    #[serde(default)]
    properties: HashMap<String, String>,
    #[serde(default)]
    collision: Vec<[f64; 6]>,
    #[serde(default)]
    outline: Vec<[f64; 6]>,
    #[serde(default)]
    light: u32,
    #[serde(default)]
    air: bool,
    #[serde(default)]
    opaque: bool,
    #[serde(default)]
    fluid: Option<String>,
    #[serde(default)]
    fluid_empty: Option<bool>,
    #[serde(default)]
    neighbours: bool,
}

/// One block state's oracle data. Box coordinates are block-local (0..=1).
#[derive(Clone, Debug)]
pub struct BlockShapeState {
    pub id: BlockStateId,
    pub block: String,
    pub properties: Vec<(String, String)>,
    pub collision: Vec<Aabb>,
    pub outline: Vec<Aabb>,
    pub light: u32,
    pub air: bool,
    pub opaque: bool,
    pub fluid: Option<String>,
    pub fluid_empty: Option<bool>,
    /// Shape may depend on neighbours (heuristic flag from the recorder; see FORMATS.md).
    pub neighbours: bool,
}

/// Indexed block-shape oracle. Lookup by state id is O(1); by (block, properties) is a map.
#[derive(Clone, Debug)]
pub struct BlockShapes {
    states: Vec<BlockShapeState>,
    by_id: Vec<Option<usize>>, // state id -> index into `states` (sparse-safe)
}

impl BlockShapes {
    pub fn load_dir(dir: &Path) -> Result<BlockShapes> {
        Self::load(&dir.join("block_shapes.json"))
    }

    pub fn load(path: &Path) -> Result<BlockShapes> {
        let file: BlockShapesFile = serde_json::from_reader(std::io::BufReader::new(
            std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?,
        ))
        .with_context(|| format!("parsing {}", path.display()))?;
        check_version(file.version, "block_shapes.json")?;

        let mut states = Vec::with_capacity(file.states.len());
        let mut by_id = Vec::new();
        for s in file.states {
            let idx = states.len();
            let id = s.id as usize;
            if by_id.len() <= id {
                by_id.resize(id + 1, None);
            }
            // JSON property order is not guaranteed stable across Gson versions; sort like
            // blocks.json lists them (by name) so haste-data's `properties()` agrees.
            let mut properties: Vec<(String, String)> = s.properties.into_iter().collect();
            properties.sort_by(|a, b| a.0.cmp(&b.0));
            by_id[id] = Some(idx);
            states.push(BlockShapeState {
                id: BlockStateId(s.id),
                block: s.block,
                properties,
                collision: s.collision.iter().map(box_of).collect(),
                outline: s.outline.iter().map(box_of).collect(),
                light: s.light,
                air: s.air,
                opaque: s.opaque,
                fluid: s.fluid,
                fluid_empty: s.fluid_empty,
                neighbours: s.neighbours,
            });
        }
        Ok(BlockShapes { states, by_id })
    }

    pub fn len(&self) -> usize {
        self.states.len()
    }

    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    /// All states, ordered by state id (ids are unique; gaps would be `None` in between).
    pub fn states(&self) -> &[BlockShapeState] {
        &self.states
    }

    pub fn state(&self, id: BlockStateId) -> Option<&BlockShapeState> {
        self.by_id
            .get(id.0 as usize)
            .and_then(|i| i.map(|i| &self.states[i]))
    }

    /// First state of `block` whose properties are a superset of `props` (exact match if `props`
    /// lists every property; partial otherwise, e.g. `("oak_slab", &[("type","bottom")])`).
    pub fn find(&self, block: &str, props: &[(&str, &str)]) -> Option<&BlockShapeState> {
        self.states.iter().find(|s| {
            s.block == block
                && props
                    .iter()
                    .all(|(k, v)| s.properties.iter().any(|(pk, pv)| pk == k && pv == v))
        })
    }
}

impl haste_core::view::CollisionShapes for BlockShapes {
    fn collision_boxes(&self, state: BlockStateId) -> &[Aabb] {
        self.state(state)
            .map(|s| s.collision.as_slice())
            .unwrap_or(&[])
    }
    fn outline_boxes(&self, state: BlockStateId) -> &[Aabb] {
        self.state(state)
            .map(|s| s.outline.as_slice())
            .unwrap_or(&[])
    }
}

fn box_of(b: &[f64; 6]) -> Aabb {
    Aabb {
        min: DVec3::new(b[0], b[1], b[2]),
        max: DVec3::new(b[3], b[4], b[5]),
    }
}

// ---------------------------------------------------------------- entity_dimensions.json

#[derive(Deserialize)]
struct EntityDimensionsFile {
    version: u32,
    types: Vec<EntityTypeDimensionsJson>,
    player: HashMap<String, PlayerPoseDimensions>,
}

#[derive(Deserialize)]
struct EntityTypeDimensionsJson {
    id: u32,
    #[serde(rename = "type")]
    type_name: String,
    width: f32,
    height: f32,
    eye_height: f32,
    #[serde(default)]
    fixed: bool,
}

/// One entity type's default dimensions.
#[derive(Clone, Debug)]
pub struct EntityTypeDimensions {
    pub id: u32,
    pub type_name: String,
    pub width: f32,
    pub height: f32,
    pub eye_height: f32,
    pub fixed: bool,
}

/// Per-pose player dimensions (width, height, eye height).
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct PlayerPoseDimensions {
    pub width: f32,
    pub height: f32,
    pub eye_height: f32,
}

/// Entity dimensions oracle.
#[derive(Clone, Debug)]
pub struct EntityDimensions {
    types: Vec<EntityTypeDimensions>,
    by_name: HashMap<String, usize>,
    player: HashMap<String, PlayerPoseDimensions>,
}

impl EntityDimensions {
    pub fn load_dir(dir: &Path) -> Result<EntityDimensions> {
        Self::load(&dir.join("entity_dimensions.json"))
    }

    pub fn load(path: &Path) -> Result<EntityDimensions> {
        let file: EntityDimensionsFile = serde_json::from_reader(std::io::BufReader::new(
            std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?,
        ))
        .with_context(|| format!("parsing {}", path.display()))?;
        check_version(file.version, "entity_dimensions.json")?;
        let mut by_name = HashMap::with_capacity(file.types.len());
        for (i, t) in file.types.iter().enumerate() {
            by_name.insert(t.type_name.clone(), i);
        }
        Ok(EntityDimensions {
            types: file
                .types
                .into_iter()
                .map(|t| EntityTypeDimensions {
                    id: t.id,
                    type_name: t.type_name,
                    width: t.width,
                    height: t.height,
                    eye_height: t.eye_height,
                    fixed: t.fixed,
                })
                .collect(),
            by_name,
            player: file.player,
        })
    }

    pub fn len(&self) -> usize {
        self.types.len()
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    pub fn types(&self) -> &[EntityTypeDimensions] {
        &self.types
    }

    pub fn by_name(&self, name: &str) -> Option<&EntityTypeDimensions> {
        self.by_name.get(name).map(|i| &self.types[*i])
    }

    /// Player dimensions for a serialized pose name ("standing", "crouching", ...).
    pub fn player_pose(&self, pose: &str) -> Option<PlayerPoseDimensions> {
        self.player.get(pose).copied()
    }

    /// All (pose name, dimensions) pairs.
    pub fn player_poses(&self) -> &HashMap<String, PlayerPoseDimensions> {
        &self.player
    }
}

// ---------------------------------------------------------------- block_properties.json

#[derive(Deserialize)]
struct BlockPropertiesFile {
    version: u32,
    blocks: HashMap<String, BlockPropertiesEntry>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
struct BlockPropertiesEntry {
    #[serde(default = "default_friction")]
    friction: f32,
    #[serde(default = "one")]
    speed_factor: f32,
    #[serde(default = "one")]
    jump_factor: f32,
}

fn default_friction() -> f32 {
    0.6
}

fn one() -> f32 {
    1.0
}

/// Per-block movement properties (friction = slipperiness, speed/jump multipliers).
#[derive(Clone, Debug)]
pub struct BlockProperties {
    blocks: HashMap<String, (f32, f32, f32)>,
}

impl BlockProperties {
    pub fn load_dir(dir: &Path) -> Result<BlockProperties> {
        Self::load(&dir.join("block_properties.json"))
    }

    pub fn load(path: &Path) -> Result<BlockProperties> {
        let file: BlockPropertiesFile = serde_json::from_reader(std::io::BufReader::new(
            std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?,
        ))
        .with_context(|| format!("parsing {}", path.display()))?;
        check_version(file.version, "block_properties.json")?;
        Ok(BlockProperties {
            blocks: file
                .blocks
                .into_iter()
                .map(|(k, v)| (k, (v.friction, v.speed_factor, v.jump_factor)))
                .collect(),
        })
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// (friction, speed_factor, jump_factor); vanilla defaults (0.6, 1.0, 1.0) if absent.
    pub fn get(&self, block: &str) -> (f32, f32, f32) {
        self.blocks.get(block).copied().unwrap_or((0.6, 1.0, 1.0))
    }

    pub fn friction(&self, block: &str) -> f32 {
        self.get(block).0
    }
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    fn oracle_dir() -> std::path::PathBuf {
        haste_core::paths::oracle_dir()
    }

    #[test]
    fn loads_block_shapes_from_cache() {
        // needs cache/oracle/26.3/block_shapes.json (tools/recorder-mod/dump-server-oracles.sh)
        let path = oracle_dir().join("block_shapes.json");
        if !path.exists() {
            eprintln!("ignored: {} missing (run the dump script)", path.display());
            return;
        }
        let shapes = BlockShapes::load(&path).unwrap();
        assert_eq!(shapes.len(), 35_723);

        // stone: full cube, opaque
        let stone = shapes.find("minecraft:stone", &[]).unwrap();
        assert_eq!(stone.collision.len(), 1);
        assert_eq!(stone.collision[0].min, DVec3::new(0.0, 0.0, 0.0));
        assert_eq!(stone.collision[0].max, DVec3::new(1.0, 1.0, 1.0));
        assert_eq!(stone.outline, stone.collision);
        assert!(stone.opaque && !stone.air && stone.light == 0);

        // oak_slab bottom/top: half cube
        let slab = shapes
            .find("minecraft:oak_slab", &[("type", "bottom")])
            .unwrap();
        assert_eq!(slab.collision[0].max.y, 0.5);
        let slab = shapes
            .find("minecraft:oak_slab", &[("type", "top")])
            .unwrap();
        assert_eq!(slab.collision[0].min.y, 0.5);

        // torch: no collision, thin outline
        let torch = shapes.find("minecraft:torch", &[]).unwrap();
        assert!(torch.collision.is_empty());
        assert_eq!(torch.outline.len(), 1);

        // water: no collision, fluid tag
        let water = shapes.find("minecraft:water", &[("level", "0")]).unwrap();
        assert!(water.collision.is_empty());
        assert_eq!(water.fluid.as_deref(), Some("minecraft:water"));

        // by-id lookup agrees with by-name
        let by_id = shapes.state(stone.id).unwrap();
        assert_eq!(by_id.block, "minecraft:stone");
        assert!(shapes.state(BlockStateId(u32::MAX)).is_none());

        // CollisionShapes impl
        use haste_core::view::CollisionShapes;
        assert_eq!(shapes.collision_boxes(stone.id).len(), 1);
        assert_eq!(shapes.outline_boxes(torch.id).len(), 1);
    }

    #[test]
    fn loads_entity_dimensions_from_cache() {
        let path = oracle_dir().join("entity_dimensions.json");
        if !path.exists() {
            eprintln!("ignored: {} missing (run the dump script)", path.display());
            return;
        }
        let dims = EntityDimensions::load(&path).unwrap();
        assert_eq!(dims.len(), 161);
        let standing = dims.player_pose("standing").unwrap();
        assert_eq!(
            (standing.width, standing.height, standing.eye_height),
            (0.6, 1.8, 1.62)
        );
        let crouching = dims.player_pose("crouching").unwrap();
        assert_eq!((crouching.width, crouching.height), (0.6, 1.5));
        let swimming = dims.player_pose("swimming").unwrap();
        assert_eq!(swimming.height, 0.6);
        let zombie = dims.by_name("minecraft:zombie").unwrap();
        assert_eq!((zombie.width, zombie.height), (0.6, 1.95));
        assert!(dims.by_name("minecraft:does_not_exist").is_none());
    }

    #[test]
    fn loads_block_properties_from_cache() {
        let path = oracle_dir().join("block_properties.json");
        if !path.exists() {
            eprintln!("ignored: {} missing (run the dump script)", path.display());
            return;
        }
        let props = BlockProperties::load(&path).unwrap();
        assert_eq!(props.len(), 1_286);
        assert_eq!(props.friction("minecraft:stone"), 0.6);
        assert_eq!(props.friction("minecraft:ice"), 0.98);
        assert_eq!(props.friction("minecraft:slime_block"), 0.8);
        assert_eq!(props.friction("minecraft:blue_ice"), 0.989);
        // defaults for unknown blocks
        assert_eq!(props.get("minecraft:nope"), (0.6, 1.0, 1.0));
    }
}
