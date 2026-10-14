//! Where singleplayer worlds live: `<config dir>/singleplayer/<world>/`, one folder per world,
//! world data (`level.dat`, `region/`) directly inside (the local server runs with
//! `--universe ..`). Both sides use this module: Rust (starting the local server for a world)
//! and Java's `LevelStorageSource` (listing worlds, whose base dir is
//! [`worlds_dir`], read through the `haste.singleplayerDir` system property).
use std::path::PathBuf;

/// The directory that holds one folder per singleplayer world.
#[must_use]
pub fn worlds_dir() -> Option<PathBuf> {
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

    #[test]
    fn world_dir_is_under_singleplayer() {
        let dir = world_dir("gltest").expect("config dir exists on this system");
        let expected = std::path::Path::new("singleplayer").join("gltest");
        assert!(dir.ends_with(&expected));
        assert_ne!(world_dir("other"), Some(dir));
    }
}
