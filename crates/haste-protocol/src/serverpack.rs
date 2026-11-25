//! Server resource packs: the download behind `resource_pack_push` (vanilla
//! `DownloadedPackSource` / `DownloadQueue` / `HttpUtil.downloadFile`).
//!
//! A pushed pack downloads over http(s) with the vanilla headers, the SHA-1 is
//! verified, and the file lands in the game dir's `downloads/<pack id>/<hash>`
//! (vanilla's cache layout, `Minecraft`'s `gameDir/downloads`). At most 20 pack
//! directories are kept (vanilla `DownloadCacheCleaner.MAX_KEPT_PACKS`), the
//! oldest first. The status packets (`minecraft:resource_pack`) are the caller's:
//! this module only downloads.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use sha1::{Digest as _, Sha1};

use crate::resource_pack_status as status;

/// Vanilla's `DownloadedPackSource` pack size cap (250 MB).
pub const MAX_PACK_SIZE_BYTES: u64 = 262_144_000;

/// 26.3's resource pack format (`SharedConstants.RESOURCE_PACK_FORMAT_MAJOR`),
/// sent in the `X-Minecraft-Pack-Format` download header.
pub const RESOURCE_PACK_FORMAT_MAJOR: u32 = 97;

/// Vanilla's `DownloadCacheCleaner.vacuumCacheDir` limit.
pub const MAX_KEPT_PACKS: usize = 20;

/// The request headers vanilla's downloader sends (`createDownloadHeaders`).
pub fn download_headers(
    username: &str,
    uuid: &str,
    version_name: &str,
    version_id: &str,
    pack_format: u32,
) -> Vec<(String, String)> {
    vec![
        ("X-Minecraft-Username".into(), username.into()),
        ("X-Minecraft-UUID".into(), uuid.into()),
        ("X-Minecraft-Version".into(), version_name.into()),
        ("X-Minecraft-Version-ID".into(), version_id.into()),
        ("X-Minecraft-Pack-Format".into(), pack_format.to_string()),
        (
            "User-Agent".into(),
            format!("Minecraft Java/{version_name}"),
        ),
    ]
}

/// A pack the server pushed and Haste downloaded (hash verified): the pack id
/// and the zip's path in the downloads cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadedPack {
    /// The `resource_pack_push` id.
    pub id: uuid::Uuid,
    /// The verified file (`downloads/<id>/<sha1>`).
    pub path: PathBuf,
}

/// Parses a `resource_pack_push` URL: vanilla's `parseResourcePackUrl` keeps
/// only http and https. `Err` carries the reason for INVALID_URL.
pub fn parse_pack_url(url: &str) -> Result<reqwest::Url, String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("{e}"))?;
    match parsed.scheme() {
        "http" | "https" => Ok(parsed),
        scheme => Err(format!("protocol {scheme:?}")),
    }
}

/// Whether a `resource_pack_push` hash is a SHA-1 (vanilla's `SHA1` pattern,
/// 40 hex digits; anything else means "no hash to check").
pub fn sha1_hex(hash: &str) -> Option<[u8; 20]> {
    if hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        let mut out = [0u8; 20];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hash[i * 2..i * 2 + 2], 16).ok()?;
        }
        Some(out)
    } else {
        None
    }
}

/// Downloads one pack into `downloads_dir` (vanilla `HttpUtil.downloadFile`):
/// a cached file with a matching hash is reused (and its mtime refreshed),
/// otherwise the URL is fetched, hashed while writing, size-capped, and moved
/// to `downloads/<id>/<hash>` (no hash: a temp name from the actual hash).
/// The pack directory is pruned to the newest files like the cache cleaner.
pub async fn download_pack(
    downloads_dir: &Path,
    id: uuid::Uuid,
    url: &reqwest::Url,
    want_hash: Option<[u8; 20]>,
    headers: &[(String, String)],
) -> Result<PathBuf> {
    let dir = downloads_dir.join(id.to_string());
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    match want_hash {
        Some(hash) => {
            let want_hex = hex(&hash);
            let target = dir.join(&want_hex);
            if file_sha1(&target).map(|h| h == hash).unwrap_or(false) {
                touch(&target);
                return Ok(target);
            }
            let _ = std::fs::remove_file(&target);
            let body = fetch(url, headers).await?;
            let actual = write_hashed(&target, &body)?;
            if actual != hash {
                let _ = std::fs::remove_file(&target);
                bail!(
                    "hash of downloaded pack {id} ({}) did not match requested ({want_hex})",
                    hex(&actual)
                );
            }
            Ok(target)
        }
        None => {
            // No requested hash: download to a temp file, name it by the actual hash
            // (vanilla `cachedFilePath(targetDir, actualHash)`).
            let body = fetch(url, headers).await?;
            let actual = {
                let mut hasher = Sha1::new();
                hasher.update(&body);
                let out: [u8; 20] = hasher.finalize().into();
                out
            };
            let target = dir.join(hex(&actual));
            let tmp = dir.join(format!("{}.tmp", hex(&actual)));
            std::fs::write(&tmp, &body).with_context(|| format!("writing {}", tmp.display()))?;
            let _ = std::fs::remove_file(&target);
            std::fs::rename(&tmp, &target)
                .with_context(|| format!("moving {}", target.display()))?;
            Ok(target)
        }
    }
}

