//! The held item's decoded components (player workstream): the main-hand stack's
//! `minecraft:tool` / Efficiency / `attack_speed` data, decoded once per stack change
//! (`ItemStack.get(DataComponents.*)`), and the swap key the first-person equip compares
//! (`ItemStack.matchesIgnoringComponents`).

use haste_core::entity::ItemStackData;
use haste_core::{Identifier, RegistryIndex};

use super::super::LocalPlayer;
use super::Interaction;

/// The held main-hand stack's decoded components the game logic reads: what it mines
/// (`minecraft:tool`), the Efficiency enchantment's `mining_efficiency` contribution
/// (`Enchantments.EFFICIENCY`'s `LevelsSquared(1.0)` effect = level^2 + 1), and the
/// `minecraft:attack_speed` its attribute modifiers grant.
#[derive(Clone, Debug)]
pub(super) struct HeldItem {
    /// The stack this was decoded from (re-decode when it changes).
    pub(super) stack: ItemStackData,
    /// The stack's own `minecraft:tool` component, if it overrides the item default.
    pub(super) tool: Option<haste_physics::mining::Tool>,
    /// `minecraft:mining_efficiency` from the stack's Efficiency enchantment (0 if none).
    pub(super) efficiency: f64,
    /// The player's `minecraft:attack_speed` with this item held
    /// (`Player.getCurrentItemAttackStrengthDelay`).
    pub(super) attack_speed: f64,
    /// [`swap_key`] of this stack, for the first-person equip.
    pub(super) swap_key: u64,
}

/// A key over what `ItemStack.matchesIgnoringComponents(a, b, DataComponentType::
/// ignoreSwapAnimation)` compares beyond the item type: the count, which component types the
/// stack's patch adds and removes, and the values of all but the ignored types (`damage`,
/// `max_damage`, `max_stack_size`: those count only by presence). Equal keys mean the hand
/// swaps the shown stack without the dip.
pub(crate) fn swap_key(
    stack: &ItemStackData,
    patch: &haste_protocol::wire::components::ComponentPatch,
) -> u64 {
    use haste_protocol::wire::components::ItemComponent;
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    stack.count.hash(&mut h);
    for c in &patch.added {
        match c {
            ItemComponent::Damage(_) => "damage".hash(&mut h),
            ItemComponent::MaxDamage(_) => "max_damage".hash(&mut h),
            ItemComponent::MaxStackSize(_) => "max_stack_size".hash(&mut h),
            other => format!("{other:?}").hash(&mut h),
        }
    }
    patch.removed.hash(&mut h);
    h.finish()
}

impl Interaction {
    /// Decodes the held main-hand stack's component patch once, when the slot's stack changes
    /// (`ItemStack.get(DataComponents.TOOL)` / the Efficiency enchantment's attribute effect).
    pub(super) fn refresh_components(&mut self, stack: ItemStackData, data: &haste_data::GameData) {
        if self.components.as_ref().is_some_and(|h| h.stack == stack) {
            return;
        }
        let Some(tables) = self.component_tables.clone() else {
            return;
        };
        let patch = if stack.is_empty() || stack.components_raw.is_empty() {
            haste_protocol::wire::components::ComponentPatch::default()
        } else {
            match haste_protocol::wire::components::read_patch(&stack.components_raw, &tables) {
                Ok(p) => p,
                Err(e) => {
                    tracing::debug!("held stack component patch did not decode: {e}");
                    self.components = None;
                    return;
                }
            }
        };
        let tool = patch.tool().map(|t| tool_from_component(t, data));
        let efficiency = efficiency_from_enchantments(&patch, data);
        let attack_speed = self.item_attack_speed(&stack, data);
        let swap_key = swap_key(&stack, &patch);
        tracing::debug!(
            "held item components: attack_speed {attack_speed} ({} delay ticks), efficiency {efficiency}",
            if attack_speed > 0.0 { 20.0 / attack_speed } else { 20.0 / 4.0 }
        );
        self.components = Some(HeldItem {
            stack,
            tool,
            efficiency,
            attack_speed,
            swap_key,
        });
    }

