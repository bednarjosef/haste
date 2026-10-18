//! Sky-light storage layout pins: a uniform section is one value with no array and reads
//! that value everywhere, and the light engine writing into a uniform section
//! materialises it (copy-on-write, vanilla `DataLayer.getData`) — touching only the
//! cells propagation reaches, and collapsing back to one value when it drains.
//!
//! All on one synthetic chunk payload with a tiny oracle fixture (stone opaque), like
//! `light_engine.rs`: no cache needed.

use std::sync::Arc;

use haste_core::game::DimensionInfo;
use haste_core::view::{BlockView, DirtyTracker};
use haste_core::{BlockPos, BlockStateId, ChunkPos, SectionPos};
use haste_data::GameData;
use haste_world::chunk::synthetic_chunk_data;
use haste_world::light::SkyLight;
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

/// GameData from a fixture reports directory plus a fixture oracle (stone opaque), so the
/// light engine's dampening comes from real data paths without the cache.
fn game_data() -> Arc<GameData> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "haste-world-sky-light-storage-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let reports = dir.join("reports");
    std::fs::create_dir_all(&reports).unwrap();
    std::fs::write(reports.join("blocks.json"), FIXTURE_BLOCKS).unwrap();
    std::fs::write(reports.join("registries.json"), FIXTURE_REGISTRIES).unwrap();
    std::fs::write(dir.join("block_shapes.json"), FIXTURE_ORACLE).unwrap();
    Arc::new(GameData::load_from(&reports, Some(&dir.join("block_shapes.json"))).unwrap())
}

const FIXTURE_BLOCKS: &str = r#"{
  "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
  "minecraft:stone": { "states": [ { "id": 1, "default": true } ] }
}"#;

const FIXTURE_REGISTRIES: &str = r#"{
  "minecraft:block": {
    "default": "minecraft:air",
    "entries": {
      "minecraft:air": { "protocol_id": 0 },
      "minecraft:stone": { "protocol_id": 1 }
    }
  }
}"#;

const FIXTURE_ORACLE: &str = r#"{
  "version": 1,
  "states": [
    { "id": 0, "air": true, "collision": [], "outline": [] },
    { "id": 1, "opaque": true, "collision": [[0,0,0,1,1,1]], "outline": [[0,0,0,1,1,1]] }
  ]
}"#;

const AIR: BlockStateId = BlockStateId(0);
const STONE: BlockStateId = BlockStateId(1);

/// The `light_engine.rs` room world: a stone slab (y 96..=127) with a 9x9x16 room carved
/// out (x 4..=12, z 4..=12, y 112..=127) under a one-block roof at y 128, open sky above.
/// The slab's two sections (y 6, 7) arrive dark through the empty sky mask — one uniform
/// value each — and the roof section (y 8) with its real non-uniform array.
fn room_world() -> World {
    let mut w = World::new(overworld(), game_data());
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
    let mut roof = vec![0xFFu8; 2048];
    for b in &mut roof[..128] {
        *b = 0;
    }
    let dark = haste_core::chunk::LightData {
        empty_sky_mask: vec![1 << 11 | 1 << 12],
        sky_mask: vec![1 << 13],
        sky_arrays: vec![bytes::Bytes::from(roof)],
        ..Default::default()
    };
    w.apply_light(ChunkPos::new(0, 0), &dark);
    let _ = w.take_dirty_sections();
    w
}

