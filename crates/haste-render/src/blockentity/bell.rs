// The bell (vanilla `BellRenderer`): its layer (`BellModel.createBodyLayer`), texture, the
// shake rotation (`BellModel.setupAnim`) and the root (vanilla's submit carries no transform of
// its own).

use std::sync::OnceLock;

use glam::{Mat4, Vec3};
use haste_core::model::CubeModel;
use haste_core::{BlockPos, Direction, Identifier};

use super::object::{emit_object, empty_root, part};
use super::BlockEntityAnim;
use crate::entity::EntityVertex;

/// corner; the shake rotation is per instance.
fn bell_model() -> &'static CubeModel {
    static MODEL: OnceLock<CubeModel> = OnceLock::new();
    MODEL.get_or_init(|| CubeModel {
        texture: Identifier::minecraft("entity/bell/bell_body"),
        tex_width: 32,
        tex_height: 32,
        root: empty_root(vec![part(
            "bell_body",
            Vec3::new(8.0, 12.0, 8.0),
            Vec3::ZERO,
        )
        .cube(
            Vec3::new(-3.0, -6.0, -3.0),
            Vec3::new(6.0, 7.0, 6.0),
            0.0,
            [0, 0],
        )
        .done(vec![
            // The base hangs from the block's corner (the -8, -12, -8 offset cancels
            // BellModel's parent pivot).
            part("bell_base", Vec3::new(-8.0, -12.0, -8.0), Vec3::ZERO)
                .cube(
                    Vec3::new(4.0, 4.0, 4.0),
                    Vec3::new(8.0, 2.0, 8.0),
                    0.0,
                    [0, 13],
                )
                .done(Vec::new()),
        ])]),
    })
}

/// The texture a bell is drawn with (vanilla `BellRenderer.BELL_TEXTURE`).
pub fn bell_texture() -> Identifier {
    Identifier::minecraft("entity/bell/bell_body")
}

/// Vanilla `BellModel.setupAnim`: `sin(ticks / PI) / (4 + ticks / 3)`, as the (x, z) rotation of
/// the bell body; the click direction picks the axis and sign.
pub fn bell_shake_rotation(ticks: f32, click: Option<Direction>) -> (f32, f32) {
    let Some(click) = click else {
        return (0.0, 0.0);
    };
    let base = (((ticks / std::f32::consts::PI) as f64).sin() as f32) / (4.0 + ticks / 3.0);
    match click {
        Direction::North => (-base, 0.0),
        Direction::South => (base, 0.0),
        Direction::East => (0.0, -base),
        Direction::West => (0.0, base),
        Direction::Up | Direction::Down => (0.0, 0.0),
    }
}

/// Vanilla `BellRenderer.submit` carries no transform of its own: the block entity's pose stack
/// is the block's corner, so the root is the block position and the 1/16 (the shake rides on
/// the `bell_body` part).
fn bell_root(pos: BlockPos, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3()) * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}

/// Poses one bell into `out` (the `emit_model_block_entity` bell arm): the body shaken by the
/// ring's rotation about the click direction's axis.
pub(super) fn submit(
    out: &mut Vec<EntityVertex>,
    inst: &super::BlockEntityInstance,
    camera: glam::DVec3,
    light: (u8, u8),
    rect: [u32; 4],
) {
    let model = bell_model();
    let (ticks, click) = match inst.anim {
        Some(BlockEntityAnim::Bell { ticks, click }) => (ticks.end, Some(click)),
        _ => (0.0, None),
    };
    let (x_rot, z_rot) = bell_shake_rotation(ticks, click.flatten());
    let pose = [("bell_body", Vec3::new(x_rot, 0.0, z_rot))];
    emit_object(
        out,
        model,
        Vec3::ZERO,
        &pose,
        bell_root(inst.pos, camera),
        light,
        &mut |_| rect,
    );
}
