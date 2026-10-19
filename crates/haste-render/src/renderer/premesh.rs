//! Turn prediction for the mesh gate (`--premesh`): pre-mesh the sections the camera is
//! about to see, the way a CPU's branch predictor runs ahead of a branch. With
//! visible-only meshing (vanilla `LevelExtractor`: a dirty section compiles only once it
//! is in `visibleSections`) a quick turn shows pop-in — the sections entering the view
//! mesh a few frames late; vanilla has the same pop-in. A right guess removes it, a wrong
//! one only meshes a section early (the built mesh is kept either way).
//!
//! `cone` widens the gate's frustum toward the turn in proportion to the smoothed angular
//! velocity, and forward along the movement direction for fast movement; `predict` drives
//! the widening with a small online linear model of the next half-second of yaw. `off` is
//! exactly the visible-only gate.

use std::time::Instant;

use haste_core::SectionPos;

use super::{Camera, Renderer, ViewBob};

#[cfg(test)]
mod tests;

/// The pre-mesh mode (`--premesh`; the config's `[options] premesh`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PremeshMode {
    /// Visible-only meshing, exactly vanilla's `LevelExtractor` gate.
    #[default]
    Off,
    /// Widen the mesh gate's frustum toward the turn (never the draw frustum).
    Cone,
    /// `cone`, with the widening driven by an online predictor of the next half-second
    /// of yaw.
    Predict,
}

/// How far ahead the widened frustum looks (seconds): about how long a submitted section
/// takes to appear (snapshot, mesh, upload) — the pop-in the prediction must cover.
const HORIZON: f32 = 0.5;
/// Velocity smoothing time constant (seconds): a few frames, so one frame hitch neither
/// spikes nor hides a real turn.
const VELOCITY_TAU: f32 = 0.08;
/// The widening is capped so a wild guess cannot pull the whole view distance in.
const MAX_YAW_WIDEN: f32 = 90.0;
const MAX_PITCH_WIDEN: f32 = 45.0;
/// Below these the camera counts as still and nothing is pre-meshed (an idle camera
/// must behave exactly like `off`).
const MIN_YAW_WIDEN: f32 = 2.0;
const MIN_SPEED: f32 = 2.0;
/// A pre-meshed section's verdict window: it became visible (a hit) within this many
/// seconds, else it was waste.
const HIT_WINDOW: f32 = 2.0;
/// The predictor's features are the angular velocity and acceleration scaled to O(1)
/// (`yaw_vel / FEATURE_VEL_SCALE`, `accel / FEATURE_ACCEL_SCALE`), so one learning rate
/// suits every turn speed; the weights' seed matches cone's rule
/// (`weights[1] * vel = vel * HORIZON`).
const FEATURE_VEL_SCALE: f32 = 100.0;
const FEATURE_ACCEL_SCALE: f32 = 1000.0;
/// The least-mean-squares step (stable for the normalized features' magnitudes).
const TRAIN_RATE: f32 = 2e-3;
/// The weight clamps: the bias in degrees, the velocity gain in a band around cone's
/// rule, the acceleration gain small.
const WEIGHT_BIAS_CLAMP: f32 = 30.0;
const WEIGHT_VEL_CLAMP: f32 = 2.0 * HORIZON * FEATURE_VEL_SCALE;
const WEIGHT_ACCEL_CLAMP: f32 = 10.0;

/// Wraps a yaw difference into -180..180 (yaw is periodic).
fn wrap_deg(d: f32) -> f32 {
    let d = d % 360.0;
    if d > 180.0 {
        d - 360.0
    } else if d < -180.0 {
        d + 360.0
    } else {
        d
    }
}

/// One sample of the predictor's delay line: when it was taken, the yaw then, and the
/// features the model saw (constant, smoothed yaw velocity, smoothed yaw acceleration).
#[derive(Clone, Copy)]
struct Sample {
    yaw: f32,
    features: [f32; 3],
    t: Instant,
}

