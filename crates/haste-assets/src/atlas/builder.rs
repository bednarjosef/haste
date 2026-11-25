// This file is derived from Pomme (https://github.com/PommeMC/Client),
// pomme-client/src/renderer/chunk/atlas.rs, reused under GPL-3.0 and reworked
// for Haste: CPU-side RGBA buffers instead of Vulkan images (the wgpu renderer
// uploads them). See THIRD_PARTY.md.
//! `AtlasBuilder`: loads, stitches and mips one atlas (vanilla `SpriteLoader.stitch`).
use std::collections::HashMap;

use anyhow::Result;

use haste_core::Identifier;

use crate::assets::Assets;

use super::mipmap_generator::build_mip_chain;
use super::sprite_contents::load_source;
use super::stitcher::{pack, pixel_region, sprite_padding, MAX_MIP_LEVEL};
use super::{AnimatedSprite, AtlasRegion, AtlasRegions, MipmapStrategy, MISSING_TILE};
const MAX_ATLAS_SIZE: u32 = 8192;

impl AtlasRegions for AtlasBuilder {
    fn region_of(&self, texture_id: &str) -> AtlasRegion {
        self.region(texture_id)
    }
}

/// Packs sprites into one power-of-two RGBA atlas with a mip chain. The renderer
/// owns the GPU upload; this type produces the pixels plus the region lookup.
pub struct AtlasBuilder {
    /// Sprite key (`minecraft:block/stone` texture id string) -> placement.
    pub(super) regions: HashMap<String, AtlasRegion>,
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

    /// Mark a sprite's region translucent (the 26.3 model `force_translucent` field:
    /// a model can declare a sprite translucent whatever its texels say).
    pub fn region_mut(&mut self, texture_id: &str) -> &mut AtlasRegion {
        self.regions
            .get_mut(texture_id)
            .unwrap_or(&mut self.missing)
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

    /// Heap bytes of the retained mip chain plus the animated sprites' frames.
    pub fn pixel_bytes(&self) -> u64 {
        let mut n = self.pixels.capacity() as u64;
        for a in &self.animated {
            n += a
                .frames
                .iter()
                .map(|f| (f.width() * f.height() * 4) as u64)
                .sum::<u64>();
        }
        n
    }

    /// Drops the concatenated mip chain after the texture upload (the renderer's
    /// `upload_atlas`): the GPU texture owns the pixels from then on, and a
    /// resource-pack reload rebuilds the whole atlas from the assets, so retaining the
    /// chain is pure memory. The animated sprites' own frames stay (the sprite
    /// animator uploads them per tick); `pixel_bytes` reflects the drop.
    pub fn drop_uploaded_pixels(&mut self) {
        self.pixels = Vec::new();
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
