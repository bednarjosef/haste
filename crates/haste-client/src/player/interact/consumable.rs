//! The `minecraft:consumable` of a stack (player workstream): vanilla
//! `ItemStack.get(DataComponents.CONSUMABLE)` — the stack's own component patch first
//! (an added component overrides the item's default, a removed one makes the stack not
//! consumable), else the item's default from the data generator's reports. A command or
//! a mod can patch any stack, so the item default alone is not enough: an edible stick,
//! a stick with a custom sound, or bread with its consumable removed.
//!
//! The patch bytes are decoded through `haste_protocol::wire::components::read_patch`,
//! which knows every component's layout by registry name; a patch that does not decode
//! (an unknown component layout) falls back to the item default and logs, never fails.

use haste_core::entity::ItemStackData;
use haste_core::{Identifier, RegistryIndex};
use haste_protocol::wire::codecs::IdOr;

use super::Interaction;

impl Interaction {
    /// The stack's effective `minecraft:consumable` (`ItemStack.get`): the component the
    /// stack's patch adds, else the item's default; `None` when the patch removes it or
    /// neither names one.
    pub(super) fn stack_consumable(
        &self,
        stack: &ItemStackData,
        data: &haste_data::GameData,
    ) -> Option<haste_data::Consumable> {
        let name = stack
            .item
            .and_then(|i| data.registries.name("minecraft:item", i))?;
        if let Some(tables) = &self.component_tables {
            if !stack.components_raw.is_empty() {
                match haste_protocol::wire::components::read_patch(&stack.components_raw, tables) {
                    Ok(patch) => {
                        if patch.removes_consumable() {
                            return None;
                        }
                        if let Some(c) = patch.consumable() {
                            return Some(from_wire(c, data));
                        }
                    }
                    Err(e) => tracing::debug!("stack component patch did not decode: {e}"),
                }
            }
        }
        self.item_components.as_ref()?.consumable(name)
    }
}

/// A wire `minecraft:consumable` as the game logic's `Consumable`: the sound becomes its
/// id — the `minecraft:sound_event` registry name for a registry holder, the inline
/// event's own name otherwise (the client plays sounds by id; the inline event's fixed
/// range is the sound engine's, like the piercing-weapon sounds). A registry id the
/// report does not know plays nothing.
fn from_wire(
    c: &haste_protocol::wire::components::Consumable,
    data: &haste_data::GameData,
) -> haste_data::Consumable {
    let sound = match &c.sound {
        IdOr::Id(id) => data
            .registries
            .name("minecraft:sound_event", RegistryIndex(*id as u32))
            .cloned()
            .unwrap_or_else(|| Identifier::minecraft("intentionally_empty")),
        IdOr::Inline(event) => event.name.clone(),
    };
    haste_data::Consumable {
        consume_seconds: c.consume_seconds,
        animation: c.animation,
        sound,
        has_consume_particles: c.has_consume_particles,
    }
}
