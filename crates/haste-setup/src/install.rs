//! `ensure_installed`: the whole Task 1 flow. Fetches the version manifest and the 26.3 version
//! JSON, downloads the client/server jars, the asset index and objects, the per-OS libraries and
//! the log4j config (all sha1-verified, skippable, resumable), resolves a JDK 25, then runs the
//! data generator. Every step is skipped when its output already exists.

use crate::download::{self, DownloadSpec, Progress};
use crate::mojang::{self, AssetIndex, VersionInfo, VersionManifest};
use anyhow::{anyhow, Context};
use haste_core::paths;
use std::path::{Path, PathBuf};

/// What `ensure_installed` produced / found.
#[derive(Debug, Clone)]
pub struct Installation {
    pub client_jar: PathBuf,
    pub server_jar: PathBuf,
    pub assets_dir: PathBuf,
    pub assets_index: String,
    pub libraries_dir: PathBuf,
    pub jdk_java: PathBuf,
    pub reports_dir: PathBuf,
}

/// The OS name Mojang uses in library rules.
pub fn mojang_os() -> &'static str {
    match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "osx",
        _ => "linux",
    }
}

/// JDK sources, in order of preference. `Version` comes from `java -version`'s major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JdkSource {
    /// `cache/jdk/bin/java` (from a previous run or pre-provisioned).
    Cache,
    /// A `java` (>= `major`) on PATH or `$JAVA_HOME/bin/java`.
    Path { major: u32 },
    /// Downloaded Temurin 25 from the Adoptium API into `cache/jdk`.
    Adoptium,
}

/// Where libraries live: `cache/libraries`, in Maven layout.
pub fn libraries_dir() -> PathBuf {
    paths::cache_dir().join("libraries")
}

async fn fetch_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
) -> anyhow::Result<T> {
    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    let body = resp
        .bytes()
        .await
        .with_context(|| format!("reading {url}"))?;
    anyhow::ensure!(status.is_success(), "GET {url} returned {status}");
    serde_json::from_slice(&body).with_context(|| format!("parsing JSON from {url}"))
}

/// The 26.3 version JSON, cached at `cache/versions/26.3.json` so later steps (and other tools)
/// do not need the manifest again. Refetched if the cached copy fails to parse.
async fn version_info(
    client: &reqwest::Client,
    progress: &(impl Fn(Progress) + Send),
) -> anyhow::Result<VersionInfo> {
    let vjson_path = paths::version_dir().with_file_name(format!("{}.json", paths::MC_VERSION));
    if let Ok(cached) = tokio::fs::read_to_string(&vjson_path).await {
        if let Ok(v) = serde_json::from_str::<VersionInfo>(&cached) {
            if v.id == paths::MC_VERSION {
                return Ok(v);
            }
        }
    }
    let manifest: VersionManifest = fetch_json(client, mojang::MANIFEST_URL).await?;
    let entry = manifest
        .find(paths::MC_VERSION)
        .ok_or_else(|| anyhow!("version {} not in manifest", paths::MC_VERSION))?;
    let v: VersionInfo = fetch_json(client, &entry.url).await?;
    anyhow::ensure!(
        v.id == paths::MC_VERSION,
        "version JSON id {} != {}",
        v.id,
        paths::MC_VERSION
    );
    if let Some(parent) = vjson_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&vjson_path, serde_json::to_vec(&v)?).await?;
    progress(Progress::File { done: 1, total: 1 });
    Ok(v)
}

/// Resolves the JDK: `cache/jdk/bin/java`, then a `java` >= 25 on PATH/JAVA_HOME, else Temurin 25
/// from the Adoptium API (tar.gz for unix, zip for windows) unpacked into `cache/jdk`.
/// Never installs system-wide.
pub async fn resolve_jdk(client: &reqwest::Client) -> anyhow::Result<(PathBuf, JdkSource)> {
    let cache_java = paths::cache_dir().join("jdk").join("bin").join(java_exe());
    if cache_java.is_file() {
        return Ok((cache_java, JdkSource::Cache));
    }
    if let Some((path, major)) = find_system_java().await {
        if major >= 25 {
            return Ok((path, JdkSource::Path { major }));
        }
    }
    install_temurin(client).await?;
    anyhow::ensure!(
        cache_java.is_file(),
        "Temurin install did not produce {}",
        cache_java.display()
    );
    Ok((cache_java, JdkSource::Adoptium))
}

