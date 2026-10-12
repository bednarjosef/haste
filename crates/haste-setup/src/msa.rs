//! Microsoft auth: device-code flow (MSA -> Xbox Live -> XSTS -> Minecraft services -> profile),
//! with the `client_id` configurable via `HASTE_MSA_CLIENT_ID` (none shipped). No live calls in
//! tests; request/response types are exercised on fixtures.

use crate::config::{MsaProfile, MsaTokens};
use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBL_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_AUTH_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

/// The MSA scope needed for Minecraft (Xbox Live sign-in + a refresh token).
pub const MSA_SCOPE: &str = "XboxLive.signin offline_access";

/// Reads `HASTE_MSA_CLIENT_ID`; the caller decides what to do when absent (no client_id is
/// shipped with Haste).
pub fn client_id() -> Option<String> {
    std::env::var("HASTE_MSA_CLIENT_ID").ok().filter(|s| !s.is_empty())
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
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
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
        .form(&[
            ("client_id", id.as_str()),
            ("scope", MSA_SCOPE),
        ])
        .send()
        .await
        .context("POST devicecode")?;
    anyhow::ensure!(resp.status().is_success(), "devicecode returned {}", resp.status());
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
        Some(e) => Err(anyhow!("msa token error: {e}: {:?}", resp.error_description)),
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
pub async fn xbl_auth(client: &reqwest::Client, msa_access_token: &str) -> anyhow::Result<(String, String)> {
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
pub async fn xsts_auth(client: &reqwest::Client, xbl_token: &str) -> anyhow::Result<(String, String)> {
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
pub async fn mc_login(client: &reqwest::Client, uhs: &str, xsts_token: &str) -> anyhow::Result<McLoginResponse> {
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

/// Minecraft access token -> player profile (uuid, name).
pub async fn mc_profile(client: &reqwest::Client, mc_access_token: &str) -> anyhow::Result<MsaProfile> {
    let resp = client
        .get(MC_PROFILE_URL)
        .bearer_auth(mc_access_token)
        .send()
        .await
        .context("GET mc profile")?;
    anyhow::ensure!(resp.status().is_success(), "mc profile returned {}", resp.status());
    let v: serde_json::Value = resp.json().await.context("parsing mc profile")?;
    let id = v["id"].as_str().ok_or_else(|| anyhow!("profile has no id"))?;
    let name = v["name"].as_str().ok_or_else(|| anyhow!("profile has no name"))?;
    Ok(MsaProfile {
        uuid: id.to_string(),
        name: name.to_string(),
    })
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
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
                + mc.expires_in as i64
        ),
        profile: Some(profile),
    })
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
        let profile = MsaProfile {
            uuid: "069a79f444e94726a5befca90e38aaf5".into(),
            name: "Notch".into(),
        };
        assert_eq!(profile.name, "Notch");
    }

    #[test]
    fn builds_mc_identity_token() {
        let req = McLoginRequest { identity_token: "XBL3.0 x=uhs;xsts".into() };
        let j = serde_json::to_value(&req).unwrap();
        assert_eq!(j["identityToken"], "XBL3.0 x=uhs;xsts");
    }
}
