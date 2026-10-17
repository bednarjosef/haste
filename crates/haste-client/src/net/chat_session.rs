//! The client's side of signed chat (the chat parts of vanilla `ClientPacketListener`):
//! announcing the chat session on joining an online-mode server, tracking the signed
//! messages seen, acknowledging them and signing every chat message sent.
//!
//! - `handleLogin`: resets the chain and the last-seen window; in online mode the profile key
//!   pair starts a [`LocalChatSession`] (`setKeyPair`) announced with `chat_session_update`.
//! - `handlePlayerChat` -> `ChatListener` -> `markMessageAsProcessed`: each signed message
//!   enters the window; past 64 unacknowledged ones a `chat_ack` goes out.
//! - `handleDeleteChat`: a deleted message leaves the window unless acknowledged.
//! - `sendChat`: the last-seen update and the signature fill the outgoing `chat` packet.
//!
//! Incoming signatures are not verified (vanilla's `SignedMessageValidator` per sender): the
//! server already checked them, and acknowledging what the server sent is all it validates.

use haste_protocol::packets::play::clientbound::{DeleteChat, PlayerChat, PreviousMessage};
use haste_protocol::packets::play::serverbound as sb;
use haste_protocol::{
    ChatSigner, ClientboundPlay, LastSeenMessagesTracker, LocalChatSession, MessageSignature,
    ProfileKeyPair, ServerboundPlay,
};
use uuid::Uuid;

/// Unacknowledged messages after which vanilla sends a `chat_ack`
/// (`ClientPacketListener.markMessageAsProcessed`).
const ACK_AFTER: i32 = 64;

/// The local player's chat state for one connection.
pub struct ClientChat {
    /// Our profile id (the sender in every signed message).
    profile_id: Uuid,
    /// The profile key pair from the player certificates; `None` for offline accounts.
    keys: Option<ProfileKeyPair>,
    /// The session announced to this server, and its message chain.
    session: Option<(LocalChatSession, ChatSigner)>,
    last_seen: LastSeenMessagesTracker,
}

impl ClientChat {
    /// Chat for `profile_id`; `keys` signs messages on online-mode servers.
    pub fn new(profile_id: Uuid, keys: Option<ProfileKeyPair>) -> Self {
        Self {
            profile_id,
            keys,
            session: None,
            last_seen: LastSeenMessagesTracker::default(),
        }
    }

    /// Sets the profile key pair (before the first `receive`, so the login packet sees it).
    pub fn set_keys(&mut self, keys: Option<ProfileKeyPair>) {
        self.keys = keys;
    }

    /// The incoming packets chat cares about; replies go to `out`.
    pub fn handle(&mut self, p: &ClientboundPlay, out: &mut Vec<ServerboundPlay>) {
        match p {
            ClientboundPlay::Login(l) => self.on_login(l.online_mode, out),
            ClientboundPlay::PlayerChat(c) => self.on_player_chat(c, out),
            ClientboundPlay::DeleteChat(d) => self.on_delete_chat(d),
            _ => {}
        }
    }

    /// Vanilla `handleLogin`: a fresh chain and window; online mode announces our session.
    fn on_login(&mut self, online_mode: bool, out: &mut Vec<ServerboundPlay>) {
        self.session = None;
        self.last_seen = LastSeenMessagesTracker::default();
        if !online_mode {
            return;
        }
        let Some(keys) = self.keys.clone() else {
            tracing::info!("online-mode server, but no profile keys: chat is unsigned");
            return;
        };
        match LocalChatSession::create(keys) {
            Ok(session) => {
                out.push(ServerboundPlay::ChatSessionUpdate(session.update_packet()));
                tracing::info!("chat session {} announced", session.session_id());
                let encoder = session.message_encoder(self.profile_id);
                self.session = Some((session, encoder));
            }
            Err(e) => tracing::warn!("profile keys unusable, chat is unsigned: {e}"),
        }
    }

    /// `handlePlayerChat` through `markMessageAsProcessed` (every signed message counts as
    /// shown: Haste has no hidden or blocked senders yet).
    fn on_player_chat(&mut self, c: &PlayerChat, out: &mut Vec<ServerboundPlay>) {
        let Some(signature) = c
            .signature
            .as_deref()
            .and_then(MessageSignature::from_slice)
        else {
            return;
        };
        if self.last_seen.add_pending(signature, true) && self.last_seen.offset() > ACK_AFTER {
            self.send_ack(out);
        }
    }

    /// Vanilla `handleDeleteChat`. A message named by its cache id needs vanilla's
    /// `MessageSignatureCache`, which Haste does not keep; leaving it in the window only
    /// acknowledges it, which the server accepts.
    fn on_delete_chat(&mut self, d: &DeleteChat) {
        if let PreviousMessage::Signature(bytes) = &d.message {
            if let Some(signature) = MessageSignature::from_slice(bytes) {
                self.last_seen.ignore_pending(&signature);
            }
        }
    }

    /// Vanilla `sendChatAcknowledgement`.
    fn send_ack(&mut self, out: &mut Vec<ServerboundPlay>) {
        let offset = self.last_seen.take_offset();
        if offset > 0 {
            out.push(ServerboundPlay::ChatAck(sb::ChatAck {
                message_count: offset,
            }));
        }
    }

    /// Vanilla `sendChat` for an outgoing `chat` packet: the last-seen update always (also
    /// unsigned, as vanilla's `Encoder.UNSIGNED`), the signature when a session exists.
    /// Other packets pass unchanged.
    pub fn pack(&mut self, p: &mut ServerboundPlay) {
        let ServerboundPlay::Chat(chat) = p else {
            return;
        };
        match &mut self.session {
            Some((_, encoder)) => encoder.pack_chat(chat, &mut self.last_seen),
            None => {
                let update = self.last_seen.generate_and_apply_update();
                chat.signature = None;
                chat.message_count = update.offset;
                chat.acknowledged = update.acknowledged;
                chat.checksum = update.checksum;
            }
        }
    }
}

#[cfg(test)]
mod tests;
