//! The totem pop's item in front of the camera: vanilla
//! `ScreenEffectRenderer.renderItemActivationAnimation`. The curve below is the pose the item is
//! drawn under; the renderer draws the item itself through its item path, fixed to the view.

use glam::{Mat4, Vec3};
use haste_core::RegistryIndex;

#[cfg(test)]
mod tests;

/// Vanilla `ItemActivation.ANIMATION_LENGTH`: the countdown the curve runs over.
pub const ANIMATION_LENGTH: i32 = 40;

/// The item the camera shows this frame and the countdown's state (vanilla
/// `PlayerRenderState.ItemActivationRenderState`), copied out of the game thread's
/// `ItemActivation`.
#[derive(Clone, Debug)]
pub struct ItemActivationView {
    /// The registry index of the shown item (the totem, or the stack that carries death
    /// protection).
    pub item: RegistryIndex,
    /// The countdown's remaining ticks (`ItemActivation.ticks`), 1..=40 while shown.
    pub ticks: i32,
    /// The random offsets in -1..1 the item drifts by (`ItemActivation.offX`/`offY`).
    pub off_x: f32,
    pub off_y: f32,
}

/// The item's pose for one frame: vanilla's `renderItemActivationAnimation` pose stack, built
/// as one matrix (translate, scale 0.8, rotate Y, then X and Z). `aspect` is the window's
/// width over its height, which stretches the horizontal drift.
pub fn item_activation_pose(ticks: i32, partial: f32, off_x: f32, off_y: f32, aspect: f32) -> Mat4 {
    // `int tick = 40 - itemActivation.ticks`, then the curve's progress through the animation.
    let tick = (ANIMATION_LENGTH - ticks) as f32;
    let scale = (tick + partial) / ANIMATION_LENGTH as f32;
    let ts = scale * scale;
    let tc = scale * ts;
    let smooth = 10.25 * tc * ts - 24.95 * ts * ts + 25.5 * tc - 13.8 * ts + 4.0 * scale;
    let pi_scale = smooth * std::f32::consts::PI;
    let wobble = (pi_scale * 2.0).sin().abs();
    let off_x = off_x * 0.3 * aspect;
    let off_y = off_y * 0.3;
    let twist = (6.0 * (scale * 8.0).cos()).to_radians();
    Mat4::from_translation(Vec3::new(
        off_x * wobble,
        off_y * wobble,
        -10.0 + 9.0 * pi_scale.sin(),
    )) * Mat4::from_scale(Vec3::splat(0.8))
        * Mat4::from_rotation_y((900.0 * pi_scale.sin().abs()).to_radians())
        * Mat4::from_rotation_x(twist)
        * Mat4::from_rotation_z(twist)
}

/// The item and its pose for this frame, or `None` when no countdown runs. `width` and
/// `height` are the window's size, whose ratio the drift follows.
pub fn item_activation_for_frame(
    activation: Option<&ItemActivationView>,
    partial: f32,
    width: u32,
    height: u32,
) -> Option<(RegistryIndex, Mat4)> {
    activation.map(|a| {
        let aspect = width as f32 / height as f32;
        let pose = item_activation_pose(a.ticks, partial, a.off_x, a.off_y, aspect);
        (a.item, pose)
    })
}
