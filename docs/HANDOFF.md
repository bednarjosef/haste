# Handoff: state of Haste Phase 1 after session 2 (2026-10-13)

For the next lead session. Read `AGENTS.md`, then this, then `TASKS.md` and `NOTES.md`.

## How to run it

```
scripts/test-server.sh mine 25700 3600 minecraft:normal      # private Fabric 26.3 server (offline, creative)
cargo run --release --bin haste -- --server 127.0.0.1:25700  # window: WASD, mouse look, click to grab, Esc releases
cargo run --release --bin haste -- --server HOST --screenshot out.png [--yaw Y --pitch P --width W --height H]
cargo run --release --bin haste -- --server HOST --headless --seconds 60
```
`HASTE_SERVER=vanilla scripts/test-server.sh ...` runs Mojang's plain server (the purer oracle).
`vulkan-radeon` is now installed system-wide on the dev laptop; `WGPU_BACKEND=gl` forces the GL
fallback. Walk/interaction scripts: `HASTE_WALK_SCRIPT=crates/haste-client/walks/<name>.txt`
with the bot opped on the server.

## What works (all on main)

- **Join and stay connected** to vanilla/Fabric 26.3 servers in offline mode, on normal terrain:
  every chunk and light packet decodes (PR #10 BitSet fix), 0 decode failures over a ~180-command
  scenario with mobs, block entities, items with components, advancements, a second player.
- **Configuration both ways** (PR #17): start_configuration back to configuration, transfer
  (`/transfer` rejoins cleanly), cookies, resource pack answers, brand "vanilla".
- **Window client** (PR #8): 20 Hz game thread + winit render thread, mouse look at frame rate,
  `--screenshot` for unattended rendering checks.
- **Movement** (PR #5): `LocalPlayer` sends vanilla's per-tick packets; a 180 s scripted walk on
  the vanilla server (walk, sprint-jump, sneak edges, stairs, slabs, 20-block fall = 17 damage,
  swim, creative flight) has 0 corrections and no "moved wrongly".
- **Interaction** (PR #12): survival/creative breaking with vanilla mining speed, placing, using,
  attacking, pick block, hotbar; mouse and number keys in the window; 10/10 server-side
  confirmations in `walks/interaction.txt`.
- **Rendering** (PRs #7, #9, #11, #15): textured terrain, sky/sun/moon/stars/clouds/rain,
  Nether/End skies, entities as cube models with default skins and name tags, selection outline
  and break cracks, first-person arm/held item, the full `GuiFrame` (vanilla font widths,
  nine-slice sprites, 3D GUI items), GL fallback. Batched draws: offscreen render distance 12 at
  1.3-2.3 ms; windowed 2816x1656 ~7 ms (~140 FPS) after PR #15.
- **Vanilla HUD and screens in Java** (PR #16): vanilla's bootstrap runs in tier 3 (95
  registries / 7053 entries match the data generator), server registries via vanilla's
  `RegistryDataCollector`, vanilla `Hud` (hotbar, health, chat, boss bars, sidebar, titles, tab
  list), chat with command suggestions, death screen, chests with byte-exact `container_click`
  hashed stacks, kick screen. HUD p50 80 us/frame. **Not wired into the window yet** (see below).
- **Vanilla reference** (PR #18): `scripts/vanilla-client.sh` runs the real 26.3 client
  unattended (a window opens on the desktop) against `scripts/vanilla-server.sh` (seed 20263);
  `scripts/compare-vanilla.sh` produces vanilla | haste | difference images in
  `target/vanilla-compare/`.
- **Biome colours, water and lava** (PR #19): per-biome grass/foliage/water colours from the
  synced registry and the player's colormaps, 5x5 blending, fixed block tints; grass tops match
  the vanilla reference within ~2 per channel. Water/lava meshing with corner heights,
  flow-rotated textures, culling, translucency. Animated textures (water, lava, fire...) show
  frame 0: the atlas has no animation yet. Leaves, bark and grass sides are ~1.35x brighter than
  vanilla (lighting/shading, not tint).

## End-of-session check (main at the merge of PR #19)

- `cargo test` for haste-render/haste-assets (including GPU tests) and haste-client: all pass;
  workspace clippy `-D warnings` and fmt clean. The jvm-host server tests (`ui::tests`,
  `#[ignore]`) need their test server on port 25614 with `haste` opped and `--test-threads=1`.
- Headless 40 s on a normal-terrain Fabric server: 473 chunks, 4650 non-empty sections, 126
  entities, 0 chunk errors, no unknown packet ids, stayed connected.
- `--screenshot` on the same server: textured snowy terrain, ice, sky, outline; **blocks floating
  in mid-air** (problem 1 below).

## Known problems (documented, not fixed: the user asked for a wind-down without fixes)

1. **Floating blocks on main (regression from PR #15).** On a normal world, dirt/stone cubes
   are drawn scattered in mid-air above the terrain. Bisected with `--screenshot --yaw 180
   --pitch 25` on the same server: clean at `b5fe1fd` (PR #13), broken at `ecd321f` (PR #15).
   PR #15 moved section capture to the mesh workers (`ca0750e`) and rewrote the cave-culling
   walk over flat arrays (`d41a04a`); one of them misplaces sections (wrong section origin or a
   capture race). First task next session: find which, fix, add a regression screenshot test on a
   normal world. Fallback: `git revert -m 1 ecd321f` restores correct output at ~12.5 ms/frame.
2. **The HUD and screens are not visible in the window.** `crates/haste-client/src/ui.rs` (API
   in its doc comment and INTERFACES.md) works headless; it still has to be wired into
   `game.rs`/`app.rs`: `Ui::handle` for every packet, `tick`, `drain_outgoing`, key/mouse routing
   (screen open -> Java, else game), `frame()` into `FrameInput.gui`, `Renderer::text_metrics`
   into `set_text_metrics`, JVM bootstrap (9-47 s) on its own thread at startup.
3. **Vanilla comparison numbers are not meaningful yet**: 94-98 % differing pixels on the three
   forest shots, dominated by untinted grass and a camera ~1.2 blocks low in Haste's shot. Our own
   `--screenshot` places the eye correctly at feet + 1.62 on the lead's server, so the low camera
   is probably the comparison script's teleport race (the script teleports the bot repeatedly
   during join); a `--pos x y z` screenshot flag would remove it. GUI shots need an exact
   framebuffer size (Wayland HiDPI gives 2816x1656 for a 1280x720 window, XWayland 1408x828).
4. Machine load: eight agents plus servers ran the vanilla servers into "Can't keep up" stalls;
   a GLM worker's scripts once took 15 GB. Check `free -g` and server logs before trusting a
   tick-counted result; `HASTE_TICK_RATE` + `/tick rate` separate server lag from client bugs.

## What is left for Phase 1 (estimates in Opus-hours, from session 2's pace)

| Task | Brief | Estimate |
|---|---|---|
| Fix the PR #15 regression | - | 0.5-1 |
| Wire `Ui` (HUD, chat, inventory, menus) into the window; menus' keybinds | jvm-host next steps | 2-3 |
| Sound (`haste-audio` is an empty crate) | docs/briefs/sound.md | 1.5-2 |
| World: block entities (one unreviewed commit on branch `world-and-data`), region-file oracle, section updates | docs/briefs/world-and-data.md | 1-2 |
| Setup: `haste setup/login/singleplayer/vanilla`, player certificates (unreviewed WIP `launch.rs` on branch `setup-cli`) | docs/briefs/setup-cli.md | 1-2 |
| Protocol: chat and signing, typed data components, remaining enum checks, `select_known_packs` answering `minecraft:core` | docs/briefs/protocol-compat.md tasks 3-5 | 3 |
| Vanilla reference: GUI scenarios, more viewpoints, physics traces + replay fixes | docs/briefs/vanilla-reference.md tasks 2-4 | 2-3 |
| Block entities rendering (chests, signs with text, beds, banners, heads, shulkers) | not written | 2-3 |
| Exact mob/player models from the jar's Java code, animation, armour, skin download | not written | 3-4 |
| Particles | not written | 2-3 |
| Rendering details (animated textures, smooth light/AO vs vanilla, fog, item frames/paintings, hurt tint) | not written | 2-3 |
| Title screen, server list, options and keybind screens (vanilla Java), settings persistence | not written | 2-3 |
| Vehicles, riding, elytra | not written | 2-3 |
| Online mode end to end (needs a Microsoft app client_id and the user's login) | - | 1-2 |
| Benchmarks vs vanilla and Sodium, PHASE1-REPORT.md | - | 2 |

Roughly 30-40 Opus-hours: two to three sessions with four Opus agents. Code size at the end of
session 2: 62.5k lines of Rust, Java and WGSL (43.8k at its start).

## Team and process lessons (session 2)

- **Opus at medium effort, four at a time, is the team.** GLM workers were dropped: four of them
  committed nothing in 2.5 hours (huge contexts, 15-minute requests, connection drops) while
  Opus merged 15+ PRs. The user decided Opus only.
- **The real server is the test.** Every component checked against a real server had a bug unit
  tests missed (NOTES.md 2026-10-13). Give every brief a server-side oracle: the vanilla
  server's movement checks and logs, a second headless client as observer, region files, a local
  probe mod on a private server copy (never committed), the vanilla client via
  `scripts/vanilla-client.sh`.
- **Screenshots after every renderer merge.** The PR #15 regression passed all tests (they use
  hand-built worlds); a normal-world `--screenshot` would have caught it. Add one to the review
  routine (`scripts/review.sh`) next session.
- The lead merges in its own worktree (`$HASTE_WT/lead`), never in the user's checkout.
- `scripts/env.sh` must always return success (fixed in PR #13); it had silently broken every
  `source env.sh && ...` chain.
- Agents report through SendMessage mid-task ("task 1 done" + hash) so the lead can merge and
  unblock others early; that worked well.

## Before going public

Unchanged from session 1: history is already clean (`scripts/check-no-personal.sh` runs in CI);
push `haste-spike` to GitHub too (README and AGENTS.md link to it).
