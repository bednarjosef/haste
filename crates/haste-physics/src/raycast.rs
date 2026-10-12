//! Ray casts for the crosshair: blocks against their outline shapes, entities against their boxes.
//!
//! The look vector is the usual spherical one (yaw 0 looks towards +z, yaw 90 towards -x, pitch 90
//! straight down), computed with the movement sine table. Cells are walked in order along the ray
//! (voxel traversal) and the first cell whose outline boxes the ray enters is the hit; a ray that
//! starts inside a box hits that box at its start, on the face it is leaving through backwards.
use haste_core::view::{BlockView, CollisionShapes};
use haste_core::{Aabb, BlockPos, DVec3, Direction};

use crate::trig;

/// A block under the crosshair.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockHit {
    pub pos: BlockPos,
    /// Face of the block the ray entered through.
    pub face: Direction,
    /// World-space point where the ray met the outline.
    pub location: DVec3,
    /// Distance from the ray start.
    pub distance: f64,
    /// The ray started inside the block's outline.
    pub inside: bool,
}

/// Unit look vector for yaw/pitch in degrees.
pub fn look_vector(yaw: f32, pitch: f32) -> DVec3 {
    let p = trig::deg_to_rad(pitch);
    let y = trig::deg_to_rad(-yaw);
    let (cy, sy) = (trig::cos(y), trig::sin(y));
    let (cp, sp) = (trig::cos(p), trig::sin(p));
    DVec3::new((sy * cp) as f64, (-sp) as f64, (cy * cp) as f64)
}

/// Ray from `from` to `from + dir * reach` against one box: (t in 0..=1 along the segment, face).
pub fn clip_box(b: &Aabb, from: DVec3, delta: DVec3) -> Option<(f64, Direction, bool)> {
    let inside = (0..3).all(|a| from[a] > b.min[a] && from[a] < b.max[a]);
    if inside {
        // Leaving face reversed: the face on the side the ray comes from along its main axis.
        let a = (0..3)
            .max_by(|&i, &j| delta[i].abs().total_cmp(&delta[j].abs()))
            .unwrap_or(0);
        return Some((0.0, axis_face(a, delta[a] > 0.0), true));
    }
    let mut t_enter = f64::NEG_INFINITY;
    let mut t_exit = f64::INFINITY;
    let mut face = Direction::Up;
    for a in 0..3 {
        let d = delta[a];
        if d.abs() < 1e-12 {
            if from[a] < b.min[a] || from[a] > b.max[a] {
                return None;
            }
            continue;
        }
        let (t0, t1) = ((b.min[a] - from[a]) / d, (b.max[a] - from[a]) / d);
        let (near, far) = if t0 < t1 { (t0, t1) } else { (t1, t0) };
        if near > t_enter {
            t_enter = near;
            face = axis_face(a, d > 0.0);
        }
        t_exit = t_exit.min(far);
    }
    if t_enter > t_exit || t_enter < 0.0 || t_enter > 1.0 {
        return None;
    }
    Some((t_enter, face, false))
}

/// Face hit when entering along axis `a` in the positive (true) or negative direction.
fn axis_face(a: usize, positive: bool) -> Direction {
    match (a, positive) {
        (0, true) => Direction::West,
        (0, false) => Direction::East,
        (1, true) => Direction::Down,
        (1, false) => Direction::Up,
        (_, true) => Direction::North,
        (_, false) => Direction::South,
    }
}

/// First block whose outline the segment `from .. from + dir * reach` touches.
pub fn raycast_blocks(
    from: DVec3,
    dir: DVec3,
    reach: f64,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
) -> Option<BlockHit> {
    let delta = dir * reach;
    let mut cell = from.floor().as_ivec3();
    let end = (from + delta).floor().as_ivec3();
    let step = delta.signum().as_ivec3();
    // Ray parameter at the next cell boundary per axis, and per whole cell.
    let mut t_max = DVec3::ZERO;
    let mut t_delta = DVec3::ZERO;
    for a in 0..3 {
        if delta[a] == 0.0 {
            t_max[a] = f64::INFINITY;
            t_delta[a] = f64::INFINITY;
        } else {
            let next = if delta[a] > 0.0 {
                cell[a] as f64 + 1.0
            } else {
                cell[a] as f64
            };
            t_max[a] = (next - from[a]) / delta[a];
            t_delta[a] = 1.0 / delta[a].abs();
        }
    }
    // A segment crosses at most |dx|+|dy|+|dz|+1 cells.
    let max_cells = (end - cell).abs().element_sum() + 1;
    for _ in 0..=max_cells {
        let pos = BlockPos::new(cell.x, cell.y, cell.z);
        let state = world.block_state(pos);
        let mut best: Option<(f64, Direction, bool)> = None;
        for b in shapes.outline_boxes(state) {
            let wb = b.offset(pos.as_dvec3());
            if let Some(h) = clip_box(&wb, from, delta) {
                if best.is_none_or(|(t, _, _)| h.0 < t) {
                    best = Some(h);
                }
            }
        }
        if let Some((t, face, inside)) = best {
            return Some(BlockHit {
                pos,
                face,
                location: from + delta * t,
                distance: reach * t,
                inside,
            });
        }
        if cell == end {
            break;
        }
        let a = if t_max.x < t_max.y {
            if t_max.x < t_max.z {
                0
            } else {
                2
            }
        } else if t_max.y < t_max.z {
            1
        } else {
            2
        };
        if t_max[a] > 1.0 {
            break;
        }
        cell[a] += step[a];
        t_max[a] += t_delta[a];
    }
    None
}

