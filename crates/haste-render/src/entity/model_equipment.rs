//! The mobs' own equipment layers (vanilla `SimpleEquipmentLayer`, `WolfArmorLayer`,
//! `LlamaDecorLayer`): a horse's or wolf's body armour, a saddle, a llama's carpet. Each worn
//! piece arrives from `haste-entity` with the layer model its renderer bakes and the layer type
//! its asset's textures are keyed by; this draws it the way `EquipmentLayerRenderer.renderLayers`
//! does, every layer of that type on the model, then the wolf armour's cracks.

use super::*;

impl EntityRenderer {
    /// Draws the instance's model equipment pieces in their layer order, with the entity's
    /// pose (vanilla runs the wearer's `setupAnim` on the layer model too) and at its root.
    /// Equipment layers take no hurt flash (`renderLayers` and `maybeRenderCracks` submit with
    /// `OverlayTexture.NO_OVERLAY`). Nothing allocates: the vertex buffer is taken and put back.
    pub(super) fn emit_model_equipment(
        &mut self,
        e: &EntityInstance,
        root: Mat4,
        light: (u8, u8),
        assets: &mut Assets,
    ) {
        for piece in &e.equipment {
            let Some(layer) = &piece.layer else { continue };
            let Some(asset) = &piece.asset else { continue };
            let Some(info) = self.equipment_info(asset, assets) else {
                continue;
            };
            let model = &layer.model;
            let mut out = std::mem::take(&mut self.vertices);
            for l in info.layers_of(layer.layer_type) {
                let Some(tint) = equipment::layer_tint(l, piece.dye) else {
                    continue;
                };
                let rect = self.texture_rect(&l.texture, assets);
                emit_part_tinted(
                    &mut out,
                    model,
                    &model.root,
                    &e.pose,
                    root,
                    rect,
                    light,
                    0.0,
                    tint,
                );
            }
            // `WolfArmorLayer.maybeRenderCracks`: the crack texture on the same model, after
            // the armour (`RenderTypes.wolfArmorCracks` blends translucently, as this pass does).
            if let Some(cracks) = &layer.cracks {
                let rect = self.texture_rect(cracks, assets);
                emit_part_tinted(
                    &mut out,
                    model,
                    &model.root,
                    &e.pose,
                    root,
                    rect,
                    light,
                    0.0,
                    [1.0; 3],
                );
            }
            self.vertices = out;
        }
    }
}
