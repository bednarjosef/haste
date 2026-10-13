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
use haste_protocol::haste_core::game::{DimensionInfo, TICK_SECONDS};
use haste_protocol::haste_core::input::{InputState, InteractInput};
use haste_protocol::haste_core::math::Vec3;
use haste_protocol::haste_core::model::{EntityInstance, PoseOverrides};
use haste_protocol::haste_core::view::{BlockView, CollisionShapes, DirtyTracker};
use haste_protocol::haste_core::{
    BlockPos, BlockStateId, ChunkPos, Direction, EntityId, SectionPos,
};
use haste_world::World;
use parking_lot::{Mutex, RwLock};

use crate::net::{NetDriver, WorldSink};

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
    /// Selected hotbar slot (`LocalPlayer::held_slot`), for the scroll wheel.
    pub held_slot: i32,
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
                animation_tick: 0,
                has_position: false,
                feet: DVec3::ZERO,
                eye_height: 0.0,
                time_of_day: 6000,
                has_sky,
                view_distance,
                block_target: None,
                held_slot: 0,
                prev_look: (0.0, 0.0),
                look: (0.0, 0.0),
                prev_feet: DVec3::ZERO,
                pose: None,
            }),
            breaking: Mutex::new(Vec::new()),
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
    let (prev_eye, prev_yaw, prev_pitch) = s.player.camera(0.0);
    let (eye, yaw, pitch) = s.player.camera(1.0);
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
    view.held_slot = s.player.held_slot;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use haste_physics::testing::{states, TestShapes, TestWorld};

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
