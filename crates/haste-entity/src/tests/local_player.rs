//! The local player's model state (`Entity::local_player` + `Entity::tick_local`): the client's
//! own player runs the same animation code a tracked remote player does, fed with exact state
//! instead of packets. Each test drives both paths with the same movement and compares.
use std::f32::consts::PI;

use haste_core::entity::{MetadataEntry, MetadataValue, PlayerInfo, Pose};
use haste_core::game::GameMode;
use haste_core::ids::EntityId;
use haste_core::DVec3;
use uuid::Uuid;

use crate::local_player::{LocalTick, MirroredSwing};
use crate::store::Entity;
use crate::{animation, models};

use super::{spawn, store};

/// A `LocalTick` standing still at the origin, looking straight ahead.
fn still_tick() -> LocalTick {
    LocalTick {
        prev_pos: DVec3::ZERO,
        pos: DVec3::ZERO,
        prev_look: (0.0, 0.0),
        look: (0.0, 0.0),
        pose: Pose::Standing,
        passenger: false,
        hurt_ticks: 0,
        dead: false,
        death_ticks: 0,
        swing: MirroredSwing {
            swinging: false,
            hand: 0,
            duration: 6,
            old: 0.0,
            progress: 0.0,
        },
    }
}

fn walk_tick(prev: DVec3, pos: DVec3) -> LocalTick {
    LocalTick {
        prev_pos: prev,
        pos,
        ..still_tick()
    }
}

/// The local player's leg/arm rotations while walking match a remote player's exactly (the
/// brief's step-1 test): ten ticks of 0.2 blocks/tick — the remote driven through the store's
/// packet path (a 0.6-block delta every three ticks lands as 0.2/tick through the lerp), the
/// local through its exact per-tick positions.
#[test]
fn local_player_walks_like_a_remote_player() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    let mut local = Entity::local_player(EntityId(99), Uuid::new_v4(), DVec3::ZERO, 0.0, 0.0);
    let model = models::humanoid(false);
    for i in 1..=10 {
        if matches!(i, 1 | 4 | 7 | 10) {
            s.move_delta(EntityId(1), DVec3::new(0.6, 0.0, 0.0), true);
        }
        s.tick();
        let pos = DVec3::new(0.2 * i as f64, 0.0, 0.0);
        local.tick_local(walk_tick(local.pos, pos));
        let remote = s.get(EntityId(1)).unwrap();
        let local_pose = animation::setup_anim(&local, &model, true, 1.0, false);
        let remote_pose = animation::setup_anim(remote, &model, true, 1.0, false);
        let a = super::rotations_by_name(&local_pose);
        let b = super::rotations_by_name(&remote_pose);
        for limb in ["right_arm", "left_arm", "right_leg", "left_leg"] {
            let (x, y) = (a.get(limb).unwrap(), b.get(limb).unwrap());
            assert!(
                (x.x - y.x).abs() < 2e-4,
                "tick {i} {limb} x: local {} != remote {}",
                x.x,
                y.x
            );
        }
        // After five ticks the local player also has vanilla's numbers (the remote test's
        // -0.36070: the recurrence `speed += (0.8 - speed) * 0.4; position += speed`).
        if i == 5 {
            let rl = a.get("right_leg").unwrap();
            let ll = a.get("left_leg").unwrap();
            assert!((rl.x - -0.360_702_3).abs() < 2e-3, "right leg {}", rl.x);
            assert!((ll.x - 0.360_702_3).abs() < 2e-3, "left leg {}", ll.x);
        }
    }
}

/// The local player's attack swing (mirrored from the client's hand) reaches the remote
/// player's arm angles: a 6-tick swing four ticks in (progress 1/3 -> 1/2) leaves the right
/// arm at the remote test's numbers (`HumanoidModel.setupAttackAnimation`, age 5).
#[test]
fn local_player_swing_matches_remote() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.swing(EntityId(1), 0, 1, 6);
    for _ in 0..4 {
        s.tick();
    }
    let mut local = Entity::local_player(EntityId(99), Uuid::new_v4(), DVec3::ZERO, 0.0, 0.0);
    for _ in 0..4 {
        let mut tick = still_tick();
        tick.swing = MirroredSwing {
            swinging: true,
            hand: 0,
            duration: 6,
            old: 1.0 / 3.0,
            progress: 0.5,
        };
        local.tick_local(tick);
    }
    let model = models::humanoid(false);
    let remote = s.get(EntityId(1)).unwrap();
    let local_pose = animation::setup_anim(&local, &model, true, 1.0, false);
    let remote_pose = animation::setup_anim(remote, &model, true, 1.0, false);
    let a = super::rotations_by_name(&local_pose);
    let b = super::rotations_by_name(&remote_pose);
    let (x, y) = (a.get("right_arm").unwrap(), b.get("right_arm").unwrap());
    for (name, lv, rv) in [("x", x.x, y.x), ("y", x.y, y.y), ("z", x.z, y.z)] {
        assert!(
            (lv - rv).abs() < 2e-4,
            "right arm {name}: local {lv} != remote {rv}"
        );
    }
    assert!((x.x - -0.742_670).abs() < 2e-3, "arm x {}", x.x);
}

