//! Water and lava: contact detection, drag, buoyancy/sinking, swimming up, climbing out.
//!
//! What is documented: horizontal steady speeds in water and lava (wiki "Walking" table: 1.960 and
//! 0.784 m/s, 0.576 / 0.490 on soul sand) and sprint-swimming (wiki "Swimming": 3.918 m/s). The
//! drags below are derived from those numbers. The vertical constants were settled against the
//! 26.3 client source rather than traces: gravity divisors, swim up/down and the climb-out come
//! from `LivingEntity.travelInLava`, `LivingEntity.getFluidFallingAdjustedMovement`,
//! `LivingEntity.jumpInLiquid`/`goDownInWater`, `LivingEntity.jumpOutOfFluid` and `Player.travel`
//! (see NOTES.md). Vanilla method names are cited in the comments; no vanilla code is copied.
use haste_core::input::InputState;
use haste_core::view::{BlockView, CollisionShapes};
use haste_core::{BlockPos, DVec3};

use crate::collision::no_collision;
use crate::movement::{accelerate, apply_speed_factor, move_entity, Impulse};
use crate::trig;
use crate::{BlockPhysics, FluidKind, PlayerAttributes, PlayerState, Pose};

/// Acceleration in fluids. 0.02 (as in air); with drag 0.8 it gives 0.098 b/t = 1.960 m/s.
pub const FLUID_ACCELERATION: f32 = 0.02;
/// Water drag on all axes: derived from 1.960 m/s = 20 x 0.0196 / (1 - 0.8).
pub const WATER_DRAG: f64 = 0.8;
/// Horizontal water drag while sprint-swimming: derived from 3.918 m/s ~ 20 x 0.0196 / (1 - 0.9).
pub const WATER_DRAG_SPRINTING: f64 = 0.9;
/// Lava drag on all axes: derived from 0.784 m/s = 20 x 0.0196 / (1 - 0.5).
pub const LAVA_DRAG: f64 = 0.5;
/// Lava no deeper than the fluid jump threshold drags horizontally by 0.5 but vertically by 0.8
/// (`LivingEntity.travelInLava`, shallow branch), so the player sinks as slowly as in water.
pub const LAVA_DRAG_SHALLOW: f64 = 0.5;
/// Vertical drag in lava shallow enough to stand in (`LivingEntity.travelInLava`).
pub const LAVA_DRAG_SHALLOW_VERTICAL: f64 = 0.8;
/// Gravity in water is `gravity / 16` (`LivingEntity.getFluidFallingAdjustedMovement`).
pub const WATER_GRAVITY_DIVISOR: f64 = 16.0;
/// Gravity in lava deeper than the fluid jump threshold is `gravity / 4`
/// (`LivingEntity.travelInLava`); shallow lava takes `gravity / 16` first, then the same `gravity / 4`.
pub const LAVA_GRAVITY_DIVISOR: f64 = 4.0;
/// Holding jump in a fluid adds this to vertical velocity each tick
/// (`LivingEntity.jumpInLiquid`).
pub const SWIM_UP: f64 = 0.04;
/// Holding sneak in water subtracts this (`LocalPlayer.aiStep` -> `LivingEntity.goDownInWater`).
pub const SWIM_DOWN: f64 = 0.04;
/// A fluid no deeper than this (measured from the feet) counts as shallow
/// (`Entity.getFluidJumpThreshold`: 0.4 when the eye height is at least 0.4, else 0).
pub const FLUID_JUMP_THRESHOLD: f64 = 0.4;
/// Upward speed set when the player climbs out of a fluid against a horizontal collision
/// (`LivingEntity.jumpOutOfFluid`).
pub const JUMP_OUT_OF_FLUID_SPEED: f64 = 0.3;
/// How far above the player the climb-out probe looks (`LivingEntity.jumpOutOfFluid`).
pub const JUMP_OUT_OF_FLUID_CLEARANCE: f64 = 0.6;
/// Sprint-swimming steers y toward the look direction at this rate (`Player.travel`), faster when
/// looking down past [`SWIM_STEER_DIVE_THRESHOLD`] (0.085), else 0.06.
pub const SWIM_STEER_DIVING: f64 = 0.085;
/// Sprint-swimming steering rate when not diving (`Player.travel`).
pub const SWIM_STEER: f64 = 0.06;
/// Look-angle y below this counts as diving (`Player.travel`).
pub const SWIM_STEER_DIVE_THRESHOLD: f64 = -0.2;
/// Sinking step vanilla compares the post-drag y against when deciding the fall clamp
/// (`LivingEntity.getFluidFallingAdjustedMovement`: 0.005, i.e. gravity 0.08 / 16).
pub const FALL_ADJUSTMENT_COMPARAND: f64 = 0.005;
/// Value vanilla pins the sink to instead of subtracting gravity again in that case
/// (`LivingEntity.getFluidFallingAdjustedMovement`).
pub const FALL_ADJUSTMENT_CLAMP: f64 = -0.003;
/// Box shrink before testing fluid contact (assumption).
const CONTACT_SHRINK: f64 = 0.001;

