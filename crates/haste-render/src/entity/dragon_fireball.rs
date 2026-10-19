//! The dragon fireball renderer (`DragonFireballRenderer`): the ender dragon's fireball as
//! a camera-facing sprite from `textures/entity/enderdragon/dragon_fireball.png`, at scale 2
//! and full-bright block light.

use super::*;

impl EntityRenderer {
    /// One dragon fireball, following vanilla's `DragonFireballRenderer.submit`: one quad
    /// pushed, scaled by 2 and turned to the camera (`buildQuad`'s local span x -0.5..0.5,
    /// y -0.25..0.75, the texture covering it whole), drawn through the cutout entity pass
    /// and lit at block light 15 (`getBlockLightLevel`), sky light kept.
    pub(super) fn emit_dragon_fireball(
        &mut self,
        e: &EntityInstance,
        camera: glam::DVec3,
        orientation: glam::Quat,
        light: (u8, u8),
        assets: &mut Assets,
    ) {
        static TEXTURE: std::sync::OnceLock<Identifier> = std::sync::OnceLock::new();
        let texture =
            TEXTURE.get_or_init(|| Identifier::minecraft("entity/enderdragon/dragon_fireball"));
        let rect = self.texture_rect(texture, assets);
        let light = (15, light.1);
        let center = (e.pos - camera).as_vec3();
        let s = 2.0_f32;
        // `buildQuad`: corners (x - 0.5, y - 0.25) for x, y in 0..1, u = x, v = 1 - y.
        let corners: [(f32, f32); 4] = [(-0.5, -0.25), (0.5, -0.25), (0.5, 0.75), (-0.5, 0.75)];
        let uvs: [(f32, f32); 4] = [(0.0, 1.0), (1.0, 1.0), (1.0, 0.0), (0.0, 0.0)];
        let mut pts = [Vec3::ZERO; 4];
        for i in 0..4 {
            pts[i] = center + orientation * Vec3::new(corners[i].0 * s, corners[i].1 * s, 0.0);
        }
        let uv = uvs.map(|t| {
            [
                rect[0] as f32 + t.0 * rect[2] as f32,
                rect[1] as f32 + t.1 * rect[3] as f32,
            ]
        });
        let r16 = rect.map(|v| v as u16);
        // `setColor(-1)`: opaque white.
        let color = [1.0, 1.0, 1.0, 1.0];
        let order: [usize; 6] = [0, 1, 2, 0, 2, 3];
        for i in order {
            self.vertices.push(EntityVertex {
                pos: pts[i].to_array(),
                uv: uv[i],
                color,
                rect: r16,
                light: [light.0 as u16, light.1 as u16, 0, 0],
            });
        }
    }
}
