//! Interim loader for the WS8 oracle dumps (`block_shapes.json`, `block_properties.json`), so that
//! the physics can be checked against real shapes and replayed against traces before `haste-data`
//! (WS2) provides `GameData`. Once `GameData` exists, `BlockPhysics` should be implemented for it
//! and this module reduced to test support (see NOTES.md).
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::{bail, Context};
use haste_core::view::CollisionShapes;
use haste_core::{Aabb, BlockStateId, DVec3};
use serde::Deserialize;

use crate::{BlockPhysics, FluidInfo, FluidKind};

#[derive(Deserialize)]
struct ShapesFile {
    version: u32,
    states: Vec<StateEntry>,
}

#[derive(Deserialize)]
struct StateEntry {
    id: u32,
    block: String,
    #[serde(default)]
    properties: BTreeMap<String, String>,
    #[serde(default)]
    collision: Vec<[f64; 6]>,
    #[serde(default)]
    outline: Vec<[f64; 6]>,
    #[serde(default)]
    fluid: Option<String>,
    #[serde(default)]
    light: u8,
    #[serde(default)]
    air: bool,
    #[serde(default)]
    opaque: bool,
}

#[derive(Deserialize)]
struct PropertiesFile {
    version: u32,
    blocks: HashMap<String, BlockProps>,
}

#[derive(Deserialize, Clone, Copy)]
struct BlockProps {
    friction: f32,
    speed_factor: f32,
    jump_factor: f32,
}

impl Default for BlockProps {
    fn default() -> Self {
        Self { friction: 0.6, speed_factor: 1.0, jump_factor: 1.0 }
    }
}

struct StateInfo {
    block: u32,
    properties: Vec<(String, String)>,
    collision: Vec<Aabb>,
    outline: Vec<Aabb>,
    fluid: Option<FluidInfo>,
    light: u8,
    air: bool,
    opaque: bool,
}

/// Block states with shapes, fluids and movement properties from the oracle dumps.
pub struct OracleBlocks {
    states: Vec<StateInfo>,
    block_names: Vec<String>,
    block_props: Vec<BlockProps>,
    by_name: HashMap<String, Vec<BlockStateId>>,
}

fn to_aabb(b: &[f64; 6]) -> Aabb {
    Aabb { min: DVec3::new(b[0], b[1], b[2]), max: DVec3::new(b[3], b[4], b[5]) }
}

/// Fluid surface height for a fluid `level` property (wiki "Water": level 0 is 8/9 high, each
/// level lower by 1/9; falling fluid (level >= 8) is treated as 8/9).
fn fluid_height(level: Option<&str>) -> f32 {
    match level.and_then(|l| l.parse::<u32>().ok()) {
        Some(l @ 1..=7) => (8 - l) as f32 / 9.0,
        _ => 8.0 / 9.0,
    }
}

impl OracleBlocks {
    /// Loads `block_shapes.json` and `block_properties.json` from `dir`
    /// (normally `cache/oracle/26.3`).
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let shapes_path = dir.join("block_shapes.json");
        let props_path = dir.join("block_properties.json");
        let shapes: ShapesFile = serde_json::from_slice(&std::fs::read(&shapes_path).with_context(|| format!("reading {}", shapes_path.display()))?)
            .with_context(|| format!("parsing {}", shapes_path.display()))?;
        if shapes.version != 1 {
            bail!("block_shapes.json version {} not supported", shapes.version);
        }
        let props: PropertiesFile = serde_json::from_slice(&std::fs::read(&props_path).with_context(|| format!("reading {}", props_path.display()))?)
            .with_context(|| format!("parsing {}", props_path.display()))?;
        if props.version != 1 {
            bail!("block_properties.json version {} not supported", props.version);
        }

