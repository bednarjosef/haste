// This file is derived from Pomme (https://github.com/PommeMC/Client),
// pomme-client/src/assets.rs and parts of world/block/model.rs, reused under
// GPL-3.0. Reworked for Haste: reads the client jar directly through `zip`
// (no extracted assets dir), resolves texture variables, and exposes the
// haste-core `Identifier` API from docs/INTERFACES.md. See THIRD_PARTY.md.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use haste_core::Identifier;

use crate::model::{ModelFile, ResolvedModel};
use crate::multipart::BlockStateDefinition;

/// Open client jar + assets index/objects directory (the `Installation::assets_dir` +
/// `assets_index` from haste-setup). The jar's own `assets/` tree takes precedence when
/// the index is missing an entry; in 26.3 the client jar carries the full vanilla
/// asset tree, the index only adds the same files by hash (kept for pack parity).
pub struct Assets {
    jar: zip::ZipArchive<std::fs::File>,
    /// `<assets_dir>/objects` from the launcher download; may be absent.
    objects_dir: Option<PathBuf>,
    /// Parsed `<assets_dir>/indexes/<index>.json`, when present.
    index_hashes: HashMap<String, String>,
    model_cache: HashMap<String, Arc<ModelFile>>,
}

#[derive(Deserialize)]
struct AssetIndexFile {
    objects: HashMap<String, AssetIndexObject>,
}

#[derive(Deserialize)]
struct AssetIndexObject {
    hash: String,
}

/// `.mcmeta` parsed for a texture: animation for block textures, scaling for GUI sprites.
#[derive(Clone, Debug, Default)]
pub struct TextureMeta {
    /// `animation` from `.mcmeta`, when present (None = not an animated texture).
    pub animation: Option<AnimationMeta>,
}

/// `assets/minecraft/textures/<path>.png` `.mcmeta` `animation` section. We keep the
/// first frame for the atlas (vanilla's `initial_display_frame` when frames are listed);
/// animation playback is the renderer's job later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimationMeta {
    pub width: u32,
    pub height: u32,
    pub frametime: u32,
    /// First listed frame index (vanilla's initial display frame).
    pub first_frame: u32,
    pub frame_count: u32,
}

/// `.mcmeta` `gui.scaling` of a GUI sprite: nine-slice/tile/stretch.
#[derive(Clone, Debug, PartialEq)]
pub enum SpriteMeta {
    Stretch,
    NineSlice {
        width: u32,
        height: u32,
        border: u32,
    },
    Tile {
        width: u32,
        height: u32,
    },
}

impl Assets {
    pub fn open(client_jar: &Path, assets_dir: &Path, assets_index: &str) -> Result<Assets> {
        let file = std::fs::File::open(client_jar)
            .with_context(|| format!("opening client jar {}", client_jar.display()))?;
        let mut jar = zip::ZipArchive::new(file)
            .with_context(|| format!("reading zip {}", client_jar.display()))?;

        let objects_dir = assets_dir.join("objects");
        let index_path = assets_dir
            .join("indexes")
            .join(format!("{assets_index}.json"));
        let index_hashes = match std::fs::read(&index_path) {
            Ok(bytes) => {
                let parsed: AssetIndexFile = serde_json::from_slice(&bytes)
                    .with_context(|| format!("parsing asset index {}", index_path.display()))?;
                parsed
                    .objects
                    .into_iter()
                    .map(|(k, v)| (k, v.hash))
                    .collect()
            }
            Err(e) => {
                // 26.3's client jar carries the whole vanilla asset tree; the index
                // is an alternative view. Soft-fail so tests can run on the jar alone.
                tracing::warn!(
                    "asset index {} unreadable ({e}); using jar assets only",
                    index_path.display()
                );
                HashMap::new()
            }
        };
        let objects_dir = objects_dir.is_dir().then_some(objects_dir);

        if jar
            .by_name("assets/minecraft/blockstates/stone.json")
            .is_err()
        {
            bail!("client jar does not contain assets/minecraft/blockstates");
        }
        let _ = jar
            .by_name("assets/minecraft/lang/en_us.json")
            .context("client jar does not contain assets/minecraft/lang/en_us.json")?;

        Ok(Assets {
            jar,
            objects_dir,
            index_hashes,
            model_cache: HashMap::new(),
        })
    }

