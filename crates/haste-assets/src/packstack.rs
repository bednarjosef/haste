//! Resource packs on top of the vanilla jar (vanilla `PackRepository` /
//! `FolderRepositorySource` / `MultiPackResourceManager`): the jar stays the
//! bottom layer, then the selected packs from `<game dir>/resourcepacks`
//! (zip or folder) in vanilla's selection order. A pack higher in the stack
//! overrides the layers below (vanilla `FallbackResourceManager`).

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

mod layers;

/// A pack open for reading: a zip file or a plain folder. `asset_keys` lists
/// every `assets/<ns>/<path>` the pack provides.
enum PackFile {
    Zip {
        archive: zip::ZipArchive<std::fs::File>,
        /// `(key, archive index)` sorted by key (binary search; `zip` has no
        /// prefix iteration and file order differs from sorted order).
        keys: Vec<(String, usize)>,
    },
    Folder {
        /// Relative `assets/...` key -> absolute path, sorted by key.
        entries: Vec<(String, PathBuf)>,
    },
}

impl PackFile {
    fn read(&mut self, key: &str) -> Option<Vec<u8>> {
        match self {
            PackFile::Zip { archive, keys } => {
                let index = keys.binary_search_by(|(k, _)| k.as_str().cmp(key)).ok()?;
                let mut entry = archive.by_index(keys[index].1).ok()?;
                let mut buf = Vec::with_capacity(entry.size() as usize);
                entry.read_to_end(&mut buf).ok()?;
                Some(buf)
            }
            PackFile::Folder { entries, .. } => {
                let index = entries
                    .binary_search_by(|(k, _)| k.as_str().cmp(key))
                    .ok()?;
                std::fs::read(&entries[index].1).ok()
            }
        }
    }

    /// Whether this file provides `key` (no read).
    fn contains(&self, key: &str) -> bool {
        match self {
            PackFile::Zip { keys, .. } => {
                keys.binary_search_by(|(k, _)| k.as_str().cmp(key)).is_ok()
            }
            PackFile::Folder { entries } => entries
                .binary_search_by(|(k, _)| k.as_str().cmp(key))
                .is_ok(),
        }
    }

    /// Every `assets/...` key this file provides.
    fn keys(&self) -> Vec<&str> {
        match self {
            PackFile::Zip { keys, .. } => keys.iter().map(|(k, _)| k.as_str()).collect(),
            PackFile::Folder { entries, .. } => entries.iter().map(|(k, _)| k.as_str()).collect(),
        }
    }
}

/// One selected pack in the stack (vanilla `Pack`).
pub struct PackLayer {
    /// Id as vanilla shows it: `file/<name>` (vanilla `PackLocationInfo`).
    pub id: String,
    /// The `pack.mcmeta` `description` string (first line of a multi-line one).
    pub description: String,
    file: PackFile,
}

/// The packs on top of the vanilla jar, bottom (closest to the jar) first —
/// vanilla's selection order. An empty stack reads exactly like the plain
/// vanilla jar.
pub struct PackStack {
    packs: Vec<PackLayer>,
}

/// The `pack.mcmeta` `pack` section of a discovered pack (vanilla
/// `PackMetadataSection`): format range and description.
#[derive(Clone, Debug)]
pub struct PackInfo {
    /// The pack's id (`file/<folder or zip name>`).
    pub id: String,
    /// `description` from the `pack.mcmeta`.
    pub description: String,
    /// Inclusive min/max major format the pack declares (vanilla
    /// `PackMetadataSection.supportedFormats`; a missing mcmeta pack section
    /// or a legacy `pack_format` only both map here).
    pub min_format: Option<i64>,
    pub max_format: Option<i64>,
}

/// Haste's pack format (vanilla `SharedConstants.RESOURCE_PACK_FORMAT_MAJOR` = 97).
pub const RESOURCE_PACK_FORMAT: i64 = 97;

/// Vanilla `PackCompatibility.forVersion` (client resources, majors only):
/// unknown when the pack declares no version, too old/new by major range.
pub fn pack_compatibility(pack: &PackInfo) -> PackCompatibility {
    match (pack.min_format, pack.max_format) {
        (Some(min), Some(max)) => {
            if max < RESOURCE_PACK_FORMAT {
                PackCompatibility::TooOld
            } else if min > RESOURCE_PACK_FORMAT {
                PackCompatibility::TooNew
            } else {
                PackCompatibility::Compatible
            }
        }
        _ => PackCompatibility::Unknown,
    }
}

