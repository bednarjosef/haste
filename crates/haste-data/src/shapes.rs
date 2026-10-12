//! Collision and outline shapes per block state.
//!
//! Oracle-backed when `cache/oracle/26.3/block_shapes.json` exists; otherwise the documented
//! fallback heuristic from docs/INTERFACES.md, which is approximate (see NOTES.md). The oracle
//! file replaces it once oracles records the dump.

use haste_core::view::{FluidInfo, FluidKind};
use haste_core::{Aabb, BlockStateId, DVec3};

use crate::blocks::BlockStates;

/// Per-state shape entry (empty = no collision). `light`/`air`/`opaque`/`fluid` come from the
/// oracle dump's per-state fields (or fallback heuristics) and back `BlockPhysics` on `GameData`.
#[derive(Default, Clone)]
struct StateShapes {
    collision: Vec<Aabb>,
    outline: Vec<Aabb>,
    light: u8,
    air: bool,
    opaque: bool,
    fluid: Option<FluidInfo>,
}

pub struct BlockShapes {
    shapes: Vec<StateShapes>,
    oracle_backed: bool,
}

impl BlockShapes {
    pub(crate) fn parse_oracle(json: &str, state_count: usize) -> anyhow::Result<Self> {
        // Deserializing into Vec<OracleState> buffers all ~903k floats as f64 before we convert;
        // that double-buffering is a large part of the 30 MB file's cost (see NOTES.md).
        #[derive(serde::Deserialize)]
        struct OracleFile {
            version: u32,
            states: Vec<OracleState>,
        }
        #[derive(serde::Deserialize)]
        struct OracleState {
            id: u32,
            #[serde(default)]
            collision: Vec<[f64; 6]>,
            #[serde(default)]
            outline: Vec<[f64; 6]>,
            #[serde(default)]
            light: u8,
            #[serde(default)]
            air: bool,
            #[serde(default)]
            opaque: bool,
            /// Fluid id (`minecraft:water`, `minecraft:flowing_water`, `minecraft:lava`,
            /// `minecraft:flowing_lava`) for fluid states and waterlogged blocks; null otherwise.
            #[serde(default)]
            fluid: Option<String>,
            /// `properties` includes `level` for fluid states; absent for waterlogged blocks
            /// (their fluid is a full source).
            #[serde(default)]
            properties: std::collections::HashMap<String, String>,
        }
        let file: OracleFile =
            serde_json::from_str(json).map_err(|e| anyhow::anyhow!("block_shapes.json: {e}"))?;
        if file.version != 1 {
            anyhow::bail!("block_shapes.json: unsupported version {}", file.version);
        }
        let mut shapes = vec![StateShapes::default(); state_count];
        for s in &file.states {
            let idx = s.id as usize;
            if idx >= state_count {
                anyhow::bail!(
                    "block_shapes.json: state id {} out of range (0..{state_count})",
                    s.id
                );
            }
            let conv = |lists: &[[f64; 6]]| -> Vec<Aabb> {
                lists
                    .iter()
                    .map(|b| Aabb {
                        min: DVec3::new(b[0], b[1], b[2]),
                        max: DVec3::new(b[3], b[4], b[5]),
                    })
                    .collect()
            };
            shapes[idx] = StateShapes {
                collision: conv(&s.collision),
                outline: conv(&s.outline),
                light: s.light,
                air: s.air,
                opaque: s.opaque,
                fluid: s.fluid.as_deref().and_then(|f| {
                    let kind = match f {
                        "minecraft:water" | "minecraft:flowing_water" => FluidKind::Water,
                        "minecraft:lava" | "minecraft:flowing_lava" => FluidKind::Lava,
                        _ => return None,
                    };
                    let height = fluid_height(s.properties.get("level").map(String::as_str));
                    Some(FluidInfo { kind, height })
                }),
            };
        }
        Ok(Self {
            shapes,
            oracle_backed: true,
        })
    }

    /// Fallback heuristic from docs/INTERFACES.md. Approximate by design.
    pub(crate) fn fallback(blocks: &BlockStates) -> Self {
        let mut shapes = Vec::with_capacity(blocks.len());
        for id in 0..blocks.len() as u32 {
            let state = BlockStateId(id);
            shapes.push(fallback_shapes(blocks, state));
        }
        Self {
            shapes,
            oracle_backed: false,
        }
    }

    pub fn is_oracle_backed(&self) -> bool {
        self.oracle_backed
    }

    /// Light emitted by the state (0..=15); 0 in the fallback.
    pub(crate) fn light_emission(&self, state: BlockStateId) -> u8 {
        self.shapes[state.0 as usize].light
    }

    pub(crate) fn is_air(&self, state: BlockStateId) -> bool {
        self.shapes[state.0 as usize].air
    }

    pub(crate) fn is_opaque_full(&self, state: BlockStateId) -> bool {
        self.shapes[state.0 as usize].opaque
    }

    pub(crate) fn fluid(&self, state: BlockStateId) -> Option<FluidInfo> {
        self.shapes[state.0 as usize].fluid
    }
}

/// Vanilla fluid surface height from the `level` property: level 0 (source) = 8/9,
/// levels 1..=7 = (8 - level)/9, level >= 8 (falling) = 8/9 like a source (settled on the
/// vanilla server, NOTES.md; a column with the same fluid above is full height, which the
/// caller decides). Waterlogged blocks (no `level` property) are full sources: 8/9.
fn fluid_height(level: Option<&str>) -> f32 {
    match level.and_then(|l| l.parse::<u32>().ok()) {
        Some(l @ 1..=7) => (8 - l) as f32 / 9.0,
        _ => 8.0 / 9.0,
    }
}

