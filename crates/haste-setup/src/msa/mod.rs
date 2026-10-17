//! Microsoft auth: device-code flow (MSA -> Xbox Live -> XSTS -> Minecraft services ->
//! entitlements -> profile), with the `client_id` configurable via `HASTE_MSA_CLIENT_ID` (none
//! shipped). Every step takes an [`MsaEndpoints`], so tests point the chain at an in-process
//! mock (`tests/msa_mock_flow.rs`); no live calls in tests.

pub mod certificates;
pub mod endpoints;
pub mod errors;

pub use endpoints::MsaEndpoints;
pub use errors::MsaError;

use crate::config::{MsaProfile, MsaTokens, PlayerCertificates};
use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

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
/// [`poll_device_code`] in a poll loop (or [`poll_until_token`]).
pub async fn begin_device_code(
    client: &reqwest::Client,
    endpoints: &MsaEndpoints,
) -> anyhow::Result<DeviceCode> {
    let id = endpoints.require_client_id()?;
    let resp = client
        .post(endpoints.device_code_url())
        .form(&[("client_id", id), ("scope", MSA_SCOPE)])
        .send()
        .await
        .context("POST devicecode")?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(oauth_failure(&body, status.as_u16(), "devicecode").into());
    }
    resp.json().await.context("parsing devicecode response")
}

/// What one poll of the token endpoint said (RFC 8628 section 3.5).
#[derive(Debug, Clone)]
pub enum DevicePoll {
    /// The user approved: here are the tokens.
    Token(Box<OauthToken>),
    /// `authorization_pending`: keep polling at the same interval.
    Pending,
    /// `slow_down`: keep polling, with the interval widened by 5 seconds.
    SlowDown,
}

/// Polls the token endpoint once with the device code. Terminal errors (`expired_token`,
/// `authorization_declined`, ...) come back as [`MsaError`]s.
pub async fn poll_device_code(
    client: &reqwest::Client,
    endpoints: &MsaEndpoints,
    device_code: &DeviceCode,
) -> anyhow::Result<DevicePoll> {
    let id = endpoints.require_client_id()?;
    let resp: OauthToken = client
        .post(endpoints.token_url())
        .form(&[
            ("client_id", id),
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
        None => Ok(DevicePoll::Token(Box::new(resp))),
        Some("authorization_pending") => Ok(DevicePoll::Pending),
        Some("slow_down") => Ok(DevicePoll::SlowDown),
        Some(e) => Err(errors::oauth_error(e, resp.error_description.as_deref()).into()),
    }
}

/// An OAuth endpoint answered with an error status: its JSON error body if there is one.
fn oauth_failure(body: &str, status: u16, step: &'static str) -> MsaError {
    match serde_json::from_str::<OauthToken>(body) {
        Ok(t) if t.error.is_some() => errors::oauth_error(
            t.error.as_deref().unwrap_or_default(),
            t.error_description.as_deref(),
        ),
        _ => MsaError::Services {
            step,
            status,
            body: body.to_string(),
        },
    }
}

/// Refreshes an MSA access token with the refresh token (the "next launch" path: no browser).
pub async fn refresh(
    client: &reqwest::Client,
    endpoints: &MsaEndpoints,
    refresh_token: &str,
) -> anyhow::Result<OauthToken> {
    let id = endpoints.require_client_id()?;
    let resp: OauthToken = client
        .post(endpoints.token_url())
        .form(&[
            ("client_id", id),
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
        return Err(errors::oauth_error(e, resp.error_description.as_deref()).into());
    }
    Ok(resp)
}

/// MSA access token -> Xbox Live (XBL) token + user hash.
pub async fn xbl_auth(
    client: &reqwest::Client,
    endpoints: &MsaEndpoints,
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
    let resp = client
        .post(endpoints.xbl_auth_url())
        .json(&req)
        .send()
        .await
        .context("POST xbl auth")?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(MsaError::Services {
            step: "xbox live authenticate",
            status: status.as_u16(),
            body,
        }
        .into());
    }
    let resp: XblResponse = resp.json().await.context("parsing xbl auth response")?;
    let uhs = resp
        .display_claims
        .xui
        .first()
        .map(|c| c.uhs.clone())
        .ok_or_else(|| anyhow!("xbl response had no uhs"))?;
    Ok((resp.token, uhs))
}

/// XBL token + uhs -> XSTS token for `rp://api.minecraftservices.com/`. A 401 with an `XErr`
/// body (no Xbox profile, child account, ...) becomes the matching [`MsaError`].
pub async fn xsts_auth(
    client: &reqwest::Client,
    endpoints: &MsaEndpoints,
    xbl_token: &str,
) -> anyhow::Result<(String, String)> {
    let user_tokens = [xbl_token.to_string()];
    let req = XstsRequest {
        properties: XstsProperties {
            sandbox_id: "RETAIL",
            user_tokens: &user_tokens,
        },
        relying_party: "rp://api.minecraftservices.com/",
        token_type: "JWT",
    };
    let resp = client
        .post(endpoints.xsts_auth_url())
        .json(&req)
        .send()
        .await
        .context("POST xsts auth")?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(errors::xsts_failure(status.as_u16(), &body).into());
    }
    let resp: XblResponse = resp.json().await.context("parsing xsts response")?;
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
    endpoints: &MsaEndpoints,
    uhs: &str,
    xsts_token: &str,
) -> anyhow::Result<McLoginResponse> {
    let req = McLoginRequest {
        identity_token: format!("XBL3.0 x={uhs};{xsts_token}").into(),
    };
    let resp = client
        .post(endpoints.mc_login_url())
        .json(&req)
        .send()
        .await
        .context("POST mc login")?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(MsaError::Services {
            step: "login_with_xbox",
            status: status.as_u16(),
            body,
        }
        .into());
    }
    resp.json().await.context("parsing mc login response")
}

