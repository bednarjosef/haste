# Roadmap: every unchecked README item, as small tasks

This is the task queue. Every task comes from here, every task closes (part of) a README
checklist item, and a box moves to ✅ only when the feature is checked on screen (a screenshot or
a scripted input run, see S1) in the PR that finishes it. Tasks are small enough for one free
worker (GLM or OpenCode). **OpenCode** tasks must not need anyone to read `cache/` (tests may use
it); **GLM** gets anything that touches game files, screenshots or the server.

Status: `queued`, `running <worker>`, `review`, `done #PR`, `blocked: <what>`.
Order: top to bottom by how many boxes a task unlocks.

## S. Verification (unlocks ~12 boxes)

| # | Task | Done means | Worker | Status |
|---|---|---|---|---|
| S1 | `haste --script FILE`: offscreen client that replays key presses, typed text, mouse moves and clicks from a script and takes screenshots at marked steps (needs U1) | a script opens chat, types, sends; screenshots show it; a test parses scripts | GLM | queued (after U1) |
| S2 | Scripted checks for the 🟡 HUD/screen items with S1: chat, Tab list, pause menu, inventory, death + respawn, drop/swap, F2, game modes | one script + screenshot per item, README boxes moved | GLM | queued (after S1) |

## U. Input and screens

| # | Task | Done means | Worker | Status |
|---|---|---|---|---|
| U1 | Keyboard/mouse routed to the vanilla screens (chat, inventory, pause), cursor release | ui-input branch reviewed and merged | GLM | running ui-input |
| U2 | F3 debug screen: `Ui::set_debug` with FPS, position, chunks, entities; F3 key | screenshot with F3 open | GLM | queued |
| U3 | Creative inventory (vanilla Java screen) opens with E in creative; item tabs work | scripted screenshot | GLM | queued (after U1) |
| U4 | Sign editing, book editing, command block screens open from server packets | scripted screenshot of the sign editor | GLM | queued (after U1) |
| U5 | Advancements (L) and statistics screens | scripted screenshots | GLM | queued (after U1) |
| U6 | Recipe book in the inventory | scripted screenshot | GLM | queued (after U3) |
| U7 | Containers: furnace, crafting table, anvil, enchanting, brewing, trading checked with scripts | one script each | GLM | queued (after S1) |

## L. Launcher and menus (vanilla Java screens outside a world)

| # | Task | Done means | Worker | Status |
|---|---|---|---|---|
| L1 | Plan for menu mode (JVM UI without a joined world) | plan with steps L1a-L1e | OpenCode | done (menu-mode-research) |
| L1a | Menu-mode window: an `App` without a `Session`, JVM started at launch, `Ui::new` + text metrics, 20 Hz `ui.tick`; a headless `--menu` mode printing the screen class and texts | `--menu` run with no server prints TitleScreen texts | GLM | queued (after U1) |
| L1b | `ScreenKind::{Title, Options, Multiplayer}` in haste-jvm + `Bridge.open` cases; jvm tests for each screen's texts | jvm tests pass, Unimplemented log empty | OpenCode | running title-screens |
| L1c | Load the jar's synced registries (`load_jar_registries`) in menu mode before opening screens | test opens Title/Options after it | OpenCode | queued (after L1b) |
| L1d | Quit/disconnect from Java to Rust: poll `take_disconnect_request`, a quit flag for Title's Quit Game | jvm test clicks Quit, flag set and cleared | GLM | queued (after L1a) |
| L1e | Server list classes allowed by name in the class loader; a seam for "join this server" into `Session::join` + `Ui::joined` | Rust test of the seam; scripted join after S1 | GLM | queued (after L1d) |
| L2 | Title screen (vanilla TitleScreen) as the start screen of `haste` with no arguments | screenshot | GLM | queued (after L1) |
| L3 | Multiplayer server list + direct connect + add server (vanilla screens), saved servers, status pings from haste-protocol | scripted join from the list | GLM | queued (after L2) |
| L4 | Singleplayer world list + create world, backed by `start_singleplayer` | scripted create + join | GLM | queued (after L2) |
| L5 | Options screens (video, controls, sound, skin, chat, accessibility) reading/writing haste-setup `Options` | change FOV in the screen, it applies | GLM | queued (after L2) |
| L6 | Keybind remapping: input uses `Options.keys` (vanilla key names) instead of fixed keys | test + scripted check | OpenCode | queued |
| L7 | GUI scale and per-category volumes from `Options` | test + screenshot | OpenCode (scale) / GLM (volumes) | queued |
| L8 | Languages: load the selected language's lang file for Java screens | screenshot in another language | GLM | queued (after L5) |
| L9 | Resource packs: apply a server resource pack's assets over the jar's (textures, models, sounds) | screenshot with a test pack | GLM | queued |

