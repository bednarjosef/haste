//! Tier-3 class loading policy (passed to the Java `HasteClassLoader`) and its log.
//!
//! Patterns are JVM internal names. A pattern ending in `/` is a package prefix (subpackages
//! included); anything else is one class (its nested `$` classes included). The most specific
//! matching pattern decides (exact class > longer package prefix > shorter; deny wins ties), so a
//! value type inside a denied package can be allowed by name. A generated proxy wins over both.
//! Rationale for each entry is in `NOTES.md`.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Packages served as real vanilla Java from the client jar.
///
/// Since the registry bridge (see `NOTES.md`, "Registries"), vanilla's own bootstrap runs in tier 3:
/// every built-in registry entry (blocks, items, entity types, data component types, ...) is a real
/// vanilla object created by the player's jar. So `net/minecraft/` as a whole is allowed: the
/// definitions, codecs, value types and the GUI. What must not run from the jar is denied below
/// (the client engine and the server) or is a generated proxy (tier-1 objects, which win over the
/// jar). Client packages are re-allowed one by one.
pub const ALLOW_PACKAGES: &[&str] = &[
    "net/minecraft/",
    "net/minecraft/client/gui/",
    "net/minecraft/client/input/",
    "net/minecraft/client/tutorial/",
    "net/minecraft/client/entity/",
    "net/minecraft/client/searchtree/",
    "net/minecraft/client/waypoints/",
    "net/minecraft/client/multiplayer/chat/",
    "net/minecraft/client/renderer/entity/state/",
    "net/minecraft/client/renderer/state/",
    "net/minecraft/client/resources/language/",
    // the pack repository the Resource Packs screen browses is real vanilla Java (tier 3):
    // the skin manager stays proxied, so only the classes it needs are allowed below
    // (ALLOW_CLASSES): ClientPackSource, IndexedAssetSource, NativeImage, TextureUtil
    // the pack.mcmeta "language" section LanguageManager reads (the language list and its RTL flag)
    "net/minecraft/client/resources/metadata/language/",
    // the Telemetry Data screen's event list is real vanilla Java over these value types; Haste sends
    // no telemetry (allowsTelemetry false), so the button is greyed like vanilla's, but the screen loads
    "net/minecraft/client/telemetry/",
    "net/minecraft/server/packs/",
    "net/minecraft/server/permissions/",
    "net/minecraft/server/dialog/",
    "net/minecraft/server/jsonrpc/",
    "com/mojang/math/",
    "com/mojang/blaze3d/platform/cursor/",
];

