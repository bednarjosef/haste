//! The riptide spin layer's geometry (`SpinAttackEffectModel` through
//! `ModelPart.translateAndRotate`): the part scale about the pivot, and where each box lands
//! around a player standing at the origin.
use super::spin_attack::{box_scale, is_spin_layer, scale_about};
use super::*;

/// The generic path's model root for a player at the origin facing south with no body turn:
/// vanilla's `scale(-1, -1, 1)` flip in sixteenths (feet at y = 0 in model units).
fn root() -> Mat4 {
    Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0) / 16.0)
}

/// The world-space bounds of box `i` drawn with `yaw` (radians) in its pose.
fn box_bounds(i: usize, yaw: f32) -> (Vec3, Vec3) {
    let model = haste_entity::spin_attack::layer();
    let part = &model.root.children[i];
    let mut pose = PoseOverrides::default();
    pose.set_rotation(part.id, Vec3::new(0.0, yaw, 0.0));
    let parent = root() * scale_about(part.pivot, box_scale(i));
    let mut out = Vec::new();
    emit_part_tinted(
        &mut out,
        &model,
        part,
        &pose,
        parent,
        [0, 0, 64, 64],
        (15, 15),
        0.0,
        [1.0; 3],
    );
    assert!(!out.is_empty());
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for v in &out {
        let p = Vec3::from_array(v.pos);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    (lo, hi)
}

fn assert_vec_near(a: Vec3, b: Vec3) {
    assert!(a.abs_diff_eq(b, 1e-4), "{a:?} != {b:?}");
}

#[test]
fn the_layer_is_recognised_by_its_texture_and_scales_its_boxes() {
    assert!(is_spin_layer(&haste_entity::spin_attack::layer()));
    assert!(!is_spin_layer(&haste_entity::models::humanoid(false)));
    assert_eq!((box_scale(0), box_scale(1)), (0.75, 1.5));
}

#[test]
fn scaling_about_the_pivot_is_translate_rotate_scale() {
    // `translateAndRotate`: translate(pivot), rotate, scale. The renderer's parent scale about
    // the pivot followed by the part's own translate and rotate must give the same matrix.
    let pivot = Vec3::new(0.0, -24.0, 0.0);
    let rot = Mat4::from_rotation_y(0.7);
    let s = 1.5;
    let ours = scale_about(pivot, s) * Mat4::from_translation(pivot) * rot;
    let vanilla = Mat4::from_translation(pivot) * rot * Mat4::from_scale(Vec3::splat(s));
    for p in [Vec3::new(-8.0, 0.0, -8.0), Vec3::new(8.0, 32.0, 8.0)] {
        assert_vec_near(ours.transform_point3(p), vanilla.transform_point3(p));
    }
}

#[test]
fn the_big_box_spans_a_block_and_a_half_either_side_of_the_model_origin() {
    // Box 1: cube y 0..32 scaled 1.5 from the origin 24 units above the feet, so it reaches
    // from the origin (1.5 blocks up) to 1.5 blocks below the feet; 24 units wide.
    let (lo, hi) = box_bounds(1, 0.0);
    assert_vec_near(lo, Vec3::new(-0.75, -1.5, -0.75));
    assert_vec_near(hi, Vec3::new(0.75, 1.5, 0.75));
}

#[test]
fn the_small_box_sits_around_the_upper_body() {
    // Box 0: cube y -9.6..22.4 scaled 0.75 = -7.2..16.8 below the origin: 0.45 to 1.95 blocks
    // above the feet, 12 units wide.
    let (lo, hi) = box_bounds(0, 0.0);
    assert_vec_near(lo, Vec3::new(-0.375, 0.45, -0.375));
    assert_vec_near(hi, Vec3::new(0.375, 1.95, 0.375));
}

#[test]
fn the_box_turns_about_the_vertical_through_the_origin() {
    // A quarter turn keeps the square footprint and the height.
    let (lo, hi) = box_bounds(1, std::f32::consts::FRAC_PI_2);
    assert_vec_near(lo, Vec3::new(-0.75, -1.5, -0.75));
    assert_vec_near(hi, Vec3::new(0.75, 1.5, 0.75));
}