## W. World and rendering

| # | Task | Done means | Worker | Status |
|---|---|---|---|---|
| W1 | First-person hand and held item turned on | screenshot | GLM | running first-person-hand |
| W2 | Dropped items drawn as items | screenshot | GLM | running item-entities |
| W3 | Particles from the server, basic motion | screenshot | GLM | running particles-basic |
| W4 | Recorder mod dumps vanilla entity model geometry (ModelPart cubes, pivots, UVs) for every entity layer into `cache/oracle/26.3/entity_models.json` (runs the real game; allowed anytime) | the dump exists and covers all living entities | GLM | running model-dump |
| W5 | haste-entity loads the dumped models instead of cube placeholders | screenshot of a pig, zombie, villager | GLM | queued (after W4) |
| W6 | Walk/attack/swim animation from the oracle: record limb angles over time in the recorder mod, fit and replay | side-by-side screenshots | GLM | queued (after W5) |
| W7 | Armour and held items on mobs and players | screenshot | GLM | queued (after W5) |
| W8 | Player skins and capes downloaded from the profile textures (offline: default skins) | screenshot with a real skin | GLM | queued |
| W9 | Block entity renderers: chests (opening lid), signs with text, beds, banners, heads, shulker boxes, from the recorder's model dump (W4 covers block entity models too) | screenshot of each | GLM | queued (after W4) |
| W10 | Item frames and paintings | screenshot | GLM | queued (after W2) |
| W11 | Screen overlays: underwater, fire, pumpkin, nausea | screenshots | GLM | queued |
| W12 | Fog matched to vanilla, clear air (compare with scripts/compare-vanilla.sh) | difference images | GLM | running fog-vanilla |
| W12b | Water, lava, powder snow, blindness and darkness fog | screenshots | GLM | queued (after W12) |
| W13 | Smooth lighting brightness (leaves/bark/grass sides ~1.35x too bright) | comparison image | GLM | queued |
| W15 | Cull spike when ~100 sections become visible at once (3.5-3.8 ms in cull, from the bench's slowest frames) | bench worst frame without the spike | OpenCode | running cull-spikes |
| W16 | HUD blend modes chosen by vanilla's RenderPipeline instead of texture names (mod-friendly) | screenshot unchanged, test | GLM | queued |
| W14 | Performance compared with vanilla and Sodium (`--bench` vs the vanilla client's F3 frame times on the same path) | numbers in the README | GLM | queued |

## P. Playing and protocol

| # | Task | Done means | Worker | Status |
|---|---|---|---|---|
| P1 | Riding as a passenger: set_passengers, seat position, camera, rotation + input only, sneak to dismount | scripted ride on the server | GLM | running riding-passenger |
| P1b | Controlling vehicles: boat paddling, horse/pig/strider steering, `move_vehicle` | scripted ride on the server | GLM | queued (after P1) |
| P2 | Elytra flight (physics + packets) | scripted glide | GLM | queued |
| P3 | Spectator and adventure mode checked | scripted check | GLM | queued (after S1) |
| P4 | Typed item data components in Rust (names, enchantments, damage, ...) for game logic | tests on encoded samples | OpenCode | running item-components |
| P5 | Signed chat (chat session, message signing with the player certificates) | unit tests; live test needs online mode | OpenCode | running signed-chat |
| P6 | Online mode end to end | join an online server | lead + user | blocked: Microsoft app client id (later) |

## A. Sound

| # | Task | Done means | Worker | Status |
|---|---|---|---|---|
| A1 | Server sounds, listener, music and volumes hooked into the client | heard in the window; packet counts in the log | GLM | done #67 (to hear in the window) |
| A1b | haste-audio: per-source `playing()` (music must not count other sounds as the track) and stop-by-event-name | tests | OpenCode | queued |
| A2 | Own footsteps, block break/place sounds: block sound types dumped by the recorder mod, played on the client's own actions | log of played events | GLM | queued |

## F. Performance and footprint

From the 2026-10-20 measurements (`NOTES.md`: the JVM's cost, slow frames). Each task reports
numbers before and after (RSS, boot time, frame p50/p99/worst at rd 12 and 32).

| # | Task | Done means | Worker | Status |
|---|---|---|---|---|
| F1 | JVM class data sharing: an AppCDS archive of the vanilla and bridge classes, built by `haste setup` into `cache/` and used at JVM start | boot time (6.6 s, 4.8 s of it vanilla's bootstrap) and metaspace (68 MiB) before/after | lead | queued |
| F2 | JVM native memory measured (`-XX:NativeMemoryTracking=summary`): the JVM's real share of RSS beyond heap, metaspace and code cache | a NOTES entry with the breakdown | lead | queued |
| F3 | JVM heap limits trimmed (152 MiB committed, about 104 used at rd 12) without longer GC pauses (now at most about 2.5 ms) | settled RSS and GC pauses before/after | lead | queued (after F2) |
| F4 | JVM boot in the background while the window and the world load, so the HUD is ready sooner | time from launch to HUD ready before/after | lead | queued (after F1) |
| F5 | AFK frame throttle: vanilla's `inactivityFpsLimit` option and `FramerateLimitTracker`'s AFK limits (30 after 1 min, 10 after 10 min without input) | test, option saved in options.txt | lead | queued |
| F6 | The Max Framerate slider changes the frame limiter live (the live options carry `max_fps`) | scripted run: slider to 30, measured 30 FPS | lead | queued |
| F7 | Trace the 16.7 ms p99 at rd 32 in performance mode (`--bench-trace`) | the cause named in NOTES | lead | queued |
| F8 | GPU occlusion test with tighter boxes per facing group instead of the section's | opaque draws and GPU time at the ground and high spots before/after | lead | queued |
| F9 | GPU occlusion test once per section instead of once per draw, and no pyramid rebuild while the camera and the depth are unchanged | GPU time of the cull passes before/after | lead | queued |
| F10 | `SectionDraw` without per-draw bounds (the section's, looked up by slot) | draw-list CPU time at rd 32 before/after (0.58 ms) | lead | queued |
| F11 | A Rust UI (HUD, chat, screens) for a no-mods mode without a JVM | blocked: architecture decision (tier 3 stays vanilla Java, `AGENTS.md`); see NOTES 2026-10-20 | lead | blocked: decision |

## R. Reviews against vanilla (decompiled source)

| # | Task | Done means | Worker | Status |
|---|---|---|---|---|
| R1 | Movement differences (physics, player tick, packets) vs vanilla | ranked list; each item becomes a fix task here | GLM | running review-movement |
| R2 | World-rendering differences (lighting, AO, tint, sky, clouds, camera) vs vanilla | ranked list; each item becomes a fix task | GLM | running review-visual |
| R3 | Camera, entity rendering and GUI placement differences vs vanilla | ranked list | GLM | queued |
| R4 | Built-for-mods audit of the code so far | findings become tasks | OpenCode | running mod-audit |
| R5 | Protocol enum orders vs the 26.3 jar | no mismatch; tests pin them | GLM | done #65 |

## Done since the roadmap started

Recorded by PR number in the README's history; see `git log`.
