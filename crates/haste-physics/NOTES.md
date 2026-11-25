# haste-physics notes (physics)

Vanilla 26.3 player movement and collision. Status of every rule: **documented** (a public page
states it), **derived** (follows from documented numbers; a test checks the derivation),
**assumption** (needed to write the code, not on a public page; the traces must settle it),
**verified** (read in the 26.3 client source in `cache/source/26.3/client` and re-implemented -
the method name is cited, no vanilla code is copied). Rows for fluids were settled that way in
October 2026 (lava shallow branch, gravity divisors, climb out, swim steering). Nothing here was
copied from Pomme's `physics/`/`player/`.

## Sources

- minecraft.wiki: "Entity" (motion table), "Transportation" (speeds, fall recurrence), "Walking"
  (speed per block table), "Sprinting", "Sneaking", "Jumping", "Swimming", "Water", "Flying",
  "Attribute" (defaults).
- mcpk.wiki (parkour community, public prose descriptions): "Horizontal Movement Formulas",
  "Vertical Movement Formulas", "Angles", "Tiers". Note for the lead: the "Angles" and "Vertical
  Movement Formulas" pages also quote 1.8-era MCP code snippets; we used only the stated numbers
  and formulas (65536 angles, 0.003 threshold, 0.2 sprint-jump boost, 0.02 air acceleration),
  but please confirm this source is acceptable.
- Block shapes, properties and fluids come through `haste_data::GameData` (the physics-side oracle
  loader was deleted). Oracle dumps in `cache/oracle/26.3/`: `block_shapes.json` (collision boxes, fluids),
  `block_properties.json` (friction / speed_factor / jump_factor), `entity_dimensions.json`
  (player 0.6 x 1.8, crouching 1.5, swimming 0.6; eye 1.62 / 1.27 / 0.4).
- Reuse check: Pomme `physics/`/`player/` excluded by the rules. Pumpkin's entity movement
  (`pumpkin-core/src/entity/{mod,living}.rs`) is organised after vanilla methods and cites yarn
  names ("Part of LivingEntity.tickMovement() in yarn"), so its provenance is unclear: not
  reused, not read beyond a grep. No third-party code in this crate, so no `docs/THIRD_PARTY.md` entry.
  The AABB sweep is our own (standard voxel clipping).

## Constants

