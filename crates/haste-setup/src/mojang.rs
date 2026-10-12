//! Serde types for Mojang's version manifest (`version_manifest_v2.json`), the per-version JSON
//! and the asset index. Field names follow piston-meta; everything is `snake_case` there except
//! a few camelCase names captured with explicit `rename`s.
//!
//! These are minimal: only what `install` needs (downloads, asset index, per-OS libraries).

use serde::{Deserialize, Serialize};

pub const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

/// `version_manifest_v2.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionManifest {
    pub latest: LatestVersions,
    pub versions: Vec<ManifestVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatestVersions {
    pub release: String,
    pub snapshot: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestVersion {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String, // "release", "snapshot", ...
    pub url: String,
    #[serde(default)]
    pub release_time: String,}

impl VersionManifest {
    pub fn find(&self, id: &str) -> Option<&ManifestVersion> {
        self.versions.iter().find(|v| v.id == id)
    }
}

/// Per-version JSON (e.g. `26.3.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
    pub id: String,
    #[serde(rename = "mainClass", default)]
    pub main_class: Option<String>,
    pub downloads: VersionDownloads,
    #[serde(rename = "assetIndex", alias = "asset_index")]
    pub asset_index: AssetIndexMeta,
    #[serde(default)]
    pub assets: String,
    #[serde(rename = "javaVersion", alias = "java_version", default)]
    pub java_version: Option<JavaVersion>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default)]
    pub logging: Option<LoggingInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionDownloads {
    pub client: Download,
    pub server: Download,
}

