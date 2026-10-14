# haste-jvm notes (jvm-host)

## Layout
- Rust: `src/host.rs` (`JvmHost`, `JvmConfig`), `src/loader.rs` (allow/deny lists, class log), `src/proxies.rs`
  (`GameState`: the Rust-owned state proxies read), `src/gui.rs` (`GuiPipeline`, frame decoding), `src/keys.rs`
  (26.3 key numbering), `src/natives/` (`haste.rt.Native`), `src/error.rs`.
- Java: `java/haste/loader` (Boot, HasteClassLoader), `java/haste/proxy` (Gen), `java/haste/rt` (GuiBatch,
  Native, Unimplemented, Errors) on the JVM class path; `java/haste/gui` (Bridge and the proxy impl classes,
  compiled *against* the client jar) served by HasteClassLoader. `build-java.sh boot|game` (plain javac) writes
  `build/` (gitignored); `JvmHost::start` runs it when the sources changed (stamps per target).
- `JvmHost::start`: libjvm from `cfg.java_home` via `jni` 0.22's invocation API; registers natives; `Boot.boot`
  generates proxies into `cache/proxies/26.3/` when `proxies.conf`, the impl classes, the jar or the policy changed
  (`.stamp`, `GENERATED.txt` lists every generated body), creates the loader and `haste.gui.Bridge`.
- One JVM per process (JNI cannot make a second): `JvmHost` is a cheap clone of a process-wide host. Java-side
  GUI state is single-threaded; `GuiPipeline` serialises calls.
- JVM option `-XX:-BytecodeVerificationRemote` (diagnostic): with verification on, the verifier loads every type a
  method could assign (every screen PauseScreen can open, their listeners...) before anything runs and the first
  refused type fails the whole class. Lazy linking means only code that runs loads classes, so the class log is
  the real to-do list. The verified-off classes are the player's own vanilla jar and our generated proxies.
  Revisit for Phase 2 (mod classes).

## Registries (decision, session 2)
**Vanilla's own bootstrap runs in tier 3, and the server's registries are built by vanilla's own collector.**
- `Bridge.create` calls `SharedConstants.tryDetectVersion()` and `Bootstrap.bootStrap()` before any proxy exists:
  every built-in registry (blocks, items, entity types, data component types, menus, sound events, argument types,
  ...) is filled by the player's jar, exactly as in the game. Rust's numbering comes from the data generator run on
  the same jar, so ids agree by construction; `builtin_registries_match_the_data_generator` checks all 95 registries
  and 7053 entries name by name.
- Synchronised registries: Rust re-encodes the configuration phase's `registry_data` and `update_tags` packets
  (`GuiPipeline::registry_data`, `tags`) and `finish_configuration` runs vanilla's `RegistryDataCollector`
  (`collectGameRegistries`, the client's own path: entries decoded with their codecs, tags bound, item components
  initialised). Haste announces no known packs, so every entry carries its NBT. Play-phase `update_tags` go through
  the same collector (tags only). `Registries.access()` is what `ClientPacketListener.registryAccess`,
  `Level.registryAccess` and every packet/stack codec use.
- Without a server (tests, offline): `GuiPipeline::load_jar_registries` announces every entry of the jar's vanilla
  data pack without data, lets vanilla's loader read them from the jar (`JarDataPack`, like a known pack), and
  resolves every registry's tags from the jar's tag files with vanilla's `TagLoader` into the network form.
  Entry ids there are the sorted file order (a server may number them differently).
- Rejected: proxies or name-only holders (the previous `ItemHolder`/`EmptyRegistries`): tags, item components,
  tooltips, enchantment glint, argument types and chat types all need the real objects; writing them from Rust data
  would be re-implementing vanilla.
- Cost: the bootstrap loads ~3.6k classes from the jar and computes every block state's shape cache (vanilla does
  the same at startup): 9-22 s on this machine under load 15 (no CDS for custom loaders). `JvmHost::start` should run
  on its own thread at client start, in parallel with setup/connecting. Not optimised yet (AOT cache does not cover
  custom class loaders in JDK 25; a snapshot of the bootstrapped heap is the Phase 2 idea).
- **Phase 2 consequences:** mods register blocks/items/entities into the real `BuiltInRegistries` during their
  init, between bootstrap and freeze, exactly like Fabric (Fabric's registry sync then needs Rust to learn the
  extra ids: the data generator numbering stops being enough, Rust must take ids from Java for modded entries).
  Mixins into `Item`, `Block`, `ItemStack`, `DataComponents`, tooltips etc. apply to real vanilla classes.

