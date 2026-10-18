use super::registry::level_height;
use super::*;
use haste_assets::Direction;
use haste_core::view::{BlockStateTable, BlockView};
use haste_core::{Aabb, BlockPos, ChunkPos, DVec3, SectionPos};
use std::collections::HashMap;
use std::sync::Arc;

mod occlusion_dump;

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
struct Grid {
    blocks: HashMap<(i32, i32, i32), BlockStateId>,
    /// Occlusion shapes of partial blocks (slabs, stairs) by state.
    shapes: HashMap<BlockStateId, Vec<[f32; 6]>>,
}
impl Grid {
    fn set(&mut self, x: i32, y: i32, z: i32, s: BlockStateId) {
        self.blocks.insert((x, y, z), s);
    }
}
impl FluidWorld for Grid {
    fn state(&self, p: BlockPos) -> BlockStateId {
        self.blocks
            .get(&(p.x, p.y, p.z))
            .copied()
            .unwrap_or(BlockStateId::AIR)
    }
    fn opaque(&self, p: BlockPos) -> bool {
        self.state(p) == STONE
    }
    fn solid(&self, p: BlockPos) -> bool {
        let s = self.state(p);
        s == STONE || self.shapes.contains_key(&s)
    }
    fn face_occluder(&self, p: BlockPos, _dir: Direction) -> Occluder<'_> {
        let s = self.state(p);
        if s == STONE {
            Occluder::Full
        } else if let Some(boxes) = self.shapes.get(&s) {
            Occluder::Boxes(boxes)
        } else {
            Occluder::Empty
        }
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
    assert_eq!(fluid_flow(&g, &t, at(0, 0, 0), f), [0.0, 0.0]);
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
    // FlowingFluid.getFlow on the level-1 block: distance to the source is 7/9 - 8/9
    // (pulled west), to level 2 it is 7/9 - 6/9 (pulled east); the stone walls and floor
    // are in blocks_fluid_flow, so they pull nothing. Net 2/9 east.
    let flow = fluid_flow(&g, &t, at(1, 0, 0), f);
    assert!(
        (flow[0] - 1.0).abs() < 1e-6 && flow[1].abs() < 1e-6,
        "flow {flow:?}"
    );
    let mut quads = Vec::new();
    fluid_quads(&g, &t, at(1, 0, 0), f, &mut quads);
    let top = quads.iter().find(|q| q.dir == Direction::Up).unwrap();
    // tesselate with flow (1, 0): the middle half of the flow sprite, v running east
    // (u00 = 0.5 - c - s with c = 0, s = -0.25 and so on).
    let expect = [[0.75, 0.25], [0.25, 0.25], [0.25, 0.75], [0.75, 0.75]];
    for (uv, e) in top.uvs.iter().zip(expect) {
        assert!(
            (uv[0] - e[0]).abs() < 1e-6 && (uv[1] - e[1]).abs() < 1e-6,
            "{:?}",
            top.uvs
        );
    }
}

