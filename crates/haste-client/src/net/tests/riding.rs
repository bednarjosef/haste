//! Riding a vehicle: while the player is a passenger only rotation and input packets go out
//! (never a position), and sneaking sends the dismount flag in `player_input`.

use super::*;

#[test]
fn riding_sends_rotation_and_input_only() {
    let mut r = Rig::new();
    r.apply(spawn_packet(5, DVec3::new(1.0, 2.0, 3.0)));
    // Join: teleport, accept, loaded.
    r.apply(teleport(1, DVec3::new(1.0, 2.0, 3.0), 0));
    end_tick_once(&mut r);
    end_tick_once(&mut r);
    r.out.clear();
    // Mount.
    r.apply(ClientboundPlay::SetPassengers(cb::SetPassengers {
        vehicle: 5,
        passengers: vec![7],
    }));
    // Look changes and a key is held while riding: input (the vehicle's steering keys) and
    // rotation only, never a position packet.
    let look = haste_core::input::InputState {
        yaw: 90.0,
        forward: true,
        ..Default::default()
    };
    let mut out = Vec::new();
    let mut world = std::mem::take(&mut r.world);
    let entities = std::mem::take(&mut r.entities);
    r.session
        .end_tick(&look, &mut world, &entities, None, &mut out);
    r.world = world;
    r.entities = entities;
    let names: Vec<&str> = out
        .iter()
        .map(|p| match p {
            ServerboundPlay::MovePlayerPosRot(_) => "pos_rot",
            ServerboundPlay::MovePlayerPos(_) => "pos",
            ServerboundPlay::MovePlayerRot(_) => "rot",
            ServerboundPlay::MovePlayerStatusOnly(_) => "status",
            ServerboundPlay::PlayerInput(_) => "input",
            ServerboundPlay::ClientTickEnd(_) => "tick_end",
            _ => "other",
        })
        .collect();
    // While riding, the vehicle's position is ours; only rotation packets go out.
    assert_eq!(names, ["input", "rot", "tick_end"]);
    // Sneak while riding: player_input carries the shift (sneak) flag, the server answers
    // with set_passengers to dismount.
    let sneak = haste_core::input::InputState {
        yaw: 90.0,
        sneak: true,
        ..Default::default()
    };
    let mut out = Vec::new();
    let mut world = std::mem::take(&mut r.world);
    let entities = std::mem::take(&mut r.entities);
    r.session
        .end_tick(&sneak, &mut world, &entities, None, &mut out);
    r.world = world;
    r.entities = entities;
    let input = out.iter().find_map(|p| match p {
        ServerboundPlay::PlayerInput(i) => Some(i.flags),
        _ => None,
    });
    assert_eq!(
        input,
        Some(haste_protocol::packets::play::serverbound::input_flags::SNEAK)
    );
}