/// The renderer's pre-mesh state: the mode, the smoothed camera motion, the widened
/// frustum of the current frame, and (when the bench tracks pop-in) the metrics.
pub(super) struct Premesh {
    mode: PremeshMode,
    /// Smoothed angular velocities (deg/s) and camera velocity (blocks/s).
    yaw_vel: f32,
    pitch_vel: f32,
    /// The previous frame's sample (time, yaw, pitch, position).
    last: Option<(Instant, f32, f32, glam::DVec3)>,
    /// The latest predictor features (constant, smoothed yaw velocity, smoothed yaw
    /// acceleration): what the widening and the delay line both read.
    features: [f32; 3],
    vel: glam::DVec3,
    /// The previous frame's smoothed yaw velocity (for the acceleration feature).
    prev_yaw_vel: f32,
    /// The online predictor (predict mode): weights over the sample features, seeded
    /// with cone's rule (yaw delta = velocity * horizon) and updated by a least-mean-
    /// squares step once a sample's realized yaw delta is known.
    weights: [f32; 3],
    /// The delay line of past samples (ring, reused; no allocation per frame).
    ring: [Option<Sample>; 64],
    ring_next: usize,
    /// This frame's widened frustum and the position its tests are relative to
    /// (`None`: the camera is still, nothing to pre-mesh).
    frustum: Option<([glam::Vec4; 6], glam::DVec3)>,
    /// Pop-in tracking (the bench turns it on): positions that were meshed (a mesh with
    /// no vertices leaves no resident entry, and must not count as unmeshed forever),
    /// and the columns the feed has reported sections for (loaded chunks — the walk
    /// enters the unloaded ring of its grid as see-through, and those sections can
    /// never have a mesh, so they are not pop-in).
    tracking: bool,
    mesh_done: ahash::HashSet<SectionPos>,
    noted: ahash::HashSet<haste_core::ChunkPos>,
    /// Pre-meshed sections awaiting their verdict (position, when pre-meshed), and the
    /// counters behind `premesh_summary`. `keep` is the aging pass's spare list (both
    /// lists keep their capacity; no allocation per frame).
    premeshed: Vec<(SectionPos, Instant)>,
    keep: Vec<(SectionPos, Instant)>,
    premeshed_total: u32,
    hits: u32,
    wasted: u32,
    /// The last frame's visible-but-unmeshed section count (pop-in).
    pub(super) visible_unmeshed: u32,
}

impl Premesh {
    pub(super) fn new() -> Self {
        Premesh {
            mode: PremeshMode::Off,
            yaw_vel: 0.0,
            pitch_vel: 0.0,
            last: None,
            features: [1.0, 0.0, 0.0],
            vel: glam::DVec3::ZERO,
            prev_yaw_vel: 0.0,
            weights: [0.0, HORIZON * FEATURE_VEL_SCALE, 0.0],
            ring: [None; 64],
            ring_next: 0,
            frustum: None,
            tracking: false,
            mesh_done: ahash::HashSet::default(),
            noted: ahash::HashSet::default(),
            premeshed: Vec::new(),
            keep: Vec::new(),
            premeshed_total: 0,
            hits: 0,
            wasted: 0,
            visible_unmeshed: 0,
        }
    }

