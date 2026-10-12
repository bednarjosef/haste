//! Serverbound play packets.
//!
//! Layouts from the minecraft.wiki "Java Edition protocol/Packets" page for
//! 26.3. Note: 26.3 has no serverbound `swing` packet; arm swings are
//! `punch` (no fields) and attacks are `attack`.

use bytes::Bytes;
use haste_core::{BlockPos, DVec3};

pub use crate::packets::configuration::serverbound::{ClientInformation, CustomPayload, KeepAlive, Pong};
use crate::packets::{packet, packet_enum, Packet};
use crate::wire::{DecodeResult, LpVec3, Reader, VarInt, Writer};

packet_enum! {
    /// Serverbound play packets.
    ServerboundPlay {
        AcceptTeleportation, Attack, ChatAck, ChatCommand, Chat, ChunkBatchReceived, ClientTickEnd,
        ClientInformation, ConfigurationAcknowledged, CustomPayload, Interact, KeepAlive, MovePlayerPos,
        MovePlayerPosRot, MovePlayerRot, MovePlayerStatusOnly, PlayerAction, PlayerCommand, PlayerInput,
        PlayerLoaded, Pong, Punch, SetCarriedItem, UseItemOn, UseItem,
    }
}

/// Movement flag bits for the `move_player_*` packets.
pub mod move_flags {
    /// On ground.
    pub const ON_GROUND: u8 = 0x01;
    /// Pushing against a wall (horizontal collision).
    pub const HORIZONTAL_COLLISION: u8 = 0x02;
}

packet! {
    /// Confirms a `player_position` teleport; in 26.3 it also reports the
    /// resulting position and rotation.
    AcceptTeleportation = "minecraft:accept_teleportation" {
        /// The teleport id.
        teleport_id: i32 as VarInt,
        /// Resulting position.
        pos: DVec3,
        /// Resulting yaw.
        yaw: f32,
        /// Resulting pitch.
        pitch: f32,
    }
}

packet! {
    /// Attacks an entity.
    Attack = "minecraft:attack" {
        /// Target.
        entity_id: i32 as VarInt,
    }
}

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
        Ok(Self { message, timestamp, salt, signature, message_count, acknowledged: [a[0], a[1], a[2]], checksum: r.i8()? })
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

packet! {
    /// Acknowledges a chunk batch and asks for a send rate.
    ChunkBatchReceived = "minecraft:chunk_batch_received" {
        /// Desired chunks per tick.
        chunks_per_tick: f32,
    }
}

packet! {
    /// End of a client tick.
    ClientTickEnd = "minecraft:client_tick_end" {}
}

packet! {
    /// Acknowledges `start_configuration`.
    ConfigurationAcknowledged = "minecraft:configuration_acknowledged" {}
}

packet! {
    /// Right-click on an entity.
    Interact = "minecraft:interact" {
        /// Target.
        entity_id: i32 as VarInt,
        /// 0 main hand, 1 off hand.
        hand: i32 as VarInt,
        /// Cursor offset from the entity position.
        target_offset: DVec3 as LpVec3,
        /// Sneak key pressed.
        sneaking: bool,
    }
}

packet! {
    /// Position update.
    MovePlayerPos = "minecraft:move_player_pos" {
        /// Feet position.
        pos: DVec3,
        /// [`move_flags`] bits.
        flags: u8,
    }
}

packet! {
    /// Position and rotation update.
    MovePlayerPosRot = "minecraft:move_player_pos_rot" {
        /// Feet position.
        pos: DVec3,
        /// Yaw.
        yaw: f32,
        /// Pitch.
        pitch: f32,
        /// [`move_flags`] bits.
        flags: u8,
    }
}

packet! {
    /// Rotation update.
    MovePlayerRot = "minecraft:move_player_rot" {
        /// Yaw.
        yaw: f32,
        /// Pitch.
        pitch: f32,
        /// [`move_flags`] bits.
        flags: u8,
    }
}

packet! {
    /// Flags-only update.
    MovePlayerStatusOnly = "minecraft:move_player_status_only" {
        /// [`move_flags`] bits.
        flags: u8,
    }
}

packet! {
    /// Digging, dropping, swapping hands.
    PlayerAction = "minecraft:player_action" {
        /// 0 start digging .. 7 stab.
        status: i32 as VarInt,
        /// Block.
        pos: BlockPos,
        /// Face (Direction ordinal).
        face: u8,
        /// Block change sequence.
        sequence: i32 as VarInt,
    }
}

