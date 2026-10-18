use super::*;
use glam::{Vec3 as GVec3, Vec4};

/// The model-view of `panorama_matrix` (vanilla `CubeMap.render`: half turn about X,
/// pitch about X, spin about Y).
fn model_view(spin: f32) -> Mat4 {
    Mat4::from_rotation_x(std::f32::consts::PI)
        * Mat4::from_rotation_x(PITCH.to_radians())
        * Mat4::from_rotation_y(spin.to_radians())
}

/// The cube direction the screen centre looks along for a spin (the inverse model-view
/// applied to the view's forward axis).
fn centre_dir(spin: f32) -> GVec3 {
    model_view(spin).inverse().transform_vector3(GVec3::NEG_Z)
}

#[test]
fn centre_looks_at_the_plus_z_face_tilted_by_the_pitch() {
    let d = centre_dir(0.0);
    assert!(d.z > 0.98, "{d}");
    // 10 degrees off the horizon
    assert!((d.y.asin().to_degrees().abs() - PITCH).abs() < 1e-3, "{d}");
}

#[test]
fn spin_turns_about_the_vertical_axis() {
    let a = centre_dir(0.0);
    let b = centre_dir(90.0);
    assert!((a.y - b.y).abs() < 1e-5, "pitch unchanged: {a} {b}");
    let ha = GVec3::new(a.x, 0.0, a.z).normalize();
    let hb = GVec3::new(b.x, 0.0, b.z).normalize();
    assert!(ha.dot(hb).abs() < 1e-5, "a quarter turn: {ha} {hb}");
}

#[test]
fn every_cube_corner_is_in_front_of_the_far_plane() {
    // The corners sit on the horizon (distance sqrt(3) from the camera), far outside the
    // 85 degree frustum, so corners with w > 0 can be nearer than the near plane
    // (z/w < 0) or even behind the camera (w <= 0); vanilla clips those too, harmlessly
    // (see `the_near_plane_cannot_eat_the_cubes_surface`). What matters is only that
    // nothing the frustum looks at falls beyond the far plane 10.
    let m = panorama_matrix(37.0, 1920, 1080);
    for c in CUBE {
        let p = m * Vec4::new(c[0], c[1], c[2], 1.0);
        if p.w > 0.0 {
            assert!(p.z / p.w <= 1.0, "corner {c:?} at depth {}", p.z / p.w);
        }
    }
}

#[test]
fn the_near_plane_cannot_eat_the_cubes_surface() {
    // A surface point visible in the frustum sits at distance >= 1 from the camera (the
    // cube contains the unit ball), along a direction inside the frustum. The worst
    // such direction is a screen corner ray, whose view depth per unit distance is
    // 1/sqrt(1 + tan^2(fov/2) + tan^2(fov/2)*aspect^2); for any plausible window that
    // stays far above the near plane (0.737 at aspect 1, 0.5 even at aspect 4), so
    // near-plane clipping (the only depth clipping there is, the pass has no depth
    // test) removes no visible part, whatever the spin.
    let half = (PROJECTION_FOV / 2.0).to_radians();
    for aspect in [0.5f32, 1.0, 16.0 / 9.0, 2.0, 4.0] {
        let min_depth = (1.0 + half.tan() * half.tan() * (1.0 + aspect * aspect))
            .sqrt()
            .recip();
        assert!(
            min_depth > PROJECTION_Z_NEAR * 2.0,
            "aspect {aspect}: min visible depth {min_depth}"
        );
    }
}

#[test]
fn projection_uses_the_vertical_fov_and_aspect() {
    // A point on the top edge of the 85 degree frustum lands on the top of the screen.
    let m = Mat4::perspective_rh(
        PROJECTION_FOV.to_radians(),
        2.0,
        PROJECTION_Z_NEAR,
        PROJECTION_Z_FAR,
    );
    let half = (PROJECTION_FOV / 2.0).to_radians();
    let p = m * Vec4::new(0.0, half.tan(), -1.0, 1.0);
    assert!((p.y / p.w - 1.0).abs() < 1e-5);
}

#[test]
fn indices_cover_six_quads() {
    let idx = cube_indices();
    assert_eq!(&idx[..6], &[0, 1, 2, 2, 3, 0]);
    assert_eq!(&idx[30..], &[20, 21, 22, 22, 23, 20]);
}

#[test]
fn faces_stack_in_vanillas_layer_order_upside_down() {
    // Face i is 1x2: top row value 10*i, bottom row 10*i + 1.
    let faces: [image::RgbaImage; 6] = std::array::from_fn(|i| {
        image::RgbaImage::from_fn(1, 2, |_, y| {
            image::Rgba([(10 * i) as u8 + y as u8, 0, 0, 255])
        })
    });
    let (w, h, rgba) = stack_faces(&faces).unwrap();
    assert_eq!((w, h), (1, 2));
    let reds: Vec<u8> = rgba.chunks(4).map(|p| p[0]).collect();
    // layers take faces 1 3 5 4 0 2, each flipped (bottom row first)
    assert_eq!(reds, vec![11, 10, 31, 30, 51, 50, 41, 40, 1, 0, 21, 20]);
}

#[test]
fn faces_of_different_sizes_are_an_error() {
    let mut faces: [image::RgbaImage; 6] = std::array::from_fn(|_| image::RgbaImage::new(2, 2));
    faces[4] = image::RgbaImage::new(4, 4);
    assert!(stack_faces(&faces).is_err());
}
