use super::*;

packet! {
    /// Groups the packets up to the next delimiter into one tick.
    BundleDelimiter = "minecraft:bundle_delimiter" {}
}
packet! {
    /// Back to configuration; the client acknowledges.
    StartConfiguration = "minecraft:start_configuration" {}
}
packet! {
    /// Debug subscription value for a block (operators only). The update
    /// data is kept raw.
    DebugBlockValue = "minecraft:debug/block_value" {
        /// Block.
        pos: BlockPos,
        /// `Debug Subscription Update`, raw.
        update: Bytes as Rest,
    }
}

packet! {
    /// Debug subscription value for a chunk (operators only). Raw update.
    DebugChunkValue = "minecraft:debug/chunk_value" {
        /// Chunk.
        pos: ChunkPos as PackedChunkPos,
        /// `Debug Subscription Update`, raw.
        update: Bytes as Rest,
    }
}

packet! {
    /// Debug subscription value for an entity (operators only). Raw update.
    DebugEntityValue = "minecraft:debug/entity_value" {
        /// Entity.
        entity_id: i32 as VarInt,
        /// `Debug Subscription Update`, raw.
        update: Bytes as Rest,
    }
}

packet! {
    /// Debug event (operators only), raw.
    DebugEvent = "minecraft:debug/event" {
        /// `Debug Subscription Event`, raw.
        event: Bytes as Rest,
    }
}

packet! {
    /// Debug sample (tick times) for operators.
    DebugSample = "minecraft:debug_sample" {
        /// Samples.
        sample: Vec<i64>,
        /// 0 tick time.
        sample_type: i32 as VarInt,
    }
}
packet! {
    /// Game test highlight.
    GameTestHighlightPos = "minecraft:game_test_highlight_pos" {
        /// Absolute position.
        absolute: BlockPos,
        /// Relative position.
        relative: BlockPos,
    }
}
packet! {
    /// Answer to `ping_request`.
    PongResponse = "minecraft:pong_response" {
        /// The payload sent.
        payload: i64,
    }
}
packet! {
    /// Test instance block screen status. Kept raw: the wiki (status text,
    /// optional size) and Pumpkin (position, status id, optional message)
    /// disagree.
    TestInstanceBlockStatus = "minecraft:test_instance_block_status" {
        /// Raw body.
        body: Bytes as Rest,
    }
}

packet! {
    /// Tick rate and freeze state.
    TickingState = "minecraft:ticking_state" {
        /// Ticks per second.
        tick_rate: f32,
        /// Frozen.
        frozen: bool,
    }
}

packet! {
    /// Steps a frozen game.
    TickingStep = "minecraft:ticking_step" {
        /// Steps.
        steps: i32 as VarInt,
    }
}
