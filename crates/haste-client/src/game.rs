//! Game thread (client-app): the 20 Hz tick that drains the connection into the world, the
//! entity store and the session, and sends this tick's packets. The render thread reads the
//! same state through [`Shared`].
//!
//! Locking: the world and the entity store each sit behind a `parking_lot::RwLock`. The game
//! thread takes the write locks once per tick for the packet drain (never per packet); the
//! render thread takes read locks only to capture dirty sections for the mesher and to copy the
//! entity render instances, never across `Renderer::render`. Everything else the render thread
//! needs per frame is a small `Copy` value in [`Shared::view`].

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
use haste_protocol::haste_core::model::{BodyTransform, EntityInstance, PoseOverrides};
use haste_protocol::haste_core::view::{BlockView, CollisionShapes, DirtyTracker};
use haste_protocol::haste_core::{
    BlockPos, BlockStateId, ChunkPos, Direction, EntityId, Identifier, RegistryIndex, SectionPos,
};
use haste_world::World;
use parking_lot::{Condvar, Mutex, RwLock};

mod block_entity_ticks;

use crate::net::{NetDriver, WorldSink};

pub mod skins;
use crate::ui::Ui;
use block_entity_ticks::{tick_model_anims, TickModelAnims, MODEL_RNG};

/// The particle pool: filled from `level_particles` packets (the renderer's atlas-backed
/// particle types, so unknown/mod types are skipped there), ticked at 20 Hz by the game
/// thread. The render thread reads it per frame under this lock (no GPU state inside).
pub type SharedParticles = Mutex<haste_render::Particles>;

/// How far the third-person camera sits from the eye when nothing is in the way (vanilla's
/// `Camera` max zoom, minecraft.wiki "Perspective"). Vanilla's `Camera.alignWithEntity` scales
/// this by the entity's scale and its `CAMERA_DISTANCE` attribute (default 4.0); haste reads
/// neither yet, so the constant is the whole distance.
pub const THIRD_PERSON_DISTANCE: f64 = 4.0;

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
/// camera faces back: yaw + 180, pitch negated), pulled closer when a block is in the way
/// (vanilla's `Camera.getMaxZoom`, see [`camera_pullback`]).
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

/// `eye + dir * d` for the largest `d` no block along the way beats (at most
/// [`THIRD_PERSON_DISTANCE`]), vanilla's `Camera.getMaxZoom`: 8 rays from the eye's ±0.1
/// corners along `dir`, each cast up to the current best `d`, and every hit whose straight-line
/// distance from the eye is smaller shrinks it. The distance is measured from the eye, not
/// along the ray, and no clearance margin is subtracted: on a flat wall the camera stops
/// epsilon inside the face, and a block the eye is inside counts as a hit at the corner and
/// pulls the camera to sqrt(0.03) ≈ 0.173 blocks out.
fn camera_pullback(
    eye: DVec3,
    dir: DVec3,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
) -> DVec3 {
    let mut camera_dist = THIRD_PERSON_DISTANCE;
    for i in 0..8i32 {
        // The eight corners of the 0.2 cube around the eye (vanilla's jitter offsets).
        let corner = eye
            + DVec3::new(
                ((i & 1) * 2 - 1) as f64 * 0.1,
                ((i >> 1 & 1) * 2 - 1) as f64 * 0.1,
                ((i >> 2 & 1) * 2 - 1) as f64 * 0.1,
            );
        let hit = haste_physics::raycast::raycast_blocks(corner, dir, camera_dist, world, shapes);
        if let Some(h) = hit {
            let d = (h.location - eye).length();
            if d < camera_dist {
                camera_dist = d;
            }
        }
    }
    eye + dir * camera_dist
}

/// The local player's pose for its own model: the physics pose's render equivalent (sleeping and
/// other metadata-only poses are not modelled yet).
fn player_pose(s: &crate::net::Session) -> Option<haste_core::entity::Pose> {
    use haste_physics::Pose as PhysicsPose;
    match s.player.state.pose {
        PhysicsPose::Standing => None,
        PhysicsPose::Crouching => Some(haste_core::entity::Pose::Crouching),
        PhysicsPose::Swimming => Some(haste_core::entity::Pose::Swimming),
        PhysicsPose::FallFlying => Some(haste_core::entity::Pose::FallFlying),
    }
}

/// `AtmosphericFogEnvironment.updateRainFogState`'s sky-light factor: the sky light at the
/// camera's block, clamped `(light - 8) / 7`.
fn rain_fog_sky_light(world: &World, eye: DVec3) -> f32 {
    use haste_core::view::BlockView as _;
    let p = BlockPos::new(
        eye.x.floor() as i32,
        eye.y.floor() as i32,
        eye.z.floor() as i32,
    );
    ((world.sky_light(p) as f32) - 8.0) / 7.0
}

/// Whether the biome at the eye has precipitation (`updateRainFogState` halves the rain
/// fog when it does not).
fn biome_has_precipitation(
    world: &World,
    climates: &[haste_render::BiomeClimate],
    eye: DVec3,
) -> bool {
    use haste_render::BiomeSource as _;
    let p = BlockPos::new(
        eye.x.floor() as i32,
        eye.y.floor() as i32,
        eye.z.floor() as i32,
    );
    let biome = crate::app::biomes::WorldBiomes(world).biome(p).0 as usize;
    climates.get(biome).is_some_and(|c| c.has_precipitation)
}

/// The local player's own render instance for third person (the server does not spawn us as an
/// entity, so it is not in the store): the humanoid model at the interpolated feet, body yaw
/// and head pitched to the interpolated look, and its pose's adjustments like any player. The
/// limb swing the store drives for other entities is omitted (no per-tick walk phase for the
/// local player yet).
pub fn player_instance(
    id: EntityId,
    view: &ViewState,
    partial: f32,
    skin: &haste_entity::skin::PlayerSkin,
) -> EntityInstance {
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
    let mut inst = EntityInstance {
        id,
        model: haste_entity::models::humanoid(false),
        pos,
        yaw,
        pitch,
        head_yaw: yaw,
        scale: 1.0,
        pose,
        body: BodyTransform::default(),
        body_scale: Vec3::ONE,
        skin: None,
        nametag: None,
        below_name: None,
        // `AvatarRenderer` passes shadowRadius 0.5 (LivingEntityRenderer, player scale 1).
        shadow_radius: 0.5,
        shadow_strength: 1.0,
        invisible: false,
        is_discrete: view.pose == Some(haste_core::entity::Pose::Crouching),
        // `EntityAttachment.NAME_TAG` AT_HEIGHT fallback = the player's 1.8 bb height.
        name_tag_y: 1.8,
        // Our own model flashes red in third person like other entities: full strength on the
        // tick of the hit, fading over the 10 hurt ticks (vanilla's hurtTime / hurtDuration).
        hurt: (view.hurt_ticks.min(crate::net::HURT_TILT_TICKS) as f32
            / crate::net::HURT_TILT_TICKS as f32),
        overlays: Default::default(),
        item: None,
        decoration: None,
        hands: [None, None],
        equipment: Vec::new(),
        head_item: None,
    };
    skins::dress_local(&mut inst, skin, view, partial);
    inst
}

/// What the render thread needs each frame, copied out under a short lock.
/// `LivingEntity.isDeadOrDying` as the camera sees it: `Some(deathTime)` while the health is
/// <= 0 (0 on the tick it got there), `None` alive.
pub fn death_time(dead: bool, death_ticks: u32) -> Option<u32> {
    dead.then_some(death_ticks)
}

