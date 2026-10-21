//! Look-at tests: `Mth.wrapDegrees`, the `Mth.atan2` table against the exact angle, `Entity.lookAt`
//! angles, and the `player_look_at` packet against the real `EntityStore`.

use std::sync::Arc;

use glam::DVec3;
use haste_core::entity::EntitySpawn;
use haste_core::{EntityId, RegistryIndex};
use haste_protocol::haste_core::game::{DimensionInfo, GameMode};
use haste_protocol::packets::play::clientbound as cb;
use haste_protocol::ClientboundPlay;

use super::atan2::atan2;
use super::rotation::{look_angles, wrap_degrees};
use crate::net::{new_entity_store, Session};

/// The local player's entity id in these tests.
const OWN: i32 = 7;

fn session() -> Session {
    Session::new(
        EntityId(OWN),
        GameMode::Creative,
        DimensionInfo::overworld(),
        Arc::new(Vec::new()),
    )
}

fn look(from_anchor: i32, target: DVec3, entity: Option<(i32, i32)>) -> ClientboundPlay {
    ClientboundPlay::PlayerLookAt(cb::PlayerLookAt {
        from_anchor,
        target,
        entity: entity.map(|(entity_id, anchor)| cb::LookAtEntity { entity_id, anchor }),
    })
}

#[test]
fn wrap_degrees_keeps_the_half_open_range() {
    assert_eq!(wrap_degrees(0.0), 0.0);
    assert_eq!(wrap_degrees(190.0), -170.0);
    assert_eq!(wrap_degrees(-190.0), 170.0);
    assert_eq!(wrap_degrees(360.0), 0.0);
    assert_eq!(wrap_degrees(-90.0), -90.0);
}

#[test]
fn atan2_table_matches_the_exact_angle() {
    // The 1/256 table with its series correction is good to a few millionths of a radian (vanilla's
    // own accuracy: 1.4e-6 at 5 degrees), so the check allows 1e-5.
    for step in 0..72 {
        let a = f64::from(step) * 5.0_f64.to_radians();
        let (y, x) = (3.0 * a.sin(), 3.0 * a.cos());
        let exact = y.atan2(x);
        assert!(
            (atan2(y, x) - exact).abs() < 1e-5,
            "atan2({y}, {x}) = {} vs {exact}",
            atan2(y, x)
        );
    }
}

#[test]
fn atan2_quadrants() {
    assert!(atan2(0.0, 1.0).abs() < 1e-9);
    assert!((atan2(1.0, 0.0) - std::f64::consts::FRAC_PI_2).abs() < 1e-6);
    assert!((atan2(-1.0, -1.0) + 3.0 * std::f64::consts::FRAC_PI_4).abs() < 1e-6);
    assert!(atan2(f64::NAN, 1.0).is_nan());
}

#[test]
fn look_angles_face_the_axes() {
    let origin = DVec3::ZERO;
    // Facing +z is yaw 0; facing +x is yaw -90 (Minecraft's south is +z).
    let (yaw, pitch) = look_angles(origin, DVec3::new(0.0, 0.0, 1.0));
    assert!(yaw.abs() < 1e-3 && pitch.abs() < 1e-3, "({yaw}, {pitch})");
    let (yaw, pitch) = look_angles(origin, DVec3::new(1.0, 0.0, 0.0));
    assert!(
        (yaw + 90.0).abs() < 1e-3 && pitch.abs() < 1e-3,
        "({yaw}, {pitch})"
    );
    let (yaw, pitch) = look_angles(origin, DVec3::new(0.0, 1.0, 0.0));
    assert!(
        (yaw + 90.0).abs() < 1e-3 && (pitch + 90.0).abs() < 1e-3,
        "({yaw}, {pitch})"
    );
}

#[test]
fn look_angles_pitch_is_negative_upwards() {
    // Up and to +x: pitch is negative (the view tilts up), yaw is -90 for +x.
    let (yaw, pitch) = look_angles(DVec3::ZERO, DVec3::new(1.0, 1.0, 0.0));
    assert!((yaw + 90.0).abs() < 1e-3, "yaw {yaw}");
    assert!((pitch + 45.0).abs() < 1e-3, "pitch {pitch}");
}

#[test]
fn packet_turns_the_player_to_an_entity_feet() {
    let mut s = session();
    s.player.state.pos = DVec3::new(0.0, 64.0, 0.0);
    let mut store = new_entity_store(8);
    store.spawn(&EntitySpawn {
        id: EntityId(5),
        uuid: Default::default(),
        kind: RegistryIndex(0),
        pos: DVec3::new(10.0, 64.0, -3.0),
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 0.0,
        data: 0,
        velocity: DVec3::ZERO,
    });
    // From our eyes (standing, 1.62 up) to the zombie's feet: yaw atan2(-3, 10) - 90 and the
    // pitch of the 1.62 drop over the 10.44 run.
    let rest = s.apply_look_at(look(1, DVec3::ZERO, Some((5, 0))), &store);
    assert!(rest.is_none());
    assert!(
        (s.player.state.yaw - -106.699).abs() < 0.01,
        "yaw {}",
        s.player.state.yaw
    );
    assert!(
        (s.player.state.pitch - 8.821).abs() < 0.01,
        "pitch {}",
        s.player.state.pitch
    );
}

#[test]
fn packet_to_an_unknown_entity_turns_to_the_point() {
    let mut s = session();
    s.player.state.pos = DVec3::new(0.0, 64.0, 0.0);
    let store = new_entity_store(8);
    // The entity is not in the store, so the target point is used: from our eyes (65.62) down
    // 1.62 over the 5 blocks to (0, 64, 5), a pitch of atan(1.62 / 5) and yaw 0.
    let rest = s.apply_look_at(look(1, DVec3::new(0.0, 64.0, 5.0), Some((9, 0))), &store);
    assert!(rest.is_none());
    assert!(
        s.player.state.yaw.abs() < 1e-3,
        "yaw {}",
        s.player.state.yaw
    );
    assert!(
        (s.player.state.pitch - 17.952).abs() < 0.05,
        "pitch {}",
        s.player.state.pitch
    );
}

#[test]
fn other_packets_pass_through() {
    let mut s = session();
    let store = new_entity_store(8);
    let p = ClientboundPlay::PlayerRotation(cb::PlayerRotation {
        yaw: 1.0,
        pitch: 2.0,
        relative_yaw: false,
        relative_pitch: false,
    });
    assert!(matches!(
        s.apply_look_at(p, &store),
        Some(ClientboundPlay::PlayerRotation(_))
    ));
}
