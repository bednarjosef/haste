// This file is derived from Pomme (https://github.com/PommeMC/Client),
// pomme-client/src/renderer/chunk/atlas.rs, reused under GPL-3.0 and reworked
// for Haste: CPU-side RGBA buffers instead of Vulkan images (the wgpu renderer
// uploads them). See THIRD_PARTY.md.

use std::borrow::Cow;
use std::collections::HashMap;

use anyhow::{bail, Result};
use serde::Deserialize;

use haste_core::Identifier;

use crate::assets::{frame_at, Animation, AnimationSchedule, Assets};

/// A sprite's placement in the atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtlasRegion {
    /// Level-0 content rectangle `(x, y, width, height)` in atlas texels: the
    /// sprite itself, inset by the atlas's border from either side (vanilla
    /// `TextureAtlasSprite`: `u0 = (x + padding) / atlasWidth`). The border
    /// around it is the sprite's own edge texels, never sampled by UVs.
    pub pixel_rect: [u32; 4],
    /// Index into the atlas's sprite list; 0 is always the missing tile.
    pub sprite: u32,
    /// All level-0 texels are fully opaque (solid render pass eligible).
    pub opaque: bool,
    /// Some texel has alpha strictly between 0 and 255 (translucent class).
    pub translucent: bool,
}

impl AtlasRegion {
    pub fn u_min(&self) -> f32 {
        self.pixel_rect[0] as f32
    }
    pub fn u_max(&self) -> f32 {
        (self.pixel_rect[0] + self.pixel_rect[2]) as f32
    }
    pub fn v_min(&self) -> f32 {
        self.pixel_rect[1] as f32
    }
    pub fn v_max(&self) -> f32 {
        (self.pixel_rect[1] + self.pixel_rect[3]) as f32
    }
    pub fn width(&self) -> u32 {
        self.pixel_rect[2]
    }
    pub fn height(&self) -> u32 {
        self.pixel_rect[3]
    }
}

const MISSING_TILE: u32 = 16;

/// How a sprite's mip chain treats transparent texels (vanilla
/// `MipmapStrategy` from the texture's `.mcmeta` `texture` section; `auto`
/// picks per the sprite's own alpha).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MipmapStrategy {
    /// Average everything as-is (opaque or translucent sprites).
    Mean,
    /// Solidify first, then keep the alpha-test coverage of level 0 (cutout).
    Cutout,
    /// Like `Cutout` but against a stricter alpha reference of 0.3.
    StrictCutout,
    /// Transparent texels are filled with a darkened colour first (leaves).
    DarkCutout,
}

impl MipmapStrategy {
    /// Resolve `auto` from the sprite's pixels: any fully transparent texel
    /// makes it cutout (vanilla `Transparency.hasTransparent`).
    fn auto(data: &[u8]) -> Self {
        let has_transparent = data.as_chunks::<4>().0.iter().any(|px| px[3] == 0);
        if has_transparent {
            MipmapStrategy::Cutout
        } else {
            MipmapStrategy::Mean
        }
    }

    /// Whether the strategy treats the sprite as cutout (public for the
    /// renderer's animated-sprite updates).
    pub fn is_cutout(&self) -> bool {
        !matches!(self, MipmapStrategy::Mean)
    }

    /// The alpha-test reference the coverage match aims for (vanilla
    /// `cutoutRef`: strict uses 0.3, the rest 0.5).
    pub fn cutout_ref(&self) -> f32 {
        if matches!(self, MipmapStrategy::StrictCutout) {
            0.3
        } else {
            0.5
        }
    }
}

/// `texture` section of a texture's `.mcmeta` (vanilla `TextureMetadataSection`).
#[derive(Deserialize, Default)]
struct TextureMipmapMeta {
    #[serde(default)]
    texture: Option<TextureMipmapMetaTexture>,
}

#[derive(Deserialize, Default)]
struct TextureMipmapMetaTexture {
    #[serde(default)]
    mipmap_strategy: Option<String>,
    #[serde(default)]
    alpha_cutoff_bias: Option<f32>,
}

/// The mip strategy and alpha cutoff bias of a texture, from its `.mcmeta`
/// `texture` section (vanilla `SpriteContents` reads the same). `auto`
/// resolves per the sprite's pixels.
fn mipmap_meta(assets: &mut Assets, id: &Identifier, data: &[u8]) -> (MipmapStrategy, f32) {
    let fallback = (MipmapStrategy::auto(data), 0.0);
    let Ok(Some(bytes)) =
        assets.try_read_asset(&crate::assets::asset_key(id, "textures", ".png.mcmeta"))
    else {
        return fallback;
    };
    let Ok(meta) = serde_json::from_slice::<TextureMipmapMeta>(&bytes) else {
        return fallback;
    };
    let texture = meta.texture.unwrap_or_default();
    let strategy = match texture.mipmap_strategy.as_deref() {
        Some("mean") => MipmapStrategy::Mean,
        Some("cutout") => MipmapStrategy::Cutout,
        Some("strict_cutout") => MipmapStrategy::StrictCutout,
        Some("dark_cutout") => MipmapStrategy::DarkCutout,
        _ => MipmapStrategy::auto(data),
    };
    (strategy, texture.alpha_cutoff_bias.unwrap_or(0.0))
}

/// An animated sprite packed into the atlas: the region its current frame occupies
/// (all frames share it), the frames' RGBA pixels in schedule-index order, and the
/// playback schedule. The renderer re-uploads the region's texels each tick from
/// `frame_pixels`.
#[derive(Clone, Debug)]
pub struct AnimatedSprite {
    /// Texture id (`minecraft:block/water_still`) the animation belongs to.
    pub name: String,
    /// The atlas region all frames are shown in.
    pub region: AtlasRegion,
    /// Frames in schedule-index order; sizes match `region`.
    pub frames: Vec<image::RgbaImage>,
    pub schedule: AnimationSchedule,
    /// Mip treatment for the per-frame mip updates (vanilla `MipmapStrategy`).
    pub mipmap_strategy: MipmapStrategy,
    /// Alpha boost added when matching coverage (vanilla `alpha_cutoff_bias`).
    pub alpha_cutoff_bias: f32,
}

impl AnimatedSprite {
    /// The frame shown before animation starts (vanilla's initial display frame:
    /// the first listed frame, else frame 0). Panics when there are no frames.
    pub fn initial_frame(&self) -> &image::RgbaImage {
        let index = self.schedule.frames.first().map(|f| f.index).unwrap_or(0) as usize;
        self.frames.get(index).unwrap_or(&self.frames[0])
    }

    /// The RGBA pixels of the frame shown at `tick`, blending towards the next
    /// frame when the schedule interpolates. `Cow::Borrowed` is the exact frame
    /// (no interpolation or blend 0); blending allocates.
    pub fn frame_pixels(&self, tick: u64) -> Cow<'_, [u8]> {
        let (index, next, blend) = frame_at(&self.schedule, tick);
        let current = &self.frames[index as usize];
        if !self.schedule.interpolate || blend <= 0.0 {
            return Cow::Borrowed(current.as_raw());
        }
        let next = &self.frames[next as usize];
        Cow::Owned(blend_frames(current, next, blend))
    }
}

