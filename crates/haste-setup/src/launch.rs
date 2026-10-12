//! Launching the vanilla 26.3 client (needed by WS8's recorder and benchmarks) and optionally
//! with Fabric Loader, everything under `cache/`, never the user's `.minecraft`.

use crate::download::DownloadSpec;
use crate::mojang::VersionInfo;
use anyhow::{anyhow, Context};
use haste_core::paths;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Options for launching the (vanilla or Fabric) client.
#[derive(Debug, Clone)]
pub struct LaunchOptions {
    pub username: String,
    /// Offline UUID derived from the username when `None` (standard offline-mode v3 UUID).
    pub uuid: Option<String>,
    /// Any string for offline; a real MSA token otherwise.
    pub access_token: String,
    pub user_type: String,   // "legacy" for offline, "msa" for Microsoft
    pub version_type: String, // "release" or a custom label
    /// Install and launch Fabric Loader instead of the vanilla main class.
    pub fabric: bool,
    /// Extra JVM args (e.g. ["-Xmx4G"]).
    pub jvm_args: Vec<String>,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        LaunchOptions {
            username: "Player".into(),
            uuid: None,
            access_token: "0".into(),
            user_type: "legacy".into(),
            version_type: "release".into(),
            fabric: false,
            jvm_args: vec!["-Xmx4G".into()],
        }
    }
}

/// The game dir the client runs in: `cache/vanilla-game` (or `cache/fabric-game` with Fabric).
pub fn game_dir(fabric: bool) -> PathBuf {
    paths::cache_dir().join(if fabric { "fabric-game" } else { "vanilla-game" })
}

/// Offline-mode UUID (v3, MD5 of "OfflinePlayer:<name>"), matching vanilla servers' behavior.
pub fn offline_uuid(username: &str) -> String {
    let digest = md5(format!("OfflinePlayer:{username}").as_bytes());
    uuid::Builder::from_md5_bytes(digest).into_uuid().to_string()
}

/// Minimal MD5 (RFC 1321), used only for offline UUIDs (the `md-5` crate is not a workspace dep).
fn md5(input: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6,
        10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];
    let mut a: u32 = 0x6745_2301;
    let mut b: u32 = 0xefcd_ab89;
    let mut c: u32 = 0x98ba_dcfe;
    let mut d: u32 = 0x1032_5476;
    let mut msg = input.to_vec();
    let bitlen = (input.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bitlen.to_le_bytes());
    for chunk in msg.chunks(64) {
        let mut m = [0u32; 16];
        for (i, w) in m.iter_mut().enumerate() {
            *w = u32::from_le_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
        }
        let (mut aa, mut bb, mut cc, mut dd) = (a, b, c, d);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((bb & cc) | (!bb & dd), i),
                1 => ((dd & bb) | (!dd & cc), (5 * i + 1) % 16),
                2 => (bb ^ cc ^ dd, (3 * i + 5) % 16),
                _ => (cc ^ (bb | !dd), (7 * i) % 16),
            };
            let tmp = dd;
            dd = cc;
            cc = bb;
            let sum = aa
                .wrapping_add(f)
                .wrapping_add(K[i])
                .wrapping_add(m[g]);
            bb = bb.wrapping_add(sum.rotate_left(S[i]));
            aa = tmp;
        }
        a = a.wrapping_add(aa);
        b = b.wrapping_add(bb);
        c = c.wrapping_add(cc);
        d = d.wrapping_add(dd);
    }
    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&a.to_le_bytes());
    out[4..8].copy_from_slice(&b.to_le_bytes());
    out[8..12].copy_from_slice(&c.to_le_bytes());
    out[12..16].copy_from_slice(&d.to_le_bytes());
    out
}

