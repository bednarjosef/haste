//! Map item data packets (`ClientPacketListener.handleMapItemData`): each map's pixels and
//! decorations, stored per map id with the partial updates applied, as
//! `MapItemSavedData.createForClient` + `ClientboundMapItemDataPacket.applyToMap` fill it.
//! The stored data is the renderer's own [`haste_render::MapData`] (versioned), so the
//! per-tick publish hands it over as one clone.

use haste_core::entity::ItemStackData;
use haste_protocol::packets::play::clientbound::MapItemData;
use haste_protocol::packets::ProtocolTables;
use haste_protocol::wire::components::{read_patch, ItemComponent};

use super::Session;

impl Session {
    /// `handleMapItemData`: a map we have not seen starts empty at the packet's scale
    /// (`createForClient`, all colours `NONE`), then the packet's decorations replace the
    /// old set (`addClientSideDecorations`) and its colour patch overwrites its rectangle
    /// (`MapPatch.applyToMap`). The scale and locked flag only seed a new map, like vanilla.
    pub(super) fn apply_map_item_data(&mut self, m: &MapItemData) {
        let map = self
            .maps
            .entry(m.map_id)
            .or_insert_with(|| haste_render::MapData {
                scale: m.scale,
                locked: m.locked,
                version: 0,
                colors: vec![0; 128 * 128],
                decorations: Vec::new(),
            });
        let mut changed = false;
        if let Some(icons) = &m.icons {
            // `addClientSideDecorations`: the packet's list is the whole set.
            map.decorations = icons
                .iter()
                .filter(|i| i.kind >= 0)
                .map(|i| haste_render::MapMarker {
                    kind: i.kind as u32,
                    x: i.x,
                    y: i.z,
                    rot: (i.direction & 15) as u8,
                    name: i.name.as_ref().map(|c| c.to_plain(&|_| None)),
                })
                .collect();
            changed = true;
        }
        if let Some(patch) = &m.patch {
            // `MapPatch.applyToMap`, bounds-guarded so a bad patch writes nothing past the
            // grid (never a panic).
            let (cols, rows) = (patch.columns as usize, patch.rows as usize);
            let (x0, y0) = (patch.x as usize, patch.z as usize);
            if cols > 0
                && rows > 0
                && patch.data.len() >= cols * rows
                && x0 + cols <= 128
                && y0 + rows <= 128
            {
                for y in 0..rows {
                    for x in 0..cols {
                        map.colors[x0 + x + (y0 + y) * 128] = patch.data[x + y * cols];
                    }
                }
                changed = true;
            }
        }
        if changed {
            map.version = map.version.wrapping_add(1).max(1);
            self.maps_version = self.maps_version.wrapping_add(1);
        }
    }
}

/// The `minecraft:map_id` component of a stack (`ItemStack.get(DataComponents.MAP_ID)`),
/// decoded from the raw component patch; `None` without one.
pub(crate) fn map_id_of(stack: &ItemStackData, tables: &ProtocolTables) -> Option<i32> {
    if stack.is_empty() || stack.components_raw.is_empty() {
        return None;
    }
    let patch = read_patch(&stack.components_raw, tables).ok()?;
    patch.added.iter().find_map(|c| match c {
        ItemComponent::Other { name, raw } if name == "minecraft:map_id" => {
            haste_protocol::wire::Reader::new(raw).var_int().ok()
        }
        _ => None,
    })
}
