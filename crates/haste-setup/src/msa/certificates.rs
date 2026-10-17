//! Player certificates for chat signing (`POST /player/certificates`): the response parser, the
//! PEM and RFC 3339 helpers it needs, and the request. Split out of the login flow module.

use super::MsaEndpoints;
use crate::config::PlayerCertificates;
use crate::msa::MsaError;
use anyhow::{anyhow, Context};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::Deserialize;

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
    // optional fractional seconds: .fff... (truncated)
    let mut i = 19;
    if bytes.get(i) == Some(&b'.') {
        let start = i + 1;
        i = start;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        anyhow::ensure!(i > start, "{}", bad());
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
    Ok(secs)
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
    };
    if !v.expires_at.is_empty() {
        certs.expires_at = Some(parse_rfc3339(&v.expires_at)?);
    }
    if !v.refreshed_after.is_empty() {
        certs.refreshed_after = Some(parse_rfc3339(&v.refreshed_after)?);
    }
    Ok(certs)
}

/// Minecraft access token -> player profile key pair for chat signing (POST, empty body).
pub async fn player_certificates(
    client: &reqwest::Client,
    endpoints: &MsaEndpoints,
    mc_access_token: &str,
) -> anyhow::Result<PlayerCertificates> {
    let resp = client
        .post(endpoints.mc_certificates_url())
        .bearer_auth(mc_access_token)
        .body(String::new())
        .send()
        .await
        .context("POST player certificates")?;
    let status = resp.status();
    let body = resp.text().await.context("reading certificates body")?;
    if !status.is_success() {
        return Err(MsaError::Services {
            step: "player certificates",
            status: status.as_u16(),
            body,
        }
        .into());
    }
    parse_player_certificates(&body)
}