## HUD (decision, session 2)
The HUD is vanilla's `Hud` running as tier 3 every frame (`Bridge.render`: HUD, then the open screen, one JNI call).
`Hud`, `ChatComponent`, `ChatListener`, `Options`, `OptionInstance`, `KeyMapping`, `ToastManager`, `Tutorial`,
`Scoreboard`, boss/tab overlays are real vanilla classes. The player proxy keeps vanilla's method bodies
(`keepall`) over real vanilla parts on it: synched entity data (built by vanilla's `defineSynchedData` chain plus
`Entity`'s base accessors, updated from forwarded `set_entity_data`), `AttributeMap`, effects map, `FoodData`,
`Abilities`, inventory. Rust's primitive state (health, food, xp, abilities, game mode, selected slot, operator level,
position, time) is copied in once per frame and tick by one native call (`Mirror`). Impls (`PlayerImpl`, `LevelImpl`,
`CameraImpl`, ...) win over kept bodies for what Rust owns.
- Packets: Rust re-encodes the packets whose state is tier 3 (list in `haste-client/src/ui.rs` `FORWARDED`); Java
  decodes them with vanilla's play codec (`Packets`) and `Handlers` feeds the vanilla component that owns the state.
  Serverbound packets vanilla code sends go through vanilla's serverbound codec to Rust (`GameState::outgoing`), so
  container clicks (vanilla prediction + hashed stacks, verified: the server sends no corrections), container close,
  command suggestions, respawn are byte-exact by construction. Hash generator: CRC32C `HashOps` over the registries.
- Approximate (from observation, not vanilla code): player chat is shown through the disguised-chat path (no
  signature verification, no secure indicator logic, filter masks ignored); `delete_chat` ignored; light is full
  everywhere (`LevelLightEngine` proxy), sky darkening 0; world border default; attack cooldown 1, no item use,
  no vehicle; eyes at standing height; difficulty NORMAL; F3 is a stub; restricted command nodes need level 2.
- Dead ends: vanilla code inside `CompletableFuture`s swallows refused-class errors (the chat suggestion list failed
  silently on `Rect2i`): server tests now assert the class log has no new refusals.
- Phase 2: mixins into `Hud`, `ChatComponent`, `Options`, `KeyMapping`, the menus and screens apply to vanilla code
  that really runs; mixins into `Entity`/`Player` methods apply to the kept bodies (the generator would have to merge
  them into the proxy, `Gen.merge`).

## Inventory (decision, session 2)
The player's `Inventory` and its `ItemStack`s are real vanilla objects in Java (`Proxies.inventory`, on the player
proxy's `inventory`/`equipment` fields), not proxies. Vanilla's menus apply the server's container packets and
predict clicks by mutating stacks in place (split, grow, shrink), and `container_click` must carry vanilla's hashed
stacks of the predicted slots; all of that is vanilla code over real stacks. Rust keeps a mirror
(`GameState::inventory`/`carried`, `ItemStackData` with the raw component patch) that `InventorySync` updates after
every bridge call that can change it (packets, input, ticks): changed slots are encoded with vanilla's
`ItemStack.OPTIONAL_STREAM_CODEC` and decoded in Rust without parsing components. Cost when nothing changed: 41
`ItemStack.matches` per call. Phase 2: mods that keep state on stacks see vanilla's object lifetimes.

## Class loader policy (`src/loader.rs`)
Session 2: `net/minecraft/` is allowed as a whole; `net/minecraft/client/`, `net/minecraft/server/` and `com/mojang/`
are denied with client GUI/input/tutorial/state packages and named value types re-allowed (rationale in the list
comments). Tier-1 objects are proxies (which win). The session-1 lists below are history.
Order: JDK + boot classes from the parent; `haste.*` from `build/game`; a generated proxy if one exists (proxies
win); vanilla classes from the client jar if the policy permits; anything else from the vanilla library jars
(`cache/libraries`, or `cache/server-spike/libraries` until setup's client set exists; Fabric loader/mixin and ASM
excluded). Non-class resources (`version.json`, `assets/minecraft/lang/en_us.json`) come from the client jar.
Most specific pattern wins (class > longer package > shorter; deny wins ties), so value types inside denied
packages can be allowed by name. Every decision is logged (`JvmHost::class_log`).