#[test]
fn flow_comes_from_getflow_not_the_corner_gradient() {
    let t = table();
    let f = t.fluid(water(0)).unwrap();
    // A source on stone with open air east: its corners slope down to the east, but
    // FlowingFluid.getFlow sees no fluid beside or below the air column: still top.
    let mut g = Grid::default();
    g.set(0, 0, 0, water(0));
    g.set(0, -1, 0, STONE);
    let h = corner_heights(&g, &t, at(0, 0, 0), f);
    assert!(h[2] < 8.0 / 9.0, "corners slope: {h:?}");
    assert_eq!(fluid_flow(&g, &t, at(0, 0, 0), f), [0.0, 0.0]);
    let mut quads = Vec::new();
    fluid_quads(&g, &t, at(0, 0, 0), f, &mut quads);
    let top = quads.iter().find(|q| q.dir == Direction::Up).unwrap();
    assert_eq!(top.uvs, [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]]);

    // Water one level below the drop-off pulls the surface over the edge: distance
    // 8/9 - (8/9 - 8/9) = 8/9 toward east.
    g.set(1, -1, 0, water(0));
    let flow = fluid_flow(&g, &t, at(0, 0, 0), f);
    assert!(
        (flow[0] - 1.0).abs() < 1e-6 && flow[1].abs() < 1e-6,
        "flow {flow:?}"
    );

    // A blocks_fluid_flow neighbour (the mesher's blocks-motion approximation) between
    // them stops the pull again.
    g.set(1, 0, 0, STONE);
    assert_eq!(fluid_flow(&g, &t, at(0, 0, 0), f), [0.0, 0.0]);

    // A different fluid beside does not pull either (affectsFlow is false).
    g.set(1, 0, 0, BlockStateId(30));
    assert_eq!(fluid_flow(&g, &t, at(0, 0, 0), f), [0.0, 0.0]);
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
        // A lone source: each corner averages the source (weight 10) with two air sides
        // (weight 1 each) in `FluidRenderer.calculateAverageHeight`, then the inset.
        assert!(
            (v[1] - (8.0 / 9.0 * 10.0 / 12.0 - INSET)).abs() < 1e-6,
            "{v:?}"
        );
    }
    let down = q.iter().find(|q| q.dir == Direction::Down).unwrap();
    assert!(down.positions.iter().all(|v| v[1] == INSET));
    let north = q.iter().find(|q| q.dir == Direction::North).unwrap();
    assert!(north.positions.iter().all(|v| v[2] == INSET));
    assert!((north.positions[0][1] - (8.0 / 9.0 * 10.0 / 12.0 - INSET)).abs() < 1e-6);
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
fn backward_up_face_only_when_visible_from_above_layer() {
    // FluidState.shouldRenderBackwardUpFace: the underside of the surface is skipped
    // when the whole 3x3 above is stone or the same fluid; one open block brings it back.
    let t = table();
    let f = t.fluid(water(0)).unwrap();
    let mut g = Grid::default();
    g.set(0, 0, 0, water(0));
    let ups = |g: &Grid| {
        let mut q = Vec::new();
        fluid_quads(g, &t, at(0, 0, 0), f, &mut q);
        q.iter().filter(|q| q.dir == Direction::Up).count()
    };
    assert_eq!(ups(&g), 2, "open sky: front and back face");
    for x in -1..=1 {
        for z in -1..=1 {
            g.set(x, 1, z, STONE);
        }
    }
    // isFaceOccludedByState: a full block above does not hide a surface at 8/9, but the
    // back face is not needed under a closed ceiling.
    assert_eq!(ups(&g), 1, "stone ceiling: front face only");
    g.set(1, 1, 1, water(0));
    assert_eq!(ups(&g), 1, "same fluid in the ceiling counts as closed");
    g.set(1, 1, 1, BlockStateId::AIR);
    assert_eq!(
        ups(&g),
        2,
        "one open block in the 3x3 above: back face again"
    );
    g.set(1, 1, 1, STONE);
    g.set(0, 1, 0, water(0));
    assert_eq!(ups(&g), 0, "same fluid straight above: no top face");
}

#[test]
fn waterlogged_block_culls_faces_its_own_shape_covers() {
    // FluidRenderer.shouldRenderFace runs every face but the top through
    // isFaceOccludedBySelf: the water inside a waterlogged bottom slab loses its
    // bottom face (the slab sits on it), everything else stays; a top slab hides
    // nothing of the fluid (its shape does not touch the fluid's bottom face).
    let t = table();
    let f = t.fluid(water(0)).unwrap();
    let dirs = |g: &Grid| {
        let mut q = Vec::new();
        fluid_quads(g, &t, at(0, 0, 0), f, &mut q);
        q.iter()
            .map(|q| q.dir)
            .collect::<std::collections::HashSet<_>>()
    };
    let mut g = Grid::default();
    g.set(0, 0, 0, water(0));
    g.shapes
        .insert(water(0), vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]);
    let d = dirs(&g);
    assert!(
        !d.contains(&Direction::Down),
        "bottom slab covers the bottom"
    );
    assert!(d.contains(&Direction::Up));
    assert!(d.contains(&Direction::North));

    let mut g = Grid::default();
    g.set(0, 0, 0, water(0));
    g.shapes
        .insert(water(0), vec![[0.0, 0.5, 0.0, 1.0, 1.0, 1.0]]);
    let d = dirs(&g);
    assert!(
        d.contains(&Direction::Down),
        "top slab leaves the bottom open"
    );
    assert!(
        d.contains(&Direction::Up),
        "the top face has no self-occlusion"
    );
    assert!(d.contains(&Direction::North));
}

