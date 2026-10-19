//! The arrow renderer (vanilla `ArrowRenderer` / `TippableArrowRenderer` /
//! `SpectralArrowRenderer` over `ArrowModel`): the model's three flat boxes (`back` and the
//! two `cross_` quills) posed by the arrow's flight rotation and the stuck-in-ground shake,
//! textured from `textures/entity/projectiles/`.

use super::*;

/// One quad of `ArrowModel`: four corners in blocks (the part transforms already applied),
/// its uv rect in 0..1 over the 32x32 texture, and its normal in model space.
struct ArrowQuad {
    corners: [Vec3; 4],
    uv: [[f32; 2]; 4],
    normal: Vec3,
}

/// `ArrowRenderer.submit`'s pose, before the model: the camera-relative translation, the yaw
/// minus 90 about Y, the pitch about Z, then the shake the model's `setupAnim` adds to the
/// root's z rotation (`ArrowModel.setupAnim`: `-sin(shake * 3) * shake` degrees, radians).
pub(crate) fn arrow_matrix(origin: Vec3, y_rot: f32, x_rot: f32, shake: f32) -> Mat4 {
    let mut m = Mat4::from_translation(origin)
        * Mat4::from_rotation_y((y_rot - 90.0).to_radians())
        * Mat4::from_rotation_z(x_rot.to_radians());
    if shake > 0.0 {
        let pow = -(shake * 3.0).sin() * shake;
        m *= Mat4::from_rotation_z(pow * std::f32::consts::PI / 180.0);
    }
    m
}

/// `AbstractArrow.shouldRenderAtSqrDistance`: the bounding box diagonal times 10, times 64
/// and the view scale, squared — much farther than the generic entity limit.
pub(crate) fn arrow_should_render(dist_sq: f64, width: f32, height: f32, view_scale: f32) -> bool {
    let size = f64::from((width * width + height * height + width * width).sqrt()) * 10.0;
    let limit = size * 64.0 * f64::from(view_scale);
    dist_sq < limit * limit
}

