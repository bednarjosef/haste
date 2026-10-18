// The conduit (vanilla `ConduitRenderer`): its shell, cage, wind and eye layers, their
// textures, the bob and spin, and the roots that pose them (the eye billboarded to the camera
// orientation, passed in by the frame).

use std::sync::OnceLock;

use glam::{Mat4, Vec3};
use haste_core::model::CubeModel;
use haste_core::{BlockPos, Identifier};

use super::object::{emit_object, emit_object_part, empty_root, part};
use super::{BlockEntityAnim, BlockEntityInstance};
use crate::entity::EntityVertex;

/// Vanilla `ConduitRenderer.createShellLayer` (32x16): the inactive conduit's shell.
fn conduit_shell_model() -> &'static CubeModel {
    static MODEL: OnceLock<CubeModel> = OnceLock::new();
    MODEL.get_or_init(|| CubeModel {
        texture: Identifier::minecraft("entity/conduit/base"),
        tex_width: 32,
        tex_height: 16,
        root: empty_root(vec![part("shell", Vec3::ZERO, Vec3::ZERO)
            .cube(
                Vec3::new(-3.0, -3.0, -3.0),
                Vec3::new(6.0, 6.0, 6.0),
                0.0,
                [0, 0],
            )
            .done(Vec::new())]),
    })
}

/// Vanilla `ConduitRenderer.createCageLayer` (32x16): the active conduit's spinning cage.
fn conduit_cage_model() -> &'static CubeModel {
    static MODEL: OnceLock<CubeModel> = OnceLock::new();
    MODEL.get_or_init(|| CubeModel {
        texture: Identifier::minecraft("entity/conduit/cage"),
        tex_width: 32,
        tex_height: 16,
        root: empty_root(vec![part("shell", Vec3::ZERO, Vec3::ZERO)
            .cube(
                Vec3::new(-4.0, -4.0, -4.0),
                Vec3::new(8.0, 8.0, 8.0),
                0.0,
                [0, 0],
            )
            .done(Vec::new())]),
    })
}

/// Vanilla `ConduitRenderer.createWindLayer` (64x32): the wind box, drawn twice (the second copy
/// counter-rotated by the renderer).
fn conduit_wind_model() -> &'static CubeModel {
    static MODEL: OnceLock<CubeModel> = OnceLock::new();
    MODEL.get_or_init(|| CubeModel {
        texture: Identifier::minecraft("entity/conduit/wind"),
        tex_width: 64,
        tex_height: 32,
        root: empty_root(vec![part("wind", Vec3::ZERO, Vec3::ZERO)
            .cube(
                Vec3::new(-8.0, -8.0, -8.0),
                Vec3::new(16.0, 16.0, 16.0),
                0.0,
                [0, 0],
            )
            .done(Vec::new())]),
    })
}

/// Vanilla `ConduitRenderer.createEyeLayer` (16x16): the billboarded eye plane, inflated by
/// vanilla's `CubeDeformation(0.01)`.
fn conduit_eye_model() -> &'static CubeModel {
    static MODEL: OnceLock<CubeModel> = OnceLock::new();
    MODEL.get_or_init(|| CubeModel {
        texture: Identifier::minecraft("entity/conduit/open_eye"),
        tex_width: 16,
        tex_height: 16,
        root: empty_root(vec![part("eye", Vec3::ZERO, Vec3::ZERO)
            .cube(
                Vec3::new(-4.0, -4.0, 0.0),
                Vec3::new(8.0, 8.0, 0.0),
                0.01,
                [0, 0],
            )
            .done(Vec::new())]),
    })
}

/// The conduit's shell texture (vanilla `ConduitRenderer.SHELL_TEXTURE`).
pub fn conduit_shell_texture() -> Identifier {
    Identifier::minecraft("entity/conduit/base")
}

/// The conduit's cage texture (vanilla `ConduitRenderer.ACTIVE_SHELL_TEXTURE`).
pub fn conduit_cage_texture() -> Identifier {
    Identifier::minecraft("entity/conduit/cage")
}

/// The conduit's wind texture (vanilla `ConduitRenderer.WIND_TEXTURE`, or the vertical one for
/// animation phase 1).
pub fn conduit_wind_texture(phase: i32) -> Identifier {
    if phase == 1 {
        Identifier::minecraft("entity/conduit/wind_vertical")
    } else {
        Identifier::minecraft("entity/conduit/wind")
    }
}

/// The conduit's eye texture: the open one while it attacks (vanilla
/// `ConduitRenderer.OPEN_EYE_TEXTURE`), the closed one otherwise.
pub fn conduit_eye_texture(hunting: bool) -> Identifier {
    if hunting {
        Identifier::minecraft("entity/conduit/open_eye")
    } else {
        Identifier::minecraft("entity/conduit/closed_eye")
    }
}

/// Vanilla `ConduitRenderer.submit`'s `hh`: `sin(animTime * 0.1) / 2 + 0.5`, then squared plus
/// itself.
pub fn conduit_bob(anim_time: f32) -> f32 {
    let hh = ((anim_time * 0.1) as f64).sin() as f32 / 2.0 + 0.5;
    hh * hh + hh
}

