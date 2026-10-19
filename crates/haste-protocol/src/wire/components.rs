//! Typed item data components: reading a stack's component patch.
//!
//! `ItemStackData::components_raw` keeps a stack's patch exactly as it is on
//! the wire, where component data carries no length prefix: skipping a
//! component needs its layout, which `slot.rs` knows by registry name
//! (resolved through `minecraft:data_component_type` in
//! [`ProtocolTables`]). [`read_patch`] walks that slice once and decodes the
//! components game logic asks about (how damaged a tool is, its durability,
//! its enchantments, its custom name, what it mines, how it is consumed);
//! every other component
//! goes through [`skip_component`] and is kept raw as
//! [`ItemComponent::Other`], so unknown and mod-added components do not fail
//! as long as their layout is known.
//!
//! Field orders follow vanilla's data component classes (`DataComponents`,
//! `world/item/component/Tool`, `world/item/enchantment/ItemEnchantments`):
//! `Tool` is its rules then the mining and durability defaults, an enchantment
//! is a plain registry id with its level.

use bytes::Bytes;
use haste_core::Component;

use super::slot::{component_name, skip_component, skip_consume_effect};
use super::{DecodeError, DecodeResult, IdOr, IdSet, Reader, SoundEvent, Wire, MAX_LIST};
use crate::packets::ids::ProtocolTables;

/// Components in one patch (the same cap `slot::skip_patch` enforces).
const MAX_PATCH_COMPONENTS: usize = 4096;

/// Enchantments in one list (as in `slot::skip_component`).
const MAX_ENCHANTMENTS: usize = 1024;

/// Decodes a whole component patch (`add count, remove count, adds,
/// removes`) into typed components plus the removed type names.
///
/// `raw` is exactly a patch ([`crate::wire::slot`] stores nothing else);
/// anything left over is a layout mismatch and an error. An empty patch
/// allocates nothing.
pub fn read_patch(raw: &[u8], tables: &ProtocolTables) -> DecodeResult<ComponentPatch> {
    let mut r = Reader::new(raw).with_tables(tables);
    let adds = r.len_prefix(MAX_PATCH_COMPONENTS)?;
    let removes = r.len_prefix(MAX_PATCH_COMPONENTS)?;
    let mut added = Vec::with_capacity(adds.min(raw.len()));
    for _ in 0..adds {
        let ty = r.var_int()?;
        let name = component_name(&r, ty)?;
        added.push(read_component(&mut r, name)?);
    }
    let mut removed = Vec::with_capacity(removes.min(raw.len()));
    for _ in 0..removes {
        let ty = r.var_int()?;
        removed.push(component_name(&r, ty)?.to_owned());
    }
    r.finish()?;
    Ok(ComponentPatch { added, removed })
}

/// Reads one component's value by its `minecraft:data_component_type` name.
fn read_component(r: &mut Reader<'_>, name: &str) -> DecodeResult<ItemComponent> {
    let short = name.strip_prefix("minecraft:").unwrap_or(name);
    let component = match short {
        "max_stack_size" => ItemComponent::MaxStackSize(r.var_int()?),
        "max_damage" => ItemComponent::MaxDamage(r.var_int()?),
        "damage" => ItemComponent::Damage(r.var_int()?),
        "unbreakable" => ItemComponent::Unbreakable,
        "rarity" => ItemComponent::Rarity(Rarity::from_id(r.var_int()?)),
        "repair_cost" => ItemComponent::RepairCost(r.var_int()?),
        "custom_name" => ItemComponent::CustomName(r.component()?),
        "item_name" => ItemComponent::ItemName(r.component()?),
        "enchantments" => ItemComponent::Enchantments(read_enchantments(r)?),
        "stored_enchantments" => ItemComponent::StoredEnchantments(read_enchantments(r)?),
        "tool" => ItemComponent::Tool(read_tool(r)?),
        "minimum_attack_charge" => ItemComponent::MinimumAttackCharge(r.f32()?),
        "piercing_weapon" => ItemComponent::PiercingWeapon(read_piercing_weapon(r)?),
        "consumable" => ItemComponent::Consumable(read_consumable(r)?),
        // Layout known, value not modelled: keep it as written.
        _ => {
            let start = r.peek_rest();
            skip_component(r, name)?;
            let used = start.len() - r.remaining();
            ItemComponent::Other {
                name: name.to_owned(),
                raw: r.to_bytes(&start[..used]),
            }
        }
    };
    Ok(component)
}

