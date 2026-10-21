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
//! - `minecraft:glider`: the elytra (and any mod item that carries it);
//! - `minecraft:death_protection`: the totem of undying (and any mod item that carries it);
//! - `minecraft:use_effects`: what using the item does to the player (sprint, input speed).
//!
//! A missing report directory or unreadable file yields an empty table, never an error: the
//! components report is optional data.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use ahash::AHashMap;

use haste_core::{Identifier, RegistryIndex};

use crate::registries::Registries;

mod attributes;
mod consumable;
mod equippable;
mod patch;
mod tool;
mod use_effects;

#[cfg(test)]
mod tests;

pub use attributes::AttributeModifier;
pub use consumable::Consumable;
pub use equippable::{parse_equippable, ItemEquippable};
pub use tool::ToolComponent;
pub use use_effects::UseEffects;

use attributes::parse_modifiers;
use consumable::parse_consumable;
use patch::{dyed_color_type, PatchReader};
use use_effects::parse_use_effects;

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
    /// Items whose default components carry `minecraft:death_protection`.
    death_protection: HashSet<Identifier>,
    use_effects: HashMap<Identifier, UseEffects>,
    /// Items whose default components carry `minecraft:piercing_weapon` (the spears): the
    /// weapon's optional draw sound, if the report names one.
    piercing_weapons: HashMap<Identifier, Option<String>>,
    /// `minecraft:minimum_attack_charge` by item (`Player.cannotAttackWithItem`).
    minimum_attack_charges: HashMap<Identifier, f32>,
    /// The `cooldown_group` of the items whose default components carry a
    /// `minecraft:use_cooldown` that names one (`ItemCooldowns.getCooldownGroup`); an item
    /// without an entry groups under its own registry id.
    use_cooldowns: HashMap<Identifier, Identifier>,
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
        self.consumables.get(item).cloned()
    }

    /// The names of the items whose default components carry `minecraft:consumable` (the
    /// ones a use can emit effects for; the renderer packs their particle sprites).
    pub fn consumable_items(&self) -> impl Iterator<Item = &Identifier> {
        self.consumables.keys()
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

    /// The item's default `minecraft:piercing_weapon` draw sound (`weapon.makeSound`), when
    /// the item is a spear. `Some(None)` = a spear without a sound.
    pub fn piercing_weapon_sound(&self, item: &Identifier) -> Option<Option<&str>> {
        self.piercing_weapons.get(item).map(|s| s.as_deref())
    }

    /// The item's default `minecraft:minimum_attack_charge` (`Player.cannotAttackWithItem`),
    /// if it has one.
    pub fn minimum_attack_charge(&self, item: &Identifier) -> Option<f32> {
        self.minimum_attack_charges.get(item).copied()
    }

    /// The item's `minecraft:use_cooldown` group (`ItemCooldowns.getCooldownGroup`): the
    /// component's `cooldown_group`, or the item's own registry id when it names none (or has
    /// no component at all — then `None`, the caller falls back to the id it resolved).
    pub fn cooldown_group(&self, item: &Identifier) -> Option<&Identifier> {
        self.use_cooldowns.get(item)
    }

    /// The names of the items whose default components carry a `minecraft:use_cooldown` that
    /// names a group (tests).
    #[cfg(test)]
    pub fn use_cooldown_items(&self) -> impl Iterator<Item = &Identifier> {
        self.use_cooldowns.keys()
    }

    /// Whether item's default components carry `minecraft:glider` (vanilla
    /// `LivingEntity.canGlideUsing`'s `DataComponents.GLIDER`).
    pub fn is_glider(&self, item: &Identifier) -> bool {
        self.gliders.contains(item)
    }

    /// Whether item's default components carry `minecraft:death_protection` (vanilla
    /// `ItemStack.has(DataComponents.DEATH_PROTECTION)` for a stack without a patch).
    pub fn has_death_protection(&self, item: &Identifier) -> bool {
        self.death_protection.contains(item)
    }

    /// The item's `minecraft:use_effects`, `UseEffects.DEFAULT` when the report named none or
    /// the item is unknown (vanilla's `getOrDefault(USE_EFFECTS, UseEffects.DEFAULT)`: every
    /// item's prototype carries the component).
    pub fn use_effects(&self, item: &Identifier) -> UseEffects {
        self.use_effects
            .get(item)
            .copied()
            .unwrap_or(UseEffects::DEFAULT)
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

    /// Sets an item's default `minecraft:piercing_weapon` and its draw sound (tests).
    pub fn insert_piercing_weapon(&mut self, item: Identifier, sound: Option<String>) {
        self.piercing_weapons.insert(item, sound);
    }

    /// Sets an item's default `minecraft:minimum_attack_charge` (tests).
    pub fn insert_minimum_attack_charge(&mut self, item: Identifier, charge: f32) {
        self.minimum_attack_charges.insert(item, charge);
    }

    /// Sets an item's `minecraft:use_cooldown` group (tests).
    pub fn insert_use_cooldown(&mut self, item: Identifier, group: Identifier) {
        self.use_cooldowns.insert(item, group);
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
        let mut death_protection: HashSet<Identifier> = HashSet::new();
        let mut use_effects: HashMap<Identifier, UseEffects> = HashMap::new();
        let mut piercing_weapons: HashMap<Identifier, Option<String>> = HashMap::new();
        let mut minimum_attack_charges: HashMap<Identifier, f32> = HashMap::new();
        let mut use_cooldowns: HashMap<Identifier, Identifier> = HashMap::new();
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
            if let Some(p) = components.get("minecraft:piercing_weapon") {
                // The draw sound the report names ("sound": "minecraft:item.spear.attack" or
                // an inline sound object); a spear may also name none.
                let sound = p.get("sound").and_then(|s| match s {
                    serde_json::Value::String(n) => Some(n.clone()),
                    serde_json::Value::Object(_) => s
                        .get("sound_id")
                        .and_then(|v| v.as_str())
                        .map(|n| n.to_owned()),
                    _ => None,
                });
                piercing_weapons.insert(item.clone(), sound);
            }
            if let Some(c) = components.get("minecraft:minimum_attack_charge") {
                if let Some(c) = c.as_f64() {
                    minimum_attack_charges.insert(item.clone(), c as f32);
                }
            }
            if components.get("minecraft:glider").is_some() {
                gliders.insert(item.clone());
            }
            if components.get("minecraft:death_protection").is_some() {
                death_protection.insert(item.clone());
            }
            if let Some(e) = components.get("minecraft:use_effects") {
                use_effects.insert(item.clone(), parse_use_effects(e));
            }
            if let Some(group) = components
                .get("minecraft:use_cooldown")
                .and_then(|c| c.get("cooldown_group"))
                .and_then(|g| g.as_str())
                .and_then(|g| Identifier::parse(g).ok())
            {
                use_cooldowns.insert(item.clone(), group);
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
            death_protection,
            use_effects,
            piercing_weapons,
            minimum_attack_charges,
            use_cooldowns,
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
