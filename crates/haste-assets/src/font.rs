//! The default font: `font/default.json` resolved through its `reference` providers into
//! `space`, `bitmap` and `unihex` providers, first provider wins per code point (vanilla's
//! provider priority). Widths follow the observable vanilla rules documented on minecraft.wiki
//! ("Font" article): a bitmap glyph's width is its rightmost non-empty column + 1 (in cell
//! pixels, scaled by `height / cell height`), its advance is that width rounded + 1; a unihex
//! glyph is drawn at half scale and advances by half its trimmed width + 1; `space` providers
//! give advances directly. Legacy `§` formatting codes are understood by both measuring and
//! drawing, like vanilla's string drawing.

use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::Deserialize;

use haste_core::Identifier;

use crate::assets::Assets;

/// One glyph: how far it advances and what (if anything) it draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    /// Horizontal advance in GUI pixels (fractional for unihex glyphs).
    pub advance: f32,
    /// Extra offset of the second draw for bold text (1 for bitmap glyphs, 0.5 for unihex).
    pub bold_offset: f32,
    /// Drop-shadow offset (1 for bitmap glyphs, 0.5 for unihex).
    pub shadow_offset: f32,
    pub kind: GlyphKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GlyphKind {
    /// Advance only (space provider).
    Empty,
    /// A rectangle of a bitmap page, `w` x `h` in page pixels, drawn at `w * scale` x
    /// `h * scale` with its top at `y + top` (top = 7 - ascent).
    Bitmap {
        page: u16,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
        scale: f32,
        top: f32,
    },
    /// A unihex glyph (index into `BitmapFont::hex_glyph`), drawn at half scale from its
    /// trimmed column range.
    Hex { index: u32 },
    /// No provider has the code point: vanilla draws its missing-glyph box.
    Missing,
}

impl Glyph {
    pub const MISSING: Glyph = Glyph {
        advance: 6.0,
        bold_offset: 1.0,
        shadow_offset: 1.0,
        kind: GlyphKind::Missing,
    };
}

/// A unihex glyph bitmap: 16 rows, `width` columns (8, 16, ...), leftmost pixel is bit
/// `width - 1` of each row. `left..left + trimmed` is the drawn column range.
#[derive(Clone, Debug, PartialEq)]
pub struct HexGlyph {
    pub width: u8,
    pub left: u8,
    pub trimmed: u8,
    pub rows: [u32; 16],
}

impl HexGlyph {
    pub fn pixel(&self, x: u32, y: u32) -> bool {
        x < self.width as u32 && (self.rows[y as usize] >> (self.width as u32 - 1 - x)) & 1 != 0
    }
}

/// The resolved default font. `TextMetrics` is implemented over it (`haste_core::gui`).
pub struct BitmapFont {
    glyphs: HashMap<char, Glyph>,
    /// Code points 0..128 mirrored from `glyphs` by `insert` (the only write path), so table
    /// and map always agree: nearly every character the GUI draws (ASCII) avoids the hash
    /// probe.
    ascii: [Option<Glyph>; 128],
    pages: Vec<Arc<image::RgbaImage>>,
    hex: Vec<HexGlyph>,
    /// Bitmap glyphs grouped by integer advance (vanilla's obfuscated text picks a random
    /// glyph of the same width).
    by_advance: HashMap<u32, Vec<char>>,
}

impl Default for BitmapFont {
    fn default() -> Self {
        Self {
            glyphs: HashMap::new(),
            ascii: [None; 128],
            pages: Vec::new(),
            hex: Vec::new(),
            by_advance: HashMap::new(),
        }
    }
}

impl BitmapFont {
    /// The glyph for a code point, `Glyph::MISSING` when no provider has it.
    pub fn glyph(&self, ch: char) -> Glyph {
        if ch as u32 <= 127 {
            return self.ascii[ch as usize].unwrap_or(Glyph::MISSING);
        }
        self.glyphs.get(&ch).copied().unwrap_or(Glyph::MISSING)
    }

