use haste_core::{BlockPos, BlockStateId, ChunkPos};
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

/// A creative player standing on the floor at (0.5, 0, 0.5), loaded, after its spawn accept.
fn spawned_creative() -> LocalPlayer {
    let mut p = LocalPlayer::with_entity(ME, GameMode::Creative);
    p.set_attribute_ids(ids());
    let mut out = Vec::new();
    assert!(p.handle(&teleport(1, DVec3::new(0.5, 0.0, 0.5), 0.0, 0), &mut out));
    tick(&mut p, &InputState::default());
    tick(&mut p, &InputState::default());
    p
}

/// The same floor as [`world`], but every chunk reports as not loaded.
struct Unloaded {
    floor: TestWorld,
    min_y: i32,
}

impl Unloaded {
    fn new(min_y: i32) -> Self {
        Self {
            floor: TestWorld::flat(0, states::STONE),
            min_y,
        }
    }
}

impl BlockView for Unloaded {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.floor.block_state(pos)
    }
    fn is_chunk_loaded(&self, _pos: ChunkPos) -> bool {
        false
    }
    fn sky_light(&self, pos: BlockPos) -> u8 {
        self.floor.sky_light(pos)
    }
    fn block_light(&self, pos: BlockPos) -> u8 {
        self.floor.block_light(pos)
    }
    fn min_y(&self) -> i32 {
        self.min_y
    }
    fn height(&self) -> i32 {
        self.floor.height()
    }
}

fn tick(p: &mut LocalPlayer, input: &InputState) -> Vec<ServerboundPlay> {
    tick_riding(p, input, None)
}

