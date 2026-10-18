//! The painting renderer (`PaintingRenderer`): the front sprite grid, mirrored back and
//! edge strips, turned to the facing direction.

use super::*;

impl EntityRenderer {
    /// One painting, following vanilla's `PaintingRenderer`: a `width` x `height` grid of front
    /// quads at z = -1/32 with the variant's sprite, a mirrored back at z = +1/32 and the edge
    /// strips, all turned by `180 - direction.get2DDataValue() * 90`. An unknown variant draws
    /// the back sprite on the front too (never a panic).
    pub(super) fn emit_painting(
        &mut self,
        e: &EntityInstance,
        p: &PaintingDecoration,
        camera: glam::DVec3,
        sky: u8,
        block: u8,
        assets: &mut Assets,
    ) {
        let base =
            Mat4::from_translation((e.pos - camera).as_vec3()) * painting_transform(p.facing);
        // Paintings use the entity's own light (`PaintingRenderer` draws with `state.lightCoords`).
        let light = (block, sky);
        let width = p.width.max(1) as i32;
        let height = p.height.max(1) as i32;
        let back = self.texture_rect(&Identifier::minecraft("painting/back"), assets);
        let front = p
            .front
            .as_ref()
            .map(|id| self.texture_rect(id, assets))
            .unwrap_or(back);
        let nm = Mat3::from_mat4(base).inverse().transpose();
        let mut out = std::mem::take(&mut self.vertices);
        let (ox, oy) = (-(width as f32) / 2.0, -(height as f32) / 2.0);
        let (du, dv) = (1.0 / width as f32, 1.0 / height as f32);
        const Z: f32 = 0.03125;
        // The edge strips sample the back sprite's top/bottom 1/16 and left/right 1/16.
        let (edge_v, edge_u) = (0.0625f32, 0.0625f32);
        let quad = |out: &mut Vec<EntityVertex>, p: [Vec3; 4], uv: [[f32; 2]; 4], n: Vec3, rect| {
            push_world_quad(out, base, nm, p, uv, n, [1.0; 3], light, rect);
        };
        for sx in 0..width {
            for sy in 0..height {
                let x0 = ox + (sx + 1) as f32;
                let x1 = ox + sx as f32;
                let y0 = oy + (sy + 1) as f32;
                let y1 = oy + sy as f32;
                let fu0 = du * (width - sx) as f32;
                let fu1 = du * (width - (sx + 1)) as f32;
                let fv0 = dv * (height - sy) as f32;
                let fv1 = dv * (height - (sy + 1)) as f32;
                quad(
                    &mut out,
                    [
                        Vec3::new(x0, y1, -Z),
                        Vec3::new(x1, y1, -Z),
                        Vec3::new(x1, y0, -Z),
                        Vec3::new(x0, y0, -Z),
                    ],
                    [[fu1, fv0], [fu0, fv0], [fu0, fv1], [fu1, fv1]],
                    Vec3::new(0.0, 0.0, -1.0),
                    front,
                );
                quad(
                    &mut out,
                    [
                        Vec3::new(x0, y0, Z),
                        Vec3::new(x1, y0, Z),
                        Vec3::new(x1, y1, Z),
                        Vec3::new(x0, y1, Z),
                    ],
                    [[1.0, 0.0], [0.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
                    Vec3::new(0.0, 0.0, 1.0),
                    back,
                );
                if sy == height - 1 {
                    quad(
                        &mut out,
                        [
                            Vec3::new(x0, y0, -Z),
                            Vec3::new(x1, y0, -Z),
                            Vec3::new(x1, y0, Z),
                            Vec3::new(x0, y0, Z),
                        ],
                        [[0.0, 0.0], [1.0, 0.0], [1.0, edge_v], [0.0, edge_v]],
                        Vec3::new(0.0, 1.0, 0.0),
                        back,
                    );
                }
                if sy == 0 {
                    quad(
                        &mut out,
                        [
                            Vec3::new(x0, y1, Z),
                            Vec3::new(x1, y1, Z),
                            Vec3::new(x1, y1, -Z),
                            Vec3::new(x0, y1, -Z),
                        ],
                        [[0.0, 0.0], [1.0, 0.0], [1.0, edge_v], [0.0, edge_v]],
                        Vec3::new(0.0, -1.0, 0.0),
                        back,
                    );
                }
                if sx == width - 1 {
                    quad(
                        &mut out,
                        [
                            Vec3::new(x0, y0, Z),
                            Vec3::new(x0, y1, Z),
                            Vec3::new(x0, y1, -Z),
                            Vec3::new(x0, y0, -Z),
                        ],
                        [[edge_u, 0.0], [edge_u, 1.0], [0.0, 1.0], [0.0, 0.0]],
                        Vec3::new(-1.0, 0.0, 0.0),
                        back,
                    );
                }
                if sx == 0 {
                    quad(
                        &mut out,
                        [
                            Vec3::new(x1, y0, -Z),
                            Vec3::new(x1, y1, -Z),
                            Vec3::new(x1, y1, Z),
                            Vec3::new(x1, y0, Z),
                        ],
                        [[edge_u, 0.0], [edge_u, 1.0], [0.0, 1.0], [0.0, 0.0]],
                        Vec3::new(1.0, 0.0, 0.0),
                        back,
                    );
                }
            }
        }
        self.vertices = out;
    }
}