/// Single classes served as real vanilla Java inside denied packages (value types and helpers the
/// GUI and the bootstrap reach).
pub const ALLOW_CLASSES: &[&str] = &[
    // the pack repository the Resource Packs screen browses is real vanilla Java (tier 3):
    // ClientPackSource (the built-in vanilla pack entry) and IndexedAssetSource (the assets
    // dir view) are data/IO code; the skin manager stays proxied
    "net/minecraft/client/resources/ClientPackSource",
    "net/minecraft/client/resources/IndexedAssetSource",
    // the pack screen reads pack.png icons through NativeImage (pure PNG decode, no GPU);
    // DynamicTexture's constructor touches TextureUtil's statics (never called: the texture
    // manager proxy records the location and uploads nothing)
    "com/mojang/blaze3d/platform/NativeImage",
    "com/mojang/blaze3d/platform/TextureUtil",
    "net/minecraft/server/Bootstrap",
    "net/minecraft/server/LoggedPrintStream",
    "net/minecraft/server/DebugLoggedPrintStream",
    "net/minecraft/server/ServerScoreboard",
    "net/minecraft/server/level/ServerEntityGetter",
    "net/minecraft/server/level/TicketType",
    "net/minecraft/server/network/Filterable",
    "net/minecraft/server/ServerLinks",
    "net/minecraft/server/RegistryLayer",
    "net/minecraft/client/multiplayer/ClientSuggestionProvider",
    "net/minecraft/client/multiplayer/RegistryDataCollector",
    "net/minecraft/client/multiplayer/ClientRegistryLayer",
    "net/minecraft/client/multiplayer/PlayerInfo",
    // the server's recipe data (update_recipes): two maps of registry data (item property sets,
    // stonecutter recipes), no server state; the kept handler stores it on the connection proxy
    "net/minecraft/client/multiplayer/ClientRecipeContainer",
    // the recipe book itself: a map of RecipeDisplayId -> RecipeDisplayEntry plus the book settings,
    // rebuilt into RecipeCollections (all under the allowed gui/ package). No engine state: the kept
    // recipe-book handlers read it through Player.getRecipeBook() and Proxies.create builds the one
    // vanilla's LocalPlayer constructor would have (a proxy never runs that constructor)
    "net/minecraft/client/ClientRecipeBook",
    // the multiplayer screen's own data (tier 3: no hot game state). ServerList reads/writes
    // servers.dat in the game directory, ServerData is its entry value type, and the resolver
    // package is the address parsing the screen's join and direct-connect field use.
    "net/minecraft/client/multiplayer/ServerList",
    "net/minecraft/client/multiplayer/ServerData",
    "net/minecraft/client/multiplayer/resolver/",
    // the create-world screen's data loading (vanilla's CreateWorldScreen.openFresh loads the
    // vanilla data pack into registries before the screen shows). Pure data code from the
    // server package: no server thread, no world; the integrated server itself stays denied
    // (Haste plays singleplayer through haste-setup's local server process).
    "net/minecraft/server/WorldLoader",
    "net/minecraft/server/WorldStem",
    "net/minecraft/server/RegistryLayer",
    "net/minecraft/server/ReloadableServerResources",
    "net/minecraft/server/ReloadableServerRegistries",
    "net/minecraft/server/ServerFunctionLibrary",
    "net/minecraft/server/ServerAdvancementManager",
    // only its static members are used by the data loading (configurePackRepository picks the
    // vanilla pack; DEFAULT_GAME_RULES); the server object itself is never instantiated here
    "net/minecraft/server/MinecraftServer",
    // the status interface MinecraftServer implements (a parameter type of the allow-list's
    // class loading, never implemented here)
    "net/minecraft/server/ServerInfo",
    // the command implementations the data loading's ReloadableServerResources registers
    // (brigadier trees over the allowed commands package; executing them needs a server, which
    // never runs here)
    "net/minecraft/server/commands/",
    // the netty event-loop holder the server list's ping lambda builds before every ping
    // (Haste's pinger proxy ignores it; see java/haste/gui/PingerImpl)
    "net/minecraft/server/network/EventLoopGroupHolder",
    // the status ping's player sample record (ServerStatus.Players): a value type the
    // server list's tooltip shows
    "net/minecraft/server/players/NameAndId",
    // LAN server detection (a multicast listener thread); the screen starts it in init and
    // stops it in removed, exactly vanilla's lifecycle
    "net/minecraft/client/server/LanServerDetection",
    "net/minecraft/client/server/LanServer",
    "net/minecraft/client/multiplayer/SessionSearchTrees",
    "net/minecraft/client/multiplayer/ClientLevel$ClientLevelData",
    "net/minecraft/client/multiplayer/ClientAdvancements",
    "net/minecraft/client/multiplayer/CommonListenerCookie",
    // the connection's recipe container (update_recipes packets; container menus read it through
    // ClientLevel.recipeAccess)
    "net/minecraft/client/multiplayer/ClientRecipeContainer",
    "net/minecraft/client/StringSplitter",
    "net/minecraft/client/InputType",
    "net/minecraft/client/MusicToastDisplayState",
    "net/minecraft/client/NarratorStatus",
    "net/minecraft/client/ComponentCollector",
    "net/minecraft/client/ScrollWheelHandler",
    // the title screen asks the HUD for the splash text; its reload listener does not run in Haste
    "net/minecraft/client/resources/SplashManager",
    // the default player skins (the social interactions screen's entries; offline mode always
    // shows them): a table of PlayerSkin value objects, no manager state
    "net/minecraft/client/resources/DefaultPlayerSkin",
    // the creative inventory's saved hotbars: real vanilla Java over hotbar.nbt in the JVM game
    // directory (loads and saves exactly like vanilla's)
    "net/minecraft/client/HotbarManager",
    "net/minecraft/client/player/inventory/Hotbar",
    "net/minecraft/client/Options",
    "net/minecraft/client/OptionInstance",
    "net/minecraft/client/ToggleKeyMapping",
    "net/minecraft/client/KeyMapping",
    "net/minecraft/client/CloudStatus",
    "net/minecraft/client/GraphicsPreset",
    "net/minecraft/client/InactivityFpsLimit",
    "net/minecraft/client/PreferredGraphicsApi",
    "net/minecraft/client/PrioritizeChunkUpdates",
    // the Video Settings screen holds one (Minecraft.getGpuWarnlistManager): a real vanilla object
    // whose reload listener never runs here, so it always reports no warnings
    "net/minecraft/client/renderer/GpuWarnlistManager",
    "net/minecraft/client/TextureFilteringMethod",
    "net/minecraft/client/AttackIndicatorStatus",
    "com/mojang/blaze3d/platform/InputConstants",
    "com/mojang/blaze3d/platform/MacosUtil",
    "com/mojang/blaze3d/buffers/Std140SizeCalculator",
    "com/mojang/blaze3d/platform/VideoMode",
    // Window.findBestMonitor's return type (Video Settings holds one; Haste has no monitors, so null)
    "com/mojang/blaze3d/platform/Monitor",
    "com/mojang/blaze3d/font/GlyphProvider",
    // the sign editor asks the renderer for the text's dark colour (AbstractSignRenderer.getDarkColor,
    // a static formula over the SignText). Only that static method ever runs; the interface it
    // implements is loaded with it, and nothing else of the block-entity renderer does.
    "net/minecraft/client/renderer/blockentity/AbstractSignRenderer",
    "net/minecraft/client/renderer/blockentity/BlockEntityRenderer",
    "net/minecraft/client/PresenceSharing",
    "net/minecraft/client/sounds/MusicManager$MusicFrequency",
    "net/minecraft/client/sounds/SoundEngine$PlayResult",
    "net/minecraft/client/renderer/texture/MissingTextureAtlasSprite",
    "net/minecraft/client/renderer/culling/Frustum",
    "net/minecraft/client/renderer/Rect2i",
    "net/minecraft/client/renderer/fog/FogData",
    "net/minecraft/client/resources/model/sprite/Material",
    "net/minecraft/server/level/ClientInformation",
    "net/minecraft/server/level/ParticleStatus",
    "net/minecraft/client/CameraType",
    "net/minecraft/client/DeltaTracker",
    // A render pipeline is GPU state, but a pipeline *object* is only data (a location, a blend function):
    // Haste's tier-2 RenderPipelines holds the GUI's, built through vanilla's own builder out of these value
    // types, and GfxImpl reads the blend function off them (see haste.gui.Pipelines). No shader is compiled,
    // no backend is asked for anything, nothing else of the renderer runs.
    "com/mojang/renderpearl/api/GpuFormat",
    "com/mojang/renderpearl/api/pipeline/",
    "com/mojang/renderpearl/api/vertex/",
    // the builder's (empty) shader-defines map; same value-type reasoning
    "net/minecraft/client/renderer/ShaderDefines",
];

