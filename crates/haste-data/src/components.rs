//! Item default data components, from the data generator's `RegistryComponentsReport`
//! (`reports/minecraft/components/item/<name>.json`, one file per item that has any non-empty
//! default components). A stack's component patch only carries what differs from the item's prototype;
//! the defaults live in the reports, and Haste does not run vanilla's Java item registry, so it
//! reads the report instead.
//!
//! What the client reads from the reports:
//! - `minecraft:equippable`: which equipment asset an armour piece draws and in which slot
//!   (`HumanoidArmorLayer` / `WingsLayer` read it from the stack, whose default components the
//!   built-in item registry carries);
//! - `minecraft:attribute_modifiers`: the attack speed a held tool or weapon grants, as vanilla's
//!   `ItemAttributeModifiers` computes it;
//! - `minecraft:consumable`: how long an item is used and the animation played while it is;
//! - `minecraft:tool`: what an item mines and how fast (physics turns it into its mining `Tool`);
//! - `minecraft:glider`: the elytra (and any mod item that carries it).
//!
//! A missing report directory or unreadable file yields an empty table, never an error: the
//! components report is optional data.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use ahash::AHashMap;

use haste_core::model::EquipmentSlot;
use haste_core::{Identifier, RegistryIndex};

use crate::registries::Registries;

/// The `equippable` component of one item: the slot it goes in and the equipment asset it draws.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemEquippable {
    pub slot: EquipmentSlot,
    /// `equippable.asset_id` (`minecraft:iron` -> `equipment/iron.json`); `None` for items whose
    /// equippable has no asset (saddles on some mobs, the `equippable` marker without a texture).
    pub asset: Option<Identifier>,
}

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

/// An item's `minecraft:consumable` component (`world.item.component.Consumable`): how long the
/// item is used and the animation played while it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Consumable {
    /// `consume_seconds` (1.6 by default).
    pub consume_seconds: f32,
    /// `ItemUseAnimation` id (`animation`, `eat` by default).
    pub animation: i32,
}

impl Consumable {
    /// `Consumable.consumeTicks`: the use duration in ticks.
    pub fn consume_ticks(&self) -> i32 {
        (self.consume_seconds * 20.0) as i32
    }
}

/// The `blocks` predicate of a `minecraft:tool` rule, in the report's own shape.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolBlocks {
    /// `minecraft:stone` — one block.
    Block(Identifier),
    /// `#minecraft:mineable/pickaxe` — a block tag.
    Tag(Identifier),
    /// A JSON array of block names.
    List(Vec<Identifier>),
}

impl ToolBlocks {
    fn parse(v: &serde_json::Value) -> Option<ToolBlocks> {
        match v {
            serde_json::Value::String(s) => Some(match s.strip_prefix('#') {
                Some(tag) => ToolBlocks::Tag(Identifier::parse(tag).ok()?),
                None => ToolBlocks::Block(Identifier::parse(s).ok()?),
            }),
            serde_json::Value::Array(a) => Some(ToolBlocks::List(
                a.iter()
                    .filter_map(|e| e.as_str().and_then(|s| Identifier::parse(s).ok()))
                    .collect(),
            )),
            _ => None,
        }
    }
}

/// One rule of a `minecraft:tool` component.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolRule {
    pub blocks: ToolBlocks,
    pub speed: Option<f32>,
    pub correct_for_drops: Option<bool>,
}

/// The `minecraft:tool` component of one item (vanilla `ToolComponent`), parsed from the report;
/// physics turns it into its mining `Tool`.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolComponent {
    pub rules: Vec<ToolRule>,
    pub default_mining_speed: f32,
    pub damage_per_block: i32,
    /// Swords and some weapons cannot break blocks in creative.
    pub can_destroy_blocks_in_creative: bool,
}

