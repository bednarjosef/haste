use super::*;
/// The temperature model layers exist in the dump; pig and chicken babies share one baby
/// layer whatever the variant, cow babies do not.
#[test]
#[ignore = "needs cache/oracle/26.3/entity_models.json (recorder mod dump; HASTE_CACHE overrides)"]
fn temperature_layers_exist_in_the_dump() {
    let lib = model_dump::library().expect("entity model dump");
    for (entity, model) in [
        ("minecraft:pig", VariantModel::Normal),
        ("minecraft:pig", VariantModel::Cold),
        ("minecraft:cow", VariantModel::Normal),
        ("minecraft:cow", VariantModel::Warm),
        ("minecraft:cow", VariantModel::Cold),
        ("minecraft:chicken", VariantModel::Normal),
        ("minecraft:chicken", VariantModel::Cold),
    ] {
        let (adult, baby) = layers_for(entity, model);
        assert!(
            lib.get(adult, 1.0).is_some(),
            "{adult} missing from the dump"
        );
        assert!(lib.get(baby, 1.0).is_some(), "{baby} missing from the dump");
    }
}

/// With the dump present, a cold pig's variant model differs from the default one's and
/// carries the variant texture; without it both are None and the store falls back.
#[test]
#[ignore = "needs cache/oracle/26.3/entity_models.json (recorder mod dump; HASTE_CACHE overrides)"]
fn variant_model_resolves_through_the_dump() {
    let Some(names) = crate::registry::entity_type_names() else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    let kind = names
        .iter()
        .find(|(_, n)| **n == "minecraft:pig")
        .map(|(k, _)| *k)
        .unwrap();
    let mut variants = MobVariants::empty();
    variants.insert(
        "minecraft:pig_variant",
        vec![Some(
            VariantAssets::from_assets(
                "minecraft:entity/pig/pig_cold",
                Some("minecraft:entity/pig/pig_cold_baby"),
                Some("cold"),
            )
            .unwrap(),
        )],
    );
    let m = metadata(metadata_index::PIG, serializer_id::PIG_VARIANT, &[0]);
    let model = model_for(kind, &variants, &m, false).expect("cold pig model");
    assert_eq!(model.texture, Identifier::minecraft("entity/pig/pig_cold"));
    // The default (temperate) model differs.
    let plain = crate::registry::model_for_name("minecraft:pig").unwrap();
    assert_ne!(model.root, plain.root);
    // Baby: the shared baby layer with the variant's baby texture.
    let baby = model_for(kind, &variants, &m, true).expect("cold pig baby model");
    assert_eq!(
        baby.texture,
        Identifier::minecraft("entity/pig/pig_cold_baby")
    );
    // No variant in play: nothing, the store keeps the table's row.
    assert!(model_for(kind, &variants, &Metadata::default(), false).is_none());
}
