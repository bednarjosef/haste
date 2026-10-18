//! Tests for the loader itself: the runtime report list, the missing-report check, loading
//! and looking up the fixture, oracle override, load errors, and the real (ignored) reports.

use haste_core::view::CollisionShapes as _;
use haste_core::BlockStateId;
use haste_core::Identifier;

use super::{fixture, write_fixture, BLOCKS_JSON, REGISTRIES_JSON};
use crate::{missing_reports, GameData, RUNTIME_REPORTS};

/// The runtime report list is exactly the files the loader reads; a missing components
/// report must be reported (the client warns and armour draws nothing).
#[test]
fn runtime_reports_are_the_files_the_loader_reads() {
    assert_eq!(
        RUNTIME_REPORTS,
        &[
            "blocks.json",
            "registries.json",
            "minecraft/components/item"
        ]
    );
}

#[test]
fn missing_reports_lists_absent_runtime_reports() {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "haste-missing-reports-{}-{}",
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&dir).unwrap();
    // Everything missing.
    assert_eq!(missing_reports(&dir), RUNTIME_REPORTS.to_vec());
    // Only the components report missing.
    std::fs::write(dir.join("blocks.json"), "{}").unwrap();
    std::fs::write(dir.join("registries.json"), "{}").unwrap();
    assert_eq!(missing_reports(&dir), vec!["minecraft/components/item"]);
    // The components report is a directory; creating it clears the list.
    std::fs::create_dir_all(dir.join("minecraft/components/item")).unwrap();
    assert!(missing_reports(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

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
        eprintln!("skipped: no real reports at {reports:?} (set HASTE_CACHE or run haste setup)");
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
