//! Water and lava meshing.
//!
//! Behaviour from minecraft.wiki ("Water", "Lava", "Fluid") and screenshots of the real game;
//! the rules implemented:
//!
//! - Fluid per block state: `water`/`lava` with `level` 0 (source, height 8/9), 1..7 (flowing,
//!   height (8 - level)/9), 8..15 (falling, height 8/9); `waterlogged=true` states and the
//!   always-underwater plants (`kelp`, `kelp_plant`, `seagrass`, `tall_seagrass`,
//!   `bubble_column`) hold a water source.
//! - Column height for corner averaging: 1.0 when the same fluid is also above it, else its own
//!   height; a non-fluid block counts as 0 (it pulls the corner down) unless it is "solid", in
//!   which case it is ignored. Corner height: 1.0 if either side column is full; otherwise a
//!   weighted average of the block, the two side columns and (when a side holds fluid) the
//!   diagonal column, where columns of height >= 0.8 weigh 10 and lower ones 1.
//!   APPROXIMATE: "solid" is "full opaque cube, or any block with a model outside the cutout
//!   layers" (vanilla uses the block's blocks-motion flag).
//! - Faces: the top is drawn unless the same fluid is above (and also drawn reversed, so it is
//!   visible from inside the fluid); sides are culled against the same fluid and full opaque
//!   blocks and drawn on both sides; the bottom is drawn unless the same fluid or a full
//!   opaque block is below. APPROXIMATE: partial blocks (slabs, stairs) never cull a fluid
//!   face.
//! - Textures: still sprite on the bottom and on tops without flow; flowing tops use the flow
//!   sprite rotated so the texture runs along the flow (the downhill gradient of the corner
//!   heights), sampling the middle half of the 32px flow sprite; sides use the flow sprite's
//!   left half, cut at the surface height. Water sides next to glass or leaves use
//!   `water_overlay` (single-sided). Animated sprites show their first frame (the atlas has no
//!   animation yet).
//! - Light: the brighter of the block's own cell and the cell above per channel (bottom: the
//!   cell below and its own), no ambient occlusion; directional face shade like blocks.
//! - Water is tinted by the blended biome water colour and goes to the translucent layer;
//!   lava is untinted and solid.

use haste_assets::{BakedBlockModels, Direction, RenderLayer};
use haste_core::view::BlockStateTable;
use haste_core::{BlockPos, BlockStateId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fluid {
    Water,
    Lava,
}

/// How a fluid is tinted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FluidTint {
    /// No tint (lava).
    None,
    /// Biome water colour blend (water).
    BiomeWater,
}

/// A fluid's rendering properties (the `Fluids` registry values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FluidDef {
    pub tint: FluidTint,
    /// Render layer the fluid draws in.
    pub layer: RenderLayer,
    /// Whether sides facing glass/leaves use the water overlay sprite.
    pub overlay: bool,
    pub still: u16,
    pub flow: u16,
}

/// Registry of fluids: fluid id -> rendering properties. Water and lava are
/// registered by [`Fluids::vanilla`]; mods add their own fluids here.
#[derive(Clone, Copy, Debug)]
pub struct Fluids {
    water: FluidDef,
    lava: FluidDef,
}

impl Fluids {
    /// The two vanilla fluids, with their atlas sprites.
    pub fn vanilla(sprites: &FluidSprites) -> Fluids {
        Fluids {
            water: FluidDef {
                tint: FluidTint::BiomeWater,
                layer: RenderLayer::Translucent,
                overlay: true,
                still: sprites.water_still,
                flow: sprites.water_flow,
            },
            lava: FluidDef {
                tint: FluidTint::None,
                layer: RenderLayer::Solid,
                overlay: false,
                still: sprites.lava_still,
                flow: sprites.lava_flow,
            },
        }
    }

    /// Rendering properties of a fluid.
    pub fn def(&self, fluid: Fluid) -> FluidDef {
        match fluid {
            Fluid::Water => self.water,
            Fluid::Lava => self.lava,
        }
    }
}

impl Default for Fluids {
    fn default() -> Fluids {
        Fluids::vanilla(&FluidSprites::default())
    }
}

/// Decides which fluid a block state holds (resolved once per state at load).
///
/// The generic vanilla rules are: any state with `waterlogged=true` holds
/// water; the liquid blocks (`water`/`lava`) use their `level` property for
/// the height. Blocks that always hold a fluid source (kelp, seagrass, bubble
/// column, ...) are registered in [`FluidRules::vanilla`]; mods add more with
/// [`FluidRules::register_block`].
#[derive(Clone, Debug)]
pub struct FluidRules {
    /// Block path -> fluid, for blocks that always hold a fluid source.
    sources: std::collections::HashMap<&'static str, Fluid>,
}

