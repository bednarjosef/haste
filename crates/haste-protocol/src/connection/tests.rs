//! Connection tests against an in-process fake server that speaks the same
//! state machine (written from the wiki's login/configuration sequence).

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use haste_core::chunk::{ChunkData, LightData};
use haste_core::{ChunkPos, Component, Identifier};
use rsa::pkcs8::EncodePublicKey;
use rsa::rand_core::OsRng;
use rsa::{Pkcs1v15Encrypt, RsaPrivateKey};
use tokio::net::TcpListener;
use uuid::Uuid;

use super::*;
use crate::packets::configuration::clientbound as ccb;
use crate::packets::configuration::ServerboundConfiguration;
use crate::packets::handshake::ServerboundHandshake;
use crate::packets::login::clientbound as lcb;
use crate::packets::login::ServerboundLogin;
use crate::packets::play::clientbound as pcb;
use crate::packets::status::ServerboundStatus as ServerboundStatusAlias;
use crate::packets::PacketIds;

const FAKE_UNKNOWN: &str = "minecraft:fake_unknown";

/// Ids assigned in our own declaration order, so the test needs no cache.
fn test_tables() -> Arc<ProtocolTables> {
    fn names<T>(d: &'static [(&'static str, crate::packets::DecodeFn<T>)]) -> Vec<&'static str> {
        d.iter().map(|(n, _)| *n).collect()
    }
    let mut play_cb = names(ClientboundPlay::DECODERS);
    play_cb.push(FAKE_UNKNOWN);
    let lists: Vec<(State, Direction, Vec<&'static str>)> = vec![
        (
            State::Handshake,
            Direction::Serverbound,
            names(ServerboundHandshake::DECODERS),
        ),
        (
            State::Status,
            Direction::Clientbound,
            names(ClientboundStatus::DECODERS),
        ),
        (
            State::Status,
            Direction::Serverbound,
            names(ServerboundStatusAlias::DECODERS),
        ),
        (
            State::Login,
            Direction::Clientbound,
            names(ClientboundLogin::DECODERS),
        ),
        (
            State::Login,
            Direction::Serverbound,
            names(ServerboundLogin::DECODERS),
        ),
        (
            State::Configuration,
            Direction::Clientbound,
            names(ClientboundConfiguration::DECODERS),
        ),
        (
            State::Configuration,
            Direction::Serverbound,
            names(ServerboundConfiguration::DECODERS),
        ),
        (State::Play, Direction::Clientbound, play_cb),
        (
            State::Play,
            Direction::Serverbound,
            names(ServerboundPlay::DECODERS),
        ),
    ];
    let borrowed: Vec<(State, Direction, &[&str])> = lists
        .iter()
        .map(|(s, d, v)| (*s, *d, v.as_slice()))
        .collect();
    Arc::new(ProtocolTables::default().with_packets(PacketIds::from_names(&borrowed)))
}

struct Server {
    dec: TcpNetworkDecoder<BufReader<OwnedReadHalf>>,
    enc: TcpNetworkEncoder<OwnedWriteHalf>,
    tables: Arc<ProtocolTables>,
}

impl Server {
    async fn send<P: Packet>(&mut self, state: State, p: &P) {
        let b =
            crate::packets::encode_with_id(&self.tables.packets, state, Direction::Clientbound, p)
                .expect("clientbound id");
        self.enc.write_packet(b).await.unwrap();
        self.enc.flush().await.unwrap();
    }
    async fn send_raw(&mut self, state: State, name: &str, body: &[u8]) {
        let mut w = Writer::new();
        w.var_int(
            self.tables
                .packets
                .id(state, Direction::Clientbound, name)
                .unwrap(),
        );
        w.bytes(body);
        self.enc.write_packet(w.into_inner().into()).await.unwrap();
        self.enc.flush().await.unwrap();
    }
    async fn recv<T>(
        &mut self,
        state: State,
        decoders: &'static [(&'static str, crate::packets::DecodeFn<T>)],
    ) -> T {
        let raw = tokio::time::timeout(Duration::from_secs(5), self.dec.get_raw_packet())
            .await
            .expect("server recv timeout")
            .unwrap();
        let d = Dispatch::new(
            &self.tables.packets,
            state,
            Direction::Serverbound,
            decoders,
        );
        match d.decode(raw.id, &raw.payload, Some(&self.tables)) {
            Decoded::Ok(p) => p,
            Decoded::NoDecoder => panic!("server: no decoder for id {} in {state:?}", raw.id),
            Decoded::Failed(n, e) => panic!("server: {n}: {e}"),
        }
    }
}

