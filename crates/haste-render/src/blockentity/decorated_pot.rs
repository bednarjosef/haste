// The decorated pot (vanilla `DecoratedPotRenderer`): its base and sides layers, the side
// texture per sherd, the direction turn (`modelTransformation`) and the wobble.

use std::sync::OnceLock;

use glam::{Mat4, Vec3};
use haste_core::model::CubeModel;
use haste_core::{BlockPos, Direction, Identifier};

use super::object::{emit_object, empty_root, part};
use super::{to_y_rot, BlockEntityAnim, BlockEntityInstance};
use crate::entity::EntityVertex;

/// Vanilla `DecoratedPotRenderer.createBaseLayer` (32x32): the neck (a deflated ring plus the
/// inflated flare) above the block's top, and the top and bottom planes capping it.
fn pot_base_model() -> &'static CubeModel {
    static MODEL: OnceLock<CubeModel> = OnceLock::new();
    MODEL.get_or_init(|| CubeModel {
        texture: Identifier::minecraft("entity/decorated_pot/decorated_pot_base"),
        tex_width: 32,
        tex_height: 32,
        root: empty_root(vec![
            part(
                "neck",
                Vec3::new(0.0, 37.0, 16.0),
                Vec3::new(std::f32::consts::PI, 0.0, 0.0),
            )
            .cube(
                Vec3::new(4.0, 17.0, 4.0),
                Vec3::new(8.0, 3.0, 8.0),
                -0.1,
                [0, 0],
            )
            .cube(
                Vec3::new(5.0, 20.0, 5.0),
                Vec3::new(6.0, 1.0, 6.0),
                0.2,
                [0, 5],
            )
            .done(Vec::new()),
            // Both planes share vanilla's `topBottomPlane` (texOffs(-14, 13)); the negative u
            // is vanilla's own, stored wrapped in the cube's `u32` offset.
            part("top", Vec3::new(1.0, 16.0, 1.0), Vec3::ZERO)
                .cube(
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(14.0, 0.0, 14.0),
                    0.0,
                    [-14i32 as u32, 13],
                )
                .done(Vec::new()),
            part("bottom", Vec3::new(1.0, 0.0, 1.0), Vec3::ZERO)
                .cube(
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(14.0, 0.0, 14.0),
                    0.0,
                    [-14i32 as u32, 13],
                )
                .done(Vec::new()),
        ]),
    })
}

/// Vanilla `DecoratedPotRenderer.createSidesLayer` (16x16): one side plane per direction, the
/// same 14x16 quad turned onto each wall. `front` faces the block's `facing`.
fn pot_sides_model() -> &'static CubeModel {
    static MODEL: OnceLock<CubeModel> = OnceLock::new();
    MODEL.get_or_init(|| {
        let side = |name: &str, pivot: Vec3, rotation: Vec3| {
            part(name, pivot, rotation)
                .cube(
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(14.0, 16.0, 0.0),
                    0.0,
                    [1, 0],
                )
                .done(Vec::new())
        };
        CubeModel {
            texture: Identifier::minecraft("entity/decorated_pot/decorated_pot_side"),
            tex_width: 16,
            tex_height: 16,
            root: empty_root(vec![
                side(
                    "back",
                    Vec3::new(15.0, 16.0, 1.0),
                    Vec3::new(0.0, 0.0, std::f32::consts::PI),
                ),
                side(
                    "left",
                    Vec3::new(1.0, 16.0, 1.0),
                    Vec3::new(0.0, -std::f32::consts::FRAC_PI_2, std::f32::consts::PI),
                ),
                side(
                    "right",
                    Vec3::new(15.0, 16.0, 15.0),
                    Vec3::new(0.0, std::f32::consts::FRAC_PI_2, std::f32::consts::PI),
                ),
                side(
                    "front",
                    Vec3::new(1.0, 16.0, 15.0),
                    Vec3::new(std::f32::consts::PI, 0.0, 0.0),
                ),
            ]),
        }
    })
}

/// The decorated pot's base texture (vanilla `Sheets.DECORATED_POT_BASE`).
pub fn pot_base_texture() -> Identifier {
    Identifier::minecraft("entity/decorated_pot/decorated_pot_base")
}

