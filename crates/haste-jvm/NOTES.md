# haste-jvm notes (WS6)

## Layout
- `src/host.rs` `JvmHost::start`: compiles the Java side if stale (`build-java.sh`, plain javac, output in
  `build/`, gitignored), starts the JVM through `jni` 0.22's invocation API with libjvm from `cfg.java_home`
  (`cache/jdk`), registers the `haste.rt.Native` natives, calls `haste.loader.Boot.boot`, which generates the
  proxies into `cache/proxies/26.3/` (only when `proxies.conf`, the impl classes, the jar or the allow-lists
  changed: `.stamp`), creates `HasteClassLoader` and returns `haste.gui.Bridge`.
- One JVM per process (JNI cannot create a second one): `JvmHost` is a cheap clone of a process-wide host.
- Class path of the JVM: `build/boot` (haste.loader / haste.proxy / haste.rt, no vanilla references) and the
  ASM jars. Everything that references vanilla types (`haste.gui.*`, compiled *against* the client jar) is
  served by `HasteClassLoader` from `build/game`. The client jar is never on the class path.

## Class loader policy (`src/loader.rs`)
Order: JDK + boot classes from the parent; `haste.*` from `build/game`; a generated proxy if one exists for
the name (proxies win); vanilla classes from the client jar if allow-listed and not deny-listed; anything else
from the vanilla library jars (`cache/libraries`, or `cache/server-spike/libraries` until WS7 downloads the
client set; Fabric loader/mixin and ASM are excluded). Non-class resources (`assets/minecraft/lang/en_us.json`
for `Language`) are served from the client jar. Every decision is logged (`JvmHost::class_log`).

Allow-list rationale:
- `core`, `resources`, `network/chat`, `network/codec`, `nbt`, `util`, `locale`, `com/mojang/math`, root
  helpers (`ChatFormatting`, `IdentifierException`, `SharedConstants`, `CrashReport*`, `TracingExecutor`, ...):
  value types, codecs and helpers with no game state. `Identifier`'s static init pulls in `ByteBufCodecs`
  (network/codec) and `nbt.Tag`; `BlockPos` -> `Mth` -> `Util` (executors) -> `TracingExecutor`.
- `network/FriendlyByteBuf`, `RegistryFriendlyByteBuf`, `VarInt`, ...: referenced by codec signatures.
- `client/gui/{components,screens,navigation,narration,layouts}`, `client/input`: the Screen/widget family runs
  as real vanilla Java on top of the batched `GuiGraphicsExtractor` proxy.
- `client/StringSplitter`, `client/gui/ActiveTextCollector`, `TextAlignment`, `ComponentPath`: text layout
  and focus helpers widgets use; stateless.

Deny-list rationale: tier-1 objects owned by Rust (`Minecraft`, `client/player`, `client/multiplayer`
(ClientLevel, packet listener), `world/entity`, `world/level`, `ItemStack`, `server`), engine objects that
must be proxies (`GuiGraphicsExtractor`, `Font`, `Gui`, `Hud`) and `BuiltInRegistries` (its static init
bootstraps every registry: tier-1 data from Rust).

Observed refusals with the value-type test: `javax.servlet.Servlet`, `jakarta.servlet.Servlet` (log4j probes,
harmless), `net.minecraft.world.level.Level` (the test asks for it on purpose).
