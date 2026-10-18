//! Client light engine tests: a placed and removed glowstone (block light 15, 14, 13 ...
//! and back to darkness), a hole dug through a roof (skylight 15 down the column, 14
//! sideways), the re-mesh marking, and a timing measurement (run with `--ignored`).
//!
//! All on one synthetic chunk payload with a tiny oracle fixture (glowstone's light 15):
//! no cache needed. The world is a stone slab (y 96..=127) with a 9x9x16 room carved out
//! (x 4..=12, z 4..=12, y 112..=127) under a one-block roof at y 128, open sky above;
//! the slab's sections arrive dark (all-zero sky layers, like the server sends for
//! underground), so the room's cells read 0 until a test lets light in.

use std::collections::HashSet;
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

/// GameData from a fixture reports directory plus a fixture oracle (glowstone light 15),
/// so the light engine's emission and dampening come from real data paths without the cache.
fn game_data() -> Arc<GameData> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "haste-world-light-{}-{}",
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
  "minecraft:stone": { "states": [ { "id": 1, "default": true } ] },
  "minecraft:glowstone": { "states": [ { "id": 2, "default": true } ] }
}"#;

const FIXTURE_REGISTRIES: &str = r#"{
  "minecraft:block": {
    "default": "minecraft:air",
    "entries": {
      "minecraft:air": { "protocol_id": 0 },
      "minecraft:stone": { "protocol_id": 1 },
      "minecraft:glowstone": { "protocol_id": 2 }
    }
  }
}"#;

const FIXTURE_ORACLE: &str = r#"{
  "version": 1,
  "states": [
    { "id": 0, "air": true, "collision": [], "outline": [] },
    { "id": 1, "opaque": true, "collision": [[0,0,0,1,1,1]], "outline": [[0,0,0,1,1,1]] },
    { "id": 2, "light": 15, "collision": [[0,0,0,1,1,1]], "outline": [[0,0,0,1,1,1]] }
  ]
}"#;

const AIR: BlockStateId = BlockStateId(0);
const STONE: BlockStateId = BlockStateId(1);
const GLOWSTONE: BlockStateId = BlockStateId(2);

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
    // the slab's two sections arrive dark (all-zero sky layers, `light_update`'s empty
    // mask): wire indices of sections 6 (y 96..111) and 7 (y 112..127). The roof section
    // (8, wire index 13) arrives like the server sends it: 15 above the roof blocks, 0 at
    // their cells (sky light does not enter opaque blocks).
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

fn dirty_set(dirty: Vec<SectionPos>) -> HashSet<SectionPos> {
    dirty.into_iter().collect()
}

#[test]
fn glowstone_lights_neighbours_and_removal_restores_darkness() {
    let mut w = room_world();
    let source = BlockPos::new(8, 120, 8);
    w.set_block(source, GLOWSTONE);
    // vanilla BlockLightEngine: 15 at the source, one less per block in every direction
    assert_eq!(w.block_light(source), 15);
    assert_eq!(w.block_light(BlockPos::new(9, 120, 8)), 14);
    assert_eq!(w.block_light(BlockPos::new(10, 120, 8)), 13);
    assert_eq!(w.block_light(BlockPos::new(11, 120, 8)), 12);
    assert_eq!(w.block_light(BlockPos::new(7, 120, 8)), 14);
    assert_eq!(w.block_light(BlockPos::new(8, 119, 8)), 14);
    assert_eq!(w.block_light(BlockPos::new(8, 116, 8)), 11);
    assert_eq!(w.block_light(BlockPos::new(8, 125, 8)), 10);
    // the room section and its neighbours re-mesh (the light changed their cells)
    let dirty = dirty_set(w.take_dirty_sections());
    assert!(dirty.contains(&SectionPos::new(0, 7, 0)), "room section");
    assert!(dirty.contains(&SectionPos::new(1, 7, 0)), "neighbour chunk");
    // removal: the decrease pulls the light back out of every lit cell
    w.set_block(source, AIR);
    assert_eq!(w.block_light(source), 0);
    assert_eq!(w.block_light(BlockPos::new(9, 120, 8)), 0);
    assert_eq!(w.block_light(BlockPos::new(8, 116, 8)), 0);
    assert_eq!(w.block_light(BlockPos::new(10, 120, 8)), 0);
    let _ = w.take_dirty_sections();
}

