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

## Measured (this machine: AMD Radeon 860M, RADV, wgpu 30 Vulkan)

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
