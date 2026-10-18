//! Tests for `FluidOccluders` built from the oracle dump's `occlusion` field: the fence
//! override (an occlusion shape smaller than its outline) and slab neighbours, plus the
//! outline fallback for states the field is absent for.

use haste_assets::Direction;
use haste_core::{Aabb, BlockStateId, DVec3};

use crate::fluid::FluidOccluders;
use crate::fluid_occlusion::{is_face_occluded, Occluder};

fn aabb(min: [f64; 3], max: [f64; 3]) -> Aabb {
    Aabb {
        min: DVec3::new(min[0], min[1], min[2]),
        max: DVec3::new(max[0], max[1], max[2]),
    }
}

/// A fence's dump data: the outline is post plus a north-south arm (reaching the north and
/// south planes), the occlusion shape the thin post only (vanilla
/// `FenceBlock.getOcclusionShape`, `makeShapes(4, 16, 2, 6, 15)`), which reaches no side
/// plane. Built from the occlusion field, the table carries the post alone — never the arm.
#[test]
fn fence_occlusion_is_the_post_not_the_outline() {
    let outline = [
        aabb([0.375, 0.0, 0.375], [0.625, 1.5, 0.625]),
        aabb([0.4375, 0.0, 0.0], [0.5625, 0.75, 1.0]),
    ];
    let occlusion = [aabb([0.375, 0.0, 0.375], [0.625, 0.9375, 0.625])];
    // State 1: the fence with occlusion data. State 2: the same outline, no occlusion
    // field (a dump from before the regeneration).
    let table = FluidOccluders::from_sources(
        3,
        |s| (s == BlockStateId(1)).then_some(occlusion.as_slice()),
        |s| {
            if s == BlockStateId(2) {
                outline.as_slice()
            } else {
                &[]
            }
        },
        |_| false,
    );
    match table.occluder(BlockStateId(1), || false) {
        Occluder::Boxes(boxes) => {
            assert_eq!(boxes, &[[0.375, 0.0, 0.375, 0.625, 0.9375, 0.625]])
        }
        o => panic!("the fence occludes with its post, got {o:?}"),
    }
    // The outline fallback keeps the arm box (two boxes) for the same fence.
    match table.occluder(BlockStateId(2), || false) {
        Occluder::Boxes(boxes) => assert_eq!(boxes.len(), 2),
        o => panic!("the old-dump fence falls back to its outline, got {o:?}"),
    }
    // Vanilla `FluidRenderer.isFaceOccludedByNeighbor` against the fence: the post
    // reaches no side plane, so no water face on any side is covered.
    let fence_occluder = table.occluder(BlockStateId(1), || false);
    for dir in [
        Direction::North,
        Direction::South,
        Direction::West,
        Direction::East,
    ] {
        assert!(
            !is_face_occluded(dir, 1.0, fence_occluder),
            "a fence never covers a side face"
        );
    }
}

/// A slab neighbour from the dump's occlusion field (a half box): a water block lower than
/// the slab loses its side face to it, a source keeps the part above the slab, and a
/// full-cube occlusion shape is the full occluder without consulting the models.
#[test]
fn slab_occlusion_from_the_dump() {
    let slab = [aabb([0.0, 0.0, 0.0], [1.0, 0.5, 1.0])];
    let full = [aabb([0.0, 0.0, 0.0], [1.0, 1.0, 1.0])];
    // 1 bottom slab, 2 stone (full-cube occlusion), 3 glass (empty occlusion).
    let table = FluidOccluders::from_sources(
        4,
        |s| match s {
            BlockStateId(1) => Some(slab.as_slice()),
            BlockStateId(2) => Some(full.as_slice()),
            BlockStateId(3) => Some(&[][..]),
            _ => None,
        },
        |_| &[],
        // Solid render is never consulted when the occlusion shape is present.
        |_| unreachable!("the occlusion shape decides"),
    );
    let slab_occluder = table.occluder(BlockStateId(1), || false);
    assert!(is_face_occluded(Direction::East, 2.0 / 9.0, slab_occluder));
    assert!(!is_face_occluded(Direction::East, 8.0 / 9.0, slab_occluder));
    // A full-cube occlusion shape is solid render by definition: full occluder.
    assert!(matches!(
        table.occluder(BlockStateId(2), || false),
        Occluder::Full
    ));
    // Glass: empty occlusion shape, never occludes.
    assert!(matches!(
        table.occluder(BlockStateId(3), || true),
        Occluder::Empty
    ));
    // State 0 has no occlusion entry and an empty outline: empty.
    assert!(matches!(
        table.occluder(BlockStateId(0), || false),
        Occluder::Empty
    ));
}