/// The natives dir for this run: `cache/natives/<version>`, populated from the version's
/// natives-containing libraries (26.3 ships none, so this is usually just created).
fn extract_natives(v: &VersionInfo, natives_dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(natives_dir)?;
    let os = crate::install::mojang_os();
    for lib in &v.libraries {
        let Some(classifiers) = &lib.downloads.classifiers else { continue };
        let Some(natives) = &lib.natives else { continue };
        let key = match os {
            "linux" => "linux",
            "osx" => "osx",
            _ => "windows",
        };
        let Some(classifier) = natives.get(key) else { continue };
        let Some(artifact) = classifiers.get(classifier) else { continue };
        let jar_path = crate::install::libraries_dir().join(&artifact.path);
        if !jar_path.is_file() {
            continue; // not downloaded for this OS
        }
        let file = std::fs::File::open(&jar_path)?;
        let mut zip = zip::ZipArchive::new(file)?;
        zip.extract(natives_dir)?;
    }
    Ok(())
}

/// Builds the classpath: the version's OS-allowed library jars plus the client jar.
pub fn classpath(v: &VersionInfo) -> anyhow::Result<String> {
    let os = crate::install::mojang_os();
    let libs = crate::install::libraries_dir();
    let mut parts = Vec::new();
    for lib in &v.libraries {
        if !lib.allowed_on_os(os) {
            continue;
        }
        if let Some(a) = &lib.downloads.artifact {
            let p = libs.join(&a.path);
            if p.is_file() {
                parts.push(p);
            }
        }
    }
    parts.push(paths::client_jar());
    Ok(std::env::join_paths(parts)?.into_string().expect("classpath is valid UTF-8 paths"))
}

/// The substituted game arguments for the vanilla (or Fabric Knot) main class.
fn game_args(opts: &LaunchOptions, game_dir: &Path, index_id: &str) -> Vec<String> {
    [
        "--username", &opts.username,
        "--version", paths::MC_VERSION,
        "--gameDir", &game_dir.to_string_lossy(),
        "--assetsDir", &paths::assets_dir().to_string_lossy(),
        "--assetIndex", index_id,
        "--uuid", &opts.uuid.clone().unwrap_or_else(|| offline_uuid(&opts.username)),
        "--accessToken", &opts.access_token,
        "--userType", &opts.user_type,
        "--versionType", &opts.version_type,
    ]
    .into_iter()
    .map(|s| s.to_string())
    .collect()
}

fn jvm_args(v: &VersionInfo, natives_dir: &Path, cp: &str, extra: &[String]) -> Vec<String> {
    let _ = v;
    let mut args: Vec<String> = vec![
        "-Djava.library.path=".to_string() + &natives_dir.join("java").to_string_lossy(),
        "-Djna.tmpdir=".to_string() + &natives_dir.join("jna").to_string_lossy(),
        "-Dorg.lwjgl.system.SharedLibraryExtractPath=".to_string()
            + &natives_dir.join("lwjgl").to_string_lossy(),
        "-Dio.netty.native.workdir=".to_string() + &natives_dir.join("netty").to_string_lossy(),
        "--enable-native-access=ALL-UNNAMED".into(),
        "-cp".into(),
        cp.into(),
    ];
    args.extend(extra.iter().cloned());
    args
}

/// A launched client process. Killing it is the caller's business; the game dir (and logs) stay.
pub struct LaunchedClient {
    pub child: std::process::Child,
    pub game_dir: PathBuf,
    pub log_path: PathBuf,
}

