//! Block movement effects (vanilla 26.3): stuck-in-block speed multipliers (cobweb, powder
//! snow, sweet berry bush), the slime bounce and slime speed factor, honey sliding, and water
//! currents. Sources: `Entity.makeStuckInBlock` and the blocks' `entityInside`, `Entity.move`
//! (consume/reset of the stuck multiplier), `Entity.restituteMovementAfterCollisions` (bounce via
//! `Block.getBounceRestitution`), `SlimeBlock.stepOn`, `HoneyBlock.isSlidingDown`/`doSlideMovement`,
//! and `Entity.updateFluidInteraction` + `EntityFluidInteraction` (0.014 push). Block identity
//! comes through `BlockPhysics` (haste-data resolves it from registry names); nothing here
//! compares state ids.
use haste_core::view::{BlockView, CollisionShapes};
use haste_core::{BlockPos, BlockStateId, DVec3};

use crate::movement::ground_block;
use crate::{BlockPhysics, FluidKind, PlayerAttributes, PlayerState};

/// Cobweb multiplier (`WebBlock.entityInside`): 0.25 horizontal, 0.05 vertical.
pub const COBWEB_MULTIPLIER: DVec3 = DVec3::new(0.25, 0.05, 0.25);
/// Powder snow multiplier (`PowderSnowBlock.entityInside`): 0.9 horizontal, 1.5 vertical.
pub const POWDER_SNOW_MULTIPLIER: DVec3 = DVec3::new(0.9, 1.5, 0.9);
/// Sweet berry bush multiplier (`SweetBerryBushBlock.entityInside`).
pub const SWEET_BERRY_BUSH_MULTIPLIER: DVec3 = DVec3::new(0.8, 0.75, 0.8);

/// Water current push scale (`Entity.updateFluidInteraction` -> `applyCurrentTo(0.014)`).
pub const WATER_CURRENT_SCALE: f64 = 0.014;
/// Below this |v| the player counts as nearly stationary and the push is normalized to
/// `WATER_CURRENT_MIN_IMPULSE` (`CurrentAccumulator.applyTo`).
const NEARLY_STATIONARY: f64 = 0.003;
/// Minimum water current impulse for a nearly stationary player (`applyTo`).
pub const WATER_CURRENT_MIN_IMPULSE: f64 = 0.0045;
/// Accumulated flow vectors shorter than this (squared) give no push (`applyTo`).
const WATER_CURRENT_MIN_LENGTH_SQR: f64 = 1.0e-5;
/// Flow vectors are scaled by the fluid height below this (`EntityFluidInteraction.update`).
const WATER_CURRENT_HEIGHT_CUTOFF: f64 = 0.4;
/// Margin by which the box is shrunk before the fluid scan (`getFluidInteractionBox`).
const FLUID_BOX_MARGIN: f64 = 0.001;

/// `SlimeBlock.stepOn`: while on the ground on slime with |vy| below this and not sneaking,
/// horizontal velocity is scaled by `0.4 + |vy| x 0.2`.
const SLIME_STEP_MAX_VERTICAL_SPEED: f64 = 0.1;
const SLIME_STEP_SCALE_BASE: f64 = 0.4;
const SLIME_STEP_SCALE_PER_VERTICAL_SPEED: f64 = 0.2;

/// Honey slide (`HoneyBlock`): `getOldDeltaY(vy) = vy / 0.98 + 0.08`; sliding starts when it is
/// below -0.08, horizontal speed is reduced when below -0.13, and the vertical speed is
/// throttled to `getNewDeltaY(-0.05) = -0.1274`.
const HONEY_OLD_DELTA_DIVISOR: f64 = 0.98;
const HONEY_OLD_DELTA_OFFSET: f64 = 0.08;
const HONEY_SLIDE_STARTS_BELOW: f64 = -0.08;
const HONEY_SLIDE_SLOWS_BELOW: f64 = -0.13;
const HONEY_SLIDE_HORIZONTAL_SPEED: f64 = -0.05;
/// The honey block's collision column is 14/16 wide and 15/16 tall (`HoneyBlock.SHAPE`); the
/// slide needs the player's centre further than this from the cell centre in x or z.
const HONEY_SIDE_OVERLAP_BASE: f64 = 0.4375;
const HONEY_FEET_MAX_HEIGHT: f64 = 0.9375;
const EPS: f64 = 1.0e-7;

