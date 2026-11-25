//! Tests for the face-skip rules (`mesher/skip.rs`, vanilla
//! `BlockBehaviour.skipRendering`). The dump fixtures are hand-made (the real
//! `block_classes.json` comes from the recorder mod's dump run); the mesher
//! tests at the bottom use the client jar's baked models from the cache.

use super::*;
use crate::mesher::{mesh_section_with, MeshSnapshot};
use haste_assets::Direction;
use haste_core::view::BlockStateTable;
use haste_core::{BlockPos, BlockStateId, Identifier, SectionPos};

/// A minimal block-state table for the fast tests: one row per state.
struct FakeStates {
    rows: Vec<(Identifier, Vec<(String, String)>)>,
}

impl FakeStates {
    fn new(rows: Vec<(&str, Vec<(String, String)>)>) -> FakeStates {
        FakeStates {
            rows: rows
                .into_iter()
                .map(|(name, props)| (Identifier::parse(name).unwrap(), props))
                .collect(),
        }
    }

    fn state(&self, row: usize) -> BlockStateId {
        BlockStateId(row as u32)
    }
}

impl BlockStateTable for FakeStates {
    fn len(&self) -> usize {
        self.rows.len()
    }
    fn placeholder_state(&self) -> BlockStateId {
        BlockStateId(0)
    }
    fn block_of(&self, s: BlockStateId) -> &Identifier {
        &self.rows[s.0 as usize].0
    }
    fn properties(&self, s: BlockStateId) -> Vec<(String, String)> {
        self.rows[s.0 as usize].1.clone()
    }
    fn property_value(&self, s: BlockStateId, name: &str) -> Option<&str> {
        self.rows[s.0 as usize]
            .1
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// A hand-made `block_classes.json` (the recorder mod's `BlockClassDump`
/// writes the same shape): the vanilla class chains of the blocks the tests
/// use, plus one block the dump does not know.
fn dump() -> BlockClassDump {
    BlockClassDump::parse(
        r#"{"version":1,"minecraft":"26.3","blocks":{
            "minecraft:stone": ["StoneBlock", "Block"],
            "minecraft:glass": ["GlassBlock", "TransparentBlock", "HalfTransparentBlock", "Block"],
            "minecraft:tinted_glass": ["TintedGlassBlock", "TransparentBlock", "HalfTransparentBlock", "Block"],
            "minecraft:powder_snow": ["PowderSnowBlock", "Block"],
            "minecraft:glass_pane": ["IronBarsBlock", "CrossCollisionBlock", "Block"],
            "minecraft:iron_bars": ["IronBarsBlock", "CrossCollisionBlock", "Block"],
            "minecraft:oak_leaves": ["LeavesBlock", "Block"],
            "minecraft:mangrove_roots": ["MangroveRootsBlock", "Block"],
            "minecraft:muddy_mangrove_roots": ["RotatedPillarBlock", "Block"]
        }}"#,
    )
    .unwrap()
}

