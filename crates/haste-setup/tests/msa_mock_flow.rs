//! The whole Microsoft login chain against an in-process HTTP mock: device code -> token
//! polling (one `authorization_pending`, one `slow_down`, then the token) -> Xbox Live
//! user.authenticate -> XSTS authorize -> login_with_xbox -> entitlements -> profile, the
//! stored account's refresh-token reuse on the next launch (plus chat-signing certificates),
//! and logout deleting the stored account. Nothing here talks to a real Microsoft or Mojang
//! host: every base URL points at the mock, and the client id is a made-up string.

use haste_setup::config::Config;
use haste_setup::msa::{self, MsaEndpoints, MsaError};
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const CLIENT_ID: &str = "00000000-mock-app-id";
const DEVICE_CODE: &str = "device-code-123";
const USER_CODE: &str = "ABCD-EFGH";
const MSA_ACCESS: &str = "msa-access-token";
const MSA_REFRESH: &str = "msa-refresh-token";
const MSA_REFRESH_2: &str = "msa-refresh-token-2";
const XBL_TOKEN: &str = "xbl-token";
const XSTS_TOKEN: &str = "xsts-token";
const UHS: &str = "user-hash-42";
const MC_TOKEN: &str = "mc-access-token";
const MC_TOKEN_2: &str = "mc-access-token-2";

/// What the mock should answer at the XSTS step.
#[derive(Clone, Copy)]
enum XstsMode {
    Ok,
    XErr(u64),
}

/// The mock's script and the record of what it saw.
struct Mock {
    /// Token polls so far (the first answers `authorization_pending`, the second `slow_down`).
    token_polls: AtomicUsize,
    /// Which MSA access token the refresh grant hands out next.
    xsts: XstsMode,
    entitled: bool,
    has_profile: bool,
    /// Every request, in order: method, path, body, and the Authorization header if any.
    log: Mutex<Vec<(String, String, String, Option<String>)>>,
}

impl Mock {
    fn new(xsts: XstsMode, entitled: bool, has_profile: bool) -> Arc<Self> {
        Arc::new(Self {
            token_polls: AtomicUsize::new(0),
            xsts,
            entitled,
            has_profile,
            log: Mutex::new(Vec::new()),
        })
    }

    fn paths(&self) -> Vec<String> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .map(|(m, p, _, _)| format!("{m} {p}"))
            .collect()
    }
}

fn json(status: StatusCode, body: serde_json::Value) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body.to_string())))
        .unwrap()
}

fn form(body: &str) -> HashMap<String, String> {
    body.split('&')
        .filter(|kv| !kv.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (
                k.to_string(),
                v.replace('+', " ")
                    .replace("%3A", ":")
                    .replace("%2F", "/")
                    .replace("%2D", "-"),
            )
        })
        .collect()
}

