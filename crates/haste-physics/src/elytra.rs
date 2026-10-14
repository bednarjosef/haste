//! Elytra flight (gliding).
//!
//! - [`glide_velocity`] is vanilla `LivingEntity.updateFallFlyingMovement`: the look vector, the
//!   lift that comes out of the pitch, the vertical-to-horizontal exchange while descending, the
//!   extra push when the pitch is up, the turn towards the look direction, and the drag
//!   0.99 / 0.98 / 0.99.
//! - [`travel`] is vanilla `LivingEntity.travelFallFlying`: that update and then the move/collide
//!   step. Its second half (`handleFallFlyingCollisions`) is server-side damage and is not
//!   modelled; `updateFallFlying`'s durability loss is server-side too.
//! - [`PlayerState::can_start_fall_flying`] is vanilla `Player.tryToStartFallFlying` under
//!   `LocalPlayer.aiStep`'s jump condition; [`PlayerState::should_stop_fall_flying`] is the
//!   `LivingEntity.updateFallFlying` stop check (`canGlide` no longer true).
//! - [`firework_boost`] is the velocity push vanilla's `FireworkRocketEntity.tick` applies to a
//!   fall-flying entity it is attached to, once per tick. The rocket entity itself (its
//!   attached-to id in entity data, its position following the player's hand) is the client's to
//!   track; the boost is applied from the client's tick (see `haste-client`'s net driver).
use haste_core::view::{BlockPhysics, BlockView, CollisionShapes};
use haste_core::DVec3;

use crate::movement::move_entity;
use crate::trig;
use crate::{PlayerAttributes, PlayerState};

/// Horizontal drag per glide tick (vanilla's 0.99 on the fall-flying multiply).
pub const GLIDE_HORIZONTAL_DRAG: f64 = 0.99;
/// Vertical drag per glide tick (vanilla's 0.98, the usual air drag).
pub const GLIDE_VERTICAL_DRAG: f64 = 0.98;
/// Gravity while falling under slow falling (vanilla clamps the gravity attribute to it).
pub const SLOW_FALLING_GRAVITY: f64 = 0.01;
/// Fall-flying pose box: 0.6 x 0.6 (vanilla's fall_flying pose, eye height 0.4).
pub const GLIDER_BOX_WIDTH: f32 = 0.6;
pub const GLIDER_BOX_HEIGHT: f32 = 0.6;

/// Look vector for yaw/pitch in degrees, vanilla `Entity.calculateViewVector`. f32 maths (the
/// table sine and cosine, then two f32 products per component), widened into the vector.
pub fn look_angle(yaw: f32, pitch: f32) -> DVec3 {
    let real_x_rot = trig::deg_to_rad_f32(pitch);
    let real_y_rot = -yaw * trig::DEG_TO_RAD_F32;
    let y_cos = trig::cos_d(real_y_rot as f64);
    let y_sin = trig::sin_d(real_y_rot as f64);
    let x_cos = trig::cos_d(real_x_rot as f64);
    let x_sin = trig::sin_d(real_x_rot as f64);
    DVec3::new(
        (y_sin * x_cos) as f64,
        (-x_sin) as f64,
        (y_cos * x_cos) as f64,
    )
}

/// One tick of the attached firework rocket's push, vanilla `FireworkRocketEntity.tick` while
/// the rocket is attached to a fall-flying entity:
///
/// ```text
/// v += look * 0.1 + (look * 1.5 - v) * 0.5        (per component)
/// ```
///
/// i.e. half the old velocity plus 0.85 x the look vector (`0.1 + 1.5 * 0.5`). All doubles, the
/// same order of operations as vanilla's `Vec3.add`. The caller applies this once per tick while
/// a firework rocket attached to the local player is tracked and `state.fall_flying` is set.
pub fn firework_boost(velocity: DVec3, yaw: f32, pitch: f32) -> DVec3 {
    let look = look_angle(yaw, pitch);
    DVec3::new(
        velocity.x + look.x * 0.1 + (look.x * 1.5 - velocity.x) * 0.5,
        velocity.y + look.y * 0.1 + (look.y * 1.5 - velocity.y) * 0.5,
        velocity.z + look.z * 0.1 + (look.z * 1.5 - velocity.z) * 0.5,
    )
}