#[derive(Clone, Debug)]
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
    /// World age in ticks (`set_time`'s `world_age`), for game-time-keyed animations
    /// (vanilla `Level.getGameTime`: `BannerRenderer`'s wave phase).
    pub world_age: i64,
    /// Whether the current dimension has a sky.
    pub has_sky: bool,
    /// Server view distance in chunks.
    pub view_distance: i32,
    /// Block under the crosshair and the face hit (`LocalPlayer::target`).
    pub block_target: Option<(BlockPos, Direction)>,
    /// Whether the player's eyes are in water (`PlayerState::eyes_in_water`), for the HUD's
    /// air bubbles and the water overlay.
    pub eye_in_water: bool,
    /// Whether the camera is under the lava surface at the eye's block (vanilla's
    /// `Camera.getFluidInCamera` lava rule), for the submerged FOV.
    pub eye_in_lava: bool,
    /// Blindness effect remaining ticks (`None` without it), for the fog environment.
    pub blindness: Option<i32>,
    /// `LocalPlayer.getWaterVision()` (0 out of water), for the underwater fog distance.
    pub water_vision: f32,
    /// Whether we are a spectator (`Entity.isSpectator`), for the lava/powder-snow fog.
    pub spectator: bool,
    /// Whether we have fire resistance, for the lava fog distance.
    pub fire_resistance: bool,
    /// `BossHealthOverlay.shouldCreateWorldFog()`, for the thick boss fog.
    pub boss_world_fog: bool,
    /// Selected hotbar slot (`LocalPlayer::held_slot`), for the scroll wheel.
    pub held_slot: i32,
    /// What the first-person view holds this tick (`None` draws no hand: spectator, not yet
    /// placed). The skin it is drawn with is [`Shared::skin`], set once at join.
    pub first_person: Option<FirstPersonView>,
    /// The smoothed FOV modifier, last tick's value and this tick's (`Session::fov_modifier`
    /// / `old_fov_modifier`), interpolated per frame.
    pub fov_modifier: f32,
    pub fov_modifier_old: f32,
    /// Walk distance bracketing the last tick (`Session::walk_dist` / `walk_dist_o`).
    pub walk_dist: f32,
    pub walk_dist_o: f32,
    /// The bob amount bracketing the last tick (`Session::bob` / `bob_o`).
    pub bob: f32,
    pub bob_o: f32,
    /// Hurt ticks left and the damage direction in degrees (`Session::hurt_ticks` /
    /// `hurt_dir`).
    pub hurt_ticks: u32,
    pub hurt_dir: f32,
    /// Ticks since the local player died (`Session::death_ticks`, vanilla `deathTime`).
    pub death_ticks: u32,
    /// `LivingEntity.isDeadOrDying`: the player's health is <= 0 (true from the tick it hits 0,
    /// while `death_ticks` is still 0).
    pub dead: bool,
    /// Look angles (yaw, pitch) before and after the last tick, for the player's own model in
    /// third person.
    pub prev_look: (f32, f32),
    pub look: (f32, f32),
    /// Feet positions bracketing the last tick (like `prev_eye`/`eye`, for the own model).
    pub prev_feet: DVec3,
    /// Our cape's trailing point (`ClientAvatarState`'s cloak), ticked with the feet.
    pub cloak: haste_entity::avatar::Cloak,
    /// Body pose metadata for the own model (`None`: the physics pose only knows standing,
    /// crouching and swimming).
    pub pose: Option<haste_core::entity::Pose>,
    /// This tick's screen effects (`screen_effects`): the block the camera is inside, the
    /// worn head item's overlay, on fire, freeze ticks, portal and nausea, the light at the
    /// eye and whether we are scoping a spyglass.
    pub screen: crate::screen_effects::ViewEffects,
    /// Lightmap per-tick state the render thread folds into the lightmap by partial tick:
    /// the block-light flicker, the night-vision effect duration (`None` without it), the
    /// darkness effect's blend state, the boss-overlay world darkening (this tick and the
    /// last), the lightning sky flash, and the player's tick count for the darkness pulse.
    pub lightmap_flicker: f32,
    pub lightmap_night_vision: Option<i32>,
    /// Darkness effect remaining ticks (`None` without it), for the fog environment.
    pub lightmap_darkness: Option<i32>,
    pub lightmap_darkness_blend: haste_render::lightmap::BlendState,
    pub lightmap_boss_darkening: (f32, f32),
    pub lightmap_lightning: bool,
    pub lightmap_tick_count: i32,
    /// The weather levels bracketing the tick (`Level.oRainLevel`/`rainLevel` and the
    /// thunder pair), lerped by partial tick for the sky, fog and weather columns.
    pub rain_level: (f32, f32),
    pub thunder_level: (f32, f32),
    /// `AtmosphericFogEnvironment.rainFogMultiplier`, eased per frame by the tick.
    pub rain_fog_multiplier: f32,
    /// `ClientLevel.seaLevel` (the spawn info's; biome precipitation's snow line).
    pub sea_level: i32,
    /// `ClientLevelData.getHorizonHeight` (the dark disc's threshold).
    pub horizon_height: f64,
}

impl ViewState {
    /// [`death_time`] of this tick's state, for the death FOV and roll.
    pub fn death(&self) -> Option<u32> {
        death_time(self.dead, self.death_ticks)
    }
}

/// The per-tick part of the first-person hand: the held item, the light at the eye and this
/// tick's hand animation (swing, equip progress, bob). Every animated value is the pair of the
/// value at the start and at the end of the tick; the renderer lerps them by partial tick, the
/// way vanilla's `FirstPersonHandsAndItems.extractRenderState` does. The skin lives in
/// [`Shared::skin`] because it never changes during a connection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FirstPersonView {
    /// The main-hand stack's item (`None` draws the bare arm).
    pub item: Option<RegistryIndex>,
    /// (sky light, block light) at the eye position, `FirstPerson::light`'s order.
    pub light: (u8, u8),
    /// Swing progress 0..1 over the swing's ticks (0 when the hand does not swing).
    pub attack: (f32, f32),
    /// Equip progress: 1 held, 0 while the item switches.
    pub equip: (f32, f32),
    /// The hand's pitch lag (`LocalPlayer.xBob`), in degrees.
    pub x_bob: (f32, f32),
    /// The hand's yaw lag (`LocalPlayer.yBob`), in degrees.
    pub y_bob: (f32, f32),
}

impl FirstPersonView {
    /// The first-person hand for `item` in `world` at `eye`: the item held and the light at the
    /// block containing the eye (`EntityRenderer.getPackedLightCoords` probes
    /// `Entity.getLightProbePosition`, the eye position); block light is 15 while the player is
    /// on fire (`EntityRenderer.getBlockLightLevel`).
    pub fn sample(
        world: &dyn BlockView,
        eye: DVec3,
        item: Option<RegistryIndex>,
        on_fire: bool,
    ) -> Self {
        let p = BlockPos::new(
            eye.x.floor() as i32,
            eye.y.floor() as i32,
            eye.z.floor() as i32,
        );
        let block_light = if on_fire { 15 } else { world.block_light(p) };
        FirstPersonView {
            item,
            light: (world.sky_light(p), block_light),
            attack: (0.0, 0.0),
            equip: (1.0, 1.0),
            x_bob: (0.0, 0.0),
            y_bob: (0.0, 0.0),
        }
    }

    /// This tick's hand animation from the player's [`crate::hand::Hand`].
    pub fn with_hand(&mut self, hand: &crate::hand::Hand) {
        self.item = hand.shown_item();
        self.attack = hand.attack();
        self.equip = hand.equip();
        self.x_bob = hand.x_bob();
        self.y_bob = hand.y_bob();
    }

    /// The renderer's first-person hand for this view drawn with `skin` and, when `sleeve`,
    /// its right sleeve: two equal views with the same skin produce equal hands, so the
    /// caller's "changed?" check compares the view and the skin.
    pub fn with_skin(
        &self,
        skin: &haste_entity::skin::PlayerSkin,
        sleeve: bool,
    ) -> haste_render::FirstPerson {
        haste_render::FirstPerson {
            skin: Some(skin.body.clone()),
            slim: skin.model.is_slim(),
            sleeve,
            item: self.item,
            light: self.light,
            attack: pair(self.attack),
            equip: pair(self.equip),
            x_bob: pair(self.x_bob),
            y_bob: pair(self.y_bob),
        }
    }
}

