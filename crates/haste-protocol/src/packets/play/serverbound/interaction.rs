use super::*;

packet! {
    /// Attacks an entity.
    Attack = "minecraft:attack" {
        /// Target.
        entity_id: i32 as VarInt,
    }
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
/// `player_action` status values. 26.3 inserted `CHANGE_DESTROY_DIRECTION`
/// at 1 and renumbered the rest (observed on the 26.3 server by the player
/// workstream; minecraft.wiki still lists the old order).
pub mod player_action_status {
    /// Start breaking a block.
    pub const START_DESTROY_BLOCK: i32 = 0;
    /// The face being broken changed (new in 26.3).
    pub const CHANGE_DESTROY_DIRECTION: i32 = 1;
    /// Cancel breaking.
    pub const ABORT_DESTROY_BLOCK: i32 = 2;
    /// Finish breaking.
    pub const STOP_DESTROY_BLOCK: i32 = 3;
    /// Drop the whole held stack.
    pub const DROP_ALL_ITEMS: i32 = 4;
    /// Drop one held item.
    pub const DROP_ITEM: i32 = 5;
    /// Release the item in use (bow, shield, ...).
    pub const RELEASE_USE_ITEM: i32 = 6;
    /// Swap main and off hand.
    pub const SWAP_ITEM_WITH_OFFHAND: i32 = 7;
    /// Spear stab.
    pub const STAB: i32 = 8;
}
packet! {
    /// Digging, dropping, swapping hands.
    PlayerAction = "minecraft:player_action" {
        /// A [`player_action_status`] value (26.3 order).
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
    /// Arm swing.
    Punch = "minecraft:punch" {}
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
packet! {
    /// Pick block on a block.
    PickItemFromBlock = "minecraft:pick_item_from_block" {
        /// Block.
        pos: BlockPos,
        /// Include block data (creative).
        include_data: bool,
    }
}
packet! {
    /// Pick block on an entity.
    PickItemFromEntity = "minecraft:pick_item_from_entity" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// Include data.
        include_data: bool,
    }
}
packet! {
    /// Spectator left click.
    SpectatorAction = "minecraft:spectator_action" {
        /// Targeted entity id + 1, 0 for none.
        entity: i32 as VarInt,
    }
}
packet! {
    /// Spectator teleport to an entity.
    TeleportToEntity = "minecraft:teleport_to_entity" {
        /// Target UUID.
        target: Uuid,
    }
}
