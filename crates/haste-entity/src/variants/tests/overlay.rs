use super::*;
/// The sheep's overlays from the packed wool byte: colour in the low nibble, sheared
/// bit 16; the undercoat only for non-white adults (SheepWoolUndercoatLayer).
#[test]
#[ignore = "needs cache/oracle/26.3 (registries.json + model dump; HASTE_CACHE overrides)"]
fn sheep_overlays_from_the_wool_byte() {
    let Some(sheep) = kind_of("minecraft:sheep") else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    // Wool byte absent: no overlays (nothing to draw yet).
    assert!(overlays_for(sheep, &Metadata::default(), false)
        .iter()
        .all(|o| o.is_none()));
    // Red wool (colour 14): the wool layer and the undercoat, both tinted red.
    let red = byte_metadata(&[(metadata_index::SHEEP_WOOL, 14)]);
    let [wool, undercoat] = overlays_for(sheep, &red, false);
    let wool = wool.expect("red wool layer");
    assert_eq!(
        wool.model.texture,
        Identifier::minecraft("entity/sheep/sheep_wool")
    );
    assert_eq!(wool.tint, dye_tint(14));
    let undercoat = undercoat.expect("undercoat for non-white wool");
    assert_eq!(
        undercoat.model.texture,
        Identifier::minecraft("entity/sheep/sheep_wool_undercoat")
    );
    // White adult: the wool but no undercoat (the same white would be invisible).
    let white = byte_metadata(&[(metadata_index::SHEEP_WOOL, 0)]);
    let [wool, undercoat] = overlays_for(sheep, &white, false);
    assert!(wool.is_some());
    assert!(undercoat.is_none());
    // Sheared (bit 16): no wool, but the undercoat keeps a non-white sheep visible.
    let sheared_red = byte_metadata(&[(metadata_index::SHEEP_WOOL, 14 | 16)]);
    let [wool, undercoat] = overlays_for(sheep, &sheared_red, false);
    assert!(wool.is_none());
    assert!(undercoat.is_some());
    // A baby: wool only.
    let [wool, undercoat] = overlays_for(sheep, &red, true);
    assert!(wool.is_some());
    assert!(undercoat.is_none());
}

/// Collars show only for tamed mobs (TamableAnimal flags bit 4), with the collar dye's
/// tint; the cat uses its collar layer, the wolf its own base model (WolfCollarLayer
/// re-draws the parent model).
#[test]
#[ignore = "needs cache/oracle/26.3 (registries.json + model dump; HASTE_CACHE overrides)"]
fn collars_only_when_tamed() {
    let tamed = || byte_metadata(&[(metadata_index::TAME_FLAGS, metadata_index::TAME_FLAG_TAMED)]);
    let Some(cat) = kind_of("minecraft:cat") else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    // Untamed: no collar.
    assert!(overlays_for(cat, &Metadata::default(), false)
        .iter()
        .all(|o| o.is_none()));
    // Tamed with a blue collar (11): the cat collar layer, tinted blue.
    let blue = tamed().apply_extra(int_metadata(&[(metadata_index::CAT_COLLAR, 11)]));
    let [collar, none] = overlays_for(cat, &blue, false);
    let collar = collar.expect("tamed cat collar");
    assert_eq!(
        collar.model.texture,
        Identifier::minecraft("entity/cat/cat_collar")
    );
    assert_eq!(collar.tint, dye_tint(11));
    assert!(none.is_none());
    // The wolf collar re-draws the wolf's own shape with the collar texture.
    let Some(wolf) = kind_of("minecraft:wolf") else {
        eprintln!("skipped: minecraft:wolf not in the registry");
        return;
    };
    let lime = tamed().apply_extra(int_metadata(&[(metadata_index::WOLF_COLLAR, 5)]));
    let [collar, _] = overlays_for(wolf, &lime, false);
    let collar = collar.expect("tamed wolf collar");
    assert_eq!(
        collar.model.texture,
        Identifier::minecraft("entity/wolf/wolf_collar")
    );
    assert_eq!(collar.tint, dye_tint(5));
}

/// The tropical fish: the packed variant picks the shape (small/large), the pattern
/// texture and both dye tints; the shulker's colour byte picks its texture.
#[test]
#[ignore = "needs cache/oracle/26.3 (registries.json + model dump; HASTE_CACHE overrides)"]
fn tropical_fish_and_shulker_from_packed_metadata() {
    // Packed variant: pattern flopper (LARGE base 1, index 0 -> 0x100), base colour
    // magenta (2), pattern colour yellow (4).
    let packed = 0x100 | 2 << 16 | 4 << 24;
    let fish_meta = int_metadata(&[(metadata_index::TROPICAL_FISH_VARIANT, packed)]);
    let model = tropical_fish_model(&fish_meta).expect("large fish model");
    assert_eq!(
        model.texture,
        Identifier::minecraft("entity/fish/tropical_b")
    );
    let Some(fish) = kind_of("minecraft:tropical_fish") else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    let [pattern, none] = overlays_for(fish, &fish_meta, false);
    let pattern = pattern.expect("pattern overlay");
    assert_eq!(
        pattern.model.texture,
        Identifier::minecraft("entity/fish/tropical_b_pattern_1")
    );
    assert_eq!(pattern.tint, dye_tint(4));
    assert!(none.is_none());
    // A small-shape pattern (kob: base 0, index 0) uses the small layer and tropical_a.
    let kob = int_metadata(&[(metadata_index::TROPICAL_FISH_VARIANT, 0)]);
    let model = tropical_fish_model(&kob).expect("small fish model");
    assert_eq!(
        model.texture,
        Identifier::minecraft("entity/fish/tropical_a")
    );
    // The shulker's colour byte: 16 (or absent) is the plain shulker, 0..15 the dyed
    // sprite.
    assert_eq!(shulker_texture(&Metadata::default()), None);
    assert_eq!(
        shulker_texture(&byte_metadata(&[(metadata_index::SHULKER_COLOR, 16)])),
        None
    );
    assert_eq!(
        shulker_texture(&byte_metadata(&[(metadata_index::SHULKER_COLOR, 9)])),
        Some(Identifier::minecraft("entity/shulker/shulker_cyan"))
    );
}

/// Horse markings: the same horse shape re-drawn with the markings texture, no tint;
/// NONE (0) draws nothing.
#[test]
#[ignore = "needs cache/oracle/26.3 (registries.json + model dump; HASTE_CACHE overrides)"]
fn horse_markings_layer() {
    let Some(horse) = kind_of("minecraft:horse") else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    // No variant int: nothing (also what Markings.NONE gives).
    assert!(overlays_for(horse, &Metadata::default(), false)
        .iter()
        .all(|o| o.is_none()));
    // Variant 4 (black coat) with black dots markings (4 << 8).
    let meta = int_metadata(&[(metadata_index::HORSE_TYPE_VARIANT, 4 | 4 << 8)]);
    let [markings, none] = overlays_for(horse, &meta, false);
    let markings = markings.expect("markings layer");
    assert_eq!(
        markings.model.texture,
        Identifier::minecraft("entity/horse/horse_markings_blackdots")
    );
    assert_eq!(markings.tint, [1.0; 3]);
    assert!(none.is_none());
}
