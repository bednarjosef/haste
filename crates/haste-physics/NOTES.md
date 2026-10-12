# haste-physics notes (WS5)

Vanilla 26.3 player movement and collision. Status of every rule: **documented** (a public page
states it), **derived** (follows from documented numbers; a test checks the derivation),
**assumption** (needed to write the code, not on a public page; the traces must settle it).
Nothing here was taken from decompiled Minecraft code or from Pomme's `physics/`/`player/`.

## Sources

- minecraft.wiki: "Entity" (motion table), "Transportation" (speeds, fall recurrence), "Walking"
  (speed per block table), "Sprinting", "Sneaking", "Jumping", "Swimming", "Water", "Flying",
  "Attribute" (defaults).
- mcpk.wiki (parkour community, public prose descriptions): "Horizontal Movement Formulas",
  "Vertical Movement Formulas", "Angles", "Tiers". Note for the lead: the "Angles" and "Vertical
  Movement Formulas" pages also quote 1.8-era MCP code snippets; we used only the stated numbers
  and formulas (65536 angles, 0.003 threshold, 0.2 sprint-jump boost, 0.02 air acceleration),
  but please confirm this source is acceptable.
- WS8 oracle dumps in `cache/oracle/26.3/`: `block_shapes.json` (collision boxes, fluids),
  `block_properties.json` (friction / speed_factor / jump_factor), `entity_dimensions.json`
  (player 0.6 x 1.8, crouching 1.5, swimming 0.6; eye 1.62 / 1.27 / 0.4).
- Reuse check: Pomme `physics/`/`player/` excluded by the rules. Pumpkin's entity movement
  (`pumpkin-core/src/entity/{mod,living}.rs`) is organised after vanilla methods and cites yarn
  names ("Part of LivingEntity.tickMovement() in yarn"), so its provenance is unclear: not
  reused, not read beyond a grep. No third-party code in this crate, so no `THIRD_PARTY.md` entry.
  The AABB sweep is our own (standard voxel clipping).

## Constants

| Constant | Value | Status / source |
|---|---|---|
| gravity | 0.08 b/t² | documented: wiki "Entity", "Attribute" (gravity 0.08) |
| vertical drag (air) | 0.98 as f32 | documented: wiki "Entity" ("0.98 (float)"); order Position, Acceleration, Drag |
| terminal velocity | 3.92 b/t (78.4 m/s) | documented: wiki "Entity"/"Transportation"; test `free_fall_*` |
| horizontal drag | slipperiness x 0.91 (f32 product), 0.91 in air | documented: wiki "Entity" |
| default slipperiness | 0.6; ice 0.98, blue ice 0.989, slime 0.8 | documented: wiki "Entity"; oracle block_properties.json |
| ground acceleration | movement_speed x (0.6 / slip)³ | mcpk.wiki; derived check: wiki "Walking" table speeds on ice 4.157, blue ice 4.376, slime 3.040 m/s all reproduced to 1e-3 |
| air acceleration | 0.02 (x1.3 sprinting) | mcpk.wiki; derived check: sprint-jump average 7.127 m/s reproduced |
| input scale | 0.98 per key impulse | mcpk.wiki ("Default 0.98"); derived: walking 4.317 m/s = 20 x 0.098 / (1 - 0.546) |
| input normalisation | vector normalised only if longer than 1 | derived: wiki "Walking" (45° walking is faster), wiki "Sneaking" (1.3 m/s straight, 1.8 diagonal) |
| movement_speed | 0.1 | documented: wiki "Attribute" |
| sprint | x1.3 on movement speed | documented: wiki "Sprinting" (30 %); 5.612 m/s reproduced |
| sneaking_speed | 0.3 | documented: wiki "Attribute"; 1.295 m/s ("about 1.3") reproduced |
| jump_strength | 0.42 | documented: wiki "Attribute", "Transportation" |
| jump height | 1.2522 | documented: wiki "Jumping"; reproduced (apex test) |
| flat jump airtime | 12 ticks | documented: mcpk.wiki "Tiers"; reproduced |
| sprint-jump boost | +0.2 along facing | mcpk.wiki; derived check: 7.127 m/s average reproduced to 0.005 |
| jump cooldown | 10 ticks while jump held | documented: wiki "Jumping" (half a second) |
| step_height | 0.6 | documented: wiki "Attribute" |
| velocity threshold | components below 0.003 zeroed at tick start | mcpk.wiki (1.9+) |
| block speed factor | soul sand 0.4, honey 0.4; applied to horizontal velocity after the move | value: oracle; order derived from wiki "Walking": soul sand 2.508 m/s, soul sand under water 0.576, under lava 0.490 all reproduced |
| jump factor | honey 0.5 (oracle) | value: oracle. Not checked: wiki "Jumping" says honey jumps reach 3/16 block, our model gives more; **not determined**, see open questions |
| water drag | 0.8 (all axes) | derived: wiki "Walking" water 1.960 m/s = 20 x 0.0196 / 0.2 |
| water drag sprint-swimming | 0.9 horizontal | derived: wiki "Swimming" 3.918 m/s ≈ 20 x 0.0196 / 0.1 |
| lava drag | 0.5 (all axes) | derived: wiki "Walking" lava 0.784 m/s |
| fluid acceleration | 0.02 | assumption consistent with the water/lava derivations (one equation, two unknowns) |
| water gravity | gravity / 16 | **UNVERIFIED assumption** |
| lava gravity | gravity / 4 | **UNVERIFIED assumption** |
| swim up (jump in fluid) | +0.04 / tick | **UNVERIFIED assumption** |
| swim down (sneak in water) | -0.04 / tick | **UNVERIFIED assumption** (wiki only says "sinks faster") |
| fluid height | level 0: 8/9, level L: (8-L)/9, falling: 8/9, 1.0 with same fluid above | documented: wiki "Water" |
| flying acceleration | flying_speed 0.05 (x2 sprinting) | derived: wiki "Flying" 10.92 m/s (we get 10.89) and 21.6 (we get 21.78) |
| flying vertical | ±3 x flying_speed per tick, vertical drag 0.6 | **UNVERIFIED assumption** |
| sine table | 65536 f32 entries, index = (int)(rad x 10430.378) & 65535, cos = +16384 | mcpk.wiki "Angles"; assumption that 26.3 still does this (`trig::USE_TABLE`) |
| deg -> rad | (deg x π) / 180 in f32; sprint-jump uses deg x 0.017453292 | mcpk.wiki "Angles" (1.8-era); assumption |
| collision epsilon | 1e-7 | assumption (typical f64 voxel sweep value) |
| sneak edge back-off step | 0.05 | assumption |
| box half-width | 0.6f32 / 2 widened to f64 | entity dimensions are floats; consequence: against a wall the player stops at x.69999998808, not x.7 (test `walking_into_a_full_block_stops`) |