fn component_mul(a: DVec3, b: DVec3) -> DVec3 {
    DVec3::new(a.x * b.x, a.y * b.y, a.z * b.z)
}

/// `HoneyBlock.getOldDeltaY`: the velocity a tick of gravity + air drag would have produced.
fn old_delta_y(dy: f64) -> f64 {
    dy / HONEY_OLD_DELTA_DIVISOR + HONEY_OLD_DELTA_OFFSET
}

/// `HoneyBlock.getNewDeltaY`: the inverse of `getOldDeltaY`.
fn new_delta_y(dy: f64) -> f64 {
    (dy - HONEY_OLD_DELTA_OFFSET) * HONEY_OLD_DELTA_DIVISOR
}

/// Consume the pending stuck multiplier at the start of the move (`Entity.move`): the motion is
/// multiplied by it, the multiplier is reset and the velocity is zeroed (26.3 also zeroes the
/// velocity here, unlike older versions).
pub(crate) fn consume_stuck_multiplier(state: &mut PlayerState, motion: DVec3) -> DVec3 {
    if state.stuck_multiplier.length_squared() > 1.0e-7 {
        let motion = component_mul(motion, state.stuck_multiplier);
        state.stuck_multiplier = DVec3::ZERO;
        state.velocity = DVec3::ZERO;
        motion
    } else {
        motion
    }
}

/// `Entity.restituteMovementAfterCollisions` (vertical part): after a vertical collision while
/// moving down, bounce off blocks with restitution (slime 1.0) when falling faster than gravity
/// and not sneaking; otherwise the vertical velocity is zeroed. `motion_y` is the velocity the
/// move started with, `moved_y` the clipped movement.
pub(crate) fn vertical_collision_velocity(
    state: &PlayerState,
    blocks: &dyn BlockPhysics,
    ground: BlockStateId,
    motion_y: f64,
    moved_y: f64,
    gravity: f64,
    air_drag: f64,
) -> f64 {
    let restitution = if !state.sneaking && -motion_y > gravity {
        blocks.bounce_restitution(ground) as f64
    } else {
        0.0
    };
    if restitution > 0.0 && motion_y != 0.0 {
        let portion = moved_y / motion_y;
        let gravity_compensation = portion * gravity;
        let effective_drag = 1.0 + (air_drag - 1.0) * portion;
        (gravity_compensation - motion_y) * effective_drag * restitution
    } else {
        0.0
    }
}

/// End-of-tick block effects (`Entity.applyEffectsFromBlocks`): `stepOn` (slime) first, then the
/// `entityInside` scan (honey slide and the stuck-in-block multipliers).
pub(crate) fn apply_end_of_tick_effects(
    state: &mut PlayerState,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    blocks: &dyn BlockPhysics,
) {
    if state.on_ground {
        apply_step_on(state, attrs, world, shapes, blocks);
    }
    apply_inside_block_effects(state, attrs, world, blocks);
}

/// `SlimeBlock.stepOn`: on the ground on slime with |vy| < 0.1 and not sneaking, horizontal
/// velocity x (0.4 + |vy| x 0.2).
fn apply_step_on(
    state: &mut PlayerState,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    blocks: &dyn BlockPhysics,
) {
    let ground = ground_block(state, attrs, world, shapes);
    if !blocks.is_slime(ground) {
        return;
    }
    let abs_vy = state.velocity.y.abs();
    if abs_vy >= SLIME_STEP_MAX_VERTICAL_SPEED || state.sneaking {
        return;
    }
    let scale = SLIME_STEP_SCALE_BASE + abs_vy * SLIME_STEP_SCALE_PER_VERTICAL_SPEED;
    state.velocity.x *= scale;
    state.velocity.z *= scale;
}

