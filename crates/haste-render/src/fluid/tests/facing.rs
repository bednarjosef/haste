//! The facing groups of fluid quads (`mesher::axis_facing`): a face is drawn in the group
//! it faces, which for `FluidRenderer.tesselate`'s reversed copies (the underside of the
//! surface, the inner sides) is the opposite of the direction they are shaded by.

use super::*;
use crate::mesher::axis_facing;

#[test]
fn fluid_faces_are_grouped_by_the_side_they_face() {
    // A lone source in the open: a flat top with its underside (`shouldRenderBackwardUpFace`),
    // four sides with their inner copies, and the bottom.
    let t = table();
    let f = t.fluid(water(0)).unwrap();
    let mut g = Grid::default();
    g.set(0, 0, 0, water(0));
    let mut q = Vec::new();
    fluid_quads(&g, &t, at(0, 0, 0), f, &mut q);
    for d in Direction::ALL {
        let faces: Vec<_> = q
            .iter()
            .filter(|q| q.dir == d)
            .map(|q| axis_facing(&q.positions))
            .collect();
        assert!(!faces.is_empty(), "{d:?}");
        // The first quad of each direction is the front face; any copy faces the other way.
        assert_eq!(faces[0], Some(d), "{d:?}");
        for f in &faces[1..] {
            assert_eq!(*f, Some(d.opposite()), "{d:?}");
        }
    }
    let ups = q.iter().filter(|q| q.dir == Direction::Up).count();
    assert_eq!(ups, 2, "the underside is drawn under open sky");
}

#[test]
fn a_sloped_surface_is_never_skipped() {
    // A source next to a lower flowing block: the top slopes, so it lies in no axis plane.
    let t = table();
    let f = t.fluid(water(0)).unwrap();
    let mut g = Grid::default();
    g.set(0, 0, 0, water(0));
    g.set(1, 0, 0, water(3));
    g.set(0, -1, 0, STONE);
    g.set(1, -1, 0, STONE);
    let mut q = Vec::new();
    fluid_quads(&g, &t, at(0, 0, 0), f, &mut q);
    let tops: Vec<_> = q.iter().filter(|q| q.dir == Direction::Up).collect();
    assert!(!tops.is_empty());
    for top in tops {
        assert_eq!(axis_facing(&top.positions), None, "{:?}", top.positions);
    }
}