/// Crouching through the ticked state matches a remote crouching player's flags: the body
/// tilts 0.5, the arms raise 0.4, and the head follows the look's pitch (the old hand-written
/// pose froze it).
#[test]
fn local_player_crouch_follows_the_look() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: 0,
            value: MetadataValue::Byte(0x02),
        }],
    );
    s.tick();
    let mut local = Entity::local_player(EntityId(99), Uuid::new_v4(), DVec3::ZERO, 0.0, 0.0);
    let mut tick = walk_tick(DVec3::ZERO, DVec3::ZERO);
    tick.pose = Pose::Crouching;
    tick.look = (0.0, -30.0);
    tick.prev_look = (0.0, -30.0);
    local.tick_local(tick);
    let model = models::humanoid(false);
    let remote = s.get(EntityId(1)).unwrap();
    let local_pose = animation::setup_anim(&local, &model, true, 1.0, false);
    let remote_pose = animation::setup_anim(remote, &model, true, 1.0, false);
    let a = super::rotations_by_name(&local_pose);
    let b = super::rotations_by_name(&remote_pose);
    // The remote crouch has no look pitch; compare the crouch-driven parts then the head.
    let (body_l, body_r) = (a.get("body").unwrap(), b.get("body").unwrap());
    assert!((body_l.x - 0.5).abs() < 1e-6);
    assert!((body_l.x - body_r.x).abs() < 1e-6);
    let (arm_l, arm_r) = (a.get("right_arm").unwrap(), b.get("right_arm").unwrap());
    assert!((arm_l.x - arm_r.x).abs() < 1e-6);
    assert!((arm_l.x - 0.4).abs() < 0.06, "arm x {}", arm_l.x);
    // The head follows the look (pitch -30 degrees = looking up).
    let head = a.get("head").unwrap();
    assert!(
        (head.x - (-30.0f32).to_radians()).abs() < 1e-5,
        "head {}",
        head.x
    );
    assert!(local.metadata.shared.crouching, "isDiscrete");
}

/// Swimming and elytra flight ramp like a remote player's: `updateSwimAmount`'s 0.09 per tick
/// feeds `AvatarRenderer.setupRotations`' swim pitch (and the fall-flying counter its scale),
/// with the crawl's leg kick once fully in the water.
#[test]
fn local_player_swim_and_elytra_ramp_like_remote() {
    let model = models::humanoid(false);
    // A remote swimmer: the pose metadata drives the store's swim amount. It needs a
    // player-info entry (the swim body rotations are the player-only `AvatarRenderer`
    // `setupRotations`).
    let mut s = store();
    let sp = spawn(1, DVec3::ZERO);
    let uuid = sp.uuid;
    s.player_info(&[PlayerInfo {
        uuid,
        name: "Swimmer".into(),
        display_name: None,
        latency: 20,
        game_mode: Some(GameMode::Survival),
        listed: true,
        skin_texture_url: None,
    }]);
    s.spawn(&sp);
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: 6,
            value: MetadataValue::Pose(Pose::Swimming),
        }],
    );
    let mut local = Entity::local_player(EntityId(99), Uuid::new_v4(), DVec3::ZERO, 0.0, 0.0);
    let mut tick = walk_tick(DVec3::ZERO, DVec3::ZERO);
    tick.pose = Pose::Swimming;
    for _ in 0..12 {
        s.tick();
        local.tick_local(tick);
    }
    let remote = s.get(EntityId(1)).unwrap();
    assert!((local.swim_amount - remote.swim_amount).abs() < 1e-6);
    assert!((local.swim_amount - 1.0).abs() < 1e-6);
    let local_body = animation::body_transform(&local, 1.0, false);
    let remote_body = animation::body_transform(remote, 1.0, false);
    assert_eq!(local_body.ops[..2], remote_body.ops[..2]);
    // The crawl stroke owns the legs (`HumanoidModel.setupSwimAnimation`): standing still,
    // the legs settle on the kick values, not the walk's zero.
    let pose = animation::setup_anim(&local, &model, true, 1.0, false);
    let rot = super::rotations_by_name(&pose);
    let rl = rot.get("right_leg").unwrap();
    let ll = rot.get("left_leg").unwrap();
    assert!((rl.x - 0.3).abs() < 1e-5, "right leg {}", rl.x);
    assert!((ll.x - 0.3 * (PI).cos()).abs() < 1e-5, "left leg {}", ll.x);

    // Elytra flight: the fall-flying counter ramps the whole-model pitch (fully tipped after
    // 10 ticks), and the legs follow the body (they are not left standing).
    let mut local = Entity::local_player(EntityId(99), Uuid::new_v4(), DVec3::ZERO, 0.0, 0.0);
    let mut tick = walk_tick(DVec3::ZERO, DVec3::ZERO);
    tick.pose = Pose::FallFlying;
    for _ in 0..10 {
        local.tick_local(tick);
    }
    assert_eq!(local.fall_flying_ticks, 10);
    let body = animation::body_transform(&local, 1.0, false);
    let rot = body
        .iter()
        .find(|op| matches!(op, haste_core::model::BodyOp::RotateX(_)))
        .copied();
    match rot {
        Some(haste_core::model::BodyOp::RotateX(v)) => {
            assert!((v - (-PI / 2.0)).abs() < 1e-4, "body tipped {v}");
        }
        _ => panic!("no fall-flying body rotation"),
    }
    // The head tips to vanilla's -pi/4 while gliding (`HumanoidModel.setupAnim`).
    let pose = animation::setup_anim(&local, &model, true, 1.0, false);
    let rot = super::rotations_by_name(&pose);
    let head = rot.get("head").unwrap();
    assert!((head.x - (-PI / 4.0)).abs() < 1e-5, "head {}", head.x);
}
