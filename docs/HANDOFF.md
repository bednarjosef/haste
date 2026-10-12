# Handoff: state of Haste Phase 1 after session 1 (2026-10-12)

For the next lead session. Read `AGENTS.md`, then this, then `TASKS.md` and `NOTES.md`.

## Where things are

- Repo: `$HASTE_ROOT`, branch `main`, remote `git@github.com:bednarjosef/haste.git`
  (private for now; it will be made public). Merges go through pull requests opened and merged by
  the lead agent (`gh pr create` / `gh pr merge --merge`); see AGENTS.md "Branches and merging".
- Worktrees: `$HASTE_WT/ws{1..8}-*` on branches of the same name,
  each with `./cache` symlinked to the shared cache and a `.claude/settings.local.json` with the
  worker allow/deny rules. Rebuild or delete freely; they hold nothing that is not on their branch.
- Cache (derived Mojang data, never committed): real location
  `$HASTE_CACHE`; `./cache` in every checkout is a symlink to it.
  `scripts/rebuild-cache.sh` recreates it from nothing in ~5 minutes. `source scripts/env.sh` sets
  `HASTE_CACHE`, `JAVA_HOME` (JDK 25 in the cache) and the project-local Vulkan driver.
- Oracle data in `cache/oracle/26.3/`: `block_shapes.json`, `entity_dimensions.json`,
  `block_properties.json` (real, from the recorder mod on the dedicated server). `traces/`,
  `screenshots/`, `packets/` are empty: they need the user to run the vanilla client once
  (`docs/RECORDING.md`).

## Milestones

| | Status |
|---|---|
| M1 Join | **done headless**: `haste --headless --server 127.0.0.1:25565 --seconds 60` (local Fabric 26.3 server in `cache/server-spike`; start with `crates/haste-protocol/scripts/local-server.sh`). 473 chunks, ~117 entities, every play packet decoded. Uses a stand-in world/entity sink in `crates/haste-client/src/net.rs` until `haste-world` lands. Online mode untested (needs Microsoft tokens). |
| M2 First frame | not reached: `haste-assets` is merged (models baked, atlas, font); `haste-render` (mesher, wgpu) is WS3's branch at a checkpoint; `haste-world` is WS2's branch at a checkpoint. |
| M3 Walk | physics crate done and tested against documented numbers; validation against traces waits for the recording. |
| M4 Alive | entity store + models done; drawing them needs the renderer. |
| M5 UI via tier 3 | **done headless**: real `PauseScreen`/`ChatScreen`/`InventoryScreen` in the embedded JVM, one JNI call per frame. Drawing the `GuiFrame` needs the renderer. |
| M6 Singleplayer + numbers | `ensure_installed` done; `LocalServer`, vanilla launcher, MSA auth and benchmarks remain. |

## What to do next, in order

1. Merge WS2's `haste-world` checkpoint and WS3's `haste-render` checkpoint (branches `ws2-world`,
   `ws3-render`) if they were not merged at the end of session 1; see their NOTES.md for the TODOs.
2. Wire the real `World`/`EntityStore` into `crates/haste-client/src/net.rs` (`WorldSink`/`EntitySink`
   one-line forwards, delete the stubs), then the window/render loop (`app.rs`) with
   `Renderer::update_world` + `render`: that is M2.
3. `player.rs`: input sampling, `haste_physics::tick`, movement packets (M3 minus trace validation).
4. Ask the user to run the recorder (`docs/RECORDING.md`); then `cargo test -p haste-physics
   --test replay -- --ignored` and fix divergences.
5. `ui.rs`: HUD in Rust + `GuiPipeline` when a screen is open; draw `GuiFrame` in the renderer (M4/M5 visible).
6. JVM: bridge registry data into Java (`BuiltInRegistries` is refused today), then tooltips/components.
7. WS7 task 2 remainder: `LocalServer`, vanilla launcher (needed for benchmarks vs vanilla/Sodium), MSA auth (needs a `client_id` from the user).
8. `PHASE1-REPORT.md` at M6.

## Team lessons (session 1)

- Opus (medium effort) agents finished whole briefs in 30-80 minutes with better reports; GLM
  workers took 2-3 hours per task and needed relaunches. Put precision/judgement work on Opus.
- Eight GLM workers on two API keys produced 429 storms and one gate stall (restart `glm-gate`).
  Run at most five, and give them single-line commands and `--allow Bash` (the deny list lives in
  each worktree's `.claude/settings.local.json`).
- Something deleted the repo's `cache/` once; keep the cache outside the tree (done) and never
  track the symlink (`.gitignore` has `/cache`, not `/cache/`).
- Workers ignore "ask before editing root Cargo.toml"; review the manifest diff on every PR.

## Before going public

- Rewrite the author email on the early commits to the GitHub noreply address (later commits use it).
- Parameterise the absolute `~/...` paths in `docs/briefs/*`, `scripts/*.sh`, `NOTES.md`.
- Push `haste-spike` to GitHub too (README and AGENTS.md link to `bednarjosef/haste-spike`).