    /// `Some(glyph)` only when a provider defines the code point.
    pub fn defined_glyph(&self, ch: char) -> Option<Glyph> {
        if ch as u32 <= 127 {
            return self.ascii[ch as usize];
        }
        self.glyphs.get(&ch).copied()
    }

    /// Advance in whole pixels of a character (rounded up), None when undefined.
    pub fn char_width(&self, ch: char) -> Option<u32> {
        self.glyphs.get(&ch).map(|g| g.advance.ceil() as u32)
    }

    pub fn page(&self, index: usize) -> Option<&Arc<image::RgbaImage>> {
        self.pages.get(index)
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn hex_glyph(&self, index: u32) -> Option<&HexGlyph> {
        self.hex.get(index as usize)
    }

    /// Number of defined code points.
    pub fn len(&self) -> usize {
        self.glyphs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    /// Characters whose bitmap glyph advances exactly `advance` pixels (obfuscated text).
    pub fn chars_with_advance(&self, advance: f32) -> &[char] {
        if advance.fract() != 0.0 {
            return &[];
        }
        self.by_advance
            .get(&(advance as u32))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Exact vanilla string width as a float: § codes are not drawn, bold adds the bold
    /// offset per character.
    pub fn width_f32(&self, text: &str) -> f32 {
        let mut total = 0.0;
        for_each_formatted(text, TextFormat::default(), |ch, fmt| {
            let g = self.glyph(ch);
            total += g.advance + if fmt.bold { g.bold_offset } else { 0.0 };
        });
        total
    }

    fn insert(&mut self, ch: char, glyph: Glyph) {
        if self.glyphs.contains_key(&ch) {
            return; // earlier providers win
        }
        if let GlyphKind::Bitmap { .. } = glyph.kind {
            if glyph.advance.fract() == 0.0 && ch != ' ' {
                self.by_advance
                    .entry(glyph.advance as u32)
                    .or_default()
                    .push(ch);
            }
        }
        self.glyphs.insert(ch, glyph);
        if (ch as u32) < 128 {
            self.ascii[ch as usize] = Some(glyph);
        }
    }

    fn add_bitmap_provider(&mut self, p: &BitmapProviderRaw, image: image::RgbaImage) {
        let rows = p.chars.len() as u32;
        let cols = p
            .chars
            .iter()
            .map(|r| r.chars().count() as u32)
            .max()
            .unwrap_or(0);
        if rows == 0 || cols == 0 {
            return;
        }
        let (img_w, img_h) = image.dimensions();
        let cell_w = img_w / cols;
        let cell_h = img_h / rows;
        if cell_w == 0 || cell_h == 0 {
            return;
        }
        let height = p.height.unwrap_or(8) as f32;
        let scale = height / cell_h as f32;
        let ascent = p.ascent.unwrap_or(7) as f32;
        let page = self.pages.len() as u16;
        for (row, line) in p.chars.iter().enumerate() {
            for (col, ch) in line.chars().enumerate() {
                if ch == '\0' {
                    continue;
                }
                let x = col as u32 * cell_w;
                let y = row as u32 * cell_h;
                let width = measured_width(&image, x, y, cell_w, cell_h);
                let glyph = Glyph {
                    advance: (0.5 + width as f32 * scale).floor() + 1.0,
                    bold_offset: 1.0,
                    shadow_offset: 1.0,
                    kind: GlyphKind::Bitmap {
                        page,
                        x: x as u16,
                        y: y as u16,
                        w: width as u16,
                        h: cell_h as u16,
                        scale,
                        top: 7.0 - ascent,
                    },
                };
                self.insert(ch, glyph);
            }
        }
        self.pages.push(Arc::new(image));
    }

    fn add_unihex_provider(&mut self, hex_zip: &[u8], overrides: &[SizeOverride]) -> Result<()> {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(hex_zip))?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i)?;
            if !entry.name().ends_with(".hex") {
                continue;
            }
            let mut text = String::with_capacity(entry.size() as usize);
            entry.read_to_string(&mut text)?;
            for line in text.lines() {
                let Some((cp, data)) = line.split_once(':') else {
                    continue;
                };
                let Some(ch) = u32::from_str_radix(cp, 16).ok().and_then(char::from_u32) else {
                    continue;
                };
                let Some(mut glyph) = parse_hex_glyph(data) else {
                    continue;
                };
                if let Some(o) = overrides.iter().find(|o| o.from <= ch && ch <= o.to) {
                    glyph.left = o.left.min(glyph.width.saturating_sub(1));
                    glyph.trimmed = o.right.saturating_sub(o.left).saturating_add(1);
                }
                let index = self.hex.len() as u32;
                let advance = glyph.trimmed as f32 / 2.0 + 1.0;
                self.hex.push(glyph);
                self.insert(
                    ch,
                    Glyph {
                        advance,
                        bold_offset: 0.5,
                        shadow_offset: 0.5,
                        kind: GlyphKind::Hex { index },
                    },
                );
            }
        }
        Ok(())
    }
}

