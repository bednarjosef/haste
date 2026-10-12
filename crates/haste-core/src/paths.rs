//! Where derived Mojang data lives. Never inside the repo tree except the gitignored `cache/`.
use std::path::PathBuf;

/// `$HASTE_CACHE`, else `~/.cache/haste`. Layout (owned by `haste-setup`):
/// `versions/26.3/client.jar`, `versions/26.3/server.jar`, `assets/`, `libraries/`,
/// `datagen/26.3/reports/{blocks,registries,packets}.json`, `oracle/26.3/*.json`, `jdk/`.
pub fn cache_dir() -> PathBuf {
    if let Ok(p) = std::env::var("HASTE_CACHE") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".cache").join("haste")
}

pub const MC_VERSION: &str = "26.3";

pub fn version_dir() -> PathBuf {
    cache_dir().join("versions").join(MC_VERSION)
}
pub fn client_jar() -> PathBuf {
    version_dir().join("client.jar")
}
pub fn server_jar() -> PathBuf {
    version_dir().join("server.jar")
}
pub fn datagen_reports() -> PathBuf {
    cache_dir().join("datagen").join(MC_VERSION).join("reports")
}
pub fn oracle_dir() -> PathBuf {
    cache_dir().join("oracle").join(MC_VERSION)
}
pub fn assets_dir() -> PathBuf {
    cache_dir().join("assets")
}
