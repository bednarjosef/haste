//! Signed chat (online mode): chat sessions, message signing and the
//! last-seen acknowledgement state.
//!
//! Layouts from the minecraft.wiki "Java Edition protocol/Chat" page for
//! 26.3. The signed payload layout (version, sender, session, index, salt,
//! timestamp, message, last-seen entries) follows the wiki; the exact
//! encoding of the last-seen part and of the message string could not be
//! verified against the wiki or vanilla source in this worktree and is
//! documented on [`ChatSigner::payload`].

use std::collections::VecDeque;

use bytes::Bytes;
use rsa::pkcs8::DecodePrivateKey;
use rsa::{Pkcs1v15Sign, RsaPrivateKey};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::packets::play::clientbound::ChatSession;
use crate::packets::play::serverbound::{Chat, ChatSessionUpdate};

/// A seen message's signature: when it was sent and its 256-byte signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageSignature {
    /// Milliseconds since the epoch.
    pub timestamp: i64,
    /// 256-byte signature (all zeros when the message was unsigned).
    pub bytes: [u8; 256],
}

/// The last-seen window and acknowledgement state that travels with every
/// signed chat packet: the `chat` packet's `message_count`, `acknowledged`
/// and `checksum` fields, and the last-seen part of the signed payload.
///
/// Entries are ordered oldest first; at most [`Self::MAX_ENTRIES`] are kept.
/// Bit `i` of the acknowledgement bit set corresponds to `entries[i]`
/// (bit 0 = the oldest entry) and is written little-endian into 3 bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LastSeenMessages {
    entries: VecDeque<MessageSignature>,
    /// Entries not yet acknowledged by a `chat_ack`.
    message_count: i32,
    /// Bit `i` set when `entries[i]` has been acknowledged.
    acknowledged: u32,
}

impl LastSeenMessages {
    /// The window size: the last 20 messages are kept.
    pub const MAX_ENTRIES: usize = 20;

    /// Adds a newly seen message. The oldest entry is evicted when the
    /// window is full.
    pub fn push(&mut self, signature: MessageSignature) {
        if self.entries.len() == Self::MAX_ENTRIES {
            let evicted_unacknowledged = self.acknowledged & 1 == 0;
            self.entries.pop_front();
            self.acknowledged >>= 1;
            if evicted_unacknowledged {
                self.message_count -= 1;
            }
        }
        self.entries.push_back(signature);
        self.message_count += 1;
    }

    /// Marks the entry at `index` (0 = oldest) as acknowledged.
    pub fn acknowledge(&mut self, index: usize) {
        if index < self.entries.len() && self.acknowledged & (1 << index) == 0 {
            self.acknowledged |= 1 << index;
            self.message_count -= 1;
        }
    }

    /// The `chat` packet's `message_count`: entries seen since the last
    /// `chat_ack`.
    pub fn message_count(&self) -> i32 {
        self.message_count
    }

    /// The `chat` packet's `acknowledged`: the 20-bit bit set as 3 bytes,
    /// little-endian.
    pub fn acknowledged_bytes(&self) -> [u8; 3] {
        let bits = self.acknowledged & 0xF_FFFF;
        [
            (bits & 0xFF) as u8,
            ((bits >> 8) & 0xFF) as u8,
            ((bits >> 16) & 0xFF) as u8,
        ]
    }

    /// The `chat` packet's `checksum`, as vanilla's `LastSeenMessages.computeChecksum`: start at
    /// 1, for each entry `31 * c + hash(entry)` where the hash is Java's `Arrays.hashCode` of the
    /// signature bytes (signed bytes, wrapping 32-bit arithmetic); the low byte, or 1 if that is
    /// 0 (0 means "not checked" on the wire).
    pub fn checksum(&self) -> i8 {
        let mut checksum: i32 = 1;
        for entry in &self.entries {
            checksum = checksum
                .wrapping_mul(31)
                .wrapping_add(java_bytes_hash(&entry.bytes));
        }
        match checksum as i8 {
            0 => 1,
            c => c,
        }
    }

    /// The last-seen entries, oldest first, for the signed payload.
    pub fn entries(&self) -> impl Iterator<Item = MessageSignature> + '_ {
        self.entries.iter().copied()
    }

    /// The `chat_ack` payload: how many messages were pending, then resets
    /// the acknowledgement state.
    pub fn take_ack(&mut self) -> i32 {
        let count = self.message_count;
        self.message_count = 0;
        self.acknowledged = 0;
        count
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

/// Java's `Arrays.hashCode(byte[])`.
fn java_bytes_hash(bytes: &[u8]) -> i32 {
    bytes.iter().fold(1i32, |h, &b| {
        h.wrapping_mul(31).wrapping_add(i32::from(b as i8))
    })
}

#[cfg(test)]
mod tests;