/// Rightmost non-transparent column + 1 within the cell (0 for an empty cell).
fn measured_width(image: &image::RgbaImage, x: u32, y: u32, cell_w: u32, cell_h: u32) -> u32 {
    for cx in (0..cell_w).rev() {
        for cy in 0..cell_h {
            if image.get_pixel(x + cx, y + cy).0[3] != 0 {
                return cx + 1;
            }
        }
    }
    0
}

fn parse_hex_glyph(data: &str) -> Option<HexGlyph> {
    let width = match data.len() {
        32 => 8,
        48 => 12,
        64 => 16,
        96 => 24,
        _ => return None,
    };
    let digits_per_row = width / 4;
    let mut rows = [0u32; 16];
    for (r, row) in rows.iter_mut().enumerate() {
        let s = data.get(r * digits_per_row..(r + 1) * digits_per_row)?;
        *row = u32::from_str_radix(s, 16).ok()?;
    }
    let all = rows.iter().fold(0u32, |a, r| a | r);
    let (left, trimmed) = if all == 0 {
        (0, 0)
    } else {
        let w = width as u32;
        // Leftmost pixel is bit w-1.
        let leftmost = w - 1 - (31 - all.leading_zeros());
        let rightmost = w - 1 - all.trailing_zeros();
        (leftmost as u8, (rightmost - leftmost + 1) as u8)
    };
    Some(HexGlyph {
        width: width as u8,
        left,
        trimmed,
        rows,
    })
}

/// Formatting state while walking a string with legacy `§` codes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextFormat {
    /// RGB override from a colour code (None = the caller's colour).
    pub color: Option<u32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub obfuscated: bool,
}

/// The sixteen legacy chat colours (`§0`..`§f`), RGB.
pub const LEGACY_COLORS: [u32; 16] = [
    0x000000, 0x0000AA, 0x00AA00, 0x00AAAA, 0xAA0000, 0xAA00AA, 0xFFAA00, 0xAAAAAA, 0x555555,
    0x5555FF, 0x55FF55, 0x55FFFF, 0xFF5555, 0xFF55FF, 0xFFFF55, 0xFFFFFF,
];

/// Walks `text` calling `f` for every drawn character with its formatting. `§x` codes are
/// consumed: a colour resets the style flags (legacy behaviour), `§r` resets to `base`,
/// `§k..§o` set obfuscated/bold/strikethrough/underline/italic. A trailing `§` is dropped.
pub fn for_each_formatted(text: &str, base: TextFormat, mut f: impl FnMut(char, TextFormat)) {
    let mut fmt = base;
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '§' {
            f(ch, fmt);
            continue;
        }
        let Some(code) = chars.next() else { break };
        match code.to_ascii_lowercase() {
            c @ ('0'..='9' | 'a'..='f') => {
                let i = c.to_digit(16).unwrap() as usize;
                fmt = TextFormat {
                    color: Some(LEGACY_COLORS[i]),
                    ..base
                };
                fmt.bold = false;
                fmt.italic = false;
                fmt.underline = false;
                fmt.strikethrough = false;
                fmt.obfuscated = false;
            }
            'k' => fmt.obfuscated = true,
            'l' => fmt.bold = true,
            'm' => fmt.strikethrough = true,
            'n' => fmt.underline = true,
            'o' => fmt.italic = true,
            'r' => fmt = base,
            _ => {}
        }
    }
}

