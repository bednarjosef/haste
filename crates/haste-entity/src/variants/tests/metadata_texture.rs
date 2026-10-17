use super::*;
/// The metadata-driven variant textures: each renderer's own table, including the
/// fallbacks vanilla applies for unknown ids (and the killer rabbit's 99). Needs
/// registries.json for the kind lookup.
#[test]
#[ignore = "needs cache/datagen (registries.json; HASTE_CACHE overrides)"]
fn metadata_variant_textures_match_the_renderers() {
    let cases: Vec<(&str, Metadata, bool, Option<&str>, &str)> = vec![
        // Axolotl: variant 3 (cyan), and an unknown id falls back to LUCY.
        (
            "minecraft:axolotl",
            int_metadata(&[(metadata_index::AXOLOTL, 3)]),
            false,
            None,
            "entity/axolotl/axolotl_cyan",
        ),
        (
            "minecraft:axolotl",
            int_metadata(&[(metadata_index::AXOLOTL, 9)]),
            true,
            None,
            "entity/axolotl/axolotl_lucy_baby",
        ),
        // Fox: snow variant sleeping (flags bit 32), baby.
        (
            "minecraft:fox",
            int_metadata(&[(metadata_index::FOX_TYPE, 1)])
                .apply_extra(byte_metadata(&[(metadata_index::FOX_FLAGS, 32)])),
            true,
            None,
            "entity/fox/fox_snow_sleep_baby",
        ),
        // Rabbit: the killer rabbit (99), Toast's magic name, and the default.
        (
            "minecraft:rabbit",
            int_metadata(&[(metadata_index::RABBIT, 99)]),
            false,
            None,
            "entity/rabbit/rabbit_caerbannog",
        ),
        (
            "minecraft:rabbit",
            int_metadata(&[(metadata_index::RABBIT, 0)]),
            false,
            Some("Toast"),
            "entity/rabbit/rabbit_toast",
        ),
        (
            "minecraft:rabbit",
            int_metadata(&[(metadata_index::RABBIT, 42)]),
            true,
            None,
            "entity/rabbit/rabbit_brown_baby",
        ),
        // Horse: variant 4 (black).
        (
            "minecraft:horse",
            int_metadata(&[(metadata_index::HORSE_TYPE_VARIANT, 4)]),
            false,
            None,
            "entity/horse/horse_black",
        ),
        // Llama: variant 3 (gray), baby.
        (
            "minecraft:llama",
            int_metadata(&[(metadata_index::LLAMA_VARIANT, 3)]),
            true,
            None,
            "entity/llama/llama_gray_baby",
        ),
        // Parrot: variant 2 (green).
        (
            "minecraft:parrot",
            int_metadata(&[(metadata_index::PARROT, 2)]),
            false,
            None,
            "entity/parrot/parrot_green",
        ),
        // Panda: main brown (4) with hidden brown shows brown; with a different hidden
        // gene it shows normal.
        (
            "minecraft:panda",
            byte_metadata(&[
                (metadata_index::PANDA_MAIN_GENE, 4),
                (metadata_index::PANDA_HIDDEN_GENE, 4),
            ]),
            false,
            None,
            "entity/panda/brown_panda",
        ),
        (
            "minecraft:panda",
            byte_metadata(&[
                (metadata_index::PANDA_MAIN_GENE, 4),
                (metadata_index::PANDA_HIDDEN_GENE, 0),
            ]),
            true,
            None,
            "entity/panda/panda_baby",
        ),
        // Mooshroom: variant 1 (brown).
        (
            "minecraft:mooshroom",
            int_metadata(&[(metadata_index::MOOSHROOM_TYPE, 1)]),
            true,
            None,
            "entity/cow/mooshroom_brown_baby",
        ),
        // Strider: suffocating (cold) boolean at 19.
        (
            "minecraft:strider",
            Metadata::default().apply_extra(bool_metadata(&[(
                metadata_index::STRIDER_SUFFOCATING,
                true,
            )])),
            false,
            None,
            "entity/strider/strider_cold",
        ),
    ];
    for (name, meta, baby, custom, want) in cases {
        let Some(kind) = kind_of(name) else {
            eprintln!("skipped: {name} not in registries.json");
            return;
        };
        let got = texture_for(kind, &meta, baby, custom, 0).unwrap();
        assert_eq!(
            got,
            Identifier::minecraft(want),
            "{name} baby={baby} custom={custom:?}"
        );
    }
    // A type with no metadata variant: nothing.
    if let Some(pig) = kind_of("minecraft:pig") {
        assert!(texture_for(
            pig,
            &int_metadata(&[(metadata_index::RABBIT, 1)]),
            false,
            None,
            0
        )
        .is_none());
    }
}

/// The bee's texture: nectar (flags bit 8) x anger (end time past the world age) x baby
/// (`BeeRenderer.getTextureLocation`); the anger end time in the past means calm.
#[test]
#[ignore = "needs cache/datagen (registries.json; HASTE_CACHE overrides)"]
fn bee_textures_angry_nectar_and_age() {
    let Some(bee) = kind_of("minecraft:bee") else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    let bee_meta = |flags: i8, anger_end: i64| {
        bool_metadata(&[(0, false)])
            .apply_extra(long_metadata(&[(metadata_index::BEE_ANGER_END, anger_end)]))
            .apply_extra(byte_metadata(&[(metadata_index::BEE_FLAGS, flags)]))
    };
    // Calm, no nectar.
    assert_eq!(
        texture_for(bee, &bee_meta(0, -1), false, None, 100).unwrap(),
        Identifier::minecraft("entity/bee/bee")
    );
    // Nectar only.
    assert_eq!(
        texture_for(bee, &bee_meta(8, -1), false, None, 100).unwrap(),
        Identifier::minecraft("entity/bee/bee_nectar")
    );
    // Angry (end time 200 > age 100), nectar, baby.
    assert_eq!(
        texture_for(bee, &bee_meta(8, 200), true, None, 100).unwrap(),
        Identifier::minecraft("entity/bee/bee_angry_nectar_baby")
    );
    // The anger already ended: calm.
    assert_eq!(
        texture_for(bee, &bee_meta(0, 50), false, None, 100).unwrap(),
        Identifier::minecraft("entity/bee/bee")
    );
    // Angry without nectar.
    assert_eq!(
        texture_for(bee, &bee_meta(0, 200), false, None, 100).unwrap(),
        Identifier::minecraft("entity/bee/bee_angry")
    );
}

/// A strider that is not suffocating keeps the plain texture (the boolean defaults
/// false when absent).
#[test]
#[ignore = "needs cache/datagen (registries.json; HASTE_CACHE overrides)"]
fn strider_plain_when_not_suffocating() {
    let Some(strider) = kind_of("minecraft:strider") else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    assert_eq!(
        texture_for(strider, &Metadata::default(), false, None, 0).unwrap(),
        Identifier::minecraft("entity/strider/strider")
    );
}
