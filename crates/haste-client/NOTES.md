# haste-client notes (client-app)

- **Threads.** Main thread: winit loop and renderer. `haste-game`: the 20 Hz tick (`game.rs`).
  Two `haste-net` tokio workers: the connection. The world and the entity store sit behind
  `parking_lot::RwLock`s. The game thread write-locks each one once per tick, for the packet
  drain. The render thread read-locks them only to capture sections and to copy entity
  instances, never across `Renderer::render`.
- **Dirty sections.** `World` marks loaded and changed sections. It does not mark the sections
  of an unloaded chunk, or of every chunk on a respawn, so `game::TrackedWorld` adds those so
  the renderer replaces their meshes with empty ones.
- **Mesh feeding** (`app::WorldFeed`). Nearest first, capped at 4 ms of capture per frame (each
  `MeshSnapshot::capture` is 3x110592 `BlockView` calls). Sections that are empty and were
  never meshed are skipped: on a flat world that is 23 of 24 sections per chunk.
- **Camera.** The game thread publishes `LocalPlayer::camera(0.0)` and `camera(1.0)` each tick,
  and the render thread interpolates between them by partial tick. The look is the input
  layer's yaw and pitch plus `LocalPlayer::look_offset`, so mouse look runs at frame rate.
- **Screenshot.** It is "settled" when the feed queue is empty, `meshes_pending == 0`, the
  renderer holds a mesh entry for every section submitted, and nothing new has arrived for
  1.5 s. Width*4 must be a multiple of 256 (`read_back`), hence the 960x540 default.
- **Approximate.** Time of day comes from the first clock in `set_time`, and that this is the
  overworld day clock is not verified. Vsync cannot be turned off: the renderer always
  presents with FIFO.