/// Launches the client as configured. Vanilla: `net.minecraft.client.main.Main` from the version
/// JSON. Fabric: installs the Fabric Loader (meta.fabricmc.net profile for this version) into
/// the same game dir and launches `net.fabricmc.loader.impl.launch.knot.KnotClient` with the
/// loader libraries; mods go in `<game_dir>/mods/`.
pub async fn launch_vanilla(opts: LaunchOptions) -> anyhow::Result<LaunchedClient> {
    let v: VersionInfo = crate::install::load_cached_version_info().await?;
    let dir = game_dir(opts.fabric);
    std::fs::create_dir_all(&dir)?;
    let natives_dir = paths::cache_dir().join("natives").join(paths::MC_VERSION);
    extract_natives(&v, &natives_dir)?;

    let (main_class, classpath_str, extra_jvm) = if opts.fabric {
        let fabric = install_fabric(&v, &dir).await?;
        let mut cp = fabric.libraries.clone();
        cp.push(paths::client_jar());
        (
            "net.fabricmc.loader.impl.launch.knot.KnotClient".to_string(),
            std::env::join_paths(cp)?.into_string().unwrap(),
            fabric.jvm_args,
        )
    } else {
        (v.main_class.clone().unwrap_or_else(|| "net.minecraft.client.main.Main".into()), classpath(&v)?, vec![])
    };

    let java = crate::install::resolve_cached_java()
        .ok_or_else(|| anyhow!("no cache/jdk/bin/java; run ensure_installed first"))?;

    let log_path = dir.join("client.log");
    let log = std::fs::File::create(&log_path).with_context(|| format!("creating {}", log_path.display()))?;
    let mut cmd = std::process::Command::new(&java);
    cmd.args(jvm_args(&v, &natives_dir, &classpath_str, &{
        let mut a = opts.jvm_args.clone();
        a.extend(extra_jvm);
        a
    }))
    .arg(main_class)
    .args(game_args(&opts, &dir, &v.asset_index.id))
    .current_dir(&dir)
    .stdin(Stdio::null())
    .stdout(Stdio::from(log))
    .stderr(Stdio::inherit());
    let child = cmd.spawn().with_context(|| format!("launching client with {}", java.display()))?;
    Ok(LaunchedClient { child, game_dir: dir, log_path })
}

const FABRIC_META: &str = "https://meta.fabricmc.net/v2";

struct FabricInstall {
    libraries: Vec<PathBuf>,
    jvm_args: Vec<String>,
}