/// A per-tick pair into the renderer's [`haste_render::TickPair`].
fn pair(v: (f32, f32)) -> haste_render::TickPair {
    haste_render::TickPair {
        start: v.0,
        end: v.1,
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
    /// The current perspective is first person (vanilla's camera type; the spyglass FOV
    /// zoom only applies in first person).
    pub first_person: bool,
}

/// State shared between the game thread and the render thread.
pub struct Shared {
    /// Game data (block states, registries, shapes), loaded once.
    pub data: Arc<GameData>,
    /// The world.
    pub world: RwLock<World>,
    /// The render thread's weather extraction buffer (reused every frame, no allocation
    /// after warmup). Only the render thread takes it.
    pub weather_scratch: Mutex<haste_render::WeatherFrame>,
    /// The entities.
    pub entities: RwLock<EntityStore>,
    /// Sections changed since the render thread last looked (deduplicated there).
    pub dirty: Mutex<Vec<SectionPos>>,
    /// Per-frame view values.
    pub view: Mutex<ViewState>,
    /// Break cracks to draw (`Session::breaking_stages`), copied once per tick.
    pub breaking: Mutex<Vec<(BlockPos, u8)>>,
    /// Lid animations to draw (`Session::lid_states`): (position, open value at the last
    /// tick's start and end), copied once per tick; the render thread lerps by partial tick.
    pub block_entity_lids: Mutex<Vec<(BlockPos, (f32, f32))>>,
    /// Skull animations to draw: (position, animation tick at the last tick's start and
    /// end) for powered skulls (vanilla `SkullBlockEntity.animation`'s counter), copied
    /// once per tick.
    pub skull_anims: Mutex<Vec<(BlockPos, (f32, f32))>>,
    /// Vault and spawner spins by block position: (spin at the last tick's start and end) in
    /// degrees. Vanilla's `VaultClientData.updateDisplayItemSpin` (10 degrees a tick) and
    /// `BaseSpawner.clientTick`'s / `TrialSpawner.tickClient`'s mob spin; the game tick
    /// advances them, the renderer interpolates them.
    pub block_entity_spins: Mutex<Vec<(BlockPos, (f32, f32))>>,
    /// The object-model block entities' animation state, by block position, ticked once per
    /// tick (vanilla's block entity tickers): the bell shake, the decorated pot wobble, the
    /// enchanting table's book and the conduit's spin. The render thread copies it per frame.
    pub block_entity_anims:
        Mutex<std::collections::HashMap<BlockPos, haste_render::BlockEntityAnim>>,
    /// Block entity type names by synced registry id (`minecraft:block_entity_type`), for
    /// resolving what each stored block entity is; the static registry's order when the server
    /// did not sync one (vanilla servers' ids agree with the data generator's).
    pub block_entity_names: Vec<haste_core::Identifier>,
    /// Biome climates of the synced registry (index = biome id), for the fog's per-biome
    /// environment attributes (`FogRenderer` resolves them from the camera biome).
    pub biome_climates: Vec<haste_render::BiomeClimate>,
    /// Particles (`level_particles`): the game thread spawns and ticks them, the render
    /// thread draws them.
    pub particles: SharedParticles,
    /// Input for the next tick.
    pub input: Mutex<SharedInput>,
    /// The sound and music output (None until the client sets it up; headless leaves it
    /// silent). Driven by the game thread only.
    pub sound: Mutex<Option<crate::sound::Sounds>>,
    /// Ask the game thread to stop.
    pub stop: AtomicBool,
    /// Why the connection ended, once it has.
    pub closed: Mutex<Option<String>>,
    /// The skin manager: profiles' skins and capes, downloaded off-thread (`skins`).
    pub skins: Mutex<skins::SkinManager>,
    /// The HUD and chat (ui.rs): set by the game thread once the JVM has started and
    /// [`Ui::joined`] ran; the render thread clones it per frame ([`Ui`] is cheap to clone).
    pub ui: Mutex<Option<Ui>>,
    /// The HUD's JVM failed to start (no JDK, missing cache): there will be no HUD, ever.
    pub hud_failed: AtomicBool,
    /// The HUD is off for this run (vanilla's F1: no HUD, no hand). The game thread then
    /// never starts the JVM, so `hud_failed` never becomes true and nobody waits for it.
    /// Vanilla's F1 also hides the block outline (`GameRenderer.shouldRenderBlockOutline`
    /// checks `hud.isHidden`) and the first-person hand (`GameRenderer.renderItemInHand`);
    /// the render thread reads this instead of receiving a flag per frame.
    pub hud_enabled: AtomicBool,
    /// The options the game is running with right now: loaded at join, then updated live from
    /// the vanilla options screens through the option bridge ([`crate::ui::Ui`]). The render
    /// thread copies it once per frame; the game thread is the only writer.
    pub live_options: Mutex<haste_setup::options::Options>,
    /// Bumped every time [`Shared::live_options`] changes, so the render thread copies the
    /// options only when they changed (never a per-frame clone).
    pub options_version: AtomicU64,
    /// A resource pack reload the render thread owes (vanilla
    /// `Minecraft.reloadResourcePacks`): the pack screen's committed selection
    /// (repository ids bottom first) and the server's downloaded packs (push
    /// order, layered above the selection). The renderer rebuilds from a fresh
    /// asset stack (atlas, models, fonts, GUI sprites) when it takes this,
    /// without restarting. `None` = no reload pending.
    pub resource_pack_reload: Mutex<Option<PackReload>>,

    /// The server packs applied to the running asset stack (push order): a
    /// new download joins them so a reload layers every server pack, not only
    /// the newest.
    pub applied_server_packs: Mutex<Vec<std::path::PathBuf>>,
    /// F3's tps line: packets sent per second (f64 bits), updated once per second by the game
    /// thread from the connection's counters.
    pub debug_tx: AtomicU64,
    /// Packets received per second (f64 bits).
    pub debug_rx: AtomicU64,
    /// Chunk packets received (`NetCounters.chunks_loaded`), copied once per second by the
    /// game thread so diagnostics (the screenshot summary) can show them.
    pub chunks_loaded: AtomicU64,
    /// Chunk unload packets received.
    pub chunks_unloaded: AtomicU64,
    /// Wakes the game thread the moment a play packet arrives (the reader task
    /// notifies through [`Connection`'s wake hook]). Vanilla drains the packet
    /// queue every frame (`Minecraft.runTick`), so a packet never waits for the
    /// next 20 Hz tick to be applied; the game thread sleeps on this condvar
    /// between packets and ticks, and re-sleeps when the queue is empty.
    pub packet_wake: (Mutex<u64>, Condvar),
}

impl Shared {
    /// Notifies the game thread that a packet arrived. Cheap when nobody
    /// sleeps: one lock, one counter bump, one notify.
    pub fn wake_on_packet(&self) {
        let mut count = self.packet_wake.0.lock();
        *count += 1;
        self.packet_wake.1.notify_all();
    }
}

/// What a resource pack rebuild applies.
#[derive(Clone, Debug)]
pub struct PackReload {
    /// Repository ids of the selection, bottom first (options.txt's order).
    pub ids: Vec<String>,
    /// The server's downloaded pack files, in push order, above the selection.
    pub server: Vec<std::path::PathBuf>,
}

impl Shared {
    /// Shared state for a fresh connection. `block_entity_names` are the
    /// `minecraft:block_entity_type` names by synced id (see [`Shared::block_entity_names`]).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        data: Arc<GameData>,
        world: World,
        entities: EntityStore,
        view_distance: i32,
        particles: haste_render::Particles,
        skins: skins::SkinManager,
        block_entity_names: Vec<Identifier>,
        biome_climates: Vec<haste_render::BiomeClimate>,
        options: haste_setup::options::Options,
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
                world_age: 0,
                has_sky,
                view_distance,
                block_target: None,
                eye_in_water: false,
                eye_in_lava: false,
                blindness: None,
                water_vision: 0.0,
                spectator: false,
                fire_resistance: false,
                boss_world_fog: false,
                held_slot: 0,
                first_person: None,
                fov_modifier: 0.0,
                fov_modifier_old: 0.0,
                walk_dist: 0.0,
                walk_dist_o: 0.0,
                bob: 0.0,
                bob_o: 0.0,
                hurt_ticks: 0,
                hurt_dir: 0.0,
                death_ticks: 0,
                dead: false,
                prev_look: (0.0, 0.0),
                look: (0.0, 0.0),
                prev_feet: DVec3::ZERO,
                cloak: Default::default(),
                pose: None,
                screen: crate::screen_effects::ViewEffects::default(),
                lightmap_flicker: 0.0,
                lightmap_night_vision: None,
                lightmap_darkness: None,
                lightmap_darkness_blend: Default::default(),
                lightmap_boss_darkening: (0.0, 0.0),
                lightmap_lightning: false,
                lightmap_tick_count: 0,
                rain_level: (0.0, 0.0),
                thunder_level: (0.0, 0.0),
                rain_fog_multiplier: 0.0,
                sea_level: 63,
                horizon_height: 63.0,
            }),
            breaking: Mutex::new(Vec::new()),
            block_entity_lids: Mutex::new(Vec::new()),
            skull_anims: Mutex::new(Vec::new()),
            block_entity_spins: Mutex::new(Vec::new()),
            block_entity_anims: Mutex::new(std::collections::HashMap::new()),
            block_entity_names,
            biome_climates,
            particles: Mutex::new(particles),
            weather_scratch: Mutex::new(haste_render::WeatherFrame::default()),
            input: Mutex::new(SharedInput::default()),
            sound: Mutex::new(None),
            stop: AtomicBool::new(false),
            closed: Mutex::new(None),
            skins: Mutex::new(skins),
            ui: Mutex::new(None),
            hud_failed: AtomicBool::new(false),
            hud_enabled: AtomicBool::new(true),
            live_options: Mutex::new(options),
            options_version: AtomicU64::new(1),
            resource_pack_reload: Mutex::new(None),
            applied_server_packs: Mutex::new(Vec::new()),
            debug_tx: AtomicU64::new(0),
            debug_rx: AtomicU64::new(0),
            chunks_loaded: AtomicU64::new(0),
            chunks_unloaded: AtomicU64::new(0),
            packet_wake: (Mutex::new(0), Condvar::new()),
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
    fn block_state_at(&self, pos: BlockPos) -> BlockStateId {
        WorldSink::block_state_at(self.world, pos)
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
/// `existing_ui`: a UI over an already-started JVM (menu mode joins through the title screen's
/// multiplayer list, whose JVM and UI stay for the whole run); without one the HUD's JVM starts
/// on its own thread in parallel with joining.
pub fn run(mut driver: NetDriver, shared: Arc<Shared>, existing_ui: Option<Ui>) {
    let tick = Duration::from_secs_f64(TICK_SECONDS);
    let mut next = Instant::now();
    let mut gone = Vec::new();
    // F3's tx/rx rates: the connection's counters sampled once per second (vanilla's
    // `Connection.getAverageSentPackets` is a 1 s window too).
    let mut stats_second = Instant::now();
    let mut last_in = 0u64;
    let mut last_out = 0u64;
    // The JVM for the HUD takes tens of seconds to start: on its own thread, in parallel with
    // joining (the connection would time out if we waited here). One warning, no retries: a
    // client without a JDK or cache keeps playing without a HUD. With the HUD off (`--hud
    // off`, vanilla's F1), it is never started.
    let mut jvm = existing_ui
        .map(|ui| {
            // The menu already ticked this Ui; the game thread takes it over from here.
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = tx.send(Some(HudArrival::Menu(ui)));
            HudSlot { rx: Some(rx) }
        })
        .unwrap_or_else(|| {
            if shared.hud_enabled.load(Ordering::Relaxed) {
                spawn_jvm(&shared)
            } else {
                HudSlot { rx: None }
            }
        });
    // The server pack set version this thread last applied (starts at the
    // connection's: the configuration-phase packs were applied at session build).
    let mut server_packs_version = driver.connection().server_packs_version();
    // Packets are drained on arrival, not only at ticks: vanilla runs the packet
    // queue every frame (`Minecraft.runTick` -> `processQueuedPackets`), so a
    // `container_set_slot` shows on the frame after it arrives instead of up to
    // one tick (50 ms) later. Between ticks the thread sleeps on the wake below
    // and only drains packets (no game logic, no input): the 20 Hz tick order
    // and rate are unchanged.
    let mut drained_wakes = 0u64;
    let mut failure: Option<String> = None;
    let reason = loop {
        if shared.stop.load(Ordering::Relaxed) {
            break "left the game".to_string();
        }
        if let Some(f) = failure.take() {
            break f;
        }
        if let Err(e) = tick_once(
            &mut driver,
            &shared,
            &mut gone,
            &mut jvm,
            &mut server_packs_version,
        ) {
            break format!("tick failed: {e:#}");
        }
        if stats_second.elapsed() >= Duration::from_secs(1) {
            let s = driver.stats();
            shared.debug_tx.store(
                ((s.packets_out - last_out) as f64).to_bits(),
                Ordering::Relaxed,
            );
            shared.debug_rx.store(
                ((s.packets_in - last_in) as f64).to_bits(),
                Ordering::Relaxed,
            );
            let c = &driver.session.counters;
            shared
                .chunks_loaded
                .store(c.chunks_loaded, Ordering::Relaxed);
            shared
                .chunks_unloaded
                .store(c.chunks_unloaded, Ordering::Relaxed);
            last_out = s.packets_out;
            last_in = s.packets_in;
            stats_second = Instant::now();
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
            // Sleep until the next tick, but wake the moment a packet arrives
            // (the reader bumps the counter and notifies). The counter, not the
            // notify, carries the signal, so a wake that arrived while we were
            // awake is still seen on the next lock.
            while Instant::now() < next && failure.is_none() {
                let left = next.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                let mut seen = shared.packet_wake.0.lock();
                if *seen == drained_wakes {
                    let _ = shared.packet_wake.1.wait_for(&mut seen, left);
                }
                let arrived = *seen != drained_wakes;
                drop(seen);
                if arrived {
                    if let Err(e) =
                        drain_packets(&mut driver, &shared, &mut gone, &mut server_packs_version)
                    {
                        failure = Some(format!("tick failed: {e:#}"));
                        break;
                    }
                    drained_wakes = *shared.packet_wake.0.lock();
                }
            }
        } else if now - next > tick * 10 {
            // Far behind (debugger, suspend): skip instead of bursting ticks.
            next = now;
        }
    };
    tracing::info!("game thread stopping: {reason}");
    *shared.closed.lock() = Some(reason);
    driver.disconnect();
}

/// Applies the option changes a vanilla screen made to the running client: the shared live
/// options (FOV, GUI scale, mouse sensitivity, volumes, ... the render thread reads them), the
/// render distance the mesher uses (capped by the server's view distance, like at join) and the
/// sound volumes. One info log line per applied option, the way vanilla applies each change the
/// moment the player makes it (`OptionInstance.set` -> `onValueUpdate`).
fn apply_option_changes(shared: &Shared, driver: &NetDriver, changes: Vec<(String, String)>) {
    let mut sound_changed = false;
    let mut render_distance = None;
    let mut applied = false;
    {
        let mut live = shared.live_options.lock();
        for (key, value) in changes {
            if !live.apply_one(&key, &value) {
                continue;
            }
            tracing::info!("applied option {key} = {value}");
            applied = true;
            sound_changed |= key.starts_with("soundCategory_");
            if key == "renderDistance" {
                render_distance = Some(live.render_distance as i32);
            }
        }
    }
    if !applied {
        return;
    }
    // Bumped after the lock is released: a reader that sees the new version will see the new
    // options when it locks.
    shared.options_version.fetch_add(1, Ordering::Relaxed);
    if let Some(rd) = render_distance {
        // The server's view distance still caps what we mesh, like at join.
        shared.view.lock().view_distance = driver.login().view_distance.min(rd);
    }
    if sound_changed {
        let volumes = shared.live_options.lock().sound;
        if let Some(sounds) = shared.sound.lock().as_mut() {
            sounds.set_volumes(&volumes);
        }
    }
}

/// Starts the JVM for the HUD on its own thread (tens of seconds; the connection would time/// out if the game thread waited). On failure: one warning and `shared.hud_failed` set, then
/// the client keeps running without a HUD.
fn spawn_jvm(shared: &Arc<Shared>) -> HudSlot {
    let (tx, rx) = std::sync::mpsc::channel();
    let shared = shared.clone();
    std::thread::Builder::new()
        .name("haste-jvm".into())
        .spawn(move || {
            let t = Instant::now();
            let result =
                haste_jvm::JvmConfig::from_cache(&haste_protocol::haste_core::paths::cache_dir())
                    .map_err(anyhow::Error::from)
                    .map(|mut cfg| {
                        // the same game directory the menu's JVM and the asset pack
                        // selection use (vanilla's options.txt and resourcepacks/ live there)
                        cfg.game_dir = crate::app::game_dir();
                        cfg
                    })
                    .and_then(JvmHost::start);
            match result {
                Ok(host) => {
                    tracing::info!(
                        "HUD JVM ready in {:.1} s: {}",
                        t.elapsed().as_secs_f32(),
                        host.proxy_summary()
                    );
                    let _ = tx.send(Some(HudArrival::Fresh(host)));
                }
                Err(e) => {
                    tracing::warn!("no HUD: the JVM did not start: {e:#}");
                    shared.hud_failed.store(true, Ordering::Relaxed);
                }
            }
        })
        .expect("spawning the JVM thread");
    HudSlot { rx: Some(rx) }
}

/// What the HUD's slot delivers: a freshly started JVM, or menu mode's already-built UI.
enum HudArrival {
    /// The JVM thread started one (spawn_jvm): the `Ui` is built around it here.
    Fresh(JvmHost),
    /// Menu mode's UI (title screen, registries already loaded in Java).
    Menu(Ui),
}

/// The JVM (or menu UI) arriving on its own thread: `take()` once, at the start of a game tick.
struct HudSlot {
    rx: Option<std::sync::mpsc::Receiver<Option<HudArrival>>>,
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

impl HudSlot {
    /// The JVM (or menu UI), if it arrived since the last tick. `None` while it is still
    /// starting (or after it failed or was taken).
    fn take_ui(&mut self) -> Option<HudArrival> {
        take_slot(&mut self.rx)
    }
}

/// Drains the packets that arrived since the last call into the session, world,
/// entities and UI, and publishes the dirty sections. Runs both at the start of
/// a tick and whenever a packet wakes the game thread between ticks (vanilla
/// runs the packet queue every frame, `Minecraft.runTick` ->
/// `packetProcessor.processQueuedPackets`, before the tick's own work).
fn drain_packets(
    driver: &mut NetDriver,
    shared: &Shared,
    gone: &mut Vec<SectionPos>,
    server_packs_version: &mut u64,
) -> anyhow::Result<()> {
    let mut world = shared.world.write();
    let mut entities = shared.entities.write();
    let mut tracked = TrackedWorld {
        world: &mut world,
        gone,
    };
    let ui = shared.ui.lock().clone();
    driver.receive(&mut tracked, &mut *entities, ui.as_ref())?;
    drop(entities);
    skins::request_profiles(&shared.skins, &mut driver.session.profiles);
    let mut dirty = world.take_dirty_sections();
    // The sections of forgotten chunks ride the same dirty set: the render thread's
    // mesh workers find no chunk for them and free the resident meshes (vanilla drops
    // a chunk's render data with the chunk, `ClientChunkCache.drop`).
    dirty.append(gone);
    if !dirty.is_empty() {
        shared.dirty.lock().append(&mut dirty);
    }
    // The server's live pack set changed (vanilla ticks its ServerPackManager the
    // same way: a download finished, a re-push replaced one, or a pop arrived):
    // apply the whole set on top of the stack and reload, like the pack screen's
    // Done. The connection's reply path already sent statuses.
    let version = driver.connection().server_packs_version();
    if version != *server_packs_version {
        *server_packs_version = version;
        let packs = driver.connection().server_packs();
        for pack in &packs {
            tracing::info!(
                "server resource pack {} downloaded to {}",
                pack.id,
                pack.path.display()
            );
        }
        *shared.applied_server_packs.lock() = packs.iter().map(|p| p.path.clone()).collect();
        let ids = crate::app::selected_resource_packs();
        let server = shared.applied_server_packs.lock().clone();
        *shared.resource_pack_reload.lock() = Some(PackReload { ids, server });
    }
    Ok(())
}

fn tick_once(
    driver: &mut NetDriver,
    shared: &Shared,
    gone: &mut Vec<SectionPos>,
    jvm: &mut HudSlot,
    server_packs_version: &mut u64,
) -> anyhow::Result<()> {
    // Once the JVM arrives (the session joined before the game thread started): build the
    // `Ui` and give the JVM the registries, tags and the local player. Menu mode's UI arrives
    // pre-built (its JVM opened the title screen); only `joined` runs for it.
    if let Some(ui) = jvm.take_ui() {
        let mut created = false;
        {
            let mut slot = shared.ui.lock();
            if slot.is_none() {
                let ui = match ui {
                    HudArrival::Fresh(host) => Ui::new(host),
                    HudArrival::Menu(ui) => ui,
                };
                let t = Instant::now();
                match ui.joined(
                    driver.connection(),
                    driver.login(),
                    driver.session.player.game_mode,
                ) {
                    Ok(()) => {
                        tracing::info!(
                            "HUD joined in {:.0} ms; the vanilla HUD and chat are live",
                            t.elapsed().as_secs_f64() * 1000.0
                        );
                        *slot = Some(ui);
                        created = true;
                    }
                    Err(e) => tracing::warn!("no HUD: joining the UI failed: {e:#}"),
                }
            }
        }
        // Play packets arrived while the JVM booted; give them to the HUD now, in order, so a
        // fresh join's tab list, boss bars and scoreboard are not lost.
        if created {
            let backlog = driver.take_ui_backlog();
            if !backlog.is_empty() {
                tracing::info!(
                    "HUD catching up on {} packets from the JVM boot",
                    backlog.len()
                );
                if let Some(ui) = shared.ui.lock().as_ref() {
                    let world = shared.world.read();
                    for p in backlog {
                        ui.handle(&p, &|pos| WorldSink::block_state_at(&*world, pos));
                    }
                }
            }
        }
    }
    // This tick's UI output (packets vanilla code sent, chat and commands typed).
    let mut ui_out = Vec::new();
    drain_packets(driver, shared, gone, server_packs_version)?;
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
        // A screen being open is vanilla's `handleKeybinds` gate (`Minecraft.tick` runs the key
        // bindings only when `gui.screen() == null`): no attack or use happens while a screen
        // holds the input, and clicks queued for this tick are dropped, not saved (the release
        // that ends a use that opened a screen is the screen's, and a held button must not
        // repeat the use through the server's block editors — the sign editor would reopen).
        let screen_open = shared.ui.lock().as_ref().is_some_and(|ui| ui.screen_open());
        // Held buttons stay; the press-only middle click, drop/swap presses and the hotbar choice
        // are consumed.
        let mut interact = shared_input.interact;
        // Vanilla runs `Minecraft.handleKeybinds` (the attack/use clicks and the held use)
        // only with no screen open (`Minecraft.tick`: `if (gui.overlay() == null &&
        // gui.screen() == null)`). A screen taking the mouse also owns the buttons: a use
        // held from before the screen opened must not keep firing (it would re-open the
        // container the screen shows, server-side).
        let attack_clicks;
        let use_clicks;
        if screen_open {
            interact.attack = false;
            interact.use_item = false;
            attack_clicks = 0;
            use_clicks = 0;
            shared_input.interact.attack_clicks = 0;
            shared_input.interact.use_clicks = 0;
        } else {
            // The click counts (vanilla's KeyMapping.clickCount) are drained into the player,
            // which consumes them in `Minecraft.handleKeybinds`' consumeClick loops.
            attack_clicks = std::mem::take(&mut shared_input.interact.attack_clicks);
            use_clicks = std::mem::take(&mut shared_input.interact.use_clicks);
        }
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
        for _ in 0..attack_clicks {
            driver.session.player.queue_attack_click();
        }
        for _ in 0..use_clicks {
            driver.session.player.queue_use_click();
        }
        SharedInput {
            toggle_flight: shared_input.toggle_flight,
            interact: shared_input.interact,
            first_person: shared_input.first_person,
            state: shared_input.state,
        }
    };
    // The horizontal distance travelled this tick, for the hand's walk bob: vanilla's
    // `LocalPlayer.move` adds `length(dx, dz) * 0.6` to `ClientAvatarState.walkDist`.
    let tick_moved = {
        let tick_start = driver.session.player.state.pos;
        // Write lock: the player's breaking predicts block changes into the world.
        let mut world = shared.world.write();
        let mut entities = shared.entities.write();
        // The world probe for the screen effects (vanilla's
        // `NetherPortalBlock.entityInside` and `LevelExtractor.getViewBlockingState`): the
        // portal under our feet feeds the portal effect's tick below, and the block the
        // camera is inside plus the eye's light go into this tick's view.
        {
            let p = &driver.session.player;
            let probe = crate::screen_effects::Probe {
                world: &*world,
                shapes: &shared.data.shapes,
                blocks: &shared.data.blocks,
                tags: p.int.block_tags(),
                eye: p.camera(1.0).0,
                feet: p.state.pos,
                bb_width: p.attrs.box_width(),
                bb_height: p.attrs.height_for(p.state.pose),
                scale: f64::from(p.attrs.scale),
            }
            .sample();
            driver.session.screen.set_inside_portal(probe.in_portal);
            // The local player's freeze ticks are client-predicted (the server does not send the
            // owner their own metadata): powder snow adds one, fire clears them.
            driver.session.player.own_flags.frozen_ticks = crate::screen_effects::tick_frozen(
                driver.session.player.own_flags.frozen_ticks,
                probe.in_powder_snow,
                probe.in_fire,
            );
            driver.session.screen_probe = probe;
        }
        driver.end_tick(&input.state, &mut *world, &mut *entities)?;
        // The weather effects' tick (`ClientLevel.tick` -> `tickWeatherEffects`, after the
        // entities): the rain splash particles join this tick's spawn list and the rain
        // sound joins the sound requests below. The levels' bracket shifts here too (the
        // render thread lerps it by partial tick).
        driver.session.weather.tick();
        if crate::net::WeatherState::can_have_weather(&driver.session.dimension) {
            let camera = driver.session.player.camera(1.0).0;
            driver.session.weather.tick_effects(
                camera,
                haste_render::weather::WEATHER_RADIUS,
                &world,
                &shared.biome_climates,
                driver.session.sea_level,
                driver.session.dimension.min_y,
                &mut driver.session.particle_spawns,
                &mut driver.session.sound_requests,
            );
        }
        let mut dirty = world.take_dirty_sections();
        if !dirty.is_empty() {
            shared.dirty.lock().append(&mut dirty);
        }
        let moved = driver.session.player.state.pos - tick_start;
        ((moved.x * moved.x + moved.z * moved.z).sqrt()) as f32
    };
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
    // The lids' open targets (vanilla routes `block_event` to the block entity).
    {
        let mut lids = shared.block_entity_lids.lock();
        lids.clear();
        driver.session.lid_states(&mut lids);
    }
    // The skulls' animation counters (vanilla ticks each `SkullBlockEntity.animation`:
    // while the block state is powered the counter grows, powering the jaw/ear swing).
    {
        let world = shared.world.read();
        let blocks = &shared.data.blocks;
        let names = &shared.block_entity_names;
        let mut anims = shared.skull_anims.lock();
        anims.retain_mut(|(pos, anim)| {
            // Gone (chunk unloaded, block replaced): drop. Otherwise a skull keeps its
            // entry; powered ones advance, unpowered ones freeze at the last value (vanilla's
            // counter stops but keeps its value).
            let Some(be) = world.block_entity(*pos) else {
                return false;
            };
            let is_skull = names
                .get(be.kind as usize)
                .is_some_and(|n| n.namespace() == "minecraft" && n.path() == "skull");
            if !is_skull {
                return false;
            }
            let powered = blocks
                .property_value(world.block_state(*pos), "powered")
                .is_some_and(|v| v == "true");
            anim.0 = anim.1;
            if powered {
                anim.1 += 1.0;
            }
            true
        });
        // The object-model block entities' animation state: the bell shakes, the decorated pot
        // wobbles and the conduit and enchanting-table ticks (vanilla's block entity tickers for
        // `BellBlockEntity`, `DecoratedPotBlockEntity`, `ConduitBlockEntity` and
        // `EnchantingTableBlockEntity`). The events come from the session's `block_event` handling;
        // everything else is driven from the world each tick, like the skulls above.
        {
            let tags = driver.session.player.int.block_tags();
            let mut bell_events = Vec::new();
            let mut pot_events = Vec::new();
            driver
                .session
                .block_event_anims(&mut bell_events, &mut pot_events);
            let view = shared.view.lock();
            let world_age = view.world_age;
            let feet = view.feet;
            drop(view);
            let mut anims = shared.block_entity_anims.lock();
            let mut rng = MODEL_RNG.lock();
            // The guard derefs to `&World` through the binding's type annotation.
            let world: &World = &world;
            tick_model_anims(TickModelAnims {
                anims: &mut anims,
                world,
                blocks,
                names,
                tags,
                world_age,
                feet: DVec3::new(feet.x, feet.y, feet.z),
                rng: &mut rng,
                bell_events: &bell_events,
                pot_events: &pot_events,
            });
        }
        // Newly powered skulls (loaded with powered=true, or toggled since) start at 0.
        'outer: for chunk in world.loaded_chunks() {
            for (pos, be) in world.block_entities_in(chunk) {
                let Some(name) = names.get(be.kind as usize) else {
                    continue;
                };
                if name.namespace() != "minecraft" || name.path() != "skull" {
                    continue;
                }
                let powered = blocks
                    .property_value(world.block_state(pos), "powered")
                    .is_some_and(|v| v == "true");
                if powered && !anims.iter().any(|(p, _)| p == &pos) {
                    anims.push((pos, (0.0, 0.0)));
                    if anims.len() > 1024 {
                        break 'outer;
                    }
                }
            }
        }
    }
    // The vaults' and spawners' spins (vanilla `VaultClientData.updateDisplayItemSpin` and
    // the spawners' client tick): every vault's displayed item turns 10 degrees a tick, a
    // spawner's display mob turns at the rate its state's spinning mob speed and its remaining
    // spawn delay give (a plain mob spawner only while a player is near it).
    {
        let world = shared.world.read();
        let blocks = &shared.data.blocks;
        let names = &shared.block_entity_names;
        let player = driver.session.player.state.pos;
        let game_time = driver.session.world_age;
        let mut spins = shared.block_entity_spins.lock();
        spins.retain_mut(|(pos, spin)| {
            let Some(be) = world.block_entity(*pos) else {
                return false;
            };
            let Some(name) = names.get(be.kind as usize) else {
                return false;
            };
            let minecraft = name.namespace() == "minecraft";
            let rate = if minecraft && name.path() == "vault" {
                // `VaultClientData.ROTATION_SPEED` (10 degrees a tick, wrapped).
                Some(10.0)
            } else if minecraft && matches!(name.path(), "mob_spawner" | "trial_spawner") {
                let trial = name.path() == "trial_spawner";
                if trial || mob_spawner_spins(&be.nbt, *pos, player) {
                    Some(spawner_spin_rate(
                        &be.nbt,
                        trial,
                        game_time,
                        blocks
                            .property_value(world.block_state(*pos), "trial_spawner_state")
                            .unwrap_or(""),
                    ))
                } else {
                    // No player in range: vanilla holds the spin (`oSpin = spin`).
                    Some(0.0)
                }
            } else {
                None
            };
            let Some(rate) = rate else {
                return false;
            };
            spin.0 = spin.1;
            if rate > 0.0 {
                // Vanilla's `(spin + rate) % 360`; a trial spawner in a state with no spinning
                // mob holds its spin.
                spin.1 = (spin.1 + rate).rem_euclid(360.0);
            }
            true
        });
        // New vaults and spawners start their spin at 0.
        'outer: for chunk in world.loaded_chunks() {
            for (pos, be) in world.block_entities_in(chunk) {
                let Some(name) = names.get(be.kind as usize) else {
                    continue;
                };
                if name.namespace() != "minecraft"
                    || !matches!(name.path(), "vault" | "mob_spawner" | "trial_spawner")
                    || spins.iter().any(|(p, _)| p == &pos)
                {
                    continue;
                }
                spins.push((pos, (0.0, 0.0)));
                if spins.len() > 1024 {
                    break 'outer;
                }
            }
        }
    }
    // Particles: spawn what the packets brought, then one 20 Hz simulation step. The camera
    // for `ClientLevel.doAddParticle`'s 32-block limiter is the player's eye.
    {
        let mut particles = shared.particles.lock();
        let camera = driver.session.player.camera(1.0).0;
        let spawns = std::mem::take(&mut driver.session.particle_spawns);
        // Block particles: our own break/mining/sprint and the server's level events.
        // The world (state, shapes, biomes) is read here, where the game thread holds it.
        let requests = std::mem::take(&mut driver.session.block_particle_requests);
        // The ambient block particles (`ClientLevel.animateTick`, vanilla runs it
        // right before the particle tick): torch flames, fire smoke, campfires, lava,
        // portals. Rain-gated parts (leaf drip) stay dormant until weather is decoded.
        let mut ambient = std::mem::take(&mut driver.session.ambient_spawns);
        {
            let world = shared.world.read();
            let data = shared.data.clone();
            let biomes = crate::app::biomes::WorldBiomes(&world);
            for s in &spawns {
                particles.spawn(s, camera, Some(&biomes));
            }
            driver.session.particle_spawned = spawns.len();
            let state_count = data.blocks.len();
            for r in &requests {
                match *r {
                    crate::block_particles::BlockParticleRequest::Destroy { pos, state } => {
                        // Vanilla's `Block.stateById` answers air (nothing spawns) for
                        // ids the state table does not know.
                        if (state.0 as usize) < state_count {
                            particles.spawn_destroy(pos, state, &data.shapes, &biomes);
                        }
                    }
                    crate::block_particles::BlockParticleRequest::Crack { pos, face } => {
                        let state = world.block_state(pos);
                        if !haste_physics::BlockPhysics::is_air(data.as_ref(), state) {
                            particles.spawn_crack(pos, face, state, &data.shapes, &biomes);
                        }
                    }
                    crate::block_particles::BlockParticleRequest::Sprint { feet, velocity } => {
                        // `getOnPosLegacy`: the support block 0.2 below the feet.
                        let support = haste_core::BlockPos::new(
                            feet.x.floor() as i32,
                            (feet.y - 0.2).floor() as i32,
                            feet.z.floor() as i32,
                        );
                        let state = world.block_state(support);
                        if !haste_physics::BlockPhysics::is_air(data.as_ref(), state) {
                            particles.spawn_sprint(feet, velocity, state, &data.shapes, &biomes);
                        }
                    }
                }
            }
            if driver.session.player.has_position {
                let feet = haste_core::BlockPos::from_dvec3(driver.session.player.state.pos);
                crate::ambient_particles::animate_tick(
                    &*world,
                    data.as_ref(),
                    feet,
                    false,
                    &mut ambient,
                );
            }
        }
        for s in &ambient {
            particles.spawn(s, camera, None);
        }
        driver.session.ambient_spawns = ambient;
        particles.tick();
    }
    if let Some(ui) = shared.ui.lock().as_ref() {
        // The hotbar authority is the player (the client picks with 1-9 and the wheel and tells
        // the server itself; `set_selected_slot` also marks it sent so the UI does not send it
        // a second time). The server's `set_held_slot` reaches both sides alike.
        ui.set_selected_slot(driver.session.player.held_slot);
        // A use opened a screen (command block, writable book): the UI opens it, gated like
        // vanilla's `Player.canUseGameMasterBlocks` (creative + operator level >= 2) for the
        // command block.
        match driver.session.player.take_used_screen() {
            Some(crate::player::interact::UsedScreen::CommandBlock(pos)) => {
                let (creative, level) = ui.game_master_state();
                if creative && level >= 2 {
                    // The editor's entity is built from the real state (the use itself opens
                    // no packet the block state would ride on; vanilla's chunk has it).
                    let state = WorldSink::block_state_at(&*shared.world.read(), pos);
                    ui.record_block_state(pos, state);
                    ui.open_used_command_block(pos);
                }
            }
            Some(crate::player::interact::UsedScreen::ItemGui(hand)) => ui.open_item_gui(hand),
            None => {}
        }
        ui.tick();
        // Inventory edits vanilla Java code predicted client-side (creative slot sets, menu
        // clicks, pick block): the server does not echo them back, so the client takes them
        // from the Java mirror. Without this the hand keeps the old item (the reported
        // creative-inventory bug: the hotbar shows the item, the hand does not).
        for (slot, stack) in ui.drain_inventory_changes() {
            driver.session.player.set_inventory_slot(slot, &stack);
        }
        // Every option a vanilla screen changed this tick reaches the running client live
        // (vanilla's `OptionInstance.onValueUpdate` callbacks, reported through the bridge).
        let changes = ui.drain_option_changes();
        if !changes.is_empty() {
            apply_option_changes(shared, driver, changes);
        }
        // The pack screen's Done: vanilla reloads its resource manager (Minecraft.
        // reloadResourcePacks); the renderer rebuilds from the new selection on its next
        // frame (AtlasBuilder + bake + GUI textures read the new asset stack).
        if let Some(ids) = ui.take_resource_pack_selection() {
            tracing::info!("resource pack selection changed: [{}]", ids.join(", "));
            let server = shared.applied_server_packs.lock().clone();
            *shared.resource_pack_reload.lock() = Some(PackReload { ids, server });
        }
        ui.drain_outgoing(&mut ui_out);
    }
    for p in ui_out {
        driver.queue(p);
    }
    // The per-tick camera effects (FOV modifier smoothing and the hurt and death counters;
    // the render thread interpolates them per frame). The view bobbing's walk distance and
    // bob are the hand's own, ticked below (vanilla's one `ClientAvatarState` serves both).
    driver
        .session
        .tick_camera(Some(shared.data.as_ref()), input.first_person);
    // The lightmap's per-tick state (vanilla `GameRenderer.tick` + the effect durations).
    driver.session.lightmap.tick();
    // The first-person hand's tick, in vanilla's order (`LivingEntity.baseTick`, then
    // `Player.tick`, then `FirstPersonHandsAndItems.tick`, with the walk bob in `aiStep` before
    // them); the interaction step above ran the attacks and uses that started the swings. In
    // spectator the hand is not drawn at all (vanilla hides it there), so it does not advance.
    let first_person = if driver.session.player.game_mode == GameMode::Spectator {
        None
    } else {
        let on_fire = driver.session.own_flags().on_fire || driver.session.screen_probe.in_fire;
        let p = &mut driver.session.player;
        let attack_speed = p.held_attack_speed();
        p.hand.tick(
            tick_moved,
            (p.state.yaw, p.state.pitch),
            p.state.on_ground,
            matches!(p.state.pose, haste_physics::Pose::Swimming),
            p.held_stack(),
            attack_speed,
        );
        let eye = p.camera(1.0).0;
        let hand = &p.hand;
        let mut fp =
            FirstPersonView::sample(&*shared.world.read(), eye, hand.shown_item(), on_fire);
        fp.with_hand(hand);
        Some(fp)
    };
    let s = &driver.session;
    let (prev_eye, prev_yaw, prev_pitch) = s.player.camera(0.0);
    let (eye, yaw, pitch) = s.player.camera(1.0);
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
    let prev_feet = view.prev_feet;
    view.cloak.tick(prev_feet);
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
    let lm = &s.lightmap;
    view.lightmap_flicker = lm.flicker;
    view.lightmap_night_vision = lm.night_vision;
    view.lightmap_darkness = lm.darkness;
    view.lightmap_darkness_blend = lm.darkness_blend;
    view.lightmap_boss_darkening = (lm.boss_darkening_o, lm.boss_darkening);
    view.lightmap_lightning = lm.sky_flash_time > 0;
    view.lightmap_tick_count = lm.tick_count;
    // The weather levels bracketing this tick (the render thread lerps by partial tick)
    // and the rain fog multiplier (`AtmosphericFogEnvironment.updateRainFogState` eased
    // by `deltaTicks * 0.2` toward rain level x sky-light factor x biome precipitation).
    {
        let w = &s.weather;
        view.rain_level = (w.rain_level_o, w.rain_level);
        view.thunder_level = (w.thunder_level_o, w.thunder_level);
        let target = if crate::net::WeatherState::can_have_weather(&s.dimension) {
            let world = shared.world.read();
            let precipitation = if biome_has_precipitation(&world, &shared.biome_climates, eye) {
                1.0
            } else {
                0.5
            };
            w.rain_level(1.0) * rain_fog_sky_light(&world, eye) * precipitation
        } else {
            0.0
        };
        view.rain_fog_multiplier += (target - view.rain_fog_multiplier) * 0.2;
        view.sea_level = s.sea_level;
        view.horizon_height =
            haste_render::sky::dark_disc::horizon_height(s.is_flat, s.dimension.min_y);
    }
    view.eye_height = s.player.eye_height();
    view.time_of_day = s.time_of_day.unwrap_or(6000);
    view.world_age = s.world_age;
    view.has_sky = s.dimension.has_skylight;
    view.block_target = s.player.target();
    view.eye_in_water = s.player.state.eyes_in_water;
    view.eye_in_lava = eye_in_lava(&shared.world.read(), &shared.data, eye);
    // The fog environment's player state (vanilla `FogRenderer.setupFog` reads these off the
    // entity): blindness and fire resistance from the active effects, spectator from the mode,
    // the water-vision ramp, and the boss overlay's world fog.
    let eff = |id: Option<i32>| {
        id.and_then(|i| s.player.int.effects.get(&i))
            .map(|e| e.duration)
    };
    view.blindness = eff(s.player.int.lightmap_effect_ids.blindness);
    view.fire_resistance = eff(s.player.int.lightmap_effect_ids.fire_resistance).is_some();
    view.spectator = s.player.game_mode == GameMode::Spectator;
    view.water_vision = s.player.get_water_vision();
    view.boss_world_fog = s.lightmap.boss_world_fog;
    view.held_slot = s.player.held_slot;
    view.first_person = first_person;
    view.fov_modifier = s.fov_modifier;
    view.fov_modifier_old = s.old_fov_modifier;
    // The view bobbing's walk distance and bob are the hand's own (vanilla's one
    // `ClientAvatarState` ticks both).
    let (walk_dist_o, walk_dist) = s.player.hand.walk_dist();
    view.walk_dist = walk_dist;
    view.walk_dist_o = walk_dist_o;
    let (bob_o, bob) = s.player.hand.bob();
    view.bob = bob;
    view.bob_o = bob_o;
    view.hurt_ticks = s.hurt_ticks;
    view.hurt_dir = s.hurt_dir;
    view.death_ticks = s.death_ticks;
    view.dead = s.player.health <= 0.0;
    // The screen effects, from this tick's world probe (the block the camera is inside, the
    // eye's light), the own entity's flags and freeze ticks, the worn head item and the active
    // effects. The eyes being in water comes from the physics' fluid at the eye position.
    let head_item = s.head_item().and_then(|i| {
        shared
            .data
            .registries
            .name("minecraft:item", i)
            .cloned()
            .and_then(|name| crate::screen_effects::camera_overlay(&name))
    });
    let screen = s.screen_effects();
    view.screen = crate::screen_effects::ViewEffects {
        block_overlay: s.screen_probe.block_overlay.clone(),
        eye_in_water: view.eye_in_water,
        eye_light: s.screen_probe.eye_light,
        // The client predicts its own fire from the fire block it overlaps
        // (`BaseFireBlock.fireIgnite`); the server never sends the owner their own
        // `set_entity_data`, so `own_flags.on_fire` stays false and the probe carries it.
        on_fire: s.own_flags().on_fire || s.screen_probe.in_fire,
        camera_overlay: head_item,
        frozen_ticks: s.own_flags().frozen_ticks,
        scoping: s.player.scoping(&shared.data),
        portal: (screen.prev_portal(), screen.portal()),
        nausea: s.player.int.nausea_blend_pair(),
        spinning_time: screen.spinning_time(),
        spinning_speed: screen.spinning_speed(),
    };
    Ok(())
}