fn overworld_type() -> Bytes {
    let mut c = NbtCompound::new();
    c.put_int("min_y", -64);
    c.put_int("height", 384);
    c.put_bool("has_skylight", true);
    let mut w = Writer::new();
    w.nbt_compound(Some(&c));
    w.into_inner().into()
}

fn big_chunk() -> ChunkData {
    ChunkData {
        pos: ChunkPos::new(3, -4),
        heightmaps: vec![(4, vec![7; 37])],
        sections: Bytes::from((0..20_000u32).map(|i| (i % 251) as u8).collect::<Vec<u8>>()),
        block_entities: vec![],
        light: LightData::default(),
    }
}

/// Runs the server side of one login. Returns what it observed.
async fn serve(
    listener: TcpListener,
    tables: Arc<ProtocolTables>,
    encrypt: bool,
    compression: Option<i32>,
) -> Vec<String> {
    let (stream, _) = listener.accept().await.unwrap();
    let (r, w) = stream.into_split();
    let mut s = Server {
        dec: TcpNetworkDecoder::new(BufReader::new(r)),
        enc: TcpNetworkEncoder::new(w),
        tables,
    };
    let mut log = Vec::new();

    let ServerboundHandshake::Intention(h) = s
        .recv(State::Handshake, ServerboundHandshake::DECODERS)
        .await
    else {
        panic!()
    };
    assert_eq!(h.intent, INTENT_LOGIN);
    assert_eq!(h.protocol_version, 777);
    let ServerboundLogin::Hello(hello) = s.recv(State::Login, ServerboundLogin::DECODERS).await
    else {
        panic!("expected hello")
    };
    log.push(format!("hello {}", hello.name));

    if encrypt {
        let key = RsaPrivateKey::new(&mut OsRng, 1024).unwrap();
        let der = key.to_public_key().to_public_key_der().unwrap();
        let token = [9u8, 8, 7, 6];
        s.send(
            State::Login,
            &lcb::Hello {
                server_id: String::new(),
                public_key: Bytes::copy_from_slice(der.as_bytes()),
                verify_token: Bytes::copy_from_slice(&token),
                should_authenticate: false,
            },
        )
        .await;
        let ServerboundLogin::Key(k) = s.recv(State::Login, ServerboundLogin::DECODERS).await
        else {
            panic!("expected key")
        };
        let secret = key.decrypt(Pkcs1v15Encrypt, &k.shared_secret).unwrap();
        assert_eq!(
            key.decrypt(Pkcs1v15Encrypt, &k.verify_token).unwrap(),
            token
        );
        let secret: [u8; 16] = secret.try_into().unwrap();
        s.enc.set_encryption(&secret).unwrap();
        s.dec.set_encryption(&secret).unwrap();
        log.push("encrypted".into());
    }
    if let Some(t) = compression {
        s.send(State::Login, &lcb::LoginCompression { threshold: t })
            .await;
        s.enc.set_compression((t as usize, 6));
        s.dec.set_compression(t as usize);
    }
    let profile = GameProfile {
        uuid: Uuid::from_u128(42),
        name: hello.name.clone(),
        properties: vec![],
    };
    s.send(
        State::Login,
        &lcb::LoginFinished {
            profile,
            session_id: Uuid::from_u128(1),
        },
    )
    .await;
    let ServerboundLogin::LoginAcknowledged(_) =
        s.recv(State::Login, ServerboundLogin::DECODERS).await
    else {
        panic!("expected ack")
    };

    // Configuration.
    let ServerboundConfiguration::ClientInformation(ci) = s
        .recv(State::Configuration, ServerboundConfiguration::DECODERS)
        .await
    else {
        panic!("expected client information")
    };
    log.push(format!("locale {}", ci.locale));
    let ServerboundConfiguration::CustomPayload(brand) = s
        .recv(State::Configuration, ServerboundConfiguration::DECODERS)
        .await
    else {
        panic!("expected brand")
    };
    log.push(format!(
        "brand {}",
        Reader::new(&brand.data).string().unwrap()
    ));
    s.send(
        State::Configuration,
        &ccb::SelectKnownPacks {
            packs: vec![KnownPack {
                namespace: "minecraft".into(),
                id: "core".into(),
                version: "26.3".into(),
            }],
        },
    )
    .await;
    let ServerboundConfiguration::SelectKnownPacks(k) = s
        .recv(State::Configuration, ServerboundConfiguration::DECODERS)
        .await
    else {
        panic!("expected known packs")
    };
    assert!(k.packs.is_empty());
    s.send(
        State::Configuration,
        &ccb::RegistryData {
            registry: Identifier::minecraft("dimension_type"),
            entries: vec![RegistryEntry {
                id: Identifier::minecraft("overworld"),
                data: Some(overworld_type()),
            }],
        },
    )
    .await;
    s.send(
        State::Configuration,
        &ccb::RegistryData {
            registry: Identifier::minecraft("worldgen/biome"),
            entries: vec![RegistryEntry {
                id: Identifier::minecraft("plains"),
                data: None,
            }],
        },
    )
    .await;
    s.send(
        State::Configuration,
        &ccb::UpdateTags { registries: vec![] },
    )
    .await;
    s.send(State::Configuration, &ccb::KeepAlive { id: 77 })
        .await;
    let ServerboundConfiguration::KeepAlive(ka) = s
        .recv(State::Configuration, ServerboundConfiguration::DECODERS)
        .await
    else {
        panic!("expected keep alive")
    };
    assert_eq!(ka.id, 77);
    s.send(State::Configuration, &ccb::FinishConfiguration {})
        .await;
    let ServerboundConfiguration::FinishConfiguration(_) = s
        .recv(State::Configuration, ServerboundConfiguration::DECODERS)
        .await
    else {
        panic!("expected finish ack")
    };

    // Play.
    s.send(
        State::Play,
        &pcb::Login {
            entity_id: 5,
            hardcore: false,
            dimension_names: vec![Identifier::minecraft("overworld")],
            max_players: 20,
            view_distance: 8,
            simulation_distance: 6,
            reduced_debug_info: false,
            enable_respawn_screen: true,
            do_limited_crafting: false,
            spawn: pcb::SpawnInfo {
                dimension_type: 0,
                dimension: Identifier::minecraft("overworld"),
                hashed_seed: 0,
                game_mode: 1,
                previous_game_mode: 0,
                is_debug: false,
                is_flat: false,
                death_location: None,
                portal_cooldown: 0,
                sea_level: 63,
            },
            online_mode: false,
            enforces_secure_chat: false,
        },
    )
    .await;
    s.send(State::Play, &pcb::KeepAlive { id: 99 }).await;
    let ServerboundPlay::KeepAlive(ka) = s.recv(State::Play, ServerboundPlay::DECODERS).await
    else {
        panic!("expected play keep alive")
    };
    assert_eq!(ka.id, 99);
    s.send(State::Play, &pcb::ChunkBatchStart {}).await;
    s.send(
        State::Play,
        &pcb::LevelChunkWithLight { chunk: big_chunk() },
    )
    .await;
    s.send(State::Play, &pcb::ChunkBatchFinished { batch_size: 1 })
        .await;
    let ServerboundPlay::ChunkBatchReceived(cb) =
        s.recv(State::Play, ServerboundPlay::DECODERS).await
    else {
        panic!("expected batch ack")
    };
    log.push(format!("chunks/tick {}", cb.chunks_per_tick));
    s.send_raw(State::Play, FAKE_UNKNOWN, &[1, 2, 3]).await;
    // A known packet with a broken body: counted as a decode failure.
    s.send_raw(State::Play, "minecraft:set_health", &[0]).await;
    let ServerboundPlay::ClientTickEnd(_) = s.recv(State::Play, ServerboundPlay::DECODERS).await
    else {
        panic!("expected tick end")
    };
    log.push("tick end".into());
    s.send(
        State::Play,
        &pcb::Disconnect {
            reason: Component::text("bye"),
        },
    )
    .await;
    log
}

