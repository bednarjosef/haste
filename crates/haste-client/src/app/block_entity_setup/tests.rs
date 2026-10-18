// Tests for the session's block-entity setup against the real game data: the block names the
// state-kind map matches are vanilla's, so every family resolves to the type id the server
// synced (a name that matches nothing, or the wrong family, fails here).

use super::*;

/// The state-kind map against the real blocks: the chest family, the suffix families and the
/// types whose name differs from the block's all resolve, and a block without an entity has
/// no entry. Needs the data generator reports in the cache.
#[test]
#[ignore = "needs cache/datagen/26.3/reports (GameData::load)"]
fn state_kinds_cover_the_real_blocks() {
    let data = crate::net::load_game_data().unwrap();
    let names = block_entity_type_names(&haste_protocol::SyncedRegistries::default(), &data);
    let kinds = block_entity_state_kinds(&data, &names);
    let id = |type_path: &str| {
        names
            .iter()
            .position(|n| n.path() == type_path)
            .unwrap_or_else(|| panic!("{type_path} missing from the type registry")) as u32
    };
    let kind_of = |block: &str| {
        let state = data
            .blocks
            .default_state(&Identifier::minecraft(block))
            .unwrap_or_else(|| panic!("{block} missing from the block registry"));
        kinds.get(&state).copied()
    };
    assert_eq!(kind_of("chest"), Some(id("chest")));
    assert_eq!(kind_of("exposed_copper_chest"), Some(id("chest")));
    assert_eq!(kind_of("trapped_chest"), Some(id("trapped_chest")));
    assert_eq!(kind_of("ender_chest"), Some(id("ender_chest")));
    assert_eq!(kind_of("spawner"), Some(id("mob_spawner")));
    assert_eq!(kind_of("suspicious_sand"), Some(id("brushable_block")));
    assert_eq!(kind_of("cyan_shulker_box"), Some(id("shulker_box")));
    assert_eq!(kind_of("oak_hanging_sign"), Some(id("hanging_sign")));
    assert_eq!(kind_of("poplar_shelf"), Some(id("shelf")));
    assert_eq!(kind_of("wither_skeleton_wall_skull"), Some(id("skull")));
    // A block without an entity has no entry.
    assert_eq!(kind_of("stone"), None);
    assert_eq!(kind_of("piston_head"), None);
    // Every state of the sign, banner, skull and shulker families is in the map: with all
    // their variants and rotations the block-entity states number in the thousands.
    assert!(
        kinds.len() > 1500,
        "only {} block entity states",
        kinds.len()
    );
}
