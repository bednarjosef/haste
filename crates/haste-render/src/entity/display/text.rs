//! The text display renderer (`DisplayRenderer.TextDisplayRenderer`): the text split into
//! lines at the line width (`TextDisplay.cacheDisplay`, kept per entity until the text
//! changes), the background box, and each line aligned and drawn glyph by glyph like
//! vanilla's `Font.PreparedTextBuilder` and `BakedSheetGlyph` (drop shadow, bold, italic,
//! underline and strikethrough), into the polygon-offset or see-through text pass.

use std::ops::Range;

use haste_assets::TextFormat;
use haste_core::display::TextDisplayDecoration;
use haste_core::Component;

use crate::entity::item_display::argb_linear;

use super::*;
use crate::entity::world_glyph::GlyphSprite;
use crate::gui::{string_decomposer, string_splitter};

/// `9 + lineSpacing`: the distance between two lines' tops.
pub(crate) const LINE_HEIGHT: f32 = 10.0;
/// The layering in text pixels toward the viewer: the background behind the glyphs
/// (`Font.UNDER_EFFECT_DEPTH`), a shadowed glyph above its drop shadow and bold's second copy
/// a hair above its first (`BakedSheetGlyph.renderChar`), underline and strikethrough above
/// the glyphs (`Font.PreparedTextBuilder.accept`).
const BACKGROUND_DEPTH: f32 = -0.01;
const SHADOW_DEPTH: f32 = 0.03;
const BOLD_DEPTH: f32 = 0.001;
const EFFECT_DEPTH: f32 = 0.01;
/// `BakedSheetGlyph.extraThickness` for bold glyphs.
const BOLD_THICKNESS: f32 = 0.1;

/// `TextDisplay.CachedInfo`: the split lines (ranges into the styled characters) with their
/// widths, and the widest.
pub(crate) struct CachedText {
    text: Arc<Component>,
    line_width: i32,
    pub(crate) chars: Vec<(char, TextFormat)>,
    pub(crate) lines: Vec<(Range<usize>, i32)>,
    pub(crate) width: i32,
    seen: u64,
}

impl CachedText {
    /// `TextDisplayRenderer.splitLines`: `Font.split` at `line_width`, each line's
    /// `Font.width`, and the maximum.
    pub(crate) fn build(
        text: Arc<Component>,
        line_width: i32,
        font: &haste_assets::BitmapFont,
    ) -> Self {
        let mut chars = Vec::new();
        string_decomposer::flatten(&text, TextFormat::default(), &mut chars);
        let width_of = |ch, style| string_splitter::char_width(font, ch, style);
        let mut ranges = Vec::new();
        string_splitter::split_lines(&chars, line_width, width_of, &mut ranges);
        let lines: Vec<(Range<usize>, i32)> = ranges
            .into_iter()
            .map(|r| {
                let w = string_splitter::width(&chars[r.clone()], width_of);
                (r, w)
            })
            .collect();
        let width = lines.iter().map(|(_, w)| *w).max().unwrap_or(0);
        CachedText {
            text,
            line_width,
            chars,
            lines,
            width,
            seen: 0,
        }
    }
}

/// The text displays' line caches by entity (`clientDisplayCache`, rebuilt when the render
/// state's text `Arc` changes; entities not drawn for a frame are dropped).
#[derive(Default)]
pub(crate) struct LayoutCache {
    entries: HashMap<EntityId, CachedText>,
    frame: u64,
}

impl LayoutCache {
    pub(crate) fn get(
        &mut self,
        id: EntityId,
        text: &Arc<Component>,
        line_width: i32,
        font: &haste_assets::BitmapFont,
    ) -> &CachedText {
        let frame = self.frame;
        let entry = self
            .entries
            .entry(id)
            .and_modify(|c| {
                if !Arc::ptr_eq(&c.text, text) || c.line_width != line_width {
                    *c = CachedText::build(text.clone(), line_width, font);
                }
            })
            .or_insert_with(|| CachedText::build(text.clone(), line_width, font));
        entry.seen = frame;
        entry
    }

    pub(super) fn end_frame(&mut self) {
        let frame = self.frame;
        self.entries.retain(|_, c| c.seen == frame);
        self.frame += 1;
    }
}

/// `Display.TextDisplay.getAlign` and the line offset `submitInner` gives it.
pub(crate) fn line_offset(flags: u8, width: i32, line_width: i32) -> f32 {
    if flags & TextDisplayDecoration::FLAG_ALIGN_LEFT != 0 {
        0.0
    } else if flags & TextDisplayDecoration::FLAG_ALIGN_RIGHT != 0 {
        (width - line_width) as f32
    } else {
        width as f32 / 2.0 - line_width as f32 / 2.0
    }
}

/// The background colour `submitInner` draws: the options' text background (the default
/// `getBackgroundOpacity(0.25)`, alpha only) when the default-background flag is set, else
/// the synched colour.
pub(crate) fn background_color(t: &TextDisplayDecoration) -> u32 {
    if t.flags & TextDisplayDecoration::FLAG_USE_DEFAULT_BACKGROUND != 0 {
        ((0.25f32 * 255.0) as u32) << 24
    } else {
        t.background
    }
}