#[test]
fn sides_cull_against_slabs_and_stairs() {
    // isFaceOccludedByNeighbor with the neighbour's shape: a bottom slab east of a
    // low flowing block hides the east face; next to a source it does not.
    let t = table();
    let mut g = Grid::default();
    const SLAB: BlockStateId = BlockStateId(2);
    g.shapes.insert(SLAB, vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]);
    g.set(1, 0, 0, SLAB);
    // Walls around so the corner heights stay at the block's own height.
    for (x, z) in [(-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 1)] {
        g.set(x, 0, z, STONE);
    }
    g.set(0, -1, 0, STONE);
    let east = |g: &Grid, level: u8| {
        g_fluid_quads(g, &t, level)
            .iter()
            .filter(|q| q.dir == Direction::East)
            .count()
    };
    assert_eq!(east(&g, 6), 0, "level 6 (2/9 high) hidden by the slab");
    assert_eq!(east(&g, 0), 2, "a source (8/9) shows above the slab");
    // Below: a top slab hides the bottom face, a bottom slab does not.
    g.set(0, -1, 0, SLAB);
    assert!(g_fluid_quads(&g, &t, 0)
        .iter()
        .any(|q| q.dir == Direction::Down));
    const TOP_SLAB: BlockStateId = BlockStateId(3);
    g.shapes
        .insert(TOP_SLAB, vec![[0.0, 0.5, 0.0, 1.0, 1.0, 1.0]]);
    g.set(0, -1, 0, TOP_SLAB);
    assert!(!g_fluid_quads(&g, &t, 0)
        .iter()
        .any(|q| q.dir == Direction::Down));
}

fn g_fluid_quads(g: &Grid, t: &FluidTable, level: u8) -> Vec<FluidQuad> {
    let mut g2 = Grid {
        blocks: g.blocks.clone(),
        shapes: g.shapes.clone(),
    };
    g2.set(0, 0, 0, water(level));
    let f = t.fluid(water(level)).unwrap();
    let mut q = Vec::new();
    fluid_quads(&g2, t, at(0, 0, 0), f, &mut q);
    q
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

#[test]
#[ignore = "needs the client jar and datagen reports in the cache"]
fn jar_fluid_sprites_are_in_the_atlas() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::cache_dir().join("assets"), "26.3")
            .unwrap();
    let states = haste_assets::BlocksJsonStates::load(
        &haste_core::paths::datagen_reports().join("blocks.json"),
    )
    .unwrap();
    let mut atlas = haste_assets::AtlasBuilder::build(&mut assets, &[], false).unwrap();
    let _baked = haste_assets::BakedBlockModels::bake(&mut assets, &states, &mut atlas).unwrap();
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

// ---------------------------------------------------------------- fluid/occlusion.rs

/// An outline source for `FluidOccluders::from_outlines` (the unit-test stand-in for
/// haste-data's per-state outline shapes).
fn outline_table<'a>(outlines: &'a [&'a [Aabb]]) -> impl Fn(BlockStateId) -> &'a [Aabb] + 'a {
    |s: BlockStateId| outlines.get(s.0 as usize).copied().unwrap_or(&[])
}

fn cube() -> Aabb {
    Aabb {
        min: DVec3::new(0.0, 0.0, 0.0),
        max: DVec3::new(1.0, 1.0, 1.0),
    }
}

