// Expected values are full-precision prints of the f32 results.
#![allow(clippy::excessive_precision)]

use super::*;
use glam::Vec3;

#[test]
fn star_brightness_follows_the_day_timeline() {
    let close = |t: i64, want: f32| {
        let got = star_brightness(t);
        assert!((got - want).abs() < 1e-5, "tick {t}: {got} vs {want}");
    };
    close(6000, 0.0);
    close(12399, 0.143);
    close(12729, 0.258);
    close(18000, 0.5);
    close(92, 0.037);
    // Halfway between 12399 and 12729.
    close(12564, (0.143 + 0.258) / 2.0);
    // The wrap segment 23758 -> 24092: 0.101 down to 0.037.
    close(23925, 0.101 + (0.037 - 0.101) * (167.0 / 334.0));
    close(24000 + 12399, 0.143);
}

/// The first stars of vanilla's field (seed 10842, Java's LCG with float rounding),
/// computed independently from `SkyRenderer.buildStars`' steps: 780 of the 1500 attempts
/// land in the shell.
#[test]
fn star_field_matches_vanilla_seed() {
    let quads = star_quads();
    assert_eq!(quads.len(), 780);
    let expect: [([f32; 3], f32); 3] = [
        ([-53.246_872, 69.925_743, 47.698_662], 0.150_184_26),
        ([-96.890_892, -18.466_675, 16.466_272], 0.248_971_67),
        ([-98.799_294, 9.651_537, 12.064_331], 0.197_489_31),
    ];
    for (q, (centre, size)) in quads.iter().zip(expect) {
        let corners = q.map(Vec3::from);
        let mid = (corners[0] + corners[1] + corners[2] + corners[3]) / 4.0;
        let c = Vec3::from(centre);
        assert!((mid - c).length() < 1e-3, "{mid:?} vs {c:?}");
        // A square of half-size `size`, facing the origin.
        for k in corners {
            assert!(((k - c).length() - size * 2f32.sqrt()).abs() < 1e-4);
            assert!((k - c).dot(c).abs() < 1e-2);
        }
    }
}

/// `rotateTowards(-centre, +Y)` maps local +Z onto the inward direction and keeps local +Y
/// in the plane of +Y and the direction.
#[test]
fn towards_maps_z_onto_the_direction() {
    let dir = Vec3::new(0.3, -0.5, 0.8).normalize();
    let m = towards(dir, Vec3::Y);
    assert!((m * Vec3::Z - dir).length() < 1e-6);
    let up = m * Vec3::Y;
    assert!(up.dot(dir).abs() < 1e-6);
    assert!(up.dot(Vec3::Y) > 0.0);
    // Left = up x dir.
    assert!((m * Vec3::X - Vec3::Y.cross(dir).normalize()).length() < 1e-6);
}
