use super::*;

#[test]
fn star_brightness_follows_the_day_timeline() {
    let close = |t: i64, want: f32| {
        let got = star_brightness(t);
        assert!((got - want).abs() < 1e-5, "tick {t}: {got} vs {want}");
    };
    close(6000, 0.0);
    close(12399, 0.143);
    close(12729, 0.258);
    close(18000, 0.5);
    close(92, 0.037);
    // Halfway between 12399 and 12729.
    close(12564, (0.143 + 0.258) / 2.0);
    // The wrap segment 23758 -> 24092: 0.101 down to 0.037.
    close(23925, 0.101 + (0.037 - 0.101) * (167.0 / 334.0));
    close(24000 + 12399, 0.143);
}
