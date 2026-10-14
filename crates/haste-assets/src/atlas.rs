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
    /// Level-0 content rectangle `(x, y, width, height)` in atlas texels.
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

        let mip_align = if with_mips { 1 << MAX_MIP_LEVEL } else { 1 };
        let mut total_area: u64 = (MISSING_TILE * MISSING_TILE) as u64;
        for source in &sources {
            if !source.data.is_empty() {
                total_area += u64::from(source.w.next_multiple_of(mip_align))
                    * u64::from(source.h.next_multiple_of(mip_align));
            }
        }
        let mut atlas_size = (((total_area as f64) * 1.4).sqrt().ceil() as u32).next_power_of_two();

        let (placements, missing_region) = loop {
            let (result, all_fit) = pack(&sources, atlas_size, mip_align);
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
        // Vanilla `MissingTextureAtlasSprite.generateMissingImage`.
        for py in 0..MISSING_TILE {
            for px in 0..MISSING_TILE {
                let pink = (py < MISSING_TILE / 2) ^ (px < MISSING_TILE / 2);
                let color: [u8; 4] = if pink {
                    [248, 0, 248, 255]
                } else {
                    [0, 0, 0, 255]
                };
                let idx = ((py * atlas_size + px) * 4) as usize;
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

        let pixels = if mip_levels > 0 {
            // Sprite list for the mip chain: the missing tile plus every
            // placed sprite, with its mip treatment (vanilla generates mips
            // per sprite, so cutout sprites never average in their
            // neighbours or the transparent-black texels).
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
            build_mip_chain(atlas_pixels, atlas_size, mip_levels, &mip_sprites)
        } else {
            atlas_pixels
        };

        tracing::info!(
            "Atlas built: {atlas_size}x{atlas_size}, mip levels {mip_levels}, {} regions",
            regions.len()
        );

        Ok(AtlasBuilder {
            regions,
            rects,
            pixels,
            size: atlas_size,
            mip_levels,
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
fn pack(sources: &[Source], atlas_size: u32, mip_align: u32) -> (PackResult, bool) {
    let mut placements: HashMap<String, Option<(u32, u32)>> = HashMap::new();
    let missing_region = pixel_region(0, 0, MISSING_TILE, MISSING_TILE);
    let mut cursor_x = MISSING_TILE;
    let mut cursor_y = 0;
    let mut shelf_h = MISSING_TILE;
    let mut all_fit = true;
    for src in sources {
        if src.data.is_empty() {
            placements.insert(src.name.clone(), None);
            continue;
        }
        if cursor_x + src.w > atlas_size {
            cursor_y = (cursor_y + shelf_h).next_multiple_of(mip_align);
            cursor_x = 0;
            shelf_h = 0;
        }
        if cursor_y + src.h > atlas_size {
            all_fit = false;
            placements.insert(src.name.clone(), None);
            continue;
        }
        placements.insert(src.name.clone(), Some((cursor_x, cursor_y)));
        cursor_x = (cursor_x + src.w).next_multiple_of(mip_align);
        shelf_h = shelf_h.max(src.h);
    }
    ((placements, missing_region), all_fit)
}

/// Concatenated mip chain, built per sprite (vanilla `MipmapGenerator`: mips
/// never cross a sprite boundary, and each sprite's chain follows its
/// `MipmapStrategy` — `Mean` is the plain linear-light 2x2 box filter; the
/// cutout variants solidify or darken level 0 first, then rescale each level's
/// alpha so the alpha-test coverage of level 0 survives the averaging).
/// `sprites` are level-0 rects with their strategy and cutoff bias; regions
/// outside sprites stay zero (never sampled).
fn build_mip_chain(
    mut level0: Vec<u8>,
    size: u32,
    mip_levels: u32,
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
    // vanilla TextureUtil: 3 * channel / 4 in int arithmetic (u8 would overflow)
    let dark = |c: u8| (3 * u16::from(c) / 4) as u8;
    let darkened = [dark(darkest[0]), dark(darkest[1]), dark(darkest[2])];
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
        let ((placements, _), all_fit) = pack(&sources, 64, 1);
        assert!(all_fit);
        let rects: Vec<(u32, u32)> = sources
            .iter()
            .map(|s| placements[&s.name].unwrap())
            .collect();
        for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                let (ax, ay) = rects[i];
                let (bx, by) = rects[j];
                let (aw, ah) = (sources[i].w, sources[i].h);
                let (bw, bh) = (sources[j].w, sources[j].h);
                let overlap = ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah;
                assert!(!overlap, "sprites {i} and {j} overlap");
            }
        }
    }

    #[test]
    fn mip_chain_concatenates_levels() {
        let pixels = vec![255u8; 32 * 32 * 4];
        let sprites = [([0, 0, 32, 32], MipmapStrategy::Mean, 0.0f32)];
        let chain = build_mip_chain(pixels.clone(), 32, 2, &sprites);
        assert_eq!(chain.len(), (32 * 32 + 16 * 16 + 8 * 8) * 4);
        assert_eq!(&chain[..pixels.len()], &pixels[..]);
        // every level is solid white
        for px in chain.as_chunks::<4>().0 {
            assert_eq!(px, &[255, 255, 255, 255]);
        }
    }

    /// An 8x8 cutout sprite (an "E" of green on transparent) placed next to
    /// an opaque red sprite in a 16x16 atlas, with a 2-level mip chain: the
    /// exact situation of grass tufts bleeding their neighbour's colour.
    fn cutout_next_to_red() -> Vec<u8> {
        let mut level0 = vec![0u8; 16 * 16 * 4];
        let mut put = |x: u32, y: u32, color: [u8; 4]| {
            let at = ((y * 16 + x) * 4) as usize;
            level0[at..at + 4].copy_from_slice(&color);
        };
        // Opaque red sprite at (8, 0)-(15, 7).
        for y in 0..8 {
            for x in 8..16 {
                put(x, y, [200, 20, 20, 255]);
            }
        }
        // Cutout sprite at (0, 0)-(7, 7): green texels with alpha 255 in an
        // E shape, transparent (alpha 0) around.
        for y in 0..8 {
            for x in 0..8 {
                let solid = x < 2 || (y == 0 || y == 3 || y == 7) && x < 6;
                put(
                    x,
                    y,
                    if solid {
                        [20, 180, 40, 255]
                    } else {
                        [0, 0, 0, 0]
                    },
                );
            }
        }
        let sprites = [
            ([0, 0, 8, 8], MipmapStrategy::Cutout, 0.0f32),
            ([8, 0, 8, 8], MipmapStrategy::Mean, 0.0f32),
        ];
        build_mip_chain(level0, 16, 2, &sprites)
    }

    /// No mip texel of the cutout sprite contains the red neighbour's colour:
    /// the per-sprite chain never averages across the sprite boundary.
    #[test]
    fn cutout_mips_never_blend_the_red_neighbour() {
        let chain = cutout_next_to_red();
        // Level 1 (8x8) starts after the 16x16 level 0; level 2 (4x4) after that.
        let level1 = &chain[16 * 16 * 4..][..8 * 8 * 4];
        let level2 = &chain[(16 * 16 + 8 * 8) * 4..][..4 * 4 * 4];
        for (level, dim) in [(level1, 8u32), (level2, 4u32)] {
            let half = dim / 2; // the cutout sprite's slot is the left half
            for y in 0..dim {
                for x in 0..half {
                    let at = ((y * dim + x) * 4) as usize;
                    // Not the red of the neighbouring sprite (and not a mix
                    // that keeps most of it).
                    let [r, _, _, _] = [level[at], level[at + 1], level[at + 2], level[at + 3]];
                    assert!(
                        r < 100,
                        "cutout mip texel ({x},{y}) is reddish: {r} — neighbour bled in"
                    );
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
        let level0 = &chain[..16 * 16 * 4];
        let level1 = &chain[16 * 16 * 4..][..8 * 8 * 4];
        // Every fully-opaque texel of level 0 within the cutout sprite.
        for y in 0..8u32 {
            for x in 0..8u32 {
                let at = ((y * 16 + x) * 4) as usize;
                if level0[at + 3] != 255 {
                    continue;
                }
                // Its level-1 covering texel keeps the green channel: a
                // darkened edge (transparent black mixed in) would sag well
                // below the opaque neighbours' green.
                let mip = ((y / 2) * 8 + x / 2) as usize * 4;
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
