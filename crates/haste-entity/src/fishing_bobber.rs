//! The tracked fishing bobbers (`FishingHook`, `minecraft:fishing_bobber`) for the client's
//! draw list. Vanilla's `FishingHookRenderer` reads its owner from the hook (`getPlayerOwner`);
//! the spawn packet carries that owner's id in the `add_entity` data field (the hook's own id
//! when it has none), so the owner is looked up through `slot.data`.
//!
//! The bobber itself is a billboard at the hook's interpolated position, and its line runs from
//! the owner's rod hand (`FishingHookRenderer.getPlayerHandPos`). The hand is resolved here in
//! third person; the first-person hand needs the camera and is the renderer's job.

use haste_core::entity::Pose;
use haste_core::ids::{EntityId, RegistryIndex};
use haste_core::{DVec3, Vec3};

use crate::dims::EntityDimensions;
use crate::fishing_hook::{
    fraction, hand_pos_third_person, holding_arm_is_right, line_origin_offset, string_point,
    STRING_SEGMENTS,
};
use crate::store::{Entity, EntityStore};

/// The line's sample points: `STRING_SEGMENTS + 1` fractions along it, from the bobber to the hand.
pub const LINE_POINTS: usize = STRING_SEGMENTS as usize + 1;

/// One bobber this frame: its interpolated position and, while its owner is a player, the
/// line's points from the bobber to the owner's rod hand (`FishingHookRenderer.stringVertex`
/// at each fraction, relative to `pos`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FishingBobber {
    pub id: EntityId,
    pub pos: DVec3,
    pub line: Option<[Vec3; LINE_POINTS]>,
    /// Block and sky light at the bobber, sampled by the client after the build (the store
    /// holds no world); zero until then.
    pub light: (u8, u8),
}

/// Whether an entity type is a fishing bobber (`EntityType.FISHING_BOBBER`).
pub fn is_fishing_bobber(kind: RegistryIndex) -> bool {
    crate::registry::entity_type_name(kind) == Some("minecraft:fishing_bobber")
}

/// The fishing rod's item name, as the registry names it.
const FISHING_ROD: &str = "minecraft:fishing_rod";

/// The hand's item from the synced equipment (`0` main, `1` off hand).
fn hand_item(e: &Entity, slot: u8) -> Option<RegistryIndex> {
    e.equipment
        .iter()
        .find(|(s, _)| *s == slot)
        .and_then(|(_, stack)| if stack.is_empty() { None } else { stack.item })
}

/// Whether the item is a fishing rod: its registry name is `minecraft:fishing_rod`.
fn is_fishing_rod(item: Option<RegistryIndex>) -> bool {
    match item {
        Some(index) => crate::thrown_item::item_indices()
            .and_then(|names| names.get(FISHING_ROD))
            .is_some_and(|rod| *rod == index),
        None => false,
    }
}

/// The bobbers in the store, each at `partial` with the owner's rod hand (none without a
/// player owner, which vanilla draws without a line).
pub fn bobbers(store: &EntityStore, partial: f32, out: &mut Vec<FishingBobber>) {
    out.clear();
    for slot in store.slots.iter().flatten() {
        if !is_fishing_bobber(slot.kind) {
            continue;
        }
        let pos = slot.prev_pos.lerp(slot.pos, f64::from(partial));
        let line = store
            .get(EntityId(slot.data))
            .filter(|owner| owner.is_player_type)
            .map(|owner| line_points(owner_hand(owner, &store.dims, partial), pos));
        out.push(FishingBobber {
            id: slot.id,
            pos,
            line,
            light: (0, 0),
        });
    }
}

/// The line's points for a hand at `hand` and a bobber at `hook_pos`: `string_point` at each
/// of the fractions the 16 segments are drawn between.
fn line_points(hand: DVec3, hook_pos: DVec3) -> [Vec3; LINE_POINTS] {
    let line = line_origin_offset(Some(hand), hook_pos);
    std::array::from_fn(|i| string_point(line, fraction(i as u32, STRING_SEGMENTS)))
}

/// The owner's rod hand, third person: the arm that holds the rod (`getHoldingArm`) at the
/// eye position moved by the body yaw.
fn owner_hand(owner: &Entity, dims: &EntityDimensions, partial: f32) -> DVec3 {
    let main_is_right = crate::animation::arm_pose::main_arm_is_right(owner);
    let main_is_rod = is_fishing_rod(hand_item(owner, 0));
    let holding_right = holding_arm_is_right(main_is_right, main_is_rod);
    let pos = owner.prev_pos.lerp(owner.pos, f64::from(partial));
    let eye = pos + DVec3::new(0.0, f64::from(dims.eye_height(owner.kind, false)), 0.0);
    hand_pos_third_person(
        eye,
        owner.body_yaw_at(partial),
        1.0,
        holding_right,
        owner.pose == Pose::Crouching,
    )
}
