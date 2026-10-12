# WS6: haste-jvm (embedded JVM, tier 3, tier 2 proxies, batched GUI pipeline)

You own `crates/haste-jvm` (Rust + the Java sources under `crates/haste-jvm/java`), and later
`crates/haste-client/src/ui.rs` (after the lead says M2 is merged). Read the spike reports first:
`../haste-spike/REPORT.md` and `REPORT-2.md`, and the proxy generator you
inherit: `crates/haste-jvm/java/proxy/Gen.java` (ASM, generates same-name classes with native /
getter / delegate / stub bodies and merges mixins) and `java/rt/` (Unimplemented log, GuiBatch).
ASM jars: `cache/mojang-spike/asm-*.jar` (copy nothing; reference by path). JDK: `cache/jdk`.

Architecture reminder (AGENTS.md): tier 3 = real vanilla Java loaded at runtime from the player's
client jar for stateless classes; tier 2 = generated proxies for stateful tier-1 objects; the
whole jar is NEVER on the classpath; GUI drawing is batched into `haste_core::gui::GuiFrame`.

## Task 1: JVM host + tier-3 loader
- `JvmHost::start(cfg)`: start the JVM (`jni` 0.22 invocation; locate libjvm under `cfg.java_home`
  = `cache/jdk`), with a Java-side `HasteClassLoader` that serves classes from (a) our runtime
  classes, (b) generated proxy class files (a directory), (c) the client jar, but ONLY for an
  allow-list of packages/classes (start with `net.minecraft.core`, `net.minecraft.resources`,
  `net.minecraft.network.chat`, `net.minecraft.util`, `net.minecraft.ChatFormatting`, `com.mojang.serialization`,
  `net.minecraft.client.gui.components`, `net.minecraft.client.gui.screens`, `net.minecraft.client.gui.navigation`,
  `net.minecraft.client.gui.narration`, `net.minecraft.client.input`, `net.minecraft.client.gui.layouts`)
  and a deny-list for tier-1 types which must resolve to proxies (`net.minecraft.client.Minecraft`,
  `net.minecraft.client.player.*`, `net.minecraft.world.entity.*`, `net.minecraft.world.item.ItemStack`,
  `net.minecraft.world.entity.player.Inventory`, `net.minecraft.client.multiplayer.ClientLevel`, ...).
  Log every class served from the jar, every proxy served, every class refused (that log is a to-do list).
  Vanilla's third-party libraries (fastutil, guava, gson, joml, jspecify, slf4j, datafixerupper,
  brigadier) are needed by tier-3 classes: get them from the version JSON's `libraries` list
  (WS7's `haste-setup` downloads them to `cache/libraries`; until then use the jars under
  `cache/server-spike/libraries` which has most of them). Static initialisers of vanilla classes run
  for real in tier 3; when one touches a tier-1 type, that is a proxy request.
- Test (ignored without cache): start JVM, call `net.minecraft.resources.Identifier.parse("minecraft:stone")`
  and `toString()` back, construct `BlockPos(1,2,3).offset(...)`, `Component.literal("hi").getString()`.
  Report JVM start time and first-call time.

## Task 2: proxies + GUI pipeline
- Port `Gen.java` to generate from a config file in the crate (`proxies.conf`): `Minecraft`
  (static `getInstance()` returns the one canonical proxy; `player`, `level`, `options`, `font`,
  `screen`, `setScreen`, `getWindow`), `Font` (`width(String)`, `width(FormattedText/Component)`,
  `lineHeight` field = 9, via natives into Rust `TextMetrics`), `GuiGraphicsExtractor` (every
  draw method as a batched delegate into `GuiBatch`: fill, fillGradient, text/centeredText,
  blitSprite/blit (texture), renderItem, scissor, pose push/pop/translate), `Window` (width/
  height/guiScale fields), `Options`, `KeyMapping` (stub bodies that do not throw for the ones
  screens call), `SoundManager` (no-op). Record in NOTES.md every method you had to implement.
- `GuiPipeline` per INTERFACES.md: `render()` = one JNI call into a Java `Frame.render(screen,
  w, h, scale, mouse, partial)` which calls `screen.extractRenderState(gfx, ...)` and returns the
  batch as one `int[]` + one `String[]` (+ one `Object[]` for identifiers/items); Rust converts to
  `GuiFrame`. Input methods forward to `screen.keyPressed(KeyEvent)`, `charTyped`,
  `mouseClicked(MouseButtonEvent, ...)`, `mouseScrolled` with the 26.3 signatures (check with
  `javap` on the client jar). `set_text_metrics` wires `Font.width` natives.
- Screens, in order: `PauseScreen` (buttons as sprites + text), `ChatScreen` (EditBox typing, the
  chat history comes from Rust via a `ChatComponent` proxy or a simple list), `InventoryScreen`
  (needs `Inventory`/`ItemStack` proxies reading Rust state through natives; item icons as
  `GuiCommand::Item`). Each screen is "done" when `render()` returns a sensible `GuiFrame` that a
  Rust test asserts on (contains the expected texts/buttons), keys/mouse change its state, and
  the Unimplemented log for it is empty or listed in NOTES.md.
- Benchmark: `render()` of PauseScreen per frame in microseconds (target < 100 µs after warm-up).

Rust-side interfaces you consume: `haste_core::gui::{GuiFrame, GuiCommand, TextMetrics}`. If the
command set lacks something a screen needs (e.g. item rendering details), request it from the lead.
Do not touch: haste-render (WS3 draws GuiFrame), haste-entity, haste-world.
