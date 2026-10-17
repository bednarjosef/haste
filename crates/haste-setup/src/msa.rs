//! Microsoft auth: device-code flow (MSA -> Xbox Live -> XSTS -> Minecraft services -> profile),
//! with the `client_id` configurable via `HASTE_MSA_CLIENT_ID` (none shipped). No live calls in
//! tests; request/response types are exercised on fixtures.

use crate::config::{MsaProfile, MsaTokens, PlayerCertificates};
use anyhow::{anyhow, Context};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBL_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_AUTH_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

/// Refresh a stored access token this many seconds before it actually expires.
const TOKEN_EXPIRY_MARGIN_SECS: i64 = 60;

/// Current unix time in seconds; tests pass their own `now`.
fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The MSA scope needed for Minecraft (Xbox Live sign-in + a refresh token).
pub const MSA_SCOPE: &str = "XboxLive.signin offline_access";

/// Reads `HASTE_MSA_CLIENT_ID`; the caller decides what to do when absent (no client_id is
/// shipped with Haste).
pub fn client_id() -> Option<String> {
    std::env::var("HASTE_MSA_CLIENT_ID")
        .ok()
        .filter(|s| !s.is_empty())
}

/// Step 1 response: what the user must enter where.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub expires_in: u64,
    #[serde(default)]
    pub interval: u64,
    #[serde(default)]
    pub message: String,
}

/// MSA OAuth token response (device-code grant and refresh grant share the shape). On error
/// responses (`authorization_pending`, ...) only the `error` fields are present.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OauthToken {
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub expires_in: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_description: Option<String>,
}

impl OauthToken {
    pub fn expires_at(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
            + self.expires_in as i64
    }
}

// --- Xbox Live / XSTS ---
// These APIs use PascalCase JSON keys; the serde renames map them to snake_case fields.

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct XblProperties<'a> {
    auth_method: &'static str,
    site_name: &'static str,
    #[serde(rename = "RpsTicket")]
    rps_ticket: std::borrow::Cow<'a, str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct XblRequest<'a> {
    properties: XblProperties<'a>,
    relying_party: &'static str,
    token_type: &'static str,
}

#[derive(Debug, Clone, Deserialize)]
pub struct XblResponse {
    #[serde(rename = "Token")]
    pub token: String,
    #[serde(rename = "DisplayClaims")]
    pub display_claims: XblDisplayClaims,
}

#[derive(Debug, Clone, Deserialize)]
pub struct XblDisplayClaims {
    #[serde(rename = "xui")]
    pub xui: Vec<XuiClaim>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct XuiClaim {
    #[serde(rename = "uhs", default)]
    pub uhs: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct XstsProperties<'a> {
    #[serde(rename = "SandboxId")]
    sandbox_id: &'static str,
    user_tokens: &'a [String],
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct XstsRequest<'a> {
    properties: XstsProperties<'a>,
    relying_party: &'static str,
    token_type: &'static str,
}

// --- Minecraft services ---

#[derive(Debug, Serialize)]
struct McLoginRequest<'a> {
    #[serde(rename = "identityToken")]
    identity_token: std::borrow::Cow<'a, str>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McLoginResponse {
    #[serde(rename = "access_token")]
    pub access_token: String,
    #[serde(rename = "expires_in", default)]
    pub expires_in: u64,
}

/// The full result of the device-code flow.
#[derive(Debug, Clone)]
pub struct MsaAuth {
    pub tokens: MsaTokens,
}

/// Starts the flow: returns the device code the user must approve. Then call
/// [`complete_device_code`] in a poll loop.
pub async fn begin_device_code(client: &reqwest::Client) -> anyhow::Result<DeviceCode> {
    let id = client_id().ok_or_else(|| anyhow!("HASTE_MSA_CLIENT_ID is not set"))?;
    let resp = client
        .post(DEVICE_CODE_URL)
        .form(&[("client_id", id.as_str()), ("scope", MSA_SCOPE)])
        .send()
        .await
        .context("POST devicecode")?;
    anyhow::ensure!(
        resp.status().is_success(),
        "devicecode returned {}",
        resp.status()
    );
    resp.json().await.context("parsing devicecode response")
}