/// Updates `in_water`/`in_lava` (water wins) and returns whether the eyes are under water.
/// Contact = the slightly shrunk box overlaps a fluid cell below its surface. The surface is
/// `FluidInfo::height`, or the full cell when the same fluid is directly above (wiki "Water").
pub(crate) fn update_fluid_state(
    state: &mut PlayerState,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    blocks: &dyn BlockPhysics,
) -> bool {
    let b = state.aabb(attrs);
    let eye_y = state.pos.y + attrs.eye_height_for(state.pose);
    let (min, max) = (
        b.min + DVec3::splat(CONTACT_SHRINK),
        b.max - DVec3::splat(CONTACT_SHRINK),
    );
    let (mut water, mut lava, mut eyes) = (false, false, false);
    for x in min.x.floor() as i32..=max.x.floor() as i32 {
        for z in min.z.floor() as i32..=max.z.floor() as i32 {
            for y in min.y.floor() as i32..=max.y.max(eye_y).floor() as i32 {
                let p = BlockPos::new(x, y, z);
                let Some(f) = blocks.fluid(world.block_state(p)) else {
                    continue;
                };
                let full = blocks
                    .fluid(world.block_state(p.add(0, 1, 0)))
                    .is_some_and(|a| a.kind == f.kind);
                let surface = y as f64 + if full { 1.0 } else { f.height as f64 };
                if f.kind == FluidKind::Water
                    && eye_y >= y as f64
                    && eye_y < surface
                    && x == state.pos.x.floor() as i32
                    && z == state.pos.z.floor() as i32
                {
                    eyes = true;
                }
                if (y as f64) < max.y && surface > min.y {
                    match f.kind {
                        FluidKind::Water => water = true,
                        FluidKind::Lava => lava = true,
                    }
                }
            }
        }
    }
    state.in_water = water;
    state.in_lava = lava && !water;
    if water {
        state.fall_distance = 0.0;
    }
    eyes && water
}

pub(crate) fn swim_up(state: &mut PlayerState) {
    state.velocity.y += SWIM_UP;
}

