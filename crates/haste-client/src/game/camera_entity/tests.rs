//! The camera entity: `ClientPacketListener.handleSetCamera`'s choice, the store refresh (and
//! a spectated entity removed under the camera), `Camera.alignWithEntity`'s interpolation and
//! the world pass's hidden entity.

use super::*;
use haste_core::entity::{EntitySpawn, MetadataEntry, MetadataValue};
use haste_core::game::GameMode;
use haste_core::RegistryIndex;
use haste_entity::EntityDimensions;

const OWN: EntityId = EntityId(7);
const PIG: EntityId = EntityId(42);

fn store_with(id: EntityId, pos: DVec3) -> EntityStore {
    let mut store = EntityStore::new(EntityDimensions::default());
    store.spawn(&EntitySpawn {
        id,
        uuid: uuid::Uuid::from_u128(u128::from(id.0 as u32)),
        kind: RegistryIndex(0),
        pos,
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 30.0,
        data: 0,
        velocity: DVec3::ZERO,
    });
    store
}

#[test]
fn set_camera_follows_handle_set_camera() {
    let store = store_with(PIG, DVec3::new(3000.5, 200.0, 3000.5));
    let mut camera = CameraEntity::default();
    assert!(
        camera.snapshot().is_none(),
        "the player starts as the camera entity"
    );
    // An id this level does not know (`getEntity(level)` is null): nothing changes.
    camera.set_camera(PIG, OWN, false);
    camera.refresh(&store);
    assert!(camera.snapshot().is_none());
    // A known entity becomes the camera entity.
    camera.set_camera(PIG, OWN, true);
    camera.refresh(&store);
    let s = camera.snapshot().expect("spectating the pig");
    assert_eq!(s.id, PIG);
    assert_eq!(s.pos, DVec3::new(3000.5, 200.0, 3000.5));
    // A living entity looks along its head (`LivingEntity.getViewYRot`).
    assert_eq!(s.rot, (30.0, 0.0));
    // Another unknown id while spectating leaves the pig the camera entity.
    camera.set_camera(EntityId(99), OWN, false);
    assert_eq!(camera.snapshot().map(|s| s.id), Some(PIG));
    // Our own id: the player is the camera entity again (sneaking out of spectating).
    camera.set_camera(OWN, OWN, true);
    assert!(camera.snapshot().is_none());
}

#[test]
fn the_eye_height_target_is_the_camera_entitys() {
    let mut store = store_with(PIG, DVec3::ZERO);
    let mut camera = CameraEntity::default();
    camera.set_camera(PIG, OWN, true);
    camera.refresh(&store);
    // The default box's 1.62 without the recorder's dump; a baby halves it (the age scale).
    assert_eq!(camera.snapshot().unwrap().eye_height, 1.62);
    store.set_metadata(
        PIG,
        &[MetadataEntry {
            index: haste_entity::registry::BABY_METADATA_INDEX,
            value: MetadataValue::Boolean(true),
        }],
    );
    camera.refresh(&store);
    assert_eq!(camera.snapshot().unwrap().eye_height, 0.81);
}

#[test]
fn a_removed_camera_entity_keeps_its_last_pose_until_the_next_set_camera() {
    let mut store = store_with(PIG, DVec3::new(1.0, 64.0, 1.0));
    let mut camera = CameraEntity::default();
    camera.set_camera(PIG, OWN, true);
    camera.refresh(&store);
    let before = *camera.snapshot().unwrap();
    store.remove(&[PIG]);
    camera.refresh(&store);
    // Vanilla's camera keeps the removed `Entity` object: same pose, still the camera.
    assert_eq!(camera.snapshot(), Some(&before));
    // The server's answer (`set_camera` with our id) gives the camera back to the player.
    camera.set_camera(OWN, OWN, true);
    assert!(camera.snapshot().is_none());
}

#[test]
fn camera_at_lerps_the_position_the_eye_pair_and_the_view_rotation() {
    let s = CameraEntitySnapshot {
        id: PIG,
        prev_pos: DVec3::new(0.0, 64.0, 0.0),
        pos: DVec3::new(1.0, 64.0, 0.0),
        prev_rot: (170.0, 0.0),
        rot: (-170.0, 10.0),
        eye_height: 0.8,
        tilt: None,
    };
    let (eye, yaw, pitch) = s.camera_at(0.5, (1.62, 1.21));
    assert_eq!(
        eye,
        DVec3::new(0.5, 64.0 + f64::from(1.62f32 + (1.21 - 1.62) * 0.5), 0.0)
    );
    // `Mth.rotLerp` goes the short way across +-180.
    assert_eq!(yaw, 180.0);
    assert_eq!(pitch, 5.0);
    // At 1 the view rotation is the current one exactly (`getViewYRot`'s `a == 1` branch).
    let (eye, yaw, pitch) = s.camera_at(1.0, (1.62, 1.21));
    assert_eq!((yaw, pitch), (-170.0, 10.0));
    assert_eq!(eye, DVec3::new(1.0, 64.0 + f64::from(1.21f32), 0.0));
    // The pick starts at the entity's own eye height, not the camera's smoothed one.
    assert_eq!(
        s.eye_position(),
        DVec3::new(1.0, 64.0 + f64::from(0.8f32), 0.0)
    );
}

#[test]
fn the_camera_pose_is_the_players_until_another_entity_is_the_camera() {
    let player = crate::player::LocalPlayer::with_entity(OWN, GameMode::Spectator);
    let mut camera = CameraEntity::default();
    assert_eq!(camera_pose(&camera, &player, 1.0), player.camera(1.0));
    let store = store_with(PIG, DVec3::new(10.0, 70.0, -4.0));
    camera.set_camera(PIG, OWN, true);
    camera.refresh(&store);
    let (eye, yaw, _) = camera_pose(&camera, &player, 1.0);
    // The pig's position, raised by the camera's own (still the player's) eye height pair.
    let eye_heights = player.camera_eye_heights();
    assert_eq!(eye, DVec3::new(10.0, 70.0 + f64::from(eye_heights.1), -4.0));
    assert_eq!(yaw, 30.0);
}

#[test]
fn the_world_hides_the_camera_entity_only_in_first_person() {
    let local = OWN;
    // The player is the camera: hidden in first person, drawn in third person.
    assert_eq!(hidden_camera_entity(None, true, local), Some(local));
    assert_eq!(hidden_camera_entity(None, false, local), None);
    // Spectating: the spectated entity is hidden in first person, drawn when detached.
    let view = CameraEntityView {
        entity: CameraEntitySnapshot {
            id: PIG,
            prev_pos: DVec3::ZERO,
            pos: DVec3::ZERO,
            prev_rot: (0.0, 0.0),
            rot: (0.0, 0.0),
            eye_height: 0.8,
            tilt: None,
        },
        eye_heights: (0.8, 0.8),
    };
    assert_eq!(hidden_camera_entity(Some(&view), true, local), Some(PIG));
    assert_eq!(hidden_camera_entity(Some(&view), false, local), None);
}
