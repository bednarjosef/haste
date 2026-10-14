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
use haste_jvm::JvmHost;
use haste_protocol::haste_core::chunk::{BlockEntityData, ChunkData, LightData};
use haste_protocol::haste_core::game::{DimensionInfo, GameMode, TICK_SECONDS};
use haste_protocol::haste_core::input::{InputState, InteractInput};
use haste_protocol::haste_core::math::Vec3;
use haste_protocol::haste_core::model::{EntityInstance, PoseOverrides};
use haste_protocol::haste_core::view::{BlockView, CollisionShapes, DirtyTracker};
use haste_protocol::haste_core::{
    BlockPos, BlockStateId, ChunkPos, Direction, EntityId, Identifier, RegistryIndex, SectionPos,
};
use haste_world::World;
use parking_lot::{Mutex, RwLock};

use crate::net::{NetDriver, WorldSink};
use crate::ui::Ui;

/// How far the third-person camera sits from the eye when nothing is in the way (vanilla's
/// `Camera` max zoom, minecraft.wiki "Perspective").
pub const THIRD_PERSON_DISTANCE: f64 = 4.0;
/// The camera stops this far before the first solid block along the ray, so it does not clip
/// into it (vanilla's behaviour of pulling the camera closer against a wall).
pub const OBSTRUCTION_CLEARANCE: f64 = 0.1;

/// The perspective F5 cycles through (vanilla's three: first person, third person behind,
/// third person in front).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Perspective {
    #[default]
    FirstPerson,
    /// The camera sits behind the player, looking where the player looks.
    ThirdPersonBehind,
    /// The camera sits in front of the player, looking back at it.
    ThirdPersonFront,
}

impl Perspective {
    /// The `--perspective` screenshot flag's names.
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        match s {
            "first" => Ok(Perspective::FirstPerson),
            "back" => Ok(Perspective::ThirdPersonBehind),
            "front" => Ok(Perspective::ThirdPersonFront),
            _ => anyhow::bail!("unknown perspective {s:?} (first, back or front)"),
        }
    }

    /// F5: the next perspective, vanilla's cycle.
    pub fn next(self) -> Self {
        match self {
            Perspective::FirstPerson => Perspective::ThirdPersonBehind,
            Perspective::ThirdPersonBehind => Perspective::ThirdPersonFront,
            Perspective::ThirdPersonFront => Perspective::FirstPerson,
        }
    }

    /// Whether the local player's own model is drawn (only in third person; in first person
    /// the world is seen through its eyes).
    pub fn draws_player(self) -> bool {
        !matches!(self, Perspective::FirstPerson)
    }
}

/// Camera position and look for a perspective: at the eye in first person; otherwise
/// [`THIRD_PERSON_DISTANCE`] blocks from the eye along the view direction (in front mode the
/// camera faces back: yaw + 180, pitch negated), pulled closer when a block outline is in the
/// way (the same raycast the crosshair's block picking uses, so a wall that blocks the view
/// also stops the camera).
pub fn third_person_camera(
    perspective: Perspective,
    eye: DVec3,
    yaw: f32,
    pitch: f32,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
) -> (DVec3, f32, f32) {
    match perspective {
        Perspective::FirstPerson => (eye, yaw, pitch),
        Perspective::ThirdPersonBehind => {
            let back = -haste_physics::raycast::look_vector(yaw, pitch);
            (camera_pullback(eye, back, world, shapes), yaw, pitch)
        }
        Perspective::ThirdPersonFront => {
            let forward = haste_physics::raycast::look_vector(yaw, pitch);
            (
                camera_pullback(eye, forward, world, shapes),
                yaw + 180.0,
                -pitch,
            )
        }
    }
}

