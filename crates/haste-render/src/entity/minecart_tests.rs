//! The minecart renderer's pure parts: the jitter seed's formula and the TNT cargo's swell,
//! flash and white-overlay numbers, against vanilla's
//! (`AbstractMinecartRenderer.submit`, `TntRenderer.getSwellAmount`/`isLit`,
//! `OverlayTexture`).
use super::minecart::{offset_seed, tnt_lit, tnt_swell, TNT_WHITE_OVERLAY};

/// Vanilla `entity.getId() * 493286711L` then `seed² * 4392167121L + seed * 98761L`, with
/// Java's wrapping long arithmetic: id 0 gives 0, and the three jitter bits the renderer
/// extracts always land inside the ±0.00175 the formula's `(v + 0.5) / 8 - 0.5` times 0.004
/// can produce.
#[test]
fn offset_seed_wraps_like_java_longs() {
    assert_eq!(offset_seed(0), 0);
    // The same product chain with explicit wrapping (guards a regression to i32 math or a
    // dropped wrap, which would change the jitter entirely).
    let java = |id: i64| {
        let seed = id.wrapping_mul(493286711);
        seed.wrapping_mul(seed)
            .wrapping_mul(4392167121)
            .wrapping_add(seed.wrapping_mul(98761))
    };
    for id in [1, 7, 1000, -3, i32::MAX as i64, i32::MIN as i64] {
        assert_eq!(offset_seed(id as i32), java(id), "entity id {id}");
    }
    for id in 1..64 {
        let seed = offset_seed(id);
        for shift in [16, 20, 24] {
            let v = ((seed >> shift) & 7) as f32;
            let offset = ((v + 0.5) / 8.0 - 0.5) * 0.004;
            assert!(
                (-0.002..0.002).contains(&offset),
                "id {id} shift {shift}: {offset}"
            );
        }
    }
}

/// `TntRenderer.getSwellAmount`: `(1 - fuse/10)` clamped to 0..1, fourth power, times 0.3.
#[test]
fn tnt_swell_follows_vanillas_curve() {
    assert!(
        (tnt_swell(10.0) - 0.0).abs() < 1e-6,
        "at ten ticks: nothing"
    );
    assert!(
        (tnt_swell(5.0) - 0.018_75).abs() < 1e-6,
        "half way: 0.5^4 * 0.3"
    );
    assert!(
        (tnt_swell(-1.0) - 0.3).abs() < 1e-6,
        "the last tick: full swell"
    );
    assert!((tnt_swell(0.0) - 0.3).abs() < 1e-6, "at zero: full swell");
}

/// `TntRenderer.isLit`: on while the fuse counts and its five-tick phase is even.
#[test]
fn tnt_lit_flashes_in_five_tick_phases() {
    assert!(!tnt_lit(-1.0), "not primed");
    assert!(tnt_lit(0.0));
    assert!(tnt_lit(4.9));
    assert!(!tnt_lit(5.0), "the second phase is dark");
    assert!(!tnt_lit(9.9));
    assert!(tnt_lit(10.0), "and on again");
}

/// The white overlay's strength is vanilla's `OverlayTexture` white row at full progress:
/// alpha `(1 - 15/15 * 0.75) * 255 = 63` of 255.
#[test]
fn tnt_white_overlay_is_the_overlay_textures_alpha() {
    assert!((TNT_WHITE_OVERLAY - 63.0 / 255.0).abs() < 1e-6);
}
