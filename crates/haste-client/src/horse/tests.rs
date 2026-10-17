//! Tests for the horse family's ridden movement against vanilla's own formulas (the series each
//! tick composes, computed independently from `AbstractHorse`'s constants), on the flat stone
//! platform of the evidence area.
use super::*;
use crate::travel::{GROUND_ACCELERATION, HORIZONTAL_DRAG, VERTICAL_DRAG};
use haste_physics::testing::{states, TestShapes, TestWorld};

/// The horse's box, from its `EntityType`'s `sized(1.3964844F, 1.6F)`.
const HORSE: (f32, f32) = (1.396_484_4, 1.6);

/// A horse with the family's default attributes on the stone platform (flat, top at y = 0).
fn horse_at(pos: DVec3) -> Horse {
    Horse::new(
        pos,
        0.0,
        HORSE,
        DEFAULT_MOVEMENT_SPEED,
        DEFAULT_JUMP_STRENGTH,
        DEFAULT_GRAVITY,
    )
}

fn env<'a>(world: &'a TestWorld, shapes: &'a TestShapes) -> HorseEnv<'a> {
    HorseEnv {
        world,
        shapes,
        blocks: shapes,
    }
}

/// One tick with the rider's held keys.
fn tick_keys(h: &mut Horse, e: &HorseEnv<'_>, input: &InputState, look: (f32, f32)) {
    let mv = rider_input(input);
    h.set_input(mv.0, mv.1, look);
    h.tick(e);
}

/// A horse that has been standing: vanilla's standing loop keeps a downward delta of
/// `gravity * 0.98` (gravity applies after each tick's move, the collision zeroes it), so the
/// move is never zero and `onGround` holds. A freshly constructed horse starts at zero velocity
/// and needs two ticks to fall into that loop, exactly like a spawned horse landing.
fn settled(pos: DVec3) -> Horse {
    let world = TestWorld::flat(0, states::STONE);
    let shapes = TestShapes::default();
    let e = env(&world, &shapes);
    let mut h = horse_at(pos);
    for _ in 0..3 {
        tick_keys(&mut h, &e, &InputState::default(), (0.0, 0.0));
    }
    assert!(h.on_ground, "the settle should end on the ground");
    h
}

/// The ground acceleration of one forward tick: the attribute speed times vanilla's
/// `0.216 / slip^3` (stone's slipperiness 0.6), times the rider's input scale.
fn forward_acceleration() -> f64 {
    let slip = 0.6f32;
    let accel = 0.225f32 * (GROUND_ACCELERATION / (slip * slip * slip));
    f64::from(INPUT_SCALE * accel)
}

/// The horizontal drag of the stone platform.
fn drag() -> f64 {
    f64::from(0.6f32 * HORIZONTAL_DRAG)
}

/// The forward speed after `ticks` ticks of riding forward: each tick adds the acceleration
/// `a` and the drag `f` takes the rest away, so the move of tick k is `a (1 - f^k) / (1 - f)`
/// and the speed that survives the tick is that times `f`.
fn walk_speed(ticks: u32) -> f64 {
    let (a, f) = (forward_acceleration(), drag());
    a * (1.0 - f.powi(ticks as i32)) / (1.0 - f) * f
}

/// The distance `ticks` ticks of riding forward cover (the sum of the moves).
fn walk_distance(ticks: u32) -> f64 {
    let (a, f) = (forward_acceleration(), drag());
    a / (1.0 - f) * (f64::from(ticks) - f * (1.0 - f.powi(ticks as i32)) / (1.0 - f))
}

#[test]
fn a_ridden_horse_walks_forward_at_vanillas_speed() {
    let world = TestWorld::flat(0, states::STONE);
    let shapes = TestShapes::default();
    let e = env(&world, &shapes);
    let mut h = settled(DVec3::new(0.5, 0.0, 0.5));
    let forward = InputState {
        forward: true,
        ..Default::default()
    };
    for _ in 0..20 {
        tick_keys(&mut h, &e, &forward, (0.0, 0.0));
    }
    let speed = walk_speed(20);
    assert!(
        (h.delta.z - speed).abs() < 1.0e-6,
        "forward speed: {} vs {}",
        h.delta.z,
        speed
    );
    assert!(h.delta.x.abs() < 1.0e-9, "x drifted: {}", h.delta.x);
    assert!(h.on_ground, "the horse fell through the floor");
    // The rider's yaw is the horse's yaw (tickRidden copies it).
    assert_eq!(h.yaw, 0.0);
    // The twenty ticks of walking covered the series' distance.
    let distance = walk_distance(20);
    assert!(
        (h.pos.z - (0.5 + distance)).abs() < 1.0e-6,
        "z: {} vs {}",
        h.pos.z,
        0.5 + distance
    );
}