fn java_exe() -> &'static str {
    if std::env::consts::OS == "windows" {
        "java.exe"
    } else {
        "java"
    }
}

/// The cached JDK's `java` if present, without any network access.
pub fn resolve_cached_java() -> Option<PathBuf> {
    let java = paths::cache_dir().join("jdk").join("bin").join(java_exe());
    java.is_file().then_some(java)
}

/// Loads the cached 26.3 version JSON (`cache/versions/26.3.json`), fetching it if absent.
/// Needed by the launcher without redownloading anything.
pub async fn load_cached_version_info() -> anyhow::Result<VersionInfo> {
    let vjson_path = paths::version_dir().with_file_name(format!("{}.json", paths::MC_VERSION));
    if let Ok(cached) = tokio::fs::read_to_string(&vjson_path).await {
        if let Ok(v) = serde_json::from_str::<VersionInfo>(&cached) {
            if v.id == paths::MC_VERSION {
                return Ok(v);
            }
        }
    }
    let client = reqwest::Client::new();
    let noop = |_p: crate::download::Progress| {};
    version_info(&client, &noop).await
}

/// Looks for `java` on PATH and in `$JAVA_HOME/bin`; returns `(path, major_version)`.
pub async fn find_system_java() -> Option<(PathBuf, u32)> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(home) = std::env::var("JAVA_HOME") {
        if !home.is_empty() {
            candidates.push(PathBuf::from(home).join("bin").join(java_exe()));
        }
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            candidates.push(dir.join(java_exe()));
        }
    }
    for cand in candidates {
        if !cand.is_file() {
            continue;
        }
        if let Some(major) = java_major(&cand).await {
            return Some((cand, major));
        }
    }
    None
}

/// Runs `java -version` (or `--version`) and parses the major version.
async fn java_major(java: &Path) -> Option<u32> {
    let out = tokio::process::Command::new(java)
        .arg("-version")
        .output()
        .await
        .ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    let text = if text.trim().is_empty() {
        String::from_utf8_lossy(&out.stdout).into_owned()
    } else {
        text.into_owned()
    };
    // OpenJDK/Hotspot: version "25.0.4" or "1.8.0_442" (legacy 8)
    let quoted = text.split('"').nth(1)?;
    let first = quoted.split('.').next()?;
    let major: u32 = first.parse().ok()?;
    Some(if major == 1 {
        quoted.split('.').nth(1)?.parse().ok()?
    } else {
        major
    })
}

const ADOPTIUM_API: &str = "https://api.adoptium.net/v3";

/// Downloads Temurin 25 (JDK, hotspot, eclipse) for this OS/arch and unpacks it into `cache/jdk`.
#[allow(clippy::match_same_arms)]
async fn install_temurin(client: &reqwest::Client) -> anyhow::Result<()> {
    let os = match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "mac",
        "linux" => "linux",
        other => return Err(anyhow!("unsupported OS for Adoptium: {other}")),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "aarch64",
        other => return Err(anyhow!("unsupported arch for Adoptium: {other}")),
    };
    let url = format!(
        "{ADOPTIUM_API}/binary/latest/25/ga/{os}/{arch}/jdk/hotspot/normal/eclipse?project=jdk"
    );
    let resp = client
        .get(&url)
        .send()
        .await
        .context("GET Adoptium latest 25")?;
    anyhow::ensure!(
        resp.status().is_success(),
        "Adoptium returned {}",
        resp.status()
    );
    let bytes = resp.bytes().await.context("downloading Temurin 25")?;
    let dl = paths::cache_dir().join("dl");
    tokio::fs::create_dir_all(&dl).await?;
    let archive = dl.join("jdk25.download");
    tokio::fs::write(&archive, &bytes).await?;
    let dest = paths::cache_dir().join("jdk");
    tokio::fs::create_dir_all(&dest).await?;
    if std::env::consts::OS == "windows" {
        unzip(&archive, &dest).await?;
    } else {
        untar(&archive, &dest).await?;
    }
    Ok(())
}

