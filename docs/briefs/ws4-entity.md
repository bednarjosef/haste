# WS4: haste-entity

You own `crates/haste-entity`. Implement `EntityStore` exactly as in docs/INTERFACES.md. Consumers:
`haste-client` (applies packet-derived calls), `haste-render` (draws `EntityInstance`s), WS5
(player attributes come from elsewhere; you do not own the local player).

## Task 1: tracking + interpolation
- `EntityStore` with dense storage (slot map or `Vec` + free list; `EntityId -> slot` map). No
  per-tick allocation. `spawn`, `remove`, `move_delta` (deltas are 1/4096 block units as sent;
  convert), `set_rotation`, `teleport`, `set_head_yaw`, `set_velocity`, `set_metadata`
  (store entries by index; interpret the shared ones: index 0 flags (sneaking, sprinting,
  invisible, glowing), index 6 pose; keep the rest raw per entity), `set_equipment`, `player_info`.
- Interpolation: keep `prev_pos`/`pos` per tick; `render_instances(partial_tick)` lerps. Vanilla
  smooths remote entity moves over 3 ticks ("lerp steps"); document the scheme you implement
  (observed behaviour, not decompiled code) in NOTES.md.
- Dimensions: from `cache/oracle/26.3/entity_dimensions.json` when present (WS8 writes it;
  format `{"version":1,"entities":{"minecraft:zombie":{"width":0.6,"height":1.95,"eye_height":1.74},...}}`),
  else 0.6 x 1.8. Pose changes (sneaking/swimming) adjust height as documented on the wiki.
- `tick()` advances interpolation, applies velocity for entities without server moves only if the
  lead asks (not now).
- Tests: spawn/get/remove; delta moves accumulate exactly; teleport resets interpolation; metadata
  pose sets height; `render_instances` at partial 0.5 is the midpoint; 10k entities tick in < 1 ms (timed test, release).

## Task 2: models as data (`haste_core::model`)
- Humanoid/player model (standard 64x64 skin layout: head 8x8x8 at uv 0,0; body 8x12x4 at 16,16;
  right arm 4x12x4 at 40,16; left arm 32,48; right leg 0,16; left leg 16,48; outer layers at the
  known offsets; slim arms 3 wide at the same uvs) as a `CubeModel`. A generic box model from
  dimensions for every other entity type (textured with a flat placeholder identifier
  `haste:entity_box`). Quadruped and a few common mobs can come later.
- Animation: walking limb swing from horizontal speed (vanilla's swing amplitude is public
  knowledge: limbs oscillate with `sin(limbSwing * 0.6662)`), head yaw/pitch, sneaking pose.
  Output as `PoseOverrides` per instance.
- Nametags: player entities get their name from `player_info` (uuid match).
- Tests: model part tree has expected parts; walking animation is zero at rest and bounded.

Do not touch: haste-render (you provide data, WS3 draws it), haste-physics, haste-protocol.
If you need a new field on `EntityInstance` or `CubeModel`, request it from the lead.