/// Linear per-channel blend of two same-size RGBA frames (vanilla `interpolate`;
/// alpha too, so a fading frame fades its coverage as well).
fn blend_frames(current: &image::RgbaImage, next: &image::RgbaImage, blend: f32) -> Vec<u8> {
    let (w, h) = current.dimensions();
    let mut out = Vec::with_capacity(current.as_raw().len());
    for y in 0..h {
        for x in 0..w {
            let a = current.get_pixel(x, y);
            let b = next.get_pixel(x, y);
            let mixed = [
                (f32::from(a.0[0]) * (1.0 - blend) + f32::from(b.0[0]) * blend).round() as u8,
                (f32::from(a.0[1]) * (1.0 - blend) + f32::from(b.0[1]) * blend).round() as u8,
                (f32::from(a.0[2]) * (1.0 - blend) + f32::from(b.0[2]) * blend).round() as u8,
                (f32::from(a.0[3]) * (1.0 - blend) + f32::from(b.0[3]) * blend).round() as u8,
            ];
            out.extend_from_slice(&mixed);
        }
    }
    out
}

type Placements = HashMap<String, Option<(u32, u32)>>;
type PackResult = (Placements, AtlasRegion);
const MAX_ATLAS_SIZE: u32 = 8192;
/// Vanilla's block-atlas mip chain depth (4 levels).
pub const MAX_MIP_LEVEL: u32 = 4;

/// Border width vanilla puts around every sprite in an atlas with `mip_levels`
/// levels (`Stitcher`: `padding = 1 << mipLevel`, with no anisotropic
/// filtering). The border holds the sprite's own edge texels, so a texel fetch
/// or a mip average at the sprite's edge stays inside the sprite instead of
/// pulling in the neighbour's.
pub const fn sprite_padding(mip_levels: u32) -> u32 {
    1 << mip_levels
}

/// Region lookup over a built atlas (so baking and tests can share it).
pub trait AtlasRegions {
    fn region_of(&self, texture_id: &str) -> AtlasRegion;
}

impl AtlasRegions for AtlasBuilder {
    fn region_of(&self, texture_id: &str) -> AtlasRegion {
        self.region(texture_id)
    }
}

/// Packs sprites into one power-of-two RGBA atlas with a mip chain. The renderer
/// owns the GPU upload; this type produces the pixels plus the region lookup.
pub struct AtlasBuilder {
    /// Sprite key (`minecraft:block/stone` texture id string) -> placement.
    regions: HashMap<String, AtlasRegion>,
    /// Level-0 rectangles by sprite index (index 0 = missing tile).
    rects: Vec<[u32; 4]>,
    /// Concatenated mip chain, level-major, RGBA8.
    pixels: Vec<u8>,
    size: u32,
    mip_levels: u32,
    /// Border width around every sprite's content (vanilla `Stitcher.padding`).
    padding: u32,
    missing: AtlasRegion,
    /// Sprites with an `.mcmeta` animation; packed with their first frame.
    animated: Vec<AnimatedSprite>,
}

impl AtlasBuilder {
    /// Packs the given textures (loaded through `assets`, animated ones reduced to
    /// their first frame) into an atlas of at most 8192². Missing textures become
    /// the missing tile (pink/black checker, vanilla `generateMissingImage`).
    pub fn build(
        assets: &mut Assets,
        texture_ids: &[Identifier],
        with_mips: bool,
    ) -> Result<AtlasBuilder> {
        let mut sources = Vec::with_capacity(texture_ids.len());
        for id in texture_ids {
            sources.push(load_source(assets, id));
        }
        // Sort by height descending for the shelf packer (Pomme).
        sources.sort_by_key(|s| std::cmp::Reverse(s.h.max(MISSING_TILE)));

        // Vanilla `SpriteLoader.stitch` picks the atlas mip level from the
        // smallest sprite before stitching; the `Stitcher` then derives every
        // sprite's border from it (`padding = 1 << mipLevel`), which doubles as
        // the mip alignment: both the border and the sprite then divide exactly
        // at every mip level.
        let mip_levels = if with_mips {
            let level = sources
                .iter()
                .filter(|s| !s.data.is_empty())
                .fold(MAX_MIP_LEVEL, |level, s| {
                    level.min(s.w.trailing_zeros().min(s.h.trailing_zeros()))
                });
            level.min(MAX_MIP_LEVEL)
        } else {
            0
        };
        let padding = sprite_padding(mip_levels);
        let mut total_area: u64 = u64::from((MISSING_TILE + 2 * padding).pow(2));
        for source in &sources {
            if !source.data.is_empty() {
                total_area += u64::from((source.w + 2 * padding).next_multiple_of(padding))
                    * u64::from(source.h + 2 * padding);
            }
        }
        let mut atlas_size = (((total_area as f64) * 1.4).sqrt().ceil() as u32).next_power_of_two();

        let (placements, missing_region) = loop {
            let (result, all_fit) = pack(&sources, atlas_size, padding);
            if all_fit || atlas_size >= MAX_ATLAS_SIZE {
                if !all_fit {
                    tracing::warn!(
                        "Atlas at {MAX_ATLAS_SIZE} cap; oversize sources fall back to missing tile"
                    );
                }
                break result;
            }
            atlas_size *= 2;
        };

        let mut atlas_pixels = vec![0u8; (atlas_size * atlas_size * 4) as usize];
        // Vanilla `MissingTextureAtlasSprite.generateMissingImage`, written at
        // the missing tile's content rect (its own border is filled with its
        // own edge texels below, like every other sprite's).
        for py in 0..MISSING_TILE {
            for px in 0..MISSING_TILE {
                let pink = (py < MISSING_TILE / 2) ^ (px < MISSING_TILE / 2);
                let color: [u8; 4] = if pink {
                    [248, 0, 248, 255]
                } else {
                    [0, 0, 0, 255]
                };
                let idx = (((py + padding) * atlas_size + px + padding) * 4) as usize;
                atlas_pixels[idx..idx + 4].copy_from_slice(&color);
            }
        }

        let mut regions = HashMap::new();
        let mut rects = vec![missing_region.pixel_rect];
        let mut animated = Vec::new();
        for src in &sources {
            match placements.get(src.name.as_str()) {
                Some(Some((cx, cy))) => {
                    let mut region = pixel_region(*cx, *cy, src.w, src.h);
                    region.sprite = rects.len() as u32;
                    region.opaque = src.opaque;
                    region.translucent = src.translucent;
                    rects.push(region.pixel_rect);
                    for py in 0..src.h {
                        for px in 0..src.w {
                            let s = ((py * src.w + px) * 4) as usize;
                            let d = (((cy + py) * atlas_size + cx + px) * 4) as usize;
                            atlas_pixels[d..d + 4].copy_from_slice(&src.data[s..s + 4]);
                        }
                    }
                    if let Some(animation) = &src.animation {
                        animated.push(AnimatedSprite {
                            name: src.name.clone(),
                            region,
                            frames: animation.frames.clone(),
                            schedule: animation.schedule.clone(),
                            mipmap_strategy: src.mipmap_strategy,
                            alpha_cutoff_bias: src.alpha_cutoff_bias,
                        });
                    }
                    regions.insert(src.name.clone(), region);
                }
                _ => {
                    regions.insert(src.name.clone(), missing_region);
                }
            }
        }

        let mip_levels = if with_mips {
            let level = sources
                .iter()
                .filter(|s| !s.data.is_empty())
                .fold(MAX_MIP_LEVEL, |level, s| {
                    level.min(s.w.trailing_zeros().min(s.h.trailing_zeros()))
                });
            level.min(MAX_MIP_LEVEL)
        } else {
            0
        };

        // Sprite list for the mip chain: the missing tile plus every placed
        // sprite, with its mip treatment (vanilla generates mips per sprite, so
        // cutout sprites never average in their neighbours or the
        // transparent-black texels). Without mips the chain is level 0 alone,
        // still with each sprite's border filled in.
        let mut mip_sprites = vec![(missing_region.pixel_rect, MipmapStrategy::Mean, 0.0f32)];
        for src in &sources {
            if let Some((cx, cy)) = placements.get(src.name.as_str()).copied().flatten() {
                mip_sprites.push((
                    [cx, cy, src.w, src.h],
                    src.mipmap_strategy,
                    src.alpha_cutoff_bias,
                ));
            }
        }
        let pixels = build_mip_chain(atlas_pixels, atlas_size, mip_levels, padding, &mip_sprites);

        tracing::info!(
            "Atlas built: {atlas_size}x{atlas_size}, mip levels {mip_levels}, border {padding}, {} regions",
            regions.len()
        );

        Ok(AtlasBuilder {
            regions,
            rects,
            pixels,
            size: atlas_size,
            mip_levels,
            padding,
            missing: missing_region,
            animated,
        })
    }

