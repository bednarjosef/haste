//! User config: TOML in the platform config dir, marked 0600 (it will hold MSA tokens;
//! encrypting them at rest is out of scope).

use crate::options::Options;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// MSA tokens as consumed by protocol's `Account::Microsoft`. The struct is coordinated through the
/// lead; this is haste-setup's copy of the shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MsaTokens {
    /// The Minecraft access token (`Authorization: Bearer ...`).
    pub access_token: String,
    /// Microsoft refresh token, used to refresh the access token.
    #[serde(default)]
    pub refresh_token: String,
    /// Expiry (unix seconds) of the access token.
    #[serde(default)]
    pub expires_at: Option<i64>,
    /// The player profile the tokens belong to.
    #[serde(default)]
    pub profile: Option<MsaProfile>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MsaProfile {
    pub uuid: String,
    pub name: String,
}

/// The player's profile key pair for chat signing on online-mode servers
/// (`POST /player/certificates`, minecraft.wiki "Mojang API"). Keys are the raw DER bytes of
/// the PEM the API returns; `expires_at`/`refreshed_after` are unix seconds. Cached in the
/// config until `refreshed_after` (see `msa::ensure_fresh_account`). No crypto crate: Haste
/// hands the DER bytes and signatures to protocol as-is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PlayerCertificates {
    /// RSA private key, DER (PEM armour stripped and base64-decoded by `msa::parse_player_certificates`).
    pub private_key_der: Vec<u8>,
    /// RSA public key, DER.
    pub public_key_der: Vec<u8>,
    /// Mojang's signature (v1) over the public key, base64-decoded; may be empty (absent in
    /// newer responses).
    #[serde(default, with = "serde_bytes_base64")]
    pub public_key_signature: Vec<u8>,
    /// Mojang's signature (v2) over the public key, base64-decoded.
    #[serde(default, with = "serde_bytes_base64")]
    pub public_key_signature_v2: Vec<u8>,
    /// `expiresAt` as unix seconds.
    #[serde(default)]
    pub expires_at: Option<i64>,
    /// `refreshedAfter` as unix seconds; refetch the certificates at/after this time.
    #[serde(default)]
    pub refreshed_after: Option<i64>,
}

/// TOML has no byte arrays: store `Vec<u8>` as standard base64 (no line wrapping).
mod serde_bytes_base64 {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Vec<u8>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        STANDARD
            .decode(text.as_bytes())
            .map_err(serde::de::Error::custom)
    }
}

/// User preferences + account state. TOML in the platform config dir.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub username: String,
    pub render_distance: i32,
    pub gui_scale: u32,
    pub fov: f32,
    #[serde(default)]
    pub last_server: Option<String>,
    #[serde(default)]
    pub msa: Option<MsaTokens>,
    /// Player profile key pair for chat signing; fetched/refreshed by `msa::ensure_fresh_account`.
    #[serde(default)]
    pub player_certificates: Option<PlayerCertificates>,
    #[serde(default)]
    pub options: Options,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            username: "Player".into(),
            render_distance: 8,
            gui_scale: 2,
            fov: 70.0,
            last_server: None,
            msa: None,
            player_certificates: None,
            options: Options::default(),
        }
    }
}

/// `$XDG_CONFIG_HOME/haste/config.toml`, else `~/.config/haste/config.toml` on Linux;
/// `directories` handles the platform mapping.
pub fn config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "haste").map(|d| d.config_dir().join("config.toml"))
}

impl Config {
    /// Loads the config from the platform config dir, creating it with defaults if absent.
    pub fn load() -> anyhow::Result<Config> {
        let path = config_path().context("no platform config dir on this system")?;
        Config::load_from(&path)
    }

    /// Loads the config at `path`, creating it with defaults if absent. The file (and its
    /// directory) is created with 0600/0700 permissions because of the MSA tokens. Path-
    /// parameterized so tests (and anything else) never have to touch the real config dir or
    /// mutate environment variables.
    pub fn load_from(path: &std::path::Path) -> anyhow::Result<Config> {
        if !path.exists() {
            let cfg = Config::default();
            cfg.save_to(path)?;
            return Ok(cfg);
        }
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut cfg: Config =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        cfg.options = std::mem::take(&mut cfg.options).sanitized();
        Ok(cfg)
    }

    /// Writes the config back to the platform config dir, 0600.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = config_path().context("no platform config dir on this system")?;
        self.save_to(&path)
    }

    /// Writes the config to `path` (creating its directory), 0600.
    pub fn save_to(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
            }
        }
        let text = toml::to_string_pretty(self)?;
        std::fs::write(path, text)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_toml() {
        let cfg = Config {
            username: "Josef".into(),
            render_distance: 12,
            gui_scale: 3,
            fov: 110.0,
            last_server: Some("localhost:25565".into()),
            msa: Some(MsaTokens {
                access_token: "tok".into(),
                refresh_token: "ref".into(),
                expires_at: Some(1900000000),
                profile: Some(MsaProfile {
                    uuid: "u".into(),
                    name: "Josef".into(),
                }),
            }),
            ..Config::default()
        };
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.username, "Josef");
        assert_eq!(back.render_distance, 12);
        assert_eq!(back.fov, 110.0);
        assert_eq!(back.last_server.as_deref(), Some("localhost:25565"));
        assert_eq!(back.msa.as_ref().unwrap().access_token, "tok");
        assert_eq!(back.msa.as_ref().unwrap().expires_at, Some(1900000000));
    }

    #[test]
    fn parses_minimal_toml_with_defaults() {
        let text = r#"
username = "Steve"
render_distance = 4
gui_scale = 1
fov = 70.0
"#;
        let cfg: Config = toml::from_str(text).unwrap();
        assert_eq!(cfg.username, "Steve");
        assert_eq!(cfg.last_server, None);
        assert_eq!(cfg.msa, None);
    }

    #[cfg(unix)]
    #[test]
    fn save_load_creates_0600_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("haste").join("config.toml");
        let cfg = Config {
            username: "Hermes".into(),
            ..Config::default()
        };
        cfg.save_to(&path).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "config must be 0600");
        let loaded = Config::load_from(&path).unwrap();
        assert_eq!(loaded.username, "Hermes");
    }

    #[test]
    fn load_from_missing_file_creates_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        let loaded = Config::load_from(&path).unwrap();
        assert_eq!(loaded.username, "Player");
        assert!(path.is_file(), "default config was written");
        // a second load reads what was written
        assert_eq!(Config::load_from(&path).unwrap().username, "Player");
    }
}
