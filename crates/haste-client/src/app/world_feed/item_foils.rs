//! The glint of the items the frame's entities show (`ItemStack.hasFoil` for a dropped
//! item, a framed item, the held items and the worn equipment), filled onto the instances the
//! entity store built. The store keeps the stacks but no component tables, so the decode lives
//! here on the render thread, like the frame maps: the stacks are compared per frame and a
//! patch is decoded only when its stack changed.

use haste_core::entity::{ItemStackData, MetadataValue};
use haste_core::model::{Decoration, EntityInstance, EquipmentSlot, FoilType};
use haste_protocol::haste_core::EntityId;

use crate::game::Shared;

use super::WorldFeed;

/// The cache key of a dropped item's stack (the equipment uses the wire slots 0..=7).
const DROPPED_ITEM: u8 = 200;
/// The cache key of a framed item's stack.
const FRAMED_ITEM: u8 = 201;

impl WorldFeed {
    /// Fills every instance's foils from its entity's stacks: `local` is the player's own
    /// instance, whose stacks live in the shared local model rather than the store.
    pub(super) fn patch_item_foils(&mut self, shared: &Shared, local: EntityId) {
        let store = shared.entities.read();
        let local_model = shared.local_model.lock();
        let cache = &mut self.item_foils;
        let mut foil = |id: EntityId, key: u8, stack: &ItemStackData| -> FoilType {
            if stack.is_empty() {
                return FoilType::None;
            }
            match cache.get(&(id, key)) {
                Some((s, f)) if s == stack => *f,
                _ => {
                    let f = crate::game::item_foil::stack_foil(
                        stack,
                        &shared.component_tables,
                        &shared.data.item_components,
                    );
                    cache.insert((id, key), (stack.clone(), f));
                    f
                }
            }
        };
        for inst in self.instances.iter_mut() {
            let entity = if inst.id == local {
                Some(&*local_model)
            } else {
                store.get(inst.id)
            };
            let Some(entity) = entity else { continue };
            if let Some(dropped) = inst.item.as_mut() {
                if let Some(stack) = &entity.item {
                    dropped.foil = foil(inst.id, DROPPED_ITEM, stack);
                }
                continue;
            }
            if let Some(Decoration::ItemFrame(f)) = inst.decoration.as_mut() {
                let stack = entity
                    .metadata
                    .get(haste_entity::decoration::ITEM_FRAME_ITEM)
                    .and_then(|e| match &e.value {
                        MetadataValue::ItemStack(s) => Some(s),
                        _ => None,
                    });
                f.item_foil = stack.map_or(FoilType::None, |s| foil(inst.id, FRAMED_ITEM, s));
                continue;
            }
            for (wire, stack) in &entity.equipment {
                let Some(slot) = EquipmentSlot::from_wire(*wire) else {
                    continue;
                };
                let f = foil(inst.id, *wire, stack);
                set_slot_foil(inst, slot, f);
            }
        }
        // Forget the stacks of entities that are gone (rarely: only once the cache has grown
        // well past what the frame shows).
        if cache.len() > 64 + 4 * self.instances.len() {
            let live: std::collections::HashSet<EntityId> =
                self.instances.iter().map(|i| i.id).collect();
            cache.retain(|(id, _), _| live.contains(id));
        }
    }
}

/// Puts one equipment slot's foil where the instance draws that slot: the hands, the custom
/// head item, or the worn piece.
fn set_slot_foil(inst: &mut EntityInstance, slot: EquipmentSlot, foil: FoilType) {
    match slot {
        EquipmentSlot::MainHand => inst.hand_foils[0] = foil,
        EquipmentSlot::OffHand => inst.hand_foils[1] = foil,
        _ => {
            if slot == EquipmentSlot::Head && inst.head_item.is_some() {
                inst.head_item_foil = foil;
            }
            for piece in inst.equipment.iter_mut().filter(|p| p.slot == slot) {
                piece.foil = foil;
            }
        }
    }
}
