//! The player tick: input -> rotation, flags and pose -> jump -> acceleration -> collision ->
//! block speed factor -> gravity and drag. Constants are named here and listed in `NOTES.md` with
//! their source (wiki page, derivation from documented speeds, or "assumption pending traces").
use haste_core::input::InputState;
use haste_core::view::{BlockView, CollisionShapes};
use haste_core::{Aabb, BlockPos, BlockStateId, DVec3};

use crate::collision::{collide_with_step, no_collision, supporting_block};
use crate::{fluids, flying, trig, BlockPhysics, PlayerAttributes, PlayerState, Pose, TickResult};

/// Velocity components with a smaller magnitude are zeroed at the start of the tick
/// (mcpk.wiki "Horizontal/Vertical Movement Formulas": 0.003 in 1.9+).
pub const VELOCITY_THRESHOLD: f64 = 0.003;
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
/// Ticks between jumps while jump stays held (wiki "Jumping": half a second delay).
pub const JUMP_COOLDOWN_TICKS: u8 = 10;
/// Sneak edge back-off step (assumption, not on a public page; to be settled by traces).
pub const EDGE_BACKOFF_STEP: f64 = 0.05;
/// Offset below the feet used to find the block whose friction/jump factor applies (assumption).
pub const BLOCK_BELOW_OFFSET: f64 = 0.5;

/// Movement impulse for this tick: (strafe, forward), already scaled.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Impulse {
    pub strafe: f32,
    pub forward: f32,
}

/// One 50 ms player tick. `blocks` stands in for `GameData` until haste-data provides it (see
/// NOTES.md, interface requests).
pub fn tick(
    state: &mut PlayerState,
    input: &InputState,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    blocks: &dyn BlockPhysics,
) -> TickResult {
    let old_pos = state.pos;
    let old_ground = state.on_ground;
    let rotated = state.yaw != input.yaw || state.pitch != input.pitch;

    // Rotation comes straight from input.
    state.yaw = input.yaw;
    state.pitch = input.pitch.clamp(-90.0, 90.0);

    // Negligible velocities are cancelled first.
    let v = &mut state.velocity;
    for c in [&mut v.x, &mut v.y, &mut v.z] {
        if c.abs() < VELOCITY_THRESHOLD {
            *c = 0.0;
        }
    }

    // Fluid contact, sneak/sprint flags, pose.
    let eyes_in_water = fluids::update_fluid_state(state, attrs, world, blocks);
    update_flags_and_pose(state, input, attrs, eyes_in_water, world, shapes);

    if state.jump_cooldown > 0 {
        state.jump_cooldown -= 1;
    }
    if state.flying {
        state.jump_cooldown = 0;
    } else if input.jump {
        if state.in_water || state.in_lava {
            fluids::swim_up(state);
        } else if state.on_ground && state.jump_cooldown == 0 {
            jump(state, attrs, world, blocks);
            state.jump_cooldown = JUMP_COOLDOWN_TICKS;
        }
    } else {
        state.jump_cooldown = 0;
    }

    let impulse = impulse(state, input, attrs);

    if state.flying {
        flying::travel(state, input, impulse, attrs, world, shapes, blocks);
    } else if state.in_water {
        fluids::travel_water(state, input, impulse, attrs, world, shapes, blocks);
    } else if state.in_lava {
        fluids::travel_lava(state, input, impulse, attrs, world, shapes, blocks);
    } else {
        travel_normal(state, impulse, attrs, world, shapes, blocks);
    }

    TickResult {
        moved: state.pos != old_pos,
        rotated,
        on_ground_changed: state.on_ground != old_ground,
        collided_horizontally: state.horizontal_collision,
    }
}

fn update_flags_and_pose(
    state: &mut PlayerState,
    input: &InputState,
    attrs: &PlayerAttributes,
    eyes_in_water: bool,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
) {
    state.sneaking = input.sneak;
    let forward = input.forward && !input.back;
    // Sprint: needs forward; ends on horizontal collision (wiki "Sprinting": collision at more than
    // 8 degrees ends it; the angle test is not modelled) or when forward is released; releasing
    // the sprint key alone does not end it. Hunger and blindness are the caller's business.
    if state.sprinting {
        if !forward || state.horizontal_collision {
            state.sprinting = false;
        }
    } else if input.sprint
        && forward
        && state.pose != Pose::Crouching
        && !state.horizontal_collision
    {
        // Not restarted while still pushing into a wall (assumption; avoids on/off flapping).
        state.sprinting = true;
    }
    state.swimming =
        state.sprinting && state.in_water && (eyes_in_water || state.swimming) && !state.flying;

    let desired = if state.swimming {
        Pose::Swimming
    } else if state.sneaking && !state.flying {
        Pose::Crouching
    } else {
        Pose::Standing
    };
    // Keep a smaller pose if the desired one does not fit (e.g. crouching under a 1.5 high ceiling).
    let fits = |p: Pose| {
        no_collision(
            &Aabb::entity(state.pos, attrs.box_width(), attrs.height_for(p)),
            world,
            shapes,
        )
    };
    state.pose = if fits(desired) {
        desired
    } else if desired == Pose::Standing && fits(Pose::Crouching) {
        Pose::Crouching
    } else if desired != Pose::Swimming && !fits(Pose::Crouching) {
        Pose::Swimming
    } else {
        state.pose
    };
}

