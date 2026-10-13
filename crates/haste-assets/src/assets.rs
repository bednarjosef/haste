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
    /// Where this was opened from (`reopen`).
    source: (PathBuf, PathBuf, String),
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
/// the full playback order and timing live in `AnimationSchedule` (see `Assets::animation`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimationMeta {
    pub width: u32,
    pub height: u32,
    pub frametime: u32,
    /// First listed frame index (vanilla's initial display frame).
    pub first_frame: u32,
    pub frame_count: u32,
    /// `interpolate`: blend linearly towards the next frame over each frame's time.
    pub interpolate: bool,
}

/// One entry of an `animation.frames` list: which frame of the sheet to show and for
/// how many ticks (`time`, defaulting to the mcmeta `frametime`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimationFrame {
    pub index: u32,
    pub time: u32,
}

/// The playback order of an animated texture: the mcmeta `frames` list (or every frame
/// of the sheet in order when it gives none), with per-frame times.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimationSchedule {
    pub frames: Vec<AnimationFrame>,
    /// `interpolate`: blend linearly towards the next frame over each frame's time.
    pub interpolate: bool,
}

impl AnimationSchedule {
    /// Every frame of the sheet in order at `frametime` (no explicit `frames` list).
    pub fn sequential(frame_count: u32, frametime: u32, interpolate: bool) -> Self {
        AnimationSchedule {
            frames: (0..frame_count)
                .map(|index| AnimationFrame {
                    index,
                    time: frametime,
                })
                .collect(),
            interpolate,
        }
    }

    /// Total playback length in ticks (0 for an empty schedule).
    pub fn total_ticks(&self) -> u64 {
        self.frames.iter().map(|f| u64::from(f.time)).sum()
    }
}

/// The frame to show at `tick`, wrapping over the schedule's total length: the current
/// frame's index into the sheet, the next frame's index in playback order (wrapping),
/// and the blend factor towards it (always 0 when the schedule does not interpolate,
/// else `ticks spent in the frame / frame time`).
pub fn frame_at(schedule: &AnimationSchedule, tick: u64) -> (u32, u32, f32) {
    if schedule.frames.is_empty() {
        return (0, 0, 0.0);
    }
    let total = schedule.total_ticks();
    let mut elapsed = if total == 0 { 0 } else { tick % total };
    for (position, frame) in schedule.frames.iter().enumerate() {
        let time = u64::from(frame.time);
        if elapsed < time {
            let next = schedule.frames[(position + 1) % schedule.frames.len()].index;
            let blend = if schedule.interpolate && time > 0 {
                elapsed as f32 / time as f32
            } else {
                0.0
            };
            return (frame.index, next, blend);
        }
        elapsed -= time;
    }
    // Unreachable for schedules built by `animation_schedule` (times are >= 1 so the
    // wrapped tick lands inside a frame); keeps hand-built zero-time schedules total.
    let index = schedule.frames[0].index;
    (index, index, 0.0)
}

/// An animated texture resolved for playback: the sheet split into same-size frames
/// (the index order `frame_at` refers to) plus its schedule.
#[derive(Clone, Debug)]
pub struct Animation {
    /// Frames in sheet index order (row-major grid; a vertical strip is top to bottom).
    pub frames: Vec<image::RgbaImage>,
    pub schedule: AnimationSchedule,
}

impl Animation {
    /// The frame shown before animation starts (vanilla's initial display frame:
    /// the first listed frame, else frame 0). Panics when there are no frames.
    pub fn initial_frame(&self) -> &image::RgbaImage {
        let index = self.schedule.frames.first().map(|f| f.index).unwrap_or(0) as usize;
        self.frames.get(index).unwrap_or(&self.frames[0])
    }
}

/// `.mcmeta` `gui.scaling` of a GUI sprite: nine-slice/tile/stretch.
#[derive(Clone, Debug, PartialEq)]
pub enum SpriteMeta {
    Stretch,
    NineSlice {
        width: u32,
        height: u32,
        /// Left, top, right, bottom (a single number in the file means all four).
        border: [u32; 4],
        /// `stretch_inner`: the centre is stretched instead of tiled.
        stretch_inner: bool,
    },
    Tile {
        width: u32,
        height: u32,
    },
}

