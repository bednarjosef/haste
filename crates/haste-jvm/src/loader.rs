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
    // the multiplayer screen's own data (tier 3: no hot game state). ServerList reads/writes
    // servers.dat in the game directory, ServerData is its entry value type, and the resolver
    // package is the address parsing the screen's join and direct-connect field use.
    "net/minecraft/client/multiplayer/ServerList",
    "net/minecraft/client/multiplayer/ServerData",
    "net/minecraft/client/multiplayer/resolver/",
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
    "net/minecraft/client/multiplayer/CommonListenerCookie",
    "net/minecraft/client/StringSplitter",
    "net/minecraft/client/InputType",
    "net/minecraft/client/MusicToastDisplayState",
    "net/minecraft/client/NarratorStatus",
    "net/minecraft/client/ComponentCollector",
    "net/minecraft/client/ScrollWheelHandler",
    // the title screen asks the HUD for the splash text; its reload listener does not run in Haste
    "net/minecraft/client/resources/SplashManager",
    "net/minecraft/client/Options",
    "net/minecraft/client/OptionInstance",
    "net/minecraft/client/ToggleKeyMapping",
    "net/minecraft/client/KeyMapping",
    "net/minecraft/client/CloudStatus",
    "net/minecraft/client/GraphicsPreset",
    "net/minecraft/client/InactivityFpsLimit",
    "net/minecraft/client/PreferredGraphicsApi",
    "net/minecraft/client/PrioritizeChunkUpdates",
    "net/minecraft/client/TextureFilteringMethod",
    "net/minecraft/client/AttackIndicatorStatus",
    "com/mojang/blaze3d/platform/InputConstants",
    "com/mojang/blaze3d/platform/MacosUtil",
    "com/mojang/blaze3d/buffers/Std140SizeCalculator",
    "com/mojang/blaze3d/platform/VideoMode",
    "com/mojang/blaze3d/font/GlyphProvider",
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
