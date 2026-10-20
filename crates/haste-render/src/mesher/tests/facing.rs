//! `axis_facing`: the facing group a quad is drawn in, from its geometry.

use haste_assets::Direction;

use crate::mesher::axis_facing;

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
fn crossed_and_sloped_quads_are_never_skipped() {
    // A crossed plant plane (`cross` model, 45 degrees) lies in no axis plane, whatever
    // nearest direction its baked quad carries.
    let cross = [
        [0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 1.0, 1.0],
        [1.0, 0.0, 1.0],
    ];
    assert_eq!(axis_facing(&cross), None);
    // A fluid surface sloping down towards a lower neighbour.
    let slope = [
        [0.0, 0.9, 0.0],
        [0.0, 0.9, 1.0],
        [1.0, 0.5, 1.0],
        [1.0, 0.5, 0.0],
    ];
    assert_eq!(axis_facing(&slope), None);
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
    use crate::mesher::GROUP_ORDER;
    // From outside a section on every axis the camera sees one side per axis plus the
    // never-skipped group; the draw list joins adjacent groups. Four separate draws
    // become one from two corners and at most three from any.
    let mut total = 0;
    for corner in 0..8 {
        let visible = |d: usize| match d {
            6 => true,
            0 | 1 => (corner & 1 != 0) == (d == 1),
            2 | 3 => (corner & 2 != 0) == (d == 3),
            _ => (corner & 4 != 0) == (d == 5),
        };
        let mut runs = 0;
        let mut prev = false;
        for d in GROUP_ORDER {
            let v = visible(d);
            if v && !prev {
                runs += 1;
            }
            prev = v;
        }
        assert!(runs <= 3, "corner {corner}: {runs} runs");
        total += runs;
        if corner == 7 || corner == 0 {
            assert_eq!(runs, 1, "corner {corner}");
        }
    }
    assert_eq!(total, 16, "two runs on average");
}
