//! Answering the server's teleports: `accept_teleportation` (vanilla
//! `ClientPacketListener.handleMovePlayer` sends it with the teleport) and the first
//! `player_loaded` after the join teleport.

use haste_protocol::packets::play::serverbound as sb;
use haste_protocol::ServerboundPlay;

use super::LocalPlayer;

impl LocalPlayer {
    /// Sends the `accept_teleportation` (and the first `player_loaded`) for a teleport that
    /// arrived a full tick ago. A vanilla client answers teleports from its game loop,
    /// never from the network reader; answering within microseconds of the join teleport
    /// races the server's join bookkeeping and it logs
    /// "moved too quickly! <spawn position>" (observed on the vanilla 26.3 server, ~1 join
    /// in 8). One full tick later the server holds the login position, so the accept's
    /// position matches it. 26.3's accept carries the resulting position and rotation; the
    /// first move_player_pos_rot follows one tick after it, as vanilla's `sendPosition`
    /// does (xLast untouched by the teleport — see the PlayerPosition handler).
    pub(super) fn flush_pending_accept(&mut self, out: &mut Vec<ServerboundPlay>) {
        if self.pending_accept.is_none() {
            return;
        }
        self.pending_accept_age += 1;
        if self.pending_accept_age < 2 && !self.loaded {
            // Age 1 is the tick the teleport arrived in (the driver drains packets and
            // ends the tick in one call). The join teleport (the player not yet `loaded`)
            // answers on the next: replying within microseconds of arrival races the
            // server's join bookkeeping. Every later teleport answers in its arrival tick,
            // ahead of that tick's interactions — the server drops `use_item_on` while a
            // teleport is unconfirmed (`ServerGamePacketListenerImpl.handleUseItemOn`'s
            // `awaitingPositionFromClient` gate), so an accept that waits a tick lets a
            // use slip out before it and vanish server-side.
            return;
        }
        let teleport_id = self.pending_accept.take().unwrap();
        tracing::debug!(
            "accept_teleportation {teleport_id} at ({:.1}, {:.1}, {:.1})",
            self.state.pos.x,
            self.state.pos.y,
            self.state.pos.z
        );
        out.push(ServerboundPlay::AcceptTeleportation(
            sb::AcceptTeleportation {
                teleport_id,
                pos: self.state.pos,
                yaw: self.state.yaw,
                pitch: self.state.pitch,
            },
        ));
        if !self.loaded {
            // Observed on the 26.3 server: a movement packet between accept_teleportation
            // and player_loaded gets the client kicked (invalid_player_movement), so the
            // first move goes out next tick.
            out.push(ServerboundPlay::PlayerLoaded(sb::PlayerLoaded {}));
            self.loaded = true;
        }
    }
}
