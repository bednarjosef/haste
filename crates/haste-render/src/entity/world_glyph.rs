//! World-space glyphs (signs, text displays): where a font glyph's bitmap sits in the entity
//! atlas and how big it is drawn, following the GUI text pass's geometry
//! (`GuiRenderer::glyph`, vanilla `BakedSheetGlyph`).

use super::*;

/// A glyph's sprite: its page rectangle in the entity atlas, the drawn size in text pixels,
/// the source rectangle normalized within the page, and the top offset from the line's y.
pub(super) struct GlyphSprite {
    pub rect: [u32; 4],
    pub size: [f32; 2],
    pub src: [f32; 4],
    pub top: f32,
}

impl EntityRenderer {
    /// The sprite of a glyph: bitmap glyphs draw their page rectangle at `y + top` scaled,
    /// unihex glyphs at half scale from their trimmed range. `None` for glyphs that draw
    /// nothing (spaces, and the missing-glyph box, which only the GUI pass draws today).
    pub(super) fn glyph_sprite(&mut self, kind: haste_assets::GlyphKind) -> Option<GlyphSprite> {
        let sprite = match kind {
            haste_assets::GlyphKind::Empty | haste_assets::GlyphKind::Missing => return None,
            haste_assets::GlyphKind::Bitmap {
                x,
                y,
                w,
                h,
                scale,
                top,
                ..
            } => {
                let r = self.glyph_page_rect(kind)?;
                GlyphSprite {
                    rect: r,
                    size: [w as f32 * scale, h as f32 * scale],
                    src: [
                        x as f32 / r[2] as f32,
                        y as f32 / r[3] as f32,
                        w as f32 / r[2] as f32,
                        h as f32 / r[3] as f32,
                    ],
                    top,
                }
            }
            haste_assets::GlyphKind::Hex { .. } => {
                let r = self.glyph_page_rect(kind)?;
                GlyphSprite {
                    rect: r,
                    size: [r[2] as f32 / 2.0, r[3] as f32 / 2.0],
                    src: [0.0, 0.0, 1.0, 1.0],
                    top: 0.0,
                }
            }
        };
        let empty = sprite.size[0] <= 0.0 || sprite.size[1] <= 0.0;
        (!empty && sprite.rect[2] != 0 && sprite.rect[3] != 0).then_some(sprite)
    }
}
