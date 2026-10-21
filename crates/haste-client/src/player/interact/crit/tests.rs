// `Player.canCriticalAttack` and `Player.attack`'s `fullStrengthAttack` (`> 0.9F`), with the
// vanilla conditions one at a time.

use super::{can_critical_attack, full_strength, CritInputs};

/// A falling attacker in the air, not sprinting, on no climbable, in no water, not blind, not riding.
fn falling() -> CritInputs {
    CritInputs {
        fall_distance: 1.5,
        on_ground: false,
        on_climbable: false,
        in_water: false,
        blind: false,
        passenger: false,
        sprinting: false,
    }
}

#[test]
fn full_strength_is_strictly_above_point_nine() {
    assert!(!full_strength(0.9));
    assert!(full_strength(0.91));
    assert!(full_strength(1.0));
    assert!(!full_strength(0.5));
}

#[test]
fn falling_in_the_air_is_a_crit() {
    assert!(can_critical_attack(&falling()));
}

#[test]
fn no_fall_distance_is_not_a_crit() {
    let c = CritInputs {
        fall_distance: 0.0,
        ..falling()
    };
    assert!(!can_critical_attack(&c));
}

#[test]
fn standing_on_the_ground_is_not_a_crit() {
    let c = CritInputs {
        on_ground: true,
        ..falling()
    };
    assert!(!can_critical_attack(&c));
}

#[test]
fn each_blocking_condition_rules_the_crit_out() {
    let blockers: [fn(&mut CritInputs); 5] = [
        |c| c.on_climbable = true,
        |c| c.in_water = true,
        |c| c.blind = true,
        |c| c.passenger = true,
        |c| c.sprinting = true,
    ];
    for block in blockers {
        let mut c = falling();
        block(&mut c);
        assert!(!can_critical_attack(&c), "{c:?}");
    }
}