pub(crate) fn travel_water(
    state: &mut PlayerState,
    input: &InputState,
    imp: Impulse,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    blocks: &dyn BlockPhysics,
) {
    // `LivingEntity.travelInFluid` decides `isFalling` from the velocity the tick started with,
    // before the sneak sink, the swim steering, the acceleration and the move.
    let is_falling = state.velocity.y <= 0.0;
    let old_y = state.pos.y;
    if input.sneak {
        state.velocity.y -= SWIM_DOWN;
    }
    // Sprint-swimming steers y toward the look direction (vanilla `Player.travel`, applied before
    // the fluid travel itself; in lava the sprint-swim flag is never set, so this is water only).
    if state.swimming {
        steer_swimming(state, input, world, blocks);
    }
    let h_drag = if state.sprinting {
        WATER_DRAG_SPRINTING
    } else {
        WATER_DRAG
    };
    accelerate(state, imp, FLUID_ACCELERATION);
    let motion = state.velocity;
    move_entity(state, attrs, motion, world, shapes, blocks);
    apply_speed_factor(state, world, blocks);
    state.velocity.x *= h_drag;
    state.velocity.y *= WATER_DRAG;
    state.velocity.z *= h_drag;
    state.velocity.y = fluid_falling_adjusted_movement(
        attrs.gravity,
        is_falling,
        state.sprinting,
        state.velocity.y,
    );
    jump_out_of_fluid(state, attrs, old_y, world, shapes, blocks);
}

pub(crate) fn travel_lava(
    state: &mut PlayerState,
    _input: &InputState,
    imp: Impulse,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    blocks: &dyn BlockPhysics,
) {
    let is_falling = state.velocity.y <= 0.0;
    let old_y = state.pos.y;
    accelerate(state, imp, FLUID_ACCELERATION);
    let motion = state.velocity;
    move_entity(state, attrs, motion, world, shapes, blocks);
    apply_speed_factor(state, world, blocks);
    // Vanilla reads the fluid height after the move (the trackers are refreshed inside `move`),
    // so the shallow/deep branch is decided from the position the player ended up in.
    let shallow = fluid_height(state, attrs, world, blocks, FluidKind::Lava)
        <= fluid_jump_threshold(attrs, state.pose);
    if shallow {
        state.velocity.x *= LAVA_DRAG_SHALLOW;
        state.velocity.y *= LAVA_DRAG_SHALLOW_VERTICAL;
        state.velocity.z *= LAVA_DRAG_SHALLOW;
        state.velocity.y = fluid_falling_adjusted_movement(
            attrs.gravity,
            is_falling,
            state.sprinting,
            state.velocity.y,
        );
    } else {
        state.velocity *= LAVA_DRAG;
    }
    if attrs.gravity != 0.0 {
        state.velocity.y -= attrs.gravity / LAVA_GRAVITY_DIVISOR;
    }
    jump_out_of_fluid(state, attrs, old_y, world, shapes, blocks);
}

/// Vanilla `LivingEntity.getFluidFallingAdjustedMovement`: with gravity and no sprint, the fluid
/// sink subtracts `base_gravity / 16` (the `is_falling` clamp is unreachable for the player's
/// gravity 0.08 - the two windows it compares y against do not overlap - but kept for exactness).
fn fluid_falling_adjusted_movement(
    base_gravity: f64,
    is_falling: bool,
    sprinting: bool,
    y: f64,
) -> f64 {
    if base_gravity == 0.0 || sprinting {
        return y;
    }
    let step = base_gravity / WATER_GRAVITY_DIVISOR;
    if is_falling && (y - FALL_ADJUSTMENT_COMPARAND).abs() >= 0.003 && (y - step).abs() < 0.003 {
        FALL_ADJUSTMENT_CLAMP
    } else {
        y - step
    }
}

/// Vanilla `Entity.getFluidJumpThreshold`: 0.4 for a player-sized eye height, 0.0 for smaller.
fn fluid_jump_threshold(attrs: &PlayerAttributes, pose: Pose) -> f64 {
    if attrs.eye_height_for(pose) < FLUID_JUMP_THRESHOLD {
        0.0
    } else {
        FLUID_JUMP_THRESHOLD
    }
}