## Tick order (as implemented)

1. Rotation from input. 2. Zero |v| < 0.003. 3. Fluid contact (shrunk box vs fluid surface),
sprint/sneak flags, pose (keeps crouching/swimming if the bigger pose does not fit).
4. Jump cooldown; jump (on ground) or swim up (in fluid). 5. Impulse: keys -> ±1, x sneaking
speed when crouching/crawling, x 0.98. 6. Travel: normal / water / lava / flying:
accelerate (rotated by yaw, sine table) -> move (sneak edge back-off, sweep with step-up) ->
zero velocity on collided axes, on_ground = vertical collision while moving down, fall distance ->
block speed factor -> gravity, vertical drag -> horizontal drag.

## Approximate / not modelled

- Step-up uses a single candidate height (full step, clipped by ceilings). Whether 26.3 tries
  several heights: not determined. Stairs/slabs traces will tell.
- Supporting block for friction: block 0.5 below the feet, else the colliding block under the box
  closest to its centre. Approximation of vanilla's supporting block.
- Speed/jump factor block: the block at the feet if non-default, else the one 0.5 below.
- Sprint stop on collision ignores the wiki's "> 8 degrees" angle condition; sprint is not
  restarted while still colliding (assumption). Hunger/blindness: caller clears `input.sprint`.
- Sneak edge protection near (not on) ground follows the wiki's "up to 0.6 blocks above ground"
  via `fall_distance < step_height` and a probe box; exact condition not determined.
- Swimming: pitch-steered vertical swimming, surface bobbing (wiki 2.20 m/s at the surface),
  the climb-out-of-water boost, water currents (1.39 m/s push), bubble columns, depth strider,
  dolphin's grace: not implemented.
- Not implemented: ladders/vines (climbing), cobweb/berry bush/powder snow slowdowns, slime
  bounce, honey sliding, elytra, levitation/slow falling/jump boost/speed effects, world border,
  unloaded-chunk freeze, entity pushing, knockback. `PlayerAttributes` carries no effect state yet.
- Landing ends creative flight (assumption about the exact condition). Double-tap to fly and
  double-tap sprint are input-layer concerns (haste-client).
- `fall_distance` is f32 (interface); vanilla's type not determined.

## Open questions for the traces

- Honey jump height: oracle jump_factor 0.5 vs wiki 3/16 block jump.
- Sprint-jump gap: we cross a 3-block gap (jump onto the 4th block); a 4-air-block gap fails in
  our model without 45° strafing. The wiki's "jump across up to four blocks" does not settle
  which is meant.
- Water/lava vertical constants, flying vertical constants (UNVERIFIED rows above).
- Sine table and the two degree-to-radian constants.
- Trace input alignment (does `ticks[i].input` produce `ticks[i].pos`?): `replay::InputAlignment`.

## Interface notes (requested from the lead, see report)

- `tick` takes `blocks: &dyn BlockPhysics` instead of `data: &GameData` (GameData does not exist
  yet). `BlockPhysics` (friction, speed_factor, jump_factor, fluid) is defined in this crate;
  `oracle::OracleBlocks` implements it from the oracle dumps meanwhile.
- `collide` is the pure sweep; step-up is `collide_with_step(aabb, movement, step_height,
  on_ground, world, shapes)`.
- `PlayerState`/`PlayerAttributes` gained fields (pose, in_lava, swimming, jump_cooldown,
  collision flags; sneaking_speed, flying_speed, crouching/swimming heights).

## Performance

`cargo bench -p haste-physics` (std timing, best of 7 batches, shared 16-core machine, release):
collide walking ~450 ns, terminal-velocity fall ~380 ns, collide_with_step ~480 ns (into a wall
~580 ns), full tick ~1.2 µs. Boxes are gathered once per call into a thread-local reusable buffer
(no allocation after the first call); this was ~2x faster than scanning the world per axis
(4x when blocked by a wall). Above 96 boxes it falls back to the per-axis scan (same results,
unit test `list_and_world_paths_agree`).