/// `haste_core::gui::TextMetrics` over the font: vanilla `Font.width` (ceil of the float sum
/// of advances), line height 9.
impl haste_core::gui::TextMetrics for BitmapFont {
    fn width(&self, text: &str) -> i32 {
        self.width_f32(text).ceil() as i32
    }
}

// ---- loading ----

#[derive(Deserialize)]
struct FontFile {
    #[serde(default)]
    providers: Vec<FontProviderRaw>,
}

#[derive(Deserialize)]
struct FontProviderRaw {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    filter: HashMap<String, bool>,
    #[serde(flatten)]
    rest: serde_json::Value,
}

#[derive(Deserialize)]
struct BitmapProviderRaw {
    file: String,
    height: Option<u32>,
    ascent: Option<i32>,
    chars: Vec<String>,
}

#[derive(Deserialize)]
struct UnihexProviderRaw {
    hex_file: String,
    #[serde(default)]
    size_overrides: Vec<SizeOverride>,
}

#[derive(Deserialize)]
struct SizeOverride {
    from: char,
    to: char,
    left: u8,
    right: u8,
}

impl Assets {
    /// `font/default.json` resolved through its `reference` providers (space, the bitmap pages,
    /// and unifont from the assets index when it is available).
    pub fn font_default(&mut self) -> Result<BitmapFont> {
        let mut font = BitmapFont::default();
        self.load_font_into(&Identifier::minecraft("default"), &mut font, 0)?;
        Ok(font)
    }

