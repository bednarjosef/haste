//! Runtime registries loaded from the data-generator reports and oracle dumps in the cache (block states, registries, block shapes). Owner: world-and-data.
//!
//! See docs/INTERFACES.md for the contract this crate implements and AGENTS.md for the rules.

mod blocks;
// MERGE(item-data side):
mod item_components;
// MERGE(main side):
mod components;
// MERGE end
mod physics;
mod registries;
mod shapes;
mod sounds;

use std::path::Path;

pub use blocks::BlockStates;
// MERGE(item-data side):
pub use item_components::{AttributeModifier, Consumable, ItemComponents};
// MERGE(main side):
pub use components::{ItemComponents, ItemEquippable};
// MERGE end
pub use physics::{BlockProperties, BlockProps};
pub use registries::Registries;
pub use shapes::BlockShapes;
pub use sounds::{BlockSoundType, BlockSoundTypes};

/// All game data, loaded once at startup and shared (`Arc`) with the world, render, physics.
pub struct GameData {
    pub blocks: BlockStates,
    pub registries: Registries,
    pub shapes: BlockShapes,
    pub physics: BlockProperties,
    pub sounds: BlockSoundTypes,
    /// Item default components (the `equippable` component armour needs), from the report's
    /// `components/item` directory. Empty when the report is absent.
    pub item_components: ItemComponents,
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

