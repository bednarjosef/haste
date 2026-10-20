use haste_assets::RenderLayer;

use super::pyramid::level_sizes;
use super::*;

#[test]
fn pyramid_levels_are_the_mip_chain_of_half_the_target() {
    let sizes = level_sizes((1408, 828));
    assert_eq!(sizes[0], (704, 414));
    assert_eq!(sizes[1], (352, 207));
    assert_eq!(sizes[2], (176, 103));
    assert_eq!(*sizes.last().unwrap(), (1, 1));
    // wgpu's limit for a 704-wide texture: floor(log2(704)) + 1 levels.
    assert_eq!(sizes.len(), 10);
    assert_eq!(level_sizes((1, 1)), vec![(1, 1)]);
    assert_eq!(level_sizes((5, 3)), vec![(2, 1), (1, 1)]);
}

/// The source range `block` in pyramid.wgsl reduces into destination texel `x`.
fn block(x: u32, out: u32, src: u32) -> (u32, u32) {
    let end = if x == out - 1 { src - 1 } else { 2 * x + 1 };
    (2 * x, end.min(src - 1))
}

#[test]
fn every_pixel_lies_under_the_texel_the_test_reads() {
    // The cull reads level L's texel min(floor(px / 2^(L + 1)), last): that texel's
    // reduction must include the pixel, or a hidden-looking box could hide a visible
    // pixel. Follow each texel's pixel range down the chain, odd sizes included.
    for width in [1u32, 2, 3, 7, 828, 1407, 1408] {
        let sizes: Vec<u32> = level_sizes((width, 1)).iter().map(|s| s.0).collect();
        // Pixel range of each texel, level by level.
        let mut ranges: Vec<(u32, u32)> =
            (0..sizes[0]).map(|x| block(x, sizes[0], width)).collect();
        for (level, &w) in sizes.iter().enumerate() {
            if level > 0 {
                let prev = ranges.clone();
                ranges = (0..w)
                    .map(|x| {
                        let (a, b) = block(x, w, sizes[level - 1]);
                        (prev[a as usize].0, prev[b as usize].1)
                    })
                    .collect();
            }
            for px in 0..width {
                let texel = (px >> (level + 1)).min(w - 1);
                let (lo, hi) = ranges[texel as usize];
                assert!(
                    (lo..=hi).contains(&px),
                    "width {width} level {level}: pixel {px} not under texel {texel} ({lo}..={hi})"
                );
            }
        }
    }
}

fn draw(layer: RenderLayer, arena: u32, slot: u32) -> SectionDraw {
    SectionDraw {
        layer,
        arena,
        key: 0.0,
        first_index: 0,
        index_count: 6,
        base_vertex: 0,
        kind: crate::quad_format::SpanKind::Indexed,
        origin: [0; 3],
        slot,
        lo: [0.0; 3],
        hi: [1.0; 3],
    }
}

#[test]
fn runs_follow_the_draw_loops_layer_and_arena_split() {
    let draws = [
        draw(RenderLayer::Solid, 0, 4),
        draw(RenderLayer::Solid, 0, 5),
        draw(RenderLayer::Solid, 1, 6),
        draw(RenderLayer::Cutout, 1, 6),
        draw(RenderLayer::Translucent, 1, 7),
        draw(RenderLayer::Translucent, 0, 8),
    ];
    let mut out = Vec::new();
    assert_eq!(cull_draws(&draws, &mut out), 5);
    let runs: Vec<(u32, u32)> = out.iter().map(|d| (d.run, d.run_start)).collect();
    assert_eq!(runs, [(0, 0), (0, 0), (1, 2), (2, 3), (3, 4), (4, 5)]);
    let translucent: Vec<u32> = out.iter().map(|d| d.translucent).collect();
    assert_eq!(translucent, [0, 0, 0, 0, 1, 1]);
    assert_eq!(out[1].slot, 5);
    assert_eq!(cull_draws(&[], &mut out), 0);
    assert!(out.is_empty());
}
