//! `StuckInBodyLayer`: the arrows and bee stingers stuck in a humanoid's body. Each stuck item
//! picks a random body part and cube, a random point in that cube (on one of its faces for
//! `ON_SURFACE`), and is drawn pointing away from the cube's centre. The random sequence is
//! vanilla's per-entity thread-local `SingleThreadedRandomSource`, seeded with the entity id,
//! so the same entity always shows its items in the same places.

use glam::{Mat4, Vec3};
use haste_core::model::{CubeModel, ModelPart, PoseOverrides};

use super::model_parts::pose_rotation;

/// Body parts `PlayerModel.bodyParts` lists, in vanilla's order (`getRandomBodyPart` indexes
/// this list).
pub(crate) const PLAYER_BODY_PARTS: [&str; 6] = [
    "head",
    "body",
    "left_arm",
    "right_arm",
    "left_leg",
    "right_leg",
];

/// `StuckInBodyLayer.PlacementStyle`: arrows sit anywhere in the cube, stingers on one face.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Placement {
    InCube,
    OnSurface,
}

/// `SingleThreadedRandomSource`: the same 48-bit linear congruential generator as
/// `java.util.Random`, scrambled from the seed the same way.
pub(crate) struct StuckRandom {
    seed: i64,
}

const MULTIPLIER: i64 = 0x5DEE_CE66D;
const ADDEND: i64 = 0xB;
const MASK: i64 = (1 << 48) - 1;

impl StuckRandom {
    pub(crate) fn new(seed: i64) -> Self {
        Self {
            seed: (seed ^ MULTIPLIER) & MASK,
        }
    }

    /// The top `bits` bits of the next 48-bit state (`Random.next`).
    fn next_bits(&mut self, bits: u32) -> i32 {
        self.seed = (self.seed.wrapping_mul(MULTIPLIER).wrapping_add(ADDEND)) & MASK;
        (self.seed >> (48 - bits)) as i32
    }

    /// Uniform in `0..bound` (`Random.nextInt(bound)`, bound > 0): a multiply for powers of
    /// two, otherwise rejection sampling so every value is equally likely.
    pub(crate) fn next_int(&mut self, bound: i32) -> i32 {
        let mut r = self.next_bits(31);
        let m = bound - 1;
        if bound & m == 0 {
            return ((bound as i64 * r as i64) >> 31) as i32;
        }
        let mut u = r;
        loop {
            r = u % bound;
            if u.wrapping_sub(r).wrapping_add(m) >= 0 {
                return r;
            }
            u = self.next_bits(31);
        }
    }

    /// Uniform in `[0, 1)` with 24 bits of precision (`Random.nextFloat`).
    pub(crate) fn next_float(&mut self) -> f32 {
        self.next_bits(24) as f32 * 5.960_464_5E-8
    }
}

/// `StuckInBodyLayer.snapToFace`: a coordinate on a cube's face, the far side (1.0) when it
/// was past the middle, the near side (0.5) otherwise.
pub(crate) fn snap_to_face(v: f32) -> f32 {
    if v > 0.5 {
        1.0
    } else {
        0.5
    }
}

/// Finds a part by name anywhere in the tree (humanoid part names are unique).
pub(crate) fn find_part<'a>(part: &'a ModelPart, name: &str) -> Option<&'a ModelPart> {
    if part.name == name {
        return Some(part);
    }
    part.children.iter().find_map(|c| find_part(c, name))
}

/// `ModelPart.translateAndRotate`: the part's own transform (pivot, then Z Y X rotation, pose
/// overrides added), without its parents, on top of `parent`.
fn own_matrix(part: &ModelPart, pose: &PoseOverrides, parent: Mat4) -> Mat4 {
    let rot = part.rotation + pose_rotation(pose, part.id).unwrap_or(Vec3::ZERO);
    let pivot = part.pivot + pose.offset_of(part.id).unwrap_or(Vec3::ZERO);
    parent
        * Mat4::from_translation(pivot)
        * Mat4::from_rotation_z(rot.z)
        * Mat4::from_rotation_y(rot.y)
        * Mat4::from_rotation_x(rot.x)
}

/// Calls `item` with the transform of each stuck item, in model units (1/16 block) on top of
/// `root` (the entity's transform with the model scale): `StuckInBodyLayer.submit`, with
/// `submitStuckItem`'s rotation (`yaw - 90` about Y, then the pitch about Z) folded in. The
/// item's own geometry is drawn through the returned matrix. No allocation: the body part is
/// looked up by index each time.
#[allow(clippy::too_many_arguments)]
pub(crate) fn for_each_stuck_item(
    root: Mat4,
    model: &CubeModel,
    pose: &PoseOverrides,
    body_parts: &[&str],
    count: u32,
    seed: i64,
    placement: Placement,
    mut item: impl FnMut(Mat4),
) {
    if count == 0 || body_parts.is_empty() {
        return;
    }
    let mut rng = StuckRandom::new(seed);
    for _ in 0..count {
        let name = body_parts[rng.next_int(body_parts.len() as i32) as usize];
        let Some(part) = find_part(&model.root, name) else {
            continue;
        };
        if part.cubes.is_empty() {
            continue;
        }
        let cube = &part.cubes[rng.next_int(part.cubes.len() as i32) as usize];
        let own = own_matrix(part, pose, root);
        let mut mid = Vec3::new(rng.next_float(), rng.next_float(), rng.next_float());
        if placement == Placement::OnSurface {
            match rng.next_int(3) {
                0 => mid.x = snap_to_face(mid.x),
                1 => mid.y = snap_to_face(mid.y),
                _ => mid.z = snap_to_face(mid.z),
            }
        }
        let inflate = Vec3::splat(cube.inflate);
        let lo = cube.origin - inflate;
        let hi = cube.origin + cube.size + inflate;
        let pos = lo + (hi - lo) * mid;
        // The direction from the cube's centre out through the chosen point.
        let dir = -(mid * 2.0 - Vec3::ONE);
        let dir_xz = (dir.x * dir.x + dir.z * dir.z).sqrt();
        let y_rot = dir.x.atan2(dir.z).to_degrees();
        let x_rot = dir.y.atan2(dir_xz).to_degrees();
        let m = own
            * Mat4::from_translation(pos)
            * Mat4::from_rotation_y((y_rot - 90.0).to_radians())
            * Mat4::from_rotation_z(x_rot.to_radians());
        item(m);
    }
}