/// `eye + dir * d` for the largest `d` that keeps the camera [`OBSTRUCTION_CLEARANCE`] blocks
/// before the first block outline along the ray (at most [`THIRD_PERSON_DISTANCE`]). A hit the
/// ray starts inside (the eye in a block, e.g. a snow layer) is not an obstruction.
fn camera_pullback(
    eye: DVec3,
    dir: DVec3,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
) -> DVec3 {
    let reach = THIRD_PERSON_DISTANCE + OBSTRUCTION_CLEARANCE;
    let hit = haste_physics::raycast::raycast_blocks(eye, dir, reach, world, shapes);
    let d = match hit {
        Some(h) if !h.inside => (h.distance - OBSTRUCTION_CLEARANCE).max(0.0),
        _ => THIRD_PERSON_DISTANCE,
    };
    eye + dir * d
}

/// The local player's pose for its own model: the physics pose's render equivalent (sleeping,
/// elytra and other metadata-only poses are not modelled yet).
fn player_pose(s: &crate::net::Session) -> Option<haste_core::entity::Pose> {
    use haste_physics::Pose as PhysicsPose;
    match s.player.state.pose {
        PhysicsPose::Standing => None,
        PhysicsPose::Crouching => Some(haste_core::entity::Pose::Crouching),
        PhysicsPose::Swimming => Some(haste_core::entity::Pose::Swimming),
    }
}