async fn handle(mock: Arc<Mock>, req: Request<Incoming>) -> Response<Full<Bytes>> {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let auth = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let body =
        String::from_utf8(req.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    mock.log
        .lock()
        .unwrap()
        .push((method.clone(), path.clone(), body.clone(), auth.clone()));
    let bearer = |token: &str| auth.as_deref() == Some(&format!("Bearer {token}"));
    match (method.as_str(), path.as_str()) {
        ("POST", "/consumers/oauth2/v2.0/devicecode") => {
            let f = form(&body);
            assert_eq!(f["client_id"], CLIENT_ID);
            assert_eq!(f["scope"], msa::MSA_SCOPE);
            json(
                StatusCode::OK,
                serde_json::json!({
                    "user_code": USER_CODE,
                    "device_code": DEVICE_CODE,
                    "verification_uri": "https://www.microsoft.com/link",
                    "expires_in": 900,
                    "interval": 5,
                    "message": format!("open https://www.microsoft.com/link and enter {USER_CODE}"),
                }),
            )
        }
        ("POST", "/consumers/oauth2/v2.0/token") => {
            let f = form(&body);
            assert_eq!(f["client_id"], CLIENT_ID);
            match f["grant_type"].as_str() {
                "urn:ietf:params:oauth:grant-type:device_code" => {
                    assert_eq!(f["device_code"], DEVICE_CODE);
                    let n = mock.token_polls.fetch_add(1, Ordering::SeqCst);
                    match n {
                        0 => json(
                            StatusCode::BAD_REQUEST,
                            serde_json::json!({"error": "authorization_pending", "error_description": "pending"}),
                        ),
                        1 => json(
                            StatusCode::BAD_REQUEST,
                            serde_json::json!({"error": "slow_down", "error_description": "slow down"}),
                        ),
                        _ => json(
                            StatusCode::OK,
                            serde_json::json!({
                                "token_type": "Bearer", "scope": "XboxLive.signin offline_access",
                                "expires_in": 3600, "access_token": MSA_ACCESS, "refresh_token": MSA_REFRESH,
                            }),
                        ),
                    }
                }
                "refresh_token" => {
                    assert_eq!(f["refresh_token"], MSA_REFRESH);
                    json(
                        StatusCode::OK,
                        serde_json::json!({
                            "token_type": "Bearer", "expires_in": 3600,
                            "access_token": MSA_ACCESS, "refresh_token": MSA_REFRESH_2,
                        }),
                    )
                }
                other => panic!("unexpected grant_type {other}"),
            }
        }
        ("POST", "/user/authenticate") => {
            let v: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!(v["Properties"]["AuthMethod"], "RPS");
            assert_eq!(v["Properties"]["SiteName"], "user.auth.xboxlive.com");
            assert_eq!(v["Properties"]["RpsTicket"], format!("d={MSA_ACCESS}"));
            assert_eq!(v["RelyingParty"], "http://auth.xboxlive.com");
            assert_eq!(v["TokenType"], "JWT");
            json(
                StatusCode::OK,
                serde_json::json!({
                    "IssueInstant": "2026-10-17T00:00:00.0000000Z",
                    "NotAfter": "2026-10-18T00:00:00.0000000Z",
                    "Token": XBL_TOKEN,
                    "DisplayClaims": {"xui": [{"uhs": UHS}]},
                }),
            )
        }
        ("POST", "/xsts/authorize") => {
            let v: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!(v["Properties"]["SandboxId"], "RETAIL");
            assert_eq!(v["Properties"]["UserTokens"][0], XBL_TOKEN);
            assert_eq!(v["RelyingParty"], "rp://api.minecraftservices.com/");
            match mock.xsts {
                XstsMode::Ok => json(
                    StatusCode::OK,
                    serde_json::json!({
                        "IssueInstant": "2026-10-17T00:00:00.0000000Z",
                        "NotAfter": "2026-10-18T00:00:00.0000000Z",
                        "Token": XSTS_TOKEN,
                        "DisplayClaims": {"xui": [{"uhs": UHS}]},
                    }),
                ),
                XstsMode::XErr(code) => json(
                    StatusCode::UNAUTHORIZED,
                    serde_json::json!({
                        "Identity": "0", "XErr": code, "Message": "",
                        "Redirect": "https://start.ui.xboxlive.com/CreateAccount",
                    }),
                ),
            }
        }
        ("POST", "/authentication/login_with_xbox") => {
            let v: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!(v["identityToken"], format!("XBL3.0 x={UHS};{XSTS_TOKEN}"));
            // the second login (after a refresh) hands out a different Minecraft token
            let logins = mock
                .log
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, p, _, _)| p == "/authentication/login_with_xbox")
                .count();
            let token = if logins > 1 { MC_TOKEN_2 } else { MC_TOKEN };
            json(
                StatusCode::OK,
                serde_json::json!({
                    "username": "069a79f444e94726a5befca90e38aaf5",
                    "roles": [], "access_token": token, "token_type": "Bearer", "expires_in": 86400,
                }),
            )
        }
        ("GET", "/entitlements/mcstore") => {
            assert!(
                bearer(MC_TOKEN) || bearer(MC_TOKEN_2),
                "entitlements without the mc token"
            );
            let items = if mock.entitled {
                serde_json::json!([
                    {"name": "product_minecraft", "signature": "sig"},
                    {"name": "game_minecraft", "signature": "sig"},
                ])
            } else {
                serde_json::json!([])
            };
            json(
                StatusCode::OK,
                serde_json::json!({"items": items, "signature": "jwt", "keyId": "1"}),
            )
        }
        ("GET", "/minecraft/profile") => {
            assert!(
                bearer(MC_TOKEN) || bearer(MC_TOKEN_2),
                "profile without the mc token"
            );
            if mock.has_profile {
                json(
                    StatusCode::OK,
                    serde_json::json!({
                        "id": "069a79f444e94726a5befca90e38aaf5", "name": "Notch",
                        "skins": [], "capes": [],
                    }),
                )
            } else {
                json(
                    StatusCode::NOT_FOUND,
                    serde_json::json!({"path": "/minecraft/profile", "errorType": "NOT_FOUND", "error": "NOT_FOUND"}),
                )
            }
        }
        ("POST", "/player/certificates") => {
            assert!(
                bearer(MC_TOKEN_2),
                "certificates must use the refreshed mc token"
            );
            json(
                StatusCode::OK,
                serde_json::json!({
                    "keyPair": {
                        "privateKey": "-----BEGIN RSA PRIVATE KEY-----\nMIIEvgAB\n-----END RSA PRIVATE KEY-----\n",
                        "publicKey": "-----BEGIN RSA PUBLIC KEY-----\nMFwwDQYD\n-----END RSA PUBLIC KEY-----\n"
                    },
                    "publicKeySignature": "3q2+7w==",
                    "publicKeySignatureV2": "AQIDBAU=",
                    "expiresAt": "2099-01-01T00:00:00.000000000Z",
                    "refreshedAfter": "2098-12-30T00:00:00.000000000Z",
                }),
            )
        }
        _ => json(
            StatusCode::NOT_FOUND,
            serde_json::json!({"error": format!("no mock route for {method} {path}")}),
        ),
    }
}

