//! Shared test fixtures: the tiny hand-written blocks/registries reports every test module
//! loads through `GameData::load_from` (the temp dirs are unique per call).

mod block_physics;
mod loader;
mod sounds;

/// Tiny hand-written fixture: a few blocks and two registries.
pub(crate) const BLOCKS_JSON: &str = r#"{
  "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
  "minecraft:stone": { "states": [ { "id": 1, "default": true } ] },
  "minecraft:water": {
    "properties": { "level": ["0", "1", "2"] },
    "states": [
      { "id": 2, "default": true, "properties": { "level": "0" } },
      { "id": 3, "properties": { "level": "1" } },
      { "id": 4, "properties": { "level": "2" } }
    ]
  },
  "minecraft:oak_stairs": {
    "properties": {
      "facing": ["north", "south", "west", "east"],
      "half": ["top", "bottom"],
      "shape": ["straight", "inner_left", "inner_right", "outer_left", "outer_right"],
      "waterlogged": ["true", "false"]
    },
    "states": [
      { "id": 5, "properties": { "facing": "north", "half": "top", "shape": "straight", "waterlogged": "true" } },
      { "id": 6, "properties": { "facing": "north", "half": "top", "shape": "straight", "waterlogged": "false" } },
      { "id": 7, "default": true, "properties": { "facing": "north", "half": "top", "shape": "inner_left", "waterlogged": "true" } },
      { "id": 8, "properties": { "facing": "north", "half": "top", "shape": "inner_left", "waterlogged": "false" } },
      { "id": 9, "properties": { "facing": "north", "half": "top", "shape": "inner_right", "waterlogged": "true" } },
      { "id": 10, "properties": { "facing": "north", "half": "top", "shape": "inner_right", "waterlogged": "false" } },
      { "id": 11, "properties": { "facing": "north", "half": "top", "shape": "outer_left", "waterlogged": "true" } },
      { "id": 12, "properties": { "facing": "north", "half": "top", "shape": "outer_left", "waterlogged": "false" } },
      { "id": 13, "properties": { "facing": "north", "half": "top", "shape": "outer_right", "waterlogged": "true" } },
      { "id": 14, "properties": { "facing": "north", "half": "top", "shape": "outer_right", "waterlogged": "false" } },
      { "id": 15, "properties": { "facing": "north", "half": "bottom", "shape": "straight", "waterlogged": "true" } },
      { "id": 16, "properties": { "facing": "north", "half": "bottom", "shape": "straight", "waterlogged": "false" } }
    ]
  }
}"#;

pub(crate) const REGISTRIES_JSON: &str = r#"{
  "minecraft:block": {
    "default": "minecraft:air",
    "entries": {
      "minecraft:air": { "protocol_id": 0 },
      "minecraft:stone": { "protocol_id": 1 },
      "minecraft:water": { "protocol_id": 2 },
      "minecraft:oak_stairs": { "protocol_id": 3 }
    }
  },
  "minecraft:entity_type": {
    "default": "minecraft:pig",
    "entries": {
      "minecraft:pig": { "protocol_id": 0 },
      "minecraft:zombie": { "protocol_id": 1 },
      "minecraft:player": { "protocol_id": 2 }
    }
  }
}"#;

pub(crate) fn fixture() -> crate::GameData {
    let dir = write_fixture(BLOCKS_JSON, REGISTRIES_JSON, None);
    crate::GameData::load_from(&dir.0, dir.1.as_deref()).unwrap()
}

/// Writes the fixture into a unique temp dir; returns (reports dir, oracle path).
/// `oracle`, when given, is written as `block_shapes.json` in the fixture dir.
pub(crate) fn write_fixture(
    blocks: &str,
    registries: &str,
    oracle: Option<&str>,
) -> (std::path::PathBuf, Option<std::path::PathBuf>) {
    write_fixture_with(blocks, registries, oracle, None)
}

/// Like [`write_fixture`], plus a `block_sounds.json` written next to the oracle
/// path when `sounds` is given (the sibling lookup `GameData::load_from` does).
pub(crate) fn write_fixture_with(
    blocks: &str,
    registries: &str,
    oracle: Option<&str>,
    sounds: Option<&str>,
) -> (std::path::PathBuf, Option<std::path::PathBuf>) {
    let dir = write_fixture_dir(blocks, registries);
    let oracle_path = oracle.map(|o| {
        let p = dir.join("block_shapes.json");
        std::fs::write(&p, o).unwrap();
        p
    });
    if let Some(s) = sounds {
        std::fs::write(dir.join("block_sounds.json"), s).unwrap();
    }
    (dir.join("reports"), oracle_path)
}

/// Creates the fixture dir and writes blocks/registries; returns the dir itself.
pub(crate) fn write_fixture_dir(blocks: &str, registries: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("haste-data-test-{}-{}", std::process::id(), n));
    let reports = dir.join("reports");
    std::fs::create_dir_all(&reports).unwrap();
    std::fs::write(reports.join("blocks.json"), blocks).unwrap();
    std::fs::write(reports.join("registries.json"), registries).unwrap();
    dir
}