impl Assets {
    pub fn open(client_jar: &Path, assets_dir: &Path, assets_index: &str) -> Result<Assets> {
        let source = (
            client_jar.to_path_buf(),
            assets_dir.to_path_buf(),
            assets_index.to_string(),
        );
        let file = std::fs::File::open(client_jar)
            .with_context(|| format!("opening client jar {}", client_jar.display()))?;
        let mut jar = zip::ZipArchive::new(file)
            .with_context(|| format!("reading zip {}", client_jar.display()))?;

        let objects_dir = assets_dir.join("objects");
        let mut index_path = assets_dir
            .join("indexes")
            .join(format!("{assets_index}.json"));
        if !index_path.exists() {
            // The launcher names the index by its asset-index id (`34`), not the game version;
            // with a single index present that is the one.
            if let Ok(dir) = std::fs::read_dir(assets_dir.join("indexes")) {
                let found: Vec<PathBuf> = dir
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "json"))
                    .collect();
                if found.len() == 1 {
                    index_path = found[0].clone();
                }
            }
        }
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
            source,
        })
    }

    /// A second handle on the same jar and assets (the renderer keeps one for lazy loads).
    pub fn reopen(&self) -> Result<Assets> {
        Assets::open(&self.source.0, &self.source.1, &self.source.2)
    }

    /// `assets/<ns>/<path>` of a full texture id such as `minecraft:textures/gui/x.png`, or
    /// `textures/<path>.png` of a short one (`minecraft:gui/x`), decoded.
    pub fn texture_any(&mut self, id: &Identifier) -> Result<image::RgbaImage> {
        let key = if id.path().ends_with(".png") {
            format!("assets/{}/{}", id.namespace(), id.path())
        } else {
            asset_key(id, "textures", ".png")
        };
        let bytes = self.read_asset(&key)?;
        decode_png(&bytes, &key)
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

    /// `assets/minecraft/textures/gui/sprites/<path>.png` with its `.mcmeta` scaling
    /// (`gui/scaling`: nine_slice with a uniform border in 26.3, tile, or stretch).
    pub fn gui_sprite(&mut self, id: &Identifier) -> Result<(image::RgbaImage, SpriteMeta)> {
        let key = asset_key(id, "textures/gui/sprites", ".png");
        let bytes = self.read_asset(&key)?;
        let image = decode_png(&bytes, &key)?;
        let meta_key = format!("{key}.mcmeta");
        let mut image = image;
        let meta = match self.try_read_asset(&meta_key)? {
            Some(m) => {
                let f: TextureMetaFile =
                    serde_json::from_slice(&m).with_context(|| format!("parsing {meta_key}"))?;
                if let Some(anim) = &f.animation {
                    image = first_animation_frame(image, anim);
                }
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

    /// File names directly under `dir` (an `assets/...` key prefix) ending with `ext`, path
    /// relative to `dir`, without the extension.
    pub fn names_in(&mut self, dir: &str, ext: &str) -> impl Iterator<Item = String> {
        self.names_under(dir, ext)
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
        let mut gui_display: Option<crate::model::DisplayTransform> = None;
        let mut firstperson_display: Option<crate::model::DisplayTransform> = None;
        let mut ground_display: Option<crate::model::DisplayTransform> = None;
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
            if firstperson_display.is_none() {
                firstperson_display = model
                    .display
                    .get("firstperson_righthand")
                    .and_then(crate::model::DisplayTransform::parse);
            }
            if gui_display.is_none() {
                gui_display = model
                    .display
                    .get("gui")
                    .and_then(crate::model::DisplayTransform::parse);
            }
            if ground_display.is_none() {
                ground_display = model
                    .display
                    .get("ground")
                    .and_then(crate::model::DisplayTransform::parse);
            }
            if gui_light.is_none() && model.gui_light.is_some() {
                gui_light = Some(model.gui_light.as_deref() == Some("front"));
            }
            match &model.parent {
                // `builtin/generated` (flat items) and `builtin/entity` have no file.
                Some(parent)
                    if parent
                        .trim_start_matches("minecraft:")
                        .starts_with("builtin/") =>
                {
                    break
                }
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
            gui_display,
            firstperson_display,
            ground_display,
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

    /// The downloaded assets (index + objects) take precedence over the jar: the jar carries
    /// stubs for what the launcher downloads (`font/include/unifont.json` is empty in the jar).
    /// Index keys have no `assets/` prefix.
    pub(crate) fn try_read_asset(&mut self, key: &str) -> Result<Option<Vec<u8>>> {
        let index_key = key.strip_prefix("assets/").unwrap_or(key);
        if let (Some(objects), Some(hash)) = (&self.objects_dir, self.index_hashes.get(index_key)) {
            let path = objects.join(&hash[..2]).join(hash);
            if let Ok(bytes) = std::fs::read(&path) {
                return Ok(Some(bytes));
            }
            tracing::warn!("indexed asset {key} missing at {}", path.display());
        }
        if let Ok(mut entry) = self.jar.by_name(key) {
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut buf)?;
            return Ok(Some(buf));
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

/// The first frame of an animated strip (frames stacked vertically; frame size from the
/// `.mcmeta` `width`/`height`, else square frames of the image width). Uses the first listed
/// frame index when `frames` is given.
fn first_animation_frame(image: image::RgbaImage, anim: &AnimationMetaRaw) -> image::RgbaImage {
    let (w, h) = image.dimensions();
    let fw = anim.width.unwrap_or(w).min(w).max(1);
    let fh = anim
        .height
        .unwrap_or(if anim.width.is_some() { h } else { w })
        .min(h)
        .max(1);
    let per_row = (w / fw).max(1);
    let count = per_row * (h / fh).max(1);
    let first = match anim.frames.as_ref().and_then(|f| f.first()) {
        Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(0) as u32,
        Some(serde_json::Value::Object(o)) => {
            o.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as u32
        }
        _ => 0,
    };
    let first = if first < count { first } else { 0 };
    if fw == w && fh == h {
        return image;
    }
    let (fx, fy) = ((first % per_row) * fw, (first / per_row) * fh);
    image::imageops::crop_imm(&image, fx, fy, fw, fh).to_image()
}

/// How an animation sheet divides into frames (vanilla's rules, as
/// `first_animation_frame` applies them): `width`/`height` from the `.mcmeta`
/// clamped to the image, else square frames of the image width. Returns the
/// frame size, frames per grid row and the total frame count.
struct AnimationLayout {
    frame_size: (u32, u32),
    per_row: u32,
    frame_count: u32,
}

fn animation_layout(image: &image::RgbaImage, anim: &AnimationMetaRaw) -> AnimationLayout {
    let (w, h) = image.dimensions();
    let fw = anim.width.unwrap_or(w).min(w).max(1);
    let fh = anim
        .height
        .unwrap_or(if anim.width.is_some() { h } else { w })
        .min(h)
        .max(1);
    let per_row = (w / fw).max(1);
    AnimationLayout {
        frame_size: (fw, fh),
        per_row,
        frame_count: per_row * (h / fh).max(1),
    }
}

/// The mcmeta `frames` list as playback entries: plain ints are indices at
/// `frametime`, objects give `index` and `time` (defaulting to `frametime`).
/// Without a list every frame of the sheet plays in order at `frametime`.
/// Indices beyond the sheet and non-positive times fall back to vanilla-ish
/// safe values (frame 0, `frametime`).
fn animation_frame_list(raw: &AnimationMetaRaw, frame_count: u32) -> Vec<AnimationFrame> {
    let frametime = raw.frametime.unwrap_or(1).max(1);
    let parse_entry = |value: &serde_json::Value| -> Option<AnimationFrame> {
        match value {
            serde_json::Value::Number(n) => {
                let index = n.as_u64()? as u32;
                (index < frame_count).then_some(AnimationFrame {
                    index,
                    time: frametime,
                })
            }
            serde_json::Value::Object(o) => {
                let index = o.get("index")?.as_u64()? as u32;
                if index >= frame_count {
                    return None;
                }
                let time = o
                    .get("time")
                    .and_then(serde_json::Value::as_u64)
                    .map_or(frametime, |t| (t as u32).max(1));
                Some(AnimationFrame { index, time })
            }
            _ => None,
        }
    };
    match &raw.frames {
        Some(list) => list.iter().filter_map(parse_entry).collect(),
        None => AnimationSchedule::sequential(frame_count, frametime, false).frames,
    }
}

/// `textures/<path>.png` reduced to its first animation frame when it has an `.mcmeta`
/// animation (block textures used by GUI items).
impl Assets {
    pub fn texture_first_frame(&mut self, id: &Identifier) -> Result<image::RgbaImage> {
        let image = self.texture_any(id)?;
        let meta_key = if id.path().ends_with(".png") {
            format!("assets/{}/{}.mcmeta", id.namespace(), id.path())
        } else {
            asset_key(id, "textures", ".png.mcmeta")
        };
        Ok(match self.try_read_asset(&meta_key)? {
            Some(m) => match serde_json::from_slice::<TextureMetaFile>(&m) {
                Ok(TextureMetaFile {
                    animation: Some(anim),
                    ..
                }) => first_animation_frame(image, &anim),
                _ => image,
            },
            None => image,
        })
    }

    /// An animated texture fully resolved for playback: the sheet split into
    /// same-size frames plus its `AnimationSchedule` (mcmeta `frames` list, or every
    /// frame in order at `frametime`). `None` when the texture has no animation
    /// or a malformed `.mcmeta` (the same fallbacks `texture_meta` uses).
    /// The `.mcmeta` is read before the PNG so static textures cost one lookup.
    pub fn animation(&mut self, id: &Identifier) -> Option<Animation> {
        let meta_key = if id.path().ends_with(".png") {
            format!("assets/{}/{}.mcmeta", id.namespace(), id.path())
        } else {
            asset_key(id, "textures", ".png.mcmeta")
        };
        let meta = self.try_read_asset(&meta_key).ok().flatten()?;
        let raw = serde_json::from_slice::<TextureMetaFile>(&meta).ok()?;
        let anim = raw.animation?;
        let image = self.texture_any(id).ok()?;
        Some(animation_from_raw(image, &anim))
    }
}

/// Splits an animation sheet into same-size frames (row-major grid; a vertical
/// strip is top to bottom) and resolves its schedule from the mcmeta section.
fn animation_from_raw(image: image::RgbaImage, raw: &AnimationMetaRaw) -> Animation {
    let AnimationLayout {
        frame_size: (fw, fh),
        per_row,
        frame_count,
    } = animation_layout(&image, raw);
    let mut frames = Vec::with_capacity(frame_count as usize);
    for index in 0..frame_count {
        let (fx, fy) = ((index % per_row) * fw, (index / per_row) * fh);
        frames.push(image::imageops::crop_imm(&image, fx, fy, fw, fh).to_image());
    }
    Animation {
        frames,
        schedule: AnimationSchedule {
            frames: animation_frame_list(raw, frame_count),
            interpolate: raw.interpolate.unwrap_or(false),
        },
    }
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
#[serde(untagged)]
enum BorderSerde {
    Uniform(u32),
    Sides {
        left: u32,
        top: u32,
        right: u32,
        bottom: u32,
    },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SpriteMetaSerde {
    NineSlice {
        width: u32,
        height: u32,
        border: BorderSerde,
        #[serde(default)]
        stretch_inner: bool,
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
                stretch_inner,
            } => SpriteMeta::NineSlice {
                width,
                height,
                border: match border {
                    BorderSerde::Uniform(b) => [b; 4],
                    BorderSerde::Sides {
                        left,
                        top,
                        right,
                        bottom,
                    } => [left, top, right, bottom],
                },
                stretch_inner,
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
    interpolate: Option<bool>,
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
            interpolate,
            frames,
        } = raw;
        AnimationMeta {
            width: width.unwrap_or(1),
            height: height.unwrap_or(1),
            frametime: frametime.unwrap_or(1).max(1),
            interpolate: interpolate.unwrap_or(false),
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
            assert!(font.defined_glyph('A').is_some());
            assert_eq!(font.char_width(' '), Some(4));
            assert!(font.char_width('W').unwrap() > font.char_width('i').unwrap_or(0));
            let en = a.language("en_us").unwrap();
            assert!(en.contains_key("block.minecraft.stone"));
            use haste_core::gui::TextMetrics;
            // Vanilla widths: glyph columns + 1 spacing column.
            assert_eq!(font.width("i"), 2);
            assert_eq!(font.width("l"), 3);
            assert_eq!(font.width("A"), 6);
            assert_eq!(font.width("W"), 6);
            assert_eq!(font.width(" "), 4);
            assert_eq!(font.width("Hello World"), 55); // H6 e6 l3 l3 o6 _4 W6 o6 r6 l3 d6
            assert!(
                font.len() > 10_000,
                "unifont not loaded from the assets index"
            );
            // Accented page (9x12 cells, height 12): measured in its own cell size.
            assert_eq!(font.width("É"), 6);
            // Unifont from the assets index (CJK: 16 columns at half scale + 1).
            if font.len() > 10_000 {
                assert_eq!(font.width("中"), 9);
            }
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
                    border: [10; 4],
                    stretch_inner: false
                }
            );
            let (_, meta) = a
                .gui_sprite(&Identifier::minecraft("tooltip/frame"))
                .unwrap();
            assert!(matches!(
                meta,
                SpriteMeta::NineSlice {
                    stretch_inner: true,
                    ..
                }
            ));
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

    // ---- animation schedule and frame splitting (synthetic mcmeta + images) ----

    /// A vertical 3-frame strip of 2x2 frames, colours distinguishable per frame.
    fn three_frame_strip() -> image::RgbaImage {
        let mut sheet = image::RgbaImage::new(2, 6);
        for (frame, color) in [(60u8, 0, 0, 255), (120, 60, 0, 255), (180, 120, 0, 255)]
            .iter()
            .enumerate()
        {
            for y in 0..2 {
                for x in 0..2 {
                    sheet.put_pixel(
                        x as u32,
                        (frame * 2 + y) as u32,
                        image::Rgba((*color).into()),
                    );
                }
            }
        }
        sheet
    }

    fn mcmeta(json: &str) -> AnimationMetaRaw {
        let file: TextureMetaFile = serde_json::from_str(&format!("{{\"animation\":{json}}}"))
            .unwrap_or_else(|e| panic!("bad test mcmeta {json}: {e}"));
        file.animation
            .expect("test mcmeta should have an animation section")
    }

    fn animation_of(sheet: image::RgbaImage, json: &str) -> Animation {
        animation_from_raw(sheet, &mcmeta(json))
    }

    #[test]
    fn default_schedule_steps_through_all_frames() {
        // No frames list, frametime 2: frames 0,1,2 each for 2 ticks, then wraps.
        let animation = animation_of(three_frame_strip(), r#"{"frametime":2}"#);
        assert_eq!(
            animation.schedule,
            AnimationSchedule::sequential(3, 2, false)
        );
        for (tick, frame) in [
            (0u64, 0u32),
            (1, 0),
            (2, 1),
            (3, 1),
            (4, 2),
            (5, 2),
            (6, 0),
            (7, 0),
        ] {
            assert_eq!(
                frame_at(&animation.schedule, tick),
                (frame, (frame + 1) % 3, 0.0),
                "tick {tick}"
            );
        }
    }

    #[test]
    fn custom_frames_with_per_frame_times() {
        // Index 2 for 1 tick, index 0 for 5 ticks (time overrides frametime).
        let animation = animation_of(
            three_frame_strip(),
            r#"{"frametime":2,"frames":[{"index":2,"time":1},{"index":0,"time":5}]}"#,
        );
        assert_eq!(
            animation.schedule.frames,
            vec![
                AnimationFrame { index: 2, time: 1 },
                AnimationFrame { index: 0, time: 5 }
            ]
        );
        assert_eq!(frame_at(&animation.schedule, 0), (2, 0, 0.0));
        assert_eq!(frame_at(&animation.schedule, 1), (0, 2, 0.0));
        assert_eq!(frame_at(&animation.schedule, 5), (0, 2, 0.0));
        assert_eq!(frame_at(&animation.schedule, 6), (2, 0, 0.0)); // wrap at total 6
                                                                   // An object without `time` uses frametime.
        let animation = animation_of(
            three_frame_strip(),
            r#"{"frametime":3,"frames":[1,{"index":2}]}"#,
        );
        assert_eq!(
            animation.schedule.frames,
            vec![
                AnimationFrame { index: 1, time: 3 },
                AnimationFrame { index: 2, time: 3 }
            ]
        );
    }

    #[test]
    fn frames_given_as_plain_ints() {
        let animation = animation_of(three_frame_strip(), r#"{"frametime":4,"frames":[2,0,2]}"#);
        assert_eq!(
            animation.schedule.frames,
            vec![
                AnimationFrame { index: 2, time: 4 },
                AnimationFrame { index: 0, time: 4 },
                AnimationFrame { index: 2, time: 4 }
            ]
        );
        assert_eq!(frame_at(&animation.schedule, 4), (0, 2, 0.0));
        // Last tick of the cycle: the next entry wraps to the first (index 2 again).
        assert_eq!(frame_at(&animation.schedule, 11), (2, 2, 0.0));
        assert_eq!(frame_at(&animation.schedule, 12), (2, 0, 0.0)); // wrapped
    }

    #[test]
    fn interpolate_blends_towards_next_frame() {
        let animation = animation_of(three_frame_strip(), r#"{"frametime":4,"interpolate":true}"#);
        assert!(animation.schedule.interpolate);
        // Blend is elapsed/time within each frame: 0 at the start, 0.5 halfway.
        assert_eq!(frame_at(&animation.schedule, 0), (0, 1, 0.0));
        assert_eq!(frame_at(&animation.schedule, 1), (0, 1, 0.25));
        assert_eq!(frame_at(&animation.schedule, 2), (0, 1, 0.5));
        assert_eq!(frame_at(&animation.schedule, 3), (0, 1, 0.75));
        // Next frame in playback order, wrapping at the end of the schedule.
        assert_eq!(frame_at(&animation.schedule, 10), (2, 0, 0.5));
        // Without interpolate the blend stays 0.
        let plain = animation_of(three_frame_strip(), r#"{"frametime":4}"#);
        assert_eq!(frame_at(&plain.schedule, 2), (0, 1, 0.0));
    }

    #[test]
    fn grid_layout_splits_frames_in_index_order() {
        // 2x2 grid of 2x2 frames: index order is row-major (left-right, top-bottom).
        let mut sheet = image::RgbaImage::new(4, 4);
        let colors = [
            (10u8, 0, 0, 255),
            (20, 0, 0, 255),
            (30, 0, 0, 255),
            (40, 0, 0, 255),
        ];
        for (index, color) in colors.iter().enumerate() {
            let (fx, fy) = ((index % 2) * 2, (index / 2) * 2);
            for y in 0..2 {
                for x in 0..2 {
                    sheet.put_pixel(
                        (fx + x) as u32,
                        (fy + y) as u32,
                        image::Rgba((*color).into()),
                    );
                }
            }
        }
        let animation = animation_of(sheet, r#"{"width":2,"height":2,"frametime":1}"#);
        assert_eq!(animation.frames.len(), 4);
        for (index, color) in colors.iter().enumerate() {
            assert_eq!(
                animation.frames[index].get_pixel(0, 0),
                &image::Rgba((*color).into()),
                "frame {index}"
            );
        }
        assert_eq!(animation.schedule.frames.len(), 4);
    }

    #[test]
    fn interpolate_flag_parses_and_initial_frame_is_first_listed() {
        let animation = animation_of(
            three_frame_strip(),
            r#"{"interpolate":true,"frames":[{"index":1,"time":2}]}"#,
        );
        assert!(animation.schedule.interpolate);
        assert_eq!(
            animation.initial_frame().get_pixel(0, 0),
            &image::Rgba([120, 60, 0, 255])
        );
        // The meta path carries the flag too.
        assert!(AnimationMeta::from(mcmeta(r#"{"interpolate":true}"#)).interpolate);
        assert!(!AnimationMeta::from(mcmeta(r#"{}"#)).interpolate);
    }
}
