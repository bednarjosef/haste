//! The client-spawned block particles (block breaking and mining cracks), the glue
//! between the packets/predictions that cause them and the renderer's pool.
//!
//! Vanilla spawns these itself (they never arrive as `level_particles`):
//! - our own break: the prediction path `MultiPlayerGameMode.destroyBlock` ->
//!   `Block.playerWillDestroy` plays `levelEvent(2001)` on the client, whose
//!   `addDestroyBlockEffect` dissolves the block into the terrain-particle grid
//!   (the server's 2001 broadcast excludes the breaker, so ours must be local);
//! - other players' breaks: the server's `level_event` 2001 (`LevelEventHandler`
//!   case 2001 -> `addDestroyBlockEffect`);
//! - mining cracks: one particle per tick on the hit face. Our own start tick is
//!   client-side (`startDestroyBlock` -> `addBreakingBlockEffects`); every tick
//!   after that comes from the server, which broadcasts `level_event` 2020 (every
//!   4th tick, with the hit sound) or 2019 (the rest, silent) for anyone mining
//!   (`ServerPlayerGameMode.tick` -> `LevelEventHandler` cases 2019/2020).
//!
//! The game thread queues requests here (packet decode and prediction stay free of
//! renderer types) and applies them to the pool in its tick, where the world, the
//! block shapes and the biome source are at hand.

use haste_core::{BlockPos, BlockStateId, Direction};

/// A block particle the client spawns itself, queued for the game thread's tick.
#[derive(Clone, Copy, Debug)]
pub enum BlockParticleRequest {
    /// A block broke: the terrain-particle grid (`addDestroyBlockEffect`).
    Destroy {
        pos: BlockPos,
        /// The state before it broke: the 2001 payload (`Block.stateById(data)`).
        state: BlockStateId,
    },
    /// A mining tick on a block face (`addBreakingParticles`). The block state is
    /// read from the world when the request is applied (vanilla's event handler
    /// reads it at the same point in the tick).
    Crack {
        pos: BlockPos,
        /// The face hit (`Direction.values()[data]`, the packet's payload).
        face: Direction,
    },
}

/// `LevelEventHandler.levelEvent`'s block-particle cases; `None` for every other
/// event id (only the particle-spawning events are decoded here; the sounds come
/// as their own packets). `data` is the block state id for 2001 and the direction
/// ordinal for 2019/2020.
pub fn request_from_level_event(
    event: i32,
    pos: BlockPos,
    data: i32,
) -> Option<BlockParticleRequest> {
    match event {
        // 2001: a block broke. The payload names the broken state (validated
        // against the state table when the request is applied).
        2001 => Some(BlockParticleRequest::Destroy {
            pos,
            state: BlockStateId(data as u32),
        }),
        // 2019/2020: a mining tick on `pos`, the payload the face's ordinal
        // (`Mth.clamp(data, 0, 5)` like the event handler's direction lookup).
        2019 | 2020 => Some(BlockParticleRequest::Crack {
            pos,
            face: Direction::from_ordinal(data.clamp(0, 5) as u8)?,
        }),
        _ => None,
    }
}
