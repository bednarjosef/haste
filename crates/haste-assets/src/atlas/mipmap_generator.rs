// This file is derived from Pomme (https://github.com/PommeMC/Client),
// pomme-client/src/renderer/chunk/atlas.rs, reused under GPL-3.0 and reworked
// for Haste: CPU-side RGBA buffers instead of Vulkan images (the wgpu renderer
// uploads them). See THIRD_PARTY.md.
//! The per-sprite mip chain (vanilla `MipmapGenerator`).
use super::border::extend_border;
use super::srgb::{linear_10_to_srgb, linear_to_srgb_f32, srgb_to_linear_10, srgb_to_linear_f32};
use super::texture_util::{fill_empty_with_dark, solidify};
use super::MipmapStrategy;
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
pub(super) fn build_mip_chain(
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
