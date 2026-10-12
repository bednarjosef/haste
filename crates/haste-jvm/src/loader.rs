//! Tier-3 class loading policy (passed to the Java `HasteClassLoader`) and its log.
//!
//! Patterns are JVM internal names. A pattern ending in `/` is a package prefix (subpackages
//! included); anything else is one class (its nested `$` classes included). The most specific
//! matching pattern decides (exact class > longer package prefix > shorter; deny wins ties), so a
//! value type inside a denied package can be allowed by name. A generated proxy wins over both.
//! Rationale for each entry is in `NOTES.md`.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Packages served as real vanilla Java from the client jar: stateless value types, chat, codecs,
/// the Screen/widget family and their helpers.
pub const ALLOW_PACKAGES: &[&str] = &[
    "net/minecraft/core/",
    "net/minecraft/resources/",
    "net/minecraft/network/chat/",
    "net/minecraft/network/codec/",
    "net/minecraft/tags/",
    "net/minecraft/commands/",
    "net/minecraft/network/protocol/",
    "net/minecraft/nbt/",
    "net/minecraft/util/",
    "net/minecraft/locale/",
    "net/minecraft/client/gui/components/",
    "net/minecraft/client/gui/screens/",
    "net/minecraft/client/gui/navigation/",
    "net/minecraft/client/gui/narration/",
    "net/minecraft/client/gui/layouts/",
    "net/minecraft/client/input/",
    "com/mojang/math/",
    "com/mojang/blaze3d/platform/cursor/",
];

/// Single classes served as real vanilla Java (outside the allowed packages).
pub const ALLOW_CLASSES: &[&str] = &[
    "net/minecraft/ChatFormatting",
    "net/minecraft/CharPredicate",
    "net/minecraft/IdentifierException",
    "net/minecraft/Optionull",
    "net/minecraft/SharedConstants",
    "net/minecraft/DetectedVersion",
    "net/minecraft/WorldVersion",
    "net/minecraft/CrashReport",
    "net/minecraft/CrashReportCategory",
    "net/minecraft/CrashReportDetail",
    "net/minecraft/ReportedException",
    "net/minecraft/ReportType",
    "net/minecraft/world/item/TooltipFlag",
    "net/minecraft/world/level/storage/DataVersion",
    "net/minecraft/server/packs/metadata/pack/PackFormat",
    "net/minecraft/server/ServerLinks",
    "net/minecraft/server/permissions/",
    "net/minecraft/client/multiplayer/chat/ChatAbilities",
    "net/minecraft/client/multiplayer/ClientSuggestionProvider",
    "net/minecraft/client/multiplayer/chat/ChatRestriction",
    "net/minecraft/client/multiplayer/chat/GuiMessage",
    "net/minecraft/client/multiplayer/chat/GuiMessageTag",
    "net/minecraft/client/multiplayer/chat/GuiMessageSource",
    "net/minecraft/network/ClientboundPacketListener",
    "net/minecraft/network/ServerboundPacketListener",
    "net/minecraft/network/PacketListener",
    "net/minecraft/network/TickablePacketListener",
    "net/minecraft/network/PacketSendListener",
    "net/minecraft/network/ConnectionProtocol",
    "net/minecraft/network/ProtocolInfo",
    "net/minecraft/network/DisconnectionDetails",
    "net/minecraft/network/LpVec3",
    "net/minecraft/network/SkipPacketException",
    "net/minecraft/network/SkipPacketDecoderException",
    "net/minecraft/network/SkipPacketEncoderException",
    "net/minecraft/network/FriendlyByteBuf",
    "net/minecraft/network/RegistryFriendlyByteBuf",
    "net/minecraft/network/VarInt",
    "net/minecraft/network/VarLong",
    "net/minecraft/network/Utf8String",
    "net/minecraft/network/HashedPatchMap",
    "net/minecraft/network/HashedStack",
    "net/minecraft/SystemReport",
    "net/minecraft/TracingExecutor",
    "net/minecraft/DefaultUncaughtExceptionHandler",
    "net/minecraft/DefaultUncaughtExceptionHandlerWithName",
    "net/minecraft/client/StringSplitter",
    "net/minecraft/client/InputType",
    "net/minecraft/client/MusicToastDisplayState",
    "net/minecraft/client/NarratorStatus",
    "net/minecraft/world/entity/player/ChatVisiblity",
    "net/minecraft/client/ComponentCollector",
    "net/minecraft/client/gui/ActiveTextCollector",
    "net/minecraft/client/gui/TextAlignment",
    "net/minecraft/client/gui/ComponentPath",
    "net/minecraft/client/gui/GuiGraphicsExtractor$HoveredTextEffects",
    "net/minecraft/client/gui/Font$DisplayMode",
];

/// Tier-1 types (stateful game objects owned by Rust) and engine types that must never be loaded
/// from the jar: they resolve to generated proxies, or fail and land in the log as a to-do.
pub const DENY: &[&str] = &[
    "net/minecraft/client/Minecraft",
    "net/minecraft/client/player/",
    "net/minecraft/client/multiplayer/",
    "net/minecraft/world/entity/",
    "net/minecraft/world/item/ItemStack",
    "net/minecraft/world/level/",
    "net/minecraft/server/",
    "net/minecraft/client/gui/GuiGraphicsExtractor",
    "net/minecraft/client/gui/Font",
    "net/minecraft/client/gui/Gui",
    "net/minecraft/client/gui/Hud",
    "net/minecraft/core/registries/BuiltInRegistries",
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
            let Some((how, name)) = line.split_once('\t') else { continue };
            let src = match how {
                "jar" => ClassSource::Jar,
                "proxy" => ClassSource::Proxy,
                "game" => ClassSource::Game,
                "lib" => ClassSource::Library,
                other => ClassSource::Refused(other.strip_prefix("refused: ").unwrap_or(other).to_owned()),
            };
            classes.insert(name.to_owned(), src);
        }
        ClassLog { classes }
    }
    /// Names with the given source.
    pub fn with(&self, pred: impl Fn(&ClassSource) -> bool) -> Vec<&str> {
        self.classes.iter().filter(|(_, s)| pred(s)).map(|(n, _)| n.as_str()).collect()
    }
    /// Refused classes with reasons.
    pub fn refused(&self) -> Vec<(&str, &str)> {
        self.classes
            .iter()
            .filter_map(|(n, s)| match s { ClassSource::Refused(r) => Some((n.as_str(), r.as_str())), _ => None })
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
                let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
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
        assert_eq!(log.with(|s| *s == ClassSource::Proxy), vec!["net.minecraft.client.Minecraft"]);
        assert_eq!(log.refused(), vec![("net.minecraft.world.level.Level", "deny-listed")]);
    }

    #[test]
    fn deny_covers_tier1_types() {
        for t in ["net/minecraft/client/Minecraft", "net/minecraft/world/item/ItemStack", "net/minecraft/client/player/"] {
            assert!(DENY.contains(&t), "{t}");
        }
        assert!(ALLOW_PACKAGES.iter().all(|p| p.ends_with('/')));
    }
}
