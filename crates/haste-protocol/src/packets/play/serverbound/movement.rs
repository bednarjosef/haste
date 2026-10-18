use super::*;

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
    /// Client-controlled vehicle movement.
    MoveVehicle = "minecraft:move_vehicle" {
        /// Position.
        pos: DVec3,
        /// Yaw.
        yaw: f32,
        /// Pitch.
        pitch: f32,
        /// On ground.
        on_ground: bool,
    }
}
packet! {
    /// Boat paddle state.
    PaddleBoat = "minecraft:paddle_boat" {
        /// Left paddle turning.
        left: bool,
        /// Right paddle turning.
        right: bool,
    }
}
packet! {
    /// Flying state.
    PlayerAbilities = "minecraft:player_abilities" {
        /// 0x02 flying.
        flags: i8,
    }
}
