// Derived from Pomme (GPL-3.0); see THIRD_PARTY.md.

use anyhow::{Context, Result};
use haste_core::Identifier;
use serde::Deserialize;

use super::Assets;

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
    /// `assets/<ns>/<path>` of a full texture id such as `minecraft:textures/gui/x.png`, or
    /// `textures/<path>.png` of a short one (`minecraft:gui/x`), decoded.
    pub fn texture_any(&mut self, id: &Identifier) -> Result<image::RgbaImage> {
        if let Some(image) = self.dynamic.get(id) {
            return Ok(image.as_ref().clone());
        }
        let key = if id.path().ends_with(".png") {
            format!("assets/{}/{}", id.namespace(), id.path())
        } else {
            super::asset_key(id, "textures", ".png")
        };
        let bytes = self.read_asset(&key)?;
        decode_png(&bytes, &key)
    }

    /// `textures/<path>.png` of an identifier, e.g. `minecraft:block/stone`.
    /// The PNG is decoded; for animated textures the first frame is returned along
    /// with the parsed `AnimationMeta` (full-size image + meta when not animated).
    pub fn texture(&mut self, id: &Identifier) -> Result<image::RgbaImage> {
        let key = super::asset_key(id, "textures", ".png");
        let bytes = self.read_asset(&key)?;
        decode_png(&bytes, &key)
    }

    /// Parsed `.mcmeta` next to a texture, or default (not animated) when absent.
    pub fn texture_meta(&mut self, id: &Identifier) -> Result<TextureMeta> {
        let key = super::asset_key(id, "textures", ".png.mcmeta");
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

    /// The gui atlas sprite `id` with its `.mcmeta` scaling (`gui/scaling`:
    /// nine_slice with a uniform border in 26.3, tile, or stretch). The gui atlas
    /// definition's directory sources say where the texture lives (vanilla
    /// `Hud.getMobEffectSprite` ids like `mob_effect/regeneration` are under
    /// `textures/mob_effect/`, not `gui/sprites`); without a definition, the
    /// `gui/sprites` guess.
    pub fn gui_sprite(&mut self, id: &Identifier) -> Result<(image::RgbaImage, SpriteMeta)> {
        let key = self
            .atlas_sprite_texture_key(id, "gui")
            .unwrap_or_else(|| super::asset_key(id, "textures/gui/sprites", ".png"));
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

    /// All `assets/minecraft/textures/block/*.png` names (without extension).
    pub fn block_texture_names(&mut self) -> Vec<Identifier> {
        self.names_under("assets/minecraft/textures/block/", ".png")
            .map(|p| Identifier::minecraft(&format!("block/{p}")))
            .collect()
    }
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
            super::asset_key(id, "textures", ".png.mcmeta")
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
            super::asset_key(id, "textures", ".png.mcmeta")
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
pub(super) fn animation_from_raw(image: image::RgbaImage, raw: &AnimationMetaRaw) -> Animation {
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
pub(super) struct TextureMetaFile {
    pub(super) animation: Option<AnimationMetaRaw>,
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
pub(super) struct AnimationMetaRaw {
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