    /// The player's `minecraft:attack_speed` with `stack` held: the base 4.0 plus the item's
    /// main-hand `attack_speed` attribute modifiers, as `ItemAttributeModifiers.compute` does.
    /// Falls back to the server-synced attribute for an item without any such modifier.
    fn item_attack_speed(&self, stack: &ItemStackData, data: &haste_data::GameData) -> f64 {
        let fallback = self.attrs.attack_speed;
        let Some(item) = stack
            .item
            .and_then(|i| data.registries.name("minecraft:item", i))
        else {
            return fallback;
        };
        let attr = Identifier::minecraft("attack_speed");
        let Some(ic) = &self.item_components else {
            return fallback;
        };
        if ic
            .attribute_modifiers(item)
            .iter()
            .any(|m| m.attribute == attr && m.applies_to_main_hand())
        {
            ic.main_hand_attribute(item, &attr, crate::hand::DEFAULT_ATTACK_SPEED)
        } else {
            fallback
        }
    }
}

impl LocalPlayer {
    /// The main-hand stack as the first-person equip compares it
    /// (`FirstPersonHandsAndItems.shouldInstantlyReplaceVisibleItem`): its type and the swap key
    /// of its decoded components, or of the raw patch when it has not been decoded.
    pub fn held_stack(&self) -> crate::hand::HeldStack {
        let stack = self.main_hand();
        let swap_key = match self.int.components.as_ref() {
            Some(h) if h.stack == *stack => h.swap_key,
            _ => {
                use std::hash::{Hash, Hasher};
                let mut h = std::hash::DefaultHasher::new();
                stack.count.hash(&mut h);
                stack.components_raw.hash(&mut h);
                h.finish()
            }
        };
        crate::hand::HeldStack {
            item: stack.item,
            swap_key,
        }
    }

    /// The player's `minecraft:attack_speed` with the held item (base 4.0 plus the item's
    /// attribute modifiers; the server-synced attribute when the item has none). Used by
    /// `Player.getCurrentItemAttackStrengthDelay` for the equip and the attack indicator.
    pub fn held_attack_speed(&self) -> f64 {
        let stack = self.main_hand();
        self.int
            .components
            .as_ref()
            .filter(|h| h.stack == *stack)
            .map_or(self.int.attrs.attack_speed, |h| h.attack_speed)
    }
}

/// Converts a stack's `minecraft:tool` component (protocol `IdSet`s) into the physics tool the
/// mining code matches against block tags and names.
fn tool_from_component(
    t: &haste_protocol::wire::components::Tool,
    data: &haste_data::GameData,
) -> haste_physics::mining::Tool {
    use haste_physics::mining::{BlockSet, ToolRule};
    use haste_protocol::wire::IdSet;
    let rules = t
        .rules
        .iter()
        .map(|r| {
            let blocks = match &r.blocks {
                IdSet::Tag(tag) => BlockSet::Tag(tag.clone()),
                IdSet::Ids(ids) => BlockSet::List(
                    ids.iter()
                        .filter_map(|i| {
                            data.registries
                                .name("minecraft:block", RegistryIndex(*i as u32))
                                .cloned()
                        })
                        .collect(),
                ),
            };
            ToolRule {
                blocks,
                speed: r.speed,
                correct_for_drops: r.correct_for_drops,
            }
        })
        .collect();
    haste_physics::mining::Tool {
        rules,
        default_mining_speed: t.default_mining_speed,
        damage_per_block: t.damage_per_block,
        can_destroy_blocks_in_creative: t.can_destroy_blocks_in_creative,
    }
}

/// The `minecraft:mining_efficiency` a stack's Efficiency enchantment grants
/// (`Enchantments.EFFICIENCY`, `LevelsSquared(1.0)` = level^2 + 1; ids from
/// `minecraft:enchantment`).
fn efficiency_from_enchantments(
    patch: &haste_protocol::wire::components::ComponentPatch,
    data: &haste_data::GameData,
) -> f64 {
    let Some(ench) = patch.enchantments() else {
        return 0.0;
    };
    ench.iter()
        .filter_map(|e| {
            let name = data
                .registries
                .name("minecraft:enchantment", RegistryIndex(e.id as u32))?;
            (name.namespace() == "minecraft" && name.path() == "efficiency").then(|| {
                let l = f64::from(e.level);
                l * l + 1.0
            })
        })
        .sum()
}
