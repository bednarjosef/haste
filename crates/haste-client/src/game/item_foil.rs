//! `ItemStack.hasFoil` for the client's own stacks (the held item, item entities, framed and
//! worn items): the stack's component patch decoded for the components the decision reads,
//! with the item's defaults applied by `haste-data` (`ItemComponents::foil_type`).
//!
//! The GUI does not come through here: its stacks are real vanilla `ItemStack`s in Java,
//! which answer `hasFoil` themselves (`GfxImpl.foil`).

use haste_core::entity::ItemStackData;
use haste_core::model::FoilType;
use haste_data::{FoilPatch, ItemComponents, Patched};
use haste_protocol::packets::ProtocolTables;
use haste_protocol::wire::components::{read_patch, ComponentPatch, ItemComponent};

/// The data component names `hasFoil` reads.
const GLINT_OVERRIDE: &str = "minecraft:enchantment_glint_override";
const ENCHANTMENTS: &str = "minecraft:enchantments";
const LODESTONE_TRACKER: &str = "minecraft:lodestone_tracker";

/// What a decoded component patch does to the components `hasFoil` reads.
pub(crate) fn foil_patch(patch: &ComponentPatch) -> FoilPatch {
    let mut out = FoilPatch::default();
    for c in &patch.added {
        match c {
            ItemComponent::Enchantments(list) => {
                out.enchantments = Patched::Set(!list.is_empty());
            }
            ItemComponent::Other { name, raw } if name == GLINT_OVERRIDE => {
                // `ByteBufCodecs.BOOL`: one byte.
                out.glint_override = Patched::Set(raw.first().is_some_and(|b| *b != 0));
            }
            ItemComponent::Other { name, .. } if name == LODESTONE_TRACKER => {
                out.lodestone_tracker = Patched::Set(());
            }
            _ => {}
        }
    }
    for name in &patch.removed {
        match name.as_str() {
            GLINT_OVERRIDE => out.glint_override = Patched::Removed,
            ENCHANTMENTS => out.enchantments = Patched::Removed,
            LODESTONE_TRACKER => out.lodestone_tracker = Patched::Removed,
            _ => {}
        }
    }
    out
}

/// The stack's foil type. A patch that does not decode counts as no patch (the stack's
/// item defaults still decide). The special foil of the compasses and the clock needs the
/// item tags, which the render thread does not hold: those draw the standard glint.
pub(crate) fn stack_foil(
    stack: &ItemStackData,
    tables: &ProtocolTables,
    components: &ItemComponents,
) -> FoilType {
    let Some(item) = stack.item.filter(|_| !stack.is_empty()) else {
        return FoilType::None;
    };
    let patch = if stack.components_raw.is_empty() {
        FoilPatch::default()
    } else {
        match read_patch(&stack.components_raw, tables) {
            Ok(p) => foil_patch(&p),
            Err(e) => {
                tracing::debug!("stack component patch did not decode for its foil: {e}");
                FoilPatch::default()
            }
        }
    };
    components.foil_type(item, &patch, false)
}

#[cfg(test)]
mod tests;
