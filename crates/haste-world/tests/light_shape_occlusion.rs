//! Shape-based light occlusion tests (vanilla `LightEngine.shapeOccludes` /
//! `ChunkSkyLightSources.isEdgeOccluded`): a bottom slab blocks sky light down through
//! its bottom face but not sideways through its open half, a top slab blocks it from
//! above, stairs block on their solid sides per facing, a full block and air keep their
//! dampening-only behaviour, and old dumps without the flags keep the pre-shape
//! behaviour.
//!
//! Same synthetic world as `light_engine.rs` (a stone slab y 96..=127 with a 9x9x16 room
//! carved out under a one-block roof at y 128, arriving dark): no cache needed, the oracle
//! fixture carries the states' occlusion shapes and the `can_occlude` /
//! `use_shape_for_light_occlusion` flags.

use std::sync::Arc;

use haste_core::game::DimensionInfo;
use haste_core::view::{BlockView, DirtyTracker};
use haste_core::{BlockPos, BlockStateId, ChunkPos};
use haste_data::GameData;
use haste_world::chunk::synthetic_chunk_data;
use haste_world::World;

fn overworld() -> DimensionInfo {
    DimensionInfo {
        name: haste_core::Identifier::minecraft("overworld"),
        dimension_type: haste_core::Identifier::minecraft("overworld"),
        min_y: -64,
        height: 384,
        has_skylight: true,
    }
}

const FIXTURE_BLOCKS: &str = r#"{
  "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
  "minecraft:stone": { "states": [ { "id": 1, "default": true } ] },
  "minecraft:stone_slab": {
    "properties": { "type": ["bottom", "top"] },
    "states": [
      { "id": 2, "default": true, "properties": { "type": "bottom" } },
      { "id": 3, "properties": { "type": "top" } }
    ]
  },
  "minecraft:oak_stairs": {
    "properties": { "facing": ["east", "west", "north"] },
    "states": [
      { "id": 4, "default": true, "properties": { "facing": "east" } },
      { "id": 5, "properties": { "facing": "west" } },
      { "id": 6, "properties": { "facing": "north" } }
    ]
  },
  "minecraft:snow": { "states": [ { "id": 7, "default": true } ] }
}"#;

const FIXTURE_REGISTRIES: &str = r#"{
  "minecraft:block": {
    "default": "minecraft:air",
    "entries": {
      "minecraft:air": { "protocol_id": 0 },
      "minecraft:stone": { "protocol_id": 1 },
      "minecraft:stone_slab": { "protocol_id": 2 },
      "minecraft:oak_stairs": { "protocol_id": 3 },
      "minecraft:snow": { "protocol_id": 4 }
    }
  }
}"#;

/// The fixture oracle, with or without the shape flags (an old dump). The states'
/// occlusion shapes and dampening are the real 26.3 dump's values for these blocks: a
/// bottom slab is the half box with light dampening 0 and `use_shape_for_light_occlusion`,
/// stairs are the base slab plus the top box on their solid side, a full snow layer is a
/// solid full cube.
fn oracle(flags: bool) -> String {
    // (id, occlusion, dampening, skylight down, use shape for light occlusion)
    let states: &[(u32, &str, u8, bool, bool)] = &[
        (0, "[]", 0, true, false),
        (1, "[[0,0,0,1,1,1]]", 15, false, false),
        (2, "[[0,0,0,1,0.5,1]]", 0, true, true),
        (3, "[[0,0.5,0,1,1,1]]", 0, true, true),
        (4, "[[0,0,0,1,0.5,1],[0.5,0.5,0,1,1,1]]", 0, true, true),
        (5, "[[0,0,0,1,0.5,1],[0,0.5,0,0.5,1,1]]", 0, true, true),
        (6, "[[0,0,0,1,0.5,1],[0,0.5,0,1,1,0.5]]", 0, true, true),
        (7, "[[0,0,0,1,0.25,1]]", 0, true, true),
    ];
    let mut out = String::from("{\n  \"version\": 1,\n  \"states\": [\n");
    for (i, &(id, occlusion, dampening, skylight, use_shape)) in states.iter().enumerate() {
        if i > 0 {
            out.push_str(",\n");
        }
        out.push_str(&format!(
            "    {{ \"id\": {id}, \"occlusion\": {occlusion}, \"light_dampening\": {dampening}, \
             \"propagates_skylight_down\": {skylight}"
        ));
        if flags {
            out.push_str(&format!(
                ", \"can_occlude\": true, \"use_shape_for_light_occlusion\": {use_shape}"
            ));
        }
        out.push_str(" }");
    }
    out.push_str("\n  ]\n}");
    out
}