    /// `textures/<path>.png` of an identifier, e.g. `minecraft:block/stone`.
    /// The PNG is decoded; for animated textures the first frame is returned along
    /// with the parsed `AnimationMeta` (full-size image + meta when not animated).
    pub fn texture(&mut self, id: &Identifier) -> Result<image::RgbaImage> {
        let key = asset_key(id, "textures", ".png");
        let bytes = self.read_asset(&key)?;
        decode_png(&bytes, &key)
    }

    /// Parsed `.mcmeta` next to a texture, or default (not animated) when absent.
    pub fn texture_meta(&mut self, id: &Identifier) -> Result<TextureMeta> {
        let key = asset_key(id, "textures", ".png.mcmeta");
        match self.try_read_asset(&key)? {
            Some(bytes) => {
                let meta: TextureMetaFile =
                    serde_json::from_slice(&bytes).with_context(|| format!("parsing {key}"))?;
                Ok(TextureMeta {
                    animation: meta.animation.map(AnimationMeta::from),
                })
            }
            None => Ok(TextureMeta::default()),
        }
    }

    /// `blockstates/<path>.json` of a block, variants and multipart as written.
    pub fn blockstate(&mut self, block: &Identifier) -> Result<BlockStateDefinition> {
        let key = asset_key(block, "blockstates", ".json");
        let bytes = self.read_asset(&key)?;
        let def: BlockStateDefinition =
            serde_json::from_slice(&bytes).with_context(|| format!("parsing {key}"))?;
        Ok(def)
    }

    /// `models/<path>.json` with the `parent` chain resolved: `textures` variables
    /// (`#all` etc.) resolved to texture identifiers, elements inherited from the
    /// nearest ancestor that has any, `ambientocclusion` inherited (default true).
    pub fn model(&mut self, id: &Identifier) -> Result<ModelJson> {
        let resolved = self.resolve_model(id)?;
        Ok(ModelJson {
            textures: resolved.textures,
            elements: resolved.elements,
            ambientocclusion: resolved.ambientocclusion,
            gui_light: resolved.gui_light,
        })
    }

    /// `assets/minecraft/lang/<code>.json`.
    pub fn language(&mut self, code: &str) -> Result<HashMap<String, String>> {
        let key = format!("assets/minecraft/lang/{code}.json");
        let bytes = self.read_asset(&key)?;
        let map: HashMap<String, String> =
            serde_json::from_slice(&bytes).with_context(|| format!("parsing {key}"))?;
        Ok(map)
    }

    /// `font/default.json` resolved through its `reference` providers into bitmap
    /// providers (ascii.png, accented.png, ...) plus the `space` provider's advances.
    pub fn font_default(&mut self) -> Result<BitmapFont> {
        self.load_font(&Identifier::minecraft("default"))
    }

    /// `font/<path>.json` (used by `default`'s reference providers).
    fn load_font(&mut self, id: &Identifier) -> Result<BitmapFont> {
        let key = asset_key(id, "font", ".json");
        let bytes = self.read_asset(&key)?;
        let file: FontFile =
            serde_json::from_slice(&bytes).with_context(|| format!("parsing {key}"))?;
        let mut font = BitmapFont::default();
        for provider in file.providers {
            match provider {
                FontProvider::Bitmap(p) => {
                    // The provider's `file` is a full texture id ending in .png
                    // (e.g. minecraft:font/ascii.png), unlike `Assets::texture`
                    // which appends .png itself.
                    let id = Identifier::parse(&p.file)?;
                    let key = format!("assets/{}/textures/{}", id.namespace(), id.path());
                    let bytes = self.read_asset(&key)?;
                    let image = image::load_from_memory(&bytes)
                        .with_context(|| format!("decoding {key}"))?
                        .to_rgba8();
                    font.add_bitmap_page(p.height, p.ascent, &p.chars, image);
                }
                FontProvider::Reference { id: ref_id } => {
                    // Reference the referenced font's glyphs; depth is bounded by the
                    // recursion guard below (vanilla fonts are shallow).
                    let sub = self.load_font(&Identifier::parse(&ref_id)?)?;
                    font.merge(sub);
                }
                FontProvider::Space { advances } => {
                    font.space_advances = advances;
                }
                FontProvider::Unimplemented => {
                    tracing::debug!("unimplemented font provider type skipped");
                }
            }
        }
        Ok(font)
    }