/// Vanilla `PackCompatibility` for the selection screen's "incompatible" badge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackCompatibility {
    Compatible,
    TooOld,
    TooNew,
    /// The pack declares no format at all.
    Unknown,
}

impl PackCompatibility {
    pub fn is_compatible(self) -> bool {
        self == PackCompatibility::Compatible
    }
}

impl PackStack {
    /// The empty stack: nothing but the vanilla jar.
    pub fn empty() -> PackStack {
        PackStack { packs: Vec::new() }
    }

    /// Builds a stack of packs by id (`file/<name>`) that exist in `packs_dir`
    /// (`<game dir>/resourcepacks`), in the given order (bottom first).
    /// Unknown ids are skipped (the pack was deleted, vanilla drops them from
    /// the selection too); unreadable packs are skipped with a warning.
    pub fn open(packs_dir: &Path, ids: &[String]) -> PackStack {
        PackStack::open_with_server(packs_dir, ids, &[])
    }

    /// [`PackStack::open`] with server resource packs layered above the folder
    /// packs (vanilla's `DOWNLOADED_PACK_SELECTION`: TOP, fixed): the zips the
    /// server pushed and Haste downloaded, in push order (bottom first). A
    /// pack the server pushed twice keeps only its latest download (vanilla
    /// `markExistingPacksAsRemoved`).
    pub fn open_with_server(packs_dir: &Path, ids: &[String], server: &[PathBuf]) -> PackStack {
        let mut packs = Vec::with_capacity(ids.len() + server.len());
        for id in ids {
            // The built-in vanilla pack (BuiltInPackSource.VANILLA_ID) is the client's own
            // assets, the base every stack sits on; options.txt lists it like any pack.
            if id == VANILLA_PACK_ID {
                continue;
            }
            match open_pack_layer(packs_dir, id) {
                Ok(Some(layer)) => packs.push(layer),
                Ok(None) => tracing::info!("resource pack {id} not found, skipping"),
                Err(e) => tracing::warn!("resource pack {id} unreadable, skipping: {e:#}"),
            }
        }
        for zip in server {
            match open_server_layer(zip) {
                Ok(layer) => {
                    // The same id pushed again replaces the earlier download.
                    packs.retain(|p| p.id != layer.id);
                    packs.push(layer);
                }
                Err(e) => {
                    tracing::warn!("server resource pack {} unreadable: {e:#}", zip.display())
                }
            }
        }
        PackStack { packs }
    }

    /// The stack's pack ids, bottom first.
    pub fn ids(&self) -> Vec<String> {
        self.packs.iter().map(|p| p.id.clone()).collect()
    }

    /// Whether no packs are layered (plain vanilla jar reading).
    pub fn is_empty(&self) -> bool {
        self.packs.is_empty()
    }

    /// Reads `assets/<ns>/<path>` from the pack that provides it, searching
    /// from the top of the stack down (vanilla `FallbackResourceManager`).
    pub(crate) fn read(&mut self, key: &str) -> Option<Vec<u8>> {
        self.packs.iter_mut().rev().find_map(|p| p.file.read(key))
    }

    /// Whether any pack in the stack provides `assets/<ns>/<path>` (no read).
    pub(crate) fn contains(&self, key: &str) -> bool {
        self.packs.iter().any(|p| p.file.contains(key))
    }

    /// Every `assets/<ns>/<path>` key any pack in the stack provides
    /// (unions the layers; values come from the topmost provider).
    pub(crate) fn keys(&self) -> Vec<String> {
        let mut seen: HashMap<String, ()> = HashMap::new();
        for pack in &self.packs {
            for key in pack.file.keys() {
                seen.insert(key.to_owned(), ());
            }
        }
        let mut keys: Vec<String> = seen.into_keys().collect();
        keys.sort();
        keys
    }
}

/// Vanilla `FolderRepositorySource.discoverPacks` + `PackDetector`: a pack is
/// a directory containing `pack.mcmeta`, or a `<name>.zip` file. Entries that
/// are neither are ignored.
pub fn discover_packs(packs_dir: &Path) -> Result<Vec<PackInfo>> {
    let dir = match std::fs::read_dir(packs_dir) {
        Ok(dir) => dir,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("listing {}", packs_dir.display())),
    };
    let mut packs = Vec::new();
    for entry in dir.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let info = if path.is_dir() {
            if !path.join("pack.mcmeta").is_file() {
                tracing::info!("not a pack (no pack.mcmeta), ignoring: {name}");
                continue;
            }
            read_folder_info(&path)
        } else if name.ends_with(".zip") {
            read_zip_info(&path)
        } else {
            tracing::info!("not a pack entry, ignoring: {name}");
            continue;
        };
        match info {
            Ok(info) => packs.push(info),
            Err(e) => tracing::warn!("ignoring unreadable pack {name}: {e:#}"),
        }
    }
    packs.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(packs)
}

