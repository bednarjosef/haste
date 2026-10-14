# Haste

A Minecraft: Java Edition 26.3 client written in Rust, designed to run Fabric mods through a
compatibility layer later (Phase 2). Phase 1, this repo's current state, is a fast vanilla client
with the mod architecture designed in. Licensed GPL-3.0-or-later.

## How far along it is

What a player needs to play Haste like the original game, without noticing a difference.
Fabric mod support (Phase 2) is not counted here.

**27 of 62 done, 25 partly done, 10 not started (about 64 %, counting partly done as half).**

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
- 🟡 Saved options: stored in the config with vanilla defaults; the client uses render distance, FOV, mouse sensitivity, vsync and GUI scale (Auto like vanilla), not yet volumes or keybinds
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
- 🟡 Block entities (chests, signs, beds, banners, ...): stored with their chunks and kept up to date; chests, ender/trapped chests and shulker boxes are drawn (with their lid animation), the rest not yet

### Rendering
- ✅ Textured blocks and block models
- ✅ Water and lava with flow direction and corner heights
- ✅ Block selection outline and break cracks
- ✅ Fullscreen (F11)
- ✅ Animated textures (water, lava, fire, portals), prepared off the render thread
- ✅ Third-person view (F5)
- ✅ First-person hand and held item (hidden in third person)
- 🟡 Smooth lighting and ambient occlusion: present, but leaves, bark and grass sides are about 1.35x brighter than vanilla
- 🟡 Fog: vanilla's distance fog, colours, sky disc and sunrise for the Overworld; water, lava, Nether and End fog not yet
- 🟡 Mobs and players: vanilla's models for 51 mob types and players, default skins and name tags; other mobs are boxes; no walking animation yet
- 🟡 Particles: the server's particles are drawn with vanilla's per-type motion, size and lifetime; block-breaking and other client-side particles not yet
- 🟡 Performance: `haste --bench` measures about 900-980 FPS (1.0-1.1 ms per frame) at 1920x1080, render distance 12, on the development laptop's integrated GPU; not yet compared with vanilla and Sodium
- 🟡 Screen effects: damage tilt, red flash on hit entities and the vignette work; underwater and fire overlays, nausea and pumpkin do not
- 🟡 Dropped items drawn as themselves with vanilla's bob, spin and stack copies; item frames and paintings not yet
- 🟡 Block entities drawn: chests, ender/trapped chests and shulker boxes (lid animation included); signs with text, beds, banners, heads not yet
- ⬜ Entity animation (walking, attacking, swimming)
- ⬜ Downloaded player skins and capes
- ⬜ Armour and held items on mobs and players

### HUD and screens
These run as the game's own Java code inside Haste, with keyboard and mouse input. Checked with input scripts (`haste --script`, `scripts/checks/`) against a vanilla server.
- ✅ HUD: hotbar, health, hunger, armour, air, experience
- ✅ Chat: typing and sending (the server relays the message)
- ✅ Pause menu
- ✅ Player list (Tab), boss bars, scoreboard sidebar, titles and action bar
- ✅ Player inventory and crafting grid, with the player model
- ✅ Death screen and respawn
- 🟡 Chests (clicks match vanilla byte for byte); furnace, crafting table, anvil, enchanting, brewing and trading are untested
- 🟡 Debug screen (F3): opens with F3 and draws; the values (FPS, position, chunks) read zero in screenshot runs
- 🟡 Creative inventory: opens centred with its tabs and items; search finds nothing yet
- ⬜ Editing signs, books and command blocks
- ⬜ Advancements and statistics screens
- ⬜ Recipe book

### Playing
- ✅ Walking, sprinting, sneaking, jumping, swimming, creative flight and fall damage, accepted by the vanilla server without corrections
- ✅ Breaking and placing blocks, using items, attacking, pick block, hotbar (mouse wheel and 1-9)
- ✅ Dropping items (Q, Ctrl+Q) and swapping to the off-hand (F)
- ✅ Game modes: survival, creative, spectator (through walls) and adventure
- ✅ Screenshot key (F2): saves to the screenshots folder
- 🟡 Elytra flight: gliding physics ported from vanilla (tested); not yet started from the game
- 🟡 Riding: sitting in boats, minecarts, horses and pigs at vanilla's seat positions; steering vehicles not yet

### Sound
- ✅ 3D sound: distance falloff and stereo panning from the player's eye, entity sounds follow the entity
- ✅ Sounds of your own footsteps, mining, breaking, placing and falling, with vanilla's step distance
- 🟡 Sound effects: the server's sounds play in the window (block, entity and stopped sounds, per-category volumes); not yet checked by ear
- 🟡 Music: plays by situation (game, creative, underwater) with vanilla's pauses; not yet checked by ear

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