/// The `/entitlements/mcstore` response shape (only what the ownership check needs).
#[derive(Debug, Clone, Deserialize)]
pub struct McEntitlementsResponse {
    #[serde(default)]
    pub items: Vec<McEntitlementItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McEntitlementItem {
    #[serde(default)]
    pub name: String,
}

/// Decides ownership from an entitlements body: any item in the store means the account can
/// play (`product_minecraft`/`game_minecraft` for purchases, the Game Pass entitlements
/// otherwise); an empty list means it cannot. Split out so tests run it on fixtures.
pub fn parse_entitlements(json: &str) -> anyhow::Result<()> {
    let v: McEntitlementsResponse =
        serde_json::from_str(json).context("parsing entitlements response")?;
    if v.items.is_empty() {
        return Err(MsaError::NotEntitled.into());
    }
    Ok(())
}

/// Minecraft access token -> ownership check (`GET /entitlements/mcstore`).
pub async fn mc_entitlements(
    client: &reqwest::Client,
    endpoints: &MsaEndpoints,
    mc_access_token: &str,
) -> anyhow::Result<()> {
    let resp = client
        .get(endpoints.mc_entitlements_url())
        .bearer_auth(mc_access_token)
        .send()
        .await
        .context("GET entitlements")?;
    let status = resp.status();
    let body = resp.text().await.context("reading entitlements body")?;
    if !status.is_success() {
        return Err(MsaError::Services {
            step: "entitlements",
            status: status.as_u16(),
            body,
        }
        .into());
    }
    parse_entitlements(&body)
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

/// Minecraft access token -> player profile (uuid, name). A 404 means the account owns the
/// game but has not created a profile yet.
pub async fn mc_profile(
    client: &reqwest::Client,
    endpoints: &MsaEndpoints,
    mc_access_token: &str,
) -> anyhow::Result<MsaProfile> {
    let resp = client
        .get(endpoints.mc_profile_url())
        .bearer_auth(mc_access_token)
        .send()
        .await
        .context("GET mc profile")?;
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(MsaError::NoProfile.into());
    }
    let body = resp.text().await.context("reading mc profile body")?;
    if !status.is_success() {
        return Err(MsaError::Services {
            step: "profile",
            status: status.as_u16(),
            body,
        }
        .into());
    }
    parse_mc_profile(&body)
}

/// Runs the tail of the flow after the MSA token is in hand: XBL -> XSTS -> MC login ->
/// entitlements -> profile.
pub async fn exchange_msa_token(
    client: &reqwest::Client,
    endpoints: &MsaEndpoints,
    msa: &OauthToken,
) -> anyhow::Result<MsaTokens> {
    let (xbl_token, _uhs) = xbl_auth(client, endpoints, &msa.access_token).await?;
    let (xsts_token, uhs) = xsts_auth(client, endpoints, &xbl_token).await?;
    let mc = mc_login(client, endpoints, &uhs, &xsts_token).await?;
    mc_entitlements(client, endpoints, &mc.access_token).await?;
    let profile = mc_profile(client, endpoints, &mc.access_token).await?;
    Ok(MsaTokens {
        access_token: mc.access_token,
        refresh_token: msa.refresh_token.clone(),
        expires_at: Some(unix_now() + mc.expires_in as i64),
        profile: Some(profile),
    })
}

pub use certificates::{
    parse_player_certificates, parse_rfc3339, parse_rfc3339_millis, pem_to_der,
    player_certificates, profile_key_pair,
};

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
/// refresh -> XBL -> XSTS -> MC login -> entitlements -> profile), fetches the player's
/// chat-signing certificates when missing or past `refreshedAfter`. The caller saves the
/// config. The lead wires this in before the client connects to an online-mode server.
pub async fn ensure_fresh_account(
    client: &reqwest::Client,
    endpoints: &MsaEndpoints,
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
            let msa = refresh(client, endpoints, &refresh_token).await?;
            let refreshed = exchange_msa_token(client, endpoints, &msa).await?;
            *tokens = refreshed;
        }
    }
    if let Some(tokens) = config.msa.as_ref() {
        if certificates_need_refresh(&config.player_certificates, now) {
            let fetched = player_certificates(client, endpoints, &tokens.access_token).await?;
            config.player_certificates = Some(fetched);
        }
    }
    Ok(())
}

