use haste_physics::testing::{states, TestShapes, TestWorld};
use haste_protocol::packets::play::clientbound::{AttributeModifier, AttributeProperty};

use super::*;

const ME: EntityId = EntityId(7);
/// Flat stone floor whose top is y = 0.
fn world() -> TestWorld {
    TestWorld::flat(0, states::STONE)
}

fn ids() -> AttributeIds {
    AttributeIds {
        movement_speed: 26,
        jump_strength: 19,
        gravity: 18,
        step_height: 33,
        scale: 30,
        sneaking_speed: 31,
        block_break_speed: 7,
        mining_efficiency: 24,
        submerged_mining_speed: 34,
        block_interaction_range: 8,
        entity_interaction_range: 13,
        attack_speed: 3,
    }
}

fn teleport(id: i32, pos: DVec3, yaw: f32, flags: i32) -> ClientboundPlay {
    ClientboundPlay::PlayerPosition(cb::PlayerPosition {
        teleport_id: id,
        pos,
        velocity: DVec3::ZERO,
        yaw,
        pitch: 0.0,
        flags,
    })
}

/// A survival player standing on the floor at (0.5, 0, 0.5), loaded, after its spawn accept.
fn spawned() -> (LocalPlayer, Vec<ServerboundPlay>) {
    let mut p = LocalPlayer::with_entity(ME, GameMode::Survival);
    p.set_attribute_ids(ids());
    let mut out = Vec::new();
    assert!(p.handle(&teleport(1, DVec3::new(0.5, 0.0, 0.5), 0.0, 0), &mut out));
    // The accept waits one full tick after the teleport, not sent from the packet handler.
    tick(&mut p, &InputState::default());
    assert_eq!(
        names(&tick(&mut p, &InputState::default())),
        ["accept", "loaded", "input", "status", "tick_end"]
    );
    (p, out)
}

fn tick(p: &mut LocalPlayer, input: &InputState) -> Vec<ServerboundPlay> {
    let mut out = Vec::new();
    p.tick_with(
        input,
        &world(),
        &TestShapes::default(),
        &TestShapes::default(),
        &mut out,
    );
    out
}

fn names(out: &[ServerboundPlay]) -> Vec<&'static str> {
    out.iter()
        .map(|p| match p {
            ServerboundPlay::PlayerInput(_) => "input",
            ServerboundPlay::PlayerCommand(_) => "command",
            ServerboundPlay::PlayerAbilities(_) => "abilities",
            ServerboundPlay::MovePlayerPos(_) => "pos",
            ServerboundPlay::MovePlayerPosRot(_) => "pos_rot",
            ServerboundPlay::MovePlayerRot(_) => "rot",
            ServerboundPlay::MovePlayerStatusOnly(_) => "status",
            ServerboundPlay::ClientTickEnd(_) => "tick_end",
            ServerboundPlay::AcceptTeleportation(_) => "accept",
            ServerboundPlay::PlayerLoaded(_) => "loaded",
            _ => "other",
        })
        .collect()
}

#[test]
fn first_teleport_accepts_one_tick_later_then_reports_loaded() {
    let mut p = LocalPlayer::with_entity(ME, GameMode::Survival);
    let mut out = Vec::new();
    // Nothing is sent before a position arrives.
    p.tick_with(
        &InputState::default(),
        &world(),
        &TestShapes::default(),
        &TestShapes::default(),
        &mut out,
    );
    assert!(out.is_empty());
    // The teleport is applied at once, but the accept is not sent from the packet
    // handler: answering within microseconds of the join teleport races the server's
    // join bookkeeping ("moved too quickly! <spawn position>"). It waits one full tick.
    p.handle(&teleport(5, DVec3::new(1.5, 2.0, -3.5), 90.0, 0), &mut out);
    assert!(out.is_empty());
    assert!(p.has_position && !p.loaded);
    // The tick the teleport arrived in sends nothing (movement waits for player_loaded).
    assert!(tick(&mut p, &InputState::default()).is_empty());
    assert!(!p.loaded);
    let out = tick(&mut p, &InputState::default());
    assert_eq!(
        names(&out),
        ["accept", "loaded", "input", "status", "tick_end"]
    );
    match &out[0] {
        ServerboundPlay::AcceptTeleportation(a) => {
            assert_eq!(a.teleport_id, 5);
            assert_eq!(a.pos, DVec3::new(1.5, 2.0, -3.5));
            assert_eq!(a.yaw, 90.0);
        }
        _ => unreachable!(),
    }
    assert!(p.loaded && p.has_position);
}