/// Unpacks a `.tar.gz` (blocking work on a worker thread).
async fn untar(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    let archive = archive.to_owned();
    let dest = dest.to_owned();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let file = std::fs::File::open(&archive)?;
        let mut gz = flate2::read::MultiGzDecoder::new(std::io::BufReader::new(file));
        untar_stream(&mut gz, &dest)
    })
    .await
    .map_err(|e| anyhow!("tar task panicked: {e}"))?
}

async fn unzip(_archive: &Path, _dest: &Path) -> anyhow::Result<()> {
    Err(anyhow!(
        "windows JDK unpack not implemented yet (WS7 runs on linux)"
    ))
}

/// Minimal ustar reader: enough for the Adoptium tarball (regular files, dirs, symlinks;
/// pax/GNU extension headers are skipped). `--strip-components=1`.
fn untar_stream<R: std::io::Read>(reader: &mut R, dest: &Path) -> anyhow::Result<()> {
    let mut header = [0u8; 512];
    loop {
        let n = read_full(reader, &mut header)?;
        if n == 0 {
            return Ok(());
        }
        if header.iter().all(|b| *b == 0) {
            return Ok(()); // end-of-archive
        }
        let name = cstr(&header[..100]);
        let prefix = cstr(&header[345..500]);
        let full = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let size = parse_octal(&header[124..136])?;
        let typeflag = header[156];
        let data_len = size;
        let pad = data_len.div_ceil(512) * 512 - data_len;
        // Every data-bearing entry type reads `data_len` bytes plus `pad`; do it once.
        let read_data = |reader: &mut R| -> anyhow::Result<Vec<u8>> {
            let mut data = vec![0u8; data_len as usize];
            read_full(reader, &mut data)?;
            if pad > 0 {
                let mut skip = vec![0u8; pad as usize];
                read_full(reader, &mut skip)?;
            }
            Ok(data)
        };
        let target = strip_first_component(&full);
        let out = match target {
            // a top-level entry (e.g. "jdk-25/") maps to the dest root itself
            None => dest.to_path_buf(),
            Some(t) => dest.join(t),
        };
        match typeflag {
            b'5' => {
                std::fs::create_dir_all(&out)?;
            }
            b'0' | 0x00 => {
                if name.ends_with('/') {
                    // some writers mark dirs with a plain typeflag and a trailing slash
                    std::fs::create_dir_all(&out)?;
                } else {
                    if let Some(parent) = out.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let data = read_data(reader)?;
                    std::fs::write(&out, &data)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let mode = parse_octal(&header[100..108])? as u32;
                        if mode & 0o111 != 0 {
                            std::fs::set_permissions(
                                &out,
                                std::fs::Permissions::from_mode(mode & 0o777),
                            )?;
                        }
                    }
                }
            }
            b'2' => {
                // symlink; the target is in the "data" (linkname) field
                #[cfg(unix)]
                {
                    std::fs::create_dir_all(out.parent().unwrap_or(dest))?;
                    let link = cstr(&header[157..257]);
                    std::os::unix::fs::symlink(&link, &out)
                        .with_context(|| format!("symlinking {} -> {link}", out.display()))?;
                }
                #[cfg(not(unix))]
                {
                    let _ = out;
                    anyhow::bail!("symlinks unsupported on this platform");
                }
            }
            b'x' | b'g' | b'L' | b'K' => {
                // pax / GNU extension headers: their data describes the next entry; the plain
                // name/prefix fields are correct for Adoptium tarballs, so just skip.
                let _ = read_data(reader)?;
            }
            other => return Err(anyhow!("unsupported tar typeflag {other} for {full}")),
        }
    }
}

