//! Whether an item stack draws the enchantment glint (vanilla `ItemStack.hasFoil`) and with
//! which foil type (`ItemStackRenderState.FoilType`).
//!
//! Vanilla's answer is the stack's `minecraft:enchantment_glint_override` when it has one,
//! else the item's `Item.isFoil`: the stack carries a non-empty `minecraft:enchantments`
//! (`ItemStack.isEnchanted`; `stored_enchantments` does not count, which is why the enchanted
//! book carries the override as a default instead), and for the compass (`CompassItem.isFoil`)
//! also a `minecraft:lodestone_tracker`. A stack's components are its item's defaults (from
//! the components report) with the stack's patch on top; the patch side is decoded by the
//! caller (it needs the protocol's component layouts) and handed in as a [`FoilPatch`].

use haste_core::model::FoilType;
use haste_core::RegistryIndex;

use super::ItemComponents;

/// What a stack's component patch does to one default component.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Patched<T> {
    /// The patch leaves the item's default in place.
    #[default]
    Unchanged,
    /// The patch sets this value.
    Set(T),
    /// The patch removes the component (`DataComponentPatch`'s removed types).
    Removed,
}

/// The components `ItemStack.hasFoil` reads, as a stack's component patch changes them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FoilPatch {
    /// `minecraft:enchantment_glint_override`.
    pub glint_override: Patched<bool>,
    /// `minecraft:enchantments`: `Set(true)` when the patch's list is non-empty.
    pub enchantments: Patched<bool>,
    /// `minecraft:lodestone_tracker` (its value does not matter, only its presence).
    pub lodestone_tracker: Patched<()>,
}

/// An item's default values of the components `ItemStack.hasFoil` reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FoilDefaults {
    /// The default `minecraft:enchantment_glint_override` (the enchanted golden apple, the
    /// nether star, the enchanted book, ...).
    pub(crate) glint_override: Option<bool>,
    /// The default `minecraft:enchantments` is non-empty.
    pub(crate) enchanted: bool,
    /// The default components carry `minecraft:lodestone_tracker`.
    pub(crate) lodestone_tracker: bool,
}

impl FoilDefaults {
    /// Reads the defaults from an item's `components` object in the components report.
    pub(crate) fn parse(components: &serde_json::Value) -> FoilDefaults {
        FoilDefaults {
            glint_override: components
                .get("minecraft:enchantment_glint_override")
                .and_then(|v| v.as_bool()),
            enchanted: components
                .get("minecraft:enchantments")
                .and_then(|v| v.as_object())
                .is_some_and(|m| !m.is_empty()),
            lodestone_tracker: components.get("minecraft:lodestone_tracker").is_some(),
        }
    }
}

impl ItemComponents {
    /// `ItemStack.hasFoil` for a stack of `item` whose patch does `patch`.
    pub fn has_foil(&self, item: RegistryIndex, patch: &FoilPatch) -> bool {
        let defaults = self
            .foil_defaults
            .get(item.0 as usize)
            .copied()
            .unwrap_or_default();
        let glint_override = match patch.glint_override {
            Patched::Set(v) => Some(v),
            Patched::Removed => None,
            Patched::Unchanged => defaults.glint_override,
        };
        if let Some(v) = glint_override {
            return v;
        }
        // `Item.isFoil` -> `ItemStack.isEnchanted`: `getOrDefault(ENCHANTMENTS, EMPTY)`.
        let enchanted = match patch.enchantments {
            Patched::Set(v) => v,
            Patched::Removed => false,
            Patched::Unchanged => defaults.enchanted,
        };
        // `CompassItem.isFoil`: a lodestone compass glints too.
        if Some(item) == self.compass {
            let lodestone = match patch.lodestone_tracker {
                Patched::Set(()) => true,
                Patched::Removed => false,
                Patched::Unchanged => defaults.lodestone_tracker,
            };
            return lodestone || enchanted;
        }
        enchanted
    }

    /// The foil type an item model gives the stack (`CuboidItemModelWrapper.update`): none
    /// without foil, `Special` when `special_animated` (the item is in `#minecraft:compasses`
    /// or is the clock, `hasSpecialAnimatedTexture`; the caller resolves the tag), else
    /// `Standard`.
    pub fn foil_type(
        &self,
        item: RegistryIndex,
        patch: &FoilPatch,
        special_animated: bool,
    ) -> FoilType {
        if !self.has_foil(item, patch) {
            FoilType::None
        } else if special_animated {
            FoilType::Special
        } else {
            FoilType::Standard
        }
    }

    /// Sets item `i`'s default foil components (tests and tools).
    pub fn insert_foil_defaults(
        &mut self,
        i: RegistryIndex,
        glint_override: Option<bool>,
        enchanted: bool,
    ) {
        let at = i.0 as usize;
        if self.foil_defaults.len() <= at {
            self.foil_defaults.resize(at + 1, FoilDefaults::default());
        }
        self.foil_defaults[at] = FoilDefaults {
            glint_override,
            enchanted,
            lodestone_tracker: false,
        };
    }

    /// Marks item `i` as the compass (`CompassItem`; tests and tools).
    pub fn set_compass(&mut self, i: RegistryIndex) {
        self.compass = Some(i);
    }
}
