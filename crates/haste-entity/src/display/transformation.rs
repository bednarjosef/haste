//! `com.mojang.math.Transformation` as the display entities use it: the four synched parts
//! (translation, left rotation, scale, right rotation), their matrix and the part-wise slerp
//! `Display.TransformationInterpolator` eases between two of them with.
//!
//! The arithmetic follows JOML (MIT), the maths library vanilla runs: quaternions are used as
//! sent (never normalized), so a rotation quaternion's matrix is JOML's homogeneous form
//! (`Matrix4f.rotate(Quaternionfc)`), which scales by the squared norm of a non-unit
//! quaternion exactly like the game does.

use glam::{Mat4, Vec3, Vec4};

/// One display transformation (`Transformation(translation, leftRotation, scale,
/// rightRotation)`); quaternions as (x, y, z, w).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transformation {
    pub translation: Vec3,
    pub left_rotation: Vec4,
    pub scale: Vec3,
    pub right_rotation: Vec4,
}

impl Transformation {
    /// `Transformation.IDENTITY`, and the synched defaults (`Display.defineSynchedData`:
    /// zero translation, unit scale, identity rotations).
    pub const IDENTITY: Transformation = Transformation {
        translation: Vec3::ZERO,
        left_rotation: Vec4::new(0.0, 0.0, 0.0, 1.0),
        scale: Vec3::ONE,
        right_rotation: Vec4::new(0.0, 0.0, 0.0, 1.0),
    };

    /// `Transformation.compose`: `translation(t).rotate(left).scale(s).rotate(right)`, each
    /// step multiplied on the right like JOML's.
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_translation(self.translation)
            * rotation_matrix(self.left_rotation)
            * Mat4::from_scale(self.scale)
            * rotation_matrix(self.right_rotation)
    }

    /// `Transformation.slerp`: translation and scale lerped, both rotations slerped
    /// (`Vector3f.lerp`, `Quaternionf.slerp`).
    pub fn slerp(&self, to: &Transformation, t: f32) -> Transformation {
        Transformation {
            translation: lerp3(self.translation, to.translation, t),
            left_rotation: slerp(self.left_rotation, to.left_rotation, t),
            scale: lerp3(self.scale, to.scale, t),
            right_rotation: slerp(self.right_rotation, to.right_rotation, t),
        }
    }
}

/// JOML `Vector3f.lerp`: `a + (b - a) * t` per component (fused).
fn lerp3(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    Vec3::new(
        (b.x - a.x).mul_add(t, a.x),
        (b.y - a.y).mul_add(t, a.y),
        (b.z - a.z).mul_add(t, a.z),
    )
}

/// JOML `Quaternionf.slerp(target, alpha)`: the shorter arc (the target's sign flipped when
/// the dot product is negative), a plain lerp when the two are within 1e-6 of parallel, and
/// no normalization of the result.
pub fn slerp(q: Vec4, target: Vec4, alpha: f32) -> Vec4 {
    let cosom = q.dot(target);
    let abs_cosom = cosom.abs();
    let (scale0, mut scale1) = if 1.0 - abs_cosom > 1e-6 {
        let sin_sqr = 1.0 - abs_cosom * abs_cosom;
        let sinom = 1.0 / sin_sqr.sqrt();
        let omega = (sin_sqr * sinom).atan2(abs_cosom);
        (
            ((1.0 - alpha) * omega).sin() * sinom,
            (alpha * omega).sin() * sinom,
        )
    } else {
        (1.0 - alpha, alpha)
    };
    if cosom < 0.0 {
        scale1 = -scale1;
    }
    q * scale0 + target * scale1
}

/// JOML `Matrix4f.rotate(Quaternionfc)` from the identity: the homogeneous rotation matrix
/// (`w2 + x2 - y2 - z2` on the diagonal), which equals the usual rotation for a unit
/// quaternion and carries the squared norm as a uniform scale otherwise.
pub fn rotation_matrix(q: Vec4) -> Mat4 {
    let (x, y, z, w) = (q.x, q.y, q.z, q.w);
    let (x2, y2, z2, w2) = (x * x, y * y, z * z, w * w);
    let (dxy, dxz, dyz) = (2.0 * x * y, 2.0 * x * z, 2.0 * y * z);
    let (dxw, dyw, dzw) = (2.0 * x * w, 2.0 * y * w, 2.0 * z * w);
    Mat4::from_cols(
        Vec4::new(w2 + x2 - z2 - y2, dxy + dzw, dxz - dyw, 0.0),
        Vec4::new(dxy - dzw, y2 - z2 + w2 - x2, dyz + dxw, 0.0),
        Vec4::new(dyw + dxz, dyz - dxw, z2 - y2 - x2 + w2, 0.0),
        Vec4::W,
    )
}
