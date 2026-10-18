//! The sign renderer (`AbstractSignRenderer`): world-space sign text on both sides, four
//! centred lines each, glowing text full-bright in its dye colour.

use super::*;

impl EntityRenderer {
    /// One sign's text, following vanilla `AbstractSignRenderer.submitSignText`: both sides,
    /// up to four lines each, centred per line, glowing text full-bright in its dye colour and
    /// non-glowing text in the dark colour at the sign's light. The glyphs walk the default
    /// font like the GUI's text pass and land in the entity atlas (one draw with everything).
    pub(super) fn emit_sign_text(
        &mut self,
        inst: &crate::blockentity::BlockEntityInstance,
        hanging: bool,
        camera: glam::DVec3,
    ) {
        use crate::blockentity::{
            sign_dark_color, sign_glow_color, sign_line_metrics, sign_text_transform,
        };
        let Some(text) = inst.text.as_ref() else {
            return;
        };
        let (sky, block) = inst.light;
        let (line_height, _max_width) = sign_line_metrics(hanging);
        // `signMidpoint = 4 * lineHeight / 2`: the four lines are centred on the sign.
        let midpoint = 4.0 * line_height as f32 / 2.0;
        for (front, side) in [(true, &text.front), (false, &text.back)] {
            let m = sign_text_transform(hanging, inst.sign_angle, front, camera, inst.pos);
            let nm = Mat3::from_mat4(m).inverse().transpose();
            let glowing = side.glowing;
            let color = if glowing {
                sign_glow_color(side)
            } else {
                sign_dark_color(side)
            };
            // Vanilla `getLightCoords`: glowing text is full-bright (15728880: block 15,
            // sky 15), the rest use the sign's own light.
            let light = if glowing { (15, 15) } else { (block, sky) };
            self.emit_sign_side(&m, nm, side, color, light, line_height, midpoint);
        }
    }

    /// One side's four lines, centred each (`-width / 2`), at `i * lineHeight - midpoint`.
    #[allow(clippy::too_many_arguments)]
    fn emit_sign_side(
        &mut self,
        m: &Mat4,
        nm: Mat3,
        side: &crate::blockentity::SignSideText,
        color: u32,
        light: (u8, u8),
        line_height: i32,
        midpoint: f32,
    ) {
        let tint = |fmt_color: Option<u32>| {
            let c = fmt_color.unwrap_or(color);
            [
                ((c >> 16) & 0xFF) as f32 / 255.0,
                ((c >> 8) & 0xFF) as f32 / 255.0,
                (c & 0xFF) as f32 / 255.0,
            ]
        };
        let font = self.font.clone();
        let mut out = std::mem::take(&mut self.vertices);
        for (i, line) in side.lines.iter().enumerate() {
            if line.is_empty() {
                continue;
            }
            let w = font.width_f32(line);
            let y = i as f32 * line_height as f32 - midpoint;
            let mut cursor = -w / 2.0;
            haste_assets::for_each_formatted(line, Default::default(), |ch, fmt| {
                let glyph = font.glyph(ch);
                let t = tint(fmt.color);
                self.sign_glyph_quad(&mut out, *m, nm, glyph, cursor, y, t, light);
                cursor += glyph.advance + if fmt.bold { glyph.bold_offset } else { 0.0 };
            });
        }
        self.vertices = out;
    }

    /// One glyph quad in text space (x right, y down, z toward the viewer), following the GUI
    /// text pass's geometry (`GuiRenderer::glyph`): bitmap glyphs draw their page rectangle at
    /// `y + top` scaled, unihex glyphs at half scale from their trimmed range. `uv` is
    /// normalized 0..1 within `rect` (the page image), like `push_world_quad` maps it.
    #[allow(clippy::too_many_arguments)]
    fn sign_glyph_quad(
        &mut self,
        out: &mut Vec<EntityVertex>,
        m: Mat4,
        nm: Mat3,
        glyph: haste_assets::Glyph,
        x: f32,
        y: f32,
        tint: [f32; 3],
        light: (u8, u8),
    ) {
        // The drawn size and the source rectangle, normalized within the page image.
        let (rect, draw, src, top) = match glyph.kind {
            haste_assets::GlyphKind::Empty => return,
            // Vanilla's missing-glyph box; drawn only in the GUI pass today.
            haste_assets::GlyphKind::Missing => return,
            haste_assets::GlyphKind::Bitmap {
                x: px,
                y: py,
                w,
                h,
                scale,
                top,
                ..
            } => {
                let Some(r) = self.glyph_page_rect(glyph.kind) else {
                    return;
                };
                (
                    r,
                    [w as f32 * scale, h as f32 * scale],
                    [
                        px as f32 / r[2] as f32,
                        py as f32 / r[3] as f32,
                        w as f32 / r[2] as f32,
                        h as f32 / r[3] as f32,
                    ],
                    top,
                )
            }
            haste_assets::GlyphKind::Hex { .. } => {
                let Some(r) = self.glyph_page_rect(glyph.kind) else {
                    return;
                };
                (
                    r,
                    [r[2] as f32 / 2.0, r[3] as f32 / 2.0],
                    [0.0, 0.0, 1.0, 1.0],
                    0.0,
                )
            }
        };
        if draw[0] <= 0.0 || draw[1] <= 0.0 || rect[2] == 0 || rect[3] == 0 {
            return;
        }
        let (x0, y0) = (x, y + top);
        let (x1, y1) = (x0 + draw[0], y0 + draw[1]);
        let corners = [
            Vec3::new(x0, y0, 0.0),
            Vec3::new(x0, y1, 0.0),
            Vec3::new(x1, y1, 0.0),
            Vec3::new(x1, y0, 0.0),
        ];
        let uv = [
            [src[0], src[1]],
            [src[0], src[1] + src[3]],
            [src[0] + src[2], src[1] + src[3]],
            [src[0] + src[2], src[1]],
        ];
        push_world_quad(
            out,
            m,
            nm,
            corners,
            uv,
            Vec3::new(0.0, 0.0, 1.0),
            tint,
            light,
            rect,
        );
    }
}
