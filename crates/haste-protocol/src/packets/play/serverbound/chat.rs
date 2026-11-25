use super::*;

packet! {
    /// Acknowledges received signed chat messages.
    ChatAck = "minecraft:chat_ack" {
        /// Messages seen since the last acknowledgement.
        message_count: i32 as VarInt,
    }
}
packet! {
    /// An unsigned command (without the `/`).
    ChatCommand = "minecraft:chat_command" {
        /// Command text.
        command: String,
    }
}
/// A chat message. Haste sends unsigned messages (`signature: None`).
#[derive(Clone, Debug, PartialEq)]
pub struct Chat {
    /// Text (at most 256 chars).
    pub message: String,
    /// Milliseconds since the epoch.
    pub timestamp: i64,
    /// Salt.
    pub salt: i64,
    /// 256-byte signature.
    pub signature: Option<Bytes>,
    /// Messages seen since the last chat packet.
    pub message_count: i32,
    /// Fixed bit set (20 bits, 3 bytes) of acknowledged messages.
    pub acknowledged: [u8; 3],
    /// Checksum of the last-seen signatures (0 when none).
    pub checksum: i8,
}
impl Packet for Chat {
    const NAME: &'static str = "minecraft:chat";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let message = r.string_max(256)?;
        let timestamp = r.i64()?;
        let salt = r.i64()?;
        let signature = r.option(|r| Ok(Bytes::copy_from_slice(r.take(256)?)))?;
        let message_count = r.var_int()?;
        let a = r.take(3)?;
        Ok(Self {
            message,
            timestamp,
            salt,
            signature,
            message_count,
            acknowledged: [a[0], a[1], a[2]],
            checksum: r.i8()?,
        })
    }
    fn encode(&self, w: &mut Writer) {
        w.string(&self.message);
        w.i64(self.timestamp);
        w.i64(self.salt);
        w.option(&self.signature, |w, s| w.bytes(s));
        w.var_int(self.message_count);
        w.bytes(&self.acknowledged);
        w.i8(self.checksum);
    }
}
/// A signed command argument.
#[derive(Clone, Debug, PartialEq)]
pub struct ArgumentSignature {
    /// Argument name.
    pub name: String,
    /// 256-byte signature.
    pub signature: Bytes,
}
/// A command with signed arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatCommandSigned {
    /// Command without `/`.
    pub command: String,
    /// Milliseconds since the epoch.
    pub timestamp: i64,
    /// Salt.
    pub salt: i64,
    /// Argument signatures (at most 8).
    pub arguments: Vec<ArgumentSignature>,
    /// Messages seen.
    pub message_count: i32,
    /// Fixed bit set (20 bits).
    pub acknowledged: [u8; 3],
    /// Checksum.
    pub checksum: i8,
}
impl Packet for ChatCommandSigned {
    const NAME: &'static str = "minecraft:chat_command_signed";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let command = r.string()?;
        let timestamp = r.i64()?;
        let salt = r.i64()?;
        let arguments = r.list(8, |r| {
            Ok(ArgumentSignature {
                name: r.string_max(16)?,
                signature: Bytes::copy_from_slice(r.take(256)?),
            })
        })?;
        let message_count = r.var_int()?;
        let a = r.take(3)?;
        Ok(Self {
            command,
            timestamp,
            salt,
            arguments,
            message_count,
            acknowledged: [a[0], a[1], a[2]],
            checksum: r.i8()?,
        })
    }
    fn encode(&self, w: &mut Writer) {
        w.string(&self.command);
        w.i64(self.timestamp);
        w.i64(self.salt);
        w.list(&self.arguments, |w, a| {
            w.string(&a.name);
            w.bytes(&a.signature);
        });
        w.var_int(self.message_count);
        w.bytes(&self.acknowledged);
        w.i8(self.checksum);
    }
}
packet! {
    /// Chat session public key (signed chat).
    ChatSessionUpdate = "minecraft:chat_session_update" {
        /// Session.
        session: ChatSession,
    }
}
packet! {
    /// Tab-completion request.
    CommandSuggestion = "minecraft:command_suggestion" {
        /// Transaction id.
        id: i32 as VarInt,
        /// Text before the cursor.
        text: String,
    }
}