/// GameData from fixture reports plus the fixture oracle, in a fresh temp directory.
fn game_data(flags: bool) -> Arc<GameData> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "haste-world-light-shape-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let reports = dir.join("reports");
    std::fs::create_dir_all(&reports).unwrap();
    std::fs::write(reports.join("blocks.json"), FIXTURE_BLOCKS).unwrap();
    std::fs::write(reports.join("registries.json"), FIXTURE_REGISTRIES).unwrap();
    std::fs::write(dir.join("block_shapes.json"), oracle(flags)).unwrap();
    Arc::new(GameData::load_from(&reports, Some(&dir.join("block_shapes.json"))).unwrap())
}

const AIR: BlockStateId = BlockStateId(0);
const STONE: BlockStateId = BlockStateId(1);
const SLAB_BOTTOM: BlockStateId = BlockStateId(2);
const SLAB_TOP: BlockStateId = BlockStateId(3);
const STAIRS_EAST: BlockStateId = BlockStateId(4);
const STAIRS_WEST: BlockStateId = BlockStateId(5);
const STAIRS_NORTH: BlockStateId = BlockStateId(6);
const SNOW: BlockStateId = BlockStateId(7);

/// The `light_engine.rs` room world, with the roof hole above the centre column already
/// dug (the column x 8, z 8 is sky-lit 15 down to the room floor).
fn lit_column_world() -> World {
    let mut w = World::new(overworld(), game_data(true));
    let cd = synthetic_chunk_data(ChunkPos::new(0, 0), -4, 24, |si, x, y, z| {
        let wy = -64 + si as i32 * 16 + y as i32;
        let room = (4..=12).contains(&x) && (4..=12).contains(&z) && (112..=127).contains(&wy);
        if wy == 128 || ((96..=127).contains(&wy) && !room) {
            STONE
        } else {
            AIR
        }
    });
    w.load_chunk(&cd).unwrap();
    // the slab's sections arrive dark (all-zero sky layers), like the server sends for
    // underground; wire indices of sections 6 (y 96..111) and 7 (y 112..127)
    let dark = haste_core::chunk::LightData {
        empty_sky_mask: vec![1 << 11 | 1 << 12],
        ..Default::default()
    };
    w.apply_light(ChunkPos::new(0, 0), &dark);
    let _ = w.take_dirty_sections();
    w.set_block(BlockPos::new(8, 128, 8), AIR);
    assert_eq!(w.sky_light(BlockPos::new(8, 120, 8)), 15);
    assert_eq!(w.sky_light(BlockPos::new(8, 119, 8)), 15);
    let _ = w.take_dirty_sections();
    w
}

