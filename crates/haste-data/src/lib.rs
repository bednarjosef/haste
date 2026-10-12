//! Runtime registries loaded from the data-generator reports and oracle dumps in the cache (block states, registries, block shapes). Owner: WS2.
//!
//! See docs/INTERFACES.md for the contract this crate implements and AGENTS.md for the rules.

mod blocks;
mod registries;
mod shapes;

use std::path::Path;

pub use blocks::BlockStates;
pub use registries::Registries;
pub use shapes::BlockShapes;

/// All game data, loaded once at startup and shared (`Arc`) with the world, render, physics.
pub struct GameData {
    pub blocks: BlockStates,
    pub registries: Registries,
    pub shapes: BlockShapes,
}

impl GameData {
    /// Loads `datagen/26.3/reports/{blocks,registries}.json` and `oracle/26.3/block_shapes.json`
    /// (optional, fallback shapes otherwise) from `haste_core::paths`.
    pub fn load() -> anyhow::Result<GameData> {
        let reports = haste_core::paths::datagen_reports();
        let oracle = haste_core::paths::oracle_dir().join("block_shapes.json");
        let oracle = if oracle.is_file() { Some(oracle) } else { None };
        Self::load_from(&reports, oracle.as_deref())
    }

    /// Loads from explicit paths. `oracle` is the `block_shapes.json` path, if present.
    pub fn load_from(reports: &Path, oracle: Option<&Path>) -> anyhow::Result<GameData> {
        let blocks_json = std::fs::read_to_string(reports.join("blocks.json"))?;
        let registries_json = std::fs::read_to_string(reports.join("registries.json"))?;
        let registries = Registries::parse(&registries_json)?;
        let blocks = BlockStates::parse(&blocks_json, Some(&registries))?;
        let shapes = match oracle {
            Some(p) => BlockShapes::parse_oracle(&std::fs::read_to_string(p)?, blocks.len())?,
            None => BlockShapes::fallback(&blocks),
        };
        Ok(Self {
            blocks,
            registries,
            shapes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use haste_core::Identifier;

    /// Tiny hand-written fixture: a few blocks and two registries.
    const BLOCKS_JSON: &str = r#"{
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

    const REGISTRIES_JSON: &str = r#"{
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

    fn fixture() -> GameData {
        let dir = write_fixture(BLOCKS_JSON, REGISTRIES_JSON, None);
        GameData::load_from(&dir.0, dir.1.as_deref()).unwrap()
    }

    /// Writes the fixture into a unique temp dir; returns (reports dir, oracle path).
    fn write_fixture(
        blocks: &str,
        registries: &str,
        oracle: Option<&str>,
    ) -> (std::path::PathBuf, Option<std::path::PathBuf>) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("haste-data-test-{}-{}", std::process::id(), n));
        let reports = dir.join("reports");
        std::fs::create_dir_all(&reports).unwrap();
        std::fs::write(reports.join("blocks.json"), blocks).unwrap();
        std::fs::write(reports.join("registries.json"), registries).unwrap();
        let oracle_path = oracle.map(|o| {
            let p = dir.join("block_shapes.json");
            std::fs::write(&p, o).unwrap();
            p
        });
        (reports, oracle_path)
    }

    use haste_core::view::CollisionShapes as _;
    use haste_core::BlockStateId;

    #[test]
    fn fixture_loads_and_looks_up() {
        let d = fixture();
        assert_eq!(d.blocks.len(), 17);
        assert_eq!(
            d.blocks.block_of(BlockStateId(1)),
            &Identifier::minecraft("stone")
        );
        assert_eq!(
            d.blocks.block_of(BlockStateId(5)),
            &Identifier::minecraft("oak_stairs")
        );
        assert_eq!(
            d.blocks.block_of(BlockStateId(16)),
            &Identifier::minecraft("oak_stairs")
        );
        assert_eq!(
            d.blocks.default_state(&Identifier::minecraft("stone")),
            Some(BlockStateId(1))
        );
        assert_eq!(
            d.blocks.default_state(&Identifier::minecraft("oak_stairs")),
            Some(BlockStateId(7))
        );
        assert_eq!(
            d.blocks.state(
                &Identifier::minecraft("oak_stairs"),
                &[
                    ("facing", "north"),
                    ("half", "top"),
                    ("shape", "straight"),
                    ("waterlogged", "true")
                ]
            ),
            Some(BlockStateId(5))
        );
        assert_eq!(
            d.blocks.state(
                &Identifier::minecraft("oak_stairs"),
                &[
                    ("facing", "north"),
                    ("half", "bottom"),
                    ("shape", "straight"),
                    ("waterlogged", "false")
                ]
            ),
            Some(BlockStateId(16))
        );
        assert_eq!(
            d.blocks.state(&Identifier::minecraft("stone"), &[]),
            Some(BlockStateId(1))
        );
        assert_eq!(
            d.blocks
                .state(&Identifier::minecraft("water"), &[("level", "2")]),
            Some(BlockStateId(4))
        );
        // wrong property count
        assert_eq!(d.blocks.state(&Identifier::minecraft("water"), &[]), None);
        assert_eq!(
            d.blocks
                .with_property(BlockStateId(5), "waterlogged", "false"),
            Some(BlockStateId(6))
        );
        assert_eq!(
            d.blocks.with_property(BlockStateId(5), "half", "bottom"),
            Some(BlockStateId(15))
        );
        assert_eq!(
            d.blocks.property_value(BlockStateId(5), "facing"),
            Some("north")
        );
        assert_eq!(d.blocks.property_value(BlockStateId(1), "facing"), None);
        let props = d.blocks.properties(BlockStateId(5));
        assert_eq!(props.len(), 4);
        assert_eq!(props[0], ("facing".to_owned(), "north".to_owned()));
        assert_eq!(props[3], ("waterlogged".to_owned(), "true".to_owned()));
        // registries
        assert_eq!(
            d.registries
                .index("minecraft:entity_type", &Identifier::minecraft("player")),
            Some(haste_core::RegistryIndex(2))
        );
        assert_eq!(
            d.registries
                .index("minecraft:block", &Identifier::minecraft("oak_stairs")),
            Some(haste_core::RegistryIndex(3))
        );
        assert_eq!(
            d.registries
                .name("minecraft:entity_type", haste_core::RegistryIndex(1)),
            Some(&Identifier::minecraft("zombie"))
        );
        assert_eq!(d.registries.len("minecraft:entity_type"), 3);
        assert_eq!(d.registries.len("minecraft:nope"), 0);
        assert_eq!(
            d.blocks.block_index(&Identifier::minecraft("water")),
            Some(haste_core::RegistryIndex(2))
        );
        assert_eq!(
            d.blocks.block_index(&Identifier::minecraft("stone")),
            Some(haste_core::RegistryIndex(1))
        );
        // fallback shapes
        assert!(!d.shapes.is_oracle_backed());
        assert_eq!(d.shapes.collision_boxes(BlockStateId(1)).len(), 1); // stone: full cube
        assert_eq!(d.shapes.collision_boxes(BlockStateId(0)).len(), 0); // air: none
        assert_eq!(d.shapes.collision_boxes(BlockStateId(2)).len(), 0); // water: none
        assert_eq!(d.shapes.collision_boxes(BlockStateId(5)).len(), 1); // stairs: half cube
    }

    #[test]
    fn oracle_file_overrides_fallback() {
        let oracle = r#"{
          "version": 1,
          "states": [
            { "id": 1, "collision": [[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]], "outline": [[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]] },
            { "id": 5, "collision": [], "outline": [[0.25, 0.0, 0.25, 0.75, 0.5, 0.75]] }
          ]
        }"#;
        let (reports, oracle_path) = write_fixture(BLOCKS_JSON, REGISTRIES_JSON, Some(oracle));
        let d = GameData::load_from(&reports, oracle_path.as_deref()).unwrap();
        assert!(d.shapes.is_oracle_backed());
        assert_eq!(d.shapes.collision_boxes(BlockStateId(1)).len(), 1);
        assert_eq!(d.shapes.collision_boxes(BlockStateId(5)).len(), 0);
        assert_eq!(d.shapes.outline_boxes(BlockStateId(5)).len(), 1);
        assert_eq!(
            d.shapes.outline_boxes(BlockStateId(5))[0].size(),
            haste_core::DVec3::new(0.5, 0.5, 0.5)
        );
        // states not in the file: empty
        assert_eq!(d.shapes.collision_boxes(BlockStateId(2)).len(), 0);
    }

