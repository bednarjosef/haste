//! `FirstPersonHandsAndItemsRenderer.applyEatTransform`: the first-person eat and drink use
//! animation (vanilla shares it between `ItemUseAnimation.EAT` and `DRINK`) and the
//! `applyItemArmTransform` it composes with.
//!
//! Vanilla's `submitArmWithItem` use branch applies `applyEatTransform` and then
//! `applyItemArmTransform` again; the `!hasCustomArmTransform()` transform before the switch
//! is skipped because both EAT and DRINK have `customArmTransform` true. The swing is not
//! applied at all while a use runs.

use glam::{Mat4, Vec3};

/// `FirstPersonHandsAndItemsRenderer.applyEatTransform` for the right main arm.
/// `remaining` is `FirstPersonHandsAndItemsRenderState.useItemRemainingTicks` (vanilla passes
/// it as a float) and `duration` `state.mainHandUseDuration` (`Item.getUseDuration`, a
/// consumable's `consume_ticks`).
pub(super) fn eat_transform(partial: f32, remaining: f32, duration: i32) -> Mat4 {
    // The ticks left in the use (with the partial tick), then as a fraction of the whole use.
    let curr_usage_time = remaining - partial + 1.0;
    let scaled_usage_time = curr_usage_time / duration as f32;
    let mut m = Mat4::IDENTITY;
    // The chewing bob, once the first fifth of the use has passed.
    if scaled_usage_time < 0.8 {
        let extra_height = (curr_usage_time / 4.0 * std::f32::consts::PI).cos().abs() * 0.1;
        m *= Mat4::from_translation(Vec3::new(0.0, extra_height, 0.0));
    }
    // How far the item has moved towards the mouth: 1 for most of the use, 0 at its start.
    let eat_jiggle = 1.0 - scaled_usage_time.powf(27.0);
    // invert = 1 (the right arm).
    m *= Mat4::from_translation(Vec3::new(eat_jiggle * 0.6, eat_jiggle * -0.5, 0.0));
    m *= Mat4::from_rotation_y((eat_jiggle * 90.0f32).to_radians());
    m *= Mat4::from_rotation_x((eat_jiggle * 10.0f32).to_radians());
    m *= Mat4::from_rotation_z((eat_jiggle * 30.0f32).to_radians());
    m
}

/// `FirstPersonHandsAndItemsRenderer.applyItemArmTransform`: the held item's place in view
/// space (`0.56, -0.52 + inverseArmHeight * -0.6, -0.72`), the right arm's `invert` 1.
pub(super) fn item_arm_transform(inverse_arm_height: f32) -> Mat4 {
    Mat4::from_translation(Vec3::new(0.56, -0.52 + inverse_arm_height * -0.6, -0.72))
}

#[cfg(test)]
mod tests;