/// One tick of fall-flying movement, vanilla `LivingEntity.updateFallFlyingMovement`.
///
/// `gravity` is the entity's effective gravity (`PlayerAttributes::gravity`, clamped to
/// [`SLOW_FALLING_GRAVITY`] while falling with slow falling); the player's own output goes
/// through [`effective_gravity`].
pub fn glide_velocity(mut velocity: DVec3, yaw: f32, pitch: f32, gravity: f64) -> DVec3 {
    let look = look_angle(yaw, pitch);
    // Vanilla's lean angle: the pitch in radians, kept in f32 for the table lookups.
    let lean_angle = trig::deg_to_rad_f32(pitch);
    let look_horizontal = (look.x * look.x + look.z * look.z).sqrt();
    let move_horizontal = (velocity.x * velocity.x + velocity.z * velocity.z).sqrt();
    let lift = {
        let c = (lean_angle as f64).cos();
        c * c
    };
    // 1. Gravity is almost cancelled while gliding: only a quarter of the lift is left of it.
    velocity.y += gravity * (-1.0 + lift * 0.75);
    // 2. Descending: part of the sinking speed is exchanged for speed along the look direction.
    if velocity.y < 0.0 && look_horizontal > 0.0 {
        let convert = velocity.y * -0.1 * lift;
        velocity.x += look.x * convert / look_horizontal;
        velocity.y += convert;
        velocity.z += look.z * convert / look_horizontal;
    }
    // 3. Pitch up: trading horizontal speed for height.
    if lean_angle < 0.0 && look_horizontal > 0.0 {
        let convert = move_horizontal * -f64::from(trig::sin_d(lean_angle as f64)) * 0.04;
        velocity.x += -look.x * convert / look_horizontal;
        velocity.y += convert * 3.2;
        velocity.z += -look.z * convert / look_horizontal;
    }
    // 4. The velocity is steered towards the look direction, keeping its length.
    if look_horizontal > 0.0 {
        velocity.x += (look.x / look_horizontal * move_horizontal - velocity.x) * 0.1;
        velocity.z += (look.z / look_horizontal * move_horizontal - velocity.z) * 0.1;
    }
    velocity.x *= GLIDE_HORIZONTAL_DRAG;
    velocity.y *= GLIDE_VERTICAL_DRAG;
    velocity.z *= GLIDE_HORIZONTAL_DRAG;
    velocity
}

/// Vanilla `LivingEntity.getEffectiveGravity`: slow falling only bites while sinking.
pub fn effective_gravity(attrs: &PlayerAttributes, velocity: DVec3, slow_falling: bool) -> f64 {
    if velocity.y <= 0.0 && slow_falling {
        attrs.gravity.min(SLOW_FALLING_GRAVITY)
    } else {
        attrs.gravity
    }
}

/// One glide tick of the travelling player, vanilla `LivingEntity.travelFallFlying`.
///
/// This is the call the movement tick makes when `state.fall_flying` is set (see NOTES.md).
/// `slow_falling` is the slow falling effect (no effect state on [`PlayerAttributes`] yet).
/// Nothing here reads the movement input: gliding is steered with the look angles only.
pub fn travel(
    state: &mut PlayerState,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    blocks: &dyn BlockPhysics,
    slow_falling: bool,
) {
    // Client half of LivingEntity.updateFallFlying: while gliding the fall distance stops
    // climbing, so a glide never turns into a fall.
    if state.velocity.y > -0.5 && state.fall_distance > 1.0 {
        state.fall_distance = 1.0;
    }
    // A climbable (ladder, vines) takes the air branch and ends the glide in vanilla; the caller
    // of this function must not dispatch here in that case (LivingEntity.onClimbable).
    let gravity = effective_gravity(attrs, state.velocity, slow_falling);
    state.velocity = glide_velocity(state.velocity, state.yaw, state.pitch, gravity);
    // The glide never reads the sneak input, so the sneak edge back-off of the shared move step
    // does not apply to it.
    let sneaking = state.sneaking;
    state.sneaking = false;
    move_entity(state, attrs, state.velocity, world, shapes, blocks);
    state.sneaking = sneaking;
    // Landing ends the glide (the server clears its shared flag; the client follows the server).
    if state.on_ground {
        state.stop_fall_flying();
    }
}