/// Whether the camera is under the lava surface (vanilla's `Camera.getFluidInCamera` lava
/// rule: lava at the eye's block whose surface reaches the eye). The surface is the fluid's
/// own height, or the full block when lava sits directly above (vanilla's `getHeight`); the
/// comparison is `<=` as vanilla's `offsetPos.y <= height + y`. Vanilla probes the five
/// near-plane points rather than the eye block alone; that near-plane fan (which needs the
/// view direction and aspect) is not modelled.
fn eye_in_lava(world: &World, data: &GameData, eye: DVec3) -> bool {
    use haste_core::view::{BlockPhysics, FluidKind};
    let p = BlockPos::new(
        eye.x.floor() as i32,
        eye.y.floor() as i32,
        eye.z.floor() as i32,
    );
    let Some(f) = data.fluid(world.block_state(p)) else {
        return false;
    };
    if f.kind != FluidKind::Lava {
        return false;
    }
    let full = data
        .fluid(world.block_state(p.add(0, 1, 0)))
        .is_some_and(|a| a.kind == FluidKind::Lava);
    let surface = p.y as f64 + if full { 1.0 } else { f.height as f64 };
    eye.y <= surface
}

/// Vanilla `BaseSpawner.isNearPlayer`: an alive player within the spawner's required player
/// range of its centre. The client has only its own player, so a spawner spins its mob exactly
/// when the camera is close, like vanilla's single-player client. Only `BaseSpawner.clientTick`
/// checks this; a trial spawner's `tickClient` spins its mob in every state that has one.
fn mob_spawner_spins(nbt: &[u8], pos: BlockPos, player: DVec3) -> bool {
    let centre = DVec3::new(pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5);
    // `BaseSpawner.requiredPlayerRange` is the block entity's `RequiredPlayerRange` tag, not a
    // block state property; vanilla writes it as a short and reads it back leniently.
    let range = read_block_entity_nbt(nbt)
        .and_then(|doc| doc.root_tag.get_numeric_short("RequiredPlayerRange"))
        .unwrap_or(16) as f64;
    player.distance_squared(centre) <= range * range
}