impl FluidRules {
    /// The vanilla rules: the always-water plants.
    pub fn vanilla() -> FluidRules {
        let mut rules = FluidRules {
            sources: std::collections::HashMap::new(),
        };
        for block in [
            "kelp",
            "kelp_plant",
            "seagrass",
            "tall_seagrass",
            "bubble_column",
        ] {
            rules.register_block(block, Fluid::Water);
        }
        rules
    }

    /// Register a block that always holds a fluid source (mods).
    pub fn register_block(&mut self, block: &'static str, fluid: Fluid) {
        self.sources.insert(block, fluid);
    }

    fn fluid_of(&self, name: &str) -> Option<Fluid> {
        self.sources.get(name).copied()
    }
}

impl Default for FluidRules {
    fn default() -> FluidRules {
        FluidRules::vanilla()
    }
}

/// The fluid a block state holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FluidState {
    pub fluid: Fluid,
    /// Surface height of the block alone, 0..=8/9.
    pub height: f32,
}

/// Neighbour classes the fluid mesher needs per block state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Class {
    /// Water sides facing it use the overlay sprite (glass, leaves).
    overlay: bool,
}

/// Atlas sprite indices of the fluid textures.
#[derive(Clone, Copy, Debug, Default)]
pub struct FluidSprites {
    pub water_still: u16,
    pub water_flow: u16,
    pub water_overlay: u16,
    pub lava_still: u16,
    pub lava_flow: u16,
}

impl FluidSprites {
    pub fn from_atlas(atlas: &dyn haste_assets::AtlasRegions) -> FluidSprites {
        let s = |name: &str| atlas.region_of(name).sprite as u16;
        FluidSprites {
            water_still: s("minecraft:block/water_still"),
            water_flow: s("minecraft:block/water_flow"),
            water_overlay: s("minecraft:block/water_overlay"),
            lava_still: s("minecraft:block/lava_still"),
            lava_flow: s("minecraft:block/lava_flow"),
        }
    }
}

/// Per block state: its fluid and how fluids treat it as a neighbour.
pub struct FluidTable {
    fluids: Vec<Option<FluidState>>,
    classes: Vec<Class>,
    /// Fluid rendering properties (sprites, tint, overlay, layer).
    pub defs: Fluids,
    pub sprites: FluidSprites,
}

fn level_height(level: u8) -> f32 {
    if level == 0 || level >= 8 {
        8.0 / 9.0
    } else {
        (8 - level) as f32 / 9.0
    }
}

impl FluidTable {
    pub fn new(
        states: &dyn BlockStateTable,
        defs: &Fluids,
        rules: &FluidRules,
        sprites: FluidSprites,
    ) -> FluidTable {
        let n = states.len();
        let mut fluids = Vec::with_capacity(n);
        let mut classes = Vec::with_capacity(n);
        for s in 0..n as u32 {
            let state = BlockStateId(s);
            let name = states.block_of(state).path();
            let fluid = if states.property_value(state, "waterlogged") == Some("true") {
                Some((Fluid::Water, 0))
            } else if let Some(level) = states
                .property_value(state, "level")
                .and_then(|v| v.parse::<u8>().ok())
            {
                match name {
                    "water" => Some((Fluid::Water, level)),
                    "lava" => Some((Fluid::Lava, level)),
                    _ => None,
                }
            } else {
                rules.fluid_of(name).map(|f| (f, 0))
            };
            fluids.push(fluid.map(|(fluid, level)| FluidState {
                fluid,
                height: level_height(level),
            }));
            classes.push(Class {
                overlay: name.ends_with("glass")
                    || name.ends_with("glass_pane")
                    || name.ends_with("leaves"),
            });
        }
        FluidTable {
            fluids,
            classes,
            defs: *defs,
            sprites,
        }
    }

    /// A table with no fluids (tests, before the block table is known).
    pub fn empty() -> FluidTable {
        FluidTable {
            fluids: Vec::new(),
            classes: Vec::new(),
            defs: Fluids::default(),
            sprites: FluidSprites::default(),
        }
    }

    /// A table from explicit per-state fluids (tests).
    pub fn from_fluids(fluids: Vec<Option<FluidState>>) -> FluidTable {
        let classes = vec![Class::default(); fluids.len()];
        FluidTable {
            fluids,
            classes,
            defs: Fluids::default(),
            sprites: FluidSprites::default(),
        }
    }