#[test]
fn later_teleport_is_confirmed_by_the_accept_alone() {
    let (mut p, mut out) = spawned();
    p.handle(
        &teleport(
            2,
            DVec3::new(1.0, 0.0, 1.0),
            10.0,
            teleport_flags::X | teleport_flags::Y | teleport_flags::Z | teleport_flags::YAW,
        ),
        &mut out,
    );
    // Nothing inline; the accept carries the resulting position one full tick later.
    assert!(out.is_empty());
    assert_eq!(names(&tick(&mut p, &InputState::default())), ["tick_end"]);
    assert_eq!(
        names(&tick(&mut p, &InputState::default()))[..1],
        ["accept"]
    );
    assert_eq!(p.state.pos, DVec3::new(1.5, 0.0, 1.5));
    assert_eq!(p.state.yaw, 10.0);
}

#[test]
fn tick_order_and_input_only_on_change() {
    let (mut p, _) = spawned();
    let still = InputState::default();
    let out = tick(&mut p, &still);
    // The spawn tick already sent input; standing still with zero velocity the move does
    // not collide downwards, so on_ground drops for one more tick (status-only packet).
    assert_eq!(names(&out), ["status", "tick_end"]);
    assert_eq!(names(&tick(&mut p, &still)), ["tick_end"]);
    let walk = InputState {
        forward: true,
        ..still
    };
    let out = tick(&mut p, &walk);
    assert_eq!(names(&out), ["input", "pos", "tick_end"]);
    let out = tick(&mut p, &walk);
    assert_eq!(names(&out), ["pos", "tick_end"]);
}

#[test]
fn position_reminder_every_twenty_ticks() {
    let (mut p, _) = spawned();
    let still = InputState::default();
    let mut sent_at = Vec::new();
    for t in 1..=45 {
        if names(&tick(&mut p, &still)).contains(&"pos") {
            sent_at.push(t);
        }
    }
    // The spawn teleport counts as a sent position, and `spawned()` spent one tick of it.
    assert_eq!(sent_at, [19, 39]);
}

#[test]
fn rotation_alone_sends_rot_and_server_rotation_is_an_offset() {
    let (mut p, _) = spawned();
    tick(&mut p, &InputState::default());
    let look = InputState {
        yaw: 30.0,
        ..Default::default()
    };
    assert_eq!(names(&tick(&mut p, &look)), ["rot", "tick_end"]);
    assert_eq!(p.state.yaw, 30.0);
    // The server turns us to 90; the mouse still says 30.
    let mut out = Vec::new();
    p.handle(
        &ClientboundPlay::PlayerRotation(cb::PlayerRotation {
            yaw: 90.0,
            relative_yaw: false,
            pitch: 0.0,
            relative_pitch: false,
        }),
        &mut out,
    );
    assert_eq!(p.look_offset(), (60.0, 0.0));
    tick(&mut p, &look);
    assert_eq!(p.state.yaw, 90.0);
    let more = InputState { yaw: 35.0, ..look };
    tick(&mut p, &more);
    assert_eq!(p.state.yaw, 95.0);
}

#[test]
fn sprint_command_comes_before_the_move() {
    let (mut p, _) = spawned();
    let run = InputState {
        forward: true,
        sprint: true,
        ..Default::default()
    };
    let out = tick(&mut p, &run);
    assert_eq!(names(&out), ["input", "command", "pos", "tick_end"]);
    match &out[1] {
        ServerboundPlay::PlayerCommand(c) => {
            assert_eq!((c.entity_id, c.action), (ME.0, command::START_SPRINTING))
        }
        _ => unreachable!(),
    }
    let stop = InputState::default();
    let out = tick(&mut p, &stop);
    assert_eq!(names(&out)[..2], ["input", "command"]);
}

/// The `player_command` action ids this module sends, pinned to the declaration
/// order of `ServerboundPlayerCommandPacket$Action` in the 26.3 client jar
/// (STOP_SLEEPING=0, START_SPRINTING=1, STOP_SPRINTING=2, START_RIDING_JUMP=3,
/// STOP_RIDING_JUMP=4, OPEN_INVENTORY=5, START_FALL_FLYING=6).
#[test]
fn player_command_action_values_match_the_jar() {
    assert_eq!(command::START_SPRINTING, 1);
    assert_eq!(command::STOP_SPRINTING, 2);
}

