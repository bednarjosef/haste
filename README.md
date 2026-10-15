# Haste

A Minecraft: Java Edition 26.3 client written in Rust, designed to run Fabric mods through a
compatibility layer later (Phase 2). Phase 1, this repo's current state, is a fast vanilla client
with the mod architecture designed in. Licensed GPL-3.0-or-later.

## How far along it is

What a player needs to play Haste like the original game, without noticing a difference.
Fabric mod support (Phase 2) is not counted here.

**43 of 62 done, 15 partly done, 4 not started (about 81 %, counting partly done as half).**

✅ done: works in the game window today, checked against a real server.
🟡 partly done: some of it exists (in a library, or working without a window) but a player cannot use it fully yet.
⬜ not started.

### Launching and setup
- ✅ Install the game files (`haste setup`): downloads 26.3 from Mojang and runs the data generator; a second run is instant
- ✅ Options screens: vanilla's options screen and every sub-screen open from the pause menu (checked by script)
- ✅ Saved options: vanilla's options.txt; changes in the options screens apply live (FOV, render distance, sensitivity, max FPS, vsync, GUI scale, volumes)
- ✅ Title screen: vanilla's title screen is shown when `haste` starts
- ✅ Singleplayer: the world list, Create New World and Save and Quit to Title, played through a local server (checked by script); `haste singleplayer --world NAME` also works
- ✅ Server list: vanilla's multiplayer screen with the saved servers, status pings, Add Server and Direct Connection
- ✅ Joining a server: from the server list or Direct Connection, or `haste --server host:port`
- ✅ Languages: every language of the game, switched live from Options > Language
- 🟡 Microsoft account login: `haste login` / `haste logout` run the device-code flow and store the account, untested end to end (needs a Microsoft app id)
- 🟡 Keybind remapping: the controls follow vanilla's key mappings (unit-tested); not yet checked by hand in the window
- ⬜ Resource packs (selection screen, applying server resource packs)

### Connecting to servers
- ✅ Offline-mode vanilla and Fabric servers: join and stay connected on normal terrain, every packet decodes
- ✅ Configuration phase, server transfers, cookies, resource pack answers
- 🟡 Online-mode servers: encryption and session join need the Microsoft login above
- 🟡 Signed chat (required by online-mode servers): message and command signing match vanilla's format (unit-tested); used once online mode works
- ✅ Item data: stacks decode and round-trip, and names, enchantments, damage, rarity and tool rules read into typed values; block breaking uses the held stack's `minecraft:tool` component and Efficiency, the attack strength delay its `minecraft:attack_speed` modifiers, and item use duration its `minecraft:consumable` (verified against vanilla's tick counts, `scripts/checks/item-data-check.sh`)

### World
- ✅ Terrain: chunks, light, block and multi-block updates, chunk loading and unloading
- ✅ Biomes and their grass, foliage and water colours
- ✅ Dimensions: Overworld, Nether, End, respawn and dimension changes
- ✅ Block entities (chests, signs, banners, ...): stored with their chunks and kept up to date
- 🟡 Day and night, sun, moon, stars and clouds work; rain, snow and thunder are not drawn yet

### Rendering
- ✅ Textured blocks and block models
- ✅ Water and lava with flow direction and corner heights
- ✅ Block selection outline and break cracks
- ✅ Fullscreen (F11)
- ✅ Animated textures (water, lava, fire, portals), prepared off the render thread
- ✅ Third-person view (F5)
- ✅ First-person hand and held item (hidden in third person)
- ✅ Dropped items, item frames and paintings drawn as themselves, with vanilla's bob, spin and stack copies
- ✅ Fog: vanilla's distance fog, colours, sky disc and sunrise, and the water, lava, powder snow, Nether, End, blindness and darkness fog
- ✅ Screen effects: damage tilt, red flash on hit entities, vignette, underwater, fire, in-wall, pumpkin, powder snow, spyglass, portal and nausea
- ✅ Entity animation: walking, head and body turning, attack swings, aggressive arms, death tilt, sleeping, swimming and mob-specific poses
- ✅ Armour, held items, heads and elytra on mobs and players
- 🟡 Smooth lighting and ambient occlusion: present, but leaves, bark and grass sides are about 1.35x brighter than vanilla
- 🟡 Mobs and players: vanilla's models for all 91 mob types with babies, variants, sizes and states, default skins, name tags and shadows; a few models still have misplaced parts (horse heads)
- 🟡 Particles: the server's particles are drawn with vanilla's per-type motion, size and lifetime; block-breaking and other client-side particles not yet
- 🟡 Performance: `haste --bench` measures about 900-980 FPS (1.0-1.1 ms per frame) at 1920x1080, render distance 12, on the development laptop's integrated GPU; not yet compared with vanilla and Sodium
- 🟡 Block entities drawn: chests (lid animation), ender chests, shulker boxes, signs with text, banners and heads (beds are ordinary block models); lecterns, bells, campfire items, spawners and the other block entities not yet
- ⬜ Downloaded player skins and capes

### HUD and screens
These run as the game's own Java code inside Haste, with keyboard and mouse input. Checked with input scripts (`haste --script`, `scripts/checks/`) against a vanilla server.
- ✅ HUD: hotbar, health, hunger, armour, air, experience
- ✅ Chat: typing and sending (the server relays the message)
- ✅ Pause menu
- ✅ Player list (Tab), boss bars, scoreboard sidebar, titles and action bar
- ✅ Player inventory and crafting grid, with the player model
- ✅ Death screen and respawn
- ✅ Debug screen (F3): vanilla's default entries with real values (checked against the server)
- ✅ Creative inventory: every tab's items and search
- ✅ Advancements and statistics screens, and every other pause-menu button
- 🟡 Chests (clicks match vanilla byte for byte); furnace, crafting table, anvil, enchanting, brewing and trading are untested
- ⬜ Editing signs, books and command blocks
- ⬜ Recipe book

### Playing
- ✅ Walking, sprinting, sneaking, jumping, swimming, creative flight and fall damage, accepted by the vanilla server without corrections
- ✅ Breaking and placing blocks, using items, attacking, pick block, hotbar (mouse wheel and 1-9)
- ✅ Dropping items (Q, Ctrl+Q) and swapping to the off-hand (F)
- ✅ Game modes: survival, creative, spectator (through walls) and adventure
- ✅ Screenshot key (F2): saves to the screenshots folder
- ✅ Elytra flight: gliding started from the game, firework boost (the server sees a normal glide)
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
