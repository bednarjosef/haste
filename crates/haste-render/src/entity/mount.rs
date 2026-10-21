//! The mounts' equipment layers: the wolf armour's crack overlay (`WolfArmorLayer
//! .maybeRenderCracks`), drawn after the armour. The horse and wolf body armour and the saddle
//! layers need their dumped geometry, which only haste-entity reads (haste-render has it as a
//! dev-dependency alone); they wait for a core field that carries that geometry.

use super::*;

/// Whether an overlay is a wolf armour crack (its texture is one of the crack textures). The
/// overlay pass skips these, and `emit_wolf_crack` draws them after the armour.
pub(super) fn is_crack_overlay(model: &haste_core::model::CubeModel) -> bool {
    model
        .texture
        .path()
        .starts_with("entity/wolf/wolf_armor_crackiness")
}

impl EntityRenderer {
    /// Draws a wolf armour's crack over its model (already emitted at `root`), with the body
    /// pose minus its hidden parts as the armour uses (`pose_without_hidden`). Vanilla draws
    /// it after the armour at order(1).
    pub(super) fn emit_wolf_crack(
        &mut self,
        e: &EntityInstance,
        root: Mat4,
        light: (u8, u8),
        assets: &mut Assets,
    ) {
        let armor_pose = pose_without_hidden(&e.model, &e.pose);
        for overlay in e.overlays.iter().flatten() {
            if !is_crack_overlay(&overlay.model) {
                continue;
            }
            let rect = self.texture_rect(&overlay.model.texture, assets);
            let mut out = std::mem::take(&mut self.vertices);
            emit_part_tinted(
                &mut out,
                &overlay.model,
                &overlay.model.root,
                &armor_pose,
                root,
                rect,
                light,
                e.hurt,
                overlay.tint,
            );
            self.vertices = out;
        }
    }
}
