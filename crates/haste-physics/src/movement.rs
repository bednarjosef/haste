//! The player tick: input -> rotation, flags and pose -> jump -> acceleration -> collision ->
//! block speed factor -> gravity and drag. Constants are named here and listed in `NOTES.md` with
//! their source (wiki page, derivation from documented speeds, or "assumption pending traces").
use haste_core::input::InputState;
use haste_core::view::{BlockView, CollisionShapes};
use haste_core::{Aabb, BlockPos, BlockStateId, DVec3};

use crate::block_effects;
use crate::climbing::{self, ClimbableBlocks, NoClimbables};
use crate::collision::{collide_with_step, no_collision, supporting_block};
use crate::elytra;
use crate::{
    fluids, flying, trig, BlockPhysics, FluidKind, PlayerAttributes, PlayerState, Pose, TickResult,
};

/// Box shrink before measuring fluid contact (assumption, as in `fluids`).
const FLUID_CONTACT_SHRINK: f64 = 0.001;

/// Velocity components with a smaller magnitude are zeroed at the start of the tick
/// (mcpk.wiki "Horizontal/Vertical Movement Formulas": 0.003 in 1.9+).
pub const VELOCITY_THRESHOLD: f64 = 0.003;
/// Horizontal length squared below which both horizontal velocity components are zeroed together
/// (verified: `LivingEntity.aiStep`, the player branch; 9.0E-6 = 0.003 squared).
pub const VELOCITY_ZERO_LEN_SQ: f64 = 9.0e-6;
/// Movement key impulses are scaled by this (mcpk.wiki "Default 0.98" multiplier; it is also what
/// makes the wiki's 4.317 m/s walking speed come out of 0.1 x 0.98 / (1 - 0.546)).
pub const INPUT_SCALE: f32 = 0.98;
/// Sprint multiplies movement speed by 1.3 (wiki "Sprinting": 30 % faster; mcpk.wiki 1.3).
pub const SPRINT_MULTIPLIER: f64 = 1.3;
/// Air acceleration (mcpk.wiki "Horizontal Movement Formulas": 0.02 x movement multiplier).
pub const AIR_ACCELERATION: f32 = 0.02;
/// Default block slipperiness (wiki "Entity": "Friction is 0.6 by default").
pub const DEFAULT_SLIPPERINESS: f32 = 0.6;
/// Horizontal drag factor multiplied with slipperiness (wiki "Entity": horizontal drag 0.91).
pub const HORIZONTAL_DRAG: f32 = 0.91;
/// Vertical drag in air (wiki "Entity": 0.98, float).
pub const VERTICAL_DRAG: f32 = 0.98;
/// Horizontal boost of a sprint jump, along the facing direction (mcpk.wiki "Horizontal Movement
/// Formulas"; checked against the wiki's 7.127 m/s sprint-jump speed in tests/scenarios.rs).
pub const SPRINT_JUMP_BOOST: f32 = 0.2;
/// Extra jump power per jump-boost level (vanilla `LivingEntity.getJumpBoostPower`: `0.1F` per
/// amplifier + 1, computed in float).
pub const JUMP_BOOST_POWER_PER_LEVEL: f32 = 0.1;
/// Levitation rise per level (vanilla `LivingEntity.travelInAir`: 0.05 x (amplifier + 1)).
pub const LEVITATION_RISE_PER_LEVEL: f64 = 0.05;
/// How fast levitation blends the vertical velocity towards its rise each tick (vanilla
/// `LivingEntity.travelInAir`: 0.2).
pub const LEVITATION_BLEND: f64 = 0.2;
/// Ticks between jumps while jump stays held (wiki "Jumping": half a second delay).
pub const JUMP_COOLDOWN_TICKS: u8 = 10;
/// Sneak edge back-off step (vanilla `Player.maybeBackOffFromEdge`: 0.05).
pub const EDGE_BACKOFF_STEP: f64 = 0.05;
/// Offset below the feet for the block that affects movement (verified:
/// `Entity.getBlockPosBelowThatAffectsMyMovement` = `getOnPos(0.500001F)`; the 0.500001 makes a
/// player standing on a half slab attribute the movement to the block under the slab).
pub const BLOCK_BELOW_OFFSET: f64 = 0.500001;
/// By how much the pose-fit and edge-probe boxes are shrunk (vanilla
/// `Player.canPlayerFitWithinBlocksAndEntitiesWhen` and `Player.canFallAtLeast`: 1.0E-7).
pub const POSE_TEST_EPSILON: f64 = 1.0e-7;

/// Movement impulse for this tick: (strafe, forward), already scaled.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Impulse {
    pub strafe: f32,
    pub forward: f32,
}