/// `enchantments` / `stored_enchantments`: registry id + level pairs. The id is the plain
/// `minecraft:enchantment` registry id (vanilla `Enchantment.STREAM_CODEC` is
/// `ByteBufCodecs.holderRegistry`, which has no inline form and no +1 offset).
fn read_enchantments(r: &mut Reader<'_>) -> DecodeResult<Vec<EnchantmentLevel>> {
    r.list(MAX_ENCHANTMENTS, |r| {
        let id = r.var_int()?;
        if id < 0 {
            return Err(DecodeError::Invalid(format!(
                "enchantment id {id} is negative"
            )));
        }
        Ok(EnchantmentLevel {
            id,
            level: r.var_int()?,
        })
    })
}

/// `tool`: rules for named blocks, then the mining and damage defaults.
fn read_tool(r: &mut Reader<'_>) -> DecodeResult<Tool> {
    let rules = r.list(MAX_LIST, |r| {
        Ok(ToolRule {
            blocks: IdSet::read(r)?,
            speed: r.option(|r| r.f32())?,
            correct_for_drops: r.option(|r| r.bool())?,
        })
    })?;
    Ok(Tool {
        rules,
        default_mining_speed: r.f32()?,
        damage_per_block: r.var_int()?,
        can_destroy_blocks_in_creative: r.bool()?,
    })
}

/// `piercing_weapon`: knockback and dismount flags, then the optional draw and hit sounds
/// (vanilla `PiercingWeapon.STREAM_CODEC`).
fn read_piercing_weapon(r: &mut Reader<'_>) -> DecodeResult<PiercingWeapon> {
    Ok(PiercingWeapon {
        deals_knockback: r.bool()?,
        dismounts: r.bool()?,
        sound: r.option(SoundEvent::read)?,
        hit_sound: r.option(SoundEvent::read)?,
    })
}

/// `consumable` (vanilla `Consumable.STREAM_CODEC`): the use duration, the
/// animation played while it runs, and the sound and particles the use emits
/// (`Item.getUseDuration`/`getUseAnimation`, `ItemStack.onUseTick`). The
/// `on_consume_effects` run server-side only (vanilla `Consumable.onConsume`
/// guards them with `!level.isClientSide`), so the list is consumed by its
/// codec but not modelled.
fn read_consumable(r: &mut Reader<'_>) -> DecodeResult<Consumable> {
    let consume_seconds = r.f32()?;
    let animation = r.var_int()?;
    let sound = IdOr::<SoundEvent>::read(r)?;
    let has_consume_particles = r.bool()?;
    r.list(MAX_LIST, skip_consume_effect)?;
    Ok(Consumable {
        consume_seconds,
        animation,
        sound,
        has_consume_particles,
    })
}

/// The components one stack adds, and the ones it removes from its item's
/// defaults.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ComponentPatch {
    /// Added components, in wire order.
    pub added: Vec<ItemComponent>,
    /// Removed component type names, as registry ids.
    pub removed: Vec<String>,
}

impl ComponentPatch {
    /// `damage`: how damaged the stack is (`None` when not damaged).
    pub fn damage(&self) -> Option<i32> {
        self.added.iter().find_map(|c| match c {
            ItemComponent::Damage(v) => Some(*v),
            _ => None,
        })
    }

    /// `max_damage`: the item's durability.
    pub fn max_damage(&self) -> Option<i32> {
        self.added.iter().find_map(|c| match c {
            ItemComponent::MaxDamage(v) => Some(*v),
            _ => None,
        })
    }