/// Nearest entity box the segment touches: (index into `boxes`, distance).
pub fn raycast_entities(
    from: DVec3,
    dir: DVec3,
    reach: f64,
    boxes: impl IntoIterator<Item = Aabb>,
) -> Option<(usize, f64)> {
    let delta = dir * reach;
    let mut best: Option<(usize, f64)> = None;
    for (i, b) in boxes.into_iter().enumerate() {
        if let Some((t, _, _)) = clip_box(&b, from, delta) {
            if best.is_none_or(|(_, d)| reach * t < d) {
                best = Some((i, reach * t));
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{states, TestShapes, TestWorld};

    #[test]
    fn look_vector_axes() {
        let close = |a: DVec3, b: DVec3| assert!((a - b).length() < 1e-3, "{a:?} != {b:?}");
        close(look_vector(0.0, 0.0), DVec3::Z);
        close(look_vector(90.0, 0.0), -DVec3::X);
        close(look_vector(-90.0, 0.0), DVec3::X);
        close(look_vector(180.0, 0.0), -DVec3::Z);
        close(look_vector(0.0, 90.0), -DVec3::Y);
    }

    #[test]
    fn hits_floor_from_above_on_its_top_face() {
        let w = TestWorld::flat(0, states::STONE);
        let s = TestShapes::default();
        let h = raycast_blocks(
            DVec3::new(0.5, 1.62, 0.5),
            look_vector(0.0, 90.0),
            4.5,
            &w,
            &s,
        )
        .unwrap();
        assert_eq!(h.pos, BlockPos::new(0, -1, 0));
        assert_eq!(h.face, Direction::Up);
        assert!((h.location.y - 0.0).abs() < 1e-9 && (h.distance - 1.62).abs() < 1e-9);
    }

    #[test]
    fn hits_a_slab_outline_not_its_cell() {
        let mut w = TestWorld::new();
        w.set(2, 0, 0, states::BOTTOM_SLAB);
        let s = TestShapes::default();
        // Looking east at y 0.75 passes over the slab (top at 0.5).
        assert!(raycast_blocks(DVec3::new(0.5, 0.75, 0.5), DVec3::X, 4.5, &w, &s).is_none());
        let h = raycast_blocks(DVec3::new(0.5, 0.25, 0.5), DVec3::X, 4.5, &w, &s).unwrap();
        assert_eq!((h.pos, h.face), (BlockPos::new(2, 0, 0), Direction::West));
        assert!((h.location.x - 2.0).abs() < 1e-9);
    }

    #[test]
    fn reach_limits_and_negative_directions() {
        let mut w = TestWorld::new();
        w.set(-5, 0, 0, states::STONE);
        let s = TestShapes::default();
        let from = DVec3::new(0.5, 0.5, 0.5);
        assert!(raycast_blocks(from, -DVec3::X, 4.5, &w, &s).is_none());
        let h = raycast_blocks(from, -DVec3::X, 5.0, &w, &s).unwrap();
        assert_eq!((h.pos, h.face), (BlockPos::new(-5, 0, 0), Direction::East));
        assert!((h.distance - 4.5).abs() < 1e-9);
    }

    #[test]
    fn diagonal_ray_finds_the_first_cell() {
        let mut w = TestWorld::new();
        w.set(1, 0, 1, states::STONE);
        w.set(2, 0, 2, states::STONE);
        let s = TestShapes::default();
        let dir = DVec3::new(1.0, 0.0, 1.0).normalize();
        let h = raycast_blocks(DVec3::new(0.5, 0.5, 0.2), dir, 4.5, &w, &s).unwrap();
        // It crosses z = 1 at x = 1.3: enters (1, 0, 1) through its north face.
        assert_eq!(h.pos, BlockPos::new(1, 0, 1));
        assert_eq!(h.face, Direction::North);
    }

    #[test]
    fn entity_boxes_nearest_wins() {
        let boxes = [
            Aabb::entity(DVec3::new(0.0, 0.0, 3.0), 0.6, 1.8),
            Aabb::entity(DVec3::new(0.0, 0.0, 2.0), 0.6, 1.8),
            Aabb::entity(DVec3::new(5.0, 0.0, 2.0), 0.6, 1.8),
        ];
        let (i, d) = raycast_entities(DVec3::new(0.0, 1.0, 0.0), DVec3::Z, 3.0, boxes).unwrap();
        assert_eq!(i, 1);
        assert!((d - 1.7).abs() < 1e-9);
    }
}