impl PlayerState {
    /// Vanilla `Player.tryToStartFallFlying`: not already gliding, a glider item, not in a
    /// liquid. `has_glider` is the item check (`LivingEntity.canGlideUsing`: a glider component
    /// in the matching equippable slot, not about to break), which is the caller's to answer.
    pub fn can_start_fall_flying(&self, has_glider: bool) -> bool {
        !self.fall_flying && self.can_glide(has_glider) && !self.in_water && !self.in_lava
    }

    /// Vanilla `LivingEntity.canGlide` (with `Player`'s `!abilities.flying`): the player must be
    /// in the air and carry a glider. Being a passenger and having levitation are the caller's
    /// checks (`PlayerState` has no field for them yet).
    pub fn can_glide(&self, has_glider: bool) -> bool {
        has_glider && !self.flying && !self.on_ground
    }

    /// Whether gliding must end now: vanilla `LivingEntity.updateFallFlying` clears the flag once
    /// `canGlide` fails (landed, no glider left) and `travelFallFlying` ends it on a climbable
    /// (the caller's check on the block state). Entering a fluid only switches the player to swim
    /// physics (vanilla's `travel` takes the fluid branch before the fall-flying one), so it is
    /// listed here for the client's flag bookkeeping, not for the move.
    pub fn should_stop_fall_flying(&self, has_glider: bool) -> bool {
        !self.can_glide(has_glider) || self.in_water || self.in_lava
    }

    /// Vanilla `Player.startFallFlying` (shared flag 7). The player sees the fall-flying pose's
    /// box while this is set (see `PlayerState::aabb`).
    pub fn start_fall_flying(&mut self) {
        self.fall_flying = true;
    }