    pub fn region(&self, texture_id: &str) -> AtlasRegion {
        self.regions
            .get(texture_id)
            .copied()
            .unwrap_or(self.missing)
    }

    pub fn has(&self, texture_id: &str) -> bool {
        self.regions.contains_key(texture_id)
    }

    pub fn missing_region(&self) -> AtlasRegion {
        self.missing
    }

    /// Level-0 rectangles by sprite index (for the renderer's sprite-rect buffer).
    pub fn rects(&self) -> &[[u32; 4]] {
        &self.rects
    }

    /// Concatenated mip chain (level-major), RGBA8.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn mip_levels(&self) -> u32 {
        self.mip_levels
    }

    /// Border width around every sprite's content, in level-0 texels (vanilla
    /// `Stitcher.padding`). A sprite's slot in the atlas is its content rect
    /// grown by this much on all four sides; the border holds the sprite's own
    /// edge texels. For animated sprites, a region write must cover the whole
    /// slot so the border follows the new frame's edges.
    pub fn padding(&self) -> u32 {
        self.padding
    }

    /// Sprites with an `.mcmeta` animation, packed with their initial display frame;
    /// the renderer updates their regions each tick via `AnimatedSprite::frame_pixels`.
    pub fn animated_sprites(&self) -> &[AnimatedSprite] {
        &self.animated
    }
}

struct Source {
    name: String,
    /// First-frame RGBA pixels (empty = failed to load).
    data: Vec<u8>,
    w: u32,
    h: u32,
    opaque: bool,
    translucent: bool,
    /// Mip treatment (vanilla `MipmapStrategy`, `auto` resolved from pixels).
    mipmap_strategy: MipmapStrategy,
    /// Alpha boost for the coverage match (vanilla `alpha_cutoff_bias`).
    alpha_cutoff_bias: f32,
    /// The full animation, when the texture has an `.mcmeta` one.
    animation: Option<Animation>,
}

/// Load a texture through `Assets`, reduced to its first animation frame
/// (Pomme `load_source`); the full animation is kept for `animated_sprites`.
fn load_source(assets: &mut Assets, id: &Identifier) -> Source {
    let name = id.to_string();
    // `Assets::animation` reads the `.mcmeta` first and the PNG only when it
    // declares an animation; `texture_first_frame` is the static-path load.
    let animation = assets.animation(id);
    let image = match &animation {
        Some(a) => Ok(a.initial_frame().clone()),
        None => assets.texture_first_frame(id),
    };
    let Ok(image) = image else {
        tracing::warn!("Missing texture: {name}");
        return Source {
            name,
            data: Vec::new(),
            w: 0,
            h: 0,
            opaque: false,
            translucent: false,
            mipmap_strategy: MipmapStrategy::Mean,
            alpha_cutoff_bias: 0.0,
            animation: None,
        };
    };
    let (w, h) = image.dimensions();
    let data = image.into_raw();
    let (opaque, translucent) = sprite_transparency(&data);
    let (mipmap_strategy, alpha_cutoff_bias) = mipmap_meta(assets, id, &data);
    Source {
        name,
        data,
        w,
        h,
        opaque,
        translucent,
        mipmap_strategy,
        alpha_cutoff_bias,
        animation,
    }
}

/// (opaque, translucent) over every texel (Pomme `sprite_transparency`).
fn sprite_transparency(data: &[u8]) -> (bool, bool) {
    data.as_chunks::<4>()
        .0
        .iter()
        .fold((true, false), |(opaque, translucent), px| {
            (
                opaque && px[3] == 255,
                translucent || (px[3] != 0 && px[3] != 255),
            )
        })
}

fn pixel_region(x: u32, y: u32, w: u32, h: u32) -> AtlasRegion {
    AtlasRegion {
        pixel_rect: [x, y, w, h],
        sprite: 0,
        opaque: true,
        translucent: false,
    }
}

/// Shelf packer (Pomme `pack`): rows sorted by height, mip-aligned positions.
/// `padding` is vanilla's `Stitcher.padding` and doubles as the mip alignment
/// (both are `1 << mipLevel`): every sprite gets a `padding`-wide border on all
/// four sides, and both the slot origin and the content origin (slot + border)
/// stay aligned to `padding`, so a sprite's content and border divide exactly
/// at every mip level.
/// Placements hold the content origin (`slot + border`).
fn pack(sources: &[Source], atlas_size: u32, padding: u32) -> (PackResult, bool) {
    let mut placements: HashMap<String, Option<(u32, u32)>> = HashMap::new();
    // The missing tile is always placed, at the atlas origin, with its border.
    let missing_region = pixel_region(padding, padding, MISSING_TILE, MISSING_TILE);
    let missing_slot = MISSING_TILE + 2 * padding;
    let mut cursor_x = missing_slot;
    let mut cursor_y = 0;
    let mut shelf_h = missing_slot;
    let mut all_fit = true;
    for src in sources {
        if src.data.is_empty() {
            placements.insert(src.name.clone(), None);
            continue;
        }
        // The slot: content plus the border on both sides (vanilla
        // `Stitcher.Holder` = `smallestFittingMinTexel(w + padding * 2)`).
        let (slot_w, slot_h) = (src.w + 2 * padding, src.h + 2 * padding);
        if cursor_x + slot_w > atlas_size {
            cursor_y = (cursor_y + shelf_h).next_multiple_of(padding);
            cursor_x = 0;
            shelf_h = 0;
        }
        if cursor_y + slot_h > atlas_size {
            all_fit = false;
            placements.insert(src.name.clone(), None);
            continue;
        }
        placements.insert(
            src.name.clone(),
            Some((cursor_x + padding, cursor_y + padding)),
        );
        cursor_x = (cursor_x + slot_w).next_multiple_of(padding);
        shelf_h = shelf_h.max(slot_h);
    }
    ((placements, missing_region), all_fit)
}

