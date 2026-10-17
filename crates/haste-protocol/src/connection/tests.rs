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
fn test_tables_owned() -> ProtocolTables {
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
    ProtocolTables::default().with_packets(PacketIds::from_names(&borrowed))
}

/// [`test_tables_owned`] behind an `Arc` (the connection API).
pub(super) fn test_tables() -> Arc<ProtocolTables> {
    Arc::new(test_tables_owned())
}

pub(super) struct Server {
    pub(super) dec: TcpNetworkDecoder<BufReader<OwnedReadHalf>>,
    pub(super) enc: TcpNetworkEncoder<OwnedWriteHalf>,
    pub(super) tables: Arc<ProtocolTables>,
}

impl Server {
    pub(super) async fn send<P: Packet>(&mut self, state: State, p: &P) {
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
    pub(super) async fn recv<T>(
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

    // Configuration: brand, then client information (the wiki's order).
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
    let ServerboundConfiguration::ClientInformation(ci) = s
        .recv(State::Configuration, ServerboundConfiguration::DECODERS)
        .await
    else {
        panic!("expected client information")
    };
    log.push(format!("locale {}", ci.locale));
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
    assert!(log.contains(&"brand vanilla".to_string()));
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

// ---------------------------------------------------------------- cosmetic decode loss

/// Tables where particle id 0 is a made-up, mod-registered type: the registry
/// knows it, but `wire::particle::skip_data` has no layout for it.
fn tables_with_unknown_particle() -> ProtocolTables {
    test_tables_owned().with_particles_for_tests(&[(0, "haste_cosy:glitter")])
}

/// A raw clientbound play packet: id from the tables, the given body.
fn raw_play(tables: &ProtocolTables, name: &str, body: &[u8]) -> crate::transport::RawPacket {
    crate::transport::RawPacket {
        id: tables
            .packets
            .id(State::Play, Direction::Clientbound, name)
            .unwrap_or_else(|| panic!("no clientbound play id for {name}")),
        payload: Bytes::copy_from_slice(body),
    }
}

/// A `level_particles` body whose particle is an unknown, mod-registered type:
/// the layout fails, so the rest of the packet cannot be located either.
fn broken_level_particles() -> Vec<u8> {
    let mut w = Writer::new();
    w.var_int(0); // the particle: `level_particles`'s first field
    w.bytes(&[0xff; 8]); // what follows, unreachable once the layout fails
    w.into_inner()
}

#[test]
fn unknown_particle_level_particles_is_dropped_and_play_continues() {
    let tables = tables_with_unknown_particle();
    let d = Dispatch::new(
        &tables.packets,
        State::Play,
        Direction::Clientbound,
        ClientboundPlay::DECODERS,
    );
    let broken = raw_play(
        &tables,
        "minecraft:level_particles",
        &broken_level_particles(),
    );
    let mut health = Writer::new();
    health.f32(20.0);
    health.var_int(20);
    health.f32(5.0);
    let after = raw_play(&tables, "minecraft:set_health", &health.into_inner());

    let mut warned = WarnedOnce::default();
    // Dropped, not an error.
    assert!(decode_with(&d, &broken, &tables, State::Play, &mut warned)
        .unwrap()
        .is_none());
    // The first failure warned and marked the name; the second is silent but
    // still dropped.
    assert!(!warned.mark("minecraft:level_particles"));
    assert!(decode_with(&d, &broken, &tables, State::Play, &mut warned)
        .unwrap()
        .is_none());
    // The connection carries on: the next packet decodes.
    assert!(matches!(
        decode_with(&d, &after, &tables, State::Play, &mut warned).unwrap(),
        Some(ClientboundPlay::SetHealth(h)) if h.health == 20.0
    ));
}

#[test]
fn the_same_particle_in_explode_is_not_cosmetic() {
    let tables = tables_with_unknown_particle();
    let d = Dispatch::new(
        &tables.packets,
        State::Play,
        Direction::Clientbound,
        ClientboundPlay::DECODERS,
    );
    // `minecraft:explode` up to its particle field.
    let mut w = Writer::new();
    w.f64(0.0);
    w.f64(0.0);
    w.f64(0.0); // center
    w.f32(4.0); // radius
    w.i32(0); // block count
    w.bool(false); // no player knockback
    w.var_int(0); // the unknown particle
    let broken = raw_play(&tables, "minecraft:explode", &w.into_inner());

    // It moves the player, so it stays a protocol error.
    let mut warned = WarnedOnce::default();
    match decode_with(&d, &broken, &tables, State::Play, &mut warned) {
        Err(ProtocolError::Decode("minecraft:explode", _)) => {}
        other => panic!("explode should stay an error, got {other:?}"),
    }
    assert!(!warned.mark("minecraft:explode"));
}

#[test]
fn cosmetic_warning_fires_once_per_name() {
    let mut warned = WarnedOnce::default();
    for name in COSMETIC_LOSS_PACKETS {
        assert!(warned.mark(name), "{name} should warn the first time");
        assert!(!warned.mark(name), "{name} should warn only once");
    }
    assert!(!warned.mark("minecraft:explode"));
}

// ---------------------------------------------------------------- reconfiguration and transfer

impl Server {
    /// Accepts a connection and runs handshake + offline login.
    pub(super) async fn accept(
        listener: &TcpListener,
        tables: Arc<ProtocolTables>,
        intent: i32,
    ) -> Server {
        let (stream, _) = listener.accept().await.unwrap();
        let (r, w) = stream.into_split();
        let mut s = Server {
            dec: TcpNetworkDecoder::new(BufReader::new(r)),
            enc: TcpNetworkEncoder::new(w),
            tables,
        };
        let ServerboundHandshake::Intention(h) = s
            .recv(State::Handshake, ServerboundHandshake::DECODERS)
            .await
        else {
            panic!()
        };
        assert_eq!(h.intent, intent);
        let ServerboundLogin::Hello(hello) = s.recv(State::Login, ServerboundLogin::DECODERS).await
        else {
            panic!("expected hello")
        };
        s.send(
            State::Login,
            &lcb::LoginFinished {
                profile: GameProfile {
                    uuid: Uuid::from_u128(42),
                    name: hello.name,
                    properties: vec![],
                },
                session_id: Uuid::from_u128(1),
            },
        )
        .await;
        let ServerboundLogin::LoginAcknowledged(_) =
            s.recv(State::Login, ServerboundLogin::DECODERS).await
        else {
            panic!("expected ack")
        };
        s
    }

    pub(super) async fn recv_cfg(&mut self) -> ServerboundConfiguration {
        self.recv(State::Configuration, ServerboundConfiguration::DECODERS)
            .await
    }

    /// Expects brand + client information.
    pub(super) async fn expect_hello(&mut self) {
        assert!(matches!(
            self.recv_cfg().await,
            ServerboundConfiguration::CustomPayload(_)
        ));
        assert!(matches!(
            self.recv_cfg().await,
            ServerboundConfiguration::ClientInformation(_)
        ));
    }

    /// Sends registries with one biome named `biome`, finishes configuration
    /// and sends the play `login` with `entity_id`.
    pub(super) async fn finish_with(&mut self, biome: &str, entity_id: i32) {
        self.send(
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
        self.send(
            State::Configuration,
            &ccb::RegistryData {
                registry: Identifier::minecraft("worldgen/biome"),
                entries: vec![RegistryEntry {
                    id: Identifier::minecraft(biome),
                    data: None,
                }],
            },
        )
        .await;
        self.send(State::Configuration, &ccb::FinishConfiguration {})
            .await;
        assert!(matches!(
            self.recv_cfg().await,
            ServerboundConfiguration::FinishConfiguration(_)
        ));
        self.send(State::Play, &play_login(entity_id)).await;
    }
}

fn play_login(entity_id: i32) -> pcb::Login {
    pcb::Login {
        entity_id,
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
    }
}

/// Server A: cookies and a resource pack in configuration, play, then a
/// reconfiguration with new registries, then a transfer to server B, which
/// reads the cookie stored on A.
async fn serve_a_then_b(
    a: TcpListener,
    b: TcpListener,
    tables: Arc<ProtocolTables>,
) -> Vec<String> {
    let mut log = Vec::new();
    let mut s = Server::accept(&a, tables.clone(), INTENT_LOGIN).await;
    s.expect_hello().await;
    let key = Identifier::new("test", "cookie");
    s.send(
        State::Configuration,
        &ccb::StoreCookie {
            key: key.clone(),
            payload: Bytes::from_static(&[1, 2, 3]),
        },
    )
    .await;
    s.send(
        State::Configuration,
        &ccb::CookieRequest { key: key.clone() },
    )
    .await;
    let ServerboundConfiguration::CookieResponse(c) = s.recv_cfg().await else {
        panic!("expected cookie response")
    };
    assert_eq!(c.payload.as_deref(), Some(&[1u8, 2, 3][..]));
    s.send(
        State::Configuration,
        &ccb::ResourcePackPush {
            id: Uuid::from_u128(7),
            url: "http://127.0.0.1/pack.zip".into(),
            hash: String::new(),
            forced: true,
            prompt: None,
        },
    )
    .await;
    for _ in 0..3 {
        let ServerboundConfiguration::ResourcePack(r) = s.recv_cfg().await else {
            panic!("expected resource pack status")
        };
        log.push(format!("pack {}", r.result));
    }
    s.finish_with("plains", 5).await;

    // Reconfiguration.
    s.send(State::Play, &pcb::StartConfiguration {}).await;
    let ServerboundPlay::ConfigurationAcknowledged(_) =
        s.recv(State::Play, ServerboundPlay::DECODERS).await
    else {
        panic!("expected configuration acknowledged")
    };
    s.expect_hello().await;
    s.send(State::Configuration, &ccb::KeepAlive { id: 5 })
        .await;
    let ServerboundConfiguration::KeepAlive(k) = s.recv_cfg().await else {
        panic!("expected keep alive")
    };
    assert_eq!(k.id, 5);
    s.finish_with("desert", 6).await;
    log.push("reconfigured".into());

    // Transfer.
    let target = b.local_addr().unwrap();
    s.send(
        State::Play,
        &ccb::StoreCookie {
            key: Identifier::new("test", "ticket"),
            payload: Bytes::from_static(&[9]),
        },
    )
    .await;
    s.send(
        State::Play,
        &ccb::Transfer {
            host: target.ip().to_string(),
            port: target.port() as i32,
        },
    )
    .await;
    let mut t = Server::accept(&b, tables, INTENT_TRANSFER).await;
    t.expect_hello().await;
    t.send(
        State::Configuration,
        &ccb::CookieRequest {
            key: Identifier::new("test", "ticket"),
        },
    )
    .await;
    let ServerboundConfiguration::CookieResponse(c) = t.recv_cfg().await else {
        panic!("expected cookie response")
    };
    assert_eq!(c.payload.as_deref(), Some(&[9u8][..]));
    t.finish_with("forest", 7).await;
    log.push("transferred".into());
    t.send(State::Play, &pcb::KeepAlive { id: 1234 }).await;
    let ServerboundPlay::KeepAlive(k) = t.recv(State::Play, ServerboundPlay::DECODERS).await else {
        panic!("expected keep alive on the new server")
    };
    assert_eq!(k.id, 1234);
    t.send(
        State::Play,
        &pcb::Disconnect {
            reason: Component::text("done"),
        },
    )
    .await;
    log
}

#[tokio::test]
async fn reconfiguration_cookies_resource_pack_and_transfer() {
    let tables = test_tables();
    let a = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let b = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = a.local_addr().unwrap();
    let server = tokio::spawn(serve_a_then_b(a, b, tables.clone()));
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
    let biome = |c: &Connection| {
        c.registries()
            .entries(&Identifier::minecraft("worldgen/biome"))
            .unwrap()[0]
            .id
            .path()
            .to_string()
    };
    let mut seen = Vec::new();
    loop {
        let p = tokio::time::timeout(Duration::from_secs(5), conn.recv())
            .await
            .expect("client recv timeout");
        let Some(p) = p else { break };
        match p {
            ClientboundPlay::Login(l) => {
                seen.push(format!("login {} {}", l.entity_id, biome(&conn)))
            }
            ClientboundPlay::StartConfiguration(_) => seen.push("start configuration".into()),
            ClientboundPlay::Transfer(_) => seen.push("transfer".into()),
            ClientboundPlay::Disconnect(d) => {
                seen.push(d.reason.to_plain(&|_| None));
                break;
            }
            _ => {}
        }
    }
    assert_eq!(
        seen,
        [
            "login 5 plains",
            "start configuration",
            "login 6 desert",
            "transfer",
            "login 7 forest",
            "done"
        ]
    );
    let log = server.await.unwrap();
    assert_eq!(&log[..3], ["pack 3", "pack 4", "pack 0"]);
    assert_eq!(&log[3..], ["reconfigured", "transferred"]);
    conn.disconnect();
}

/// Serves `body` once over plain HTTP (a tiny static file server for the pack
/// download tests; vanilla downloads packs over http/https only).
pub(super) async fn serve_one_http(body: Bytes) -> reqwest::Url {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = reqwest::Url::parse(&format!("http://{addr}/pack.zip")).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        use tokio::io::AsyncReadExt as _;
        use tokio::io::AsyncWriteExt as _;
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf).await;
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).await.unwrap();
        stream.write_all(&body).await.unwrap();
    });
    url
}

