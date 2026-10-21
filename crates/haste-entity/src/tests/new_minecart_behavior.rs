//! The new minecart behaviour's track walk (`NewMinecartBehavior.lerpClientPositionAndRotation`
//! and `getCurrentLerpStep`, 26.3 source), checked against vanilla's formulas worked by hand:
//! each window of steps is spread over three ticks, `alpha = (3 - lerpDelay + partial) / 3` of
//! the window's total weight is walked, and the step that share falls in is entered from the
//! step before it (`oldLerp` for the first).
use super::*;
use crate::new_minecart_behavior::{self as new_cart, MinecartStep, NewMinecartBehavior};

/// Float-rounding closeness: vanilla mixes float shares with double positions.
fn near(got: f64, want: f64) {
    assert!((got - want).abs() < 1e-5, "{got} is not {want}");
}

fn near_pos(got: DVec3, want: DVec3) {
    assert!((got - want).length() < 1e-5, "{got:?} is not {want:?}");
}

fn near_f(got: f32, want: f32) {
    assert!((got - want).abs() < 1e-4, "{got} is not {want}");
}

/// The cart's pose before the window: (0, 64, 0), turned to 350 degrees.
fn start() -> MinecartStep {
    MinecartStep {
        position: DVec3::new(0.0, 64.0, 0.0),
        movement: DVec3::ZERO,
        y_rot: 350.0,
        x_rot: 0.0,
        weight: 0.0,
    }
}

/// A corner: one block east (weight 1), then three blocks south pitched down (weight 3). The
/// pitch is 315 degrees, which is how the wire's rotation byte decodes -45.
fn corner() -> [MinecartStep; 2] {
    [
        MinecartStep {
            position: DVec3::new(1.0, 64.0, 0.0),
            movement: DVec3::new(0.4, 0.0, 0.0),
            y_rot: 10.0,
            x_rot: 0.0,
            weight: 1.0,
        },
        MinecartStep {
            position: DVec3::new(1.0, 64.0, 3.0),
            movement: DVec3::new(0.0, 0.0, 0.4),
            y_rot: 100.0,
            x_rot: 315.0,
            weight: 3.0,
        },
    ]
}

#[test]
fn a_window_is_walked_by_weight_over_three_ticks() {
    let mut b = NewMinecartBehavior::new();
    assert!(!b.cart_has_pos_rot_lerp());
    b.add_lerp_steps(corner());
    // Tick 1 opens the window from the cart's pose and walks a third of the total weight 4:
    // 1.333 falls in the second step, 0.111 of the way from the first.
    let pose = b.lerp_client_position_and_rotation(start()).unwrap();
    assert!(b.cart_has_pos_rot_lerp());
    near_pos(pose.position, DVec3::new(1.0, 64.0, 1.0 / 3.0));
    near_pos(pose.movement, DVec3::new(0.4 * 8.0 / 9.0, 0.0, 0.4 / 9.0));
    near_f(pose.y_rot, 20.0);
    near_f(pose.x_rot, -5.0);
    // Between tick 1 and 2 the frame walks the first sixth: partial 0 is the start pose,
    // partial 0.5 two thirds into the first step (the yaw wraps from 350 through 360), and
    // partial 1 the tick's own end pose.
    near_pos(b.cart_lerp_position(0.0), start().position);
    near_f(b.cart_lerp_y_rot(0.0), 350.0);
    near_pos(b.cart_lerp_position(0.5), DVec3::new(2.0 / 3.0, 64.0, 0.0));
    near_f(b.cart_lerp_y_rot(0.5), 350.0 + 20.0 * 2.0 / 3.0);
    near_f(b.cart_lerp_x_rot(0.5), 0.0);
    near_pos(b.cart_lerp_position(1.0), pose.position);
    // Tick 2: two thirds walked (2.667 of 4), 0.556 into the second step; its frames at
    // partial 0.5 are half way (2 of 4), a third into the second step.
    let pose = b.lerp_client_position_and_rotation(pose).unwrap();
    near_pos(pose.position, DVec3::new(1.0, 64.0, 3.0 * 5.0 / 9.0));
    near_f(pose.y_rot, 10.0 + 90.0 * 5.0 / 9.0);
    near_f(pose.x_rot, -45.0 * 5.0 / 9.0);
    near_pos(b.cart_lerp_position(0.5), DVec3::new(1.0, 64.0, 1.0));
    near_f(b.cart_lerp_y_rot(0.5), 40.0);
    near_f(b.cart_lerp_x_rot(0.5), -15.0);
    // Tick 3 lands on the last step (the pitch comes out as -45: `rotLerp` takes the short
    // way from 0 to 315).
    let pose = b.lerp_client_position_and_rotation(pose).unwrap();
    near_pos(pose.position, DVec3::new(1.0, 64.0, 3.0));
    near_pos(pose.movement, DVec3::new(0.0, 0.0, 0.4));
    near_f(pose.y_rot, 100.0);
    near_f(pose.x_rot, -45.0);
    // Its frame at partial 0.25 is three quarters through (3 of 4), two thirds into `B`.
    near_pos(b.cart_lerp_position(0.25), DVec3::new(1.0, 64.0, 2.0));
    // Tick 4: the window is over and nothing new arrived, so the walk stops.
    assert_eq!(b.lerp_client_position_and_rotation(pose), None);
    assert!(!b.cart_has_pos_rot_lerp());
}

