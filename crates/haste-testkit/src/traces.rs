//! Physics traces: per-tick recorded player state, and the replayer that validates a
//! `haste-physics`-style tick function against them tick by tick.

use std::path::Path;

use anyhow::{Context, Result};
use haste_core::entity::Pose;
use haste_core::DVec3;
use serde::{Deserialize, Serialize};

const KNOWN_VERSION: u32 = 1;

#[derive(Deserialize)]
struct TraceFile {
    version: u32,
    name: String,
    #[serde(default)]
    world: TraceWorld,
    #[serde(default)]
    player: TracePlayerStart,
    ticks: Vec<TraceTickJson>,
}

#[derive(Deserialize, Default)]
struct TraceWorld {
    #[serde(default)]
    seed: i64,
    #[serde(default = "default_spawn")]
    spawn: [f64; 3],
    #[serde(default = "default_dimension")]
    dimension: String,
}

fn default_spawn() -> [f64; 3] {
    [0.5, 0.0, 0.5]
}

fn default_dimension() -> String {
    "minecraft:overworld".into()
}

#[derive(Deserialize, Default)]
struct TracePlayerStart {
    #[serde(default)]
    yaw: f32,
    #[serde(default)]
    pitch: f32,
    #[serde(default)]
    initial_pos: Option<[f64; 3]>,
}

#[derive(Deserialize)]
struct TraceTickJson {
    tick: u32,
    input: TraceInput,
    pos: [f64; 3],
    velocity: [f64; 3],
    on_ground: bool,
    #[serde(default = "default_pose")]
    pose: String,
    #[serde(default = "default_dimensions")]
    dimensions: [f32; 2],
}

fn default_pose() -> String {
    "standing".into()
}

fn default_dimensions() -> [f32; 2] {
    [0.6, 1.8]
}

/// Input as recorded per tick: impulses in [-1, 1] (1.0 = forward), plus the toggles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TraceInput {
    pub forward: f32,
    pub strafe: f32,
    pub jump: bool,
    pub sneak: bool,
    pub sprint: bool,
}

/// One recorded tick.
#[derive(Clone, Debug)]
pub struct TraceTick {
    pub tick: u32,
    pub input: TraceInput,
    /// Position at the START of the tick (the recorded input was applied to reach `velocity`).
    pub pos: DVec3,
    pub velocity: DVec3,
    pub on_ground: bool,
    pub pose: Pose,
    pub dimensions: (f32, f32),
}

/// A loaded trace.
#[derive(Clone, Debug)]
pub struct Trace {
    pub name: String,
    pub seed: i64,
    pub spawn: DVec3,
    pub dimension: String,
    pub start_yaw: f32,
    pub start_pitch: f32,
    pub initial_pos: Option<DVec3>,
    pub ticks: Vec<TraceTick>,
}

fn pose_from_name(name: &str) -> Pose {
    Pose::from_id(match name {
        "standing" => 0,
        "fall_flying" => 1,
        "sleeping" => 2,
        "swimming" => 3,
        "spin_attack" => 4,
        "crouching" => 5,
        "long_jumping" => 6,
        "dying" => 7,
        "croaking" => 8,
        "using_tongue" => 9,
        "sitting" => 10,
        "roaring" => 11,
        "sniffing" => 12,
        "emerging" => 13,
        "digging" => 14,
        "sliding" => 15,
        "shooting" => 16,
        "inhaling" => 17,
        _ => 0,
    })
    .unwrap_or(Pose::Standing)
}

