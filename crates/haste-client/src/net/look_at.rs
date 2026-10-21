// The `player_look_at` packet (vanilla `ClientPacketListener.handleLookAt`): the server turns the
// local player to face a point, or the feet or eyes of an entity, through `LocalPlayer.lookAt`
// (`Entity.lookAt`). Head and body yaw are not synced here: only the look angles change.

use glam::DVec3;
use haste_core::EntityId;
use haste_protocol::packets::play::clientbound as cb;
use haste_protocol::ClientboundPlay;

use super::{EntitySink, Session};

mod atan2;
mod rotation;
#[cfg(test)]
mod tests;

/// `EntityAnchorArgument.Anchor` ordinal on the wire for the eyes (the feet are 0).
const ANCHOR_EYES: i32 = 1;

/// `EntityAnchorArgument.Anchor.apply`: the eyes at `eye_y` above `pos`'s x and z, or the feet
/// at `pos`. Unknown anchors read as the feet, so an odd value turns the player instead of failing.
fn anchor_point(pos: DVec3, eye_y: f64, anchor: i32) -> DVec3 {
    if anchor == ANCHOR_EYES {
        DVec3::new(pos.x, eye_y, pos.z)
    } else {
        pos
    }
}

impl Session {
    /// The `player_look_at` packet; returns the packet when it is not one.
    pub(super) fn apply_look_at<E: EntitySink + ?Sized>(
        &mut self,
        p: ClientboundPlay,
        entities: &E,
    ) -> Option<ClientboundPlay> {
        use ClientboundPlay as P;
        match p {
            P::PlayerLookAt(l) => self.look_at(&l, entities),
            _ => return Some(p),
        }
        None
    }

    /// `handleLookAt`: the point is the target, or the entity's anchor when the entity is known
    /// (`getPosition`); the local player's own anchor is the `from` point.
    fn look_at<E: EntitySink + ?Sized>(&mut self, l: &cb::PlayerLookAt, entities: &E) {
        let to = l
            .entity
            .as_ref()
            .and_then(|e| self.entity_anchor(EntityId(e.entity_id), e.anchor, entities))
            .unwrap_or(l.target);
        let from = self.own_anchor(l.from_anchor);
        let (yaw, pitch) = rotation::look_angles(from, to);
        self.player.set_look(yaw, pitch);
    }

    /// The local player's anchor point: its feet, or its eyes at its feet's y plus the eye height
    /// of its current pose.
    fn own_anchor(&self, anchor: i32) -> DVec3 {
        let pos = self.player.state.pos;
        anchor_point(pos, pos.y + self.player.eye_height(), anchor)
    }

    /// An entity's anchor point; None when the store does not hold it. Our own id is the local
    /// player, which the store does not hold.
    fn entity_anchor<E: EntitySink + ?Sized>(
        &self,
        id: EntityId,
        anchor: i32,
        entities: &E,
    ) -> Option<DVec3> {
        if id == self.entity_id {
            return Some(self.own_anchor(anchor));
        }
        let (pos, _, _) = entities.pose(id)?;
        let eye_y = match anchor {
            ANCHOR_EYES => entities.entity_eye_y(id)?,
            _ => pos.y,
        };
        Some(anchor_point(pos, eye_y, anchor))
    }
}