    /// `assets/minecraft/textures/gui/sprites/<path>.png` with its `.mcmeta` scaling
    /// (`gui/scaling`: nine_slice with a uniform border in 26.3, tile, or stretch).
    pub fn gui_sprite(&mut self, id: &Identifier) -> Result<(image::RgbaImage, SpriteMeta)> {
        let key = asset_key(id, "textures/gui/sprites", ".png");
        let bytes = self.read_asset(&key)?;
        let image = decode_png(&bytes, &key)?;
        let meta_key = format!("{key}.mcmeta");
        let meta = match self.try_read_asset(&meta_key)? {
            Some(m) => {
                let f: TextureMetaFile =
                    serde_json::from_slice(&m).with_context(|| format!("parsing {meta_key}"))?;
                f.gui
                    .map(|g| g.scaling.into())
                    .unwrap_or(SpriteMeta::Stretch)
            }
            None => SpriteMeta::Stretch,
        };
        Ok((image, meta))
    }

    /// All `assets/minecraft/blockstates/*.json` block names (paths without the
    /// directory and extension). For `BakedBlockModels::bake`.
    pub fn blockstate_names(&mut self) -> Vec<Identifier> {
        self.names_under("assets/minecraft/blockstates/", ".json")
            .map(|p| Identifier::minecraft(&p))
            .collect()
    }

    /// All `assets/minecraft/textures/block/*.png` names (without extension).
    pub fn block_texture_names(&mut self) -> Vec<Identifier> {
        self.names_under("assets/minecraft/textures/block/", ".png")
            .map(|p| Identifier::minecraft(&format!("block/{p}")))
            .collect()
    }

    /// File names directly under `dir` ending with `ext`, path relative to `dir`.
    fn names_under(&mut self, dir: &str, ext: &str) -> impl Iterator<Item = String> {
        let names: Vec<String> = self
            .jar
            .file_names()
            .filter(|n| n.starts_with(dir) && n.ends_with(ext))
            .filter_map(|n| {
                n.strip_prefix(dir)
                    .and_then(|rest| rest.strip_suffix(ext))
                    .filter(|rest| !rest.contains('/'))
                    .map(str::to_owned)
            })
            .collect();
        names.into_iter()
    }

    // ---- model resolution (from Pomme world/block/model.rs, reworked) ----

    pub(crate) fn resolve_model(&mut self, id: &Identifier) -> Result<ResolvedModel> {
        let mut texture_map: HashMap<String, String> = HashMap::new();
        let mut elements: Option<Vec<crate::model::ElementDef>> = None;
        let mut ambientocclusion: Option<bool> = None;
        let mut gui_light: Option<bool> = None;
        let mut current = id.clone();

        for _ in 0..20 {
            let model = self.load_model(&current)?;
            for (key, value) in &model.textures {
                texture_map
                    .entry(key.clone())
                    .or_insert_with(|| value.clone());
            }
            if elements.is_none() && !model.elements.is_empty() {
                elements = Some(model.elements.clone());
            }
            if ambientocclusion.is_none() && model.ambientocclusion.is_some() {
                ambientocclusion = model.ambientocclusion;
            }
            if gui_light.is_none() && model.gui_light.is_some() {
                gui_light = Some(model.gui_light.as_deref() == Some("front"));
            }
            match &model.parent {
                Some(parent) => current = Identifier::parse(parent)?,
                None => break,
            }
        }

        // Resolve `#ref` texture variables (bounded depth, like Pomme's resolve_ref).
        let mut resolved = HashMap::with_capacity(texture_map.len());
        for (key, value) in &texture_map {
            resolved.insert(key.clone(), resolve_ref(value, &texture_map, 0));
        }

        Ok(ResolvedModel {
            textures: resolved,
            elements: elements.unwrap_or_default(),
            ambientocclusion: ambientocclusion.unwrap_or(true),
            gui_light: gui_light.unwrap_or(false),
        })
    }

    pub(crate) fn load_model(&mut self, id: &Identifier) -> Result<Arc<ModelFile>> {
        if let Some(cached) = self.model_cache.get(&id.to_string()) {
            return Ok(cached.clone());
        }
        let key = asset_key(id, "models", ".json");
        let bytes = self.read_asset(&key)?;
        let model: ModelFile =
            serde_json::from_slice(&bytes).with_context(|| format!("parsing {key}"))?;
        let model = Arc::new(model);
        self.model_cache.insert(id.to_string(), model.clone());
        Ok(model)
    }

