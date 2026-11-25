//! Tests for the chest model layers (`ChestModel`).

use super::*;

/// Vanilla `ChestModel.createSingleBodyLayer`: bottom, lid and lock with the `addBox` origins,
/// sizes and texture offsets (object convention, y up), lid and lock on the (0, 9, 1) pivot.
#[test]
fn single_chest_model_matches_vanilla_layer() {
    let m = chest_model(ChestHalf::Single);
    assert_eq!((m.tex_width, m.tex_height), (64, 64));
    let names: Vec<&str> = m.root.children.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["bottom", "lid", "lock"]);
    let bottom = &m.root.children[0];
    assert_eq!(bottom.pivot, Vec3::ZERO);
    assert_eq!(bottom.cubes.len(), 1);
    assert_eq!(bottom.cubes[0].origin, Vec3::new(1.0, 0.0, 1.0));
    assert_eq!(bottom.cubes[0].size, Vec3::new(14.0, 10.0, 14.0));
    assert_eq!(bottom.cubes[0].uv, [0, 19]);
    let lid = &m.root.children[1];
    assert_eq!(lid.pivot, Vec3::new(0.0, 9.0, 1.0));
    assert_eq!(lid.cubes[0].origin, Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(lid.cubes[0].size, Vec3::new(14.0, 5.0, 14.0));
    assert_eq!(lid.cubes[0].uv, [0, 0]);
    // The latch: `addBox(7, -2, 14, 2, 4, 1)`, texOffs(0, 0), on the lid's pivot.
    let lock = &m.root.children[2];
    assert_eq!(lock.pivot, Vec3::new(0.0, 9.0, 1.0));
    assert_eq!(lock.cubes[0].origin, Vec3::new(7.0, -2.0, 14.0));
    assert_eq!(lock.cubes[0].size, Vec3::new(2.0, 4.0, 1.0));
    assert_eq!(lock.cubes[0].uv, [0, 0]);
}

/// Vanilla `ChestModel.createDoubleBodyLeftLayer`/`Right`: the bottom and lid widen to 15 and
/// the lock moves to the outer edge (left x 0, right x 15), still 1x4x1.
#[test]
fn double_chest_models_match_vanilla_layers() {
    let left = chest_model(ChestHalf::Left);
    assert_eq!(
        left.root.children[0].cubes[0].origin,
        Vec3::new(0.0, 0.0, 1.0)
    );
    assert_eq!(
        left.root.children[0].cubes[0].size,
        Vec3::new(15.0, 10.0, 14.0)
    );
    assert_eq!(
        left.root.children[1].cubes[0].origin,
        Vec3::new(0.0, 0.0, 0.0)
    );
    assert_eq!(
        left.root.children[1].cubes[0].size,
        Vec3::new(15.0, 5.0, 14.0)
    );
    assert_eq!(
        left.root.children[2].cubes[0].origin,
        Vec3::new(0.0, -2.0, 14.0)
    );
    assert_eq!(
        left.root.children[2].cubes[0].size,
        Vec3::new(1.0, 4.0, 1.0)
    );

    let right = chest_model(ChestHalf::Right);
    assert_eq!(
        right.root.children[0].cubes[0].origin,
        Vec3::new(1.0, 0.0, 1.0)
    );
    assert_eq!(
        right.root.children[0].cubes[0].size,
        Vec3::new(15.0, 10.0, 14.0)
    );
    assert_eq!(
        right.root.children[2].cubes[0].origin,
        Vec3::new(15.0, -2.0, 14.0)
    );
    assert_eq!(
        right.root.children[2].cubes[0].size,
        Vec3::new(1.0, 4.0, 1.0)
    );
}