    /// Feeds the frame's camera (from `update_mesh_gate`, render thread, once per
    /// rendered frame) and recomputes the widened frustum.
    fn update(&mut self, camera: &Camera, aspect: f32, now: Instant) {
        if let Some((t, yaw, pitch, pos)) = self.last {
            if t < now {
                let dt = (now - t).as_secs_f32().clamp(0.001, 0.25);
                let a = 1.0 - (-dt / VELOCITY_TAU).exp();
                self.yaw_vel += (wrap_deg(camera.yaw - yaw) / dt - self.yaw_vel) * a;
                self.pitch_vel += ((camera.pitch - pitch) / dt - self.pitch_vel) * a;
                self.vel += ((camera.pos - pos) / dt as f64 - self.vel) * a as f64;
                let accel = ((self.yaw_vel - self.prev_yaw_vel) / dt)
                    .clamp(-20.0 * FEATURE_ACCEL_SCALE, 20.0 * FEATURE_ACCEL_SCALE);
                self.prev_yaw_vel = self.yaw_vel;
                self.features = [
                    1.0,
                    self.yaw_vel / FEATURE_VEL_SCALE,
                    accel / FEATURE_ACCEL_SCALE,
                ];
                if self.mode == PremeshMode::Predict {
                    self.record_sample(camera.yaw, self.features, now);
                    self.train(now, camera.yaw);
                }
            }
        }
        self.last = Some((now, camera.yaw, camera.pitch, camera.pos));
        self.frustum = None;
        if self.mode == PremeshMode::Off {
            return;
        }
        // The widening: cone extrapolates the smoothed velocity, predict asks the model.
        let yaw_widen = match self.mode {
            PremeshMode::Predict => (self.weights[0] * self.features[0]
                + self.weights[1] * self.features[1]
                + self.weights[2] * self.features[2])
                .clamp(-MAX_YAW_WIDEN, MAX_YAW_WIDEN),
            _ => (self.yaw_vel * HORIZON).clamp(-MAX_YAW_WIDEN, MAX_YAW_WIDEN),
        };
        let pitch_widen = (self.pitch_vel * HORIZON).clamp(-MAX_PITCH_WIDEN, MAX_PITCH_WIDEN);
        let speed = self.vel.length() as f32;
        if yaw_widen.abs() < MIN_YAW_WIDEN && pitch_widen.abs() < MIN_YAW_WIDEN && speed < MIN_SPEED
        {
            // Still camera: nothing to pre-mesh, exactly the `off` behaviour.
            return;
        }
        // The widened gate frustum: this frame's camera turned and pitched ahead, at the
        // origin (camera-relative planes, like the gate's own frustum). The frustum the
        // frame draws with is never touched.
        let ahead = Camera {
            pos: glam::DVec3::ZERO,
            yaw: camera.yaw + yaw_widen,
            pitch: (camera.pitch + pitch_widen).clamp(-89.9, 89.9),
            fov_deg: camera.fov_deg,
            hand_fov_deg: camera.hand_fov_deg,
            roll_deg: camera.roll_deg,
            bob: ViewBob::default(),
        };
        let planes = Renderer::frustum_planes(&Renderer::view_projection(&ahead, aspect, None));
        // Fast movement (sprint, flight, elytra): the cone also reaches forward along
        // the movement direction, so its tests run relative to the predicted position.
        let origin = if speed >= MIN_SPEED {
            camera.pos + self.vel * HORIZON as f64
        } else {
            camera.pos
        };
        self.frustum = Some((planes, origin));
    }

    /// Stores a sample in the delay line (ring, reused).
    fn record_sample(&mut self, yaw: f32, features: [f32; 3], t: Instant) {
        self.ring[self.ring_next] = Some(Sample { yaw, features, t });
        self.ring_next = (self.ring_next + 1) % self.ring.len();
    }

    /// Trains the predictor (predict mode): once a sample is `HORIZON` old, its realized
    /// yaw delta is the target and the weights take a least-mean-squares step toward it.
    fn train(&mut self, now: Instant, yaw: f32) {
        let mut found = None;
        for slot in self.ring.iter().flatten() {
            let age = now.saturating_duration_since(slot.t).as_secs_f32();
            if age > 0.0 && (age - HORIZON).abs() <= 0.02 {
                found = Some(*slot);
                break;
            }
        }
        let Some(sample) = found else {
            return;
        };
        // The realized yaw delta over the sample's lifetime, clamped to the widening
        // window (a turn beyond it is capped anyway; wrapping it instead would alias a
        // long turn into the opposite direction and poison the model).
        let realized = (yaw - sample.yaw).clamp(-MAX_YAW_WIDEN, MAX_YAW_WIDEN);
        let predicted = self.weights[0] * sample.features[0]
            + self.weights[1] * sample.features[1]
            + self.weights[2] * sample.features[2];
        let err = realized - predicted;
        for (w, f) in self.weights.iter_mut().zip(sample.features) {
            *w += TRAIN_RATE * err * f;
        }
        // Keep the model sane: the bias and the acceleration term stay small, the
        // velocity gain stays in a band around cone's rule.
        self.weights[0] = self.weights[0].clamp(-WEIGHT_BIAS_CLAMP, WEIGHT_BIAS_CLAMP);
        self.weights[1] = self.weights[1].clamp(0.0, WEIGHT_VEL_CLAMP);
        self.weights[2] = self.weights[2].clamp(-WEIGHT_ACCEL_CLAMP, WEIGHT_ACCEL_CLAMP);
    }

    /// Records a meshed position (including a mesh that produced no vertices) for the
    /// pop-in count. Only filled while tracking.
    pub(super) fn note_mesh_done(&mut self, pos: SectionPos) {
        if self.tracking {
            self.mesh_done.insert(pos);
        }
    }