/// `Entity.checkInsideBlocks` over the cells the box overlaps: honey slide per honey cell, and
/// the stuck-in-block multiplier for cobweb / powder snow (whose in-block state must be powder
/// snow) / sweet berry bush. `makeStuckInBlock` also resets the fall distance.
fn apply_inside_block_effects(
    state: &mut PlayerState,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    blocks: &dyn BlockPhysics,
) {
    let aabb = state.aabb(attrs);
    let feet = world.block_state(BlockPos::from_dvec3(state.pos));
    let mut stuck: Option<DVec3> = None;
    let x0 = aabb.min.x.floor() as i32;
    let y0 = aabb.min.y.floor() as i32;
    let z0 = aabb.min.z.floor() as i32;
    let x1 = aabb.max.x.floor() as i32;
    let y1 = aabb.max.y.floor() as i32;
    let z1 = aabb.max.z.floor() as i32;
    for x in x0..=x1 {
        for y in y0..=y1 {
            for z in z0..=z1 {
                let pos = BlockPos::new(x, y, z);
                let s = world.block_state(pos);
                if let Some(m) = stuck_multiplier(blocks, s, feet) {
                    stuck = Some(m);
                }
                if blocks.is_honey(s) {
                    apply_honey_slide(state, attrs, pos);
                }
            }
        }
    }
    if let Some(m) = stuck {
        state.stuck_multiplier = m;
        state.fall_distance = 0.0;
    }
}

/// The multiplier `makeStuckInBlock` applies for a block the player is inside, if it is one of
/// the stuck-in-block blocks. `in_block` is the player's in-block state (vanilla
/// `getInBlockState`, the feet block); powder snow only sticks when that is powder snow.
fn stuck_multiplier(
    blocks: &dyn BlockPhysics,
    inside: BlockStateId,
    in_block: BlockStateId,
) -> Option<DVec3> {
    if blocks.is_cobweb(inside) {
        Some(COBWEB_MULTIPLIER)
    } else if blocks.is_powder_snow(inside) && blocks.is_powder_snow(in_block) {
        Some(POWDER_SNOW_MULTIPLIER)
    } else if blocks.is_sweet_berry_bush(inside) {
        Some(SWEET_BERRY_BUSH_MULTIPLIER)
    } else {
        None
    }
}

/// `HoneyBlock.entityInside` -> `isSlidingDown`/`doSlideMovement`: falling past the side of a
/// honey block throttles the vertical speed to `getNewDeltaY(-0.05)` and, when falling fast
/// enough, reduces horizontal speed by `-0.05 / getOldDeltaY(vy)`.
fn apply_honey_slide(state: &mut PlayerState, attrs: &PlayerAttributes, pos: BlockPos) {
    if state.on_ground {
        return;
    }
    if state.pos.y > pos.y as f64 + HONEY_FEET_MAX_HEIGHT - EPS {
        return;
    }
    let old = old_delta_y(state.velocity.y);
    if old >= HONEY_SLIDE_STARTS_BELOW {
        return;
    }
    let overlap = HONEY_SIDE_OVERLAP_BASE + attrs.box_width() / 2.0;
    let dx = (pos.x as f64 + 0.5 - state.pos.x).abs() + EPS;
    let dz = (pos.z as f64 + 0.5 - state.pos.z).abs() + EPS;
    if dx <= overlap && dz <= overlap {
        return;
    }
    let (vx, vz) = if old < HONEY_SLIDE_SLOWS_BELOW {
        let factor = HONEY_SLIDE_HORIZONTAL_SPEED / old;
        (state.velocity.x * factor, state.velocity.z * factor)
    } else {
        (state.velocity.x, state.velocity.z)
    };
    state.velocity = DVec3::new(vx, new_delta_y(HONEY_SLIDE_HORIZONTAL_SPEED), vz);
    state.fall_distance = 0.0;
}

