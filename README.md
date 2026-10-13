# Haste

A Minecraft: Java Edition 26.3 client written in Rust, designed to run Fabric mods through a
compatibility layer later (Phase 2). Phase 1, this repo's current state, is a fast vanilla client
with the mod architecture designed in. Licensed GPL-3.0-or-later.

## How far along it is

What a player needs to play Haste like the original game, without noticing a difference.
Fabric mod support (Phase 2) is not counted here.

**14 of 62 done, 26 partly done, 22 not started (about 44 %, counting partly done as half).**

✅ done: works in the game window today, checked against a real server.
🟡 partly done: some of it exists (in a library, or working without a window) but a player cannot use it fully yet.
⬜ not started.

### Launching and setup
- ✅ Install the game files (`haste setup`): downloads 26.3 from Mojang and runs the data generator; a second run is instant
- 🟡 Singleplayer: `haste singleplayer --world NAME` starts a local world and opens the game in it; no world list or create-world screen
- 🟡 Joining a server: `haste --server host:port` from the command line; no direct-connect screen
- 🟡 Server list: status ping works (`haste --ping --server host:port`); no server list screen
- 🟡 Microsoft account login: `haste login` / `haste logout` run the device-code flow and store the account, untested end to end (needs a Microsoft app id)
- 🟡 Saved options: stored in the config with vanilla defaults; the client uses render distance, FOV, mouse sensitivity and vsync, not yet GUI scale, volumes or keybinds
- ⬜ Title screen
- ⬜ Options screens (video, controls, sound, skin, chat, accessibility)
- ⬜ Keybind remapping (controls are fixed to the vanilla defaults)
- ⬜ Resource packs (selection screen, applying server resource packs)
- ⬜ Languages other than English

### Connecting to servers
- ✅ Offline-mode vanilla and Fabric servers: join and stay connected on normal terrain, every packet decodes
- ✅ Configuration phase, server transfers, cookies, resource pack answers
- 🟡 Online-mode servers: encryption and session join need the Microsoft login above
- 🟡 Item data: stacks with components decode and round-trip; names, enchantments, damage and the like are not decoded into typed values in Rust yet
- ⬜ Signed chat (required by online-mode servers)

### World
- ✅ Terrain: chunks, light, block and multi-block updates, chunk loading and unloading
- ✅ Biomes and their grass, foliage and water colours
- ✅ Dimensions: Overworld, Nether, End, respawn and dimension changes
- ✅ Day and night, weather, sun, moon, stars and clouds
- 🟡 Block entities (chests, signs, beds, banners, ...): stored with their chunks and kept up to date; not drawn yet

### Rendering
- ✅ Textured blocks and block models
- ✅ Water and lava with flow direction and corner heights
- ✅ First-person hand and held item
- ✅ Block selection outline and break cracks
- ✅ Fullscreen (F11)
- 🟡 Animated textures (water, lava, fire, portals): all frames load; they do not animate on screen yet
- 🟡 Smooth lighting and ambient occlusion: present, but leaves, bark and grass sides are about 1.35x brighter than vanilla
- 🟡 Fog: distance fog, not matched to vanilla
- 🟡 Mobs and players: drawn as cube models with default skins and name tags, not vanilla's models
- 🟡 Particles: particle textures load (114 particle types); no particles are drawn
- 🟡 Performance: about 140 FPS windowed at render distance 12 on the development laptop; not yet measured against vanilla and Sodium
- ⬜ Block entities drawn: chests, signs with text, beds, banners, heads, shulker boxes
- ⬜ Entity animation (walking, attacking, swimming)
- ⬜ Downloaded player skins and capes
- ⬜ Armour and held items on mobs and players
- ⬜ Dropped items, item frames and paintings drawn as themselves
- ⬜ Third-person view (F5)
- ⬜ Screen effects: damage tilt and red flash, underwater and fire overlays, nausea, vignette, pumpkin

### HUD and screens
These run as the game's own Java code inside Haste and work against a real server, but are not shown in the game window yet.
- 🟡 HUD: hotbar, health, hunger, armour, air, experience
- 🟡 Chat: reading, typing, command suggestions
- 🟡 Player list (Tab), boss bars, scoreboard sidebar, titles and action bar
- 🟡 Pause menu
- 🟡 Player inventory and crafting grid
- 🟡 Chests (clicks match vanilla byte for byte); furnace, crafting table, anvil, enchanting, brewing and trading are untested
- 🟡 Death screen and respawn
- ⬜ Creative inventory
- ⬜ Debug screen (F3)
- ⬜ Editing signs, books and command blocks
- ⬜ Advancements and statistics screens
- ⬜ Recipe book

### Playing
- ✅ Walking, sprinting, sneaking, jumping, swimming, creative flight and fall damage, accepted by the vanilla server without corrections
- ✅ Breaking and placing blocks, using items, attacking, pick block, hotbar (mouse wheel and 1-9)
- 🟡 Dropping items (Q, Ctrl+Q) and swapping to the off-hand (F): sent like vanilla; not yet checked by hand on a server
- 🟡 Game modes: survival and creative work; spectator and adventure are untested
- ⬜ Riding boats, minecarts, horses and pigs
- ⬜ Elytra flight
- ⬜ Screenshot key (F2)

### Sound
- 🟡 Sound effects: sounds.json and the variant choice are done; no audio output yet
- 🟡 3D sound: distance falloff and stereo panning are computed but not played
- 🟡 Music: choosing tracks and pauses by situation is done; nothing is played
- ⬜ Sounds of your own footsteps, block breaking and placing

## Try it

```bash
cargo run --release --bin haste -- setup                         # once: game files and data generator
cargo run --release --bin haste -- singleplayer --world test     # a local world, opens the game window in it
cargo run --release --bin haste -- --server host:port             # or join any offline-mode 26.3 server
```

Any offline-mode 26.3 server works with `--server` (`--username NAME` sets the player name).
Controls are vanilla's defaults: WASD, space, shift, ctrl, mouse to look, left click to break,
right click to place or use, 1-9 and the mouse wheel for the hotbar, Q / Ctrl+Q to drop, F to
swap hands, F11 for fullscreen. There is
no HUD in the window yet, so health and inventory are not visible.

## Build

Pinned stable Rust (`rust-toolchain.toml`), no system-wide installs. First run:

```bash
cargo run --release --bin haste -- setup        # downloads 26.3 from Mojang into the cache, runs the data generator
```

Everything downloaded or derived from Mojang's files lives in `cache/` (gitignored) or
`$HASTE_CACHE`. Nothing from it is ever committed.

See `TASKS.md` (board), `NOTES.md` (decisions) and `docs/HANDOFF.md` (state and known problems).