/// A uniform wire array (everywhere 5) is stored as one value with no array, reads 5 at
/// every cell, feeds the walk-up of the absent section below, and the mesher's snapshot
/// carries it without building an array.
#[test]
fn uniform_section_stores_one_value_and_reads_it_everywhere() {
    let mut w = World::new(overworld(), game_data());
    let cd = synthetic_chunk_data(ChunkPos::new(0, 0), -4, 24, |_, _, _, _| AIR);
    w.load_chunk(&cd).unwrap();
    // wire index 12 = section y 7 (blocks 112..127): a uniform array of 5s
    let light = haste_core::chunk::LightData {
        sky_mask: vec![1 << 12],
        sky_arrays: vec![bytes::Bytes::from(vec![0x55u8; 2048])],
        ..Default::default()
    };
    w.apply_light(ChunkPos::new(0, 0), &light);
    let _ = w.take_dirty_sections();
    let chunk = w.chunk(ChunkPos::new(0, 0)).unwrap();
    assert!(matches!(chunk.light.sky[11], SkyLight::Uniform(5)));
    for y in [112, 120, 127] {
        for (x, z) in [(0, 0), (8, 8), (15, 15)] {
            assert_eq!(w.sky_light(BlockPos::new(x, y, z)), 5, "({x},{y},{z})");
        }
    }
    // an absent section below reads the uniform value through the walk-up (its bottom row)
    assert_eq!(w.sky_light(BlockPos::new(8, 100, 8)), 5);
    // the mesher's snapshot of the uniform section shares it without building an array
    let snap = w.snapshot_section(SectionPos::new(0, 7, 0)).unwrap();
    assert_eq!(snap.sky_light(8, 8, 8), 5);
}

/// The dark sections arrive as one uniform zero, read 0 everywhere (the room before any
/// light is let in); the roof section keeps its real array.
#[test]
fn empty_marked_sections_are_uniform_zero_and_read_zero_everywhere() {
    let w = room_world();
    let chunk = w.chunk(ChunkPos::new(0, 0)).unwrap();
    assert!(matches!(chunk.light.sky[10], SkyLight::Uniform(0)));
    assert!(matches!(chunk.light.sky[11], SkyLight::Uniform(0)));
    assert!(matches!(chunk.light.sky[12], SkyLight::Array(_)));
    for y in [96, 104, 112, 120, 127] {
        for (x, z) in [(0, 0), (4, 4), (8, 8), (12, 12), (15, 15)] {
            assert_eq!(w.sky_light(BlockPos::new(x, y, z)), 0, "({x},{y},{z})");
        }
    }
}

/// Writing into a uniform section materialises it (copy-on-write): digging the roof hole
/// turns the room section into a real array whose cells are exactly what propagation
/// says — the open column lit, every cell propagation did not reach still 0 — and
/// re-roofing drains it back into one uniform value with no array.
#[test]
fn writing_into_a_uniform_section_materialises_only_the_written_cells() {
    let mut w = room_world();
    // dig through the one-block roof above the room's centre column
    w.set_block(BlockPos::new(8, 128, 8), AIR);
    let chunk = w.chunk(ChunkPos::new(0, 0)).unwrap();
    assert!(
        matches!(chunk.light.sky[11], SkyLight::Array(_)),
        "the uniform section was materialised"
    );
    // the written cells: 15 down the open column, one less per step sideways
    assert_eq!(w.sky_light(BlockPos::new(8, 120, 8)), 15);
    assert_eq!(w.sky_light(BlockPos::new(9, 120, 8)), 14);
    // every cell propagation did not reach keeps the uniform value: the stone slab
    // around the room (light does not enter opaque blocks)
    assert_eq!(w.sky_light(BlockPos::new(2, 120, 8)), 0);
    assert_eq!(w.sky_light(BlockPos::new(15, 112, 15)), 0);
    let _ = w.take_dirty_sections();

    // put the roof back: the column drains and the section collapses to one value again
    w.set_block(BlockPos::new(8, 128, 8), STONE);
    let chunk = w.chunk(ChunkPos::new(0, 0)).unwrap();
    assert!(matches!(chunk.light.sky[11], SkyLight::Uniform(0)));
    for y in 112..=127 {
        assert_eq!(w.sky_light(BlockPos::new(8, y, 8)), 0, "y {y}");
    }
    let _ = w.take_dirty_sections();
}