/// `Entity.updateFluidInteraction` + `EntityFluidInteraction`: accumulate the flow vectors of
/// the water cells the (shrunk) box overlaps and add the average x 0.014 to the velocity; a
/// nearly stationary player gets a normalized 0.0045 push instead. Runs before the move
/// (vanilla `baseTick` precedes `aiStep`).
pub(crate) fn apply_water_currents(
    state: &mut PlayerState,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    blocks: &dyn BlockPhysics,
) {
    if !state.in_water {
        return;
    }
    let fluid_box = state.aabb(attrs);
    let min = fluid_box.min + DVec3::splat(FLUID_BOX_MARGIN);
    let max = fluid_box.max - DVec3::splat(FLUID_BOX_MARGIN);
    let entity_y = fluid_box.min.y;
    let x0 = min.x.floor() as i32;
    let y0 = min.y.floor() as i32;
    let z0 = min.z.floor() as i32;
    let x1 = max.x.ceil() as i32 - 1;
    let y1 = max.y.ceil() as i32 - 1;
    let z1 = max.z.ceil() as i32 - 1;
    let mut accumulated = DVec3::ZERO;
    let mut count = 0;
    let mut height: f64 = 0.0;
    for x in x0..=x1 {
        for y in y0..=y1 {
            for z in z0..=z1 {
                let s = world.block_state(BlockPos::new(x, y, z));
                let Some(f) = blocks.fluid(s) else {
                    continue;
                };
                if f.kind != FluidKind::Water {
                    continue;
                }
                let fluid_top = y as f64 + f.height as f64;
                if fluid_top < min.y {
                    continue;
                }
                height = height.max(fluid_top - entity_y);
                let Some(flow) = blocks.water_flow(s) else {
                    continue;
                };
                let flow = if height < WATER_CURRENT_HEIGHT_CUTOFF {
                    flow * height
                } else {
                    flow
                };
                accumulated += flow;
                count += 1;
            }
        }
    }
    if count == 0 || accumulated.length_squared() < WATER_CURRENT_MIN_LENGTH_SQR {
        return;
    }
    let mut impulse = accumulated * (1.0 / count as f64);
    impulse *= WATER_CURRENT_SCALE;
    if state.velocity.x.abs() < NEARLY_STATIONARY
        && state.velocity.z.abs() < NEARLY_STATIONARY
        && impulse.length() < WATER_CURRENT_MIN_IMPULSE
    {
        impulse = impulse.normalize() * WATER_CURRENT_MIN_IMPULSE;
    }
    state.velocity += impulse;
}