    // ---- raw access ----

    /// Asset bytes by key (`assets/<ns>/<path>`), preferring the jar, falling back to
    /// the index/objects dir.
    pub fn read_asset(&mut self, key: &str) -> Result<Vec<u8>> {
        self.try_read_asset(key)?
            .with_context(|| format!("asset {key} not found"))
    }

    fn try_read_asset(&mut self, key: &str) -> Result<Option<Vec<u8>>> {
        if let Ok(mut entry) = self.jar.by_name(key) {
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut buf)?;
            return Ok(Some(buf));
        }
        if let (Some(objects), Some(hash)) = (&self.objects_dir, self.index_hashes.get(key)) {
            let path = objects.join(&hash[..2]).join(hash);
            let bytes = std::fs::read(&path)
                .with_context(|| format!("reading indexed asset {key} from {}", path.display()))?;
            return Ok(Some(bytes));
        }
        Ok(None)
    }
}

/// Texture references resolve through `#variable` chains with bounded depth
/// (Pomme `resolve_ref`, vanilla `Material` resolution).
fn resolve_ref(value: &str, map: &HashMap<String, String>, depth: u32) -> String {
    if depth > 10 {
        return value.to_string();
    }
    if let Some(ref_name) = value.strip_prefix('#') {
        if let Some(target) = map.get(ref_name) {
            return resolve_ref(target, map, depth + 1);
        }
    }
    value.to_string()
}

pub(crate) fn asset_key(id: &Identifier, category: &str, suffix: &str) -> String {
    format!(
        "assets/{}/{}/{}{}",
        id.namespace(),
        category,
        id.path(),
        suffix
    )
}

fn decode_png(bytes: &[u8], key: &str) -> Result<image::RgbaImage> {
    let image = image::load_from_memory(bytes).with_context(|| format!("decoding png {key}"))?;
    Ok(image.to_rgba8())
}

#[derive(Deserialize)]
struct TextureMetaFile {
    animation: Option<AnimationMetaRaw>,
    gui: Option<GuiMetaRaw>,
}

#[derive(Deserialize)]
struct GuiMetaRaw {
    scaling: SpriteMetaSerde,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SpriteMetaSerde {
    NineSlice {
        width: u32,
        height: u32,
        border: u32,
    },
    Tile {
        width: u32,
        height: u32,
    },
    Stretch,
}

impl From<SpriteMetaSerde> for SpriteMeta {
    fn from(s: SpriteMetaSerde) -> SpriteMeta {
        match s {
            SpriteMetaSerde::NineSlice {
                width,
                height,
                border,
            } => SpriteMeta::NineSlice {
                width,
                height,
                border,
            },
            SpriteMetaSerde::Tile { width, height } => SpriteMeta::Tile { width, height },
            SpriteMetaSerde::Stretch => SpriteMeta::Stretch,
        }
    }
}

#[derive(Deserialize)]
struct AnimationMetaRaw {
    width: Option<u32>,
    height: Option<u32>,
    frametime: Option<u32>,
    frames: Option<Vec<serde_json::Value>>,
}

impl From<AnimationMetaRaw> for AnimationMeta {
    /// Normalizes to Pomme's `AnimationLayout` rules: frame size defaults to the
    /// image's min dimension (or the other given one), frametime defaults to 1.
    fn from(raw: AnimationMetaRaw) -> AnimationMeta {
        let AnimationMetaRaw {
            width,
            height,
            frametime,
            frames,
        } = raw;
        AnimationMeta {
            width: width.unwrap_or(1),
            height: height.unwrap_or(1),
            frametime: frametime.unwrap_or(1).max(1),
            first_frame: match &frames {
                Some(list) => match list.first() {
                    Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(0) as u32,
                    Some(serde_json::Value::Object(o)) => {
                        o.get("index")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0) as u32
                    }
                    _ => 0,
                },
                None => 0,
            },
            frame_count: 1,
        }
    }
}

