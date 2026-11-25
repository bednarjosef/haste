//! Tests for the block sound types: the recorder mod's `block_sounds.json` dump (per-block
//! defaults, per-state overrides) and the absent-file case.

use haste_core::Identifier;

use super::{write_fixture, write_fixture_with, BLOCKS_JSON, REGISTRIES_JSON};
use crate::GameData;

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
