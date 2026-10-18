// The chat packets: system, disguised and player chat lines pushed onto the counters' chat
// log.

use haste_protocol::ClientboundPlay;

use super::Session;

impl Session {
    /// The system, disguised and player chat packets; returns the packet when it is none of
    /// them.
    pub(super) fn apply_chat(&mut self, p: ClientboundPlay) -> Option<ClientboundPlay> {
        use ClientboundPlay as P;
        match p {
            P::SystemChat(c) if !c.overlay => self.push_chat(c.content.to_plain(&|_| None)),
            P::DisguisedChat(c) => self.push_chat(format!(
                "<{}> {}",
                c.sender_name.to_plain(&|_| None),
                c.message.to_plain(&|_| None)
            )),
            P::PlayerChat(c) => self.push_chat(format!(
                "<{}> {}",
                c.sender_name.to_plain(&|_| None),
                c.message
            )),
            _ => return Some(p),
        }
        None
    }

    fn push_chat(&mut self, line: String) {
        tracing::info!("chat: {line}");
        self.counters.chat.push(line);
        if self.counters.chat.len() > 20 {
            self.counters.chat.remove(0);
        }
    }
}
