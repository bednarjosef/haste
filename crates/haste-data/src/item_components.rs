//! Item default data components from the data generator's item component reports
//! (`reports/minecraft/components/item/<item>.json`).
//!
//! A stack's component patch only carries what differs from the item's prototype; the defaults
//! live in the reports. This module reads the ones game logic needs that are not block-related:
//! `minecraft:attribute_modifiers` (the attack speed a held tool or weapon grants, as vanilla's
//! `ItemAttributeModifiers` computes it).

use std::collections::HashMap;
use std::path::Path;

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

/// The item default components game logic reads, by item.
#[derive(Default)]
pub struct ItemComponents {
    attributes: HashMap<Identifier, Vec<AttributeModifier>>,
}

impl ItemComponents {
    /// Reads `<reports>/minecraft/components/item/*.json`.
    pub fn load(reports: &Path) -> anyhow::Result<ItemComponents> {
        let dir = reports.join("minecraft/components/item");
        let mut attributes = HashMap::new();
        for e in std::fs::read_dir(&dir).map_err(|e| anyhow::anyhow!("{}: {e}", dir.display()))? {
            let path = e?.path();
            if path.extension().is_none_or(|x| x != "json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
            if let Some(a) = v
                .get("components")
                .and_then(|c| c.get("minecraft:attribute_modifiers"))
            {
                attributes.insert(Identifier::minecraft(stem), parse_modifiers(a));
            }
        }
        Ok(ItemComponents { attributes })
    }

    /// From the default cache location (`haste_core::paths::datagen_reports`).
    pub fn load_default() -> anyhow::Result<ItemComponents> {
        Self::load(&haste_core::paths::datagen_reports())
    }

    pub fn insert(&mut self, item: Identifier, modifiers: Vec<AttributeModifier>) {
        self.attributes.insert(item, modifiers);
    }

    /// The item's default `minecraft:attribute_modifiers`.
    pub fn attribute_modifiers(&self, item: &Identifier) -> &[AttributeModifier] {
        self.attributes.get(item).map_or(&[], Vec::as_slice)
    }

    /// `ItemAttributeModifiers.compute` for the main hand: `base` plus every main-hand modifier
    /// for `attribute`, in vanilla's operation order (add_value, then add_multiplied_base, then
    /// add_multiplied_total).
    pub fn main_hand_attribute(
        &self,
        item: &Identifier,
        attribute: &Identifier,
        base: f64,
    ) -> f64 {
        let mods = self
            .attribute_modifiers(item)
            .iter()
            .filter(|m| &m.attribute == attribute && m.applies_to_main_hand());
        let mut value = base;
        for op in 0..=2 {
            for m in mods.clone().filter(|m| m.operation == op) {
                value += match op {
                    0 => m.amount,
                    1 => m.amount * base,
                    _ => m.amount * value,
                };
            }
        }
        value
    }

    pub fn len(&self) -> usize {
        self.attributes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.attributes.is_empty()
    }
}

/// Parses the component value: the `ItemAttributeModifiers` codec is a bare list of entries, but
/// a wrapped `{"modifiers": [...]}` is accepted too.
fn parse_modifiers(v: &serde_json::Value) -> Vec<AttributeModifier> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn attr(name: &str) -> Identifier {
        Identifier::minecraft(name)
    }

    fn modifier(slot: &str, amount: f64) -> AttributeModifier {
        AttributeModifier {
            attribute: attr("attack_speed"),
            amount,
            operation: 0,
            slot: slot.to_owned(),
        }
    }

    #[test]
    fn main_hand_attribute_sums_only_main_hand_modifiers() {
        let mut c = ItemComponents::default();
        c.insert(
            attr("diamond_sword"),
            vec![
                AttributeModifier {
                    attribute: attr("attack_damage"),
                    amount: 7.0,
                    operation: 0,
                    slot: "mainhand".into(),
                },
                modifier("mainhand", -2.4),
                // Off-hand and armor modifiers are not the held item's.
                modifier("offhand", 100.0),
                modifier("armor", 100.0),
            ],
        );
        // Player base attack speed 4.0 plus the sword's -2.4 modifier: 1.6.
        assert!(
            (c.main_hand_attribute(&attr("diamond_sword"), &attr("attack_speed"), 4.0) - 1.6).abs()
                < 1e-9
        );
        // An item without the attribute keeps the base.
        assert_eq!(
            c.main_hand_attribute(&attr("stick"), &attr("attack_speed"), 4.0),
            4.0
        );
    }

    #[test]
    fn parses_the_codec_shape() {
        let v = serde_json::json!([
            {"type": "minecraft:attack_speed", "id": "minecraft:base_attack_speed",
             "amount": -2.8, "operation": "add_value", "slot": "mainhand"}
        ]);
        let m = parse_modifiers(&v);
        assert_eq!(m, vec![modifier("mainhand", -2.8)]);
        // A wrapped object and a missing slot (defaults to `any`) both parse.
        let wrapped = serde_json::json!({"modifiers": [
            {"type": "minecraft:attack_speed", "amount": 0.5}
        ]});
        assert_eq!(
            parse_modifiers(&wrapped),
            vec![AttributeModifier {
                attribute: attr("attack_speed"),
                amount: 0.5,
                operation: 0,
                slot: "any".into(),
            }]
        );
    }

    #[test]
    fn operation_order_matches_vanilla() {
        let mut c = ItemComponents::default();
        c.insert(
            attr("test_item"),
            vec![
                AttributeModifier {
                    attribute: attr("attack_speed"),
                    amount: 0.5,
                    operation: 2, // add_multiplied_total (applied last)
                    slot: "mainhand".into(),
                },
                AttributeModifier {
                    attribute: attr("attack_speed"),
                    amount: 1.0,
                    operation: 0, // add_value (applied first)
                    slot: "mainhand".into(),
                },
                AttributeModifier {
                    attribute: attr("attack_speed"),
                    amount: 0.25,
                    operation: 1, // add_multiplied_base
                    slot: "mainhand".into(),
                },
            ],
        );
        // base 4 -> +1 = 5 -> +0.25*4 = 6 -> +0.5*6 = 9.
        assert_eq!(
            c.main_hand_attribute(&attr("test_item"), &attr("attack_speed"), 4.0),
            9.0
        );
    }

    #[test]
    #[ignore = "needs cache/datagen/26.3/reports (data generator)"]
    fn loads_the_data_generator_attribute_modifiers() {
        let c = ItemComponents::load_default().unwrap();
        let speed = attr("attack_speed");
        // Diamond pickaxe: base 4.0 - 2.8 = 1.2 (the report stores the float32 value).
        let pick = c.main_hand_attribute(&attr("diamond_pickaxe"), &speed, 4.0);
        assert!((pick - 1.2).abs() < 1e-6, "diamond pickaxe {pick}");
        // Diamond sword: base 4.0 - 2.4 = 1.6.
        let sword = c.main_hand_attribute(&attr("diamond_sword"), &speed, 4.0);
        assert!((sword - 1.6).abs() < 1e-6, "diamond sword {sword}");
        assert!(c.len() > 30, "{} items", c.len());
    }
}
