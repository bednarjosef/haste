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
| L1 | Run the JVM UI without a joined world: a "menu mode" window that shows a vanilla Screen before any server | the window shows a vanilla screen with no server | GLM | queued |
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
| W4 | Recorder mod dumps vanilla entity model geometry (ModelPart cubes, pivots, UVs) for every entity layer into `cache/oracle/26.3/entity_models.json` (runs the real game; allowed anytime) | the dump exists and covers all living entities | GLM | queued |
| W5 | haste-entity loads the dumped models instead of cube placeholders | screenshot of a pig, zombie, villager | GLM | queued (after W4) |
| W6 | Walk/attack/swim animation from the oracle: record limb angles over time in the recorder mod, fit and replay | side-by-side screenshots | GLM | queued (after W5) |
| W7 | Armour and held items on mobs and players | screenshot | GLM | queued (after W5) |
| W8 | Player skins and capes downloaded from the profile textures (offline: default skins) | screenshot with a real skin | GLM | queued |
| W9 | Block entity renderers: chests (opening lid), signs with text, beds, banners, heads, shulker boxes, from the recorder's model dump (W4 covers block entity models too) | screenshot of each | GLM | queued (after W4) |
| W10 | Item frames and paintings | screenshot | GLM | queued (after W2) |
| W11 | Screen overlays: underwater, fire, pumpkin, nausea | screenshots | GLM | queued |
| W12 | Fog matched to vanilla (compare with scripts/compare-vanilla.sh) | difference images | GLM | queued |
| W13 | Smooth lighting brightness (leaves/bark/grass sides ~1.35x too bright) | comparison image | GLM | queued |
| W14 | Performance compared with vanilla and Sodium (`--bench` vs the vanilla client's F3 frame times on the same path) | numbers in the README | GLM | queued |

## P. Playing and protocol

| # | Task | Done means | Worker | Status |
|---|---|---|---|---|
| P1 | Riding: boats, minecarts, horses, pigs (vehicle movement packets, camera) | scripted ride on the server | GLM | queued |
| P2 | Elytra flight (physics + packets) | scripted glide | GLM | queued |
| P3 | Spectator and adventure mode checked | scripted check | GLM | queued (after S1) |
| P4 | Typed item data components in Rust (names, enchantments, damage, ...) for game logic | tests on encoded samples | OpenCode | queued |
| P5 | Signed chat (chat session, message signing with the player certificates) | unit tests; live test needs online mode | OpenCode | queued |
| P6 | Online mode end to end | join an online server | lead + user | blocked: Microsoft app client id (later) |

## A. Sound

| # | Task | Done means | Worker | Status |
|---|---|---|---|---|
| A1 | Server sounds, listener, music and volumes hooked into the client | heard in the window; packet counts in the log | GLM | running sound-in-game |
| A2 | Own footsteps, block break/place sounds: block sound types dumped by the recorder mod, played on the client's own actions | log of played events | GLM | queued |

## Done since the roadmap started

Recorded by PR number in the README's history; see `git log`.
