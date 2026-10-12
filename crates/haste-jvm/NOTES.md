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

## Class loader policy (`src/loader.rs`)
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

## Proxies (`proxies.conf`; every method body listed in `cache/proxies/26.3/GENERATED.txt`)
76 classes, 4024 methods: 190 impl, 338 no-op, the rest logging stubs. Implemented methods and why:
- `Minecraft` (ClientImpl): `getInstance` (canonical proxy), `getWindow`, `getSoundManager`, `getConnection`,
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
  `serverLinks` EMPTY.
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

## Measurements (load average 15-35 on this machine during the session: numbers are noisy)
- JVM create 50-250 ms; boot (stamp check ~0.3 s + Bridge.create, i.e. loading/initialising the vanilla classes
  the canonical proxies touch) 2.7-11 s; Java build when sources changed 5-40 s (javac against the client jar).
- First tier-3 call (`Identifier.parse` in a fresh JVM before the bridge preloaded anything): 0.5-1.9 s; after
  boot ~1 ms.
- `PauseScreen` `render()` after warm-up (release): p50 36-49 us, mean 43-69 us (target < 100 us).
