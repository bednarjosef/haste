//! Server packets that touch the interaction state (player workstream): the inventory sync
//! (`ContainerSetContent`/`ContainerSetSlot` for the player inventory menu,
//! `SetPlayerInventory`), the mob-effect add/remove, and `block_changed_ack` that ends our
//! block predictions.

use haste_core::entity::ItemStackData;
use haste_protocol::ClientboundPlay;

use super::super::LocalPlayer;
use super::EffectData;

impl LocalPlayer {
    pub(crate) fn handle_interact_packet(&mut self, p: &ClientboundPlay) -> bool {
        use ClientboundPlay as P;
        let me = self.entity_id.0;
        match p {
            P::BlockChangedAck(a) => {
                tracing::debug!("block_changed_ack {}", a.sequence);
                // `endPredictionsUpTo`: the covered predictions release their server states
                // for the driver's `ClientLevel.syncBlockState` (the restore and the
                // snap-back live there, with the world).
                self.int.prediction.end_predictions_up_to(a.sequence);
            }
            P::ContainerSetContent(c) if c.window_id == 0 => {
                for (i, s) in c.slots.iter().enumerate() {
                    self.set_inventory_menu_slot(i as i32, s);
                }
            }
            P::ContainerSetSlot(c) if c.window_id == 0 => {
                self.set_inventory_menu_slot(c.slot as i32, &c.item)
            }
            P::SetPlayerInventory(s) => self.set_inventory_slot(s.slot, &s.item),
            P::UpdateMobEffect(e) if e.entity_id == me => {
                // Vanilla's blend durations per effect (`MobEffects`): only nausea and
                // darkness set them, everything else blends at once.
                use crate::screen_effects as se;
                let (blend_in, blend_out, blend_out_advance) =
                    if Some(e.effect) == self.int.nausea_id() {
                        (
                            se::NAUSEA_BLEND_IN,
                            se::NAUSEA_BLEND_OUT,
                            se::NAUSEA_BLEND_OUT_ADVANCE,
                        )
                    } else if self.int.darkness_id(e.effect) {
                        (se::DARKNESS_BLEND, se::DARKNESS_BLEND, se::DARKNESS_BLEND)
                    } else {
                        (0, 0, 0)
                    };
                let mut effect = EffectData::new(
                    e.amplifier,
                    e.duration,
                    e.flags & 0x08 != 0,
                    blend_in,
                    blend_out,
                    blend_out_advance,
                );
                // `LivingEntity.forceAddEffect` copies the previous instance's blend state.
                if let Some(prev) = self.int.effects.get(&e.effect) {
                    effect.copy_blend_from(prev);
                }
                self.int.effects.insert(e.effect, effect);
            }
            P::RemoveMobEffect(e) if e.entity_id == me => {
                self.int.effects.remove(&e.effect);
            }
            _ => return false,
        }
        true
    }

    /// Slot of the player inventory menu (window 0): 5..=8 armor (helmet, chest, legs, boots),
    /// 36..=44 hotbar, 45 off hand (wiki "Inventory" slot numbering).
    fn set_inventory_menu_slot(&mut self, slot: i32, item: &ItemStackData) {
        match slot {
            5..=8 => self.int.armor[(slot - 5) as usize] = item.clone(),
            36..=44 => self.int.hotbar[(slot - 36) as usize] = item.clone(),
            45 => self.int.offhand = item.clone(),
            _ => {}
        }
    }

    /// A slot of the player's own inventory in vanilla `Inventory` numbering (0-8 hotbar,
    /// 9-35 main, 36-39 armor feet..head, 40 off hand), as `set_player_inventory` carries
    /// and Java's `InventorySync` mirrors. One path for the server's packets and the
    /// client-side edits vanilla Java code predicts.
    pub fn set_inventory_slot(&mut self, slot: i32, item: &ItemStackData) {
        match slot {
            0..=8 => self.int.hotbar[slot as usize] = item.clone(),
            // `Inventory` indices: 36 feet, 37 legs, 38 chest, 39 head, 40 off hand.
            36..=39 => self.int.armor[(39 - slot) as usize] = item.clone(),
            40 => self.int.offhand = item.clone(),
            // 9..=35 main inventory: not held anywhere yet (no use reads it).
            _ => {}
        }
    }
}
