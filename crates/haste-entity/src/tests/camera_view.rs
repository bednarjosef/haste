//! What a camera viewing from an entity reads off it (`crate::camera_view`): the view rotation
//! (`Entity.getViewYRot` / `getViewXRot`), the eye height (`Entity.getEyeHeight` with
//! `Avatar.POSES` and `LivingEntity.SLEEPING_DIMENSIONS`) and the hurt and death counters.

use super::*;

fn pose(p: Pose) -> Vec<MetadataEntry> {
    vec![MetadataEntry {
        index: 6,
        value: MetadataValue::Pose(p),
    }]
}

#[test]
fn a_living_entity_looks_along_its_head_and_lerps_it_like_rot_lerp() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    // `rotate_head` eases the head over 3 ticks (`lerpHeadTo(yaw, 3)`); the body stays.
    s.set_head_yaw(EntityId(1), 90.0);
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert!(e.is_living());
    assert_eq!(e.view_y_rot(1.0), e.head_yaw);
    assert_eq!(e.view_y_rot(0.0), e.head_yaw_at(0.0));
    let half = e.head_yaw_at(0.0) + 0.5 * (e.head_yaw - e.head_yaw_at(0.0));
    assert_eq!(e.view_y_rot(0.5), half);
    assert_ne!(
        e.view_y_rot(1.0),
        e.yaw,
        "the head turned, the body did not"
    );
    assert_eq!(e.view_x_rot(1.0), e.pitch);
}

#[test]
fn eye_height_follows_the_type_the_pose_and_the_avatar_table() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    // The default box (no oracle dump in tests): 1.62.
    assert_eq!(s.eye_height(EntityId(1)), Some(1.62));
    // A sleeping living entity: `LivingEntity.SLEEPING_DIMENSIONS`' 0.2.
    s.set_metadata(EntityId(1), &pose(Pose::Sleeping));
    assert_eq!(s.eye_height(EntityId(1)), Some(0.2));
    // A player crouching: `Avatar.POSES`' 1.27; swimming 0.4.
    let sp = spawn(2, DVec3::ZERO);
    s.player_info(&[PlayerInfo {
        uuid: sp.uuid,
        name: "Alex".into(),
        display_name: None,
        latency: 0,
        game_mode: Some(GameMode::Survival),
        listed: true,
        skin_texture_url: None,
    }]);
    s.spawn(&sp);
    assert_eq!(s.eye_height(EntityId(2)), Some(1.62));
    s.set_metadata(EntityId(2), &pose(Pose::Crouching));
    assert_eq!(s.eye_height(EntityId(2)), Some(1.27));
    s.set_metadata(EntityId(2), &pose(Pose::Swimming));
    assert_eq!(s.eye_height(EntityId(2)), Some(0.4));
    // Untracked: nothing.
    assert_eq!(s.eye_height(EntityId(3)), None);
}

#[test]
fn living_tilt_counts_hurt_time_down_and_death_time_up() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    let tilt = |s: &EntityStore| s.get(EntityId(1)).unwrap().living_tilt().unwrap();
    assert_eq!(tilt(&s).hurt_time, 0);
    s.mark_hurt(EntityId(1));
    assert_eq!(tilt(&s).hurt_time, 10);
    s.tick();
    assert_eq!(tilt(&s).hurt_time, 9);
    assert_eq!(tilt(&s).death_time, None);
    // Death: `deathTime` is 0 on the tick of death and counts up; the blow's hurt runs out.
    s.mark_dead(EntityId(1));
    assert_eq!(tilt(&s).death_time, Some(0));
    assert_eq!(tilt(&s).hurt_time, 10);
    for _ in 0..5 {
        s.tick();
    }
    assert_eq!(tilt(&s).death_time, Some(5));
    assert_eq!(tilt(&s).hurt_time, 5);
}