/// The first movement packet after the join teleport carries the server's position, so the
/// server-side delta is ~0 (regression test for the join "moved too quickly!" whose logged
/// delta equalled the whole spawn position).
#[test]
fn first_move_after_join_carries_the_server_position() {
    // A floor whose top is the spawn y, so physics leaves the player standing in place.
    let mut p = LocalPlayer::with_entity(ME, GameMode::Survival);
    p.set_attribute_ids(ids());
    let mut out = Vec::new();
    let spawn = DVec3::new(7.5, -60.0, -3.5);
    assert!(p.handle(&teleport(1, spawn, 0.0, 0), &mut out));
    assert!(out.is_empty());
    let floor = TestWorld::flat(-60, states::STONE);
    // The tick the teleport arrived in: the accept is held for one full tick.
    let mut out = Vec::new();
    p.tick_with(
        &InputState::default(),
        &floor,
        &TestShapes::default(),
        &TestShapes::default(),
        &mut out,
    );
    assert!(out.is_empty());
    p.tick_with(
        &InputState::default(),
        &floor,
        &TestShapes::default(),
        &TestShapes::default(),
        &mut out,
    );
    // accept (with the server's position) + player_loaded, then the per-tick packets.
    let names = names(&out);
    assert_eq!(names[..3], ["accept", "loaded", "input"]);
    match &out[0] {
        ServerboundPlay::AcceptTeleportation(a) => {
            assert_eq!(a.teleport_id, 1);
            assert_eq!(a.pos, spawn);
        }
        _ => unreachable!(),
    }
    // The first movement packet, when it comes, is at the server's position (delta ~0).
    let first_move = out.iter().find(|p| {
        matches!(
            p,
            ServerboundPlay::MovePlayerPos(_) | ServerboundPlay::MovePlayerPosRot(_)
        )
    });
    if let Some(m) = first_move {
        let pos = match m {
            ServerboundPlay::MovePlayerPos(m) => m.pos,
            ServerboundPlay::MovePlayerPosRot(m) => m.pos,
            _ => unreachable!(),
        };
        assert!(
            (pos - spawn).length_squared() < POSITION_THRESHOLD_SQ,
            "move packet position {pos:?} not at the server's position {spawn:?}"
        );
    }
}

#[test]
fn no_sprint_when_hungry() {
    let (mut p, _) = spawned();
    p.food = 6;
    let run = InputState {
        forward: true,
        sprint: true,
        ..Default::default()
    };
    let out = tick(&mut p, &run);
    assert!(!names(&out).contains(&"command"));
    assert!(!p.state.sprinting);
}

#[test]
fn falling_sends_on_ground_flag_on_landing() {
    let (mut p, mut out) = spawned();
    p.handle(&teleport(2, DVec3::new(0.5, 3.0, 0.5), 0.0, 0), &mut out);
    let still = InputState::default();
    let mut landed = None;
    for _ in 0..40 {
        for pk in tick(&mut p, &still) {
            if let ServerboundPlay::MovePlayerPos(m) = pk {
                if m.flags & move_flags::ON_GROUND != 0 {
                    landed = Some(m.pos.y);
                }
            }
        }
        if landed.is_some() {
            break;
        }
    }
    assert_eq!(landed, Some(0.0));
}

#[test]
fn creative_flight_toggle_sends_abilities() {
    let mut p = LocalPlayer::with_entity(ME, GameMode::Creative);
    let mut out = Vec::new();
    p.handle(&teleport(1, DVec3::new(0.5, 0.0, 0.5), 0.0, 0), &mut out);
    p.toggle_flight();
    // The tick the teleport arrived in waits (no accept yet); the next sends the spawn
    // accept, player_loaded, then the abilities.
    assert!(tick(&mut p, &InputState::default()).is_empty());
    let out = tick(&mut p, &InputState::default());
    assert_eq!(names(&out)[..4], ["accept", "loaded", "input", "abilities"]);
    assert!(p.state.flying);
    // Survival cannot fly: toggling does nothing.
    let (mut s, _) = spawned();
    s.toggle_flight();
    assert!(!names(&tick(&mut s, &InputState::default())).contains(&"abilities"));
}

#[test]
fn abilities_packet_sets_flying_and_speed() {
    let (mut p, mut out) = spawned();
    p.handle(
        &ClientboundPlay::PlayerAbilities(cb::PlayerAbilities {
            flags: ability_flags::MAY_FLY | ability_flags::FLYING,
            flying_speed: 0.1,
            fov_modifier: 0.1,
        }),
        &mut out,
    );
    assert!(p.state.flying && p.abilities.may_fly);
    assert_eq!(p.attrs.flying_speed, 0.1);
}

