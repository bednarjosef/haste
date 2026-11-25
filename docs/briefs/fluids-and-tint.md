# fluids-and-tint: water, lava and biome colors in the chunk mesher

Worker: GLM. Worktree `$HASTE_WT/fluids-and-tint`, branch `fluids-and-tint`, test server port
**25616**. Read `docs/briefs/COMMON.md` first.

You own two new files in `crates/haste-render/src`: `fluid.rs` and `tint.rs`, plus the smallest
possible hooks into `mesher.rs`, `lib.rs` and `renderer.rs` (a few lines each: the renderer
workstream edits the same crate in parallel, so do not restructure existing code; if you need
to, stop and report). Read `mesher.rs`, `vertex.rs` and `renderer.rs` first.

## Fluids (`fluid.rs`)
Today water and lava are not meshed. Vanilla's visible behaviour (minecraft.wiki "Water",
"Lava", "Fluid"; observe with screenshots of your test server):
- Fluid states come from block states: `minecraft:water` / `minecraft:lava` with property
  `level` 0..15 (0 = source, 1..7 flowing with decreasing height, 8..15 falling), and any block
  with `waterlogged=true` contains a water source. `haste_core::view::BlockPhysics::fluid` already
  reports kind and level per state; use it.
- Surface height per corner is the average of the heights of the (up to four) fluid columns
  sharing that corner; a column under the same fluid is full height; source height is 8/9.
  Document exactly the rule you implement and what you verified by screenshot.
- Faces: top face drawn unless the same fluid is above; side faces culled against the same
  fluid and against full opaque blocks; bottom face drawn when not the same fluid below.
- Textures: `block/water_still`, `block/water_flow`, `block/lava_still`, `block/lava_flow`
  (animated, `.mcmeta` frames: the atlas must handle animation; if it does not yet, use frame 0
  and report). Flowing top faces use the flow texture rotated along the flow direction (the
  gradient of the corner heights); sides use the flow texture.
- Water is translucent (tinted by the biome water color), lava is opaque and emits light 15.
- Pomme's fluid meshing (`cache/pomme`, look for fluid/water in its renderer) may be reused
  under COMMON.md's rules (GPL header, docs/THIRD_PARTY.md entry).

## Biome tint (`tint.rs`)
Today the tint palette index in the vertex format is not filled from biomes. Implement:
- Biome colors from the server's synced `minecraft:worldgen/biome` registry
  (`haste_protocol::SyncedRegistries`, raw NBT per biome: `temperature`, `downfall`, `effects`
  with `water_color`, optional `grass_color`, `foliage_color`, `dry_foliage_color`,
  `grass_color_modifier`). Without explicit colors, grass and foliage come from the colormaps
  `textures/colormap/grass.png` / `foliage.png` sampled by clamped temperature and downfall
  (minecraft.wiki "Color" / "Biome" pages document the lookup). `dark_forest` and `swamp`
  modifiers per the wiki (swamp uses a noise: implement what the wiki documents, mark it).
- Biome blending: vanilla's default blend averages over a square around the block (Video
  Settings "Biome Blend", default 2 = 5x5). Implement radius as a parameter, default 2.
- Fixed block tints that do not depend on biome (spruce/birch leaves, lily pad, redstone wire
  by power, stems by age, ...) per minecraft.wiki, as a table keyed by block identifier.
- API: `pub struct BiomeClimate { temperature, downfall, water_color, grass_color: Option<u32>,
  foliage_color: Option<u32>, dry_foliage_color: Option<u32>, grass_modifier }` (plain data, no
  protocol types: `haste-render` does not depend on `haste-protocol`) and `pub struct BiomeColors`
  built by `BiomeColors::new(biomes: Vec<BiomeClimate> /* index = biome registry id */, assets: &Assets)`,
  handed to the renderer with `Renderer::set_biome_colors(BiomeColors)` (new method; tell the lead
  the signature). The conversion from the synced registry NBT to `Vec<BiomeClimate>` goes in a new
  file `crates/haste-client/src/biomes.rs` (yours) as `pub fn biome_climates(&SyncedRegistries) -> Vec<BiomeClimate>`
  with a unit test; the lead wires it. The mesher computes per-vertex tint from the section
  snapshot's biomes.

## Tasks, in order (commit after each)
1. `tint.rs` with unit tests (colormap lookup, explicit colors, blending on a synthetic grid).
2. Hook tint into the mesher; offscreen screenshot of grass and leaves on your test server
   (`--screenshot` arrives from the client-app workstream; until then, the offscreen tests in
   `haste-render` that build a world by hand).
3. `fluid.rs` with unit tests on hand-built fluid grids (corner heights, culling, flow direction).
4. Hook fluids into the mesher; screenshot of a pool, a waterfall and lava
   (`/fill` and `/setblock` on your server).

## Done
`cargo test -p haste-render` and clippy clean, screenshots under `target/render-checks/` (never
committed) described in your report, exact list of approximations.
