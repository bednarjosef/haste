use super::*;

fn set_pose(s: &mut EntityStore, id: EntityId, pose: Pose) {
    s.set_metadata(
        id,
        &[MetadataEntry {
            index: 6,
            value: MetadataValue::Pose(pose),
        }],
    );
}

/// `LivingEntity.stopSleeping`: the wake-up animation makes a sleeping player standing again, with
/// the standing dimensions back.
#[test]
fn wake_up_turns_a_sleeping_entity_standing() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    let standing = s.get(EntityId(1)).unwrap().dimensions;
    set_pose(&mut s, EntityId(1), Pose::Sleeping);
    assert_eq!(s.get(EntityId(1)).unwrap().pose, Pose::Sleeping);
    s.wake_up(EntityId(1));
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(e.pose, Pose::Standing);
    assert_eq!(e.dimensions, standing);
}

/// A wake-up for an entity that is not asleep, or not in the store, changes nothing and does not
/// panic.
#[test]
fn wake_up_leaves_other_entities_alone() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    set_pose(&mut s, EntityId(1), Pose::Crouching);
    s.wake_up(EntityId(1));
    assert_eq!(s.get(EntityId(1)).unwrap().pose, Pose::Crouching);
    s.wake_up(EntityId(99));
    assert!(s.get(EntityId(99)).is_none());
}
