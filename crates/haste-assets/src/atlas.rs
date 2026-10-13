// This file is derived from Pomme (https://github.com/PommeMC/Client),
// pomme-client/src/renderer/chunk/atlas.rs, reused under GPL-3.0 and reworked
// for Haste: CPU-side RGBA buffers instead of Vulkan images (the wgpu renderer
// uploads them). See THIRD_PARTY.md.

use std::collections::HashMap;

use anyhow::{bail, Result};

use haste_core::Identifier;

use crate::assets::Assets;

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
            build_mip_chain(atlas_pixels, atlas_size, mip_levels)
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
}

struct Source {
    name: String,
    /// First-frame RGBA pixels (empty = failed to load).
    data: Vec<u8>,
    w: u32,
    h: u32,
    opaque: bool,
    translucent: bool,
}

/// Load a texture through `Assets`, reduced to its first animation frame
/// (Pomme `load_source`).
fn load_source(assets: &mut Assets, id: &Identifier) -> Source {
    let name = id.to_string();
    // `texture_first_frame` applies vanilla's frame-size defaults (square frames of the image
    // width when the .mcmeta gives no size); the raw `AnimationMeta` sizes default to 1.
    let Ok(image) = assets.texture_first_frame(id) else {
        tracing::warn!("Missing texture: {name}");
        return Source {
            name,
            data: Vec::new(),
            w: 0,
            h: 0,
            opaque: false,
            translucent: false,
        };
    };
    let (w, h) = image.dimensions();
    let data = image.into_raw();
    let (opaque, translucent) = sprite_transparency(&data);
    Source {
        name,
        data,
        w,
        h,
        opaque,
        translucent,
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

/// Concatenated mip chain: plain 2x2 box filter per level (Pomme `build_mip_chain`
/// without the per-texture strategies; cutout sprites get nearest-preserving alpha
/// through `mean_linear`, which is linear-light averaging of RGB plus alpha mean).
fn build_mip_chain(mut level0: Vec<u8>, size: u32, mip_levels: u32) -> Vec<u8> {
    let mut levels = Vec::with_capacity((mip_levels + 1) as usize);
    levels.push(level0.clone());
    let mut current = size;
    for _ in 1..=mip_levels {
        let next = current / 2;
        let mut mip = vec![0u8; (next * next * 4) as usize];
        for y in 0..next {
            for x in 0..next {
                let mut pixels = [[0u8; 4]; 4];
                for (i, (ox, oy)) in [(0u32, 0u32), (1, 0), (0, 1), (1, 1)]
                    .into_iter()
                    .enumerate()
                {
                    let src = (((y * 2 + oy) * current + x * 2 + ox) * 4) as usize;
                    pixels[i].copy_from_slice(&level0[src..src + 4]);
                }
                let color = mean_linear(pixels);
                let dst = ((y * next + x) * 4) as usize;
                mip[dst..dst + 4].copy_from_slice(&color);
            }
        }
        levels.push(mip.clone());
        level0 = mip;
        current = next;
    }
    levels.concat()
}

/// sRGB-correct 2x2 average (Pomme `mean_linear` via 10-bit linear tables).
fn mean_linear(pixels: [[u8; 4]; 4]) -> [u8; 4] {
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
            },
            Source {
                name: "b".into(),
                data: vec![255; 16 * 16 * 4],
                w: 16,
                h: 16,
                opaque: true,
                translucent: false,
            },
            Source {
                name: "c".into(),
                data: vec![255; 32 * 32 * 4],
                w: 32,
                h: 32,
                opaque: true,
                translucent: false,
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
        let chain = build_mip_chain(pixels.clone(), 32, 2);
        assert_eq!(chain.len(), (32 * 32 + 16 * 16 + 8 * 8) * 4);
        assert_eq!(&chain[..pixels.len()], &pixels[..]);
        // every level is solid white
        for px in chain.as_chunks::<4>().0 {
            assert_eq!(px, &[255, 255, 255, 255]);
        }
    }
}
