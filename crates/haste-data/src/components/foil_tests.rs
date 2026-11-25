//! `ItemStack.hasFoil` from the item's default components and the stack's patch.

use super::*;
use haste_core::model::FoilType;
use haste_core::RegistryIndex;

const SWORD: RegistryIndex = RegistryIndex(1);
const ENCHANTED_BOOK: RegistryIndex = RegistryIndex(2);
const BOOK: RegistryIndex = RegistryIndex(3);
const COMPASS: RegistryIndex = RegistryIndex(4);
const NETHER_STAR: RegistryIndex = RegistryIndex(5);

/// A table shaped like the 26.3 report: the enchanted book and the nether star default to the
/// glint override, everything defaults to empty enchantments.
fn table() -> ItemComponents {
    let mut c = ItemComponents::default();
    c.insert_foil_defaults(SWORD, None, false);
    c.insert_foil_defaults(ENCHANTED_BOOK, Some(true), false);
    c.insert_foil_defaults(BOOK, None, false);
    c.insert_foil_defaults(COMPASS, None, false);
    c.insert_foil_defaults(NETHER_STAR, Some(true), false);
    c.set_compass(COMPASS);
    c
}

fn enchanted() -> FoilPatch {
    FoilPatch {
        enchantments: Patched::Set(true),
        ..FoilPatch::default()
    }
}

#[test]
fn a_plain_stack_has_no_foil() {
    let c = table();
    assert!(!c.has_foil(SWORD, &FoilPatch::default()));
    assert_eq!(
        c.foil_type(SWORD, &FoilPatch::default(), false),
        FoilType::None
    );
}

#[test]
fn an_enchanted_stack_has_foil() {
    let c = table();
    assert!(c.has_foil(SWORD, &enchanted()));
    assert_eq!(c.foil_type(SWORD, &enchanted(), false), FoilType::Standard);
    // An empty enchantment list is not enchanted (`ItemEnchantments.isEmpty`).
    let empty = FoilPatch {
        enchantments: Patched::Set(false),
        ..FoilPatch::default()
    };
    assert!(!c.has_foil(SWORD, &empty));
}

#[test]
fn the_glint_override_wins_both_ways() {
    let c = table();
    let off = FoilPatch {
        glint_override: Patched::Set(false),
        enchantments: Patched::Set(true),
        ..FoilPatch::default()
    };
    assert!(!c.has_foil(SWORD, &off), "override false hides the glint");
    let on = FoilPatch {
        glint_override: Patched::Set(true),
        ..FoilPatch::default()
    };
    assert!(c.has_foil(SWORD, &on), "override true glints unenchanted");
    // The nether star's default override glints; a patch can turn it off.
    assert!(c.has_foil(NETHER_STAR, &FoilPatch::default()));
    let star_off = FoilPatch {
        glint_override: Patched::Set(false),
        ..FoilPatch::default()
    };
    assert!(!c.has_foil(NETHER_STAR, &star_off));
}

#[test]
fn removing_the_default_override_falls_back_to_the_enchantments() {
    let c = table();
    let removed = FoilPatch {
        glint_override: Patched::Removed,
        ..FoilPatch::default()
    };
    assert!(!c.has_foil(NETHER_STAR, &removed));
    let removed_enchanted = FoilPatch {
        glint_override: Patched::Removed,
        enchantments: Patched::Set(true),
        ..FoilPatch::default()
    };
    assert!(c.has_foil(NETHER_STAR, &removed_enchanted));
}

#[test]
fn stored_enchantments_glint_only_through_the_book_s_override() {
    let c = table();
    // An enchanted book's `stored_enchantments` are not `isEnchanted`; the book glints
    // through its default override, with or without stored enchantments.
    assert!(c.has_foil(ENCHANTED_BOOK, &FoilPatch::default()));
    // A plain book given `stored_enchantments` does not glint: the patch side never reports
    // them, they are not read by `hasFoil`.
    assert!(!c.has_foil(BOOK, &FoilPatch::default()));
}

#[test]
fn a_lodestone_compass_glints_and_other_items_ignore_the_tracker() {
    let c = table();
    let tracked = FoilPatch {
        lodestone_tracker: Patched::Set(()),
        ..FoilPatch::default()
    };
    assert!(c.has_foil(COMPASS, &tracked));
    assert!(!c.has_foil(COMPASS, &FoilPatch::default()));
    assert!(!c.has_foil(SWORD, &tracked), "only `CompassItem` reads it");
    assert_eq!(c.foil_type(COMPASS, &tracked, true), FoilType::Special);
}

#[test]
fn unknown_items_default_to_no_foil() {
    let c = table();
    assert!(!c.has_foil(RegistryIndex(999), &FoilPatch::default()));
    assert!(c.has_foil(RegistryIndex(999), &enchanted()));
}

#[test]
fn defaults_parse_from_the_report_shape() {
    let json: serde_json::Value = serde_json::from_str(
        r#"{"minecraft:enchantment_glint_override": true, "minecraft:enchantments": {},
            "minecraft:stored_enchantments": {}}"#,
    )
    .unwrap();
    assert_eq!(
        FoilDefaults::parse(&json),
        FoilDefaults {
            glint_override: Some(true),
            enchanted: false,
            lodestone_tracker: false,
        }
    );
    let enchanted: serde_json::Value =
        serde_json::from_str(r#"{"minecraft:enchantments": {"minecraft:sharpness": 5}}"#).unwrap();
    assert!(FoilDefaults::parse(&enchanted).enchanted);
}

/// The real 26.3 report: the items vanilla's `Items` gives the glint override.
#[test]
#[ignore = "needs the data generator's reports in the cache (haste setup)"]
fn the_report_s_glint_overrides() {
    let reports = haste_core::paths::datagen_reports();
    let registries = crate::registries::Registries::parse(
        &std::fs::read_to_string(reports.join("registries.json")).unwrap(),
    )
    .unwrap();
    let c = ItemComponents::load(
        &reports.join("minecraft").join("components").join("item"),
        &registries,
    );
    let item = |n: &str| {
        registries
            .index("minecraft:item", &haste_core::Identifier::minecraft(n))
            .unwrap()
    };
    for glinting in [
        "enchanted_book",
        "nether_star",
        "enchanted_golden_apple",
        "experience_bottle",
        "written_book",
        "end_crystal",
        "debug_stick",
    ] {
        assert!(
            c.has_foil(item(glinting), &FoilPatch::default()),
            "{glinting}"
        );
    }
    for plain in ["diamond_sword", "book", "compass", "golden_apple"] {
        assert!(!c.has_foil(item(plain), &FoilPatch::default()), "{plain}");
    }
    assert!(c.has_foil(item("diamond_sword"), &enchanted()));
    let tracked = FoilPatch {
        lodestone_tracker: Patched::Set(()),
        ..FoilPatch::default()
    };
    assert!(c.has_foil(item("compass"), &tracked));
}