/// Writes a sprite's own edge texels into the `border` texels around its
/// content at one mip level (vanilla `TextureAtlas.uploadInitialContents`: each
/// sprite is blitted into its padded slot through a clamp-to-edge sampler, so
/// every border texel — corners included — is a copy of the nearest edge texel
/// of the sprite's own mip level, never of a neighbouring sprite).
/// `content` is the sprite's rect at this level; `level_size` is the atlas side
/// at this level.
fn extend_border(pixels: &mut [u8], level_size: u32, content: [u32; 4], border: u32) {
    let [cx, cy, cw, ch] = content;
    if border == 0 || cw == 0 || ch == 0 || cx < border || cy < border {
        return;
    }
    let read = |pixels: &[u8], px: u32, py: u32| -> [u8; 4] {
        let at = ((py * level_size + px) * 4) as usize;
        [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
    };
    // Rows above and below: the content's first / last row, edge-extended
    // sideways (so the corners are the corner texel, like the clamped sample).
    for offset in 0..border {
        for (row, to) in [(cy, cy - border + offset), (cy + ch - 1, cy + ch + offset)] {
            let left = read(pixels, cx, row);
            let right = read(pixels, cx + cw - 1, row);
            for step in 0..border {
                write_texel(pixels, level_size, cx - border + step, to, left);
                write_texel(pixels, level_size, cx + cw + step, to, right);
            }
            for px in 0..cw {
                let color = read(pixels, cx + px, row);
                write_texel(pixels, level_size, cx + px, to, color);
            }
        }
    }
    // Columns beside the content rows.
    for py in 0..ch {
        let left = read(pixels, cx, cy + py);
        let right = read(pixels, cx + cw - 1, cy + py);
        for step in 0..border {
            write_texel(pixels, level_size, cx - border + step, cy + py, left);
            write_texel(pixels, level_size, cx + cw + step, cy + py, right);
        }
    }
}

fn write_texel(pixels: &mut [u8], size: u32, px: u32, py: u32, color: [u8; 4]) {
    let at = ((py * size + px) * 4) as usize;
    pixels[at..at + 4].copy_from_slice(&color);
}

/// Concatenated mip chain, built per sprite (vanilla `MipmapGenerator`: mips
/// never cross a sprite boundary, and each sprite's chain follows its
/// `MipmapStrategy` — `Mean` is the plain linear-light 2x2 box filter; the
/// cutout variants solidify or darken level 0 first, then rescale each level's
/// alpha so the alpha-test coverage of level 0 survives the averaging).
/// `padding` is the border width around each sprite (vanilla `Stitcher.padding`);
/// at every level the border is filled with the sprite's own edge texels
/// (`extend_border`), which is what keeps a mip average at the sprite's edge
/// from pulling in the neighbour. `sprites` are level-0 content rects with
/// their strategy and cutoff bias; regions outside the sprites' slots stay
/// zero (never sampled).
fn build_mip_chain(
    mut level0: Vec<u8>,
    size: u32,
    mip_levels: u32,
    padding: u32,
    sprites: &[([u32; 4], MipmapStrategy, f32)],
) -> Vec<u8> {
    // Level 0 pre-treatment per sprite, in place: cutout sprites get their
    // transparent texels' RGB replaced (alpha stays 0, so level 0 rendering
    // is unchanged — only the mip averages see the new colours).
    for (rect, strategy, _) in sprites {
        match strategy {
            MipmapStrategy::Cutout | MipmapStrategy::StrictCutout => {
                solidify(&mut level0, size, *rect);
            }
            MipmapStrategy::DarkCutout => {
                fill_empty_with_dark(&mut level0, size, *rect);
            }
            MipmapStrategy::Mean => {}
        }
    }

    let mut levels = Vec::with_capacity((mip_levels + 1) as usize);
    levels.push(level0);
    // Every level's border is the edge extension of that level's content, so
    // the border is written after the level's own content is final.
    for (rect, _, _) in sprites {
        extend_border(&mut levels[0], size, *rect, padding);
    }
    for level in 1..=mip_levels {
        let next = size >> level;
        let mut mip = vec![0u8; (next * next * 4) as usize];
        for (rect, strategy, bias) in sprites {
            let [x, y, w, h] = *rect;
            // The sprite's slot at the previous and this level (the packer's
            // mip alignment keeps both exact). Rows of the previous level are
            // atlas-wide, so the source stride is that level's size.
            let stride = size >> (level - 1);
            let (sx, sy) = (x >> (level - 1), y >> (level - 1));
            let (dx, dy, dw, dh) = (x >> level, y >> level, w >> level, h >> level);
            let previous = &levels[(level - 1) as usize];
            for py in 0..dh {
                for px in 0..dw {
                    let mut quad = [[0u8; 4]; 4];
                    for (i, (ox, oy)) in [(0u32, 0u32), (1, 0), (0, 1), (1, 1)]
                        .into_iter()
                        .enumerate()
                    {
                        let src = (((sy + py * 2 + oy) * stride + sx + px * 2 + ox) * 4) as usize;
                        quad[i].copy_from_slice(&previous[src..src + 4]);
                    }
                    let color = if matches!(strategy, MipmapStrategy::DarkCutout) {
                        darkened_alpha_blend(quad)
                    } else {
                        mean_linear(quad)
                    };
                    let dst = (((dy + py) * next + dx + px) * 4) as usize;
                    mip[dst..dst + 4].copy_from_slice(&color);
                }
            }
            // Cutout chains: keep the alpha-test coverage of level 0.
            if strategy.is_cutout() {
                let desired = coverage_of(&levels[0], size, *rect, strategy.cutout_ref());
                scale_alpha_to_coverage(
                    &mut mip,
                    next,
                    [dx, dy, dw, dh],
                    desired,
                    strategy.cutout_ref(),
                    *bias,
                );
            }
        }
        for (rect, _, _) in sprites {
            let [x, y, w, h] = *rect;
            extend_border(
                &mut mip,
                next,
                [x >> level, y >> level, w >> level, h >> level],
                padding >> level,
            );
        }
        levels.push(mip);
    }
    levels.concat()
}

/// Nearest-opaque-colour fill of transparent texels within `rect` (vanilla
/// `TextureUtil.solidify`: a BFS from every non-transparent texel; ties pick
/// whichever colour reached the texel first). Alpha is left untouched.
/// Public so the renderer can treat animated cutout sprites (fire) the same.
pub fn solidify(pixels: &mut [u8], size: u32, rect: [u32; 4]) {
    let [x, y, w, h] = rect;
    let index = |px: u32, py: u32| ((py * size + px) * 4) as usize;
    // Distance (in BFS steps) and colour of the nearest opaque texel.
    let mut distances = vec![u32::MAX; (w * h) as usize];
    let mut nearest = vec![[0u8; 3]; (w * h) as usize];
    let mut queue = std::collections::VecDeque::new();
    for py in 0..h {
        for px in 0..w {
            let at = (py * w + px) as usize;
            let alpha = pixels[index(x + px, y + py) + 3];
            if alpha != 0 {
                distances[at] = 0;
                nearest[at] = [
                    pixels[index(x + px, y + py)],
                    pixels[index(x + px, y + py) + 1],
                    pixels[index(x + px, y + py) + 2],
                ];
                queue.push_back((px, py));
            }
        }
    }
    while let Some((px, py)) = queue.pop_front() {
        let at = (py * w + px) as usize;
        for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (px as i32 + dx, py as i32 + dy);
            if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                continue;
            }
            let (nx, ny) = (nx as u32, ny as u32);
            let nat = (ny * w + nx) as usize;
            if distances[nat] > distances[at] + 1 {
                distances[nat] = distances[at] + 1;
                nearest[nat] = nearest[at];
                queue.push_back((nx, ny));
            }
        }
    }
    for py in 0..h {
        for px in 0..w {
            let at = (py * w + px) as usize;
            if pixels[index(x + px, y + py) + 3] == 0 {
                let base = index(x + px, y + py);
                pixels[base..base + 3].copy_from_slice(&nearest[at]);
            }
        }
    }
}

