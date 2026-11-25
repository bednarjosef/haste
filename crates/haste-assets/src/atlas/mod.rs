// This file is derived from Pomme (https://github.com/PommeMC/Client),
// pomme-client/src/renderer/chunk/atlas.rs, reused under GPL-3.0 and reworked
// for Haste: CPU-side RGBA buffers instead of Vulkan images (the wgpu renderer
// uploads them). See docs/THIRD_PARTY.md.

//! The block and item texture atlas: sprites loaded from the packs, stitched into one
//! power-of-two RGBA image with a per-sprite mip chain (vanilla's `SpriteLoader`,
//! `Stitcher`, `SpriteContents` and `MipmapGenerator`).
use anyhow::{bail, Result};

mod animated_sprite;
mod border;
mod builder;
mod mipmap_generator;
mod mipmap_strategy;
mod sprite_contents;
mod srgb;
mod stitcher;
#[cfg(test)]
mod tests;
mod texture_util;

pub use animated_sprite::AnimatedSprite;
pub use builder::AtlasBuilder;
pub use mipmap_generator::{
    coverage_of, darkened_alpha_blend, mean_linear, scale_alpha_to_coverage,
};
pub use mipmap_strategy::MipmapStrategy;
pub use stitcher::{sprite_padding, MAX_MIP_LEVEL};
pub use texture_util::{fill_empty_with_dark, solidify};
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

/// Region lookup over a built atlas (so baking and tests can share it).
pub trait AtlasRegions {
    fn region_of(&self, texture_id: &str) -> AtlasRegion;
}

/// Guard for the atlas constructor in tests.
pub fn assert_power_of_two(n: u32) -> Result<()> {
    if n.is_power_of_two() {
        Ok(())
    } else {
        bail!("{n} is not a power of two")
    }
}