/// The local player's own render instance for third person (the server does not spawn us as an
/// entity, so it is not in the store): the humanoid model at the interpolated feet, body yaw
/// and head pitched to the interpolated look, and its pose's adjustments like any player. The
/// limb swing the store drives for other entities is omitted (no per-tick walk phase for the
/// local player yet).
pub fn player_instance(id: EntityId, uuid: u128, view: &ViewState, partial: f32) -> EntityInstance {
    let pos = interpolate(view.prev_feet, view.feet, partial);
    let yaw = view.prev_look.0 + (view.look.0 - view.prev_look.0) * partial;
    let pitch = view.prev_look.1 + (view.look.1 - view.prev_look.1) * partial;
    let mut pose = PoseOverrides::default();
    match view.pose {
        Some(haste_core::entity::Pose::Crouching) => {
            pose.rotations
                .push(("body".into(), Vec3::new(0.5, 0.0, 0.0)));
            pose.rotations
                .push(("head".into(), Vec3::new(-0.5, 0.0, 0.0)));
        }
        Some(
            haste_core::entity::Pose::Swimming
            | haste_core::entity::Pose::FallFlying
            | haste_core::entity::Pose::SpinAttack,
        ) => {
            pose.rotations.push((
                "body".into(),
                Vec3::new(std::f32::consts::FRAC_PI_2, 0.0, 0.0),
            ));
            pose.rotations.push((
                "right_arm".into(),
                Vec3::new(-std::f32::consts::FRAC_PI_2, 0.0, 0.4),
            ));
            pose.rotations.push((
                "left_arm".into(),
                Vec3::new(-std::f32::consts::FRAC_PI_2, 0.0, -0.4),
            ));
        }
        _ => {
            // Head aim: the head follows the look's pitch (the body and head yaw are the same
            // here, so no yaw component).
            pose.rotations
                .push(("head".into(), Vec3::new(pitch.to_radians(), 0.0, 0.0)));
        }
    }
    EntityInstance {
        id,
        model: haste_entity::models::humanoid(false),
        pos,
        yaw,
        pitch,
        head_yaw: yaw,
        scale: 1.0,
        pose,
        // The skin servers send for other players is not synced for ourselves; the renderer's
        // default skin for our UUID is what vanilla shows without a downloaded skin.
        skin: Some(haste_render::default_skin(uuid).0),
        nametag: None,
        // Our own model flashes red in third person like other entities: the hurt tilt follows
        // the same 10-tick ease-out curve, so its size (out of the 14-degree peak) is the
        // hurt strength.
        hurt: (view.hurt_tilt.abs() / 14.0).clamp(0.0, 1.0),
    }
}

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
    /// Whether the player's eyes are in water (`PlayerState::eyes_in_water`), for the HUD's
    /// air bubbles and the water overlay.
    pub eye_in_water: bool,
    /// Selected hotbar slot (`LocalPlayer::held_slot`), for the scroll wheel.
    pub held_slot: i32,
    /// What the first-person view holds this tick (`None` draws no hand: spectator, not yet
    /// placed). The skin it is drawn with is [`Shared::skin`], set once at join.
    pub first_person: Option<FirstPersonView>,
    /// Hurt-tilt camera roll in degrees (`Session::hurt_tilt_degrees`).
    pub hurt_tilt: f32,
    /// Look angles (yaw, pitch) before and after the last tick, for the player's own model in
    /// third person.
    pub prev_look: (f32, f32),
    pub look: (f32, f32),
    /// Feet positions bracketing the last tick (like `prev_eye`/`eye`, for the own model).
    pub prev_feet: DVec3,
    /// Body pose metadata for the own model (`None`: the physics pose only knows standing,
    /// crouching and swimming).
    pub pose: Option<haste_core::entity::Pose>,
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
    /// The sound and music output (None until the client sets it up; headless leaves it
    /// silent). Driven by the game thread only.
    pub sound: Mutex<Option<crate::sound::Sounds>>,
    /// Ask the game thread to stop.
    pub stop: AtomicBool,
    /// Why the connection ended, once it has.
    pub closed: Mutex<Option<String>>,
    /// The skin the first-person arm is drawn with and whether it is the slim model (the local
    /// player's default skin for its profile UUID, `haste_render::default_skin`); set once at
    /// join, it never changes during a connection.
    pub skin: (Identifier, bool),
    /// The HUD and chat (ui.rs): set by the game thread once the JVM has started and
    /// [`Ui::joined`] ran; the render thread clones it per frame ([`Ui`] is cheap to clone).
    pub ui: Mutex<Option<Ui>>,
    /// The HUD's JVM failed to start (no JDK, missing cache): there will be no HUD, ever.
    pub hud_failed: AtomicBool,
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
                eye_in_water: false,
                held_slot: 0,
                first_person: None,
                hurt_tilt: 0.0,
                prev_look: (0.0, 0.0),
                look: (0.0, 0.0),
                prev_feet: DVec3::ZERO,
                pose: None,
            }),
            breaking: Mutex::new(Vec::new()),
            input: Mutex::new(SharedInput::default()),
            sound: Mutex::new(None),
            stop: AtomicBool::new(false),
            closed: Mutex::new(None),
            skin,
            ui: Mutex::new(None),
            hud_failed: AtomicBool::new(false),
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
    // The JVM for the HUD takes tens of seconds to start: on its own thread, in parallel with
    // joining (the connection would time out if we waited here). One warning, no retries: a
    // client without a JDK or cache keeps playing without a HUD.
    let mut jvm = spawn_jvm(&shared);
    let reason = loop {
        if shared.stop.load(Ordering::Relaxed) {
            break "left the game".to_string();
        }
        if let Err(e) = tick_once(&mut driver, &shared, &mut gone, &mut jvm) {
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

/// Starts the JVM for the HUD on its own thread (tens of seconds; the connection would time
/// out if the game thread waited). On failure: one warning and `shared.hud_failed` set, then
/// the client keeps running without a HUD.
fn spawn_jvm(shared: &Arc<Shared>) -> JvmSlot {
    let (tx, rx) = std::sync::mpsc::channel();
    let shared = shared.clone();
    std::thread::Builder::new()
        .name("haste-jvm".into())
        .spawn(move || {
            let t = Instant::now();
            let result =
                haste_jvm::JvmConfig::from_cache(&haste_protocol::haste_core::paths::cache_dir())
                    .map_err(anyhow::Error::from)
                    .and_then(JvmHost::start);
            match result {
                Ok(host) => {
                    tracing::info!(
                        "HUD JVM ready in {:.1} s: {}",
                        t.elapsed().as_secs_f32(),
                        host.proxy_summary()
                    );
                    let _ = tx.send(Some(host));
                }
                Err(e) => {
                    tracing::warn!("no HUD: the JVM did not start: {e:#}");
                    shared.hud_failed.store(true, Ordering::Relaxed);
                    let _ = tx.send(None);
                }
            }
        })
        .expect("spawning the JVM thread");
    JvmSlot { rx: Some(rx) }
}

/// The JVM arriving on its own thread: `take()` once, at the start of a game tick.
struct JvmSlot {
    rx: Option<std::sync::mpsc::Receiver<Option<JvmHost>>>,
}

/// Takes `item` from a slot that a background thread fills once: `None` while it is still
/// working (the slot stays), the item when it arrived, `None` for good after a failure or once
/// taken (the slot is dropped, so later calls are `None` without touching the channel).
fn take_slot<T>(rx: &mut Option<std::sync::mpsc::Receiver<Option<T>>>) -> Option<T> {
    let r = rx.take()?;
    match r.try_recv() {
        Ok(item) => item,
        Err(std::sync::mpsc::TryRecvError::Empty) => {
            *rx = Some(r);
            None
        }
        Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
    }
}

impl JvmSlot {
    /// The started JVM, if it arrived since the last tick. `None` while it is still starting
    /// (or after it failed or was taken).
    fn take(&mut self) -> Option<JvmHost> {
        take_slot(&mut self.rx)
    }
}

fn tick_once(
    driver: &mut NetDriver,
    shared: &Shared,
    gone: &mut Vec<SectionPos>,
    jvm: &mut JvmSlot,
) -> anyhow::Result<()> {
    // Once the JVM arrives (the session joined before the game thread started): build the
    // `Ui` and give the JVM the registries, tags and the local player.
    if let Some(host) = jvm.take() {
        let mut slot = shared.ui.lock();
        if slot.is_none() {
            let ui = Ui::new(host);
            let t = Instant::now();
            match ui.joined(driver.connection(), driver.login()) {
                Ok(()) => {
                    tracing::info!(
                        "HUD joined in {:.0} ms; the vanilla HUD and chat are live",
                        t.elapsed().as_secs_f64() * 1000.0
                    );
                    *slot = Some(ui);
                }
                Err(e) => tracing::warn!("no HUD: joining the UI failed: {e:#}"),
            }
        }
    }
    // This tick's UI output (packets vanilla code sent, chat and commands typed).
    let mut ui_out = Vec::new();
    {
        let mut world = shared.world.write();
        let mut entities = shared.entities.write();
        let mut tracked = TrackedWorld {
            world: &mut world,
            gone,
        };
        let ui = shared.ui.lock().clone();
        driver.receive(&mut tracked, &mut *entities, ui.as_ref())?;
        let mut dirty = world.take_dirty_sections();
        dirty.append(gone);
        if !dirty.is_empty() {
            shared.dirty.lock().append(&mut dirty);
        }
    }
    // This tick's sound packets, played through the output (when the client set one up).
    // This runs again after `end_tick` below to pick up the player's own block sounds
    // (footsteps, mining, placing) that the player's tick just produced.
    {
        let requests = &mut driver.session.sound_requests;
        let mut sound = shared.sound.lock();
        if let Some(sounds) = sound.as_mut() {
            // Entity positions for `sound_entity`, from the store the packets just updated.
            let entities = shared.entities.read();
            sounds.apply(requests, |id| {
                entities
                    .get(haste_protocol::haste_core::EntityId(id))
                    .map(|e| e.pos)
            });
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
        let mut interact = shared_input.interact;
        shared_input.interact.pick_block = false;
        shared_input.interact.drop_item = false;
        shared_input.interact.drop_stack = false;
        shared_input.interact.swap_offhand = false;
        shared_input.interact.hotbar = None;
        // A hotbar slot the UI picked since the last tick (the HUD's digits and wheel, or
        // Java-side code) goes through the normal interact path, like the client's own
        // digits did before the UI owned them: `set_carried_item` is sent and a changed
        // held item restarts breaking. The server's `set_held_slot` changes both sides
        // alike, so it never looks like a UI pick.
        if interact.hotbar.is_none() {
            if let Some(ui) = shared.ui.lock().as_ref() {
                let slot = ui.selected_slot();
                if slot != driver.session.player.held_slot {
                    interact.hotbar = Some(slot.clamp(0, 8) as u8);
                }
            }
        }
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
    // The player's own block sounds this tick (footsteps, mining, placing), played through
    // the same output the packets went through above.
    {
        let requests = &mut driver.session.sound_requests;
        let mut sound = shared.sound.lock();
        if let Some(sounds) = sound.as_mut() {
            sounds.apply(requests, |_| None);
        }
    }
    {
        let mut breaking = shared.breaking.lock();
        breaking.clear();
        driver.session.breaking_stages(&mut breaking);
    }
    if let Some(ui) = shared.ui.lock().as_ref() {
        // The hotbar authority is the player (the client picks with 1-9 and the wheel and tells
        // the server itself; `set_selected_slot` also marks it sent so the UI does not send it
        // a second time). The server's `set_held_slot` reaches both sides alike.
        ui.set_selected_slot(driver.session.player.held_slot);
        ui.tick();
        ui.drain_outgoing(&mut ui_out);
    }
    for p in ui_out {
        driver.queue(p);
    }
    let s = &driver.session;
    let (prev_eye, prev_yaw, prev_pitch) = s.player.camera(0.0);
    let (eye, yaw, pitch) = s.player.camera(1.0);
    // The first-person hand this tick: hidden in spectator (vanilla hides it there), otherwise
    // the main-hand item lit like entities are. Sampled under the world's read lock, like the
    // breaking stages above; the render thread only reads the copy in the view state.
    let first_person = if s.player.game_mode == GameMode::Spectator {
        None
    } else {
        let item = s.player.main_hand().item;
        Some(FirstPersonView::sample(&*shared.world.read(), eye, item))
    };
    // The sound output's tick: listener from the eye, music situation, then 20 Hz tick.
    {
        let mut sound = shared.sound.lock();
        if let Some(sounds) = sound.as_mut() {
            let situation =
                crate::sound::music_situation(s.player.game_mode, s.player.state.eyes_in_water);
            sounds.tick(eye, yaw, situation);
        }
    }
    let mut view = shared.view.lock();
    // The feet bracket the tick like the eye does; the eye height can change within it
    // (sneaking), so the old feet are the last view's, not the eye minus its height.
    let feet = s.player.state.pos;
    view.prev_feet = if view.has_position { view.feet } else { feet };
    view.prev_eye = prev_eye;
    view.eye = eye;
    view.prev_look = (prev_yaw, prev_pitch);
    view.look = (yaw, pitch);
    view.look_offset = s.player.look_offset();
    view.tick_at = Instant::now();
    view.animation_tick = view.animation_tick.wrapping_add(1);
    view.has_position = s.player.has_position;
    view.feet = feet;
    view.pose = player_pose(s);
    view.eye_height = s.player.eye_height();
    view.time_of_day = s.time_of_day.unwrap_or(6000);
    view.has_sky = s.dimension.has_skylight;
    view.block_target = s.player.target();
    view.eye_in_water = s.player.state.eyes_in_water;
    view.held_slot = s.player.held_slot;
    view.first_person = first_person;
    view.hurt_tilt = s.hurt_tilt_degrees();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use haste_physics::testing::{states, TestShapes, TestWorld};

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

    #[test]
    fn jvm_slot_waits_arrives_and_stays_taken() {
        let (tx, rx) = std::sync::mpsc::channel::<Option<u8>>();
        let mut slot = Some(rx);
        // Still starting: nothing, and the slot stays for the next tick.
        assert_eq!(take_slot(&mut slot), None);
        assert!(slot.is_some());
        // It failed (the thread sent None and ended): nothing now, and nothing ever after.
        tx.send(None).unwrap();
        drop(tx);
        assert_eq!(take_slot(&mut slot), None);
        assert!(slot.is_none());
        // A fresh slot that arrives: the item once, then the slot is spent.
        let (tx, rx) = std::sync::mpsc::channel::<Option<u8>>();
        tx.send(Some(7)).unwrap();
        let mut slot = Some(rx);
        assert_eq!(take_slot(&mut slot), Some(7));
        assert_eq!(take_slot(&mut slot), None);
        assert!(slot.is_none());
    }

    /// Eye at (0.5, 0.5, 0.5). Yaw -90 looks east (+x), yaw 90 west, like
    /// `raycast::look_vector`; the movement sine table is not exactly axis-aligned, so position
    /// assertions carry a small tolerance.
    const EYE: DVec3 = DVec3::new(0.5, 0.5, 0.5);
    const EPS: f64 = 1e-6;

    #[test]
    fn perspective_cycle_is_vanillas_order() {
        use Perspective::*;
        assert_eq!(FirstPerson.next(), ThirdPersonBehind);
        assert_eq!(ThirdPersonBehind.next(), ThirdPersonFront);
        assert_eq!(ThirdPersonFront.next(), FirstPerson);
        assert!(!FirstPerson.draws_player());
        assert!(ThirdPersonBehind.draws_player());
        assert!(ThirdPersonFront.draws_player());
    }

    #[test]
    fn first_person_is_the_eye_itself() {
        let w = TestWorld::new();
        let s = TestShapes::default();
        let (pos, yaw, pitch) =
            third_person_camera(Perspective::FirstPerson, EYE, -90.0, 30.0, &w, &s);
        assert_eq!((pos, yaw, pitch), (EYE, -90.0, 30.0));
    }

    #[test]
    fn behind_no_obstacle_sits_exactly_four_blocks_back() {
        let w = TestWorld::new();
        let s = TestShapes::default();
        let (pos, yaw, pitch) =
            third_person_camera(Perspective::ThirdPersonBehind, EYE, -90.0, 0.0, &w, &s);
        // Looking east: the camera is four blocks west, still looking east.
        assert_eq!((yaw, pitch), (-90.0, 0.0));
        let want = EYE + DVec3::new(-THIRD_PERSON_DISTANCE, 0.0, 0.0);
        assert!((pos - want).length() < EPS, "{pos:?} != {want:?}");
    }

    #[test]
    fn behind_stops_one_point_nine_at_a_wall_at_two_blocks() {
        let mut w = TestWorld::new();
        // Looking west, so the behind camera goes east; stone cell (2, 0, 0) puts its
        // outline's west face at x = 2 ("a wall at 2 blocks").
        w.set(2, 0, 0, states::STONE);
        let s = TestShapes::default();
        let (pos, yaw, _) =
            third_person_camera(Perspective::ThirdPersonBehind, EYE, 90.0, 0.0, &w, &s);
        assert_eq!(yaw, 90.0);
        // The camera stops 0.1 blocks before the outline: x = 2 - 0.1 = 1.9.
        assert!((pos.x - 1.9).abs() < EPS, "{pos:?}");
        assert!(
            (pos.y - 0.5).abs() < EPS && (pos.z - 0.5).abs() < EPS,
            "{pos:?}"
        );
    }

    #[test]
    fn front_mode_faces_back_at_the_player() {
        let w = TestWorld::new();
        let s = TestShapes::default();
        let (pos, yaw, pitch) =
            third_person_camera(Perspective::ThirdPersonFront, EYE, -90.0, 30.0, &w, &s);
        // Looking east and 30 degrees down: the camera sits four blocks along the look, facing
        // back (yaw + 180, pitch negated).
        assert_eq!(yaw, 90.0);
        assert_eq!(pitch, -30.0);
        let dir = haste_physics::raycast::look_vector(-90.0, 30.0);
        let want = EYE + dir * THIRD_PERSON_DISTANCE;
        assert!((pos - want).length() < EPS, "{pos:?} != {want:?}");
    }

    #[test]
    fn a_block_the_eye_starts_inside_is_not_an_obstruction() {
        let mut w = TestWorld::new();
        // The eye is inside this block's outline (an `inside` hit): the pullback ignores it.
        w.set(0, 0, 0, states::STONE);
        let s = TestShapes::default();
        let (pos, _, _) =
            third_person_camera(Perspective::ThirdPersonBehind, EYE, -90.0, 0.0, &w, &s);
        // No further hit, so the full distance.
        assert!((pos.x - 0.5 + THIRD_PERSON_DISTANCE).abs() < EPS, "{pos:?}");
    }
}