    /// Loads from explicit paths. `oracle` is the `block_shapes.json` path, if present;
    /// `block_properties.json` and `block_sounds.json` are looked up next to it when they
    /// exist.
    pub fn load_from(reports: &Path, oracle: Option<&Path>) -> anyhow::Result<GameData> {
        let blocks_json = std::fs::read_to_string(reports.join("blocks.json"))?;
        let registries_json = std::fs::read_to_string(reports.join("registries.json"))?;
        let registries = Registries::parse(&registries_json)?;
        let blocks = BlockStates::parse(&blocks_json, Some(&registries))?;
        let shapes = match oracle {
            Some(p) => BlockShapes::parse_oracle(&std::fs::read_to_string(p)?, &blocks)?,
            None => BlockShapes::fallback(&blocks),
        };
        let sibling = |name: &str| oracle.map(|p| p.with_file_name(name));
        let physics = match sibling("block_properties.json") {
            Some(p) if p.is_file() => {
                BlockProperties::parse(&std::fs::read_to_string(p)?, &blocks)?
            }
            _ => BlockProperties::defaults(&blocks),
        };
        let sounds = match sibling("block_sounds.json") {
            Some(p) if p.is_file() => {
                BlockSoundTypes::parse(&std::fs::read_to_string(p)?, &blocks)?
            }
            _ => BlockSoundTypes::empty(&blocks),
        };
        let item_components = ItemComponents::load(
            &reports.join("minecraft").join("components").join("item"),
            &registries,
        );
        Ok(Self {
            blocks,
            registries,
            shapes,
            physics,
            sounds,
            item_components,
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
    /// `oracle`, when given, is written as `block_shapes.json` in the fixture dir.
    fn write_fixture(
        blocks: &str,
        registries: &str,
        oracle: Option<&str>,
    ) -> (std::path::PathBuf, Option<std::path::PathBuf>) {
        write_fixture_with(blocks, registries, oracle, None)
    }

    /// Like [`write_fixture`], plus a `block_sounds.json` written next to the oracle
    /// path when `sounds` is given (the sibling lookup `GameData::load_from` does).
    fn write_fixture_with(
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
    fn write_fixture_dir(blocks: &str, registries: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("haste-data-test-{}-{}", std::process::id(), n));
        let reports = dir.join("reports");
        std::fs::create_dir_all(&reports).unwrap();
        std::fs::write(reports.join("blocks.json"), blocks).unwrap();
        std::fs::write(reports.join("registries.json"), registries).unwrap();
        dir
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

    /// `BlockStateTable for BlockStates` delegates to the inherent methods.
    #[test]
    fn block_state_table_impl() {
        let d = fixture();
        // Go through the trait object so the trait impl itself is what gets exercised.
        let t: &dyn haste_core::view::BlockStateTable = &d.blocks;
        assert_eq!(t.len(), 17);
        assert!(!t.is_empty());
        assert_eq!(t.block_of(BlockStateId(1)), &Identifier::minecraft("stone"));
        assert_eq!(
            t.block_of(BlockStateId(5)),
            &Identifier::minecraft("oak_stairs")
        );
        let props = t.properties(BlockStateId(5));
        assert_eq!(props.len(), 4);
        assert_eq!(props[0], ("facing".to_owned(), "north".to_owned()));
        // name-sorted
        let names: Vec<&String> = props.iter().map(|(k, _)| k).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        assert_eq!(t.property_value(BlockStateId(5), "facing"), Some("north"));
        assert_eq!(t.property_value(BlockStateId(5), "half"), Some("top"));
        assert_eq!(t.property_value(BlockStateId(1), "facing"), None);
    }

    /// `BlockPhysics for GameData`: friction/speed/jump from `block_properties.json`
    /// (defaults 0.6/1.0/1.0), fluid height from the `level` property, light/air/opaque and
    /// waterlogged fluids from `block_shapes.json`.
    #[test]
    fn block_physics_impl() {
        use haste_core::view::BlockPhysics as _;
        // Fixture: air/stone/water(level 0..2)/oak_stairs from BLOCKS_JSON; oracle dump with
        // light/air/opaque/fluid; block_properties.json next to it.
        let oracle = r#"{
          "version": 1,
          "states": [
            { "id": 0, "collision": [], "outline": [], "light": 0, "air": true, "opaque": false },
            { "id": 1, "collision": [[0.0,0.0,0.0,1.0,1.0,1.0]], "outline": [[0.0,0.0,0.0,1.0,1.0,1.0]], "light": 0, "air": false, "opaque": true },
            { "id": 2, "collision": [], "outline": [], "light": 0, "air": false, "opaque": false,
              "fluid": "minecraft:water", "properties": { "level": "0" } },
            { "id": 3, "collision": [], "outline": [], "light": 0, "air": false, "opaque": false,
              "fluid": "minecraft:flowing_water", "properties": { "level": "1" } },
            { "id": 4, "collision": [], "outline": [], "light": 0, "air": false, "opaque": false,
              "fluid": "minecraft:flowing_water", "properties": { "level": "8" } },
            { "id": 5, "collision": [], "outline": [], "light": 0, "air": false, "opaque": false,
              "fluid": "minecraft:flowing_water", "properties": { "level": "2" } },
            { "id": 6, "collision": [[0,0,0,1,1,1]], "outline": [[0,0,0,1,1,1]], "light": 14, "air": false, "opaque": false },
            { "id": 8, "collision": [], "outline": [], "light": 0, "air": false, "opaque": false,
              "fluid": "minecraft:water" }
          ]
        }"#;
        let props = r#"{
          "version": 1,
          "minecraft": "26.3",
          "blocks": {
            "minecraft:ice": { "friction": 0.98, "speed_factor": 1.0, "jump_factor": 1.0 },
            "minecraft:slime_block": { "friction": 0.8, "speed_factor": 1.0, "jump_factor": 1.0 },
            "minecraft:honey_block": { "friction": 0.6, "speed_factor": 0.4, "jump_factor": 0.5 },
            "minecraft:stone": { "friction": 0.6, "speed_factor": 1.0, "jump_factor": 1.0,
              "destroy_time": 1.5, "requires_correct_tool_for_drops": true },
            "minecraft:oak_stairs": { "friction": 0.6, "speed_factor": 1.0, "jump_factor": 1.0,
              "destroy_time": 2.0, "requires_correct_tool_for_drops": false,
              "states": { "8": { "destroy_time": 3.0, "requires_correct_tool_for_drops": true } } }
          }
        }"#;
        // blocks.json with the physics-distinguished blocks (ice etc.) added
        let blocks_json = r#"{
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
  "minecraft:ice": { "states": [ { "id": 5, "default": true } ] },
  "minecraft:slime_block": { "states": [ { "id": 6, "default": true } ] },
  "minecraft:honey_block": { "states": [ { "id": 7, "default": true } ] },
  "minecraft:oak_stairs": {
    "properties": {
      "facing": ["north", "south", "west", "east"],
      "half": ["top", "bottom"],
      "shape": ["straight", "inner_left", "inner_right", "outer_left", "outer_right"],
      "waterlogged": ["true", "false"]
    },
    "states": [
      { "id": 8, "properties": { "facing": "north", "half": "top", "shape": "straight", "waterlogged": "true" } },
      { "id": 9, "default": true, "properties": { "facing": "north", "half": "top", "shape": "straight", "waterlogged": "false" } }
    ]
  }
}"#;
        let registries_json = r#"{
  "minecraft:block": {
    "default": "minecraft:air",
    "entries": {
      "minecraft:air": { "protocol_id": 0 },
      "minecraft:stone": { "protocol_id": 1 },
      "minecraft:water": { "protocol_id": 2 },
      "minecraft:ice": { "protocol_id": 3 },
      "minecraft:slime_block": { "protocol_id": 4 },
      "minecraft:honey_block": { "protocol_id": 5 },
      "minecraft:oak_stairs": { "protocol_id": 6 }
    }
  }
}"#;
        let dir = write_fixture_dir(blocks_json, registries_json);
        std::fs::write(dir.join("block_shapes.json"), oracle).unwrap();
        std::fs::write(dir.join("block_properties.json"), props).unwrap();
        let d = GameData::load_from(&dir.join("reports"), Some(&dir.join("block_shapes.json")))
            .unwrap();

        // friction/speed/jump from block_properties.json; missing blocks get defaults
        assert!((d.friction(BlockStateId(5)) - 0.98).abs() < 1e-6); // ice
        assert!((d.friction(BlockStateId(6)) - 0.8).abs() < 1e-6); // slime
        assert!((d.friction(BlockStateId(1)) - 0.6).abs() < 1e-6); // stone: default
        assert!((d.speed_factor(BlockStateId(7)) - 0.4).abs() < 1e-6); // honey
        assert!((d.jump_factor(BlockStateId(7)) - 0.5).abs() < 1e-6); // honey
        assert!((d.speed_factor(BlockStateId(1)) - 1.0).abs() < 1e-6);

        // breaking data: per block, per-state overrides, NaN when the dump has none
        assert_eq!(d.destroy_time(BlockStateId(1)), 1.5);
        assert!(d.requires_correct_tool(BlockStateId(1)));
        assert_eq!(d.destroy_time(BlockStateId(9)), 2.0);
        assert!(!d.requires_correct_tool(BlockStateId(9)));
        assert_eq!(d.destroy_time(BlockStateId(8)), 3.0);
        assert!(d.requires_correct_tool(BlockStateId(8)));
        assert!(d.destroy_time(BlockStateId(5)).is_nan()); // ice: no breaking data in fixture

        // fluid heights: level 0 = 8/9, 1 = 7/9, 2+ (we treat 8 as falling) = ...
        let f = d.fluid(BlockStateId(2)).unwrap();
        assert_eq!(f.kind, haste_core::view::FluidKind::Water);
        assert!((f.height - 8.0 / 9.0).abs() < 1e-6);
        let f = d.fluid(BlockStateId(3)).unwrap();
        assert!((f.height - 7.0 / 9.0).abs() < 1e-6);
        // level 8+ = falling: 8/9 like a source (vanilla server, NOTES.md)
        let f = d.fluid(BlockStateId(4)).unwrap();
        assert!((f.height - 8.0 / 9.0).abs() < 1e-6);
        // waterlogged stairs: fluid present, no level property -> full source height
        let f = d.fluid(BlockStateId(8)).unwrap();
        assert_eq!(f.kind, haste_core::view::FluidKind::Water);
        assert!((f.height - 8.0 / 9.0).abs() < 1e-6);
        // dry stairs: no fluid
        assert!(d.fluid(BlockStateId(9)).is_none());

        // light/air/opaque from block_shapes.json
        assert_eq!(d.light_emission(BlockStateId(6)), 14); // slime: from the dump
        assert_eq!(d.light_emission(BlockStateId(1)), 0);
        assert!(d.is_air(BlockStateId(0)));
        assert!(!d.is_air(BlockStateId(1)));
        assert!(d.is_opaque_full(BlockStateId(1)));
        assert!(!d.is_opaque_full(BlockStateId(0)));
        assert!(!d.is_opaque_full(BlockStateId(2))); // water: not opaque
    }

