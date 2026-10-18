//! Tests for `BlockPhysics for GameData`: friction/speed/jump, breaking data, fluids, the
//! block movement-effect identities and the water flow direction.

use haste_core::BlockStateId;
use haste_core::Identifier;

use super::{write_fixture, write_fixture_dir};

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
    let d = crate::GameData::load_from(&dir.join("reports"), Some(&dir.join("block_shapes.json")))
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
    let d = crate::GameData::load_from(&reports, None).unwrap();
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
        eprintln!("skipped: no real reports at {reports:?} (set HASTE_CACHE or run haste setup)");
        return;
    }
    let d = crate::GameData::load().unwrap();
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
    let d = crate::GameData::load_from(&reports, None).unwrap();
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
    // A pool's edge: source water with air (over stone) on one side. Vanilla's getFlow
    // reads the empty neighbour's below cell, finds no fluid there, and contributes
    // nothing (distance 0), so still water at a pool's edge pushes nothing.
    let mut w = World(Default::default());
    w.0.insert(origin, source);
    w.0.insert(BlockPos::new(1, 0, 0), source);
    w.0.insert(BlockPos::new(0, 0, 1), source);
    w.0.insert(BlockPos::new(0, 0, -1), source);
    w.0.insert(BlockPos::new(0, -1, 1), stone);
    let flow = d.water_flow(&w, origin, source).unwrap();
    assert_eq!(flow, DVec3::ZERO, "edge flow: {flow:?}");
    // The same edge through the real oracle data (the live rig's geometry): the stone
    // below the air neighbour is what the lane's edge looks like on the server.
    let reports = haste_core::paths::datagen_reports();
    if !reports.join("blocks.json").is_file() {
        eprintln!("skipped: no real reports for the edge-flow check");
        return;
    }
    let real = crate::GameData::load().unwrap();
    let water = real
        .blocks
        .default_state(&Identifier::minecraft("water"))
        .unwrap();
    let stone_real = real
        .blocks
        .default_state(&Identifier::minecraft("stone"))
        .unwrap();
    let mut wr = World(Default::default());
    wr.0.insert(origin, water);
    wr.0.insert(BlockPos::new(1, 0, 0), water);
    wr.0.insert(BlockPos::new(0, 0, 1), water);
    wr.0.insert(BlockPos::new(0, 0, -1), water);
    wr.0.insert(BlockPos::new(-1, -1, 0), stone_real);
    let flow = real.water_flow(&wr, origin, water).unwrap();
    assert_eq!(flow, DVec3::ZERO, "real edge flow: {flow:?}");
}
