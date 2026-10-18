//! The camera (vanilla's `Camera`): the F5 perspectives and the third-person pullback, plus
//! the lava-submersion rule the fog and FOV read off the camera's position.

use glam::DVec3;
use haste_core::view::{BlockView, CollisionShapes};
use haste_core::BlockPos;
use haste_data::GameData;
use haste_world::World;

/// How far the third-person camera sits from the eye when nothing is in the way (vanilla's
/// `Camera` max zoom, minecraft.wiki "Perspective"). Vanilla's `Camera.alignWithEntity` scales
/// this by the entity's scale and its `CAMERA_DISTANCE` attribute (default 4.0); haste reads
/// neither yet, so the constant is the whole distance.
pub const THIRD_PERSON_DISTANCE: f64 = 4.0;

/// The perspective F5 cycles through (vanilla's three: first person, third person behind,
/// third person in front).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Perspective {
    #[default]
    FirstPerson,
    /// The camera sits behind the player, looking where the player looks.
    ThirdPersonBehind,
    /// The camera sits in front of the player, looking back at it.
    ThirdPersonFront,
}

impl Perspective {
    /// The `--perspective` screenshot flag's names.
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        match s {
            "first" => Ok(Perspective::FirstPerson),
            "back" => Ok(Perspective::ThirdPersonBehind),
            "front" => Ok(Perspective::ThirdPersonFront),
            _ => anyhow::bail!("unknown perspective {s:?} (first, back or front)"),
        }
    }

    /// F5: the next perspective, vanilla's cycle.
    pub fn next(self) -> Self {
        match self {
            Perspective::FirstPerson => Perspective::ThirdPersonBehind,
            Perspective::ThirdPersonBehind => Perspective::ThirdPersonFront,
            Perspective::ThirdPersonFront => Perspective::FirstPerson,
        }
    }

    /// Whether the local player's own model is drawn (only in third person; in first person
    /// the world is seen through its eyes).
    pub fn draws_player(self) -> bool {
        !matches!(self, Perspective::FirstPerson)
    }
}

/// Camera position and look for a perspective: at the eye in first person; otherwise
/// [`THIRD_PERSON_DISTANCE`] blocks from the eye along the view direction (in front mode the
/// camera faces back: yaw + 180, pitch negated), pulled closer when a block is in the way
/// (vanilla's `Camera.getMaxZoom`, see [`camera_pullback`]).
pub fn third_person_camera(
    perspective: Perspective,
    eye: DVec3,
    yaw: f32,
    pitch: f32,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
) -> (DVec3, f32, f32) {
    match perspective {
        Perspective::FirstPerson => (eye, yaw, pitch),
        Perspective::ThirdPersonBehind => {
            let back = -haste_physics::raycast::look_vector(yaw, pitch);
            (camera_pullback(eye, back, world, shapes), yaw, pitch)
        }
        Perspective::ThirdPersonFront => {
            let forward = haste_physics::raycast::look_vector(yaw, pitch);
            (
                camera_pullback(eye, forward, world, shapes),
                yaw + 180.0,
                -pitch,
            )
        }
    }
}

/// `eye + dir * d` for the largest `d` no block along the way beats (at most
/// [`THIRD_PERSON_DISTANCE`]), vanilla's `Camera.getMaxZoom`: 8 rays from the eye's ±0.1
/// corners along `dir`, each cast up to the current best `d`, and every hit whose straight-line
/// distance from the eye is smaller shrinks it. The distance is measured from the eye, not
/// along the ray, and no clearance margin is subtracted: on a flat wall the camera stops
/// epsilon inside the face, and a block the eye is inside counts as a hit at the corner and
/// pulls the camera to sqrt(0.03) ≈ 0.173 blocks out.
fn camera_pullback(
    eye: DVec3,
    dir: DVec3,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
) -> DVec3 {
    let mut camera_dist = THIRD_PERSON_DISTANCE;
    for i in 0..8i32 {
        // The eight corners of the 0.2 cube around the eye (vanilla's jitter offsets).
        let corner = eye
            + DVec3::new(
                ((i & 1) * 2 - 1) as f64 * 0.1,
                ((i >> 1 & 1) * 2 - 1) as f64 * 0.1,
                ((i >> 2 & 1) * 2 - 1) as f64 * 0.1,
            );
        let hit = haste_physics::raycast::raycast_blocks(corner, dir, camera_dist, world, shapes);
        if let Some(h) = hit {
            let d = (h.location - eye).length();
            if d < camera_dist {
                camera_dist = d;
            }
        }
    }
    eye + dir * camera_dist
}

/// Whether the camera is under the lava surface (vanilla's `Camera.getFluidInCamera` lava
/// rule: lava at the eye's block whose surface reaches the eye). The surface is the fluid's
/// own height, or the full block when lava sits directly above (vanilla's `getHeight`); the
/// comparison is `<=` as vanilla's `offsetPos.y <= height + y`. Vanilla probes the five
/// near-plane points rather than the eye block alone; that near-plane fan (which needs the
/// view direction and aspect) is not modelled.
pub(super) fn eye_in_lava(world: &World, data: &GameData, eye: DVec3) -> bool {
    use haste_core::view::{BlockPhysics, FluidKind};
    let p = BlockPos::new(
        eye.x.floor() as i32,
        eye.y.floor() as i32,
        eye.z.floor() as i32,
    );
    let Some(f) = data.fluid(world.block_state(p)) else {
        return false;
    };
    if f.kind != FluidKind::Lava {
        return false;
    }
    let full = data
        .fluid(world.block_state(p.add(0, 1, 0)))
        .is_some_and(|a| a.kind == FluidKind::Lava);
    let surface = p.y as f64 + if full { 1.0 } else { f.height as f64 };
    eye.y <= surface
}