/// The degrees a spawner's display mob turns in one tick, vanilla `BaseSpawner.clientTick`'s
/// `1000 / (spawnDelay + 200)` and `TrialSpawner.tickClient`'s
/// `spinningMobSpeed / (spawnDelay + 200)`. A trial spawner's spinning mob speed comes from
/// its block state (`TrialSpawnerState`: 200 waiting for players, 1000 active, -1 for the
/// states with no spinning mob) and its remaining spawn delay from `next_mob_spawns_at`.
fn spawner_spin_rate(nbt: &[u8], trial: bool, game_time: i64, state: &str) -> f32 {
    // The trial spawner's state decides whether a mob spins at all (vanilla
    // `TrialSpawnerState.hasSpinningMob`).
    let speed = if trial {
        trial_spinning_mob_speed(state)
    } else {
        1000.0
    };
    if speed <= 0.0 {
        return 0.0;
    }
    let delay = if trial {
        trial_spawn_delay(nbt, game_time)
    } else {
        spawner_delay(nbt)
    };
    (speed / (delay.max(0) as f64 + 200.0)) as f32
}

/// Vanilla `BaseSpawner.spawnDelay`, from the block entity's `Delay` tag (a short, default 20).
fn spawner_delay(nbt: &[u8]) -> i32 {
    read_block_entity_nbt(nbt)
        .and_then(|doc| doc.root_tag.get_short("Delay"))
        .unwrap_or(20) as i32
}

