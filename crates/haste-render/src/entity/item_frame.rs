//! The item frame renderer (`ItemFrameRenderer`): the frame block model quads and the
//! framed item at `rotation * 45` degrees, half scale.

use super::*;

impl EntityRenderer {
    /// One item frame, following vanilla's `ItemFrameRenderer.submit`: the frame block model at
    /// `direction * 0.46875` turned to face `direction`, then the framed item rotated by
    /// `rotation * 45` degrees at scale 0.5. Glow frames light the frame at least block-light 5
    /// and draw the item full-bright.
    pub(super) fn emit_item_frame(
        &mut self,
        e: &EntityInstance,
        f: &ItemFrameDecoration,
        camera: glam::DVec3,
        sky: u8,
        block: u8,
        assets: &mut Assets,
    ) {
        let base =
            Mat4::from_translation((e.pos - camera).as_vec3()) * item_frame_transform(f.facing);

        let map = f
            .item
            .and_then(|i| self.item_names.get(i.0 as usize))
            .is_some_and(|n| n.path() == "filled_map");
        // `ItemFrameRenderer.extractRenderState`: an invisible frame clears its block model,
        // so only the item inside draws, pushed flush to the wall (z 0.5 instead of 0.4375,
        // `ItemFrameRenderer.submit`).
        let mut out = std::mem::take(&mut self.vertices);
        if !e.invisible {
            let quads = self.frame_quads(f.glow, map, assets);
            // `ItemFrameRenderer.getBlockLightLevel`: glow frames are at least block-light 5.
            let frame_light = if f.glow {
                (block.max(5), sky)
            } else {
                (block, sky)
            };
            let model = base * Mat4::from_translation(Vec3::splat(-0.5));
            let nm = Mat3::from_mat4(model).inverse().transpose();
            for q in quads.iter() {
                let rect = self.texture_rect(&parse_tex(&q.texture), assets);
                let o = q.direction.offset();
                push_world_quad(
                    &mut out,
                    model,
                    nm,
                    q.positions.map(Vec3::from),
                    q.uvs,
                    Vec3::new(o[0] as f32, o[1] as f32, o[2] as f32),
                    [1.0; 3],
                    frame_light,
                    rect,
                );
            }
        }
        if let Some(idx) = f.item {
            if let Some(item) = self.item_model_for(idx, assets) {
                // Glow frames draw the item full-bright (`getLightCoords(isGlowFrame, 15728880)`,
                // block 15 / sky 15); otherwise the frame's own (block, sky) light.
                let item_light = if f.glow { (15, 15) } else { (block, sky) };
                let root = base * item_frame_item_transform(f.rotation, e.invisible);
                self.emit_item(
                    &mut out,
                    &item,
                    root,
                    item_light,
                    ItemDisplay::Fixed,
                    assets,
                );
            }
        }
        self.vertices = out;
    }
    /// The item frame block model quads for a `(glow, map)` combination, resolved from the jar's
    /// `item_frame` / `glow_item_frame` blockstate (`BlockModelResolver.updateForItemFrame`) once
    /// and cached. An unreadable model yields no quads, never a panic.
    fn frame_quads(
        &mut self,
        glow: bool,
        map: bool,
        assets: &mut Assets,
    ) -> Arc<Vec<haste_assets::BakedQuad>> {
        if let Some(q) = self.frame_models.get(&(glow, map)) {
            return q.clone();
        }
        let block = if glow {
            "glow_item_frame"
        } else {
            "item_frame"
        };
        let mut quads = Vec::new();
        if let Ok(def) = assets.blockstate(&Identifier::minecraft(block)) {
            let props = vec![("map".to_string(), map.to_string())];
            for r in def.model_refs_for(&props) {
                if let Ok(id) = Identifier::parse(&r.model) {
                    if let Ok(mut q) = assets.model_quads(&id) {
                        quads.append(&mut q);
                    }
                }
            }
        }
        let arc = Arc::new(quads);
        self.frame_models.insert((glow, map), arc.clone());
        arc
    }
}
