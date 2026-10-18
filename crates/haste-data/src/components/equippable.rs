//! The `minecraft:equippable` item component: the slot an item equips into and the equipment
//! asset it draws.

use haste_core::model::EquipmentSlot;
use haste_core::Identifier;

/// The `equippable` component of one item: the slot it goes in and the equipment asset it draws.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemEquippable {
    pub slot: EquipmentSlot,
    /// `equippable.asset_id` (`minecraft:iron` -> `equipment/iron.json`); `None` for items whose
    /// equippable has no asset (saddles on some mobs, the `equippable` marker without a texture).
    pub asset: Option<Identifier>,
}

/// The `minecraft:equippable` component of one `{"components": {...}}` report file, or `None`
/// when the item has none (or the file is malformed).
pub fn parse_equippable(bytes: &[u8]) -> Option<ItemEquippable> {
    let json: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let eq = json.get("components")?.get("minecraft:equippable")?;
    let slot = eq.get("slot")?.as_str()?;
    let slot = match slot {
        "mainhand" => EquipmentSlot::MainHand,
        "offhand" => EquipmentSlot::OffHand,
        "feet" => EquipmentSlot::Feet,
        "legs" => EquipmentSlot::Legs,
        "chest" => EquipmentSlot::Chest,
        "head" => EquipmentSlot::Head,
        "body" => EquipmentSlot::Body,
        "saddle" => EquipmentSlot::Saddle,
        _ => return None,
    };
    let asset = eq
        .get("asset_id")
        .and_then(|a| a.as_str())
        .and_then(|a| Identifier::parse(a).ok());
    Some(ItemEquippable { slot, asset })
}
