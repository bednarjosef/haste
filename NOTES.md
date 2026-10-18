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
- **2026-10-13 Protocol enum orders audited against the 26.3 jar.** Every enum Haste sends or reads
  as an ordinal (player action and command, client command, hands, pose, game mode, difficulty,
  client information, resource pack status, recipe book type, boss bar, scoreboard, equipment slot,
  direction, animate and swing, game event, entity metadata serializers, teleport/player info/input
  flags, position path, structure/test/command block and sign packets, seen advancements, team
  rules, container click mode, sound source, consume effects, resolvable profile) was checked with
  `javap` on the enum constants and static initialisers, no decompiling. No mismatch: the only 26.3
  shift (`CHANGE_DESTROY_DIRECTION` inserted at 1 in player action) was already handled. Equipment
  slots are sent as the enum ordinal, not `getIndex()`; metadata type 13 is an optional plain UUID.
  26.3 has no title-action enum (three separate packets). The orders are pinned by
  `enum_order_tests` in `crates/haste-protocol/src/packets/mod.rs`, each naming its jar class;
  registry-driven ids (sounds, particles, items, blocks, entity types, chat types) are not pinned
  because they come from the reports at runtime.
- **2026-10-13 Read the decompiled client instead of guessing.** Workers were filling gaps with
  minecraft.wiki numbers and guesses (seat offsets, falling-water height, physics constants marked
  UNVERIFIED). Rule 5 now allows reading the 26.3 client decompiled with Vineflower into
  `cache/source/26.3/client` (`scripts/decompile.sh`) for exact behaviour, while Haste's code is
  still written by us and never contains Mojang code; the recorder mod and `javap` stay the checks.
  Earlier entries saying "no decompiled source by rule" are superseded; their open questions can
  now be settled by reading the source.
- **2026-10-13 GUI blend modes come from vanilla's RenderPipeline.** Haste's tier-2
  `RenderPipelines` builds the GUI pipelines through vanilla's own builder (so their locations and
  blend functions are vanilla's), and `GfxImpl.blend` maps a pipeline's blend function to the
  three GUI blend codes (normal, multiply for the vignette, invert for the crosshair). The value
  types this needs are allow-listed in `loader.rs` (`com/mojang/renderpearl/api/{pipeline,vertex}`,
  `ShaderDefines`). A mod pipeline with a known blend function maps correctly; the texture-name
  rule is only the fallback for a null pipeline. Additive pipelines (`GUI_TEXT_HIGHLIGHT`) still
  draw normal until the renderer gets an additive GUI blend.
- **2026-10-18 Wrapped sprite UVs need continuous derivatives.** The greedy mesher stretches a
  quad's sprite-local UV over the merged rectangle and `chunk.wgsl` wraps it per block with
  `fract()`. Taking `dpdx`/`dpdy` of the wrapped atlas UV jumps a whole sprite at every block edge,
  `textureSampleGrad` then picks the coarsest mip and draws the atlas-average grey as a 1-2 px line
  on each edge (user bug "dashed lines on block edges"). The derivatives now come from the
  continuous sprite UV scaled into atlas space. Vanilla never merges faces, so it has no wraps.
- **2026-10-18 Face culling by neighbour coverage.** `mesher/culling.rs` culls a face when the
  neighbour's opaque, flush, axis-aligned model quads cover it: the visible result of vanilla
  `Block.shouldRenderFace`'s shape comparison (snow against snow, slab against slab), -28% quads
  on the bench world. Vanilla compares occlusion shapes, not models; the oracle dump now carries
  `occlusion` per state, and branch dump-occlusion-light moves the rule onto it.
  `skipRendering` (glass against glass, iron bars) is not implemented yet.
- **2026-10-18 The oracle dump carries occlusion and light per state.** `block_shapes.json` has
  `occlusion` (`getOcclusionShape`), `light_dampening` and `propagates_skylight_down`; the light
  engine and fluid occlusion should read them instead of name matching. `dump-server-oracles.sh`
  removes its old outputs first (it waits for the files to appear, so a rerun over an existing
  dump used to stop the server before it wrote anything).
- **2026-10-18 The embedded JVM must be warm before the render thread calls it.** The 100-600 ms
  frame hitches were the JVM's boot full GCs (code cache, metaspace thresholds) and first-frame
  class loading, landing in the render thread's GUI calls right after the HUD went live. A fresh
  HUD now renders warm-up frames on the game thread before it is published (`ui/hud.rs`
  `HudWarmup`), and the packets that arrive meanwhile are handed over before it goes live.
  `HASTE_JVM_GC_LOG=1` adds `-Xlog:gc`.
- **2026-10-18 Mesh upload memory.** One `queue.write_buffer` per mesh left ~3 GB of driver
  staging resident at render distance 32, and the unbounded mesh-result channel piled up the whole
  view distance's meshes behind the upload budget (glibc arenas never shrink after that). Uploads
  go through a `StagingBelt` with one encoder per frame, and the mesh backlog is capped at 3072
  sections (`terrain_feed.rs`): RSS 4.1-4.8 GB -> ~1.0 GB (vanilla ~2 GB). GPU memory is still
  ~2.8 GB because every section in view distance is meshed; vanilla meshes only visible sections
  (`LevelExtractor`), branch mesh-visible-sections.
- **2026-10-18 The "entities" GPU sub-pass was water.** The entities timestamp bracket stayed open
  across the translucent terrain draws, so translucent read 0 and entities 2.5 ms at rd 32
  (`timing.rs` `SubpassBrackets` now). Block entities also drew at any distance; vanilla's
  `BlockEntityRenderer.shouldRender` limit (64 blocks, piston 68, beacon horizontal render
  distance) is in `blockentity/view_distance.rs`.
