//! The server resource pack prompt against the in-process fake server
//! (vanilla `handleResourcePackPush` and its `PackConfirmScreen`, the
//! `ServerPackManager` statuses): what asks, what the answers send, in which
//! order, and that the join keeps answering keep-alives while the screen is
//! open.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use tokio::net::TcpListener;
use uuid::Uuid;

use super::tests::{serve_one_http, test_tables, Server};
use super::*;
use crate::packets::configuration::clientbound as ccb;
use crate::packets::configuration::ServerboundConfiguration;
use crate::resource_pack_status as st;

/// A confirm screen the test scripts: it records what it showed and gives
/// `answer` once `ready` is set (so the server can check that the join keeps
/// going while it waits).
#[derive(Default)]
struct FakeScreen {
    shown: Mutex<Vec<Vec<(Uuid, bool)>>>,
    answered: Mutex<Vec<PackAnswer>>,
    answer: Mutex<Option<bool>>,
    ready: AtomicBool,
}

impl PackPrompter for FakeScreen {
    fn show(&self, pushes: &[PendingPackPush]) {
        self.shown
            .lock()
            .unwrap()
            .push(pushes.iter().map(|p| (p.id, p.forced)).collect());
    }
    fn take_answer(&self) -> Option<bool> {
        if self.ready.load(Ordering::SeqCst) {
            self.answer.lock().unwrap().take()
        } else {
            None
        }
    }
    fn answered(&self, answer: PackAnswer) {
        self.answered.lock().unwrap().push(answer);
    }
}

fn sha1_hex(body: &[u8]) -> String {
    use sha1::Digest as _;
    let mut h = sha1::Sha1::new();
    h.update(body);
    let d: [u8; 20] = h.finalize().into();
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// A pack served once over HTTP and its push.
async fn served_push(id: u128, forced: bool) -> ccb::ResourcePackPush {
    let body = Bytes::from(format!("pack body {id}"));
    let hash = sha1_hex(&body);
    let url = serve_one_http(body).await;
    ccb::ResourcePackPush {
        id: Uuid::from_u128(id),
        url: url.to_string(),
        hash,
        forced,
        prompt: Some(haste_core::Component::text("please")),
    }
}

async fn cfg_status(s: &mut Server) -> (u128, i32) {
    match s.recv_cfg().await {
        ServerboundConfiguration::ResourcePack(r) => (r.id.as_u128(), r.result),
        other => panic!("expected a resource pack status, got {other:?}"),
    }
}

fn options(
    addr: std::net::SocketAddr,
    policy: ResourcePackPolicy,
    screen: &Arc<FakeScreen>,
    downloads: &std::path::Path,
) -> ConnectOptions {
    let mut opts = ConnectOptions::new(
        addr.to_string(),
        Account::Offline {
            username: "tester".into(),
        },
    );
    opts.tables = Some(test_tables());
    opts.resource_packs = policy;
    opts.pack_prompt = Some(screen.clone() as PackPromptHook);
    opts.downloads_dir = Some(downloads.to_path_buf());
    opts
}

/// Prompt: two pushes wait on one screen (shown again for the second, like
/// `addOrUpdatePackPrompt`), keep-alives are answered meanwhile and no status
/// goes out; yes sends ACCEPTED for both, then DOWNLOADED for both, then
/// SUCCESSFULLY_LOADED for both, and settles the policy, so a third push
/// downloads without asking (and the reload answers SUCCESSFULLY_LOADED for
/// every live pack again, like vanilla's reload callback).
#[tokio::test]
async fn prompt_batches_pushes_keeps_alive_and_follows_yes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let screen = Arc::new(FakeScreen::default());
    *screen.answer.lock().unwrap() = Some(true);
    let (p1, p2, p3) = (
        served_push(1, false).await,
        served_push(2, false).await,
        served_push(3, true).await,
    );
    let server_screen = screen.clone();
    let server = tokio::spawn(async move {
        let mut s = Server::accept(&listener, test_tables(), INTENT_LOGIN).await;
        s.expect_hello().await;
        s.send(State::Configuration, &p1).await;
        s.send(State::Configuration, &p2).await;
        // The screen is open: the join still answers a keep-alive.
        s.send(State::Configuration, &ccb::KeepAlive { id: 77 })
            .await;
        match s.recv_cfg().await {
            ServerboundConfiguration::KeepAlive(k) => assert_eq!(k.id, 77),
            other => panic!("expected the keep-alive answer first, got {other:?}"),
        }
        server_screen.ready.store(true, Ordering::SeqCst);
        let mut got = Vec::new();
        for _ in 0..6 {
            got.push(cfg_status(&mut s).await);
        }
        s.send(State::Configuration, &p3).await;
        for _ in 0..5 {
            got.push(cfg_status(&mut s).await);
        }
        s.finish_with("plains", 5).await;
        got
    });
    let opts = options(addr, ResourcePackPolicy::Prompt, &screen, downloads.path());
    let (conn, _) = tokio::time::timeout(Duration::from_secs(10), Connection::connect(opts))
        .await
        .expect("connect timeout")
        .unwrap();
    let got = server.await.unwrap();
    assert_eq!(
        got,
        vec![
            (1, st::ACCEPTED),
            (2, st::ACCEPTED),
            (1, st::DOWNLOADED),
            (2, st::DOWNLOADED),
            (1, st::SUCCESSFULLY_LOADED),
            (2, st::SUCCESSFULLY_LOADED),
            (3, st::ACCEPTED),
            (3, st::DOWNLOADED),
            (1, st::SUCCESSFULLY_LOADED),
            (2, st::SUCCESSFULLY_LOADED),
            (3, st::SUCCESSFULLY_LOADED),
        ]
    );
    let shown = screen.shown.lock().unwrap().clone();
    let (u1, u2) = (Uuid::from_u128(1), Uuid::from_u128(2));
    assert_eq!(shown, vec![vec![(u1, false)], vec![(u1, false), (u2, false)]]);
    assert_eq!(
        *screen.answered.lock().unwrap(),
        vec![PackAnswer {
            accepted: true,
            required: false
        }]
    );
    assert_eq!(conn.resource_pack_policy(), ResourcePackPolicy::Accept);
    let ids: Vec<u128> = conn.server_packs().iter().map(|p| p.id.as_u128()).collect();
    assert_eq!(ids, vec![1, 2, 3]);
    conn.disconnect();
}