    /// `max_stack_size` for this particular stack.
    pub fn max_stack_size(&self) -> Option<i32> {
        self.added.iter().find_map(|c| match c {
            ItemComponent::MaxStackSize(v) => Some(*v),
            _ => None,
        })
    }

    /// `unbreakable`: the stack never loses durability.
    pub fn unbreakable(&self) -> bool {
        self.added
            .iter()
            .any(|c| matches!(c, ItemComponent::Unbreakable))
    }

    /// `rarity`.
    pub fn rarity(&self) -> Option<Rarity> {
        self.added.iter().find_map(|c| match c {
            ItemComponent::Rarity(v) => Some(*v),
            _ => None,
        })
    }

    /// `repair_cost`: anvil repair levels this stack has already cost.
    pub fn repair_cost(&self) -> Option<i32> {
        self.added.iter().find_map(|c| match c {
            ItemComponent::RepairCost(v) => Some(*v),
            _ => None,
        })
    }

    /// `custom_name`: the player-renamed name, if any.
    pub fn custom_name(&self) -> Option<&Component> {
        self.added.iter().find_map(|c| match c {
            ItemComponent::CustomName(v) => Some(v),
            _ => None,
        })
    }

    /// `item_name`: the name shown when `custom_name` has no styling (iron
    /// blocks, potions, enchanted books).
    pub fn item_name(&self) -> Option<&Component> {
        self.added.iter().find_map(|c| match c {
            ItemComponent::ItemName(v) => Some(v),
            _ => None,
        })
    }

    /// `enchantments` on the stack itself (not `stored_enchantments`).
    pub fn enchantments(&self) -> Option<&[EnchantmentLevel]> {
        self.added.iter().find_map(|c| match c {
            ItemComponent::Enchantments(v) => Some(v.as_slice()),
            _ => None,
        })
    }

    /// `stored_enchantments` (as on an enchanted book).
    pub fn stored_enchantments(&self) -> Option<&[EnchantmentLevel]> {
        self.added.iter().find_map(|c| match c {
            ItemComponent::StoredEnchantments(v) => Some(v.as_slice()),
            _ => None,
        })
    }

    /// `tool`: what this stack mines and how fast.
    pub fn tool(&self) -> Option<&Tool> {
        self.added.iter().find_map(|c| match c {
            ItemComponent::Tool(v) => Some(v),
            _ => None,
        })
    }

    /// `minimum_attack_charge`: the attack strength (0..1) below which the item cannot attack
    /// (`Player.cannotAttackWithItem`).
    pub fn minimum_attack_charge(&self) -> Option<f32> {
        self.added.iter().find_map(|c| match c {
            ItemComponent::MinimumAttackCharge(v) => Some(*v),
            _ => None,
        })
    }

    /// `piercing_weapon`: the item is a spear; a left click is a STAB, not an attack or a
    /// block break (`Minecraft.startAttack`).
    pub fn piercing_weapon(&self) -> Option<&PiercingWeapon> {
        self.added.iter().find_map(|c| match c {
            ItemComponent::PiercingWeapon(v) => Some(v),
            _ => None,
        })
    }

    /// `minecraft:consumable` the patch adds, if any (the stack's own component,
    /// overriding the item's default).
    pub fn consumable(&self) -> Option<&Consumable> {
        self.added.iter().find_map(|c| match c {
            ItemComponent::Consumable(v) => Some(v),
            _ => None,
        })
    }

    /// Whether the patch removes `minecraft:consumable` (the stack is not
    /// consumable, whatever the item's default).
    pub fn removes_consumable(&self) -> bool {
        self.removed.iter().any(|n| n == "minecraft:consumable")
    }
}

