// The lectern's book (vanilla `LecternRenderer`).

use glam::{Mat4, Vec3};
use haste_core::{BlockPos, Direction};

use super::book::{book_openness, book_pose, emit_book};
use super::{clockwise, to_y_rot, BlockEntityInstance};
use crate::entity::EntityVertex;

/// Vanilla `LecternRenderer.submit`'s transform: (0.5, 1.0625, 0.5), `-yRot` about y, 67.5
/// degrees about z, then (0, -0.125, 0). `yRot` is the block's
/// `facing.getClockWise().toYRot()`.
fn lectern_root(pos: BlockPos, facing: Direction, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 1.0625, 0.5))
        * Mat4::from_rotation_y((-to_y_rot(clockwise(facing))).to_radians())
        * Mat4::from_rotation_z(67.5_f32.to_radians())
        * Mat4::from_translation(Vec3::new(0.0, -0.125, 0.0))
        * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}

/// Vanilla `LecternRenderer.submit`: the book at the lectern's top, posed by the fixed
/// `BOOK_STATE`.
pub(super) fn submit(
    out: &mut Vec<EntityVertex>,
    inst: &BlockEntityInstance,
    camera: glam::DVec3,
    light: (u8, u8),
    rect: [u32; 4],
) {
    // Vanilla `LecternRenderer`'s fixed `BOOK_STATE`: `forAnimation(0, 0.1, 0.9, 1.2)`.
    let openness = book_openness(0.0, 1.2);
    let (pose, offset) = book_pose(openness, 0.1, 0.9);
    emit_book(
        out,
        &pose,
        offset,
        lectern_root(inst.pos, inst.facing, camera),
        light,
        rect,
    );
}