#[derive(Deserialize)]
struct FontFile {
    #[serde(default)]
    providers: Vec<FontProvider>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum FontProvider {
    // NOTE: every variant needs an explicit rename: serde's internally-tagged
    // matching is case-sensitive and does NOT apply rename_all here.
    #[serde(rename = "bitmap")]
    Bitmap(BitmapProviderRaw),
    #[serde(rename = "reference")]
    Reference { id: String },
    #[serde(rename = "space")]
    Space { advances: HashMap<String, u32> },
    #[serde(other)]
    Unimplemented,
}

#[derive(Deserialize)]
struct BitmapProviderRaw {
    file: String,
    height: Option<u32>,
    ascent: Option<i32>,
    chars: Vec<String>,
}

/// A resolved bitmap font: glyph pages (each row of `chars` names 16 columns of an
/// 8-px-wide font page) plus space-provider advances. Space = 4 px comes from
/// `include/space.json`'s `" ": 4`; measurement defaults to non-empty columns + 1.
#[derive(Default)]
pub struct BitmapFont {
    pub glyphs: HashMap<char, Glyph>,
    pub space_advances: HashMap<String, u32>,
    pages: Vec<Page>,
}

#[derive(Clone, Debug)]
pub struct Glyph {
    /// Glyph page this glyph lives in (index in `BitmapFont::pages` order).
    pub page: usize,
    /// Position in the page in pixels.
    pub x: u32,
    pub y: u32,
    /// Width in pixels (non-empty columns + 1, vanilla's bitmap measurement).
    pub width: u32,
    /// Glyph height as declared by the provider (`height` field, default 8).
    pub height: u32,
    /// `ascent` (distance from the top of the line box to the glyph baseline).
    pub ascent: i32,
}

impl BitmapFont {
    /// A bitmap page: `chars` rows of 16 code points, an 8-px cell grid over the image.
    fn add_bitmap_page(
        &mut self,
        height: Option<u32>,
        ascent: Option<i32>,
        chars: &[String],
        image: image::RgbaImage,
    ) {
        let height = height.unwrap_or(8);
        let ascent = ascent.unwrap_or(7);
        let page = Page {
            image: Arc::new(image),
        };
        let page_index = self.pages.len();
        let cell = 8;
        for (row, line) in chars.iter().enumerate() {
            for (col, ch) in line.chars().enumerate() {
                if ch == '\0' {
                    continue;
                }
                let x = (col as u32) * cell;
                let y = (row as u32) * cell;
                let width = page.glyph_width(x, y, height);
                let glyph = Glyph {
                    page: page_index,
                    x,
                    y,
                    width,
                    height,
                    ascent,
                };
                self.glyphs.entry(ch).or_insert(glyph);
            }
        }
        self.pages.push(page);
    }

    /// Merge another font's pages/glyphs (a `reference` provider); later pages
    /// never override earlier glyphs, matching provider priority order.
    fn merge(&mut self, other: BitmapFont) {
        let page_base = self.pages.len();
        for (ch, mut g) in other.glyphs {
            g.page += page_base;
            self.glyphs.entry(ch).or_insert(g);
        }
        self.pages.extend(other.pages);
        if self.space_advances.is_empty() {
            self.space_advances = other.space_advances;
        }
    }