/// One tick that knows where the vehicle we ride is (the passenger tick follows it).
fn tick_riding(
    p: &mut LocalPlayer,
    input: &InputState,
    vehicle: Option<VehiclePose>,
) -> Vec<ServerboundPlay> {
    let mut out = Vec::new();
    p.tick_with(
        input,
        &world(),
        &TestShapes::default(),
        &TestShapes::default(),
        vehicle,
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
            ServerboundPlay::PaddleBoat(_) => "paddle",
            ServerboundPlay::MoveVehicle(_) => "move_vehicle",
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
        None,
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
        None,
        &mut out,
    );
    assert!(out.is_empty());
    p.tick_with(
        &InputState::default(),
        &floor,
        &TestShapes::default(),
        &TestShapes::default(),
        None,
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

/// Vanilla `LocalPlayer.aiStep` toggles creative flight on the second press of a double tap of
/// jump inside a 7-tick window (`jumpTriggerTime`, counted down by `Player.aiStep`). The opposite
/// sprint double tap uses `options.sprintWindow` (default 7 too, `Options.java`), which haste does
/// not have: sprinting starts from the sprint key, matching vanilla's `keyPresses.sprint()` branch.
#[test]
fn creative_flight_needs_a_double_tap_within_seven_ticks() {
    assert_eq!(JUMP_DOUBLE_TAP_TICKS, 7);
    let mut p = spawned_creative();
    // The first press arms the window only; no flight, no abilities packet.
    p.toggle_flight();
    let out = tick(&mut p, &InputState::default());
    assert!(!names(&out).contains(&"abilities") && !p.state.flying);
    // The second one inside the window turns flight on and reports the new ability.
    p.toggle_flight();
    let out = tick(&mut p, &InputState::default());
    assert!(names(&out).contains(&"abilities"));
    assert!(p.state.flying);
    // Turning it off again is the same double tap.
    p.toggle_flight();
    tick(&mut p, &InputState::default());
    p.toggle_flight();
    let out = tick(&mut p, &InputState::default());
    assert!(names(&out).contains(&"abilities"));
    assert!(!p.state.flying);
    // Survival cannot fly: the same double tap does nothing.
    let (mut s, _) = spawned();
    s.toggle_flight();
    tick(&mut s, &InputState::default());
    s.toggle_flight();
    let out = tick(&mut s, &InputState::default());
    assert!(!names(&out).contains(&"abilities") && !s.state.flying);
}

#[test]
fn the_double_tap_window_runs_out_after_seven_ticks() {
    let press = InputState {
        jump: true,
        ..Default::default()
    };
    // A second press 5 ticks after the first is still inside the window; 6 ticks later the
    // counter has reached zero and that press only arms it again.
    for (idle, toggles) in [(5u32, true), (6, false)] {
        let mut p = spawned_creative();
        tick(&mut p, &press);
        for _ in 0..idle {
            tick(&mut p, &InputState::default());
        }
        let out = tick(&mut p, &press);
        assert_eq!(p.state.flying, toggles, "idle {idle} ticks");
        assert_eq!(names(&out).contains(&"abilities"), toggles, "idle {idle}");
    }
}

/// Vanilla's toggle also calls `LivingEntity.jumpFromGround` when flight turns on while the
/// player stands on the ground: the vertical movement becomes the jump strength (0.42).
#[test]
fn flight_turning_on_on_the_ground_jumps() {
    let mut p = spawned_creative();
    // One tick to settle on the floor after the spawn teleport.
    tick(&mut p, &InputState::default());
    assert!(p.state.on_ground);
    p.toggle_flight();
    tick(&mut p, &InputState::default());
    p.toggle_flight();
    let out = tick(&mut p, &InputState::default());
    assert!(names(&out).contains(&"abilities"));
    assert!(p.state.flying);
    assert!(!p.state.on_ground);
    // The tick after the jump carries the player up by the jump strength, exactly.
    assert!((p.state.pos.y - 0.42).abs() < 1e-12);
}

/// Vanilla `LocalPlayer.aiStep`: the sprint double tap of forward. A fresh press of forward
/// while the player may start sprinting arms `sprintTriggerTime` (`options.sprintWindow`, default
/// 7); a second fresh press inside the window sets sprinting, which the `player_command`
/// START_SPRINTING reports.
#[test]
fn sprint_double_tap_of_forward_starts_sprinting() {
    let run = InputState {
        forward: true,
        ..Default::default()
    };
    let (mut p, _) = spawned();
    // First press: arms the window only, no sprint yet.
    let out = tick(&mut p, &run);
    assert!(!names(&out).contains(&"command"));
    assert!(!p.state.sprinting);
    // Release, then a second press 2 ticks later: sprinting starts this tick.
    tick(&mut p, &InputState::default());
    tick(&mut p, &InputState::default());
    let out = tick(&mut p, &run);
    assert!(p.state.sprinting, "double tap starts sprinting");
    match out
        .iter()
        .find(|p| matches!(p, ServerboundPlay::PlayerCommand(_)))
    {
        Some(ServerboundPlay::PlayerCommand(c)) => {
            assert_eq!((c.entity_id, c.action), (ME.0, command::START_SPRINTING))
        }
        _ => panic!("no sprint command in {:?}", names(&out)),
    }
    // Holding forward across ticks does not re-arm or start anything on its own: the window
    // only sees fresh presses (vanilla reads `hasForwardImpulse` before `input.tick()`).
    let (mut q, _) = spawned();
    tick(&mut q, &run);
    tick(&mut q, &run);
    assert!(!q.state.sprinting, "a held key is not a double tap");
    // Backward cancels the window: a double tap around it does not sprint.
    let (mut b, _) = spawned();
    tick(&mut b, &run);
    tick(
        &mut b,
        &InputState {
            back: true,
            ..Default::default()
        },
    );
    let out = tick(&mut b, &run);
    assert!(!b.state.sprinting && !names(&out).contains(&"command"));
}

/// The sprint double-tap window runs out: a second press 6 idle ticks after the first only arms
/// it again (vanilla counts `sprintTriggerTime` down before the keys are looked at; the window
/// is 7, so the sixth idle tick leaves it at zero and that press only re-arms it).
#[test]
fn sprint_double_tap_window_runs_out_after_seven_ticks() {
    let run = InputState {
        forward: true,
        ..Default::default()
    };
    // 5 idle ticks between the presses: still inside; 6: outside.
    for (idle, sprints) in [(5u32, true), (6, false)] {
        let (mut p, _) = spawned();
        tick(&mut p, &run);
        for _ in 0..idle {
            tick(&mut p, &InputState::default());
        }
        tick(&mut p, &run);
        assert_eq!(p.state.sprinting, sprints, "idle {idle} ticks");
    }
}

/// Vanilla `LocalPlayer.aiStep`'s hunger gate (`isSprintingPossible` ->
/// `hasEnoughFoodToDoExhaustiveManoeuvres`, food > 6 or may-fly): a hungry player's double tap
/// does not start the sprint.
#[test]
fn no_sprint_double_tap_when_hungry() {
    let (mut p, _) = spawned();
    p.food = 6;
    let run = InputState {
        forward: true,
        ..Default::default()
    };
    tick(&mut p, &run);
    tick(&mut p, &InputState::default());
    let out = tick(&mut p, &run);
    assert!(!p.state.sprinting && !names(&out).contains(&"command"));
}

/// Vanilla `LocalPlayer.aiStep`: sneaking cancels the sprint window (`wasShiftKeyDown`).
#[test]
fn sneak_cancels_the_sprint_window() {
    let run = InputState {
        forward: true,
        ..Default::default()
    };
    let (mut p, _) = spawned();
    tick(&mut p, &run);
    tick(
        &mut p,
        &InputState {
            sneak: true,
            ..Default::default()
        },
    );
    let out = tick(&mut p, &run);
    assert!(!p.state.sprinting && !names(&out).contains(&"command"));
}

/// Vanilla `LivingEntity.travelInAir`: with the chunk below not loaded the client skips gravity
/// and pins the vertical movement to -0.1 (the 0.98 vertical drag on top of it) while the player
/// is above the world's minimum; at or below the minimum it is 0.0.
#[test]
fn unloaded_chunk_below_falls_at_vanillas_rate() {
    assert!((UNLOADED_CHUNK_FALL_SPEED + 0.098).abs() < 1.0e-8);
    let (mut p, _) = spawned();
    // One tick to settle on the floor after the spawn teleport.
    tick(&mut p, &InputState::default());
    assert!(p.state.on_ground);
    let w = Unloaded::new(-64);
    let mut out = Vec::new();
    p.tick_with(
        &InputState::default(),
        &w,
        &TestShapes::default(),
        &TestShapes::default(),
        None,
        &mut out,
    );
    assert_eq!(p.state.velocity.y, UNLOADED_CHUNK_FALL_SPEED);
    assert!(!p.state.on_ground);
    // Gravity stays skipped: the same step every tick, until the chunk is loaded again.
    for _ in 0..3 {
        p.tick_with(
            &InputState::default(),
            &w,
            &TestShapes::default(),
            &TestShapes::default(),
            None,
            &mut out,
        );
    }
    assert!((p.state.pos.y - 4.0 * UNLOADED_CHUNK_FALL_SPEED).abs() < 1.0e-12);

    // At or below the world's minimum the vertical movement is 0.0: the player stays put.
    let (mut p, mut out) = spawned();
    assert!(p.handle(&teleport(2, DVec3::new(0.5, -65.0, 0.5), 0.0, 0), &mut out));
    p.tick_with(
        &InputState::default(),
        &w,
        &TestShapes::default(),
        &TestShapes::default(),
        None,
        &mut out,
    );
    assert_eq!(p.state.velocity.y, 0.0);
    assert_eq!(p.state.pos.y, -65.0);
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

/// Vanilla `Camera.tick`: the camera halves its distance to the pose's eye height each tick,
/// and the pose only changes after the movement (`Player.updatePlayerPose` at the end of
/// `Player.tick`), so the eye height at the END of the tick shift is pressed in is still the
/// standing one, and the following ticks approach 1.27 (vanilla `Avatar.POSES` crouching eye
/// height) by halving: 1.62, 1.445, 1.3575, 1.31375, 1.291875, ... The values are the f32
/// arithmetic vanilla does (`eyeHeight += (getEyeHeight() - eyeHeight) * 0.5F`).
#[test]
fn camera_eye_height_halves_towards_the_crouching_eye_height() {
    let (mut p, _) = spawned();
    let shift = InputState {
        sneak: true,
        ..Default::default()
    };
    // t is the tick count since shift was pressed; the target seen by the camera pair is the
    // pose at the end of the previous tick, so the sequence starts one tick late.
    let mut expected_eye = 1.62f32;
    for t in 0..5 {
        tick(&mut p, &shift);
        if t > 0 {
            expected_eye += (1.27f32 - expected_eye) * 0.5;
        }
        assert_eq!(p.state.pose, haste_physics::Pose::Crouching);
        let (eye_end, _, _) = p.camera(1.0);
        // The camera pair brackets the smoothed value: start of the tick is the last one.
        assert_eq!(eye_end.y - p.state.pos.y, f64::from(expected_eye));
    }
    // Releasing shift mirrors it back towards 1.62, again one tick late: the first tick
    // after release still halves towards 1.27 (the pose was crouching when the camera
    // ticked), the second one towards 1.62.
    let stand = InputState::default();
    for t in 0..3 {
        tick(&mut p, &stand);
        let target = if t == 0 { 1.27f32 } else { 1.62f32 };
        expected_eye += (target - expected_eye) * 0.5;
        let (eye_end, _, _) = p.camera(1.0);
        assert_eq!(eye_end.y - p.state.pos.y, f64::from(expected_eye));
    }
}

/// The partial-tick interpolation runs between the camera's old and new eye height (vanilla
/// `Camera.setup`: `Mth.lerp(partialTicks, eyeHeightOld, eyeHeight)`), a smooth ramp within
/// each tick rather than a jump.
#[test]
fn camera_eye_height_interpolates_within_the_tick() {
    let (mut p, _) = spawned();
    let shift = InputState {
        sneak: true,
        ..Default::default()
    };
    tick(&mut p, &shift); // pose -> Crouching, camera pair still 1.62 -> 1.62
    tick(&mut p, &shift); // camera pair 1.62 -> 1.445
    let y0 = p.camera(0.0).0.y - p.state.pos.y;
    let half = p.camera(0.5).0.y - p.state.pos.y;
    let y1 = p.camera(1.0).0.y - p.state.pos.y;
    assert_eq!(y0, f64::from(1.62f32));
    assert_eq!(y1, f64::from(1.62f32 + (1.27f32 - 1.62f32) * 0.5));
    // The lerp is vanilla's f32 `Mth.lerp`; allow its rounding against the f64 mean.
    assert!(
        (half - (y0 + y1) * 0.5).abs() < 1e-6,
        "half {half} vs mean {}",
        (y0 + y1) * 0.5
    );
}

/// Pressing shift sends `player_input` with the SNEAK bit, releasing it sends it cleared
/// (vanilla `LocalPlayer.sendChanges` sends `ServerboundPlayerInputPacket` when the held keys
/// changed).
#[test]
fn shift_sends_player_input_with_the_sneak_bit() {
    let (mut p, _) = spawned();
    let shift = InputState {
        sneak: true,
        ..Default::default()
    };
    let out = tick(&mut p, &shift);
    assert_eq!(names(&out), ["input", "status", "tick_end"]);
    match &out[0] {
        ServerboundPlay::PlayerInput(i) => {
            assert_eq!(i.flags, input_flags::SNEAK);
        }
        _ => unreachable!(),
    }
    // Held: nothing more. Released: the bit clears.
    assert_eq!(names(&tick(&mut p, &shift)), ["tick_end"]);
    let out = tick(&mut p, &InputState::default());
    assert_eq!(names(&out), ["input", "tick_end"]);
    match &out[0] {
        ServerboundPlay::PlayerInput(i) => assert_eq!(i.flags, 0),
        _ => unreachable!(),
    }
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

// ---------------------------------------------------------------------------
// Riding: the passenger tick (vanilla `rideTick`).
// ---------------------------------------------------------------------------

/// A vehicle at `pos` facing yaw 0 (the yaw matters only in `the_seat_turns_with_the_vehicle`).
fn vehicle_at(pos: DVec3) -> VehiclePose {
    VehiclePose { pos, yaw: 0.0 }
}

/// Mounted on a vehicle at (0.5, 64, 0.5) with a seat 0.1 above it, after the join flow.
fn riding() -> LocalPlayer {
    let (mut p, _) = spawned();
    p.set_vehicle(
        Some(EntityId(9)),
        SeatPoint {
            x: 0.0,
            y: 0.1,
            z: 0.0,
        },
        None,
        None,
        None,
    );
    p
}

#[test]
fn the_passenger_sits_at_the_seat_and_follows_the_vehicle() {
    let mut p = riding();
    let pose = vehicle_at(DVec3::new(10.0, 64.0, 10.0));
    let out = tick_riding(&mut p, &InputState::default(), Some(pose));
    assert_eq!(p.state.pos, DVec3::new(10.0, 64.1, 10.0));
    // Rotation only; never a position packet.
    assert_eq!(names(&out), ["tick_end"]);
    // The vehicle moves: the passenger moves with it, and the camera follows (prev_pos
    // brackets the tick like always).
    let out = tick_riding(
        &mut p,
        &InputState::default(),
        Some(vehicle_at(DVec3::new(11.0, 64.0, 10.0))),
    );
    assert_eq!(p.state.pos, DVec3::new(11.0, 64.1, 10.0));
    assert_eq!(p.prev_pos, DVec3::new(10.0, 64.1, 10.0));
    let (eye, _, _) = p.camera(0.5);
    assert!((eye.x - 10.5).abs() < 1e-9, "{eye:?}");
    assert_eq!(names(&out), ["tick_end"]);
    // An untracked vehicle leaves the passenger where it was.
    tick_riding(&mut p, &InputState::default(), None);
    assert_eq!(p.state.pos, DVec3::new(11.0, 64.1, 10.0));
}

#[test]
fn the_seat_turns_with_the_vehicle() {
    let mut p = riding();
    // A seat 0.2 to one side of the vehicle: at yaw 0 that offset is along x, at yaw 90 along
    // z (`Entity.positionRider` runs every tick, so the seat follows the vehicle).
    p.set_vehicle(
        Some(EntityId(9)),
        SeatPoint {
            x: 0.2,
            y: 0.0,
            z: 0.0,
        },
        None,
        None,
        None,
    );
    let pose = VehiclePose {
        pos: DVec3::new(10.0, 64.0, 10.0),
        yaw: 90.0,
    };
    tick_riding(&mut p, &InputState::default(), Some(pose));
    let d = p.state.pos - pose.pos;
    assert!(d.x.abs() < 1e-6 && (d.z - 0.2).abs() < 1e-6, "{d:?}");
}

#[test]
fn a_passenger_looks_freely_and_sends_rotation_only() {
    let mut p = riding();
    let look = InputState {
        yaw: 30.0,
        pitch: 10.0,
        ..Default::default()
    };
    let out = tick_riding(
        &mut p,
        &look,
        Some(vehicle_at(DVec3::new(10.0, 64.0, 10.0))),
    );
    assert_eq!((p.state.yaw, p.state.pitch), (30.0, 10.0));
    // The look changed: one rotation packet, no position (the keys have not changed, so no
    // player_input with it).
    assert_eq!(names(&out), ["rot", "tick_end"]);
    // Same look again: nothing but the tick end.
    let out = tick_riding(
        &mut p,
        &look,
        Some(vehicle_at(DVec3::new(10.0, 64.0, 10.0))),
    );
    assert_eq!(names(&out), ["tick_end"]);
}

#[test]
fn sneak_while_riding_sets_the_shift_flag_in_player_input() {
    let mut p = riding();
    let sneak = InputState {
        sneak: true,
        ..Default::default()
    };
    let out = tick_riding(
        &mut p,
        &sneak,
        Some(vehicle_at(DVec3::new(10.0, 64.0, 10.0))),
    );
    let flags = out.iter().find_map(|p| match p {
        ServerboundPlay::PlayerInput(i) => Some(i.flags),
        _ => None,
    });
    assert_eq!(
        flags,
        Some(haste_protocol::packets::play::serverbound::input_flags::SNEAK)
    );
    // Still mounted: the server answers the shift flag with set_passengers (not our doing).
    assert_eq!(p.vehicle(), Some(EntityId(9)));
}

// ---------------------------------------------------------------------------
// Riding a boat: the vehicle we steer (vanilla `LocalPlayer.rideTick`).
// ---------------------------------------------------------------------------

/// The boat's box, from its `EntityType`'s `sized(1.375F, 0.5625F)`.
const BOAT_DIMS: (f32, f32) = (1.375, 0.5625);

/// Open water the boat floats on: a 17x17 pool of source water at y = 63.
fn pool() -> TestWorld {
    let mut world = TestWorld::new();
    world.fill((-8, 63, -8), (8, 63, 8), states::WATER_SOURCE);
    world
}

/// The water surface, as the status check stores it (vanilla's float `y + getHeight`).
fn surface() -> f64 {
    (63.0f32 + 8.0 / 9.0) as f64
}

/// Mounted on a boat floating on `pool`'s water, at the seat vanilla's attachment points give
/// the first passenger of an oak boat.
fn in_the_boat(world: &TestWorld) -> LocalPlayer {
    let shapes = TestShapes::default();
    let env = crate::boat::BoatEnv {
        world,
        shapes: &shapes,
        blocks: &shapes,
    };
    let surface = surface();
    let boat = crate::boat::Boat::new(
        DVec3::new(0.5, surface - 0.65 * f64::from(BOAT_DIMS.1), 0.5),
        0.0,
        BOAT_DIMS,
        &env,
    );
    let (mut p, _) = spawned();
    let seat = haste_entity::passenger_seat(haste_entity::riding::SeatQuery {
        vehicle_type: "oak_boat",
        vehicle_dims: BOAT_DIMS,
        seat: 0,
        passengers: 1,
        passenger: haste_entity::riding::PassengerKind::Player,
        passenger_type: "player",
    });
    p.set_vehicle(Some(EntityId(9)), seat, Some(boat), None, None);
    p
}

/// One tick of a passenger, against `world`.
fn tick_paddling(
    p: &mut LocalPlayer,
    input: &InputState,
    world: &TestWorld,
) -> Vec<ServerboundPlay> {
    let mut out = Vec::new();
    let shapes = TestShapes::default();
    p.tick_with(input, world, &shapes, &shapes, None, &mut out);
    out
}

/// The paddled boat is simulated on our side (vanilla's `AbstractBoat.tick` for a
/// client-authoritative instance): the keys our `rideTick` hands it are the ones the next tick's
/// `controlBoat` uses, the boat sends `paddle_boat`, and after the passenger's rotation the
/// `move_vehicle` carries the boat's own simulation to the server.
#[test]
fn steering_a_boat_sends_paddle_boat_and_move_vehicle() {
    let world = pool();
    let mut p = in_the_boat(&world);
    let forward = InputState {
        forward: true,
        ..Default::default()
    };
    let out = tick_paddling(&mut p, &forward, &world);
    assert_eq!(names(&out), ["input", "paddle", "move_vehicle", "tick_end"]);
    // First tick: the keys we came aboard with were empty, so the paddles do not turn yet.
    let paddle = out.iter().find_map(|p| match p {
        ServerboundPlay::PaddleBoat(p) => Some((p.left, p.right)),
        _ => None,
    });
    assert_eq!(paddle, Some((false, false)));
    // The second tick paddles: both paddles turn and the boat has started forward.
    let out = tick_paddling(&mut p, &forward, &world);
    assert_eq!(names(&out), ["paddle", "move_vehicle", "tick_end"]);
    match &out[0] {
        ServerboundPlay::PaddleBoat(p) => assert!(p.left && p.right),
        other => panic!("expected paddle_boat, got {other:?}"),
    }
    let boat = p.boat().expect("still steering the boat");
    assert!(boat.delta.z > 0.0, "the boat paddled: {}", boat.delta.z);
    // The rider sits at the seat of the boat we are paddling, not at the server's copy of it.
    // (A boat's first passenger has no sideways offset, and its seat height is the boat's
    // `rideHeight` minus the player's own vehicle attachment point.)
    assert!((p.state.pos.x - boat.pos.x).abs() < 1.0e-9);
    assert!((p.state.pos.z - boat.pos.z).abs() < 1.0e-9);
    assert!(
        (p.state.pos.y - (boat.pos.y + p.seat().y)).abs() < 1.0e-9,
        "seat height: {} vs {}",
        p.state.pos.y,
        boat.pos.y + p.seat().y
    );
    // The `move_vehicle` is the boat's own pose (its pitch is zero: a boat never pitches).
    match &out[1] {
        ServerboundPlay::MoveVehicle(m) => {
            assert_eq!(m.pos, boat.pos);
            assert_eq!(m.yaw, boat.yaw);
            assert_eq!(m.pitch, 0.0);
        }
        other => panic!("expected move_vehicle, got {other:?}"),
    }
}

/// The seat follows the boat the paddles push, tick by tick, and the boat keeps its own
/// registration in the world (the packet that carries it says where the boat is now).
#[test]
fn the_seat_follows_the_paddled_boat() {
    let world = pool();
    let mut p = in_the_boat(&world);
    let forward = InputState {
        forward: true,
        ..Default::default()
    };
    let start = p.state.pos;
    for _ in 0..20 {
        tick_paddling(&mut p, &forward, &world);
    }
    // The boat paddled forward; the rider left with it (the seat is fixed in the boat's frame).
    assert!(p.state.pos.z > start.z + 0.4, "{}", p.state.pos.z);
    assert!(
        (p.state.pos.x - start.x).abs() < 1.0e-9,
        "{}",
        p.state.pos.x
    );
}

#[test]
fn dismount_resumes_physics_from_the_seat() {
    let mut p = riding();
    tick_riding(
        &mut p,
        &InputState::default(),
        Some(vehicle_at(DVec3::new(0.5, 64.0, 0.5))),
    );
    assert_eq!(p.state.pos, DVec3::new(0.5, 64.1, 0.5));
    // Dismount: gravity takes over from the seat position. The first tick after it builds
    // velocity (the ride zeroed it); the fall starts the tick after.
    p.set_vehicle(None, SeatPoint::ZERO, None, None, None);
    tick_riding(&mut p, &InputState::default(), None);
    assert_eq!(p.state.pos, DVec3::new(0.5, 64.1, 0.5));
    let out = tick_riding(&mut p, &InputState::default(), None);
    assert!(
        p.state.pos.y < 64.1 && p.state.pos.y > 63.9,
        "gravity resumed: {}",
        p.state.pos.y
    );
    // And movement packets are position packets again.
    assert!(out.iter().any(|p| {
        matches!(
            p,
            ServerboundPlay::MovePlayerPos(_) | ServerboundPlay::MovePlayerPosRot(_)
        )
    }));
}

// ---------------------------------------------------------------------------
// Riding a horse: the steered mount of the horse family (vanilla `AbstractHorse`).
// ---------------------------------------------------------------------------

/// The horse's box, from its `EntityType`'s `sized(1.3964844F, 1.6F)`.
const HORSE_DIMS: (f32, f32) = (1.396_484_4, 1.6);

/// One tick of a passenger, against `world`.
fn tick_riding_world(
    p: &mut LocalPlayer,
    input: &InputState,
    world: &TestWorld,
) -> Vec<ServerboundPlay> {
    let mut out = Vec::new();
    let shapes = TestShapes::default();
    p.tick_with(input, world, &shapes, &shapes, None, &mut out);
    out
}

/// Mounted on a saddled horse standing on the flat stone floor (top at y = 0), at the seat
/// vanilla's attachment points give the first passenger of a horse.
fn on_the_horse() -> (LocalPlayer, TestWorld) {
    let world = TestWorld::flat(0, states::STONE);
    let horse = crate::horse::Horse::new(
        DVec3::new(0.5, 0.0, 0.5),
        0.0,
        HORSE_DIMS,
        crate::horse::DEFAULT_MOVEMENT_SPEED,
        crate::horse::DEFAULT_JUMP_STRENGTH,
        crate::horse::DEFAULT_GRAVITY,
    );
    let (mut p, _) = spawned();
    let seat = haste_entity::passenger_seat(haste_entity::riding::SeatQuery {
        vehicle_type: "horse",
        vehicle_dims: HORSE_DIMS,
        seat: 0,
        passengers: 1,
        passenger: haste_entity::riding::PassengerKind::Player,
        passenger_type: "player",
    });
    p.set_vehicle(Some(EntityId(9)), seat, None, Some(horse), None);
    (p, world)
}

/// The steered horse is simulated on our side (vanilla's `AbstractHorse` tick for a
/// client-authoritative instance): riding forward sends `move_vehicle` with the horse's own
/// simulated pose each tick, and the seat follows it.
#[test]
fn steering_a_horse_sends_move_vehicle_of_its_simulation() {
    let (mut p, world) = on_the_horse();
    let forward = InputState {
        forward: true,
        ..Default::default()
    };
    // The first tick: the horse has not got our keys yet (they arrive with this tick, one tick
    // late, like the boat's paddles), so it stands still but still reports its pose.
    let out = tick_riding_world(&mut p, &forward, &world);
    assert_eq!(names(&out), ["input", "move_vehicle", "tick_end"]);
    let horse = p.horse().expect("still steering the horse");
    assert!(horse.delta.z.abs() < 1.0e-9, "{}", horse.delta.z);
    // The second tick rides.
    let out = tick_riding_world(&mut p, &forward, &world);
    assert_eq!(names(&out), ["move_vehicle", "tick_end"]);
    match &out[0] {
        ServerboundPlay::MoveVehicle(m) => {
            assert_eq!(m.pos, p.horse().unwrap().pos);
            assert_eq!(m.yaw, p.horse().unwrap().yaw);
            assert!(m.pos.z > 0.5, "the horse walked: {}", m.pos.z);
        }
        other => panic!("expected move_vehicle, got {other:?}"),
    }
    // The rider sits at the seat of the horse we steer.
    let horse = p.horse().unwrap();
    assert!((p.state.pos.x - horse.pos.x).abs() < 1.0e-9);
    assert!((p.state.pos.y - (horse.pos.y + p.seat().y)).abs() < 1.0e-9);
    assert!((p.state.pos.z - horse.pos.z).abs() < 1.0e-9);
}

/// Holding jump charges the mount's jump (vanilla `LocalPlayer.aiStep`'s `jumpRidingTicks`
/// series); releasing it sends the `player_command` START_RIDING_JUMP with the charge, and the
/// horse fires the jump on its next ground tick.
#[test]
fn holding_and_releasing_jump_charges_and_sends_the_riding_jump() {
    let (mut p, world) = on_the_horse();
    let jump = InputState {
        jump: true,
        ..Default::default()
    };
    // Press: the charge starts empty. Tick two climbs to 0.1 (vanilla counts the press tick).
    tick_riding_world(&mut p, &jump, &world);
    assert_eq!(p.jump_riding_scale(), 0.0);
    tick_riding_world(&mut p, &jump, &world);
    assert!((p.jump_riding_scale() - 0.1).abs() < 1.0e-6);
    for _ in 0..3 {
        tick_riding_world(&mut p, &jump, &world);
    }
    assert!((p.jump_riding_scale() - 0.4).abs() < 1.0e-6);
    // Release: the START_RIDING_JUMP carries the floor of the scale in percent.
    let out = tick_riding_world(&mut p, &InputState::default(), &world);
    let command = out.iter().find_map(|p| match p {
        ServerboundPlay::PlayerCommand(c) => Some(c),
        _ => None,
    });
    let c = command.expect("a riding jump command");
    assert_eq!(c.action, command::START_RIDING_JUMP);
    assert_eq!(c.jump_boost, 40);
    // The horse fires the charged jump on its next tick (the scale is the pending 0.4 + 0.4 x
    // 40/90).
    tick_riding_world(&mut p, &InputState::default(), &world);
    let horse = p.horse().unwrap();
    assert!(!horse.on_ground, "the horse jumped: {}", horse.pos.y);
    // The scale itself outlives the release (vanilla keeps it until the ten-tick lockout
    // counts up to zero, so the HUD bar drains rather than vanishes).
    assert_eq!(p.jump_riding_scale(), 0.4);
    for _ in 0..10 {
        tick_riding_world(&mut p, &InputState::default(), &world);
    }
    assert_eq!(p.jump_riding_scale(), 0.0);
    // A fresh press charges from zero again.
    tick_riding_world(&mut p, &jump, &world);
    tick_riding_world(&mut p, &jump, &world);
    assert!((p.jump_riding_scale() - 0.1).abs() < 1.0e-6);
}

/// A new mount starts uncharged, and a dismount clears the simulation (the horse keeps its
/// server-driven position again).
#[test]
fn a_dismount_clears_the_horse_simulation() {
    let (mut p, world) = on_the_horse();
    let jump = InputState {
        jump: true,
        ..Default::default()
    };
    tick_riding_world(&mut p, &jump, &world);
    tick_riding_world(&mut p, &jump, &world);
    assert!(p.jump_riding_scale() > 0.0);
    p.set_vehicle(None, SeatPoint::ZERO, None, None, None);
    assert!(p.horse().is_none());
    assert_eq!(p.jump_riding_scale(), 0.0);
}

// ---------------------------------------------------------------------------
// Riding a pig with a carrot on a stick (vanilla `Pig`).
// ---------------------------------------------------------------------------

/// The pig's box, from its `EntityType`'s `sized(0.9F, 0.9F)`.
const PIG_DIMS: (f32, f32) = (0.9, 0.9);

/// Mounted on a saddled pig standing on the flat stone floor, holding a carrot on a stick
/// (item index 42, marked as a steer item) in the main hand.
fn on_the_pig() -> (LocalPlayer, TestWorld) {
    let world = TestWorld::flat(0, states::STONE);
    let pig = crate::pig::Pig::new(
        DVec3::new(0.5, 0.0, 0.5),
        0.0,
        PIG_DIMS,
        crate::pig::DEFAULT_MOVEMENT_SPEED,
        crate::horse::DEFAULT_GRAVITY,
    );
    let (mut p, _) = spawned();
    let seat = haste_entity::passenger_seat(haste_entity::riding::SeatQuery {
        vehicle_type: "pig",
        vehicle_dims: PIG_DIMS,
        seat: 0,
        passengers: 1,
        passenger: haste_entity::riding::PassengerKind::Player,
        passenger_type: "player",
    });
    p.set_vehicle(Some(EntityId(9)), seat, None, None, Some(pig));
    // The carrot on a stick in the main hand, and the steer-item set that names it.
    let stick = haste_core::RegistryIndex(42);
    p.int.steer_items = Some([stick].into_iter().collect());
    p.int.hotbar[0] = haste_core::entity::ItemStackData {
        count: 1,
        item: Some(stick),
        components_raw: Default::default(),
    };
    (p, world)
}

/// The pig walks forward on its own while the rider holds the carrot on a stick
/// (`Pig.getRiddenInput` is a constant full forward): our simulation drives the seat and the
/// `move_vehicle`, and putting the stick away stands it down (the server simulates again).
#[test]
fn a_pig_walks_while_the_carrot_stick_is_held() {
    let (mut p, world) = on_the_pig();
    let hold = InputState::default();
    // The pig has no look yet (it arrives one tick late, like the horse's input).
    let out = tick_riding_world(&mut p, &hold, &world);
    assert_eq!(names(&out), ["move_vehicle", "tick_end"]);
    let out = tick_riding_world(&mut p, &hold, &world);
    let pig = p.pig().expect("still steering the pig");
    assert!(pig.delta.z > 0.0, "the pig walked: {}", pig.delta.z);
    match &out[0] {
        ServerboundPlay::MoveVehicle(m) => {
            assert_eq!(m.pos, p.pig().unwrap().pos);
            assert!(m.pos.z > 0.5, "the pig moved: {}", m.pos.z);
        }
        other => panic!("expected move_vehicle, got {other:?}"),
    }
    // The rider sits at the seat of the pig we steer.
    let pig = p.pig().unwrap();
    assert!((p.state.pos.z - pig.pos.z).abs() < 1.0e-9);
    // Putting the stick away (an empty hotbar slot): our simulation stands down, the pig
    // keeps whatever the server says about it.
    p.held_slot = 1;
    let before = p.pig().unwrap().pos;
    for _ in 0..5 {
        tick_riding_world(&mut p, &hold, &world);
    }
    assert_eq!(p.pig().unwrap().pos, before, "the pig stood down");
}

/// `LivingEntity.isHolding` looks at both hands: the carrot on a stick in the off hand steers
/// the pig too.
#[test]
fn the_carrot_stick_in_the_off_hand_steers_the_pig() {
    let (mut p, world) = on_the_pig();
    p.int.offhand = p.int.hotbar[0].clone();
    p.held_slot = 1;
    let hold = InputState::default();
    tick_riding_world(&mut p, &hold, &world);
    let out = tick_riding_world(&mut p, &hold, &world);
    assert!(
        out.iter()
            .any(|o| matches!(o, ServerboundPlay::MoveVehicle(_))),
        "the off-hand stick steers"
    );
    assert!(p.pig().unwrap().pos.z > 0.5, "the pig walked");
}

// ---------------------------------------------------------------------------
// Elytra flight (vanilla `Player.tryToStartFallFlying`, shared flag 7).
// ---------------------------------------------------------------------------

/// Equips a glider in the chest slot: item registry index `index` is marked as a glider item
/// (the reports' `minecraft:glider` component) and put in the chest armor slot.
fn equip_glider(p: &mut LocalPlayer, index: u32) {
    let item = haste_core::RegistryIndex(index);
    p.int.glider_items = Some([item].into_iter().collect());
    p.int.armor[1] = haste_core::entity::ItemStackData {
        count: 1,
        item: Some(item),
        components_raw: Default::default(),
    };
}

/// Put the loaded player in the air with no velocity.
fn airborne(p: &mut LocalPlayer) {
    p.state.pos = DVec3::new(0.5, 100.0, 0.5);
    p.state.on_ground = false;
    p.state.velocity = DVec3::ZERO;
}

#[test]
fn jump_in_the_air_with_a_glider_starts_falling_flying() {
    let (mut p, _) = spawned();
    equip_glider(&mut p, 42);
    airborne(&mut p);
    let out = tick(
        &mut p,
        &InputState {
            jump: true,
            ..Default::default()
        },
    );
    assert!(p.state.fall_flying, "the local flag is set at once");
    assert!(
        out.iter().any(|m| matches!(
            m,
            ServerboundPlay::PlayerCommand(c) if c.action == command::START_FALL_FLYING
        )),
        "START_FALL_FLYING is sent: {out:?}"
    );
    // Vanilla `Player.getDesiredPose` -> FALL_FLYING; the pose's eye height is 0.4.
    assert_eq!(p.state.pose, haste_physics::Pose::FallFlying);
    assert!((p.eye_height() - 0.4).abs() < 1e-6, "{}", p.eye_height());
    // The glide already moves this tick (it sinks from the look-ahead pitch 0).
    assert!(p.state.pos.y < 100.0, "{}", p.state.pos);
}

#[test]
fn jump_in_the_air_without_a_glider_does_not_start() {
    let (mut p, _) = spawned();
    airborne(&mut p);
    let out = tick(
        &mut p,
        &InputState {
            jump: true,
            ..Default::default()
        },
    );
    assert!(!p.state.fall_flying);
    assert!(!out
        .iter()
        .any(|m| matches!(m, ServerboundPlay::PlayerCommand(_))));
    assert_ne!(p.state.pose, haste_physics::Pose::FallFlying);
}

#[test]
fn a_glider_in_the_chest_only_helps_with_the_glider_component() {
    let (mut p, _) = spawned();
    // A plain chest item (not in the glider set): no start.
    p.int.armor[1] = haste_core::entity::ItemStackData {
        count: 1,
        item: Some(haste_core::RegistryIndex(42)),
        components_raw: Default::default(),
    };
    assert!(!p.has_glider());
    // The same item once the reports mark it as a glider.
    p.int.glider_items = Some([haste_core::RegistryIndex(42)].into_iter().collect());
    assert!(p.has_glider());
    // An empty stack is never a glider.
    p.int.armor[1] = Default::default();
    assert!(!p.has_glider());
}

#[test]
fn server_shared_flag_7_sets_and_clears_fall_flying() {
    use haste_core::entity::{MetadataEntry, MetadataValue};
    let (mut p, mut out) = spawned();
    let set = |b: i8| {
        ClientboundPlay::SetEntityData(cb::SetEntityData {
            entity_id: ME.0,
            metadata: vec![MetadataEntry {
                index: 0,
                value: MetadataValue::Byte(b),
            }],
        })
    };
    assert!(p.handle(&set(0x80u8 as i8), &mut out));
    assert!(p.state.fall_flying);
    assert_eq!(
        p.state.pose,
        haste_physics::Pose::Standing,
        "pose follows next tick"
    );
    p.handle(&set(0), &mut out);
    assert!(!p.state.fall_flying);
}

#[test]
fn landing_ends_the_glide_and_the_pose() {
    let (mut p, _) = spawned();
    p.state.fall_flying = true;
    p.state.pos = DVec3::new(0.5, 1.5, 0.5);
    p.state.on_ground = false;
    p.state.velocity = DVec3::new(0.0, -0.5, 0.0);
    for _ in 0..200 {
        tick(&mut p, &InputState::default());
        if p.state.on_ground {
            break;
        }
    }
    assert!(p.state.on_ground, "{}", p.state.pos);
    assert!(
        !p.state.fall_flying,
        "vanilla `LivingEntity.travelFallFlying`"
    );
    assert_ne!(p.state.pose, haste_physics::Pose::FallFlying);
}
