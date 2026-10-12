//! `haste-packet-proxy`: a TCP proxy for OFFLINE servers that records decompressed clientbound
//! play packets to `cache/oracle/26.3/packets/<name>.bin` (FORMATS.md) while forwarding every
//! byte unchanged.
//!
//! Usage: haste-packet-proxy --listen 127.0.0.1:25566 --server 127.0.0.1:25599 --name world-load
//!
//! Offline mode means no encryption, so the proxy can stay a pure pipe: it parses clientbound
//! frames only to track the connection state (handshake -> login -> configuration -> play) and
//! the compression threshold; in the play state it also appends each decompressed frame to the
//! recording. Serverbound traffic is piped without parsing. One client per run: the process
//! exits after that client disconnects.

use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use bytes::Bytes;

use haste_testkit::proxy::{default_header, write_recording, FrameIo};

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Handshake,
    Login,
    Config,
    Play,
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut listen = "127.0.0.1:25566".to_string();
    let mut server = "127.0.0.1:25565".to_string();
    let mut name = String::new();
    let mut out_dir = haste_core::paths::oracle_dir().join("packets");
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--listen" => listen = args.next().context("--listen needs a value")?,
            "--server" => server = args.next().context("--server needs a value")?,
            "--name" => name = args.next().context("--name needs a value")?,
            "--out" => out_dir = args.next().context("--out needs a value")?.into(),
            "--help" | "-h" => {
                println!("usage: haste-packet-proxy --listen HOST:PORT --server HOST:PORT --name NAME [--out DIR]");
                return Ok(());
            }
            other => bail!("unknown argument {other}"),
        }
    }
    if name.is_empty() {
        bail!("--name is required (the recording file stem)");
    }

    let listener = TcpListener::bind(&listen).with_context(|| format!("binding {listen}"))?;
    eprintln!("listening on {listen}, forwarding to {server}, recording as {name}");

    let (stream, peer) = listener.accept().context("accepting client")?;
    eprintln!("client {peer} connected");
    let upstream =
        TcpStream::connect(&server).with_context(|| format!("connecting to {server}"))?;
    upstream.set_nodelay(true).ok();
    stream.set_nodelay(true).ok();

    let recorded = ProxySession::run(stream, upstream)?;
    let path = out_dir.join(format!("{name}.bin"));
    write_recording(&path, &default_header(), &recorded)?;
    eprintln!("wrote {} ({} play packets)", path.display(), recorded.len());
    Ok(())
}

struct ProxySession {
    compression_threshold: Option<i32>,
    state: State,
    recorded: Vec<(i32, Bytes)>,
}

impl ProxySession {
    /// Runs one client<->server session; returns the recorded clientbound play packets.
    fn run(client: TcpStream, server: TcpStream) -> Result<Vec<(i32, Bytes)>> {
        let client_to_server = client.try_clone().context("cloning client stream")?;
        let to_client = client;
        let server_for_shutdown = server.try_clone().context("cloning server stream")?;

        // Serverbound (client -> server) is piped raw: we never need to parse it.
        let (done_tx, done_rx) = mpsc::channel();
        let pipe = std::thread::spawn(move || {
            let mut from = client_to_server;
            let mut to = server;
            let _ = std::io::copy(&mut from, &mut to);
            let _ = done_tx.send(());
        });

        let mut session = ProxySession {
            compression_threshold: None,
            state: State::Handshake,
            recorded: Vec::new(),
        };
        // Clientbound: server -> client, parsed + recorded in the play state.
        let mut io = FrameIo::new(server_for_shutdown);
        let result = loop {
            match io.read_frame(session.compressed()) {
                Ok(Some(frame)) => {
                    session.on_packet(frame.id, &frame.body);
                    io.write_raw(&frame.raw).context("forwarding to client")?;
                }
                Ok(None) => break Ok(session.recorded), // server closed
                Err(e) => {
                    // Keep the pipe intact for whatever was already forwarded; give up recording.
                    eprintln!("clientbound parse error: {e:#}");
                    break Err(e);
                }
            }
        };
        let _ = to_client.shutdown(std::net::Shutdown::Both);
        let _ = done_rx.recv_timeout(Duration::from_millis(500));
        let _ = pipe.join();
        result
    }

    fn compressed(&self) -> bool {
        self.compression_threshold.is_some()
    }

    /// Clientbound state machine. Packet ids are per-state; in offline mode the flow is:
    /// handshake (clientbound only after status/login intent) -> login (compression=3,
    /// success=2) -> configuration (finish=2 in 26.3) -> play (everything recorded).
    ///
    /// These ids are parsed from the stream as observed, not hardcoded from a wiki; if 26.3's
    /// configuration protocol differs the session still forwards everything (recording just
    /// starts late or never). The datagen packets.json has the authoritative ids for WS1.
    fn on_packet(&mut self, id: i32, body: &[u8]) {
        match self.state {
            State::Handshake => {
                // First clientbound frame after our client's handshake+login start.
                self.state = State::Login;
                self.on_packet(id, body); // re-dispatch in the login state
            }
            State::Login => match id {
                3 => {
                    // set_compression: body is a varint threshold
                    if let Some((threshold, _)) =
                        haste_testkit::proxy::try_read_varint(&Bytes::copy_from_slice(body))
                    {
                        self.compression_threshold = Some(threshold);
                        eprintln!("compression threshold {threshold}");
                    }
                }
                2 => {
                    self.state = State::Config;
                    eprintln!("login success -> configuration");
                }
                _ => {}
            },
            State::Config => {
                if id == 2 {
                    // finish_configuration (client then sends its own finish; play begins)
                    self.state = State::Play;
                    eprintln!("configuration finished -> play");
                }
            }
            State::Play => {
                self.recorded.push((id, Bytes::copy_from_slice(body)));
            }
        }
    }
}
