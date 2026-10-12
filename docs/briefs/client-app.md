# client-app: the playable client binary (first frame milestone)

Worker: Opus. Worktree `$HASTE_WT/client-app`, branch `client-app`, test server port **25611**.
Read `docs/briefs/COMMON.md` first.

You own `crates/haste-client` except `src/main.rs` (lead; send the exact lines you need),
`src/player.rs` (player workstream) and `src/ui.rs` (jvm-host workstream). You may add modules
(`app.rs`, `game.rs`, `net.rs` changes, ...) and edit `crates/haste-client/Cargo.toml`
dependencies that are already workspace dependencies. Ask the lead for the one-line `mod`
declarations and the `main()` branch to call your entry point; until then a temporary edit of
`main.rs` on your branch is fine, the lead merges it by hand.

## Goal
`haste --server 127.0.0.1:25611` opens a window and shows the world you joined, around the
player's spawn position, at a smooth frame rate, and stays connected for as long as the window is
open. Today the client only runs headless with a stand-in world (`src/net.rs` `StubWorld`,
`StubEntities`).

## Tasks, in order (commit after each, tell the lead after task 1)
1. **Real world and entities in the net driver.** Implement `WorldSink` / `EntitySink` (or replace
   them) with `haste_world::World` and `haste_entity::EntityStore`, delete the stubs, keep headless
   mode working: `--headless --seconds 60` against your server must print chunk/section/entity
   counts from the real `World` and decode every chunk without error. Load `GameData` once
   (`haste_data::GameData::load()`), share it with `Arc`. Biome palette sizes come from the synced
   `minecraft:worldgen/biome` registry (`Connection::registries()`). This is the step the other
   workstreams wait for: commit it and report "task 1 done" to the lead before going on.
2. **Game thread + render thread.** Per `docs/INTERFACES.md` "haste-client wiring": a game thread
   at 20 Hz (drain `Connection::try_recv` into world/entities/player, tick, send) and the winit
   event loop on the main thread calling `Renderer::update_world` with the world's dirty sections
   (`DirtyTracker`) and `Renderer::render` every frame. Share state with as little locking as
   possible: the render thread needs the world for snapshots and the entity render instances; do
   not hold a lock across `render()`. Interpolate camera and entities by partial tick.
3. **Camera before the player lands.** Until the player workstream's `LocalPlayer` is merged, the
   camera sits at the position from the first `player_position` packet (eye height 1.62), mouse
   look with a grabbed cursor (Escape releases), and keep sending the movement/keep-alive/
   `client_tick_end` packets the headless driver already sends so the server keeps you connected.
   When `LocalPlayer` lands (API in docs/INTERFACES.md "haste-client player"), wire it in: input
   sampling from winit (WASD, space, shift, ctrl, mouse) into `InputState`, its tick on the game
   thread, its camera on the render thread.
4. **Window behaviour.** Resize, focus loss (release keys, stop sending input), F11 fullscreen,
   F3 overlay text deferred to the HUD workstream (print FPS and frame time to the title bar for
   now), clean shutdown (disconnect packet, join threads). Vsync off by default, `--vsync` flag.
5. **A screenshot mode for tests.** `--screenshot <file> --seconds N`: join, wait until no meshes
   are pending, render one frame offscreen (`Renderer::new_offscreen` + `read_back`), save it,
   exit. This is how we and CI-on-dev-machines check rendering without a person watching.

## Done
- `--headless` on your server: 60 s, real `World`, chunk count and non-empty sections printed, no
  decode errors, server log shows no kick.
- `--screenshot` produces a PNG of the flat world with sky; open it and describe it in the report.
- Windowed mode: run it for 30 s (`timeout 30`), report FPS from the title/log. Note: on this
  machine a Wayland window that nobody looks at can stall in `get_current_texture`
  (NOTES.md); if that happens, say so and rely on `--screenshot`.
- Tests: unit tests for the sink forwarding and the partial-tick interpolation; clippy clean.
