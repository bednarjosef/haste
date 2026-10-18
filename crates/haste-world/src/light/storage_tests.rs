//! Tests for `storage::light_dampening`: the oracle dump's `light_dampening` field wins, and
//! the old approximation only runs for dumps that predate the field.

use haste_core::BlockStateId;
use haste_data::GameData;

use super::storage::light_dampening;

/// air 0, stone 1, glass 2, tinted_glass 3, water 4, oak_leaves 7;
/// stone_slab 5 (waterlogged) / 6 (dry).
const BLOCKS_JSON: &str = r#"{
  "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
  "minecraft:stone": { "states": [ { "id": 1, "default": true } ] },
  "minecraft:glass": { "states": [ { "id": 2, "default": true } ] },
  "minecraft:tinted_glass": { "states": [ { "id": 3, "default": true } ] },
  "minecraft:water": { "states": [ { "id": 4, "default": true } ] },
  "minecraft:stone_slab": {
    "properties": { "waterlogged": ["true", "false"] },
    "states": [
      { "id": 5, "default": true, "properties": { "waterlogged": "true" } },
      { "id": 6, "properties": { "waterlogged": "false" } }
    ]
  },
  "minecraft:oak_leaves": { "states": [ { "id": 7, "default": true } ] }
}"#;

const REGISTRIES_JSON: &str = r#"{
  "minecraft:block": {
    "default": "minecraft:air",
    "entries": {
      "minecraft:air": { "protocol_id": 0 },
      "minecraft:stone": { "protocol_id": 1 },
      "minecraft:glass": { "protocol_id": 2 },
      "minecraft:tinted_glass": { "protocol_id": 3 },
      "minecraft:water": { "protocol_id": 4 },
      "minecraft:stone_slab": { "protocol_id": 5 },
      "minecraft:oak_leaves": { "protocol_id": 6 }
    }
  }
}"#;

/// A `GameData` with the fixture blocks and `oracle` as `block_shapes.json` (None: the
/// fallback table, no oracle dump at all). Each call gets its own temp dir.
fn game_data(oracle: Option<&str>) -> GameData {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("haste-world-light-test-{}-{n}", std::process::id()));
    let reports = dir.join("reports");
    std::fs::create_dir_all(&reports).unwrap();
    std::fs::write(reports.join("blocks.json"), BLOCKS_JSON).unwrap();
    std::fs::write(reports.join("registries.json"), REGISTRIES_JSON).unwrap();
    let oracle_path = oracle.map(|o| {
        let p = dir.join("block_shapes.json");
        std::fs::write(&p, o).unwrap();
        p
    });
    GameData::load_from(&reports, oracle_path.as_deref()).unwrap()
}

/// The dump's `light_dampening` is used as-is: the values vanilla's server reports for the
/// states the approximation used to guess at (glass 0, tinted glass 15, waterlogged slab 1).
#[test]
fn the_dump_value_wins() {
    let oracle = r#"{
  "version": 1,
  "states": [
    { "id": 0, "light_dampening": 0 },
    { "id": 1, "occlusion": [[0,0,0,1,1,1]], "light_dampening": 15 },
    { "id": 2, "occlusion": [], "light_dampening": 0 },
    { "id": 3, "occlusion": [], "light_dampening": 15 },
    { "id": 4, "occlusion": [], "light_dampening": 1 },
    { "id": 5, "occlusion": [[0,0,0,1,0.5,1]], "light_dampening": 1 },
    { "id": 6, "occlusion": [[0,0,0,1,0.5,1]], "light_dampening": 0 },
    { "id": 7, "occlusion": [], "light_dampening": 1 }
  ]
}"#;
    let data = game_data(Some(oracle));
    let d = |id: u32| light_dampening(&data, BlockStateId(id));
    assert_eq!(d(0), 0, "air");
    assert_eq!(d(1), 15, "stone");
    assert_eq!(d(2), 0, "glass");
    assert_eq!(d(3), 15, "tinted glass");
    assert_eq!(d(4), 1, "water");
    assert_eq!(d(5), 1, "waterlogged slab");
    assert_eq!(d(6), 0, "dry slab");
    assert_eq!(d(7), 1, "leaves");
}

/// Dumps from before the field keep the approximation: same answers where the rule is right
/// (glass, tinted glass, water), guessed from names and shape fullness as before.
#[test]
fn old_dumps_keep_the_approximation() {
    let oracle = r#"{
  "version": 1,
  "states": [
    { "id": 1, "collision": [[0,0,0,1,1,1]], "outline": [[0,0,0,1,1,1]], "opaque": true },
    { "id": 2, "collision": [[0,0,0,1,1,1]], "outline": [[0,0,0,1,1,1]] },
    { "id": 3, "collision": [[0,0,0,1,1,1]], "outline": [[0,0,0,1,1,1]] },
    { "id": 4, "collision": [], "outline": [], "fluid": "minecraft:water" },
    { "id": 5, "collision": [[0,0,0,1,0.5,1]], "outline": [[0,0,0,1,0.5,1]], "fluid": "minecraft:water" },
    { "id": 6, "collision": [[0,0,0,1,0.5,1]], "outline": [[0,0,0,1,0.5,1]] }
  ]
}"#;
    let data = game_data(Some(oracle));
    let d = |id: u32| light_dampening(&data, BlockStateId(id));
    assert_eq!(d(1), 15, "stone (opaque)");
    assert_eq!(d(2), 0, "glass (path rule)");
    assert_eq!(d(3), 15, "tinted glass (path rule)");
    assert_eq!(d(4), 1, "water (fluid)");
    assert_eq!(d(5), 1, "waterlogged slab (fluid)");
    assert_eq!(d(6), 0, "dry slab (not a full cube)");

    // No oracle dump at all: the approximation over the fallback shapes, which know no
    // `opaque` flag (every unnamed full cube is opaque) — glass diverges there (15, the
    // documented fallback gap the real dump closes).
    let fallback = game_data(None);
    let d = |id: u32| light_dampening(&fallback, BlockStateId(id));
    assert_eq!(d(1), 15, "stone (full cube)");
    assert_eq!(d(2), 15, "glass (fallback gap: opaque full cube)");
    assert_eq!(d(3), 15, "tinted glass (opaque full cube)");
    assert_eq!(d(4), 1, "water (fluid)");
}
