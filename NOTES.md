# Notes: decisions and dead ends

Newest at the top. One entry per decision, with the why; link the brief or report it came from.

- **2026-10-13 Floating-block regression from PR #15.** Normal-world screenshots show blocks
  scattered in mid-air after PR #15 (capture on mesh workers / flat-array cull walk); clean at
  PR #13. All renderer tests passed because they use hand-built worlds: review renderer merges
  with a normal-world `--screenshot`. Details in docs/HANDOFF.md.
- **2026-10-13 Vanilla HUD and registries in tier 3.** Vanilla's own bootstrap runs in Java;
  server registries are built by vanilla's `RegistryDataCollector` from the exact configuration
  packets; tier-3 packets cross in vanilla's wire format both ways, so container clicks and their
  hashed stacks are byte-exact. The inventory is Java-canonical with a Rust mirror (vanilla's
  menus mutate stacks in place during click prediction). Bootstrap costs 9-47 s: start it on its
  own thread. Phase 2: mods register into the real `BuiltInRegistries`; Rust then takes modded ids
  from Java.
- **2026-10-13 Item stack templates and configuration.** 26.x item stack templates are item,
  count, patch (container, bundle_contents, charged_projectiles, use_remainder, pot_decorations as
  four optional templates without count, advancement icons); `equippable` has five bools. On a
  transfer, close the old connection before connecting or the server kicks the old session.
  We answer `select_known_packs` with none, vanilla with `minecraft:core` (detectable).
- **2026-10-13 Renderer performance.** Per-section buffers cost ~8 ms at ~700 sections; shared
  arenas + multi-draw-indirect fixed it (5 draw calls). SipHash maps on the cull path cost 2-16 ms
  per frame: flat arrays or ahash there. Bake stores atlas-texel UVs; the mesher converts to
  sprite-local. Unifont comes from the assets index, not the jar.
- **2026-10-13 Vanilla client unattended.** Fabric Knot + recorder scenario (`-Dhaste.scenario`)
  that quits the game itself; client mixins need their own config; `graphicsPreset` must be
  "custom" in options.txt; Wayland framebuffers are HiDPI-scaled (record the real size).
- **2026-10-13 Session 2: the real server is the test.** Every component first checked against a
  real 26.3 server had a bug that unit tests missed: section header (i16 block + fluid counts),
  direct biome width (ceil(log2(synced biome count))), packed arrays LSB-first, light-mask BitSet
  (VarInt byte count + little-endian bytes, trailing zeros dropped; flat worlds hid it),
  `player_action` renumbered (1 change destroy direction, 2 abort, 3 finish; the wiki is stale),
  `accept_teleportation` alone confirms a teleport (an extra move packet gets a kick after
  relative /tp), falling water 8/9 high. Rule: verify every enum order and encoding against the
  server; a local probe mod on a private test-server copy (never committed) is fine for observing.
- **2026-10-13 GLM workers dropped.** Four GLM workers (two keys) committed nothing in 2.5 hours
  (huge contexts, 15-minute requests, connection drops); Opus agents at medium effort merged 12
  PRs in the same time. The user chose: Opus only, four at a time, GLM tasks queued for Opus.
- **2026-10-13 Per-worktree test servers** (`scripts/test-server.sh <ws> <port>`, optional
  `HASTE_SERVER=vanilla`); `scripts/env.sh` must always return success (it silently broke
  `source env.sh && ...` chains when HASTE_VULKAN_ICD was unset).

- **2026-10-12 History rewritten, repo recreated.** Early commits carried a personal email (global
  git config) and files carried absolute home paths. History was rewritten (identities to the GitHub
  noreply address, paths to `$HASTE_*` variables; timestamps unchanged) and the GitHub repo was
  deleted and recreated so no old commit survives behind PR refs. PR numbers restart at #1.
  `scripts/check-no-personal.sh` (run by CI) prevents a repeat.

- **2026-10-12 Renderer works offscreen (first frame milestone minus wiring).** haste-render draws a 625-section flat
  world at render distance 12 in 4.75 ms/frame (211 FPS) on the Radeon 860M through the local RADV;
  readback PNGs verified. The windowed loop blocks on `get_current_texture` when the Wayland
  compositor sends no frame callbacks to an unviewed window: test windowed mode in an interactive
  session. Wiring `World` -> `Renderer::update_world` in the client is the lead's next step.
- **2026-10-12 Open: falling-water height.** `haste-data`'s `BlockPhysics::fluid` reports height 1.0
  for fluid level >= 8 (falling), `haste-physics`'s own loader (`src/oracle.rs`) reports 8/9. Which
  matches vanilla is not determined (no decompiled source by rule); settle it with a swim/fall
  trace, then delete the physics-side loader in favour of the shared trait.
- **2026-10-12 vanilla screens milestone reached (headless).** Real vanilla `PauseScreen`, `ChatScreen` and
  `InventoryScreen` run in the embedded JVM through the allow-listed tier-3 loader and 76 generated
  proxies, with one JNI call per frame returning the `GuiFrame`; p50 34 µs per PauseScreen frame.
  Bytecode verification is off (`-XX:-BytecodeVerificationRemote`) so classes link lazily; revisit
  for Phase 2 mod classes. The wall for the next step is `BuiltInRegistries` (tier-1 data): bridging
  Rust's registry data into Java is the next JVM task; until then tags, item components and
  tooltips are approximate. Drawing the frame needs renderer's renderer (first frame milestone).