#[test]
fn the_charge_of_a_held_jump_follows_vanillas_formula() {
    // The scale climbs a tenth per tick to 0.9, then approaches 1 without reaching it.
    for (ticks, want) in [
        (0, 0.0),
        (1, 0.1),
        (5, 0.5),
        (9, 0.9),
        (10, 0.8 + 2.0 / 1.0 * 0.1),
        (11, 0.8 + 2.0 / 2.0 * 0.1),
        (50, 0.8 + 2.0 / 41.0 * 0.1),
    ] {
        assert!(
            (charge_scale(ticks) - want).abs() < 1.0e-6,
            "tick {ticks}: {} vs {want}",
            charge_scale(ticks)
        );
    }
    // The packet amount is the floor of the scale in percent.
    assert_eq!(charge_amount(0.0), 0);
    assert_eq!(charge_amount(0.55), 55);
    assert_eq!(charge_amount(1.0), 100);
    // The pending scale a horse fires with: 40% at nothing held, full at 90.
    assert!((pending_scale(0) - 0.4).abs() < 1.0e-7);
    assert!((pending_scale(45) - 0.6).abs() < 1.0e-7);
    assert!((pending_scale(90) - 1.0).abs() < 1.0e-7);
}

#[test]
fn a_released_jump_fires_from_the_ground_with_the_forward_push() {
    let world = TestWorld::flat(0, states::STONE);
    let shapes = TestShapes::default();
    let e = env(&world, &shapes);
    let mut h = settled(DVec3::new(0.5, 0.0, 0.5));
    // A 45% charge: the horse fires with the pending scale 0.6.
    h.on_player_jump(45);
    assert!((h.player_jump_pending_scale - 0.6).abs() < 1.0e-7);
    // Riding forward while releasing.
    let forward = InputState {
        forward: true,
        ..Default::default()
    };
    tick_keys(&mut h, &e, &forward, (0.0, 0.0));
    // executeRidersJump fired on the ground: y = jumpStrength * scale (the scale is f32 in
    // vanilla, so its 0.6 is 0.60000002384...), plus the 0.4 forward push scaled by the charge
    // along the yaw. Gravity and the drags apply after the move.
    let scale = f64::from(pending_scale(45));
    let impulse = DEFAULT_JUMP_STRENGTH * scale;
    assert!(
        (h.delta.y - (impulse - DEFAULT_GRAVITY) * VERTICAL_DRAG as f64).abs() < 1.0e-9,
        "jump impulse: {}",
        h.delta.y
    );
    // The forward delta is the jump push plus this tick's walk acceleration (tickRidden's jump
    // fires before travel adds it), all after the drag. Both are f32 sums in vanilla; the
    // assertion tolerance covers the f32 rounding of the sum.
    let pushed = f64::from(JUMP_PUSH) * scale;
    let walk = forward_acceleration();
    assert!(
        (h.delta.z - (pushed + walk) * drag()).abs() < 1.0e-7,
        "forward push: {}",
        h.delta.z
    );
    assert!(!h.on_ground, "the jump lifted it off the ground");
    // The charge is spent.
    assert_eq!(h.player_jump_pending_scale, 0.0);
    assert!((h.pos.y - impulse).abs() < 1.0e-9, "y: {}", h.pos.y);
}

#[test]
fn the_jump_arc_follows_gravity_and_the_vertical_drag() {
    let world = TestWorld::flat(0, states::STONE);
    let shapes = TestShapes::default();
    let e = env(&world, &shapes);
    let mut h = settled(DVec3::new(0.5, 0.0, 0.5));
    // Full charge: the impulse is the jump strength itself.
    h.on_player_jump(100);
    let still = InputState::default();
    tick_keys(&mut h, &e, &still, (0.0, 0.0));
    // The jump tick moved the horse up by the impulse; gravity and the drag apply after the
    // move, so the delta that survives is (impulse - gravity) * 0.98.
    let mut y = DEFAULT_JUMP_STRENGTH;
    let mut dy = (DEFAULT_JUMP_STRENGTH - DEFAULT_GRAVITY) * VERTICAL_DRAG as f64;
    for t in 0..10 {
        tick_keys(&mut h, &e, &still, (0.0, 0.0));
        y += dy;
        dy = (dy - DEFAULT_GRAVITY) * VERTICAL_DRAG as f64;
        assert!(
            (h.pos.y - y).abs() < 1.0e-9,
            "tick {t}: y {} vs {}",
            h.pos.y,
            y
        );
    }
    assert!(y > 0.0, "the horse should still be in the air: {y}");
}

#[test]
fn the_horse_turns_with_the_rider_and_halves_the_pitch() {
    let world = TestWorld::flat(0, states::STONE);
    let shapes = TestShapes::default();
    let e = env(&world, &shapes);
    let mut h = settled(DVec3::new(0.5, 0.0, 0.5));
    let forward = InputState {
        forward: true,
        ..Default::default()
    };
    // The rider looks 90 degrees left with 20 degrees pitch: the horse faces the same yaw and
    // half the pitch, and walks along its own new facing.
    tick_keys(&mut h, &e, &forward, (90.0, 20.0));
    assert_eq!(h.yaw, 90.0);
    assert_eq!(h.pitch, 10.0);
    assert!(h.delta.x < 0.0, "walked west: {}", h.delta.x);
    assert!(h.delta.z.abs() < 1.0e-9, "z drifted: {}", h.delta.z);
}

