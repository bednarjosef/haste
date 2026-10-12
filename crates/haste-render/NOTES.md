# haste-render notes

## Design (from Pomme, GPL-3.0 — see THIRD_PARTY.md)

- **Vertex packing** (`vertex.rs`): 20-byte packed vertex. u16 section-local
  positions quantized over `[-4, 20]` (POS_RANGE 24 / POS_BIAS 4): a 16-block
  shift is an exact integer number of u16 steps, so world positions encode
  identically in adjacent sections (no seams). Sprite-local u16 UVs (4095 per
  unit) keep greedy-quad repeats exactly representable. Light is sky/block
  NIBBLES resolved through a lightmap in the shader — time-of-day darkening
  never triggers a remesh (differs from Pomme, which bakes brightness).
  wgpu requires attribute offsets aligned to format size, hence the layout
  (pos u16x4, uv u16x2, light u8x4, sprite u16, pad).
- **Meshing** (`mesher.rs`): `MeshSnapshot` captures the 3x3x3 section
  neighbourhood (48^3 blocks + light) on the calling thread (world data is not
  shared across threads); workers mesh from the immutable snapshot.
  Per-section meshing: cullface culling against full-opaque neighbours,
  vanilla AO (AdjacencyInfo per-face corner table, verified against Pomme's
  rows by test), smooth light averaged over the AO neighbour cells, AO-aware
  diagonal flip, four render-layer index lists. The pool runs on rayon
  (`spawn` + `into_par_iter`) with a bounded result channel (256) and a
  32-meshes-per-frame drain budget (Pomme's MeshDispatcher back-pressure
  design).
- **Occlusion** (`occlusion.rs`): Pomme's VisGraph flood fill per section
  (VisibilitySet, 36-bit face-to-face connectivity) + SectionOcclusionGraph
  walk for cave culling. Wired as `section_visibility`; the render loop does
  frustum culling today — the occlusion walk is ready to add as a second
  culling stage.
- **Shaders** (`chunk.wgsl`): WGSL port of Pomme's chunk.vert/frag design:
  sprite-rect atlas wrapping with `fract()` + derivatives (greedy-quad-safe),
  cardinal face shading, AO brightness table, two-band fog. Section origins
  are an instance-rate vertex buffer (wgpu has no push constants on all
  backends).

## Session 2 (GUI, entities, sky, overlays, performance)

- **GUI** (`gui.rs`): one vertex buffer per frame over a lazily filled 4096^2 atlas (sprites,
  textures, font pages, unifont glyphs, item textures), one draw per scissor change. Drawn into a
  non-sRGB view of the target so blending is on sRGB values like vanilla; on backends without
  view formats (GL) it renders linear colour into the sRGB target (half-transparent fills then
  blend in linear space). Text with alpha 0 is not drawn (vanilla since 1.21.6).
- **Entities** (`entity.rs`): CPU-posed cubes, one draw; child pivots are relative to the parent
  (vanilla PartPose). Light comes from `sample_world` / `sample_entity_lights` (the renderer
  holds no world). Hurt tint needs an `EntityInstance` field (not there).
- **Sky** (`sky.rs`): time-of-day curves from minecraft.wiki; sunrise colour, stars, rain columns
  (no heightmap: rain falls through roofs) and flat clouds are approximations.
- **Terrain draw path**: section meshes in shared arenas (`arena.rs`), one
  `multi_draw_indexed_indirect` per layer and arena (needs INDIRECT_FIRST_INSTANCE), cave
  culling walk (`occlusion.rs`) over flat arrays, rerun on camera section change or at most
  4x/s while sightline data arrives. Dead end: per-section buffers cost ~8 ms/frame in wgpu
  encode + submit at ~700 sections; std HashMap (SipHash) lookups in the cull loop and walk cost
  2-16 ms.
- **Bug fixed**: the bake stores atlas-texel UVs; the mesher must convert back to sprite-local
  0..1 (every face showed one texel before).

## Measured

- Windowed client, normal world, render distance 12, 2816x1656, Mailbox, loaded machine (load
  ~19): ~7 ms/frame (~140 FPS) steady, from ~12.5 ms; breakdown per frame: prepare 1.6, acquire
  0.6, cull 1.7, encode 0.15, submit 2.0, present 0.7 ms CPU; GPU main pass 5.3-5.9 ms.
  `Renderer::timings()` and the client's 2 s log line give this breakdown.
- `examples/perf_world synthetic 12` (1280x720 offscreen): 1.3-2.3 ms/frame.

## Earlier measurement (session 1)

this machine: AMD Radeon 860M, RADV, wgpu 30 Vulkan)

- Offscreen 640x480, render distance 12, flat world, 625 sections (348
  drawn after frustum culling): **4.75 ms/frame = 211 FPS** steady state
  (100-frame average, `examples/offscreen_dump --distance 12`).
- The GPU test in `renderer.rs` (ignored without the jar) renders a 3x3-section
  flat world and verifies the readback image contains terrain.

## Known limitations / next steps

- **Windowed loop stalls in this background session**: `get_current_texture`
  blocks after the first frame because the Wayland compositor never sends
  frame callbacks to a window nobody is looking at. The offscreen path
  proves rendering + performance; `examples/view_world.rs` works in an
  interactive session (it opens, renders its first frame, and waits).
- **Per-section draw calls** (348 draws): fine at 200+ FPS, but the intended
  end state is Pomme's big-buffer sub-allocator (`buffer.rs`, on the reuse
  list) + multi-draw-indirect. Deferred until a real joined world needs it.
- **Greedy meshing** (Pomme's MIT-derived `greedy.rs`) is on the reuse list
  but not ported yet: the current mesher emits per-face quads. The vertex
  format and shader already support greedy quads (sprite-local UVs wrap).
- **Lightmap** is an approximate curve (pow 1.55) rather than vanilla's
  light texture; tint palette is white-only (grass/foliage colormaps pending).
- **update_world ignores view_distance/camera_chunk** (no unloading yet —
  sections accumulate until the Renderer is dropped).