async fn run(encrypt: bool, compression: Option<i32>) {
    let tables = test_tables();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(serve(listener, tables.clone(), encrypt, compression));

    let mut opts = ConnectOptions::new(
        addr.to_string(),
        Account::Offline {
            username: "tester".into(),
        },
    );
    opts.tables = Some(tables);
    let (mut conn, login) =
        tokio::time::timeout(Duration::from_secs(10), Connection::connect(opts))
            .await
            .expect("connect timeout")
            .unwrap();
    assert_eq!(login.entity_id, EntityId(5));
    assert_eq!(login.dimension.min_y, -64);
    assert_eq!(login.dimension.height, 384);
    assert!(login.dimension.has_skylight);
    assert_eq!(login.game_mode, GameMode::Creative);
    assert_eq!(login.profile.name, "tester");
    assert_eq!(
        conn.registries().index_of(
            &Identifier::minecraft("worldgen/biome"),
            &Identifier::minecraft("plains")
        ),
        Some(0)
    );

    let mut got_chunk = false;
    let mut got_unknown = false;
    let mut sent_tick = false;
    loop {
        let p = tokio::time::timeout(Duration::from_secs(5), conn.recv())
            .await
            .expect("client recv timeout");
        let Some(p) = p else { break };
        match p {
            ClientboundPlay::LevelChunkWithLight(c) => {
                assert_eq!(c.chunk.pos, ChunkPos::new(3, -4));
                assert_eq!(c.chunk.sections, big_chunk().sections);
                got_chunk = true;
            }
            ClientboundPlay::Unknown { .. } => {
                got_unknown = true;
                if !sent_tick && conn.stats().decode_failures.len() == 1 {
                    conn.send(ServerboundPlay::ClientTickEnd(
                        play::serverbound::ClientTickEnd {},
                    ))
                    .unwrap();
                    sent_tick = true;
                }
            }
            ClientboundPlay::Disconnect(d) => {
                assert_eq!(d.reason.to_plain(&|_| None), "bye");
                break;
            }
            _ => {}
        }
    }
    assert!(got_chunk && got_unknown && sent_tick);
    let stats = conn.stats();
    assert_eq!(
        stats.unknown,
        vec![(
            tables_id(&conn, FAKE_UNKNOWN),
            Some(FAKE_UNKNOWN.to_string()),
            1
        )]
    );
    assert_eq!(stats.decode_failures.len(), 1);
    assert_eq!(stats.decode_failures[0].0, "minecraft:set_health");
    let log = server.await.unwrap();
    assert!(log.contains(&"hello tester".to_string()));
    assert!(log.contains(&"brand haste".to_string()));
    assert!(log.contains(&"chunks/tick 64".to_string()));
    assert_eq!(log.contains(&"encrypted".to_string()), encrypt);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(conn.close_reason().unwrap().contains("bye"));
    conn.disconnect();
}

