# haste-entity notes

Newest at the top.

- **2026-10-12 (Task 1) Constructor pending WS2.** `docs/INTERFACES.md` specifies
  `EntityStore::new(data: Arc<haste_data::GameData>)`, but `haste-data` does not export `GameData`
  yet (WS2 task 1 in flight; its `lib.rs` is the scaffold stub). The only thing the store needs it
  for is the `minecraft:entity_type` name -> `RegistryIndex` mapping when applying oracle
  dimensions. Interim signature: `EntityStore::new(dims: EntityDimensions)`; switch to the
  documented one when WS2 lands (one-line change plus the mapping in `dims.rs`).
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
  entity_type registry — that mapping waits for WS2's `GameData`. Pose heights: crouching scales
  standing height by 1.5/1.8, swimming/elytra/spin-attack by 0.6/1.8 (wiki hitbox values for
  player-like entities; the oracle dump replaces the ratios for odd shapes when it exists).
- **2026-10-12 (Task 1) Deltas.** `move_delta` takes blocks; the 1/4096 unit conversion happens
  in the caller (WS1/haste-client), matching docs/INTERFACES.md's "short deltas /4096" note.
- **2026-10-12 (Task 1) Velocity is stored, not applied.** Per the brief: prediction only if the
  lead asks. `set_velocity` records it on the entity for whoever needs it later.
- **2026-10-12 (Task 1) Metadata.** All entries are kept per entity by index (later updates
  replace same-index entries, matching the wire semantics of partial updates). Interpreted: index
  0 flags byte (on fire, crouching, riding, invisible, glowing, elytra flying), index 6 `Pose`
  (modern layout; older versions differ). Crouching via the flags byte without a pose entry
  adjusts the height too. Everything else stays raw for future consumers.
