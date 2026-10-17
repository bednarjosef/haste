use super::*;
use crate::assets::{AnimationSchedule, Assets};

use super::mipmap_generator::build_mip_chain;
use super::sprite_contents::{sprite_transparency, Source};
use super::stitcher::{pack, pixel_region};

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
