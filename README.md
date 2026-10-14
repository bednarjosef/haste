# Haste

A Minecraft: Java Edition 26.3 client written in Rust, designed to run Fabric mods through a
compatibility layer later (Phase 2). Phase 1, this repo's current state, is a fast vanilla client
with the mod architecture designed in. Licensed GPL-3.0-or-later.

## How far along it is

What a player needs to play Haste like the original game, without noticing a difference.
Fabric mod support (Phase 2) is not counted here.

**17 of 62 done, 30 partly done, 15 not started (about 52 %, counting partly done as half).**

✅ done: works in the game window today, checked against a real server.
🟡 partly done: some of it exists (in a library, or working without a window) but a player cannot use it fully yet.
⬜ not started.

### Launching and setup
- ✅ Install the game files (`haste setup`): downloads 26.3 from Mojang and runs the data generator; a second run is instant
- 🟡 Singleplayer: `haste singleplayer --world NAME` starts a local world and opens the game in it; no world list or create-world screen
- 🟡 Joining a server: `haste --server host:port` from the command line; no direct-connect screen
- 🟡 Server list: status ping works (`haste --ping --server host:port`); no server list screen
- 🟡 Microsoft account login: `haste login` / `haste logout` run the device-code flow and store the account, untested end to end (needs a Microsoft app id)
- 🟡 Title screen: vanilla's title screen opens in the embedded UI (tested); not yet shown when `haste` starts
- 🟡 Options screens: vanilla's options screen opens in the embedded UI (tested); not reachable from the game yet, and the sub-screens are untested
- 🟡 Saved options: stored in the config with vanilla defaults; the client uses render distance, FOV, mouse sensitivity and vsync, not yet GUI scale, volumes or keybinds
- ⬜ Keybind remapping (controls are fixed to the vanilla defaults)
- ⬜ Resource packs (selection screen, applying server resource packs)
- ⬜ Languages other than English

### Connecting to servers
- ✅ Offline-mode vanilla and Fabric servers: join and stay connected on normal terrain, every packet decodes
- ✅ Configuration phase, server transfers, cookies, resource pack answers
- 🟡 Online-mode servers: encryption and session join need the Microsoft login above
- 🟡 Signed chat (required by online-mode servers): message and command signing match vanilla's format (unit-tested); used once online mode works
- 🟡 Item data: stacks decode and round-trip, and names, enchantments, damage, rarity and tool rules read into typed values; the game logic does not use them yet

### World
- ✅ Terrain: chunks, light, block and multi-block updates, chunk loading and unloading
- ✅ Biomes and their grass, foliage and water colours
- ✅ Dimensions: Overworld, Nether, End, respawn and dimension changes
- ✅ Day and night, weather, sun, moon, stars and clouds
- 🟡 Block entities (chests, signs, beds, banners, ...): stored with their chunks and kept up to date; not drawn yet

### Rendering
- ✅ Textured blocks and block models
- ✅ Water and lava with flow direction and corner heights
- ✅ Block selection outline and break cracks
- ✅ Fullscreen (F11)
- ✅ Animated textures (water, lava, fire, portals), prepared off the render thread
- ✅ Third-person view (F5)
- ✅ First-person hand and held item (hidden in third person)
- 🟡 Smooth lighting and ambient occlusion: present, but leaves, bark and grass sides are about 1.35x brighter than vanilla
- 🟡 Fog: distance fog, not matched to vanilla
- 🟡 Mobs and players: drawn as cube models with default skins and name tags, not vanilla's models
- 🟡 Particles: particle textures load (114 particle types); no particles are drawn
- 🟡 Performance: `haste --bench` measures about 900-980 FPS (1.0-1.1 ms per frame) at 1920x1080, render distance 12, on the development laptop's integrated GPU; not yet compared with vanilla and Sodium
- 🟡 Screen effects: damage tilt, red flash on hit entities and the vignette work; underwater and fire overlays, nausea and pumpkin do not
- ⬜ Block entities drawn: chests, signs with text, beds, banners, heads, shulker boxes
- ⬜ Entity animation (walking, attacking, swimming)
- ⬜ Downloaded player skins and capes
- ⬜ Armour and held items on mobs and players
- ⬜ Dropped items, item frames and paintings drawn as themselves

### HUD and screens
These run as the game's own Java code inside Haste. The HUD is drawn in the game window; keyboard and mouse input to the screens is not connected yet.
- ✅ HUD: hotbar, health, hunger, armour, air, experience
- 🟡 Chat: messages are shown, and keyboard input now reaches the chat box; typing and sending not yet checked
- 🟡 Player list (Tab), boss bars, scoreboard sidebar, titles and action bar
- 🟡 Pause menu
- 🟡 Player inventory and crafting grid
- 🟡 Chests (clicks match vanilla byte for byte); furnace, crafting table, anvil, enchanting, brewing and trading are untested
- 🟡 Death screen and respawn
- 🟡 Debug screen (F3): opens with F3 and shows FPS, position, chunks, sections and entities; not yet checked on screen
- ⬜ Creative inventory
- ⬜ Editing signs, books and command blocks
- ⬜ Advancements and statistics screens
- ⬜ Recipe book

### Playing
- ✅ Walking, sprinting, sneaking, jumping, swimming, creative flight and fall damage, accepted by the vanilla server without corrections
- ✅ Breaking and placing blocks, using items, attacking, pick block, hotbar (mouse wheel and 1-9)
- 🟡 Dropping items (Q, Ctrl+Q) and swapping to the off-hand (F): sent like vanilla; not yet checked by hand on a server
- 🟡 Game modes: survival and creative work; spectator and adventure are untested
- 🟡 Screenshot key (F2): saves to the screenshots folder; not yet checked by hand in the window
- ⬜ Riding boats, minecarts, horses and pigs
- ⬜ Elytra flight

### Sound
- 🟡 Sound effects: the server's sounds play in the window (block, entity and stopped sounds, per-category volumes); not yet checked by ear
- 🟡 3D sound: distance falloff and stereo panning from the player's eye, entity sounds follow the entity; not yet checked by ear
- 🟡 Music: plays by situation (game, creative, underwater) with vanilla's pauses; not yet checked by ear
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
swap hands, F11 for fullscreen. The HUD shows
health, hunger and the hotbar; F5 switches to third person.

## Build

Pinned stable Rust (`rust-toolchain.toml`), no system-wide installs. First run:

```bash
cargo run --release --bin haste -- setup        # downloads 26.3 from Mojang into the cache, runs the data generator
```

Everything downloaded or derived from Mojang's files lives in `cache/` (gitignored) or
`$HASTE_CACHE`. Nothing from it is ever committed.

See `TASKS.md` (board), `NOTES.md` (decisions) and `docs/HANDOFF.md` (state and known problems).