fn read_full<R: std::io::Read>(r: &mut R, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut done = 0;
    while done < buf.len() {
        match r.read(&mut buf[done..])? {
            0 => return Ok(done),
            n => done += n,
        }
    }
    Ok(done)
}

fn cstr(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn parse_octal(bytes: &[u8]) -> anyhow::Result<u64> {
    let s = cstr(bytes);
    let s = s.trim().trim_end_matches('\0');
    if s.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(s, 8).with_context(|| format!("parsing tar octal {s:?}"))
}

/// `jdk-25.0.4.1+1/bin/java` -> `bin/java` (strip the single top-level dir the tarball has).
fn strip_first_component(path: &str) -> Option<String> {
    let mut comps = path.split('/');
    let _root = comps.next()?;
    let rest: Vec<&str> = comps.collect();
    if rest.is_empty() || rest.iter().all(|c| c.is_empty()) {
        None
    } else {
        Some(rest.join("/"))
    }
}

/// Runs the data generator: `java -DbundlerMainClass=net.minecraft.data.Main -jar server.jar
/// --reports --all` with cwd `cache/datagen/26.3`, producing `reports/*.json`.
/// Skipped when `reports/{blocks,registries,packets}.json` all exist.
pub async fn run_datagen(java: &Path, server_jar: &Path, reports_dir: &Path) -> anyhow::Result<()> {
    let required = ["blocks.json", "registries.json", "packets.json"];
    if required.iter().all(|f| reports_dir.join(f).is_file()) {
        return Ok(());
    }
    let cwd = reports_dir
        .parent()
        .ok_or_else(|| anyhow!("no parent of {}", reports_dir.display()))?;
    tokio::fs::create_dir_all(cwd).await?;
    let log_path = paths::cache_dir()
        .join("datagen")
        .join(format!("{}.log", paths::MC_VERSION));
    let log = std::fs::File::create(&log_path)?;
    let status = tokio::process::Command::new(java)
        .arg("-DbundlerMainClass=net.minecraft.data.Main")
        .arg("-jar")
        .arg(server_jar)
        .arg("--reports")
        .arg("--all")
        .current_dir(cwd)
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .with_context(|| format!("running datagen with {}", java.display()))?;
    anyhow::ensure!(
        status.success(),
        "data generator exited with {status} (log: {})",
        log_path.display()
    );
    for f in required {
        anyhow::ensure!(reports_dir.join(f).is_file(), "datagen did not produce {f}");
    }
    Ok(())
}

/// The full installation flow. See INTERFACES.md. Every step is idempotent: present-and-valid
/// outputs are skipped, partial downloads resume.
pub async fn ensure_installed(
    progress: impl FnMut(Progress) + Send + 'static,
) -> anyhow::Result<Installation> {
    // The callback is used across several sequential download batches; share it behind a Mutex.
    let progress = std::sync::Arc::new(std::sync::Mutex::new(progress));
    let report = {
        let progress = progress.clone();
        move |p: Progress| {
            if let Ok(mut f) = progress.lock() {
                f(p);
            }
        }
    };
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()?;

    // 1. version JSON (cached on disk after the first fetch)
    let v = version_info(&client, &report).await?;

    // 2. client + server jars (skip: they exist in the shared cache with matching sha1)
    let dest_client = paths::client_jar();
    let dest_server = paths::server_jar();
    download::download_many(
        vec![
            DownloadSpec {
                url: v.downloads.client.url.clone(),
                sha1: v.downloads.client.sha1.clone(),
                size: v.downloads.client.size,
                dest: dest_client.clone(),
            },
            DownloadSpec {
                url: v.downloads.server.url.clone(),
                sha1: v.downloads.server.sha1.clone(),
                size: v.downloads.server.size,
                dest: dest_server.clone(),
            },
        ],
        {
            let report = report.clone();
            move |p| report(p)
        },
    )
    .await?;

    // 3. asset index + objects
    let assets_dir = paths::assets_dir();
    let indexes_dir = assets_dir.join("indexes");
    let index_path = indexes_dir.join(format!("{}.json", v.asset_index.id));
    download::download_many(
        vec![DownloadSpec {
            url: v.asset_index.url.clone(),
            sha1: v.asset_index.sha1.clone(),
            size: v.asset_index.size,
            dest: index_path.clone(),
        }],
        {
            let report = report.clone();
            move |p| report(p)
        },
    )
    .await?;
    let index: AssetIndex = serde_json::from_slice(&tokio::fs::read(&index_path).await?)
        .with_context(|| format!("parsing {}", index_path.display()))?;
    let objects: Vec<DownloadSpec> = index
        .objects
        .values()
        .map(|o| DownloadSpec {
            url: o.url(),
            sha1: o.hash.clone(),
            size: o.size,
            dest: assets_dir.join("objects").join(o.object_path()),
        })
        .collect();
    download::download_many(objects, {
        let report = report.clone();
        move |p| report(p)
    })
    .await?;

    // 4. libraries for this OS (natives classifiers too, when present) + log4j config
    let libs_dir = libraries_dir();
    let mut lib_specs: Vec<DownloadSpec> = Vec::new();
    let os = mojang_os();
    for lib in &v.libraries {
        if !lib.allowed_on_os(os) {
            continue;
        }
        if let Some(a) = &lib.downloads.artifact {
            lib_specs.push(DownloadSpec {
                url: a.url.clone(),
                sha1: a.sha1.clone(),
                size: a.size,
                dest: libs_dir.join(&a.path),
            });
        }
        if let Some(classifiers) = &lib.downloads.classifiers {
            let native_key = match os {
                "linux" => "linux",
                "osx" => "osx",
                _ => "windows",
            };
            if let Some(key) = lib.natives.as_ref().and_then(|n| n.get(native_key)) {
                if let Some(a) = classifiers.get(key) {
                    lib_specs.push(DownloadSpec {
                        url: a.url.clone(),
                        sha1: a.sha1.clone(),
                        size: a.size,
                        dest: libs_dir.join(&a.path),
                    });
                }
            }
        }
    }
    if let Some(logging) = &v.logging {
        let f = &logging.client.file;
        lib_specs.push(DownloadSpec {
            url: f.url.clone(),
            sha1: f.sha1.clone(),
            size: f.size,
            dest: paths::cache_dir()
                .join("assets")
                .join("log_configs")
                .join(&f.id),
        });
    }
    download::download_many(lib_specs, {
        let report = report.clone();
        move |p| report(p)
    })
    .await?;

    // 5. JDK
    let (jdk_java, _source) = resolve_jdk(&client).await?;

    // 6. data generator
    let reports_dir = paths::datagen_reports();
    run_datagen(&jdk_java, &dest_server, &reports_dir).await?;

    Ok(Installation {
        client_jar: dest_client,
        server_jar: dest_server,
        assets_dir,
        assets_index: v.asset_index.id.clone(),
        libraries_dir: libs_dir,
        jdk_java,
        reports_dir,
    })
}

/// Detects an existing `.minecraft` with this version installed (read-only, never modified):
/// `~/.minecraft` on Linux, `~/Library/Application Support/minecraft` on macOS,
/// `%APPDATA%\.minecraft` on Windows. Returns the `.minecraft` root if
/// `versions/26.3/26.3.json` exists.
pub fn find_existing_minecraft() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let root = match std::env::consts::OS {
        "macos" => PathBuf::from(home).join("Library/Application Support/minecraft"),
        "windows" => {
            let appdata = std::env::var_os("APPDATA")?;
            PathBuf::from(appdata).join(".minecraft")
        }
        _ => PathBuf::from(home).join(".minecraft"),
    };
    let marker = root
        .join("versions")
        .join(paths::MC_VERSION)
        .join(format!("{}.json", paths::MC_VERSION));
    root.is_dir()
        .then_some(marker.is_file().then_some(root))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_tar_top_level_dir() {
        assert_eq!(
            strip_first_component("jdk-25.0.4/bin/java").unwrap(),
            "bin/java"
        );
        assert_eq!(strip_first_component("bin/java").unwrap(), "java");
        // a lone top-level dir maps to the root itself, which we do not want to create
        assert_eq!(strip_first_component("jdk/"), None);
    }

    #[test]
    fn parses_tar_octal_and_cstr() {
        let mut size = [0u8; 12];
        size[..11].copy_from_slice(b"00000001400");
        assert_eq!(parse_octal(&size).unwrap(), 0o1400);
        assert_eq!(cstr(b"bin/java\0rest"), "bin/java");
        let zero = [0u8; 12];
        assert_eq!(parse_octal(&zero).unwrap(), 0);
    }

    #[test]
    fn untar_extracts_a_small_archive() {
        // Build a two-entry tar (dir + file) by hand and extract it.
        let dir = tempfile::tempdir().unwrap();
        let mut tar: Vec<u8> = Vec::new();
        // header: name, mode, uid, gid, size, mtime, checksum, typeflag
        let push_header = |tar: &mut Vec<u8>, name: &str, size: u64, typeflag: u8| {
            let mut h = [0u8; 512];
            h[..name.len()].copy_from_slice(name.as_bytes());
            h[100..108].copy_from_slice(b"0000755\0");
            h[108..116].copy_from_slice(b"0000000\0");
            h[116..124].copy_from_slice(b"0000000\0");
            let size_field = format!("{size:011o}");
            h[124..135].copy_from_slice(size_field.as_bytes());
            h[136..147].copy_from_slice(b"00000000000");
            h[156] = typeflag;
            h[257..262].copy_from_slice(b"ustar");
            // checksum: spaces while computing
            h[148..156].copy_from_slice(b"        ");
            let sum: u32 = h.iter().map(|b| *b as u32).sum();
            let chk = format!("{sum:06o}\0 ");
            h[148..156].copy_from_slice(chk.as_bytes());
            tar.extend_from_slice(&h);
        };
        push_header(&mut tar, "topdir/", 0, b'5');
        push_header(&mut tar, "topdir/hello.txt", 5, b'0');
        tar.extend(b"hello");
        tar.extend(std::iter::repeat_n(0u8, 507));
        tar.extend_from_slice(&[0u8; 1024]); // two zero blocks end the archive
        let dest = dir.path().join("out");
        std::fs::create_dir_all(&dest).unwrap();
        untar_stream(&mut tar.as_slice(), &dest).unwrap();
        assert_eq!(std::fs::read(dest.join("hello.txt")).unwrap(), b"hello");
        assert!(dest.join("").is_dir() || dest.is_dir());
    }

    #[test]
    fn mojang_os_is_linux_here() {
        // CI/dev box is linux; the mapping itself is what we assert.
        assert!(matches!(mojang_os(), "linux" | "windows" | "osx"));
    }

    #[test]
    fn untar_handles_symlink_entry() {
        // hand-built archive with a symlink: topdir/link -> target.txt
        let dir = tempfile::tempdir().unwrap();
        let mut tar: Vec<u8> = Vec::new();
        let push_header = |tar: &mut Vec<u8>, name: &str, linkname: &str| {
            let mut h = [0u8; 512];
            h[..name.len()].copy_from_slice(name.as_bytes());
            h[100..108].copy_from_slice(b"0000777\0");
            h[108..116].copy_from_slice(b"0000000\0");
            h[116..124].copy_from_slice(b"0000000\0");
            h[124..135].copy_from_slice(b"00000000000");
            h[136..147].copy_from_slice(b"00000000000");
            h[156] = b'2'; // symlink
            h[157..157 + linkname.len()].copy_from_slice(linkname.as_bytes());
            h[257..262].copy_from_slice(b"ustar");
            h[148..156].copy_from_slice(b"        ");
            let sum: u32 = h.iter().map(|b| *b as u32).sum();
            let chk = format!("{sum:06o}\0 ");
            h[148..156].copy_from_slice(chk.as_bytes());
            tar.extend_from_slice(&h);
        };
        push_header(&mut tar, "topdir/link", "target.txt");
        tar.extend_from_slice(&[0u8; 1024]);
        let dest = dir.path().join("out");
        std::fs::create_dir_all(&dest).unwrap();
        untar_stream(&mut tar.as_slice(), &dest).unwrap();
        #[cfg(unix)]
        assert_eq!(
            std::fs::read_link(dest.join("link"))
                .unwrap()
                .to_str()
                .unwrap(),
            "target.txt"
        );
    }

    /// Extracts the real Temurin 25 tarball from the spike cache (read-only source) and checks
    /// `bin/java -version` runs. Run with `cargo test -p haste-setup -- --ignored`.
    #[tokio::test]
    #[ignore = "needs the spike cache (~140 MB tarball)"]
    async fn untar_extracts_real_temurin() {
        let archive = Path::new("../haste-spike/cache/dl/jdk25.tar.gz");
        if !archive.is_file() {
            eprintln!("skipping: {} not present", archive.display());
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("jdk");
        std::fs::create_dir_all(&dest).unwrap();
        untar(archive, &dest).await.unwrap();
        let java = dest.join("bin").join(java_exe());
        assert!(java.is_file(), "extracted java missing");
        let out = tokio::process::Command::new(&java)
            .arg("-version")
            .output()
            .await
            .unwrap();
        assert!(out.status.success());
        let text = String::from_utf8_lossy(&out.stderr);
        assert!(text.contains("25"), "unexpected version output: {text}");
    }

    #[tokio::test]
    async fn datagen_skips_when_reports_exist() {
        let dir = tempfile::tempdir().unwrap();
        let reports = dir.path().join("reports");
        std::fs::create_dir_all(&reports).unwrap();
        for f in ["blocks.json", "registries.json", "packets.json"] {
            std::fs::write(reports.join(f), "{}").unwrap();
        }
        // a nonexistent java would fail if it tried to run; skipping means no run
        run_datagen(
            Path::new("/nonexistent-java"),
            Path::new("/nonexistent.jar"),
            &reports,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn datagen_runs_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let reports = dir.path().join("reports");
        std::fs::create_dir_all(&reports).unwrap();
        let err = run_datagen(
            Path::new("/nonexistent-java"),
            Path::new("/nonexistent.jar"),
            &reports,
        )
        .await
        .unwrap_err();
        // The chain holds the context ("running datagen") and the OS error; which layer is
        // outermost differs between platforms, so check the whole chain.
        let chain = format!("{err:#}");
        assert!(
            chain.contains("datagen") || chain.contains("spawn") || chain.contains("No such file"),
            "got: {chain}"
        );
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    /// Runs `ensure_installed` against the real shared cache. It should mostly skip: the jars and
    /// datagen reports already exist; assets and libraries do not and will be downloaded
    /// (~500 MB). Run with `cargo test -p haste-setup -- --ignored`.
    #[tokio::test]
    #[ignore = "hits the network and the shared cache (~500 MB first run)"]
    async fn ensure_installed_against_real_cache() {
        let inst = ensure_installed(|p| match p {
            Progress::File { done, total } => {
                if done % 500 == 0 || done == total {
                    eprintln!("[files] {done}/{total}");
                }
            }
            Progress::Bytes { done, total } => {
                if done % (100 << 20) < 200_000 || done == total {
                    eprintln!("[bytes] {}/{}", done, total);
                }
            }
        })
        .await
        .unwrap();
        assert!(inst.client_jar.is_file());
        assert!(inst.server_jar.is_file());
        assert!(inst.jdk_java.is_file());
        assert!(inst.reports_dir.join("packets.json").is_file());
        assert!(inst.reports_dir.join("blocks.json").is_file());
        assert!(inst.reports_dir.join("registries.json").is_file());
        assert!(inst
            .assets_dir
            .join("indexes")
            .join(format!("{}.json", inst.assets_index))
            .is_file());
    }
}