/// A bottom slab in the column: the slab's own cell stays a sky source (its open top lets
/// skylight in), everything below is blocked from above and re-lit only sideways — one
/// less per step around the slab — and light still crosses the slab's open half sideways.
#[test]
fn bottom_slab_blocks_sky_light_down_through_its_bottom() {
    let mut w = lit_column_world();
    w.set_block(BlockPos::new(8, 120, 8), SLAB_BOTTOM);
    // the slab cell itself: still fully lit through its open top
    assert_eq!(w.sky_light(BlockPos::new(8, 120, 8)), 15, "the slab cell");
    // sideways out of the slab's open half: one less per step as usual
    assert_eq!(
        w.sky_light(BlockPos::new(9, 120, 8)),
        14,
        "sideways of the slab"
    );
    assert_eq!(
        w.sky_light(BlockPos::new(7, 120, 8)),
        14,
        "sideways of the slab"
    );
    // below the slab: blocked from above (the bottom face is solid), lit only around it
    // (14 at the slab's side, 13 one down, 12 under the slab)
    assert_eq!(w.sky_light(BlockPos::new(9, 119, 8)), 13, "beside, below");
    assert_eq!(w.sky_light(BlockPos::new(8, 119, 8)), 12, "under the slab");
    assert_eq!(w.sky_light(BlockPos::new(8, 118, 8)), 11, "further under");
    let _ = w.take_dirty_sections();
}

/// A top slab closes the column from above (its top face is solid): the slab's own cell
/// stops being a sky source and keeps only the sideways light that enters through its
/// open lower half.
#[test]
fn top_slab_blocks_sky_light_from_above() {
    let mut w = lit_column_world();
    w.set_block(BlockPos::new(8, 120, 8), SLAB_TOP);
    assert_eq!(w.sky_light(BlockPos::new(8, 121, 8)), 15, "above the slab");
    // the slab cell: no source anymore, lit only sideways from the cells beside it above
    // ((9, 121) is 14, one down 13, one across 12 — through the open lower half)
    assert_eq!(w.sky_light(BlockPos::new(8, 120, 8)), 12, "the slab cell");
    assert_eq!(w.sky_light(BlockPos::new(8, 119, 8)), 11, "below the slab");
    let _ = w.take_dirty_sections();
}

/// Stairs in the column block sky light through their solid side and pass it through
/// their open side, per facing: the neighbour on the solid side loses the direct light
/// from the stair cell (it keeps only the one-from-above path, 13 instead of 14), the
/// neighbour on the open side keeps it.
#[test]
fn stairs_block_sky_light_on_their_solid_sides() {
    // east-facing (top box on the east): solid east, open west
    let mut w = lit_column_world();
    w.set_block(BlockPos::new(8, 120, 8), STAIRS_EAST);
    assert_eq!(
        w.sky_light(BlockPos::new(8, 120, 8)),
        15,
        "still a source (open top half)"
    );
    assert_eq!(
        w.sky_light(BlockPos::new(9, 120, 8)),
        13,
        "blocked through the solid east"
    );
    assert_eq!(
        w.sky_light(BlockPos::new(7, 120, 8)),
        14,
        "through the open west"
    );
    let _ = w.take_dirty_sections();
    // west-facing: solid west, open east
    let mut w = lit_column_world();
    w.set_block(BlockPos::new(8, 120, 8), STAIRS_WEST);
    assert_eq!(w.sky_light(BlockPos::new(8, 120, 8)), 15, "still a source");
    assert_eq!(
        w.sky_light(BlockPos::new(7, 120, 8)),
        13,
        "blocked through the solid west"
    );
    assert_eq!(
        w.sky_light(BlockPos::new(9, 120, 8)),
        14,
        "through the open east"
    );
    let _ = w.take_dirty_sections();
    // north-facing (top box on the north): solid north, open south
    let mut w = lit_column_world();
    w.set_block(BlockPos::new(8, 120, 8), STAIRS_NORTH);
    assert_eq!(w.sky_light(BlockPos::new(8, 120, 8)), 15, "still a source");
    assert_eq!(
        w.sky_light(BlockPos::new(8, 120, 7)),
        13,
        "blocked through the solid north"
    );
    assert_eq!(
        w.sky_light(BlockPos::new(8, 120, 9)),
        14,
        "through the open south"
    );
    let _ = w.take_dirty_sections();
}

