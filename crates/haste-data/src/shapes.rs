//! Collision and outline shapes per block state.
//!
//! Oracle-backed when `cache/oracle/26.3/block_shapes.json` exists; otherwise the documented
//! fallback heuristic from docs/INTERFACES.md, which is approximate (see NOTES.md). The oracle
//! file replaces it once oracles records the dump.

use haste_core::view::{FluidInfo, FluidKind};
use haste_core::{Aabb, BlockStateId, DVec3, Direction};

use crate::blocks::BlockStates;

mod face_occlusion;

use face_occlusion::{FaceOcclusion, OcclusionTable};

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
    /// `occlusion` dump field, empty when the dump predates it (see [`OcclusionTable`]).
    occlusion: OcclusionTable,
    /// The light engines' shape-occlusion table over the occlusion shapes and the
    /// `can_occlude`/`use_shape_for_light_occlusion` flags (see [`FaceOcclusion`]).
    face_occlusion: FaceOcclusion,
    /// `light_dampening` dump field per state ([`DAMPENING_ABSENT`] without the field).
    light_dampening: Vec<u8>,
    /// `propagates_skylight_down` per state: 0 false, 1 true, [`SKYLIGHT_ABSENT`] without
    /// the field.
    skylight_down: Vec<u8>,
}

/// `light_dampening` is 0..=15; this marks a state the field is absent for.
const DAMPENING_ABSENT: u8 = u8::MAX;
/// `skylight_down` encoding for a state the field is absent for.
const SKYLIGHT_ABSENT: u8 = 2;

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
    /// Vanilla `BlockState.getOcclusionShape`. Absent in pre-regeneration dumps.
    #[serde(default)]
    occlusion: Option<Vec<[f64; 6]>>,
    /// Vanilla `BlockState.getLightDampening` (0..=15). Absent in
    /// pre-regeneration dumps.
    #[serde(default)]
    light_dampening: Option<u8>,
    /// Vanilla `BlockState.propagatesSkylightDown`. Absent in
    /// pre-regeneration dumps.
    #[serde(default)]
    propagates_skylight_down: Option<bool>,
    /// Vanilla `BlockState.canOcclude` / `useShapeForLightOcclusion`: the light engine's
    /// gates for shape-based light occlusion (`LightEngine.isEmptyShape`). Absent in
    /// dumps from before the flags were recorded.
    #[serde(default)]
    can_occlude: Option<bool>,
    #[serde(default)]
    use_shape_for_light_occlusion: Option<bool>,
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
        let file: OracleFile =
            serde_json::from_str(json).map_err(|e| anyhow::anyhow!("block_shapes.json: {e}"))?;
        if file.version != 1 {
            anyhow::bail!("block_shapes.json: unsupported version {}", file.version);
        }
        let mut shapes = vec![StateShapes::default(); state_count];
        // The three optional fields are all-or-nothing per dump in practice (the recorder
        // writes them unconditionally), but each state is checked on its own so a partial
        // dump still answers for the states it has.
        let mut occlusion = OcclusionTable::default();
        let mut dampening = Vec::new();
        let mut skylight = Vec::new();
        if file.states.iter().any(|s| s.occlusion.is_some()) {
            occlusion.ranges = vec![face_occlusion::OCCLUSION_ABSENT; state_count];
            let mut intern = std::collections::HashMap::with_capacity(128);
            for s in &file.states {
                if let Some(list) = &s.occlusion {
                    occlusion.intern(s.id, list, &mut intern);
                }
            }
        }
        if file.states.iter().any(|s| s.light_dampening.is_some()) {
            dampening = vec![DAMPENING_ABSENT; state_count];
        }
        if file
            .states
            .iter()
            .any(|s| s.propagates_skylight_down.is_some())
        {
            skylight = vec![SKYLIGHT_ABSENT; state_count];
        }
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
            if let Some(v) = s.light_dampening {
                dampening[idx] = v;
            }
            if let Some(v) = s.propagates_skylight_down {
                skylight[idx] = u8::from(v);
            }
        }
        let face_occlusion = FaceOcclusion::build(&occlusion, &file.states, state_count);
        Ok(Self {
            shapes,
            oracle_backed: true,
            placeholder: blocks.placeholder_state(),
            occlusion,
            face_occlusion,
            light_dampening: dampening,
            skylight_down: skylight,
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
            occlusion: OcclusionTable::default(),
            face_occlusion: FaceOcclusion::absent(),
            light_dampening: Vec::new(),
            skylight_down: Vec::new(),
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

    /// The state's occlusion shape (vanilla `BlockState.getOcclusionShape`): what face
    /// culling (`Block.shouldRenderFace`, `FluidRenderer.isFaceOccludedByNeighbor`) and
    /// light occlusion compare against. `None` when the dump predates the field — an empty
    /// slice is a real shape (the `noOcclusion` family occludes nothing). Ids outside the
    /// table resolve to the placeholder's entry like the shape lookups above.
    pub fn occlusion_boxes(&self, state: BlockStateId) -> Option<&[Aabb]> {
        self.occlusion.boxes_of(self.row(state))
    }

    /// Vanilla `LightEngine.shapeOccludes(fromState, toState, direction)`: whether the
    /// two states' face occlusion shapes block light travelling `from -> to` along
    /// `direction` (a bottom slab blocks down through its bottom face but not sideways
    /// through its open half). False on old dumps without the flags, which is the
    /// pre-shape behaviour. Ids outside the table resolve to the placeholder's entry.
    pub fn shape_occludes(
        &self,
        from: BlockStateId,
        to: BlockStateId,
        direction: Direction,
    ) -> bool {
        self.face_occlusion
            .occludes(self.row(from), self.row(to), direction)
    }

    /// Vanilla `LightEngine.isEmptyShape`: `!canOcclude || !useShapeForLightOcclusion`,
    /// the gate that decides a state's shape never occludes light. True on old dumps.
    pub fn is_empty_shape(&self, state: BlockStateId) -> bool {
        self.face_occlusion.is_empty_shape(self.row(state))
    }

    /// Vanilla `BlockState.useShapeForLightOcclusion` (the flag half of
    /// `LightEngine.hasDifferentLightProperties`): `None` on old dumps.
    pub fn use_shape_for_light_occlusion(&self, state: BlockStateId) -> Option<bool> {
        self.face_occlusion.use_shape(self.row(state))
    }

    /// Vanilla `BlockState.getLightDampening` (0..=15, `BlockStateBase.initCache`'s cached
    /// value): 15 for solid-render states, else 0 when skylight propagates down through the
    /// shape, 1 otherwise, with class overrides already in. `None` when the dump predates
    /// the field (the caller keeps its approximation). Ids outside the table resolve to
    /// the placeholder's entry.
    pub fn light_dampening(&self, state: BlockStateId) -> Option<u8> {
        let value = *self.light_dampening.get(self.row(state))?;
        (value != DAMPENING_ABSENT).then_some(value)
    }

    /// Vanilla `BlockState.propagatesSkylightDown`: a non-full shape (or a class override
    /// like `TransparentBlock` for glass) lets skylight through unchanged. `None` when the
    /// dump predates the field. Ids outside the table resolve to the placeholder's entry.
    pub fn propagates_skylight_down(&self, state: BlockStateId) -> Option<bool> {
        match self.skylight_down.get(self.row(state)) {
            Some(&0) => Some(false),
            Some(&1) => Some(true),
            _ => None,
        }
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
mod tests;
