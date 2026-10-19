//! The maps' plumbing between the game and render threads (like `pickup`): the game thread
//! publishes the session's map data once per tick, but only when a map's packets changed
//! it (`Session::maps_version` moved); the render thread hands the snapshot to the
//! renderer, which re-uploads only the changed maps' textures (vanilla's
//! `MapTextureManager` publishes the same way, through `update`'s dirty flag).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::shared::Shared;
use crate::net::NetDriver;

/// Copies the session's maps into the shared snapshot when they changed since the last
/// publish (a clone of the changed set only; nothing per frame).
pub(super) fn publish(driver: &NetDriver, shared: &Shared) {
    let version = driver.session.maps_version;
    if shared.maps_version.load(Ordering::Relaxed) == version {
        return;
    }
    let snapshot = Arc::new(driver.session.maps.clone());
    *shared.maps.lock() = snapshot;
    shared.maps_version.store(version, Ordering::Relaxed);
}