- **2026-10-12 join milestone reached (headless).** `haste --headless --server 127.0.0.1:25565 --seconds 60`
  stays connected to the local Fabric 26.3 server: 473 chunks, ~117 entities, ~29k packets, every
  play packet decoded, zero unknown ids. Server-verified corrections to the wiki (protocol):
  light-mask bit sets are VarLong-encoded; movement must wait for `player_loaded` after a
  teleport or the server kicks with `invalid_player_movement`; 26.3 has `punch`/`attack`, no
  serverbound `swing`. Online mode is untested until Microsoft tokens exist (setup).
- **2026-10-12 Cache wiped and rebuilt.** At 19:38 the repo's `cache/` directory was deleted and
  replaced by a symlink to itself (which worker did it is not determined; no Opus transcript shows
  the command). Everything in it is derived data, so `scripts/rebuild-cache.sh` recreates it in ~5
  minutes. The real cache now lives OUTSIDE the repo tree at `$HASTE_CACHE`
  and `./cache` (repo and worktrees) is a symlink to it, so a stray `rm -rf cache` in a checkout
  removes only the link. Nothing in the tree depends on cache contents being committed.
- **2026-10-12 Physics sources.** haste-physics is built from minecraft.wiki plus mcpk.wiki (the
  parkour community's observed formulas; only numbers and prose were used, not the decompiled
  snippets that site also quotes). Every constant is tabled in `crates/haste-physics/NOTES.md`
  with its source; water/lava gravity, swim up/down and flight vertical speed are marked
  UNVERIFIED until the recorder traces exist. Pumpkin's entity movement was not reused (it is
  organised after yarn-named vanilla methods). Full tick ~1.2 µs, collide ~450 ns.
- **2026-10-12 Team mix.** Eight GLM workers on two API keys caused HTTP 429 storms and one gate
  stall (restart fixed it). Protocol (protocol), physics (physics) and the JVM host (jvm-host), the precision- and
  judgement-heavy streams, moved to Claude Opus subagents (medium effort) in the same worktrees;
  GLM keeps world/data, entities, renderer assets, setup and oracles (five concurrent).
- **2026-10-12 Renderer: wgpu, with Pomme's azalea-free pieces.** Pomme's renderer is raw Vulkan
  (pyronyx + shaderc GLSL), ~41k lines, and its net/physics/light are either azalea-bound or
  self-declared vanilla ports. We keep wgpu and reuse Pomme's asset index, block model baking,
  atlas stitching, greedy mesher (MIT upstream), occlusion graph and the mesher/buffer design
  (GPL-3.0, attributed). Pomme's Tauri launcher is not reused in Phase 1 (CLI flow), but its
  azalea-free MSA OAuth flow (`pomme-launcher/.../auth.rs`) is the reference for setup. Evaluation
  report: worker `glm-pomme-eval` (2026-10-12).
- **2026-10-12 Protocol: vendor Pumpkin's transport, own packets.** Pumpkin (`cache/pumpkin`, GPL-3.0,
  stable Rust, protocol 777) has a tested framing/compression/encryption layer and NBT, but only
  ~27% of clientbound packets decode (it is a server) and its `pumpkin-data` crate is 1.5M generated
  lines that downloads the server jar at build time. We vendor ~7k lines (transport, wire
  primitives, NBT) into `haste-protocol` and write packet structs on `haste_core` types with ids
  from `packets.json` at runtime. Evaluation report: worker `glm-pumpkin-eval` (2026-10-12).
- **2026-10-12 Vulkan on the dev box.** No system ICD (`vulkan-radeon` not installed, no sudo). A
  project-local RADV built by the fastcraft project works via `VK_DRIVER_FILES` (see `scripts/env.sh`).

- **2026-10-12 Runtime data, not build-time codegen.** Block states, registries and packet ids are
  loaded at startup from the data-generator reports in the cache (`haste-data`, `PacketIds::load`).
  Build-time codegen from gitignored data would make every build depend on the cache and every
  worktree need it before `cargo build` works; runtime loading costs ~100 ms and keeps the legal
  rule simple (nothing derived from Mojang data is ever in the tree).
- **2026-10-12 Collision shapes come from the oracle, with a fallback.** `blocks.json` has no shapes.
  The recorder mod dumps `block_shapes.json` from the real game (server side, headless); until then
  `haste-data` uses a documented heuristic fallback so first frame/walk milestones work can proceed.
- **2026-10-12 No azalea.** Its 26.3 branch needs shifting nightly features and failed to build on
  three nightlies in the spikes. Pomme depends on azalea's 26.2 branch for protocol/crypto/auth, so
  Pomme's protocol layer is not reusable as-is either (evaluation in progress).
- **2026-10-12 Worktrees share a prebuilt target.** `cargo build` of the workspace deps (wgpu, winit,
  tokio) is copied into each worktree's `target/` so eight workers do not compile them eight times.
