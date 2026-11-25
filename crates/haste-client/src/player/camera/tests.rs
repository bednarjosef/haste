//! The player's tick while another entity is the camera entity (vanilla
//! `LocalPlayer.isControlledCamera` false): no movement, no position packets, the input packet
//! (sneak to stop spectating) still sent, the camera's eye height easing toward the camera
//! entity's (`Camera.tick`), and the pick from the camera entity's eyes.

use glam::DVec3;
use haste_core::game::GameMode;
use haste_core::input::InputState;
use haste_core::EntityId;
use haste_physics::testing::{states, TestShapes, TestWorld};
use haste_protocol::packets::play::clientbound as cb;
use haste_protocol::packets::play::serverbound::input_flags;
use haste_protocol::{ClientboundPlay, ServerboundPlay};

use crate::game::camera_entity::CameraEntitySnapshot;
use crate::player::LocalPlayer;

const ME: EntityId = EntityId(7);
const PIG: EntityId = EntityId(42);

fn ids() -> crate::player::AttributeIds {
    crate::player::AttributeIds {
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

fn tick(p: &mut LocalPlayer, input: &InputState) -> Vec<ServerboundPlay> {
    let mut out = Vec::new();
    p.tick_with(
        input,
        &TestWorld::flat(0, states::STONE),
        &TestShapes::default(),
        &TestShapes::default(),
        None,
        &mut out,
    );
    out
}

/// A spectator placed at (0.5, 10, 0.5), after its spawn accept, flying.
fn spectator() -> LocalPlayer {
    let mut p = LocalPlayer::with_entity(ME, GameMode::Spectator);
    p.set_attribute_ids(ids());
    let mut out = Vec::new();
    p.handle(
        &ClientboundPlay::PlayerPosition(cb::PlayerPosition {
            teleport_id: 1,
            pos: DVec3::new(0.5, 10.0, 0.5),
            velocity: DVec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            flags: 0,
        }),
        &mut out,
    );
    p.handle(
        &ClientboundPlay::GameEvent(cb::GameEvent {
            event: 3,
            value: 3.0,
        }),
        &mut out,
    );
    for _ in 0..3 {
        tick(&mut p, &InputState::default());
    }
    p
}

fn pig(eye_height: f32) -> CameraEntitySnapshot {
    CameraEntitySnapshot {
        id: PIG,
        prev_pos: DVec3::new(20.0, 10.0, 0.0),
        pos: DVec3::new(20.0, 10.0, 1.0),
        prev_rot: (90.0, 0.0),
        rot: (90.0, 15.0),
        eye_height,
        tilt: None,
    }
}

fn moves(out: &[ServerboundPlay]) -> usize {
    out.iter()
        .filter(|o| {
            matches!(
                o,
                ServerboundPlay::MovePlayerPos(_)
                    | ServerboundPlay::MovePlayerPosRot(_)
                    | ServerboundPlay::MovePlayerRot(_)
                    | ServerboundPlay::MovePlayerStatusOnly(_)
            )
        })
        .count()
}

#[test]
fn while_spectating_the_keys_move_nothing_and_no_position_leaves() {
    let mut p = spectator();
    p.set_camera_entity(Some(pig(0.765)));
    assert!(!p.is_controlled_camera());
    let start = p.state.pos;
    let fly = InputState {
        forward: true,
        jump: true,
        yaw: 45.0,
        ..Default::default()
    };
    for _ in 0..20 {
        let out = tick(&mut p, &fly);
        // `LocalPlayer.sendPosition` sends nothing (the 20-tick position reminder included).
        assert_eq!(moves(&out), 0, "no move packet while spectating: {out:?}");
    }
    assert_eq!(p.state.pos, start, "the movement keys do not move us");
    // Sneak: the input packet still carries the shift flag, the server's cue to end it.
    let out = tick(
        &mut p,
        &InputState {
            sneak: true,
            ..Default::default()
        },
    );
    assert!(out.iter().any(|o| matches!(
        o,
        ServerboundPlay::PlayerInput(i) if i.flags & input_flags::SNEAK != 0
    )));
    // The camera is ours again: the keys move us and the position goes out.
    p.set_camera_entity(None);
    let out = tick(&mut p, &fly);
    assert!(p.state.pos != start);
    assert_eq!(moves(&out), 1);
}

#[test]
fn the_camera_eye_height_halves_toward_the_camera_entitys_and_back() {
    let mut p = spectator();
    let standing = 1.62f32;
    assert_eq!(p.camera_eye_heights(), (standing, standing));
    // `Camera.tick`: `eyeHeight += (entity.getEyeHeight() - eyeHeight) * 0.5`, the camera's one
    // pair continuing across the switch (no reset in `setEntity`).
    p.set_camera_entity(Some(pig(0.765)));
    let mut expected = standing;
    for _ in 0..4 {
        tick(&mut p, &InputState::default());
        let old = expected;
        expected += (0.765 - expected) * 0.5;
        assert_eq!(p.camera_eye_heights(), (old, expected));
    }
    // Back to the player: the pair eases up to the standing eye height again.
    p.set_camera_entity(None);
    tick(&mut p, &InputState::default());
    let old = expected;
    expected += (standing - expected) * 0.5;
    assert_eq!(p.camera_eye_heights(), (old, expected));
}

#[test]
fn the_pick_starts_at_the_camera_entitys_eyes_and_skips_it() {
    let mut p = spectator();
    let (eye, look, skip) = p.pick_view((0.0, 0.0));
    assert_eq!((eye, look, skip), (p.eye(), (0.0, 0.0), ME));
    p.set_camera_entity(Some(pig(0.765)));
    let (eye, look, skip) = p.pick_view((0.0, 0.0));
    assert_eq!(eye, DVec3::new(20.0, 10.0 + f64::from(0.765f32), 1.0));
    assert_eq!(look, (90.0, 15.0));
    assert_eq!(skip, PIG);
}