    /// Advance width of one character, or None for an unknown char.
    pub fn char_width(&self, ch: char) -> Option<u32> {
        if let Some(glyph) = self.glyphs.get(&ch) {
            return Some(glyph.width);
        }
        if ch == ' ' {
            return Some(4); // include/space.json: " ": 4
        }
        self.space_advances
            .get(ch.to_string().as_str())
            .copied()
            .or(if ch == ' ' { Some(4) } else { None })
    }
}

pub(crate) struct Page {
    pub image: Arc<image::RgbaImage>,
}

impl Page {
    /// Width by non-empty columns: count columns with any non-zero-alpha pixel in
    /// the glyph's cell, then +1 for the spacing column (vanilla's `GlyphInfo` for
    /// bitmap fonts measures the advance as the last non-empty column + 1).
    fn glyph_width(&self, x: u32, y: u32, height: u32) -> u32 {
        let (w, _h) = self.image.dimensions();
        let mut last_nonempty = 0;
        let mut any = false;
        for cx in 0..8 {
            if x + cx >= w {
                break;
            }
            let mut nonempty = false;
            for cy in 0..height {
                if y + cy >= self.image.height() {
                    break;
                }
                if self.image.get_pixel(x + cx, y + cy).0[3] != 0 {
                    nonempty = true;
                    break;
                }
            }
            if nonempty {
                last_nonempty = cx + 1;
                any = true;
            }
        }
        if any {
            last_nonempty
        } else {
            4 // blank glyph cell (vanilla gives spaces advance 4 through the space provider)
        }
    }
}

/// The model with parent chain resolved, as returned by `Assets::model`.
/// (Pomme's `ResolvedModel` shape, restricted to what block baking needs.)
#[derive(Clone, Debug)]
pub struct ModelJson {
    /// Texture variable name (`all`, `up`, ...) -> resolved texture id.
    pub textures: HashMap<String, String>,
    pub elements: Vec<crate::model::ElementDef>,
    pub ambientocclusion: bool,
    /// `gui_light`: true = `front` (flat items). Only used by item rendering later.
    pub gui_light: bool,
}

/// `haste_core::gui::TextMetrics` over the bitmap font: width from glyph
/// advances (space provider's 4 px for spaces), line height 9 like vanilla.
impl haste_core::gui::TextMetrics for BitmapFont {
    fn width(&self, text: &str) -> i32 {
        text.chars()
            .map(|ch| self.char_width(ch).unwrap_or(6) as i32)
            .sum()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn open_assets() -> Option<Assets> {
        let jar = haste_core::paths::client_jar();
        if !jar.exists() {
            return None;
        }
        let assets_dir = haste_core::paths::cache_dir().join("assets");
        Assets::open(&jar, &assets_dir, "26.3").ok()
    }

    fn jar_test(name: &str, f: impl FnOnce(&mut Assets)) {
        let mut assets = match open_assets() {
            Some(a) => a,
            None => {
                eprintln!("ignored: {name} needs the client jar in the cache");
                return;
            }
        };
        f(&mut assets);
    }

    #[test]
    fn asset_keys() {
        let id = Identifier::parse("minecraft:block/stone").unwrap();
        assert_eq!(
            asset_key(&id, "blockstates", ".json"),
            "assets/minecraft/blockstates/block/stone.json"
        );
        let id = Identifier::parse("stone").unwrap();
        assert_eq!(
            asset_key(&id, "textures", ".png"),
            "assets/minecraft/textures/stone.png"
        );
    }

    #[test]
    #[ignore = "needs the client jar in the cache"]
    fn jar_texture_and_blockstate() {
        jar_test("texture", |a| {
            let img = a.texture(&Identifier::minecraft("block/stone")).unwrap();
            assert_eq!(img.dimensions(), (16, 16));
            let bs = a.blockstate(&Identifier::minecraft("stone")).unwrap();
            assert!(bs.variants.is_some());
        });
    }

    #[test]
    #[ignore = "needs the client jar in the cache"]
    fn jar_font_and_language() {
        jar_test("font", |a| {
            let font = a.font_default().unwrap();
            assert!(font.glyphs.contains_key(&'A'));
            assert_eq!(font.char_width(' '), Some(4));
            assert!(font.char_width('W').unwrap() > font.char_width('i').unwrap_or(0));
            let en = a.language("en_us").unwrap();
            assert!(en.contains_key("block.minecraft.stone"));
            use haste_core::gui::TextMetrics;
            assert_eq!(font.width("i"), 1); // 1 px glyph + 0 extra
            assert!(font.width("W") > font.width("i"));
            assert_eq!(font.width("ii"), 2);
        });
    }

    #[test]
    #[ignore = "needs the client jar in the cache"]
    fn jar_gui_sprite_nine_slice() {
        jar_test("gui_sprite", |a| {
            let (img, meta) = a.gui_sprite(&Identifier::minecraft("hud/hotbar")).unwrap();
            assert!(img.dimensions().0 > 0);
            assert_eq!(meta, SpriteMeta::Stretch);
            let (_, meta) = a
                .gui_sprite(&Identifier::minecraft("advancements/title_box"))
                .unwrap();
            assert_eq!(
                meta,
                SpriteMeta::NineSlice {
                    width: 200,
                    height: 26,
                    border: 10
                }
            );
        });
    }

    #[test]
    #[ignore = "needs the client jar in the cache"]
    fn jar_model_parent_resolution() {
        jar_test("model", |a| {
            // stone -> block/cube_all -> block/cube -> block/block: one element,
            // textures all -> #all -> block/stone.
            let model = a.model(&Identifier::minecraft("block/stone")).unwrap();
            assert_eq!(model.elements.len(), 1);
            assert_eq!(
                model.textures.get("all").map(String::as_str),
                Some("minecraft:block/stone")
            );
            assert!(model.ambientocclusion);
        });
    }
}
