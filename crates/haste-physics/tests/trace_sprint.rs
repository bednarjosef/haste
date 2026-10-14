#[test]
fn trace_sprint_jump() {
    use haste_core::input::InputState;
    use haste_core::DVec3;
    use haste_physics::testing::{states::*, TestShapes, TestWorld};
    use haste_physics::{tick, PlayerAttributes, PlayerState};
    let world = TestWorld::flat(0, STONE);
    let shapes = TestShapes::default();
    let attrs = PlayerAttributes::default();
    let mut s = PlayerState::at(DVec3::new(0.5, 0.0, 0.5));
    s.on_ground = false;
    let input = InputState {
        forward: true,
        sprint: true,
        jump: true,
        ..Default::default()
    };
    for _ in 0..3 {
        tick(
            &mut s,
            &InputState::default(),
            &attrs,
            &world,
            &shapes,
            &shapes,
        );
    }
    for i in 0..30 {
        tick(&mut s, &input, &attrs, &world, &shapes, &shapes);
        println!(
            "t={i:2} y={:.4} vy={:.4} vz={:.4} ground={} sprint={} cd={}",
            s.pos.y, s.velocity.y, s.velocity.z, s.on_ground, s.sprinting, s.jump_cooldown
        );
    }
}
