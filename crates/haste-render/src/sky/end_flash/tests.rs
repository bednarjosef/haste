// Expected values are full-precision prints of the f32 results (the Java LCG of
// java_random.rs, vanilla's draw order from EndFlashState.calculateFlashParameters).
#![allow(clippy::excessive_precision)]

use super::*;
use glam::Vec3;

/// The flash parameters of the first intervals, from vanilla's draw order: one discarded
/// `nextFloat`, then the offset, the duration, the x angle and the y angle.
#[test]
fn flash_params_follow_the_seeded_random() {
    let expect = [
        (0i64, 34i32, 160i32, -17.555_839_539f32, 49.470_260_62f32),
        (1, 133, 311, -31.479_215_622, -105.222_671_509),
        (2, 138, 296, -59.709_075_928, -1.143_890_381),
        (3, 86, 289, -3.632_457_733, -155.836_822_510),
    ];
    for (seed, offset, duration, x, y) in expect {
        let p = flash_params(seed);
        assert_eq!(p.offset, offset, "seed {seed}");
        assert_eq!(p.duration, duration, "seed {seed}");
        assert!((p.x_angle - x).abs() < 1e-6, "seed {seed}");
        assert!((p.y_angle - y).abs() < 1e-6, "seed {seed}");
    }
}

/// Every interval draws its parameters from vanilla's ranges: offset 0..=200 (which keeps
/// `min(380, 600 - offset)` at 380), duration 100..=380, x -60..10, y -180..180.
#[test]
fn flash_params_stay_in_vanillas_ranges() {
    for seed in 0..64 {
        let p = flash_params(seed);
        assert!((0..=200).contains(&p.offset), "seed {seed}");
        assert!((100..=380).contains(&p.duration), "seed {seed}");
        assert!((-60.0..=10.0).contains(&p.x_angle), "seed {seed}");
        assert!((-180.0..=180.0).contains(&p.y_angle), "seed {seed}");
    }
}

/// The brightness is a half-sine over `offset ..= offset + duration` and 0 outside it
/// (interval 1: offset 133, duration 311).
#[test]
fn intensity_is_a_half_sine_over_the_window() {
    let params = flash_params(1);
    assert_eq!((params.offset, params.duration), (133, 311));
    let at = |t: i64| intensity_at(600 + t, &params);
    assert_eq!(at(132), 0.0);
    assert_eq!(at(133), 0.0, "sin(0) at the window's start");
    // The peak sits at half the duration (155.5): both neighbours are within 0.1% of 1.
    assert!(at(133 + 155) > 0.999 && at(133 + 156) > 0.999);
    assert!(at(133 + 311).abs() < 1e-5, "sin(pi) at the window's end");
    assert_eq!(at(133 + 312), 0.0);
    assert_eq!(at(599), 0.0);
}

/// The state keeps vanilla's field defaults through interval 0, brackets the intensity
/// between ticks and reports when the flash starts (the sound cue's trigger).
#[test]
fn state_ticks_bracket_and_interpolate() {
    let mut state = EndFlashState::new();
    // Interval 0 keeps the zero parameters (Java's field defaults): no flash before 600.
    state.tick(599);
    assert_eq!(state.intensity(1.0), 0.0);
    assert_eq!(state.x_angle(), 0.0);
    // Interval 1 (offset 133, duration 311): the flash is on inside its window at 800.
    // The frame value interpolates from the last tick's 0 (partial 0) to this one's.
    state.tick(800);
    assert_eq!(state.intensity(0.0), 0.0, "partial 0 is the last tick");
    assert!(state.intensity(1.0) > 0.0);
    assert!(state.flash_started_this_tick());
    // The frame interpolation runs between the last two ticks' values.
    let half = state.intensity(0.5);
    state.tick(801);
    assert!(state.intensity(0.5) > half);
    // The direction angles are the interval's parameters.
    assert!((state.x_angle() - (-31.479_215_622)).abs() < 1e-6);
    assert!((state.y_angle() - (-105.222_671_509)).abs() < 1e-6);
    // Past the window the flash is over (offset 133 + duration 311 = 444).
    state.tick(600 + 445);
    assert_eq!(state.intensity(1.0), 0.0);
    assert!(!state.flash_started_this_tick());
}

/// Vanilla quirk: interval 0's zero parameters make tick 0 evaluate `sin(0 / 0)`, a NaN;
/// the `intensity > 1e-5` draw gate then draws nothing (NaN compares false), as vanilla's
/// `endFlashIntensity > 1.0E-5F` does.
#[test]
fn zero_parameters_at_clock_zero_are_nan_but_never_drawn() {
    let mut state = EndFlashState::new();
    state.tick(0);
    assert!(state.intensity(1.0).is_nan());
    assert!(state.intensity(1.0).partial_cmp(&1e-5) != Some(std::cmp::Ordering::Greater));
}

/// `renderEndFlash`'s pose: the quad sits 100 blocks out in the direction the two angles
/// pick, half-size 60 (`END_FLASH_HEIGHT` / `END_FLASH_SCALE`), tilted by the x angle.
#[test]
fn pose_places_the_quad_at_height_and_scale() {
    let center = |x: f32, y: f32| pose(x, y).transform_point3(Vec3::ZERO);
    // x = y = 0: rotateY(180) * rotateX(-90) carries (0, 100, 0) to (0, 0, 100).
    assert!((center(0.0, 0.0) - Vec3::new(0.0, 0.0, 100.0)).length() < 1e-4);
    // The y angle turns the direction about the world Y axis.
    assert!((center(0.0, 90.0) - Vec3::new(-100.0, 0.0, 0.0)).length() < 1e-4);
    // The x angle tilts the direction down by its own amount; the distance stays 100.
    let c = center(10.0, 0.0);
    assert!((c.y - (-100.0 * 10.0_f32.to_radians().sin())).abs() < 1e-3);
    assert!((c.length() - 100.0).abs() < 1e-3);
    // The quad's corners reach 60 blocks from its center, in its own plane.
    let m = pose(0.0, 0.0);
    let corner = m.transform_point3(Vec3::new(1.0, 0.0, 0.0));
    assert!((corner - Vec3::new(-60.0, 0.0, 100.0)).length() < 1e-3);
}