#[test]
fn steps_arriving_mid_window_wait_for_the_next_one() {
    let mut b = NewMinecartBehavior::new();
    b.add_lerp_steps(corner());
    let mut pose = b.lerp_client_position_and_rotation(start()).unwrap();
    // A packet during the window: its step is walked only once this window ends.
    let next = MinecartStep {
        position: DVec3::new(1.0, 64.0, 4.0),
        movement: DVec3::new(0.0, 0.0, 0.4),
        y_rot: 100.0,
        x_rot: 0.0,
        weight: 1.0,
    };
    b.add_lerp_steps([next]);
    pose = b.lerp_client_position_and_rotation(pose).unwrap();
    near_pos(pose.position, DVec3::new(1.0, 64.0, 3.0 * 5.0 / 9.0));
    pose = b.lerp_client_position_and_rotation(pose).unwrap();
    near_pos(pose.position, DVec3::new(1.0, 64.0, 3.0));
    // The new window starts where the cart is (`setOldLerpValues`) and walks a third.
    pose = b.lerp_client_position_and_rotation(pose).unwrap();
    near_pos(pose.position, DVec3::new(1.0, 64.0, 3.0 + 1.0 / 3.0));
    near_pos(b.cart_lerp_position(0.0), DVec3::new(1.0, 64.0, 3.0));
    // Its partial-0 rotation is the old window's end, -45 pitch included.
    near_f(b.cart_lerp_x_rot(0.0), -45.0);
}

#[test]
fn a_weightless_window_snaps_and_ends_next_tick() {
    // `adjustToRails(..., instant)` sends weight-0 steps: the window gets no ticks
    // (`lerpDelay = 0`), the walk passes over every weightless step and rests on the last,
    // entered from the one before it.
    let mut b = NewMinecartBehavior::new();
    let snap = |x: f64, yaw: f32| MinecartStep {
        position: DVec3::new(x, 64.0, 5.0),
        movement: DVec3::ZERO,
        y_rot: yaw,
        x_rot: 0.0,
        weight: 0.0,
    };
    b.add_lerp_steps([snap(4.0, 45.0), snap(5.0, 90.0)]);
    let pose = b.lerp_client_position_and_rotation(start()).unwrap();
    near_pos(pose.position, DVec3::new(5.0, 64.0, 5.0));
    near_f(pose.y_rot, 90.0);
    // Every partial tick shows the last step.
    near_pos(b.cart_lerp_position(0.0), DVec3::new(5.0, 64.0, 5.0));
    assert_eq!(b.lerp_client_position_and_rotation(pose), None);
}

#[test]
fn a_zero_weight_step_inside_a_window_is_the_next_steps_start() {
    // `getCurrentLerpStep` skips weightless steps when counting, but the step it lands in
    // is still entered from the list entry before it, weightless or not.
    let mut b = NewMinecartBehavior::new();
    let [a, mut c] = corner();
    let snap = MinecartStep {
        position: DVec3::new(2.0, 64.0, 0.0),
        weight: 0.0,
        ..a
    };
    c.position = DVec3::new(2.0, 64.0, 3.0);
    b.add_lerp_steps([a, snap, c]);
    b.lerp_client_position_and_rotation(start()).unwrap();
    // Partial 0.5 after tick 2: half of weight 4 is 2, a third into `c`, from `snap`.
    b.lerp_client_position_and_rotation(start()).unwrap();
    near_pos(b.cart_lerp_position(0.5), DVec3::new(2.0, 64.0, 1.0));
}

/// A store with one entity (id 1) running the new behaviour, at the start pose.
fn cart_store() -> EntityStore {
    let mut s = store();
    s.spawn(&EntitySpawn {
        yaw: 350.0,
        ..spawn(1, start().position)
    });
    s.get_mut(EntityId(1))
        .unwrap()
        .start_new_minecart_behavior();
    s
}

