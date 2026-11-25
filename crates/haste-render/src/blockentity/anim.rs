// The object-model block entities' animation state (bell shake, pot wobble, enchanting table
// book, conduit spin, moving piston), as the game thread's block entity tickers leave it.

use haste_core::Direction;

use crate::entity::TickPair;

/// The per-instance animation state of the object-model block entities, as vanilla's block
/// entity tickers leave it at the ends of the last tick (the partial tick lerps the two ends).
/// Build it in the client's tick (like [`crate::TickPair`] for the chest lids).
#[derive(Clone, Copy, Debug)]
pub enum BlockEntityAnim {
    /// The enchanting table's book (vanilla `EnchantingTableBlockEntity.bookAnimationTick`):
    /// `time` is the tick counter, the three pairs are `flip`, `open` and `rot`, and the last
    /// three are the tick's own bookkeeping (`tRot`, `flipT` and `flipA`).
    EnchantTable {
        time: f32,
        flip: TickPair,
        open: TickPair,
        rot: TickPair,
        t_rot: f32,
        flip_t: f32,
        flip_a: f32,
    },
    /// A bell that was rung (vanilla `BellBlockEntity.triggerEvent` then `BellBlockEntity.tick`):
    /// the shake ticks and the direction the bell was hit from (`clickDirection`).
    Bell {
        ticks: TickPair,
        click: Option<Direction>,
    },
    /// A wobbling decorated pot (vanilla `DecoratedPotBlockEntity.triggerEvent`): ticks since
    /// the wobble started, at the ends of the last tick, and the wobble style's duration
    /// (`WobbleStyle.duration`, 7 for POSITIVE and 10 for NEGATIVE) the progress is measured in.
    Pot { wobble: TickPair, duration: u32 },
    /// A conduit (vanilla `ConduitBlockEntity.clientTick`): the tick counter, the spin counter
    /// (vanilla's `activeRotation`, which the renderer names in degrees for the shell and in
    /// radians for the cage), and the shape's active/hunting flags.
    Conduit {
        tick_count: f32,
        rotation: f32,
        active: bool,
        hunting: bool,
    },
    /// A moving piston (vanilla `PistonMovingBlockEntity.tick`): the extension progress, which
    /// advances 0.5 per tick to 1.0 and then the block entity is gone (the server replaces the
    /// moving_piston block with the moved one).
    Piston { progress: TickPair },
}

impl BlockEntityAnim {
    /// The same animation at `partial` through the tick, vanilla's `Mth.lerp(partialTicks, oX,
    /// X)` per field: the counters the renderers read (`BellBlockEntity.ticks` +
    /// `partialTicks`, `DecoratedPotBlockEntity.wobbleStartedAtTick` + `partialTicks`, the
    /// enchanting table's `flip`/`open`/`rot`). The game thread keeps the two ends; the render
    /// thread hands this the partial tick it already has.
    pub fn at(self, partial: f32) -> BlockEntityAnim {
        match self {
            BlockEntityAnim::EnchantTable {
                time,
                flip,
                open,
                rot,
                t_rot,
                flip_t,
                flip_a,
            } => BlockEntityAnim::EnchantTable {
                time: time + partial,
                flip: TickPair {
                    start: flip.start,
                    end: flip.at(partial),
                },
                open: TickPair {
                    start: open.start,
                    end: open.at(partial),
                },
                // Vanilla `extractRenderState`: `oRot` plus the difference wrapped into
                // [-PI, PI) times the partial tick (`rot` wraps at the start of the tick, so
                // a plain lerp would spin the book the long way round).
                rot: TickPair {
                    start: rot.start,
                    end: rot.start + wrap_pi(rot.end - rot.start) * partial,
                },
                t_rot,
                flip_t,
                flip_a,
            },
            BlockEntityAnim::Bell { ticks, click } => BlockEntityAnim::Bell {
                ticks: TickPair {
                    start: ticks.start,
                    end: ticks.at(partial),
                },
                click,
            },
            BlockEntityAnim::Pot { wobble, duration } => BlockEntityAnim::Pot {
                wobble: TickPair {
                    start: wobble.start,
                    end: wobble.at(partial),
                },
                duration,
            },
            // Vanilla's `ConduitRenderState`: `animTime` is `tickCount + partialTicks` and
            // `getActiveRotation(partialTicks)` adds the partial tick to the spin counter; the
            // animation phase stays on the integer tick count.
            BlockEntityAnim::Conduit {
                tick_count,
                rotation,
                active,
                hunting,
            } => BlockEntityAnim::Conduit {
                tick_count: tick_count + partial,
                rotation: rotation + partial,
                active,
                hunting,
            },
            BlockEntityAnim::Piston { progress } => BlockEntityAnim::Piston {
                progress: TickPair {
                    start: progress.start,
                    end: progress.at(partial),
                },
            },
        }
    }
}

/// Vanilla's wrap of an angle into [-PI, PI) (the `while` loops of
/// `EnchantingTableBlockEntity.bookAnimationTick` and `EnchantTableRenderer.extractRenderState`).
pub fn wrap_pi(mut v: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    while v >= PI {
        v -= TAU;
    }
    while v < -PI {
        v += TAU;
    }
    v
}

#[cfg(test)]
mod tests;