    #[test]
    fn fallback_shape_cases() {
        let blocks_json = r#"{
  "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
  "minecraft:snow": {
    "properties": { "layers": ["1", "2", "3"] },
    "states": [
      { "id": 1, "default": true, "properties": { "layers": "1" } },
      { "id": 2, "properties": { "layers": "3" } }
    ]
  },
  "minecraft:stone_slab": {
    "properties": { "type": ["top", "bottom", "double"], "waterlogged": ["true", "false"] },
    "states": [
      { "id": 3, "properties": { "type": "top", "waterlogged": "true" } },
      { "id": 4, "default": true, "properties": { "type": "top", "waterlogged": "false" } },
      { "id": 5, "properties": { "type": "bottom", "waterlogged": "true" } },
      { "id": 6, "properties": { "type": "bottom", "waterlogged": "false" } },
      { "id": 7, "properties": { "type": "double", "waterlogged": "true" } },
      { "id": 8, "properties": { "type": "double", "waterlogged": "false" } }
    ]
  },
  "minecraft:tall_grass": { "states": [ { "id": 9, "default": true } ] },
  "minecraft:torch": { "states": [ { "id": 10, "default": true } ] }
}"#;
        let registries_json = r#"{
  "minecraft:block": {
    "default": "minecraft:air",
    "entries": {
      "minecraft:air": { "protocol_id": 0 },
      "minecraft:snow": { "protocol_id": 1 },
      "minecraft:stone_slab": { "protocol_id": 2 },
      "minecraft:tall_grass": { "protocol_id": 3 },
      "minecraft:torch": { "protocol_id": 4 }
    }
  }
}"#;
        let (reports, _) = write_fixture(blocks_json, registries_json, None);
        let d = GameData::load_from(&reports, None).unwrap();
        let box_of = |s: BlockStateId| d.shapes.collision_boxes(s)[0];
        assert!((box_of(BlockStateId(1)).max.y - 1.0 / 8.0).abs() < 1e-9); // snow 1 layer
        assert!((box_of(BlockStateId(2)).max.y - 3.0 / 8.0).abs() < 1e-9); // snow 3 layers
        assert!((box_of(BlockStateId(3)).min.y - 0.5).abs() < 1e-9); // slab top
        assert!((box_of(BlockStateId(5)).max.y - 0.5).abs() < 1e-9); // slab bottom
        assert!((box_of(BlockStateId(7)).max.y - 1.0).abs() < 1e-9); // slab double
        assert_eq!(d.shapes.collision_boxes(BlockStateId(9)).len(), 0); // tall_grass keyword
        assert_eq!(d.shapes.collision_boxes(BlockStateId(10)).len(), 0); // torch keyword
    }

    #[test]
    fn load_errors() {
        // air not state 0
        let bad = r#"{
  "minecraft:air": { "states": [ { "id": 1, "default": true } ] },
  "minecraft:stone": { "states": [ { "id": 0, "default": true } ] }
}"#;
        let (reports, _) = write_fixture(bad, REGISTRIES_JSON, None);
        assert!(GameData::load_from(&reports, None).is_err());
        // non-dense
        let bad = r#"{
  "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
  "minecraft:stone": { "states": [ { "id": 3, "default": true } ] }
}"#;
        let (reports, _) = write_fixture(bad, REGISTRIES_JSON, None);
        assert!(GameData::load_from(&reports, None).is_err());
        // no default state
        let bad = r#"{
  "minecraft:air": { "states": [ { "id": 0 } ] },
  "minecraft:stone": { "states": [ { "id": 1 } ] }
}"#;
        let (reports, _) = write_fixture(bad, REGISTRIES_JSON, None);
        assert!(GameData::load_from(&reports, None).is_err());
    }

    /// Real reports from the cache. `--ignored` (needs `cache/datagen/26.3/reports`).
    #[test]
    #[ignore = "needs cache/datagen/26.3/reports (run via haste setup; HASTE_CACHE overrides)"]
    fn real_reports() {
        let reports = haste_core::paths::datagen_reports();
        if !reports.join("blocks.json").is_file() {
            eprintln!(
                "skipped: no real reports at {reports:?} (set HASTE_CACHE or run haste setup)"
            );
            return;
        }
        let t0 = std::time::Instant::now();
        let d = GameData::load().unwrap();
        let dt = t0.elapsed();
        assert!(d.blocks.len() > 20000, "only {} states", d.blocks.len());
        let stone = Identifier::minecraft("stone");
        let st = d.blocks.default_state(&stone).expect("stone default");
        assert_eq!(st, BlockStateId(1));
        assert_eq!(d.blocks.block_of(st), &stone);
        let stairs = Identifier::minecraft("oak_stairs");
        let s = d
            .blocks
            .state(
                &stairs,
                &[
                    ("facing", "east"),
                    ("half", "bottom"),
                    ("shape", "straight"),
                    ("waterlogged", "false"),
                ],
            )
            .expect("oak_stairs facing=east");
        assert_eq!(d.blocks.block_of(s), &stairs);
        assert_eq!(
            d.registries
                .index("minecraft:entity_type", &Identifier::minecraft("player")),
            Some(haste_core::RegistryIndex(159))
        );
        let air = d
            .blocks
            .default_state(&Identifier::minecraft("air"))
            .unwrap();
        assert_eq!(air, BlockStateId::AIR);
        // Oracle dump presence is optional at this stage; report which one we got.
        if d.shapes.is_oracle_backed() {
            // oak_stairs facing=east/bottom/straight/unwaterlogged = two boxes (real dump).
            let boxes = haste_core::view::CollisionShapes::collision_boxes(&d.shapes, s);
            assert!(
                boxes.len() >= 1,
                "oracle-backed stairs have no collision boxes"
            );
        }
        eprintln!(
            "real reports: {} states, {} blocks, {} entity types, oracle_backed={}, load {:?}",
            d.blocks.len(),
            d.registries.len("minecraft:block"),
            d.registries.len("minecraft:entity_type"),
            d.shapes.is_oracle_backed(),
            dt
        );
    }
}