/// GETs the URL, following redirects (vanilla `setInstanceFollowRedirects`),
/// enforcing the size cap via Content-Length and while reading.
async fn fetch(url: &reqwest::Url, headers: &[(String, String)]) -> Result<bytes::Bytes> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::default())
        .build()
        .context("building the pack download client")?;
    let mut request = client.get(url.clone());
    for (k, v) in headers {
        request = request.header(k, v);
    }
    let response = request.send().await.with_context(|| format!("GET {url}"))?;
    if !response.status().is_success() {
        bail!("GET {url}: HTTP {}", response.status());
    }
    if let Some(len) = response.content_length() {
        if len > MAX_PACK_SIZE_BYTES {
            bail!(
                "GET {url}: {len} bytes exceeds the {} byte pack limit",
                MAX_PACK_SIZE_BYTES
            );
        }
    }
    let body = response
        .bytes()
        .await
        .with_context(|| format!("reading {url}"))?;
    if body.len() as u64 > MAX_PACK_SIZE_BYTES {
        bail!(
            "GET {url}: {} bytes exceeds the {} byte pack limit",
            body.len(),
            MAX_PACK_SIZE_BYTES
        );
    }
    Ok(body)
}

/// Writes `body` to `target` and returns its SHA-1 (write-then-hash; packs are
/// small enough to hold in memory under the size cap).
fn write_hashed(target: &Path, body: &[u8]) -> Result<[u8; 20]> {
    std::fs::write(target, body).with_context(|| format!("writing {}", target.display()))?;
    Ok(sha1_of(body))
}

fn sha1_of(data: &[u8]) -> [u8; 20] {
    let mut hasher = Sha1::new();
    hasher.update(data);
    hasher.finalize().into()
}

fn file_sha1(path: &Path) -> Result<[u8; 20]> {
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(sha1_of(&data))
}

fn touch(path: &Path) {
    let now = filetime::FileTime::now();
    let _ = filetime::set_file_mtime(path, now);
}

/// Lowercase hex, vanilla `HashCode.toString`.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Prunes the downloads cache to the newest `MAX_KEPT_PACKS` pack directories
/// (vanilla `DownloadCacheCleaner.vacuumCacheDir`: oldest by mtime first).
pub fn vacuum(downloads_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(downloads_dir) else {
        return;
    };
    let mut dirs: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter_map(|p| {
            let mtime = std::fs::metadata(&p).ok()?.modified().ok()?;
            Some((mtime, p))
        })
        .collect();
    if dirs.len() <= MAX_KEPT_PACKS {
        return;
    }
    dirs.sort_by_key(|(mtime, _)| *mtime);
    let excess = dirs.len() - MAX_KEPT_PACKS;
    for (_, path) in dirs.into_iter().take(excess) {
        if let Err(e) = std::fs::remove_dir_all(&path) {
            tracing::warn!("pruning download dir {}: {e}", path.display());
        }
    }
}

/// Downloads an accepted batch of pushes (vanilla `ServerPackManager.updateDownloads`
/// hands every requested pack of a tick to one `DownloadQueue` batch): the
/// verified file of each, `None` where the download or the hash check failed.
/// The ids keep the batch's order.
pub async fn fetch_batch(
    downloads_dir: &Path,
    batch: &[(uuid::Uuid, String, String)],
    headers: &[(String, String)],
) -> Vec<(uuid::Uuid, Option<PathBuf>)> {
    let mut out = Vec::with_capacity(batch.len());
    for (id, url, hash) in batch {
        let path = match parse_pack_url(url) {
            Ok(url) => match download_pack(downloads_dir, *id, &url, sha1_hex(hash), headers).await
            {
                Ok(path) => Some(path),
                Err(e) => {
                    tracing::warn!("server resource pack {id} failed: {e:#}");
                    None
                }
            },
            Err(e) => {
                tracing::warn!("server resource pack {id}: invalid url: {e}");
                None
            }
        };
        out.push((*id, path));
    }
    out
}

