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
    "net/minecraft/client/gui/components/",
    "net/minecraft/client/gui/screens/",
    "net/minecraft/client/gui/navigation/",
    "net/minecraft/client/gui/narration/",
    "net/minecraft/client/gui/layouts/",
    "net/minecraft/client/gui/contextualbar/",
    "net/minecraft/client/gui/font/",
    "net/minecraft/client/input/",
    "net/minecraft/client/multiplayer/chat/",
    "net/minecraft/client/renderer/entity/state/",
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
    "net/minecraft/client/multiplayer/CommonListenerCookie",
    "net/minecraft/client/StringSplitter",
    "net/minecraft/client/InputType",
    "net/minecraft/client/MusicToastDisplayState",
    "net/minecraft/client/NarratorStatus",
    "net/minecraft/client/ComponentCollector",
    "net/minecraft/client/ScrollWheelHandler",
    "net/minecraft/client/AttackIndicatorStatus",
    "net/minecraft/client/CameraType",
    "net/minecraft/client/DeltaTracker",
    "net/minecraft/client/gui/ActiveTextCollector",
    "net/minecraft/client/gui/TextAlignment",
    "net/minecraft/client/gui/ComponentPath",
    "net/minecraft/client/gui/BundleMouseActions",
    "net/minecraft/client/gui/ItemSlotMouseAction",
    "net/minecraft/client/gui/Hud",
    "net/minecraft/client/gui/GuiGraphicsExtractor$HoveredTextEffects",
    "net/minecraft/client/gui/Font$DisplayMode",
];

/// The client engine (rendering, sound, window, the `Minecraft` object, the connection) and the
/// server: never loaded from the jar. Tier-1 objects inside allowed packages (`Level`, `Entity`,
/// `Player`, ...) are generated proxies, which win over the jar; see `proxies.conf`.
pub const DENY: &[&str] = &[
    "net/minecraft/client/",
    "net/minecraft/server/",
    "com/mojang/",
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
        let log = ClassLog::parse("jar\tnet.minecraft.core.BlockPos\nproxy\tnet.minecraft.client.Minecraft\nrefused: deny-listed\tnet.minecraft.world.level.Level\nlib\tcom.google.common.base.Preconditions\n");
        assert_eq!(log.classes["net.minecraft.core.BlockPos"], ClassSource::Jar);
        assert_eq!(
            log.with(|s| *s == ClassSource::Proxy),
            vec!["net.minecraft.client.Minecraft"]
        );
        assert_eq!(
            log.refused(),
            vec![("net.minecraft.world.level.Level", "deny-listed")]
        );
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
