//! Absent sky-light sections read the bottom row of the nearest section above with data,
//! like vanilla `SkyLightSectionStorage.getLightValue` (which flattens y and walks up).
//! All on synthetic chunk payloads (no cache needed).

use std::sync::Arc;

use haste_core::game::DimensionInfo;
use haste_core::view::{BlockView, DirtyTracker};
use haste_core::{BlockPos, BlockStateId, ChunkPos, SectionPos};
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

/// GameData stub is not possible (no Default), so tests need the fixture loader from
/// haste-data's test helper... simplest: build the tiny fixture here.
fn game_data() -> Arc<GameData> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "haste-world-sky-light-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let reports = dir.join("reports");
    std::fs::create_dir_all(&reports).unwrap();
    std::fs::write(reports.join("blocks.json"), FIXTURE_BLOCKS).unwrap();
    std::fs::write(reports.join("registries.json"), FIXTURE_REGISTRIES).unwrap();
    Arc::new(GameData::load_from(&reports, None).unwrap())
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

/// A world with one stone chunk (sections -4..24, all present) and no light data.
fn world() -> World {
    let mut w = World::new(overworld(), game_data());
    let cd = synthetic_chunk_data(ChunkPos::new(0, 0), -4, 24, |_si, _x, _y, _z| {
        BlockStateId(1)
    });
    w.load_chunk(&cd).unwrap();
    let _ = w.take_dirty_sections();
    w
}

/// A sky array whose bottom row (y = 0) is `v` at every x, z; other rows are 0.
fn sky_bottom_row(v: u8) -> bytes::Bytes {
    let mut sky = vec![0u8; 2048];
    // nibble index y*256 + z*16 + x: only y = 0
    for z in 0..16usize {
        for x in 0..16usize {
            let i = z * 16 + x;
            sky[i / 2] |= if i.is_multiple_of(2) { v } else { v << 4 };
        }
    }
    bytes::Bytes::from(sky)
}

#[test]
fn absent_section_reads_bottom_row_above() {
    let mut w = world();
    // Data at wire index 9 (section y 4, blocks 64..79): bottom row 4, other rows 0.
    // Data at wire index 11 (section y 6, blocks 96..111): bottom row 8, other rows 0.
    // Sections y 0..3 (blocks -64..63) and y 5 (blocks 80..95) arrive absent.
    let light = haste_core::chunk::LightData {
        sky_mask: vec![(1 << 9) | (1 << 11)],
        block_mask: vec![0],
        empty_sky_mask: vec![0],
        empty_block_mask: vec![0],
        sky_arrays: vec![sky_bottom_row(4), sky_bottom_row(8)],
        block_arrays: vec![],
    };
    w.apply_light(ChunkPos::new(0, 0), &light);
    let _ = w.take_dirty_sections();

    // Every cell of an absent section reads 4, whatever its y (vanilla flattens y for
    // the walk): the bottom row of the nearest section above, at the same x, z.
    for y in [-64, -60, -49, 0, 63] {
        assert_eq!(w.sky_light(BlockPos::new(0, y, 0)), 4);
        assert_eq!(w.sky_light(BlockPos::new(5, y, 7)), 4);
        assert_eq!(w.sky_light(BlockPos::new(15, y, 15)), 4);
    }
    // The absent section sandwiched between the two takes the nearest one above
    // (section y 6, row 8), not the farther one below it.
    assert_eq!(w.sky_light(BlockPos::new(5, 80, 7)), 8);
    assert_eq!(w.sky_light(BlockPos::new(5, 95, 7)), 8);
    // The present sections themselves read their own arrays: bottom row, then 0.
    assert_eq!(w.sky_light(BlockPos::new(5, 64, 7)), 4);
    assert_eq!(w.sky_light(BlockPos::new(5, 65, 7)), 0);
    assert_eq!(w.sky_light(BlockPos::new(5, 79, 7)), 0);
    assert_eq!(w.sky_light(BlockPos::new(5, 96, 7)), 8);
    assert_eq!(w.sky_light(BlockPos::new(5, 97, 7)), 0);

    // The snapshots the mesher captures from carry the same walk.
    let snap = w.snapshot_section(SectionPos::new(0, 0, 0)).unwrap();
    assert_eq!(snap.sky_light(5, 0, 7), 4);
    assert_eq!(snap.sky_light(5, 15, 7), 4);
    let snap = w.snapshot_section(SectionPos::new(0, 5, 0)).unwrap();
    assert_eq!(snap.sky_light(5, 0, 7), 8);
    assert_eq!(snap.sky_light(5, 15, 7), 8);
    let snap = w.snapshot_section(SectionPos::new(0, 4, 0)).unwrap();
    assert_eq!(snap.sky_light(5, 0, 7), 4);
    assert_eq!(snap.sky_light(5, 1, 7), 0);
}

#[test]
fn absent_section_under_nothing_reads_15() {
    let w = world();
    // No section anywhere above has sky data: 15, as before.
    assert_eq!(w.sky_light(BlockPos::new(0, 0, 0)), 15);
    assert_eq!(w.sky_light(BlockPos::new(5, 79, 7)), 15);
    // Top section of the column: nothing above it.
    assert_eq!(w.sky_light(BlockPos::new(5, 380, 7)), 15);
    let snap = w.snapshot_section(SectionPos::new(0, 0, 0)).unwrap();
    assert_eq!(snap.sky_light(0, 0, 0), 15);

    // A section under an all-15 (deduplicated to absent) one reads 15.
    let mut w2 = world();
    let light = haste_core::chunk::LightData {
        sky_mask: vec![1 << 9],
        block_mask: vec![0],
        empty_sky_mask: vec![0],
        empty_block_mask: vec![0],
        sky_arrays: vec![bytes::Bytes::from(vec![0xFF; 2048])],
        block_arrays: vec![],
    };
    w2.apply_light(ChunkPos::new(0, 0), &light);
    let _ = w2.take_dirty_sections();
    assert_eq!(w2.sky_light(BlockPos::new(5, 0, 7)), 15);
}
