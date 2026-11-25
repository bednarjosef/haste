// This file is derived from Pomme (https://github.com/PommeMC/Client),
// pomme-client/src/renderer/chunk/atlas.rs, reused under GPL-3.0 and reworked
// for Haste: CPU-side RGBA buffers instead of Vulkan images (the wgpu renderer
// uploads them). See docs/THIRD_PARTY.md.
//! The shelf packer and sprite borders (vanilla `Stitcher`).
use std::collections::HashMap;

use super::sprite_contents::Source;
use super::{AtlasRegion, MISSING_TILE};
pub(super) type Placements = HashMap<String, Option<(u32, u32)>>;
pub(super) type PackResult = (Placements, AtlasRegion);
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

pub(super) fn pixel_region(x: u32, y: u32, w: u32, h: u32) -> AtlasRegion {
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
pub(super) fn pack(sources: &[Source], atlas_size: u32, padding: u32) -> (PackResult, bool) {
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