/// Depth of the deepest `kind` fluid around the player, measured from the feet
/// (vanilla `EntityFluidInteraction.update` -> `Entity.getFluidHeight`: the highest fluid surface
/// among the cells the slightly shrunk box touches).
fn fluid_height(
    state: &PlayerState,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    blocks: &dyn BlockPhysics,
    kind: FluidKind,
) -> f64 {
    let b = state.aabb(attrs);
    let min = b.min + DVec3::splat(CONTACT_SHRINK);
    let max = b.max - DVec3::splat(CONTACT_SHRINK);
    let mut height = 0.0f64;
    for x in min.x.floor() as i32..=(max.x.ceil() as i32 - 1) {
        for z in min.z.floor() as i32..=(max.z.ceil() as i32 - 1) {
            for y in min.y.floor() as i32..=(max.y.ceil() as i32 - 1) {
                let p = BlockPos::new(x, y, z);
                let Some(f) = blocks.fluid(world.block_state(p)) else {
                    continue;
                };
                if f.kind != kind {
                    continue;
                }
                let full = blocks
                    .fluid(world.block_state(p.add(0, 1, 0)))
                    .is_some_and(|a| a.kind == kind);
                let surface = y as f64 + if full { 1.0 } else { f.height as f64 };
                if surface >= min.y {
                    height = height.max(surface - b.min.y);
                }
            }
        }
    }
    height
}

/// Vanilla `Player.travel`: while sprint-swimming, y velocity is pulled toward the look direction
/// (0.085 per tick when looking down past -0.2, else 0.06), but only when looking down, holding
/// jump, or with fluid in the block 0.9 above the feet (there the player is pushed up anyway).
fn steer_swimming(
    state: &mut PlayerState,
    input: &InputState,
    world: &dyn BlockView,
    blocks: &dyn BlockPhysics,
) {
    // `Entity.getLookAngle().y` = -sin(pitch) through the movement sine table.
    let look_y = -(trig::sin(trig::deg_to_rad(state.pitch)) as f64);
    let multiplier = if look_y < SWIM_STEER_DIVE_THRESHOLD {
        SWIM_STEER_DIVING
    } else {
        SWIM_STEER
    };
    let above = BlockPos::from_dvec3(DVec3::new(
        state.pos.x,
        state.pos.y + 1.0 - 0.1,
        state.pos.z,
    ));
    let fluid_above = blocks.fluid(world.block_state(above)).is_some();
    if look_y <= 0.0 || input.jump || fluid_above {
        state.velocity.y += (look_y - state.velocity.y) * multiplier;
    }
}

/// Vanilla `LivingEntity.jumpOutOfFluid`: when the player runs into something horizontally while
/// in a fluid and the box, lifted by the current vertical velocity plus 0.6, is clear of blocks and
/// of liquid, y is set to 0.3 - this is what climbs the player onto the shore when swimming into it.
fn jump_out_of_fluid(
    state: &mut PlayerState,
    attrs: &PlayerAttributes,
    old_y: f64,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    blocks: &dyn BlockPhysics,
) {
    let v = state.velocity;
    if !state.horizontal_collision {
        return;
    }
    let dy = v.y + JUMP_OUT_OF_FLUID_CLEARANCE - state.pos.y + old_y;
    let probe = state.aabb(attrs).offset(DVec3::new(v.x, dy, v.z));
    // Vanilla `Entity.isFree`: no block collision and no liquid inside the moved box
    // (`LevelReader.containsAnyLiquid`).
    if no_collision(&probe, world, shapes) && !box_contains_fluid(&probe, world, blocks) {
        state.velocity.y = JUMP_OUT_OF_FLUID_SPEED;
    }
}