/// Polls the token endpoint with the device code; `Ok(None)` = still pending (`authorization_pending`).
pub async fn poll_device_code(
    client: &reqwest::Client,
    device_code: &DeviceCode,
) -> anyhow::Result<Option<OauthToken>> {
    let id = client_id().ok_or_else(|| anyhow!("HASTE_MSA_CLIENT_ID is not set"))?;
    let resp: OauthToken = client
        .post(TOKEN_URL)
        .form(&[
            ("client_id", id.as_str()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", device_code.device_code.as_str()),
        ])
        .send()
        .await
        .context("POST token (device_code)")?
        .json()
        .await
        .context("parsing token response")?;
    match resp.error.as_deref() {
        None => Ok(Some(resp)),
        Some("authorization_pending") | Some("slow_down") => Ok(None),
        Some(e) => Err(anyhow!(
            "msa token error: {e}: {:?}",
            resp.error_description
        )),
    }
}

/// Refreshes an MSA access token with the refresh token.
pub async fn refresh(client: &reqwest::Client, refresh_token: &str) -> anyhow::Result<OauthToken> {
    let id = client_id().ok_or_else(|| anyhow!("HASTE_MSA_CLIENT_ID is not set"))?;
    let resp: OauthToken = client
        .post(TOKEN_URL)
        .form(&[
            ("client_id", id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("scope", MSA_SCOPE),
        ])
        .send()
        .await
        .context("POST token (refresh)")?
        .json()
        .await
        .context("parsing refresh response")?;
    if let Some(e) = resp.error.as_deref() {
        return Err(anyhow!("msa refresh error: {e}"));
    }
    Ok(resp)
}

/// MSA access token -> Xbox Live (XBL) token + user hash.
pub async fn xbl_auth(
    client: &reqwest::Client,
    msa_access_token: &str,
) -> anyhow::Result<(String, String)> {
    let req = XblRequest {
        properties: XblProperties {
            auth_method: "RPS",
            site_name: "user.auth.xboxlive.com",
            rps_ticket: format!("d={msa_access_token}").into(),
        },
        relying_party: "http://auth.xboxlive.com",
        token_type: "JWT",
    };
    let resp: XblResponse = client
        .post(XBL_AUTH_URL)
        .json(&req)
        .send()
        .await
        .context("POST xbl auth")?
        .json()
        .await
        .context("parsing xbl auth response")?;
    let uhs = resp
        .display_claims
        .xui
        .first()
        .map(|c| c.uhs.clone())
        .ok_or_else(|| anyhow!("xbl response had no uhs"))?;
    Ok((resp.token, uhs))
}

/// XBL token + uhs -> XSTS token for `rp://api.minecraftservices.com/`.
pub async fn xsts_auth(
    client: &reqwest::Client,
    xbl_token: &str,
) -> anyhow::Result<(String, String)> {
    let user_tokens = [xbl_token.to_string()];
    let req = XstsRequest {
        properties: XstsProperties {
            sandbox_id: "XBOXRETAIL.XBOX.COM",
            user_tokens: &user_tokens,
        },
        relying_party: "rp://api.minecraftservices.com/",
        token_type: "JWT",
    };
    let resp: XblResponse = client
        .post(XSTS_AUTH_URL)
        .json(&req)
        .send()
        .await
        .context("POST xsts auth")?
        .json()
        .await
        .context("parsing xsts response")?;
    let uhs = resp
        .display_claims
        .xui
        .first()
        .map(|c| c.uhs.clone())
        .ok_or_else(|| anyhow!("xsts response had no uhs"))?;
    Ok((resp.token, uhs))
}

/// XSTS token + uhs -> Minecraft services access token.
pub async fn mc_login(
    client: &reqwest::Client,
    uhs: &str,
    xsts_token: &str,
) -> anyhow::Result<McLoginResponse> {
    let req = McLoginRequest {
        identity_token: format!("XBL3.0 x={uhs};{xsts_token}").into(),
    };
    let resp: McLoginResponse = client
        .post(MC_LOGIN_URL)
        .json(&req)
        .send()
        .await
        .context("POST mc login")?
        .json()
        .await
        .context("parsing mc login response")?;
    Ok(resp)
}

/// The `/minecraft/profile` response shape (only the fields Haste uses).
#[derive(Debug, Clone, Deserialize)]
pub struct McProfileResponse {
    #[serde(rename = "id")]
    pub uuid: String,
    pub name: String,
}

/// Parses a `/minecraft/profile` response body. Split out of [`mc_profile`] so tests can run it
/// on hand-written JSON without a network.
pub fn parse_mc_profile(json: &str) -> anyhow::Result<MsaProfile> {
    let v: McProfileResponse = serde_json::from_str(json).context("parsing mc profile")?;
    Ok(MsaProfile {
        uuid: v.uuid,
        name: v.name,
    })
}

/// Minecraft access token -> player profile (uuid, name).
pub async fn mc_profile(
    client: &reqwest::Client,
    mc_access_token: &str,
) -> anyhow::Result<MsaProfile> {
    let resp = client
        .get(MC_PROFILE_URL)
        .bearer_auth(mc_access_token)
        .send()
        .await
        .context("GET mc profile")?;
    anyhow::ensure!(
        resp.status().is_success(),
        "mc profile returned {}",
        resp.status()
    );
    let body = resp.text().await.context("reading mc profile body")?;
    parse_mc_profile(&body)
}

/// Runs the tail of the flow after the MSA token is in hand: XBL -> XSTS -> MC login -> profile.
pub async fn exchange_msa_token(
    client: &reqwest::Client,
    msa: &OauthToken,
) -> anyhow::Result<MsaTokens> {
    let (xbl_token, _uhs) = xbl_auth(client, &msa.access_token).await?;
    let (xsts_token, uhs) = xsts_auth(client, &xbl_token).await?;
    let mc = mc_login(client, &uhs, &xsts_token).await?;
    let profile = mc_profile(client, &mc.access_token).await?;
    Ok(MsaTokens {
        access_token: mc.access_token,
        refresh_token: msa.refresh_token.clone(),
        expires_at: Some(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
                + mc.expires_in as i64,
        ),
        profile: Some(profile),
    })
}

// --- Player certificates (chat signing) ---

/// The `/player/certificates` response shape (minecraft.wiki "Mojang API"). Keys arrive as PEM;
/// Haste keeps the raw DER bytes (armour stripped, base64 decoded) — no crypto crate needed.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct McCertificatesResponse {
    key_pair: McKeyPairResponse,
    #[serde(default)]
    public_key_signature: String,
    #[serde(default)]
    public_key_signature_v2: String,
    #[serde(default)]
    expires_at: String,
    #[serde(default)]
    refreshed_after: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct McKeyPairResponse {
    #[serde(default)]
    private_key: String,
    #[serde(default)]
    public_key: String,
}

/// Strips PEM armour (`-----BEGIN/END ...-----` lines) and base64-decodes the body to DER bytes.
/// Accepts any label and `\n` or `\r\n` line endings; errors on armour without a decodable body.
pub fn pem_to_der(pem: &str) -> anyhow::Result<Vec<u8>> {
    let body = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");
    anyhow::ensure!(!body.is_empty(), "PEM contains no base64 body");
    let der = BASE64
        .decode(body.as_bytes())
        .map_err(|e| anyhow!("decoding PEM body as base64: {e}"))?;
    Ok(der)
}

/// Parses RFC 3339 `YYYY-MM-DDTHH:MM:SS(.fff)(Z|+hh:mm|-hh:mm)` into unix seconds, without a
/// date crate. Fractional seconds are truncated; the offset must be whole minutes (the values
/// Mojang sends are UTC with milliseconds, but inputs with offsets are handled too).
pub fn parse_rfc3339(s: &str) -> anyhow::Result<i64> {
    Ok(parse_rfc3339_millis(s)?.div_euclid(1000))
}

/// [`parse_rfc3339`] in unix milliseconds (fractions below a millisecond truncated), as
/// Java's `Instant.parse(..).toEpochMilli()` gives: the chat key's signed payload carries the
/// expiry in milliseconds, so the seconds are not enough there.
pub fn parse_rfc3339_millis(s: &str) -> anyhow::Result<i64> {
    let bytes = s.as_bytes();
    let bad = || anyhow!("not an RFC 3339 time: {s:?}");
    anyhow::ensure!(bytes.len() >= 20, "{}", bad());
    let (y, mo, d) = (
        s.get(0..4)
            .and_then(|v| v.parse::<i64>().ok())
            .ok_or_else(bad)?,
        s.get(5..7)
            .and_then(|v| v.parse::<i64>().ok())
            .ok_or_else(bad)?,
        s.get(8..10)
            .and_then(|v| v.parse::<i64>().ok())
            .ok_or_else(bad)?,
    );
    anyhow::ensure!(
        bytes[4] == b'-' && bytes[7] == b'-' && (bytes[10] == b'T' || bytes[10] == b't'),
        "{}",
        bad()
    );
    let (h, mi, sec) = (
        s.get(11..13)
            .and_then(|v| v.parse::<i64>().ok())
            .ok_or_else(bad)?,
        s.get(14..16)
            .and_then(|v| v.parse::<i64>().ok())
            .ok_or_else(bad)?,
        s.get(17..19)
            .and_then(|v| v.parse::<i64>().ok())
            .ok_or_else(bad)?,
    );
    anyhow::ensure!(bytes[13] == b':' && bytes[16] == b':', "{}", bad());
    anyhow::ensure!(
        (1..=12).contains(&mo) && (1..=31).contains(&d) && h < 24 && mi < 60 && sec < 60,
        "{}",
        bad()
    );
    // optional fractional seconds: .fff... (truncated to milliseconds)
    let mut i = 19;
    let mut millis = 0i64;
    if bytes.get(i) == Some(&b'.') {
        let start = i + 1;
        i = start;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            if i - start < 3 {
                millis = millis * 10 + i64::from(bytes[i] - b'0');
            }
            i += 1;
        }
        anyhow::ensure!(i > start, "{}", bad());
        for _ in (i - start)..3 {
            millis *= 10;
        }
    }
    // offset: Z / z / ±hh:mm
    let mut offset_secs = 0i64;
    match bytes.get(i) {
        Some(&b'Z') | Some(&b'z') => i += 1,
        Some(&sign @ (b'+' | b'-')) => {
            anyhow::ensure!(bytes.len() >= i + 6, "{}", bad());
            let oh = s
                .get(i + 1..i + 3)
                .and_then(|v| v.parse::<i64>().ok())
                .ok_or_else(bad)?;
            let om = s
                .get(i + 4..i + 6)
                .and_then(|v| v.parse::<i64>().ok())
                .ok_or_else(bad)?;
            anyhow::ensure!(bytes[i + 3] == b':' && oh < 24 && om < 60, "{}", bad());
            let mag = oh * 3600 + om * 60;
            offset_secs = if sign == b'-' { -mag } else { mag };
            i += 6;
        }
        _ => return Err(bad()),
    }
    anyhow::ensure!(i == bytes.len(), "{}", bad());
    // days since 1970-01-01 (civil-from-days, valid for any year i64 can hold)
    let days = days_from_civil(y, mo, d);
    let secs = days * 86400 + h * 3600 + mi * 60 + sec - offset_secs;
    Ok(secs * 1000 + millis)
}

/// Days since 1970-01-01 for a civil (year, month, day); Howard Hinnant's algorithm.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = (m + 9) % 12; // [0, 11]
    let doy = (153 * mp + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// Parses a `/player/certificates` response body (PEM keys -> raw DER bytes, signatures as
/// bytes, times as unix seconds). Split out of [`player_certificates`] so tests run it on
/// hand-written JSON without a network.
pub fn parse_player_certificates(json: &str) -> anyhow::Result<PlayerCertificates> {
    let v: McCertificatesResponse =
        serde_json::from_str(json).context("parsing player certificates")?;
    let mut certs = PlayerCertificates {
        private_key_der: pem_to_der(&v.key_pair.private_key)?,
        public_key_der: pem_to_der(&v.key_pair.public_key)?,
        public_key_signature: BASE64
            .decode(v.public_key_signature.as_bytes())
            .map_err(|e| anyhow!("decoding publicKeySignature: {e}"))?,
        public_key_signature_v2: BASE64
            .decode(v.public_key_signature_v2.as_bytes())
            .map_err(|e| anyhow!("decoding publicKeySignatureV2: {e}"))?,
        expires_at: None,
        refreshed_after: None,
        expires_at_millis: None,
        refreshed_after_millis: None,
    };
    if !v.expires_at.is_empty() {
        let ms = parse_rfc3339_millis(&v.expires_at)?;
        certs.expires_at = Some(ms.div_euclid(1000));
        certs.expires_at_millis = Some(ms);
    }
    if !v.refreshed_after.is_empty() {
        let ms = parse_rfc3339_millis(&v.refreshed_after)?;
        certs.refreshed_after = Some(ms.div_euclid(1000));
        certs.refreshed_after_millis = Some(ms);
    }
    Ok(certs)
}

/// The chat key pair protocol signs with, from the certificates: the v2 signature (the one
/// vanilla's authlib reads as `publicKeySignature`) and the expiry in milliseconds. `None`
/// when the response lacked a v2 signature or an expiry.
pub fn profile_key_pair(c: &PlayerCertificates) -> Option<haste_protocol::ProfileKeyPair> {
    let expires_at_millis = c.expires_at_millis.or(c.expires_at.map(|s| s * 1000))?;
    if c.public_key_signature_v2.is_empty() {
        return None;
    }
    Some(haste_protocol::ProfileKeyPair {
        private_key_der: c.private_key_der.clone(),
        public_key_der: c.public_key_der.clone(),
        key_signature: c.public_key_signature_v2.clone(),
        expires_at_millis,
        refreshed_after_millis: c
            .refreshed_after_millis
            .or(c.refreshed_after.map(|s| s * 1000))
            .unwrap_or(expires_at_millis),
    })
}

/// Minecraft access token -> player profile key pair for chat signing (POST, empty body), on
/// the services host (`haste_protocol::auth::services_host`: Mojang's unless
/// `HASTE_SERVICES_HOST` points at a local fake API).
pub async fn player_certificates(
    client: &reqwest::Client,
    mc_access_token: &str,
) -> anyhow::Result<PlayerCertificates> {
    let url = format!(
        "{}/player/certificates",
        haste_protocol::auth::services_host()
    );
    player_certificates_at(client, &url, mc_access_token).await
}

/// [`player_certificates`] against an explicit URL.
pub async fn player_certificates_at(
    client: &reqwest::Client,
    url: &str,
    mc_access_token: &str,
) -> anyhow::Result<PlayerCertificates> {
    let resp = client
        .post(url)
        .bearer_auth(mc_access_token)
        .body(String::new())
        .send()
        .await
        .context("POST player certificates")?;
    anyhow::ensure!(
        resp.status().is_success(),
        "player certificates returned {}",
        resp.status()
    );
    let body = resp.text().await.context("reading certificates body")?;
    parse_player_certificates(&body)
}

/// The token in `config.msa` needs a refresh once it is past `expires_at` (with a small margin
/// so we never connect with a token about to lapse). `now` is a parameter so tests decide it.
pub fn mc_token_needs_refresh(tokens: &MsaTokens, now: i64) -> bool {
    tokens
        .expires_at
        .is_none_or(|at| now >= at - TOKEN_EXPIRY_MARGIN_SECS)
}

/// The stored certificates must be fetched again once `refreshedAfter` has passed (a missing
/// certificate or refresh time means fetch). `now` is a parameter so tests decide it.
pub fn certificates_need_refresh(certs: &Option<PlayerCertificates>, now: i64) -> bool {
    match certs {
        None => true,
        Some(c) => c.refreshed_after.is_none_or(|at| now >= at),
    }
}

/// Brings the stored account up to date for online play without user interaction: refreshes
/// the Minecraft access token from the stored Microsoft refresh token when expired (MSA
/// refresh -> XBL -> XSTS -> MC login), fetches the player's chat-signing certificates when
/// missing or past `refreshedAfter`, and saves the config. The lead wires this in before the
/// client connects to an online-mode server.
pub async fn ensure_fresh_account(
    client: &reqwest::Client,
    config: &mut crate::config::Config,
) -> anyhow::Result<()> {
    let now = unix_now();
    if let Some(tokens) = config.msa.as_mut() {
        if mc_token_needs_refresh(tokens, now) {
            let refresh_token = tokens.refresh_token.clone();
            anyhow::ensure!(
                !refresh_token.is_empty(),
                "the stored account has no refresh token; run `haste login` again"
            );
            let msa = refresh(client, &refresh_token).await?;
            let refreshed = exchange_msa_token(client, &msa).await?;
            *tokens = refreshed;
        }
    }
    if let Some(tokens) = config.msa.as_ref() {
        if certificates_need_refresh(&config.player_certificates, now) {
            let fetched = player_certificates(client, &tokens.access_token).await?;
            config.player_certificates = Some(fetched);
        }
    }
    Ok(())
}

/// Runs the full device-code flow on a fresh HTTP client: prints the verification URL and user
/// code, polls the token endpoint (honoring `interval`, bounding by `expires_in`), then does the
/// XBL -> XSTS -> MC login -> profile exchange. `on_code` shows the code to the user.
pub async fn device_code_login<F: FnOnce(&DeviceCode)>(on_code: F) -> anyhow::Result<MsaTokens> {
    let client = reqwest::Client::new();
    let device = begin_device_code(&client).await?;
    on_code(&device);
    let msa = poll_until_token(&client, &device).await?;
    exchange_msa_token(&client, &msa).await
}

/// Polls `poll_device_code` every `interval` seconds until the user approves or the device code
/// expires. `slow_down` widens the interval by 5s (per the spec) instead of just retrying.
async fn poll_until_token(
    client: &reqwest::Client,
    device: &DeviceCode,
) -> anyhow::Result<OauthToken> {
    let interval = if device.interval > 0 {
        device.interval
    } else {
        5
    };
    let deadline = tokio::time::Instant::now()
        + std::time::Duration::from_secs(device.expires_in.max(interval).saturating_sub(2));
    let mut interval = std::time::Duration::from_secs(interval);
    loop {
        tokio::time::sleep(interval).await;
        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!("the sign-in code expired, run `haste login` again"));
        }
        match poll_device_code(client, device).await {
            Ok(Some(token)) => return Ok(token),
            Ok(None) => {}
            // server asked us to slow down: widen the interval by 5s and keep waiting
            Err(e) if e.to_string().contains("slow_down") => {
                interval += std::time::Duration::from_secs(5)
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_device_code_response() {
        let json = r#"{
            "user_code": "BCDWTJGLJ",
            "device_code": "AQABAAEAAAAmVhLqqVIVhOvI",
            "verification_uri": "https://www.microsoft.com/link",
            "expires_in": 900,
            "interval": 5,
            "message": "To sign in, use a web browser to open the page https://www.microsoft.com/link and enter the code BCDWTJGLJ."
        }"#;
        let d: DeviceCode = serde_json::from_str(json).unwrap();
        assert_eq!(d.user_code, "BCDWTJGLJ");
        assert_eq!(d.verification_uri, "https://www.microsoft.com/link");
        assert_eq!(d.interval, 5);
        assert!(d.message.contains("BCDWTJGLJ"));
    }

    #[test]
    fn parses_token_response_and_pending_error() {
        let ok: OauthToken = serde_json::from_str(
            r#"{"token_type":"Bearer","scope":"XboxLive.signin","expires_in":3599,"access_token":"at","refresh_token":"rt"}"#,
        )
        .unwrap();
        assert_eq!(ok.access_token, "at");
        assert_eq!(ok.refresh_token, "rt");
        assert!(ok.error.is_none());
        assert!(ok.expires_at() > 0);

        let pending: OauthToken = serde_json::from_str(
            r#"{"error":"authorization_pending","error_description":"Authorization is pending"}"#,
        )
        .unwrap();
        assert_eq!(pending.error.as_deref(), Some("authorization_pending"));
    }

    #[test]
    fn builds_xbl_request_body() {
        let req = XblRequest {
            properties: XblProperties {
                auth_method: "RPS",
                site_name: "user.auth.xboxlive.com",
                rps_ticket: "d=TOKEN".into(),
            },
            relying_party: "http://auth.xboxlive.com",
            token_type: "JWT",
        };
        let j = serde_json::to_value(&req).unwrap();
        assert_eq!(j["Properties"]["RpsTicket"], "d=TOKEN");
        assert_eq!(j["RelyingParty"], "http://auth.xboxlive.com");
    }

    #[test]
    fn parses_xbl_and_xsts_responses() {
        let json = r#"{
            "IssueInstant":"2025-10-01T00:00:00.0000000Z",
            "NotAfter":"2025-10-01T01:00:00.0000000Z",
            "Token":"xbl-token",
            "DisplayClaims":{"xui":[{"uhs":"uhs-1234"}]}
        }"#;
        let r: XblResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.token, "xbl-token");
        assert_eq!(r.display_claims.xui[0].uhs, "uhs-1234");
    }

    #[test]
    fn parses_mc_login_and_profile() {
        let login: McLoginResponse =
            serde_json::from_str(r#"{"access_token":"mc-token","expires_in":86400}"#).unwrap();
        assert_eq!(login.access_token, "mc-token");
        // /minecraft/profile response shape (minecraft.wiki "Mojang API")
        let profile = parse_mc_profile(
            r#"{"id":"069a79f444e94726a5befca90e38aaf5","name":"Notch","skins":[],"capes":[]}"#,
        )
        .unwrap();
        assert_eq!(profile.uuid, "069a79f444e94726a5befca90e38aaf5");
        assert_eq!(profile.name, "Notch");
    }

    #[test]
    fn mc_profile_without_name_is_an_error() {
        assert!(parse_mc_profile(r#"{"id":"069a79f444e94726a5befca90e38aaf5"}"#).is_err());
        assert!(parse_mc_profile("{}").is_err());
    }

    #[test]
    fn builds_mc_identity_token() {
        let req = McLoginRequest {
            identity_token: "XBL3.0 x=uhs;xsts".into(),
        };
        let j = serde_json::to_value(&req).unwrap();
        assert_eq!(j["identityToken"], "XBL3.0 x=uhs;xsts");
    }

    #[test]
    fn pem_to_der_strips_armour_and_decodes() {
        // "MIIEvgAB" is base64 for 30 82 04 be 00 01
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIEvgAB\n-----END RSA PRIVATE KEY-----\n";
        assert_eq!(
            pem_to_der(pem).unwrap(),
            vec![0x30, 0x82, 0x04, 0xbe, 0x00, 0x01]
        );
        // CRLF line endings and any label work too
        let pem_crlf = "-----BEGIN PUBLIC KEY-----\r\nMIIEvgAB\r\n-----END PUBLIC KEY-----\r\n";
        assert_eq!(
            pem_to_der(pem_crlf).unwrap(),
            vec![0x30, 0x82, 0x04, 0xbe, 0x00, 0x01]
        );
        // bad base64 inside the armour is an error, as is armour without a body
        assert!(pem_to_der("-----BEGIN KEY-----\n!!not base64!!\n-----END KEY-----").is_err());
        assert!(pem_to_der("-----BEGIN KEY-----\n-----END KEY-----").is_err());
    }

    #[test]
    fn parses_rfc3339_milliseconds() {
        assert_eq!(
            parse_rfc3339_millis("2025-06-15T12:30:45.123Z").unwrap(),
            1749990645123
        );
        assert_eq!(
            parse_rfc3339_millis("2025-06-15T12:30:45.1Z").unwrap(),
            1749990645100
        );
        assert_eq!(
            parse_rfc3339_millis("2025-06-15T12:30:45.123999Z").unwrap(),
            1749990645123
        );
        assert_eq!(
            parse_rfc3339_millis("2025-06-15T12:30:45Z").unwrap(),
            1749990645000
        );
        assert_eq!(parse_rfc3339("1969-12-31T23:59:59.500Z").unwrap(), -1);
    }

    #[test]
    fn certificates_url_defaults_to_mojang() {
        assert_eq!(
            format!(
                "{}/player/certificates",
                haste_protocol::auth::MOJANG_SERVICES_HOST
            ),
            "https://api.minecraftservices.com/player/certificates"
        );
    }

    #[test]
    fn parses_rfc3339_times() {
        assert_eq!(parse_rfc3339("2025-06-15T12:30:45Z").unwrap(), 1749990645);
        // fractional seconds are truncated
        assert_eq!(
            parse_rfc3339("2025-06-15T12:30:45.123Z").unwrap(),
            1749990645
        );
        // a +02:00 offset denotes the same instant
        assert_eq!(
            parse_rfc3339("2025-06-15T14:30:45+02:00").unwrap(),
            1749990645
        );
        // leap day and the 1970 epoch
        assert_eq!(parse_rfc3339("2024-02-29T23:59:59Z").unwrap(), 1709251199);
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z").unwrap(), 0);
        // lowercase t/z and a negative offset
        assert_eq!(parse_rfc3339("2025-06-15t12:30:45z").unwrap(), 1749990645);
        assert_eq!(
            parse_rfc3339("2025-06-15T10:30:45-02:00").unwrap(),
            1749990645
        );
        // rejects malformed times
        assert!(parse_rfc3339("").is_err());
        assert!(parse_rfc3339("not a time").is_err());
        assert!(parse_rfc3339("2025-13-01T00:00:00Z").is_err());
        assert!(parse_rfc3339("2025-06-15T25:00:00Z").is_err());
        assert!(parse_rfc3339("2025-06-15 12:30:45Z").is_err());
        assert!(parse_rfc3339("2025-06-15T12:30:00+25:00").is_err());
    }

    #[test]
    fn parses_player_certificates_response() {
        // shape from minecraft.wiki "Mojang API" / "Microsoft authentication"; keys are fake PEM
        // ("MIIEvgAB" -> 30 82 04 be 00 01, "MFwwDQYD" -> 30 5c 30 0d 06 03)
        let json = r#"{
            "keyPair": {
                "privateKey": "-----BEGIN RSA PRIVATE KEY-----\nMIIEvgAB\n-----END RSA PRIVATE KEY-----\n",
                "publicKey": "-----BEGIN RSA PUBLIC KEY-----\nMFwwDQYD\n-----END RSA PUBLIC KEY-----\n"
            },
            "publicKeySignature": "3q2+7w==",
            "publicKeySignatureV2": "AQIDBAU=",
            "expiresAt": "2025-07-01T00:00:00.00000000Z",
        "refreshedAfter": "2025-06-15T12:30:45.123Z"}"#;
        let c = parse_player_certificates(json).unwrap();
        assert_eq!(c.private_key_der, vec![0x30, 0x82, 0x04, 0xbe, 0x00, 0x01]);
        assert_eq!(c.public_key_der, vec![0x30, 0x5c, 0x30, 0x0d, 0x06, 0x03]);
        assert_eq!(c.public_key_signature, vec![0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(
            c.public_key_signature_v2,
            vec![0x01, 0x02, 0x03, 0x04, 0x05]
        );
        assert_eq!(c.expires_at, Some(1751328000));
        assert_eq!(c.refreshed_after, Some(1749990645));
        // the milliseconds survive: the chat key's signature covers them
        assert_eq!(c.expires_at_millis, Some(1751328000000));
        assert_eq!(c.refreshed_after_millis, Some(1749990645123));
        let keys = profile_key_pair(&c).unwrap();
        assert_eq!(keys.expires_at_millis, 1751328000000);
        assert_eq!(keys.refreshed_after_millis, 1749990645123);
        assert_eq!(
            keys.key_signature,
            vec![0x01, 0x02, 0x03, 0x04, 0x05],
            "v2, not v1"
        );
        assert_eq!(keys.public_key_der, c.public_key_der);
        // V1 signature may be absent on newer accounts; empty times are tolerated
        let json_min = r#"{
            "keyPair": {
                "privateKey": "-----BEGIN RSA PRIVATE KEY-----\nMIIEvgAB\n-----END RSA PRIVATE KEY-----\n",
                "publicKey": "-----BEGIN RSA PUBLIC KEY-----\nMFwwDQYD\n-----END RSA PUBLIC KEY-----\n"
            },
            "publicKeySignatureV2": "AQIDBAU="}"#;
        let c = parse_player_certificates(json_min).unwrap();
        assert!(c.public_key_signature.is_empty());
        assert_eq!(c.expires_at, None);
        assert_eq!(c.refreshed_after, None);
        assert!(profile_key_pair(&c).is_none(), "no expiry: nothing to sign");
        // bad base64 in a signature is an error
        let json_bad = json.replace("3q2+7w==", "!!!not base64!!!");
        assert!(parse_player_certificates(&json_bad).is_err());
    }

    #[test]
    fn certificate_refresh_decisions_with_explicit_now() {
        let certs = PlayerCertificates {
            private_key_der: vec![1],
            public_key_der: vec![2],
            public_key_signature: vec![3],
            public_key_signature_v2: vec![4],
            expires_at: Some(1751328000),
            refreshed_after: Some(1749990645),
            ..PlayerCertificates::default()
        };
        let now: i64 = 1749990000; // 2025-06-15T12:20:00Z, an hour before refreshedAfter
        assert!(!certificates_need_refresh(&Some(certs.clone()), now));
        let now: i64 = 1749990645; // exactly refreshedAfter
        assert!(certificates_need_refresh(&Some(certs.clone()), now));
        let now: i64 = 1749990646; // past it
        assert!(certificates_need_refresh(&Some(certs.clone()), now));
        // no certificates at all: fetch
        assert!(certificates_need_refresh(&None, 0));
        // no refresh time known: err on fetching again rather than keeping stale keys forever
        let no_time = PlayerCertificates {
            refreshed_after: None,
            ..certs
        };
        assert!(certificates_need_refresh(&Some(no_time), 0));
    }

    #[test]
    fn mc_token_refresh_decisions_with_explicit_now() {
        let tokens = MsaTokens {
            access_token: "tok".into(),
            refresh_token: "ref".into(),
            expires_at: Some(1751328000),
            profile: None,
        };
        // well before expiry (and before the margin): no refresh
        assert!(!mc_token_needs_refresh(&tokens, 1751328000 - 3600));
        // inside the 60s margin or past expiry: refresh
        assert!(mc_token_needs_refresh(&tokens, 1751328000 - 30));
        assert!(mc_token_needs_refresh(&tokens, 1751328000));
        assert!(mc_token_needs_refresh(&tokens, 1751328000 + 1));
        // no expiry recorded (old configs): refresh rather than guess
        let no_expiry = MsaTokens {
            expires_at: None,
            ..tokens
        };
        assert!(mc_token_needs_refresh(&no_expiry, 0));
    }

    #[test]
    fn certificates_roundtrip_through_toml_config() {
        // what `ensure_fresh_account` stores must survive a config save/load cycle
        let certs = PlayerCertificates {
            private_key_der: vec![0x30, 0x82, 0x04, 0xbe, 0x00, 0x01],
            public_key_der: vec![0x30, 0x5c],
            public_key_signature: vec![0xde, 0xad, 0xbe, 0xef],
            public_key_signature_v2: vec![0x01, 0x02, 0x03, 0x04, 0x05],
            expires_at: Some(1751328000),
            refreshed_after: Some(1749990645),
            ..PlayerCertificates::default()
        };
        let cfg = crate::config::Config {
            player_certificates: Some(certs),
            ..crate::config::Config::default()
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        cfg.save_to(&path).unwrap();
        let back = crate::config::Config::load_from(&path).unwrap();
        assert_eq!(back.player_certificates, cfg.player_certificates);
        // a config written before this feature exists still loads (field defaults to None)
        let old = r#"
username = "Steve"
render_distance = 4
gui_scale = 1
fov = 70.0
"#;
        let parsed: crate::config::Config = toml::from_str(old).unwrap();
        assert!(parsed.player_certificates.is_none());
    }
}
