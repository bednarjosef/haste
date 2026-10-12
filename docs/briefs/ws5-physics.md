# WS5: haste-physics

You own `crates/haste-physics` and later `crates/haste-client/src/player.rs` (input sampling +
movement packets; wait for the lead to say M1 is merged before touching haste-client).

Goal: vanilla-exact player movement and collision, validated tick by tick against recorded traces
from the real game (WS8 records them into `cache/oracle/26.3/traces/*.json`; format in
`crates/haste-testkit/FORMATS.md`: per tick `input` (forward/back/left/right/jump/sneak/sprint,
yaw, pitch), `pos`, `velocity`, `on_ground`, `pose`). Until traces exist, build from the public
documentation and unit tests; the traces will then tell you where you are wrong.

## Legal constraint (important)
Implement from the public descriptions (minecraft.wiki pages on player movement, sprinting,
jumping, falling, "Entity/Movement", block collision) and from observation. Do NOT read or
translate decompiled Minecraft code, and do NOT reuse Pomme's `physics/` or `player/` modules
(they were matched line by line against decompiled source). You may reuse Pomme's generic AABB
sweep helpers only if they are clearly generic geometry and you keep the header + THIRD_PARTY entry.

## Task 1: collision, pure and tested
- `collide(aabb, movement, world, shapes) -> DVec3`: axis-separated sweep (Y first, then X/Z
  ordered by larger magnitude as vanilla does), against `CollisionShapes::collision_boxes` of the
  block states in the swept region, with step-up (step height 0.6) when moving horizontally and
  on ground. No allocation per call beyond a reusable scratch buffer.
- Tests with a tiny in-memory `BlockView` + `CollisionShapes` (full cubes, slabs, stairs-ish
  boxes): falling stops on a floor exactly at the surface; walking into a wall stops at the face;
  step up a half slab; cannot step a full block; corner cases where two boxes meet.

## Task 2: movement tick
- `tick(...)` per INTERFACES.md: input -> acceleration (walk 0.1, sprint x1.3, sneak x0.3, air
  control 0.02), friction 0.6 ground / 0.91 air (slipperiness x 0.91 on ground... use the wiki's
  documented values and name the source), gravity 0.08 with 0.98 drag, jump 0.42 (+ sprint jump
  boost), fall distance, on-ground detection, swimming/water drag and buoyancy, flying (creative),
  sneaking edge-safety. Rotation from input.
- Properties that depend on blocks (slipperiness of ice, soul sand speed, water/lava) come from
  `GameData` block identifiers for now; say what is hard-coded.
- Tests: terminal velocity and fall timing from known heights match the documented numbers;
  sprint-jump distance ~ documented; 100 ticks of walking on flat ground produce a constant speed.

## Task 3: trace replay (when traces exist)
- `cargo test -p haste-physics --ignored replay`: for every trace in the cache, replay the inputs
  from the first tick's state and report the first tick where position differs by > 1e-6 and the
  max error. Goal: zero divergence over each trace. Iterate on the rules until it holds.

Do not touch: haste-world (consume `BlockView`), haste-entity, haste-render, haste-protocol.
