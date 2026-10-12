# jvm-host: the HUD and screens as real vanilla Java

Worker: Opus. Worktree `$HASTE_WT/jvm-host`, branch `jvm-host`, test server port **25614**.
Read `docs/briefs/COMMON.md` first, then `crates/haste-jvm/NOTES.md`.

You own `crates/haste-jvm` and `crates/haste-client/src/ui.rs` (new; the client-app workstream
calls it from the game and render threads).

## Where it stands
Real vanilla `PauseScreen`, `ChatScreen` and `InventoryScreen` run in the embedded JVM through the
allow-listed tier-3 loader and generated proxies, one JNI call per frame, p50 34 µs. The wall is
`BuiltInRegistries`: tier-1 registry data is refused, so tags, item components and tooltips are
approximate.

## Why the HUD goes through Java
Fabric mods hook the in-game HUD (`Gui` and its chat, hotbar, boss bars, scoreboard, titles) more
than almost anything else. Running the real vanilla `Gui` class as tier 3, fed by proxies of our
Rust state, means Phase 2 mixins into it work unchanged. A Rust-drawn HUD would have to be thrown
away. Same pattern as the screens; the cost budget is the same one JNI call per frame for HUD and
screen together.

## Tasks, in order (commit after each)
1. **Registries into Java.** Bridge Rust's registry data (`haste_data::Registries`, the synced
   registries from the server, tags from `update_tags`) into vanilla's registry objects so that
   `BuiltInRegistries.ITEM`/`BLOCK`/... lookups, `Holder`s and tag checks work for tier-3 code.
   Decide how (populate real vanilla `MappedRegistry` instances at startup from our data via
   tier-3 code vs proxies) and write the decision with its Phase 2 consequences in your NOTES.md.
   Item stacks crossing into Java carry their real data components (decode the raw component
   bytes; Pumpkin has component codecs you may adapt), so tooltips, names and enchantment glint
   decisions are vanilla's own.
2. **The HUD**: run vanilla `Gui` (hotbar with items, selected slot, crosshair, health, hunger,
   armor, air, experience bar and level, held item name, action bar text, titles/subtitles,
   chat overlay with the vanilla fade, boss bars, sidebar scoreboard, player list on Tab,
   effects) every frame from `ui.rs` into the same `GuiFrame` as the open screen. Feed it from
   Rust state: the player's health/food/xp/abilities/inventory (from the packets; keep that
   state in Rust in one `GameState`-like struct the client owns, proxied into Java), chat lines
   from `system_chat`/`player_chat`/`disguised_chat`, boss events, scoreboard packets, titles,
   tab list. The F3 debug overlay can be a stub that shows FPS, position and chunk counts.
3. **Screens complete enough to play**: chat input with command suggestions
   (`command_suggestion` round trip; the `commands` graph from the server for client-side
   parsing via Brigadier, which is in the jar), the death screen with respawn
   (`client_command`), the inventory with real container clicks (`container_click` with 26.3's
   hashed item stacks; coordinate the hashing with the lead if `haste-protocol` lacks it), other
   containers opened by `open_screen` (chests, furnaces, crafting) as their vanilla screen
   classes, the disconnect/kick screen showing the reason.
4. **`ui.rs` API** for the client: key/char/mouse/scroll input routing (screen open: to the
   screen, else HUD keybinds such as T, /, E, Escape, F1, Tab, number keys, scroll for hotbar),
   `fn frame(&mut self, width, height, scale, partial) -> &GuiFrame`, and the outgoing packets
   that screens produce (chat, commands, clicks, close) as `ServerboundPlay` values drained by
   the client each tick. Write the API in your report so the lead puts it in INTERFACES.md.

## Done
- `cargo test -p haste-jvm` (including `--ignored` with the cache) and clippy clean; per-frame
  cost of HUD + open screen measured (p50/p99) and reported.
- A headless test that joins your server, receives chat and renders a HUD `GuiFrame` with the
  hotbar items and the chat line in it.
