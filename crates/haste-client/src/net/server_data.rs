// The server's MOTD and icon (vanilla `ClientPacketListener.handleServerData`). Vanilla writes
// them to the server-list entry it is connected through; Haste keeps them on the session.

use haste_protocol::packets::play::clientbound as cb;

use super::Session;

/// `ServerData.validateIcon`'s limit on each side of the icon, in pixels.
const MAX_ICON_SIDE: u32 = 1024;

/// `PngInfo`'s signature: the eight bytes every PNG file starts with.
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// `ServerData.validateIcon`, read from the PNG header alone: the signature, the IHDR chunk's
/// length and type, and a width and height of at most 1024. `PngInfo`'s bit depth and colour
/// type checks are not ported, so a file with a bad header byte there is still kept.
pub(crate) fn valid_icon(bytes: &[u8]) -> bool {
    let Some(header) = bytes.get(..24) else {
        return false;
    };
    if header[..8] != PNG_SIGNATURE || header[8..12] != [0, 0, 0, 13] || header[12..16] != *b"IHDR"
    {
        return false;
    }
    let width = u32::from_be_bytes([header[16], header[17], header[18], header[19]]);
    let height = u32::from_be_bytes([header[20], header[21], header[22], header[23]]);
    width <= MAX_ICON_SIDE && height <= MAX_ICON_SIDE
}

impl Session {
    /// `handleServerData`: the packet's MOTD replaces the stored one; its icon replaces the stored
    /// icon only when it validates (vanilla maps the icon through `validateIcon`, so an absent or
    /// invalid icon leaves the old one in place).
    pub(super) fn set_server_data(&mut self, s: &cb::ServerData) {
        self.server_motd = Some(s.motd.clone());
        if let Some(icon) = s.icon.as_ref().filter(|icon| valid_icon(icon)) {
            self.server_icon = Some(icon.to_vec());
        }
    }
}

#[cfg(test)]
mod tests;
