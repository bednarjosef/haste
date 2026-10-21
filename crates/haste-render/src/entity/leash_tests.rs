//! The leash rope strip (`LeashFeatureRenderer.prepare`/`addVertexPair`) with vanilla's numbers:
//! 25 forward and 25 backward pairs, a 0.05 width, the 0.7 stripe, light lerped per step.

use glam::DVec3;

use super::leash::{push_leash, LeashLine};
use super::vertex::EntityVertex;

const WHITE: [u32; 4] = [0, 0, 2, 2];
const EPS: f32 = 1e-4;

fn rope(start: DVec3, end: DVec3, start_light: (u8, u8), end_light: (u8, u8)) -> LeashLine {
    LeashLine {
        start,
        end,
        start_light,
        end_light,
    }
}

fn strip(line: &LeashLine, camera: DVec3) -> Vec<EntityVertex> {
    let mut out = Vec::new();
    push_leash(&mut out, line, camera, WHITE);
    out
}

#[test]
fn strip_is_fifty_pairs_of_two_triangles() {
    // 50 pairs make 49 quads between neighbours, two triangles each.
    let verts = strip(
        &rope(DVec3::ZERO, DVec3::new(0.0, 4.0, 0.0), (0, 0), (0, 0)),
        DVec3::ZERO,
    );
    assert_eq!(verts.len(), 49 * 2 * 3);
    assert!(verts.iter().all(|v| v.pos.iter().all(|c| c.is_finite())));
}

#[test]
fn vertical_rope_stays_within_the_span_and_the_width() {
    let verts = strip(
        &rope(DVec3::ZERO, DVec3::new(0.0, 4.0, 0.0), (0, 0), (0, 0)),
        DVec3::ZERO,
    );
    for v in &verts {
        let [x, y, z] = v.pos;
        assert!(
            x.abs() < EPS && z.abs() < EPS,
            "vertical rope drifted: {:?}",
            v.pos
        );
        assert!((-EPS..=4.0 + 0.05 + EPS).contains(&y), "y out of span: {y}");
    }
}

#[test]
fn horizontal_rope_is_wide_across_its_direction_only() {
    // Along +x the rope's pair spreads in z by half the 0.05 width each way.
    let verts = strip(
        &rope(DVec3::ZERO, DVec3::new(4.0, 0.0, 0.0), (0, 0), (0, 0)),
        DVec3::ZERO,
    );
    let max_z = verts.iter().map(|v| v.pos[2].abs()).fold(0.0, f32::max);
    assert!((max_z - 0.025).abs() < EPS, "half width {max_z}");
    assert!(verts.iter().all(|v| v.pos[1].abs() < 0.05 + EPS));
}

#[test]
fn positions_are_camera_relative() {
    let start = DVec3::new(100.0, 64.0, -30.0);
    let verts = strip(
        &rope(start, start + DVec3::new(0.0, 2.0, 0.0), (0, 0), (0, 0)),
        start,
    );
    // The first pair sits at the attached end, which is now the camera's origin.
    assert!(verts[0].pos.iter().all(|c| c.abs() < 0.06));
}

#[test]
fn light_ramps_from_the_start_end_to_the_holder_end() {
    let verts = strip(
        &rope(DVec3::ZERO, DVec3::new(4.0, 0.0, 0.0), (15, 15), (3, 0)),
        DVec3::ZERO,
    );
    let block: Vec<u16> = verts.iter().map(|v| v.light[0]).collect();
    let sky: Vec<u16> = verts.iter().map(|v| v.light[1]).collect();
    assert_eq!(*block.iter().max().unwrap(), 15);
    assert_eq!(*block.iter().min().unwrap(), 3);
    assert_eq!(*sky.iter().max().unwrap(), 15);
    assert_eq!(*sky.iter().min().unwrap(), 0);
}

#[test]
fn stripes_alternate_between_full_and_seventy_percent_colour() {
    let verts = strip(
        &rope(DVec3::ZERO, DVec3::new(0.0, 4.0, 0.0), (0, 0), (0, 0)),
        DVec3::ZERO,
    );
    let has = |r: f32, g: f32, b: f32| {
        verts.iter().any(|v| {
            (v.color[0] - r).abs() < EPS
                && (v.color[1] - g).abs() < EPS
                && (v.color[2] - b).abs() < EPS
        })
    };
    // `0.5, 0.4, 0.3` on the lit steps and the same times 0.7 on the dark ones.
    assert!(has(0.5, 0.4, 0.3));
    assert!(has(0.35, 0.28, 0.21));
    assert!(verts.iter().all(|v| (v.color[3] - 1.0).abs() < EPS));
}

#[test]
fn colour_is_unshaded_and_uses_the_white_texel() {
    let verts = strip(
        &rope(DVec3::ZERO, DVec3::new(3.0, 1.0, 2.0), (0, 0), (0, 0)),
        DVec3::ZERO,
    );
    for v in &verts {
        assert_eq!(v.rect, [0, 0, 2, 2]);
        assert!(v.uv[0] >= 0.0 && v.uv[0] <= 2.0 && v.uv[1] >= 0.0 && v.uv[1] <= 2.0);
    }
}

#[test]
fn vertical_rope_is_finite_though_it_has_no_horizontal_width() {
    // `Mth.invSqrt(0)` is infinite in vanilla; here the factor stays zero.
    let verts = strip(
        &rope(DVec3::ZERO, DVec3::new(0.0, 1.0, 0.0), (0, 0), (0, 0)),
        DVec3::ZERO,
    );
    assert!(verts.iter().all(|v| v.pos.iter().all(|c| c.is_finite())));
}