/// Fill of transparent texels with the sprite's darkest colour, darkened to
/// 3/4 (vanilla `TextureUtil.fillEmptyAreasWithDarkColor`).
/// Public for the renderer's animated-sprite updates.
pub fn fill_empty_with_dark(pixels: &mut [u8], size: u32, rect: [u32; 4]) {
    let [x, y, w, h] = rect;
    let index = |px: u32, py: u32| ((py * size + px) * 4) as usize;
    let mut darkest = None;
    let mut min_brightness = u32::MAX;
    for py in 0..h {
        for px in 0..w {
            let at = index(x + px, y + py);
            if pixels[at + 3] != 0 {
                let brightness =
                    u32::from(pixels[at]) + u32::from(pixels[at + 1]) + u32::from(pixels[at + 2]);
                if brightness < min_brightness {
                    min_brightness = brightness;
                    darkest = Some([pixels[at], pixels[at + 1], pixels[at + 2]]);
                }
            }
        }
    }
    let Some(darkest) = darkest else { return };
    // Vanilla computes `3 * channel / 4` in `int`
    // (TextureUtil.fillEmptyAreasWithDarkColor); widen here so the multiply
    // cannot overflow a byte before the division.
    let darkened = [
        (3 * u32::from(darkest[0]) / 4) as u8,
        (3 * u32::from(darkest[1]) / 4) as u8,
        (3 * u32::from(darkest[2]) / 4) as u8,
    ];
    for py in 0..h {
        for px in 0..w {
            let at = index(x + px, y + py);
            if pixels[at + 3] == 0 {
                pixels[at..at + 3].copy_from_slice(&darkened);
            }
        }
    }
}

/// 2x2 alpha blend that skips transparent texels entirely (vanilla
/// `MipmapGenerator.darkenedAlphaBlend`): each channel — alpha included — is
/// averaged in linear light over the four slots, with transparent texels
/// contributing nothing, then converted back.
/// Public for the renderer's animated-sprite updates.
pub fn darkened_alpha_blend(quad: [[u8; 4]; 4]) -> [u8; 4] {
    let mut totals = [0f32; 4];
    for pixel in quad {
        if pixel[3] != 0 {
            for channel in 0..4 {
                totals[channel] += srgb_to_linear_f32(pixel[channel]);
            }
        }
    }
    let mut out = [0u8; 4];
    for channel in 0..4 {
        out[channel] = linear_to_srgb_f32(totals[channel] / 4.0);
    }
    out
}

/// sRGB channel value (0..=255) as a linear float in [0, 1] (vanilla
/// `ARGB.srgbToLinearChannel` over the 10-bit table).
fn srgb_to_linear_f32(channel: u8) -> f32 {
    f32::from(srgb_to_linear_10(channel)) / 1023.0
}

/// Linear float back to an sRGB byte (vanilla `ARGB.linearToSrgbChannel`).
fn linear_to_srgb_f32(linear: f32) -> u8 {
    let index = (linear * 1023.0).floor().clamp(0.0, 1023.0) as u16;
    linear_10_to_srgb(index)
}

/// Fraction of a sprite's area that passes an alpha test at `reference`
/// (vanilla `MipmapGenerator.alphaTestCoverage`: 4x4 subsamples of the
/// bilinear interpolation of each 2x2 block, alpha at the reference or above
/// counts; averaged over the blocks).
/// Public for the renderer's animated-sprite updates.
pub fn coverage_of(pixels: &[u8], size: u32, rect: [u32; 4], reference: f32) -> f32 {
    let [x, y, w, h] = rect;
    if w < 2 || h < 2 {
        return f32::NAN; // vanilla divides by (w-1)*(h-1) and gets NaN
    }
    let alpha = |px: u32, py: u32| f32::from(pixels[((py * size + px) * 4 + 3) as usize]) / 255.0;
    let mut coverage = 0.0f32;
    for py in 0..h - 1 {
        for px in 0..w - 1 {
            let a00 = alpha(x + px, y + py);
            let a10 = alpha(x + px + 1, y + py);
            let a01 = alpha(x + px, y + py + 1);
            let a11 = alpha(x + px + 1, y + py + 1);
            let mut block_coverage = 0.0f32;
            for sy in 0..4u32 {
                let fy = (sy as f32 + 0.5) / 4.0;
                for sx in 0..4u32 {
                    let fx = (sx as f32 + 0.5) / 4.0;
                    let interpolated = a00 * (1.0 - fx) * (1.0 - fy)
                        + a10 * fx * (1.0 - fy)
                        + a01 * (1.0 - fx) * fy
                        + a11 * fx * fy;
                    if interpolated > reference {
                        block_coverage += 1.0;
                    }
                }
            }
            coverage += block_coverage / 16.0;
        }
    }
    coverage / ((w - 1) * (h - 1)) as f32
}

