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
/// `ao_solid` is vanilla `getShadeBrightness <= 0.2` (see `is_ao_solid`). `render_shape_invisible`
/// is `RenderShape.INVISIBLE` (no model) and `suffocating` is `BlockState.isSuffocating` with an
/// empty level (see the two query methods below).
#[derive(Default, Clone)]
struct StateShapes {
    collision: Vec<Aabb>,
    outline: Vec<Aabb>,
    light: u8,
    air: bool,
    opaque: bool,
    fluid: Option<FluidInfo>,
    ao_solid: bool,
    render_shape_invisible: bool,
    suffocating: bool,
}

pub struct BlockShapes {
    shapes: Vec<StateShapes>,
    oracle_backed: bool,
    /// Placeholder for ids outside the table: `minecraft:stone`'s default state. A modded
    /// server can send ids past the end of the table; every lookup below resolves them to
    /// this state's entry instead of panicking.
    placeholder: BlockStateId,
}

impl BlockShapes {
    /// Table row for `state`: the id itself when in range, else the placeholder's row.
    /// One bounds check, no allocation.
    fn row(&self, state: BlockStateId) -> usize {
        let id = state.0 as usize;
        if id < self.shapes.len() {
            id
        } else {
            self.placeholder.0 as usize
        }
    }

    pub(crate) fn parse_oracle(json: &str, blocks: &BlockStates) -> anyhow::Result<Self> {
        let state_count = blocks.len();
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
            /// "invisible" | "model" (vanilla `RenderShape`, which has only those two constants
            /// in 26.3). Absent in pre-regeneration dumps: treated as "model".
            #[serde(default)]
            render_shape: Option<String>,
            /// Vanilla `BlockState.isSuffocating` with an empty level. Absent in
            /// pre-regeneration dumps: falls back to the full-collision-cube rule below.
            #[serde(default)]
            suffocating: Option<bool>,
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
            let collision = conv(&s.collision);
            let outline = conv(&s.outline);
            // Pre-regeneration dumps have no suffocating field: fall back to today's best rule,
            // the full-collision-cube half of vanilla's default predicate (the `causes_suffocation`
            // tag half is only known to haste-client through the server-synced tags, see
            // crates/haste-client/src/screen_effects/in_block.rs; glass diverges here).
            let suffocating = s
                .suffocating
                .unwrap_or_else(|| is_collision_full_block(&collision));
            shapes[idx] = StateShapes {
                ao_solid: ao_solid_for(blocks.block_of(BlockStateId(s.id)).path(), &collision),
                render_shape_invisible: s.render_shape.as_deref() == Some("invisible"),
                suffocating,
                collision,
                outline,
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
            placeholder: blocks.placeholder_state(),
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
            placeholder: blocks.placeholder_state(),
        }
    }

    pub fn is_oracle_backed(&self) -> bool {
        self.oracle_backed
    }

    /// Light emitted by the state (0..=15); 0 in the fallback. Ids outside the table
    /// resolve to the placeholder's entry.
    pub(crate) fn light_emission(&self, state: BlockStateId) -> u8 {
        self.shapes[self.row(state)].light
    }

    pub(crate) fn is_air(&self, state: BlockStateId) -> bool {
        self.shapes[self.row(state)].air
    }

    pub(crate) fn is_opaque_full(&self, state: BlockStateId) -> bool {
        self.shapes[self.row(state)].opaque
    }

    /// Whether the state darkens its neighbours' ambient occlusion: vanilla
    /// `BlockBehaviour.getShadeBrightness` returns 0.2 for a full collision cube
    /// and 1.0 otherwise. `TransparentBlock` (glass family) and the other
    /// always-1.0 overrides win over the shape test; `SnowLayerBlock` is 0.2
    /// only at layers = 8 (its full-cube state), which the shape test already
    /// says. Ids outside the table resolve to the placeholder's entry.
    pub fn is_ao_solid(&self, state: BlockStateId) -> bool {
        self.shapes[self.row(state)].ao_solid
    }

    /// Whether the state's collision shape is a full cube (vanilla
    /// `isCollisionShapeFullBlock`): decides `BlockModelLighter`'s faceCubic
    //  test for a quad that does not lie on the block boundary. Ids outside
    /// the table resolve to the placeholder's entry.
    pub fn is_collision_full(&self, state: BlockStateId) -> bool {
        is_collision_full_block(&self.shapes[self.row(state)].collision)
    }