impl ToolComponent {
    /// Parses the JSON value of a `minecraft:tool` component; `None` when malformed. One
    /// unparseable rule fails the whole component, as vanilla's codec does (a decode error means
    /// the component is absent and `getOrDefault` falls back to no tool).
    pub fn from_json(v: &serde_json::Value) -> Option<ToolComponent> {
        let raw: RawTool = serde_json::from_value(v.clone()).ok()?;
        let rules = raw
            .rules
            .iter()
            .map(|r| {
                Some(ToolRule {
                    blocks: ToolBlocks::parse(r.get("blocks")?)?,
                    speed: r.get("speed").and_then(|s| s.as_f64()).map(|s| s as f32),
                    correct_for_drops: r.get("correct_for_drops").and_then(|c| c.as_bool()),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(ToolComponent {
            rules,
            default_mining_speed: raw.default_mining_speed,
            damage_per_block: raw.damage_per_block,
            can_destroy_blocks_in_creative: raw.can_destroy_blocks_in_creative,
        })
    }
}

#[derive(serde::Deserialize)]
struct RawTool {
    #[serde(default)]
    rules: Vec<serde_json::Value>,
    #[serde(default = "one")]
    default_mining_speed: f32,
    #[serde(default = "one_i")]
    damage_per_block: i32,
    #[serde(default = "yes")]
    can_destroy_blocks_in_creative: bool,
}

fn one() -> f32 {
    1.0
}

fn one_i() -> i32 {
    1
}

fn yes() -> bool {
    true
}

/// Item default components: the `equippable` per `minecraft:item` registry index, and the
/// `attribute_modifiers`, `consumable`, `tool` and `glider` sets by item name. Items without a
/// component (or unknown names) map to `None`/absent.
#[derive(Clone, Debug, Default)]
pub struct ItemComponents {
    equippable: Vec<Option<ItemEquippable>>,
    /// The `minecraft:dyed_color` data-component-type id, from the registry report. `None` when
    /// the report is absent.
    dyed_color_type: Option<i32>,
    attributes: HashMap<Identifier, Vec<AttributeModifier>>,
    consumables: HashMap<Identifier, Consumable>,
    /// `minecraft:food` by item: its `can_always_eat` flag (`FoodProperties.canAlwaysEat`).
    foods: HashMap<Identifier, bool>,
    tools: HashMap<Identifier, ToolComponent>,
    gliders: HashSet<Identifier>,
}

impl ItemComponents {
    /// The `equippable` of item `i`, if the report named one.
    pub fn equippable(&self, i: RegistryIndex) -> Option<&ItemEquippable> {
        self.equippable.get(i.0 as usize)?.as_ref()
    }

    /// The item's default `minecraft:attribute_modifiers`.
    pub fn attribute_modifiers(&self, item: &Identifier) -> &[AttributeModifier] {
        self.attributes.get(item).map_or(&[], Vec::as_slice)
    }

    /// The item's default `minecraft:consumable`, if it has one.
    pub fn consumable(&self, item: &Identifier) -> Option<Consumable> {
        self.consumables.get(item).copied()
    }

    /// The item's default `minecraft:food`, if it has one: `Some(can_always_eat)`
    /// (`FoodProperties.canAlwaysEat`, false by default).
    pub fn food(&self, item: &Identifier) -> Option<bool> {
        self.foods.get(item).copied()
    }

    /// The item's default `minecraft:tool` component, if it has one.
    pub fn tool(&self, item: &Identifier) -> Option<&ToolComponent> {
        self.tools.get(item)
    }

    pub fn tools(&self) -> &HashMap<Identifier, ToolComponent> {
        &self.tools
    }

    /// Items whose default components carry `minecraft:glider`.
    pub fn gliders(&self) -> &HashSet<Identifier> {
        &self.gliders
    }

    /// Whether item's default components carry `minecraft:glider` (vanilla
    /// `LivingEntity.canGlideUsing`'s `DataComponents.GLIDER`).
    pub fn is_glider(&self, item: &Identifier) -> bool {
        self.gliders.contains(item)
    }

    /// `ItemAttributeModifiers.compute` for the main hand: `base` plus every main-hand modifier
    /// for `attribute`, in vanilla's operation order (add_value, then add_multiplied_base, then
    /// add_multiplied_total).
    pub fn main_hand_attribute(&self, item: &Identifier, attribute: &Identifier, base: f64) -> f64 {
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

    /// The `dyed_color` (ARGB) a stack's component patch sets, if it is the patch's first added
    /// component (`DyedItemColor.STREAM_CODEC` is a plain int; vanilla's `getOrDefault` falls
    /// back to the layer's `color_when_undyed` when absent). A patch whose first add is a
    /// component Haste cannot skip returns `None` (dye is then simply not applied).
    pub fn dyed_color(&self, components_raw: &[u8]) -> Option<u32> {
        let dyed = self.dyed_color_type?;
        let mut r = PatchReader::new(components_raw);
        let adds = r.var_int()?;
        let _removes = r.var_int()?;
        if adds <= 0 {
            return None;
        }
        let ty = r.var_int()?;
        if ty != dyed {
            return None;
        }
        r.i32().map(|c| c as u32)
    }

    /// Sets an item's default `minecraft:attribute_modifiers` (tests and tools).
    pub fn insert(&mut self, item: Identifier, modifiers: Vec<AttributeModifier>) {
        self.attributes.insert(item, modifiers);
    }

    /// Sets an item's default `minecraft:consumable` (tests and tools).
    pub fn insert_consumable(&mut self, item: Identifier, consumable: Consumable) {
        self.consumables.insert(item, consumable);
    }

    /// Sets an item's default `minecraft:food` (tests and tools).
    pub fn insert_food(&mut self, item: Identifier, can_always_eat: bool) {
        self.foods.insert(item, can_always_eat);
    }

    /// How many items carry `minecraft:attribute_modifiers`.
    pub fn len(&self) -> usize {
        self.attributes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.attributes.is_empty()
    }

    /// Loads every `reports/minecraft/components/item/*.json` under `components_dir` (the
    /// report's `minecraft/components/item` directory), mapping each file's name through
    /// `registries`. A missing directory or unreadable file yields an empty table (armour then
    /// draws nothing), never an error: the components report is optional data. `GameData::load`
    /// warns (naming `haste setup`) when the directory is absent.
    pub fn load(components_dir: &Path, registries: &Registries) -> ItemComponents {
        let mut by_name: AHashMap<String, ItemEquippable> = AHashMap::new();
        let mut attributes: HashMap<Identifier, Vec<AttributeModifier>> = HashMap::new();
        let mut consumables: HashMap<Identifier, Consumable> = HashMap::new();
        let mut foods: HashMap<Identifier, bool> = HashMap::new();
        let mut tools: HashMap<Identifier, ToolComponent> = HashMap::new();
        let mut gliders: HashSet<Identifier> = HashSet::new();
        let Ok(entries) = std::fs::read_dir(components_dir) else {
            return ItemComponents {
                dyed_color_type: dyed_color_type(registries),
                ..ItemComponents::default()
            };
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            if let Some(eq) = parse_equippable(&bytes) {
                by_name.insert(stem.to_owned(), eq);
            }
            let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            let item = Identifier::minecraft(stem);
            let Some(components) = json.get("components") else {
                continue;
            };
            if let Some(a) = components.get("minecraft:attribute_modifiers") {
                attributes.insert(item.clone(), parse_modifiers(a));
            }
            if let Some(c) = components.get("minecraft:consumable") {
                consumables.insert(item.clone(), parse_consumable(c));
            }
            if let Some(f) = components.get("minecraft:food") {
                let always = f
                    .get("can_always_eat")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                foods.insert(item.clone(), always);
            }
            if let Some(t) = components.get("minecraft:tool") {
                if let Some(t) = ToolComponent::from_json(t) {
                    tools.insert(item.clone(), t);
                }
            }
            if components.get("minecraft:glider").is_some() {
                gliders.insert(item);
            }
        }
        let len = registries.len("minecraft:item");
        let mut equippable = vec![None; len];
        for (name, eq) in by_name {
            if let Some(i) = registries.index("minecraft:item", &Identifier::minecraft(&name)) {
                equippable[i.0 as usize] = Some(eq);
            }
        }
        ItemComponents {
            equippable,
            dyed_color_type: dyed_color_type(registries),
            attributes,
            consumables,
            foods,
            tools,
            gliders,
        }
    }

    /// Loads from the default cache location (`haste_core::paths::datagen_reports`); `Err` when
    /// the registry report itself is absent (the item report may still be missing then — that
    /// loads as an empty table).
    pub fn load_default() -> anyhow::Result<ItemComponents> {
        let reports = haste_core::paths::datagen_reports();
        let registries =
            Registries::parse(&std::fs::read_to_string(reports.join("registries.json"))?)?;
        Ok(Self::load(
            &reports.join("minecraft").join("components").join("item"),
            &registries,
        ))
    }
}

fn dyed_color_type(registries: &Registries) -> Option<i32> {
    registries
        .index(
            "minecraft:data_component_type",
            &Identifier::minecraft("dyed_color"),
        )
        .map(|i| i.0 as i32)
}

/// A minimal varint/int reader over a component patch's raw bytes (only what
/// [`ItemComponents::dyed_color`] needs).
struct PatchReader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> PatchReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        PatchReader { bytes, at: 0 }
    }

    fn var_int(&mut self) -> Option<i32> {
        let mut value = 0u32;
        let mut shift = 0;
        loop {
            let byte = *self.bytes.get(self.at)?;
            self.at += 1;
            value |= ((byte & 0x7F) as u32) << shift;
            if byte & 0x80 == 0 {
                return Some(value as i32);
            }
            shift += 7;
            if shift >= 32 {
                return None;
            }
        }
    }

    fn i32(&mut self) -> Option<i32> {
        let b = self.bytes.get(self.at..self.at + 4)?;
        self.at += 4;
        Some(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
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

/// Parses `minecraft:consumable` (`Consumable.CODEC`): `consume_seconds` and `animation` with
/// their defaults.
fn parse_consumable(v: &serde_json::Value) -> Consumable {
    Consumable {
        consume_seconds: v
            .get("consume_seconds")
            .and_then(|s| s.as_f64())
            .unwrap_or(1.6) as f32,
        animation: use_animation_id(v.get("animation").and_then(|a| a.as_str())),
    }
}

/// `ItemUseAnimation` name -> id.
fn use_animation_id(name: Option<&str>) -> i32 {
    match name {
        Some("none") => 0,
        Some("drink") => 2,
        Some("block") => 3,
        Some("bow") => 4,
        Some("trident") => 5,
        Some("crossbow") => 6,
        Some("spyglass") => 7,
        Some("toot_horn") => 8,
        Some("brush") => 9,
        Some("bundle") => 10,
        Some("spear") => 11,
        // `eat` and any unknown name are vanilla's default.
        _ => 1,
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
    fn parses_the_equippable_component() {
        let json = br#"{"components":{"minecraft:equippable":{"slot":"chest","asset_id":"minecraft:iron"}}}"#;
        let eq = parse_equippable(json).unwrap();
        assert_eq!(eq.slot, EquipmentSlot::Chest);
        assert_eq!(eq.asset, Some(Identifier::minecraft("iron")));
    }

    #[test]
    fn missing_equippable_is_none() {
        assert_eq!(parse_equippable(br#"{"components":{}}"#), None);
        assert_eq!(parse_equippable(b"not json"), None);
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
    fn parses_consumables_with_defaults() {
        // `Consumable.CODEC` defaults: 1.6 s and `eat`.
        assert_eq!(
            parse_consumable(&serde_json::json!({})),
            Consumable {
                consume_seconds: 1.6,
                animation: 1,
            }
        );
        // A drink with its own time: `consumeTicks` = seconds * 20.
        let drink = parse_consumable(&serde_json::json!({
            "consume_seconds": 1.6, "animation": "drink"
        }));
        assert_eq!(drink.animation, 2);
        assert_eq!(drink.consume_ticks(), 32);
    }

    #[test]
    fn parses_the_tool_component() {
        let t = ToolComponent::from_json(&serde_json::json!({
            "rules": [
                {"blocks": "minecraft:cobweb", "correct_for_drops": true, "speed": 15.0},
                {"blocks": ["minecraft:stone", "minecraft:dirt"], "speed": 1.5},
                {"blocks": "#minecraft:mineable/pickaxe", "speed": 8.0}
            ],
            "default_mining_speed": 1.0,
            "damage_per_block": 1,
            "can_destroy_blocks_in_creative": false
        }))
        .unwrap();
        assert_eq!(t.rules.len(), 3);
        assert_eq!(
            t.rules[0].blocks,
            ToolBlocks::Block(Identifier::minecraft("cobweb"))
        );
        assert_eq!(t.rules[0].speed, Some(15.0));
        assert_eq!(
            t.rules[2].blocks,
            ToolBlocks::Tag(Identifier::minecraft("mineable/pickaxe"))
        );
        assert!(!t.can_destroy_blocks_in_creative);
        // Defaults: speed 1, damage 1, creative break allowed; malformed is skipped.
        let d = ToolComponent::from_json(&serde_json::json!({"rules": []})).unwrap();
        assert_eq!((d.default_mining_speed, d.damage_per_block), (1.0, 1));
        assert!(d.can_destroy_blocks_in_creative);
        assert_eq!(
            ToolComponent::from_json(&serde_json::json!({"rules": [{"blocks": 5}]})),
            None
        );
    }

    #[test]
    fn loads_by_registry_name_and_ignores_unknown_items() {
        let dir = std::env::temp_dir().join(format!(
            "haste-components-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let item_dir = dir.join("item");
        std::fs::create_dir_all(&item_dir).unwrap();
        std::fs::write(
            item_dir.join("iron_chestplate.json"),
            br#"{"components":{"minecraft:equippable":{"slot":"chest","asset_id":"minecraft:iron"}}}"#,
        )
        .unwrap();
        std::fs::write(item_dir.join("unknown_item.json"), br#"{"components":{}}"#).unwrap();
        let registries_json = r#"{
  "minecraft:item": {
    "entries": {
      "minecraft:air": { "protocol_id": 0 },
      "minecraft:iron_chestplate": { "protocol_id": 1 }
    }
  },
  "minecraft:data_component_type": {
    "entries": {
      "minecraft:damage": { "protocol_id": 0 },
      "minecraft:dyed_color": { "protocol_id": 1 }
    }
  }
}"#;
        let regs = Registries::parse(registries_json).unwrap();
        let components = ItemComponents::load(&item_dir, &regs);
        let eq = components.equippable(RegistryIndex(1)).unwrap();
        assert_eq!(eq.asset, Some(Identifier::minecraft("iron")));
        assert!(components.equippable(RegistryIndex(0)).is_none());
        // `dyed_color` as the patch's first add (type id 1, a big-endian int) decodes.
        let patch = [1u8, 0, 1, 0x00, 0x11, 0x22, 0x33];
        assert_eq!(components.dyed_color(&patch), Some(0x0011_2233));
        // A patch whose first add is another component gives up (dye not applied).
        assert_eq!(components.dyed_color(&[1, 0, 0, 5]), None);
        assert_eq!(components.dyed_color(&[0, 0]), None);
        // A missing directory is empty, not an error.
        let empty = ItemComponents::load(&dir.join("nope"), &regs);
        assert!(empty.equippable(RegistryIndex(1)).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// One report file per component: the load reads them all in one pass.
    #[test]
    fn loads_equippable_attributes_consumables_tools_and_gliders() {
        let dir = std::env::temp_dir().join(format!(
            "haste-components-all-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let item_dir = dir.join("item");
        std::fs::create_dir_all(&item_dir).unwrap();
        std::fs::write(
            item_dir.join("iron_chestplate.json"),
            br#"{"components":{"minecraft:equippable":{"slot":"chest","asset_id":"minecraft:iron"}}}"#,
        )
        .unwrap();
        std::fs::write(
            item_dir.join("diamond_sword.json"),
            br#"{"components":{
                "minecraft:attribute_modifiers": [
                    {"type": "minecraft:attack_speed", "amount": -2.4, "operation": "add_value", "slot": "mainhand"}
                ],
                "minecraft:tool": {"rules": [
                    {"blocks": "minecraft:cobweb", "speed": 15.0, "correct_for_drops": true}
                ], "can_destroy_blocks_in_creative": false}
            }}"#,
        )
        .unwrap();
        std::fs::write(
            item_dir.join("apple.json"),
            br#"{"components":{"minecraft:consumable": {"consume_seconds": 1.6, "animation": "eat"}}}"#,
        )
        .unwrap();
        std::fs::write(
            item_dir.join("elytra.json"),
            br#"{"components":{"minecraft:glider": {}}}"#,
        )
        .unwrap();
        // A malformed tool entry is skipped, not fatal.
        std::fs::write(
            item_dir.join("broken_pickaxe.json"),
            br#"{"components":{"minecraft:tool": {"rules": [{"blocks": 5}]}}}"#,
        )
        .unwrap();
        let registries_json = r#"{
  "minecraft:item": {
    "entries": {
      "minecraft:air": { "protocol_id": 0 },
      "minecraft:iron_chestplate": { "protocol_id": 1 },
      "minecraft:diamond_sword": { "protocol_id": 2 },
      "minecraft:apple": { "protocol_id": 3 },
      "minecraft:elytra": { "protocol_id": 4 },
      "minecraft:broken_pickaxe": { "protocol_id": 5 }
    }
  },
  "minecraft:data_component_type": {
    "entries": {
      "minecraft:dyed_color": { "protocol_id": 0 }
    }
  }
}"#;
        let regs = Registries::parse(registries_json).unwrap();
        let c = ItemComponents::load(&item_dir, &regs);
        // equippable by registry index.
        assert!(c.equippable(RegistryIndex(1)).is_some());
        assert!(c.equippable(RegistryIndex(0)).is_none());
        // attribute modifiers and consumable by name.
        let sword = attr("diamond_sword");
        assert_eq!(
            c.main_hand_attribute(&sword, &attr("attack_speed"), 4.0),
            1.6
        );
        assert_eq!(c.consumable(&attr("apple")).unwrap().consume_ticks(), 32);
        // the tool component parses (and the malformed one was skipped).
        let tool = c.tool(&sword).unwrap();
        assert_eq!(tool.rules.len(), 1);
        assert!(!tool.can_destroy_blocks_in_creative);
        assert!(c.tool(&attr("broken_pickaxe")).is_none());
        assert_eq!(c.tools().len(), 1);
        // glider.
        assert!(c.is_glider(&attr("elytra")));
        assert!(!c.is_glider(&attr("stick")));
        let _ = std::fs::remove_dir_all(&dir);
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
        // Food and drink carry a `minecraft:consumable`.
        let apple = c.consumable(&attr("apple")).expect("apple consumable");
        assert_eq!((apple.animation, apple.consume_ticks()), (1, 32));
        let potion = c.consumable(&attr("potion")).expect("potion consumable");
        assert_eq!(potion.animation, 2);
    }
}