/// Declining a required pack: the client closes the connection itself with
/// vanilla's reason and sends no status.
#[tokio::test]
async fn declining_a_required_pack_disconnects_without_a_status() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let screen = Arc::new(FakeScreen::default());
    *screen.answer.lock().unwrap() = Some(false);
    screen.ready.store(true, Ordering::SeqCst);
    let push = served_push(9, true).await;
    let server = tokio::spawn(async move {
        let mut s = Server::accept(&listener, test_tables(), INTENT_LOGIN).await;
        s.expect_hello().await;
        s.send(State::Configuration, &push).await;
        // The next read ends: the client closed without a status.
        tokio::time::timeout(Duration::from_secs(5), s.dec.get_raw_packet())
            .await
            .expect("the client should close")
            .is_err()
    });
    let opts = options(addr, ResourcePackPolicy::Prompt, &screen, downloads.path());
    let err = tokio::time::timeout(Duration::from_secs(10), Connection::connect(opts))
        .await
        .expect("connect timeout")
        .err()
        .expect("the join must fail");
    assert!(matches!(err, ProtocolError::RequiredPackDeclined), "{err}");
    assert!(server.await.unwrap(), "no packet after the decline");
    assert_eq!(
        *screen.answered.lock().unwrap(),
        vec![PackAnswer {
            accepted: false,
            required: true
        }]
    );
}