packet! {
    /// Sprinting, leaving bed, horse jumps.
    PlayerCommand = "minecraft:player_command" {
        /// Player entity id (ignored by vanilla).
        entity_id: i32 as VarInt,
        /// 0 leave bed, 1 start sprint, 2 stop sprint, 3/4 horse jump, 5 vehicle inventory, 6 elytra.
        action: i32 as VarInt,
        /// Horse jump boost 0..100.
        jump_boost: i32 as VarInt,
    }
}

/// `player_input` flag bits.
pub mod input_flags {
    #![allow(missing_docs)]
    pub const FORWARD: u8 = 0x01;
    pub const BACKWARD: u8 = 0x02;
    pub const LEFT: u8 = 0x04;
    pub const RIGHT: u8 = 0x08;
    pub const JUMP: u8 = 0x10;
    pub const SNEAK: u8 = 0x20;
    pub const SPRINT: u8 = 0x40;
}

packet! {
    /// Held movement keys.
    PlayerInput = "minecraft:player_input" {
        /// [`input_flags`] bits.
        flags: u8,
    }
}

packet! {
    /// The client finished loading terrain.
    PlayerLoaded = "minecraft:player_loaded" {}
}

packet! {
    /// Arm swing.
    Punch = "minecraft:punch" {}
}

packet! {
    /// Selected hotbar slot.
    SetCarriedItem = "minecraft:set_carried_item" {
        /// Slot 0..8.
        slot: i16,
    }
}

packet! {
    /// Use the held item on a block.
    UseItemOn = "minecraft:use_item_on" {
        /// 0 main hand, 1 off hand.
        hand: i32 as VarInt,
        /// Block.
        pos: BlockPos,
        /// Face (Direction ordinal).
        face: i32 as VarInt,
        /// Cursor x in the block, 0..1.
        cursor_x: f32,
        /// Cursor y.
        cursor_y: f32,
        /// Cursor z.
        cursor_z: f32,
        /// Head inside a block.
        inside_block: bool,
        /// World border hit.
        world_border_hit: bool,
        /// Block change sequence.
        sequence: i32 as VarInt,
    }
}

packet! {
    /// Use the held item.
    UseItem = "minecraft:use_item" {
        /// 0 main hand, 1 off hand.
        hand: i32 as VarInt,
        /// Block change sequence.
        sequence: i32 as VarInt,
        /// Yaw.
        yaw: f32,
        /// Pitch.
        pitch: f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::test_util::round_trip;

    #[test]
    fn round_trips() {
        round_trip(&AcceptTeleportation { teleport_id: 1, pos: DVec3::new(0.5, 64.0, 0.5), yaw: 90.0, pitch: 0.0 });
        round_trip(&Attack { entity_id: 5 });
        round_trip(&ChatAck { message_count: 2 });
        round_trip(&ChatCommand { command: "time set day".into() });
        round_trip(&Chat { message: "hi".into(), timestamp: 1, salt: 2, signature: None, message_count: 0, acknowledged: [0; 3], checksum: 0 });
        round_trip(&Chat { message: "hi".into(), timestamp: 1, salt: 2, signature: Some(Bytes::from(vec![1; 256])), message_count: 1, acknowledged: [1, 0, 0], checksum: 5 });
        round_trip(&ChunkBatchReceived { chunks_per_tick: 64.0 });
        round_trip(&ClientTickEnd {});
        round_trip(&ClientInformation::defaults("en_us", 8));
        round_trip(&ConfigurationAcknowledged {});
        round_trip(&Interact { entity_id: 1, hand: 0, target_offset: DVec3::ZERO, sneaking: false });
        round_trip(&MovePlayerPos { pos: DVec3::new(1.0, 2.0, 3.0), flags: move_flags::ON_GROUND });
        round_trip(&MovePlayerPosRot { pos: DVec3::ZERO, yaw: 1.0, pitch: 2.0, flags: 0 });
        round_trip(&MovePlayerRot { yaw: 1.0, pitch: 2.0, flags: 3 });
        round_trip(&MovePlayerStatusOnly { flags: 1 });
        round_trip(&PlayerAction { status: 0, pos: BlockPos::new(1, 2, 3), face: 1, sequence: 4 });
        round_trip(&PlayerCommand { entity_id: 1, action: 1, jump_boost: 0 });
        round_trip(&PlayerInput { flags: input_flags::FORWARD | input_flags::SPRINT });
        round_trip(&PlayerLoaded {});
        round_trip(&Punch {});
        round_trip(&SetCarriedItem { slot: 3 });
        round_trip(&UseItemOn { hand: 0, pos: BlockPos::new(0, 64, 0), face: 1, cursor_x: 0.5, cursor_y: 1.0, cursor_z: 0.5, inside_block: false, world_border_hit: false, sequence: 7 });
        round_trip(&UseItem { hand: 1, sequence: 8, yaw: 0.0, pitch: 0.0 });
    }
}