async fn spawn_mock(mock: Arc<Mock>) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let mock = mock.clone();
            let service = service_fn(move |req| {
                let mock = mock.clone();
                async move { Ok::<_, hyper::Error>(handle(mock, req).await) }
            });
            tokio::spawn(async move {
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    addr
}

fn endpoints_for(addr: SocketAddr) -> MsaEndpoints {
    let e = MsaEndpoints::mock(&format!("http://{addr}"), CLIENT_ID);
    assert!(!e.login_base.contains("microsoft"));
    e
}

#[tokio::test]
async fn full_device_code_login_then_refresh_then_logout() {
    let mock = Mock::new(XstsMode::Ok, true, true);
    let addr = spawn_mock(mock.clone()).await;
    let endpoints = endpoints_for(addr);

    // `haste login`: device code shown to the user, token polled (pending, slow_down, token),
    // then the Xbox and Minecraft services chain.
    let mut shown = None;
    let tokens = msa::device_code_login(&endpoints, |d| {
        shown = Some((d.verification_uri.clone(), d.user_code.clone()));
    })
    .await
    .unwrap();
    assert_eq!(
        shown,
        Some((
            "https://www.microsoft.com/link".to_string(),
            USER_CODE.to_string()
        ))
    );
    assert_eq!(tokens.access_token, MC_TOKEN);
    assert_eq!(tokens.refresh_token, MSA_REFRESH);
    let profile = tokens.profile.clone().unwrap();
    assert_eq!(profile.name, "Notch");
    assert_eq!(profile.uuid, "069a79f444e94726a5befca90e38aaf5");
    assert!(tokens.expires_at.unwrap() > 0);
    assert_eq!(mock.token_polls.load(Ordering::SeqCst), 3);
    assert_eq!(
        mock.paths(),
        vec![
            "POST /consumers/oauth2/v2.0/devicecode",
            "POST /consumers/oauth2/v2.0/token",
            "POST /consumers/oauth2/v2.0/token",
            "POST /consumers/oauth2/v2.0/token",
            "POST /user/authenticate",
            "POST /xsts/authorize",
            "POST /authentication/login_with_xbox",
            "GET /entitlements/mcstore",
            "GET /minecraft/profile",
        ]
    );

    // Stored in the config like the CLI does.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let mut config = Config {
        msa: Some(tokens),
        ..Config::default()
    };
    config.save_to(&path).unwrap();

    // Next launch: the stored Minecraft token has expired, so the refresh token is used (no
    // browser step), and the chat-signing certificates are fetched with the new token.
    let mut config = Config::load_from(&path).unwrap();
    config.msa.as_mut().unwrap().expires_at = Some(0);
    mock.log.lock().unwrap().clear();
    let client = reqwest::Client::new();
    msa::ensure_fresh_account(&client, &endpoints, &mut config)
        .await
        .unwrap();
    let refreshed = config.msa.as_ref().unwrap();
    assert_eq!(refreshed.access_token, MC_TOKEN_2);
    assert_eq!(refreshed.refresh_token, MSA_REFRESH_2);
    assert_eq!(refreshed.profile.as_ref().unwrap().name, "Notch");
    assert!(config.player_certificates.is_some());
    assert_eq!(
        mock.paths(),
        vec![
            "POST /consumers/oauth2/v2.0/token",
            "POST /user/authenticate",
            "POST /xsts/authorize",
            "POST /authentication/login_with_xbox",
            "GET /entitlements/mcstore",
            "GET /minecraft/profile",
            "POST /player/certificates",
        ]
    );
    config.save_to(&path).unwrap();

    // A launch with a still-valid token and fresh certificates talks to nobody.
    let mut config = Config::load_from(&path).unwrap();
    mock.log.lock().unwrap().clear();
    msa::ensure_fresh_account(&client, &endpoints, &mut config)
        .await
        .unwrap();
    assert!(mock.paths().is_empty());

    // `haste logout`: the account and its certificates are gone from the saved config.
    let removed = msa::remove_stored_account(&path).unwrap().unwrap();
    assert_eq!(removed.profile.unwrap().name, "Notch");
    let after = Config::load_from(&path).unwrap();
    assert!(after.msa.is_none());
    assert!(after.player_certificates.is_none());
    assert!(msa::remove_stored_account(&path).unwrap().is_none());
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(!raw.contains(MC_TOKEN_2) && !raw.contains(MSA_REFRESH_2));
}

