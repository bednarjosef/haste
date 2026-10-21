//! The block display renderer (`DisplayRenderer.BlockDisplayRenderer`): the block state's
//! model, drawn like a block item (`BlockModelRenderState.submit` through the item cutout
//! sheet), at the display's pose with its unit cube's corner on the entity.

use super::*;
use haste_assets::AtlasRegions;

/// `BlockModelRenderState.submitModel`'s light: `LightCoordsUtil.max` of the light at the
/// entity and the block's own (`BlockModelResolver.update`: its light emission as block
/// light, sky 0). `light` is (block, sky).
pub(crate) fn block_display_light(light: (u8, u8), light_emission: u8) -> (u8, u8) {
    (light.0.max(light_emission.min(15)), light.1)
}

impl EntityRenderer {
    /// One block display (`BlockDisplayRenderer.submitInner`): the terrain's baked quads of
    /// `state` through `pose`, lit like items (the entity lighting of each quad's turned
    /// normal: vanilla's item shaders light the block-item sheet from the vertex normals),
    /// textures resolved into the entity atlas like the falling block's. States without a
    /// model (air, entity-block shapes) draw nothing.
    pub(super) fn emit_block_display(
        &mut self,
        pose: Mat4,
        state: haste_core::BlockStateId,
        light_emission: u8,
        light: (u8, u8),
        assets: &mut Assets,
    ) {
        let light = block_display_light(light, light_emission);
        let normal_matrix = Mat3::from_mat4(pose).inverse().transpose();
        let baked = Arc::clone(&self.baked);
        let mut out = std::mem::take(&mut self.vertices);
        for q in baked.quads(state) {
            // Baked UVs are terrain-atlas texels; the entity vertex wants sprite-local 0..1
            // over the entity atlas's copy of the same texture.
            let region = self.terrain_atlas.region_of(&q.texture);
            let [rx, ry, rw, rh] = region.pixel_rect.map(|v| v as f64);
            let uvs = q.uvs.map(|t| {
                [
                    ((t[0] as f64 - rx) / rw.max(1.0)) as f32,
                    ((t[1] as f64 - ry) / rh.max(1.0)) as f32,
                ]
            });
            let rect = self.texture_rect(&parse_tex(&q.texture), assets);
            let o = q.direction.offset();
            push_world_quad(
                &mut out,
                pose,
                normal_matrix,
                q.positions.map(Vec3::from),
                uvs,
                Vec3::new(o[0] as f32, o[1] as f32, o[2] as f32),
                [1.0; 3],
                light,
                rect,
            );
        }
        self.vertices = out;
    }
}
