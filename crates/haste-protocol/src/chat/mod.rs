//! Signed chat (online mode), following vanilla's `net.minecraft.network.chat`:
//! - [`ChatSigner`]: `SignedMessageChain.Encoder`, the message chain signing each message
//!   (payload of `PlayerChatMessage.updateSignature`).
//! - [`LastSeenMessagesTracker`] (`last_seen.rs`): the client's window of seen messages that
//!   every chat packet acknowledges.
//! - [`LocalChatSession`] and [`ProfileKeyPair`] (`session.rs`): the session announced with
//!   `chat_session_update`, from the player certificates.

mod last_seen;
mod session;

pub use last_seen::{checksum as last_seen_checksum, LastSeenMessagesTracker, LastSeenUpdate};
pub use session::{LocalChatSession, ProfileKeyPair, SessionError};

use bytes::Bytes;
use rsa::pkcs8::DecodePrivateKey;
use rsa::{Pkcs1v15Sign, RsaPrivateKey};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::packets::play::clientbound::ChatSession;
use crate::packets::play::serverbound::{Chat, ChatSessionUpdate};

/// A chat message's 256-byte signature (vanilla `MessageSignature`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageSignature {
    /// The signature bytes.
    pub bytes: [u8; 256],
}

impl MessageSignature {
    /// The signature from a packet's bytes; `None` unless exactly 256 bytes.
    pub fn from_slice(bytes: &[u8]) -> Option<Self> {
        Some(Self {
            bytes: bytes.try_into().ok()?,
        })
    }
}

/// Signs `chat` messages and `chat_command_signed` arguments with the
/// player's profile key (online mode).
pub struct ChatSigner {
    /// The chat session the server assigned (`player_info_update`).
    session_id: Uuid,
    /// The player's own UUID: the sender in every signed payload.
    sender: Uuid,
    /// Message index: counts up per signed message, starting at 0.
    index: i32,
    private_key: RsaPrivateKey,
}

impl ChatSigner {
    /// Creates a signer for one chat session. `private_key` is the player's
    /// profile key (parsed from its DER form by the caller).
    pub fn new(session_id: Uuid, sender: Uuid, private_key: RsaPrivateKey) -> Self {
        Self {
            session_id,
            sender,
            index: 0,
            private_key,
        }
    }

    /// Creates a signer from the private key's PKCS#8 DER encoding.
    pub fn from_private_key_der(
        session_id: Uuid,
        sender: Uuid,
        der: &[u8],
    ) -> Result<Self, rsa::pkcs8::Error> {
        Ok(Self::new(
            session_id,
            sender,
            RsaPrivateKey::from_pkcs8_der(der)?,
        ))
    }

    /// The chat session id.
    pub fn session_id(&self) -> Uuid {
        self.session_id
    }

    /// The next message index (increments per signed message).
    pub fn index(&self) -> i32 {
        self.index
    }

    /// Signs a chat message and advances the message index.
    ///
    /// `timestamp` is milliseconds since the epoch (the `chat` packet field);
    /// the signed payload carries it in seconds.
    pub fn sign(
        &mut self,
        message: &str,
        timestamp: i64,
        salt: i64,
        last_seen: &[MessageSignature],
    ) -> [u8; 256] {
        let payload = self.payload(message, timestamp, salt, last_seen);
        let signature = self.sign_payload(&payload);
        self.index += 1;
        signature
    }

    /// Signs one argument of a `chat_command_signed` command. Vanilla signs each signable
    /// argument exactly like a chat message whose content is the argument's text, with the
    /// command's timestamp, salt and last-seen list, and each one advances the message index
    /// (`ClientPacketListener.sendCommand` -> `ArgumentSignatures.signCommand`).
    pub fn sign_argument(
        &mut self,
        argument: &str,
        timestamp: i64,
        salt: i64,
        last_seen: &[MessageSignature],
    ) -> [u8; 256] {
        self.sign(argument, timestamp, salt, last_seen)
    }

    /// Signs a chat message and fills the `chat` packet's signature field.
    pub fn fill_chat(&mut self, chat: &mut Chat, last_seen: &[MessageSignature]) {
        let signature = self.sign(&chat.message, chat.timestamp, chat.salt, last_seen);
        chat.signature = Some(Bytes::from(signature.to_vec()));
    }

    /// Vanilla `ClientPacketListener.sendChat`: takes the last-seen update from `tracker`
    /// (`generateAndApplyUpdate`), signs the message over it and fills the `chat` packet's
    /// signature and acknowledgement fields. `chat.timestamp` and `chat.salt` are kept.
    pub fn pack_chat(&mut self, chat: &mut Chat, tracker: &mut LastSeenMessagesTracker) {
        let update = tracker.generate_and_apply_update();
        self.fill_chat(chat, &update.entries);
        chat.message_count = update.offset;
        chat.acknowledged = update.acknowledged;
        chat.checksum = update.checksum;
    }

    /// The exact bytes that [`ChatSigner::sign`] signs, as vanilla's
    /// `PlayerChatMessage.updateSignature` feeds them to the signature (all integers big-endian,
    /// fixed width; no VarInts):
    ///
    /// 1. int 1 (the signature format version)
    /// 2. the link (`SignedMessageLink.updateSignature`): sender UUID (16 bytes), session id
    ///    (16 bytes), int message index
    /// 3. the body (`SignedMessageBody.updateSignature`): long salt, long timestamp in seconds,
    ///    int byte length of the UTF-8 content, the content
    /// 4. last seen (`LastSeenMessages.updateSignature`): int count, then each entry's 256
    ///    signature bytes
    pub fn payload(
        &self,
        message: &str,
        timestamp: i64,
        salt: i64,
        last_seen: &[MessageSignature],
    ) -> Vec<u8> {
        let message = message.as_bytes();
        let mut buf = Vec::with_capacity(4 + 36 + 20 + message.len() + 4 + last_seen.len() * 256);
        buf.extend_from_slice(&1i32.to_be_bytes());
        buf.extend_from_slice(self.sender.as_bytes());
        buf.extend_from_slice(self.session_id.as_bytes());
        buf.extend_from_slice(&self.index.to_be_bytes());
        buf.extend_from_slice(&salt.to_be_bytes());
        buf.extend_from_slice(&timestamp.div_euclid(1000).to_be_bytes());
        buf.extend_from_slice(&(message.len() as i32).to_be_bytes());
        buf.extend_from_slice(message);
        buf.extend_from_slice(&(last_seen.len() as i32).to_be_bytes());
        for entry in last_seen {
            buf.extend_from_slice(&entry.bytes);
        }
        buf
    }

    fn sign_payload(&self, payload: &[u8]) -> [u8; 256] {
        let digest = Sha256::digest(payload);
        let signature = self
            .private_key
            .sign(Pkcs1v15Sign::new::<Sha256>(), &digest)
            .expect("chat signing failed");
        signature.try_into().expect("signature is 256 bytes")
    }
}

impl ChatSessionUpdate {
    /// Builds a `chat_session_update` from the profile key inputs: a fresh
    /// random session id, the key expiry (ms since epoch), the public key
    /// DER and Mojang's signature of it.
    pub fn new(session_id: Uuid, expires_at: i64, public_key: Bytes, key_signature: Bytes) -> Self {
        Self {
            session: ChatSession {
                session_id,
                expires_at,
                public_key,
                key_signature,
            },
        }
    }
}

#[cfg(test)]
mod tests;