/// The text pose `submitInner` builds on the display's: half a turn about Y, scale -0.025,
/// then the block of text centred horizontally with its bottom on the origin.
pub(crate) fn text_pose(pose: Mat4, width: i32, lines: usize) -> Mat4 {
    let height = lines as f32 * LINE_HEIGHT - 1.0;
    pose * Mat4::from_rotation_y(std::f32::consts::PI)
        * Mat4::from_scale(Vec3::splat(-0.025))
        * Mat4::from_translation(Vec3::new(1.0 - width as f32 / 2.0, -height, 0.0))
}

/// `ARGB.scaleRGB(color, 0.25)`: the drop shadow's colour, alpha kept.
pub(crate) fn shadow_color(color: u32) -> u32 {
    let scale = |shift: u32| ((((color >> shift) & 0xFF) as f32 * 0.25) as u32) << shift;
    (color & 0xFF00_0000) | scale(16) | scale(8) | scale(0)
}

/// `ARGB.color(alpha, rgb)` for a style colour over the line colour (`getTextColor`).
pub(crate) fn text_color(base: u32, style: Option<u32>) -> u32 {
    style.map_or(base, |rgb| (base & 0xFF00_0000) | (rgb & 0xFF_FFFF))
}

/// Where one text display's quads go: the pose, the (block, sky) light and the white texel
/// for backgrounds and effects.
#[derive(Clone, Copy)]
struct TextTarget {
    m: Mat4,
    light: (u8, u8),
    white: [u32; 4],
}

impl TextTarget {
    /// One quad (corners in vanilla's order, already counter-clockwise from the readable
    /// side; uv in atlas texels) with an ARGB colour. Nothing when the colour's alpha is
    /// under the text shader's 0.1 cut, where every fragment would be discarded.
    fn quad(
        &self,
        out: &mut Vec<EntityVertex>,
        p: [Vec3; 4],
        uv: [[f32; 2]; 4],
        color: u32,
        rect: [u32; 4],
    ) {
        let alpha = (color >> 24) as f32 / 255.0;
        if alpha < 0.1 {
            return;
        }
        let [r, g, b] = argb_linear(color);
        let rect = rect.map(|v| v as u16);
        for i in [0usize, 1, 2, 0, 2, 3] {
            out.push(EntityVertex {
                pos: self.m.transform_point3(p[i]).to_array(),
                uv: uv[i],
                color: [r, g, b, alpha],
                rect,
                light: [self.light.0 as u16, self.light.1 as u16, 0, 0],
            });
        }
    }

    /// `BakedSheetGlyph.buildEffect`: a solid rectangle in the white texel, shifted by
    /// `offset` on both axes.
    fn effect(
        &self,
        out: &mut Vec<EntityVertex>,
        (x0, y0, x1, y1): (f32, f32, f32, f32),
        offset: f32,
        z: f32,
        color: u32,
    ) {
        let (x0, y0, x1, y1) = (x0 + offset, y0 + offset, x1 + offset, y1 + offset);
        let p = [
            Vec3::new(x0, y1, z),
            Vec3::new(x1, y1, z),
            Vec3::new(x1, y0, z),
            Vec3::new(x0, y0, z),
        ];
        let c = [self.white[0] as f32 + 0.5, self.white[1] as f32 + 0.5];
        self.quad(out, p, [c; 4], color, self.white);
    }

    /// `BakedSheetGlyph.renderEffect`: the effect's drop shadow, then the effect 0.03 above.
    fn effect_with_shadow(
        &self,
        out: &mut Vec<EntityVertex>,
        r: (f32, f32, f32, f32),
        color: u32,
        shadow: Option<(u32, f32)>,
    ) {
        let mut z = EFFECT_DEPTH;
        if let Some((shadow_color, offset)) = shadow {
            self.effect(out, r, offset, z, shadow_color);
            z += SHADOW_DEPTH;
        }
        self.effect(out, r, 0.0, z, color);
    }

    /// `BakedSheetGlyph.render`: one copy of a glyph at `(x, y, z)`, italic sheared by
    /// `1 - 0.25 * y` at its top and bottom, bold grown by 0.1 on every side.
    fn glyph(
        &self,
        out: &mut Vec<EntityVertex>,
        sprite: &GlyphSprite,
        (x, y, z): (f32, f32, f32),
        color: u32,
        italic: bool,
        bold: bool,
    ) {
        let up = sprite.top;
        let down = sprite.top + sprite.size[1];
        let (x0, x1) = (x, x + sprite.size[0]);
        let (y0, y1) = (y + up, y + down);
        let (shear_top, shear_bottom) = if italic {
            (1.0 - 0.25 * up, 1.0 - 0.25 * down)
        } else {
            (0.0, 0.0)
        };
        let t = if bold { BOLD_THICKNESS } else { 0.0 };
        let p = [
            Vec3::new(x0 + shear_top - t, y0 - t, z),
            Vec3::new(x0 + shear_bottom - t, y1 + t, z),
            Vec3::new(x1 + shear_bottom + t, y1 + t, z),
            Vec3::new(x1 + shear_top + t, y0 - t, z),
        ];
        let [rx, ry, rw, rh] = sprite.rect.map(|v| v as f32);
        let [sx, sy, sw, sh] = sprite.src;
        let (u0, v0) = (rx + sx * rw, ry + sy * rh);
        let (u1, v1) = (u0 + sw * rw, v0 + sh * rh);
        let uv = [[u0, v0], [u0, v1], [u1, v1], [u1, v0]];
        self.quad(out, p, uv, color, sprite.rect);
    }
}

