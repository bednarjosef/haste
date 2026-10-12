//! Trace replay: run the recorded inputs through [`crate::tick`] from the first tick's state and
//! compare every tick's position with the recording. Reports the first tick whose position error
//! exceeds the tolerance and the maximum error over the trace.
//!
//! Trace format: `crates/haste-testkit/FORMATS.md` (WS8, version 1). Parsed here with our own
//! lenient structs so this crate does not depend on haste-testkit. Assumptions about the format
//! that the document does not settle are listed on [`InputAlignment`] and in NOTES.md.
use std::path::Path;

use anyhow::{bail, Context};
use haste_core::input::InputState;
use haste_core::view::{BlockView, CollisionShapes};
use haste_core::{BlockPos, BlockStateId, ChunkPos, DVec3};
use serde::Deserialize;

use crate::{tick, BlockPhysics, PlayerAttributes, PlayerState, Pose};

/// Default divergence tolerance on position (brief: 1e-6).
pub const DEFAULT_TOLERANCE: f64 = 1e-6;

#[derive(Deserialize, Debug, Clone)]
pub struct Trace {
    pub version: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub world: Option<TraceWorld>,
    #[serde(default)]
    pub player: Option<TracePlayer>,
    pub ticks: Vec<TraceTick>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct TraceWorld {
    #[serde(default)]
    pub seed: Option<i64>,
    #[serde(default)]
    pub spawn: Option<[f64; 3]>,
    #[serde(default)]
    pub dimension: Option<String>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct TracePlayer {
    #[serde(default)]
    pub yaw: f32,
    #[serde(default)]
    pub pitch: f32,
    #[serde(default)]
    pub initial_pos: Option<[f64; 3]>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct TraceTick {
    #[serde(default)]
    pub tick: u64,
    pub input: TraceInput,
    pub pos: [f64; 3],
    #[serde(default)]
    pub velocity: Option<[f64; 3]>,
    #[serde(default)]
    pub on_ground: Option<bool>,
    #[serde(default)]
    pub pose: Option<String>,
    /// Per-tick look angles, if the recorder adds them (FORMATS.md v1 only has them in `player`).
    #[serde(default)]
    pub yaw: Option<f32>,
    #[serde(default)]
    pub pitch: Option<f32>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct TraceInput {
    /// Impulse: > 0 forward, < 0 back.
    #[serde(default)]
    pub forward: f32,
    /// Impulse: > 0 left, < 0 right (vanilla's sign convention for strafe; assumption).
    #[serde(default)]
    pub strafe: f32,
    #[serde(default)]
    pub jump: bool,
    #[serde(default)]
    pub sneak: bool,
    #[serde(default)]
    pub sprint: bool,
    #[serde(default)]
    pub yaw: Option<f32>,
    #[serde(default)]
    pub pitch: Option<f32>,
}

/// Which recorded input produced tick `i`'s position. FORMATS.md does not say; `SameTick` assumes
/// `ticks[i].pos` is the position after applying `ticks[i].input` to the state of `ticks[i - 1]`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum InputAlignment {
    #[default]
    SameTick,
    PreviousTick,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Divergence {
    pub tick: u64,
    pub expected: DVec3,
    pub actual: DVec3,
    pub error: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReplayReport {
    pub name: String,
    pub ticks: usize,
    pub first_divergence: Option<Divergence>,
    /// Maximum position error (Euclidean) over all ticks, and the tick where it happened.
    pub max_error: f64,
    pub max_error_tick: u64,
    /// Maximum velocity error over ticks that recorded a velocity.
    pub max_velocity_error: f64,
    pub on_ground_mismatches: usize,
    pub pose_mismatches: usize,
}

impl std::fmt::Display for ReplayReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {} ticks, ", self.name, self.ticks)?;
        match &self.first_divergence {
            Some(d) => write!(
                f,
                "first divergence at tick {} (expected {:?}, got {:?}, error {:.3e})",
                d.tick, d.expected, d.actual, d.error
            )?,
            None => write!(f, "no divergence")?,
        }
        write!(
            f,
            ", max error {:.3e} at tick {}, max velocity error {:.3e}, on_ground mismatches {}, pose mismatches {}",
            self.max_error, self.max_error_tick, self.max_velocity_error, self.on_ground_mismatches, self.pose_mismatches
        )
    }
}

pub fn load_trace(path: &Path) -> anyhow::Result<Trace> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let mut t: Trace = serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))?;
    if t.version != 1 {
        bail!("{}: trace version {} not supported", path.display(), t.version);
    }
    if t.name.is_empty() {
        t.name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    }
    Ok(t)
}