/// The `resource_pack_push` flow with a downloads directory (vanilla's
/// `DownloadedPackSource`): a good push downloads the pack (hash-checked, in
/// the `downloads/<id>/<sha1>` cache) with vanilla's statuses ACCEPTED,
/// DOWNLOADED, SUCCESSFULLY_LOADED; a wrong hash answers ACCEPTED then
/// FAILED_DOWNLOAD; a non-http URL answers INVALID_URL.
#[tokio::test]
async fn server_pack_push_downloads_and_reports_statuses() {
    let tables = test_tables();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // The pack: a small body with its SHA-1 as the push's hash.
    let pack_bytes = Bytes::from_static(b"a server pushed this pack, hash-checked");
    use sha1::Digest as _;
    let mut hasher = sha1::Sha1::new();
    hasher.update(&pack_bytes);
    let digest: [u8; 20] = hasher.finalize().into();
    let hex = digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let pack_url = serve_one_http(pack_bytes.clone()).await;
    let hex_for_task = hex.clone();

    let downloads = tempfile::tempdir().unwrap();
    let downloads_dir = downloads.path().to_path_buf();

    let task_tables = tables.clone();
    let server = tokio::spawn(async move {
        let mut s = Server::accept(&listener, task_tables, INTENT_LOGIN).await;
        s.expect_hello().await;
        let mut statuses = Vec::new();
        // 1. A good push with the correct hash.
        s.send(
            State::Configuration,
            &ccb::ResourcePackPush {
                id: Uuid::from_u128(11),
                url: pack_url.to_string(),
                hash: hex_for_task,
                forced: false,
                prompt: None,
            },
        )
        .await;
        for _ in 0..3 {
            let ServerboundConfiguration::ResourcePack(r) = s.recv_cfg().await else {
                panic!("expected resource pack status")
            };
            statuses.push(format!("good {}", r.result));
        }
        // 2. The same URL with a wrong hash: ACCEPTED then FAILED_DOWNLOAD.
        s.send(
            State::Configuration,
            &ccb::ResourcePackPush {
                id: Uuid::from_u128(12),
                url: pack_url.to_string(),
                hash: "0".repeat(40),
                forced: true,
                prompt: None,
            },
        )
        .await;
        for _ in 0..2 {
            let ServerboundConfiguration::ResourcePack(r) = s.recv_cfg().await else {
                panic!("expected resource pack status")
            };
            statuses.push(format!("badhash {}", r.result));
        }
        // 3. A non-http URL: INVALID_URL only.
        s.send(
            State::Configuration,
            &ccb::ResourcePackPush {
                id: Uuid::from_u128(13),
                url: "file:///etc/passwd".into(),
                hash: String::new(),
                forced: false,
                prompt: None,
            },
        )
        .await;
        let ServerboundConfiguration::ResourcePack(r) = s.recv_cfg().await else {
            panic!("expected resource pack status")
        };
        statuses.push(format!("badurl {}", r.result));
        // 4. Pop the good pack: the live set empties.
        s.send(
            State::Configuration,
            &ccb::ResourcePackPop {
                id: Some(Uuid::from_u128(11)),
            },
        )
        .await;
        s.finish_with("plains", 5).await;
        statuses.push("finished".into());
        statuses
    });

    let mut opts = ConnectOptions::new(
        addr.to_string(),
        Account::Offline {
            username: "tester".into(),
        },
    );
    opts.tables = Some(tables.clone());
    opts.downloads_dir = Some(downloads_dir.clone());
    let (conn, login) = tokio::time::timeout(Duration::from_secs(10), Connection::connect(opts))
        .await
        .expect("connect timeout")
        .unwrap();
    assert_eq!(login.entity_id, EntityId(5));

    // The good push downloaded (the file is in the cache, hash-named, with the
    // exact bytes) but the pop removed it from the set before the play state,
    // so the live set is empty (vanilla `handleResourcePackPop`).
    let packs = conn.server_packs();
    let file = downloads_dir
        .join(Uuid::from_u128(11).to_string())
        .join(&hex);
    assert_eq!(std::fs::read(&file).unwrap(), &pack_bytes[..]);
    assert!(packs.is_empty(), "the pop removed the pack: {packs:?}");
    assert_eq!(conn.server_packs_version(), 0, "the set never existed live");

    let statuses = server.await.unwrap();
    assert_eq!(
        statuses,
        vec![
            "good 3", // ACCEPTED
            "good 4", // DOWNLOADED
            "good 0", // SUCCESSFULLY_LOADED
            "badhash 3",
            "badhash 2", // FAILED_DOWNLOAD
            "badurl 5",  // INVALID_URL
            "finished",
        ]
    );
    conn.disconnect();
}