/// The test states: air, stone, glass, tinted glass, powder snow, a glass
/// pane connected north (state 5) and one connected south (state 6), a pane
/// with no connections (7), iron bars connected south (8), oak leaves (9),
/// mangrove roots (10), muddy mangrove roots (11) and an undumped block (12).
fn states() -> FakeStates {
    let pane = |props: &[(&str, &str)]| -> Vec<(String, String)> {
        let mut all: Vec<(String, String)> = [
            ("north", "false"),
            ("east", "false"),
            ("south", "false"),
            ("west", "false"),
            ("waterlogged", "false"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        for (k, v) in props {
            if let Some(slot) = all.iter_mut().find(|(n, _)| n == k) {
                slot.1 = v.to_string();
            }
        }
        all
    };
    let plain =
        || -> Vec<(String, String)> { vec![("waterlogged".to_string(), "false".to_string())] };
    FakeStates::new(vec![
        ("minecraft:air", vec![]),
        ("minecraft:stone", vec![]),
        ("minecraft:glass", vec![]),
        ("minecraft:tinted_glass", vec![]),
        ("minecraft:powder_snow", vec![]),
        ("minecraft:glass_pane", pane(&[("north", "true")])),
        ("minecraft:glass_pane", pane(&[("south", "true")])),
        ("minecraft:glass_pane", pane(&[])),
        ("minecraft:iron_bars", pane(&[("south", "true")])),
        ("minecraft:oak_leaves", plain()),
        ("minecraft:mangrove_roots", plain()),
        (
            "minecraft:muddy_mangrove_roots",
            vec![("axis".to_string(), "y".to_string())],
        ),
        ("minecraft:modded_block", vec![]),
    ])
}

#[test]
fn glass_skips_against_glass_both_ways() {
    let t = SkipTable::from_dump(&states(), &dump());
    let s = states();
    let glass = s.state(2);
    // HalfTransparentBlock: the neighbour being the same block skips the
    // shared face, from either side of it.
    assert!(t.skips(glass, glass, Direction::North));
    assert!(t.skips(glass, glass, Direction::Up));
    // A different HalfTransparentBlock-family block (tinted glass) does not:
    // vanilla's `neighborState.is(this)` compares blocks, not classes.
    let tinted = s.state(3);
    assert!(!t.skips(glass, tinted, Direction::North));
    assert!(!t.skips(tinted, glass, Direction::South));
    // Powder snow carries the same rule as its own override.
    let snow = s.state(4);
    assert!(t.skips(snow, snow, Direction::West));
    assert!(!t.skips(snow, glass, Direction::West));
}

#[test]
fn glass_keeps_its_face_against_other_blocks() {
    let t = SkipTable::from_dump(&states(), &dump());
    let s = states();
    let glass = s.state(2);
    // The rule itself keeps the face against stone (the full-opaque check
    // before it may still cull it, as in vanilla shouldRenderFace).
    assert!(!t.skips(glass, s.state(1), Direction::North));
    // Blocks without a dump entry and out-of-table ids never skip.
    assert!(!t.skips(glass, s.state(12), Direction::North));
    assert!(!t.skips(s.state(12), s.state(12), Direction::North));
    assert!(!t.skips(glass, BlockStateId(999), Direction::North));
    assert!(!t.skips(BlockStateId(999), glass, Direction::North));
    // The absent table (no dump) skips nothing at all.
    let absent = SkipTable::absent();
    assert!(!absent.skips(glass, glass, Direction::North));
}

#[test]
fn panes_follow_iron_bars_connections() {
    let t = SkipTable::from_dump(&states(), &dump());
    let s = states();
    let (north_conn, south_conn, unconnected) = (s.state(5), s.state(6), s.state(7));
    // A pane's north face towards a south-connected pane: both sides of the
    // connection are set, so the face skips.
    assert!(t.skips(north_conn, south_conn, Direction::North));
    assert!(t.skips(south_conn, north_conn, Direction::South));
    // Either side missing keeps the face (vanilla reads both states'
    // connection properties).
    assert!(!t.skips(unconnected, south_conn, Direction::North));
    assert!(!t.skips(north_conn, unconnected, Direction::North));
    // Vertical faces between bars blocks skip regardless of connections.
    assert!(t.skips(north_conn, unconnected, Direction::Up));
    assert!(t.skips(unconnected, north_conn, Direction::Down));
    // The cross-block half of the rule (vanilla's #minecraft:bars tag,
    // approximated by the IronBarsBlock class): a pane connected north
    // against iron bars connected south skips; a non-bars neighbour (glass)
    // does not.
    let bars = s.state(8);
    assert!(t.skips(north_conn, bars, Direction::North));
    assert!(!t.skips(north_conn, s.state(2), Direction::North));
}

#[test]
fn leaves_and_mangrove_roots_follow_their_rules() {
    let mut t = SkipTable::from_dump(&states(), &dump());
    let s = states();
    let leaves = s.state(9);
    // Cutout leaves (vanilla's default graphics option, Haste's only mode):
    // leaves never skip.
    assert!(t.cutout_leaves);
    assert!(!t.skips(leaves, leaves, Direction::North));
    // With cutout leaves off (vanilla's "fast" leaves), they do.
    t.cutout_leaves = false;
    assert!(t.skips(leaves, leaves, Direction::North));
    assert!(!t.skips(leaves, s.state(2), Direction::North));
    // Mangrove roots: vertical faces against clean mangrove roots only.
    let roots = s.state(10);
    let muddy = s.state(11);
    assert!(t.skips(roots, roots, Direction::Up));
    assert!(t.skips(roots, roots, Direction::Down));
    assert!(!t.skips(roots, roots, Direction::North));
    assert!(!t.skips(roots, muddy, Direction::Up));
}

#[test]
fn dump_version_is_checked() {
    assert!(BlockClassDump::parse(r#"{"version":2,"blocks":{}}"#).is_err());
    assert!(BlockClassDump::parse(r#"{"version":1,"blocks":{}}"#).is_ok());
}

// ------------------------------------------------------------------ mesher tests

/// The dump fixture as a table over the real blocks.json states.
fn glass_dump_table(
    states: &dyn BlockStateTable,
) -> (crate::mesher::SkipTable, crate::mesher::SkipTable) {
    let skip = SkipTable::from_dump(states, &dump());
    let none = SkipTable::absent();
    (skip, none)
}

/// Four glass pillars two deep (x = 0, 2, 4, 6, y 0, z 0..2 — spaced so only
/// the z-neighbours share a face): the shared faces must vanish through the
/// HalfTransparentBlock rule, both directions and only those (glass is
/// translucent, so nothing merges and the quad count is exact).
/// (Needs the client jar and datagen reports in the cache; without them the
/// helper returns and the test passes as skipped.)
#[test]
fn glass_wall_interior_faces_cull_in_the_mesh() {
    let Some((states, baked, atlas)) = crate::mesher::tests::load_baked() else {
        return;
    };
    let glass = find_state(&states, "glass", &[]);
    struct Wall {
        glass: BlockStateId,
    }
    impl haste_core::view::BlockView for Wall {
        fn block_state(&self, p: BlockPos) -> BlockStateId {
            if p.y == 0 && (0..8).contains(&p.x) && p.x % 2 == 0 && (0..2).contains(&p.z) {
                self.glass
            } else {
                BlockStateId::AIR
            }
        }
        fn is_chunk_loaded(&self, _: haste_core::ChunkPos) -> bool {
            true
        }
        fn sky_light(&self, p: BlockPos) -> u8 {
            if p.y < 0 {
                0
            } else {
                15
            }
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
    let world = Wall { glass };
    let pos = SectionPos::new(0, 0, 0);
    let snapshot = MeshSnapshot::capture(&world, pos);
    let (skip, none) = glass_dump_table(&states);
    let quads = |skip: crate::mesher::SkipTable| {
        let tables = tables_with(&states, skip);
        let mesh = mesh_section_with(&snapshot, &baked, &atlas, &tables, pos);
        mesh.layers
            .iter()
            .map(|l| l.indices.len() / 6)
            .sum::<usize>()
    };
    let (with, without) = (quads(skip), quads(none.clone()));
    // Exactly the interior plane's faces disappear: 4 columns, each losing
    // its south face (z=0 block) and north face (z=1 block) — one quad each
    // in the glass model.
    let south = baked
        .quads(glass)
        .iter()
        .filter(|q| q.cullface == Some(Direction::South))
        .count();
    let north = baked
        .quads(glass)
        .iter()
        .filter(|q| q.cullface == Some(Direction::North))
        .count();
    assert_eq!(south, 1, "glass model: one south-cullface quad");
    assert_eq!(north, 1, "glass model: one north-cullface quad");
    // 8 glass blocks x 6 faces = 48 quads; the 8 interior ones are culled.
    assert_eq!((with, without), (40, 48), "glass wall quad counts");
    assert_eq!(without - with, 8, "4 columns x 2 interior faces culled");
    assert!(with > 0, "the wall's outer faces stay");
}

/// A connected pane pair hides the arm faces between them; an unconnected
/// pair keeps them. Panes follow IronBarsBlock: the horizontal face skips
/// only when both states connect towards each other.
/// (Needs the client jar and datagen reports in the cache; without them the
/// helper returns and the test passes as skipped.)
#[test]
fn connected_pane_pair_culls_the_shared_faces() {
    let Some((states, baked, atlas)) = crate::mesher::tests::load_baked() else {
        return;
    };
    let connected = find_state(
        &states,
        "glass_pane",
        &[("north", "true"), ("south", "true")],
    );
    let unconnected = find_state(
        &states,
        "glass_pane",
        &[("north", "false"), ("south", "false")],
    );
    struct Pair {
        a: BlockStateId,
        b: BlockStateId,
    }
    impl haste_core::view::BlockView for Pair {
        fn block_state(&self, p: BlockPos) -> BlockStateId {
            if p.y == 0 && p.x == 0 {
                match p.z {
                    0 => self.a,
                    1 => self.b,
                    _ => BlockStateId::AIR,
                }
            } else {
                BlockStateId::AIR
            }
        }
        fn is_chunk_loaded(&self, _: haste_core::ChunkPos) -> bool {
            true
        }
        fn sky_light(&self, p: BlockPos) -> u8 {
            if p.y < 0 {
                0
            } else {
                15
            }
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
    let pos = SectionPos::new(0, 0, 0);
    let (skip, none) = glass_dump_table(&states);
    let quads = |a, b, skip: crate::mesher::SkipTable| {
        let world = Pair { a, b };
        let snapshot = MeshSnapshot::capture(&world, pos);
        let tables = tables_with(&states, skip);
        let mesh = mesh_section_with(&snapshot, &baked, &atlas, &tables, pos);
        mesh.layers
            .iter()
            .map(|l| l.indices.len() / 6)
            .sum::<usize>()
    };
    // Connected pair: the arm faces on the shared plane disappear.
    let (with, without) = (
        quads(connected, connected, skip.clone()),
        quads(connected, connected, none),
    );
    let south = baked
        .quads(connected)
        .iter()
        .filter(|q| q.cullface == Some(Direction::South))
        .count();
    let north = baked
        .quads(connected)
        .iter()
        .filter(|q| q.cullface == Some(Direction::North))
        .count();
    // 44 quads for two panes; the connected pair's two arm faces are culled.
    assert_eq!((with, without), (42, 44), "pane pair quad counts");
    assert_eq!(
        without - with,
        south + north,
        "the pane pair's shared faces"
    );
    assert!(without > with);
    // Unconnected pair: nothing changes (IronBarsBlock keeps the face when
    // either side does not connect).
    let (with, without) = (
        quads(unconnected, unconnected, skip),
        quads(unconnected, unconnected, SkipTable::absent()),
    );
    assert_eq!(with, without, "unconnected panes keep every face");
}

/// A dumped default block (stone) culls nothing new: with the class dump's
/// stone entry installed, the mesh is quad-for-quad the no-skip mesh.
/// (Needs the client jar and datagen reports in the cache; without them the
/// helper returns and the test passes as skipped.)
#[test]
fn default_rule_culls_nothing_new() {
    let Some((states, baked, atlas)) = crate::mesher::tests::load_baked() else {
        return;
    };
    let stone = BlockStateId(1);
    // Two layers of stone over air (a private stand-in for mesher/tests.rs's
    // FlatWorld, whose fields stay private to that module).
    struct Stone {
        ground: BlockStateId,
    }
    impl haste_core::view::BlockView for Stone {
        fn block_state(&self, p: BlockPos) -> BlockStateId {
            if (0..2).contains(&p.y) && (0..16).contains(&p.x) && (0..16).contains(&p.z) {
                self.ground
            } else {
                BlockStateId::AIR
            }
        }
        fn is_chunk_loaded(&self, _: haste_core::ChunkPos) -> bool {
            true
        }
        fn sky_light(&self, p: BlockPos) -> u8 {
            if p.y <= 0 {
                0
            } else {
                15
            }
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
    let world = Stone { ground: stone };
    let pos = SectionPos::new(0, 0, 0);
    let snapshot = MeshSnapshot::capture(&world, pos);
    let (skip, none) = glass_dump_table(&states);
    let quads = |skip: crate::mesher::SkipTable| {
        let tables = tables_with(&states, skip);
        let mesh = mesh_section_with(&snapshot, &baked, &atlas, &tables, pos);
        mesh.layers
            .iter()
            .map(|l| l.indices.len() / 6)
            .sum::<usize>()
    };
    assert_eq!(quads(skip), quads(none));
}

/// MeshTables with the given skip table (the mesher tests' standard set).
fn tables_with(
    states: &dyn BlockStateTable,
    skip: crate::mesher::SkipTable,
) -> crate::mesher::MeshTables {
    crate::mesher::MeshTables::new(
        std::sync::Arc::new(crate::tint::TintTable::new(states)),
        std::sync::Arc::new(crate::tint::BiomeColors::default()),
        std::sync::Arc::new(crate::fluid::FluidTable::empty()),
        std::sync::Arc::new(crate::mesher::AoTable::all_solid(states.len())),
        std::sync::Arc::new(crate::fluid::FluidOccluders::absent()),
        std::sync::Arc::new(crate::mesher::FaceOccluders::absent()),
        std::sync::Arc::new(skip),
    )
}

/// The state of `path` (blocks.json name, no namespace) whose properties
/// match `want` (unlisted properties take any value).
fn find_state(states: &dyn BlockStateTable, path: &str, want: &[(&str, &str)]) -> BlockStateId {
    (0..states.len() as u32)
        .map(BlockStateId)
        .find(|s| {
            states.block_of(*s).path() == path
                && want
                    .iter()
                    .all(|(k, v)| states.property_value(*s, k) == Some(v))
        })
        .unwrap_or_else(|| panic!("{path} in blocks.json"))
}