fn modifier(path: &str, amount: f64, operation: u8) -> AttributeModifier {
    AttributeModifier {
        id: Identifier::minecraft(path),
        amount,
        operation,
    }
}

#[test]
fn attributes_apply_and_ignore_the_sprint_modifier() {
    let (mut p, mut out) = spawned();
    p.handle(
        &ClientboundPlay::UpdateAttributes(cb::UpdateAttributes {
            entity_id: ME.0,
            properties: vec![
                AttributeProperty {
                    id: 26,
                    value: 0.1,
                    modifiers: vec![
                        modifier("sprinting", 0.3, 2),
                        modifier("effect.speed", 0.2, 2),
                    ],
                },
                AttributeProperty {
                    id: 30,
                    value: 1.0,
                    modifiers: vec![modifier("test", 1.0, 0)],
                },
                AttributeProperty {
                    id: 33,
                    value: 1.0,
                    modifiers: vec![],
                },
            ],
        }),
        &mut out,
    );
    // Another entity's attributes are not ours.
    assert!(!p.handle(
        &ClientboundPlay::UpdateAttributes(cb::UpdateAttributes {
            entity_id: 99,
            properties: vec![],
        }),
        &mut out,
    ));
    tick(&mut p, &InputState::default());
    assert!((p.attrs.movement_speed - 0.12).abs() < 1e-12);
    assert_eq!(p.attrs.scale, 2.0);
    assert_eq!(p.attrs.step_height, 1.0);
    assert_eq!(p.attrs.box_width(), 1.2000000476837158);
}

#[test]
fn attribute_operations() {
    let p = AttributeProperty {
        id: 0,
        value: 2.0,
        modifiers: vec![
            modifier("a", 1.0, 0),
            modifier("b", 0.5, 1),
            modifier("c", 1.0, 2),
        ],
    };
    // (2 + 1) = 3; + 3 x 0.5 = 4.5; x 2 = 9.
    assert_eq!(attribute_value(&p, |_| false), 9.0);
}

#[test]
fn own_motion_sets_velocity() {
    let (mut p, mut out) = spawned();
    assert!(p.handle(
        &ClientboundPlay::SetEntityMotion(cb::SetEntityMotion {
            entity_id: ME.0,
            velocity: DVec3::new(0.0, 0.5, 0.0),
        }),
        &mut out,
    ));
    assert_eq!(p.state.velocity.y, 0.5);
    tick(&mut p, &InputState::default());
    assert!(p.state.pos.y > 0.4);
}

#[test]
fn camera_interpolates_and_adds_eye_height() {
    let (mut p, _) = spawned();
    let walk = InputState {
        forward: true,
        ..Default::default()
    };
    tick(&mut p, &walk);
    let (eye0, _, _) = p.camera(0.0);
    let (eye1, _, _) = p.camera(1.0);
    assert!((eye1.y - (p.state.pos.y + 1.6200000047683716)).abs() < 1e-12);
    assert!(eye1.z > eye0.z, "yaw 0 walks towards +z");
}

#[test]
fn respawn_waits_for_a_new_position() {
    let (mut p, mut out) = spawned();
    let spawn = haste_protocol::packets::play::clientbound::SpawnInfo {
        dimension_type: 0,
        dimension: Identifier::minecraft("overworld"),
        hashed_seed: 0,
        game_mode: 1,
        previous_game_mode: 0,
        is_debug: false,
        is_flat: true,
        death_location: None,
        portal_cooldown: 0,
        sea_level: 63,
    };
    assert!(p.handle(
        &ClientboundPlay::Respawn(cb::Respawn {
            spawn,
            data_kept: 0
        }),
        &mut out,
    ));
    assert!(!p.loaded && !p.has_position);
    assert_eq!(p.game_mode, GameMode::Creative);
    assert!(tick(&mut p, &InputState::default()).is_empty());
    p.handle(&teleport(9, DVec3::new(0.5, 0.0, 0.5), 0.0, 0), &mut out);
    // The respawn teleport's accept and the new player_loaded go out one full tick later.
    assert!(!p.loaded);
    assert!(tick(&mut p, &InputState::default()).is_empty());
    assert_eq!(
        names(&tick(&mut p, &InputState::default()))[..2],
        ["accept", "loaded"]
    );
    assert!(p.loaded);
}