#[tokio::test]
async fn xsts_no_xbox_account_is_a_clear_error() {
    let mock = Mock::new(XstsMode::XErr(2148916233), true, true);
    let addr = spawn_mock(mock).await;
    let endpoints = endpoints_for(addr);
    let err = msa::device_code_login(&endpoints, |_| {})
        .await
        .unwrap_err();
    assert_eq!(
        err.downcast_ref::<MsaError>(),
        Some(&MsaError::NoXboxAccount)
    );
    assert!(err.to_string().contains("no Xbox profile"));
}

#[tokio::test]
async fn xsts_child_account_is_a_clear_error() {
    let mock = Mock::new(XstsMode::XErr(2148916238), true, true);
    let addr = spawn_mock(mock).await;
    let endpoints = endpoints_for(addr);
    let err = msa::device_code_login(&endpoints, |_| {})
        .await
        .unwrap_err();
    assert_eq!(
        err.downcast_ref::<MsaError>(),
        Some(&MsaError::ChildAccountNeedsFamily)
    );
}

#[tokio::test]
async fn account_without_the_game_is_rejected_at_entitlements() {
    let mock = Mock::new(XstsMode::Ok, false, true);
    let addr = spawn_mock(mock.clone()).await;
    let endpoints = endpoints_for(addr);
    let err = msa::device_code_login(&endpoints, |_| {})
        .await
        .unwrap_err();
    assert_eq!(err.downcast_ref::<MsaError>(), Some(&MsaError::NotEntitled));
    // the profile is never asked for
    assert!(!mock
        .paths()
        .iter()
        .any(|p| p.ends_with("/minecraft/profile")));
}

#[tokio::test]
async fn account_without_a_profile_is_a_clear_error() {
    let mock = Mock::new(XstsMode::Ok, true, false);
    let addr = spawn_mock(mock).await;
    let endpoints = endpoints_for(addr);
    let err = msa::device_code_login(&endpoints, |_| {})
        .await
        .unwrap_err();
    assert_eq!(err.downcast_ref::<MsaError>(), Some(&MsaError::NoProfile));
}

#[tokio::test]
async fn expired_device_code_stops_polling() {
    // a device code that expires after 3 "seconds" (milliseconds here) with a 5-second
    // interval: the loop gives up before the first poll
    let mock = Mock::new(XstsMode::Ok, true, true);
    let addr = spawn_mock(mock.clone()).await;
    let endpoints = endpoints_for(addr);
    let client = reqwest::Client::new();
    let mut device = msa::begin_device_code(&client, &endpoints).await.unwrap();
    device.expires_in = 3;
    let err = msa::poll_until_token(&client, &endpoints, &device)
        .await
        .unwrap_err();
    assert_eq!(err.downcast_ref::<MsaError>(), Some(&MsaError::CodeExpired));
    assert_eq!(mock.token_polls.load(Ordering::SeqCst), 0);
}

#[test]
fn missing_client_id_fails_before_any_request() {
    let mut endpoints = MsaEndpoints::mock("http://127.0.0.1:9", CLIENT_ID);
    endpoints.client_id = None;
    let err = endpoints.require_client_id().unwrap_err();
    assert!(err.to_string().contains("HASTE_MSA_CLIENT_ID"));
}
