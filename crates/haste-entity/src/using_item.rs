//! An entity the server drives using an item (vanilla `LivingEntity.useItem` /
//! `useItemRemaining` on the client): the synced living-flags byte starts and stops the
//! use (`LivingEntity.onSyncedDataUpdated` for `DATA_LIVING_ENTITY_FLAGS`), the equipment
//! names the item (`startUsingItem` syncs the held stack before it sets the flag), and
//! the per-tick step (`LivingEntity.tick` -> `updatingUsingItem` -> `updateUsingItem`)
//! advances it and records one [`UseTick`] per tick the client can emit effects for.
//!
//! The effects themselves (`ItemStack.onUseTick` -> `Consumable` -> the item particles and
//! the sound) run in the client, where the item's data lives; the store hands it the used
//! item, how long the use has run, and the pose the particle math reads
//! (`LivingEntity.spawnItemParticles` positions from the feet, eye and rotations).

use haste_core::entity::ItemStackData;
use haste_core::ids::{EntityId, RegistryIndex};
use haste_core::DVec3;

use crate::store::Entity;

/// One tick of a remote entity's item use the client can emit effects for (vanilla
/// `updateUsingItem` -> `ItemStack.onUseTick`): everything the emit needs — the used item,
/// how long the use has run and the pose the particle math reads — recorded before the
/// remaining time ticks down, the value vanilla's `onUseTick` saw.
#[derive(Clone, Copy, Debug)]
pub struct UseTick {
    /// The entity using the item.
    pub entity: EntityId,
    /// The used stack's item (`ItemStack.isSameItem` held it this tick).
    pub item: RegistryIndex,
    /// Ticks the use has run when `onUseTick` fired (vanilla's `useItemRemaining` is the
    /// item's use duration minus this; the duration comes from the item's
    /// `minecraft:consumable`, resolved by the client).
    pub used: i32,
    /// The entity's feet position (`getX`/`getZ`).
    pub pos: DVec3,
    /// `getEyeY()` (the feet's y plus the type's eye height, halved for babies like
    /// `LivingEntity.getDefaultDimensions`'s age scale).
    pub eye_y: f64,
    /// (`getYRot`, `getXRot`) in degrees.
    pub look: (f32, f32),
}

/// `LivingEntity.useItem` / `useItemRemaining` for a remote entity: the item the use
/// started with and the ticks it has run. Vanilla keeps the remaining ticks instead of the
/// elapsed ones, but the use duration lives in the item's consumable, which this crate
/// does not resolve; the client maps `used` back (`remaining = duration - used`).
#[derive(Clone, Copy, Debug, Default)]
pub struct UsingItem {
    /// The stack the use started with (`getItemInHand` at the flag), `None` while no use
    /// runs. Vanilla's `useItem` empty means the same.
    pub item: Option<RegistryIndex>,
    /// Ticks the use has run (the count `onUseTick` has fired for).
    used: i32,
}

/// `LivingEntity.onSyncedDataUpdated` for `DATA_LIVING_ENTITY_FLAGS`: a use the server
/// started adopts the hand item and restarts the count (`useItemRemaining =
/// getUseDuration`); a clear ends it (`stopUsingItem`'s client half clears the state, the
/// flag itself stays server-owned). Only called when the parsed flags changed.
pub(super) fn on_living_flags(e: &mut Entity) {
    if e.metadata.living.using_item {
        // A use already running keeps its count (vanilla only fills an empty `useItem`).
        if e.using_item.item.is_none() {
            // `getUsedItemHand`: the flags' off-hand bit picks the equipment slot.
            let slot = if e.metadata.living.use_offhand { 1 } else { 0 };
            if let Some(item) = hand_item(&e.equipment, slot) {
                e.using_item = UsingItem {
                    item: Some(item),
                    used: 0,
                };
            }
        }
    } else {
        e.using_item = UsingItem::default();
    }
}

/// One entity's use step (vanilla `LivingEntity.tick` -> `updatingUsingItem` ->
/// `updateUsingItem`, on the client): while the synced using flag is set, the hand item
/// must still be the one the use started with (`ItemStack.isSameItem`; a swap or an
/// emptied hand stops the use without a packet), the use runs one tick (`onUseTick` reads
/// the remaining time first, then it ticks down), and the tick is recorded for the
/// client's emit. The eye height comes from the dimensions table (`getEyeY`).
pub(super) fn step(e: &mut Entity, dims: &crate::dims::EntityDimensions, out: &mut Vec<UseTick>) {
    if !e.is_living || !e.metadata.living.using_item {
        return;
    }
    let slot = if e.metadata.living.use_offhand { 1 } else { 0 };
    let hand = hand_item(&e.equipment, slot);
    if let Some(item) = hand.filter(|i| e.using_item.item == Some(*i)) {
        let baby = !e.is_player_type && crate::registry::is_baby(&e.metadata);
        let used = e.using_item.used;
        out.push(UseTick {
            entity: e.id,
            item,
            used,
            pos: e.pos,
            eye_y: e.pos.y + f64::from(dims.eye_height(e.kind, baby)),
            look: (e.yaw, e.pitch),
        });
        // Vanilla runs `onUseTick` with the current remaining time, then decrements it.
        e.using_item.used += 1;
    } else {
        // `stopUsingItem` on the client: the state clears, the flag stays server-owned.
        e.using_item = UsingItem::default();
    }
}

impl crate::store::EntityStore {
    /// The use steps this tick recorded (vanilla `LivingEntity.updateUsingItem` on the
    /// client), for the client's consume effects; empty until the next tick's step.
    pub fn use_ticks(&self) -> &[UseTick] {
        &self.use_ticks
    }
}

/// `getItemInHand`: the item of an equipment slot (0 main hand, 1 off hand), `None` when
/// the slot is empty, unknown or holds an empty stack.
fn hand_item(equipment: &[(u8, ItemStackData)], slot: u8) -> Option<RegistryIndex> {
    equipment
        .iter()
        .find(|(s, _)| *s == slot)
        .and_then(|(_, stack)| if stack.is_empty() { None } else { stack.item })
}