#[test]
fn the_tick_moves_the_cart_and_brackets_the_old_position() {
    let mut s = cart_store();
    s.handle_minecart_along_track(EntityId(1), corner());
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    near_pos(e.prev_pos, start().position);
    near_pos(e.pos, DVec3::new(1.0, 64.0, 1.0 / 3.0));
    near_pos(e.velocity, DVec3::new(0.4 * 8.0 / 9.0, 0.0, 0.4 / 9.0));
    near_f(e.yaw, 20.0);
    near_f(e.pitch, -5.0);
    // The render state at a partial tick is the walk's (not the straight line between the
    // two tick positions).
    let (pos, y_rot, x_rot) = new_cart::render_pose(e, 0.5).unwrap();
    near_pos(pos, DVec3::new(2.0 / 3.0, 64.0, 0.0));
    near_f(y_rot, 350.0 + 20.0 * 2.0 / 3.0);
    near_f(x_rot, 0.0);
    s.tick();
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    near_pos(e.prev_pos, DVec3::new(1.0, 64.0, 3.0 * 5.0 / 9.0));
    near_pos(e.pos, DVec3::new(1.0, 64.0, 3.0));
    // After the window: the plain interpolated position and the present rotation.
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    let (pos, y_rot, x_rot) = new_cart::render_pose(e, 0.5).unwrap();
    near_pos(pos, DVec3::new(1.0, 64.0, 3.0));
    near_f(y_rot, 100.0);
    near_f(x_rot, -45.0);
}

#[test]
fn packets_set_a_new_carts_position_at_once() {
    // `InterpolationHandler.NO_OP`: no three-tick easing for a cart on the new behaviour.
    let mut s = cart_store();
    s.teleport(EntityId(1), DVec3::new(2.0, 64.0, 0.0), 45.0, 10.0, true);
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(e.pos, DVec3::new(2.0, 64.0, 0.0));
    assert_eq!(e.prev_pos, start().position);
    assert_eq!((e.yaw, e.pitch), (45.0, 10.0));
    s.move_delta(EntityId(1), DVec3::new(1.0, 0.0, 0.0), true);
    s.set_rotation(EntityId(1), 90.0, 0.0, true);
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(e.pos, DVec3::new(3.0, 64.0, 0.0));
    assert_eq!((e.yaw, e.pitch), (90.0, 0.0));
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(e.prev_pos, DVec3::new(3.0, 64.0, 0.0));
    assert_eq!(e.pos, DVec3::new(3.0, 64.0, 0.0));
    // Any other entity still eases a teleport over three ticks.
    s.spawn(&spawn(2, DVec3::ZERO));
    s.teleport(EntityId(2), DVec3::new(3.0, 0.0, 0.0), 0.0, 0.0, true);
    assert_eq!(s.get(EntityId(2)).unwrap().pos, DVec3::ZERO);
    s.tick();
    near(s.get(EntityId(2)).unwrap().pos.x, 1.0);
}

#[test]
fn steps_for_entities_without_the_new_behaviour_are_ignored() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.handle_minecart_along_track(EntityId(1), corner());
    // Unknown ids too.
    s.handle_minecart_along_track(EntityId(7), corner());
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(e.pos, DVec3::ZERO);
    assert!(new_cart::render_pose(e, 0.5).is_none());
    // `use_new_minecart_behavior` only takes minecart types (kind 0 is none).
    s.use_new_minecart_behavior(EntityId(1));
    assert!(s.get(EntityId(1)).unwrap().new_minecart.is_none());
}

#[test]
fn passengers_ride_along_the_walk() {
    // `EntityRenderer.extractRenderState`'s `passengerOffset`: the walk's point minus the
    // cart's own interpolated position.
    let mut s = cart_store();
    s.spawn(&spawn(2, DVec3::new(0.0, 65.0, 0.0)));
    s.set_passengers(EntityId(1), &[EntityId(2)]);
    s.handle_minecart_along_track(EntityId(1), corner());
    s.tick();
    let cart = s.get(EntityId(1)).unwrap();
    let offset = new_cart::passenger_offset(cart, 0.5).unwrap();
    // Walk (2/3, 64, 0) minus the straight half way to (1, 64, 1/3).
    near_pos(offset, DVec3::new(2.0 / 3.0 - 0.5, 0.0, -1.0 / 6.0));
    let mut out = Vec::new();
    s.render_instances(0.5, &mut out);
    let passenger = out.iter().find(|i| i.id == EntityId(2)).unwrap();
    near_pos(passenger.pos, DVec3::new(0.0, 65.0, 0.0) + offset);
    // Once the walk is over there is no offset.
    for _ in 0..3 {
        s.tick();
    }
    assert!(new_cart::passenger_offset(s.get(EntityId(1)).unwrap(), 0.5).is_none());
}

#[test]
fn new_render_turns_pitches_then_lifts() {
    // `AbstractMinecartRenderer.newRender`: Y by yRot, Z by -xRot, then up 0.375.
    let body = new_cart::new_render(90.0, 30.0);
    let ops: Vec<BodyOp> = body.iter().copied().collect();
    assert_eq!(
        ops,
        vec![
            BodyOp::RotateY(90f32.to_radians()),
            BodyOp::RotateZ((-30f32).to_radians()),
            BodyOp::Translate(haste_core::math::Vec3::new(0.0, 0.375, 0.0)),
        ]
    );
}