fn bottom_slab_aabb() -> Aabb {
    Aabb {
        min: DVec3::new(0.0, 0.0, 0.0),
        max: DVec3::new(1.0, 0.5, 1.0),
    }
}

/// `BlockBehaviour.initCache`'s classification of a state's occlusion shape: full cubes
/// occlude as `Shapes.block()` exactly when solid render (the `noOcclusion` family —
/// glass, leaves, slime — is never solid render), empty shapes never occlude, and any
/// other shape is the occluder as its boxes.
#[test]
fn occluder_table_classifies_outline_shapes() {
    let full = [cube()];
    let slab = [bottom_slab_aabb()];
    let none: &[Aabb] = &[];
    // 0: stone (full outline, solid render), 1: glass (full outline, not solid render),
    // 2: bottom slab, 3: torch (empty outline).
    let outlines: [&[Aabb]; 4] = [&full, &full, &slab, none];
    let t = FluidOccluders::from_outlines(4, outline_table(&outlines), |s| s == BlockStateId(0));
    assert!(matches!(
        t.occluder(BlockStateId(0), || true),
        Occluder::Full
    ));
    assert!(matches!(
        t.occluder(BlockStateId(1), || false),
        Occluder::Empty
    ));
    match t.occluder(BlockStateId(2), || false) {
        Occluder::Boxes(boxes) => assert_eq!(boxes, &[[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]),
        o => panic!("slab must occlude with its boxes, got {o:?}"),
    }
    assert!(matches!(
        t.occluder(BlockStateId(3), || false),
        Occluder::Empty
    ));
    // No data (the table is absent) and past-the-end ids: the caller's opaque flag
    // decides, like the mesher did before shape data.
    let absent = FluidOccluders::absent();
    assert!(matches!(
        absent.occluder(BlockStateId(0), || true),
        Occluder::Full
    ));
    assert!(matches!(
        absent.occluder(BlockStateId(0), || false),
        Occluder::Empty
    ));
    assert!(matches!(
        t.occluder(BlockStateId(99), || true),
        Occluder::Full
    ));
    assert!(matches!(
        t.occluder(BlockStateId(99), || false),
        Occluder::Empty
    ));
}

/// The mesher's fluid view: the same lines `FluidView::face_occluder` runs, over a test
/// grid. Water beside a bottom slab keeps its whole side face when the slab covers only
/// its lower half (vanilla's `Shapes.blockOccludes` is whole-face) and drops it when the
/// slab covers the face entirely; the water inside a waterlogged bottom slab loses the
/// bottom face to its own shape; glass never occludes.
struct OccludedGrid<'a> {
    grid: Grid,
    occluders: &'a FluidOccluders,
}

impl FluidWorld for OccludedGrid<'_> {
    fn state(&self, p: BlockPos) -> BlockStateId {
        self.grid.state(p)
    }
    fn opaque(&self, p: BlockPos) -> bool {
        self.grid.opaque(p)
    }
    fn solid(&self, p: BlockPos) -> bool {
        self.grid.solid(p)
    }
    fn face_occluder(&self, p: BlockPos, _dir: Direction) -> Occluder<'_> {
        let state = self.state(p);
        self.occluders.occluder(state, || self.opaque(p))
    }
}

