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
use haste_protocol::haste_core::chunk::{BlockEntityData, ChunkData, LightData};
use haste_protocol::haste_core::game::{DimensionInfo, GameMode, TICK_SECONDS};
use haste_protocol::haste_core::input::{InputState, InteractInput};
use haste_protocol::haste_core::view::{BlockView, DirtyTracker};
use haste_protocol::haste_core::{
    BlockPos, BlockStateId, ChunkPos, Direction, Identifier, RegistryIndex, SectionPos,
};
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
    /// Game ticks since join (animated atlas textures play at this tick).
    pub animation_tick: u64,
    /// Whether the server has placed the player yet.
    pub has_position: bool,
    /// Player feet position after the last tick (`PlayerState::pos`).
    pub feet: DVec3,
    /// Eye height for the current pose (the camera sits this far above the feet).
    pub eye_height: f64,
    /// Time of day in ticks (first world clock of `set_time`; 6000 = noon until one arrives).
    pub time_of_day: i64,
    /// Whether the current dimension has a sky.
    pub has_sky: bool,
    /// Server view distance in chunks.
    pub view_distance: i32,
    /// Block under the crosshair and the face hit (`LocalPlayer::target`).
    pub block_target: Option<(BlockPos, Direction)>,
    /// Selected hotbar slot (`LocalPlayer::held_slot`), for the scroll wheel.
    pub held_slot: i32,
    /// What the first-person view holds this tick (`None` draws no hand: spectator, not yet
    /// placed). The skin it is drawn with is [`Shared::skin`], set once at join.
    pub first_person: Option<FirstPersonView>,
}

/// The per-tick part of the first-person hand: the held item and the light at the eye. The skin
/// lives in [`Shared::skin`] because it never changes during a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FirstPersonView {
    /// The main-hand stack's item (`None` draws the bare arm).
    pub item: Option<RegistryIndex>,
    /// (sky light, block light) at the eye position, `FirstPerson::light`'s order.
    pub light: (u8, u8),
}

impl FirstPersonView {
    /// The first-person hand for `item` in `world` at `eye`: the item held and the light at the
    /// eye, sampled the same way entities are (`Renderer::sample_entity_lights`).
    pub fn sample(world: &dyn BlockView, eye: DVec3, item: Option<RegistryIndex>) -> Self {
        let p = BlockPos::new(
            eye.x.floor() as i32,
            (eye.y + 0.5).floor() as i32,
            eye.z.floor() as i32,
        );
        FirstPersonView {
            item,
            light: (world.sky_light(p), world.block_light(p)),
        }
    }

    /// The renderer's first-person hand for this view drawn with `skin` (the connection's
    /// fixed skin): since the skin never changes, two views that compare equal produce equal
    /// hands, so comparing the views is the whole "changed?" check.
    pub fn with_skin(&self, skin: &(Identifier, bool)) -> haste_render::FirstPerson {
        haste_render::FirstPerson {
            skin: Some(skin.0.clone()),
            slim: skin.1,
            item: self.item,
            light: self.light,
        }
    }
}

/// Input sampled by the render thread from winit, read by the game thread once per tick.
#[derive(Clone, Copy, Debug, Default)]
pub struct SharedInput {
    /// Keys and the input layer's look angles (without the server's offset).
    pub state: InputState,
    /// Double-tap jump seen since the last tick (`LocalPlayer::toggle_flight`).
    pub toggle_flight: bool,
    /// Mouse buttons held, middle click and hotbar selection since the last tick.
    pub interact: InteractInput,
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
    /// Break cracks to draw (`Session::breaking_stages`), copied once per tick.
    pub breaking: Mutex<Vec<(BlockPos, u8)>>,
    /// Input for the next tick.
    pub input: Mutex<SharedInput>,
    /// Ask the game thread to stop.
    pub stop: AtomicBool,
    /// Why the connection ended, once it has.
    pub closed: Mutex<Option<String>>,
    /// The skin the first-person arm is drawn with and whether it is the slim model (the local
    /// player's default skin for its profile UUID, `haste_render::default_skin`); set once at
    /// join, it never changes during a connection.
    pub skin: (Identifier, bool),
}