fn tables_id(conn: &Connection, name: &str) -> i32 {
    conn.tables()
        .packets
        .id(State::Play, Direction::Clientbound, name)
        .unwrap()
}

#[tokio::test]
async fn offline_plain() {
    run(false, None).await;
}

#[tokio::test]
async fn offline_compressed() {
    run(false, Some(256)).await;
}

#[tokio::test]
async fn encrypted_and_compressed() {
    run(true, Some(64)).await;
}

#[tokio::test]
async fn online_server_rejects_offline_account() {
    let tables = test_tables();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let t2 = tables.clone();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (r, w) = stream.into_split();
        let mut s = Server {
            dec: TcpNetworkDecoder::new(BufReader::new(r)),
            enc: TcpNetworkEncoder::new(w),
            tables: t2,
        };
        let _ = s
            .recv(State::Handshake, ServerboundHandshake::DECODERS)
            .await;
        let _ = s.recv(State::Login, ServerboundLogin::DECODERS).await;
        let key = RsaPrivateKey::new(&mut OsRng, 1024).unwrap();
        let der = key.to_public_key().to_public_key_der().unwrap();
        s.send(
            State::Login,
            &lcb::Hello {
                server_id: String::new(),
                public_key: Bytes::copy_from_slice(der.as_bytes()),
                verify_token: Bytes::from_static(&[1]),
                should_authenticate: true,
            },
        )
        .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
    });
    let mut opts = ConnectOptions::new(
        addr.to_string(),
        Account::Offline {
            username: "x".into(),
        },
    );
    opts.tables = Some(tables);
    let err = Connection::connect(opts).await.unwrap_err();
    assert!(matches!(err, ProtocolError::Protocol(_)), "{err}");
    server.await.unwrap();
}