| Constant | Value | Status / source |
|---|---|---|
| gravity | 0.08 b/t² | documented: wiki "Entity", "Attribute" (gravity 0.08) |
| vertical drag (air) | 0.98 as f32 | documented: wiki "Entity" ("0.98 (float)"); order Position, Acceleration, Drag |
| terminal velocity | 3.92 b/t (78.4 m/s) | documented: wiki "Entity"/"Transportation"; test `free_fall_*` |
| horizontal drag | slipperiness x 0.91 (f32 product), 0.91 in air | documented: wiki "Entity" |
| default slipperiness | 0.6; ice 0.98, blue ice 0.989, slime 0.8 | documented: wiki "Entity"; oracle block_properties.json |
| ground acceleration | movement_speed x (0.6 / slip)³ | mcpk.wiki; derived check: wiki "Walking" table speeds on ice 4.157, blue ice 4.376 reproduced to 1e-3; slime 3.040 m/s **not** reproduced — see slime stepOn row |
| air acceleration | 0.02 (x1.3 sprinting) | mcpk.wiki; derived check: sprint-jump average 7.127 m/s reproduced |
| input scale | 0.98 per key impulse | mcpk.wiki ("Default 0.98"); derived: walking 4.317 m/s = 20 x 0.098 / (1 - 0.546) |
| input normalisation | vector normalised only if longer than 1 | derived: wiki "Walking" (45° walking is faster), wiki "Sneaking" (1.3 m/s straight, 1.8 diagonal) |
| movement_speed | 0.1 | documented: wiki "Attribute" |
| sprint | x1.3 on movement speed | documented: wiki "Sprinting" (30 %); 5.612 m/s reproduced |
| sprint stop/start | stops on a non-minor horizontal collision; starting has no collision condition, but not while crawling on land | verified: `LocalPlayer.shouldStopRunSprinting` / `isHorizontalCollisionMinor` (angle between input direction and actual movement < 0.13962634 rad) / `canStartSprinting` (requires not `isMovingSlowly`: crouching or swimming pose out of water) |
| sneaking_speed | 0.3 | documented: wiki "Attribute"; 1.295 m/s ("about 1.3") reproduced |
| jump_strength | 0.42 | documented: wiki "Attribute", "Transportation" |
| jump height | 1.2522 | documented: wiki "Jumping"; reproduced (apex test) |
| flat jump airtime | 12 ticks | documented: mcpk.wiki "Tiers"; reproduced |
| sprint-jump boost | +0.2 along facing | mcpk.wiki; derived check: 7.127 m/s average reproduced to 0.005 |
| jump cooldown | 10 ticks while jump held | documented: wiki "Jumping" (half a second) |
| jump rise | y = max(jump power, current y) | verified: `LivingEntity.jumpFromGround` (jumping keeps a faster rise) |
| step_height | 0.6 | documented: wiki "Attribute" |
| block below offset | 0.500001 below the feet | verified: `Entity.getBlockPosBelowThatAffectsMyMovement` = `getOnPos(0.500001F)`; on the ground the supporting block's X/Z is used (fences, walls, gates keep the supporting pos) |
| velocity threshold | x and z zeroed together below horizontal length squared 9.0E-6, y below 0.003 on its own | verified: `LivingEntity.aiStep` (the player branch) |
| block speed factor | soul sand 0.4, honey 0.4; applied to horizontal velocity after the move | value: oracle; order derived from wiki "Walking": soul sand 2.508 m/s, soul sand under water 0.576, under lava 0.490 all reproduced |
| jump factor | honey 0.5 (oracle) | value: oracle. Not checked: wiki "Jumping" says honey jumps reach 3/16 block, our model gives more; **not determined**, see open questions |
| water drag | 0.8 (all axes) | derived: wiki "Walking" water 1.960 m/s = 20 x 0.0196 / 0.2 |
| water drag sprint-swimming | 0.9 horizontal | derived: wiki "Swimming" 3.918 m/s ≈ 20 x 0.0196 / 0.1 |
| lava drag, deep | 0.5 on every axis | derived: wiki "Walking" lava 0.784 m/s |
| lava drag, shallow (depth ≤ 0.4) | 0.5 horizontal, 0.8 vertical | verified: 26.3 `LivingEntity.travelInLava` shallow branch |
| fluid acceleration | 0.02 | assumption consistent with the water/lava derivations (one equation, two unknowns); matches `travelInLava` |
| water gravity | gravity / 16 | verified: `LivingEntity.getFluidFallingAdjustedMovement`; skipped while sprinting (any pose); the isFalling clamp (0.005 / -0.003) is unreachable at gravity 0.08 (the two y windows do not overlap) |
| lava gravity, deep | gravity / 4 | verified: `LivingEntity.travelInLava` (`baseGravity != 0`) |
| lava gravity, shallow | gravity / 16, then the same gravity / 4 | verified: same two vanilla methods applied in order |
| swim up (jump in fluid) | +0.04 / tick | verified: `LivingEntity.jumpInLiquid`; trigger conditions follow `LivingEntity.aiStep` jump section: in a fluid the player jumps normally (`jumpFromGround`) only while on ground with the fluid no deeper than the jump threshold, otherwise holding jump swims up |
| swim down (sneak in water) | -0.04 / tick | verified: `LivingEntity.goDownInWater`, called from `LocalPlayer.aiStep` when in water and sneaking |
| fluid jump threshold | 0.4 (0 when the eye height is below 0.4) | verified: `Entity.getFluidJumpThreshold`; decides lava shallow vs deep |
| fluid depth | max fluid surface above the feet over the cells the shrunk box touches | verified: `EntityFluidInteraction.update` -> `Entity.getFluidHeight`; lava reads it after the move (vanilla refreshes the trackers inside `move`) |
| climb out of fluid | y = 0.3, probe 0.6 above | verified: `LivingEntity.jumpOutOfFluid` - horizontal collision in a fluid plus a block- and liquid-free box lifted by velocity.y + 0.6 |
| swim steering | 0.085 diving, else 0.06; dive below look y -0.2 | verified: `Player.travel` - sprint-swimming pulls y toward `-sin(pitch)`; skipped when looking up, not jumping and the block 0.9 above the feet is air |
| fluid height | level 0: 8/9, level L: (8-L)/9, falling: 8/9, 1.0 with same fluid above | documented: wiki "Water"; falling level 8 = 8/9 checked on the vanilla server (below) |
| flying acceleration | flying_speed 0.05 (x2 sprinting) | derived: wiki "Flying" 10.92 m/s (we get 10.89) and 21.6 (we get 21.78) |
| flying vertical | ±3 x flying_speed per tick, vertical drag 0.6 | **UNVERIFIED assumption** |
| sine table | 65536 f32 entries, index = (int)(rad x 10430.378) & 65535, cos = +16384 | mcpk.wiki "Angles"; assumption that 26.3 still does this (`trig::USE_TABLE`) |
| deg -> rad | (deg x π) / 180 in f32, also for the sprint-jump boost | verified: `LivingEntity.jumpFromGround` uses the same `(float)(PI / 180.0)` conversion as the movement code (the separate 0.017453292 jump constant is gone) |
| collision epsilon | 1e-7 | assumption (typical f64 voxel sweep value) |
| sneak edge back-off step | 0.05 | vanilla `Player.maybeBackOffFromEdge` (also its loop structure and the thin-slab `canFallAtLeast` probe, shrunk 1.0E-7) |
| pose-fit / edge probe shrink | 1.0E-7 | vanilla `Player.canPlayerFitWithinBlocksAndEntitiesWhen` / `Player.canFallAtLeast` |
| box half-width | 0.6f32 / 2 widened to f64 | entity dimensions are floats; consequence: against a wall the player stops at x.69999998808, not x.7 (test `walking_into_a_full_block_stops`) |
| cobweb stuck multiplier | (0.25, 0.05, 0.25) | `Entity.makeStuckInBlock` via `WebBlock.entityInside`; set end of tick, consumed (x motion, velocity zeroed) at next `Entity.move`; resets fall distance |
| powder snow stuck multiplier | (0.9, 1.5, 0.9) | `PowderSnowBlock.entityInside`; only when the feet block is also powder snow |
| sweet berry bush stuck multiplier | (0.8, 0.75, 0.8) | `SweetBerryBushBlock.entityInside`; living entities only (not foxes/bees) |
| slime bounce restitution | 1.0 (`Block.getBounceRestitution`) | `Entity.restituteMovementAfterCollisions`: bounce only when falling faster than gravity (`-vy > 0.08`) and not sneaking; formula in `block_effects::vertical_collision_velocity` |
| slime stepOn scale | 0.4 + \|vy\| x 0.2 | `SlimeBlock.stepOn`: on ground, not sneaking, \|vy\| < 0.1; applied to horizontal velocity after drag at end of tick. Steady walk speed on slime: 1.1857 m/s (not the wiki's 3.040 which ignores stepOn) |
| honey slide | see `block_effects` constants | `HoneyBlock.isSlidingDown`/`doSlideMovement`: `getOldDeltaY(vy) = vy/0.98 + 0.08`; slide when < -0.08; horizontal slowdown when < -0.13; vertical throttled to `getNewDeltaY(-0.05) = -0.1274`; side overlap > 0.4375 + width/2 |
| honey speed/jump factor | 0.4 / 0.5 | oracle block_properties.json (already in table above) |
| water current push | 0.014 x average flow | `Entity.updateFluidHeightAndDoFluidPushing` -> `EntityFluidInteraction`; nearly stationary (\|v\| < 0.003) gets normalized 0.0045; flow scaled by fluid height < 0.4; min accumulated length² 1e-5 |
| glide drag | 0.99 horizontal, 0.98 vertical per tick | vanilla 26.3 `LivingEntity.updateFallFlyingMovement` (read in the decompiled client: the drag is the last thing in the function, on all three components) |
| glide lift | cos²(lean), lean = pitch in radians | same vanilla function |
| glide gravity term | gravity x (-1 + 0.75 x lift) | same |
| glide descent exchange | velocity.y x -0.1 x lift, routed along the look direction | same |
| glide pull-up (pitch < 0) | horizontal speed x -sin(lean) x 0.04 on y, x 3.2 | same |
| glide steering | 0.1 of the way to the look direction, keeping the length | same |
| slow falling gravity | min(gravity, 0.01) while `deltaMovement.y <= 0` | vanilla `LivingEntity.getEffectiveGravity` |
| gliding fall distance | clamped to 1.0 per tick | vanilla `Entity.checkFallDistanceAccumulation`, called from `updateFallFlying` |
| fall-flying box | 0.6 x 0.6 | vanilla's fall_flying entity dimensions (eye height 0.4) |

## Elytra flight (`src/elytra.rs`)

Vanilla 26.3, read in the decompiled client (never copied): `LivingEntity.updateFallFlyingMovement`,
`LivingEntity.travelFallFlying`, `LivingEntity.getEffectiveGravity`, `LivingEntity.canGlide` /
`canGlideUsing`, `LivingEntity.updateFallFlying`, `Player.tryToStartFallFlying` /
`Player.startFallFlying` / `LivingEntity.stopFallFlying` (shared flag 7) and
`LocalPlayer.aiStep`'s jump condition (`jump && !wasJumping && !onClimbable`).

- `glide_velocity(velocity, yaw, pitch, gravity) -> velocity` is the pure velocity update. The
  look vector is vanilla `Entity.calculateViewVector` (f32 table sine/cosine, two f32 products per
  component, then widened); the lift uses `cos²(lean)` with the double `cos` of the f32 angle,
  while the pitch-up branch uses the table `sin`. Both are spelled out in the code comments.
- `elytra::travel(state, attrs, world, shapes, slow_falling)` is `travelFallFlying`: the clamp of
  the fall distance, the velocity update, the move/collide step, and gliding ends when the player
  lands. It ignores the movement input entirely (gliding is steered with the look angles only), so
  the sneak edge back-off of the shared move step is skipped while it runs.
- Collision damage (`handleFallFlyingCollisions`) is server-side and not modelled; so is the
  elytra's durability loss every 10 ticks (`updateFallFlying`). Both noted in the module docs.
- Not modelled: firework boosts (see below), several glider equipment slots (`EquipmentSlot.VALUES`
  is scanned in vanilla; we take the caller's `has_glider` for the chest), the passenger and
  levitation conditions of `canGlide` (no `PlayerState` field yet), and the climbable case
  (`travelFallFlying` falls back to air travel and stops gliding on a ladder).
- The glide never turns into a fall: the fall distance is clamped to 1.0 per tick (test
  `landing_ends_the_glide`), and landing ends the flag.

### The call the movement tick must make

In `movement::tick`'s travel dispatch (`src/movement.rs:93`), after the flying branch and before
the fluid branches (vanilla's `LivingEntity.travel` checks the fluid state first, then the
fall-flying flag):

    } else if state.fall_flying && !climbable {
        elytra::travel(state, attrs, world, shapes, slow_falling);
    }

- `climbable` is the caller's check on the block state at the player's position
  (`LivingEntity.onClimbable`): on a ladder vanilla takes the air branch and stops gliding. The
  block view needed for it is already in scope. `climbable` may be `false` for now if the tag is
  not available yet (then a ladder keeps gliding, which is wrong but harmless).
- `slow_falling` is the slow falling effect: `PlayerAttributes` has no effect state, so it is a
  parameter. Without effects it is `false`.
- The gliding branch must come **before** `state.in_water` / `state.in_lava`: vanilla takes the
  fluid branch first, and `should_stop_fall_flying` also reports the fluids for the client's flag
  bookkeeping.
- The jump-to-start wiring (client side, later): on a freshly pressed jump key, with
  `can_start_fall_flying(has_glider)` true (plus the caller's not-a-passenger, no-levitation and
  not-on-a-climbable checks), call `state.start_fall_flying()` and send the player command
  `START_FALL_FLYING` (packet id 2 in 26.3, carried by the connection, not by this crate).
- The stop wiring: `state.stop_fall_flying()` when `should_stop_fall_flying(has_glider)` is true
  (the server also clears the flag through entity metadata; the client follows it).

### Interface requests for the lead

- `PlayerState.aabb` now returns the 0.6 x 0.6 box while `fall_flying` is set (`src/lib.rs`). The
  pose enum is unchanged, so `haste_core::entity::Pose` still has no fall-flying variant and the
  client's `player_pose` keeps compiling. If a fall-flying pose is wanted (eye height 0.4 while
  gliding, the model's elytra animation), add `Pose::FallFlying` in `attributes.rs` and map it in
  `crates/haste-client/src/game.rs:130`; the physics side only needs the box.
- The eye height while gliding is still the standing 1.62 (the pose is `Standing`), so the
  crosshair ray and the camera eye sit 1.22 blocks too high during a glide. Same fix as above.

### Firework boost (not implemented, entity side)

Vanilla `FireworkRocketEntity.tick`: while the rocket is attached to a living entity that is fall
flying, each tick the entity's velocity gets

    v += look.x * 0.1 + (look.x * 1.5 - v.x) * 0.5   (same for y and z)

i.e. a third of the way from the current velocity towards 1.5x the look vector, plus a 0.1 nudge
along the look. The client needs: the firework entity tracked (spawned by the server, its
attached-to entity id in its entity data, `FireworkRocketEntity.isAttachedToEntity`), and a pure
`firework_boost(velocity, yaw, pitch) -> velocity` on `elytra` (the same body as the formula above,
using `look_angle`) called once per tick from the movement tick while such a rocket is attached to
the local player and `state.fall_flying` is set. The rocket's own position follows the player's
firework-holding hand (`getHandHoldingItemAngle`), which is a render concern.

### Sine table details

`trig::sin_d` / `trig::cos_d` index the table the way vanilla's `Mth.sin(double)` /
`Mth.cos(double)` do: the angle is multiplied by the double `10430.378350470453` and cast to long
*with the 0.98-quarter-turn offset added first for the cosine*, then masked with 65535. The f32
`units_per_radian` used by the movement direction multiplies in f32 and can land one entry away;
the elytra uses the double form. How much vanilla's own `Mth.sin` for angles differs from the
movement path's is **not determined** (assumption, as before).

Assumption also that Rust's `f64::cos` matches Java's `Math.cos` bit for bit on the f32 angle; a
difference of an ulp would move the lift term by ~1e-16, far below the table's own 5e-5 error at
60/-30 degrees (which is why the tests' hand-computed multi-tick values carry 1e-3 tolerances).

## Tick order (as implemented)

1. Rotation from input. 2. Zero x/z together below horizontal length squared 9.0E-6, y below
   0.003 on its own (vanilla `LivingEntity.aiStep`). 3. Fluid contact (shrunk box vs fluid
   surface) and water currents (0.014 push) (vanilla `Entity.baseTick`:
   `updateFluidHeightAndDoFluidPushing`), then the input-driven flags (vanilla
   `LocalPlayer.aiStep`): `sneaking`, `crouching` (from the pose box the last tick ended with),
   sprint start (`canStartSprinting`) and stop (`shouldStopRunSprinting`), then the
   sprint-swimming state. 4. Jump cooldown; jump (on ground, or in a fluid no deeper than the
   jump threshold while on the ground) or swim up (otherwise in a fluid). 5. Impulse: keys -> ±1,
   x sneaking speed when crouching/crawling, x 0.98. 6. Travel: normal / water / lava / flying /
   **gliding** (`elytra::travel` when `state.fall_flying` is set - see below), with the pose box
   from the END of the last tick: accelerate (rotated by yaw, sine table) -> move (sneak edge
   back-off, sweep with step-up; stuck-in-block multiplier consumed at move start; slime bounce
   on vertical collision) -> zero velocity on collided axes, on_ground = vertical collision while
   moving down, fall distance -> block speed factor -> gravity, vertical drag -> horizontal drag.
7. End of tick: the `minorHorizontalCollision` flag (vanilla `Entity.move`), then
   `Entity.applyEffectsFromBlocks` (slime stepOn, then the entityInside scan: honey slide,
   stuck-in-block multipliers for cobweb / powder snow / sweet berry bush), then
   `Player.updatePlayerPose` (vanilla `Player.tick`: desired pose from shift/swimming, crouching
   fallback, swimming fallback; left alone when even swimming does not fit). The camera eye
   height follows the pose one tick later still (vanilla `Camera.tick` runs before the player's
   tick; see haste-client).
In water the tick starts with the sneak sink, then the sprint-swim steering (`Player.travel`),
and ends with `jump_out_of_fluid`; in lava the shallow/deep branch (depth after the move) picks
the drags, then gravity/4, then the same `jump_out_of_fluid`.

## Approximate / not modelled

- Step-up tries every candidate height from the collider Y coordinates below the max step,
  ascending, keeping the first that covers more horizontal distance than the plain sweep
  (verified: `Entity.collide` / `Entity.collectCandidateStepUpHeights`).
- Supporting block for friction: `getOnPos(0.500001F)`; on the ground the supporting block's
  X/Z with `floor(y - 0.500001)` (verified: `Entity.getBlockPosBelowThatAffectsMyMovement`).
  The fence/wall/fence-gate refinement of `getOnPos` (those blocks keep the supporting pos
  itself) is not modelled: `BlockPhysics` carries no fence/wall/gate identity, and for a fence
  the cell comes out the same either way while the player stands on its 1.5 tall top.
- Jump factor block: the block at the feet if non-default, else the supporting block below
  (verified: `Entity.getBlockJumpFactor`). The speed factor block in the fluids path is still
  the simple `floor(pos - 0.500001)` without the supporting-block refinement (see the comment
  on `apply_speed_factor`).
- Sprint stops on a non-minor horizontal collision (input direction vs actual movement within
  8 degrees, `LocalPlayer.isHorizontalCollisionMinor`); starting has no collision condition
  but requires not moving slowly (`isMovingSlowly`: crouching or the swimming pose out of water,
  unless under water). The stop is evaluated after the start so a persistent collision keeps
  the sprint off. Hunger/blindness: caller clears `input.sprint`.
- Sprint-swimming is established on the same tick the sprint starts in water (the flag is
  computed after the sprint logic); vanilla's `updateSwimming` runs in `baseTick` before
  `aiStep`, so it establishes a tick later.
- Sneak edge protection near (not on) ground follows the wiki's "up to 0.6 blocks above ground"
  via `fall_distance < step_height` (vanilla `Player.isAboveGround`); the probe is vanilla
  `Player.canFallAtLeast`: only the slab below the feet of the moved box (X/Z shrunk 1e-7, from
  `minY - step - 1e-7` to `minY`), not the whole body shifted down.
- Swimming: pitch-steered vertical swimming, surface bobbing, the climb-out-of-water boost,
  bubble columns, depth strider, dolphin's grace: not implemented.
- Not implemented: ladders/vines (climbing), levitation/slow falling/jump boost/speed effects,
  world border, entity pushing, knockback. `PlayerAttributes` carries no effect state yet
  (the slow-falling clamp of the elytra takes the effect as a parameter).
- Elytra: implemented in `elytra.rs` (see its section); not modelled there are the firework boost
  and the server-side collision damage and durability loss.
- Landing ends creative flight except for spectators (verified: `LocalPlayer.aiStep`:
  `onGround() && abilities.flying && !gameMode.isSpectator()`). Double-tap to fly and
  double-tap sprint are input-layer concerns (haste-client).
- `fall_distance` is f32 (interface); vanilla's type not determined.

## Open questions for the traces

- Honey jump height: oracle jump_factor 0.5 vs wiki 3/16 block jump.
- Sprint-jump gap: we cross a 3-block gap (jump onto the 4th block); a 4-air-block gap fails in
  our model without 45° strafing. The wiki's "jump across up to four blocks" does not settle
  which is meant.
- Flying vertical constants (UNVERIFIED row above); the water/lava vertical constants were settled
  against the 26.3 client source, see the constants table.
- Sine table and the two degree-to-radian constants.
- Trace input alignment (does `ticks[i].input` produce `ticks[i].pos`?): `replay::InputAlignment`.

## Falling-water height (settled 2026-10-12 against the vanilla server)

`crates/haste-client/walks/falling-water.txt`, `/tick freeze` so the water stays put, player
standing on snow under one water cell, the server's air supply (entity metadata 1) as the oracle
for "eyes in water": eyes 0.995 above the cell floor in source water (level 0) and in falling
water (level 8, no water above): air stays 300 (not in water); eyes 0.87 above it in falling
water: air drops 59 in 60 ticks. So falling water without water above is lower than 0.995, i.e.
8/9 as on the wiki, not 1.0. Also shows the eye test has no extra offset below 8/9. Levels 9..15
not tested. haste-data must report 8/9 for level >= 8 (it reported 1.0; the full-cell case with
water above is handled here in `fluids.rs`).

## Interface notes (settled by the lead)

- `tick` takes `blocks: &dyn haste_core::view::BlockPhysics` (friction, speed_factor,
  jump_factor, fluid, light_emission, is_air, is_opaque_full); haste-data implements it for
  `GameData`. `testing::TestShapes` implements it for unit tests.
- `collide` is the pure sweep; step-up is `collide_with_step(aabb, movement, step_height,
  on_ground, world, shapes)` (in INTERFACES.md).
- Extra `PlayerState`/`PlayerAttributes` fields accepted.

## Performance

`cargo bench -p haste-physics` (std timing, best of 7 batches, shared 16-core machine, release):
collide walking ~450 ns, terminal-velocity fall ~380 ns, collide_with_step ~480 ns (into a wall
~580 ns), full tick ~1.2 µs. Boxes are gathered once per call into a thread-local reusable buffer
(no allocation after the first call); this was ~2x faster than scanning the world per axis
(4x when blocked by a wall). Above 96 boxes it falls back to the per-axis scan (same results,
unit test `list_and_world_paths_agree`).

## Trace replay (`src/replay.rs`, `tests/replay.rs`)

`replay(trace, attrs, world, shapes, blocks, tolerance, alignment) -> ReplayReport`: initial state
from the first tick (pos, velocity, on_ground, pose, look angles), then `tick` per recorded input;
reports the first tick whose position error exceeds the tolerance (default 1e-6), the maximum
position error, the maximum velocity error and on_ground/pose mismatch counts.
`cargo test -p haste-physics --test replay -- --ignored --nocapture` replays every
`cache/oracle/26.3/traces/*.json`; with no traces it prints that the harness is ready and passes.
Assumptions about the traces, all to confirm with oracles: the world is a default superflat
(bedrock, dirt, dirt, grass; surface y = -60); `ticks[i].input` produced `ticks[i].pos`
(`InputAlignment::SameTick`; on divergence the test also reports `PreviousTick`); look angles come
from `input.yaw`/`input.pitch` or a per-tick `yaw`/`pitch` if present, else the `player` header
(FORMATS.md v1 has no per-tick look angles, which a turning trace needs); `strafe > 0` is left.
The parser is our own (lenient serde structs) so this crate does not depend on haste-testkit.

## Interaction (raycast.rs, mining.rs)

- Crosshair: voxel walk over outline boxes; look vector from the movement sine table (assumption:
  the game uses the same table for the view vector). A ray starting inside a box hits at its start.
- Mining speed: minecraft.wiki "Breaking" formula; tool rules from the data generator's item
  component reports (`minecraft:tool`, first matching rule per field), tags from the server,
  hardness and correct-tool flag from the recorder dump via `GameData`. Checked against the
  wiki's break-time table; on the vanilla server: hand and diamond-pickaxe stone breaks, an
  Efficiency V + Haste II instant break (the server sends Efficiency as `mining_efficiency`).
- Progress is summed in f32 per tick, so stone by hand finishes on tick 151, not the wiki's 150
  (assumption that the game sums floats the same way; the server accepts either, its check is
  70 % of the progress).
- Not modelled: item component patches on held stacks (protocol does not decode components yet),
  Aqua Affinity other than through `submerged_mining_speed`, adventure-mode `can_break`.