/// Cells the AABB overlaps (helper for tests).
#[cfg(test)]
fn aabb_cells(aabb: &haste_core::Aabb) -> Vec<BlockPos> {
    let mut out = Vec::new();
    for x in aabb.min.x.floor() as i32..=aabb.max.x.floor() as i32 {
        for y in aabb.min.y.floor() as i32..=aabb.max.y.floor() as i32 {
            for z in aabb.min.z.floor() as i32..=aabb.max.z.floor() as i32 {
                out.push(BlockPos::new(x, y, z));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use haste_core::input::InputState;
    use haste_core::{Aabb, DVec3};

    use crate::movement::VERTICAL_DRAG;

    use super::*;
    use crate::testing::{states, TestShapes, TestWorld};
    use crate::{tick, PlayerAttributes, PlayerState};

    const GRAVITY: f64 = 0.08;
    const AIR_DRAG: f64 = 0.98;

    fn attrs() -> PlayerAttributes {
        PlayerAttributes::default()
    }

    /// One idle tick with the given starting velocity; returns the state after.
    fn tick_from(
        world: &TestWorld,
        shapes: &TestShapes,
        attrs: &PlayerAttributes,
        mut state: PlayerState,
    ) -> PlayerState {
        tick(
            &mut state,
            &InputState::default(),
            attrs,
            world,
            shapes,
            shapes,
        );
        state
    }

    // ---- stuck-in-block multipliers -------------------------------------------------------------

    #[test]
    fn cobweb_sets_multiplier_and_next_tick_consumes_it() {
        let mut world = TestWorld::new();
        world.set(0, 0, 0, states::COBWEB);
        let shapes = TestShapes::default();
        let attrs = attrs();
        let mut state = PlayerState::at(DVec3::new(0.5, 0.5, 0.5));
        // Tick 1: no multiplier yet; the move happens, then entityInside sets it.
        state = tick_from(&world, &shapes, &attrs, state);
        assert_eq!(state.stuck_multiplier, COBWEB_MULTIPLIER);
        assert_eq!(state.fall_distance, 0.0);
        // Tick 2: the move is multiplied by it and the velocity is zeroed (Entity.move).
        let drag = VERTICAL_DRAG as f64;
        let vy = -GRAVITY * drag;
        state.velocity = DVec3::new(0.0, vy, 0.0);
        let before = state.pos;
        state = tick_from(&world, &shapes, &attrs, state);
        assert_eq!(state.pos.y, before.y + vy * COBWEB_MULTIPLIER.y);
        assert_eq!(state.velocity, DVec3::new(0.0, vy, 0.0));
        assert_eq!(state.stuck_multiplier, COBWEB_MULTIPLIER);
        assert_eq!(state.fall_distance, 0.0);
    }

    #[test]
    fn powder_snow_multiplier_needs_powder_snow_under_the_feet() {
        let shapes = TestShapes::default();
        let attrs = attrs();
        // Feet block is powder snow: stuck.
        let mut world = TestWorld::new();
        world.set(0, 0, 0, states::POWDER_SNOW);
        let state = PlayerState::at(DVec3::new(0.5, 0.5, 0.5));
        let state = tick_from(&world, &shapes, &attrs, state);
        assert_eq!(state.stuck_multiplier, POWDER_SNOW_MULTIPLIER);
        // Box intersects powder snow but the feet block is air: not stuck.
        let mut world = TestWorld::new();
        world.set(1, 1, 0, states::POWDER_SNOW);
        let mut state = PlayerState::at(DVec3::new(0.75, 1.5, 0.5));
        state.velocity = DVec3::new(0.0, -0.0784, 0.0);
        let state = tick_from(&world, &shapes, &attrs, state);
        assert_eq!(state.stuck_multiplier, DVec3::ZERO);
    }

    #[test]
    fn sweet_berry_bush_sets_multiplier() {
        let mut world = TestWorld::new();
        world.set(0, 0, 0, states::SWEET_BERRY_BUSH);
        let shapes = TestShapes::default();
        let attrs = attrs();
        let state = PlayerState::at(DVec3::new(0.5, 0.5, 0.5));
        let state = tick_from(&world, &shapes, &attrs, state);
        assert_eq!(state.stuck_multiplier, SWEET_BERRY_BUSH_MULTIPLIER);
        assert_eq!(state.fall_distance, 0.0);
    }

    // ---- slime bounce ---------------------------------------------------------------------------

    #[test]
    fn slime_bounce_velocity_exact_formula() {
        // Entity.restituteMovementAfterCollisions: restitution 1.0 (slime), gravity 0.08,
        // air drag 0.98; motion_y -0.5 clipped to moved_y -0.1.
        let shapes = TestShapes::default();
        let state = PlayerState::at(DVec3::ZERO);
        let v = vertical_collision_velocity(
            &state,
            &shapes,
            states::SLIME,
            -0.5,
            -0.1,
            GRAVITY,
            AIR_DRAG,
        );
        let portion = -0.1 / -0.5;
        let expected = (portion * GRAVITY - -0.5) * (1.0 + (AIR_DRAG - 1.0) * portion) * 1.0;
        assert_eq!(v, expected);
        assert!((v - 0.513936).abs() < 1e-12, "bounce velocity {v}");
    }

    fn land_on_slime(
        world: &TestWorld,
        attrs: &PlayerAttributes,
        shapes: &TestShapes,
        y: f64,
        input: &InputState,
    ) -> PlayerState {
        let mut state = PlayerState::at(DVec3::new(0.5, y, 0.5));
        state.on_ground = false;
        for _ in 0..20 {
            tick(&mut state, input, attrs, world, shapes, shapes);
            if state.on_ground {
                break;
            }
        }
        state
    }

    #[test]
    fn slime_bounce_only_when_falling_faster_than_gravity_and_not_sneaking() {
        let world = TestWorld::flat(0, states::SLIME);
        let shapes = TestShapes::default();
        let attrs = attrs();
        let idle = InputState::default();
        // Landing at |vy| = 0.0784 (<= gravity): no bounce, stays on the ground.
        let state = land_on_slime(&world, &attrs, &shapes, 0.05, &idle);
        assert!(state.on_ground);
        assert_eq!(state.pos.y, 0.0);
        assert_eq!(state.velocity.y, -GRAVITY * VERTICAL_DRAG as f64);
        // Landing at |vy| = 0.155232 (> gravity): bounces up.
        let state = land_on_slime(&world, &attrs, &shapes, 0.2, &idle);
        assert!(state.velocity.y > 0.0, "bounced up: {}", state.velocity.y);
        assert_eq!(state.pos.y, 0.0);
        // Same fall while sneaking: no bounce.
        let sneak = InputState {
            sneak: true,
            ..Default::default()
        };
        let state = land_on_slime(&world, &attrs, &shapes, 0.2, &sneak);
        assert!(state.on_ground);
        assert_eq!(state.velocity.y, -GRAVITY * VERTICAL_DRAG as f64);
    }

    #[test]
    fn slime_bounce_velocity_after_landing_tick() {
        let world = TestWorld::flat(0, states::SLIME);
        let shapes = TestShapes::default();
        let attrs = attrs();
        // Mirror the ticks: gravity after each move; the landing tick falls 0.1216 into the
        // floor with motion_y -0.155232, then the bounce, then gravity + drag.
        let drag = VERTICAL_DRAG as f64;
        let vy = -GRAVITY * drag;
        let y = 0.2 + vy;
        let motion_y = (vy - GRAVITY) * drag;
        let moved_y = -y;
        let portion = moved_y / motion_y;
        let bounced = (portion * GRAVITY - motion_y) * (1.0 + (drag - 1.0) * portion) * 1.0;
        let expected = (bounced - GRAVITY) * drag;
        let state = land_on_slime(&world, &attrs, &shapes, 0.2, &InputState::default());
        assert_eq!(state.velocity.y, expected);
        assert!(state.velocity.y > 0.0);
    }

    // ---- slime speed factor (stepOn) ------------------------------------------------------------

    #[test]
    fn slime_step_on_scales_horizontal_velocity() {
        let world = TestWorld::flat(0, states::SLIME);
        let shapes = TestShapes::default();
        let attrs = attrs();
        let drag = (0.8f32 * 0.91f32) as f64;
        let vy = -GRAVITY * VERTICAL_DRAG as f64;
        let scale = SLIME_STEP_SCALE_BASE + vy.abs() * SLIME_STEP_SCALE_PER_VERTICAL_SPEED;
        // Walking east at 0.5 b/t: friction drag then the stepOn scale.
        let mut state = PlayerState::at(DVec3::new(0.5, 0.0, 0.5));
        state.on_ground = true;
        state.velocity = DVec3::new(0.5, vy, 0.0);
        state = tick_from(&world, &shapes, &attrs, state);
        assert_eq!(state.velocity.x, 0.5 * drag * scale);
        // Sneaking: no scale.
        let sneak = InputState {
            sneak: true,
            ..Default::default()
        };
        let mut state = PlayerState::at(DVec3::new(0.5, 0.0, 0.5));
        state.on_ground = true;
        state.velocity = DVec3::new(0.5, vy, 0.0);
        tick(&mut state, &sneak, &attrs, &world, &shapes, &shapes);
        assert_eq!(state.velocity.x, 0.5 * drag);
    }

    // ---- honey slide ---------------------------------------------------------------------------

    #[test]
    fn honey_slide_exact_numbers() {
        let attrs = attrs();
        // Falling fast past the side: vertical throttled to getNewDeltaY(-0.05), horizontal
        // reduced by -0.05 / getOldDeltaY(vy).
        let mut state = PlayerState::at(DVec3::new(0.75, 0.5, 0.5));
        state.velocity = DVec3::new(0.3, -0.5, 0.2);
        apply_honey_slide(&mut state, &attrs, BlockPos::new(1, 0, 0));
        let old = old_delta_y(-0.5);
        let factor = HONEY_SLIDE_HORIZONTAL_SPEED / old;
        assert_eq!(
            state.velocity,
            DVec3::new(
                0.3 * factor,
                new_delta_y(HONEY_SLIDE_HORIZONTAL_SPEED),
                0.2 * factor
            )
        );
        assert_eq!(state.velocity.y, -0.1274);
        assert_eq!(state.fall_distance, 0.0);
        // Falling slowly (getOldDeltaY(vy) in [-0.13, -0.08)): vertical throttled, horizontal kept.
        let mut state = PlayerState::at(DVec3::new(0.75, 0.5, 0.5));
        state.velocity = DVec3::new(0.3, -0.16, 0.2);
        apply_honey_slide(&mut state, &attrs, BlockPos::new(1, 0, 0));
        assert_eq!(state.velocity, DVec3::new(0.3, -0.1274, 0.2));
    }

    #[test]
    fn honey_slide_conditions() {
        let attrs = attrs();
        // On the ground: no slide.
        let mut state = PlayerState::at(DVec3::new(0.75, 0.5, 0.5));
        state.on_ground = true;
        state.velocity = DVec3::new(0.0, -0.5, 0.0);
        apply_honey_slide(&mut state, &attrs, BlockPos::new(1, 0, 0));
        assert_eq!(state.velocity.y, -0.5);
        // Feet above 0.9375 in the cell: no slide.
        let mut state = PlayerState::at(DVec3::new(0.75, 0.95, 0.5));
        state.velocity = DVec3::new(0.0, -0.5, 0.0);
        apply_honey_slide(&mut state, &attrs, BlockPos::new(1, 0, 0));
        assert_eq!(state.velocity.y, -0.5);
        // Falling too slowly (getOldDeltaY(vy) >= -0.08): no slide.
        let mut state = PlayerState::at(DVec3::new(0.75, 0.5, 0.5));
        state.velocity = DVec3::new(0.0, -0.1, 0.0);
        apply_honey_slide(&mut state, &attrs, BlockPos::new(1, 0, 0));
        assert_eq!(state.velocity.y, -0.1);
        // Centred over the block (dx, dz within 0.4375 + width/2): no slide.
        let mut state = PlayerState::at(DVec3::new(1.5, 0.5, 0.5));
        state.velocity = DVec3::new(0.0, -0.5, 0.0);
        apply_honey_slide(&mut state, &attrs, BlockPos::new(1, 0, 0));
        assert_eq!(state.velocity.y, -0.5);
    }

    #[test]
    fn honey_slide_through_tick() {
        let mut world = TestWorld::new();
        world.set(1, 0, 0, states::HONEY);
        let shapes = TestShapes::default();
        let attrs = attrs();
        let mut state = PlayerState::at(DVec3::new(0.76, 0.5, 0.5));
        state.velocity = DVec3::new(0.0, -0.5, 0.0);
        state = tick_from(&world, &shapes, &attrs, state);
        // After gravity + drag vy = -0.5684; old_delta_y = -0.5; factor 0.1; vertical -0.1274.
        assert_eq!(state.velocity, DVec3::new(0.0, -0.1274, 0.0));
        assert_eq!(state.fall_distance, 0.0);
    }

    // ---- water currents ------------------------------------------------------------------------

    #[test]
    fn water_current_pushes_with_average_flow() {
        let mut world = TestWorld::new();
        world.fill((0, 0, 0), (0, 2, 0), states::FLOWING_WATER);
        let shapes = TestShapes::default();
        let attrs = attrs();
        let mut state = PlayerState::at(DVec3::new(0.5, 0.5, 0.5));
        state.in_water = true;
        state.velocity = DVec3::ZERO;
        apply_water_currents(&mut state, &attrs, &world, &shapes);
        let h = (8.0f32 / 9.0f32) as f64;
        let first = 1.0 * (h - 0.5); // height 8/9 - 0.5 < 0.4: scaled
        let accumulated = first + 1.0 + 1.0;
        let expected = accumulated / 3.0 * WATER_CURRENT_SCALE;
        assert_eq!(state.velocity.x, expected);
        assert_eq!(state.velocity.y, 0.0);
        assert_eq!(state.velocity.z, 0.0);
    }

    #[test]
    fn water_current_minimum_impulse_when_nearly_stationary() {
        let mut world = TestWorld::new();
        world.set(0, 0, 0, states::FLOWING_WATER_SLOW);
        let shapes = TestShapes::default();
        let attrs = attrs();
        let mut state = PlayerState::at(DVec3::new(0.5, 0.5, 0.5));
        state.in_water = true;
        state.velocity = DVec3::ZERO;
        apply_water_currents(&mut state, &attrs, &world, &shapes);
        assert_eq!(state.velocity.x, WATER_CURRENT_MIN_IMPULSE);
    }

    #[test]
    fn water_current_ignored_outside_water_and_without_flow() {
        let mut world = TestWorld::new();
        world.set(0, 0, 0, states::FLOWING_WATER);
        let shapes = TestShapes::default();
        let attrs = attrs();
        // Not in water.
        let mut state = PlayerState::at(DVec3::new(0.5, 0.5, 0.5));
        state.velocity = DVec3::new(1.0, 0.0, 0.0);
        apply_water_currents(&mut state, &attrs, &world, &shapes);
        assert_eq!(state.velocity.x, 1.0);
        // In water but the cell carries no flow vector.
        let mut world = TestWorld::new();
        world.set(0, 0, 0, states::WATER_SOURCE);
        let mut state = PlayerState::at(DVec3::new(0.5, 0.5, 0.5));
        state.in_water = true;
        state.velocity = DVec3::ZERO;
        apply_water_currents(&mut state, &attrs, &world, &shapes);
        assert_eq!(state.velocity, DVec3::ZERO);
    }

    #[test]
    fn aabb_cells_covers_the_box() {
        let aabb = Aabb::entity(DVec3::new(0.5, 0.5, 0.5), 0.6, 1.8);
        let cells = aabb_cells(&aabb);
        assert!(cells.contains(&BlockPos::new(0, 0, 0)));
        assert!(cells.contains(&BlockPos::new(0, 2, 0)));
        assert_eq!(cells.len(), 3);
    }
}