/// `ArrowModel.createBodyLayer`, baked to world-space quads: the layer scales every
/// `PartPose` by 0.9 (`mesh.transformed(pose -> pose.scaled(0.9F))`), which moves the back
/// part's pivot from -11 to -9.9 and scales both parts' geometry (`withScale(0.8)` on the
/// back becomes 0.72, the crosses' 1.0 becomes 0.9); each part then translates by its pivot,
/// rotates about X and scales (`ModelPart.translateAndRotate`), and its cube corners count
/// in sixteenths of a block.
fn arrow_quads() -> &'static [ArrowQuad; 6] {
    use std::sync::OnceLock;
    static QUADS: OnceLock<[ArrowQuad; 6]> = OnceLock::new();
    QUADS.get_or_init(|| {
        // One part's transform: `translateAndRotate` (pivot, rotation ZYX, scale) with the
        // cube corners in sixteenths folded in.
        let part = |pivot: Vec3, x_rot: f32, scale: f32| {
            Mat4::from_translation(pivot / 16.0)
                * Mat4::from_rotation_x(x_rot)
                * Mat4::from_scale(Vec3::splat(scale / 16.0))
        };
        // `face` takes uv coordinates already normalised to 0..1 over the 32x32 texture (the
        // cross boxes' v axis is divided by 25.6 — their texScale is (1.0, 0.8) — so the
        // normalisation happens per face, not here).
        let face = |m: Mat4, corners: [Vec3; 4], uv: [[f32; 2]; 4], normal: Vec3| ArrowQuad {
            corners: corners.map(|c| m.transform_point3(c)),
            uv,
            // The normal rides the same part rotation as the corners (the uniform scale does
            // not turn it).
            normal: m.transform_vector3(normal).normalize_or_zero(),
        };
        // The back part: cube (0, -2.5, -2.5) size (0, 5, 5) at texOffs (0, 0) with `withScale
        // (0.8)`, rotated pi/4 about X, pivot (-11, 0, 0) scaled by the layer's 0.9. Only the
        // west and east faces are non-degenerate; the cube's uvs (texOffs 0,0, size 0x5x5)
        // put them at (0,5)-(5,10) and (5,5)-(10,10).
        let back = part(Vec3::new(-9.9, 0.0, 0.0), std::f32::consts::FRAC_PI_4, 0.72);
        // The cross boxes: cube (-12, -2, 0) size (16, 4, 0) at texOffs (0, 0) with texScale
        // (1.0, 0.8) (the v axis is divided by 25.6, not 32), rotated pi/4 and 3pi/4 about
        // X, no pivot. Only the north and south faces are non-degenerate.
        let cross = |x_rot: f32| part(Vec3::ZERO, x_rot, 0.9);
        [
            // back, west: corners (t0, l0, l3, t3), uv (5,5) (0,5) (0,10) (5,10) / 32.
            face(
                back,
                [
                    Vec3::new(0.0, -2.5, -2.5),
                    Vec3::new(0.0, -2.5, 2.5),
                    Vec3::new(0.0, 2.5, 2.5),
                    Vec3::new(0.0, 2.5, -2.5),
                ],
                [[5.0, 5.0], [0.0, 5.0], [0.0, 10.0], [5.0, 10.0]]
                    .map(|t| [t[0] / 32.0, t[1] / 32.0]),
                Vec3::X * -1.0,
            ),
            // back, east: corners (l1, t1, t2, l2), uv (10,5) (5,5) (5,10) (10,10) / 32.
            face(
                back,
                [
                    Vec3::new(0.0, -2.5, 2.5),
                    Vec3::new(0.0, -2.5, -2.5),
                    Vec3::new(0.0, 2.5, -2.5),
                    Vec3::new(0.0, 2.5, 2.5),
                ],
                [[10.0, 5.0], [5.0, 5.0], [5.0, 10.0], [10.0, 10.0]]
                    .map(|t| [t[0] / 32.0, t[1] / 32.0]),
                Vec3::X,
            ),
            // cross_1, north: corners (t1, t0, t3, t2), uv (16,0) (0,0) (0,4) (16,4), v / 25.6.
            face(
                cross(std::f32::consts::FRAC_PI_4),
                [
                    Vec3::new(4.0, -2.0, 0.0),
                    Vec3::new(-12.0, -2.0, 0.0),
                    Vec3::new(-12.0, 2.0, 0.0),
                    Vec3::new(4.0, 2.0, 0.0),
                ],
                [[16.0, 0.0], [0.0, 0.0], [0.0, 4.0], [16.0, 4.0]]
                    .map(|t| [t[0] / 32.0, t[1] / 25.6]),
                Vec3::Z * -1.0,
            ),
            // cross_1, south: corners (l0, l1, l2, l3), uv (32,0) (16,0) (16,4) (32,4), v / 25.6.
            face(
                cross(std::f32::consts::FRAC_PI_4),
                [
                    Vec3::new(-12.0, -2.0, 0.0),
                    Vec3::new(4.0, -2.0, 0.0),
                    Vec3::new(4.0, 2.0, 0.0),
                    Vec3::new(-12.0, 2.0, 0.0),
                ],
                [[32.0, 0.0], [16.0, 0.0], [16.0, 4.0], [32.0, 4.0]]
                    .map(|t| [t[0] / 32.0, t[1] / 25.6]),
                Vec3::Z,
            ),
            // cross_2: the same cube rotated 3pi/4.
            face(
                cross(3.0 * std::f32::consts::FRAC_PI_4),
                [
                    Vec3::new(4.0, -2.0, 0.0),
                    Vec3::new(-12.0, -2.0, 0.0),
                    Vec3::new(-12.0, 2.0, 0.0),
                    Vec3::new(4.0, 2.0, 0.0),
                ],
                [[16.0, 0.0], [0.0, 0.0], [0.0, 4.0], [16.0, 4.0]]
                    .map(|t| [t[0] / 32.0, t[1] / 25.6]),
                Vec3::Z * -1.0,
            ),
            face(
                cross(3.0 * std::f32::consts::FRAC_PI_4),
                [
                    Vec3::new(-12.0, -2.0, 0.0),
                    Vec3::new(4.0, -2.0, 0.0),
                    Vec3::new(4.0, 2.0, 0.0),
                    Vec3::new(-12.0, 2.0, 0.0),
                ],
                [[32.0, 0.0], [16.0, 0.0], [16.0, 4.0], [32.0, 4.0]]
                    .map(|t| [t[0] / 32.0, t[1] / 25.6]),
                Vec3::Z,
            ),
        ]
    })
}

