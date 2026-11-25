use super::*;

/// Vanilla `EnchantTableRenderer.extractRenderState` turns the book the short way: when `rot`
/// wrapped from just under PI to just over -PI during the tick, the interpolated yaw keeps
/// going past PI instead of spinning back through zero.
#[test]
fn enchant_rot_lerps_the_short_way_across_the_wrap() {
    let book = BlockEntityAnim::EnchantTable {
        time: 0.0,
        flip: TickPair::fixed(0.0),
        open: TickPair::fixed(1.0),
        rot: TickPair {
            start: 3.0,
            end: -3.0,
        },
        t_rot: -3.0,
        flip_t: 0.0,
        flip_a: 0.0,
    };
    let BlockEntityAnim::EnchantTable { rot, .. } = book.at(0.5) else {
        panic!("not a book");
    };
    // The step is 2 PI - 6 (about 0.283) forwards; half of it from 3.0.
    let expected = 3.0 + (std::f32::consts::TAU - 6.0) * 0.5;
    assert!((rot.end - expected).abs() < 1e-5, "{}", rot.end);
}

/// Vanilla's angle wrap lands in [-PI, PI).
#[test]
fn wrap_pi_lands_in_range() {
    use std::f32::consts::PI;
    assert!((wrap_pi(PI) + PI).abs() < 1e-6);
    assert!((wrap_pi(3.0 * PI + 0.5) - (PI + 0.5 - 2.0 * PI)).abs() < 1e-5);
    assert!((wrap_pi(-PI - 0.25) - (PI - 0.25)).abs() < 1e-5);
    assert_eq!(wrap_pi(0.5), 0.5);
}
