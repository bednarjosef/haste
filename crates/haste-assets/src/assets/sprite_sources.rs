//! Sprite id → texture key resolution through an atlas definition (vanilla
//! `SpriteSourceList` + `DirectoryLister`): `assets/minecraft/atlases/<atlas>.json`
//! lists sources, and every pack layer's definition is stacked on the jar's own
//! (vanilla reads the whole resource stack, so packs and mods add sources).
//! A `minecraft:directory` source lists `assets/<ns>/textures/<source>/<rest>.png`
//! as sprite id `<prefix><rest>` in every namespace — the gui atlas's `gui/sprites`
//! and `mob_effect` directories, or whatever a pack adds.

use serde::Deserialize;
use std::io::Read as _;

use super::Assets;
use haste_core::Identifier;

/// One directory source: the directory under `textures/` it lists and the prefix
/// prepended to the file path to make the sprite id (vanilla
/// `DirectoryLister(sourcePath, idPrefix)`).
pub type DirectorySource = (String, String);

/// `atlases/<name>.json`: the sprite sources of one atlas definition.
#[derive(Deserialize)]
struct AtlasDefinition {
    #[serde(default)]
    sources: Vec<AtlasSource>,
}

/// One source entry; only `minecraft:directory` sources map sprites to texture
/// files. Unknown source types are skipped, not failed on: a gui consumer must keep
/// serving the sprites it knows.
#[derive(Deserialize)]
struct AtlasSource {
    #[serde(rename = "type")]
    kind: String,
    source: String,
    #[serde(default)]
    prefix: String,
}

impl Assets {
    /// The `assets/<ns>/textures/<dir>/<rest>.png` key a sprite id's texture lives
    /// at, found through the atlas's directory sources: the sprite path with a
    /// source's prefix stripped is the file under the source's directory, in the
    /// sprite's own namespace (vanilla `DirectoryLister`, inverted; the namespace
    /// is how a mod's effect icon reaches the `mob_effect` directory). `None` when
    /// no source lists the sprite — callers fall back to their legacy guess.
    pub fn atlas_sprite_texture_key(&mut self, sprite: &Identifier, atlas: &str) -> Option<String> {
        for (dir, prefix) in self.directory_sources(atlas) {
            let Some(rest) = sprite.path().strip_prefix(prefix.as_str()) else {
                continue;
            };
            let key = format!(
                "assets/{}/textures/{}/{}.png",
                sprite.namespace(),
                dir,
                rest
            );
            if self.has_asset(&key) {
                return Some(key);
            }
        }
        None
    }

    /// The atlas's directory sources, stacked over every pack layer and the jar
    /// (vanilla `SpriteSourceList.load` over `getResourceStack`; bottom first).
    /// Cached until the pack selection changes.
    fn directory_sources(&mut self, atlas: &str) -> Vec<DirectorySource> {
        if let Some(sources) = self.atlas_sources.get(atlas) {
            return sources.clone();
        }
        let key = format!("assets/minecraft/atlases/{atlas}.json");
        let mut sources = Vec::new();
        for bytes in self.packs.read_all_layers(&key) {
            sources.extend(parse_directory_sources(&bytes, &key));
        }
        if let Ok(mut entry) = self.jar.by_name(&key) {
            let mut buf = Vec::with_capacity(entry.size() as usize);
            if entry.read_to_end(&mut buf).is_ok() {
                sources.extend(parse_directory_sources(&buf, &key));
            }
        }
        self.atlas_sources.insert(atlas.to_owned(), sources.clone());
        sources
    }
}

/// The `minecraft:directory` sources of one definition file; unreadable ones are
/// skipped with a warning (vanilla logs the error and keeps the other layers').
fn parse_directory_sources(bytes: &[u8], key: &str) -> Vec<DirectorySource> {
    let parsed: AtlasDefinition = match serde_json::from_slice(bytes) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("atlas definition {key} unreadable, skipping its sources: {e}");
            return Vec::new();
        }
    };
    parsed
        .sources
        .into_iter()
        .filter(|s| s.kind == "minecraft:directory")
        .map(|s| (s.source, s.prefix))
        .collect()
}
