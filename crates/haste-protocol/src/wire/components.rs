//! Typed item data components: reading a stack's component patch.
//!
//! `ItemStackData::components_raw` keeps a stack's patch exactly as it is on
//! the wire, where component data carries no length prefix: skipping a
//! component needs its layout, which `slot.rs` knows by registry name
//! (resolved through `minecraft:data_component_type` in
//! [`ProtocolTables`]). [`read_patch`] walks that slice once and decodes the
//! components game logic asks about (how damaged a tool is, its durability,
//! its enchantments, its custom name, what it mines); every other component
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

use super::slot::{component_name, skip_component};
use super::{DecodeError, DecodeResult, IdSet, Reader, Wire, MAX_LIST};
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
mod tests {
    use crate::wire::Writer;

    use super::*;
    use haste_core::Identifier;

    /// A patch body: the add/remove counts, the adds as (type id, data) and
    /// the removes as type ids.
    fn patch(components: &[(i32, &[u8])], removed: &[i32]) -> Vec<u8> {
        let mut w = Writer::new();
        w.len_prefix(components.len());
        w.len_prefix(removed.len());
        for (ty, data) in components {
            w.var_int(*ty);
            w.bytes(data);
        }
        for ty in removed {
            w.var_int(*ty);
        }
        w.into_inner()
    }

    /// The type ids used by these tests, with the names the tables resolve
    /// them to.
    fn tables() -> ProtocolTables {
        ProtocolTables::for_tests(&[
            (1, "minecraft:max_stack_size"),
            (2, "minecraft:max_damage"),
            (3, "minecraft:damage"),
            (4, "minecraft:unbreakable"),
            (5, "minecraft:repair_cost"),
            (6, "minecraft:custom_name"),
            (7, "minecraft:item_name"),
            (8, "minecraft:enchantments"),
            (9, "minecraft:stored_enchantments"),
            (10, "minecraft:tool"),
            (11, "minecraft:food"),
            (12, "minecraft:rarity"),
            (13, "minecraft:mystery"),
        ])
    }

    fn text(w: &mut Writer, s: &str) {
        w.component(&Component::text(s));
    }

    #[test]
    fn every_typed_component_reads_its_value() {
        let tables = tables();
        let mut tool = Writer::new();
        w_tool(&mut tool);
        let mut name = Writer::new();
        text(&mut name, "Killing Blow");
        let mut item_name = Writer::new();
        text(&mut item_name, "Iron Block");
        let body = patch(
            &[
                (3, &[5]),
                (2, &[0x99, 0x0C]),
                (1, &[1]),
                (4, &[]),
                (5, &[3]),
                (6, &name.buf),
                (7, &item_name.buf),
                (8, &[1, 8, 3]), // sharpness (id 8), level 3
                // id 0 is a valid enchantment (the first in the registry)
                (9, &[2, 0, 1, 39, 5]),
                (10, &tool.buf),
                (12, &[2]),
            ],
            &[],
        );
        let parsed = read_patch(&body, &tables).unwrap();
        assert_eq!(parsed.damage(), Some(5));
        assert_eq!(parsed.max_damage(), Some(1561));
        assert_eq!(parsed.max_stack_size(), Some(1));
        assert!(parsed.unbreakable());
        assert_eq!(parsed.repair_cost(), Some(3));
        assert_eq!(parsed.custom_name(), Some(&Component::text("Killing Blow")));
        assert_eq!(parsed.item_name(), Some(&Component::text("Iron Block")));
        assert_eq!(
            parsed.enchantments(),
            Some(&[EnchantmentLevel { id: 8, level: 3 }][..])
        );
        assert_eq!(
            parsed.stored_enchantments(),
            Some(
                &[
                    EnchantmentLevel { id: 0, level: 1 },
                    EnchantmentLevel { id: 39, level: 5 }
                ][..]
            )
        );
        let rules = &parsed.tool().unwrap().rules;
        assert_eq!(
            rules[0].blocks,
            IdSet::Tag(Identifier::minecraft("mineable/axe"))
        );
        assert_eq!(rules[0].speed, Some(12.0));
        assert_eq!(rules[0].correct_for_drops, Some(true));
        assert_eq!(rules[1].blocks, IdSet::Ids(vec![17, 18]));
        assert_eq!(rules[1].speed, None);
        assert_eq!(rules[1].correct_for_drops, None);
        assert_eq!(parsed.tool().unwrap().default_mining_speed, 1.0);
        assert_eq!(parsed.tool().unwrap().damage_per_block, 2);
        assert!(parsed.tool().unwrap().can_destroy_blocks_in_creative);
        assert_eq!(parsed.rarity(), Some(Rarity::Rare));
    }