impl Trace {
    pub fn load(path: &Path) -> Result<Trace> {
        let file: TraceFile = serde_json::from_reader(std::io::BufReader::new(
            std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?,
        ))
        .with_context(|| format!("parsing {}", path.display()))?;
        if file.version != KNOWN_VERSION {
            anyhow::bail!(
                "trace {}: unsupported version {}",
                path.display(),
                file.version
            );
        }
        Ok(Trace {
            name: file.name,
            seed: file.world.seed,
            spawn: DVec3::from(file.world.spawn),
            dimension: file.world.dimension,
            start_yaw: file.player.yaw,
            start_pitch: file.player.pitch,
            initial_pos: file.player.initial_pos.map(DVec3::from),
            ticks: file
                .ticks
                .into_iter()
                .map(|t| TraceTick {
                    tick: t.tick,
                    input: t.input,
                    pos: DVec3::from(t.pos),
                    velocity: DVec3::from(t.velocity),
                    on_ground: t.on_ground,
                    pose: pose_from_name(&t.pose),
                    dimensions: (t.dimensions[0], t.dimensions[1]),
                })
                .collect(),
        })
    }
}

/// Result of replaying a trace against a tick function.
#[derive(Clone, Debug, Serialize)]
pub struct TraceReport {
    pub name: String,
    pub ticks: usize,
    /// First tick where the simulation diverged beyond tolerance, if any.
    pub first_divergence_tick: Option<u32>,
    /// Max per-axis position error over the run.
    pub max_pos_error: f64,
    /// Mean position error over the run.
    pub mean_pos_error: f64,
    /// Ticks whose recorded on_ground the simulation disagreed with.
    pub on_ground_mismatches: u32,
    /// Per-tick position error (for plotting); `errors[i]` corresponds to `ticks[i]`.
    pub errors: Vec<f64>,
    pub pass: bool,
}

