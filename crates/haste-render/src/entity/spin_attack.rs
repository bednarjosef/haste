//! `SpinAttackEffectLayer.submit`: the riptide spin's two boxes (`SpinAttackEffectModel`, built
//! by haste-entity's `spin_attack` as an overlay with the layer's texture) drawn in the player's
//! own pose while it auto-spins.
//!
//! The boxes carry a part scale (`SpinAttackEffectModel.createLayer`: 0.75 and 1.5) that
//! haste-core's `ModelPart` cannot hold, so the overlay is recognised by its texture and each box
//! is drawn through a parent transform that scales about the part's pivot: the part's
//! own translate and rotate then compose to `ModelPart.translateAndRotate`'s translate, rotate,
//! scale (a uniform scale commutes with the rotation). The cubes stay unscaled, so their box
//! UVs cover the 64 x 64 texture exactly. The layer draws with `OverlayTexture.NO_OVERLAY` (no
//! hurt flash), and on an invisible player too: the layer checks only the spin flag.

use super::*;

/// `SpinAttackEffectLayer.TEXTURE`, which marks the overlay as this layer.
const TEXTURE: &str = "textures/entity/trident/trident_riptide.png";

/// Whether an overlay model is the spin layer (by its texture, without allocating).
pub(super) fn is_spin_layer(model: &CubeModel) -> bool {
    model.texture.namespace() == "minecraft" && model.texture.path() == TEXTURE
}

/// `SpinAttackEffectModel.createLayer`: box `i`'s part scale, `0.75 * (i + 1)`.
pub(super) fn box_scale(i: usize) -> f32 {
    0.75 * (i + 1) as f32
}

impl EntityRenderer {
    /// Draws the spin layer when it is among the instance's overlays (`dress` puts it there
    /// only while `isAutoSpinAttack`).
    pub(super) fn emit_spin_attack(
        &mut self,
        e: &EntityInstance,
        root: Mat4,
        light: (u8, u8),
        assets: &mut Assets,
    ) {
        let Some(overlay) = e
            .overlays
            .iter()
            .flatten()
            .find(|o| is_spin_layer(&o.model))
        else {
            return;
        };
        let model = &overlay.model;
        let rect = self.texture_rect(&model.texture, assets);
        for (i, part) in model.root.children.iter().enumerate() {
            let parent = root * scale_about(part.pivot, box_scale(i));
            emit_part_tinted(
                &mut self.vertices,
                model,
                part,
                &e.pose,
                parent,
                rect,
                light,
                0.0,
                overlay.tint,
            );
        }
    }
}

/// A uniform scale by `s` about `pivot` (model units).
pub(super) fn scale_about(pivot: Vec3, s: f32) -> Mat4 {
    Mat4::from_translation(pivot)
        * Mat4::from_scale(Vec3::splat(s))
        * Mat4::from_translation(-pivot)
}
