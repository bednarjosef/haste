//! Game thread (client-app): the 20 Hz tick that drains the connection into the world, the
//! entity store and the session, and sends this tick's packets. The render thread reads the
//! same state through [`Shared`].
//!
//! Locking: the world and the entity store each sit behind a `parking_lot::RwLock`. The game
//! thread takes the write locks once per tick for the packet drain (never per packet); the
//! render thread takes read locks only to capture dirty sections for the mesher and to copy the
//! entity render instances, never across `Renderer::render`. Everything else the render thread
//! needs per frame is a small `Copy` value in [`Shared::view`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::DVec3;
use haste_data::GameData;
use haste_entity::EntityStore;
use haste_protocol::haste_core::chunk::{ChunkData, LightData};
use haste_protocol::haste_core::game::{DimensionInfo, TICK_SECONDS};
use haste_protocol::haste_core::input::InputState;
use haste_protocol::haste_core::view::DirtyTracker;
use haste_protocol::haste_core::{BlockPos, BlockStateId, ChunkPos, SectionPos};
use haste_world::World;
use parking_lot::{Mutex, RwLock};

use crate::net::{NetDriver, WorldSink};

/// What the render thread needs each frame, copied out under a short lock.
#[derive(Clone, Copy, Debug)]
pub struct ViewState {
    /// Player eye position at the start of the last tick (`LocalPlayer::camera(0.0)`).
    pub prev_eye: DVec3,
    /// Player eye position after the last tick (`LocalPlayer::camera(1.0)`).
    pub eye: DVec3,
    /// Server rotation offset to add to the input layer's look (`LocalPlayer::look_offset`).
    pub look_offset: (f32, f32),
    /// When the last tick finished (partial tick is measured from here).
    pub tick_at: Instant,
    /// Whether the server has placed the player yet.
    pub has_position: bool,
    /// Time of day in ticks (first world clock of `set_time`; 6000 = noon until one arrives).
    pub time_of_day: i64,
    /// Whether the current dimension has a sky.
    pub has_sky: bool,
    /// Server view distance in chunks.
    pub view_distance: i32,
}

/// Input sampled by the render thread from winit, read by the game thread once per tick.
#[derive(Clone, Copy, Debug, Default)]
pub struct SharedInput {
    /// Keys and the input layer's look angles (without the server's offset).
    pub state: InputState,
    /// Double-tap jump seen since the last tick (`LocalPlayer::toggle_flight`).
    pub toggle_flight: bool,
}

/// State shared between the game thread and the render thread.
pub struct Shared {
    /// Game data (block states, registries, shapes), loaded once.
    pub data: Arc<GameData>,
    /// The world.
    pub world: RwLock<World>,
    /// The entities.
    pub entities: RwLock<EntityStore>,
    /// Sections changed since the render thread last looked (deduplicated there).
    pub dirty: Mutex<Vec<SectionPos>>,
    /// Per-frame view values.
    pub view: Mutex<ViewState>,
    /// Input for the next tick.
    pub input: Mutex<SharedInput>,
    /// Ask the game thread to stop.
    pub stop: AtomicBool,
    /// Why the connection ended, once it has.
    pub closed: Mutex<Option<String>>,
}

impl Shared {
    /// Shared state for a fresh connection.
    pub fn new(
        data: Arc<GameData>,
        world: World,
        entities: EntityStore,
        view_distance: i32,
    ) -> Self {
        let has_sky = world.dimension().has_skylight;
        Self {
            data,
            world: RwLock::new(world),
            entities: RwLock::new(entities),
            dirty: Mutex::new(Vec::new()),
            view: Mutex::new(ViewState {
                prev_eye: DVec3::ZERO,
                eye: DVec3::ZERO,
                look_offset: (0.0, 0.0),
                tick_at: Instant::now(),
                has_position: false,
                time_of_day: 6000,
                has_sky,
                view_distance,
            }),
            input: Mutex::new(SharedInput::default()),
            stop: AtomicBool::new(false),
            closed: Mutex::new(None),
        }
    }

    /// Whether the game thread has ended (connection closed or stop requested).
    pub fn is_closed(&self) -> bool {
        self.closed.lock().is_some()
    }
}

/// Fraction of the current tick that has elapsed at `now`, in `0..=1`.
pub fn partial_tick(tick_at: Instant, now: Instant) -> f32 {
    let dt = now.saturating_duration_since(tick_at).as_secs_f64();
    (dt / TICK_SECONDS).clamp(0.0, 1.0) as f32
}

/// Position between the last two ticks (vanilla's `lerp(partialTick, old, new)`).
pub fn interpolate(prev: DVec3, cur: DVec3, partial: f32) -> DVec3 {
    prev.lerp(cur, partial as f64)
}

/// Forwards to the world and records sections the world itself does not mark dirty but the
/// renderer must drop: every section of an unloaded chunk and of every chunk on a respawn.
struct TrackedWorld<'a> {
    world: &'a mut World,
    gone: &'a mut Vec<SectionPos>,
}