    fn load_font_into(&mut self, id: &Identifier, font: &mut BitmapFont, depth: u32) -> Result<()> {
        if depth > 8 {
            anyhow::bail!("font reference chain too deep at {id}");
        }
        let key = crate::assets::asset_key(id, "font", ".json");
        let bytes = self.read_asset(&key)?;
        let file: FontFile =
            serde_json::from_slice(&bytes).with_context(|| format!("parsing {key}"))?;
        for p in file.providers {
            // Filters name options (`uniform`, `jp`) that default to off: a provider is used
            // when every filter entry matches "off".
            if p.filter.values().any(|v| *v) {
                continue;
            }
            match p.kind.trim_start_matches("minecraft:") {
                "reference" => {
                    let Some(r) = p.rest.get("id").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    self.load_font_into(&Identifier::parse(r)?, font, depth + 1)?;
                }
                "space" => {
                    let advances: HashMap<String, f32> = p
                        .rest
                        .get("advances")
                        .cloned()
                        .map(serde_json::from_value)
                        .transpose()?
                        .unwrap_or_default();
                    for (s, adv) in advances {
                        if let Some(ch) = s.chars().next() {
                            font.insert(
                                ch,
                                Glyph {
                                    advance: adv,
                                    bold_offset: 1.0,
                                    shadow_offset: 1.0,
                                    kind: GlyphKind::Empty,
                                },
                            );
                        }
                    }
                }
                "bitmap" => {
                    let raw: BitmapProviderRaw = serde_json::from_value(p.rest)?;
                    // `file` is a full texture id with `.png` (minecraft:font/ascii.png).
                    let fid = Identifier::parse(&raw.file)?;
                    let key = format!("assets/{}/textures/{}", fid.namespace(), fid.path());
                    let bytes = self.read_asset(&key)?;
                    let image = image::load_from_memory(&bytes)
                        .with_context(|| format!("decoding {key}"))?
                        .to_rgba8();
                    font.add_bitmap_provider(&raw, image);
                }
                "unihex" => {
                    let raw: UnihexProviderRaw = serde_json::from_value(p.rest)?;
                    let hid = Identifier::parse(&raw.hex_file)?;
                    let key = format!("assets/{}/{}", hid.namespace(), hid.path());
                    match self.read_asset(&key) {
                        Ok(bytes) => font.add_unihex_provider(&bytes, &raw.size_overrides)?,
                        Err(e) => tracing::warn!("unihex font {key} unavailable: {e:#}"),
                    }
                }
                other => tracing::debug!("font provider type {other} not implemented"),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use haste_core::gui::TextMetrics;

    fn tiny_font() -> BitmapFont {
        // 2 cols x 1 row of 8x8 cells: 'a' 3 px wide, 'b' empty.
        let mut img = image::RgbaImage::new(16, 8);
        for y in 0..8 {
            for x in 0..3 {
                img.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }
        let mut f = BitmapFont::default();
        f.insert(
            ' ',
            Glyph {
                advance: 4.0,
                bold_offset: 1.0,
                shadow_offset: 1.0,
                kind: GlyphKind::Empty,
            },
        );
        let raw = BitmapProviderRaw {
            file: String::new(),
            height: None,
            ascent: Some(7),
            chars: vec!["ab".into()],
        };
        f.add_bitmap_provider(&raw, img);
        f.insert(
            'λ',
            Glyph {
                advance: 5.0,
                ..Glyph::MISSING
            },
        );
        f
    }

    #[test]
    fn bitmap_advance_is_width_plus_one() {
        let f = tiny_font();
        assert_eq!(f.glyph('a').advance, 4.0);
        assert_eq!(f.glyph('b').advance, 1.0);
        assert_eq!(f.width("a a"), 12);
        assert!(matches!(
            f.glyph('a').kind,
            GlyphKind::Bitmap { w: 3, h: 8, .. }
        ));
    }

    #[test]
    fn formatting_codes_are_not_measured_and_bold_adds_one() {
        let f = tiny_font();
        assert_eq!(f.width("§ca"), 4);
        assert_eq!(f.width("§la"), 5);
        assert_eq!(f.width("§l§ca"), 4, "a colour code resets bold");
        assert_eq!(f.width("§c§la§ra"), 9);
        assert_eq!(f.width("a§"), 4);
    }

    #[test]
    fn unknown_chars_use_the_missing_glyph() {
        let f = tiny_font();
        assert_eq!(f.glyph('Z'), Glyph::MISSING);
        assert_eq!(f.width("Z"), 6);
    }

    #[test]
    fn hex_glyph_parse_and_trim() {
        // 8-wide glyph with a single column set at x=2 on every row.
        let row = "20";
        let data: String = std::iter::repeat_n(row, 16).collect();
        let g = parse_hex_glyph(&data).unwrap();
        assert_eq!((g.width, g.left, g.trimmed), (8, 2, 1));
        assert!(g.pixel(2, 0) && !g.pixel(1, 0));
        let empty = parse_hex_glyph(&"0".repeat(64)).unwrap();
        assert_eq!((empty.width, empty.trimmed), (16, 0));
    }

    #[test]
    fn first_provider_wins() {
        let mut f = tiny_font();
        f.insert(
            'a',
            Glyph {
                advance: 9.0,
                ..Glyph::MISSING
            },
        );
        assert_eq!(f.glyph('a').advance, 4.0);
    }

    #[test]
    fn ascii_table_agrees_with_the_map() {
        let f = tiny_font();
        for cp in 0..0x250u32 {
            let Some(ch) = char::from_u32(cp) else {
                continue;
            };
            assert_eq!(
                f.glyph(ch),
                f.glyphs.get(&ch).copied().unwrap_or(Glyph::MISSING),
                "glyph mismatch for U+{cp:04X}"
            );
            assert_eq!(
                f.defined_glyph(ch),
                f.glyphs.get(&ch).copied(),
                "defined_glyph mismatch for U+{cp:04X}"
            );
        }
    }

    #[test]
    fn non_ascii_glyph_resolves_from_the_map() {
        let f = tiny_font();
        let g = f.glyph('λ');
        assert_eq!(g.advance, 5.0);
        assert_eq!(f.defined_glyph('λ'), Some(g));
    }
}
