// Derived from Pomme (GPL-3.0); see docs/THIRD_PARTY.md.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use super::Assets;

#[derive(Deserialize)]
struct AssetIndexFile {
    objects: HashMap<String, AssetIndexObject>,
}

#[derive(Deserialize)]
struct AssetIndexObject {
    hash: String,
}

impl Assets {
    pub fn open(client_jar: &Path, assets_dir: &Path, assets_index: &str) -> Result<Assets> {
        let source = (
            client_jar.to_path_buf(),
            assets_dir.to_path_buf(),
            assets_index.to_string(),
        );
        let file = std::fs::File::open(client_jar)
            .with_context(|| format!("opening client jar {}", client_jar.display()))?;
        let mut jar = zip::ZipArchive::new(file)
            .with_context(|| format!("reading zip {}", client_jar.display()))?;

        let objects_dir = assets_dir.join("objects");
        let mut index_path = assets_dir
            .join("indexes")
            .join(format!("{assets_index}.json"));
        if !index_path.exists() {
            // The launcher names the index by its asset-index id (`34`), not the game version;
            // with a single index present that is the one.
            if let Ok(dir) = std::fs::read_dir(assets_dir.join("indexes")) {
                let found: Vec<PathBuf> = dir
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "json"))
                    .collect();
                if found.len() == 1 {
                    index_path = found[0].clone();
                }
            }
        }
        let index_hashes = match std::fs::read(&index_path) {
            Ok(bytes) => {
                let parsed: AssetIndexFile = serde_json::from_slice(&bytes)
                    .with_context(|| format!("parsing asset index {}", index_path.display()))?;
                parsed
                    .objects
                    .into_iter()
                    .map(|(k, v)| (k, v.hash))
                    .collect()
            }
            Err(e) => {
                // 26.3's client jar carries the whole vanilla asset tree; the index
                // is an alternative view. Soft-fail so tests can run on the jar alone.
                tracing::warn!(
                    "asset index {} unreadable ({e}); using jar assets only",
                    index_path.display()
                );
                HashMap::new()
            }
        };
        let objects_dir = objects_dir.is_dir().then_some(objects_dir);

        if jar
            .by_name("assets/minecraft/blockstates/stone.json")
            .is_err()
        {
            bail!("client jar does not contain assets/minecraft/blockstates");
        }
        let _ = jar
            .by_name("assets/minecraft/lang/en_us.json")
            .context("client jar does not contain assets/minecraft/lang/en_us.json")?;

        Ok(Assets {
            jar,
            objects_dir,
            index_hashes,
            packs: crate::packstack::PackStack::empty(),
            packs_dir: PathBuf::new(),
            server_packs: Vec::new(),
            model_cache: HashMap::new(),
            atlas_sources: HashMap::new(),
            dynamic: Default::default(),
            source,
            pack_ids: Vec::new(),
        })
    }

    /// A second handle on the same jar and assets (the renderer keeps one for lazy loads).
    /// Re-applies the same pack stack when one was applied.
    pub fn reopen(&self) -> Result<Assets> {
        let mut assets = Assets::open(&self.source.0, &self.source.1, &self.source.2)?;
        if !self.pack_ids.is_empty() || !self.server_packs.is_empty() {
            assets.apply_server_packs(&self.packs_dir, &self.pack_ids, &self.server_packs);
        }
        assets.dynamic = self.dynamic.clone();
        Ok(assets)
    }

    /// Layers the selected resource packs on top of the jar (bottom first),
    /// reading them from `packs_dir` (vanilla's `<game dir>/resourcepacks`).
    /// Must be called before any asset is read; clears the model cache of
    /// pack-provided models.
    pub fn apply_packs(&mut self, packs_dir: &Path, ids: &[String]) {
        self.apply_server_packs(packs_dir, ids, &[]);
    }

    /// [`Assets::apply_packs`] with the server's downloaded packs layered above
    /// the selection (vanilla's `DownloadedPackSource` sits at the top of the
    /// repository): the zips the server pushed, in push order.
    pub fn apply_server_packs(&mut self, packs_dir: &Path, ids: &[String], server: &[PathBuf]) {
        self.packs_dir = packs_dir.to_path_buf();
        self.packs = crate::packstack::PackStack::open_with_server(packs_dir, ids, server);
        self.pack_ids = ids.to_vec();
        self.server_packs = server.to_vec();
        self.model_cache.clear();
        self.atlas_sources.clear();
    }

    /// The pack ids currently layered (bottom first), as `apply_packs` got them.
    pub fn pack_ids(&self) -> &[String] {
        &self.pack_ids
    }

    /// The server packs currently layered above the selection, as
    /// `apply_server_packs` got them.
    pub fn server_packs(&self) -> &[PathBuf] {
        &self.server_packs
    }

    /// The directory packs were layered from (vanilla's `resourcepacks` folder).
    pub fn packs_dir(&self) -> &Path {
        &self.packs_dir
    }
    /// File names directly under `dir` (an `assets/...` key prefix) ending with `ext`, path
    /// relative to `dir`, without the extension.
    pub fn names_in(&mut self, dir: &str, ext: &str) -> impl Iterator<Item = String> {
        self.names_under(dir, ext)
    }

    /// File names directly under `dir` ending with `ext`, path relative to `dir`.
    pub(super) fn names_under(&mut self, dir: &str, ext: &str) -> impl Iterator<Item = String> {
        let mut names: Vec<String> = self
            .jar
            .file_names()
            .filter(|n| n.starts_with(dir) && n.ends_with(ext))
            .filter_map(|n| {
                n.strip_prefix(dir)
                    .and_then(|rest| rest.strip_suffix(ext))
                    .filter(|rest| !rest.contains('/'))
                    .map(str::to_owned)
            })
            .collect();
        // Packs add entries (vanilla `FallbackResourceManager.listResources`):
        // union of all layers, deduped.
        if !self.packs.is_empty() {
            for key in self.packs.keys() {
                if let Some(name) = key
                    .strip_prefix(dir)
                    .and_then(|rest| rest.strip_suffix(ext))
                {
                    if !name.contains('/') && !names.iter().any(|n| n == name) {
                        names.push(name.to_owned());
                    }
                }
            }
        }
        names.into_iter()
    }

    /// Asset bytes by key (`assets/<ns>/<path>`), preferring the jar, falling back to
    /// the index/objects dir.
    pub fn read_asset(&mut self, key: &str) -> Result<Vec<u8>> {
        self.try_read_asset(key)?
            .with_context(|| format!("asset {key} not found"))
    }

    /// Whether the asset `key` (`assets/<ns>/<path>`) exists in a selected pack, the
    /// downloaded objects or the jar, without reading it: the same sources, in the same
    /// order, as [`Assets::read_asset`] (for load-time checks like vanilla's
    /// `SoundManager.validateSoundResource`).
    pub fn has_asset(&self, key: &str) -> bool {
        if self.packs.contains(key) {
            return true;
        }
        let index_key = key.strip_prefix("assets/").unwrap_or(key);
        if let (Some(objects), Some(hash)) = (&self.objects_dir, self.index_hashes.get(index_key)) {
            if objects.join(&hash[..2]).join(hash).is_file() {
                return true;
            }
        }
        self.jar.index_for_name(key).is_some()
    }

    /// The downloaded assets (index + objects) take precedence over the jar: the jar carries
    /// stubs for what the launcher downloads (`font/include/unifont.json` is empty in the jar).
    /// Index keys have no `assets/` prefix. Selected resource packs override both (vanilla
    /// `FallbackResourceManager`: top of the stack wins).
    pub(crate) fn try_read_asset(&mut self, key: &str) -> Result<Option<Vec<u8>>> {
        if !self.packs.is_empty() {
            if let Some(bytes) = self.packs.read(key) {
                return Ok(Some(bytes));
            }
        }
        let index_key = key.strip_prefix("assets/").unwrap_or(key);
        if let (Some(objects), Some(hash)) = (&self.objects_dir, self.index_hashes.get(index_key)) {
            let path = objects.join(&hash[..2]).join(hash);
            if let Ok(bytes) = std::fs::read(&path) {
                return Ok(Some(bytes));
            }
            tracing::warn!("indexed asset {key} missing at {}", path.display());
        }
        if let Ok(mut entry) = self.jar.by_name(key) {
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut buf)?;
            return Ok(Some(buf));
        }
        Ok(None)
    }
}