/// A Mojang download: `url` is authoritative, `sha1`/`size` are for verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Download {
    pub url: String,
    pub sha1: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetIndexMeta {
    pub id: String,
    pub url: String,
    pub sha1: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub total_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JavaVersion {
    pub component: String,
    #[serde(rename = "majorVersion", alias = "major_version")]
    pub major_version: u32,
}

/// One `libraries[]` entry. `rules` gate the platform; `downloads.artifact` is the main jar,
/// `downloads.classifiers` the platform natives (absent for 26.3 but kept for other versions).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Library {
    pub name: String,
    #[serde(default)]
    pub downloads: LibraryDownloads,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<Vec<Rule>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub natives: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>, // maven repo base for libs without per-artifact urls
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibraryDownloads {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<Artifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifiers: Option<std::collections::BTreeMap<String, Artifact>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub path: String, // Maven path relative to the libraries dir
    pub url: String,
    pub sha1: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub action: RuleAction, // "allow" or "deny"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<OsRule>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsRule {
    pub name: String, // "linux", "windows", "osx", ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
}

impl Library {
    /// Does this library's rule list allow the given OS (`"linux"`, `"windows"`, `"osx"`)?
    /// No rules means allowed everywhere; otherwise the last matching rule wins.
    pub fn allowed_on_os(&self, os: &str) -> bool {
        let Some(rules) = &self.rules else { return true };
        let mut allowed = false; // rules start from a "deny all" state
        for r in rules {
            let matches = match &r.os {
                Some(o) => o.name == os,
                None => true,
            };
            if matches {
                allowed = r.action == RuleAction::Allow;
            }
        }
        allowed
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingInfo {
    pub client: LogConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    pub argument: String, // "-Dlog4j.configurationFile=${path}"
    #[serde(rename = "file")]
    pub file: Download,
    #[serde(rename = "type")]
    pub kind: String, // "log4j2-xml"
}

/// The asset index JSON (`34.json` for 26.3): sha1 -> {hash, size}.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetIndex {
    pub objects: std::collections::BTreeMap<String, AssetObject>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetObject {
    pub hash: String,
    #[serde(default)]
    pub size: u64,
}

impl AssetObject {
    /// Where this object lives under `assets/objects/`: first two hex chars, then the full hash.
    pub fn object_path(&self) -> String {
        format!("{}/{}", &self.hash[..2], self.hash)
    }

    /// Download URL on Mojang's resources CDN.
    pub fn url(&self) -> String {
        format!("https://resources.download.minecraft.net/{}/{}", &self.hash[..2], self.hash)
    }
}

/// Parses a library `name` (`group:artifact:version`) into a Maven path
/// `group/with/slashes/artifact/version/artifact-version[-suffix].jar`.
/// Returns None on names that are not `g:a:v`.
pub fn maven_path(name: &str) -> Option<String> {
    let mut parts = name.split(':');
    let group = parts.next()?;
    let artifact = parts.next()?;
    let version = parts.next()?;
    let suffix = parts.next(); // classifier, may be absent
    let group_path = group.replace('.', "/");
    let file = match suffix {
        Some(s) => format!("{artifact}-{version}-{s}.jar"),
        None => format!("{artifact}-{version}.jar"),
    };
    Some(format!("{group_path}/{artifact}/{version}/{file}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{
        "latest": {"release": "26.3", "snapshot": "26.3"},
        "versions": [
            {"id": "26.3", "type": "release", "url": "https://piston-meta.mojang.com/v1/packages/abc/26.3.json", "releaseTime": "2025-10-01T10:00:00Z"},
            {"id": "25w10a", "type": "snapshot", "url": "https://example.invalid/25w10a.json"}
        ]
    }"#;

    #[test]
    fn parses_manifest() {
        let m: VersionManifest = serde_json::from_str(MANIFEST).unwrap();
        assert_eq!(m.latest.release, "26.3");
        assert_eq!(m.versions.len(), 2);
        let v = m.find("26.3").expect("26.3 present");
        assert_eq!(v.kind, "release");
        assert_eq!(v.url, "https://piston-meta.mojang.com/v1/packages/abc/26.3.json");
        assert!(m.find("26.4").is_none());
    }

    const VERSION: &str = r#"{
        "id": "26.3",
        "mainClass": "net.minecraft.client.main.Main",
        "downloads": {
            "client": {"sha1": "aa", "size": 100, "url": "https://piston-data.mojang.com/v1/objects/aa/client.jar"},
            "server": {"sha1": "bb", "size": 200, "url": "https://piston-data.mojang.com/v1/objects/bb/server.jar"}
        },
        "assetIndex": {"id": "34", "sha1": "cc", "size": 5, "totalSize": 500, "url": "https://piston-meta.mojang.com/v1/packages/cc/34.json"},
        "assets": "34",
        "javaVersion": {"component": "java-runtime-epsilon", "majorVersion": 25},
        "libraries": [
            {"name": "at.yawk.lz4:lz4-java:1.10.1",
             "downloads": {"artifact": {"path": "at/yawk/lz4/lz4-java/1.10.1/lz4-java-1.10.1.jar", "url": "https://libraries.minecraft.net/at/yawk/lz4/lz4-java/1.10.1/lz4-java-1.10.1.jar", "sha1": "dd", "size": 1}}},
            {"name": "ca.weblite:java-objc-bridge:1.1",
             "downloads": {"artifact": {"path": "ca/weblite/java-objc-bridge/1.1/java-objc-bridge-1.1.jar", "url": "https://example.invalid/objc.jar", "sha1": "ee", "size": 2}},
             "rules": [{"action": "allow", "os": {"name": "osx"}}]},
            {"name": "ca.weblite:java-objc-bridge:1.1",
             "downloads": {"artifact": {"path": "ca/weblite/java-objc-bridge/1.1/java-objc-bridge-1.1.jar", "url": "https://example.invalid/objc.jar", "sha1": "ee", "size": 2}},
             "rules": [{"action": "allow"}, {"action": "deny", "os": {"name": "osx"}}]}
        ],
        "logging": {"client": {"argument": "-Dlog4j.configurationFile=${path}", "file": {"id": "client-1.21.2.xml", "sha1": "ff", "size": 3, "url": "https://example.invalid/log.xml"}, "type": "log4j2-xml"}}
    }"#;

    #[test]
    fn parses_version_info() {
        let v: VersionInfo = serde_json::from_str(VERSION).unwrap();
        assert_eq!(v.id, "26.3");
        assert_eq!(v.downloads.client.sha1, "aa");
        assert_eq!(v.downloads.server.size, 200);
        assert_eq!(v.asset_index.id, "34");
        assert_eq!(v.java_version.as_ref().unwrap().major_version, 25);
        assert_eq!(v.libraries.len(), 3);
        let log = v.logging.as_ref().unwrap().client.clone();
        assert_eq!(log.kind, "log4j2-xml");
        assert_eq!(log.file.sha1, "ff");
    }

    #[test]
    fn library_os_rules() {
        let v: VersionInfo = serde_json::from_str(VERSION).unwrap();
        assert!(v.libraries[0].allowed_on_os("linux"));
        assert!(v.libraries[0].allowed_on_os("osx"));
        // osx-only
        assert!(!v.libraries[1].allowed_on_os("linux"));
        assert!(v.libraries[1].allowed_on_os("osx"));
        // allow-all then deny osx
        assert!(v.libraries[2].allowed_on_os("linux"));
        assert!(!v.libraries[2].allowed_on_os("osx"));
    }

    #[test]
    fn parses_asset_index() {
        let idx: AssetIndex = serde_json::from_str(
            r#"{"objects": {
                "minecraft/sounds/random/click.ogg": {"hash": "0cabc...def0", "size": 42},
                "minecraft/textures/block/stone.png": {"hash": "1234567890abcdef", "size": 7}
            }}"#,
        )
        .unwrap();
        assert_eq!(idx.objects.len(), 2);
        let o = &idx.objects["minecraft/textures/block/stone.png"];
        assert_eq!(o.object_path(), "12/1234567890abcdef");
        assert_eq!(o.url(), "https://resources.download.minecraft.net/12/1234567890abcdef");
    }

    #[test]
    fn maven_paths() {
        assert_eq!(
            maven_path("at.yawk.lz4:lz4-java:1.10.1").unwrap(),
            "at/yawk/lz4/lz4-java/1.10.1/lz4-java-1.10.1.jar"
        );
        assert_eq!(
            maven_path("org.lwjgl:lwjgl:3.3.1:natives-linux").unwrap(),
            "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-linux.jar"
        );
        assert_eq!(
            maven_path("com.mojang:logging:1.2.7").unwrap(),
            "com/mojang/logging/1.2.7/logging-1.2.7.jar"
        );
        assert!(maven_path("not-a-name").is_none());
    }
}