/// Opens a pack by id (`file/<name>`) from `packs_dir` for layering.
/// The built-in vanilla pack's id in the selection (vanilla `BuiltInPackSource.VANILLA_ID`).
pub const VANILLA_PACK_ID: &str = "vanilla";

fn open_pack_layer(packs_dir: &Path, id: &str) -> Result<Option<PackLayer>> {
    let name = id
        .strip_prefix("file/")
        .context("pack id must start with file/")?;
    let path = packs_dir.join(name);
    let (description, file) = if path.is_dir() {
        if !path.join("pack.mcmeta").is_file() {
            return Ok(None);
        }
        let entries = list_folder_assets(&path)?;
        let PackMeta { description } = read_mcmeta(&path.join("pack.mcmeta"))?;
        (description, PackFile::Folder { entries })
    } else if name.ends_with(".zip") {
        if !path.is_file() {
            return Ok(None);
        }
        let file = std::fs::File::open(&path)
            .with_context(|| format!("opening pack {}", path.display()))?;
        let mut archive = zip::ZipArchive::new(file)
            .with_context(|| format!("reading zip {}", path.display()))?;
        let mut keys: Vec<(String, usize)> = archive
            .file_names()
            .enumerate()
            .filter(|(_, n)| n.starts_with("assets/") && !n.ends_with('/'))
            .map(|(i, n)| (n.to_owned(), i))
            .collect();
        keys.sort_by(|a, b| a.0.cmp(&b.0));
        let PackMeta { description } = read_mcmeta_zip(&mut archive)?;
        (description, PackFile::Zip { archive, keys })
    } else {
        return Ok(None);
    };
    Ok(Some(PackLayer {
        id: id.to_string(),
        description,
        file,
    }))
}

/// A pack's `pack.mcmeta` `pack` section as far as opening a layer needs it.
#[derive(Deserialize)]
struct PackMeta {
    #[serde(default)]
    description: String,
}

/// Opens a downloaded server pack (a zip at `downloads/<id>/<hash>`, no
/// extension; vanilla `FilePackResources.FileResourcesSupplier`). The layer's
/// id is `server/<uuid>` (vanilla's `server/<serial>/<uuid>`).
fn open_server_layer(zip: &Path) -> Result<PackLayer> {
    let file =
        std::fs::File::open(zip).with_context(|| format!("opening pack {}", zip.display()))?;
    let mut archive =
        zip::ZipArchive::new(file).with_context(|| format!("reading zip {}", zip.display()))?;
    let mut keys: Vec<(String, usize)> = archive
        .file_names()
        .enumerate()
        .filter(|(_, n)| n.starts_with("assets/") && !n.ends_with('/'))
        .map(|(i, n)| (n.to_owned(), i))
        .collect();
    keys.sort_by(|a, b| a.0.cmp(&b.0));
    let PackMeta { description } = read_mcmeta_zip(&mut archive)?;
    // The pack's directory name is its push id (downloads/<id>/<hash>).
    let id = zip
        .parent()
        .and_then(|d| d.file_name())
        .map(|n| format!("server/{}", n.to_string_lossy()))
        .unwrap_or_else(|| "server".to_owned());
    Ok(PackLayer {
        id,
        description,
        file: PackFile::Zip { archive, keys },
    })
}

fn read_mcmeta(path: &Path) -> Result<PackMeta> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
}

fn read_mcmeta_zip(archive: &mut zip::ZipArchive<std::fs::File>) -> Result<PackMeta> {
    let mut entry = archive
        .by_name("pack.mcmeta")
        .context("pack.mcmeta missing")?;
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut bytes)
        .context("reading pack.mcmeta")?;
    serde_json::from_slice(&bytes).context("parsing pack.mcmeta")
}

/// Vanilla's metadata failure rule (Pack.readMetaAndCreate): a `pack.mcmeta`
/// without a readable `pack` section means the entry is not a pack. Folder
/// discovery therefore treats a bad mcmeta as absent.
fn read_folder_info(root: &Path) -> Result<PackInfo> {
    let PackMetaFile { pack } = read_folder_mcmeta(root)?;
    let pack = pack.context("pack.mcmeta has no pack section")?;
    let (min_format, max_format) = pack.format_range();
    Ok(PackInfo {
        id: format!("file/{}", file_name(root)),
        description: pack.description,
        min_format,
        max_format,
    })
}