/// Disabled: an optional pack is declined without asking; a required one
/// still asks, and yes then downloads it. An invalid URL answers INVALID_URL
/// before any asking. In play, a prompted push waits for the game thread's
/// answer; no declines it (DECLINED in the play state).
#[tokio::test]
async fn disabled_declines_optional_asks_for_required_and_play_waits() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let screen = Arc::new(FakeScreen::default());
    *screen.answer.lock().unwrap() = Some(true);
    screen.ready.store(true, Ordering::SeqCst);
    let optional = served_push(4, false).await;
    let required = served_push(5, true).await;
    let (play_tx, play_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let mut s = Server::accept(&listener, test_tables(), INTENT_LOGIN).await;
        s.expect_hello().await;
        let mut got = Vec::new();
        s.send(
            State::Configuration,
            &ccb::ResourcePackPush {
                id: Uuid::from_u128(6),
                url: "file:///etc/passwd".into(),
                hash: String::new(),
                forced: true,
                prompt: None,
            },
        )
        .await;
        got.push(cfg_status(&mut s).await);
        s.send(State::Configuration, &optional).await;
        got.push(cfg_status(&mut s).await);
        s.send(State::Configuration, &required).await;
        for _ in 0..3 {
            got.push(cfg_status(&mut s).await);
        }
        s.finish_with("plains", 5).await;
        // Play: the connection's policy is Accept now (the answer), so a
        // fresh Prompt is simulated by the test resetting it below.
        play_rx.await.unwrap();
        s.send(
            State::Play,
            &ccb::ResourcePackPush {
                id: Uuid::from_u128(8),
                url: "http://127.0.0.1:1/never.zip".into(),
                hash: String::new(),
                forced: false,
                prompt: None,
            },
        )
        .await;
        match s
            .recv(State::Play, crate::packets::play::ServerboundPlay::DECODERS)
            .await
        {
            ServerboundPlay::ResourcePack(r) => got.push((r.id.as_u128(), r.result)),
            other => panic!("expected a play resource pack status, got {other:?}"),
        }
        got
    });
    let opts = options(addr, ResourcePackPolicy::Decline, &screen, downloads.path());
    let (conn, _) = tokio::time::timeout(Duration::from_secs(10), Connection::connect(opts))
        .await
        .expect("connect timeout")
        .unwrap();
    assert_eq!(conn.resource_pack_policy(), ResourcePackPolicy::Accept);
    conn.packs.set_policy(ResourcePackPolicy::Prompt);
    play_tx.send(()).unwrap();
    // The game thread's side: the push waits until answered.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while conn.pending_pack_pushes().is_empty() {
        assert!(std::time::Instant::now() < deadline, "the play push never waited");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(conn.pending_pack_pushes()[0].id, Uuid::from_u128(8));
    let answer = conn.answer_server_packs(false).expect("a push was pending");
    assert!(!answer.disconnects());
    assert_eq!(answer.saved_status(), Some(Some(false)));
    assert!(conn.pending_pack_pushes().is_empty());
    let got = server.await.unwrap();
    assert_eq!(
        got,
        vec![
            (6, st::INVALID_URL),
            (4, st::DECLINED),
            (5, st::ACCEPTED),
            (5, st::DOWNLOADED),
            (5, st::SUCCESSFULLY_LOADED),
            (8, st::DECLINED),
        ]
    );
    // Only the required pack asked.
    assert_eq!(
        *screen.shown.lock().unwrap(),
        vec![vec![(Uuid::from_u128(5), true)]]
    );
    assert_eq!(conn.resource_pack_policy(), ResourcePackPolicy::Decline);
    conn.disconnect();
}

#[test]
fn answers_follow_the_confirm_screen_callback() {
    let push = |forced| PendingPackPush {
        id: Uuid::nil(),
        url: String::new(),
        hash: String::new(),
        forced,
        prompt: None,
    };
    // The newest push words the screen (vanilla `PackConfirmScreen.update`).
    let a = PackAnswer::for_pushes(false, &[push(true), push(false)]);
    assert!(!a.required && !a.disconnects());
    assert_eq!(a.saved_status(), Some(Some(false)));
    let a = PackAnswer::for_pushes(false, &[push(false), push(true)]);
    assert!(a.disconnects());
    assert_eq!(a.saved_status(), None, "a required decline keeps the status");
    let a = PackAnswer::for_pushes(true, &[push(true)]);
    assert_eq!(a.saved_status(), Some(Some(true)));
    assert_eq!(a.policy(), ResourcePackPolicy::Accept);

    use ResourcePackPolicy::*;
    assert!(pack_push_asks(Prompt, false) && pack_push_asks(Prompt, true));
    assert!(!pack_push_asks(Decline, false) && pack_push_asks(Decline, true));
    assert!(!pack_push_asks(Accept, false) && !pack_push_asks(Accept, true));
}
