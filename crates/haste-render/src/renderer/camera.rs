// Camera math (vanilla `GameRenderer`/`Camera`): the pose stack of hurt tilt, death
// roll and view bobbing multiplied onto the projection, the view rotation, the
// nausea/portal screen warp, and frustum-plane extraction for the culls.
use super::{Camera, SpinWarp};

impl super::Renderer {
    /// The pose stack vanilla's `GameRenderer` multiplies onto the projection matrix
    /// (`renderLevel`: bobHurt, then bobView), plus the camera's plain roll. In view space
    /// (x right, y up, z back), as rotations about +Z roll the horizon one way.
    pub(crate) fn view_transform(camera: &Camera) -> glam::Mat4 {
        // The plain roll sits in front of everything else (`Camera.setRotation`).
        glam::Mat4::from_axis_angle(glam::Vec3::Z, camera.roll_deg.to_radians())
            * Self::bob_transform(camera)
    }

    /// The same stack without the plain camera roll: the death roll, the hurt tilt and the
    /// view bobbing (`GameRenderer.bobHurt` + `bobView`). Vanilla's `renderItemInHand` puts
    /// exactly this onto the hand's pose stack, after the view rotation and its inverse have
    /// cancelled, so the hand bobs with the world but never takes the camera's plain roll.
    pub(crate) fn bob_transform(camera: &Camera) -> glam::Mat4 {
        let b = camera.bob;
        let z = glam::Vec3::Z;
        let y = glam::Vec3::Y;
        // The death roll (a plain z-axis rotation), then the hurt tilt around the z-axis
        // turned by the damage direction.
        let m = glam::Mat4::from_axis_angle(z, b.death_roll_deg.to_radians())
            * glam::Mat4::from_axis_angle(y, -b.hurt_dir_deg.to_radians())
            * glam::Mat4::from_axis_angle(z, b.hurt_tilt_deg.to_radians())
            * glam::Mat4::from_axis_angle(y, b.hurt_dir_deg.to_radians());
        // The view bobbing: translate in the camera plane, then roll, then pitch.
        m * glam::Mat4::from_translation(glam::Vec3::new(b.x, b.y, 0.0))
            * glam::Mat4::from_axis_angle(z, b.roll_deg.to_radians())
            * glam::Mat4::from_axis_angle(glam::Vec3::X, b.pitch_deg.to_radians())
    }

    /// Build the view-projection matrix (right-handed, OpenGL-style depth like
    /// wgpu's default reverse... wgpu uses 0..1 z; glam's perspective_rh
    /// produces -1..1, so use perspective_infinite_rh style correction).
    pub(crate) fn view_projection(
        camera: &Camera,
        aspect: f32,
        spin: Option<SpinWarp>,
    ) -> glam::Mat4 {
        let mut proj =
            glam::Mat4::perspective_rh(camera.fov_deg.to_radians(), aspect, 0.05, 4096.0);
        if let Some(spin) = spin {
            // Vanilla's `GameRenderer.renderLevel`: rotate the projection about
            // (0, √2/2, √2/2) by the spinning angle, squeeze x by 1/skew, rotate back. The
            // multiplications are right-to-left in effect, so this is
            // P * R(axis, angle) * S(1/skew, 1, 1) * R(axis, -angle).
            let axis = glam::Vec3::new(
                0.0,
                std::f32::consts::SQRT_2 / 2.0,
                std::f32::consts::SQRT_2 / 2.0,
            );
            let angle = spin.angle_deg.to_radians();
            let skew = spin.skew();
            proj *= glam::Mat4::from_axis_angle(axis, angle);
            proj *= glam::Mat4::from_diagonal(glam::Vec4::new(1.0 / skew, 1.0, 1.0, 1.0));
            proj *= glam::Mat4::from_axis_angle(axis, -angle);
        }
        let eye = glam::Vec3::new(
            camera.pos.x as f32,
            camera.pos.y as f32,
            camera.pos.z as f32,
        );
        let view = Self::view_rotation(camera) * glam::Mat4::from_translation(-eye);
        proj * Self::view_transform(camera) * view
    }

    /// The camera's rotation as vanilla builds it (`Camera.setRotation`: a YXZ rotation from yaw
    /// and pitch, plus the roll), so looking straight up or down (pitch +-90) is exact and never
    /// flips, unlike a look-at with a fixed up vector. View space looks along -Z.
    pub(super) fn view_rotation(camera: &Camera) -> glam::Mat4 {
        glam::Mat4::from_rotation_z(-camera.roll_deg.to_radians())
            * glam::Mat4::from_rotation_x(camera.pitch.to_radians())
            * glam::Mat4::from_rotation_y((camera.yaw + 180.0).to_radians())
    }

    /// Frustum culling of sections (Pomme cull.comp reference; CPU here).
    pub(crate) fn frustum_planes(m: &glam::Mat4) -> [glam::Vec4; 6] {
        // Gribb/Hartmann: planes are row3 +- row_i of the clip matrix (clip = M * p).
        // wgpu clip z is 0..w: the near plane is row2 alone.
        let r = |i: usize| m.row(i);
        let (r0, r1, r2, r3) = (r(0), r(1), r(2), r(3));
        [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2]
            .map(|p| p / p.truncate().length().max(1e-12))
    }

    /// AABB vs the frustum planes (positive-vertex test, conservative).
    pub(super) fn aabb_visible(planes: &[glam::Vec4; 6], min: [f32; 3], max: [f32; 3]) -> bool {
        // Conservative: AABB vs plane with the positive vertex.
        for p in planes {
            let px = if p.x >= 0.0 { max[0] } else { min[0] };
            let py = if p.y >= 0.0 { max[1] } else { min[1] };
            let pz = if p.z >= 0.0 { max[2] } else { min[2] };
            if p.x * px + p.y * py + p.z * pz + p.w < 0.0 {
                return false;
            }
        }
        true
    }
}
