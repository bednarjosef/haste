//! Unit tests for `crate::leash`: the rope's geometry against vanilla's numbers, and the link map.

use haste_core::{DVec3, EntityId};

use crate::leash::{
    attach_offset, knot_center, rope_hold, y_rot, Leashes, KNOT_OFFSET, KNOT_ROPE_HOLD_Y,
    MOB_ROPE_HOLD,
};

fn close(a: DVec3, b: DVec3) -> bool {
    (a - b).length() < 1e-9
}

#[test]
fn knot_offset_is_the_box_centre() {
    // `LeashFenceKnotEntity` sits at its block corner; the box centre is +0.5, +0.375, +0.5.
    assert_eq!(KNOT_OFFSET, DVec3::new(0.5, 0.375, 0.5));
    assert!(close(
        knot_center(DVec3::new(10.0, 64.0, -3.0)),
        DVec3::new(10.5, 64.375, -2.5)
    ));
}

#[test]
fn y_rot_quarter_turn() {
    // `Vec3.yRot(90 deg)`: (1, 0, 0) becomes (0, 0, -1).
    let r = y_rot(DVec3::X, std::f64::consts::FRAC_PI_2);
    assert!(close(r, DVec3::new(0.0, 0.0, -1.0)), "{r:?}");
}

#[test]
fn attach_offset_faces_the_body_at_zero_yaw() {
    // Body yaw 0: the rope starts at eye height, 0.4 of the width straight out along +z.
    let o = attach_offset(1.62, 0.6, 0.0);
    assert!(close(o, DVec3::new(0.0, 1.62, 0.24)), "{o:?}");
}

#[test]
fn attach_offset_turns_with_the_body() {
    // Body yaw 90: the offset is 0.4 * width along -x (the `yRot(-yaw)` turn).
    let o = attach_offset(1.0, 1.0, 90.0);
    assert!(close(o, DVec3::new(-0.4, 1.0, 0.0)), "{o:?}");
}

#[test]
fn mob_rope_hangs_at_seventy_percent_of_eye_height() {
    let p = rope_hold(DVec3::new(1.0, 2.0, 3.0), 1.62, false);
    assert!((p.y - (2.0 + 1.62 * MOB_ROPE_HOLD)).abs() < 1e-12);
    assert_eq!((p.x, p.z), (1.0, 3.0));
}

#[test]
fn knot_rope_point_is_fixed_above_origin() {
    let p = rope_hold(DVec3::new(0.0, 64.375, 0.0), 0.0, true);
    assert!((p.y - (64.375 + KNOT_ROPE_HOLD_Y)).abs() < 1e-12);
}

#[test]
fn link_map_sets_replaces_and_unties() {
    let mut l = Leashes::default();
    l.set(EntityId(5), Some(EntityId(9)));
    l.set(EntityId(5), Some(EntityId(11)));
    let links: Vec<_> = l.iter().collect();
    assert_eq!(links, vec![(EntityId(5), EntityId(11))]);
    l.set(EntityId(5), None);
    assert_eq!(l.iter().count(), 0);
}

#[test]
fn removing_either_end_drops_the_link() {
    let mut l = Leashes::default();
    l.set(EntityId(1), Some(EntityId(100)));
    l.set(EntityId(2), Some(EntityId(200)));
    l.set(EntityId(3), Some(EntityId(300)));
    l.remove_entities(&[EntityId(1), EntityId(200)]);
    let links: Vec<_> = l.iter().collect();
    assert_eq!(links, vec![(EntityId(3), EntityId(300))]);
    l.clear();
    assert_eq!(l.iter().count(), 0);
}