/// A full block still blocks by light dampening alone and air still lets everything
/// through; snow layers behave like a thin bottom slab.
#[test]
fn full_block_and_air_are_unchanged_and_snow_behaves_like_a_thin_slab() {
    let mut w = lit_column_world();
    w.set_block(BlockPos::new(8, 120, 8), STONE);
    assert_eq!(w.sky_light(BlockPos::new(8, 120, 8)), 0, "inside stone");
    // under stone: the stone cell is dark, so the sideways path is one step longer than
    // under a slab (whose cell stays lit)
    assert_eq!(
        w.sky_light(BlockPos::new(8, 119, 8)),
        11,
        "under stone, sideways-lit"
    );
    w.set_block(BlockPos::new(8, 120, 8), AIR);
    assert_eq!(
        w.sky_light(BlockPos::new(8, 119, 8)),
        15,
        "air restores the column"
    );
    w.set_block(BlockPos::new(8, 120, 8), SNOW);
    assert_eq!(w.sky_light(BlockPos::new(8, 120, 8)), 15, "the snow cell");
    assert_eq!(w.sky_light(BlockPos::new(8, 119, 8)), 12, "under the snow");
    let _ = w.take_dirty_sections();
}

/// Old dumps (no flags) keep the pre-shape behaviour: the slab never occludes, the column
/// runs straight through it.
#[test]
fn old_dumps_keep_the_pre_shape_column() {
    let mut w = World::new(overworld(), game_data(false));
    let cd = synthetic_chunk_data(ChunkPos::new(0, 0), -4, 24, |si, x, y, z| {
        let wy = -64 + si as i32 * 16 + y as i32;
        let room = (4..=12).contains(&x) && (4..=12).contains(&z) && (112..=127).contains(&wy);
        if wy == 128 || ((96..=127).contains(&wy) && !room) {
            STONE
        } else {
            AIR
        }
    });
    w.load_chunk(&cd).unwrap();
    let dark = haste_core::chunk::LightData {
        empty_sky_mask: vec![1 << 11 | 1 << 12],
        ..Default::default()
    };
    w.apply_light(ChunkPos::new(0, 0), &dark);
    let _ = w.take_dirty_sections();
    w.set_block(BlockPos::new(8, 128, 8), AIR);
    assert_eq!(w.sky_light(BlockPos::new(8, 119, 8)), 15);
    w.set_block(BlockPos::new(8, 120, 8), SLAB_BOTTOM);
    assert_eq!(w.sky_light(BlockPos::new(8, 120, 8)), 15, "the slab cell");
    assert_eq!(
        w.sky_light(BlockPos::new(8, 119, 8)),
        15,
        "the column runs through the slab"
    );
    let _ = w.take_dirty_sections();
}

/// Timing: how long one light-relevant block change takes with the shape test in the
/// propagation path (a slab swap exercises the per-step occlusion lookups; the plain
/// stone swap measures the common no-shape case). Run with `--ignored`.
#[test]
#[ignore = "a timing measurement, not a correctness check; run with --ignored"]
fn block_change_light_cost_with_shapes() {
    let mut w = lit_column_world();
    let mid = BlockPos::new(8, 120, 8);
    for _ in 0..10 {
        w.set_block(mid, SLAB_BOTTOM);
        w.set_block(mid, AIR);
        w.set_block(mid, STONE);
        w.set_block(mid, AIR);
    }
    let n = 1000;
    let mut measure = |name: &str, change: &dyn Fn(&mut World)| {
        let start = std::time::Instant::now();
        for _ in 0..n {
            change(&mut w);
        }
        let us = start.elapsed().as_nanos() as f64 / n as f64 / 2.0 / 1000.0;
        println!("{us:8.1} us per change: {name}");
        assert!(us < 500.0, "{name}: {us} us per change");
    };
    measure("slab swapped for air and back", &|w| {
        w.set_block(mid, SLAB_BOTTOM);
        w.set_block(mid, AIR);
    });
    measure("stone swapped for air and back", &|w| {
        w.set_block(mid, STONE);
        w.set_block(mid, AIR);
    });
    let _ = w.take_dirty_sections();
}