#[test]
fn mesher_view_culls_fluid_faces_against_partial_shapes() {
    // Outline shapes by state id: 1 stone (full cube, solid render), 2 bottom slab,
    // 3 glass (a full outline that never occludes: the `noOcclusion` family is not
    // solid render), 4 the waterlogged bottom slab (water by the logged table below,
    // slab outline). Water states are 10 + level as in `table`.
    let full = [cube()];
    let slab = [bottom_slab_aabb()];
    let outlines: [&[Aabb]; 5] = [&full, &full, &slab, &full, &slab];
    let occluders =
        FluidOccluders::from_outlines(50, outline_table(&outlines), |s| s == BlockStateId(1));
    let world = |neighbour: BlockStateId, fluid: BlockStateId| OccludedGrid {
        grid: {
            let mut g = Grid::default();
            g.set(0, 0, 0, fluid);
            g.set(0, -1, 0, BlockStateId(1));
            // Walls around so the corner heights stay at the block's own height.
            for (x, z) in [(-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 1)] {
                g.set(x, 0, z, BlockStateId(1));
            }
            g.set(1, 0, 0, neighbour);
            g
        },
        occluders: &occluders,
    };
    let dirs = |g: &OccludedGrid, table: &FluidTable, state: BlockStateId| {
        let f = table.fluid(state).unwrap();
        let mut q = Vec::new();
        fluid_quads(g, table, at(0, 0, 0), f, &mut q);
        q.iter()
            .map(|q| q.dir)
            .collect::<std::collections::HashSet<_>>()
    };

    // A low flowing block (2/9 high) beside a bottom slab: the slab covers the whole
    // east face, vanilla drops it (the stone walls take north/south/west and the
    // bottom; the surface stays).
    let low = world(BlockStateId(2), water(6));
    let d = dirs(&low, &table(), water(6));
    assert!(!d.contains(&Direction::East), "slab hides the low face");
    assert!(!d.contains(&Direction::Down), "stone below");
    assert!(d.contains(&Direction::Up));
    // A source (8/9 high) keeps the whole face; the slab covers only its lower half.
    let high = world(BlockStateId(2), water(0));
    assert!(
        dirs(&high, &table(), water(0)).contains(&Direction::East),
        "source shows above the slab"
    );
    // Low water beside glass: a full outline that is not solid render never occludes.
    let glass = world(BlockStateId(3), water(6));
    assert!(
        dirs(&glass, &table(), water(6)).contains(&Direction::East),
        "glass never occludes"
    );

    // The water inside a waterlogged bottom slab (state 4 carries both the water and
    // the slab's shape) loses its bottom face to its own shape. Open air around, so the
    // side faces above the slab's half height stay.
    let mut fluids = vec![None; 50];
    fluids[4] = Some(FluidState {
        fluid: Fluid::Water,
        height: 8.0 / 9.0,
    });
    let logged_table = FluidTable::from_fluids(fluids);
    let mut logged_grid = Grid::default();
    logged_grid.set(0, 0, 0, BlockStateId(4));
    let logged = OccludedGrid {
        grid: logged_grid,
        occluders: &occluders,
    };
    let d = dirs(&logged, &logged_table, BlockStateId(4));
    assert!(!d.contains(&Direction::Down), "the slab covers the bottom");
    assert!(d.contains(&Direction::Up));
    assert!(d.contains(&Direction::North));
}

