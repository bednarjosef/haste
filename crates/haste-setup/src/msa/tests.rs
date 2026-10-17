//! Unit tests for the Microsoft auth module (fixtures only, no network).

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
