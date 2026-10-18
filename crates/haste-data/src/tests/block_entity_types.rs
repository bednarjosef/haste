//! Tests for the `block_entity_types.json` loader: parsing a hand-written fixture, the
//! lookups (block entries, per-state entries, unknowns), the empty no-dump table, load
//! errors, and the `GameData` sibling lookup.

use haste_core::{BlockStateId, Identifier};

use super::{fixture, write_fixture_dir, BLOCKS_JSON, REGISTRIES_JSON};
use crate::{BlockEntityTypes, GameData};

/// A small hand-written dump (FORMATS.md): two block entries, one per-state entry.
const DUMP: &str = r#"{
  "version": 1,
  "minecraft": "26.3",
  "types": [
    { "id": 0, "type": "minecraft:chest", "valid_blocks": ["minecraft:chest"] }
  ],
  "blocks": {
    "minecraft:chest": "minecraft:chest",
    "minecraft:oak_sign": "minecraft:sign"
  },
  "states": { "5": "minecraft:sign" }
}"#;

#[test]
fn parses_and_looks_up() {
    let t = BlockEntityTypes::parse(DUMP).unwrap();
    assert!(t.is_loaded());
    let chest = Identifier::minecraft("chest");
    let sign = Identifier::minecraft("sign");
    // The block's entry, for any of its states.
    assert_eq!(t.type_of(&chest, BlockStateId(0)), Some(&chest));
    assert_eq!(t.type_of(&chest, BlockStateId(7)), Some(&chest));
    assert_eq!(
        t.type_of(&Identifier::minecraft("oak_sign"), BlockStateId(0)),
        Some(&sign)
    );
    // A per-state entry wins over the block's own (state ids are global).
    assert_eq!(
        t.type_of(&Identifier::minecraft("stone"), BlockStateId(5)),
        Some(&sign)
    );
    // Blocks and states the dump does not know carry nothing.
    assert_eq!(
        t.type_of(&Identifier::minecraft("stone"), BlockStateId(0)),
        None
    );
    assert_eq!(
        t.type_of(&Identifier::minecraft("mod:chest"), BlockStateId(0)),
        None
    );
    // The `states` key is optional.
    let no_states = BlockEntityTypes::parse(
        r#"{ "version": 1, "blocks": { "minecraft:chest": "minecraft:chest" } }"#,
    )
    .unwrap();
    assert_eq!(no_states.type_of(&chest, BlockStateId(0)), Some(&chest));
}

/// Without a dump the table is empty: no block carries an entity, nothing panics.
#[test]
fn empty_table_has_no_types() {
    let t = BlockEntityTypes::empty();
    assert!(!t.is_loaded());
    assert_eq!(
        t.type_of(&Identifier::minecraft("chest"), BlockStateId(0)),
        None
    );
}

#[test]
fn bad_dumps_are_rejected() {
    // Unknown version.
    assert!(BlockEntityTypes::parse(r#"{ "version": 2, "blocks": {}, "states": {} }"#).is_err());
    // Malformed identifiers and state ids.
    assert!(BlockEntityTypes::parse(
        r#"{ "version": 1, "blocks": { "minecraft:chest": "Not A Type" }, "states": {} }"#
    )
    .is_err());
    assert!(BlockEntityTypes::parse(
        r#"{ "version": 1, "blocks": { "Bad Block": "minecraft:chest" }, "states": {} }"#
    )
    .is_err());
    assert!(BlockEntityTypes::parse(
        r#"{ "version": 1, "blocks": {}, "states": { "x": "minecraft:sign" } }"#
    )
    .is_err());
}

/// `GameData::load_from` picks the dump up next to `block_shapes.json`.
#[test]
fn game_data_loads_the_sibling_dump() {
    let dir = write_fixture_dir(BLOCKS_JSON, REGISTRIES_JSON);
    let oracle = dir.join("block_shapes.json");
    std::fs::write(&oracle, r#"{ "version": 1, "states": [] }"#).unwrap();
    std::fs::write(dir.join("block_entity_types.json"), DUMP).unwrap();
    let data = GameData::load_from(&dir.join("reports"), Some(&oracle)).unwrap();
    assert!(data.block_entity_types.is_loaded());
    assert_eq!(
        data.block_entity_types
            .type_of(&Identifier::minecraft("chest"), BlockStateId(0)),
        Some(&Identifier::minecraft("chest"))
    );
}

/// Without the dump (no oracle at all) the table stays empty.
#[test]
fn game_data_without_the_dump_is_empty() {
    let data = fixture();
    assert!(!data.block_entity_types.is_loaded());
}
