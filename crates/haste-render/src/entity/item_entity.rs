//! The dropped item's transform steps (vanilla's `ItemEntityRenderer` and `ItemEntity.getSpin`),
//! moved out of `entity.rs` as is.

/// Dropped items spin about the vertical axis: `ItemEntity.getSpin`, the age in ticks
/// (plus the partial tick) over 20, plus the entity's random phase, in radians.
pub(crate) fn item_entity_spin(age: f32, bob_offs: f32) -> f32 {
    age / 20.0 + bob_offs
}

/// ... and bob up and down: `ItemEntityRenderer.submit`'s
/// `sin(age / 10 + phase) * 0.1 + 0.1` blocks, on top of the lift that puts the
/// model's bounding box above the ground (see `item_entity_lift`).
pub(crate) fn item_entity_bob(age: f32, bob_offs: f32) -> f32 {
    ((age / 10.0 + bob_offs).sin()) * 0.1 + 0.1
}

/// The lift `ItemEntityRenderer.submit` applies so the model's bounding box sits above
/// the ground: `-bbox.minY + 1/16`. The bounding box is the model drawn through its
/// `display.ground` transform (which carries vanilla's 0.5/0.25 ground scales), so no
/// extra scale is applied here.
pub(crate) fn item_entity_lift(bbox_min_y: f32) -> f32 {
    -bbox_min_y + 1.0 / 16.0
}

/// How many copies of the item a stack draws: `ItemClusterRenderState
/// .getRenderedAmount` — 1 for a single item, then one more per full 16 of the count,
/// up to 5.
pub(crate) fn item_entity_copies(count: i32) -> u32 {
    match count {
        ..=1 => 1,
        2..=16 => 2,
        17..=32 => 3,
        33..=48 => 4,
        _ => 5,
    }
}
