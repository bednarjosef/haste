# haste-entity notes

Newest at the top.

- **2026-10-14 Mob models from the recorder dump.** `model_dump.rs` loads
  `cache/oracle/26.3/entity_models.json` once (361 layers) and converts each layer to a shared
  `Arc<CubeModel>`; the dump is in vanilla model space (y down, ground at y = 24 model units),
  so conversion shifts every pivot's y by -24 - cube origins are pivot-relative and stay. The
  dump's root `PartPose` is not recorded (the recorder walks the root's children), which means
  layers whose vanilla `LayerDefinition` was built with a root `MeshTransformer.scaling(f)`
  (villager/witch/illagers 0.9375, husk 1.0625, wither skeleton 1.2, cave spider 0.7, ghast 4.5,
  happy ghast 4.0, elder guardian 2.35, giant 6.0) lose it - `registry.rs`'s table carries the
  scale per entity type and `ModelLibrary::get` applies it (a clone per type, once, then
  cached). Per-axis inflate and part scale are folded to their mean (vanilla uses uniform
  values nearly everywhere; the exceptions are spin-attack arrows and end-crystal cores, not in
  the table). `registry.rs` maps 51 entity types to (layer, texture, scale) after their vanilla
  renderers (adult, default variant only); kind -> name goes through the `entity_type` section
  of the datagen registries.json read directly (haste-data's `Registries::parse` is private);
  everything else keeps the dimension box. Mod-readiness: the table is a fallback default, not
  a registry - when mods add entities their layers should register into a lookup this module
  exposes, keyed by layer id, before the static table.
- **2026-10-14 Head aim on dumped models.** `animation::head_only` gives non-player models
  with a "head" part the same yaw/pitch aim players get (clamped to vanilla's 75 degrees);
  limb animation for mobs is still pending (the models stand in their rest pose, walking is a
  later task). It allocates one rotation entry per entity per frame, same as the player path.

- **2026-10-12 (Task 2) Player-type detection.** Without the entity_type registry (world-and-data) we cannot
  resolve `RegistryIndex -> minecraft:player`. Heuristic: any entity whose UUID has a player-info
  entry is a player (real players always have one, other entity types never do). When world-and-data lands,
  `is_player_type` should be set from `kind == Registries::index("minecraft:entity_type",
  "minecraft:player")` at spawn instead; the heuristic then remains only as a fallback.
- **2026-10-12 (Task 2) Model space convention.** y grows downward (vanilla `PartPose`), feet at
  y=0, units 1/16 block, rotations additive around pivots in x,y,z order. The renderer applies
  `EntityInstance.yaw` to the whole model and the "head" override carries the look direction
  (head_yaw - yaw clamped to +-75 degrees, pitch); if renderer would rather aim the head itself from
  `EntityInstance.head_yaw`, it should ignore the "head" entries we emit — same information
  expressed once, coordinate with renderer.
- **2026-10-12 (Task 2) Limb swing.** `limb_swing` accumulates horizontal distance x 0.5 per
  tick, `limb_speed` (amplitude) smooths towards distance x 4 by 0.4 per tick (publicly known
  vanilla constants: limbs oscillate with `cos(limb_swing * 0.6662)`). At rest the amplitude is
  zero and we emit no limb rotations at all (avoids per-frame String allocation for the overrides).
  Only humanoids get `PoseOverrides`; the generic box has no parts to animate.
- **2026-10-12 (Task 2) Skin layout.** Classic (4-wide) arms are the default model; the slim
  (3-wide, Alex) variant is built and cached too, selected by the skin's "slim" metadata once renderer
  loads skins. Texture id `haste:entity/humanoid` is a placeholder pending the skin pipeline;
  `haste:entity_box` is the flat placeholder for the generic box.

- **2026-10-12 (Task 1) Constructor pending world-and-data.** `docs/INTERFACES.md` specifies
  `EntityStore::new(data: Arc<haste_data::GameData>)`, but `haste-data` does not export `GameData`
  yet (world-and-data task 1 in flight; its `lib.rs` is the scaffold stub). The only thing the store needs it
  for is the `minecraft:entity_type` name -> `RegistryIndex` mapping when applying oracle
  dimensions. Interim signature: `EntityStore::new(dims: EntityDimensions)`; switch to the
  documented one when world-and-data lands (one-line change plus the mapping in `dims.rs`).
- **2026-10-12 (Task 1) Interpolation scheme (3-tick lerp window).** Vanilla remote entities
  smooth position packets over 3 ticks: a delta/teleport-style move sets a target and a 3-tick
  window; each tick the entity travels `remaining`-th of the distance left, so the last step lands
  exactly on the target and a steady 20 Hz packet stream trails the server by ~3 ticks. A new
  packet mid-flight extends the target and restarts the window (observed behaviour; not
  translated from decompiled code). Teleports are a discontinuity: `pos` and `prev_pos` both snap
  so the renderer never draws a frame crossing the gap. `prev_pos`/`pos` bracket each tick's step;
  `render_instances(partial)` lerps between them.
- **2026-10-12 (Task 1) Dimensions.** No oracle file exists yet (`cache/oracle/26.3/` is empty),
  so every entity gets the 0.6 x 1.8 default per the brief. `dims.rs` already parses
  `entity_dimensions.json` when it appears, but cannot map names to `RegistryIndex` without the
  entity_type registry — that mapping waits for world-and-data's `GameData`. Pose heights: crouching scales
  standing height by 1.5/1.8, swimming/elytra/spin-attack by 0.6/1.8 (wiki hitbox values for
  player-like entities; the oracle dump replaces the ratios for odd shapes when it exists).
- **2026-10-12 (Task 1) Deltas.** `move_delta` takes blocks; the 1/4096 unit conversion happens
  in the caller (protocol/haste-client), matching docs/INTERFACES.md's "short deltas /4096" note.
- **2026-10-12 (Task 1) Velocity is stored, not applied.** Per the brief: prediction only if the
  lead asks. `set_velocity` records it on the entity for whoever needs it later.
- **2026-10-12 (Task 1) Metadata.** All entries are kept per entity by index (later updates
  replace same-index entries, matching the wire semantics of partial updates). Interpreted: index
  0 flags byte (on fire, crouching, riding, invisible, glowing, elytra flying), index 6 `Pose`
  (modern layout; older versions differ). Crouching via the flags byte without a pose entry
  adjusts the height too. Everything else stays raw for future consumers.