    /// An axe: two rules, one with a speed and drop override, one bare.
    fn w_tool(w: &mut Writer) {
        w.var_int(2);
        w.write(&IdSet::Tag(Identifier::minecraft("mineable/axe")));
        w.option(&Some(12.0f32), |w, v| w.f32(*v));
        w.option(&Some(true), |w, v| w.bool(*v));
        w.write(&IdSet::Ids(vec![17, 18]));
        w.bool(false);
        w.bool(false);
        w.f32(1.0);
        w.var_int(2);
        w.bool(true);
    }

    #[test]
    fn other_components_keep_their_bytes() {
        let tables = tables();
        let food = [4u8, 0, 0, 32, 64, 0]; // nutrition 4, saturation 2.5, always edible
        let body = patch(&[(11, &food)], &[]);
        let parsed = read_patch(&body, &tables).unwrap();
        assert_eq!(
            parsed.added,
            vec![ItemComponent::Other {
                name: "minecraft:food".to_string(),
                raw: Bytes::copy_from_slice(&food),
            }]
        );
        assert_eq!(parsed.damage(), None);
        assert_eq!(parsed.tool(), None);
        assert_eq!(parsed.enchantments(), None);
    }

    #[test]
    fn removals_name_the_removed_types() {
        let tables = tables();
        let body = patch(&[(3, &[7])], &[4, 12]);
        let parsed = read_patch(&body, &tables).unwrap();
        assert_eq!(parsed.damage(), Some(7));
        assert_eq!(
            parsed.removed,
            vec!["minecraft:unbreakable", "minecraft:rarity"]
        );
    }

    #[test]
    fn empty_patch_is_empty_and_exact() {
        let tables = tables();
        let parsed = read_patch(&[0, 0], &tables).unwrap();
        assert_eq!(parsed, ComponentPatch::default());
        assert!(parsed.added.is_empty() && parsed.removed.is_empty());
    }

    /// Every typed variant consumes exactly its bytes and no more.
    #[test]
    fn typed_components_consume_exactly_their_bytes() {
        let tables = tables();
        let mut tool = Writer::new();
        w_tool(&mut tool);
        let mut name = Writer::new();
        text(&mut name, "n");
        let mut enchantments = Writer::new();
        enchantments.var_int(1);
        enchantments.var_int(9);
        enchantments.var_int(3);
        let samples: &[(&str, &[u8])] = &[
            ("minecraft:max_stack_size", &[16]),
            ("minecraft:max_damage", &[0x99, 0x0C]),
            ("minecraft:damage", &[5]),
            ("minecraft:unbreakable", &[]),
            ("minecraft:repair_cost", &[3]),
            ("minecraft:rarity", &[3]),
            ("minecraft:custom_name", &name.buf),
            ("minecraft:item_name", &name.buf),
            ("minecraft:enchantments", &enchantments.buf),
            ("minecraft:stored_enchantments", &enchantments.buf),
            ("minecraft:tool", &tool.buf),
            // Not modelled, but a known layout: kept raw.
            ("minecraft:food", &[4, 0, 0, 32, 64, 0]),
        ];
        for (name, bytes) in samples {
            let mut wire = bytes.to_vec();
            wire.push(0xAB);
            let mut r = Reader::new(&wire).with_tables(&tables);
            read_component(&mut r, name).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(
                r.peek_rest(),
                &[0xAB],
                "{name} did not consume exactly its bytes"
            );
        }
    }

    #[test]
    fn unknown_layouts_and_ids_fail() {
        let tables = tables();
        // A component whose layout is unknown cannot be kept raw.
        let body = patch(&[(13, &[1])], &[]);
        assert!(read_patch(&body, &tables).is_err());
        // A type id the tables do not know.
        let body = patch(&[(99, &[])], &[]);
        assert!(read_patch(&body, &tables).is_err());
        // Trailing bytes after the patch are a layout mismatch.
        let mut body = patch(&[(3, &[1])], &[]);
        body.push(0);
        assert!(read_patch(&body, &tables).is_err());
        // A truncated enchantment list (count 2, one entry).
        let body = patch(&[(8, &[2, 0, 3])], &[]);
        assert!(read_patch(&body, &tables).is_err());
    }
}
