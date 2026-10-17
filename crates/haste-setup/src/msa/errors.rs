//! The failures a Microsoft login can end in, mapped to messages that tell the player what to
//! do: OAuth device-flow errors (RFC 8628 section 3.5), XSTS `XErr` codes, and the Minecraft
//! services answers for accounts without the game or without a profile.

use serde::Deserialize;

/// A login failure with a player-facing message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MsaError {
    /// The device code expired before the user approved it (`expired_token`).
    #[error("the sign-in code expired, run `haste login` again")]
    CodeExpired,
    /// The user declined the sign-in in the browser (`authorization_declined`).
    #[error("the sign-in was declined in the browser")]
    Declined,
    /// The server did not recognise the device code (`bad_verification_code`).
    #[error("Microsoft did not recognise the sign-in code, run `haste login` again")]
    BadVerificationCode,
    /// Any other OAuth error (`invalid_client`, `invalid_grant` on refresh, ...).
    #[error("Microsoft sign-in failed: {code}: {description}")]
    OAuth { code: String, description: String },
    /// XSTS refused: the Microsoft account has no Xbox profile (XErr 2148916233).
    #[error("this Microsoft account has no Xbox profile; create one at https://www.xbox.com/live and try again")]
    NoXboxAccount,
    /// XSTS refused: Xbox Live is not available in the account's country (XErr 2148916235).
    #[error("Xbox Live is not available in this account's country")]
    XboxLiveUnavailable,
    /// XSTS refused: the account needs adult verification (XErr 2148916236, 2148916237).
    #[error("this account needs adult verification on the Xbox page before it can sign in")]
    AdultVerificationNeeded,
    /// XSTS refused: a child account that is not in a family yet (XErr 2148916238).
    #[error(
        "this is a child account; an adult must add it to a Microsoft family before it can play"
    )]
    ChildAccountNeedsFamily,
    /// XSTS refused with a code Haste does not know.
    #[error("Xbox sign-in refused (XErr {xerr}): {message}")]
    XstsRefused { xerr: u64, message: String },
    /// Minecraft services say the account owns no copy of the game.
    #[error("this account does not own Minecraft: Java Edition")]
    NotEntitled,
    /// The account owns the game but never created a profile (Game Pass and new purchases).
    #[error(
        "this account has no Minecraft profile yet; create one in the official launcher first"
    )]
    NoProfile,
    /// Any other Minecraft services failure.
    #[error("Minecraft services {step} failed: HTTP {status}: {body}")]
    Services {
        step: &'static str,
        status: u16,
        body: String,
    },
}

/// Maps an OAuth token-endpoint error code (RFC 8628 section 3.5 and RFC 6749) to its error.
/// `authorization_pending` and `slow_down` are not errors and are handled by the poll loop.
pub fn oauth_error(code: &str, description: Option<&str>) -> MsaError {
    match code {
        "expired_token" => MsaError::CodeExpired,
        "authorization_declined" | "access_denied" => MsaError::Declined,
        "bad_verification_code" => MsaError::BadVerificationCode,
        _ => MsaError::OAuth {
            code: code.to_string(),
            description: description.unwrap_or_default().to_string(),
        },
    }
}

/// XSTS error body (`HTTP 401`): `{"Identity":"0","XErr":2148916233,"Message":"","Redirect":"..."}`.
#[derive(Debug, Clone, Deserialize)]
pub struct XstsErrorBody {
    #[serde(rename = "XErr", default)]
    pub xerr: u64,
    #[serde(rename = "Message", default)]
    pub message: String,
}

/// XErr: the Microsoft account has no Xbox profile.
pub const XERR_NO_XBOX_ACCOUNT: u64 = 2148916233;
/// XErr: Xbox Live is banned in the account's country.
pub const XERR_XBOX_LIVE_UNAVAILABLE: u64 = 2148916235;
/// XErr: the account needs adult verification (South Korea).
pub const XERR_ADULT_VERIFICATION: u64 = 2148916236;
/// XErr: the account needs adult verification (South Korea, second code).
pub const XERR_ADULT_VERIFICATION_2: u64 = 2148916237;
/// XErr: the account is a child and not part of a family yet.
pub const XERR_CHILD_ACCOUNT: u64 = 2148916238;

/// Maps an XSTS `XErr` to its error.
pub fn xsts_error(xerr: u64, message: &str) -> MsaError {
    match xerr {
        XERR_NO_XBOX_ACCOUNT => MsaError::NoXboxAccount,
        XERR_XBOX_LIVE_UNAVAILABLE => MsaError::XboxLiveUnavailable,
        XERR_ADULT_VERIFICATION | XERR_ADULT_VERIFICATION_2 => MsaError::AdultVerificationNeeded,
        XERR_CHILD_ACCOUNT => MsaError::ChildAccountNeedsFamily,
        _ => MsaError::XstsRefused {
            xerr,
            message: message.to_string(),
        },
    }
}

/// Maps a non-success XSTS response (status and body) to its error: a parsable `XErr` body
/// goes through [`xsts_error`], anything else is reported with the raw body.
pub fn xsts_failure(status: u16, body: &str) -> MsaError {
    match serde_json::from_str::<XstsErrorBody>(body) {
        Ok(b) if b.xerr != 0 => xsts_error(b.xerr, &b.message),
        _ => MsaError::Services {
            step: "xsts authorize",
            status,
            body: body.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_codes_map_to_clear_messages() {
        assert_eq!(oauth_error("expired_token", None), MsaError::CodeExpired);
        assert_eq!(
            oauth_error("authorization_declined", None),
            MsaError::Declined
        );
        assert_eq!(
            oauth_error("bad_verification_code", None),
            MsaError::BadVerificationCode
        );
        let other = oauth_error("invalid_grant", Some("AADSTS70000: refresh token expired"));
        assert_eq!(
            other,
            MsaError::OAuth {
                code: "invalid_grant".into(),
                description: "AADSTS70000: refresh token expired".into()
            }
        );
        assert!(other.to_string().contains("invalid_grant"));
        assert!(MsaError::CodeExpired.to_string().contains("haste login"));
    }

    #[test]
    fn xsts_xerr_codes_map_to_clear_messages() {
        assert_eq!(xsts_error(2148916233, ""), MsaError::NoXboxAccount);
        assert_eq!(xsts_error(2148916235, ""), MsaError::XboxLiveUnavailable);
        assert_eq!(
            xsts_error(2148916236, ""),
            MsaError::AdultVerificationNeeded
        );
        assert_eq!(
            xsts_error(2148916237, ""),
            MsaError::AdultVerificationNeeded
        );
        assert_eq!(
            xsts_error(2148916238, ""),
            MsaError::ChildAccountNeedsFamily
        );
        assert_eq!(
            xsts_error(42, "odd"),
            MsaError::XstsRefused {
                xerr: 42,
                message: "odd".into()
            }
        );
        assert!(MsaError::NoXboxAccount.to_string().contains("Xbox profile"));
        assert!(MsaError::ChildAccountNeedsFamily
            .to_string()
            .contains("child account"));
    }

    #[test]
    fn xsts_failure_parses_the_real_body_shape() {
        let body = r#"{"Identity":"0","XErr":2148916238,"Message":"","Redirect":"https://start.ui.xboxlive.com/AddChildToFamily"}"#;
        assert_eq!(xsts_failure(401, body), MsaError::ChildAccountNeedsFamily);
        // a body without XErr keeps the status and text so the player can report it
        assert_eq!(
            xsts_failure(503, "gateway down"),
            MsaError::Services {
                step: "xsts authorize",
                status: 503,
                body: "gateway down".into()
            }
        );
    }
}