#[tokio::test]
async fn login_disconnect_is_reported() {
    let tables = test_tables();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let t2 = tables.clone();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (r, w) = stream.into_split();
        let mut s = Server {
            dec: TcpNetworkDecoder::new(BufReader::new(r)),
            enc: TcpNetworkEncoder::new(w),
            tables: t2,
        };
        let _ = s
            .recv(State::Handshake, ServerboundHandshake::DECODERS)
            .await;
        let _ = s.recv(State::Login, ServerboundLogin::DECODERS).await;
        s.send(
            State::Login,
            &lcb::LoginDisconnect {
                reason: Component::text("Outdated client"),
            },
        )
        .await;
    });
    let mut opts = ConnectOptions::new(
        addr.to_string(),
        Account::Offline {
            username: "x".into(),
        },
    );
    opts.tables = Some(tables);
    match Connection::connect(opts).await {
        Err(ProtocolError::Disconnected(r)) => assert_eq!(r, "Outdated client"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn status_ping() {
    let tables = test_tables();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let t2 = tables.clone();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (r, w) = stream.into_split();
        let mut s = Server {
            dec: TcpNetworkDecoder::new(BufReader::new(r)),
            enc: TcpNetworkEncoder::new(w),
            tables: t2,
        };
        let ServerboundHandshake::Intention(h) = s
            .recv(State::Handshake, ServerboundHandshake::DECODERS)
            .await
        else {
            panic!()
        };
        assert_eq!(h.intent, INTENT_STATUS);
        let _ = s
            .recv(State::Status, ServerboundStatusAlias::DECODERS)
            .await;
        s.send(
            State::Status,
            &status::clientbound::StatusResponse {
                json: r#"{"version":{"name":"26.3","protocol":777}}"#.into(),
            },
        )
        .await;
        let ServerboundStatusAlias::PingRequest(p) = s
            .recv(State::Status, ServerboundStatusAlias::DECODERS)
            .await
        else {
            panic!()
        };
        s.send(
            State::Status,
            &status::clientbound::PongResponse {
                timestamp: p.timestamp,
            },
        )
        .await;
    });
    let st = ping(&addr.to_string(), Some(tables)).await.unwrap();
    assert_eq!(st.json["version"]["protocol"], 777);
}

#[test]
fn host_port_parsing() {
    assert_eq!(
        split_host_port("127.0.0.1:25565"),
        ("127.0.0.1".into(), Some(25565))
    );
    assert_eq!(split_host_port("example.org"), ("example.org".into(), None));
    assert_eq!(split_host_port("[::1]:25566"), ("::1".into(), Some(25566)));
    assert_eq!(split_host_port("::1"), ("::1".into(), None));
}
