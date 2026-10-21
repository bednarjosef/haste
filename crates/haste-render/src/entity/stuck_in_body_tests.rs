use glam::{Mat4, Vec3, Vec4Swizzles};
use haste_core::model::{Cube, CubeModel, ModelPart, PoseOverrides};
use haste_core::parts::intern;
use haste_core::Identifier;

use super::stuck_in_body::*;

fn part(name: &str, cubes: Vec<Cube>) -> ModelPart {
    ModelPart {
        name: name.to_string(),
        id: intern(name),
        pivot: Vec3::ZERO,
        rotation: Vec3::ZERO,
        visible: true,
        cubes,
        children: Vec::new(),
    }
}

/// A humanoid with every `PLAYER_BODY_PARTS` part holding one 8x8x8 cube centred on its pivot,
/// so each cube spans -4..4 on every axis (model units).
fn humanoid() -> CubeModel {
    let cube = Cube {
        origin: Vec3::splat(-4.0),
        size: Vec3::splat(8.0),
        inflate: 0.0,
        uv: [0, 0],
        mirror: false,
    };
    let children = PLAYER_BODY_PARTS
        .iter()
        .map(|n| part(n, vec![cube.clone()]))
        .collect();
    CubeModel {
        texture: Identifier::minecraft("entity/player/wide/steve"),
        tex_width: 64,
        tex_height: 64,
        root: ModelPart {
            children,
            ..part("root", Vec::new())
        },
    }
}

fn collect(count: u32, seed: i64, placement: Placement) -> Vec<Mat4> {
    let mut out = Vec::new();
    for_each_stuck_item(
        Mat4::IDENTITY,
        &humanoid(),
        &PoseOverrides::default(),
        &PLAYER_BODY_PARTS,
        count,
        seed,
        placement,
        |m| out.push(m),
    );
    out
}

#[test]
fn snap_to_face_uses_the_middle_as_the_threshold() {
    assert_eq!(snap_to_face(0.0), 0.5);
    assert_eq!(snap_to_face(0.5), 0.5);
    assert_eq!(snap_to_face(0.51), 1.0);
    assert_eq!(snap_to_face(0.99), 1.0);
}

#[test]
fn random_matches_java_random_for_seed_42() {
    // `new Random(42)`: nextInt() is -1170105035, so next(31) % 10 is 0; nextFloat is 0.7275637.
    assert_eq!(StuckRandom::new(42).next_int(10), 0);
    assert!((StuckRandom::new(42).next_float() - 0.727_563_7).abs() < 1e-6);
}

#[test]
fn next_int_stays_in_bounds_for_powers_of_two_and_others() {
    let mut rng = StuckRandom::new(7);
    for _ in 0..1000 {
        assert!((0..8).contains(&rng.next_int(8)));
        assert!((0..6).contains(&rng.next_int(6)));
        assert!((0..3).contains(&rng.next_int(3)));
    }
}

#[test]
fn next_float_is_in_unit_range() {
    let mut rng = StuckRandom::new(-99);
    for _ in 0..1000 {
        let f = rng.next_float();
        assert!((0.0..1.0).contains(&f));
    }
}

#[test]
fn zero_count_draws_nothing() {
    assert!(collect(0, 1, Placement::InCube).is_empty());
}

#[test]
fn each_stuck_item_is_emitted_and_the_seed_fixes_the_layout() {
    let a = collect(5, 1234, Placement::InCube);
    let b = collect(5, 1234, Placement::InCube);
    assert_eq!(a.len(), 5);
    assert_eq!(a, b);
    assert_ne!(a, collect(5, 1235, Placement::InCube));
}

#[test]
fn in_cube_items_sit_inside_their_cube() {
    for m in collect(40, 99, Placement::InCube) {
        let p = m.w_axis.xyz();
        for v in [p.x, p.y, p.z] {
            assert!((-4.0..=4.0).contains(&v), "{v} outside the cube");
        }
    }
}

#[test]
fn surface_items_sit_on_a_face_of_their_cube() {
    for m in collect(40, 99, Placement::OnSurface) {
        let p = m.w_axis.xyz();
        // The snapped axis is the cube's middle (0) or its far side (4).
        let on_face = [p.x, p.y, p.z]
            .iter()
            .any(|v| v.abs() < 1e-4 || (v - 4.0).abs() < 1e-4);
        assert!(on_face, "{p} is not on a face");
    }
}