/// Installs the Fabric Loader for this MC version into the game dir (profile JSON + libraries
/// into cache/libraries), returning the loader library paths and JVM args. Idempotent.
async fn install_fabric(_v: &VersionInfo, game_dir: &Path) -> anyhow::Result<FabricInstall> {
    let profile_url = format!("{FABRIC_META}/versions/loader/{}?limit=1", paths::MC_VERSION);
    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .get(&profile_url)
        .send()
        .await
        .context("GET fabric meta loader version")?
        .json()
        .await
        .context("parsing fabric meta response")?;
    let version = resp[0]["loader"]["version"]
        .as_str()
        .ok_or_else(|| anyhow!("fabric meta returned no loader version"))?
        .to_string();

    // the profile JSON tells us the libraries and main class
    let profile_json_url = format!("{FABRIC_META}/versions/loader/{}/{}/profile/json", paths::MC_VERSION, version);
    let profile: serde_json::Value = client
        .get(&profile_json_url)
        .send()
        .await
        .context("GET fabric profile json")?
        .json()
        .await
        .context("parsing fabric profile json")?;
    std::fs::create_dir_all(game_dir)?;
    std::fs::write(
        game_dir.join(format!("fabric-loader-{}.json", paths::MC_VERSION)),
        serde_json::to_vec_pretty(&profile)?,
    )?;

    let libs_dir = crate::install::libraries_dir();
    let mut specs = Vec::new();
    let mut libraries = Vec::new();
    if let Some(libs) = profile["libraries"].as_array() {
        for lib in libs {
            let name = lib["name"].as_str().context("fabric library without name")?;
            let url = lib["url"].as_str().unwrap_or("https://maven.fabricmc.net/").to_string();
            let path = crate::mojang::maven_path(name)
                .ok_or_else(|| anyhow!("unparseable fabric library name {name}"))?;
            let dest = libs_dir.join(&path);
            // the profile carries sha1 for all libraries except fabric-loader itself
            let sha1 = lib["sha1"].as_str().unwrap_or("").to_string();
            if !dest.exists() {
                // '+' is legal in Maven artifact names but must be percent-encoded in URLs
                let dl = format!("{url}{}", path.replace('+', "%2B"));
                specs.push(DownloadSpec { url: dl, sha1, size: 0, dest: dest.clone() });
            }
            libraries.push(dest);
        }
    }
    if !specs.is_empty() {
        crate::download::download_many(specs, |_| {}).await?;
    }

    let mut fabric_jvm = vec!["-Dfabric.development=false".to_string()];
    if let Some(args) = profile["arguments"]["jvm"].as_array() {
        for a in args {
            if let Some(s) = a.as_str() {
                fabric_jvm.push(s.to_string());
            }
        }
    }
    Ok(FabricInstall { libraries, jvm_args: fabric_jvm })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_uuid_matches_known_value() {
        // v3 UUID of MD5("OfflinePlayer:Notch"), matching Java's UUID.nameUUIDFromBytes
        assert_eq!(offline_uuid("Notch"), "b50ad385-829d-3141-a216-7e7d7539ba7f");
        assert_eq!(offline_uuid("Player"), "a01e3843-e521-3998-958a-f459800e4d11");
        assert_ne!(offline_uuid("Notch"), offline_uuid("Steve"));
    }

    #[test]
    fn md5_matches_rfc_vectors() {
        assert_eq!(hex::encode(md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex::encode(md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            hex::encode(md5(b"The quick brown fox jumps over the lazy dog")),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
    }

    #[test]
    fn game_args_shape() {
        let dir = Path::new("/tmp/g");
        let args = game_args(
            &LaunchOptions { username: "Steve".into(), ..Default::default() },
            dir,
            "34",
        );
        let joined = args.join(" ");
        assert!(joined.contains("--username Steve"));
        assert!(joined.contains("--assetIndex 34"));
        assert!(joined.contains("--gameDir /tmp/g"));
    }

    /// Launches the real vanilla client against the cache. On a headless box it dies at window
    /// creation (GLFW), which still proves the JVM, classpath, natives and game args are right.
    /// Run with `cargo test -p haste-setup -- --ignored`.
    #[tokio::test]
    #[ignore = "needs the full cache (jars+assets+libraries+jdk); opens/tries to open a window"]
    async fn launches_vanilla_client_headless() {
        if crate::install::resolve_cached_java().is_none() {
            eprintln!("skipping: no cache/jdk");
            return;
        }
        let mut client = launch_vanilla(LaunchOptions {
            username: "HasteTest".into(),
            ..Default::default()
        })
        .await
        .expect("launch");
        // give it time to get past class loading
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        let status = client.child.try_wait().expect("child is ours");
        let log = std::fs::read_to_string(&client.log_path).unwrap_or_default();
        eprintln!("--- client.log (first 2000 chars) ---\n{}", &log[..log.len().min(2000)]);
        match status {
            None => {
                // still running (window may have opened on this box); kill it
                let _ = client.child.kill();
                // running this far means Main loaded fine
            }
            Some(code) => {
                // must NOT be a classpath/JVM failure (those exit with code 1 and a stack trace
                // about ClassNotFound / module issues); GLFW/display failures are acceptable.
                let bad = log.contains("ClassNotFoundException")
                    || log.contains("NoClassDefFoundError")
                    || log.contains("Error occurred during initialization of VM");
                assert!(!bad, "client failed to boot the JVM/classpath (exit {code}): {log}");
            }
        }
    }

    /// Installs Fabric Loader (meta.fabricmc.net profile for 26.3) and launches KnotClient.
    /// Verifies the loader downloads (percent-encoded '+' URLs) and starts.
    /// Run with `cargo test -p haste-setup -- --ignored`.
    #[tokio::test]
    #[ignore = "network (fabric meta + maven) + the full cache; opens/tries to open a window"]
    async fn launches_fabric_client() {
        if crate::install::resolve_cached_java().is_none() {
            eprintln!("skipping: no cache/jdk");
            return;
        }
        let mut client = launch_vanilla(LaunchOptions {
            username: "HasteTest".into(),
            fabric: true,
            ..Default::default()
        })
        .await
        .expect("launch");
        tokio::time::sleep(std::time::Duration::from_secs(25)).await;
        let status = client.child.try_wait().expect("child is ours");
        let log = std::fs::read_to_string(&client.log_path).unwrap_or_default();
        eprintln!("--- fabric client.log (first 2000 chars) ---\n{}", &log[..log.len().min(2000)]);
        let bad = log.contains("ClassNotFoundException")
            || log.contains("NoClassDefFoundError")
            || log.contains("Error occurred during initialization of VM")
            || log.contains("failed to download");
        if status.is_none() {
            let _ = client.child.kill();
        }
        assert!(!bad, "fabric client failed to boot (status {status:?}): {log}");
    }
}