#[test]
fn backwards_and_sideways_inputs_are_scaled_down() {
    let world = TestWorld::flat(0, states::STONE);
    let shapes = TestShapes::default();
    let e = env(&world, &shapes);
    // Backwards: the impulse is a quarter of the forward one.
    let mut back = settled(DVec3::new(0.5, 0.0, 0.5));
    let back_input = InputState {
        back: true,
        ..Default::default()
    };
    tick_keys(&mut back, &e, &back_input, (0.0, 0.0));
    assert!(back.delta.z < 0.0, "walked backwards: {}", back.delta.z);
    let mut fwd = settled(DVec3::new(0.5, 0.0, 0.5));
    let forward = InputState {
        forward: true,
        ..Default::default()
    };
    tick_keys(&mut fwd, &e, &forward, (0.0, 0.0));
    assert!(
        (back.delta.z + fwd.delta.z / 4.0).abs() < 1.0e-9,
        "backwards {} vs forward {}/4",
        back.delta.z,
        fwd.delta.z
    );
    // Sideways: half the strafe, perpendicular to the yaw (left is +x at yaw 0).
    let mut side = settled(DVec3::new(0.5, 0.0, 0.5));
    let left = InputState {
        left: true,
        ..Default::default()
    };
    tick_keys(&mut side, &e, &left, (0.0, 0.0));
    assert!(side.delta.x > 0.0, "walked left: {}", side.delta.x);
    assert!(side.delta.z.abs() < 1.0e-9, "z drifted: {}", side.delta.z);
    // Half the strafe of a forward walk's push: the same acceleration, half the input.
    let mut right = settled(DVec3::new(0.5, 0.0, 0.5));
    let right_input = InputState {
        right: true,
        ..Default::default()
    };
    tick_keys(&mut right, &e, &right_input, (0.0, 0.0));
    assert!(
        (side.delta.x + right.delta.x).abs() < 1.0e-9,
        "left {} mirrors right {}",
        side.delta.x,
        right.delta.x
    );
    assert!(
        (side.delta.x - fwd.delta.z / 2.0).abs() < 1.0e-9,
        "sideways {} vs forward {}/2",
        side.delta.x,
        fwd.delta.z
    );
}

#[test]
fn a_rearing_horse_stands_still_until_the_stand_wears_off() {
    let world = TestWorld::flat(0, states::STONE);
    let shapes = TestShapes::default();
    let e = env(&world, &shapes);
    let forward = InputState {
        forward: true,
        ..Default::default()
    };
    // A charged jump rears the horse up and fires from the ground anyway
    // (allowStandSliding: the pending scale is what matters, not the stand).
    let mut jumping = settled(DVec3::new(0.5, 0.0, 0.5));
    jumping.on_player_jump(0);
    assert!(jumping.stand_counter > 0);
    tick_keys(&mut jumping, &e, &forward, (0.0, 0.0));
    assert!(!jumping.on_ground, "the jump fired");
    // A rear-up without a charge blocks the input while it lasts. AbstractHorse.tick counts the
    // stand down after super.tick's ridden travel, so all twenty ticks hold the horse and the
    // one after walks.
    let mut rearing = settled(DVec3::new(0.5, 0.0, 0.5));
    rearing.stand_counter = STAND_TICKS;
    rearing.player_jump_pending_scale = 0.0;
    rearing.allow_stand_sliding = false;
    for t in 0..STAND_TICKS {
        tick_keys(&mut rearing, &e, &forward, (0.0, 0.0));
        assert!(
            rearing.pos.z < 0.5 + 1.0e-9,
            "tick {t}: the rear-up held it at {}",
            rearing.pos.z
        );
    }
    // The tick after the rear-up walks again.
    tick_keys(&mut rearing, &e, &forward, (0.0, 0.0));
    assert!(
        rearing.pos.z > 0.5,
        "walked after the rear-up: {}",
        rearing.pos.z
    );
}

#[test]
fn the_horse_types_this_module_steers() {
    assert!(is_horse_type("horse"));
    assert!(is_horse_type("donkey"));
    assert!(is_horse_type("mule"));
    assert!(is_horse_type("skeleton_horse"));
    assert!(is_horse_type("zombie_horse"));
    assert!(!is_horse_type("camel"));
    assert!(!is_horse_type("llama"));
    assert!(!is_horse_type("pig"));
    assert!(!is_horse_type("strider"));
    assert!(!is_horse_type("oak_boat"));
}

#[test]
fn the_server_correction_snaps_the_horse() {
    let world = TestWorld::flat(0, states::STONE);
    let shapes = TestShapes::default();
    let e = env(&world, &shapes);
    let mut h = settled(DVec3::new(0.5, 0.0, 0.5));
    let forward = InputState {
        forward: true,
        ..Default::default()
    };
    tick_keys(&mut h, &e, &forward, (0.0, 0.0));
    h.correct(DVec3::new(10.0, 0.0, 10.0), 180.0);
    assert_eq!(h.pos, DVec3::new(10.0, 0.0, 10.0));
    assert_eq!(h.yaw, 180.0);
}