/// Vanilla `LevelReader.containsAnyLiquid`: any block cell the box touches holds a fluid.
fn box_contains_fluid(
    aabb: &haste_core::Aabb,
    world: &dyn BlockView,
    blocks: &dyn BlockPhysics,
) -> bool {
    for x in aabb.min.x.floor() as i32..=aabb.max.x.floor() as i32 {
        for z in aabb.min.z.floor() as i32..=aabb.max.z.floor() as i32 {
            for y in aabb.min.y.floor() as i32..=aabb.max.y.floor() as i32 {
                if blocks
                    .fluid(world.block_state(BlockPos::new(x, y, z)))
                    .is_some()
                {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{states::*, TestShapes, TestWorld};
    use haste_core::input::InputState;

    fn input(keys: impl FnOnce(&mut InputState)) -> InputState {
        let mut i = InputState::default();
        keys(&mut i);
        i
    }

    fn tick_once(world: &TestWorld, state: &mut PlayerState, input: &InputState) {
        let attrs = PlayerAttributes::default();
        let shapes = TestShapes::default();
        crate::tick(state, input, &attrs, world, &shapes, &shapes);
    }

    /// One lava cell at y=0 (surface 8/9) over a stone floor.
    fn lava_pool() -> TestWorld {
        let mut w = TestWorld::flat(0, STONE);
        w.fill((-2, 0, -2), (2, 0, 2), LAVA_SOURCE);
        w
    }

    #[test]
    fn shallow_lava_drags_like_water_and_takes_gravity_sixteenths() {
        // Feet 0.5 inside a source cell (surface 8/9): depth 0.3889 <= 0.4, so the shallow branch
        // of vanilla LivingEntity.travelInLava applies: drag (0.5, 0.8, 0.5), then gravity/16
        // (y is rising, so isFalling is false), then gravity/4. Vanilla reads the fluid depth after
        // the move; with a level velocity that is the same 0.3889.
        let w = lava_pool();
        let mut s = PlayerState::at(DVec3::new(0.5, 0.5, 0.5));
        s.on_ground = false;
        s.velocity = DVec3::new(0.2, 0.0, 0.0);
        tick_once(&w, &mut s, &InputState::default());
        assert_eq!(s.pos, DVec3::new(0.7, 0.5, 0.5));
        assert!(
            (s.velocity.x - 0.1).abs() < 1e-12
                && (s.velocity.y - (0.0 - 0.08 / 16.0 - 0.08 / 4.0)).abs() < 1e-12,
            "shallow lava velocity {:?}",
            s.velocity
        );
    }

    #[test]
    fn deep_lava_keeps_uniform_drag_and_gravity_quarters() {
        // Feet on the floor: depth 8/9 > 0.4, so the deep branch keeps drag 0.5 on every axis and
        // only gravity/4.
        let w = lava_pool();
        let mut s = PlayerState::at(DVec3::new(0.5, 0.0, 0.5));
        s.on_ground = false;
        s.velocity = DVec3::new(0.2, 0.0, 0.0);
        tick_once(&w, &mut s, &InputState::default());
        assert!(
            (s.velocity.x - 0.1).abs() < 1e-12 && (s.velocity.y - (0.0 - 0.08 / 4.0)).abs() < 1e-12,
            "deep lava velocity {:?}",
            s.velocity
        );
    }

    #[test]
    fn lava_depth_near_the_jump_threshold_switches_branch() {
        // The branch is the fluid depth against 0.4 (vanilla LivingEntity.isInShallowFluid with
        // Entity.getFluidJumpThreshold): feet at 0.45 sit in 0.4389 of lava (deep, drag 0.5 and
        // gravity/4), feet at 0.5 in 0.3889 (shallow, vertical drag 0.8 and gravity/16 then /4).
        for (feet, deep) in [(0.45, true), (0.5, false)] {
            let w = lava_pool();
            let mut s = PlayerState::at(DVec3::new(0.5, feet, 0.5));
            s.on_ground = false;
            s.velocity = DVec3::new(0.2, 0.0, 0.0);
            tick_once(&w, &mut s, &InputState::default());
            let expected = if deep {
                -0.08 / 4.0
            } else {
                -0.08 / 16.0 - 0.08 / 4.0
            };
            assert!(
                (s.velocity.y - expected).abs() < 1e-12,
                "feet {feet}: velocity {:?}",
                s.velocity
            );
        }
    }

    #[test]
    fn fluid_jump_threshold_follows_the_eye_height() {
        // Vanilla Entity.getFluidJumpThreshold: 0.4 unless the eye height is below 0.4 (then 0).
        let attrs = PlayerAttributes::default();
        assert_eq!(
            fluid_jump_threshold(&attrs, Pose::Standing),
            FLUID_JUMP_THRESHOLD
        );
        assert_eq!(
            fluid_jump_threshold(&attrs, Pose::Swimming),
            FLUID_JUMP_THRESHOLD
        );
        let mut tiny = attrs;
        tiny.scale = 0.2; // eye 1.62 x 0.2 = 0.324, below 0.4
        assert_eq!(fluid_jump_threshold(&tiny, Pose::Standing), 0.0);
    }

    /// Water up to y=10 (surface 10 8/9) with the eyes in air: in water, not sprint-swimming.
    fn water_pool_eyes_dry() -> TestWorld {
        let mut w = TestWorld::flat(0, STONE);
        w.fill((-2, 0, -2), (2, 10, 2), WATER_SOURCE);
        w
    }

    fn water_player() -> PlayerState {
        let mut s = PlayerState::at(DVec3::new(0.5, 10.0, 0.5));
        s.on_ground = false;
        s.velocity = DVec3::new(0.2, 0.1, 0.0);
        s
    }

    #[test]
    fn water_gravity_sixteenths_applies_without_sprint() {
        let w = water_pool_eyes_dry();
        let mut s = water_player();
        tick_once(&w, &mut s, &InputState::default());
        // drag 0.8 on y, then gravity/16 (rising, so isFalling false).
        assert!(
            (s.velocity.y - (0.08 - 0.08 / 16.0)).abs() < 1e-12,
            "sinking velocity {:?}",
            s.velocity.y
        );
    }

    #[test]
    fn water_gravity_sixteenths_skipped_while_sprinting() {
        // Vanilla LivingEntity.getFluidFallingAdjustedMovement skips the sink when sprinting,
        // whatever the pose (the swim flag is irrelevant).
        let w = water_pool_eyes_dry();
        let mut s = water_player();
        s.sprinting = true;
        tick_once(
            &w,
            &mut s,
            &input(|i| {
                i.forward = true;
                i.sprint = true;
            }),
        );
        assert!(
            (s.velocity.y - 0.08).abs() < 1e-12,
            "sprinting sink velocity {:?}",
            s.velocity.y
        );
    }

    /// Water only at y=0 (surface 8/9), plus a stone wall at x=2 reaching above the player.
    fn shore() -> TestWorld {
        let mut w = TestWorld::flat(0, STONE);
        w.fill((-1, 0, -1), (1, 0, 1), WATER_SOURCE);
        w.fill((2, 0, 0), (2, 4, 0), STONE);
        w
    }

    #[test]
    fn swimming_into_the_shore_sets_the_climb_out_speed() {
        // Feet 0.5 above the floor in 0.888 of water, pushed into the wall: the horizontal
        // collision flags, and the box lifted by (velocity.y + 0.6) is out of the water and clear,
        // so vanilla LivingEntity.jumpOutOfFluid sets y to 0.3.
        let w = shore();
        let mut s = PlayerState::at(DVec3::new(1.5, 0.5, 0.5));
        s.on_ground = false;
        s.velocity = DVec3::new(0.3, 0.0, 0.0);
        tick_once(&w, &mut s, &InputState::default());
        assert!(s.horizontal_collision, "ran into the wall");
        assert!(s.velocity.x.abs() < 1e-12, "wall stops x: {:?}", s.velocity);
        assert!(s.velocity.y > 0.0, "climbs out: {:?}", s.velocity.y);
        assert!(
            (s.velocity.y - JUMP_OUT_OF_FLUID_SPEED).abs() < 1e-12,
            "climb-out velocity {:?}",
            s.velocity.y
        );
        // The sink it replaces: 0 after the drag, minus gravity/16.
        assert!(s.pos.y > 0.499, "still level: {}", s.pos.y);
    }

    #[test]
    fn no_climb_out_when_the_space_above_stays_under_water() {
        // Same push, but the water is three cells deep: the lifted box is still inside liquid, so
        // vanilla's isFree test fails and the player keeps sinking.
        let mut w = shore();
        w.fill((-1, 1, -1), (1, 2, 1), WATER_SOURCE);
        let mut s = PlayerState::at(DVec3::new(1.5, 0.5, 0.5));
        s.on_ground = false;
        s.velocity = DVec3::new(0.3, 0.0, 0.0);
        tick_once(&w, &mut s, &InputState::default());
        assert!(s.horizontal_collision);
        assert!(
            (s.velocity.y - (0.0 - 0.08 / 16.0)).abs() < 1e-12,
            "keeps sinking: {:?}",
            s.velocity.y
        );
    }

    /// Water at y=0 and y=2 (air at y=1): the eyes are under water standing, and once the swimming
    /// pose is used the block 0.9 above the feet is air.
    fn shallow_water() -> TestWorld {
        let mut w = TestWorld::flat(0, STONE);
        w.fill((-2, 0, -2), (2, 0, 2), WATER_SOURCE);
        w.fill((-2, 2, -2), (2, 2, 2), WATER_SOURCE);
        w
    }

    #[test]
    fn sprint_swimming_steers_y_toward_the_look_direction() {
        // Vanilla Player.travel: while sprint-swimming, y is pulled toward the look direction
        // (0.085 per tick looking down past -0.2, else 0.06). Looking straight down from a standstill
        // in shallow water: 0 - 1 * 0.085, then the water drag 0.8, and no gravity while sprinting.
        let w = shallow_water();
        let mut s = PlayerState::at(DVec3::new(0.5, 0.5, 0.5));
        s.on_ground = false;
        tick_once(
            &w,
            &mut s,
            &input(|i| {
                i.forward = true;
                i.sprint = true;
            }),
        );
        assert!(s.swimming, "sprint-swimming established");
        s.pos = DVec3::new(0.5, 0.4, 0.5);
        s.velocity = DVec3::ZERO;
        let look_y = -(trig::sin(trig::deg_to_rad(90.0)) as f64);
        assert!(look_y < SWIM_STEER_DIVE_THRESHOLD);
        tick_once(
            &w,
            &mut s,
            &input(|i| {
                i.forward = true;
                i.sprint = true;
                i.pitch = 90.0;
            }),
        );
        let expected = (look_y - 0.0) * SWIM_STEER_DIVING * WATER_DRAG;
        assert!(
            (s.velocity.y - expected).abs() < 1e-12,
            "dive steering: {} expected {}",
            s.velocity.y,
            expected
        );
    }

    #[test]
    fn no_steering_when_looking_up_with_air_above_the_feet() {
        // Looking up, no jump, and the block 0.9 above the feet is air: vanilla Player.travel does
        // not steer, and sprinting skips the gravity sink, so y stays put.
        let w = shallow_water();
        let mut s = PlayerState::at(DVec3::new(0.5, 0.5, 0.5));
        s.on_ground = false;
        tick_once(
            &w,
            &mut s,
            &input(|i| {
                i.forward = true;
                i.sprint = true;
            }),
        );
        assert!(s.swimming);
        s.pos = DVec3::new(0.5, 0.4, 0.5);
        s.velocity = DVec3::ZERO;
        let look_y = -(trig::sin(trig::deg_to_rad(-45.0)) as f64);
        assert!(look_y > 0.0);
        tick_once(
            &w,
            &mut s,
            &input(|i| {
                i.forward = true;
                i.sprint = true;
                i.pitch = -45.0;
            }),
        );
        assert!(
            s.velocity.y.abs() < 1e-12,
            "no steering, no sink: {:?}",
            s.velocity.y
        );
    }
}
