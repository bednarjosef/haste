// The enchanting table's floating book (vanilla `EnchantTableRenderer`).

use glam::{Mat4, Vec3};
use haste_core::BlockPos;

use super::book::{book_model, book_openness, book_pose};
use super::{emit_object, BlockEntityAnim, BlockEntityInstance};
use crate::entity::EntityVertex;

/// Vanilla `EnchantTableRenderer.submit`'s page flips: `frac(flip + offset) * 1.6 - 0.3`,
/// clamped to 0..1, at offsets 0.25 and 0.75.
pub fn enchant_page_flip(flip: f32, offset: f32) -> f32 {
    let f = flip + offset - (flip + offset).floor();
    (f * 1.6 - 0.3).clamp(0.0, 1.0)
}

/// Vanilla `EnchantTableRenderer.submit`'s transform: (0.5, 0.75, 0.5), the bob
/// `0.1 + sin(time * 0.1) * 0.01`, `-rot` about y and 80 degrees about z. `rot` is the
/// interpolated `rot` (radians).
fn enchant_root(pos: BlockPos, time: f32, rot: f32, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.75, 0.5))
        * Mat4::from_translation(Vec3::new(
            0.0,
            0.1 + ((time * 0.1) as f64).sin() as f32 * 0.01,
            0.0,
        ))
        * Mat4::from_rotation_y(-rot)
        * Mat4::from_rotation_z(80.0_f32.to_radians())
        * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}

/// Vanilla `EnchantTableRenderer.submit`: the floating book over the table.
pub(super) fn submit(
    out: &mut Vec<EntityVertex>,
    inst: &BlockEntityInstance,
    camera: glam::DVec3,
    light: (u8, u8),
    rect: [u32; 4],
) {
    // Vanilla `EnchantTableRenderer.submit`'s book state from the block entity's tick.
    let (time, flip, open, rot) = match inst.anim {
        Some(BlockEntityAnim::EnchantTable {
            time,
            flip,
            open,
            rot,
            ..
        }) => (time, flip.end, open.end, rot.end),
        _ => (0.0, 0.0, 0.0, 0.0),
    };
    let ff1 = enchant_page_flip(flip, 0.25);
    let ff2 = enchant_page_flip(flip, 0.75);
    let openness = book_openness(time, open);
    let (pose, offset) = book_pose(openness, ff1, ff2);
    let model = book_model();
    emit_object(
        out,
        model,
        offset,
        &pose,
        enchant_root(inst.pos, time, rot, camera),
        light,
        &mut |_| rect,
    );
}
