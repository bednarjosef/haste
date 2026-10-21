use super::*;

/// `FireworkEntityRenderer.submit`'s shot-at-angle turn (Z 180, Y 180, X 90, in that order):
/// the rocket's up (+Y) ends pointing to -Z.
#[test]
fn firework_flight_turn_is_z_then_y_then_x() {
    let mut b = BodyTransform::default();
    b.push(BodyOp::RotateZ(std::f32::consts::PI));
    b.push(BodyOp::RotateY(std::f32::consts::PI));
    b.push(BodyOp::RotateX(std::f32::consts::FRAC_PI_2));
    let p = body_matrix(&b).transform_point3(Vec3::Y);
    assert!(p.x.abs() < 1e-5, "x {}", p.x);
    assert!(p.y.abs() < 1e-5, "y {}", p.y);
    assert!((p.z - -1.0).abs() < 1e-5, "z {}", p.z);
}