fn read_zip_info(path: &Path) -> Result<PackInfo> {
    let file =
        std::fs::File::open(path).with_context(|| format!("opening pack {}", path.display()))?;
    let mut archive =
        zip::ZipArchive::new(file).with_context(|| format!("reading zip {}", path.display()))?;
    let PackMetaFile { pack } = read_zip_mcmeta(&mut archive)?;
    let pack = pack.context("pack.mcmeta has no pack section")?;
    let (min_format, max_format) = pack.format_range();
    Ok(PackInfo {
        id: format!("file/{}", file_name(path)),
        description: pack.description,
        min_format,
        max_format,
    })
}

fn read_folder_mcmeta(root: &Path) -> Result<PackMetaFile> {
    let bytes = std::fs::read(root.join("pack.mcmeta"))
        .with_context(|| format!("reading {}/pack.mcmeta", root.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing {}/pack.mcmeta", root.display()))
}

fn read_zip_mcmeta(archive: &mut zip::ZipArchive<std::fs::File>) -> Result<PackMetaFile> {
    let mut entry = archive
        .by_name("pack.mcmeta")
        .context("pack.mcmeta missing")?;
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut bytes)
        .context("reading pack.mcmeta")?;
    serde_json::from_slice(&bytes).context("parsing pack.mcmeta")
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// `pack.mcmeta` root: `{ "pack": { ... } }`.
#[derive(Deserialize)]
struct PackMetaFile {
    pack: Option<PackMetaSection>,
}

/// The `pack` section (vanilla `PackMetadataSection`, client fields).
#[derive(Deserialize)]
struct PackMetaSection {
    #[serde(default)]
    description: String,
    /// Legacy single field (vanilla `pack_format`).
    pack_format: Option<i64>,
    /// Legacy range field (vanilla `supported_formats`).
    supported_formats: Option<SupportedFormats>,
    /// Current form (vanilla `min_format`/`max_format`).
    min_format: Option<i64>,
    max_format: Option<i64>,
}

/// `supported_formats` may be a single number or a `[min, max]` pair.
#[derive(Deserialize)]
#[serde(untagged)]
enum SupportedFormats {
    Single(i64),
    Range(Vec<i64>),
}

impl PackMetaSection {
    /// The declared major format range, or (None, None) when the pack
    /// declares nothing (vanilla: compatibility UNKNOWN).
    fn format_range(&self) -> (Option<i64>, Option<i64>) {
        if let (Some(min), Some(max)) = (self.min_format, self.max_format) {
            return (Some(min), Some(max));
        }
        match &self.supported_formats {
            Some(SupportedFormats::Single(n)) => (Some(*n), Some(*n)),
            Some(SupportedFormats::Range(v)) if v.len() == 2 => (Some(v[0]), Some(v[1])),
            _ => match self.pack_format {
                Some(n) => (Some(n), Some(n)),
                None => (None, None),
            },
        }
    }
}

/// Walks a pack folder collecting `assets/...` keys with their absolute paths
/// (files only, sorted by key). Symlinked files inside a folder pack are
/// followed by vanilla's PathPackResources (it walks the tree with
/// `Files.walk` semantics on the resolved root); we skip symlinked dirs.
fn list_folder_assets(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut entries = Vec::new();
    let assets = root.join("assets");
    if !assets.is_dir() {
        return Ok(entries);
    }
    walk(&assets, Path::new("assets"), &mut entries)?;
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    return Ok(entries);

    fn walk(dir: &Path, prefix: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<()> {
        for entry in std::fs::read_dir(dir)
            .with_context(|| format!("listing {}", dir.display()))?
            .flatten()
        {
            let path = entry.path();
            let name = entry.file_name();
            if name == ".DS_Store" {
                continue;
            }
            let rel = prefix.join(&*name);
            // Zip entries use `/`; keep keys canonical for the binary search.
            let key = rel.to_string_lossy().replace('\\', "/");
            if path.is_dir() {
                walk(&path, &rel, out)?;
            } else if path.is_file() {
                out.push((key, path));
            }
        }
        Ok(())
    }
}

// `bail!` keeps the discovery API total (a bad entry never fails the list).

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// Builds a temp dir holding a folder pack and a zip pack, each overriding
    /// `minecraft:textures/block/stone.png` (16x16 red) and adding a new texture.
    /// Returns (packs_dir, temp_dir_keeper).
    fn packs_fixture() -> (PathBuf, tempfile::TempDir) {
        let temp = tempfile::tempdir().unwrap();
        let packs_dir = temp.path().join("resourcepacks");
        std::fs::create_dir_all(packs_dir.join("RedPack")).unwrap();
        std::fs::write(
            packs_dir.join("RedPack/pack.mcmeta"),
            r#"{"pack":{"min_format":97,"max_format":97,"description":"Red pack"}}"#,
        )
        .unwrap();
        let red_png = solid_png(255, 0, 0);
        std::fs::create_dir_all(packs_dir.join("RedPack/assets/minecraft/textures/block")).unwrap();
        std::fs::write(
            packs_dir.join("RedPack/assets/minecraft/textures/block/stone.png"),
            red_png.as_slice(),
        )
        .unwrap();
        std::fs::create_dir_all(packs_dir.join("RedPack/assets/minecraft/textures/item")).unwrap();
        std::fs::write(
            packs_dir.join("RedPack/assets/minecraft/textures/item/gold_ingot.png"),
            red_png.as_slice(),
        )
        .unwrap();

        // Zip pack: overrides snow (blue) and gold_ingot (green, different key
        // through both packs).
        let zip_path = packs_dir.join("BluePack.zip");
        let blue_png = solid_png(0, 0, 255);
        let green_png = solid_png(0, 255, 0);
        {
            let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(
                std::fs::File::create(&zip_path).unwrap(),
            ));
            zip.start_file::<_, ()>("pack.mcmeta", Default::default())
                .unwrap();
            zip.write_all(
                br#"{"pack":{"min_format":97,"max_format":97,"description":"Blue pack"}}"#,
            )
            .unwrap();
            zip.start_file::<_, ()>(
                "assets/minecraft/textures/block/snow.png",
                Default::default(),
            )
            .unwrap();
            zip.write_all(blue_png.as_slice()).unwrap();
            zip.start_file::<_, ()>(
                "assets/minecraft/textures/item/gold_ingot.png",
                Default::default(),
            )
            .unwrap();
            zip.write_all(green_png.as_slice()).unwrap();
            zip.finish().unwrap();
        }

        // Non-pack entries: a folder without pack.mcmeta and a stray file.
        std::fs::create_dir_all(packs_dir.join("NotAPack")).unwrap();
        std::fs::write(packs_dir.join("readme.txt"), b"hello").unwrap();

        (packs_dir, temp)
    }

    /// A 16x16 solid RGBA png.
    fn solid_png(r: u8, g: u8, b: u8) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(16, 16, image::Rgba([r, g, b, 255]));
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        buf
    }

    #[test]
    fn discovery_finds_only_packs() {
        let (packs_dir, _temp) = packs_fixture();
        let packs = discover_packs(&packs_dir).unwrap();
        let ids: Vec<&str> = packs.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["file/BluePack.zip", "file/RedPack"]);
        assert_eq!(packs[0].description, "Blue pack");
        assert_eq!(packs[1].description, "Red pack");
        // Both declare pack_format 97 = our version: compatible.
        for pack in &packs {
            assert_eq!(pack_compatibility(pack), PackCompatibility::Compatible);
        }
    }

    #[test]
    fn compatibility_by_major_range() {
        let mk = |min: i64, max: i64| PackInfo {
            id: "file/x".into(),
            description: String::new(),
            min_format: Some(min),
            max_format: Some(max),
        };
        assert_eq!(
            pack_compatibility(&mk(97, 97)),
            PackCompatibility::Compatible
        );
        assert_eq!(pack_compatibility(&mk(1, 96)), PackCompatibility::TooOld);
        assert_eq!(pack_compatibility(&mk(98, 99)), PackCompatibility::TooNew);
        assert_eq!(
            pack_compatibility(&mk(90, 99)),
            PackCompatibility::Compatible
        );
        let unknown = PackInfo {
            id: "file/x".into(),
            description: String::new(),
            min_format: None,
            max_format: None,
        };
        assert_eq!(pack_compatibility(&unknown), PackCompatibility::Unknown);
    }

    #[test]
    fn stack_overrides_and_lists() {
        let (packs_dir, _temp) = packs_fixture();
        // Top of the stack = last in the list (BluePack.zip).
        let mut stack = PackStack::open(
            &packs_dir,
            &["file/RedPack".to_string(), "file/BluePack.zip".to_string()],
        );
        assert_eq!(stack.ids().len(), 2);
        // gold_ingot exists in both: the top (BluePack, green) wins.
        let gold = stack
            .read("assets/minecraft/textures/item/gold_ingot.png")
            .unwrap();
        let img = image::load_from_memory(&gold).unwrap().to_rgba8();
        assert_eq!(*img.get_pixel(0, 0), image::Rgba([0, 255, 0, 255]));
        // Keys: union of all packs' assets.
        let keys = stack.keys();
        assert!(keys.contains(&"assets/minecraft/textures/block/snow.png".to_string()));
        assert!(keys.contains(&"assets/minecraft/textures/item/gold_ingot.png".to_string()));
        assert!(!keys.contains(&"assets/minecraft/lang/en_us.json".to_string()));
    }

    #[test]
    fn server_packs_layer_above_folder_packs() {
        let (packs_dir, _temp) = packs_fixture();
        // A downloaded server pack (downloads/<id>/<sha1>, no extension) overriding the
        // folder packs' snow. The id is any push id (a uuid in practice).
        let downloads = _temp.path().join("downloads");
        let id = "12345678-1234-5678-1234-567812345678";
        let pack_dir = downloads.join(id);
        std::fs::create_dir_all(&pack_dir).unwrap();
        let server_zip = pack_dir.join("0123456789abcdef0123456789abcdef01234567");
        {
            let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(
                std::fs::File::create(&server_zip).unwrap(),
            ));
            zip.start_file::<_, ()>("pack.mcmeta", Default::default())
                .unwrap();
            zip.write_all(
                br#"{"pack":{"min_format":97,"max_format":97,"description":"server pack"}}"#,
            )
            .unwrap();
            zip.start_file::<_, ()>(
                "assets/minecraft/textures/block/snow.png",
                Default::default(),
            )
            .unwrap();
            zip.write_all(solid_png(180, 20, 220).as_slice()).unwrap();
            zip.finish().unwrap();
        }
        // The server pack overrides the folder packs' snow (BluePack's).
        let mut stack = PackStack::open_with_server(
            &packs_dir,
            &["file/RedPack".to_string(), "file/BluePack.zip".to_string()],
            std::slice::from_ref(&server_zip),
        );
        let snow = stack
            .read("assets/minecraft/textures/block/snow.png")
            .unwrap();
        let img = image::load_from_memory(&snow).unwrap().to_rgba8();
        assert_eq!(*img.get_pixel(0, 0), image::Rgba([180, 20, 220, 255]));
        // Its layer id names the push (server/<uuid>).
        assert_eq!(stack.ids().last().unwrap(), &format!("server/{id}"));
        // A folder pack's unique texture still resolves below it.
        let gold = stack
            .read("assets/minecraft/textures/item/gold_ingot.png")
            .unwrap();
        let img = image::load_from_memory(&gold).unwrap().to_rgba8();
        assert_eq!(*img.get_pixel(0, 0), image::Rgba([0, 255, 0, 255]));
        // The same id pushed again replaces the earlier download.
        let stack = PackStack::open_with_server(
            &packs_dir,
            &["file/RedPack".to_string()],
            &[server_zip.clone(), server_zip],
        );
        assert_eq!(
            stack.ids(),
            vec!["file/RedPack".to_string(), format!("server/{id}")]
        );
    }

    #[test]
    fn stack_skips_missing_and_keeps_order() {
        let (packs_dir, _temp) = packs_fixture();
        let stack = PackStack::open(
            &packs_dir,
            &[
                "file/BluePack.zip".to_string(),
                "file/Deleted".to_string(),
                "file/RedPack".to_string(),
            ],
        );
        assert_eq!(
            stack.ids(),
            vec!["file/BluePack.zip".to_string(), "file/RedPack".to_string()]
        );
        // The built-in vanilla id is the base assets, not a layer (and not an error).
        let base = PackStack::open(
            &packs_dir,
            &["vanilla".to_string(), "file/RedPack".to_string()],
        );
        assert_eq!(base.ids(), vec!["file/RedPack".to_string()]);
        let empty = PackStack::open(&packs_dir, &["file/Deleted".to_string()]);
        assert!(empty.ids().is_empty());
    }

    #[test]
    fn missing_dir_discovers_nothing() {
        let packs = discover_packs(Path::new("no/such/dir")).unwrap();
        assert!(packs.is_empty());
    }
}