/// One added data component with its decoded value.
#[derive(Clone, Debug, PartialEq)]
pub enum ItemComponent {
    /// `max_stack_size`.
    MaxStackSize(i32),
    /// `max_damage`: the item's durability.
    MaxDamage(i32),
    /// `damage`: how damaged this stack is.
    Damage(i32),
    /// `unbreakable`: no value.
    Unbreakable,
    /// `rarity`.
    Rarity(Rarity),
    /// `repair_cost`.
    RepairCost(i32),
    /// `custom_name`.
    CustomName(Component),
    /// `item_name`.
    ItemName(Component),
    /// `enchantments`.
    Enchantments(Vec<EnchantmentLevel>),
    /// `stored_enchantments`.
    StoredEnchantments(Vec<EnchantmentLevel>),
    /// `tool`.
    Tool(Tool),
    /// `minimum_attack_charge`.
    MinimumAttackCharge(f32),
    /// `piercing_weapon`.
    PiercingWeapon(PiercingWeapon),
    /// `consumable`: how the stack is used and what using it emits.
    Consumable(Consumable),
    /// Any other component: its type name as a registry id and its data as
    /// written (skipped by `slot::skip_component`).
    Other {
        /// Type name, e.g. `minecraft:food`.
        name: String,
        /// The component's encoded data.
        raw: Bytes,
    },
}

/// One enchantment with its level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnchantmentLevel {
    /// Index into `minecraft:enchantment` (the wire sends this plus one).
    pub id: i32,
    /// Level.
    pub level: i32,
}

/// `tool`: the rules that make a stack mine, plus the defaults for blocks
/// no rule matches.
#[derive(Clone, Debug, PartialEq)]
pub struct Tool {
    /// Rules in order; the first matching block wins.
    pub rules: Vec<ToolRule>,
    /// Mining speed for blocks no rule speeds up.
    pub default_mining_speed: f32,
    /// Durability spent per broken block.
    pub damage_per_block: i32,
    /// Whether the tool can break blocks while the player is creative.
    pub can_destroy_blocks_in_creative: bool,
}

/// One `tool` rule: a set of blocks and what the tool does to them.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolRule {
    /// Blocks the rule covers.
    pub blocks: IdSet,
    /// Mining speed for those blocks.
    pub speed: Option<f32>,
    /// Whether those blocks drop themselves.
    pub correct_for_drops: Option<bool>,
}

/// `piercing_weapon`: a spear's stab parameters (vanilla `PiercingWeapon`), the draw and hit
/// sounds optional.
#[derive(Clone, Debug, PartialEq)]
pub struct PiercingWeapon {
    pub deals_knockback: bool,
    pub dismounts: bool,
    /// The draw sound (`weapon.makeSound`).
    pub sound: Option<SoundEvent>,
    /// The hit sound (`weapon.makeHitSound`).
    pub hit_sound: Option<SoundEvent>,
}

/// `consumable` (vanilla `Consumable`): how long a stack is used for and what
/// using it emits — `consume_seconds`, the `ItemUseAnimation` id, the sound
/// (a `minecraft:sound_event` registry id or an inline event) and the
/// particle flag. `on_consume_effects` is not carried (server-side only).
#[derive(Clone, Debug, PartialEq)]
pub struct Consumable {
    /// `consume_seconds`: the use duration in seconds (`consume_ticks` is
    /// `consume_seconds * 20`).
    pub consume_seconds: f32,
    /// `ItemUseAnimation` id (`eat` by default).
    pub animation: i32,
    /// The sound the use emits (`Holder<SoundEvent>`: registry id or inline).
    pub sound: IdOr<SoundEvent>,
    /// `has_consume_particles`: whether the use spawns item particles.
    pub has_consume_particles: bool,
}

/// `rarity`: vanilla's four values; ids are the wire's VarInt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rarity {
    /// `common`.
    Common,
    /// `uncommon`.
    Uncommon,
    /// `rare`.
    Rare,
    /// `epic`.
    Epic,
    /// An id outside 0..4 (a newer version or a mod-added rarity).
    Unknown(i32),
}

impl Rarity {
    fn from_id(id: i32) -> Self {
        match id {
            0 => Rarity::Common,
            1 => Rarity::Uncommon,
            2 => Rarity::Rare,
            3 => Rarity::Epic,
            other => Rarity::Unknown(other),
        }
    }
}

#[cfg(test)]
mod tests;
