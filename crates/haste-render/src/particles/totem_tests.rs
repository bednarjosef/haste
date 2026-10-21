use super::*;

use crate::particles::rng::Rng;

#[test]
fn fade_starts_after_half_the_life() {
    // `age > lifetime / 2` (strict): at exactly half the alpha is still untouched.
    assert_eq!(fade_alpha(30.0, 60.0), None);
    assert_eq!(fade_alpha(45.0, 60.0), Some(0.75));
    assert_eq!(fade_alpha(60.0, 60.0), Some(0.5));
}

#[test]
fn colour_branches_stay_in_vanilla_ranges() {
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    let mut gold = 0;
    for _ in 0..2000 {
        let c = totem_color(&mut rng);
        if c[0] >= 0.6 {
            // The gold branch: red 0.6..0.8, green 0.6..0.9, blue 0..0.2.
            gold += 1;
            assert!(c[0] <= 0.8 && c[1] >= 0.6 && c[1] <= 0.9 && c[2] <= 0.2);
        } else {
            // The green branch: red 0.1..0.3, green 0.4..0.7, blue 0..0.2.
            assert!(c[0] >= 0.1 && c[0] <= 0.3 && c[1] >= 0.4 && c[1] <= 0.7);
            assert!(c[2] <= 0.2);
        }
    }
    // One in four is gold: well inside 20..30 % over 2000 rolls.
    assert!((400..600).contains(&gold), "gold rolls: {gold}");
}
