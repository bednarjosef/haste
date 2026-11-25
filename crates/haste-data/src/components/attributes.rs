//! The `minecraft:attribute_modifiers` item component (`ItemAttributeModifiers`).

use haste_core::Identifier;

/// One entry of an item's `minecraft:attribute_modifiers` component
/// (`ItemAttributeModifiers.Entry`).
#[derive(Clone, Debug, PartialEq)]
pub struct AttributeModifier {
    /// The attribute, e.g. `minecraft:attack_speed`.
    pub attribute: Identifier,
    /// The modifier amount.
    pub amount: f64,
    /// `AttributeModifier.Operation`: 0 add_value, 1 add_multiplied_base, 2 add_multiplied_total.
    pub operation: i32,
    /// `EquipmentSlotGroup` name: `any`, `mainhand`, `hand`, `offhand`, ...
    pub slot: String,
}

impl AttributeModifier {
    /// Whether this modifier applies to an item held in the main hand
    /// (`EquipmentSlotGroup.MAINHAND`): `any`, `mainhand` or `hand`.
    pub fn applies_to_main_hand(&self) -> bool {
        matches!(self.slot.as_str(), "any" | "mainhand" | "hand")
    }
}

/// Parses the component value: the `ItemAttributeModifiers` codec is a bare list of entries, but
/// a wrapped `{"modifiers": [...]}` is accepted too.
pub(crate) fn parse_modifiers(v: &serde_json::Value) -> Vec<AttributeModifier> {
    let list = v
        .as_array()
        .or_else(|| v.get("modifiers").and_then(|m| m.as_array()));
    let Some(list) = list else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|e| {
            Some(AttributeModifier {
                attribute: Identifier::parse(e.get("type")?.as_str()?).ok()?,
                amount: e.get("amount")?.as_f64()?,
                operation: operation_id(e.get("operation").and_then(|o| o.as_str())),
                slot: e
                    .get("slot")
                    .and_then(|s| s.as_str())
                    .unwrap_or("any")
                    .to_owned(),
            })
        })
        .collect()
}

/// `AttributeModifier.Operation` name -> id (unknown or absent: add_value, vanilla's default).
fn operation_id(name: Option<&str>) -> i32 {
    match name {
        Some("add_multiplied_base") => 1,
        Some("add_multiplied_total") => 2,
        _ => 0,
    }
}