/// The real path with the cache: `FluidOccluders::from_shapes` over haste-data's oracle
/// dump and the jar's baked models classifies vanilla's states (stone full, glass empty,
/// slabs as their half-cube boxes), and the section mesher culls a low water face
/// against a slab while keeping a source's face above it.
#[test]
#[ignore = "needs the client jar and datagen reports in the cache"]
fn mesher_culls_fluid_faces_with_real_shapes() {
    let jar = haste_core::paths::client_jar();
    if !jar.exists() {
        eprintln!("ignored: needs the client jar in the cache");
        return;
    }
    let data = haste_data::GameData::load().unwrap();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::cache_dir().join("assets"), "26.3")
            .unwrap();
    let states = haste_assets::BlocksJsonStates::load(
        &haste_core::paths::datagen_reports().join("blocks.json"),
    )
    .unwrap();
    let mut atlas = haste_assets::AtlasBuilder::build(&mut assets, &[], false).unwrap();
    let baked = haste_assets::BakedBlockModels::bake(&mut assets, &states, &mut atlas).unwrap();

    let state = |name: &str, props: &[(&str, &str)]| -> BlockStateId {
        (0..states.len() as u32)
            .map(BlockStateId)
            .find(|s| {
                states.block_of(*s).path() == name
                    && props
                        .iter()
                        .all(|(k, v)| states.property_value(*s, k) == Some(*v))
            })
            .unwrap_or_else(|| panic!("no state for {name} {props:?}"))
    };
    let occluders = FluidOccluders::from_shapes(&states, &data.shapes, &baked);
    match occluders.occluder(state("oak_slab", &[("type", "bottom")]), || false) {
        Occluder::Boxes(boxes) => assert_eq!(boxes, &[[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]),
        o => panic!("bottom slab must occlude with its boxes, got {o:?}"),
    }
    assert!(matches!(
        occluders.occluder(state("stone", &[]), || true),
        Occluder::Full
    ));
    assert!(matches!(
        occluders.occluder(state("glass", &[]), || false),
        Occluder::Empty
    ));

    // Mesh a section with water beside a bottom slab on stone and count the water
    // vertices on the shared plane: a level-6 block (2/9 high) loses the face, a source
    // keeps it (the slab covers only its lower half).
    let sprites = FluidSprites::from_atlas(&atlas);
    let tables = crate::mesher::MeshTables::new(
        Arc::new(crate::tint::TintTable::new(&states)),
        Arc::new(crate::tint::BiomeColors::default()),
        Arc::new(FluidTable::new(
            &states,
            &Fluids::vanilla(&sprites),
            &FluidRules::vanilla(),
            sprites,
        )),
        Arc::new(crate::mesher::AoTable::all_solid(states.len())),
        Arc::new(occluders),
    );
    let west_face_vertices = |fluid_state: BlockStateId| -> usize {
        struct OnePair {
            floor: BlockStateId,
            slab: BlockStateId,
            fluid: BlockStateId,
        }
        impl BlockView for OnePair {
            fn block_state(&self, p: BlockPos) -> BlockStateId {
                match (p.x, p.y, p.z) {
                    (0..=2, 0, 0..=2) => self.floor,
                    (1, 1, 1) => self.fluid,
                    (0, 1, 1) => self.slab,
                    _ => BlockStateId::AIR,
                }
            }
            fn is_chunk_loaded(&self, _: ChunkPos) -> bool {
                true
            }
            fn sky_light(&self, _: BlockPos) -> u8 {
                15
            }
            fn block_light(&self, _: BlockPos) -> u8 {
                0
            }
            fn min_y(&self) -> i32 {
                -64
            }
            fn height(&self) -> i32 {
                384
            }
        }
        let world = OnePair {
            floor: state("stone", &[]),
            // waterlogged=false: the plain slab, not its waterlogged twin (whose own
            // water would take the same-fluid culling instead).
            slab: state("oak_slab", &[("type", "bottom"), ("waterlogged", "false")]),
            fluid: fluid_state,
        };
        let snapshot = crate::mesher::MeshSnapshot::capture(&world, SectionPos::new(0, 0, 0));
        let mesh = crate::mesher::mesh_section_with(
            &snapshot,
            &baked,
            &atlas,
            &tables,
            SectionPos::new(0, 0, 0),
        );
        mesh.vertices
            .iter()
            // The fluid block sits at x = 1; only its west face plane (INSET inside
            // the block) carries vertices there — models sit on the block boundary.
            // Positions are u16-quantized (24 blocks over 65535), so compare with the
            // quantization step's slack, well below INSET.
            .filter(|v| {
                let x = f32::from(v.pos[0]) / 65535.0 * crate::vertex::POS_RANGE
                    - crate::vertex::POS_BIAS;
                (x - (1.0 + INSET)).abs() < 5e-4
            })
            .count()
    };
    // Level 6 water (2/9 high): vanilla drops the face the slab covers.
    assert_eq!(
        west_face_vertices(state("water", &[("level", "6")])),
        0,
        "the slab covers a 2/9-high face entirely"
    );
    // A source (8/9 high): the face stays; the slab covers only its lower half. Both
    // the front and the back quad are drawn (no overlay neighbour), so 8 vertices.
    assert_eq!(
        west_face_vertices(state("water", &[("level", "0")])),
        8,
        "a source keeps its side face above the slab"
    );
}