/// The statuses after a batch's downloads finished, and the live set updated
/// (vanilla `ServerPackManager.onDownload`, then `triggerReloadIfNeeded` and
/// `cleanupRemovedPacks`; the ACCEPTED before the download is the caller's).
///
/// - Any failure fails the whole batch: the failed packs answer FAILED_DOWNLOAD
///   and the others DISCARDED, in batch order, and none is DOWNLOADED; the live
///   set stays as it was.
/// - Otherwise every pack answers DOWNLOADED in batch order, joins the live set
///   (a re-push of an id replaces the old one, which moves to the end), and the
///   reload that follows answers SUCCESSFULLY_LOADED for every pack in the live
///   set, in its order: vanilla's reload callback reports APPLIED for all packs
///   it loads, the ones that were already active included. Vanilla sends that
///   after its resource reload; Haste's reload is the client's (the pack reaches
///   it through [`DownloadedPack`]), so it is reported here.
pub fn settle_batch(
    fetched: Vec<(uuid::Uuid, Option<PathBuf>)>,
    live: &mut Vec<DownloadedPack>,
) -> Vec<(uuid::Uuid, i32)> {
    if fetched.iter().any(|(_, path)| path.is_none()) {
        return fetched
            .iter()
            .map(|(id, path)| {
                let result = if path.is_none() {
                    status::FAILED_DOWNLOAD
                } else {
                    status::DISCARDED
                };
                (*id, result)
            })
            .collect();
    }
    let mut out: Vec<(uuid::Uuid, i32)> = fetched
        .iter()
        .map(|(id, _)| (*id, status::DOWNLOADED))
        .collect();
    for (id, path) in fetched {
        live.retain(|p| p.id != id);
        live.push(DownloadedPack {
            id,
            path: path.expect("checked above"),
        });
    }
    out.extend(live.iter().map(|p| (p.id, status::SUCCESSFULLY_LOADED)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_parsing_follows_vanillas_pattern() {
        assert_eq!(
            sha1_hex("0beec7b5ea3f0fdbc95d0dd47f3c5bc275da8a33"),
            Some([
                0x0b, 0xee, 0xc7, 0xb5, 0xea, 0x3f, 0x0f, 0xdb, 0xc9, 0x5d, 0x0d, 0xd4, 0x7f, 0x3c,
                0x5b, 0xc2, 0x75, 0xda, 0x8a, 0x33
            ])
        );
        // Vanilla's SHA1 pattern: exactly 40 hex digits.
        assert_eq!(sha1_hex(""), None);
        assert_eq!(sha1_hex("xyz"), None);
        assert_eq!(sha1_hex("0beec7b5ea3f0fdbc95d0dd47f3c5bc275da8a3"), None);
        assert_eq!(sha1_hex("0beec7b5ea3f0fdbc95d0dd47f3c5bc275da8a333"), None);
    }

    #[test]
    fn urls_keep_only_http_and_https() {
        assert!(parse_pack_url("http://example.com/pack.zip").is_ok());
        assert!(parse_pack_url("https://example.com/pack.zip").is_ok());
        // Vanilla rejects other protocols with INVALID_URL.
        assert!(parse_pack_url("file:///etc/passwd").is_err());
        assert!(parse_pack_url("ftp://example.com/pack.zip").is_err());
        assert!(parse_pack_url("not a url").is_err());
    }

    #[test]
    fn a_failed_batch_discards_the_rest_and_a_good_one_reloads_every_pack() {
        let id = uuid::Uuid::from_u128;
        let path = |n: &str| Some(PathBuf::from(n));
        let mut live = vec![DownloadedPack {
            id: id(1),
            path: PathBuf::from("one"),
        }];
        // One failure fails the batch (vanilla `onDownload`): no DOWNLOADED,
        // the live set unchanged.
        let out = settle_batch(vec![(id(2), path("two")), (id(3), None)], &mut live);
        assert_eq!(
            out,
            vec![(id(2), status::DISCARDED), (id(3), status::FAILED_DOWNLOAD)]
        );
        assert_eq!(live.len(), 1);
        // A good batch: DOWNLOADED in batch order, then the reload answers
        // SUCCESSFULLY_LOADED for every live pack; a re-push moves to the end.
        let out = settle_batch(vec![(id(2), path("two")), (id(1), path("one'"))], &mut live);
        assert_eq!(
            out,
            vec![
                (id(2), status::DOWNLOADED),
                (id(1), status::DOWNLOADED),
                (id(2), status::SUCCESSFULLY_LOADED),
                (id(1), status::SUCCESSFULLY_LOADED),
            ]
        );
        assert_eq!(live[1].path, PathBuf::from("one'"));
    }

    #[test]
    fn vacuum_keeps_the_newest_twenty() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        // 25 pack dirs, one second apart (mtime resolution).
        for i in 0..25u64 {
            let p = dir.join(format!("{i:02x}"));
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("pack"), b"x").unwrap();
            let t = filetime::FileTime::from_unix_time(i as i64 * 10, 0);
            filetime::set_file_mtime(p.join("pack"), t).unwrap();
            filetime::set_file_mtime(&p, t).unwrap();
        }
        vacuum(dir);
        let kept: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(kept.len(), MAX_KEPT_PACKS);
        assert!(!kept.contains(&"00".to_owned()), "the oldest must go");
        assert!(kept.contains(&"18".to_owned()), "the newest stay");
    }
}