    pub(crate) fn fluid(&self, state: BlockStateId) -> Option<FluidInfo> {
        self.shapes[self.row(state)].fluid
    }

    /// Whether the state renders no block model (vanilla `RenderShape.INVISIBLE`: air, the
    /// liquid and bubble-column family, barrier, light, structure void, end portal/gateway,
    /// moving piston). Consumers skip such blocks where vanilla does, e.g.
    /// `EntityRenderer.extractShadowPiece` when drawing entity shadows. Ids outside the table
    /// are `false` (not invisible): unlike the shape lookups above they do not resolve to the
    /// placeholder, so an unknown modded state keeps today's behaviour.
    pub fn render_shape_invisible(&self, state: BlockStateId) -> bool {
        let id = state.0 as usize;
        id < self.shapes.len() && self.shapes[id].render_shape_invisible
    }

    /// Whether an entity inside this state suffocates (vanilla `BlockState.isSuffocating`,
    /// evaluated with an empty level in the oracle dump; the path `LocalPlayer.suffocatesAt` ->
    /// `Level.collidesWithSuffocatingBlock` uses it to push the camera out of blocks). Ids
    /// outside the table are `false` (no push-out), never a panic.
    pub fn is_suffocating(&self, state: BlockStateId) -> bool {
        let id = state.0 as usize;
        id < self.shapes.len() && self.shapes[id].suffocating
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
        &self.shapes[self.row(state)].collision
    }
    fn outline_boxes(&self, state: BlockStateId) -> &[Aabb] {
        &self.shapes[self.row(state)].outline
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
    let mut shapes = fallback_shapes_inner(blocks, state);
    let path = blocks.block_of(state).path();
    shapes.ao_solid = ao_solid_for(path, &shapes.collision);
    // No tag data in haste-data: the shape half of vanilla's default suffocation predicate
    // (see parse_oracle for the tag caveat). Air and fluids never render a model in vanilla.
    shapes.suffocating = is_collision_full_block(&shapes.collision);
    shapes
}

fn fallback_shapes_inner(blocks: &BlockStates, state: BlockStateId) -> StateShapes {
    let path = blocks.block_of(state).path().to_owned();
    let path = path.as_str();
    let prop = |k: &str| blocks.property_value(state, k);
    // Air variants and fluids never collide. Fallback physics fields: air variants are air;
    // water/lava carry a full-source fluid; everything else is an opaque full cube.
    if matches!(path, "air" | "cave_air" | "void_air") {
        return StateShapes {
            air: true,
            render_shape_invisible: true,
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
            render_shape_invisible: true,
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

/// Vanilla `Block.isShapeFullBlock` on a state's collision shape: the shape
/// covers the whole unit cube. Vanilla VoxelShapes merge into non-overlapping
/// boxes inside 0..1, so that is exactly total volume 1 (a full cube is
/// overwhelmingly the single-box case, checked first).
fn is_collision_full_block(boxes: &[Aabb]) -> bool {
    if boxes.len() == 1 && is_full_cube(boxes[0]) {
        return true;
    }
    let mut volume = 0.0;
    for b in boxes {
        if b.min.x < -1e-9
            || b.min.y < -1e-9
            || b.min.z < -1e-9
            || b.max.x > 1.0 + 1e-9
            || b.max.y > 1.0 + 1e-9
            || b.max.z > 1.0 + 1e-9
        {
            return false;
        }
        let d = b.max - b.min;
        volume += d.x * d.y * d.z;
    }
    (volume - 1.0).abs() < 1e-6
}

/// Vanilla `getShadeBrightness <= 0.2` for a state: a full collision cube, minus
/// the blocks whose class overrides the brightness back to 1.0 (the glass
/// `TransparentBlock` family — `glass`, `stained_glass`, `tinted_glass`, the
/// weathering `copper_grate` — `LightBlock`, `StructureVoidBlock`,
/// `BarrierBlock`). The always-0.2 overrides (`MudBlock`, `SoulSandBlock`) are
/// full cubes anyway, so the shape test alone already covers them, and
/// `SnowLayerBlock`'s 0.2-at-layers-8 matches its full-cube shape.
fn ao_solid_for(path: &str, collision: &[Aabb]) -> bool {
    if matches!(
        path,
        "glass" | "tinted_glass" | "light" | "structure_void" | "barrier"
    ) || path.ends_with("_stained_glass")
        || path.ends_with("_copper_grate")
        || path == "copper_grate"
    {
        return false;
    }
    is_collision_full_block(collision)
}

#[cfg(test)]
mod tests {
    use super::*;
    use haste_core::view::CollisionShapes as _;

    /// air = 0, stone = 1 (the placeholder): the fallback gives stone a solid full cube.
    const BLOCKS_JSON: &str = r#"{
  "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
  "minecraft:stone": { "states": [ { "id": 1, "default": true } ] }
}"#;

    fn fallback_shapes() -> BlockShapes {
        let blocks = BlockStates::parse(BLOCKS_JSON, None).unwrap();
        BlockShapes::fallback(&blocks)
    }

    #[test]
    fn out_of_range_ids_use_the_placeholder_entry() {
        let s = fallback_shapes();
        let stone = BlockStateId(1);
        assert!(s.is_opaque_full(stone));
        assert!(!s.is_air(stone));
        assert_eq!(s.collision_boxes(stone).len(), 1);
        for id in [2u32, 100, u32::MAX] {
            let unknown = BlockStateId(id);
            assert_eq!(s.light_emission(unknown), s.light_emission(stone));
            assert_eq!(s.is_air(unknown), s.is_air(stone));
            assert_eq!(s.is_opaque_full(unknown), s.is_opaque_full(stone));
            assert_eq!(s.fluid(unknown), s.fluid(stone));
            assert_eq!(s.collision_boxes(unknown), s.collision_boxes(stone));
            assert_eq!(s.outline_boxes(unknown), s.outline_boxes(stone));
        }
    }

    #[test]
    fn oracle_backed_shapes_are_total_too() {
        let blocks = BlockStates::parse(BLOCKS_JSON, None).unwrap();
        let oracle = r#"{
  "version": 1,
  "states": [
    { "id": 0, "air": true, "collision": [], "outline": [], "light": 0, "opaque": false },
    { "id": 1, "air": false, "collision": [[0,0,0,1,1,1]], "outline": [[0,0,0,1,1,1]], "light": 0, "opaque": true }
  ]
}"#;
        let s = BlockShapes::parse_oracle(oracle, &blocks).unwrap();
        assert!(s.is_oracle_backed());
        let stone = blocks.placeholder_state();
        for id in [2u32, u32::MAX] {
            let unknown = BlockStateId(id);
            assert_eq!(s.collision_boxes(unknown), s.collision_boxes(stone));
            assert_eq!(s.outline_boxes(unknown), s.outline_boxes(stone));
            assert_eq!(s.is_opaque_full(unknown), s.is_opaque_full(stone));
            assert_eq!(s.is_air(unknown), s.is_air(stone));
            assert_eq!(s.light_emission(unknown), s.light_emission(stone));
            assert_eq!(s.fluid(unknown), s.fluid(stone));
        }
    }

    /// The AO occluder rule (vanilla `BlockBehaviour.getShadeBrightness`, read
    /// from the 26.3 source): a full collision cube darkens (0.2) unless the
    /// block's class overrides the brightness to 1.0 — glass
    /// (`TransparentBlock`) keeps a full collision cube but stays bright. A
    /// slab (half cube) and leaves' neighbors in the model sense stay bright
    /// only when the shape is not full: leaves ARE a full collision cube, so
    /// they darken like stone in vanilla.
    #[test]
    fn ao_solid_follows_the_collision_shape_not_render_opacity() {
        // 0 air, 1 stone, 2 glass, 3 oak_leaves, 4 bottom slab, 5 stained glass,
        // 6 snow (2 layers), 7 snow (8 layers).
        let blocks_json = r#"{
  "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
  "minecraft:stone": { "states": [ { "id": 1, "default": true } ] },
  "minecraft:glass": { "states": [ { "id": 2, "default": true } ] },
  "minecraft:oak_leaves": { "states": [ { "id": 3, "default": true } ] },
  "minecraft:stone_slab": { "states": [ { "id": 4, "default": true } ] },
  "minecraft:orange_stained_glass": { "states": [ { "id": 5, "default": true } ] },
  "minecraft:snow": {
    "properties": { "layers": ["2", "8"] },
    "states": [
      { "id": 6, "default": true, "properties": { "layers": "2" } },
      { "id": 7, "properties": { "layers": "8" } }
    ]
  }
}"#;
        let blocks = BlockStates::parse(blocks_json, None).unwrap();
        let oracle = r#"{
  "version": 1,
  "states": [
    { "id": 0, "air": true, "collision": [], "outline": [], "light": 0, "opaque": false },
    { "id": 1, "collision": [[0,0,0,1,1,1]], "outline": [[0,0,0,1,1,1]], "light": 0, "opaque": true },
    { "id": 2, "collision": [[0,0,0,1,1,1]], "outline": [[0,0,0,1,1,1]], "light": 0, "opaque": false },
    { "id": 3, "collision": [[0,0,0,1,1,1]], "outline": [[0,0,0,1,1,1]], "light": 0, "opaque": false },
    { "id": 4, "collision": [[0,0,0,1,0.5,1]], "outline": [[0,0,0,1,0.5,1]], "light": 0, "opaque": false },
    { "id": 5, "collision": [[0,0,0,1,1,1]], "outline": [[0,0,0,1,1,1]], "light": 0, "opaque": false },
    { "id": 6, "collision": [[0,0,0,1,0.25,1]], "outline": [[0,0,0,1,0.25,1]], "light": 0, "opaque": false },
    { "id": 7, "collision": [[0,0,0,1,1,1]], "outline": [[0,0,0,1,1,1]], "light": 0, "opaque": true }
  ]
}"#;
        let s = BlockShapes::parse_oracle(oracle, &blocks).unwrap();
        // Full collision cube, no class override: darkens.
        assert!(s.is_ao_solid(BlockStateId(1)), "stone darkens");
        // Full cube but glass (TransparentBlock): stays bright.
        assert!(!s.is_ao_solid(BlockStateId(2)), "glass stays bright");
        assert!(
            !s.is_ao_solid(BlockStateId(5)),
            "stained glass stays bright"
        );
        // Leaves: full collision cube, no override — vanilla darkens them.
        assert!(s.is_ao_solid(BlockStateId(3)), "leaves darken (full cube)");
        // Half cube: bright.
        assert!(!s.is_ao_solid(BlockStateId(4)), "slab stays bright");
        // Snow: 0.2 only at layers = 8 (the full-cube state).
        assert!(!s.is_ao_solid(BlockStateId(6)), "thin snow stays bright");
        assert!(s.is_ao_solid(BlockStateId(7)), "full snow darkens");
    }