    /// Records whether a column's chunk is loaded (the feed's section-state notes):
    /// the pop-in count only counts sections that could have a mesh.
    pub(super) fn note_column(&mut self, column: haste_core::ChunkPos, loaded: bool) {
        if !self.tracking {
            return;
        }
        if loaded {
            self.noted.insert(column);
        } else {
            self.noted.remove(&column);
        }
    }

    /// The feed tells the renderer a section was submitted for pre-meshing (not because
    /// it was visible): it now waits to become visible within [`HIT_WINDOW`].
    pub(super) fn note_premeshed(&mut self, pos: SectionPos) {
        self.premeshed.push((pos, Instant::now()));
        self.premeshed_total += 1;
    }

    /// Ages one batch of tracked pre-meshed sections against the visible gate: one the
    /// gate can see is a hit, one older than [`HIT_WINDOW`] without ever becoming
    /// visible is waste; the survivors land in `keep` (both lists keep their capacity).
    /// A free function so the render thread can hand it a closure over the renderer.
    fn age_premeshed(
        now: Instant,
        visible: &dyn Fn(SectionPos) -> bool,
        pending: &mut Vec<(SectionPos, Instant)>,
        keep: &mut Vec<(SectionPos, Instant)>,
    ) -> (u32, u32) {
        let mut hits = 0;
        let mut wasted = 0;
        for (pos, at) in pending.drain(..) {
            if visible(pos) {
                hits += 1;
            } else if now.saturating_duration_since(at).as_secs_f32() > HIT_WINDOW {
                wasted += 1;
            } else {
                keep.push((pos, at));
            }
        }
        (hits, wasted)
    }

    /// (pre-meshed total, hits, waste) for the bench's summary.
    pub(super) fn summary(&self) -> (u32, u32, u32) {
        (self.premeshed_total, self.hits, self.wasted)
    }

    /// Drops the tracked positions with the world they belong to (the run's counters
    /// stay): `Renderer::clear_world`.
    pub(super) fn world_cleared(&mut self) {
        self.mesh_done.clear();
        self.noted.clear();
        self.premeshed.clear();
        self.frustum = None;
    }
}

impl Renderer {
    /// The pre-mesh mode (`--premesh` / the config's `[options] premesh`): which dirty
    /// sections the gate lets through ahead of the view. `off` is exactly the
    /// visible-only behaviour, so both can be benchmarked from one binary.
    pub fn set_premesh(&mut self, mode: PremeshMode) {
        self.premesh.mode = mode;
        self.premesh.frustum = None;
    }

    /// The pre-mesh mode in use.
    pub fn premesh_mode(&self) -> PremeshMode {
        self.premesh.mode
    }

    /// Whether pre-meshing is on at all (the feed skips its scan entirely when off).
    pub fn premesh_enabled(&self) -> bool {
        self.premesh.mode != PremeshMode::Off
    }

    /// Whether this frame's widened cone exists (the camera is turning or moving fast):
    /// while it does not, the feed's pre-mesh scan is skipped, so a still camera costs
    /// nothing.
    pub fn premesh_active(&self) -> bool {
        self.premesh.frustum.is_some()
    }

    /// Turns pop-in tracking on or off (the bench turns it on): counts the frame's
    /// visible-but-unmeshed sections and the pre-mesh hit rate. Sections already meshed
    /// when it turns on are seeded, so the count starts from zero.
    pub fn set_popin_tracking(&mut self, on: bool) {
        if on == self.premesh.tracking {
            return;
        }
        self.premesh.tracking = on;
        if on {
            self.premesh.mesh_done.clear();
            for s in self.sections.iter() {
                self.premesh.mesh_done.insert(s.pos);
            }
        }
    }

    /// The last frame's visible-but-unmeshed section count (pop-in; 0 unless
    /// `set_popin_tracking(true)`).
    pub fn visible_unmeshed(&self) -> u32 {
        self.premesh.visible_unmeshed
    }

    /// The pre-mesh counters for the bench's summary: (pre-meshed, hits, waste).
    pub fn premesh_summary(&self) -> (u32, u32, u32) {
        self.premesh.summary()
    }

    /// The feed reports a section submitted for pre-meshing (not visible yet).
    pub fn note_premeshed(&mut self, pos: SectionPos) {
        self.premesh.note_premeshed(pos);
    }