fn impulse(state: &PlayerState, input: &InputState, attrs: &PlayerAttributes) -> Impulse {
    let mut forward = (input.forward as i32 - input.back as i32) as f32;
    let mut strafe = (input.left as i32 - input.right as i32) as f32;
    // Slowed while crouching or crawling (swimming pose outside water), not normalised:
    // wiki "Sneaking": 1.3 m/s straight, 1.8 m/s diagonal.
    let slow = !state.flying
        && (state.pose == Pose::Crouching || (state.pose == Pose::Swimming && !state.in_water));
    if slow {
        forward *= attrs.sneaking_speed as f32;
        strafe *= attrs.sneaking_speed as f32;
    }
    Impulse {
        strafe: strafe * INPUT_SCALE,
        forward: forward * INPUT_SCALE,
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

/// Block whose friction/jump factor applies: the block half a block below the feet, or, when that
/// has no collision (standing on an edge), the closest supporting block.
fn ground_block(
    state: &PlayerState,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
) -> BlockStateId {
    let below = world.block_state(block_below(state));
    if !shapes.collision_boxes(below).is_empty() {
        return below;
    }
    match supporting_block(&state.aabb(attrs), world, shapes) {
        Some(p) => world.block_state(p),
        None => below,
    }
}

/// Speed/jump factor: the block at the feet if it has a non-default factor (soul sand and honey
/// are shorter than a block, so the player stands inside their cell), else the block below.
fn block_factor(
    state: &PlayerState,
    world: &dyn BlockView,
    f: impl Fn(BlockStateId) -> f32,
) -> f32 {
    let at = f(world.block_state(BlockPos::from_dvec3(state.pos)));
    if at != 1.0 {
        at
    } else {
        f(world.block_state(block_below(state)))
    }
}

fn jump(
    state: &mut PlayerState,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    blocks: &dyn BlockPhysics,
) {
    let factor = block_factor(state, world, |s| blocks.jump_factor(s));
    state.velocity.y = attrs.jump_strength * factor as f64;
    if state.sprinting {
        let r = trig::deg_to_rad_jump(state.yaw);
        state.velocity.x -= (trig::sin(r) * SPRINT_JUMP_BOOST) as f64;
        state.velocity.z += (trig::cos(r) * SPRINT_JUMP_BOOST) as f64;
    }
}

/// Walking/falling in air: accelerate, move, block speed factor, gravity + vertical drag,
/// horizontal drag (slipperiness x 0.91 on ground, 0.91 in air).
fn travel_normal(
    state: &mut PlayerState,
    imp: Impulse,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    blocks: &dyn BlockPhysics,
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
    let motion = state.velocity;
    move_entity(state, attrs, motion, world, shapes);
    apply_speed_factor(state, world, blocks);
    state.velocity.y -= attrs.gravity;
    state.velocity.y *= VERTICAL_DRAG as f64;
    let drag = (slip * HORIZONTAL_DRAG) as f64;
    state.velocity.x *= drag;
    state.velocity.z *= drag;
}

/// Block speed factor (soul sand, honey) on horizontal velocity after the move. The order is
/// derived from the wiki "Walking" table: soul sand 2.508 m/s = 20 x 0.098 / (1 - 0.4 x 0.546),
/// i.e. the factor multiplies the velocity that carries into the next tick, not this tick's move.
pub(crate) fn apply_speed_factor(
    state: &mut PlayerState,
    world: &dyn BlockView,
    blocks: &dyn BlockPhysics,
) {
    let f = block_factor(state, world, |s| blocks.speed_factor(s));
    if f != 1.0 {
        state.velocity.x *= f as f64;
        state.velocity.z *= f as f64;
    }
}

/// Moves by `motion` with sneak edge back-off and step-up, updates position, collision flags,
/// on-ground, fall distance and zeroes velocity on collided axes. Returns the actual movement.
pub(crate) fn move_entity(
    state: &mut PlayerState,
    attrs: &PlayerAttributes,
    motion: DVec3,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
) -> DVec3 {
    let aabb = state.aabb(attrs);
    let mut motion = motion;
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
        state.velocity.y = 0.0;
    }

    if state.on_ground || state.flying || state.in_water {
        state.fall_distance = 0.0;
    } else if moved.y < 0.0 {
        state.fall_distance -= moved.y as f32;
    }
    moved
}

/// On ground, or less than a step above ground (wiki "Sneaking": edge protection works "up to 0.6
/// blocks above the ground" since 1.16.2).
fn above_ground(
    state: &PlayerState,
    aabb: &Aabb,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
) -> bool {
    state.on_ground
        || ((state.fall_distance as f64) < attrs.step_height
            && !no_collision(
                &aabb.offset(DVec3::new(
                    0.0,
                    state.fall_distance as f64 - attrs.step_height,
                    0.0,
                )),
                world,
                shapes,
            ))
}

fn toward_zero(v: f64) -> f64 {
    if v.abs() <= EDGE_BACKOFF_STEP {
        0.0
    } else {
        v - EDGE_BACKOFF_STEP * v.signum()
    }
}

/// Shrinks horizontal motion in `EDGE_BACKOFF_STEP` steps until the box, moved and lowered by the
/// step height, still has something under it: X alone, Z alone, then both together.
fn back_off_from_edge(
    aabb: &Aabb,
    motion: DVec3,
    step: f64,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
) -> DVec3 {
    let (mut x, mut z) = (motion.x, motion.z);
    let free =
        |dx: f64, dz: f64| no_collision(&aabb.offset(DVec3::new(dx, -step, dz)), world, shapes);
    while x != 0.0 && free(x, 0.0) {
        x = toward_zero(x);
    }
    while z != 0.0 && free(0.0, z) {
        z = toward_zero(z);
    }
    while x != 0.0 && z != 0.0 && free(x, z) {
        x = toward_zero(x);
        z = toward_zero(z);
    }
    DVec3::new(x, motion.y, z)
}
