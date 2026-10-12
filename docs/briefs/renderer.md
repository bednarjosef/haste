# renderer: everything on screen beyond plain blocks

Worker: Opus. Worktree `$HASTE_WT/renderer`, branch `renderer`, test server port **25613**.
Read `docs/briefs/COMMON.md` first.

You own `crates/haste-render` and `crates/haste-assets`, **except** `crates/haste-render/src/fluid.rs`
and the biome tint code, which the fluids-and-tint workstream (a GLM worker) writes in parallel:
it adds `fluid.rs`, a tint module and one call from the mesher. Keep your changes to `mesher.rs`
small so the two merge cleanly, and tell the lead if you need to restructure it.

## Where it stands
Chunks mesh on worker threads and draw at 211 FPS offscreen at render distance 12
(`Renderer::new_offscreen` + `read_back` work; `NOTES.md`). Nothing else is drawn: `FrameInput`
already carries `entities`, `gui` and `sky`, and they are ignored.

## Tasks, in order (commit after each; offscreen PNGs are your test oracle, look at them)
1. **GUI drawing** (`FrameInput::gui`, `haste_core::gui::GuiFrame`): every `GuiCommand` variant,
   in order, in one or a few draw calls per frame (batched quads, one texture array or atlas for
   GUI sprites from `assets/minecraft/textures/gui/sprites` with their `.mcmeta` nine-slice/tile
   scaling, the bitmap fonts from `font/default.json` including unicode/accented pages, shadow,
   bold/italic/underline/strikethrough/obfuscated styles), scissor stack, translate stack, GUI
   scale. `GuiCommand::Item`: draw the item in the GUI the way vanilla does, from the 26.3 item
   model definitions (`assets/minecraft/items/*.json` -> models; block items as the 3D
   isometric block, flat items as extruded sprites seen flat); the stack count text is drawn by
   whoever emits the frame. Implement `Renderer::text_metrics` exactly from the font's glyph
   advances (vanilla's widths are what Java screens lay out with; the jvm-host workstream calls
   it). Test: a `GuiFrame` built by hand with each command, rendered offscreen, compared to
   expectations (pixel probes at known coordinates), and the jvm-host test frames if available.
2. **Entities** (`FrameInput::entities`, `haste_core::model::EntityInstance` from `haste-entity`):
   cube models with their textures (`textures/entity/...`), per-part pose, light at the entity's
   position, hurt tint, name tags for players. Batch by texture; instancing where it pays off.
   Player skins: the default skins from the jar (`textures/entity/player/wide|slim/*`) chosen by
   UUID as vanilla does; downloaded skins later.
3. **Sky, fog, weather**: sky color and sunrise/sunset from `SkyState::time_of_day`, sun, moon
   phases, stars, clouds (`textures/environment/clouds.png`), rain and snow (`textures/environment/
   rain.png`, `snow.png`), fog by render distance and dimension (`DimensionInfo`, the synced
   dimension type's effects), underwater fog later. Nether/End skies.
4. **World overlays**: block selection outline (from the oracle's outline shapes in
   `haste_data`), block-break cracks (`textures/block/destroy_stage_0..9`) at a position and stage
   the player workstream will supply (`FrameInput` extension: send the field you need to the
   lead), the first-person hand and held item (simple version: arm cube + item model).
5. **Performance pass**: report frame time at render distance 12 and 32 on a normal (not flat)
   world: `scripts/test-server.sh renderer 25613 3600 minecraft:normal` after deleting the copy's
   `world/`. Profile the worst frame-time component and fix it if it is ours.

## Done
- `cargo test -p haste-render -p haste-assets` and clippy clean; offscreen PNGs for each task
  saved under `target/render-checks/` (never committed) and described in the report.
- `FrameInput` / `Renderer` API changes listed for the lead to put in `docs/INTERFACES.md`.