/// The client engine (rendering, sound, window, the `Minecraft` object, the connection) and the
/// server: never loaded from the jar. Tier-1 objects inside allowed packages (`Level`, `Entity`,
/// `Player`, ...) are generated proxies, which win over the jar; see `proxies.conf`.
pub const DENY: &[&str] = &[
    "net/minecraft/client/",
    "net/minecraft/client/gui/render/",
    "net/minecraft/server/",
    "com/mojang/",
    // one class out of the otherwise-allowed screens package: vanilla's ConnectScreen starts
    // its own netty connection, but the connection is tier 1 (Rust owns it). Haste's
    // replacement (see REPLACEMENTS) hands the address to Rust instead.
    "net/minecraft/client/gui/screens/ConnectScreen",
];

/// Classes the title screen's Singleplayer click loads on first use: the world-selection screen
/// tree (SelectWorldScreen, WorldSelectionList and its entries, the layout and widget classes it
/// builds, the NBT and level-summary classes the world list reads), captured by diffing the class
/// log around the click. Menu mode loads them on a background thread once the title screen is up
/// (see `JvmHost::warm_screen_classes`), so the first click does not pay for them.
pub const WARM_CLASSES: &[&str] = &[
    // the screens themselves
    "net.minecraft.client.gui.screens.worldselection.SelectWorldScreen",
    "net.minecraft.client.gui.screens.worldselection.WorldSelectionList",
    "net.minecraft.client.gui.screens.LoadingDotsText",
    "net.minecraft.client.gui.screens.FaviconTexture",
    // the widgets and layouts they build
    "net.minecraft.client.gui.components.AbstractContainerWidget",
    "net.minecraft.client.gui.components.AbstractScrollArea",
    "net.minecraft.client.gui.components.AbstractSelectionList",
    "net.minecraft.client.gui.components.AbstractStringWidget",
    "net.minecraft.client.gui.components.EditBox",
    "net.minecraft.client.gui.components.ObjectSelectionList",
    "net.minecraft.client.gui.components.SelectableEntry",
    "net.minecraft.client.gui.components.StringWidget",
    "net.minecraft.client.gui.layouts.AbstractLayout",
    "net.minecraft.client.gui.layouts.FrameLayout",
    "net.minecraft.client.gui.layouts.GridLayout",
    "net.minecraft.client.gui.layouts.HeaderAndFooterLayout",
    "net.minecraft.client.gui.layouts.LinearLayout",
    "net.minecraft.client.gui.navigation.FocusNavigationEvent",
    // the text shaping EditBox and the list entries use
    "net.minecraft.client.StringSplitter",
    "net.minecraft.client.ComponentCollector",
    "net.minecraft.client.gui.components.ComponentRenderUtils",
    "net.minecraft.client.gui.components.TextCursorUtils",
    "net.minecraft.network.chat.FormattedText",
    // the world summary read (level.dat: NBT into LevelSettings/LevelSummary)
    "net.minecraft.nbt.NbtIo",
    "net.minecraft.nbt.NbtUtils",
    "net.minecraft.nbt.TagTypes",
    "net.minecraft.nbt.visitors.SkipFields",
    "net.minecraft.world.level.LevelSettings",
    "net.minecraft.world.level.WorldDataConfiguration",
    "net.minecraft.world.level.levelgen.WorldOptions",
    "net.minecraft.world.level.storage.LevelStorageSource",
    "net.minecraft.world.level.storage.LevelSummary",
    "net.minecraft.world.level.storage.LevelVersion",
    "net.minecraft.util.DirectoryLock",
    "net.minecraft.util.FastBufferedInputStream",
    // the rest of the click's first-use tree, captured by diffing the class log around the
    // click (see tests/click_lag.rs): lazily resolved references and anonymous/nested classes
    // `getDeclaredClasses` does not reach (they have no InnerClasses entry of their own)
    "com.google.common.hash.HashFunction",
    "com.mojang.blaze3d.platform.TextInputManager",
    "com.mojang.math.Divisor",
    "com.mojang.serialization.OptionalDynamic",
    "it.unimi.dsi.fastutil.ints.IntIterator",
    "org.apache.commons.lang3.StringUtils",
    "net.minecraft.CharPredicate",
    "net.minecraft.client.StringSplitter$1",
    "net.minecraft.client.gui.ComponentPath",
    "net.minecraft.client.gui.layouts.LayoutSettings",
    "net.minecraft.client.renderer.texture.TextureManager",
    "net.minecraft.nbt.CompoundTag$2",
    "net.minecraft.nbt.IntTag$Cache",
    "net.minecraft.nbt.NbtIo$1",
    "net.minecraft.nbt.NbtOps$1",
    "net.minecraft.nbt.StreamTagVisitor$EntryResult",
    "net.minecraft.nbt.StreamTagVisitor$ValueResult",
    "net.minecraft.nbt.visitors.CollectToTag",
    "net.minecraft.nbt.visitors.FieldSelector",
    "net.minecraft.nbt.visitors.FieldTree",
    "net.minecraft.network.chat.FormattedText$3",
    "net.minecraft.network.chat.FormattedText$4",
    "net.minecraft.util.StringUtil",
    "net.minecraft.world.level.storage.LevelResource",
    // the TextureManager proxy's impl class (favicons register through it)
    "haste.gui.TextureImpl",
];

