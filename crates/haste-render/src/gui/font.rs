//! Text drawing: strings laid out glyph by glyph from the bitmap font pages and unifont
//! (vanilla `Font` / `GuiGraphics.drawString`).

use haste_assets::{for_each_formatted, Assets, GlyphKind, TextFormat};
use haste_core::gui::TextStyle;

use super::*;

impl GuiRenderer {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn text(
        &mut self,
        o: [f32; 2],
        x: f32,
        y: f32,
        text: &str,
        color: u32,
        shadow: bool,
        style: TextStyle,
        assets: &mut Assets,
    ) {
        // Vanilla (since 1.21.6) skips text whose colour has zero alpha.
        if color >> 24 == 0 || text.is_empty() {
            return;
        }
        let base = TextFormat {
            color: None,
            bold: style.bold,
            italic: style.italic,
            underline: style.underline,
            strikethrough: style.strikethrough,
            obfuscated: style.obfuscated,
        };
        if shadow {
            self.text_pass(o, x, y, text, color, base, true, assets);
        }
        self.text_pass(o, x, y, text, color, base, false, assets);
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn text_pass(
        &mut self,
        o: [f32; 2],
        x: f32,
        y: f32,
        text: &str,
        color: u32,
        base: TextFormat,
        shadow: bool,
        assets: &mut Assets,
    ) {
        let font = self.font.clone();
        let mut cursor = x;
        let alpha = color & 0xFF00_0000;
        let mut rng = self.rng;
        for_each_formatted(text, base, |ch, fmt| {
            let argb = fmt.color.map(|rgb| alpha | rgb).unwrap_or(color);
            let argb = if shadow { shadow_color(argb) } else { argb };
            let col = rgba(argb);
            let mut glyph = font.glyph(ch);
            if fmt.obfuscated && ch != ' ' {
                let pool = font.chars_with_advance(glyph.advance);
                if !pool.is_empty() {
                    rng ^= rng << 13;
                    rng ^= rng >> 17;
                    rng ^= rng << 5;
                    glyph = font.glyph(pool[rng as usize % pool.len()]);
                }
            }
            let sh = if shadow { glyph.shadow_offset } else { 0.0 };
            let gx = cursor + sh;
            let gy = y + sh;
            let passes: &[f32] = if fmt.bold {
                &[0.0, glyph.bold_offset]
            } else {
                &[0.0]
            };
            for &bold in passes {
                self.glyph(o, glyph.kind, gx + bold, gy, fmt.italic, col, assets);
            }
            let advance = glyph.advance + if fmt.bold { glyph.bold_offset } else { 0.0 };
            if fmt.strikethrough {
                self.solid(o, [gx - 1.0, gy + 3.5, gx + advance, gy + 4.5], col, col);
            }
            if fmt.underline {
                self.solid(o, [gx - 1.0, gy + 8.0, gx + advance, gy + 9.0], col, col);
            }
            cursor += advance;
        });
        self.rng = rng;
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn glyph(
        &mut self,
        o: [f32; 2],
        kind: GlyphKind,
        x: f32,
        y: f32,
        italic: bool,
        color: [u8; 4],
        assets: &mut Assets,
    ) {
        // Italic shears by 1 - 0.25 * (vertex y relative to the line top).
        let skew = |vy: f32| if italic { 1.0 - 0.25 * (vy - y) } else { 0.0 };
        let (rect, src, size, top) = match kind {
            GlyphKind::Empty => return,
            GlyphKind::Missing => {
                // A 5x8 outlined box.
                for r in [
                    [x, y, x + 5.0, y + 1.0],
                    [x, y + 7.0, x + 5.0, y + 8.0],
                    [x, y + 1.0, x + 1.0, y + 7.0],
                    [x + 4.0, y + 1.0, x + 5.0, y + 7.0],
                ] {
                    self.solid(o, r, color, color);
                }
                return;
            }
            GlyphKind::Bitmap {
                page,
                x: gx,
                y: gy,
                w,
                h,
                scale,
                top,
            } => {
                if w == 0 {
                    return;
                }
                let (rect, _) = self.lookup(&TexKey::FontPage(page), assets);
                (
                    rect,
                    [gx as f32, gy as f32, w as f32, h as f32],
                    [w as f32 * scale, h as f32 * scale],
                    top,
                )
            }
            GlyphKind::Hex { index } => {
                let Some(g) = self.font.hex_glyph(index) else {
                    return;
                };
                if g.trimmed == 0 {
                    return;
                }
                let (rect, _) = self.lookup(&TexKey::Hex(index), assets);
                (
                    rect,
                    [0.0, 0.0, rect[2] as f32, rect[3] as f32],
                    [rect[2] as f32 / 2.0, rect[3] as f32 / 2.0],
                    0.0,
                )
            }
        };
        let (x0, y0) = (x + o[0], y + top + o[1]);
        let (x1, y1) = (x0 + size[0], y0 + size[1]);
        let (st, sb) = (skew(y + top), skew(y + top + size[1]));
        let u0 = rect[0] as f32 + src[0];
        let v0 = rect[1] as f32 + src[1];
        let (u1, v1) = (u0 + src[2], v0 + src[3]);
        self.quad(
            [[x0 + st, y0], [x0 + sb, y1], [x1 + sb, y1], [x1 + st, y0]],
            [[u0, v0], [u0, v1], [u1, v1], [u1, v0]],
            [color; 4],
            rect,
        );
    }
}
