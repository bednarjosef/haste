//! Where singleplayer worlds live: `<config dir>/singleplayer/<world>/`, one folder per world,
//! world data (`level.dat`, `region/`) directly inside (the local server runs with
//! `--universe ..`). Both sides use this module: Rust (starting the local server for a world)
//! and Java's `LevelStorageSource` (listing worlds, whose base dir is
//! [`worlds_dir`], read through the `haste.singleplayerDir` system property).
use std::path::PathBuf;
use std::sync::OnceLock;

static OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// Points the singleplayer worlds somewhere else (`--singleplayer-dir`, the feature checks:
/// their worlds never land in the player's real saves). Before the first [`worlds_dir`].
pub fn set_dir(dir: PathBuf) {
    let _ = OVERRIDE.set(dir);
}

/// The directory that holds one folder per singleplayer world: the override when one was
/// set, else `$HASTE_SINGLEPLAYER_DIR`, else `<config dir>/singleplayer`.
#[must_use]
pub fn worlds_dir() -> Option<PathBuf> {
    if let Some(dir) = OVERRIDE.get() {
        return Some(dir.clone());
    }
    if let Some(dir) = std::env::var_os("HASTE_SINGLEPLAYER_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    Some(
        crate::servers::path()?
            .parent()
            .expect("servers.dat always has a parent")
            .to_path_buf(),
    )
    .map(|dir| dir.join("singleplayer"))
}

/// The world folder for `name` (whether or not it exists yet).
#[must_use]
pub fn world_dir(name: &str) -> Option<PathBuf> {
    Some(worlds_dir()?.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// `worlds_dir` reads `HASTE_SINGLEPLAYER_DIR`; the env test sets it, so the tests that read
    /// the dir must not run while it does (they run in parallel by default).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn world_dir_is_under_singleplayer() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = world_dir("gltest").expect("config dir exists on this system");
        let expected = std::path::Path::new("singleplayer").join("gltest");
        assert!(dir.ends_with(&expected));
        assert_ne!(world_dir("other"), Some(dir));
    }

    #[test]
    fn env_var_overrides_the_dir() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: one env var, set and restored around the calls (tests run in parallel
        // processes of this crate's test binary, but within it serially; the var is ours).
        let key = "HASTE_SINGLEPLAYER_DIR";
        let prev = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, "/tmp/haste-test-singleplayer");
        }
        assert_eq!(
            worlds_dir().as_deref(),
            Some(std::path::Path::new("/tmp/haste-test-singleplayer"))
        );
        match prev {
            Some(v) => unsafe {
                std::env::set_var(key, v);
            },
            None => unsafe {
                std::env::remove_var(key);
            },
        }
    }
}