- **2026-10-18 Screenshots are opaque.** The framebuffer alpha behind leaves and water is 0.5;
  vanilla `Screenshot.takeScreenshot` forces alpha 255 and so does Haste's readback now. Before,
  images looked washed out over a light background.
- **2026-10-18 Offscreen scripts and keys.** A `--script` client drops keys pressed before the
  JVM's KeyMappings exist; scripts that hold keys start with `wait_until settled`.
- **2026-10-18 GPU sub-pass timestamps are expensive on RADV.** Writing the 16 inside-pass
  timestamps every frame cost ~2.5 ms of GPU time at rd 32 (Radeon 860M); the sub-pass brackets
  are now sampled every 64th frame (`timing.rs` `SUBPASS_SAMPLE_FRAMES`), and a bracket counts as
  run only when its timestamp was written. The frame at rd 32 is vertex/draw-bound, not
  fragment-bound (13x fewer pixels cut GPU time by 23%): grouping draws by arena (2761 -> 649
  calls) and skipping back-facing face-direction groups per section (-30% quads) took the frame
  from 11.6 to 6.4 ms. Phase 2 note: a section layer's draws are per face direction
  (`LayerDraw.dir`, 6 = whole layer, used by translucent to keep emission order).
- **2026-10-18 Ctrl state in the JVM.** Vanilla polls the live keyboard for `hasControlDown()`
  when a click is consumed (`Minecraft.handleKeybinds` drop: one item or the stack). In Haste the
  JVM keeps the SDL keymods of the last key event, also with no screen open
  (`ClientImpl.ctrlDown()`); later "Ctrl held at the consume" reads use it.
- **2026-10-18 Reported "holes" in the held block were texels.** The grey spots on a held dirt
  block are dirt.png's own grey pebbles over grey ground; an offscreen test now checks that a
  held block draws as a solid cube (`haste-render/tests/held_block_offscreen.rs`).
- **2026-10-18 Vanilla reference runs have clouds off.** `scripts/vanilla-options.sh` sets
  `renderClouds:"false"` (clouds drift, so shots would not compare). Haste ignores the cloud
  option and always draws fancy clouds (brief cloud-option); benches against vanilla are not
  like-for-like until it honours `Options.cloudStatus`.
- **2026-10-18 Zero-copy light arrays pinned whole chunk packets.** Light nibble arrays were
  `Bytes` slices of the `level_chunk_with_light` packet, so every non-uniform section kept its
  whole packet alive (~125 MB of 139 MB received at rd 32). Light arrays are now owned
  (`Arc<[u8; 2048]>`, copied out of the packet) and uniform sky sections store one value, like
  vanilla's `DataLayer`: RSS -80 MB (-30%) at rd 32. Any other long-lived data sliced out of a
  packet with `Bytes::slice` has the same trap.
- **2026-10-18 Which block carries which block entity comes from the game.** The
  `block_entity_types.json` dump records `EntityBlock.newBlockEntity` per state. The old
  hand-written table had `moving_piston -> piston` (vanilla returns null there: the piston's block
  event creates that entity) and missed the copper golem statue variants.
- **2026-10-18 Face culling order in the mesher.** A cullface is drawn when the state's
  `skipRendering` rule keeps it (`mesher/skip.rs`, rules per vanilla class from the
  `block_classes.json` dump) and `Block.shouldRenderFace`'s occlusion-shape test shows it
  (`mesher/face_occlusion.rs`, from `block_shapes.json`); an old dump falls back to the model
  rule. Cross models (grass, flowers) light by their 26.3 `shade_direction_override` ("up"),
  not their side shades.
- **2026-10-18 Container clicks are sent inside the input event.** Vanilla's
  `MultiPlayerGameMode.handleContainerInput` sends at once; queuing to the next tick added
  50-100 ms before the crafting result appeared.
- **2026-10-18 Visible-only meshing: the trade the lead accepted.** Meshing only the sections in
  `visibleSections` (vanilla `LevelExtractor`), windowed bench at the reference spot, two runs
  each: rd 32 average frame 6.47/6.80 -> 6.20/6.26 ms, p99 9.5/13.0 -> 9.1/9.6 ms, but p99.9 and
  worst frames 12.7-26 -> 25-30 ms; GPU 2.6-2.8 GB -> 0.82 GB, RSS -50 to -110 MB. rd 12 average
  2.06/2.14 -> 2.19/2.24 ms, p99 6.1/7.8 -> 7.9/9.9 ms, GPU 0.82-0.95 -> 0.56 GB. The worst
  frames are the pop-in catch-up when sections become visible; turn prediction and spreading
  background work across frames are meant to win them back.
- **2026-10-18 Validate oracle values on an exact state.** `validate-dumps.py`'s `find` returns
  the first matching state, and for slabs and stairs that is the waterlogged one (light
  dampening 1, not 0). Pin every property that matters, `waterlogged` included.
- **2026-10-18 Item particle sprites for every item.** The sprite table for `minecraft:item`
  particles was built from the items whose default components are consumable, so a stack made
  consumable by its component patch, and every server item particle of a non-food item
  (snowball and egg hits, tool breaks), drew the missing sprite. It now covers the whole item
  registry: +35 ms once at join, same 1024x1024 atlas page.
- **2026-10-18 Another entity eating: particles yes, sound no.** The watcher's client runs
  `LivingEntity.updateUsingItem` for other entities from the synced living flags and spawns the
  crumbs itself, but `ClientLevel.playSeededSound` only plays for our own player: the eating
  sound of others arrives as the server's broadcast sound packet. The 16-particle finish burst
  (entity event 9) is sent only to the eater's own client.