    pub fn fluid(&self, state: BlockStateId) -> Option<FluidState> {
        self.fluids.get(state.0 as usize).copied().flatten()
    }

    fn overlay(&self, state: BlockStateId) -> bool {
        self.classes
            .get(state.0 as usize)
            .is_some_and(|c| c.overlay)
    }
}

/// What the fluid mesher reads around a block (the mesh snapshot, or a test grid).
pub trait FluidWorld {
    fn state(&self, p: BlockPos) -> BlockStateId;
    /// Full opaque cube (culls fluid faces).
    fn opaque(&self, p: BlockPos) -> bool;
    /// Ignored by corner averaging (vanilla: blocks motion).
    fn solid(&self, p: BlockPos) -> bool;
}

/// Whether a baked block counts as solid for corner averaging (see the module docs).
pub fn baked_solid(baked: &BakedBlockModels, state: BlockStateId) -> bool {
    state != BlockStateId::AIR
        && (baked.is_full_opaque(state)
            || (!baked.quads(state).is_empty()
                && !matches!(
                    baked.render_layer(state),
                    RenderLayer::Cutout | RenderLayer::CutoutMipped
                )))
}

/// The four top-corner heights of a fluid block, in (x, z) order
/// [(0,0), (0,1), (1,1), (1,0)] (north-west, south-west, south-east, north-east).
pub fn corner_heights(
    world: &dyn FluidWorld,
    table: &FluidTable,
    p: BlockPos,
    fluid: FluidState,
) -> [f32; 4] {
    let same = |q: BlockPos| {
        table
            .fluid(world.state(q))
            .filter(|f| f.fluid == fluid.fluid)
    };
    if same(p.add(0, 1, 0)).is_some() {
        return [1.0; 4];
    }
    // Column height: 1 when the same fluid continues above, own height, 0 for open non-fluid
    // blocks, -1 (ignored) for solid ones.
    let column = |q: BlockPos| -> f32 {
        match same(q) {
            Some(f) => {
                if same(q.add(0, 1, 0)).is_some() {
                    1.0
                } else {
                    f.height
                }
            }
            None if world.solid(q) => -1.0,
            None => 0.0,
        }
    };
    let own = fluid.height;
    let corner = |dx: i32, dz: i32| -> f32 {
        let side_x = column(p.add(dx, 0, 0));
        let side_z = column(p.add(0, 0, dz));
        if side_x >= 1.0 || side_z >= 1.0 {
            return 1.0;
        }
        let (mut total, mut weight) = (0.0f32, 0.0f32);
        let mut add = |h: f32| {
            if h >= 0.8 {
                total += h * 10.0;
                weight += 10.0;
            } else if h >= 0.0 {
                total += h;
                weight += 1.0;
            }
        };
        if side_x > 0.0 || side_z > 0.0 {
            let diagonal = column(p.add(dx, 0, dz));
            if diagonal >= 1.0 {
                return 1.0;
            }
            add(diagonal);
        }
        add(own);
        add(side_x);
        add(side_z);
        if weight > 0.0 {
            total / weight
        } else {
            own
        }
    };
    [corner(-1, -1), corner(-1, 1), corner(1, 1), corner(1, -1)]
}

/// Flow direction on the surface (x, z), pointing downhill; zero when flat.
pub fn flow_direction(h: [f32; 4]) -> [f32; 2] {
    // h = [nw, sw, se, ne]; +x is east, +z is south.
    let fx = (h[0] + h[1]) - (h[2] + h[3]);
    let fz = (h[0] + h[3]) - (h[1] + h[2]);
    let len = (fx * fx + fz * fz).sqrt();
    if len < 1e-4 {
        [0.0, 0.0]
    } else {
        [fx / len, fz / len]
    }
}

/// Vanilla's `FluidRenderer.tesselate` face inset (0.001 blocks).
pub const INSET: f32 = 0.001;

/// One emitted fluid quad (block-local positions 0..1, sprite-local UVs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FluidQuad {
    pub positions: [[f32; 3]; 4],
    pub uvs: [[f32; 2]; 4],
    pub sprite: u16,
    /// Face direction (shade and light).
    pub dir: Direction,
}

fn reversed(q: FluidQuad) -> FluidQuad {
    FluidQuad {
        positions: [
            q.positions[3],
            q.positions[2],
            q.positions[1],
            q.positions[0],
        ],
        uvs: [q.uvs[3], q.uvs[2], q.uvs[1], q.uvs[0]],
        ..q
    }
}