/// Haste-written replacement classes (sources: `java/haste/replace/<internal>.java`), served from the
/// game build under their vanilla name whatever the package rules above say.
///
/// A replacement exists for a vanilla class that must not run in Haste (deny-listed) but which vanilla
/// code instantiates unconditionally. It extends the real vanilla class it stands in for and keeps the
/// methods that code calls as no-ops.
pub const REPLACEMENTS: &[&str] = &[
    // The title screen builds one in `init()` and ticks, renders and clicks it. Haste has no Realms, so
    // the notification bell is a plain `Screen` that draws nothing.
    "com/mojang/realmsclient/gui/screens/RealmsNotificationsScreen",
    // Vanilla's ConnectScreen opens its own netty connection in a Java thread; in Haste the
    // connection is tier 1 (haste-protocol, Rust). The replacement shows vanilla's connecting
    // screen and hands the address to Rust, which joins and takes over the window.
    "net/minecraft/client/gui/screens/ConnectScreen",
    // Vanilla's WorldOpenFlows plays a world through the integrated server; Haste's singleplayer
    // is haste-setup's local dedicated server (Rust owns the process). The replacement keeps the
    // screen flow and hands the world (and creation settings) to Rust.
    "net/minecraft/client/gui/screens/worldselection/WorldOpenFlows",
];