/// Vanilla `TrialSpawnerStateData.nextMobSpawnsAt` as the remaining delay
/// (`max(0, nextMobSpawnsAt - level.getGameTime())`).
fn trial_spawn_delay(nbt: &[u8], game_time: i64) -> i32 {
    read_block_entity_nbt(nbt)
        .and_then(|doc| doc.root_tag.get_long("next_mob_spawns_at"))
        .map(|at| (at - game_time).max(0) as i32)
        .unwrap_or(0)
}

/// Vanilla `TrialSpawnerState.spinningMobSpeed` for the state the block is in: waiting for
/// players 200, active 1000, every other state has no spinning mob (-1).
fn trial_spinning_mob_speed(state: &str) -> f64 {
    match state {
        "waiting_for_players" => 200.0,
        "active" => 1000.0,
        _ => -1.0,
    }
}

/// A block entity's NBT payload (the network variant `read_unnamed` reads), or `None`.
fn read_block_entity_nbt(nbt: &[u8]) -> Option<haste_protocol::nbt::Nbt> {
    use haste_protocol::nbt::deserializer::NbtReadHelperJava;
    let mut cursor = std::io::Cursor::new(nbt);
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    haste_protocol::nbt::Nbt::read_unnamed(&mut helper).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use haste_physics::testing::{states, TestShapes, TestWorld};

    #[test]
    fn first_person_view_change_detection() {
        // With the skin fixed, view equality is the whole "did the hand change?" check:
        // equal views produce equal hands, differing ones differing hands.
        let skin = haste_entity::skin::steve();
        let skin = &*skin;
        let a = FirstPersonView::sample(&lit(), DVec3::ZERO, Some(RegistryIndex(5)), false);
        let mut b = a;
        b.item = None;
        assert_eq!(a, a);
        assert_ne!(a, b);
        let ha = a.with_skin(skin, true);
        let hb = b.with_skin(skin, true);
        assert_eq!(ha, a.with_skin(skin, true));
        assert_ne!(ha, hb);
        assert_eq!(ha.skin, Some(skin.body.clone()));
        assert!(!ha.slim);
        assert!(ha.sleeve);
        assert_eq!(ha.item, Some(RegistryIndex(5)));
        // Sampled at full sky light.
        assert_eq!(ha.light, (15, 0));
        // A differing light also differs (the eye moved into shade).
        let mut c = a;
        c.light = (4, 7);
        assert_ne!(a, c);
        assert_ne!(ha, c.with_skin(skin, true));
    }

    /// A world at full sky light, for sampling the hand's light.
    fn lit() -> LightWorld {
        LightWorld {
            sky: 15,
            block: 0,
            asked: parking_lot::Mutex::new(Vec::new()),
        }
    }

    #[test]
    fn the_view_carries_the_hand_animation_to_the_renderer() {
        // The per-tick pairs go through to the renderer's hand unchanged, so a swinging hand
        // (or a dipping equip progress, or the walk bob) is a different hand each tick.
        let skin = haste_entity::skin::PlayerSkin::new(
            haste_core::Identifier::minecraft("entity/player/slim/steve"),
            None,
            haste_entity::skin::PlayerModelType::Slim,
            true,
        );
        let mut hand = crate::hand::Hand::new();
        hand.swing();
        let mut v = FirstPersonView::sample(&lit(), DVec3::ZERO, None, false);
        // Two ticks of walking 0.2 blocks with the look at (10, 5) and the hand swinging.
        for _ in 0..2 {
            hand.tick(
                0.2,
                (10.0, 5.0),
                true,
                false,
                crate::hand::HeldStack::of(None),
                4.0,
            );
        }
        v.with_hand(&hand);
        let moving = v.with_skin(&skin, true);
        // The skin's slim arm is passed through.
        assert!(moving.slim);
        // The swing is `SwingState.ticks / 6` through: 0 on the attack tick, 1/6 on the next.
        assert_eq!(v.attack, (0.0, 1.0 / 6.0));
        assert_eq!(moving.attack.start, 0.0);
        assert!((moving.attack.end - 1.0 / 6.0).abs() < 1e-7);
        // Walking feeds the walk bob (the camera's, no longer the hand's own fields); the
        // hand only carries the view lag by half the remaining angle each tick.
        let (bob_o, bob) = hand.bob();
        assert!((bob - 0.04).abs() < 1e-6, "{bob_o} -> {bob}");
        let (walk_o, walk) = hand.walk_dist();
        assert!((walk - 0.24).abs() < 1e-6, "{walk_o} -> {walk}");
        assert!((v.y_bob.1 - 7.5).abs() < 1e-6);
        assert!((v.x_bob.1 - 3.75).abs() < 1e-6);
        // The next tick's step is a different hand, so the renderer is told again.
        hand.tick(
            0.2,
            (10.0, 5.0),
            true,
            false,
            crate::hand::HeldStack::of(None),
            4.0,
        );
        let mut next = v;
        next.with_hand(&hand);
        assert_ne!(next, v);
        assert_ne!(next.with_skin(&skin, true), moving);
    }

    #[test]
    fn first_person_sample_reads_the_eye_block_light() {
        // Sky 12, block 7: sampled at the eye's block, in FirstPerson::light's order.
        let world = LightWorld {
            sky: 12,
            block: 7,
            asked: parking_lot::Mutex::new(Vec::new()),
        };
        let v = FirstPersonView::sample(
            &world,
            DVec3::new(3.2, 65.5, -7.8),
            Some(RegistryIndex(1)),
            false,
        );
        assert_eq!(v.item, Some(RegistryIndex(1)));
        assert_eq!(v.light, (12, 7));
        // The eye at y = 65.5 samples the block containing it, y = 65
        // (`Entity.getLightProbePosition` is the eye position, floored by `BlockPos.containing`),
        // not the block above: in a 2-high tunnel the eye at feet + 1.62 is in the lit upper
        // block and the hand must not take the ceiling's darkness.
        assert_eq!(world.asked.lock().clone(), vec![BlockPos::new(3, 65, -8)]);
    }

    #[test]
    fn first_person_sample_on_fire_forces_full_block_light() {
        // `EntityRenderer.getBlockLightLevel`: 15 while on fire, sky light still from the world.
        let world = LightWorld {
            sky: 9,
            block: 2,
            asked: parking_lot::Mutex::new(Vec::new()),
        };
        let v = FirstPersonView::sample(&world, DVec3::new(0.5, 70.9, 0.5), None, true);
        assert_eq!(v.light, (9, 15));
        let v = FirstPersonView::sample(&world, DVec3::new(0.5, 70.9, 0.5), None, false);
        assert_eq!(v.light, (9, 2));
    }

    /// A world that answers one sky/block light pair and records where it was asked.
    #[derive(Default)]
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
    fn behind_wall_distance_is_measured_straight_line_from_the_eye() {
        let mut w = TestWorld::new();
        // Looking west, so the behind camera goes east; stone cell (2, 0, 0) puts its
        // outline's west face at x = 2, 1.5 blocks ahead of the eye.
        w.set(2, 0, 0, states::STONE);
        let s = TestShapes::default();
        let (pos, yaw, _) =
            third_person_camera(Perspective::ThirdPersonBehind, EYE, 90.0, 0.0, &w, &s);
        assert_eq!(yaw, 90.0);
        // All eight corner rays hit the face: the hit is 1.5 blocks along the ray but
        // sqrt(1.5^2 + 0.1^2 + 0.1^2) = sqrt(2.27) from the eye, and no clearance margin is
        // subtracted, so the camera stops epsilon inside the wall (vanilla's
        // `Camera.getMaxZoom`).
        let want = 0.5 + (2.27f64).sqrt();
        assert!((pos.x - want).abs() < EPS, "{pos:?}");
        assert!(pos.x > 2.0, "{pos:?}");
        assert!(
            (pos.y - 0.5).abs() < EPS && (pos.z - 0.5).abs() < EPS,
            "{pos:?}"
        );
    }

    #[test]
    fn diagonal_wall_takes_the_shortest_of_the_eight_corner_rays() {
        let mut w = TestWorld::new();
        // A diagonal wall one cell behind and to the side: every corner ray meets its x = 1
        // face, but at straight-line distances from the eye of sqrt(0.75), sqrt(0.99) and
        // sqrt(1.31) — the shortest wins.
        w.set(1, 0, 1, states::STONE);
        let s = TestShapes::default();
        let eye = DVec3::new(0.3, 0.5, 0.7);
        let (pos, yaw, pitch) =
            third_person_camera(Perspective::ThirdPersonBehind, eye, 135.0, 0.0, &w, &s);
        assert_eq!((yaw, pitch), (135.0, 0.0));
        let back = -haste_physics::raycast::look_vector(135.0, 0.0);
        let want = eye + back * (0.75f64).sqrt();
        assert!((pos - want).length() < 1e-5, "{pos:?} != {want:?}");
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
    fn a_block_the_eye_starts_inside_pulls_the_camera_to_its_corner() {
        let mut w = TestWorld::new();
        // The eye is inside this block: all eight corner rays start inside its outline, and
        // vanilla counts that as a hit at the corner, 0.1 * sqrt(3) from the eye.
        w.set(0, 0, 0, states::STONE);
        let s = TestShapes::default();
        let (pos, _, _) =
            third_person_camera(Perspective::ThirdPersonBehind, EYE, -90.0, 0.0, &w, &s);
        assert!((pos.x - (0.5 - (0.03f64).sqrt())).abs() < EPS, "{pos:?}");
    }

    /// A forgotten chunk lists every section it had, so the dirty set reaches the renderer
    /// and its resident meshes are freed (vanilla drops the render data with the chunk,
    /// `ClientChunkCache.drop`). Needs the data generator reports in the cache.
    #[test]
    #[ignore = "needs cache/datagen/26.3/reports (GameData::load)"]
    fn tracked_world_lists_gone_sections() {
        let data = crate::net::load_game_data().unwrap();
        let dim = DimensionInfo::overworld();
        let mut world = World::new_with_biomes(dim.clone(), data.clone(), 8);
        let c = haste_world::chunk::synthetic_chunk_data(
            ChunkPos::new(3, -2),
            dim.min_section(),
            dim.section_count(),
            |s, _, _, _| {
                if s < 2 {
                    BlockStateId(1)
                } else {
                    BlockStateId::AIR
                }
            },
        );
        WorldSink::load_chunk(&mut world, &c).unwrap();
        let mut gone = Vec::new();
        let mut tracked = TrackedWorld {
            world: &mut world,
            gone: &mut gone,
        };
        tracked.unload_chunk(ChunkPos::new(3, -2));
        assert!(world.chunk(ChunkPos::new(3, -2)).is_none());
        // Every section of the chunk, exactly once, and nothing else.
        let min = dim.min_section();
        let expect: std::collections::HashSet<SectionPos> = (min..min + dim.section_count() as i32)
            .map(|y| SectionPos::new(3, y, -2))
            .collect();
        assert_eq!(gone.len(), expect.len());
        assert!(gone.iter().all(|p| expect.contains(p)));
        // The world itself dirties only the neighbours: without `gone` riding the dirty
        // set, no packet would ever mention these sections again.
        assert!(world
            .take_dirty_sections()
            .into_iter()
            .all(|p| p.chunk() != ChunkPos::new(3, -2)));
    }
}