/// The quads of the fluid at `p` (block-local), appended to `out`.
pub fn fluid_quads(
    world: &dyn FluidWorld,
    table: &FluidTable,
    p: BlockPos,
    fluid: FluidState,
    out: &mut Vec<FluidQuad>,
) {
    let sprites = &table.sprites;
    let def = table.defs.def(fluid.fluid);
    let (still, flow) = (def.still, def.flow);
    let same = |q: BlockPos| {
        table
            .fluid(world.state(q))
            .is_some_and(|f| f.fluid == fluid.fluid)
    };
    let h = corner_heights(world, table, p, fluid);

    // `FluidRenderer.tesselate` pulls every face a hair into the block so fluid does not
    // z-fight with the faces of glass, slabs and other neighbours: top corners down by
    // `INSET` (which the sides then share), sides in by `INSET`, and the bottom (plus the
    // sides' bottom edge) up by `INSET` only when the bottom face is drawn.
    let below = p.add(0, -1, 0);
    let render_down = !same(below) && !world.opaque(below);
    let bottom = if render_down { INSET } else { 0.0 };
    // Vanilla skips the top face when the block above occludes it
    // (`isFaceOccludedByNeighbor` with the minimum corner height); a full opaque block
    // above always occludes.
    let render_up = !same(p.add(0, 1, 0)) && !world.opaque(p.add(0, 1, 0));
    let [nw, sw, se, ne] = if render_up { h.map(|c| c - INSET) } else { h };

    // Top: vertex order like a model's up face (NW, SW, SE, NE).
    if render_up {
        let positions = [
            [0.0, nw, 0.0],
            [0.0, sw, 1.0],
            [1.0, se, 1.0],
            [1.0, ne, 0.0],
        ];
        let corners = [[0.0f32, 0.0f32], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
        let [fx, fz] = flow_direction(h);
        let (uvs, sprite) = if fx == 0.0 && fz == 0.0 {
            (corners, still)
        } else {
            // Texture v runs along the flow, u across it; the window is the middle half of
            // the flow sprite.
            let uvs = corners.map(|[x, z]| {
                let (rx, rz) = (x - 0.5, z - 0.5);
                [
                    0.5 + 0.5 * (rx * -fz + rz * fx),
                    0.5 + 0.5 * (rx * fx + rz * fz),
                ]
            });
            (uvs, flow)
        };
        let top = FluidQuad {
            positions,
            uvs,
            sprite,
            dir: Direction::Up,
        };
        out.push(top);
        out.push(reversed(top));
    }

    // Bottom (model down-face order).
    if render_down {
        out.push(FluidQuad {
            positions: [
                [0.0, bottom, 1.0],
                [0.0, bottom, 0.0],
                [1.0, bottom, 0.0],
                [1.0, bottom, 1.0],
            ],
            uvs: [[0.0, 1.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
            sprite: still,
            dir: Direction::Down,
        });
    }

    // Sides: (direction, neighbour offset, the face's two top corners as (x, z, height) in
    // model vertex order: top-left then top-right seen from outside). The face plane sits
    // `INSET` inside the block (vanilla's `z + 0.001F` / `x + 1.0F - 0.001F`).
    let (lo, hi) = (INSET, 1.0 - INSET);
    type Side = (Direction, [i32; 3], [f32; 3], [f32; 3]);
    let sides: [Side; 4] = [
        (Direction::North, [0, 0, -1], [1.0, lo, ne], [0.0, lo, nw]),
        (Direction::South, [0, 0, 1], [0.0, hi, sw], [1.0, hi, se]),
        (Direction::West, [-1, 0, 0], [lo, 0.0, nw], [lo, 1.0, sw]),
        (Direction::East, [1, 0, 0], [hi, 1.0, se], [hi, 0.0, ne]),
    ];
    for (dir, o, a, b) in sides {
        let q = p.add(o[0], o[1], o[2]);
        if same(q) || world.opaque(q) {
            continue;
        }
        let overlay = def.overlay && table.overlay(world.state(q));
        // Model side-face order: top-left, bottom-left, bottom-right, top-right.
        let positions = [
            [a[0], a[2], a[1]],
            [a[0], bottom, a[1]],
            [b[0], bottom, b[1]],
            [b[0], b[2], b[1]],
        ];
        let uvs = [
            [0.0, (1.0 - a[2]) * 0.5],
            [0.0, 0.5],
            [0.5, 0.5],
            [0.5, (1.0 - b[2]) * 0.5],
        ];
        let quad = FluidQuad {
            positions,
            uvs,
            sprite: if overlay { sprites.water_overlay } else { flow },
            dir,
        };
        out.push(quad);
        if !overlay {
            out.push(reversed(quad));
        }
    }
}

/// Render layer of a fluid.
pub fn fluid_layer(defs: &Fluids, fluid: Fluid) -> RenderLayer {
    defs.def(fluid).layer
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const STONE: BlockStateId = BlockStateId(1);
    /// State ids 10 + level: water; 30 + level: lava.
    fn water(level: u8) -> BlockStateId {
        BlockStateId(10 + level as u32)
    }

    fn table() -> FluidTable {
        let mut fluids = vec![None; 50];
        for l in 0..16u8 {
            fluids[10 + l as usize] = Some(FluidState {
                fluid: Fluid::Water,
                height: level_height(l),
            });
            fluids[30 + l as usize] = Some(FluidState {
                fluid: Fluid::Lava,
                height: level_height(l),
            });
        }
        FluidTable::from_fluids(fluids)
    }

    struct MockStates {
        states: Vec<(haste_core::Identifier, Vec<(String, String)>)>,
    }

    fn mock_states(entries: &[(&str, &[(&str, &str)])]) -> MockStates {
        MockStates {
            states: entries
                .iter()
                .map(|(name, props)| {
                    (
                        haste_core::Identifier::minecraft(name),
                        props
                            .iter()
                            .map(|(k, v)| (k.to_string(), v.to_string()))
                            .collect(),
                    )
                })
                .collect(),
        }
    }

    impl BlockStateTable for MockStates {
        fn placeholder_state(&self) -> BlockStateId {
            BlockStateId(0)
        }
        fn len(&self) -> usize {
            self.states.len()
        }
        fn block_of(&self, state: BlockStateId) -> &haste_core::Identifier {
            &self.states[state.0 as usize].0
        }
        fn properties(&self, state: BlockStateId) -> Vec<(String, String)> {
            self.states[state.0 as usize].1.clone()
        }
        fn property_value(&self, state: BlockStateId, name: &str) -> Option<&str> {
            self.states[state.0 as usize]
                .1
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str())
        }
    }

    fn source(fluid: Fluid) -> Option<FluidState> {
        Some(FluidState {
            fluid,
            height: 8.0 / 9.0,
        })
    }

    #[test]
    fn waterlogged_states_hold_water() {
        let states = mock_states(&[
            ("oak_stairs", &[("waterlogged", "true")]),
            ("stone", &[("waterlogged", "true")]),
        ]);
        let t = FluidTable::new(
            &states,
            &Fluids::default(),
            &FluidRules::default(),
            FluidSprites::default(),
        );
        assert_eq!(t.fluid(BlockStateId(0)), source(Fluid::Water));
        assert_eq!(t.fluid(BlockStateId(1)), source(Fluid::Water));
    }

    #[test]
    fn level_property_gives_height() {
        let states = mock_states(&[
            ("water", &[("level", "0")]),
            ("water", &[("level", "3")]),
            ("water", &[("level", "8")]),
            ("lava", &[("level", "5")]),
        ]);
        let t = FluidTable::new(
            &states,
            &Fluids::default(),
            &FluidRules::default(),
            FluidSprites::default(),
        );
        assert_eq!(t.fluid(BlockStateId(0)), source(Fluid::Water));
        assert_eq!(
            t.fluid(BlockStateId(1)),
            Some(FluidState {
                fluid: Fluid::Water,
                height: 5.0 / 9.0,
            })
        );
        assert_eq!(t.fluid(BlockStateId(2)), source(Fluid::Water));
        assert_eq!(
            t.fluid(BlockStateId(3)),
            Some(FluidState {
                fluid: Fluid::Lava,
                height: 3.0 / 9.0,
            })
        );
    }

    #[test]
    fn registered_always_water_blocks() {
        let states = mock_states(&[("kelp", &[]), ("seagrass", &[]), ("bubble_column", &[])]);
        let t = FluidTable::new(
            &states,
            &Fluids::default(),
            &FluidRules::default(),
            FluidSprites::default(),
        );
        for s in 0..3 {
            assert_eq!(t.fluid(BlockStateId(s)), source(Fluid::Water));
        }
    }

    #[test]
    fn register_block_for_mods() {
        let states = mock_states(&[("mod_coral", &[]), ("mod_magma", &[])]);
        let mut rules = FluidRules::vanilla();
        rules.register_block("mod_coral", Fluid::Water);
        rules.register_block("mod_magma", Fluid::Lava);
        let t = FluidTable::new(&states, &Fluids::default(), &rules, FluidSprites::default());
        assert_eq!(t.fluid(BlockStateId(0)), source(Fluid::Water));
        assert_eq!(t.fluid(BlockStateId(1)), source(Fluid::Lava));
    }

    #[test]
    fn unknown_block_has_no_fluid() {
        let states = mock_states(&[("stone", &[]), ("dirt", &[])]);
        let t = FluidTable::new(
            &states,
            &Fluids::default(),
            &FluidRules::default(),
            FluidSprites::default(),
        );
        assert_eq!(t.fluid(BlockStateId(0)), None);
        assert_eq!(t.fluid(BlockStateId(1)), None);
    }

    #[derive(Default)]
    struct Grid(HashMap<(i32, i32, i32), BlockStateId>);
    impl Grid {
        fn set(&mut self, x: i32, y: i32, z: i32, s: BlockStateId) {
            self.0.insert((x, y, z), s);
        }
    }
    impl FluidWorld for Grid {
        fn state(&self, p: BlockPos) -> BlockStateId {
            self.0
                .get(&(p.x, p.y, p.z))
                .copied()
                .unwrap_or(BlockStateId::AIR)
        }
        fn opaque(&self, p: BlockPos) -> bool {
            self.state(p) == STONE
        }
        fn solid(&self, p: BlockPos) -> bool {
            self.state(p) == STONE
        }
    }

    fn assert_close(h: [f32; 4], v: f32) {
        assert!(h.iter().all(|c| (c - v).abs() < 1e-5), "{h:?} != {v}");
    }

    fn at(x: i32, y: i32, z: i32) -> BlockPos {
        BlockPos::new(x, y, z)
    }

    #[test]
    fn heights_by_level() {
        assert_eq!(level_height(0), 8.0 / 9.0);
        assert_eq!(level_height(1), 7.0 / 9.0);
        assert_eq!(level_height(7), 1.0 / 9.0);
        assert_eq!(level_height(8), 8.0 / 9.0);
    }

    #[test]
    fn pool_interior_is_flat_and_still() {
        let t = table();
        let mut g = Grid::default();
        for x in -2..=2 {
            for z in -2..=2 {
                g.set(x, 0, z, water(0));
                g.set(x, -1, z, STONE);
            }
        }
        let f = t.fluid(water(0)).unwrap();
        let h = corner_heights(&g, &t, at(0, 0, 0), f);
        assert_close(h, 8.0 / 9.0);
        assert_eq!(flow_direction(h), [0.0, 0.0]);
        // Interior: top + reversed top only (sides against water, bottom on stone).
        let mut quads = Vec::new();
        fluid_quads(&g, &t, at(0, 0, 0), f, &mut quads);
        assert_eq!(quads.len(), 2);
        assert!(quads.iter().all(|q| q.dir == Direction::Up));
    }

    #[test]
    fn edge_next_to_air_slopes_down_and_solid_walls_do_not() {
        let t = table();
        let f = t.fluid(water(0)).unwrap();
        // A lone source on stone, air around: each corner averages the source (weight 10)
        // with two air columns (weight 1 each, height 0): 8/9 * 10 / 12.
        let mut g = Grid::default();
        g.set(0, 0, 0, water(0));
        g.set(0, -1, 0, STONE);
        let h = corner_heights(&g, &t, at(0, 0, 0), f);
        let expect = (8.0 / 9.0) * 10.0 / 12.0;
        for c in h {
            assert!((c - expect).abs() < 1e-6, "{h:?}");
        }
        // Enclosed by stone on all sides: walls are ignored, the surface stays at 8/9.
        for (x, z) in [
            (-1, -1),
            (-1, 0),
            (-1, 1),
            (0, -1),
            (0, 1),
            (1, -1),
            (1, 0),
            (1, 1),
        ] {
            g.set(x, 0, z, STONE);
        }
        assert_close(corner_heights(&g, &t, at(0, 0, 0), f), 8.0 / 9.0);
        // Same fluid above: full height.
        g.set(0, 1, 0, water(0));
        assert_eq!(corner_heights(&g, &t, at(0, 0, 0), f), [1.0; 4]);
    }

    #[test]
    fn flowing_water_flows_downhill() {
        let t = table();
        // Source at x=0, level 1 at x=1, level 2 at x=2 in a stone channel (z walls).
        let mut g = Grid::default();
        for x in -1..=3 {
            g.set(x, -1, 0, STONE);
            g.set(x, 0, -1, STONE);
            g.set(x, 0, 1, STONE);
        }
        g.set(-1, 0, 0, STONE);
        g.set(0, 0, 0, water(0));
        g.set(1, 0, 0, water(1));
        g.set(2, 0, 0, water(2));
        let f = t.fluid(water(1)).unwrap();
        let h = corner_heights(&g, &t, at(1, 0, 0), f);
        // West corners lean on the source (>= 0.8, weight 10), east corners on level 2.
        assert!(h[0] > h[3] && h[1] > h[2], "{h:?}");
        let [fx, fz] = flow_direction(h);
        assert!(fx > 0.99 && fz.abs() < 1e-6, "flow ({fx}, {fz})");
        let mut quads = Vec::new();
        fluid_quads(&g, &t, at(1, 0, 0), f, &mut quads);
        let top = quads.iter().find(|q| q.dir == Direction::Up).unwrap();
        // Flow sprite (index 0 in the test sprites, same as still): UVs rotated so v grows
        // toward +x (east).
        let east = top.uvs[2][1]; // SE corner
        let west = top.uvs[1][1]; // SW corner
        assert!(east > west, "{:?}", top.uvs);
        // UVs stay inside the middle half of the sprite.
        for uv in top.uvs {
            assert!((0.25..=0.75).contains(&uv[0]) && (0.25..=0.75).contains(&uv[1]));
        }
    }

    #[test]
    fn faces_cull_against_same_fluid_and_opaque_blocks() {
        let t = table();
        let f = t.fluid(water(0)).unwrap();
        let mut g = Grid::default();
        g.set(0, 0, 0, water(0));
        let count = |g: &Grid| {
            let mut q = Vec::new();
            fluid_quads(g, &t, at(0, 0, 0), f, &mut q);
            q
        };
        // Floating source: top x2, bottom, 4 sides x2.
        assert_eq!(count(&g).len(), 11);
        g.set(1, 0, 0, STONE);
        g.set(0, -1, 0, STONE);
        g.set(-1, 0, 0, water(3));
        // East culled by stone, bottom by stone, west by water: top x2 + north/south x2.
        let quads = count(&g);
        assert_eq!(quads.len(), 6);
        assert!(!quads
            .iter()
            .any(|q| matches!(q.dir, Direction::East | Direction::West | Direction::Down)));
        // Lava next to water is not "same": sides are drawn.
        let lava = t.fluid(BlockStateId(30)).unwrap();
        let mut q = Vec::new();
        fluid_quads(&g, &t, at(-1, 0, 0), lava, &mut q);
        assert!(q.iter().any(|q| q.dir == Direction::East));
    }

    #[test]
    fn faces_are_inset_like_vanilla() {
        // FluidRenderer.tesselate: top corners 0.001 down, sides 0.001 in, bottom (and the
        // sides' bottom edge) 0.001 up only when the bottom face is drawn.
        let t = table();
        let f = t.fluid(water(0)).unwrap();
        let mut g = Grid::default();
        g.set(0, 0, 0, water(0));
        let mut q = Vec::new();
        fluid_quads(&g, &t, at(0, 0, 0), f, &mut q);
        let up = q.iter().find(|q| q.dir == Direction::Up).unwrap();
        for v in up.positions {
            assert!((v[1] - (8.0 / 9.0 - INSET)).abs() < 1e-6, "{v:?}");
        }
        let down = q.iter().find(|q| q.dir == Direction::Down).unwrap();
        assert!(down.positions.iter().all(|v| v[1] == INSET));
        let north = q.iter().find(|q| q.dir == Direction::North).unwrap();
        assert!(north.positions.iter().all(|v| v[2] == INSET));
        assert!((north.positions[0][1] - (8.0 / 9.0 - INSET)).abs() < 1e-6);
        assert_eq!(north.positions[1][1], INSET);
        let east = q.iter().find(|q| q.dir == Direction::East).unwrap();
        assert!(east.positions.iter().all(|v| v[0] == 1.0 - INSET));

        // Stone below: the bottom face is gone and the sides reach y = 0.
        g.set(0, -1, 0, STONE);
        q.clear();
        fluid_quads(&g, &t, at(0, 0, 0), f, &mut q);
        let west = q.iter().find(|q| q.dir == Direction::West).unwrap();
        assert_eq!(west.positions[1][1], 0.0);
    }

    #[test]
    fn side_uvs_cut_at_the_surface() {
        let t = table();
        let f = t.fluid(water(0)).unwrap();
        let mut g = Grid::default();
        g.set(0, 0, 0, water(0));
        for (x, z) in [(-1, -1), (-1, 0), (-1, 1), (0, -1), (1, -1), (1, 0), (1, 1)] {
            g.set(x, 0, z, STONE);
        }
        let mut q = Vec::new();
        fluid_quads(&g, &t, at(0, 0, 0), f, &mut q);
        let south: Vec<_> = q.iter().filter(|q| q.dir == Direction::South).collect();
        assert_eq!(south.len(), 2);
        let s = south[0];
        // Top-left vertex at the corner height (pulled down by the air to the south),
        // v = (1 - h) / 2.
        let h = s.positions[0][1];
        assert!(h > 0.5 && h < 8.0 / 9.0, "{h}");
        assert!((s.uvs[0][1] - (1.0 - h) * 0.5).abs() < 1e-6);
        assert_eq!(s.uvs[1], [0.0, 0.5]);
    }

    #[test]
    fn up_face_winding_matches_model_faces() {
        // Counter-clockwise seen from above (+y) like haste-assets' up face: the normal of
        // (v1 - v0) x (v2 - v0) points up.
        let t = table();
        let f = t.fluid(water(0)).unwrap();
        let mut g = Grid::default();
        g.set(0, 0, 0, water(0));
        let mut q = Vec::new();
        fluid_quads(&g, &t, at(0, 0, 0), f, &mut q);
        let normal = |q: &FluidQuad| {
            let [a, b, c, _] = q.positions;
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ]
        };
        // Model up face (0,1,0),(0,1,1),(1,1,1): normal = (0,1,1)x(1,1,1)... compare signs
        // with the model's own order instead of assuming a handedness.
        let model_up = FluidQuad {
            positions: [
                [0.0, 1.0, 0.0],
                [0.0, 1.0, 1.0],
                [1.0, 1.0, 1.0],
                [1.0, 1.0, 0.0],
            ],
            uvs: [[0.0; 2]; 4],
            sprite: 0,
            dir: Direction::Up,
        };
        let top = q.iter().find(|q| q.dir == Direction::Up).unwrap();
        assert!(normal(top)[1].signum() == normal(&model_up)[1].signum());
        for (dir, model) in [
            (
                Direction::North,
                [
                    [1.0, 1.0, 0.0],
                    [1.0, 0.0, 0.0],
                    [0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0],
                ],
            ),
            (
                Direction::East,
                [
                    [1.0, 1.0, 1.0],
                    [1.0, 0.0, 1.0],
                    [1.0, 0.0, 0.0],
                    [1.0, 1.0, 0.0],
                ],
            ),
        ] {
            let m = FluidQuad {
                positions: model,
                ..model_up
            };
            let side = q.iter().find(|q| q.dir == dir).unwrap();
            let (a, b) = (normal(side), normal(&m));
            assert!(a[0] * b[0] + a[1] * b[1] + a[2] * b[2] > 0.0, "{dir:?}");
        }
    }
}

#[cfg(test)]
mod jar_tests {
    #[test]
    #[ignore = "needs the client jar and datagen reports in the cache"]
    fn jar_fluid_sprites_are_in_the_atlas() {
        let jar = haste_core::paths::client_jar();
        let mut assets = haste_assets::Assets::open(
            &jar,
            &haste_core::paths::cache_dir().join("assets"),
            "26.3",
        )
        .unwrap();
        let states = haste_assets::BlocksJsonStates::load(
            &haste_core::paths::datagen_reports().join("blocks.json"),
        )
        .unwrap();
        let mut atlas = haste_assets::AtlasBuilder::build(&mut assets, &[], false).unwrap();
        let _baked =
            haste_assets::BakedBlockModels::bake(&mut assets, &states, &mut atlas).unwrap();
        let sprites = super::FluidSprites::from_atlas(&atlas);
        eprintln!("{sprites:?}");
        for name in ["minecraft:block/lava_flow", "minecraft:block/water_still"] {
            let r = atlas.region(name);
            let [x, y, w, h] = r.pixel_rect;
            eprintln!("{name}: {:?}", r.pixel_rect);
            let size = atlas.size();
            let px = |dx: u32, dy: u32| {
                let i = (((y + dy) * size + x + dx) * 4) as usize;
                atlas.pixels()[i..i + 4].to_vec()
            };
            eprintln!(
                "  {:?} {:?} {:?}",
                px(0, 0),
                px(w / 2, h / 2),
                px(w - 1, h - 1)
            );
            assert!(r.sprite != 0 && w > 1 && h > 1);
        }
    }
}
