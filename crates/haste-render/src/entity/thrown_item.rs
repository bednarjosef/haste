//! The thrown-item renderer (`ThrownItemRenderer`): snowballs, eggs, ender pearls, eyes of
//! ender, splash and lingering potions, experience bottles and the two fireballs — the
//! entity's item as a camera-facing sprite at the renderer's scale, through the model's
//! `ground` display transform.

use super::*;

#[cfg(test)]
mod tests;

/// The billboard root `ThrownItemRenderer.submit` builds: the entity's position, the
/// renderer's `scale`, then the camera orientation (the item model's own `ground` display
/// transform is applied on top of this by [`EntityRenderer::emit_item`]).
pub(crate) fn thrown_item_root(origin: Vec3, scale: f32, orientation: glam::Quat) -> Mat4 {
    Mat4::from_translation(origin)
        * Mat4::from_scale(Vec3::splat(scale))
        * Mat4::from_quat(orientation)
}

impl EntityRenderer {
    /// One thrown projectile, following vanilla's `ThrownItemRenderer.submit`: the item
    /// model at the entity's position, scaled by the renderer's `scale` and rotated to face
    /// the camera. `fullBright` types (`ThrownItemRenderer.getBlockLightLevel`: the eye of
    /// ender, the fireballs) draw at block light 15, sky light kept. An unknown item draws
    /// the missing model (vanilla's missingno cube); nothing panics.
    pub(super) fn emit_thrown_item(
        &mut self,
        e: &EntityInstance,
        t: &ThrownItemDecoration,
        camera: glam::DVec3,
        orientation: glam::Quat,
        light: (u8, u8),
        assets: &mut Assets,
    ) {
        let model = self
            .item_model_for(t.item, assets)
            .unwrap_or_else(missing_item_model);
        // `ThrownItemRenderer.getBlockLightLevel` returns 15 for full-bright types.
        let light = if t.full_bright { (15, light.1) } else { light };
        // The body transform (a shot-at-angle rocket's flight turn, `FireworkEntityRenderer.submit`)
        // sits under the camera orientation, as vanilla's pose stack has it; empty for the rest.
        let root = thrown_item_root((e.pos - camera).as_vec3(), t.scale, orientation)
            * body_matrix(&e.body);
        let mut out = std::mem::take(&mut self.vertices);
        self.emit_item(
            &mut out,
            &model,
            root,
            light,
            ItemDisplay::Ground,
            haste_core::model::FoilType::None,
            assets,
        );
        self.vertices = out;
    }
}