/// Vanilla `ConduitBlockEntity.getActiveRotation`: the per-tick counter times `-0.0375`, in
/// degrees (`ConduitRenderer` spins by it directly).
pub fn conduit_spin(active_rotation: f32) -> f32 {
    active_rotation * -0.0375
}

/// Vanilla `ConduitRenderer.submit`'s inactive shell: (0.5, 0.5, 0.5) and the spin about y.
fn conduit_shell_root(pos: BlockPos, spin: f32, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3() + Vec3::splat(0.5))
        * Mat4::from_rotation_y(spin.to_radians())
        * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}

/// The active cage: (0.5, 0.3 + hh * 0.2, 0.5) and the spin about the normalised (0.5, 1, 0.5)
/// axis. `spin` is already an angle in radians: vanilla converts
/// `getActiveRotation`'s value to degrees and straight back to radians, so the cage turns by the
/// same number read as radians.
fn conduit_cage_root(pos: BlockPos, spin: f32, hh: f32, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    let axis = Vec3::new(0.5, 1.0, 0.5).normalize();
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.3 + hh * 0.2, 0.5))
        * Mat4::from_axis_angle(axis, spin)
        * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}

/// The wind box: (0.5, 0.5, 0.5) with the animation phase's quarter turn about x (phase 1) or z
/// (phase 2).
fn conduit_wind_root(pos: BlockPos, phase: i32, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    let mut m = Mat4::from_translation(p.as_vec3() + Vec3::splat(0.5));
    if phase == 1 {
        m *= Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2);
    } else if phase == 2 {
        m *= Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2);
    }
    m * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}

/// The wind box's second copy: scaled 0.875 and turned by `rotationXYZ(PI, 0, PI)`, which is a
/// half turn about y.
fn conduit_wind_back_root(pos: BlockPos, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3() + Vec3::splat(0.5))
        * Mat4::from_scale(Vec3::splat(0.875))
        * Mat4::from_rotation_y(std::f32::consts::PI)
        * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}

/// The eye: bobbed like the cage, scaled 0.5, turned to face the camera, then
/// `rotationZ(PI).rotateY(PI)` (a half turn about x) and scaled by vanilla's 1.3333334.
fn conduit_eye_root(pos: BlockPos, hh: f32, orientation: glam::Quat, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.3 + hh * 0.2, 0.5))
        * Mat4::from_scale(Vec3::splat(0.5))
        * Mat4::from_quat(orientation)
        * Mat4::from_rotation_x(std::f32::consts::PI)
        * Mat4::from_scale(Vec3::splat(1.3333334))
        * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}
/// Poses one conduit into `out` (the `emit_model_block_entity` conduit arm): the shell when
/// inactive, the cage, wind and billboarded eye when active.
pub(super) fn submit(
    out: &mut Vec<EntityVertex>,
    inst: &BlockEntityInstance,
    camera: glam::DVec3,
    orientation: glam::Quat,
    light: (u8, u8),
    rect: [u32; 4],
    rect_of: &mut dyn FnMut(&Identifier) -> [u32; 4],
) {
    let (tick_count, rotation, active, hunting) = match inst.anim {
        Some(BlockEntityAnim::Conduit {
            tick_count,
            rotation,
            active,
            hunting,
        }) => (tick_count, rotation, active, hunting),
        _ => (0.0, 0.0, false, false),
    };
    // Vanilla's `getActiveRotation`: the counter times -0.0375. The renderer names that
    // value in degrees for the shell (`rotateDegrees`) and in radians for the cage (its
    // `activeRotation * 180/PI`, then back through `* PI/180`), so the cage turns by the
    // same number read as radians.
    let spin = conduit_spin(rotation);
    let anim_time = tick_count;
    let phase = (tick_count as i32 / 66).rem_euclid(3);
    if !active {
        emit_object(
            out,
            conduit_shell_model(),
            Vec3::ZERO,
            &[],
            conduit_shell_root(inst.pos, spin, camera),
            light,
            &mut |_| rect,
        );
        return;
    }
    let hh = conduit_bob(anim_time);
    // The cage: the spin in radians about the normalised (0.5, 1, 0.5) axis.
    emit_object(
        out,
        conduit_cage_model(),
        Vec3::ZERO,
        &[],
        conduit_cage_root(inst.pos, spin, hh, camera),
        light,
        &mut |_| rect_of(&conduit_cage_texture()),
    );
    // The wind, twice (the second copy counter-rotated).
    let wind = conduit_wind_model();
    let wind_rect = rect_of(&conduit_wind_texture(phase));
    emit_object(
        out,
        wind,
        Vec3::ZERO,
        &[],
        conduit_wind_root(inst.pos, phase, camera),
        light,
        &mut |_| wind_rect,
    );
    let back = conduit_wind_back_root(inst.pos, camera);
    for part in &wind.root.children {
        emit_object_part(out, wind, part, &[], back, wind_rect, Vec3::ZERO, light);
    }
    // The eye, billboarded to the camera.
    let eye_rect = rect_of(&conduit_eye_texture(hunting));
    emit_object(
        out,
        conduit_eye_model(),
        Vec3::ZERO,
        &[],
        conduit_eye_root(inst.pos, hh, orientation, camera),
        light,
        &mut |_| eye_rect,
    );
}