Allowed packages: `core`, `resources`, `network/chat`, `network/codec`, `network/protocol` (packets are value
types), `nbt`, `tags`, `commands` (brigadier parsing for chat), `util`, `locale` (Language from the jar),
`world/inventory` (menus and slots run for real over the Inventory proxy), `client/gui/{components,screens,
navigation,narration,layouts}`, `client/input`, `client/renderer/entity/state` (plain render-state data),
`com/mojang/math`, `com/mojang/blaze3d/platform/cursor` (CursorTypes constants; CursorType itself is a proxy).
Allowed classes (each a value type or helper the screens reached): root helpers (`ChatFormatting`,
`IdentifierException`, `SharedConstants`, `DetectedVersion`, `CrashReport*`, `TracingExecutor`, ...), network
listener interfaces and buffers, `world/item/TooltipFlag`, `world/item/Item$TooltipContext`,
`world/level/storage/DataVersion`, `server/packs/metadata/pack/PackFormat`, `server/ServerLinks`,
`server/permissions/`, `client/multiplayer/chat/{ChatAbilities,ChatRestriction,GuiMessage*}`,
`client/multiplayer/ClientSuggestionProvider` (real, over the connection proxy), `world/{Nameable,Container,
Clearable}`, `world/entity/{EquipmentSlot,EquipmentSlotGroup,SlotProvider,SlotAccess,ContainerUser}`,
`world/entity/player/ChatVisiblity`, `client/{StringSplitter,InputType,MusicToastDisplayState,NarratorStatus,
ScrollWheelHandler}`, `client/gui/{ActiveTextCollector,TextAlignment,ComponentPath,BundleMouseActions,
ItemSlotMouseAction}`, `GuiGraphicsExtractor$HoveredTextEffects`, `Font$DisplayMode`.

Denied: `client/Minecraft`, `client/player`, `client/multiplayer`, `world/entity`, `world/level`,
`world/item/ItemStack`, `server`, `client/gui/{GuiGraphicsExtractor,Font,Gui,Hud}`,
`core/registries/BuiltInRegistries` (its <clinit> bootstraps every registry: tier-1 data).

Refused after all four tests (the to-do list): `BuiltInRegistries` (from `Commands.<clinit>` ->
`Permission.<clinit>` while typing a command: command usage hints do not work), `RenderPipeline` (a signature
type, harmless), log4j's servlet probes (harmless), `Blocks` (requested on purpose by the test).

## Proxies (`proxies.conf`; every method body listed in `cache/proxies/26.3-<checkout>/GENERATED.txt`)
Session 2: new generator directives `statics` (vanilla `<clinit>` and static bodies), `keep owner#name[desc]`,
`keepall class`, name-only `noop`; impls registered for a superclass cover overrides in generated subclasses;
proxies are generated per checkout (`cache/proxies/26.3-<hash>`). `ItemStack`, `Inventory`, `Options`,
`OptionInstance`, `KeyMapping`, `Hud`, `ChatComponent` are no longer proxies. The session-1 list below is history.
76 classes, 4024 methods: 190 impl, 338 no-op, the rest logging stubs. Implemented methods and why:
- `Minecraft` (ClientImpl): `getInstance` (canonical proxy), `getWindow`, `getSoundManager`, `getConnection`,
  `getFixerUpper` (vanilla's `DataFixers.getDataFixer()`, built once like the constructor does: options.txt
  loads through `Options.dataFix`),
  `getNarrator`, `getPlayerSocialManager`, `getReportingContext`, `getEntityRenderDispatcher`, `textInputManager`
  (canonical service proxies); `isDemo/isLocalServer/hasSingleplayerServer/getSingleplayerServer/
  getCurrentServer/isOfflineDeveloperMode` false/null and `isMultiplayerServer` true (Haste always talks to a
  server process; PauseScreen therefore shows "Disconnect"); `isPaused` false; `hasShift/Control/AltDown` from
  the last key event's mods; `get/setLastInputType`; `onTextInputFocusChange` (recorded for IME). Fields set:
  `font`, `fontFilterFishy` (same Font), `options`, `gui`, `mouseHandler`, `player`, `level`, `gameMode`.
