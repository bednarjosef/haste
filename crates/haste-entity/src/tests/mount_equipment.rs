//! `mount_equipment`: the layer table and `Crackiness.WOLF_ARMOR`'s thresholds, with vanilla's
//! numbers (`Crackiness.WOLF_ARMOR = (0.95, 0.69, 0.32)`).
use haste_core::model::EquipmentSlot;

use crate::mount_equipment::{cracked_fraction, crackiness_texture, layer_names};

#[test]
fn crackiness_thresholds_match_vanilla() {
    // Above 0.95 no crack; below 0.95 low, below 0.69 medium, below 0.32 high.
    assert_eq!(crackiness_texture(1.0), None);
    assert_eq!(crackiness_texture(0.95), None);
    assert_eq!(
        crackiness_texture(0.94),
        Some("entity/wolf/wolf_armor_crackiness_low")
    );
    assert_eq!(
        crackiness_texture(0.69),
        Some("entity/wolf/wolf_armor_crackiness_low")
    );
    assert_eq!(
        crackiness_texture(0.68),
        Some("entity/wolf/wolf_armor_crackiness_medium")
    );
    assert_eq!(
        crackiness_texture(0.5),
        Some("entity/wolf/wolf_armor_crackiness_medium")
    );
    assert_eq!(
        crackiness_texture(0.32),
        Some("entity/wolf/wolf_armor_crackiness_medium")
    );
    assert_eq!(
        crackiness_texture(0.31),
        Some("entity/wolf/wolf_armor_crackiness_high")
    );
    assert_eq!(
        crackiness_texture(0.0),
        Some("entity/wolf/wolf_armor_crackiness_high")
    );
}

#[test]
fn remaining_fraction_is_max_minus_damage_over_max() {
    // Undamaged armour is at 1.0 (no crack); half-worn armour sits at 0.5 (medium).
    assert_eq!(cracked_fraction(64, 0), Some(1.0));
    assert_eq!(cracked_fraction(64, 32), Some(0.5));
    assert_eq!(cracked_fraction(64, 64), Some(0.0));
    // An item without a max takes no damage.
    assert_eq!(cracked_fraction(0, 0), None);
}

#[test]
fn horse_and_wolf_body_layers() {
    assert_eq!(
        layer_names("entity/horse/horse_white", EquipmentSlot::Body),
        Some(("minecraft:horse_armor#main", "horse_body"))
    );
    assert_eq!(
        layer_names("entity/wolf/wolf", EquipmentSlot::Body),
        Some(("minecraft:wolf_armor#main", "wolf_body"))
    );
    assert_eq!(
        layer_names("entity/wolf/wolf_tame", EquipmentSlot::Body),
        Some(("minecraft:wolf_armor#main", "wolf_body"))
    );
    // Babies take no armour or saddle; the collar overlay is not a wolf body.
    assert_eq!(
        layer_names("entity/horse/horse_white_baby", EquipmentSlot::Body),
        None
    );
    assert_eq!(
        layer_names("entity/wolf/wolf_baby", EquipmentSlot::Body),
        None
    );
    assert_eq!(
        layer_names("entity/wolf/wolf_collar", EquipmentSlot::Body),
        None
    );
    // Zombie and skeleton horses take no body armour here.
    assert_eq!(
        layer_names("entity/horse/zombie_horse", EquipmentSlot::Body),
        None
    );
}

#[test]
fn saddle_layers_per_mount() {
    let saddle = |texture| layer_names(texture, EquipmentSlot::Saddle);
    assert_eq!(
        saddle("entity/horse/horse_brown"),
        Some(("minecraft:horse#saddle", "horse_saddle"))
    );
    assert_eq!(
        saddle("entity/horse/donkey"),
        Some(("minecraft:donkey#saddle", "donkey_saddle"))
    );
    assert_eq!(
        saddle("entity/horse/mule"),
        Some(("minecraft:mule#saddle", "mule_saddle"))
    );
    assert_eq!(
        saddle("entity/pig/pig_cold"),
        Some(("minecraft:pig#saddle", "pig_saddle"))
    );
    assert_eq!(
        saddle("entity/strider/strider_cold"),
        Some(("minecraft:strider#saddle", "strider_saddle"))
    );
    assert_eq!(
        saddle("entity/camel/camel"),
        Some(("minecraft:camel#saddle", "camel_saddle"))
    );
    assert_eq!(
        saddle("entity/camel/camel_husk"),
        Some(("minecraft:camel_husk#saddle", "camel_husk_saddle"))
    );
    assert_eq!(
        saddle("entity/nautilus/nautilus"),
        Some(("minecraft:nautilus#saddle", "nautilus_saddle"))
    );
    // A sheep takes no saddle.
    assert_eq!(saddle("entity/sheep/sheep"), None);
}