impl EntityRenderer {
    /// One arrow, following vanilla `ArrowRenderer.submit`: the flight rotation and the
    /// shake pose `ArrowModel`, drawn with the texture `TippableArrowRenderer` /
    /// `SpectralArrowRenderer` pick, through the same batched entity atlas as every other
    /// entity (no per-entity draw call or allocation).
    pub(super) fn emit_arrow(
        &mut self,
        e: &EntityInstance,
        a: &ArrowDecoration,
        camera: glam::DVec3,
        light: (u8, u8),
        assets: &mut Assets,
    ) {
        // `TippableArrowRenderer.getTextureLocation` / `SpectralArrowRenderer`.
        let texture = match (a.spectral, a.tipped) {
            (true, _) => Identifier::minecraft("entity/projectiles/arrow_spectral"),
            (false, true) => Identifier::minecraft("entity/projectiles/arrow_tipped"),
            (false, false) => Identifier::minecraft("entity/projectiles/arrow"),
        };
        let rect = self.texture_rect(&texture, assets);
        let m = arrow_matrix((e.pos - camera).as_vec3(), a.y_rot, a.x_rot, a.shake);
        let nm = Mat3::from_mat4(m).inverse().transpose();
        let mut out = std::mem::take(&mut self.vertices);
        for q in arrow_quads() {
            push_world_quad(
                &mut out, m, nm, q.corners, q.uv, q.normal, [1.0; 3], light, rect,
            );
        }
        self.vertices = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The arrow textures this renderer loads must exist in the jar, at the 32x32 the
    /// `ArrowModel` layer's uv layout assumes (a typo would only show as placeholder quads
    /// at runtime).
    #[test]
    #[ignore = "needs the client jar in the cache"]
    fn arrow_textures_exist_in_the_jar() {
        let jar = haste_core::paths::client_jar();
        if !jar.exists() {
            eprintln!("ignored: needs the client jar in the cache");
            return;
        }
        let mut assets = haste_assets::Assets::open(
            &jar,
            &haste_core::paths::cache_dir().join("assets"),
            "26.3",
        )
        .unwrap();
        for texture in [
            "entity/projectiles/arrow",
            "entity/projectiles/arrow_tipped",
            "entity/projectiles/arrow_spectral",
        ] {
            let image = assets
                .texture_any(&Identifier::minecraft(texture))
                .unwrap_or_else(|e| panic!("arrow texture {texture}: {e:#}"));
            assert_eq!(
                image.dimensions(),
                (32, 32),
                "{texture} must match the ArrowModel layer's 32x32"
            );
        }
    }

    #[test]
    fn quads_stay_inside_the_arrow_texture() {
        for q in arrow_quads() {
            for t in q.uv {
                assert!((0.0..=1.0).contains(&t[0]), "u {} out of range", t[0]);
                assert!((0.0..=1.0).contains(&t[1]), "v {} out of range", t[1]);
            }
        }
    }

    #[test]
    fn back_part_moves_pivot_and_scales() {
        // The back quad's x extent: the pivot -9.9/16 = -0.61875 blocks plus a zero-width box.
        let back = &arrow_quads()[0];
        // Rotated pi/4 about X, the 5x5 plate's corners (0, ±2.5, ±2.5) land on the y and z
        // axes at 2.5 * sqrt(2) * 0.72 / 16 blocks out.
        let r = 2.5 * std::f32::consts::SQRT_2 * 0.72 / 16.0;
        for c in back.corners {
            assert!((c.x - (-9.9 / 16.0)).abs() < 1.0e-6, "back x {c:?}");
            let on_y = (c.y.abs() - r).abs() < 1.0e-6 && c.z.abs() < 1.0e-6;
            let on_z = (c.z.abs() - r).abs() < 1.0e-6 && c.y.abs() < 1.0e-6;
            assert!(on_y || on_z, "back corner off the axes: {c:?}");
        }
    }

    #[test]
    fn cross_quads_reach_the_arrow_head() {
        // The cross box runs x -12..4 model units, scaled 0.9/16: the head reaches
        // 4 * 0.9 / 16 = 0.225 blocks, the flights -0.675.
        let north = &arrow_quads()[2];
        let xs = north.corners.map(|c| c.x);
        assert!((xs[0] - 4.0 * 0.9 / 16.0).abs() < 1.0e-6);
        assert!((xs[1] - -12.0 * 0.9 / 16.0).abs() < 1.0e-6);
        // Rotated pi/4 about X, the box's y half (2 * 0.9/16 = 0.1125) splits evenly between
        // the y and z axes (times cos(45°) each).
        for c in north.corners {
            assert!((c.y.abs() - 0.1125 * std::f32::consts::FRAC_PI_4.cos()).abs() < 1.0e-6);
            assert!((c.z.abs() - 0.1125 * std::f32::consts::FRAC_PI_4.cos()).abs() < 1.0e-6);
        }
    }

    #[test]
    fn shake_rotates_about_the_arrow_origin() {
        // No shake: identity besides the pose (yaw 90 keeps the model's x axis on x, so the
        // tip stays put); with shake, the tip turns about the origin in the xy plane.
        let origin = Vec3::new(1.0, 2.0, 3.0);
        let still = arrow_matrix(origin, 90.0, 0.0, 0.0);
        assert_eq!(
            still.transform_point3(Vec3::new(0.2, 0.0, 0.0)),
            origin + Vec3::new(0.2, 0.0, 0.0)
        );
        let shaking = arrow_matrix(origin, 90.0, 0.0, 5.0);
        let tip = shaking.transform_point3(Vec3::new(0.2, 0.0, 0.0));
        assert!((tip - origin).length() > 0.199);
        assert!((tip.y - origin.y).abs() > 1.0e-4, "shake must tilt the tip");
    }

    #[test]
    fn render_distance_follows_the_arrow_override() {
        // `AbstractArrow.shouldRenderAtSqrDistance`: bbox diagonal 0.5,0.5,0.5 -> ~0.866,
        // * 10 * 64 -> the limit at ~554 blocks, far past the generic 64.
        assert!(arrow_should_render(300.0f64 * 300.0, 0.5, 0.5, 1.0));
        assert!(!arrow_should_render(600.0f64 * 600.0, 0.5, 0.5, 1.0));
    }
}
