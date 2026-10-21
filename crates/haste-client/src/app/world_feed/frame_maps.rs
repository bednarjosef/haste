//! The item frames' map ids for the frame (vanilla `ItemFrame.getFramedMapId`), filled from the
//! framed stack's components.

use crate::game::Shared;

use super::WorldFeed;

impl WorldFeed {
    /// Each framed item stack's map id (`ItemFrame.getFramedMapId` reads the stack's
    /// `minecraft:map_id` component): filled onto the item frame instances so the renderer
    /// can draw the map instead of the item. The stack's bytes are compared per frame and
    /// the component patch decoded only when they changed (the entity store keeps no
    /// component tables, so the decode lives here on the render thread).
    pub(super) fn patch_frame_maps(&mut self, shared: &Shared) {
        let tables = &shared.component_tables;
        let store = shared.entities.read();
        for inst in self.instances.iter_mut() {
            let Some(haste_core::model::Decoration::ItemFrame(f)) = inst.decoration.as_mut() else {
                continue;
            };
            if f.item.is_none() {
                f.map_id = None;
                continue;
            }
            let Some(entity) = store.get(inst.id) else {
                f.map_id = None;
                continue;
            };
            let stack = entity
                .metadata
                .get(haste_entity::decoration::ITEM_FRAME_ITEM)
                .and_then(|e| match &e.value {
                    haste_core::entity::MetadataValue::ItemStack(s) if !s.is_empty() => Some(s),
                    _ => None,
                });
            let Some(stack) = stack else {
                f.map_id = None;
                continue;
            };
            if self
                .frame_map_ids
                .get(&inst.id)
                .is_none_or(|(s, _)| *s != *stack)
            {
                let map_id = crate::net::map_id_of(stack, tables);
                self.frame_map_ids.insert(inst.id, (stack.clone(), map_id));
            }
            f.map_id = self.frame_map_ids.get(&inst.id).unwrap().1;
        }
    }
}