impl haste_core::view::CollisionShapes for BlockShapes {
    fn collision_boxes(&self, state: BlockStateId) -> &[Aabb] {
        &self.shapes[state.0 as usize].collision
    }
    fn outline_boxes(&self, state: BlockStateId) -> &[Aabb] {
        &self.shapes[state.0 as usize].outline
    }
}

const FULL_CUBE: Aabb = Aabb {
    min: DVec3::new(0.0, 0.0, 0.0),
    max: DVec3::new(1.0, 1.0, 1.0),
};

/// Keywords from docs/INTERFACES.md for blocks that are not full cubes in the fallback.
const KEYWORDS: &[&str] = &[
    "grass",
    "fern",
    "flower",
    "sapling",
    "torch",
    "rail",
    "button",
    "lever",
    "pressure_plate",
    "carpet",
    "sign",
    "banner",
    "vine",
    "bush",
    "mushroom",
    "wheat",
    "carrot",
    "potato",
    "beetroot",
    "sugar_cane",
    "kelp",
    "seagrass",
    "lily",
    "dead_bush",
    "cobweb",
    "redstone_wire",
    "tripwire",
    "string",
    "cave_vines",
];

fn fallback_shapes(blocks: &BlockStates, state: BlockStateId) -> StateShapes {
    let path = blocks.block_of(state).path();
    let prop = |k: &str| blocks.property_value(state, k);
    // Air variants and fluids never collide. Fallback physics fields: air variants are air;
    // water/lava carry a full-source fluid; everything else is an opaque full cube.
    if matches!(path, "air" | "cave_air" | "void_air") {
        return StateShapes {
            air: true,
            ..Default::default()
        };
    }
    if matches!(path, "water" | "lava" | "bubble_column") {
        let kind = if path == "lava" {
            FluidKind::Lava
        } else {
            FluidKind::Water
        };
        let height = fluid_height(prop("level"));
        return StateShapes {
            fluid: Some(FluidInfo { kind, height }),
            ..Default::default()
        };
    }
    // Snow: 1/8 per layer.
    if path == "snow" {
        let layers = prop("layers")
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(1.0);
        let aabb = Aabb {
            min: DVec3::new(0.0, 0.0, 0.0),
            max: DVec3::new(1.0, layers / 8.0, 1.0),
        };
        return StateShapes {
            collision: vec![aabb],
            outline: vec![aabb],
            opaque: is_full_cube(aabb),
            ..Default::default()
        };
    }
    // Slabs: half cube (property is `type` in 26.3: top/bottom/double).
    if path.ends_with("_slab") {
        let ty = prop("type").unwrap_or("bottom");
        let aabb = match ty {
            "top" => Aabb {
                min: DVec3::new(0.0, 0.5, 0.0),
                max: DVec3::new(1.0, 1.0, 1.0),
            },
            "double" => FULL_CUBE,
            _ => Aabb {
                min: DVec3::new(0.0, 0.0, 0.0),
                max: DVec3::new(1.0, 0.5, 1.0),
            },
        };
        return StateShapes {
            collision: vec![aabb],
            outline: vec![aabb],
            opaque: is_full_cube(aabb),
            ..Default::default()
        };
    }
    // Stairs: approximate as a half cube by `half`.
    if path.ends_with("_stairs") {
        let top = prop("half") == Some("top");
        let aabb = if top {
            Aabb {
                min: DVec3::new(0.0, 0.5, 0.0),
                max: DVec3::new(1.0, 1.0, 1.0),
            }
        } else {
            Aabb {
                min: DVec3::new(0.0, 0.0, 0.0),
                max: DVec3::new(1.0, 0.5, 1.0),
            }
        };
        return StateShapes {
            collision: vec![aabb],
            outline: vec![aabb],
            opaque: is_full_cube(aabb),
            ..Default::default()
        };
    }
    // Pointed dripstone: half cube by vertical_direction.
    if path == "pointed_dripstone" {
        let down = prop("vertical_direction") == Some("down");
        let aabb = if down {
            Aabb {
                min: DVec3::new(0.0, 0.0, 0.0),
                max: DVec3::new(1.0, 0.5, 1.0),
            }
        } else {
            Aabb {
                min: DVec3::new(0.0, 0.5, 0.0),
                max: DVec3::new(1.0, 1.0, 1.0),
            }
        };
        return StateShapes {
            collision: vec![aabb],
            outline: vec![aabb],
            opaque: is_full_cube(aabb),
            ..Default::default()
        };
    }
    if KEYWORDS.iter().any(|k| path.contains(k)) {
        // Plants, rails and the like: no collision in the fallback.
        return StateShapes::default();
    }
    StateShapes {
        collision: vec![FULL_CUBE],
        outline: vec![FULL_CUBE],
        opaque: true,
        ..Default::default()
    }
}

/// Full 1x1x1 cube (slab "double").
fn is_full_cube(a: Aabb) -> bool {
    let d = a.max - a.min;
    (d.x - 1.0).abs() < 1e-9 && (d.y - 1.0).abs() < 1e-9 && (d.z - 1.0).abs() < 1e-9
}
