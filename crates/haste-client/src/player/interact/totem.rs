//! The stack a totem pop shows (vanilla `ClientPacketListener.findTotem`): the first hand, main
//! then off, that holds a stack with `DataComponents.DEATH_PROTECTION`, else a fresh totem of
//! undying.

use haste_core::entity::ItemStackData;
use haste_core::Identifier;

use super::Interaction;

impl Interaction {
    /// Resolves `minecraft:totem_of_undying` in `minecraft:item` once, with the other item
    /// lookups in `prepare`.
    pub(super) fn resolve_totem(&mut self, data: &haste_data::GameData) {
        if self.totem_item.is_none() {
            self.totem_item = data
                .registries
                .index("minecraft:item", &Identifier::minecraft("totem_of_undying"));
        }
    }

    /// Vanilla `ClientPacketListener.findTotem`: the main-hand stack when it carries death
    /// protection, else the off-hand stack, else a new totem of undying (count 1, default
    /// components). A stack carries death protection when it is a totem: the vanilla totem
    /// item's default `death_protection` component, which only totems carry in 26.3.
    /// `main` is the held main-hand stack (`LocalPlayer::main_hand`).
    pub fn find_totem(&self, main: &ItemStackData) -> ItemStackData {
        [main, &self.offhand]
            .into_iter()
            .find(|s| self.carries_death_protection(s))
            .cloned()
            .unwrap_or_else(|| ItemStackData {
                count: 1,
                item: self.totem_item,
                components_raw: Default::default(),
            })
    }

    /// `ItemStack.has(DataComponents.DEATH_PROTECTION)`, approximated by the totem item.
    fn carries_death_protection(&self, stack: &ItemStackData) -> bool {
        stack.item.is_some() && stack.item == self.totem_item
    }
}
