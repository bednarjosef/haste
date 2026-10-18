//! The client's last-seen message window (vanilla `LastSeenMessagesTracker`, with
//! `LastSeenMessages.computeChecksum` and the `LastSeenMessages.Update` it produces).
//!
//! Every signed chat message the client processes is added as pending; each outgoing chat
//! (and signed command) acknowledges the whole window, and the server's
//! `LastSeenMessagesValidator` replays the same steps, so the order and the bit layout must
//! match vanilla exactly or the server kicks with "chat validation failed".

use super::MessageSignature;

/// Vanilla's window size (`ClientPacketListener` creates `new LastSeenMessagesTracker(20)`).
pub const LAST_SEEN_COUNT: usize = 20;

/// One window slot (vanilla `LastSeenTrackedEntry`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TrackedEntry {
    signature: MessageSignature,
    /// Not yet sent in an acknowledgement.
    pending: bool,
}

/// The client's last-seen window (vanilla `LastSeenMessagesTracker`).
#[derive(Clone, Debug)]
pub struct LastSeenMessagesTracker {
    /// Ring buffer; `tail` is the next slot written, so the oldest entry.
    tracked: [Option<TrackedEntry>; LAST_SEEN_COUNT],
    tail: usize,
    /// Messages added since the last acknowledgement (`chat` / `chat_ack`).
    offset: i32,
    last_tracked: Option<MessageSignature>,
}

impl Default for LastSeenMessagesTracker {
    fn default() -> Self {
        Self {
            tracked: [None; LAST_SEEN_COUNT],
            tail: 0,
            offset: 0,
            last_tracked: None,
        }
    }
}

/// What one outgoing chat carries (vanilla `LastSeenMessagesTracker.Update`): the entries the
/// signature covers, and the packet's `message_count`, `acknowledged` and `checksum` fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastSeenUpdate {
    /// The acknowledged signatures, oldest first (`LastSeenMessages`).
    pub entries: Vec<MessageSignature>,
    /// Messages seen since the previous acknowledgement.
    pub offset: i32,
    /// The 20-bit acknowledgement set as vanilla's `FixedBitSet` writes it: bit `i` is window
    /// slot `i` counted from the oldest, little-endian over 3 bytes.
    pub acknowledged: [u8; 3],
    /// `LastSeenMessages.computeChecksum` of `entries`.
    pub checksum: i8,
}

impl LastSeenMessagesTracker {
    /// Vanilla `addPending`: a processed signed message enters the window (as an empty slot
    /// when it was not shown). Repeats of the last message are ignored; returns whether it was
    /// added.
    pub fn add_pending(&mut self, signature: MessageSignature, was_shown: bool) -> bool {
        if self.last_tracked == Some(signature) {
            return false;
        }
        self.last_tracked = Some(signature);
        let entry = was_shown.then_some(TrackedEntry {
            signature,
            pending: true,
        });
        self.tracked[self.tail] = entry;
        self.tail = (self.tail + 1) % LAST_SEEN_COUNT;
        self.offset += 1;
        true
    }

    /// Vanilla `ignorePending`: a deleted message (`delete_chat`) leaves the window if it has
    /// not been acknowledged yet.
    pub fn ignore_pending(&mut self, signature: &MessageSignature) {
        if let Some(slot) = self
            .tracked
            .iter_mut()
            .find(|e| e.is_some_and(|e| e.pending && e.signature == *signature))
        {
            *slot = None;
        }
    }

    /// Messages added since the last acknowledgement (vanilla `offset`).
    pub fn offset(&self) -> i32 {
        self.offset
    }

    /// Vanilla `getAndClearOffset` (the `chat_ack` payload).
    pub fn take_offset(&mut self) -> i32 {
        std::mem::take(&mut self.offset)
    }

    /// Vanilla `generateAndApplyUpdate`: acknowledges every message in the window, oldest
    /// first, and clears the offset.
    pub fn generate_and_apply_update(&mut self) -> LastSeenUpdate {
        let offset = self.take_offset();
        let mut bits = 0u32;
        let mut entries = Vec::with_capacity(LAST_SEEN_COUNT);
        for i in 0..LAST_SEEN_COUNT {
            let index = (self.tail + i) % LAST_SEEN_COUNT;
            if let Some(entry) = &mut self.tracked[index] {
                bits |= 1 << i;
                entries.push(entry.signature);
                entry.pending = false;
            }
        }
        LastSeenUpdate {
            checksum: checksum(&entries),
            entries,
            offset,
            acknowledged: [bits as u8, (bits >> 8) as u8, (bits >> 16) as u8],
        }
    }
}

/// Vanilla `LastSeenMessages.computeChecksum`: `31 * c + Arrays.hashCode(signature)` over the
/// entries from 1, as Java ints; the low byte, with 0 (meaning "not checked") replaced by 1.
pub fn checksum(entries: &[MessageSignature]) -> i8 {
    let c = entries.iter().fold(1i32, |c, e| {
        c.wrapping_mul(31).wrapping_add(java_bytes_hash(&e.bytes))
    });
    match c as i8 {
        0 => 1,
        b => b,
    }
}

/// Java's `Arrays.hashCode(byte[])` (signed bytes, wrapping int arithmetic).
fn java_bytes_hash(bytes: &[u8]) -> i32 {
    bytes.iter().fold(1i32, |h, &b| {
        h.wrapping_mul(31).wrapping_add(i32::from(b as i8))
    })
}
