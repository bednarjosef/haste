//! The login state: encryption, compression and authentication (vanilla's
//! login packet listener).

use crate::auth;
use crate::packets::login::{self, ClientboundLogin};
use crate::packets::{Direction, Dispatch, State};
use crate::wire::GameProfile;

use super::io::{decode_with, Io};
use super::{Account, ConnectOptions, ProtocolError};

/// Runs the login state: answers encryption, compression, cookie and custom
/// query packets, and returns the profile from `login_finished`.
pub(crate) async fn login_state(
    io: &mut Io,
    opts: &ConnectOptions,
) -> Result<GameProfile, ProtocolError> {
    let tables = io.tables.clone();
    let d = Dispatch::new(
        &tables.packets,
        State::Login,
        Direction::Clientbound,
        ClientboundLogin::DECODERS,
    );
    loop {
        let raw = io.recv().await?;
        let Some(p) = decode_with(&d, &raw, &tables, State::Login, &mut io.warned)? else {
            continue;
        };
        match p {
            ClientboundLogin::Hello(req) => {
                let secret = auth::new_shared_secret();
                if req.should_authenticate {
                    match &opts.account {
                        Account::Microsoft(t) => {
                            let hash = auth::server_hash(&req.server_id, &secret, &req.public_key);
                            auth::join_session(t, &hash).await?;
                        }
                        Account::Offline { .. } => {
                            return Err(ProtocolError::Protocol(
                                "server is in online mode; an offline account cannot join".into(),
                            ))
                        }
                    }
                }
                let key = login::serverbound::Key {
                    shared_secret: auth::rsa_encrypt(&req.public_key, &secret)?.into(),
                    verify_token: auth::rsa_encrypt(&req.public_key, &req.verify_token)?.into(),
                };
                io.send(State::Login, &key).await?;
                io.enc.set_encryption(&secret)?;
                io.dec.set_encryption(&secret)?;
            }
            ClientboundLogin::LoginCompression(c) => {
                if c.threshold >= 0 {
                    io.enc.set_compression((c.threshold as usize, 6));
                    io.dec.set_compression(c.threshold as usize);
                }
            }
            ClientboundLogin::LoginFinished(f) => {
                io.send(State::Login, &login::serverbound::LoginAcknowledged {})
                    .await?;
                return Ok(f.profile);
            }
            ClientboundLogin::LoginDisconnect(d) => {
                return Err(ProtocolError::Disconnected(d.reason.to_plain(&|_| None)))
            }
            ClientboundLogin::CustomQuery(q) => {
                io.send(
                    State::Login,
                    &login::serverbound::CustomQueryAnswer {
                        message_id: q.message_id,
                        data: None,
                    },
                )
                .await?;
            }
            ClientboundLogin::CookieRequest(c) => {
                let payload = opts.cookies.get(&c.key);
                io.send(
                    State::Login,
                    &login::serverbound::CookieResponse {
                        key: c.key,
                        payload,
                    },
                )
                .await?;
            }
            ClientboundLogin::Unknown { .. } => {}
        }
    }
}
