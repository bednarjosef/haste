//! Tests for the eat/drink use transform (`applyEatTransform`).

use super::*;
use crate::entity::hand::item_arm_matrix;
use crate::entity::FirstPerson;

/// One bread's `useDuration` (`Consumable.consumeTicks`, 1.6 s).
const BREAD: i32 = 32;

/// The transform applied to a point (the vanilla pose stack's effect), for comparing two
/// builds of the same matrix without float-equality on the whole matrix.
fn apply(m: Mat4, p: Vec3) -> Vec3 {
    m.transform_point3(p)
}

#[test]
fn the_eat_transform_is_vanillas_recipe() {
    // Rebuild the recipe independently and check a few remaining-tick values.
    for remaining in [32.0, 24.0, 16.0, 8.0, 4.0, 1.0] {
        let partial = 0.5f32;
        let curr = remaining - partial + 1.0;
        let scaled = curr / BREAD as f32;
        let mut want = Mat4::IDENTITY;
        if scaled < 0.8 {
            let extra = (curr / 4.0 * std::f32::consts::PI).cos().abs() * 0.1;
            want *= Mat4::from_translation(Vec3::new(0.0, extra, 0.0));
        }
        let jiggle = 1.0 - scaled.powf(27.0);
        want *= Mat4::from_translation(Vec3::new(jiggle * 0.6, jiggle * -0.5, 0.0));
        want *= Mat4::from_rotation_y((jiggle * 90.0f32).to_radians());
        want *= Mat4::from_rotation_x((jiggle * 10.0f32).to_radians());
        want *= Mat4::from_rotation_z((jiggle * 30.0f32).to_radians());
        let got = eat_transform(partial, remaining, BREAD);
        for p in [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(1.0, 2.0, 3.0)] {
            let (a, b) = (apply(got, p), apply(want, p));
            assert!(
                (a - b).length() < 1e-5,
                "remaining {remaining}: {a:?} != {b:?}"
            );
        }
    }
}

#[test]
fn the_wobble_only_shows_in_the_first_fifth_of_the_use() {
    // scaledUsageTime < 0.8 is the same as currUsageTime < 0.8 * duration = 25.6, so
    // with 1.6 s (32 ticks) the extra height is present while remaining <= ~24.
    let has_wobble = |remaining: f32| {
        let t = eat_transform(0.0, remaining, BREAD);
        // The Y translation of the matrix is the wobble plus the jiggle's -0.5.
        let jiggle = 1.0 - ((remaining + 1.0) / BREAD as f32).powf(27.0);
        t.w_axis.y - (jiggle * -0.5)
    };
    assert!(has_wobble(24.0).abs() > 1e-6, "wobble at remaining 24");
    assert_eq!(has_wobble(26.0), 0.0, "no wobble at remaining 26");
    // The wobble is |cos(curr/4 * PI)| * 0.1, capped at 0.1.
    for remaining in [24.0f32, 20.0, 16.0, 8.0, 4.0] {
        let curr = remaining + 1.0;
        let want = (curr / 4.0 * std::f32::consts::PI).cos().abs() * 0.1;
        assert!((has_wobble(remaining) - want).abs() < 1e-5);
    }
}

#[test]
fn the_jiggle_settles_into_the_hand_at_the_end_of_the_use() {
    // scaledUsageTime^27 -> 0 as the use runs out, so eatJiggle -> 1: the rotations are
    // the full 90/10/30 degrees at remaining 1 and zero once a full duration is left
    // (remaining 31 with partial 0 gives scaledUsageTime 1).
    let rot = |remaining: f32| {
        let scaled = (remaining + 1.0) / BREAD as f32;
        1.0 - scaled.powf(27.0)
    };
    assert!(rot(1.0) > 0.999, "jiggle at the end {}", rot(1.0));
    assert!(rot(31.0).abs() < 1e-6, "jiggle at the start {}", rot(31.0));
}

#[test]
fn the_eat_use_animation_replaces_the_swing() {
    // `FirstPersonHandsAndItemsRenderer.submitArmWithItem`'s EAT/DRINK branch: while the main
    // hand uses a consumable with ticks left, `item_arm_matrix` is `applyEatTransform` then
    // `applyItemArmTransform`, not the swing.
    let mut h = FirstPerson {
        item: Some(haste_core::RegistryIndex(1)),
        ..Default::default()
    };
    let swing = item_arm_matrix(&h, 0.0);
    h.use_eat = true;
    h.use_remaining = 20;
    h.use_duration = 32;
    let eating = item_arm_matrix(&h, 0.0);
    assert!(
        !eating.abs_diff_eq(swing, 1e-5),
        "the eat pose differs from the swing"
    );
    // A use with no ticks left draws the swing (vanilla checks useItemRemainingTicks > 0).
    h.use_remaining = 0;
    assert!(
        item_arm_matrix(&h, 0.0).abs_diff_eq(swing, 1e-6),
        "remaining 0 keeps the swing"
    );
}