/// Rescale the alpha of a mip region so its alpha-test coverage at `reference`
/// matches `desired` (vanilla `MipmapGenerator.scaleAlphaToCoverage`: a
/// 5-step bisection on a multiply-and-bias; degenerate 1-texel regions skip
/// the search, matching vanilla's NaN break).
/// Public for the renderer's animated-sprite updates.
pub fn scale_alpha_to_coverage(
    pixels: &mut [u8],
    size: u32,
    rect: [u32; 4],
    desired: f32,
    reference: f32,
    bias: f32,
) {
    let [x, y, w, h] = rect;
    let index = |px: u32, py: u32| ((py * size + px) * 4 + 3) as usize;
    let mut min_scale = 0.0f32;
    let mut max_scale = 4.0f32;
    let mut scale = 1.0f32;
    let mut best_scale = 1.0f32;
    let mut best_error = f32::MAX;
    for _ in 0..5 {
        let current = coverage_of_rect(pixels, size, rect, reference, scale);
        let error = (current - desired).abs();
        if error < best_error {
            best_error = error;
            best_scale = scale;
        }
        if current < desired {
            min_scale = scale;
        } else if current > desired {
            max_scale = scale;
        } else {
            break;
        }
        scale = (min_scale + max_scale) * 0.5;
    }
    for py in 0..h {
        for px in 0..w {
            let at = index(x + px, y + py);
            let mut alpha = f32::from(pixels[at]) / 255.0;
            alpha = (alpha * best_scale + bias + 0.025).clamp(0.0, 1.0);
            pixels[at] = (alpha * 255.0).floor() as u8;
        }
    }
}

/// `coverage_of` with every alpha pre-scaled (vanilla passes an `alphaScale`
/// through `Math.clamp(alpha * scale, 0, 1)`).
fn coverage_of_rect(pixels: &[u8], size: u32, rect: [u32; 4], reference: f32, scale: f32) -> f32 {
    let [x, y, w, h] = rect;
    if w < 2 || h < 2 {
        return f32::NAN;
    }
    let alpha = |px: u32, py: u32| {
        (f32::from(pixels[((py * size + px) * 4 + 3) as usize]) / 255.0 * scale).clamp(0.0, 1.0)
    };
    let mut coverage = 0.0f32;
    for py in 0..h - 1 {
        for px in 0..w - 1 {
            let a00 = alpha(x + px, y + py);
            let a10 = alpha(x + px + 1, y + py);
            let a01 = alpha(x + px, y + py + 1);
            let a11 = alpha(x + px + 1, y + py + 1);
            let mut block_coverage = 0.0f32;
            for sy in 0..4u32 {
                let fy = (sy as f32 + 0.5) / 4.0;
                for sx in 0..4u32 {
                    let fx = (sx as f32 + 0.5) / 4.0;
                    let interpolated = a00 * (1.0 - fx) * (1.0 - fy)
                        + a10 * fx * (1.0 - fy)
                        + a01 * (1.0 - fx) * fy
                        + a11 * fx * fy;
                    if interpolated > reference {
                        block_coverage += 1.0;
                    }
                }
            }
            coverage += block_coverage / 16.0;
        }
    }
    coverage / ((w - 1) * (h - 1)) as f32
}

/// sRGB-correct 2x2 average (Pomme `mean_linear` via 10-bit linear tables).
/// Public so the renderer can downsample animated sprite regions the same way
/// the atlas mip chain is built.
pub fn mean_linear(pixels: [[u8; 4]; 4]) -> [u8; 4] {
    let mut output = [0u8; 4];
    for channel in 0..3 {
        let linear_sum: u32 = pixels
            .iter()
            .map(|pixel| u32::from(srgb_to_linear_10(pixel[channel])))
            .sum();
        output[channel] = linear_10_to_srgb((linear_sum / 4) as u16);
    }
    output[3] = (pixels.iter().map(|pixel| u32::from(pixel[3])).sum::<u32>() / 4) as u8;
    output
}

fn srgb_to_linear_10(channel: u8) -> u16 {
    static TABLE: std::sync::OnceLock<[u16; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|index| {
            let x = index as f32 / 255.0;
            let linear = if x >= 0.04045 {
                (((f64::from(x) + 0.055) / 1.055).powf(2.4)) as f32
            } else {
                x / 12.92
            };
            (linear * 1023.0).round() as u16
        })
    })[channel as usize]
}

fn linear_10_to_srgb(channel: u16) -> u8 {
    static TABLE: std::sync::OnceLock<[u8; 1024]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|index| {
            let x = index as f32 / 1023.0;
            let srgb = if x >= 0.0031308 {
                (1.055 * f64::from(x).powf(1.0 / 2.4) - 0.055) as f32
            } else {
                12.92 * x
            };
            (srgb * 255.0).round().clamp(0.0, 255.0) as u8
        })
    })[usize::from(channel)]
}

