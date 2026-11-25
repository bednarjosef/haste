// This file is derived from Pomme (https://github.com/PommeMC/Client),
// pomme-client/src/assets.rs and parts of world/block/model.rs, reused under
// GPL-3.0. Reworked for Haste: reads the client jar directly through `zip`
// (no extracted assets dir), resolves texture variables, and exposes the
// haste-core `Identifier` API from docs/INTERFACES.md. See docs/THIRD_PARTY.md.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::model::ModelFile;

/// Open client jar + assets index/objects directory (the `Installation::assets_dir` +
/// `assets_index` from haste-setup). The jar's own `assets/` tree takes precedence when
/// the index is missing an entry; in 26.3 the client jar carries the full vanilla
/// asset tree, the index only adds the same files by hash (kept for pack parity).
pub struct Assets {
    jar: zip::ZipArchive<std::fs::File>,
    /// `<assets_dir>/objects` from the launcher download; may be absent.
    objects_dir: Option<PathBuf>,
    /// Parsed `<assets_dir>/indexes/<index>.json`, when present.
    index_hashes: HashMap<String, String>,
    /// Selected resource packs layered on top of the jar (vanilla
    /// `PackRepository` selection; empty = vanilla only).
    packs: crate::packstack::PackStack,
    /// Where the packs were read from (vanilla's `resourcepacks` folder).
    packs_dir: PathBuf,
    /// The server's downloaded packs layered above the selection.
    server_packs: Vec<PathBuf>,
    model_cache: HashMap<String, Arc<ModelFile>>,
    /// Directory sources per atlas definition (vanilla `SpriteSourceList`), cached
    /// until the pack selection changes.
    atlas_sources: HashMap<String, Vec<sprite_sources::DirectorySource>>,
    /// Where this was opened from (`reopen`).
    source: (PathBuf, PathBuf, String),
    /// The pack ids `reopen` re-applies (empty when opened plain).
    pack_ids: Vec<String>,
    /// Runtime-registered textures (downloaded skins), kept across `reopen`.
    pub(crate) dynamic: crate::dynamic::DynamicTextures,
}

pub(crate) fn asset_key(id: &haste_core::Identifier, category: &str, suffix: &str) -> String {
    format!(
        "assets/{}/{}/{}{}",
        id.namespace(),
        category,
        id.path(),
        suffix
    )
}

pub mod language;
pub mod models;
pub mod resources;
pub mod sprite_sources;
pub mod textures;

pub use models::ModelJson;
pub use textures::{
    frame_at, Animation, AnimationFrame, AnimationMeta, AnimationSchedule, SpriteMeta, TextureMeta,
};

#[cfg(test)]
mod tests;