/// The texture an undecorated pot side is drawn with (vanilla `Sheets.DECORATED_POT_SIDE`).
pub fn pot_side_texture(sherd: Option<&Identifier>) -> Identifier {
    match sherd {
        // Vanilla `DecoratedPotRenderer.getSideSprite`: the blank sprite without a pattern.
        None => Identifier::minecraft("entity/decorated_pot/decorated_pot_side"),
        // `Sheets.DECORATED_POT_MAPPER` applied to the pattern's asset id.
        Some(id) => Identifier::new(
            id.namespace(),
            &format!("entity/decorated_pot/{}", id.path()),
        ),
    }
}

/// Vanilla `DecoratedPotRenderer.modelTransformation`: `180 - facing.toYRot()` degrees about
/// the block centre (its pose stack is the block's corner, so the rotation is about 0.5, 0.5,
/// 0.5).
fn pot_direction_rotation(pos: BlockPos, facing: Direction, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3())
        * rotate_around_y(
            (180.0 - to_y_rot(facing)).to_radians(),
            Vec3::new(0.5, 0.5, 0.5),
        )
}

/// A rotation about y by `angle`, around the point `p`.
fn rotate_around_y(angle: f32, p: Vec3) -> Mat4 {
    Mat4::from_translation(p) * Mat4::from_rotation_y(angle) * Mat4::from_translation(-p)
}

/// A rotation about `axis` by `angle`, around the point `p`.
fn rotate_around(angle: f32, axis: Vec3, p: Vec3) -> Mat4 {
    Mat4::from_translation(p) * Mat4::from_axis_angle(axis, angle) * Mat4::from_translation(-p)
}

/// Vanilla `DecoratedPotRenderer.submit`: the direction turn, then the wobble. 26.3's
/// `extractRenderState` never assigns `state.wobbleStyle`, so the tilt branch is unreachable
/// and the pot turns about (0.5, 0, 0.5) by `sin(-progress * 3PI) * 0.125 * (1 - progress)`
/// instead.
fn pot_root(pos: BlockPos, facing: Direction, progress: f32, camera: glam::DVec3) -> Mat4 {
    let wobble = if (0.0..=1.0).contains(&progress) {
        let turn = ((-progress * 3.0 * std::f32::consts::PI) as f64).sin() as f32 * 0.125;
        let decay = 1.0 - progress;
        rotate_around(turn * decay, Vec3::Y, Vec3::new(0.5, 0.0, 0.5))
    } else {
        Mat4::IDENTITY
    };
    pot_direction_rotation(pos, facing, camera) * wobble * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}
/// Poses one decorated pot into `out` (the `emit_model_block_entity` pot arm): the base layer
/// plus the four sherd sides, each with its own texture.
pub(super) fn submit(
    out: &mut Vec<EntityVertex>,
    inst: &BlockEntityInstance,
    camera: glam::DVec3,
    light: (u8, u8),
    rect: [u32; 4],
    rect_of: &mut dyn FnMut(&Identifier) -> [u32; 4],
) {
    // Vanilla's wobble progress: (gameTime - start + partial) / style.duration. The turn
    // branch vanilla's (unassigned) `state.wobbleStyle` leaves reachable is the NEGATIVE
    // one, but the progress still runs over the evented style's own duration.
    let root = match inst.anim {
        Some(BlockEntityAnim::Pot { wobble, duration }) => {
            pot_root(inst.pos, inst.facing, wobble.end / duration as f32, camera)
        }
        _ => pot_root(inst.pos, inst.facing, 0.0, camera),
    };
    let base = pot_base_model();
    emit_object(out, base, Vec3::ZERO, &[], root, light, &mut |_| rect);
    // The four sherd sides, each with its own texture (the blank sprite without one).
    let sides = pot_sides_model();
    let sherds = inst.sherds.as_deref();
    emit_object(out, sides, Vec3::ZERO, &[], root, light, &mut |name| {
        let sherd = match name {
            "front" => sherds.and_then(|s| s.front.as_ref()),
            "back" => sherds.and_then(|s| s.back.as_ref()),
            "left" => sherds.and_then(|s| s.left.as_ref()),
            "right" => sherds.and_then(|s| s.right.as_ref()),
            _ => None,
        };
        rect_of(&pot_side_texture(sherd))
    });
}
