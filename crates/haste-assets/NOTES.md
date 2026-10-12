# haste-assets notes

## Decisions

- **The client jar is the asset source.** `Assets::open` reads `assets/**` out of the client jar
  zip directly (`zip` crate, cached ZipArchive). The assets index/objects dir (`assets_dir`) is
  consulted as a fallback for entries missing from the jar; in 26.3 the jar carries the whole
  vanilla asset tree, so the cache has no extracted assets dir and `Assets::open` works without
  one (the index read soft-fails with a warning). `haste_core::paths::cache_dir()/assets` is the
  expected `assets_dir`.
- **Pomme reuse** (see THIRD_PARTY.md): asset index/resolve_ref from `assets.rs`, the model
  baking geometry (`model.rs`), blockstate variants/multipart (`multipart.rs`), the shelf packer +
  sRGB-correct mips (`atlas.rs`). Pomme's Vulkan-specific code, light engine and item-model
  machinery were not reused.

## Findings from the 26.3 jar (verified against `cache/versions/26.3/client.jar`)

- **Blockstate variant keys are partial since the 1.21.2 flattening**: keys name only
  render-relevant properties (`oak_stairs`: facing/half/shape, not `waterlogged`; `oak_trapdoor`:
  facing/half/open, not `powered`/`waterlogged`). `model_refs_for` therefore matches a variant key
  when every property named in the key matches; unspecified properties are wildcards. On multiple
  matches the most specific (most properties) key wins.
- **No `render_type` field in 26.3 block models.** Render layers come from a heuristic
  (`bake.rs::classify_layer`): known vanilla `ItemBlockRenderTypes` sets by block name (glass,
  ice, water, stained glass, leaves -> cutout_mipped, ...) with a texture-alpha fallback
  (any non-opaque texture -> cutout). Approximate: exotic modded-like cases (e.g. tripwire)
  may land one layer off until compared against the real game.
- **`gui_light` is a string** (`"side"` default, `"front"` for flat items), not a boolean.
- **Bitmap font providers need explicit `#[serde(rename)]`**: serde's internally-tagged enum
  matching does not apply `rename_all`-style lowercasing, and a `#[serde(other)]` fallback
  silently swallowed the failure (all font providers parsed as "Unimplemented"). Every variant
  in `FontProvider` has an explicit rename now.
- **Font provider `file` is a full texture id ending in `.png`** (e.g.
  `minecraft:font/ascii.png`), unlike `Assets::texture` which appends `.png` itself.
- **`font/default.json` is references only**: `include/space` (a `space` provider with
  advances `{" ": 4, ZWNJ: 0}` — space = 4 px, from the file), `include/default` (three bitmap
  pages: nonlatin_european.png 67 rows, accented.png 75 rows height 12, ascii.png 16 rows ascent
  7), `include/unifont`. Glyph width = last non-empty column + 1 over the 8-px cell (verified by
  decoding ascii.png: `W` 5 px, `A` 5, `i` 1, `l` 2, space 0 -> provider advance 4).
- **GUI sprite `.mcmeta` scaling**: 47 nine_slice (uniform `border`, no per-side borders in
  26.3), 6 without `gui` section (stretch). No `tile` sprites exist in 26.3 but the type is
  parsed anyway.

## Approximate / not done

- **Animated textures**: the atlas keeps only the first frame (vanilla's initial display frame
  when `frames` lists one; otherwise frame 0). `AnimationMeta` carries width/height/frametime
  for the renderer to animate later. `.mcmeta` `frametime` per-frame and `interpolate` are not
  implemented.
- **`RenderLayer` classification is heuristic** (see above); `_glass_pane`/`_glass` name
  suffixes plus a fixed name set.
- **haste-data is not wired yet** (world-and-data still in progress on this branch): `BakedBlockModels::bake`
  takes a `BlockStatesSource` trait instead of `GameData`'s `BlockStates`. A local
  `BlocksJsonStates` reads `blocks.json` directly with the same semantics. Once haste-data
  lands, it should implement `BlockStatesSource` (or the lead wires the trait) — requested in
  the report.
- **Model `display` transforms are parsed but unused** (GUI item transforms are Task 3).
- **Mipmaps**: plain sRGB-correct 2x2 box filter (Pomme's `mean_linear`). Pomme's
  cutout/coverage-preserving strategies (`solidify_transparent_rgb`, `scale_alpha_to_coverage`)
  are not ported yet; cutout mips may bleed slightly. Port if screenshot tests need it.
- **Bake time**: ~7-8 s for all 35723 states in debug, single-threaded, including the 1024²
  atlas. Optimization (parallel per-blockstate baking) is deferred until Task 2 needs it.

## Tests

- `cargo test -p haste-assets`: 19 unit tests (fixtures, no jar needed).
- `cargo test -p haste-assets -- --ignored` (needs `cache/`): stone 6 quads + full-opaque,
  oak_stairs facing=east > 6 quads, glass translucent, oak_leaves cutout_mipped, bake time and
  atlas size reported; font/page/language/gui_sprite/model parent-chain round-trips.
