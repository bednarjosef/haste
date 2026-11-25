//! Oracle data formats, trace replay, screenshot diffing and benchmark harness. Owner: oracles.
//!
//! See docs/INTERFACES.md for the contract this crate implements and docs/AGENTS.md for the rules.
//! The on-disk formats are documented in [FORMATS.md](../FORMATS.md); everything here loads,
//! replays or compares that data. Tests that need `cache/` are `#[ignore]`d with a message.

pub mod bench;
pub mod oracles;
pub mod packets;
pub mod proxy;
pub mod report;
pub mod screenshots;
pub mod traces;

pub use bench::{Bench, BenchSample, BenchWriter};
pub use oracles::{
    BlockProperties, BlockShapeState, BlockShapes, EntityDimensions, EntityTypeDimensions,
    PlayerPoseDimensions,
};
pub use packets::PacketRecording;
pub use report::Markdown;
pub use screenshots::{compare_screenshot, DiffResult, ScreenshotMeta};
pub use traces::{replay_trace, Trace, TraceReport, TraceTick};
