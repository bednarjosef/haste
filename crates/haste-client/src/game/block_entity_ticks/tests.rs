use super::*;

/// Every draw advances the table's random (the page-flip `do while` relies on it) and stays
/// in `0..bound`, for the power-of-two shortcut and the rejection path alike.
#[test]
fn model_next_int_advances_and_stays_in_range() {
    let mut rng = 0x2545_f491_4f6c_dd1d_u64 & ((1 << 48) - 1);
    for bound in [4u32, 40] {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..400 {
            let before = rng;
            let v = model_next_int(&mut rng, bound);
            assert_ne!(rng, before, "the seed did not advance");
            assert!(v < bound, "{v} >= {bound}");
            seen.insert(v);
        }
        assert_eq!(
            seen.len(),
            bound as usize,
            "not every value drawn for {bound}"
        );
    }
}

/// The draw is Java's LCG (`LegacyRandomSource`): one step of the scrambled seed, the top
/// 31 bits, and `nextInt(4)`'s power-of-two shortcut over them (seed 0 scrambles to
/// `0x5DEECE66D`).
#[test]
fn model_next_int_matches_the_java_lcg() {
    let mut rng = 0x5_DEEC_E66D_u64;
    // next(31) by hand: one LCG step, the top 31 of 48 bits.
    let step = (0x5_DEEC_E66D_u64
        .wrapping_mul(0x5_DEEC_E66D)
        .wrapping_add(11))
        & ((1 << 48) - 1);
    let next31 = (step >> 17) as i64;
    assert_eq!(model_next_int(&mut rng, 4) as i64, (4 * next31) >> 31);
    assert_eq!(rng, step);
}

/// The page-flip loop of `bookAnimationTick` ends: with a player next to the table the first
/// ticks (open below 0.5) each change `flipT`.
#[test]
fn page_flip_target_changes_every_opening_tick() {
    let mut rng = 1u64;
    let mut flip_t = 0.0f32;
    for _ in 0..50 {
        let old = flip_t;
        loop {
            let a = model_next_int(&mut rng, 4) as i32;
            let b = model_next_int(&mut rng, 4) as i32;
            flip_t += (a - b) as f32;
            if old != flip_t {
                break;
            }
        }
        assert_ne!(old, flip_t);
    }
}
