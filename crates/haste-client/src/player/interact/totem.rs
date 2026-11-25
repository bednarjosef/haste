//! The stack a totem pop shows (vanilla `ClientPacketListener.findTotem`): the first hand, main
//! then off, whose stack has `DataComponents.DEATH_PROTECTION`, else a fresh totem of undying.
//!
//! `ItemStack.has` reads the stack's own component patch first (a command or a mod can add
//! death protection to any stack, or remove it from a totem), then the item's default
//! components from the data generator's reports. A patch that does not decode falls back to
//! the item's default and logs, never fails.

use haste_core::entity::ItemStackData;
use haste_core::{Identifier, RegistryIndex};
use haste_protocol::wire::components::{read_patch, ItemComponent};

use super::Interaction;

/// `DataComponents.DEATH_PROTECTION`'s registry name.
const DEATH_PROTECTION: &str = "minecraft:death_protection";

impl Interaction {
    /// Resolves `minecraft:totem_of_undying` and the items whose default components carry
    /// death protection as `minecraft:item` indices, once, with the other item lookups in
    /// `prepare` (after the item component reports are loaded).
    pub(super) fn resolve_totem(&mut self, data: &haste_data::GameData) {
        if self.totem_item.is_none() {
            self.totem_item = data
                .registries
                .index("minecraft:item", &Identifier::minecraft("totem_of_undying"));
        }
        if self.death_protection_items.is_none() {
            if let Some(components) = &self.item_components {
                let items = (0..data.registries.len("minecraft:item") as u32)
                    .map(RegistryIndex)
                    .filter(|&i| {
                        data.registries
                            .name("minecraft:item", i)
                            .is_some_and(|n| components.has_death_protection(n))
                    })
                    .collect();
                self.death_protection_items = Some(items);
            }
        }
    }

    /// Vanilla `ClientPacketListener.findTotem`: the main-hand stack when it carries death
    /// protection, else the off-hand stack when it does, else a new totem of undying (count 1,
    /// no patch). `main` is the held main-hand stack (`LocalPlayer::main_hand`).
    pub fn find_totem(&self, main: &ItemStackData) -> ItemStackData {
        [main, &self.offhand]
            .into_iter()
            .find(|s| self.has_death_protection(s))
            .cloned()
            .unwrap_or_else(|| ItemStackData {
                count: 1,
                item: self.totem_item,
                components_raw: Default::default(),
            })
    }

    /// `ItemStack.has(DataComponents.DEATH_PROTECTION)`: the patch's removal or addition wins,
    /// else the item's default components.
    pub(super) fn has_death_protection(&self, stack: &ItemStackData) -> bool {
        let Some(item) = stack.item.filter(|_| !stack.is_empty()) else {
            return false;
        };
        if let (Some(tables), false) = (&self.component_tables, stack.components_raw.is_empty()) {
            match read_patch(&stack.components_raw, tables) {
                Ok(patch) => {
                    if patch.removed.iter().any(|n| n == DEATH_PROTECTION) {
                        return false;
                    }
                    if patch.added.iter().any(|c| {
                        matches!(c, ItemComponent::Other { name, .. } if name == DEATH_PROTECTION)
                    }) {
                        return true;
                    }
                }
                Err(e) => tracing::debug!("stack component patch did not decode: {e}"),
            }
        }
        self.death_protection_items
            .as_ref()
            .is_some_and(|items| items.contains(&item))
    }
}