fn v3(a: [f64; 3]) -> DVec3 {
    DVec3::new(a[0], a[1], a[2])
}

fn input_of(trace: &Trace, t: &TraceTick) -> InputState {
    let head = trace.player.clone().unwrap_or_default();
    InputState {
        forward: t.input.forward > 0.0,
        back: t.input.forward < 0.0,
        left: t.input.strafe > 0.0,
        right: t.input.strafe < 0.0,
        jump: t.input.jump,
        sneak: t.input.sneak,
        sprint: t.input.sprint,
        yaw: t.input.yaw.or(t.yaw).unwrap_or(head.yaw),
        pitch: t.input.pitch.or(t.pitch).unwrap_or(head.pitch),
    }
}

/// Replays `trace` and reports divergence. The initial state is taken from the first tick
/// (position, velocity, on_ground, pose, look angles).
pub fn replay(
    trace: &Trace,
    attrs: &PlayerAttributes,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    blocks: &dyn BlockPhysics,
    tolerance: f64,
    alignment: InputAlignment,
) -> ReplayReport {
    let mut report = ReplayReport {
        name: trace.name.clone(),
        ticks: trace.ticks.len(),
        first_divergence: None,
        max_error: 0.0,
        max_error_tick: 0,
        max_velocity_error: 0.0,
        on_ground_mismatches: 0,
        pose_mismatches: 0,
    };
    let Some(first) = trace.ticks.first() else { return report };
    let first_input = input_of(trace, first);
    let mut state = PlayerState {
        pos: v3(first.pos),
        velocity: first.velocity.map(v3).unwrap_or_default(),
        on_ground: first.on_ground.unwrap_or(false),
        pose: first.pose.as_deref().and_then(Pose::from_name).unwrap_or_default(),
        yaw: first_input.yaw,
        pitch: first_input.pitch,
        ..Default::default()
    };
    for i in 1..trace.ticks.len() {
        let rec = &trace.ticks[i];
        let src = match alignment {
            InputAlignment::SameTick => rec,
            InputAlignment::PreviousTick => &trace.ticks[i - 1],
        };
        tick(&mut state, &input_of(trace, src), attrs, world, shapes, blocks);
        let expected = v3(rec.pos);
        let err = (state.pos - expected).length();
        if err > report.max_error {
            report.max_error = err;
            report.max_error_tick = rec.tick;
        }
        if err > tolerance && report.first_divergence.is_none() {
            report.first_divergence = Some(Divergence { tick: rec.tick, expected, actual: state.pos, error: err });
        }
        if let Some(v) = rec.velocity {
            report.max_velocity_error = report.max_velocity_error.max((state.velocity - v3(v)).length());
        }
        if rec.on_ground.is_some_and(|g| g != state.on_ground) {
            report.on_ground_mismatches += 1;
        }
        if rec.pose.as_deref().and_then(Pose::from_name).is_some_and(|p| p != state.pose) {
            report.pose_mismatches += 1;
        }
    }
    report
}

/// Layered flat world (superflat): `layers[i]` fills y = `bottom + i`. Default superflat in 26.3
/// is bedrock, 2 dirt, grass with the top surface at y = -60, which matches the example spawn in
/// FORMATS.md; that the traces are recorded there is an assumption.
pub struct FlatWorld {
    pub bottom: i32,
    pub layers: Vec<BlockStateId>,
    pub min_y: i32,
    pub height: i32,
}

impl BlockView for FlatWorld {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        let i = pos.y - self.bottom;
        if i < 0 {
            return BlockStateId::AIR;
        }
        self.layers.get(i as usize).copied().unwrap_or(BlockStateId::AIR)
    }
    fn is_chunk_loaded(&self, _pos: ChunkPos) -> bool {
        true
    }
    fn sky_light(&self, _pos: BlockPos) -> u8 {
        15
    }
    fn block_light(&self, _pos: BlockPos) -> u8 {
        0
    }
    fn min_y(&self) -> i32 {
        self.min_y
    }
    fn height(&self) -> i32 {
        self.height
    }
}