/// Guard for the atlas constructor in tests.
pub fn assert_power_of_two(n: u32) -> Result<()> {
    if n.is_power_of_two() {
        Ok(())
    } else {
        bail!("{n} is not a power of two")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprite_transparency_follows_vanilla_alpha_classes() {
        assert_eq!(
            sprite_transparency(&[0, 0, 0, 255, 0, 0, 0, 255]),
            (true, false)
        );
        assert_eq!(
            sprite_transparency(&[0, 0, 0, 255, 0, 0, 0, 0]),
            (false, false)
        );
        assert_eq!(
            sprite_transparency(&[0, 0, 0, 255, 0, 0, 0, 128]),
            (false, true)
        );
    }

    #[test]
    fn mean_mipmap_uses_linear_rgb() {
        let pixels = [
            [0, 0, 0, 255],
            [255, 255, 255, 255],
            [0, 0, 0, 255],
            [255, 255, 255, 255],
        ];
        assert_eq!(mean_linear(pixels), [187, 187, 187, 255]);
    }

    #[test]
    fn shelf_packer_places_no_overlap() {
        let sources = [
            Source {
                name: "a".into(),
                data: vec![255; 16 * 16 * 4],
                w: 16,
                h: 16,
                opaque: true,
                translucent: false,
                mipmap_strategy: MipmapStrategy::Mean,
                alpha_cutoff_bias: 0.0,
                animation: None,
            },
            Source {
                name: "b".into(),
                data: vec![255; 16 * 16 * 4],
                w: 16,
                h: 16,
                opaque: true,
                translucent: false,
                mipmap_strategy: MipmapStrategy::Mean,
                alpha_cutoff_bias: 0.0,
                animation: None,
            },
            Source {
                name: "c".into(),
                data: vec![255; 32 * 32 * 4],
                w: 32,
                h: 32,
                opaque: true,
                translucent: false,
                mipmap_strategy: MipmapStrategy::Mean,
                alpha_cutoff_bias: 0.0,
                animation: None,
            },
        ];
        // A 4-level mip chain: a 16-texel border on every side of each sprite
        // (vanilla `Stitcher.padding = 1 << mipLevel`), which is also the mip
        // alignment, so the content origin stays a multiple of it.
        let padding = sprite_padding(4);
        let ((placements, _), all_fit) = pack(&sources, 128, padding);
        assert!(all_fit);
        let rects: Vec<(u32, u32)> = sources
            .iter()
            .map(|s| placements[&s.name].unwrap())
            .collect();
        for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                let (ax, ay) = rects[i];
                let (bx, by) = rects[j];
                // Slots (content grown by the border) must not overlap either:
                // the border is what keeps a mip average inside the sprite.
                let (aw, ah) = (sources[i].w + 2 * padding, sources[i].h + 2 * padding);
                let (bw, bh) = (sources[j].w + 2 * padding, sources[j].h + 2 * padding);
                let overlap = ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah;
                assert!(!overlap, "sprites {i} and {j} overlap (border included)");
            }
        }
        // Content rects are inset by the border from the slot, and stay
        // multiples of it, so content and border divide exactly at every mip
        // level (vanilla `TextureAtlasSprite`: `u0 = (x + padding) / width`).
        for (x, y) in &rects {
            assert_eq!(x % padding, 0, "content x={x} is not border aligned");
            assert_eq!(y % padding, 0, "content y={y} is not border aligned");
        }
    }

    #[test]
    fn mip_chain_concatenates_levels() {
        let pixels = vec![255u8; 32 * 32 * 4];
        // One 24x24 sprite with a 4-texel border in a 32x32 atlas: slot and
        // content divide exactly at both mip levels.
        let sprites = [([4, 4, 24, 24], MipmapStrategy::Mean, 0.0f32)];
        let chain = build_mip_chain(pixels.clone(), 32, 2, 4, &sprites);
        assert_eq!(chain.len(), (32 * 32 + 16 * 16 + 8 * 8) * 4);
        assert_eq!(&chain[..pixels.len()], &pixels[..]);
        // every level is solid white
        for px in chain.as_chunks::<4>().0 {
            assert_eq!(px, &[255, 255, 255, 255]);
        }
    }

    /// Atlas side of the two-sprite test atlas below.
    const PAIR_ATLAS: u32 = 32;
    /// Border width of the two-sprite test atlas (a 2-level mip chain).
    const PAIR_PADDING: u32 = 4;
    /// Content rects of the two sprites in `PAIR_ATLAS` (their slots are these
    /// grown by `PAIR_PADDING` on all sides).
    const PAIR_CUTOUT: [u32; 4] = [4, 4, 8, 8];
    const PAIR_RED: [u32; 4] = [20, 4, 8, 8];

    /// An 8x8 cutout sprite (an "E" of green on transparent) placed next to an
    /// opaque red sprite, each with a 4-texel border, in a 32x32 atlas with a
    /// 2-level mip chain: the exact situation of grass tufts bleeding their
    /// neighbour's colour.
    fn cutout_next_to_red() -> Vec<u8> {
        let mut level0 = vec![0u8; (PAIR_ATLAS * PAIR_ATLAS * 4) as usize];
        let mut put = |x: u32, y: u32, color: [u8; 4]| {
            let at = ((y * PAIR_ATLAS + x) * 4) as usize;
            level0[at..at + 4].copy_from_slice(&color);
        };
        // Opaque red sprite.
        for y in 0..8 {
            for x in 20..28 {
                put(x, y, [200, 20, 20, 255]);
            }
        }
        // Cutout sprite: green texels with alpha 255 in an E shape, transparent
        // (alpha 0) around, and a black first column so each mip level's edge
        // colour differs per column.
        for y in 4..12 {
            for x in 4..12 {
                let solid = x < 6 || (y == 4 || y == 7 || y == 11) && x < 10;
                put(
                    x,
                    y,
                    if x == 4 {
                        [20, 20, 20, 255]
                    } else if solid {
                        [20, 180, 40, 255]
                    } else {
                        [0, 0, 0, 0]
                    },
                );
            }
        }
        let sprites = [
            (PAIR_CUTOUT, MipmapStrategy::Cutout, 0.0f32),
            (PAIR_RED, MipmapStrategy::Mean, 0.0f32),
        ];
        build_mip_chain(level0, PAIR_ATLAS, 2, PAIR_PADDING, &sprites)
    }

    /// The mip chain's level `level` as `(bytes, side)`.
    fn chain_level(chain: &[u8], level: u32) -> (&[u8], u32) {
        let side = PAIR_ATLAS >> level;
        let mut start = 0usize;
        for below in 0..level {
            let dim = PAIR_ATLAS >> below;
            start += (dim * dim * 4) as usize;
        }
        (&chain[start..start + (side * side * 4) as usize], side)
    }

    /// No mip texel of the cutout sprite contains the red neighbour's colour:
    /// the per-sprite chain never averages across the sprite boundary.
    #[test]
    fn cutout_mips_never_blend_the_red_neighbour() {
        let chain = cutout_next_to_red();
        for level in 0..=2u32 {
            let (pixels, side) = chain_level(&chain, level);
            // The sprite's slot: content plus the border on all sides.
            let (sw, sh) = (
                (PAIR_CUTOUT[2] + 2 * PAIR_PADDING) >> level,
                (PAIR_CUTOUT[3] + 2 * PAIR_PADDING) >> level,
            );
            for y in 0..sh {
                for x in 0..sw {
                    let at = ((y * side + x) * 4) as usize;
                    let [r, _, _, _] = [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]];
                    assert!(
                        r < 100,
                        "level {level} cutout texel ({x},{y}) is reddish: {r} — neighbour bled in"
                    );
                }
            }
        }
    }

    /// Every border texel of a sprite, at every mip level, is the sprite's own
    /// edge texel — the same clamp-to-edge extension vanilla's atlas blit
    /// produces — so a texel fetch or a mip average at a sprite's edge sees
    /// that sprite's edge and never its neighbour's.
    #[test]
    fn border_texels_equal_the_sprites_own_edges_at_every_mip_level() {
        let chain = cutout_next_to_red();
        for level in 0..=2u32 {
            let (pixels, side) = chain_level(&chain, level);
            let border = PAIR_PADDING >> level;
            for [cx, cy, cw, ch] in [PAIR_CUTOUT, PAIR_RED] {
                let (content, size) = ([cx >> level, cy >> level, cw >> level, ch >> level], side);
                for py in 0..(content[3] + 2 * border) {
                    for px in 0..(content[2] + 2 * border) {
                        let (bx, by) = (content[0] - border + px, content[1] - border + py);
                        let in_content = (content[0]..content[0] + content[2]).contains(&bx)
                            && (content[1]..content[1] + content[3]).contains(&by);
                        if in_content {
                            continue;
                        }
                        // The nearest edge texel of this level's content.
                        let ex = bx.clamp(content[0], content[0] + content[2] - 1);
                        let ey = by.clamp(content[1], content[1] + content[3] - 1);
                        let read = |px: u32, py: u32| {
                            let at = ((py * size + px) * 4) as usize;
                            [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
                        };
                        assert_eq!(
                            read(bx, by),
                            read(ex, ey),
                            "level {level} border texel ({bx},{by}) of sprite at ({cx},{cy}) is not the sprite's own edge"
                        );
                    }
                }
            }
        }
    }

    /// The cutout sprite's edge texels are not darkened: after solidify, the
    /// mip average of a transparent texel and its opaque neighbours is at
    /// least as bright as the darkest neighbour's channel (no black leaking
    /// from the transparent texel).
    #[test]
    fn cutout_mip_edges_are_not_darkened() {
        let chain = cutout_next_to_red();
        let (level0, side0) = chain_level(&chain, 0);
        let (level1, side1) = chain_level(&chain, 1);
        // Every fully-opaque texel of level 0 within the cutout sprite.
        for y in PAIR_CUTOUT[1]..PAIR_CUTOUT[1] + PAIR_CUTOUT[3] {
            for x in PAIR_CUTOUT[0]..PAIR_CUTOUT[0] + PAIR_CUTOUT[2] {
                let at = ((y * side0 + x) * 4) as usize;
                if level0[at + 3] != 255 {
                    continue;
                }
                // Its level-1 covering texel keeps the green channel: a
                // darkened edge (transparent black mixed in) would sag well
                // below the opaque neighbours' green.
                let mip = (((y >> 1) * side1 + (x >> 1)) * 4) as usize;
                let (g_opaque, g_mip) = (level0[at + 1], level1[mip + 1]);
                assert!(
                    g_mip as u32 + 20 > g_opaque as u32 / 2,
                    "texel ({x},{y}) green {g_opaque} -> mip {g_mip}: edge darkened"
                );
            }
        }
    }

    /// Solidify alone: transparent texels take the nearest opaque colour.
    #[test]
    fn solidify_fills_transparent_with_nearest_colour() {
        let mut pixels = vec![0u8; 4 * 4 * 4];
        let index = |x: u32, y: u32| ((y * 4 + x) * 4) as usize;
        pixels[index(0, 0)..index(0, 0) + 4].copy_from_slice(&[10, 200, 30, 255]);
        solidify(&mut pixels, 4, [0, 0, 4, 4]);
        // The far corner is 3 steps away but still gets the colour (alpha 0).
        let at = index(3, 3);
        assert_eq!(&pixels[at..at + 3], &[10, 200, 30]);
        assert_eq!(pixels[at + 3], 0);
    }

    #[test]
    fn frame_pixels_blends_black_and_white_frames() {
        // Two 1x1 frames at 2 ticks each with interpolation: halfway is ~128 gray.
        let sprite = AnimatedSprite {
            name: "test".into(),
            region: pixel_region(0, 0, 1, 1),
            frames: vec![
                image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255])),
                image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 255, 255, 255])),
            ],
            schedule: AnimationSchedule {
                frames: vec![
                    crate::assets::AnimationFrame { index: 0, time: 2 },
                    crate::assets::AnimationFrame { index: 1, time: 2 },
                ],
                interpolate: true,
            },
            mipmap_strategy: MipmapStrategy::Mean,
            alpha_cutoff_bias: 0.0,
        };
        // Not interpolating at blend 0: the exact frame, borrowed.
        assert_eq!(&*sprite.frame_pixels(0), &[0, 0, 0, 255]);
        assert_eq!(&*sprite.frame_pixels(2), &[255, 255, 255, 255]); // frame 1 starts
        assert_eq!(&*sprite.frame_pixels(4), &[0, 0, 0, 255]); // wrapped back to frame 0
                                                               // Halfway through frame 0: 50% black + 50% white.
        let blended = sprite.frame_pixels(1);
        assert_eq!(&*blended, &[128, 128, 128, 255]);
        // Without interpolation the frames switch abruptly with no blending.
        let mut plain = sprite.clone();
        plain.schedule.interpolate = false;
        assert_eq!(&*plain.frame_pixels(1), &[0, 0, 0, 255]);
        assert_eq!(&*plain.frame_pixels(2), &[255, 255, 255, 255]);
        // The initial frame is the first listed one.
        assert_eq!(
            sprite.initial_frame().get_pixel(0, 0),
            &image::Rgba([0, 0, 0, 255])
        );
    }

    /// Build time of the real block atlas with its mip chain (the renderer pays
    /// this once at startup). Needs the client jar and datagen reports in the
    /// cache (source scripts/env.sh).
    #[test]
    #[ignore = "needs the client jar and datagen reports in the cache"]
    fn perf_jar_atlas_build_time() {
        let jar = haste_core::paths::client_jar();
        if !jar.exists() {
            eprintln!("ignored: needs the client jar in the cache");
            return;
        }
        let mut assets =
            Assets::open(&jar, &haste_core::paths::cache_dir().join("assets"), "26.3").unwrap();
        let reports = haste_core::paths::datagen_reports();
        let states = crate::bake::BlocksJsonStates::load(&reports.join("blocks.json")).unwrap();
        let mut atlas = AtlasBuilder::build(&mut assets, &[], true).unwrap();
        let started = std::time::Instant::now();
        crate::bake::BakedBlockModels::bake(&mut assets, &states, &mut atlas).unwrap();
        let elapsed = started.elapsed();
        eprintln!(
            "atlas {}x{}, {} regions: bake pass + mip build {elapsed:?}",
            atlas.size(),
            atlas.size(),
            atlas.regions.len()
        );
    }

    #[test]
    #[ignore = "needs the client jar and datagen reports in the cache"]
    fn jar_atlas_animated_sprites() {
        let jar = haste_core::paths::client_jar();
        if !jar.exists() {
            eprintln!("ignored: needs the client jar in the cache");
            return;
        }
        let mut assets =
            Assets::open(&jar, &haste_core::paths::cache_dir().join("assets"), "26.3").unwrap();
        let reports = haste_core::paths::datagen_reports();
        let states = crate::bake::BlocksJsonStates::load(&reports.join("blocks.json")).unwrap();
        let mut atlas = AtlasBuilder::build(&mut assets, &[], true).unwrap();
        // Bake to discover the texture set, as the renderer does.
        crate::bake::BakedBlockModels::bake(&mut assets, &states, &mut atlas).unwrap();
        let animated = atlas.animated_sprites();
        let names: Vec<&str> = animated.iter().map(|s| s.name.as_str()).collect();
        eprintln!("animated sprites ({}): {:#?}", animated.len(), names);
        for expected in [
            "minecraft:block/water_still",
            "minecraft:block/lava_still",
            "minecraft:block/fire_0",
            "minecraft:block/nether_portal",
            "minecraft:block/sea_lantern",
        ] {
            assert!(
                names.contains(&expected),
                "expected {expected} among the animated sprites"
            );
        }
        // Every animated frame matches its atlas region's size.
        for sprite in animated {
            for frame in &sprite.frames {
                assert_eq!(
                    (frame.width(), frame.height()),
                    (sprite.region.width(), sprite.region.height()),
                    "{}",
                    sprite.name
                );
            }
        }
    }
}
