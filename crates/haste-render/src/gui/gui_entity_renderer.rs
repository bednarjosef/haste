//! Entities drawn inside a GUI box (vanilla `GuiEntityRenderer`, the inventory player).

use glam::{Mat4, Vec3};
use haste_assets::Assets;

use super::*;

impl GuiRenderer {
    /// A living entity in a GUI box facing the viewer and turning toward the mouse (the
    /// inventory player). `size` is GUI pixels per block. Approximate: the body turns by
    /// atan(dx / 40) * 20 degrees and the head by twice that and pitches by atan(dy / 40) * 20,
    /// the shape of vanilla's inventory model; the feet stand near the box bottom.
    pub(super) fn entity_in_box(
        &mut self,
        o: [f32; 2],
        e: &haste_core::model::EntityInstance,
        b: [f32; 4],
        size: f32,
        mouse: [f32; 2],
        assets: &mut Assets,
    ) {
        let (texture, model_scale) = crate::entity::texture_and_scale(e);
        let (rect, _) = self.lookup(&TexKey::Texture(texture), assets);
        let cx = (b[0] + b[2]) / 2.0;
        let height = crate::entity::model_top(&e.model.root, 0.0) * model_scale / 16.0;
        let feet = (b[1] + b[3]) / 2.0 + height * size / 2.0;
        let dx = cx - mouse[0];
        let dy = feet - height * size * 0.85 - mouse[1];
        let f = (dx / 40.0).atan();
        let g = (dy / 40.0).atan();
        let mut pose = e.pose.clone();
        // The GUI preview overrides the head aim with the mouse-follow rotation.
        pose.set_rotation(
            haste_core::parts::HEAD,
            Vec3::new(-g * 20f32.to_radians(), f * 20f32.to_radians(), 0.0),
        );
        // GUI space: x right, y down, z toward the viewer. The model's front (-z) turns to +z.
        let m = Mat4::from_translation(Vec3::new(cx + o[0], feet + o[1], 0.0))
            * Mat4::from_scale(Vec3::new(size, -size, size))
            * Mat4::from_rotation_y(std::f32::consts::PI + f * 20f32.to_radians())
            * Mat4::from_scale(Vec3::new(-model_scale, -model_scale, model_scale) / 16.0);
        let mut tris = std::mem::take(&mut self.entity_scratch);
        tris.clear();
        // GUI inventory preview: full block and sky light (vanilla's GUI entity light
        // 0xF000F0), so the model renders at full brightness.
        crate::entity::emit_part_tinted(
            &mut tris,
            &e.model,
            &e.model.root,
            &pose,
            m,
            rect,
            (15, 15),
            0.0,
            [1.0; 3],
        );
        // Back faces off, then back to front.
        let mut order: Vec<(f32, usize)> = tris
            .chunks(3)
            .enumerate()
            .filter_map(|(i, t)| {
                let p = [
                    Vec3::from(t[0].pos),
                    Vec3::from(t[1].pos),
                    Vec3::from(t[2].pos),
                ];
                let n = (p[1] - p[0]).cross(p[2] - p[0]);
                (n.z > 0.0).then_some(((p[0].z + p[1].z + p[2].z) / 3.0, i))
            })
            .collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, i) in order {
            for v in &tris[i * 3..i * 3 + 3] {
                let c = (v.color[0] * 255.0).round() as u8;
                self.vertices.push(GuiVertex {
                    pos: [v.pos[0], v.pos[1]],
                    uv: v.uv,
                    color: [c, c, c, 255],
                    rect: v.rect,
                });
            }
        }
        self.entity_scratch = tris;
    }

    // ---- items ----
}
