use super::registry::level_height;
use super::*;
use haste_assets::Direction;
use haste_core::view::BlockStateTable;
use haste_core::BlockPos;
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