    /// `BlockPhysics for GameData`: the block movement effects (cobweb, powder snow, sweet berry
    /// bush, slime, honey) resolve from the block's registry name; `bounce_restitution` is 1.0
    /// for slime. Unknown/mod blocks (and the placeholder) keep the defaults.
    #[test]
    fn block_movement_effect_identity() {
        use haste_core::view::BlockPhysics as _;
        let blocks_json = r#"{
  "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
  "minecraft:stone": { "states": [ { "id": 1, "default": true } ] },
  "minecraft:cobweb": { "states": [ { "id": 2, "default": true } ] },
  "minecraft:powder_snow": { "states": [ { "id": 3, "default": true } ] },
  "minecraft:sweet_berry_bush": {
    "properties": { "age": ["0", "1", "2", "3"] },
    "states": [
      { "id": 4, "default": true, "properties": { "age": "0" } },
      { "id": 5, "properties": { "age": "3" } }
    ]
  },
  "minecraft:slime_block": { "states": [ { "id": 6, "default": true } ] },
  "minecraft:honey_block": { "states": [ { "id": 7, "default": true } ] }
}"#;
        let registries_json = r#"{
  "minecraft:block": {
    "default": "minecraft:air",
    "entries": {
      "minecraft:air": { "protocol_id": 0 },
      "minecraft:stone": { "protocol_id": 1 },
      "minecraft:cobweb": { "protocol_id": 2 },
      "minecraft:powder_snow": { "protocol_id": 3 },
      "minecraft:sweet_berry_bush": { "protocol_id": 4 },
      "minecraft:slime_block": { "protocol_id": 5 },
      "minecraft:honey_block": { "protocol_id": 6 }
    }
  }
}"#;
        let (reports, _) = write_fixture(blocks_json, registries_json, None);
        let d = GameData::load_from(&reports, None).unwrap();
        let state = |name: &str| {
            d.blocks
                .default_state(&Identifier::minecraft(name))
                .unwrap()
        };
        let cobweb = state("cobweb");
        let powder_snow = state("powder_snow");
        let berry = state("sweet_berry_bush");
        let slime = state("slime_block");
        let honey = state("honey_block");
        let stone = state("stone");
        let air = state("air");
        assert!(d.is_cobweb(cobweb));
        assert!(d.is_powder_snow(powder_snow));
        assert!(d.is_sweet_berry_bush(berry));
        assert!(d.is_slime(slime));
        assert!(d.is_honey(honey));
        // Each identity is exclusive and stone/air keep the defaults.
        for s in [stone, air, cobweb, powder_snow, berry, honey] {
            assert!(!d.is_cobweb(s) || s == cobweb);
            assert!(!d.is_powder_snow(s) || s == powder_snow);
            assert!(!d.is_sweet_berry_bush(s) || s == berry);
            assert!(!d.is_slime(s) || s == slime);
            assert!(!d.is_honey(s) || s == honey);
        }
        assert!(!d.is_cobweb(stone));
        assert!(!d.is_powder_snow(stone));
        assert!(!d.is_sweet_berry_bush(stone));
        assert!(!d.is_slime(stone));
        assert!(!d.is_honey(stone));
        assert!(!d.is_slime(cobweb));
        assert!(!d.is_honey(slime));
        // Bounce restitution: slime 1.0, everything else 0.0.
        assert_eq!(d.bounce_restitution(slime), 1.0);
        assert_eq!(d.bounce_restitution(stone), 0.0);
        assert_eq!(d.bounce_restitution(honey), 0.0);
        assert_eq!(d.bounce_restitution(cobweb), 0.0);
        // Out-of-range ids (modded servers) resolve to the placeholder (stone): defaults.
        let unknown = BlockStateId(1000);
        assert!(!d.is_cobweb(unknown));
        assert!(!d.is_powder_snow(unknown));
        assert!(!d.is_sweet_berry_bush(unknown));
        assert!(!d.is_slime(unknown));
        assert!(!d.is_honey(unknown));
        assert_eq!(d.bounce_restitution(unknown), 0.0);
    }

    /// Real reports from the cache: the movement-effect blocks resolve by registry name.
    /// `--ignored` (needs `cache/datagen/26.3/reports`).
    #[test]
    #[ignore = "needs cache/datagen/26.3/reports (run via haste setup; HASTE_CACHE overrides)"]
    fn real_block_movement_effects() {
        let reports = haste_core::paths::datagen_reports();
        if !reports.join("blocks.json").is_file() {
            eprintln!(
                "skipped: no real reports at {reports:?} (set HASTE_CACHE or run haste setup)"
            );
            return;
        }
        let d = GameData::load().unwrap();
        use haste_core::view::BlockPhysics as _;
        let state = |name: &str| {
            d.blocks
                .default_state(&Identifier::minecraft(name))
                .unwrap()
        };
        let cobweb = state("cobweb");
        let powder_snow = state("powder_snow");
        let berry = state("sweet_berry_bush");
        let slime = state("slime_block");
        let honey = state("honey_block");
        let stone = state("stone");
        assert!(d.is_cobweb(cobweb));
        assert!(d.is_powder_snow(powder_snow));
        assert!(d.is_sweet_berry_bush(berry));
        assert!(d.is_slime(slime));
        assert!(d.is_honey(honey));
        assert!(!d.is_cobweb(stone));
        assert!(!d.is_powder_snow(stone));
        assert!(!d.is_sweet_berry_bush(stone));
        assert!(!d.is_slime(stone));
        assert!(!d.is_honey(stone));
        assert_eq!(d.bounce_restitution(slime), 1.0);
        assert_eq!(d.bounce_restitution(stone), 0.0);
        assert_eq!(d.bounce_restitution(honey), 0.0);
    }

    /// `FlowingFluid.getFlow`: the horizontal flow points down the fluid-height gradient, and
    /// falling fluid against a solid face gets the downward component. Fallback shapes provide
    /// the fluid heights (level 0 = 8/9, level 1 = 7/9, level >= 8 = falling 8/9).
    #[test]
    fn water_flow_follows_the_height_gradient() {
        use haste_core::view::{BlockPhysics as _, BlockView};
        use haste_core::{BlockPos, ChunkPos, DVec3};
        struct World(std::collections::HashMap<BlockPos, BlockStateId>);
        impl BlockView for World {
            fn block_state(&self, pos: BlockPos) -> BlockStateId {
                *self.0.get(&pos).unwrap_or(&BlockStateId::AIR)
            }
            fn is_chunk_loaded(&self, _pos: ChunkPos) -> bool {
                true
            }
            fn sky_light(&self, _pos: BlockPos) -> u8 {
                15
            }
            fn block_light(&self, _pos: BlockPos) -> u8 {
                0
            }
            fn min_y(&self) -> i32 {
                -64
            }
            fn height(&self) -> i32 {
                384
            }
        }
        let blocks_json = r#"{
  "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
  "minecraft:stone": { "states": [ { "id": 1, "default": true } ] },
  "minecraft:water": {
    "properties": { "level": ["0", "1", "8"] },
    "states": [
      { "id": 2, "default": true, "properties": { "level": "0" } },
      { "id": 3, "properties": { "level": "1" } },
      { "id": 4, "properties": { "level": "8" } }
    ]
  }
}"#;
        let registries_json = r#"{
  "minecraft:block": {
    "default": "minecraft:air",
    "entries": {
      "minecraft:air": { "protocol_id": 0 },
      "minecraft:stone": { "protocol_id": 1 },
      "minecraft:water": { "protocol_id": 2 }
    }
  }
}"#;
        let (reports, _) = write_fixture(blocks_json, registries_json, None);
        let d = GameData::load_from(&reports, None).unwrap();
        let source = BlockStateId(2);
        let lower = BlockStateId(3);
        let falling = BlockStateId(4);
        let stone = BlockStateId(1);
        let origin = BlockPos::new(0, 0, 0);
        // Source next to lower water: flow toward it (east), normalised.
        let mut w = World(Default::default());
        w.0.insert(origin, source);
        w.0.insert(BlockPos::new(1, 0, 0), lower);
        let flow = d.water_flow(&w, origin, source).unwrap();
        assert!(
            (flow - DVec3::new(1.0, 0.0, 0.0)).length() < 1e-9,
            "{flow:?}"
        );
        // Flat water (same height all round): no flow.
        let mut w = World(Default::default());
        for p in [
            origin,
            BlockPos::new(1, 0, 0),
            BlockPos::new(-1, 0, 0),
            BlockPos::new(0, 0, 1),
            BlockPos::new(0, 0, -1),
        ] {
            w.0.insert(p, source);
        }
        let flow = d.water_flow(&w, origin, source).unwrap();
        assert_eq!(flow, DVec3::ZERO);
        // Falling water against a solid face: pushes down.
        let mut w = World(Default::default());
        w.0.insert(origin, falling);
        w.0.insert(BlockPos::new(1, 0, 0), stone);
        let flow = d.water_flow(&w, origin, falling).unwrap();
        assert_eq!(flow, DVec3::new(0.0, -1.0, 0.0));
        // A non-fluid state has no flow.
        assert!(d
            .water_flow(&World(Default::default()), origin, stone)
            .is_none());
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
                !boxes.is_empty(),
                "oracle-backed stairs have no collision boxes"
            );
        }
        // Physics from the real oracle dumps (values verified against the dumps directly).
        use haste_core::view::BlockPhysics as _;
        let ice = d
            .blocks
            .default_state(&Identifier::minecraft("ice"))
            .unwrap();
        assert!((d.friction(ice) - 0.98).abs() < 1e-6);
        let slime = d
            .blocks
            .default_state(&Identifier::minecraft("slime_block"))
            .unwrap();
        assert!((d.friction(slime) - 0.8).abs() < 1e-6);
        let honey = d
            .blocks
            .default_state(&Identifier::minecraft("honey_block"))
            .unwrap();
        assert!((d.speed_factor(honey) - 0.4).abs() < 1e-6);
        assert!((d.jump_factor(honey) - 0.5).abs() < 1e-6);
        assert!((d.friction(st) - 0.6).abs() < 1e-6); // stone: default
                                                      // Water: level 0 source = 8/9, level 1 = 7/9, level 8 (falling) = 1.0; ids verified.
        let water_l = |lvl: &str| {
            d.blocks
                .state(&Identifier::minecraft("water"), &[("level", lvl)])
                .unwrap()
        };
        let f = d.fluid(water_l("0")).expect("water level 0 has fluid");
        assert_eq!(f.kind, haste_core::view::FluidKind::Water);
        assert!((f.height - 8.0 / 9.0).abs() < 1e-6);
        let f = d.fluid(water_l("1")).expect("water level 1 has fluid");
        assert!((f.height - 7.0 / 9.0).abs() < 1e-6);
        let f = d.fluid(water_l("8")).expect("water level 8 has fluid");
        // Falling water is 8/9 high (settled on the vanilla server, NOTES.md).
        assert!((f.height - 8.0 / 9.0).abs() < 1e-6, "falling water is 8/9");
        assert!(d.fluid(st).is_none()); // stone: no fluid
                                        // light/air/opaque
        let glowstone = d
            .blocks
            .default_state(&Identifier::minecraft("glowstone"))
            .unwrap();
        assert_eq!(d.light_emission(glowstone), 15);
        assert!(d.is_air(air));
        assert!(!d.is_air(st));
        assert!(d.is_opaque_full(st));
        assert!(!d.is_opaque_full(air));
        let torch = d
            .blocks
            .default_state(&Identifier::minecraft("torch"))
            .unwrap();
        assert_eq!(d.light_emission(torch), 14);
        eprintln!(
            "real reports: {} states, {} blocks, {} entity types, oracle_backed={}, load {:?}",
            d.blocks.len(),
            d.registries.len("minecraft:block"),
            d.registries.len("minecraft:entity_type"),
            d.shapes.is_oracle_backed(),
            dt
        );
        // The recorder mod's block sound dump (dump_block_sounds): stone steps, the
        // decorated pot's uncracked default (state override) differs from its cracked
        // variant (the block default, the shatter type). Present only when the oracle
        // dump was recorded (the headless walk check needs it).
        if d.sounds.is_loaded() {
            let stone = d
                .blocks
                .default_state(&Identifier::minecraft("stone"))
                .unwrap();
            assert_eq!(
                d.block_sound_type(stone).unwrap().step_sound,
                "minecraft:block.stone.step"
            );
            let pot = Identifier::minecraft("decorated_pot");
            // The pot's default state is uncracked (the state override, normal sounds).
            let uncracked = d.blocks.default_state(&pot).unwrap();
            assert_eq!(
                d.block_sound_type(uncracked).unwrap().break_sound,
                "minecraft:block.decorated_pot.break"
            );
            let cracked = d
                .blocks
                .with_property(uncracked, "cracked", "true")
                .unwrap();
            assert_eq!(
                d.block_sound_type(cracked).unwrap().break_sound,
                "minecraft:block.decorated_pot.shatter"
            );
        }
    }

    /// `block_sounds.json` (the recorder mod's `dump_block_sounds` dump): per-block
    /// defaults, per-state overrides, unknown blocks skipped, absent file = `None`
    /// everywhere.
    #[test]
    fn block_sounds_load_per_block_and_per_state() {
        // Water has three states (0/1/2); stone one (3). Waterlogged stairs-like block
        // triggers the per-state override path.
        let blocks_json = r#"{
  "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
  "minecraft:stone": { "states": [ { "id": 3, "default": true } ] },
  "minecraft:water": {
    "properties": { "level": ["0", "1", "2"] },
    "states": [
      { "id": 1, "default": true, "properties": { "level": "0" } },
      { "id": 2, "properties": { "level": "1" } }
    ]
  },
  "minecraft:decorated_pot": {
    "properties": { "cracked": ["true", "false"] },
    "states": [
      { "id": 4, "default": true, "properties": { "cracked": "true" } },
      { "id": 5, "properties": { "cracked": "false" } }
    ]
  }
}"#;
        let registries_json = r#"{
  "minecraft:block": {
    "default": "minecraft:air",
    "entries": {
      "minecraft:air": { "protocol_id": 0 },
      "minecraft:water": { "protocol_id": 1 },
      "minecraft:stone": { "protocol_id": 2 },
      "minecraft:decorated_pot": { "protocol_id": 3 }
    }
  }
}"#;
        let sounds = r#"{
  "version": 1,
  "minecraft": "26.3",
  "sound_types": [
    { "break": "minecraft:block.stone.break", "step": "minecraft:block.stone.step",
      "place": "minecraft:block.stone.place", "hit": "minecraft:block.stone.hit",
      "fall": "minecraft:block.stone.fall", "volume": 1.0, "pitch": 1.0 },
    { "break": "minecraft:block.ghast.shatter", "step": "minecraft:intentionally_empty",
      "place": "minecraft:block.ghast.shatter", "hit": "minecraft:intentionally_empty",
      "fall": "minecraft:intentionally_empty", "volume": 1.0, "pitch": 1.0 }
  ],
  "blocks": {
    "minecraft:stone": { "sound": 0 },
    "minecraft:decorated_pot": {
      "sound": 1,
      "states": [ { "properties": { "cracked": "false" }, "sound": 0 } ]
    },
    "minecraft:not_a_block": { "sound": 0 }
  }
}"#;
        // (Minimal block_shapes oracle content; the sibling lookup only needs the file.)
        let shapes = r#"{ "version": 1, "states": [] }"#;
        let (reports, oracle_path) =
            write_fixture_with(blocks_json, registries_json, Some(shapes), Some(sounds));
        let d = GameData::load_from(&reports, oracle_path.as_deref()).unwrap();
        let stone = d
            .blocks
            .default_state(&Identifier::minecraft("stone"))
            .unwrap();
        let t = d.block_sound_type(stone).unwrap();
        assert_eq!(t.step_sound, "minecraft:block.stone.step");
        assert!((t.volume - 1.0).abs() < 1e-6);
        // The decorated pot: cracked (default state 4) uses the shatter type, the
        // uncracked state 5 overrides to stone's.
        let pot = Identifier::minecraft("decorated_pot");
        let cracked = d.blocks.default_state(&pot).unwrap();
        assert_eq!(
            d.block_sound_type(cracked).unwrap().break_sound,
            "minecraft:block.ghast.shatter"
        );
        let uncracked = d.blocks.state(&pot, &[("cracked", "false")]).unwrap();
        assert_eq!(
            d.block_sound_type(uncracked).unwrap().break_sound,
            "minecraft:block.stone.break"
        );
        // Air is in the dump's blocks (nothing): no sound type.
        let air = d
            .blocks
            .default_state(&Identifier::minecraft("air"))
            .unwrap();
        assert!(d.block_sound_type(air).is_none());
        // Water (in blocks.json but not in the dump): no sound type, no panic.
        let water = d
            .blocks
            .default_state(&Identifier::minecraft("water"))
            .unwrap();
        assert!(d.block_sound_type(water).is_none());
    }

    /// Without `block_sounds.json` every lookup is `None` (the client plays nothing).
    #[test]
    fn block_sounds_absent_file_is_all_none() {
        let shapes = r#"{ "version": 1, "states": [] }"#;
        let (reports, oracle_path) = write_fixture(BLOCKS_JSON, REGISTRIES_JSON, Some(shapes));
        let d = GameData::load_from(&reports, oracle_path.as_deref()).unwrap();
        assert!(!d.sounds.is_loaded());
        let stone = d
            .blocks
            .default_state(&Identifier::minecraft("stone"))
            .unwrap();
        assert!(d.block_sound_type(stone).is_none());
    }
}