/// Replays `trace` through `tick_fn`, comparing simulated positions with the recorded ones.
///
/// `tick_fn(state, input)` receives the current (pos, velocity, on_ground) state plus the tick's
/// recorded input and returns the next state. Comparison is positional with a tolerance in
/// blocks per axis: any tick's error above it is a divergence. Errors are checked cumulatively
/// (position error is not reset on match; vanilla physics is deterministic, so a real
/// divergence only grows).
pub fn replay_trace(
    trace: &Trace,
    mut tick_fn: impl FnMut(&(DVec3, DVec3, bool), &TraceInput) -> (DVec3, DVec3, bool),
    tolerance: f64,
) -> TraceReport {
    // Ticks record pos at the START of the tick (FORMATS.md): applying tick i's input to the
    // state at tick i must land on tick i+1's recorded pos. The last tick's outcome is not in
    // the file, so the comparison window is ticks[0..n-1] against ticks[1..n].
    let mut state = (
        trace.ticks.first().map(|t| t.pos).unwrap_or(trace.spawn),
        DVec3::ZERO,
        true,
    );
    let mut errors = Vec::with_capacity(trace.ticks.len());
    let mut first_divergence_tick = None;
    let mut max_pos_error: f64 = 0.0;
    let mut sum_pos_error: f64 = 0.0;
    let mut on_ground_mismatches = 0u32;

    for i in 0..trace.ticks.len().saturating_sub(1) {
        let t = &trace.ticks[i];
        let expected = &trace.ticks[i + 1];
        state = tick_fn(&state, &t.input);
        let err = (state.0 - expected.pos).abs().max_element();
        errors.push(err);
        max_pos_error = max_pos_error.max(err);
        sum_pos_error += err;
        if state.2 != expected.on_ground {
            on_ground_mismatches += 1;
        }
        if err > tolerance && first_divergence_tick.is_none() {
            first_divergence_tick = Some(t.tick);
        }
    }

    TraceReport {
        name: trace.name.clone(),
        ticks: trace.ticks.len(),
        first_divergence_tick,
        max_pos_error,
        mean_pos_error: if trace.ticks.is_empty() {
            0.0
        } else {
            sum_pos_error / trace.ticks.len() as f64
        },
        on_ground_mismatches,
        pass: first_divergence_tick.is_none(),
        errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_trace(dir: &std::path::Path) -> std::path::PathBuf {
        let path = dir.join("traces").join("test-walk.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{
  "version": 1, "name": "test-walk",
  "world": { "seed": 42, "spawn": [0.5, -60.0, 0.5] },
  "player": { "yaw": 0.0, "pitch": 0.0, "initial_pos": [0.5, -60.0, 0.5] },
  "ticks": [
    { "tick": 0, "input": {"forward": 1.0, "strafe": 0.0, "jump": false, "sneak": false, "sprint": false},
      "pos": [0.5, -60.0, 0.5], "velocity": [0.0, 0.0, 0.0], "on_ground": true, "pose": "standing", "dimensions": [0.6, 1.8] },
    { "tick": 1, "input": {"forward": 1.0, "strafe": 0.0, "jump": false, "sneak": false, "sprint": false},
      "pos": [0.5, -60.0, 0.55], "velocity": [0.0, 0.0, 0.05], "on_ground": true, "pose": "standing", "dimensions": [0.6, 1.8] },
    { "tick": 2, "input": {"forward": 0.0, "strafe": 0.0, "jump": true, "sneak": false, "sprint": false},
      "pos": [0.5, -60.0, 0.61], "velocity": [0.0, 0.0, 0.06], "on_ground": true, "pose": "standing", "dimensions": [0.6, 1.8] },
    { "tick": 3, "input": {"forward": 0.0, "strafe": 0.0, "jump": false, "sneak": false, "sprint": false},
      "pos": [0.5, -59.9, 0.64], "velocity": [0.0, 0.1, 0.03], "on_ground": false, "pose": "standing", "dimensions": [0.6, 1.8] }
  ]
}"#,
        )
        .unwrap();
        path
    }

    #[test]
    fn round_trips_a_trace() {
        let tmp = tempdir();
        let path = write_trace(&tmp);
        let trace = Trace::load(&path).unwrap();
        assert_eq!(trace.name, "test-walk");
        assert_eq!(trace.seed, 42);
        assert_eq!(trace.ticks.len(), 4);
        assert_eq!(trace.ticks[0].pos, DVec3::new(0.5, -60.0, 0.5));
        assert!(trace.ticks[2].input.jump);
        assert_eq!(trace.ticks[3].pose, Pose::Standing);
        assert_eq!(trace.ticks[3].dimensions, (0.6, 1.8));
    }

    #[test]
    fn replay_matches_exactly_when_tick_fn_is_recorded_motion() {
        let tmp = tempdir();
        let trace = Trace::load(&write_trace(&tmp)).unwrap();
        // A tick function that lands exactly on the next tick's recorded state.
        let mut idx = 0usize;
        let report = replay_trace(
            &trace,
            |_state, _input| {
                idx += 1;
                let t = &trace.ticks[idx.min(trace.ticks.len() - 1)];
                (t.pos, t.velocity, t.on_ground)
            },
            1e-9,
        );
        assert!(report.pass, "expected no divergence: {report:?}");
        assert_eq!(report.first_divergence_tick, None);
        assert_eq!(report.on_ground_mismatches, 0);
        assert!(report.max_pos_error < 1e-9);
    }

    #[test]
    fn replay_reports_first_divergence() {
        let tmp = tempdir();
        let trace = Trace::load(&write_trace(&tmp)).unwrap();
        // Drifts by 0.1 blocks per tick from the recorded positions.
        let mut drift = 0.0;
        let report = replay_trace(
            &trace,
            |state, _input| {
                drift += 0.1;
                (state.0 + DVec3::new(0.0, 0.0, drift), state.1, state.2)
            },
            0.05,
        );
        assert!(!report.pass);
        // tick 0's error lands exactly at tolerance (0.05, not above); tick 1 exceeds it.
        assert_eq!(report.first_divergence_tick, Some(1));
        assert!(report.max_pos_error > 0.05);
    }

    fn tempdir() -> std::path::PathBuf {
        // Tests run as parallel threads of one process, so the pid alone is not unique.
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("haste-testkit-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
