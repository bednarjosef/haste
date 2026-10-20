//! `axis_facing` and `facing_group`: the facing group a quad is drawn in, from its geometry.

use haste_assets::Direction;

use crate::mesher::{axis_facing, facing_group};

#[test]
fn quads_in_an_axis_plane_face_their_winding_normal() {
    // The same top face wound both ways faces up and down.
    let up = [
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 1.0],
        [1.0, 1.0, 1.0],
        [1.0, 1.0, 0.0],
    ];
    let face = axis_facing(&up).unwrap();
    assert!(face == Direction::Up || face == Direction::Down);
    let flipped = [up[3], up[2], up[1], up[0]];
    assert_eq!(axis_facing(&flipped), Some(face.opposite()));
    // Two coincident corners (a triangle drawn as a quad) keep the sign.
    let tri = [up[0], up[1], up[2], up[2]];
    assert_eq!(axis_facing(&tri), Some(face));
}

#[test]
fn crossed_planes_face_a_diagonal_and_slopes_are_never_skipped() {
    // A crossed plant plane (`cross` model, 45 degrees) lies in no axis plane, whatever
    // nearest direction its baked quad carries: it faces a horizontal diagonal, and its
    // back copy the opposite one.
    let cross = [
        [0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 1.0, 1.0],
        [1.0, 0.0, 1.0],
    ];
    assert_eq!(axis_facing(&cross), None);
    let g = facing_group(&cross);
    assert!((7..11).contains(&g), "group {g}");
    let back = [cross[3], cross[2], cross[1], cross[0]];
    let [sx, sz] = crate::mesher::DIAGONALS[g - 7];
    assert_eq!(
        crate::mesher::DIAGONALS[facing_group(&back) - 7],
        [-sx, -sz]
    );
    // The plane runs from (0, 0) to (1, 1): its normal is (1, -1) or (-1, 1).
    assert_eq!(sx, -sz);
    // A fluid surface sloping down towards a lower neighbour.
    let slope = [
        [0.0, 0.9, 0.0],
        [0.0, 0.9, 1.0],
        [1.0, 0.5, 1.0],
        [1.0, 0.5, 0.0],
    ];
    assert_eq!(axis_facing(&slope), None);
    assert_eq!(facing_group(&slope), 6);
}

#[test]
fn a_diagonal_group_is_back_facing_only_when_the_whole_section_is_behind_it() {
    let layer = |dir: u8| crate::section_slab::SlabLayer {
        layer: haste_assets::RenderLayer::Cutout,
        first: 0,
        count: 6,
        dir,
        kind: crate::quad_format::SpanKind::Indexed,
    };
    // Group 10 faces (+x, +z). A section far off in +x, +z (camera-relative box) has
    // every plane facing away; one in -x, -z faces the camera; one straddling the
    // camera's diagonal line is mixed and kept.
    let far = ([100.0, 0.0, 100.0], [116.0, 16.0, 116.0]);
    let near = ([-116.0, 0.0, -116.0], [-100.0, 16.0, -100.0]);
    let mixed = ([-8.0, 0.0, 0.0], [8.0, 16.0, 16.0]);
    assert!(layer(10).back_facing(far.0, far.1));
    assert!(!layer(10).back_facing(near.0, near.1));
    assert!(!layer(10).back_facing(mixed.0, mixed.1));
    assert!(layer(7).back_facing(near.0, near.1));
    // Group 6 never is.
    assert!(!layer(6).back_facing(far.0, far.1));
}

#[test]
fn rounding_of_a_rotated_model_stays_in_the_plane() {
    let face = [
        [0.0, 1.0, 0.0],
        [0.0, 1.0 + 1e-6, 1.0],
        [1.0, 1.0, 1.0],
        [1.0, 1.0 - 1e-6, 0.0],
    ];
    assert!(axis_facing(&face).is_some());
}

#[test]
fn groups_seen_from_outside_a_section_form_few_runs() {
    use crate::mesher::{DIAGONALS, GROUP_ORDER};
    // From outside a section on every axis the camera sees one side per axis, the
    // never-skipped group, the diagonal pointing at it and one of the two mixed
    // diagonals (whichever horizontal axis is longer); the draw list joins adjacent
    // groups, at most three draws from any corner.
    let mut totals = [0; 2];
    for corner in 0..8 {
        for x_longer in [false, true] {
            let (cx, cz) = (
                if corner & 4 != 0 { 1.0 } else { -1.0 },
                if corner & 2 != 0 { 1.0 } else { -1.0 },
            );
            let visible = |d: usize| match d {
                6 => true,
                0 | 1 => (corner & 1 != 0) == (d == 1),
                2 | 3 => (corner & 2 != 0) == (d == 3),
                4 | 5 => (corner & 4 != 0) == (d == 5),
                _ => {
                    let [sx, sz] = DIAGONALS[d - 7];
                    let (wx, wz) = if x_longer { (1.5, 1.0) } else { (1.0, 1.5) };
                    sx * cx * wx + sz * cz * wz > 0.0
                }
            };
            // With diagonal faces (plants), and without (empty groups join).
            for with_diagonals in [true, false] {
                let mut runs = 0;
                let mut prev = false;
                for d in GROUP_ORDER.into_iter().filter(|&d| with_diagonals || d < 7) {
                    let v = visible(d);
                    if v && !prev {
                        runs += 1;
                    }
                    prev = v;
                }
                assert!(runs <= 3, "corner {corner}: {runs} runs");
                totals[usize::from(with_diagonals)] += runs;
            }
        }
    }
    // Sixteen cases each: two runs on average without diagonals, 2.1 with.
    assert_eq!(totals, [32, 34]);
}
