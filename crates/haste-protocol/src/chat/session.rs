//! The local chat session (vanilla `LocalChatSession` over a `ProfileKeyPair`).
//!
//! The key pair comes from the player certificates (`POST /player/certificates`): an RSA
//! private key (PKCS#8 DER), its public key (X.509 `SubjectPublicKeyInfo` DER) and Mojang's
//! signature over the public key (`publicKeySignatureV2`, verified by servers with
//! `ProfilePublicKey.Data.validateSignature`). On joining an online-mode server the client
//! picks a random session id, announces it with `chat_session_update` and signs every message
//! in the chain that session starts.

use bytes::Bytes;
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey};
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, RsaPublicKey};
use uuid::Uuid;

use super::ChatSigner;
use crate::packets::play::serverbound::ChatSessionUpdate;

/// The player's profile key pair (vanilla `ProfileKeyPair` with its `ProfilePublicKey.Data`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileKeyPair {
    /// RSA private key, PKCS#8 DER.
    pub private_key_der: Vec<u8>,
    /// RSA public key, X.509 DER (what `chat_session_update` carries).
    pub public_key_der: Vec<u8>,
    /// Mojang's signature over profile id, expiry and public key (`publicKeySignatureV2`).
    pub key_signature: Vec<u8>,
    /// Key expiry, milliseconds since the epoch; the signed payload uses this exact value.
    pub expires_at_millis: i64,
    /// When to fetch a new key pair, milliseconds since the epoch.
    pub refreshed_after_millis: i64,
}

/// Why a key pair cannot start a chat session.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// The private key is not PKCS#8 RSA.
    #[error("profile private key: {0}")]
    PrivateKey(String),
    /// The public key is not X.509 RSA.
    #[error("profile public key: {0}")]
    PublicKey(String),
    /// Chat signatures are 256 bytes, so the key must be RSA-2048 (as Mojang's are).
    #[error("profile key is {0} bits; chat signing needs 2048")]
    KeySize(usize),
    /// The two halves do not belong together.
    #[error("profile public key does not match the private key")]
    Mismatch,
}

/// The chat session a client announces to an online-mode server (vanilla `LocalChatSession`).
#[derive(Clone, Debug)]
pub struct LocalChatSession {
    session_id: Uuid,
    keys: ProfileKeyPair,
    private_key: RsaPrivateKey,
}

impl LocalChatSession {
    /// Vanilla `LocalChatSession.create`: a random session id over `keys`.
    pub fn create(keys: ProfileKeyPair) -> Result<Self, SessionError> {
        Self::with_session_id(Uuid::new_v4(), keys)
    }

    /// A session with a given id (tests).
    pub fn with_session_id(session_id: Uuid, keys: ProfileKeyPair) -> Result<Self, SessionError> {
        let private_key = RsaPrivateKey::from_pkcs8_der(&keys.private_key_der)
            .map_err(|e| SessionError::PrivateKey(e.to_string()))?;
        let public_key = RsaPublicKey::from_public_key_der(&keys.public_key_der)
            .map_err(|e| SessionError::PublicKey(e.to_string()))?;
        if private_key.size() != 256 {
            return Err(SessionError::KeySize(private_key.size() * 8));
        }
        if RsaPublicKey::from(&private_key) != public_key {
            return Err(SessionError::Mismatch);
        }
        Ok(Self {
            session_id,
            keys,
            private_key,
        })
    }

    /// The session id.
    pub fn session_id(&self) -> Uuid {
        self.session_id
    }

    /// The key pair.
    pub fn keys(&self) -> &ProfileKeyPair {
        &self.keys
    }

    /// Vanilla `createMessageEncoder`: the message chain for `profile_id`, from index 0.
    pub fn message_encoder(&self, profile_id: Uuid) -> ChatSigner {
        ChatSigner::new(self.session_id, profile_id, self.private_key.clone())
    }

    /// The `chat_session_update` announcing this session (`asRemote().asData()`).
    pub fn update_packet(&self) -> ChatSessionUpdate {
        ChatSessionUpdate::new(
            self.session_id,
            self.keys.expires_at_millis,
            Bytes::from(self.keys.public_key_der.clone()),
            Bytes::from(self.keys.key_signature.clone()),
        )
    }
}
