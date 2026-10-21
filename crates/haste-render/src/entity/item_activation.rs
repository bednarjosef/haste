//! The totem pop's item in front of the camera (vanilla
//! `ScreenEffectRenderer.renderItemActivationAnimation`): the activated stack flies up out of
//! the distance, spins and shrinks away again over the 40 ticks of the local player's
//! `ItemActivation`.
//!
//! Vanilla submits it among the screen effects, after the hand, with a fresh pose stack (view
//! space, no bob and no view rotation), the HUD projection the hand uses, the depth the hand
//! pass cleared, full brightness and the `ITEMS_3D` light directions. It is drawn whatever the
//! camera's perspective, only not while F1 hides the HUD. Haste draws it into the hand's vertex
//! list, so it costs no extra draw call.

use super::*;

/// The local player's `ItemActivation` as the renderer reads it each frame (vanilla
/// `PlayerRenderState.ItemActivationRenderState`): the shown item, the ticks left in the
/// countdown and the two random offsets in -1..1 the item drifts by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemActivation {
    /// The shown stack's item (the totem, or the stack with death protection).
    pub item: haste_core::RegistryIndex,
    /// `ItemActivation.ticks`: 40 when the pop starts, counting down to 1.
    pub ticks: i32,
    /// `ItemActivation.offX`.
    pub off_x: f32,
    /// `ItemActivation.offY`.
    pub off_y: f32,
}

/// Vanilla `ItemActivation.ANIMATION_LENGTH`: the ticks the curve runs over.
const ANIMATION_LENGTH: f32 = 40.0;

/// The item's pose in view space for one frame (x right, y up, -z forward), as
/// `renderItemActivationAnimation` builds it on its pose stack: the drift and the flight
/// towards the camera, the 0.8 scale, the spin about Y and the wobble about X and Z. `aspect`
/// is the window's width over its height, which stretches the horizontal drift.
pub fn item_activation_pose(a: &ItemActivation, partial: f32, aspect: f32) -> Mat4 {
    let tick = ANIMATION_LENGTH - a.ticks as f32;
    let scale = (tick + partial) / ANIMATION_LENGTH;
    let ts = scale * scale;
    let tc = scale * ts;
    let smooth = 10.25 * tc * ts - 24.95 * ts * ts + 25.5 * tc - 13.8 * ts + 4.0 * scale;
    let pi_scale = smooth * std::f32::consts::PI;
    let drift = (pi_scale * 2.0).sin().abs();
    let wobble = (6.0 * (scale * 8.0).cos()).to_radians();
    Mat4::from_translation(Vec3::new(
        a.off_x * 0.3 * aspect * drift,
        a.off_y * 0.3 * drift,
        -10.0 + 9.0 * pi_scale.sin(),
    )) * Mat4::from_scale(Vec3::splat(0.8))
        * Mat4::from_rotation_y((900.0 * pi_scale.sin().abs()).to_radians())
        * Mat4::from_rotation_x(wobble)
        * Mat4::from_rotation_z(wobble)
}

/// A rotation that turns view-space normals into the frame `entity_shade` lights, so the
/// baked shade is the one vanilla's `Lighting.Entry.ITEMS_3D` gives in view space.
///
/// `ITEMS_3D`'s two light directions are the level's two (`entity_shade`'s) through an
/// orthogonal matrix `M` with a mirror in it (`Lighting`'s item pose: scale `(1, -1, 1)`, then
/// `rotateYXZ(1.0821041, 3.2375858, 0)` and `rotateYXZ(-pi/8, 3pi/4, 0)`). The shade only
/// depends on a normal's dot products with the two lights, so the proper rotation that maps
/// the `ITEMS_3D` pair onto the level pair gives the same shade as `M`'s transpose without
/// flipping the triangles' winding.
pub(super) fn items_3d_shade_frame() -> Mat3 {
    let level = [
        Vec3::new(0.2, 1.0, -0.7).normalize(),
        Vec3::new(-0.2, 1.0, 0.7).normalize(),
    ];
    let pose = Mat3::from_diagonal(Vec3::new(1.0, -1.0, 1.0))
        * Mat3::from_rotation_y(1.082_104_1)
        * Mat3::from_rotation_x(3.237_585_8)
        * Mat3::from_rotation_y(-std::f32::consts::FRAC_PI_8)
        * Mat3::from_rotation_x(std::f32::consts::FRAC_PI_4 * 3.0);
    let items = level.map(|l| pose * l);
    // The orthonormal frames the two pairs span; the rotation takes one onto the other.
    let frame = |a: Vec3, b: Vec3| {
        let x = a;
        let y = (b - a * a.dot(b)).normalize();
        Mat3::from_cols(x, y, x.cross(y))
    };
    frame(level[0], level[1]) * frame(items[0], items[1]).transpose()
}

impl EntityRenderer {
    /// The item and the countdown the totem pop shows, or `None`: set each frame from the
    /// game's view (F1 clears it, as vanilla skips the animation while the HUD is hidden).
    pub fn set_item_activation(&mut self, activation: Option<ItemActivation>) {
        self.item_activation = activation;
    }

    /// Appends the activated item to `out` in view space (the hand pass's space): the pose for
    /// `partial`, full brightness, the `ITEMS_3D` shade. An item without a model draws
    /// vanilla's missing model.
    pub(super) fn emit_item_activation(
        &mut self,
        out: &mut Vec<EntityVertex>,
        partial: f32,
        aspect: f32,
        assets: &mut Assets,
    ) {
        let Some(activation) = self.item_activation else {
            return;
        };
        let model = self
            .item_model_for(activation.item, assets)
            .unwrap_or_else(missing_item_model);
        let shade = items_3d_shade_frame();
        let pose = item_activation_pose(&activation, partial, aspect);
        let start = out.len();
        // `15728880`: block and sky light 15.
        self.emit_item(
            out,
            &model,
            Mat4::from_mat3(shade) * pose,
            (15, 15),
            ItemDisplay::Fixed,
            assets,
        );
        // Back from the shade frame to view space (a rotation: the winding is kept).
        let back = shade.transpose();
        for v in &mut out[start..] {
            v.pos = (back * Vec3::from(v.pos)).into();
        }
    }
}
