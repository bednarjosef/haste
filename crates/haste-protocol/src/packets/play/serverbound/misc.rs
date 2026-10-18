use super::*;

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
    /// F3+I on a block.
    BlockEntityTagQuery = "minecraft:block_entity_tag_query" {
        /// Transaction id.
        transaction_id: i32 as VarInt,
        /// Block.
        pos: BlockPos,
    }
}
packet! {
    /// Changes the difficulty (operators).
    ChangeDifficulty = "minecraft:change_difficulty" {
        /// 0 peaceful .. 3 hard.
        difficulty: u8,
    }
}
packet! {
    /// Requests a game mode (F3+F4).
    ChangeGameMode = "minecraft:change_game_mode" {
        /// Game mode id.
        game_mode: i32 as VarInt,
    }
}
packet! {
    /// 0 perform respawn, 1 request stats, 2 request game rule values.
    ClientCommand = "minecraft:client_command" {
        /// Action id.
        action: i32 as VarInt,
    }
}
packet! {
    /// Active debug subscriptions.
    DebugSubscriptionRequest = "minecraft:debug_subscription_request" {
        /// Ids in `minecraft:debug_subscription`.
        subscriptions: Vec<i32> as List<VarInt>,
    }
}
packet! {
    /// F3+I on an entity.
    EntityTagQuery = "minecraft:entity_tag_query" {
        /// Transaction id.
        transaction_id: i32 as VarInt,
        /// Entity.
        entity_id: i32 as VarInt,
    }
}
packet! {
    /// Jigsaw generate button.
    JigsawGenerate = "minecraft:jigsaw_generate" {
        /// Block.
        pos: BlockPos,
        /// Levels.
        levels: i32 as VarInt,
        /// Keep jigsaws.
        keep_jigsaws: bool,
    }
}
packet! {
    /// Locks the difficulty.
    LockDifficulty = "minecraft:lock_difficulty" {
        /// Locked.
        locked: bool,
    }
}
packet! {
    /// Latency ping; answered with `pong_response`.
    PingRequest = "minecraft:ping_request" {
        /// Payload.
        payload: i64,
    }
}
packet! {
    /// Programs a command block.
    SetCommandBlock = "minecraft:set_command_block" {
        /// Block.
        pos: BlockPos,
        /// Command.
        command: String,
        /// 0 chain, 1 repeating, 2 impulse.
        mode: i32 as VarInt,
        /// 0x01 track output, 0x02 conditional, 0x04 automatic.
        flags: i8,
    }
}
packet! {
    /// Programs a command block minecart.
    SetCommandMinecart = "minecraft:set_command_minecart" {
        /// Minecart.
        entity_id: i32 as VarInt,
        /// Command.
        command: String,
        /// Track output.
        track_output: bool,
    }
}
packet! {
    /// Sets game rules (operators).
    SetGameRule = "minecraft:set_game_rule" {
        /// Rules.
        rules: Vec<GameRuleValue>,
    }
}
packet! {
    /// Programs a jigsaw block.
    SetJigsawBlock = "minecraft:set_jigsaw_block" {
        /// Block.
        pos: BlockPos,
        /// Name.
        name: Identifier,
        /// Target.
        target: Identifier,
        /// Pool.
        pool: Identifier,
        /// Final state.
        final_state: String,
        /// `rollable` or `aligned`.
        joint_type: String,
        /// Selection priority.
        selection_priority: i32 as VarInt,
        /// Placement priority.
        placement_priority: i32 as VarInt,
    }
}
packet! {
    /// Programs a structure block.
    SetStructureBlock = "minecraft:set_structure_block" {
        /// Block.
        pos: BlockPos,
        /// Action.
        action: i32 as VarInt,
        /// 0 save, 1 load, 2 corner, 3 data.
        mode: i32 as VarInt,
        /// Structure name.
        name: String,
        /// Offset x.
        offset_x: i8,
        /// Offset y.
        offset_y: i8,
        /// Offset z.
        offset_z: i8,
        /// Size x.
        size_x: i8,
        /// Size y.
        size_y: i8,
        /// Size z.
        size_z: i8,
        /// Mirror.
        mirror: i32 as VarInt,
        /// Rotation.
        rotation: i32 as VarInt,
        /// Metadata.
        metadata: String,
        /// Integrity 0..1.
        integrity: f32,
        /// Seed.
        seed: i64 as VarLong,
        /// 0x01 ignore entities, 0x02 show air, 0x04 show bounding box, 0x08 strict.
        flags: i8,
    }
}
packet! {
    /// Updates a test block.
    SetTestBlock = "minecraft:set_test_block" {
        /// Block.
        pos: BlockPos,
        /// 0 start, 1 log, 2 fail, 3 accept.
        mode: i32 as VarInt,
        /// Message.
        message: String,
    }
}
packet! {
    /// Sign text (26.3: the text slot follows the lines).
    SignUpdate = "minecraft:sign_update" {
        /// Sign.
        pos: BlockPos,
        /// Line 1.
        line1: String,
        /// Line 2.
        line2: String,
        /// Line 3.
        line3: String,
        /// Line 4.
        line4: String,
        /// 0 back, 1 front.
        text_slot: i32 as VarInt,
    }
}
packet! {
    /// Test instance block action. Kept raw: the wiki (error message as a
    /// text component) and Pumpkin (as a string) disagree.
    TestInstanceBlockAction = "minecraft:test_instance_block_action" {
        /// Raw body.
        body: Bytes as Rest,
    }
}