/// `set_packet_wake`'s hook fires when the reader queues a play packet, after
/// the send (a woken drainer sees the packet), and the queued packet carries
/// the arrival timestamp `try_recv_arrived` reports (the client's
/// arrival-to-apply latency log depends on both).
#[tokio::test]
async fn packet_wake_fires_on_arrival() {
    let tables = test_tables();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(serve(listener, tables.clone(), false, None));

    let mut opts = ConnectOptions::new(
        addr.to_string(),
        Account::Offline {
            username: "tester".into(),
        },
    );
    opts.tables = Some(tables);
    let (mut conn, _login) =
        tokio::time::timeout(Duration::from_secs(10), Connection::connect(opts))
            .await
            .expect("connect timeout")
            .unwrap();

    let wakes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let w = wakes.clone();
    conn.set_packet_wake(Arc::new(move || {
        w.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }));

    // The login packet is queued before the hook is set, so it cannot have
    // woken anything; it still arrives with a timestamp.
    let first = tokio::time::timeout(Duration::from_secs(5), conn.recv_arrived())
        .await
        .expect("login timeout")
        .expect("login");
    assert!(matches!(first.packet, ClientboundPlay::Login(_)));
    assert!(!first.at.elapsed().is_zero());

    // Drain to the keep alive the server sends in the play state.
    loop {
        let a = tokio::time::timeout(Duration::from_secs(5), conn.recv_arrived())
            .await
            .expect("recv timeout")
            .expect("connection open");
        if matches!(a.packet, ClientboundPlay::KeepAlive(_)) {
            break;
        }
    }
    // The keep alive arrived through the reader, which queued it with the hook
    // set: the hook fired (at least once — the reader may also have queued the
    // level chunk in the same batch).
    assert!(
        wakes.load(std::sync::atomic::Ordering::Relaxed) > 0,
        "the wake hook fired when a packet was queued"
    );
    server.abort();
    conn.disconnect();
}
