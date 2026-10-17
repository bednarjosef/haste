//! Where the Microsoft login chain talks to. The defaults are the real Microsoft, Xbox Live and
//! Minecraft services hosts; tests replace the base URLs with an in-process mock so the whole
//! chain (device code -> token polling -> XBL -> XSTS -> login_with_xbox -> entitlements ->
//! profile -> certificates) runs without a real app id and without touching the network.

use std::time::Duration;

/// The real OAuth host (consumers tenant, as the vanilla launcher uses).
pub const DEFAULT_LOGIN_BASE: &str = "https://login.microsoftonline.com";
/// The real Xbox Live user-authentication host.
pub const DEFAULT_XBL_BASE: &str = "https://user.auth.xboxlive.com";
/// The real XSTS host.
pub const DEFAULT_XSTS_BASE: &str = "https://xsts.auth.xboxlive.com";
/// The real Minecraft services host.
pub const DEFAULT_SERVICES_BASE: &str = "https://api.minecraftservices.com";

/// Device-flow poll interval when the authorization server sends none (RFC 8628 section 3.2).
pub const DEFAULT_POLL_INTERVAL_SECS: u64 = 5;
/// How much a `slow_down` widens the poll interval (RFC 8628 section 3.5).
pub const SLOW_DOWN_STEP_SECS: u64 = 5;

/// Base URLs plus the client id and the time unit polling counts in. One value is threaded
/// through the whole chain so a test can point every step at a mock.
#[derive(Debug, Clone)]
pub struct MsaEndpoints {
    /// `{login_base}/consumers/oauth2/v2.0/devicecode` and `.../token`.
    pub login_base: String,
    /// `{xbl_base}/user/authenticate`.
    pub xbl_base: String,
    /// `{xsts_base}/xsts/authorize`.
    pub xsts_base: String,
    /// `{services_base}/authentication/login_with_xbox`, `/entitlements/mcstore`,
    /// `/minecraft/profile`, `/player/certificates`.
    pub services_base: String,
    /// The Azure application (client) id; `None` means the user has not configured one.
    pub client_id: Option<String>,
    /// What one "second" of the device flow's `interval`/`expires_in` sleeps for. Real runs use
    /// one second; the mock test shrinks it so a slow_down round trip takes milliseconds.
    pub poll_time_unit: Duration,
}

impl Default for MsaEndpoints {
    fn default() -> Self {
        Self::from_env()
    }
}

impl MsaEndpoints {
    /// The real hosts, with the client id read from `HASTE_MSA_CLIENT_ID`.
    pub fn from_env() -> Self {
        Self {
            login_base: DEFAULT_LOGIN_BASE.to_string(),
            xbl_base: DEFAULT_XBL_BASE.to_string(),
            xsts_base: DEFAULT_XSTS_BASE.to_string(),
            services_base: DEFAULT_SERVICES_BASE.to_string(),
            client_id: super::client_id(),
            poll_time_unit: Duration::from_secs(1),
        }
    }

    /// Every host on one base URL (a local mock), with a fixed client id and millisecond polling.
    pub fn mock(base: &str, client_id: &str) -> Self {
        let base = base.trim_end_matches('/').to_string();
        Self {
            login_base: base.clone(),
            xbl_base: base.clone(),
            xsts_base: base.clone(),
            services_base: base,
            client_id: Some(client_id.to_string()),
            poll_time_unit: Duration::from_millis(1),
        }
    }

    pub fn device_code_url(&self) -> String {
        format!("{}/consumers/oauth2/v2.0/devicecode", self.login_base)
    }
    pub fn token_url(&self) -> String {
        format!("{}/consumers/oauth2/v2.0/token", self.login_base)
    }
    pub fn xbl_auth_url(&self) -> String {
        format!("{}/user/authenticate", self.xbl_base)
    }
    pub fn xsts_auth_url(&self) -> String {
        format!("{}/xsts/authorize", self.xsts_base)
    }
    pub fn mc_login_url(&self) -> String {
        format!("{}/authentication/login_with_xbox", self.services_base)
    }
    pub fn mc_entitlements_url(&self) -> String {
        format!("{}/entitlements/mcstore", self.services_base)
    }
    pub fn mc_profile_url(&self) -> String {
        format!("{}/minecraft/profile", self.services_base)
    }
    pub fn mc_certificates_url(&self) -> String {
        format!("{}/player/certificates", self.services_base)
    }

    /// The configured client id or the clear "not set" error the CLI shows.
    pub fn require_client_id(&self) -> anyhow::Result<&str> {
        self.client_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "HASTE_MSA_CLIENT_ID is not set; haste ships no Microsoft client id. \
                     Set it to an Azure application (client) id and retry."
                )
            })
    }
}
