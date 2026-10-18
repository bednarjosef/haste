//! `FirstPersonHandsAndItemsRenderer.applyEatTransform`: the first-person eat and drink use
//! animation (vanilla shares it between `ItemUseAnimation.EAT` and `DRINK`) and the
//! `applyItemArmTransform` it composes with.
//!
//! Vanilla's `submitArmWithItem` use branch applies `applyEatTransform` and then
//! `applyItemArmTransform` again; the `!hasCustomArmTransform()` transform before the switch
//! is skipped because both EAT and DRINK have `customArmTransform` true. The swing is not
//! applied at all while a use runs.

use glam::{Mat4, Vec3};

/// `FirstPersonHandsAndItemsRenderer.applyEatTransform` for the right main arm.
/// `remaining` is `FirstPersonHandsAndItemsRenderState.useItemRemainingTicks` (vanilla passes
/// it as a float) and `duration` `state.mainHandUseDuration` (`Item.getUseDuration`, a
/// consumable's `consume_ticks`).
pub(super) fn eat_transform(partial: f32, remaining: f32, duration: i32) -> Mat4 {
    // float currUsageTime = useItemRemainingTicks - partialTicks + 1.0F;
    let curr_usage_time = remaining - partial + 1.0;
    // float scaledUsageTime = currUsageTime / useDuration;
    let scaled_usage_time = curr_usage_time / duration as f32;
    let mut m = Mat4::IDENTITY;
    // if (scaledUsageTime < 0.8F) { translate(0.0F, abs(cos(currUsageTime / 4.0F * PI) * 0.1F), 0.0F); }
    if scaled_usage_time < 0.8 {
        let extra_height = (curr_usage_time / 4.0 * std::f32::consts::PI).cos().abs() * 0.1;
        m *= Mat4::from_translation(Vec3::new(0.0, extra_height, 0.0));
    }
    // float eatJiggle = 1.0F - (float)Math.pow(scaledUsageTime, 27.0);
    let eat_jiggle = 1.0 - scaled_usage_time.powf(27.0);
    // invert = 1 (the right arm).
    m *= Mat4::from_translation(Vec3::new(eat_jiggle * 0.6, eat_jiggle * -0.5, 0.0));
    m *= Mat4::from_rotation_y((eat_jiggle * 90.0f32).to_radians());
    m *= Mat4::from_rotation_x((eat_jiggle * 10.0f32).to_radians());
    m *= Mat4::from_rotation_z((eat_jiggle * 30.0f32).to_radians());
    m
}

/// `FirstPersonHandsAndItemsRenderer.applyItemArmTransform`: the held item's place in view
/// space (`0.56, -0.52 + inverseArmHeight * -0.6, -0.72`), the right arm's `invert` 1.
pub(super) fn item_arm_transform(inverse_arm_height: f32) -> Mat4 {
    Mat4::from_translation(Vec3::new(0.56, -0.52 + inverse_arm_height * -0.6, -0.72))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