    /// render_shape/suffocating from the oracle dump's per-state fields (the recorder mod's
    /// `stateJson`): vanilla `getRenderShape()` and `isSuffocating(EmptyBlockGetter, origin)`.
    /// Unknown ids answer false for both (no shadow skip, no camera push-out), unlike the shape
    /// lookups, which resolve to the placeholder.
    #[test]
    fn render_shape_and_suffocation_flags_from_the_oracle() {
        // 0 air, 1 stone, 2 glass, 3 barrier, 4 bottom slab.
        let blocks_json = r#"{
          "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
          "minecraft:stone": { "states": [ { "id": 1, "default": true } ] },
          "minecraft:glass": { "states": [ { "id": 2, "default": true } ] },
          "minecraft:barrier": { "states": [ { "id": 3, "default": true } ] },
          "minecraft:stone_slab": { "states": [ { "id": 4, "default": true } ] }
        }"#;
        let blocks = BlockStates::parse(blocks_json, None).unwrap();
        let oracle = r#"{
          "version": 1,
          "states": [
            { "id": 0, "air": true, "render_shape": "invisible", "suffocating": false },
            { "id": 1, "collision": [[0,0,0,1,1,1]], "render_shape": "model", "suffocating": true },
            { "id": 2, "collision": [[0,0,0,1,1,1]], "render_shape": "model", "suffocating": false },
            { "id": 3, "render_shape": "invisible", "suffocating": false },
            { "id": 4, "collision": [[0,0,0,1,0.5,1]], "render_shape": "model", "suffocating": false }
          ]
        }"#;
        let s = BlockShapes::parse_oracle(oracle, &blocks).unwrap();
        assert!(
            !s.render_shape_invisible(BlockStateId(1)),
            "stone is a model"
        );
        assert!(
            s.render_shape_invisible(BlockStateId(0)),
            "air is invisible"
        );
        assert!(
            s.render_shape_invisible(BlockStateId(3)),
            "barrier is invisible"
        );
        assert!(
            !s.render_shape_invisible(BlockStateId(2)),
            "glass is a model"
        );
        assert!(s.is_suffocating(BlockStateId(1)), "stone suffocates");
        assert!(
            !s.is_suffocating(BlockStateId(2)),
            "glass does not suffocate"
        );
        assert!(
            !s.is_suffocating(BlockStateId(4)),
            "half slab does not suffocate"
        );
        assert!(!s.is_suffocating(BlockStateId(0)), "air does not suffocate");
        // Unknown ids: false for both, no panic.
        for id in [5u32, 100, u32::MAX] {
            assert!(!s.render_shape_invisible(BlockStateId(id)));
            assert!(!s.is_suffocating(BlockStateId(id)));
        }
    }

    /// Dumps from before the render_shape/suffocating regeneration: render_shape defaults to
    /// "model" everywhere; suffocating falls back to the full-collision-cube rule (the shape
    /// half of vanilla's default predicate — glass diverges until the dump is regenerated).
    #[test]
    fn old_dumps_default_to_model_and_the_full_cube_rule() {
        // 0 air, 1 stone, 2 glass, 3 bottom slab.
        let blocks_json = r#"{
          "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
          "minecraft:stone": { "states": [ { "id": 1, "default": true } ] },
          "minecraft:glass": { "states": [ { "id": 2, "default": true } ] },
          "minecraft:stone_slab": { "states": [ { "id": 3, "default": true } ] }
        }"#;
        let blocks = BlockStates::parse(blocks_json, None).unwrap();
        let oracle = r#"{
          "version": 1,
          "states": [
            { "id": 1, "collision": [[0,0,0,1,1,1]] },
            { "id": 2, "collision": [[0,0,0,1,1,1]] },
            { "id": 3, "collision": [[0,0,0,1,0.5,1]] }
          ]
        }"#;
        let s = BlockShapes::parse_oracle(oracle, &blocks).unwrap();
        assert!(
            !s.render_shape_invisible(BlockStateId(1)),
            "stone stays a model"
        );
        assert!(s.is_suffocating(BlockStateId(1)), "full cube suffocates");
        assert!(
            s.is_suffocating(BlockStateId(2)),
            "full-cube fallback (glass diverges)"
        );
        assert!(!s.is_suffocating(BlockStateId(3)), "half cube does not");
    }

    /// Fallback table (no oracle dump): air and fluids render no model (vanilla
    /// `AirBlock`/`LiquidBlock` are RenderShape.INVISIBLE); suffocation uses the full-cube rule.
    #[test]
    fn fallback_render_shape_and_suffocation() {
        let s = fallback_shapes();
        let stone = BlockStateId(1);
        assert!(!s.render_shape_invisible(stone), "stone is a model");
        assert!(s.is_suffocating(stone), "stone suffocates (full cube)");
        for id in [2u32, 100, u32::MAX] {
            assert!(!s.render_shape_invisible(BlockStateId(id)));
            assert!(!s.is_suffocating(BlockStateId(id)));
        }
        // Air and fluids: no model (the fallback knows their paths directly).
        let blocks_json = r#"{
          "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
          "minecraft:water": { "states": [ { "id": 1, "default": true } ] },
          "minecraft:lava": { "states": [ { "id": 2, "default": true } ] }
        }"#;
        let blocks = BlockStates::parse(blocks_json, None).unwrap();
        let f = BlockShapes::fallback(&blocks);
        for id in [0u32, 1, 2] {
            assert!(f.render_shape_invisible(BlockStateId(id)), "air/fluid {id}");
            assert!(!f.is_suffocating(BlockStateId(id)), "no collision");
        }
    }
}
