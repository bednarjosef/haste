use super::*;
// ---------------------------------------------------------------------------
// Table coverage and baby variants.
// ---------------------------------------------------------------------------

/// Every living entity type has a table row (so it gets its vanilla model, or an explicit
/// pending entry). Runs without the cache: it checks the table, not the dump.
#[test]
fn table_covers_every_living_entity_type() {
    for name in LIVING_TYPES {
        assert!(
            crate::registry::table_entry(name).is_some(),
            "{name} has no model table row"
        );
    }
    // And nothing in the table references a type outside the known set (typos).
    let covered = crate::registry::table_textures().count();
    assert!(
        covered >= LIVING_TYPES.len(),
        "table smaller than living set"
    );
}

/// The baby rows point at real `*_baby` layers and the baby metadata flag flips them
/// (`AgeableMob.DATA_BABY_ID` = index 16; `Zombie`/`Zoglin`/`Piglin` put theirs on the same
/// slot through the same super-chain, Entity 0-7 -> LivingEntity 8-14 -> Mob 15).
#[test]
fn baby_flag_and_baby_layers() {
    // The flag: metadata index 16, boolean true.
    let baby_meta = |v: bool| {
        let mut m = crate::metadata::Metadata::default();
        m.apply(&[MetadataEntry {
            index: crate::registry::BABY_METADATA_INDEX,
            value: MetadataValue::Boolean(v),
        }]);
        m
    };
    assert!(crate::registry::is_baby(&baby_meta(true)));
    assert!(!crate::registry::is_baby(&baby_meta(false)));
    assert!(!crate::registry::is_baby(
        &crate::metadata::Metadata::default()
    ));
}

/// The shadow radii are the values the 26.3 renderer constructors pass (a sample across the
/// families), and a table-covered living type not named falls back to the usual 0.5.
#[test]
fn shadow_radius_table_matches_vanilla() {
    use crate::registry::{shadow_radius_for_name, shadow_strength_for_name};
    assert_eq!(shadow_radius_for_name("minecraft:pig"), 0.7);
    assert_eq!(shadow_radius_for_name("minecraft:cow"), 0.7);
    assert_eq!(shadow_radius_for_name("minecraft:zombie"), 0.5);
    assert_eq!(shadow_radius_for_name("minecraft:creeper"), 0.5);
    assert_eq!(shadow_radius_for_name("minecraft:item"), 0.15);
    assert_eq!(shadow_radius_for_name("minecraft:experience_orb"), 0.15);
    assert_eq!(shadow_radius_for_name("minecraft:armor_stand"), 0.0);
    assert_eq!(shadow_radius_for_name("minecraft:ghast"), 1.5);
    assert_eq!(shadow_radius_for_name("minecraft:bat"), 0.25);
    // An unknown / non-rendered type casts no shadow.
    assert_eq!(shadow_radius_for_name("minecraft:not_a_mob"), 0.0);
    assert_eq!(shadow_strength_for_name("minecraft:item"), 0.75);
    assert_eq!(shadow_strength_for_name("minecraft:pig"), 1.0);
    // The name tag anchor: the bb height, or the sniffer's 2.05 override.
    assert_eq!(
        crate::registry::name_tag_offset_for_name("minecraft:pig", 0.9),
        0.9
    );
    assert_eq!(
        crate::registry::name_tag_offset_for_name("minecraft:sniffer", 1.75),
        2.05
    );
}

/// Baby layers come from the recorder's model dump, which CI does not have.
#[test]
#[ignore = "needs cache/oracle/26.3/entity_models.json"]
fn baby_layers_from_the_dump() {
    // Rows whose renderer has a baby model (AgeableMobRenderer and friends) name it.
    // Vanilla types with a baby layer but none in our table yet would fail here.
    for (name, has_baby) in [
        ("minecraft:pig", true),
        ("minecraft:villager", true),
        ("minecraft:zombie", true),
        ("minecraft:horse", true),
        ("minecraft:sniffer", true),
        ("minecraft:creeper", false), // MobRenderer only
        ("minecraft:spider", false),
        ("minecraft:bat", false),
    ] {
        let baby = crate::registry::baby_model_for_name(name);
        assert_eq!(
            baby.is_some(),
            has_baby,
            "{name} baby layer presence mismatch"
        );
    }
    // With the dump present, the baby model differs from the adult and carries the baby
    // texture; without it both are None and the store falls back to the box.
    if let (Some(adult), Some(baby)) = (
        crate::registry::model_for_name("minecraft:pig"),
        crate::registry::baby_model_for_name("minecraft:pig"),
    ) {
        let adult_head = find_part(&adult.root, "head").unwrap();
        let baby_head = find_part(&baby.root, "head").unwrap();
        assert_ne!(adult_head.pivot, baby_head.pivot);
        assert_eq!(baby.texture.path(), "entity/pig/pig_temperate_baby");
    }
    // The sniffer's baby texture is "snifflet", not "sniffer_baby" (SnifferRenderer).
    if let Some(baby) = crate::registry::baby_model_for_name("minecraft:sniffer") {
        assert_eq!(baby.texture.path(), "entity/sniffer/snifflet");
    }
}

