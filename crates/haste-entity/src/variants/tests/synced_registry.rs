use super::*;
/// The variant field's index is the synched-data chain's next slot after the super
/// classes' fields (Entity 0-7, LivingEntity 8-14, Mob 15, AgeableMob 16-17).
#[test]
fn metadata_indices_follow_the_super_chain() {
    assert_eq!(metadata_index::PIG, 19); // boost time at 18
    assert_eq!(metadata_index::COW, 18);
    assert_eq!(metadata_index::CHICKEN, 18);
    assert_eq!(metadata_index::FROG, 18);
    assert_eq!(metadata_index::CAT, 20); // TamableAnimal 18-19
    assert_eq!(metadata_index::WOLF, 23); // TamableAnimal 18-19, wolf 20-22
    assert_eq!(metadata_index::AXOLOTL, 18);
    assert_eq!(metadata_index::RABBIT, 18);
    assert_eq!(metadata_index::HORSE_TYPE_VARIANT, 19); // AbstractHorse flags 18
    assert_eq!(metadata_index::LLAMA_VARIANT, 21); // flags 18, chest 19, strength 20
    assert_eq!(metadata_index::PARROT, 20); // TamableAnimal 18-19
    assert_eq!(metadata_index::FOX_TYPE, 18); // then flags 19, trusted 20-21
    assert_eq!(metadata_index::FOX_FLAGS, 19);
    assert_eq!(metadata_index::PANDA_MAIN_GENE, 21); // unhappy 18, sneeze 19, eat 20
    assert_eq!(metadata_index::PANDA_HIDDEN_GENE, 22);
}

/// A VarInt registry id is read from the field; a wrong serializer or a leftover byte
/// means no variant (fall back to the default).
#[test]
fn variant_id_reads_the_varint() {
    let spec = spec_for("minecraft:pig").unwrap();
    let m = metadata(metadata_index::PIG, serializer_id::PIG_VARIANT, &[2]);
    assert_eq!(variant_id(&m, spec), Some(2));
    // A multi-byte VarInt (id 300 = 0xAC 0x02).
    let m = metadata(
        metadata_index::PIG,
        serializer_id::PIG_VARIANT,
        &[0xAC, 0x02],
    );
    assert_eq!(variant_id(&m, spec), Some(300));
    // Wrong serializer (the pig's sound variant would sit on another index).
    let m = metadata(metadata_index::PIG, serializer_id::PIG_VARIANT + 1, &[0]);
    assert_eq!(variant_id(&m, spec), None);
    // Trailing bytes: not a well-formed holder id.
    let m = metadata(metadata_index::PIG, serializer_id::PIG_VARIANT, &[2, 0]);
    assert_eq!(variant_id(&m, spec), None);
}

/// Asset fields parse into the table convention; a model name we have no layer for
/// (mods) resolves to nothing, so the default look applies.
#[test]
fn asset_parsing_and_model_fallback() {
    let cold = VariantAssets::from_assets(
        "minecraft:entity/pig/pig_cold",
        Some("minecraft:entity/pig/pig_cold_baby"),
        Some("cold"),
    )
    .unwrap();
    assert_eq!(
        cold,
        VariantAssets {
            adult: Identifier::minecraft("entity/pig/pig_cold"),
            baby: Identifier::minecraft("entity/pig/pig_cold_baby"),
            model: VariantModel::Cold,
        }
    );
    // No model field: the codec's default "normal". A frog has no baby texture: the baby
    // falls back to the adult's.
    let plain =
        VariantAssets::from_assets("minecraft:entity/frog/frog_temperate", None, None).unwrap();
    assert_eq!(plain.model, VariantModel::Normal);
    assert_eq!(
        plain.baby,
        Identifier::minecraft("entity/frog/frog_temperate")
    );
    // A mod's model name we do not know: no assets (default look).
    assert!(VariantAssets::from_assets(
        "mod:entity/pig",
        Some("mod:entity/pig_baby"),
        Some("modded")
    )
    .is_none());
}

/// The synced tables are keyed by registry id; unknown ids and absent entries fall back.
#[test]
fn table_lookup_falls_back_on_unknown_ids() {
    let mut variants = MobVariants::empty();
    variants.insert(
        "minecraft:pig_variant",
        vec![
            Some(
                VariantAssets::from_assets(
                    "minecraft:entity/pig/pig_temperate",
                    Some("minecraft:entity/pig/pig_temperate_baby"),
                    None,
                )
                .unwrap(),
            ),
            None, // known pack: no NBT
            Some(
                VariantAssets::from_assets(
                    "minecraft:entity/pig/pig_cold",
                    Some("minecraft:entity/pig/pig_cold_baby"),
                    Some("cold"),
                )
                .unwrap(),
            ),
        ],
    );
    // A registry no mob reads is ignored, not stored.
    variants.insert("minecraft:made_up", vec![None]);
    // kind is resolved through the real registries.json; without the cache the name map
    // is empty, so the lookups are checked only when it is present.
    let Some(names) = crate::registry::entity_type_names() else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    let kind = names
        .iter()
        .find(|(_, n)| **n == "minecraft:pig")
        .map(|(k, _)| *k)
        .unwrap();
    let m = metadata(metadata_index::PIG, serializer_id::PIG_VARIANT, &[2]);
    let a = variants.assets(kind, &m).unwrap();
    assert_eq!(a.adult, Identifier::minecraft("entity/pig/pig_cold"));
    assert_eq!(a.model, VariantModel::Cold);
    // Id 1 has no entry: default look.
    let m = metadata(metadata_index::PIG, serializer_id::PIG_VARIANT, &[1]);
    assert!(variants.assets(kind, &m).is_none());
    // Id out of range: default look.
    let m = metadata(metadata_index::PIG, serializer_id::PIG_VARIANT, &[9]);
    assert!(variants.assets(kind, &m).is_none());
    // Non-variant type: nothing.
    let zombie = names
        .iter()
        .find(|(_, n)| **n == "minecraft:zombie")
        .map(|(k, _)| *k)
        .unwrap();
    assert!(variants.assets(zombie, &m).is_none());
}