    /// Whether a held-back dirty section may be pre-meshed this frame: it must be in the
    /// occlusion walk's set (the cave-cull half of the gate, same as `wants_section`)
    /// and inside the widened cone. Never true while the gate already wants the section
    /// or the camera is still.
    pub fn wants_premesh(&self, pos: SectionPos) -> bool {
        if !self.premesh_enabled() {
            return false;
        }
        if self.cull_sections {
            let si = pos.y - self.min_section;
            let in_mask = (0..32).contains(&si)
                && self
                    .visible_mask
                    .column(haste_core::ChunkPos::new(pos.x, pos.z))
                    & (1u32 << si)
                    != 0;
            let in_bonus = self.sections.get(pos).is_some_and(|s| s.bonus);
            if !in_mask && !in_bonus {
                return false;
            }
        }
        let Some((planes, cam)) = &self.premesh.frustum else {
            return false;
        };
        let lo = [
            ((pos.x << 4) as f64 - cam.x - 8.0) as f32,
            ((pos.y << 4) as f64 - cam.y - 8.0) as f32,
            ((pos.z << 4) as f64 - cam.z - 8.0) as f32,
        ];
        let hi = [
            ((pos.x << 4) as f64 - cam.x + 24.0) as f32,
            ((pos.y << 4) as f64 - cam.y + 24.0) as f32,
            ((pos.z << 4) as f64 - cam.z + 24.0) as f32,
        ];
        Self::aabb_visible(planes, lo, hi)
    }

    /// One rendered frame of the pre-mesh state, called from `update_mesh_gate` after
    /// this frame's gate frustum was recorded: the pop-in count over the frame's visible
    /// set, the aging of tracked pre-meshed sections against the same gate, and the
    /// turn/motion sample that widens the cone.
    pub(super) fn update_premesh(&mut self, camera: &Camera, aspect: f32) {
        // The pop-in count: sections this frame's gate wants (the occlusion walk's set
        // intersected with the gate frustum) that have no mesh — the same test
        // `wants_section` runs, plus the mesh check.
        let mut count = 0u32;
        if self.premesh.tracking && self.cull_sections {
            if let Some((planes, cam)) = &self.want_frustum {
                let tracking = &self.premesh;
                // Empty sections (the walk's see-through data) sit in the mask but draw
                // nothing — vanilla never compiles them either — so they are not pop-in.
                let empty = self.known_empty.lock();
                for (c, si) in self.visible_mask.ordered() {
                    if !tracking.noted.contains(&c) {
                        // The unloaded ring of the walk's grid: nothing could mesh.
                        continue;
                    }
                    let pos = SectionPos::new(c.x, si + self.min_section, c.z);
                    if si < 32 && empty.get(&c).is_some_and(|bits| bits & (1u32 << si) != 0) {
                        continue;
                    }
                    if self.sections.get(pos).is_none() && !tracking.mesh_done.contains(&pos) {
                        let lo = [
                            ((pos.x << 4) as f64 - cam.x - 8.0) as f32,
                            ((pos.y << 4) as f64 - cam.y - 8.0) as f32,
                            ((pos.z << 4) as f64 - cam.z - 8.0) as f32,
                        ];
                        let hi = [
                            ((pos.x << 4) as f64 - cam.x + 24.0) as f32,
                            ((pos.y << 4) as f64 - cam.y + 24.0) as f32,
                            ((pos.z << 4) as f64 - cam.z + 24.0) as f32,
                        ];
                        if Self::aabb_visible(planes, lo, hi) {
                            count += 1;
                        }
                    }
                }
            }
        }
        self.premesh.visible_unmeshed = count;
        // Age the tracked pre-meshed sections against the visible gate: the lists are
        // taken out so the gate predicate can borrow the renderer, and their capacities
        // are swapped back (no allocation per frame).
        let mut pending = std::mem::take(&mut self.premesh.premeshed);
        let mut keep = std::mem::take(&mut self.premesh.keep);
        let now = Instant::now();
        let (hits, wasted) =
            Premesh::age_premeshed(now, &|pos| self.wants_section(pos), &mut pending, &mut keep);
        self.premesh.hits += hits;
        self.premesh.wasted += wasted;
        self.premesh.premeshed = keep;
        self.premesh.keep = pending;
        self.premesh.update(camera, aspect, now);
    }
}