impl Shared {
    /// Shared state for a fresh connection.
    pub fn new(
        data: Arc<GameData>,
        world: World,
        entities: EntityStore,
        view_distance: i32,
        skin: (Identifier, bool),
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
                animation_tick: 0,
                has_position: false,
                feet: DVec3::ZERO,
                eye_height: 0.0,
                time_of_day: 6000,
                has_sky,
                view_distance,
                block_target: None,
                held_slot: 0,
                first_person: None,
            }),
            breaking: Mutex::new(Vec::new()),
            input: Mutex::new(SharedInput::default()),
            stop: AtomicBool::new(false),
            closed: Mutex::new(None),
            skin,
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
    fn set_block_entity(&mut self, pos: BlockPos, data: BlockEntityData) {
        WorldSink::set_block_entity(self.world, pos, data)
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
        // Held buttons stay; the press-only middle click, drop/swap presses and the hotbar choice
        // are consumed.
        let interact = shared_input.interact;
        shared_input.interact.pick_block = false;
        shared_input.interact.drop_item = false;
        shared_input.interact.drop_stack = false;
        shared_input.interact.swap_offhand = false;
        shared_input.interact.hotbar = None;
        driver.session.player.set_interact(interact);
        shared_input.state
    };
    {
        // Write lock: the player's breaking predicts block changes into the world.
        let mut world = shared.world.write();
        let mut entities = shared.entities.write();
        driver.end_tick(&input, &mut *world, &mut *entities)?;
        let mut dirty = world.take_dirty_sections();
        if !dirty.is_empty() {
            shared.dirty.lock().append(&mut dirty);
        }
    }
    {
        let mut breaking = shared.breaking.lock();
        breaking.clear();
        driver.session.breaking_stages(&mut breaking);
    }
    let s = &driver.session;
    let (prev_eye, _, _) = s.player.camera(0.0);
    let (eye, _, _) = s.player.camera(1.0);
    // The first-person hand this tick: hidden in spectator (vanilla hides it there), otherwise
    // the main-hand item lit like entities are. Sampled under the world's read lock, like the
    // breaking stages above; the render thread only reads the copy in the view state.
    let first_person = if s.player.game_mode == GameMode::Spectator {
        None
    } else {
        let item = s.player.main_hand().item;
        Some(FirstPersonView::sample(&*shared.world.read(), eye, item))
    };
    let mut view = shared.view.lock();
    view.prev_eye = prev_eye;
    view.eye = eye;
    view.look_offset = s.player.look_offset();
    view.tick_at = Instant::now();
    view.animation_tick = view.animation_tick.wrapping_add(1);
    view.has_position = s.player.has_position;
    view.feet = s.player.state.pos;
    view.eye_height = s.player.eye_height();
    view.time_of_day = s.time_of_day.unwrap_or(6000);
    view.has_sky = s.dimension.has_skylight;
    view.block_target = s.player.target();
    view.held_slot = s.player.held_slot;
    view.first_person = first_person;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_person_view_change_detection() {
        // The skin is fixed for a connection, so view equality is the whole "did the hand
        // change?" check: equal views produce equal hands, differing ones differing hands.
        let skin = (
            haste_core::Identifier::minecraft("entity/player/wide/steve"),
            false,
        );
        let a = FirstPersonView {
            item: Some(RegistryIndex(5)),
            light: (15, 0),
        };
        let b = FirstPersonView {
            item: None,
            light: (15, 0),
        };
        assert_eq!(a, a);
        assert_ne!(a, b);
        let ha = a.with_skin(&skin);
        let hb = b.with_skin(&skin);
        assert_eq!(ha, a.with_skin(&skin));
        assert_ne!(ha, hb);
        assert_eq!(ha.skin, Some(skin.0.clone()));
        assert!(!ha.slim);
        assert_eq!(ha.item, Some(RegistryIndex(5)));
        assert_eq!(ha.light, (15, 0));
        // A differing light also differs (the eye moved into shade).
        let c = FirstPersonView {
            item: Some(RegistryIndex(5)),
            light: (4, 7),
        };
        assert_ne!(a, c);
        assert_ne!(ha, c.with_skin(&skin));
    }

    #[test]
    fn first_person_sample_reads_the_eye_block_light() {
        // Sky 12, block 7: sampled at the eye's block, in FirstPerson::light's order.
        let world = LightWorld {
            sky: 12,
            block: 7,
            asked: parking_lot::Mutex::new(Vec::new()),
        };
        let v =
            FirstPersonView::sample(&world, DVec3::new(3.2, 65.5, -7.8), Some(RegistryIndex(1)));
        assert_eq!(v.item, Some(RegistryIndex(1)));
        assert_eq!(v.light, (12, 7));
        // The eye at y = 65.5 samples y = 66 (eye.y + 0.5 floored), like entity light.
        assert_eq!(world.asked.lock().clone(), vec![BlockPos::new(3, 66, -8)]);
    }

    /// A world that answers one sky/block light pair and records where it was asked.
    struct LightWorld {
        sky: u8,
        block: u8,
        asked: parking_lot::Mutex<Vec<BlockPos>>,
    }

    impl haste_protocol::haste_core::view::BlockView for LightWorld {
        fn block_state(&self, _pos: BlockPos) -> BlockStateId {
            BlockStateId::AIR
        }
        fn is_chunk_loaded(&self, _pos: ChunkPos) -> bool {
            true
        }
        fn sky_light(&self, pos: BlockPos) -> u8 {
            self.asked.lock().push(pos);
            self.sky
        }
        fn block_light(&self, _pos: BlockPos) -> u8 {
            self.block
        }
        fn min_y(&self) -> i32 {
            0
        }
        fn height(&self) -> i32 {
            384
        }
    }

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