impl TrackedWorld<'_> {
    fn chunk_sections(&mut self, pos: ChunkPos) {
        let dim = self.world.dimension();
        let min = dim.min_section();
        for y in min..min + dim.section_count() as i32 {
            self.gone.push(SectionPos::new(pos.x, y, pos.z));
        }
    }
}

impl WorldSink for TrackedWorld<'_> {
    fn load_chunk(&mut self, data: &ChunkData) -> anyhow::Result<()> {
        WorldSink::load_chunk(self.world, data)
    }
    fn unload_chunk(&mut self, pos: ChunkPos) {
        if self.world.chunk(pos).is_some() {
            self.chunk_sections(pos);
        }
        WorldSink::unload_chunk(self.world, pos)
    }
    fn set_block(&mut self, pos: BlockPos, state: BlockStateId) {
        WorldSink::set_block(self.world, pos, state)
    }
    fn apply_light(&mut self, pos: ChunkPos, light: &LightData) {
        WorldSink::apply_light(self.world, pos, light)
    }
    fn reset(&mut self, dim: &DimensionInfo) {
        let loaded: Vec<ChunkPos> = self.world.loaded_chunks().collect();
        for c in loaded {
            self.chunk_sections(c);
        }
        WorldSink::reset(self.world, dim)
    }
    fn chunk_count(&self) -> usize {
        WorldSink::chunk_count(self.world)
    }
    fn non_empty_sections(&self) -> usize {
        WorldSink::non_empty_sections(self.world)
    }
    fn physics(
        &self,
    ) -> Option<(
        &dyn haste_protocol::haste_core::view::BlockView,
        &haste_data::GameData,
    )> {
        WorldSink::physics(self.world)
    }
}

/// Runs the 20 Hz game loop until the connection closes or `shared.stop` is set, then closes
/// the connection. Must run inside the tokio runtime's context (the connection's tasks).
pub fn run(mut driver: NetDriver, shared: Arc<Shared>) {
    let tick = Duration::from_secs_f64(TICK_SECONDS);
    let mut next = Instant::now();
    let mut gone = Vec::new();
    let reason = loop {
        if shared.stop.load(Ordering::Relaxed) {
            break "left the game".to_string();
        }
        if let Err(e) = tick_once(&mut driver, &shared, &mut gone) {
            break format!("tick failed: {e:#}");
        }
        if driver.is_closed() {
            break driver
                .connection()
                .close_reason()
                .unwrap_or_else(|| "connection closed".into());
        }
        next += tick;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > tick * 10 {
            // Far behind (debugger, suspend): skip instead of bursting ticks.
            next = now;
        }
    };
    tracing::info!("game thread stopping: {reason}");
    *shared.closed.lock() = Some(reason);
    driver.disconnect();
}

fn tick_once(
    driver: &mut NetDriver,
    shared: &Shared,
    gone: &mut Vec<SectionPos>,
) -> anyhow::Result<()> {
    {
        let mut world = shared.world.write();
        let mut entities = shared.entities.write();
        let mut tracked = TrackedWorld {
            world: &mut world,
            gone,
        };
        driver.receive(&mut tracked, &mut *entities)?;
        let mut dirty = world.take_dirty_sections();
        dirty.append(gone);
        if !dirty.is_empty() {
            shared.dirty.lock().append(&mut dirty);
        }
    }
    // This tick's input, sampled from winit by the render thread.
    let input = {
        let mut shared_input = shared.input.lock();
        if std::mem::take(&mut shared_input.toggle_flight) {
            driver.session.player.toggle_flight();
        }
        shared_input.state
    };
    {
        let world = shared.world.read();
        let mut entities = shared.entities.write();
        driver.end_tick(&input, &*world, &mut *entities)?;
    }
    let s = &driver.session;
    let (prev_eye, _, _) = s.player.camera(0.0);
    let (eye, _, _) = s.player.camera(1.0);
    let mut view = shared.view.lock();
    view.prev_eye = prev_eye;
    view.eye = eye;
    view.look_offset = s.player.look_offset();
    view.tick_at = Instant::now();
    view.has_position = s.player.has_position;
    view.time_of_day = s.time_of_day.unwrap_or(6000);
    view.has_sky = s.dimension.has_skylight;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_tick_runs_from_zero_to_one_over_a_tick() {
        let t = Instant::now();
        assert_eq!(partial_tick(t, t), 0.0);
        let half = partial_tick(t, t + Duration::from_millis(25));
        assert!((half - 0.5).abs() < 1e-6, "{half}");
        assert_eq!(partial_tick(t, t + Duration::from_millis(80)), 1.0);
        // A clock read before the tick finished counts as the tick start.
        assert_eq!(partial_tick(t + Duration::from_millis(5), t), 0.0);
    }

    #[test]
    fn interpolation_between_ticks() {
        let a = DVec3::new(0.0, 64.0, 0.0);
        let b = DVec3::new(0.4, 64.0, -0.2);
        assert_eq!(interpolate(a, b, 0.0), a);
        assert_eq!(interpolate(a, b, 1.0), b);
        let m = interpolate(a, b, 0.25);
        assert!((m - DVec3::new(0.1, 64.0, -0.05)).length() < 1e-9);
    }
}