#[test]
fn hole_in_roof_fills_skylight_down_the_column_and_sideways() {
    let mut w = room_world();
    // dig through the one-block roof above the room's centre column
    w.set_block(BlockPos::new(8, 128, 8), AIR);
    // 15 down the whole open column, into the room and up to the sky
    for y in 112..=128 {
        assert_eq!(w.sky_light(BlockPos::new(8, y, 8)), 15, "y {y}");
    }
    // 14 one block sideways of the column, 13 two blocks (vanilla: one less per step)
    assert_eq!(w.sky_light(BlockPos::new(9, 127, 8)), 14);
    assert_eq!(w.sky_light(BlockPos::new(10, 127, 8)), 13);
    assert_eq!(w.sky_light(BlockPos::new(8, 127, 9)), 14);
    assert_eq!(w.sky_light(BlockPos::new(9, 120, 8)), 14);
    // the stone floor below the room stays dark, and so does a column still roofed
    assert_eq!(w.sky_light(BlockPos::new(8, 111, 8)), 0);
    assert_eq!(w.sky_light(BlockPos::new(9, 111, 8)), 0);
    assert_eq!(w.sky_light(BlockPos::new(2, 120, 8)), 0);
    // the lit room sections re-mesh
    let dirty = dirty_set(w.take_dirty_sections());
    assert!(dirty.contains(&SectionPos::new(0, 7, 0)));
    assert!(dirty.contains(&SectionPos::new(0, 6, 0)));
}

#[test]
fn re_roofing_the_hole_darkens_the_column_again() {
    let mut w = room_world();
    w.set_block(BlockPos::new(8, 128, 8), AIR);
    assert_eq!(w.sky_light(BlockPos::new(8, 120, 8)), 15);
    // put the roof back: the sources below the new lowest source y are removed and the
    // spread light pulled back out of the room
    w.set_block(BlockPos::new(8, 128, 8), STONE);
    for y in 112..=127 {
        assert_eq!(w.sky_light(BlockPos::new(8, y, 8)), 0, "y {y}");
    }
    assert_eq!(w.sky_light(BlockPos::new(9, 127, 8)), 0);
    let _ = w.take_dirty_sections();
}

/// Timing: how long one light-relevant block change takes, for the cases players hit:
/// a plain stone swapped for air and back (no light change: only the pull-in probes), a
/// roof block dug and put back (skylight column refill and drain through the room), and a
/// glowstone placed and removed (the worst common case: a 15-level block light cone).
#[test]
#[ignore = "a timing measurement, not a correctness check; run with --ignored"]
fn block_change_light_cost() {
    let mut w = room_world();
    let mid = BlockPos::new(8, 120, 8);
    let roof = BlockPos::new(8, 128, 8);
    // warm up (sky sources table build, allocator)
    for _ in 0..10 {
        w.set_block(mid, STONE);
        w.set_block(mid, AIR);
        w.set_block(roof, AIR);
        w.set_block(roof, STONE);
        w.set_block(mid, GLOWSTONE);
        w.set_block(mid, AIR);
    }
    let n = 1000;
    let mut measure = |name: &str, change: &dyn Fn(&mut World)| {
        let start = std::time::Instant::now();
        for _ in 0..n {
            change(&mut w);
        }
        // two block changes per iteration (place and remove)
        let us = start.elapsed().as_nanos() as f64 / n as f64 / 2.0 / 1000.0;
        println!("{us:8.1} us per change: {name}");
        assert!(us < 500.0, "{name}: {us} us per change");
    };
    measure("stone swapped for air and back", &|w| {
        w.set_block(mid, STONE);
        w.set_block(mid, AIR);
    });
    measure("roof dug and put back", &|w| {
        w.set_block(roof, AIR);
        w.set_block(roof, STONE);
    });
    measure("glowstone placed and removed", &|w| {
        w.set_block(mid, GLOWSTONE);
        w.set_block(mid, AIR);
    });
    let _ = w.take_dirty_sections();
}
