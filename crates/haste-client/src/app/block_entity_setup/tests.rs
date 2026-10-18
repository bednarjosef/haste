// Tests for the session's block-entity setup against the real game data: the block states the
// dump gives a type to get the type id the server synced, and the pairs the deleted
// hand-written table produced are all still produced (the list was frozen into
// OLD_TABLE_PAIRS by running the old function once over the real block registry).

use std::collections::HashSet;

use haste_core::RegistryIndex;

use super::*;

/// The state-kind map against the real dump: the chest family, the suffix families and the
/// types whose name differs from the block's all resolve, and a block without an entity has
/// no entry. Needs the data generator reports and the `block_entity_types.json` oracle dump in
/// the cache.
#[test]
#[ignore = "needs cache/datagen/26.3/reports + cache/oracle/26.3/block_entity_types.json"]
fn state_kinds_cover_the_real_blocks() {
    let data = crate::net::load_game_data().unwrap();
    if !data.block_entity_types.is_loaded() {
        eprintln!(
            "skipped: no block_entity_types.json (run tools/recorder-mod/dump-server-oracles.sh)"
        );
        return;
    }
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

/// Every (block, type path) pair the deleted hand-written table produced over the real 26.3
/// block registry — frozen here by running the old function once, so the dump can be checked
/// against it without keeping the table. The dump must still produce every pair; pairs only
/// the dump has were bugs of the old table (printed, not failures).
const OLD_TABLE_PAIRS: &[(&str, &str)] = &[
    ("suspicious_sand", "brushable_block"),
    ("suspicious_gravel", "brushable_block"),
    ("dispenser", "dispenser"),
    ("moving_piston", "piston"),
    ("chiseled_bookshelf", "chiseled_bookshelf"),
    ("acacia_shelf", "shelf"),
    ("bamboo_shelf", "shelf"),
    ("birch_shelf", "shelf"),
    ("cherry_shelf", "shelf"),
    ("crimson_shelf", "shelf"),
    ("dark_oak_shelf", "shelf"),
    ("jungle_shelf", "shelf"),
    ("mangrove_shelf", "shelf"),
    ("oak_shelf", "shelf"),
    ("pale_oak_shelf", "shelf"),
    ("poplar_shelf", "shelf"),
    ("spruce_shelf", "shelf"),
    ("warped_shelf", "shelf"),
    ("spawner", "mob_spawner"),
    ("creaking_heart", "creaking_heart"),
    ("chest", "chest"),
    ("furnace", "furnace"),
    ("oak_sign", "sign"),
    ("spruce_sign", "sign"),
    ("birch_sign", "sign"),
    ("acacia_sign", "sign"),
    ("cherry_sign", "sign"),
    ("jungle_sign", "sign"),
    ("dark_oak_sign", "sign"),
    ("pale_oak_sign", "sign"),
    ("mangrove_sign", "sign"),
    ("poplar_sign", "sign"),
    ("bamboo_sign", "sign"),
    ("oak_wall_sign", "sign"),
    ("spruce_wall_sign", "sign"),
    ("birch_wall_sign", "sign"),
    ("acacia_wall_sign", "sign"),
    ("cherry_wall_sign", "sign"),
    ("jungle_wall_sign", "sign"),
    ("dark_oak_wall_sign", "sign"),
    ("pale_oak_wall_sign", "sign"),
    ("mangrove_wall_sign", "sign"),
    ("poplar_wall_sign", "sign"),
    ("bamboo_wall_sign", "sign"),
    ("oak_hanging_sign", "hanging_sign"),
    ("spruce_hanging_sign", "hanging_sign"),
    ("birch_hanging_sign", "hanging_sign"),
    ("acacia_hanging_sign", "hanging_sign"),
    ("cherry_hanging_sign", "hanging_sign"),
    ("jungle_hanging_sign", "hanging_sign"),
    ("dark_oak_hanging_sign", "hanging_sign"),
    ("pale_oak_hanging_sign", "hanging_sign"),
    ("crimson_hanging_sign", "hanging_sign"),
    ("warped_hanging_sign", "hanging_sign"),
    ("mangrove_hanging_sign", "hanging_sign"),
    ("poplar_hanging_sign", "hanging_sign"),
    ("bamboo_hanging_sign", "hanging_sign"),
    ("oak_wall_hanging_sign", "hanging_sign"),
    ("spruce_wall_hanging_sign", "hanging_sign"),
    ("birch_wall_hanging_sign", "hanging_sign"),
    ("acacia_wall_hanging_sign", "hanging_sign"),
    ("cherry_wall_hanging_sign", "hanging_sign"),
    ("jungle_wall_hanging_sign", "hanging_sign"),
    ("dark_oak_wall_hanging_sign", "hanging_sign"),
    ("pale_oak_wall_hanging_sign", "hanging_sign"),
    ("mangrove_wall_hanging_sign", "hanging_sign"),
    ("poplar_wall_hanging_sign", "hanging_sign"),
    ("crimson_wall_hanging_sign", "hanging_sign"),
    ("warped_wall_hanging_sign", "hanging_sign"),
    ("bamboo_wall_hanging_sign", "hanging_sign"),
    ("jukebox", "jukebox"),
    ("enchanting_table", "enchanting_table"),
    ("brewing_stand", "brewing_stand"),
    ("end_portal", "end_portal"),
    ("ender_chest", "ender_chest"),
    ("command_block", "command_block"),
    ("beacon", "beacon"),
    ("skeleton_skull", "skull"),
    ("skeleton_wall_skull", "skull"),
    ("wither_skeleton_skull", "skull"),
    ("wither_skeleton_wall_skull", "skull"),
    ("zombie_head", "skull"),
    ("zombie_wall_head", "skull"),
    ("player_head", "skull"),
    ("player_wall_head", "skull"),
    ("creeper_head", "skull"),
    ("creeper_wall_head", "skull"),
    ("dragon_head", "skull"),
    ("dragon_wall_head", "skull"),
    ("piglin_head", "skull"),
    ("piglin_wall_head", "skull"),
    ("trapped_chest", "trapped_chest"),
    ("comparator", "comparator"),
    ("daylight_detector", "daylight_detector"),
    ("hopper", "hopper"),
    ("dropper", "dropper"),
    ("white_banner", "banner"),
    ("orange_banner", "banner"),
    ("magenta_banner", "banner"),
    ("light_blue_banner", "banner"),
    ("yellow_banner", "banner"),
    ("lime_banner", "banner"),
    ("pink_banner", "banner"),
    ("gray_banner", "banner"),
    ("light_gray_banner", "banner"),
    ("cyan_banner", "banner"),
    ("purple_banner", "banner"),
    ("blue_banner", "banner"),
    ("brown_banner", "banner"),
    ("green_banner", "banner"),
    ("red_banner", "banner"),
    ("black_banner", "banner"),
    ("white_wall_banner", "banner"),
    ("orange_wall_banner", "banner"),
    ("magenta_wall_banner", "banner"),
    ("light_blue_wall_banner", "banner"),
    ("yellow_wall_banner", "banner"),
    ("lime_wall_banner", "banner"),
    ("pink_wall_banner", "banner"),
    ("gray_wall_banner", "banner"),
    ("light_gray_wall_banner", "banner"),
    ("cyan_wall_banner", "banner"),
    ("purple_wall_banner", "banner"),
    ("blue_wall_banner", "banner"),
    ("brown_wall_banner", "banner"),
    ("green_wall_banner", "banner"),
    ("red_wall_banner", "banner"),
    ("black_wall_banner", "banner"),
    ("end_gateway", "end_gateway"),
    ("repeating_command_block", "command_block"),
    ("chain_command_block", "command_block"),
    ("shulker_box", "shulker_box"),
    ("white_shulker_box", "shulker_box"),
    ("orange_shulker_box", "shulker_box"),
    ("magenta_shulker_box", "shulker_box"),
    ("light_blue_shulker_box", "shulker_box"),
    ("yellow_shulker_box", "shulker_box"),
    ("lime_shulker_box", "shulker_box"),
    ("pink_shulker_box", "shulker_box"),
    ("gray_shulker_box", "shulker_box"),
    ("light_gray_shulker_box", "shulker_box"),
    ("cyan_shulker_box", "shulker_box"),
    ("purple_shulker_box", "shulker_box"),
    ("blue_shulker_box", "shulker_box"),
    ("brown_shulker_box", "shulker_box"),
    ("green_shulker_box", "shulker_box"),
    ("red_shulker_box", "shulker_box"),
    ("black_shulker_box", "shulker_box"),
    ("conduit", "conduit"),
    ("barrel", "barrel"),
    ("smoker", "smoker"),
    ("blast_furnace", "blast_furnace"),
    ("lectern", "lectern"),
    ("bell", "bell"),
    ("campfire", "campfire"),
    ("soul_campfire", "campfire"),
    ("crimson_sign", "sign"),
    ("warped_sign", "sign"),
    ("crimson_wall_sign", "sign"),
    ("warped_wall_sign", "sign"),
    ("structure_block", "structure_block"),
    ("jigsaw", "jigsaw"),
    ("test_block", "test_block"),
    ("test_instance_block", "test_instance_block"),
    ("bee_nest", "beehive"),
    ("beehive", "beehive"),
    ("potent_sulfur", "potent_sulfur"),
    ("sculk_sensor", "sculk_sensor"),
    ("calibrated_sculk_sensor", "calibrated_sculk_sensor"),
    ("sculk_catalyst", "sculk_catalyst"),
    ("sculk_shrieker", "sculk_shrieker"),
    ("copper_chest", "chest"),
    ("exposed_copper_chest", "chest"),
    ("weathered_copper_chest", "chest"),
    ("oxidized_copper_chest", "chest"),
    ("waxed_copper_chest", "chest"),
    ("waxed_exposed_copper_chest", "chest"),
    ("waxed_weathered_copper_chest", "chest"),
    ("waxed_oxidized_copper_chest", "chest"),
    ("copper_golem_statue", "copper_golem_statue"),
    ("decorated_pot", "decorated_pot"),
    ("crafter", "crafter"),
    ("trial_spawner", "trial_spawner"),
    ("vault", "vault"),
];

/// The dump against the frozen old-table list: a handful of known pairs first, then every
/// pair the old table produced must still be produced, and the pairs only the dump has (the
/// old table's bugs) are printed for the report. Needs the reports and the dump in the cache.
#[test]
#[ignore = "needs cache/datagen/26.3/reports + cache/oracle/26.3/block_entity_types.json"]
fn dump_covers_the_old_table() {
    let data = crate::net::load_game_data().unwrap();
    if !data.block_entity_types.is_loaded() {
        eprintln!(
            "skipped: no block_entity_types.json (run tools/recorder-mod/dump-server-oracles.sh)"
        );
        return;
    }
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
    // A handful of known pairs, one per family shape (single block, differing name, family,
    // copper chests, a shelf).
    for (block, type_path) in [
        ("chest", "chest"),
        ("trapped_chest", "trapped_chest"),
        ("oak_sign", "sign"),
        ("oak_hanging_sign", "hanging_sign"),
        ("white_banner", "banner"),
        ("player_head", "skull"),
        ("shulker_box", "shulker_box"),
        ("exposed_copper_chest", "chest"),
        ("copper_chest", "chest"),
        ("poplar_shelf", "shelf"),
    ] {
        assert_eq!(
            kind_of(block),
            Some(id(type_path)),
            "{block} -> {type_path}"
        );
    }
    // Everything the old table produced is still produced.
    for (block, type_path) in OLD_TABLE_PAIRS {
        assert_eq!(
            kind_of(block),
            Some(id(type_path)),
            "{block} -> {type_path}"
        );
    }
    // Pairs only the dump has: bugs the old table had. Informational, not a failure.
    let old: HashSet<(&str, &str)> = OLD_TABLE_PAIRS.iter().copied().collect();
    let mut missed: Vec<String> = Vec::new();
    for i in 0..data.registries.len("minecraft:block") as u32 {
        let Some(name) = data.registries.name("minecraft:block", RegistryIndex(i)) else {
            continue;
        };
        let Some(state) = data.blocks.default_state(name) else {
            continue;
        };
        if let Some(ty) = data.block_entity_types.type_of(name, state) {
            if ty.namespace() == "minecraft" && !old.contains(&(name.path(), ty.path())) {
                missed.push(format!("{} -> {}", name.path(), ty.path()));
            }
        }
    }
    if !missed.is_empty() {
        eprintln!(
            "dump has {} pairs the old table missed (old-table bugs):\n  {}",
            missed.len(),
            missed.join("\n  ")
        );
    }
    // Without the dump the kinds map is empty: logged once, no panic (the no-cache case —
    // `load_from` without an oracle dir never looks for the sibling dump).
    let no_dump =
        haste_data::GameData::load_from(&haste_core::paths::datagen_reports(), None).unwrap();
    assert!(!no_dump.block_entity_types.is_loaded());
    assert!(block_entity_state_kinds(&no_dump, &names).is_empty());
}