/// How the game class loader resolved a class (or why it did not).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClassSource {
    /// Real vanilla class from the client jar (tier 3).
    Jar,
    /// Generated proxy (tier 2).
    Proxy,
    /// Haste's own game-side Java (`haste.gui`).
    Game,
    /// A Haste-written replacement for a vanilla class (see [`REPLACEMENTS`]).
    Replacement,
    /// A third-party library jar (fastutil, guava, DFU, ...).
    Library,
    /// Refused, with the reason (the to-do list).
    Refused(String),
}

/// Snapshot of the class loader's log: class name -> how it was served.
#[derive(Clone, Debug, Default)]
pub struct ClassLog {
    /// Binary class name (`net.minecraft.core.BlockPos`) -> source.
    pub classes: BTreeMap<String, ClassSource>,
}

impl ClassLog {
    /// Parses the Java side's `"<how>\t<name>"` lines.
    pub fn parse(s: &str) -> ClassLog {
        let mut classes = BTreeMap::new();
        for line in s.lines() {
            let Some((how, name)) = line.split_once('\t') else {
                continue;
            };
            let src = match how {
                "jar" => ClassSource::Jar,
                "proxy" => ClassSource::Proxy,
                "game" => ClassSource::Game,
                "replacement" => ClassSource::Replacement,
                "lib" => ClassSource::Library,
                other => ClassSource::Refused(
                    other.strip_prefix("refused: ").unwrap_or(other).to_owned(),
                ),
            };
            classes.insert(name.to_owned(), src);
        }
        ClassLog { classes }
    }
    /// Names with the given source.
    pub fn with(&self, pred: impl Fn(&ClassSource) -> bool) -> Vec<&str> {
        self.classes
            .iter()
            .filter(|(_, s)| pred(s))
            .map(|(n, _)| n.as_str())
            .collect()
    }
    /// Refused classes with reasons.
    pub fn refused(&self) -> Vec<(&str, &str)> {
        self.classes
            .iter()
            .filter_map(|(n, s)| match s {
                ClassSource::Refused(r) => Some((n.as_str(), r.as_str())),
                _ => None,
            })
            .collect()
    }
}

/// Vanilla's third-party library jars under a Maven-layout `libraries` directory. Fabric's own jars
/// (loader, mixin) and ASM are not vanilla libraries and are skipped.
pub fn library_jars(dir: &Path) -> Vec<PathBuf> {
    fn walk(d: &Path, root: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, root, out);
            } else if p.extension().is_some_and(|x| x == "jar") {
                let rel = p
                    .strip_prefix(root)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('\\', "/");
                if !rel.starts_with("net/fabricmc/") && !rel.starts_with("org/ow2/asm/") {
                    out.push(p);
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_class_log() {
        let log = ClassLog::parse("jar\tnet.minecraft.core.BlockPos\nproxy\tnet.minecraft.client.Minecraft\nrefused: deny-listed\tnet.minecraft.world.level.Level\nlib\tcom.google.common.base.Preconditions\nreplacement\tcom.mojang.realmsclient.gui.screens.RealmsNotificationsScreen\n");
        assert_eq!(log.classes["net.minecraft.core.BlockPos"], ClassSource::Jar);
        assert_eq!(
            log.with(|s| *s == ClassSource::Proxy),
            vec!["net.minecraft.client.Minecraft"]
        );
        assert_eq!(
            log.refused(),
            vec![("net.minecraft.world.level.Level", "deny-listed")]
        );
        assert_eq!(
            log.with(|s| *s == ClassSource::Replacement),
            vec!["com.mojang.realmsclient.gui.screens.RealmsNotificationsScreen"]
        );
    }

    #[test]
    fn every_replacement_is_denied() {
        // a replacement only makes sense for a class Haste refuses from the jar
        for &internal in REPLACEMENTS {
            assert!(
                DENY.iter().any(|d| {
                    let d: &str = d;
                    if d.ends_with('/') {
                        internal.starts_with(d)
                    } else {
                        internal == d
                    }
                }),
                "{internal} is not denied"
            );
        }
    }

    #[test]
    fn deny_covers_tier1_types() {
        for t in [
            "net/minecraft/client/",
            "net/minecraft/server/",
            "com/mojang/",
        ] {
            assert!(DENY.contains(&t), "{t}");
        }
        assert!(ALLOW_PACKAGES.iter().all(|p| p.ends_with('/')));
    }
}