    /// Vanilla `LivingEntity.stopFallFlying`.
    pub fn stop_fall_flying(&mut self) {
        self.fall_flying = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{states::*, TestShapes, TestWorld};
    use crate::Pose;
    use haste_core::{Aabb, DVec3};

    /// The level-glide case (pitch 0) only needs the exact table entries 0 and 16384, so it is
    /// compared to 1e-9. The 60 and -30 degree cases use table entries up to 5e-5 away from the
    /// ideal sine (the table has one entry per 9.587e-5 rad), and the hand arithmetic in the
    /// comments below is carried to five or six digits, so the multi-tick numbers get the looser
    /// tolerances named per assertion.
    const TOL: f64 = 1e-9;
    /// Gravity in blocks per tick squared (wiki "Attribute": gravity 0.08).
    const GRAVITY: f64 = 0.08;

    fn gliding(pos: DVec3, velocity: DVec3, pitch: f32) -> PlayerState {
        let mut state = PlayerState::at(pos);
        state.velocity = velocity;
        state.pitch = pitch;
        state.start_fall_flying();
        state
    }

    /// `n` ticks of the pure velocity update, yaw 0 (looking towards +z), no world in the way.
    fn glide(velocity: DVec3, pitch: f32, n: usize) -> DVec3 {
        glide_with(velocity, pitch, n, GRAVITY)
    }

    fn glide_with(velocity: DVec3, pitch: f32, n: usize, gravity: f64) -> DVec3 {
        let mut v = velocity;
        for _ in 0..n {
            v = glide_velocity(v, 0.0, pitch, gravity);
        }
        v
    }

    /// Level glide (pitch 0): height is lost slowly, forward speed builds up.
    ///
    /// Hand-computed tick 1 from v = (0, 0, 0). Yaw 0 and pitch 0 give look = (0, -0, 1) exactly
    /// (table entries 0 and 16384 are exact), so look_horizontal = 1 and lift = cos(0)^2 = 1:
    ///   gravity: v.y = 0 + 0.08 * (-1 + 1 * 0.75)      = -0.02
    ///   descent exchange: convert = (-0.02 * -0.1) * 1 =  0.002
    ///                    v.y = -0.02 + 0.002           = -0.018
    ///                    v.z = 0 + 1 * 0.002 / 1       =  0.002
    ///   steering (move_horizontal = 0): v.z += (1 * 0 - 0.002) * 0.1 = -0.0002 -> 0.0018
    ///   drag (0.99, 0.98, 0.99): (0, -0.01764, 0.001782)
    /// Ticks 2..5 repeat those lines. The sink rate follows v.y <- 0.882 * v.y - 0.01764 (the
    /// 0.88 is the 0.9 left of the exchange, the 0.98 the vertical drag) and the forward speed
    /// the same three lines with the steering pulling it up. To six digits:
    ///   tick 1: (0, -0.01764000, 0.00178200)
    ///   tick 2: (0, -0.03319848, 0.005117904)
    ///   tick 3: (0, -0.04692106, 0.009806710)
    ///   tick 4: (0, -0.05902437, 0.015671309)
    ///   tick 5: (0, -0.06969950, 0.022555667)
    #[test]
    fn level_glide_loses_height_slowly() {
        let first = glide(DVec3::ZERO, 0.0, 1);
        assert!(first.x.abs() < TOL);
        assert!((first.y + 0.01764).abs() < TOL, "{}", first.y);
        assert!((first.z - 0.001782).abs() < TOL, "{}", first.z);

        let v5 = glide(DVec3::ZERO, 0.0, 5);
        assert!((v5.y + 0.069_699_50).abs() < 1e-6, "{}", v5.y);
        assert!((v5.z - 0.022_555_67).abs() < 1e-6, "{}", v5.z);
        // Five ticks lose 0.22648341 blocks of height (the five sink rates above added up), and
        // the last one is still under 7 cm per tick.
        let dropped: f64 = (0..5)
            .scan(DVec3::ZERO, |v, _| {
                *v = glide_velocity(*v, 0.0, 0.0, GRAVITY);
                Some(v.y)
            })
            .sum();
        assert!((dropped + 0.226_483_41).abs() < 1e-6, "{dropped}");
        assert!(v5.y.abs() < 0.07, "{v5}");
        // Stable, not terminal: the fixed point of v.y <- 0.882 * v.y - 0.01764 is
        // -0.01764 / (1 - 0.882) = -0.14949 b/t, and 60 ticks get to within 1e-4 of it.
        let v60 = glide(DVec3::ZERO, 0.0, 60);
        assert!((v60.y + 0.149_49).abs() < 1e-3, "{}", v60.y);
        // Forward speed keeps growing while the sink rate settles.
        assert!(v5.z > first.z * 5.0, "{} {}", v5.z, first.z);
    }

    /// A dive (pitch 60, yaw 0) speeds up.
    ///
    /// Table values for lean = 1.0471976 rad (index 10922): sin 0.8659934, cos 0.5000554, so
    /// look = (0, -0.8659934, 0.5000554), look_horizontal = 0.5000554 and look_z /
    /// look_horizontal = 1. lift = cos(lean)^2 = 0.24999997 (0.25, less 2.5e-8 of double
    /// rounding). Hand-computed tick 1 from v = (0, -0.5, 1.0):
    ///   gravity: v.y = -0.5 + 0.08 * (-1 + 0.25 * 0.75) = -0.56500000
    ///   descent exchange: convert = (0.56500000 * 0.1) * 0.24999997 = 0.01412500
    ///                    v.y = -0.56500000 + 0.01412500 = -0.55087500
    ///                    v.z = 1.0 + 1 * 0.01412500     = 1.01412500
    ///   steering (move_horizontal = 1.0): v.z += (1.0 - 1.01412500) * 0.1 = -0.00141250
    ///                    v.z = 1.01271250
    ///   drag: (0, -0.53985750, 1.00258537)
    /// The y component does not depend on z here, so its recurrence v.y <- 0.98 * 0.975 * (v.y -
    /// 0.065) can be iterated by hand; its fixed point is -1.39567 b/t. Continuing:
    ///   tick 5:  (0, -0.6823228, 1.0210764)   sink +36 %, forward +2 %
    ///   tick 10: (0, -0.8275321, 1.0593130)
    ///   tick 15: (0, -0.9431827, 1.1101653)
    ///   tick 20: (0, -1.0352916, 1.1700655)  two thirds of the way to the fixed point
    #[test]
    fn diving_speeds_up() {
        let start = DVec3::new(0.0, -0.5, 1.0);
        let first = glide(start, 60.0, 1);
        assert!((first.y + 0.539_857_50).abs() < 1e-8, "{}", first.y);
        assert!((first.z - 1.002_585_37).abs() < 1e-8, "{}", first.z);

        let v5 = glide(start, 60.0, 5);
        assert!((v5.y + 0.682_322_8).abs() < 1e-6, "{}", v5.y);
        assert!((v5.z - 1.021_076_4).abs() < 1e-6, "{}", v5.z);
        let v20 = glide(start, 60.0, 20);
        assert!((v20.y + 1.035_291_6).abs() < 1e-6, "{}", v20.y);
        assert!((v20.z - 1.170_065_5).abs() < 5e-4, "{}", v20.z);

        // The same start gliding level sinks harder at first but the sink rate decays towards
        // -0.14949 b/t (the level-glide fixed point), while the dive keeps accelerating; after 20
        // ticks the dive is both sinking harder and moving faster overall.
        let level20 = glide(start, 0.0, 20);
        assert!(v5.y < level20.y, "{v5} vs {level20}");
        assert!(v20.y < level20.y, "{v20} vs {level20}");
        assert!(v20.length() > level20.length(), "{v20} vs {level20}");
        // Speed (the length of the velocity) grows through the dive.
        assert!(v5.length() > start.length() + 0.1, "{v5}");
        assert!(v20.length() > v5.length() + 0.1, "{v20}");
    }

    /// Pulling up out of a dive (pitch -30) trades speed for height.
    ///
    /// Table values for lean = -0.5235988 rad (index 60075 for the sine): sin -0.4999723,
    /// cos 0.8660414, so look = (0, 0.4999723, 0.8660414) (looking up and forward),
    /// look_horizontal = 0.8660414 and look_z / look_horizontal = 1. lift = 0.74999999.
    /// Hand-computed tick 1 from the tick-5 dive state (0, -0.68232, 1.02108):
    ///   gravity: v.y = -0.68232 + 0.08 * (-1 + 0.75 * 0.75) = -0.71732
    ///   descent exchange: convert = (0.71732 * 0.1) * 0.74999999 = 0.05379921
    ///                    v.y = -0.71732 + 0.05379921  = -0.66352359
    ///                    v.z = 1.02108 + 0.05379921   = 1.07487921
    ///   pull-up:  convert = 1.02108 * 0.4999723 * 0.04 = 0.02042040
    ///       v.y += 3.2 * 0.02042040 = 0.06534527  -> -0.59817832
    ///       v.z -= 0.02042040                    -> 1.05445522
    ///   steering: v.z += (1.02108 - 1.05445522) * 0.1 = -0.00333788 -> 1.05111734
    ///   drag: (0, -0.58621475, 1.04060616)
    /// Iterating those three lines by hand (the pull-up term is 3.2 * 0.04 * move_horizontal, so
    /// the climb rate settles at 0.98 * (0.925 * 0.035 + 0.0627 * move_horizontal) once the
    /// descent exchange stops firing):
    ///   tick 1:  (0, -0.5862, 1.0406)    tick 5:  (0, -0.2764, 1.0560)
    ///   tick 2:  (0, -0.4979, 1.0532)    tick 10: (0, -0.0360, 0.9815)
    ///   tick 3:  (0, -0.4170, 1.0595)    tick 20: (0,  0.1629, 0.7464)
    #[test]
    fn pulling_up_trades_speed_for_height() {
        let dived = glide(DVec3::new(0.0, -0.5, 1.0), 60.0, 5);
        let first_up = glide(dived, -30.0, 1);
        assert!((first_up.y + 0.586_214_75).abs() < 1e-8, "{}", first_up.y);
        assert!((first_up.z - 1.040_606_16).abs() < 1e-8, "{}", first_up.z);

        let up5 = glide(dived, -30.0, 5);
        assert!((up5.y + 0.276_422_8).abs() < 1e-4, "{}", up5.y);
        assert!((up5.z - 1.055_996_6).abs() < 1e-4, "{}", up5.z);

        let up20 = glide(dived, -30.0, 20);
        let keep20 = glide(dived, 60.0, 20);
        assert!((up20.y - 0.162_9).abs() < 5e-3, "{}", up20.y);
        assert!((up20.z - 0.746_4).abs() < 5e-3, "{}", up20.z);
        // Height gained: the descent has stopped and turned into a climb.
        assert!(up20.y > 0.1, "{up20}");
        assert!(up20.y > dived.y, "{up20} vs {dived}");
        // ... against the same player who kept the nose down two thirds of the way to the
        // -1.39567 b/t dive fixed point.
        assert!(keep20.y < dived.y - 0.3, "{keep20} vs {dived}");
        // The trade itself: what the climb gains, the forward speed pays for. A fifth of the
        // forward speed is gone at pitch -30, while the dive adds to it.
        assert!(up20.z < dived.z * 0.8, "{} vs {}", up20.z, dived.z);
        assert!(keep20.z > dived.z, "{keep20} vs {dived}");
        assert!(up20.z < keep20.z * 0.7, "{} vs {}", up20.z, keep20.z);
        // The pitch, not the time in the air, is what does it: holding level from the same
        // state sinks (0.065 b/t at pitch 0) and keeps more speed, but never climbs.
        let level20 = glide(dived, 0.0, 20);
        assert!(level20.y < up20.y, "{level20} vs {up20}");
        assert!(level20.z > up20.z, "{level20} vs {up20}");
    }

    /// Slow falling changes the gravity that goes into the glide (vanilla's effective gravity
    /// clamps it to 0.01 while sinking).
    #[test]
    fn slow_falling_softens_the_gravity() {
        let attrs = PlayerAttributes::default();
        let sinking = DVec3::new(0.0, -0.2, 0.0);
        let rising = DVec3::new(0.0, 0.2, 0.0);
        assert!((effective_gravity(&attrs, sinking, true) - 0.01).abs() < TOL);
        assert!((effective_gravity(&attrs, sinking, false) - 0.08).abs() < TOL);
        assert!((effective_gravity(&attrs, rising, true) - 0.08).abs() < TOL);

        // A level glide under slow falling sinks much slower: the gravity term becomes
        // 0.01 * (-0.25) instead of 0.08 * (-0.25), which by hand moves the sink-rate fixed point
        // from -0.01764 / (1 - 0.882) = -0.14949 b/t to -0.002205 / (1 - 0.882) = -0.01869 b/t.
        let slow = glide_with(DVec3::ZERO, 0.0, 60, 0.01);
        let normal = glide(DVec3::ZERO, 0.0, 60);
        assert!(slow.y.abs() < normal.y.abs() / 5.0, "{slow} vs {normal}");
        assert!((slow.y + 0.018_69).abs() < 1e-4, "{}", slow.y);
    }

    /// The drag is on every component (0.99 horizontal, 0.98 vertical).
    #[test]
    fn drag_applies_to_every_component() {
        // Pitch 90 (straight down): look_horizontal is one table entry (9.6e-5) away from zero,
        // so only gravity and drag act on v.
        let v = glide(DVec3::new(0.0, -0.3, 0.0), 90.0, 1);
        let expected_y = (-0.3 + 0.08 * (-1.0 + 0.0)) * 0.98;
        assert!((v.y - expected_y).abs() < 1e-6, "{} vs {}", v.y, expected_y);
        // Level from a horizontal start: the steering pulls the velocity back towards the look
        // direction (v.x = 0.4 - 0.1 * 0.4 = 0.36) and both horizontal components take the 0.99
        // drag, and v.z picks up the descent exchange routed along the look direction:
        // v.z = (0.002 - 0.0002) * 0.99 = 0.001782.
        let v2 = glide(DVec3::new(0.4, 0.0, 0.0), 0.0, 1);
        assert!((v2.x - 0.3564).abs() < 1e-9, "{}", v2.x);
        assert!((v2.y + 0.01764).abs() < 1e-9, "{}", v2.y);
        assert!((v2.z - 0.041382).abs() < 1e-6, "{}", v2.z);
    }

    /// The fall-flying box is 0.6 x 0.6, so a glide passes under a block that would stop a
    /// standing player.
    #[test]
    fn gliding_uses_the_small_box() {
        // Floor at y = 100, a ceiling block at y = 102, the player gliding one block above the
        // floor: the standing box (1.8) reaches into the ceiling, the glide box (0.6) does not.
        let mut world = TestWorld::flat(100, STONE);
        world.set(0, 102, 0, STONE);
        let shapes = TestShapes::default();
        let attrs = PlayerAttributes::default();
        let state = gliding(
            DVec3::new(0.5, 101.0, 0.5),
            DVec3::new(0.2, -0.2, 0.0),
            30.0,
        );
        let aabb = state.aabb(&attrs);
        assert!((aabb.max.y - state.pos.y - 0.6).abs() < 1e-6, "{aabb:?}");
        assert!((aabb.max.x - aabb.min.x - GLIDER_BOX_WIDTH as f64).abs() < 1e-6);

        let standing = Aabb::entity(
            state.pos,
            attrs.box_width(),
            attrs.height_for(Pose::Standing),
        );
        assert!(!crate::collision::no_collision(&standing, &world, &shapes));
        assert!(crate::collision::no_collision(&aabb, &world, &shapes));

        let mut state = state;
        travel(&mut state, &attrs, &world, &shapes, &shapes, false);
        assert!(
            !state.vertical_collision,
            "the small box went under the block"
        );
        assert!(state.fall_flying, "still gliding under the block");
        assert!(state.pos.y < 101.0 && state.pos.y > 100.0, "{}", state.pos);
        let _ = &mut world;
    }

    /// Landing ends the glide, and the fall distance never climbs above 1 while gliding.
    #[test]
    fn landing_ends_the_glide() {
        let world = TestWorld::flat(100, STONE);
        let shapes = TestShapes::default();
        let attrs = PlayerAttributes::default();
        let mut state = gliding(DVec3::new(0.5, 101.0, 0.5), DVec3::new(0.0, -0.5, 0.0), 0.0);
        state.on_ground = false;
        state.fall_distance = 40.0;
        for _ in 0..20 {
            if !state.fall_flying {
                break;
            }
            travel(&mut state, &attrs, &world, &shapes, &shapes, false);
        }
        assert!(state.fall_distance <= 1.0, "{}", state.fall_distance);
        assert!(state.on_ground, "{}", state.pos);
        assert!(!state.fall_flying, "still gliding after landing");
        assert!((state.pos.y - 100.0).abs() < 1e-9, "{}", state.pos);
    }

    /// The attached firework rocket's push, vanilla `FireworkRocketEntity.tick`: each tick
    /// `v = 0.5 v + 0.85 look` (per component), so from rest the speed reaches the fixed point
    /// 0.85 / (1 - 0.5) = 1.7 blocks/tick along the look vector.
    #[test]
    fn firework_boost_pushes_towards_the_look() {
        // Yaw 0, pitch 0: look = (0, 0, 1) exactly. From rest the boost is (0, 0, 0.85).
        let first = firework_boost(DVec3::ZERO, 0.0, 0.0);
        assert!(first.x.abs() < TOL, "{first}");
        assert!(first.y.abs() < TOL, "{first}");
        assert!((first.z - 0.85).abs() < TOL, "{first}");
        // v_{n+1} = 0.5 v_n + 0.85: 0.85, 1.275, 1.4875, ... -> 1.7 (1 - 0.5^n).
        let mut v = DVec3::ZERO;
        for n in 1..=10u32 {
            v = firework_boost(v, 0.0, 0.0);
            let want = 1.7 * (1.0 - 0.5f64.powi(n as i32));
            assert!((v.z - want).abs() < 1e-12, "tick {n}: {} vs {want}", v.z);
        }
        assert!((v.z - 1.7).abs() < 2e-3, "{v}");
        // A dive (pitch 60) sends the push down and forward along the look.
        let dive = firework_boost(DVec3::ZERO, 0.0, 60.0);
        let look = look_angle(0.0, 60.0);
        for (got, want) in [(dive.x, look.x), (dive.y, look.y), (dive.z, look.z)] {
            assert!((got - want * 0.85).abs() < 1e-9, "{dive} vs {look}");
        }
        // Half the old velocity is kept.
        let kept = firework_boost(DVec3::new(0.4, -0.2, 0.8), 0.0, 0.0);
        assert!((kept.x - 0.2).abs() < TOL, "{kept}");
        assert!((kept.y + 0.1).abs() < TOL, "{kept}");
        assert!((kept.z - (0.4 + 0.85)).abs() < TOL, "{kept}");
    }

    /// The fall-flying pose's eye height (0.4) and box height (0.6), from `Avatar`'s pose
    /// dimensions; the glider box in `aabb` already uses 0.6.
    #[test]
    fn fall_flying_pose_dimensions() {
        let attrs = PlayerAttributes::default();
        assert!((attrs.eye_height_for(Pose::FallFlying) - 0.4).abs() < 1e-6);
        assert!((attrs.height_for(Pose::FallFlying) - 0.6).abs() < 1e-6);
        assert_eq!(Pose::from_name("fall_flying"), Some(Pose::FallFlying));
        assert_eq!(Pose::FallFlying.name(), "fall_flying");
    }

    /// Start/stop predicates, mirroring Player.tryToStartFallFlying and LivingEntity.canGlide.
    #[test]
    fn start_and_stop_predicates() {
        let mut state = PlayerState::at(DVec3::new(0.5, 100.0, 0.5));
        state.on_ground = false;
        assert!(
            state.can_start_fall_flying(true),
            "in the air with a glider"
        );
        assert!(!state.can_start_fall_flying(false), "no glider");
        // Not gliding yet: the flag is clear, even though the conditions allow it.
        assert!(!state.fall_flying);
        assert!(state.can_glide(true));

        state.start_fall_flying();
        assert!(state.fall_flying);
        assert!(!state.can_start_fall_flying(true), "already gliding");

        // In water the player starts swimming instead (vanilla's !isInLiquid check).
        state.in_water = true;
        assert!(!state.can_start_fall_flying(true));
        assert!(state.should_stop_fall_flying(true));
        state.in_water = false;

        // Creative flight wins over gliding (vanilla's Player.canGlide checks abilities).
        state.flying = true;
        assert!(!state.can_glide(true));
        assert!(state.should_stop_fall_flying(true));
        state.flying = false;

        // Losing the glider mid-glide ends it (vanilla's updateFallFlying stop check).
        assert!(state.should_stop_fall_flying(false));
        assert!(!state.should_stop_fall_flying(true));
        state.on_ground = true;
        assert!(state.should_stop_fall_flying(true), "landing ends it");
        assert!(!state.can_glide(true), "cannot glide on the ground");

        state.stop_fall_flying();
        assert!(!state.fall_flying);
    }
}