/// Every table texture exists in the client jar as `assets/minecraft/textures/<path>.png`.
/// Needs the installation (client jar + asset index) from `haste setup`.
#[test]
#[ignore = "needs the client jar and assets index (HASTE_CACHE; from `haste setup`)"]
fn table_textures_exist_in_jar() {
    let jar = haste_core::paths::client_jar();
    let file = std::fs::File::open(&jar).unwrap_or_else(|e| {
        panic!(
            "opening {}: {e} (run `haste setup` or set HASTE_CACHE)",
            jar.display()
        )
    });
    let zip = zip::ZipArchive::new(file).expect("reading client jar zip");
    let names: std::collections::HashSet<String> =
        zip.file_names().map(|s| s.to_string()).collect();
    let mut missing = Vec::new();
    for texture in crate::registry::table_textures() {
        let key = format!("assets/minecraft/textures/{texture}.png");
        if !names.contains(&key) {
            missing.push(key);
        }
    }
    assert!(
        missing.is_empty(),
        "{} table textures missing from the jar: {missing:?}",
        missing.len()
    );
}

/// `render_instances` resolves a variant mob's look through the synced variant table: the
/// metadata's variant id picks the assets, the model comes from the variant's model type,
/// and without a variant in play the model table's default row stands. Needs the dump and
/// registries.json (the kind is resolved through the real entity-type names).
#[test]
#[ignore = "needs cache/oracle/26.3 (recorder mod dump + registries.json; HASTE_CACHE overrides)"]
fn render_instances_picks_up_variant_models() {
    let Some(names) = crate::registry::entity_type_names() else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    let Some((kind, _)) = names.iter().find(|(_, n)| n.as_str() == "minecraft:pig") else {
        eprintln!("skipped: minecraft:pig not in the registry");
        return;
    };
    let mut s = store();
    let mut sp = spawn(1, DVec3::ZERO);
    sp.kind = *kind;
    s.spawn(&sp);
    // No variant table set: the default (temperate) pig model.
    let mut out = Vec::new();
    s.render_instances(0.5, &mut out);
    let default = out[0].model.clone();
    if crate::registry::model_for_name("minecraft:pig").is_none() {
        eprintln!("skipped: no entity model dump in the cache");
        return;
    }
    assert_eq!(
        default.texture.path(),
        "entity/pig/pig_temperate",
        "the default pig row"
    );
    // A cold pig variant (registry id 2 in vanilla's order: temperate, warm, cold).
    s.set_mob_variants(
        "minecraft:pig_variant",
        vec![
            Some(
                crate::variants::VariantAssets::from_assets(
                    "minecraft:entity/pig/pig_temperate",
                    Some("minecraft:entity/pig/pig_temperate_baby"),
                    None,
                )
                .unwrap(),
            ),
            None,
            Some(
                crate::variants::VariantAssets::from_assets(
                    "minecraft:entity/pig/pig_cold",
                    Some("minecraft:entity/pig/pig_cold_baby"),
                    Some("cold"),
                )
                .unwrap(),
            ),
        ],
    );
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: crate::variants::metadata_index::PIG,
            value: MetadataValue::Other {
                type_id: crate::variants::serializer_id::PIG_VARIANT,
                raw: vec![2].into(),
            },
        }],
    );
    let mut out = Vec::new();
    s.render_instances(0.5, &mut out);
    let cold = out[0].model.clone();
    assert_eq!(cold.texture.path(), "entity/pig/pig_cold");
    assert_ne!(cold.root, default.root, "the cold layer differs");
    // An unknown variant id (a mod's registry): the default row stands.
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: crate::variants::metadata_index::PIG,
            value: MetadataValue::Other {
                type_id: crate::variants::serializer_id::PIG_VARIANT,
                raw: vec![7].into(),
            },
        }],
    );
    let mut out = Vec::new();
    s.render_instances(0.5, &mut out);
    assert_eq!(out[0].model.texture.path(), "entity/pig/pig_temperate");
}

/// A sheep keeps its sheep model with wool metadata set: the wool byte sits at metadata 18,
/// the same slot a shulker's colour byte occupies, so the shulker texture swap must be
/// gated on the entity type (regression: a red-wool sheep used to come out as a red
/// shulker). Needs registries.json + the dump.
#[test]
#[ignore = "needs cache/oracle/26.3 (registries.json + model dump; HASTE_CACHE overrides)"]
fn sheep_with_wool_metadata_is_not_a_shulker() {
    let Some(names) = crate::registry::entity_type_names() else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    let Some((kind, _)) = names.iter().find(|(_, n)| n.as_str() == "minecraft:sheep") else {
        eprintln!("skipped: minecraft:sheep not in the registry");
        return;
    };
    let mut s = store();
    let mut sp = spawn(1, DVec3::ZERO);
    sp.kind = *kind;
    s.spawn(&sp);
    // Red wool, exactly the byte that is a red shulker's colour.
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: crate::variants::metadata_index::SHEEP_WOOL,
            value: MetadataValue::Byte(14),
        }],
    );
    let mut out = Vec::new();
    s.render_instances(0.5, &mut out);
    assert_eq!(out[0].model.texture.path(), "entity/sheep/sheep");
    // The wool overlay is there, tinted red.
    let wool = out[0].overlays[0].as_ref().expect("wool overlay");
    assert_eq!(wool.model.texture.path(), "entity/sheep/sheep_wool");
    assert_eq!(wool.tint, crate::variants::dye_tint(14));
}
