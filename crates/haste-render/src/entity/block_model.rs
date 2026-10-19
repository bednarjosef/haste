//! The moving-block submit path (vanilla `SubmitNodeCollector.submitMovingBlock`, shared by
//! `FallingBlockRenderer`, `TntRenderer` and the piston renderer): a block state's model
//! drawn through the entity vertex stream, shaded like terrain and lit at the entity.

use super::*;
use haste_assets::AtlasRegions;

/// Vanilla `CardinalLighting` (the terrain shader's face shading, `chunk.wgsl`): down 0.5,
/// up 1.0, north/south 0.8, west/east 0.6; elements with `shade: false` are unshaded.
pub(super) fn cardinal_shade(face: Option<haste_assets::Direction>) -> f32 {
    use haste_assets::Direction::*;
    match face {
        Some(Down) => 0.5,
        Some(Up) => 1.0,
        Some(North) | Some(South) => 0.8,
        Some(West) | Some(East) => 0.6,
        None => 1.0,
    }
}

/// The white flash overlay of `TntMinecartRenderer.submitWhiteSolidBlock`: it packs
/// `OverlayTexture.u(1.0)` = 15 with the white row (v 10), whose texel there is white at
/// alpha 63/255; the entity shader mixes it over the shaded colour
/// (`mix(overlayColor.rgb, color.rgb, overlayColor.a)`), before the lightmap.
pub(super) fn flash_tint(shade: f32) -> [f32; 3] {
    let a = 63.0 / 255.0;
    [(1.0 - a) + a * shade; 3]
}

/// One block-model quad: like `push_world_quad`, but the shading is the terrain's cardinal
/// face shade (folded into `color`) and the normal only settles the winding — the moving
/// block's poses never rotate, so a face direction is its world normal.
#[allow(clippy::too_many_arguments)]
fn push_block_quad(
    out: &mut Vec<EntityVertex>,
    m: Mat4,
    p: [[f32; 3]; 4],
    uv: [[f32; 2]; 4],
    n: Vec3,
    color: [f32; 3],
    light: (u8, u8),
    rect: [u32; 4],
) {
    let p = p.map(Vec3::from).map(|v| m.transform_point3(v));
    let uv = uv.map(|t| {
        [
            rect[0] as f32 + t[0] * rect[2] as f32,
            rect[1] as f32 + t[1] * rect[3] as f32,
        ]
    });
    let ccw = (p[1] - p[0]).cross(p[2] - p[0]).dot(n) >= 0.0;
    let order: [usize; 6] = if ccw {
        [0, 1, 2, 0, 2, 3]
    } else {
        [0, 2, 1, 0, 3, 2]
    };
    let r16 = rect.map(|v| v as u16);
    for i in order {
        out.push(EntityVertex {
            pos: p[i].to_array(),
            uv: uv[i],
            color: [color[0], color[1], color[2], 1.0],
            rect: r16,
            light: [light.0 as u16, light.1 as u16, 0, 0],
        });
    }
}

impl EntityRenderer {
    /// Emits one block state's baked model through `m` (`FallingBlockRenderer.submit` /
    /// `TntRenderer.submit`): the terrain's baked quads, textures resolved into the entity
    /// atlas like the piston's moved block, cardinal face shading, and `white_flash` mixing
    /// the TNT flash overlay into the colour. States without models (air, non-`MODEL`
    /// render shapes) have no quads and draw nothing, never a panic.
    pub(super) fn emit_block_model(
        &mut self,
        m: Mat4,
        state: haste_core::BlockStateId,
        light: (u8, u8),
        white_flash: bool,
        assets: &mut Assets,
    ) {
        // The terrain's bake is immutable behind the `Arc`; clone the pointer so the quad
        // slice outlives the mutable atlas lookups below.
        let baked = Arc::clone(&self.baked);
        let quads = baked.quads(state);
        for q in quads {
            // Baked UVs are terrain-atlas texels; the entity vertex wants sprite-local
            // 0..1 over the entity atlas's copy of the same texture.
            let region = self.terrain_atlas.region_of(&q.texture);
            let [rx, ry, rw, rh] = region.pixel_rect.map(|v| v as f64);
            let uvs = q.uvs.map(|t| {
                [
                    ((t[0] as f64 - rx) / rw.max(1.0)) as f32,
                    ((t[1] as f64 - ry) / rh.max(1.0)) as f32,
                ]
            });
            let rect = self.texture_rect(&parse_tex(&q.texture), assets);
            let shade = cardinal_shade(q.shade_face);
            let color = if white_flash {
                flash_tint(shade)
            } else {
                [shade, shade, shade]
            };
            let o = q.direction.offset();
            push_block_quad(
                &mut self.vertices,
                m,
                q.positions,
                uvs,
                Vec3::new(o[0] as f32, o[1] as f32, o[2] as f32),
                color,
                light,
                rect,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cardinal_shade_matches_the_terrain_shader() {
        use haste_assets::Direction::*;
        assert_eq!(cardinal_shade(Some(Down)), 0.5);
        assert_eq!(cardinal_shade(Some(Up)), 1.0);
        assert_eq!(cardinal_shade(Some(North)), 0.8);
        assert_eq!(cardinal_shade(Some(South)), 0.8);
        assert_eq!(cardinal_shade(Some(West)), 0.6);
        assert_eq!(cardinal_shade(Some(East)), 0.6);
        // `shade: false` model elements render unshaded.
        assert_eq!(cardinal_shade(None), 1.0);
    }

    #[test]
    fn flash_tint_is_the_white_overlay_texel() {
        // `OverlayTexture` at u 15: white at alpha (1 - 15/15 * 0.75) * 255 = 63.
        let t = flash_tint(1.0);
        for c in t {
            assert!((c - 1.0).abs() < 1e-6);
        }
        // A dark face flashes lighter, never darker: mix(white, 0.5, 63/255).
        let t = flash_tint(0.5);
        let mixed = 63.0 / 255.0 * 0.5 + (1.0 - 63.0 / 255.0);
        for c in t {
            assert!((c - mixed).abs() < 1e-6);
        }
    }
}