        let max_id = shapes.states.iter().map(|s| s.id).max().unwrap_or(0) as usize;
        let mut states: Vec<StateInfo> = (0..=max_id)
            .map(|_| StateInfo { block: 0, properties: Vec::new(), collision: Vec::new(), outline: Vec::new(), fluid: None, light: 0, air: false, opaque: false })
            .collect();
        let mut block_names = Vec::new();
        let mut block_index: HashMap<String, u32> = HashMap::new();
        let mut by_name: HashMap<String, Vec<BlockStateId>> = HashMap::new();
        for s in shapes.states {
            let idx = *block_index.entry(s.block.clone()).or_insert_with(|| {
                block_names.push(s.block.clone());
                (block_names.len() - 1) as u32
            });
            by_name.entry(s.block.clone()).or_default().push(BlockStateId(s.id));
            let fluid = s.fluid.as_deref().and_then(|f| {
                let kind = match f {
                    "minecraft:water" | "minecraft:flowing_water" => FluidKind::Water,
                    "minecraft:lava" | "minecraft:flowing_lava" => FluidKind::Lava,
                    _ => return None,
                };
                Some(FluidInfo { kind, height: fluid_height(s.properties.get("level").map(String::as_str)) })
            });
            states[s.id as usize] = StateInfo {
                block: idx,
                properties: s.properties.into_iter().collect(),
                collision: s.collision.iter().map(to_aabb).collect(),
                outline: s.outline.iter().map(to_aabb).collect(),
                fluid,
                light: s.light,
                air: s.air,
                opaque: s.opaque,
            };
        }
        let block_props = block_names.iter().map(|n| props.blocks.get(n).copied().unwrap_or_default()).collect();
        Ok(Self { states, block_names, block_props, by_name })
    }

    pub fn len(&self) -> usize {
        self.states.len()
    }
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }
    pub fn block_of(&self, s: BlockStateId) -> Option<&str> {
        self.states.get(s.0 as usize).map(|i| self.block_names[i.block as usize].as_str())
    }
    pub fn properties(&self, s: BlockStateId) -> &[(String, String)] {
        self.states.get(s.0 as usize).map(|i| i.properties.as_slice()).unwrap_or(&[])
    }
    /// First state of `block` whose properties include all of `props`.
    pub fn state(&self, block: &str, props: &[(&str, &str)]) -> Option<BlockStateId> {
        self.by_name.get(block)?.iter().copied().find(|s| {
            let have = self.properties(*s);
            props.iter().all(|(k, v)| have.iter().any(|(hk, hv)| hk == k && hv == v))
        })
    }
    /// Same as `CollisionShapes::collision_boxes`, without needing the trait in scope.
    pub fn collision_boxes_of(&self, s: BlockStateId) -> &[Aabb] {
        self.states.get(s.0 as usize).map(|i| i.collision.as_slice()).unwrap_or(&[])
    }
    fn props_of(&self, s: BlockStateId) -> BlockProps {
        self.states.get(s.0 as usize).map(|i| self.block_props[i.block as usize]).unwrap_or_default()
    }
}

impl CollisionShapes for OracleBlocks {
    fn collision_boxes(&self, state: BlockStateId) -> &[Aabb] {
        self.states.get(state.0 as usize).map(|i| i.collision.as_slice()).unwrap_or(&[])
    }
    fn outline_boxes(&self, state: BlockStateId) -> &[Aabb] {
        self.states.get(state.0 as usize).map(|i| i.outline.as_slice()).unwrap_or(&[])
    }
}

impl BlockPhysics for OracleBlocks {
    fn friction(&self, s: BlockStateId) -> f32 {
        self.props_of(s).friction
    }
    fn speed_factor(&self, s: BlockStateId) -> f32 {
        self.props_of(s).speed_factor
    }
    fn jump_factor(&self, s: BlockStateId) -> f32 {
        self.props_of(s).jump_factor
    }
    fn fluid(&self, s: BlockStateId) -> Option<FluidInfo> {
        self.states.get(s.0 as usize).and_then(|i| i.fluid)
    }
    fn light_emission(&self, s: BlockStateId) -> u8 {
        self.states.get(s.0 as usize).map_or(0, |i| i.light)
    }
    fn is_air(&self, s: BlockStateId) -> bool {
        self.states.get(s.0 as usize).is_none_or(|i| i.air)
    }
    fn is_opaque_full(&self, s: BlockStateId) -> bool {
        self.states.get(s.0 as usize).is_some_and(|i| i.opaque)
    }
}

/// The workspace's `cache/oracle/26.3` (the worktree symlink) if present, else
/// `haste_core::paths::oracle_dir()`.
pub fn default_oracle_dir() -> std::path::PathBuf {
    let local = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cache/oracle").join(haste_core::paths::MC_VERSION);
    if local.is_dir() {
        local
    } else {
        haste_core::paths::oracle_dir()
    }
}