/// Removes the stored account (tokens, profile and chat-signing certificates) from the config
/// at `path` and saves it; returns the removed tokens so the CLI can name the account.
/// `haste logout` calls this.
pub fn remove_stored_account(path: &std::path::Path) -> anyhow::Result<Option<MsaTokens>> {
    let mut config = crate::config::Config::load_from(path)?;
    let removed = config.msa.take();
    config.player_certificates = None;
    if removed.is_some() {
        config.save_to(path)?;
    }
    Ok(removed)
}

/// Runs the full device-code flow on a fresh HTTP client: shows the verification URL and user
/// code through `on_code`, polls the token endpoint (honoring `interval` and `slow_down`,
/// bounding by `expires_in`), then does the XBL -> XSTS -> MC login -> entitlements -> profile
/// exchange.
pub async fn device_code_login<F: FnOnce(&DeviceCode)>(
    endpoints: &MsaEndpoints,
    on_code: F,
) -> anyhow::Result<MsaTokens> {
    let client = reqwest::Client::new();
    let device = begin_device_code(&client, endpoints).await?;
    on_code(&device);
    let msa = poll_until_token(&client, endpoints, &device).await?;
    exchange_msa_token(&client, endpoints, &msa).await
}

/// Polls [`poll_device_code`] every `interval` seconds (default 5) until the user approves or
/// the device code expires (RFC 8628 section 3.5): `authorization_pending` waits the same
/// interval again, `slow_down` adds 5 seconds to the interval for the rest of the flow, and
/// `expires_in` bounds the whole wait (the server also answers `expired_token` after that).
/// Each "second" lasts `endpoints.poll_time_unit`.
pub async fn poll_until_token(
    client: &reqwest::Client,
    endpoints: &MsaEndpoints,
    device: &DeviceCode,
) -> anyhow::Result<OauthToken> {
    let unit = endpoints.poll_time_unit;
    let interval_secs = if device.interval > 0 {
        device.interval
    } else {
        endpoints::DEFAULT_POLL_INTERVAL_SECS
    };
    let deadline = tokio::time::Instant::now() + unit * device.expires_in.max(interval_secs) as u32;
    let mut interval = unit * interval_secs as u32;
    loop {
        tokio::time::sleep(interval).await;
        if tokio::time::Instant::now() >= deadline {
            return Err(MsaError::CodeExpired.into());
        }
        match poll_device_code(client, endpoints, device).await? {
            DevicePoll::Token(token) => return Ok(*token),
            DevicePoll::Pending => {}
            DevicePoll::SlowDown => interval += unit * endpoints::SLOW_DOWN_STEP_SECS as u32,
        }
    }
}

#[cfg(test)]
mod tests;
