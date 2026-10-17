use super::*;

packet! {
    /// A chat message without signature data (`/say`, console).
    DisguisedChat = "minecraft:disguised_chat" {
        /// Message.
        message: Component,
        /// Chat type (registry id or inline).
        chat_type: IdOr<ChatType>,
        /// Sender name.
        sender_name: Component,
        /// Target name.
        target_name: Option<Component>,
    }
}
/// A previous-message reference in `player_chat`: an id into the client's
/// cache, or a full signature when the id is unknown.
#[derive(Clone, Debug, PartialEq)]
pub enum PreviousMessage {
    /// Cached message id.
    Id(i32),
    /// Full 256-byte signature.
    Signature(Bytes),
}

/// A signed player chat message (decoded, signature not verified).
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerChat {
    /// Global message index.
    pub global_index: i32,
    /// Sender UUID.
    pub sender: Uuid,
    /// Per-sender index.
    pub index: i32,
    /// 256-byte signature, if signed.
    pub signature: Option<Bytes>,
    /// Message text.
    pub message: String,
    /// Timestamp in ms.
    pub timestamp: i64,
    /// Salt.
    pub salt: i64,
    /// Last-seen messages.
    pub previous: Vec<PreviousMessage>,
    /// Unsigned replacement content.
    pub unsigned_content: Option<Component>,
    /// 0 pass through, 1 fully filtered, 2 partially filtered.
    pub filter_type: i32,
    /// Filter mask when partially filtered.
    pub filter_bits: Option<Vec<u64>>,
    /// Chat type.
    pub chat_type: IdOr<ChatType>,
    /// Sender name.
    pub sender_name: Component,
    /// Target name.
    pub target_name: Option<Component>,
}
impl Packet for PlayerChat {
    const NAME: &'static str = "minecraft:player_chat";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let global_index = r.var_int()?;
        let sender = r.uuid()?;
        let index = r.var_int()?;
        let signature = r.option(|r| Ok(Bytes::copy_from_slice(r.take(256)?)))?;
        let message = r.string_max(256)?;
        let timestamp = r.i64()?;
        let salt = r.i64()?;
        let previous = r.list(20, |r| {
            let id = r.var_int()?;
            if id == 0 {
                Ok(PreviousMessage::Signature(Bytes::copy_from_slice(
                    r.take(256)?,
                )))
            } else {
                Ok(PreviousMessage::Id(id - 1))
            }
        })?;
        let unsigned_content = r.option(|r| r.component())?;
        let filter_type = r.var_int()?;
        let filter_bits = if filter_type == 2 {
            Some(r.bit_set()?)
        } else {
            None
        };
        Ok(Self {
            global_index,
            sender,
            index,
            signature,
            message,
            timestamp,
            salt,
            previous,
            unsigned_content,
            filter_type,
            filter_bits,
            chat_type: r.read()?,
            sender_name: r.component()?,
            target_name: r.read()?,
        })
    }
    fn encode(&self, w: &mut Writer) {
        w.var_int(self.global_index);
        w.uuid(&self.sender);
        w.var_int(self.index);
        w.option(&self.signature, |w, s| w.bytes(s));
        w.string(&self.message);
        w.i64(self.timestamp);
        w.i64(self.salt);
        w.list(&self.previous, |w, p| match p {
            PreviousMessage::Id(i) => w.var_int(i + 1),
            PreviousMessage::Signature(s) => {
                w.var_int(0);
                w.bytes(s);
            }
        });
        w.write(&self.unsigned_content);
        w.var_int(self.filter_type);
        if let Some(b) = &self.filter_bits {
            w.bit_set(b);
        }
        w.write(&self.chat_type);
        w.component(&self.sender_name);
        w.write(&self.target_name);
    }
}
packet! {
    /// A system message.
    SystemChat = "minecraft:system_chat" {
        /// Content.
        content: Component,
        /// Action bar instead of chat.
        overlay: bool,
    }
}
packet! {
    /// Custom chat completions (unused by vanilla servers).
    CustomChatCompletions = "minecraft:custom_chat_completions" {
        /// 0 add, 1 remove, 2 set.
        action: i32 as VarInt,
        /// Entries.
        entries: Vec<String>,
    }
}
/// Removes a signed chat message.
#[derive(Clone, Debug, PartialEq)]
pub struct DeleteChat {
    /// The message (cache id or full signature).
    pub message: PreviousMessage,
}
impl Packet for DeleteChat {
    const NAME: &'static str = "minecraft:delete_chat";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let id = r.var_int()?;
        let message = if id == 0 {
            PreviousMessage::Signature(Bytes::copy_from_slice(r.take(256)?))
        } else {
            PreviousMessage::Id(id - 1)
        };
        Ok(Self { message })
    }
    fn encode(&self, w: &mut Writer) {
        match &self.message {
            PreviousMessage::Id(i) => w.var_int(i + 1),
            PreviousMessage::Signature(s) => {
                w.var_int(0);
                w.bytes(s);
            }
        }
    }
}
