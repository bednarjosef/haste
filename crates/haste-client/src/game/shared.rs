//! The state the game thread and the render thread share: one `Shared` per connection, holding
//! the world and entity store behind locks, the per-frame view values and the HUD, sound and
//! particle outputs the tick drives.

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;
use std::time::Instant;

use glam::DVec3;
use haste_data::GameData;
use haste_entity::EntityStore;
use haste_protocol::haste_core::input::{InputState, InteractInput};
use haste_protocol::haste_core::{BlockPos, Identifier, SectionPos};
use haste_world::World;
use parking_lot::{Condvar, Mutex, RwLock};

use super::camera::Perspective;
use super::skins;
use super::view::ViewState;
use crate::ui::Ui;

/// The particle pool: filled from `level_particles` packets (the renderer's atlas-backed
/// particle types, so unknown/mod types are skipped there), ticked at 20 Hz by the game
/// thread, which then publishes [`Shared::particle_snapshots`] for the render thread (no
/// GPU state inside).
pub type SharedParticles = Mutex<haste_render::Particles>;

/// Input sampled by the render thread from winit, read by the game thread once per tick.
#[derive(Clone, Copy, Debug, Default)]
pub struct SharedInput {
    /// Keys and the input layer's look angles (without the server's offset).
    pub state: InputState,
    /// Double-tap jump seen since the last tick (`LocalPlayer::toggle_flight`).
    pub toggle_flight: bool,
    /// Mouse buttons held, middle click and hotbar selection since the last tick.
    pub interact: InteractInput,
    /// The current perspective (F5 cycles it). The sound listener and the spyglass FOV
    /// read it: both follow the camera, which only sits at the eye in first person.
    pub perspective: Perspective,
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
    /// Bumped every time the world changed (dirty sections published); the render thread's
    /// per-tick caches — the weather extraction's column heights and precipitation — refresh
    /// only when it moves, instead of re-reading the world every frame.
    pub world_version: AtomicU64,
    /// Per-frame view values.
    pub view: Mutex<ViewState>,
    /// The local player's own model state (the server never spawns us as a tracked entity):
    /// ticked per tick by the game thread ([`super::player_model::tick`]) from the client's
    /// own state, drawn by the render thread through the same `setupAnim`/`setupRotations`
    /// path a remote player takes. Short lock, same pattern as [`Shared::view`].
    pub local_model: Mutex<haste_entity::Entity>,
    /// Break cracks to draw (`Session::breaking_stages`), copied once per tick.
    pub breaking: Mutex<Vec<(BlockPos, u8)>>,
    /// The pickup particles (`take_item_entity`), copied once per tick from the session; the
    /// render thread appends one item/orb instance per particle at the eased position.
    pub pickup_particles: Mutex<Vec<crate::net::pickup::PickupParticle>>,
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
    /// The game thread's last extracted block entity render data (vanilla's per-tick
    /// `extractRenderState`): the drawable instances with tick-end values, replaced once per
    /// tick by the extraction (`app::block_entity_extract`). The render thread clones the
    /// `Arc` per frame and folds the partial tick in.
    pub block_entity_tick: Mutex<Arc<crate::app::block_entity_extract::BlockEntityTickFrame>>,
    /// The renderer's text metrics (the font, for wrapping sign lines), set by the render
    /// thread on its first frame; the game thread's sign text parsing waits for it (signs
    /// draw without text until then, retried every tick).
    pub sign_text_metrics: std::sync::OnceLock<Arc<dyn haste_core::gui::TextMetrics>>,
    /// Biome climates of the synced registry (index = biome id), for the fog's per-biome
    /// environment attributes (`FogRenderer` resolves them from the camera biome).
    pub biome_climates: Vec<haste_render::BiomeClimate>,
    /// Particles (`level_particles`): the game thread spawns and ticks them, the render
    /// thread draws them.
    pub particles: SharedParticles,
    /// The particles' per-tick render snapshot: written by the game thread's particle
    /// tick (under the locks it already holds), taken by the render thread each frame
    /// (an `Arc` copy; no world read lock, no particle mutex, no waiting on the tick).
    pub particle_snapshots: haste_render::ParticleSnapshots,
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
    /// The server resource pack confirm screen in play (vanilla's `PackConfirmScreen`).
    pub pack_prompt: Mutex<crate::net::pack_prompt::PlayPrompt>,
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
    /// `minecraft:block_entity_type` names by synced id (see [`Shared::block_entity_names`]);
    /// `local` is our own entity id and profile id, for the own model state.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        data: Arc<GameData>,
        world: World,
        entities: EntityStore,
        local: (haste_core::EntityId, uuid::Uuid),
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
            world_version: AtomicU64::new(0),
            local_model: Mutex::new(haste_entity::Entity::local_player(
                local.0,
                local.1,
                DVec3::ZERO,
                0.0,
                0.0,
            )),
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
                bb: (0.6, 1.8),
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
            pickup_particles: Mutex::new(Vec::new()),
            block_entity_lids: Mutex::new(Vec::new()),
            skull_anims: Mutex::new(Vec::new()),
            block_entity_spins: Mutex::new(Vec::new()),
            block_entity_anims: Mutex::new(std::collections::HashMap::new()),
            block_entity_names,
            block_entity_tick: Mutex::new(Arc::new(
                crate::app::block_entity_extract::BlockEntityTickFrame::default(),
            )),
            sign_text_metrics: std::sync::OnceLock::new(),
            biome_climates,
            particles: Mutex::new(particles),
            particle_snapshots: haste_render::ParticleSnapshots::default(),
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
            pack_prompt: Mutex::default(),
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