impl EntityRenderer {
    /// One text display (`TextDisplayRenderer.submitInner` and the text feature renderer):
    /// the background box, then each line aligned in the block of lines, its glyphs and then
    /// its underline and strikethrough effects (`Font.PreparedTextBuilder.visit`). `pose` is
    /// the display's pose and `light` its (block, sky) light.
    pub(super) fn emit_text_display(
        &mut self,
        id: EntityId,
        pose: Mat4,
        t: &TextDisplayDecoration,
        light: (u8, u8),
    ) {
        let font = Arc::clone(&self.font);
        let see_through = t.flags & TextDisplayDecoration::FLAG_SEE_THROUGH != 0;
        let shadow = t.flags & TextDisplayDecoration::FLAG_SHADOW != 0;
        // `textOpacity << 24 | 0xFFFFFF`: white at the (byte) opacity.
        let color = (u32::from(t.opacity as u8) << 24) | 0xFF_FFFF;
        let background = background_color(t);
        let white = self.white_rect();
        let mut out = self.displays.text.take(see_through);
        let mut layouts = std::mem::take(&mut self.displays.layouts);
        let cached = layouts.get(id, &t.text, t.line_width, &font);
        let target = TextTarget {
            m: text_pose(pose, cached.width, cached.lines.len()),
            light,
            white,
        };
        if background != 0 {
            let height = cached.lines.len() as f32 * LINE_HEIGHT - 1.0;
            let r = (-1.0, -1.0, cached.width as f32, height);
            target.effect(&mut out, r, 0.0, BACKGROUND_DEPTH, background);
        }
        let mut y = 0.0;
        for (range, line_width) in &cached.lines {
            let x = line_offset(t.flags, cached.width, *line_width);
            let line = &cached.chars[range.clone()];
            self.emit_text_line(&mut out, &target, &font, line, (x, y), color, shadow);
            y += LINE_HEIGHT;
        }
        self.displays.layouts = layouts;
        self.displays.text.put(see_through, out);
    }

    /// One line (`Font.PreparedTextBuilder.accept` per character, then `visit`): every
    /// glyph's drop shadow, shadow bold copy, glyph and bold copy (`renderChar`), then the
    /// strikethrough and underline effects, which start a pixel early on the first character.
    #[allow(clippy::too_many_arguments)]
    fn emit_text_line(
        &mut self,
        out: &mut Vec<EntityVertex>,
        target: &TextTarget,
        font: &haste_assets::BitmapFont,
        line: &[(char, TextFormat)],
        (start, y): (f32, f32),
        base: u32,
        shadow: bool,
    ) {
        let mut x = start;
        for &(ch, style) in line {
            let glyph = font.glyph(ch);
            let color = text_color(base, style.color);
            let bold_offset = if style.bold { glyph.bold_offset } else { 0.0 };
            if let Some(sprite) = self.glyph_sprite(glyph.kind) {
                let (italic, bold, so) = (style.italic, style.bold, glyph.shadow_offset);
                let mut z = 0.0;
                if shadow {
                    let sc = shadow_color(color);
                    target.glyph(out, &sprite, (x + so, y + so, 0.0), sc, italic, bold);
                    if bold {
                        let at = (x + bold_offset + so, y + so, BOLD_DEPTH);
                        target.glyph(out, &sprite, at, sc, italic, true);
                    }
                    z = SHADOW_DEPTH;
                }
                target.glyph(out, &sprite, (x, y, z), color, italic, bold);
                if bold {
                    let at = (x + bold_offset, y, z + BOLD_DEPTH);
                    target.glyph(out, &sprite, at, color, italic, true);
                }
            }
            x += glyph.advance + bold_offset;
        }
        let mut x = start;
        for (i, &(ch, style)) in line.iter().enumerate() {
            let glyph = font.glyph(ch);
            let advance = glyph.advance + if style.bold { glyph.bold_offset } else { 0.0 };
            if style.strikethrough || style.underline {
                let color = text_color(base, style.color);
                let shadow = shadow.then(|| (shadow_color(color), glyph.shadow_offset));
                let x0 = if i == 0 { x - 1.0 } else { x };
                if style.strikethrough {
                    let r = (x0, y + 4.5 - 1.0, x + advance, y + 4.5);
                    target.effect_with_shadow(out, r, color, shadow);
                }
                if style.underline {
                    let r = (x0, y + 9.0 - 1.0, x + advance, y + 9.0);
                    target.effect_with_shadow(out, r, color, shadow);
                }
            }
            x += advance;
        }
    }
}