- `Gui`: `screen()`, `setScreen` (old.removed(); added(); init(w, h)). `Gui.hud` = Hud proxy.
- `Hud` (everything else no-op; the HUD is Rust's): `getChat`, `getFont`, `getGuiTicks`.
- `Window`: GUI-scaled and pixel sizes from the last render, `getGuiScale`, `isFocused` true.
- `Font` (FontImpl): `width(String)` -> Rust `TextMetrics`; styled widths, `plainSubstrByWidth`, `substrByWidth`,
  `split`, `splitIgnoringLanguage`, `wordWrapHeight`, `getSplitter` use vanilla's real `StringSplitter` with
  per-code-point widths from Rust (cached in Java, invalidated by `set_text_metrics`; bold adds 1 px);
  `isBidirectional` false; `lineHeight` field from Rust (9).
- `GuiGraphicsExtractor` (GfxImpl, batched into `GuiBatch`): fill (both), fillGradient, horizontal/verticalLine,
  outline, textHighlight (approximated as a translucent fill), text (String/Component/FormattedCharSequence; one
  TEXT per colour run), centeredText, textWithWordWrap, textWithBackdrop, textRenderer/textRendererForWidget
  (an ActiveTextCollector whose positioning/scrolling is vanilla's default code), blit (all identifier forms),
  blitSprite (whole sprite; the sub-region form draws the whole sprite and logs to Unimplemented), item/fakeItem
  (GuiCommand::Item with the stack count), itemDecorations (count text bottom-right), enable/disableScissor,
  containsPointInScissor, pose() (a Matrix3x2fStack subclass that emits TranslatePush/Pop lazily before each
  draw), guiWidth/Height, nextStratum/blurBeforeThisStratum (no-ops), requestCursor (recorded), entity (box
  recorded), setTooltipForNextFrame/setComponentTooltipForNextFrame (all but the ClientTooltipComponent form;
  drawn at frame end as tooltip/background + tooltip/frame sprites and lines), extractDeferredElements.
- `Options` (OptionsImpl): `getMenuBackgroundBlurriness` 5 and `menuBackgroundBlurriness` (vanilla default);
  `musicToast` NEVER (Haste has no music); `saveChatDrafts` false (Haste choice, vanilla default not determined);
  `autoSuggestions` true (vanilla default); `getBackgroundColor(int)` returns the caller's default and
  `(float)` the opacity as alpha (approximate: no text-background options yet). `OptionInstance.get/set` keep
  values in Java. Every KeyMapping field is a canonical KeyMapping proxy.
- `KeyMapping` (KeyImpl): `matches(KeyEvent)` by SDL scancode with the controls-screen defaults (WASD, Space,
  LShift, LCtrl, E, F, Q, T, Tab, '/', F2, F5, F11, P, L, 1-9); attack/use/pick bound to mouse 1/3/2
  (`matchesMouse`); the other bindings are unbound (defaults not determined); `isDown`/`consumeClick` false.
- `MouseHandler`: grab/release recorded (`GuiPipeline::mouse_grabbed`), position from the last mouse event.
- `TextInputManager`: text-input focus and IME area recorded (no Rust API yet).
- `CursorType`: named proxies; the requested cursor per frame is recorded (no GuiFrame field yet).
- `ClientPacketListener`: `sendChat/sendCommand` -> Rust `GameState::sent`; `registryAccess` -> every registry
  exists and is empty (EmptyRegistries); `getSuggestionsProvider` -> vanilla's real ClientSuggestionProvider
  with no permissions; `getCommands` -> empty dispatcher (the command tree is not bridged); online players empty;
  `serverLinks` EMPTY; `searchTrees` -> vanilla's real `SessionSearchTrees` (allowed class; the creative
  inventory's search, empty until a reload populates it).
- `ChatComponent`: typed-line history (Java, 100 lines, like vanilla's recent chat), `isChatFocused`,
  `extractRenderState` draws Rust's `chat_lines` above the input (approximate layout: 320 px wide, 9 px lines,
  50% black backdrop, 10/20 lines unfocused/focused; no fading, no styles).
- `LocalPlayer/Player/LivingEntity/Entity`: `getInventory`, `hasInfiniteMaterials/isCreative/isSpectator` from
  Rust's game mode, `chatAbilities` NO_RESTRICTIONS, `closeContainer` -> ContainerInput::Close to Rust,
  `canUseSlot` true for the player, `getActiveEffects` empty (effects not bridged), `level()` the ClientLevel
  proxy. `inventoryMenu`/`containerMenu` = a real vanilla InventoryMenu over the Inventory proxy.
- `Inventory`: `getItem` (canonical ItemStack per slot), `getContainerSize` 41, `getSelectedSlot/Item`,
  `getName` ("container.inventory").
- `ItemStack` (StackImpl, natives by handle): `getCount`, `setCount` (writes Rust), `isEmpty`, `typeHolder`
  (a vanilla stand-alone Holder.Reference keyed by the Rust registry name, no tags/components), `getComponents`
  EMPTY, `getTooltipLines` (one line: registry name), `getTooltipImage` empty, `toString`. `ItemStack.EMPTY` is
  the handle -1 proxy.
- `MultiPlayerGameMode`: `handleContainerInput` -> Rust `ContainerClick` (no client prediction).
- `EntityRenderDispatcher/AvatarRenderer`: empty `EntityRenderState`s (entity rendering is Rust's).
- `RecipeBookComponent` (closed, everything else no-op): `updateScreenPosition` centres the container,
  `hasClickedOutside`, `narrationPriority` NONE. `RECIPE_BUTTON_SPRITES` set to the jar's
  `recipe_book/button(_highlighted)` sprites.
- `Level`: `isClientSide` true. No-op classes: SoundManager, SimpleSoundInstance, GameNarrator,
  PlayerSocialManager, ReportingContext. Types only: RenderPipelines, SoundEvents, DataComponents (no <clinit>:
  constants null), ServerPlayer.
- Generated superclasses that shadow allowed classes: `ReentrantBlockableEventLoop`, `BlockableEventLoop`
  (Minecraft's), `RecipeBookComponent`.

Session 3 (title-screens branch; the Title, Options and Multiplayer screens of the menu):
- `Panorama` is a tier-2 proxy (one canonical instance, `GameRenderer.panorama()`): `startSpin/holdSpin`
  keep the spin state, `extractRenderState` contributes nothing to a frame - Rust's renderer draws the
  rotating cube map later.
- `Minecraft`: `allowsTelemetry` false (Haste sends no telemetry), `isTextFilteringEnabled` false (no
  player reporting; the options screen saves its settings through it), `checkModStatus`
  `ModCheck(PROBABLY_NOT, "haste")` (no mods in phase 1), `splashManager` a real `SplashManager` with no
  user - Haste runs no resource reload, so no splash object and the title screen draws no splash text yet.
- `Window`: `getPreferredFullscreenVideoMode` empty (the display mode is Rust's decision).
- `loader`: `net/minecraft/client/resources/SplashManager` allow-listed (a helper the GUI reaches).
- `Bridge.open` is atomic: a screen whose `init` throws is dropped before the exception reaches Rust
  (`Gui.setScreen` leaves the new screen set even when `init` throws, so the next open would call
  `removed()` on half of it).
- Still blocking the multiplayer screen: `ServerStatusPinger` (`NoClassDefFoundError` from
  `JoinMultiplayerScreen.<init>`; `net/minecraft/client/multiplayer/` is deny-listed, its `init` needs
  `ServerList` too) - roadmap L1e.

Session 4 (menu-mode-window branch; the multiplayer screen and the join seam):
- `loader` allow-list: `ServerList`, `ServerData`, `resolver/` (the server list's own data, tier 3:
  vanilla reads and writes servers.dat in the game directory, which the client points at Haste's
  config directory so haste-setup's servers.dat and Java's are one file), `EventLoopGroupHolder`
  (built by the ping lambda before every ping; the proxy no-ops it, no netty loads),
  `LanServerDetection`/`LanServer` (the multicast LAN listener the screen starts and stops itself),
  `NameAndId` (the status ping's player sample record).
- `ServerStatusPinger` is a tier-2 proxy (`haste.gui.PingerImpl`): the status ping runs through
  `haste_protocol::ping` on a Rust thread (`Native.startPing`), and the results (status JSON +
  latency) are applied to the `ServerData` fields on the screen's tick (`Native.takePingResults`),
  the same fields vanilla's ping callbacks set. The favicons' `TextureManager` is a proxy too
  (`haste.gui.TextureImpl`): locations are recorded, no pixel is uploaded (the GPU is Rust's).
- `Minecraft.execute(Runnable)` runs the task at once: the ping callbacks call it from the tick
  that applied them, already on the client thread (declared on `BlockableEventLoop`, the generated
  super of the Minecraft proxy, because Minecraft itself does not override execute).
- `Minecraft.stop` -> `Native.requestQuit` (the title screen's Quit Game; the client polls the flag).
- `Bridge.disconnected` picks the parent for the disconnect screen: in menu mode (not in a world)
  the connecting screen's parent (the server list), like vanilla's ConnectScreen failure path;
  in-world kicks keep the old behaviour (no parent).
- The join seam: `net/minecraft/client/gui/screens/ConnectScreen` is denied and replaced
  (REPLACEMENTS). Vanilla's opens its own netty connection; Haste's shows vanilla's connecting
  screen (connecting text, Cancel that asks Rust to drop the join) and hands the address to Rust
  (`Native.requestJoin`), which creates the session (`Session::join`) with a clone of the menu's
  UI - its game thread then runs `Ui::joined` on it, so screens and chat history survive into the
  game. A proxy cannot stand in for the class (a `class` line regenerates the super chain and
  would shadow the real `Screen`), so the replacement extends `Screen` directly.

## Replacements (`java/haste/replace`)

Haste-written stand-ins for vanilla classes that must not run here (deny-listed) but which vanilla code
instantiates unconditionally. `loader::REPLACEMENTS` lists the internal names; the class loader serves them
from the game build under their vanilla name, whatever the allow/deny policy says, and logs them as
`ClassSource::Replacement` (`title_screen` checks it). Each one extends the real vanilla class it stands in
for and reduces what vanilla calls to no-ops.
- `com/mojang/realmsclient/gui/screens/RealmsNotificationsScreen`: TitleScreen.init builds one and ticks,
  renders and clicks it. Vanilla's builds a realms client to look for notifications; Haste has no Realms, so
  it is a plain `Screen` that draws nothing and the bell never appears. Allow-listing the real class instead
  starts `RealmsAvailability`'s async availability check, which dies on the denied `RealmsClient` and whose
  exception surfaces through `Bridge.render`; a tier-2 proxy cannot cover it, because a `class` line
  generates the whole super chain (`RealmsScreen` -> `Screen`) and would replace every real vanilla screen.

## Canonical proxies and handles
Java keeps one proxy per Rust object for the JVM's lifetime (`haste.gui.Proxies`): singletons for Minecraft, Gui,
Hud, Font, Options, Window, LocalPlayer, Inventory, services; `ItemStack` per inventory slot (handle = slot,
-1 = EMPTY). Approximation: a slot's proxy stays the same object when its contents change (vanilla moves stack
objects between slots); revisit when item stacks become real Rust objects.

## Frame format
`Bridge.render` returns `{int[] commands, String[] strings}` (`GuiBatch`, opcodes mirrored in `src/gui.rs`).
Coordinates are GUI pixels; TranslatePush/Pop and Scissor apply to the commands after them (scissor rects are in
the translated space, like the rest). Text: one command per colour run; bold/italic/obfuscated are lost.
Item counts are drawn as Text by `itemDecorations`; the renderer must not draw `Item.count` itself.

## Input numbering (observed)
26.3 uses SDL3: `KeyEvent(key, keycode, modifiers)` where `key` is the SDL scancode (`InputConstants.KEY_*`,
what Escape/Enter/key mappings match) and `keycode` the SDL keycode (EditBox matches Backspace (8) and arrows on
it). `GuiPipeline::key(key, scancode, ...)` passes them in that order; `keys::keycode` maps scancodes to US
layout keycodes. Mouse buttons: 1 left, 2 middle, 3 right. Mods: SDL keymod bits (shift 3, ctrl 192, alt 768).

## Measurements, session 2 (load average 15-36; numbers are noisy)
- Vanilla bootstrap in `Bridge.create`: 9-47 s (median ~15 s). Synchronised registries from a real server through
  `RegistryDataCollector`: 1-5 s on the first join, 80-450 ms after.
- Release, after 3000 warm-up frames: HUD alone p50 80 µs; HUD + PauseScreen p50 166 µs; p99 2.5-4.4 ms (GC/JIT
  under load 22). PauseScreen alone p50 43 µs.

## Measurements, session 1 (load average 15-35 on this machine during the session: numbers are noisy)
- JVM create 50-250 ms; boot (stamp check ~0.3 s + Bridge.create, i.e. loading/initialising the vanilla classes
  the canonical proxies touch) 2.7-11 s; Java build when sources changed 5-40 s (javac against the client jar).
- First tier-3 call (`Identifier.parse` in a fresh JVM before the bridge preloaded anything): 0.5-1.9 s; after
  boot ~1 ms.
- `PauseScreen` `render()` after warm-up (release): p50 36-49 us, mean 43-69 us (target < 100 us).