/// Depth of the deepest fluid around the player, measured from the feet (vanilla
/// `Entity.getFluidHeight` via `EntityFluidInteraction.update`: the highest fluid surface among
/// the cells the slightly shrunk box touches). Lava when the player is in lava, else water, like
/// the choice in `LivingEntity.aiStep`.
fn fluid_height(
    state: &PlayerState,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    blocks: &dyn BlockPhysics,
) -> f64 {
    let kind = if state.in_lava {
        FluidKind::Lava
    } else {
        FluidKind::Water
    };
    let b = state.aabb(attrs);
    let min = b.min + DVec3::splat(FLUID_CONTACT_SHRINK);
    let max = b.max - DVec3::splat(FLUID_CONTACT_SHRINK);
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

/// vanilla `Entity.getFluidJumpThreshold`: 0.4 unless the eye height is below 0.4 (then 0).
fn fluid_jump_threshold(attrs: &PlayerAttributes, pose: Pose) -> f64 {
    if attrs.eye_height_for(pose) < fluids::FLUID_JUMP_THRESHOLD {
        0.0
    } else {
        fluids::FLUID_JUMP_THRESHOLD
    }
}

/// One 50 ms player tick without a climbable table: nothing climbs (see [`tick_with`]).
pub fn tick(
    state: &mut PlayerState,
    input: &InputState,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    blocks: &dyn BlockPhysics,
) -> TickResult {
    tick_with(state, input, attrs, world, shapes, blocks, &NoClimbables)
}

/// One 50 ms player tick. `blocks` stands in for `GameData` until haste-data provides it (see
/// NOTES.md, interface requests); `climb` answers the climbable tag and trapdoor rule.
pub fn tick_with(
    state: &mut PlayerState,
    input: &InputState,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    blocks: &dyn BlockPhysics,
    climb: &dyn ClimbableBlocks,
) -> TickResult {
    let old_pos = state.pos;
    let old_ground = state.on_ground;
    let rotated = state.yaw != input.yaw || state.pitch != input.pitch;

    // Rotation comes straight from input.
    state.yaw = input.yaw;
    state.pitch = input.pitch.clamp(-90.0, 90.0);

    // Negligible velocities are cancelled first (vanilla `LivingEntity.aiStep`: for the player,
    // x and z are cancelled together when the horizontal length squared is below 9.0E-6;
    // y is cancelled on its own below 0.003).
    let v = &mut state.velocity;
    if v.x * v.x + v.z * v.z < VELOCITY_ZERO_LEN_SQ {
        v.x = 0.0;
        v.z = 0.0;
    }
    if v.y.abs() < VELOCITY_THRESHOLD {
        v.y = 0.0;
    }

    // Fluid contact and sneak/sprint flags (vanilla `Entity.baseTick` sets the fluid and
    // swimming state before `LivingEntity.aiStep` looks at the keys).
    let eyes_in_water = fluids::update_fluid_state(state, attrs, world, blocks);
    state.eyes_in_water = eyes_in_water;
    // Water currents (vanilla Entity.updateFluidInteraction, before the move).
    block_effects::apply_water_currents(state, attrs, world, blocks);
    update_flags(state, input, attrs, eyes_in_water, world, shapes, blocks);

    if state.jump_cooldown > 0 {
        state.jump_cooldown -= 1;
    }
    if state.flying {
        state.jump_cooldown = 0;
    } else if input.jump {
        // vanilla `LivingEntity.aiStep`, "jump" profiler section: in a fluid the player jumps
        // normally only while on ground with the fluid no deeper than the jump threshold;
        // otherwise holding jump swims up.
        let threshold = fluid_jump_threshold(attrs, state.pose);
        let height = fluid_height(state, attrs, world, blocks);
        let shallow = state.on_ground && height <= threshold;
        let jump_from_ground = state.on_ground && state.jump_cooldown == 0;
        if state.in_water || state.in_lava {
            if shallow {
                if jump_from_ground {
                    jump(state, attrs, world, shapes, blocks);
                    state.jump_cooldown = JUMP_COOLDOWN_TICKS;
                }
            } else {
                fluids::swim_up(state);
            }
        } else if jump_from_ground {
            jump(state, attrs, world, shapes, blocks);
            state.jump_cooldown = JUMP_COOLDOWN_TICKS;
        }
    } else {
        state.jump_cooldown = 0;
    }

    // The movement uses the pose box from the END of the last tick: vanilla runs
    // `Player.updatePlayerPose` after `LivingEntity.aiStep`'s travel, inside `Player.tick`.
    let impulse = impulse(state, input, attrs);

    // Vanilla `LivingEntity.aiStep`, just before `travel`: either effect resets the fall
    // distance, so slow falling and levitation never accumulate fall damage.
    if state.slow_falling || state.levitation.is_some() {
        state.fall_distance = 0.0;
    }

    if state.flying {
        // vanilla `LocalPlayer.aiStep`: while flying, jump/sneak steer y directly by three
        // times the flying speed per tick, before the travel.
        flying::apply_vertical_impulse(state, input, attrs);
        if state.in_water || state.in_lava {
            // vanilla `Player.travel`: with flight on, the y velocity is saved, the normal travel
            // runs (the fluid one in water/lava: `LivingEntity.travel` -> `travelInFluid`), then y
            // is restored to the saved value times 0.6.
            let saved_y = state.velocity.y;
            if state.in_water {
                fluids::travel_water(state, input, impulse, attrs, world, shapes, blocks, climb);
            } else {
                fluids::travel_lava(state, input, impulse, attrs, world, shapes, blocks);
            }
            state.velocity.y = saved_y * flying::VERTICAL_FLY_DRAG;
            // Landing ends creative flight, except for spectators (vanilla `LocalPlayer.aiStep`:
            // `onGround() && abilities.flying && !gameMode.isSpectator()`).
            if state.on_ground && !state.spectating {
                state.flying = false;
            }
        } else {
            flying::travel(state, input, impulse, attrs, world, shapes, blocks);
        }
    } else if state.in_water {
        // vanilla `LivingEntity.travel`: the fluid branch wins over gliding (entering water
        // switches a glider to swim physics; the server clears the flag separately).
        fluids::travel_water(state, input, impulse, attrs, world, shapes, blocks, climb);
    } else if state.in_lava {
        fluids::travel_lava(state, input, impulse, attrs, world, shapes, blocks);
    } else if state.fall_flying && !climbing::on_climbable(state, world, climb) {
        // vanilla `LivingEntity.travel` -> `travelFallFlying`, which uses the effective gravity
        // (the slow falling clamp) like the air branch.
        elytra::travel(state, attrs, world, shapes, blocks, state.slow_falling);
    } else {
        // `travelFallFlying` on a climbable: travel in air (still with the glider box), then
        // stop gliding.
        travel_normal(
            state, input.jump, impulse, attrs, world, shapes, blocks, climb,
        );
        state.fall_flying = false;
    }

    // vanilla `Entity.move`: the flag next tick's `shouldStopRunSprinting` reads - whether the
    // input direction and the movement that actually resulted (the velocity after the move, its
    // collided axes zeroed) were within 8 degrees (`LocalPlayer.isHorizontalCollisionMinor`).
    // The post-move drags only scale the velocity, so the direction is unchanged.
    state.minor_horizontal_collision = if state.horizontal_collision {
        minor_horizontal_collision(state, &impulse, state.velocity)
    } else {
        false
    };
    // End of tick (vanilla `LivingEntity.aiStep` -> `Entity.applyEffectsFromBlocks`): slime
    // stepOn first, then the entityInside scan (honey slide, stuck-in-block multipliers).
    block_effects::apply_end_of_tick_effects(state, attrs, world, shapes, blocks);
    // Vanilla `Player.tick` ends with `updatePlayerPose`: the new pose's box applies from
    // the next tick's movement, and the camera picks up its eye height the tick after that
    // (vanilla `Camera.tick` runs before the player's tick).
    update_player_pose(state, attrs, world, shapes);

    TickResult {
        moved: state.pos != old_pos,
        rotated,
        on_ground_changed: state.on_ground != old_ground,
        collided_horizontally: state.horizontal_collision,
    }
}

/// The input-driven flags for this tick's movement: `swimming` (vanilla `Entity.baseTick`), then
/// `sneaking` (raw shift key), `crouching` and the sprint state (vanilla `LocalPlayer.aiStep`, in
/// that order, from the CURRENT box, i.e. the pose the last tick ended with, before the player
/// moves).
fn update_flags(
    state: &mut PlayerState,
    input: &InputState,
    attrs: &PlayerAttributes,
    eyes_in_water: bool,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    blocks: &dyn BlockPhysics,
) {
    // `Entity.baseTick` updates the swimming state before `aiStep` looks at the keys.
    let under_water = eyes_in_water && state.in_water;
    crate::sprint::update_swimming(state, under_water, world, blocks);
    state.sneaking = input.sneak;
    // Crouching (vanilla `LocalPlayer.aiStep`): not flying, not swimming, the crouching box
    // fits, and shift is held or standing does not fit (forced crouch under a low ceiling).
    // Riding and sleeping are not modelled.
    state.crouching = !state.flying
        && !state.swimming
        && fits_pose(state, attrs, Pose::Crouching, world, shapes)
        && (state.sneaking || !fits_pose(state, attrs, Pose::Standing, world, shapes));
    crate::sprint::update_sprinting(state, input, under_water);
}

/// Whether the box of `pose` at the current position fits (vanilla
/// `Player.canPlayerFitWithinBlocksAndEntitiesWhen`: the test box is deflated by 1.0E-7).
fn fits_pose(
    state: &PlayerState,
    attrs: &PlayerAttributes,
    pose: Pose,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
) -> bool {
    no_collision(
        &deflate(&Aabb::entity(
            state.pos,
            attrs.box_width(),
            attrs.height_for(pose),
        )),
        world,
        shapes,
    )
}

/// Shrinks a box on every side (vanilla `AABB.deflate`).
fn deflate(aabb: &Aabb) -> Aabb {
    Aabb {
        min: aabb.min + DVec3::splat(POSE_TEST_EPSILON),
        max: aabb.max - DVec3::splat(POSE_TEST_EPSILON),
    }
}

/// Vanilla `Player.updatePlayerPose`, run at the end of `Player.tick` (after the movement):
/// the new pose's box applies from the next tick. When even the swimming box does not fit
/// (inside a block), the pose is left alone.
fn update_player_pose(
    state: &mut PlayerState,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
) {
    let fits = |p: Pose| fits_pose(state, attrs, p, world, shapes);
    if !fits(Pose::Swimming) {
        return;
    }
    // Vanilla `Player.getDesiredPose` (the physics subset: swimming, fall flying, crouch,
    // standing): swimming wins, then the shared flag 7's fall-flying pose, then the crouch.
    let desired = if state.swimming {
        Pose::Swimming
    } else if state.fall_flying {
        Pose::FallFlying
    } else if state.sneaking && !state.flying {
        Pose::Crouching
    } else {
        Pose::Standing
    };
    state.pose = if fits(desired) {
        desired
    } else if fits(Pose::Crouching) {
        Pose::Crouching
    } else {
        Pose::Swimming
    };
}

/// The movement key vector (f32 throughout, in vanilla's order): `KeyboardInput.tick` builds
/// (left - right, forward - back) and normalises it (`Vec2.normalized`, so a diagonal is
/// (0.707, 0.707)); `LocalPlayer.modifyInput` then scales it by 0.98, by the sneak factor while
/// moving slowly (`LocalPlayer.isMovingSlowly`: `isCrouching() || isVisuallyCrawling()`, the
/// latter being the swimming pose outside water) and finally stretches it for square movement.
/// The item-use slowdown is not modelled yet.
fn impulse(state: &PlayerState, input: &InputState, attrs: &PlayerAttributes) -> Impulse {
    let forward = (input.forward as i32 - input.back as i32) as f32;
    let strafe = (input.left as i32 - input.right as i32) as f32;
    let dist = (strafe * strafe + forward * forward).sqrt();
    if dist < 1.0e-4 {
        return Impulse::default();
    }
    let mut imp = Impulse {
        strafe: strafe / dist * INPUT_SCALE,
        forward: forward / dist * INPUT_SCALE,
    };
    let slow = state.crouching || (state.pose == Pose::Swimming && !state.in_water);
    if slow {
        let factor = attrs.sneaking_speed as f32;
        imp.strafe *= factor;
        imp.forward *= factor;
    }
    square_movement(imp)
}

/// Vanilla `LocalPlayer.modifyInputSpeedForSquareMovement`: the input vector is stretched by
/// its direction's distance to the unit square (`distanceToUnitSquare`: sqrt(1 + tan^2) of the
/// angle to the nearer axis, 1 on an axis, sqrt(2) on the diagonal) and capped at length 1. A
/// full-speed diagonal lands on the cap, a slowed diagonal (sneaking, item use) keeps its
/// per-axis speed instead of being 29 % slower (wiki "Sneaking": 1.3 m/s straight, 1.8 m/s
/// diagonal).
pub(crate) fn square_movement(imp: Impulse) -> Impulse {
    let length = (imp.strafe * imp.strafe + imp.forward * imp.forward).sqrt();
    if length <= 0.0 {
        return imp;
    }
    let inv = 1.0 / length;
    let (dx, dy) = (imp.strafe * inv, imp.forward * inv);
    let (ax, ay) = (dx.abs(), dy.abs());
    let tan = if ay > ax { ax / ay } else { ay / ax };
    let to_unit_square = (1.0 + tan * tan).sqrt();
    let modified = (length * to_unit_square).min(1.0);
    Impulse {
        strafe: dx * modified,
        forward: dy * modified,
    }
}

/// Adds the input acceleration, rotated by yaw. The impulse vector is normalised when longer than
/// 1 (this is what makes 45 degree walking slightly faster: wiki "Walking").
pub(crate) fn accelerate(state: &mut PlayerState, imp: Impulse, speed: f32) {
    let (mut s, mut f) = (imp.strafe as f64, imp.forward as f64);
    let len2 = s * s + f * f;
    if len2 < 1.0e-7 {
        return;
    }
    if len2 > 1.0 {
        let l = len2.sqrt();
        s /= l;
        f /= l;
    }
    s *= speed as f64;
    f *= speed as f64;
    let r = trig::deg_to_rad(state.yaw);
    let (sin, cos) = (trig::sin(r) as f64, trig::cos(r) as f64);
    state.velocity.x += s * cos - f * sin;
    state.velocity.z += f * cos + s * sin;
}

fn block_below(state: &PlayerState) -> BlockPos {
    BlockPos::from_dvec3(DVec3::new(
        state.pos.x,
        state.pos.y - BLOCK_BELOW_OFFSET,
        state.pos.z,
    ))
}

/// The block whose friction/jump/speed factor applies: vanilla
/// `Entity.getBlockPosBelowThatAffectsMyMovement` = `getOnPos(0.500001F)` (Entity.java:1090).
/// On the ground, the supporting block's X/Z with `floor(y - 0.500001)` (a wall or fence gate
/// keeps the supporting pos - the same cell while the player stands on its 1.5 tall top - and a
/// fence lands on that same cell either way); airborne or unsupported, `floor(pos - 0.500001)`.
pub(crate) fn ground_block(
    state: &PlayerState,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
) -> BlockStateId {
    if state.on_ground {
        if let Some(p) = supporting_block(&state.aabb(attrs), world, shapes) {
            let y = (state.pos.y - BLOCK_BELOW_OFFSET).floor() as i32;
            return world.block_state(BlockPos::new(p.x, y, p.z));
        }
    }
    world.block_state(block_below(state))
}

/// Speed/jump factor: the block at the feet if it has a non-default factor (soul sand and honey
/// are shorter than a block, so the player stands inside their cell), else the block below
/// (vanilla `Entity.getBlockJumpFactor` / `Entity.getBlockSpeedFactor`).
fn block_factor(
    state: &PlayerState,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    f: impl Fn(BlockStateId) -> f32,
) -> f32 {
    let at = f(world.block_state(BlockPos::from_dvec3(state.pos)));
    if at != 1.0 {
        at
    } else {
        f(ground_block(state, attrs, world, shapes))
    }
}

/// Angle below which a horizontal collision counts as minor (vanilla
/// `LocalPlayer.isHorizontalCollisionMinor`: 0.13962634 rad, i.e. 8 degrees).
pub const MINOR_COLLISION_ANGLE: f64 = 0.13962634;

/// vanilla `LocalPlayer.isHorizontalCollisionMinor`: the angle between the input direction
/// (rotated by yaw) and the movement the move attempted; minor when below 8 degrees, and false
/// when either vector is near zero.
fn minor_horizontal_collision(state: &PlayerState, imp: &Impulse, attempted: DVec3) -> bool {
    let r = trig::deg_to_rad(state.yaw);
    let (sin, cos) = (trig::sin(r) as f64, trig::cos(r) as f64);
    let want_x = imp.strafe as f64 * cos - imp.forward as f64 * sin;
    let want_z = imp.forward as f64 * cos + imp.strafe as f64 * sin;
    let want_len_sq = want_x * want_x + want_z * want_z;
    let got_len_sq = attempted.x * attempted.x + attempted.z * attempted.z;
    if want_len_sq < 1.0e-5 || got_len_sq < 1.0e-5 {
        return false;
    }
    let dot = want_x * attempted.x + want_z * attempted.z;
    let angle = (dot / (want_len_sq * got_len_sq).sqrt()).acos();
    angle < MINOR_COLLISION_ANGLE
}

fn jump(
    state: &mut PlayerState,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    blocks: &dyn BlockPhysics,
) {
    let factor = block_factor(state, attrs, world, shapes, |s| blocks.jump_factor(s));
    // vanilla `LivingEntity.getJumpPower` (jump_strength x block jump factor) plus
    // `getJumpBoostPower` (0.1 per jump-boost level); `jumpFromGround` (LivingEntity.java:2372)
    // then takes y = max(jump power, current y), so jumping keeps a faster rise. The sprint boost
    // uses the same deg->rad conversion as the movement code (no separate jump constant).
    let power = attrs.jump_strength * factor as f64
        + state.jump_boost.map_or(0.0, |amplifier| {
            f64::from(JUMP_BOOST_POWER_PER_LEVEL * (amplifier as f32 + 1.0))
        });
    if power > 1.0e-5 {
        state.velocity.y = power.max(state.velocity.y);
        if state.sprinting {
            let r = trig::deg_to_rad(state.yaw);
            state.velocity.x -= (trig::sin(r) * SPRINT_JUMP_BOOST) as f64;
            state.velocity.z += (trig::cos(r) * SPRINT_JUMP_BOOST) as f64;
        }
    }
}

/// Walking/falling in air (vanilla `LivingEntity.travelInAir`): accelerate, the climbable clamp,
/// move, the climb boost, block speed factor, gravity + vertical drag, horizontal drag
/// (slipperiness x 0.91 on ground, 0.91 in air). `jumping` is the jump key
/// (`LocalPlayer.applyInput`).
#[allow(clippy::too_many_arguments)]
fn travel_normal(
    state: &mut PlayerState,
    jumping: bool,
    imp: Impulse,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    blocks: &dyn BlockPhysics,
    climb: &dyn ClimbableBlocks,
) {
    let slip = if state.on_ground {
        blocks.friction(ground_block(state, attrs, world, shapes))
    } else {
        1.0
    };
    let sprint = if state.sprinting {
        SPRINT_MULTIPLIER
    } else {
        1.0
    };
    let speed = if state.on_ground {
        let k = DEFAULT_SLIPPERINESS / slip;
        (attrs.movement_speed * sprint) as f32 * (k * k * k)
    } else {
        AIR_ACCELERATION * sprint as f32
    };
    accelerate(state, imp, speed);
    climbing::handle_on_climbable(state, world, climb);
    let motion = state.velocity;
    move_entity(state, attrs, motion, world, shapes, blocks);
    climbing::climb_after_move(state, jumping, world, climb);
    apply_speed_factor(state, world, blocks);
    // Vanilla `LivingEntity.travelInAir`: levitation blends the vertical velocity towards
    // `0.05 * (amplifier + 1)` per tick and replaces gravity entirely; otherwise the effective
    // gravity applies (slow falling caps it at 0.01 while falling, `getEffectiveGravity`).
    if let Some(amplifier) = state.levitation {
        let rise = LEVITATION_RISE_PER_LEVEL * f64::from(amplifier + 1);
        state.velocity.y += (rise - state.velocity.y) * LEVITATION_BLEND;
    } else {
        state.velocity.y -= elytra::effective_gravity(attrs, state.velocity, state.slow_falling);
    }
    state.velocity.y *= VERTICAL_DRAG as f64;
    let drag = (slip * HORIZONTAL_DRAG) as f64;
    state.velocity.x *= drag;
    state.velocity.z *= drag;
}

/// Block speed factor (soul sand, honey) on horizontal velocity after the move. The order is
/// derived from the wiki "Walking" table: soul sand 2.508 m/s = 20 x 0.098 / (1 - 0.4 x 0.546),
/// i.e. the factor multiplies the velocity that carries into the next tick, not this tick's move.
/// The factor block is the block at the feet if non-default, else the block below (vanilla
/// `Entity.getBlockSpeedFactor` -> `getOnPos(0.500001F)`; the fluids path keeps this function's
/// signature, so the below block here is `floor(pos - 0.500001)` without the supporting-block
/// refinement - see NOTES.md). While the feet are in water (or a bubble column) vanilla returns
/// the feet factor alone, so a soul sand floor underwater does not slow the player.
pub(crate) fn apply_speed_factor(
    state: &mut PlayerState,
    world: &dyn BlockView,
    blocks: &dyn BlockPhysics,
) {
    let feet = world.block_state(BlockPos::from_dvec3(state.pos));
    let at = blocks.speed_factor(feet);
    let in_water = blocks
        .fluid(feet)
        .is_some_and(|f| f.kind == FluidKind::Water);
    let f = if in_water || at != 1.0 {
        at
    } else {
        blocks.speed_factor(world.block_state(block_below(state)))
    };
    if f != 1.0 {
        state.velocity.x *= f as f64;
        state.velocity.z *= f as f64;
    }
}

/// Moves by `motion` with sneak edge back-off and step-up, updates position, collision flags,
/// on-ground, fall distance and zeroes velocity on collided axes. Returns the actual movement.
///
/// The edge back-off is vanilla `Player.maybeBackOffFromEdge` (shift held, not flying, not
/// moving up, `Player.isStayingOnGroundSurface` and `Player.isAboveGround`): the horizontal
/// motion is shrunk in 0.05 steps until the player would no longer fall.
pub(crate) fn move_entity(
    state: &mut PlayerState,
    attrs: &PlayerAttributes,
    motion: DVec3,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    blocks: &dyn BlockPhysics,
) -> DVec3 {
    // Vanilla `Entity.move`'s noPhysics branch (`Player.tick` sets it in spectator): add the
    // motion without collision and report no collisions.
    if state.no_physics {
        state.pos += motion;
        state.horizontal_collision = false;
        state.vertical_collision = false;
        return motion;
    }
    let aabb = state.aabb(attrs);
    let mut motion = block_effects::consume_stuck_multiplier(state, motion);
    if state.sneaking
        && !state.flying
        && motion.y <= 0.0
        && above_ground(state, &aabb, attrs, world, shapes)
    {
        motion = back_off_from_edge(&aabb, motion, attrs.step_height, world, shapes);
    }
    let moved = collide_with_step(
        aabb,
        motion,
        attrs.step_height,
        state.on_ground,
        world,
        shapes,
    );
    state.pos += moved;

    let hx = moved.x != motion.x;
    let hz = moved.z != motion.z;
    state.horizontal_collision = hx || hz;
    state.vertical_collision = moved.y != motion.y;
    state.on_ground = state.vertical_collision && motion.y < 0.0;
    if hx {
        state.velocity.x = 0.0;
    }
    if hz {
        state.velocity.z = 0.0;
    }
    if state.vertical_collision {
        // Slime bounce (Entity.restituteMovementAfterCollisions) or zero.
        let ground = ground_block(state, attrs, world, shapes);
        state.velocity.y = block_effects::vertical_collision_velocity(
            state,
            blocks,
            ground,
            motion.y,
            moved.y,
            attrs.gravity,
            VERTICAL_DRAG as f64,
        );
    }

    if state.on_ground || state.flying || state.in_water {
        state.fall_distance = 0.0;
    } else if moved.y < 0.0 {
        state.fall_distance -= moved.y as f32;
    }
    moved
}

/// Vanilla `Player.isAboveGround`: on the ground, or fallen less than a step and unable to fall
/// the rest of it (wiki "Sneaking": edge protection works "up to 0.6 blocks above the ground"
/// since 1.16.2).
fn above_ground(
    state: &PlayerState,
    aabb: &Aabb,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
) -> bool {
    state.on_ground
        || ((state.fall_distance as f64) < attrs.step_height
            && !can_fall_at_least(
                aabb,
                0.0,
                0.0,
                attrs.step_height - f64::from(state.fall_distance),
                world,
                shapes,
            ))
}

/// Vanilla `Player.canFallAtLeast`: the XZ footprint moved by `(dx, dz)` and shrunk by 1.0E-7,
/// between a hair below the feet and `height` below them, hits nothing the player can stand on.
fn can_fall_at_least(
    aabb: &Aabb,
    dx: f64,
    dz: f64,
    height: f64,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
) -> bool {
    no_collision(
        &Aabb {
            min: DVec3::new(
                aabb.min.x + POSE_TEST_EPSILON + dx,
                aabb.min.y - height - POSE_TEST_EPSILON,
                aabb.min.z + POSE_TEST_EPSILON + dz,
            ),
            max: DVec3::new(
                aabb.max.x - POSE_TEST_EPSILON + dx,
                aabb.min.y,
                aabb.max.z - POSE_TEST_EPSILON + dz,
            ),
        },
        world,
        shapes,
    )
}

/// Shrinks horizontal motion in `EDGE_BACKOFF_STEP` steps (vanilla
/// `Player.maybeBackOffFromEdge`): X alone, Z alone, then both together, until the player can no
/// longer fall at least the step height below.
fn back_off_from_edge(
    aabb: &Aabb,
    motion: DVec3,
    step: f64,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
) -> DVec3 {
    let (mut x, mut z) = (motion.x, motion.z);
    let step_x = 0.05 * x.signum();
    let step_z = 0.05 * z.signum();
    let falls = |dx: f64, dz: f64| can_fall_at_least(aabb, dx, dz, step, world, shapes);
    while x != 0.0 && falls(x, 0.0) {
        if x.abs() <= EDGE_BACKOFF_STEP {
            x = 0.0;
            break;
        }
        x -= step_x;
    }
    while z != 0.0 && falls(0.0, z) {
        if z.abs() <= EDGE_BACKOFF_STEP {
            z = 0.0;
            break;
        }
        z -= step_z;
    }
    while x != 0.0 && z != 0.0 && falls(x, z) {
        if x.abs() <= EDGE_BACKOFF_STEP {
            x = 0.0;
        } else {
            x -= step_x;
        }
        if z.abs() <= EDGE_BACKOFF_STEP {
            z = 0.0;
        } else {
            z -= step_z;
        }
    }
    DVec3::new(x, motion.y, z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{states::*, TestShapes, TestWorld};
    use haste_core::input::InputState;

    fn settle(w: &TestWorld, s: &mut PlayerState, attrs: &PlayerAttributes, shapes: &TestShapes) {
        for _ in 0..3 {
            tick(s, &InputState::default(), attrs, w, shapes, shapes);
        }
    }

    fn fence_pool(fluid: haste_core::BlockStateId, deep: bool) -> TestWorld {
        let mut w = TestWorld::flat(0, STONE);
        w.set(0, 0, 0, FENCE_POST); // 1.5 tall post to stand on
        w.set(0, 1, 0, fluid);
        if deep {
            w.set(0, 2, 0, fluid); // surface 2 + 8/9: depth 1.389 > 0.4
        }
        w // surface 1 + 8/9: depth from the feet 0.389 <= 0.4
    }

    /// A stone floor with the fluid up to y = 19: a player at y = 10 is fully submerged.
    fn deep_fluid(fluid: haste_core::BlockStateId) -> TestWorld {
        let mut w = TestWorld::flat(0, STONE);
        w.fill((-50, 0, -50), (50, 19, 50), fluid);
        w
    }

    /// vanilla `Player.travel`: a flying player in water or lava runs the fluid travel
    /// (`LivingEntity.travel` -> `travelInFluid`), not the flight acceleration, so horizontal
    /// flight in a fluid is far slower than in air.
    #[test]
    fn flying_in_fluid_covers_less_distance_than_flying_in_air() {
        let shapes = TestShapes::default();
        let attrs = PlayerAttributes::default();
        let input = InputState {
            forward: true,
            ..Default::default()
        };
        let mut air = PlayerState::at(DVec3::new(0.5, 100.0, 0.5));
        air.flying = true;
        let mut water = PlayerState::at(DVec3::new(0.5, 10.0, 0.5));
        water.flying = true;
        let mut lava = PlayerState::at(DVec3::new(0.5, 10.0, 0.5));
        lava.flying = true;
        for _ in 0..20 {
            tick(
                &mut air,
                &input,
                &attrs,
                &TestWorld::new(),
                &shapes,
                &shapes,
            );
            tick(
                &mut water,
                &input,
                &attrs,
                &deep_fluid(WATER_SOURCE),
                &shapes,
                &shapes,
            );
            tick(
                &mut lava,
                &input,
                &attrs,
                &deep_fluid(LAVA_SOURCE),
                &shapes,
                &shapes,
            );
        }
        let air_d = air.pos.z - 0.5;
        assert!(
            water.pos.z - 0.5 < air_d * 0.5,
            "flying in water covers {} blocks against {} in air",
            water.pos.z - 0.5,
            air_d
        );
        assert!(
            lava.pos.z - 0.5 < air_d * 0.5,
            "flying in lava covers {} blocks against {} in air",
            lava.pos.z - 0.5,
            air_d
        );
        assert_eq!(water.pos.y, 10.0, "the y restore keeps the flyer hovering");
        assert_eq!(water.velocity.y, 0.0);
    }

    /// Hand-computed from vanilla's formulas for a flying player in deep water moving forward
    /// from rest (`LivingEntity.travelInWater` + `Player.travel`): each tick adds 0.98 x 0.02
    /// (the input scale x the fluid acceleration) to z, moves by it, drags z by the water drag
    /// 0.8, and `Player.travel` restores y to the tick-start value times 0.6 - 0 every tick here,
    /// so the flyer hovers.
    #[test]
    fn flying_in_water_matches_hand_computed_fluid_travel() {
        let shapes = TestShapes::default();
        let attrs = PlayerAttributes::default();
        let input = InputState {
            forward: true,
            ..Default::default()
        };
        let mut s = PlayerState::at(DVec3::new(0.5, 10.0, 0.5));
        s.flying = true;
        let a = INPUT_SCALE as f64 * fluids::FLUID_ACCELERATION as f64;
        let v1 = a;
        let v2 = v1 * fluids::WATER_DRAG + a;
        let v3 = v2 * fluids::WATER_DRAG + a;
        for _ in 0..3 {
            tick(
                &mut s,
                &input,
                &attrs,
                &deep_fluid(WATER_SOURCE),
                &shapes,
                &shapes,
            );
        }
        assert!(
            (s.pos.z - (0.5 + v1 + v2 + v3)).abs() < 1e-12,
            "pos {:?}",
            s.pos
        );
        assert_eq!(s.pos.y, 10.0, "hovering: y never moves");
        assert_eq!(
            s.velocity.y, 0.0,
            "y restored to the tick-start value times 0.6"
        );
    }

    #[test]
    fn a_player_at_a_pool_edge_is_not_pushed_out_of_the_water() {
        // The live rig: a stone floor (top y 0) with source water z >= 1 and air z < 1. A
        // player standing in the shallow water at the edge must stay put: vanilla's getFlow
        // gives the edge cell no flow (the empty neighbour's below cell has no fluid), so
        // `apply_water_currents` adds nothing.
        let mut w = TestWorld::flat(0, STONE);
        for x in -2..=2 {
            for z in 1..=4 {
                w.set(x, 0, z, WATER_SOURCE);
            }
        }
        let shapes = TestShapes::default();
        let attrs = PlayerAttributes::default();
        let mut s = PlayerState::at(DVec3::new(0.5, 0.0, 1.5));
        for _ in 0..100 {
            tick(&mut s, &InputState::default(), &attrs, &w, &shapes, &shapes);
        }
        assert!(
            (s.pos.z - 1.5).abs() < 1.0e-9,
            "the player drifted to {s:?}"
        );
        assert!(s.pos.x == 0.5, "the player drifted to {s:?}");
    }

    #[test]
    fn jump_in_shallow_water_on_ground_is_a_normal_jump() {
        // vanilla `LivingEntity.aiStep`, "jump" section: on ground with the fluid no deeper than
        // the jump threshold, the player jumps normally instead of swimming up.
        let w = fence_pool(WATER_SOURCE, false);
        let shapes = TestShapes::default();
        let attrs = PlayerAttributes::default();
        let mut s = PlayerState::at(DVec3::new(0.5, 1.5, 0.5));
        settle(&w, &mut s, &attrs, &shapes);
        assert!(s.on_ground, "standing on the post");
        assert!(s.in_water, "the cell above the post is water");
        s.velocity = DVec3::ZERO;
        let input = InputState {
            jump: true,
            ..Default::default()
        };
        tick(&mut s, &input, &attrs, &w, &shapes, &shapes);
        // jumpFromGround: y = 0.42; the move lifts 0.42, the water drag 0.8, then the water
        // gravity sink gravity/16.
        assert_eq!(s.pos.y, 1.92);
        assert!(
            (s.velocity.y - (0.42 * 0.8 - 0.08 / 16.0)).abs() < 1e-12,
            "{}",
            s.velocity.y
        );
        assert_eq!(s.jump_cooldown, JUMP_COOLDOWN_TICKS);
    }

    #[test]
    fn jump_in_deep_water_on_the_ground_swims_up() {
        let w = fence_pool(WATER_SOURCE, true);
        let shapes = TestShapes::default();
        let attrs = PlayerAttributes::default();
        let mut s = PlayerState::at(DVec3::new(0.5, 1.5, 0.5));
        settle(&w, &mut s, &attrs, &shapes);
        assert!(s.on_ground && s.in_water);
        s.velocity = DVec3::ZERO;
        let input = InputState {
            jump: true,
            ..Default::default()
        };
        tick(&mut s, &input, &attrs, &w, &shapes, &shapes);
        // jumpInLiquid: y = +0.04; the move lifts 0.04, the water drag 0.8, then the sink.
        assert_eq!(s.pos.y, 1.54);
        assert!(
            (s.velocity.y - (0.04 * 0.8 - 0.08 / 16.0)).abs() < 1e-12,
            "{}",
            s.velocity.y
        );
        assert_eq!(s.jump_cooldown, 0, "no jump cooldown when swimming up");
    }

    #[test]
    fn jump_in_shallow_lava_on_ground_is_a_normal_jump() {
        let w = fence_pool(LAVA_SOURCE, false);
        let shapes = TestShapes::default();
        let attrs = PlayerAttributes::default();
        let mut s = PlayerState::at(DVec3::new(0.5, 1.5, 0.5));
        settle(&w, &mut s, &attrs, &shapes);
        assert!(s.on_ground && s.in_lava);
        s.velocity = DVec3::ZERO;
        let input = InputState {
            jump: true,
            ..Default::default()
        };
        tick(&mut s, &input, &attrs, &w, &shapes, &shapes);
        assert_eq!(s.pos.y, 1.92);
        // jumpFromGround y = 0.42; the move lifts 0.42, the shallow-lava vertical drag 0.8, the
        // sink gravity/16, then gravity/4.
        assert!(
            (s.velocity.y - (0.42 * 0.8 - 0.08 / 16.0 - 0.08 / 4.0)).abs() < 1e-12,
            "{}",
            s.velocity.y
        );
    }

    #[test]
    fn jump_keeps_a_faster_rise() {
        // vanilla `LivingEntity.jumpFromGround`: y = max(jump power, current y).
        let w = TestWorld::flat(0, STONE);
        let shapes = TestShapes::default();
        let attrs = PlayerAttributes::default();
        let mut s = PlayerState::at(DVec3::new(0.5, 0.0, 0.5));
        s.on_ground = true;
        s.velocity.y = 0.5; // already rising faster than the jump power
        let input = InputState {
            jump: true,
            ..Default::default()
        };
        tick(&mut s, &input, &attrs, &w, &shapes, &shapes);
        assert_eq!(s.pos.y, 0.5, "y keeps the faster 0.5 rise");
        assert!(
            (s.velocity.y - (0.5 - attrs.gravity) * VERTICAL_DRAG as f64).abs() < 1e-12,
            "{}",
            s.velocity.y
        );
    }

    #[test]
    fn sprint_jump_boost_uses_the_movement_deg_to_rad() {
        // vanilla `LivingEntity.jumpFromGround`: the boost angle is yaw * (float)(PI/180), the
        // same conversion as the movement code.
        let w = TestWorld::flat(0, STONE);
        let shapes = TestShapes::default();
        let attrs = PlayerAttributes::default();
        let mut s = PlayerState::at(DVec3::new(0.5, 0.0, 0.5));
        s.on_ground = true;
        let input = InputState {
            forward: true,
            sprint: true,
            jump: true,
            yaw: 30.0,
            ..Default::default()
        };
        tick(&mut s, &input, &attrs, &w, &shapes, &shapes);
        let r = trig::deg_to_rad(30.0);
        let (sin, cos) = (trig::sin(r), trig::cos(r));
        let speed = (attrs.movement_speed * SPRINT_MULTIPLIER) as f32;
        let accel = INPUT_SCALE as f64 * speed as f64;
        let drag = (DEFAULT_SLIPPERINESS * HORIZONTAL_DRAG) as f64;
        assert_eq!(
            s.velocity.x,
            (-(sin * SPRINT_JUMP_BOOST) as f64 - accel * sin as f64) * drag
        );
        assert_eq!(
            s.velocity.z,
            ((cos * SPRINT_JUMP_BOOST) as f64 + accel * cos as f64) * drag
        );
        assert_eq!(
            s.velocity.y,
            (attrs.jump_strength - attrs.gravity) * VERTICAL_DRAG as f64
        );
    }

    #[test]
    fn player_velocity_threshold_zeroes_horizontal_components_together() {
        // vanilla `LivingEntity.aiStep`: the player zeroes x and z together when the horizontal
        // length squared is below 9.0E-6; y on its own below 0.003.
        let w = TestWorld::flat(0, STONE);
        let shapes = TestShapes::default();
        let attrs = PlayerAttributes::default();
        let drag = (DEFAULT_SLIPPERINESS * HORIZONTAL_DRAG) as f64;
        // x^2 + z^2 = 2.0E-5 >= 9.0E-6: neither component is zeroed (z alone is below 0.003).
        let mut s = PlayerState::at(DVec3::new(0.5, 0.0, 0.5));
        s.on_ground = true;
        s.velocity = DVec3::new(0.004, 0.5, 0.002);
        tick(&mut s, &InputState::default(), &attrs, &w, &shapes, &shapes);
        assert!(
            (s.velocity.x - 0.004 * drag).abs() < 1e-15,
            "{}",
            s.velocity.x
        );
        assert!(
            (s.velocity.z - 0.002 * drag).abs() < 1e-15,
            "{}",
            s.velocity.z
        );
        // x^2 + z^2 = 2.0E-6 < 9.0E-6: both are zeroed together.
        let mut s = PlayerState::at(DVec3::new(0.5, 0.0, 0.5));
        s.on_ground = true;
        s.velocity = DVec3::new(0.001, 0.5, 0.001);
        tick(&mut s, &InputState::default(), &attrs, &w, &shapes, &shapes);
        assert_eq!(s.velocity.x, 0.0);
        assert_eq!(s.velocity.z, 0.0);
    }

    #[test]
    fn sneak_edge_probe_reaches_a_ledge_exactly_step_below_the_feet() {
        // vanilla `Player.canFallAtLeast`: the probe is the slab from minY - step - 1e-7 to
        // minY, so a block top exactly `step` below the feet is still reached.
        let mut w = TestWorld::new();
        w.set(0, -1, 0, STONE); // the block the player stands on (top y = 0)
        w.set(1, 0, 0, LEDGE_0_6); // top at -0.6, exactly the step height below the feet
        let shapes = TestShapes::default();
        let aabb = Aabb::entity(DVec3::new(0.5, 0.0, 0.5), 0.6, 1.8);
        let kept = back_off_from_edge(&aabb, DVec3::new(0.8, 0.0, 0.0), 0.6, &w, &shapes);
        assert_eq!(kept.x, 0.8, "the probe reaches step + 1e-7 below the feet");
    }

    #[test]
    fn minor_collision_is_the_angle_between_input_and_actual_movement() {
        // vanilla `LocalPlayer.isHorizontalCollisionMinor`: the angle between the input direction
        // and the movement that actually resulted, minor below 8 degrees (0.13962634 rad).
        let imp = Impulse {
            strafe: 0.0,
            forward: INPUT_SCALE,
        };
        let mut s = PlayerState::at(DVec3::new(0.5, 0.0, 0.5));
        s.yaw = 0.0; // input direction +z
        for (deg, minor) in [(0.0f64, true), (7.0, true), (10.0, false), (90.0, false)] {
            let a = deg.to_radians();
            let actual = DVec3::new(a.sin(), 0.0, a.cos());
            assert_eq!(
                minor_horizontal_collision(&s, &imp, actual),
                minor,
                "{deg} degrees"
            );
        }
        // A near-zero actual movement is never minor (the player is flush against the wall).
        assert!(!minor_horizontal_collision(
            &s,
            &imp,
            DVec3::new(0.0, 0.0, 0.0)
        ));
    }

    #[test]
    fn sprinting_stops_in_a_corner_but_not_on_a_minor_collision() {
        // vanilla `LocalPlayer.shouldStopRunSprinting` / `isHorizontalCollisionMinor`.
        let shapes = TestShapes::default();
        let attrs = PlayerAttributes::default();
        // Straight into the wall: the actual movement is zeroed on the collided axis, so the
        // collision is not minor and sprinting stays off.
        let mut w = TestWorld::flat(0, STONE);
        w.fill((2, 0, -2), (2, 0, 2), STONE);
        let mut s = PlayerState::at(DVec3::new(0.5, 0.0, 0.5));
        let input = InputState {
            forward: true,
            sprint: true,
            yaw: -90.0,
            ..Default::default()
        };
        for _ in 0..10 {
            tick(&mut s, &input, &attrs, &w, &shapes, &shapes);
        }
        assert!(s.horizontal_collision);
        assert!(!s.sprinting, "pushing into a wall stops sprinting");

        // Grazing the wall at 5 degrees: the actual movement stays within 8 degrees of the input,
        // so the collision is minor and sprinting continues.
        let mut w = TestWorld::flat(0, STONE);
        w.fill((2, 0, -2), (2, 0, 60), STONE);
        let mut s = PlayerState::at(DVec3::new(1.5, 0.0, 0.5));
        let input = InputState {
            forward: true,
            sprint: true,
            yaw: -5.0,
            ..Default::default()
        };
        for _ in 0..40 {
            tick(&mut s, &input, &attrs, &w, &shapes, &shapes);
        }
        assert!(s.horizontal_collision, "grazing the wall at x={}", s.pos.x);
        assert!(s.sprinting, "a minor collision keeps sprinting");
    }

    #[test]
    fn levitation_blends_the_rise_towards_five_hundredths_per_level() {
        // vanilla `LivingEntity.travelInAir`: movementY += (0.05 * (amplifier + 1) - movementY)
        // * 0.2, then the 0.98 vertical drag. Levitation I (amplifier 0, level 1) targets 0.05.
        let w = TestWorld::flat(0, STONE);
        let shapes = TestShapes::default();
        let attrs = PlayerAttributes::default();
        let mut s = PlayerState::at(DVec3::new(0.5, 5.0, 0.5));
        s.levitation = Some(0);
        s.velocity = DVec3::ZERO;
        tick(&mut s, &InputState::default(), &attrs, &w, &shapes, &shapes);
        // (0.05 - 0.0) * 0.2 = 0.01, then * 0.98.
        assert!(
            (s.velocity.y - 0.01 * VERTICAL_DRAG as f64).abs() < 1e-15,
            "{}",
            s.velocity.y
        );
        // Levitation II (level 2) targets 0.1: a tick from rest blends 0.02, then * 0.98.
        let mut s = PlayerState::at(DVec3::new(0.5, 5.0, 0.5));
        s.levitation = Some(1);
        s.velocity = DVec3::ZERO;
        tick(&mut s, &InputState::default(), &attrs, &w, &shapes, &shapes);
        assert!(
            (s.velocity.y - 0.02 * VERTICAL_DRAG as f64).abs() < 1e-15,
            "{}",
            s.velocity.y
        );
    }

    #[test]
    fn levitation_replaces_gravity_and_resets_the_fall_distance() {
        // vanilla `LivingEntity.travelInAir`: the levitation branch is an `else if`, so no
        // gravity is applied; `LivingEntity.aiStep` resets the fall distance with the effect.
        let w = TestWorld::flat(0, STONE);
        let shapes = TestShapes::default();
        let attrs = PlayerAttributes::default();
        let mut s = PlayerState::at(DVec3::new(0.5, 5.0, 0.5));
        s.levitation = Some(0);
        s.velocity = DVec3::new(0.0, -0.5, 0.0);
        s.fall_distance = 10.0;
        tick(&mut s, &InputState::default(), &attrs, &w, &shapes, &shapes);
        // No gravity: the sink only blends towards the rise.
        assert!(
            (s.velocity.y - (-0.5 + (0.05 - -0.5) * 0.2) * VERTICAL_DRAG as f64).abs() < 1e-15,
            "{}",
            s.velocity.y
        );
        // The fall distance was reset, then the tick's descent (0.5) was added back.
        assert!((s.fall_distance - 0.5).abs() < 1e-12, "{}", s.fall_distance);
    }

    #[test]
    fn slow_falling_caps_the_fall_speed_growth_at_one_hundredth() {
        // vanilla `LivingEntity.getEffectiveGravity`: while falling (y <= 0) slow falling clamps
        // the gravity to min(gravity, 0.01); rising keeps the full gravity.
        let w = TestWorld::flat(0, STONE);
        let shapes = TestShapes::default();
        let attrs = PlayerAttributes::default();
        let mut s = PlayerState::at(DVec3::new(0.5, 5.0, 0.5));
        s.slow_falling = true;
        s.velocity = DVec3::new(0.0, -0.5, 0.0);
        tick(&mut s, &InputState::default(), &attrs, &w, &shapes, &shapes);
        assert!(
            (s.velocity.y - (-0.5 - 0.01) * VERTICAL_DRAG as f64).abs() < 1e-15,
            "{}",
            s.velocity.y
        );
        // Rising with the effect: the full 0.08 gravity applies.
        let mut s = PlayerState::at(DVec3::new(0.5, 5.0, 0.5));
        s.slow_falling = true;
        s.velocity = DVec3::new(0.0, 0.5, 0.0);
        tick(&mut s, &InputState::default(), &attrs, &w, &shapes, &shapes);
        assert!(
            (s.velocity.y - (0.5 - 0.08) * VERTICAL_DRAG as f64).abs() < 1e-15,
            "{}",
            s.velocity.y
        );
        // Without the effect the same fall loses the full 0.08.
        let mut s = PlayerState::at(DVec3::new(0.5, 5.0, 0.5));
        s.velocity = DVec3::new(0.0, -0.5, 0.0);
        tick(&mut s, &InputState::default(), &attrs, &w, &shapes, &shapes);
        assert!(
            (s.velocity.y - (-0.5 - 0.08) * VERTICAL_DRAG as f64).abs() < 1e-15,
            "{}",
            s.velocity.y
        );
    }

    #[test]
    fn slow_falling_resets_the_fall_distance() {
        // vanilla `LivingEntity.aiStep` resets the fall distance while slow falling, so a fall
        // never accumulates (and deals no damage).
        let w = TestWorld::flat(0, STONE);
        let shapes = TestShapes::default();
        let attrs = PlayerAttributes::default();
        let mut s = PlayerState::at(DVec3::new(0.5, 5.0, 0.5));
        s.slow_falling = true;
        s.velocity = DVec3::ZERO;
        s.fall_distance = 10.0;
        tick(&mut s, &InputState::default(), &attrs, &w, &shapes, &shapes);
        assert_eq!(s.fall_distance, 0.0);
    }

    #[test]
    fn jump_boost_adds_one_tenth_per_level_to_the_jump_power() {
        // vanilla `LivingEntity.getJumpBoostPower`: 0.1 per (amplifier + 1); `getJumpPower`
        // adds it to jump_strength x the block jump factor.
        let w = TestWorld::flat(0, STONE);
        let shapes = TestShapes::default();
        let attrs = PlayerAttributes::default();
        let mut s = PlayerState::at(DVec3::new(0.5, 0.0, 0.5));
        s.on_ground = true;
        s.jump_boost = Some(0);
        let input = InputState {
            jump: true,
            ..Default::default()
        };
        tick(&mut s, &input, &attrs, &w, &shapes, &shapes);
        assert!(
            (s.velocity.y
                - (attrs.jump_strength + f64::from(0.1_f32) - attrs.gravity)
                    * VERTICAL_DRAG as f64)
                .abs()
                < 1e-15,
            "{}",
            s.velocity.y
        );
    }
}
