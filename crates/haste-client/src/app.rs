//! Windowed client and screenshot mode (client-app).
//!
//! `haste --server 127.0.0.1:25611` joins, starts the game thread (`game::run`, 20 Hz) and runs
//! the winit event loop on the main thread: each frame it feeds dirty sections to the renderer
//! (`WorldFeed`), copies the interpolated entity instances, builds the camera from the
//! interpolated player position and renders. `--screenshot <file>` renders the same frame
//! offscreen once the world has finished meshing, saves it and exits.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use glam::DVec3;
use haste_protocol::haste_core::gui::{GuiCommand, GuiFrame};
use haste_protocol::haste_core::input::InputState;
use haste_protocol::haste_core::model::EntityInstance;
use haste_protocol::haste_core::{ChunkPos, EntityId, SectionPos};
use haste_protocol::{Account, ConnectOptions};
use haste_render::{Camera, FrameInput, FrameStats, Renderer, SkyState};
use haste_setup::options::Options;
use haste_world::World;
use winit::application::ApplicationHandler;
use winit::event::{
    DeviceEvent, DeviceId, ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Fullscreen, Window, WindowId};

// Declared here rather than in main.rs (lead-owned); the lead may move it.
#[path = "biomes.rs"]
mod biomes;

use crate::game::{self, Shared};
use crate::net::{self, NetDriver};
use crate::script;
use crate::ui::keys;
use crate::Args;

/// Saved options this run uses: FOV, render distance, mouse sensitivity and (unless the
/// `--vsync` flag says otherwise) vsync. `Config::load` creates the file with defaults on
/// first run, so the fallback only fires on a broken file.
fn load_options() -> haste_setup::options::Options {
    match haste_setup::config::Config::load() {
        Ok(c) => c.options,
        Err(e) => {
            tracing::warn!("config load failed, using default options: {e:#}");
            Options::default()
        }
    }
}

/// Degrees of rotation per pixel of mouse motion at sensitivity 0..=1. Vanilla's curve, as
/// the client community documents it (minecraft.wiki "Options" > Sensitivity documents the
/// 0..=1 range): `f = s * 0.6 + 0.2`, `f * f * f * 8 * 0.15` degrees per pixel.
pub fn look_deg_per_pixel(sensitivity: f32) -> f32 {
    let f = sensitivity * 0.6 + 0.2;
    f * f * f * 8.0 * 0.15
}
/// Two jump presses closer than this toggle flight (vanilla: 7 ticks).
const DOUBLE_TAP: Duration = Duration::from_millis(350);
/// Wall time the render thread may spend capturing sections for the mesher per frame.
/// Sections handed to the mesh workers per frame (they capture the snapshots themselves).
const SUBMIT_PER_FRAME: usize = 512;
/// The GUI scale the option asks for: 0 means auto (vanilla: `Options.AUTO_GUI_SCALE`).
/// Like `Minecraft.resizeGui`, auto and any fixed scale both go through
/// [`gui_scale_for`] (vanilla: `Window.calculateScale`).
pub fn resolve_gui_scale(option: u32, width: u32, height: u32) -> u32 {
    gui_scale_for(option.min(4), width, height)
}

/// The scale vanilla would use for the option at the window's framebuffer size
/// (vanilla: `Window.calculateScale`; `max_scale` 0 grows it while the framebuffer
/// keeps at least 320x240 GUI pixels).
pub fn gui_scale_for(max_scale: u32, width: u32, height: u32) -> u32 {
    let mut scale = 1;
    while scale != max_scale
        && scale < width
        && scale < height
        && width / (scale + 1) >= 320
        && height / (scale + 1) >= 240
    {
        scale += 1;
    }
    scale
}

/// The GUI-scaled size at `scale` (vanilla: `Window.setGuiScale` uses ceil division).
pub fn gui_scaled_size(width: u32, height: u32, scale: u32) -> (u32, u32) {
    (width.div_ceil(scale.max(1)), height.div_ceil(scale.max(1)))
}

/// F3's numbers for this frame: the cheap part (position, counts) always; the strings (biome,
/// targeted block/fluid with properties and tags) only while the overlay is on, like vanilla
/// building its lines only then. `look` is the yaw/pitch the player is rendered with.
#[allow(clippy::too_many_arguments)]
fn debug_info(
    env: &DebugEnv,
    shared: &Shared,
    renderer: &Renderer,
    look: (f32, f32),
    fps: i32,
    stats: &FrameStats,
    entities_rendered: i32,
    gui_scale: u32,
    gpu: &str,
) -> haste_jvm::DebugInfo {
    use haste_core::view::{BlockPhysics as _, BlockView};
    use haste_protocol::haste_core::BlockPos;
    use haste_render::BiomeSource as _;
    let view = *shared.view.lock();
    let world = shared.world.read();
    // The full look the player is rendered with (input look plus the server's offset), like
    // frame_view; vanilla's F3 shows the entity's rotation.
    let look = (
        look.0 + view.look_offset.0,
        (look.1 + view.look_offset.1).clamp(-90.0, 90.0),
    );
    let mut d = haste_jvm::DebugInfo {
        fps,
        pos: [view.feet.x, view.feet.y, view.feet.z],
        rot: [look.0, look.1],
        chunks: world.loaded_chunks().count() as i32,
        sections_drawn: stats.chunk_sections_drawn as i32,
        sections_total: stats.chunk_sections_total as i32,
        entities: shared.entities.read().len() as i32,
        entities_rendered,
        simulation_distance: env.simulation_distance,
        view_distance: view.view_distance,
        framerate_limit: 260,
        refresh_rate: 0,
        display_size: [renderer.size().0 as i32, renderer.size().1 as i32],
        gui_scale: gui_scale as i32,
        light: [0, 0],
        ..Default::default()
    };
    if !shared.ui.lock().as_ref().is_some_and(|ui| ui.debug_shown()) {
        return d;
    }
    d.gpu = gpu.to_owned();
    d.cpu = cpu_info();
    d.dimension = world.dimension().name.to_string();
    // Biome and light at the feet position (vanilla DebugEntryBiome/DebugEntryLight).
    let feet = BlockPos::new(
        view.feet.x.floor() as i32,
        view.feet.y.floor() as i32,
        view.feet.z.floor() as i32,
    );
    d.biome = {
        let id = self::biomes::WorldBiomes(&world).biome(feet).0 as usize;
        env.biome_names
            .get(id)
            .cloned()
            .unwrap_or_else(|| "minecraft:plains".to_owned())
    };
    d.light = [
        i32::from(world.sky_light(feet)),
        i32::from(world.block_light(feet)),
    ];
    // The looking-at block/fluid (vanilla Entity.pick, 20 blocks, block without fluids, fluid
    // with them): both rays from the interpolated eye along the look direction.
    let partial = game::partial_tick(view.tick_at, Instant::now());
    let eye = game::interpolate(view.prev_eye, view.eye, partial);
    let dir = haste_physics::raycast::look_vector(look.0, look.1);
    let data = &shared.data;
    if let Some(hit) = haste_physics::raycast::raycast_blocks(eye, dir, 20.0, &*world, &data.shapes)
    {
        let state = world.block_state(hit.pos);
        d.target_block = Some(haste_jvm::TargetedState {
            name: data.blocks.block_of(state).to_string(),
            pos: [hit.pos.x, hit.pos.y, hit.pos.z],
            properties: data.blocks.properties(state),
            tags: data
                .blocks
                .block_index(data.blocks.block_of(state))
                .and_then(|r| env.block_tags.get(&r.0))
                .cloned()
                .unwrap_or_default(),
        });
    }
    d.target_fluid = raycast_fluid(eye, dir, 20.0, &world, data).map(|(state, pos)| {
        let info = data.fluid(state).expect("checked: has a fluid");
        let name = match info.kind {
            haste_core::view::FluidKind::Water => "minecraft:water",
            haste_core::view::FluidKind::Lava => "minecraft:lava",
        };
        haste_jvm::TargetedState {
            name: name.to_owned(),
            pos: [pos.x, pos.y, pos.z],
            properties: data
                .blocks
                .properties(state)
                .into_iter()
                .filter(|(k, _)| k == "level")
                .collect(),
            tags: haste_protocol::haste_core::Identifier::parse(name)
                .ok()
                .and_then(|n| data.registries.index("minecraft:fluid", &n))
                .and_then(|r| env.fluid_tags.get(&r.0))
                .cloned()
                .unwrap_or_default(),
        }
    });
    d.tx_per_s = f64::from_bits(shared.debug_tx.load(Ordering::Relaxed));
    d.rx_per_s = f64::from_bits(shared.debug_rx.load(Ordering::Relaxed));
    d.brand = "vanilla".to_owned();
    d
}

/// The first fluid cell the segment `from .. from + dir * reach` touches (vanilla
/// `Entity.pick(withLiquids)`: the first block whose fluid state is non-empty). Returns the
/// block state (it carries the fluid and its `level` property) and the cell position.
fn raycast_fluid(
    from: DVec3,
    dir: DVec3,
    reach: f64,
    world: &World,
    data: &haste_data::GameData,
) -> Option<(
    haste_protocol::haste_core::BlockStateId,
    haste_protocol::haste_core::BlockPos,
)> {
    use haste_core::view::{BlockPhysics as _, BlockView};
    let delta = dir * reach;
    let mut cell = from.floor().as_ivec3();
    let end = (from + delta).floor().as_ivec3();
    let step = delta.signum().as_ivec3();
    let mut t_max = DVec3::ZERO;
    let mut t_delta = DVec3::ZERO;
    for a in 0..3 {
        if delta[a] == 0.0 {
            t_max[a] = f64::INFINITY;
            t_delta[a] = f64::INFINITY;
        } else {
            let next = if delta[a] > 0.0 {
                cell[a] as f64 + 1.0
            } else {
                cell[a] as f64
            };
            t_max[a] = (next - from[a]) / delta[a];
            t_delta[a] = 1.0 / delta[a].abs();
        }
    }
    let max_cells = (end - cell).abs().element_sum() + 1;
    for _ in 0..=max_cells {
        let pos = haste_protocol::haste_core::BlockPos::new(cell.x, cell.y, cell.z);
        let state = world.block_state(pos);
        if data.fluid(state).is_some() {
            return Some((state, pos));
        }
        if cell == end {
            break;
        }
        let a = if t_max.x < t_max.y {
            if t_max.x < t_max.z {
                0
            } else {
                2
            }
        } else if t_max.y < t_max.z {
            1
        } else {
            2
        };
        if t_max[a] > 1.0 {
            break;
        }
        cell[a] += step[a];
        t_max[a] += t_delta[a];
    }
    None
}

/// CPU as vanilla's DebugEntrySystemSpecs prints it ("<logical>x <model name>"), read once.
fn cpu_info() -> String {
    static CPU: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CPU.get_or_init(|| {
        let model = std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|c| {
                c.lines().find_map(|l| {
                    l.strip_prefix("model name")
                        .and_then(|r| r.split_once(':'))
                        .map(|(_, n)| n.trim().to_owned())
                })
            })
            .unwrap_or_else(|| "unknown".to_owned());
        format!(
            "{}x {}",
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            model
        )
    })
    .clone()
}

/// The GPU as F3's system group prints it ("name (backend)"), read from the adapter once.
fn gpu_info(renderer: &Renderer) -> String {
    static GPU: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    GPU.get_or_init(|| {
        let i = renderer.adapter_info();
        format!("{} ({})", i.name, i.backend)
    })
    .clone()
}

/// A joined game: runtime, game thread and shared state.
struct Session {
    /// Kept alive for the connection's tasks; dropped after the game thread.
    _rt: tokio::runtime::Runtime,
    shared: Arc<Shared>,
    game_thread: Option<std::thread::JoinHandle<()>>,
    /// Biome colours of the synced biome registry (index = biome id), for the renderer.
    biomes: Vec<haste_render::BiomeClimate>,
    /// What the F3 overlay needs that only the connection phase knows (biome names, tags).
    debug_env: DebugEnv,
    /// The saved options this run uses (loaded once).
    options: Options,
    /// The local player's entity id and UUID (`login`), for its own model in third person.
    local: (EntityId, u128),
    /// The particle atlas types (uploaded to the renderer once it exists).
    particle_types: std::sync::Arc<haste_render::ParticleTypes>,
}

/// Connection-phase data the F3 overlay shows: biome names of the synced registry (index = the
/// biome id the world's sections store) and the block/fluid tags (registry id -> "#tag" names),
/// both fixed for the connection; plus the server's simulation distance.
#[derive(Clone, Default)]
struct DebugEnv {
    biome_names: Vec<String>,
    /// Block registry id -> tags.
    block_tags: std::collections::HashMap<u32, Vec<String>>,
    /// Fluid registry id -> tags.
    fluid_tags: std::collections::HashMap<u32, Vec<String>>,
    simulation_distance: i32,
}

impl DebugEnv {
    fn new(regs: &haste_protocol::SyncedRegistries, simulation_distance: i32) -> Self {
        use haste_protocol::haste_core::Identifier;
        let biome_names = regs
            .entries(&Identifier::minecraft("worldgen/biome"))
            .map(|es| es.iter().map(|e| e.id.to_string()).collect())
            .unwrap_or_default();
        // Tag entries are registry ids; invert tag -> ids into id -> tags for both registries the
        // looking-at entries print.
        let invert = |registry: &str, out: &mut std::collections::HashMap<u32, Vec<String>>| {
            for rt in regs
                .tags
                .iter()
                .filter(|t| t.registry == Identifier::minecraft(registry))
            {
                for tag in &rt.tags {
                    for &id in &tag.entries {
                        out.entry(u32::try_from(id).unwrap_or(u32::MAX))
                            .or_default()
                            .push(format!("#{}", tag.name));
                    }
                }
            }
        };
        let mut block_tags = std::collections::HashMap::new();
        let mut fluid_tags = std::collections::HashMap::new();
        invert("block", &mut block_tags);
        invert("fluid", &mut fluid_tags);
        Self {
            biome_names,
            block_tags,
            fluid_tags,
            simulation_distance,
        }
    }
}

impl Session {
    /// `sound`: open the audio output (the window only; screenshots and the benchmark stay
    /// silent so they neither make noise nor measure the audio thread).
    fn join(args: &Args, sound: bool) -> anyhow::Result<Self> {
        let address = args.server.clone().ok_or_else(|| {
            anyhow::anyhow!("--server <host[:port]> is required (no launcher yet)")
        })?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .thread_name("haste-net")
            .build()?;
        let data = net::load_game_data()?;
        let mut options = load_options();
        // `--render-distance` overrides the saved option for this run (bench mode compares
        // render distances without editing the config file).
        if let Some(rd) = args.render_distance {
            options.render_distance = rd;
        }
        let mut opts = ConnectOptions::new(
            address.clone(),
            Account::Offline {
                username: args.username.clone(),
            },
        );
        // The saved render distance is what we advertise in client_information; the server's
        // own view distance (login packet) caps what we mesh below.
        opts.client_information.view_distance = options.render_distance as i8;
        let mut driver: NetDriver = rt.block_on(NetDriver::connect(opts))?;
        driver.session.fov_effect_scale = options.fov_effect_scale;
        let (biomes, biome_climates, type_names, sounds, painting_variants, mob_variants) = {
            let regs = driver.connection().registries();
            let sounds = if sound {
                open_sounds(regs, &data, &options.sound)
            } else {
                None
            };
            (
                net::synced_biome_count(regs).unwrap_or(64),
                biomes::biome_climates(regs),
                net::entity_type_names(regs, &data),
                sounds,
                net::painting_variants(regs),
                net::mob_variants(regs),
            )
        };
        let login = driver.login().clone();
        let debug_env = DebugEnv::new(driver.connection().registries(), login.simulation_distance);
        let local = (login.entity_id, login.profile.uuid.as_u128());
        tracing::info!(
            "joined {address} as {}: {:?}, dimension {}, view distance {}",
            driver.connection().profile().name,
            login.game_mode,
            login.dimension.name,
            login.view_distance
        );
        let world = World::new_with_biomes(login.dimension.clone(), data.clone(), biomes);
        let entity_types = data.registries.len("minecraft:entity_type");
        let mut driver = driver;
        driver.session.set_entity_type_names(type_names);
        let view_distance = login.view_distance.min(options.render_distance as i32);
        // The particle types of the datagen registry (the synced one has no data we need):
        // one atlas built once, shared by the pool (name resolution) and the renderer.
        let particle_ids: Vec<haste_protocol::haste_core::Identifier> =
            (0..data.registries.len("minecraft:particle_type") as u32)
                .filter_map(|i| {
                    data.registries
                        .name("minecraft:particle_type", haste_core::RegistryIndex(i))
                        .cloned()
                })
                .collect();
        let particle_types = std::sync::Arc::new(
            haste_render::ParticleTypes::build(&mut open_assets()?, &particle_ids)
                .context("building the particle atlas")?,
        );
        let mut entities = net::new_entity_store(entity_types);
        tracing::info!("synced painting variants: {}", painting_variants.len());
        entities.set_painting_variants(painting_variants);
        for (registry, table) in mob_variants {
            tracing::info!(
                "synced {}: {} entries",
                registry,
                table.iter().filter(|e| e.is_some()).count()
            );
            entities.set_mob_variants(&registry, table);
        }
        let shared = Arc::new(Shared::new(
            data,
            world,
            entities,
            view_distance,
            haste_render::Particles::new(particle_types.clone()),
            haste_render::default_skin(login.profile.uuid.as_u128()),
        ));
        if sounds.is_some() {
            *shared.sound.lock() = sounds;
        }
        let handle = rt.handle().clone();
        let sh = shared.clone();
        let game_thread = std::thread::Builder::new()
            .name("haste-game".into())
            .spawn(move || {
                let _rt = handle.enter();
                game::run(driver, sh)
            })?;
        Ok(Self {
            _rt: rt,
            shared,
            game_thread: Some(game_thread),
            biomes: biome_climates,
            debug_env,
            options,
            local,
            particle_types,
        })
    }

    /// Stops the game thread (it closes the connection) and lets the writer flush.
    fn leave(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.game_thread.take() {
            let _ = t.join();
        }
        // The runtime's worker threads flush the writer meanwhile.
        std::thread::sleep(Duration::from_millis(100));
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.leave();
    }
}

/// Feeds dirty sections to the renderer within a per-frame time budget, nearest first, and
/// skips sections that are empty and were never meshed (nothing to draw, nothing to clear).
#[derive(Default)]
pub struct WorldFeed {
    /// Sections waiting, sorted so the nearest is last (`pop`).
    queue: Vec<SectionPos>,
    queued: HashSet<SectionPos>,
    /// Sections whose last submitted mesh was non-empty.
    meshed: HashSet<SectionPos>,
    sort_needed: bool,
    last_arrival: Option<Instant>,
    /// Entity instances for this frame (reused).
    instances: Vec<EntityInstance>,
    /// The shared world as the mesh workers see it (created on the first feed).
    source: Option<Arc<dyn haste_render::SnapshotSource>>,
    /// Sections submitted this frame (reused).
    batch: Vec<SectionPos>,
    /// The first-person state last sent to the renderer (None also before the first).
    first_person: Option<game::FirstPersonView>,
}

/// Lets the mesh worker threads capture section snapshots from the shared world under its
/// read lock, so the render thread never copies blocks.
struct SharedWorldSource(Arc<Shared>);

impl haste_render::SnapshotSource for SharedWorldSource {
    fn capture(&self, pos: SectionPos, blend_radius: i32) -> Option<haste_render::MeshSnapshot> {
        let world = self.0.world.read();
        world.chunk(pos.chunk())?;
        Some(haste_render::MeshSnapshot::capture_with_biomes(
            &*world,
            Some(&biomes::WorldBiomes(&world)),
            blend_radius,
            pos,
        ))
    }
}

impl WorldFeed {
    /// Pulls new dirty sections from the game thread.
    fn collect(&mut self, shared: &Shared) {
        let mut dirty = shared.dirty.lock();
        if dirty.is_empty() {
            return;
        }
        for p in dirty.drain(..) {
            if self.queued.insert(p) {
                self.queue.push(p);
            }
        }
        self.sort_needed = true;
        self.last_arrival = Some(Instant::now());
    }

    /// Hands queued sections (nearest first) to the mesh workers, which capture and mesh them
    /// off the render thread, and lets the renderer upload finished meshes.
    fn feed(
        &mut self,
        shared: &Arc<Shared>,
        renderer: &mut Renderer,
        camera: SectionPos,
        view_distance: i32,
    ) {
        self.collect(shared);
        if self.sort_needed {
            let d = |p: &SectionPos| {
                let (dx, dy, dz) = (p.x - camera.x, p.y - camera.y, p.z - camera.z);
                dx * dx + dy * dy + dz * dz
            };
            self.queue.sort_unstable_by_key(|p| std::cmp::Reverse(d(p)));
            self.sort_needed = false;
        }
        let camera_chunk = ChunkPos::new(camera.x, camera.z);
        let source = self
            .source
            .get_or_insert_with(|| Arc::new(SharedWorldSource(shared.clone())))
            .clone();
        let mut batch = std::mem::take(&mut self.batch);
        batch.clear();
        {
            let world = shared.world.read();
            while batch.len() < SUBMIT_PER_FRAME {
                let Some(pos) = self.queue.pop() else { break };
                self.queued.remove(&pos);
                let non_empty = world
                    .chunk(pos.chunk())
                    .and_then(|c| c.section(pos.y))
                    .is_some_and(|s| !s.is_empty());
                if !non_empty && !self.meshed.contains(&pos) {
                    continue;
                }
                if non_empty {
                    self.meshed.insert(pos);
                } else {
                    self.meshed.remove(&pos);
                }
                batch.push(pos);
            }
        }
        // Also uploads meshes that finished since the last frame.
        renderer.update_world_from(&source, &batch, camera_chunk, view_distance);
        self.batch = batch;
    }

    /// Copies the interpolated entity instances, gives players their default skin and samples
    /// each entity's light for the renderer. The local player's own model is always included
    /// (the server does not spawn us as an entity, so it is built here from the shared view):
    /// the GUI's inventory preview needs it in first person too, and the caller tells the
    /// renderer not to draw it in the world then (`FrameInput::camera_entity`).
    fn entities(
        &mut self,
        shared: &Shared,
        renderer: &mut Renderer,
        local: (EntityId, u128),
        partial: f32,
    ) -> &[EntityInstance] {
        self.instances.clear();
        {
            let store = shared.entities.read();
            store.render_instances(partial, &mut self.instances);
            for inst in &mut self.instances {
                if inst.skin.is_none() {
                    if let Some(e) = store.get(inst.id).filter(|e| e.is_player_type) {
                        // The slim-arm flag is not used: haste-entity's humanoid model is wide.
                        inst.skin = Some(haste_render::default_skin(e.uuid.as_u128()).0);
                    }
                }
            }
        }
        {
            let view = *shared.view.lock();
            self.instances
                .push(game::player_instance(local.0, local.1, &view, partial));
        }
        renderer.sample_entity_lights(&*shared.world.read(), &self.instances);
        &self.instances
    }

    /// Hands the renderer the first-person hand (`Renderer::set_first_person`) when the view's
    /// per-tick part changed since the last call. The skin is fixed for the connection, so
    /// comparing the per-tick part is enough to catch every change; the skin is cloned only when
    /// one happens (no allocation per frame).
    /// The first-person hand only exists in the first-person perspective (F5 hides it).
    fn set_first_person(
        &mut self,
        shared: &Shared,
        renderer: &mut Renderer,
        perspective: game::Perspective,
    ) {
        let view = match perspective {
            game::Perspective::FirstPerson => shared.view.lock().first_person,
            _ => None,
        };
        if view == self.first_person {
            return;
        }
        self.first_person = view;
        renderer.set_first_person(view.map(|v| v.with_skin(&shared.skin)));
    }

    /// Whether everything the game thread produced has been meshed and uploaded, and nothing
    /// new arrived for `quiet`.
    ///
    /// `meshes_pending` (pending + queued results + in-flight) already guarantees every
    /// submitted section was meshed and uploaded; comparing the renderer's resident-section
    /// count against a client-side count cannot work, because a section with blocks can still
    /// mesh to zero vertices (fully enclosed) and the renderer then keeps no entry for it.
    fn settled(&self, stats: &FrameStats, quiet: Duration) -> bool {
        self.queue.is_empty()
            && stats.meshes_pending == 0
            && self.last_arrival.is_some_and(|t| t.elapsed() >= quiet)
    }
}

/// `Camera.calculateFov` + `Camera.modifyFovBasedOnDeathOrFluid` (vanilla): the saved FOV
/// times the interpolated smoothed modifier, then the death and fluid adjustments (in that
/// order, as vanilla applies them).
pub fn frame_fov(
    base_fov: f32,
    old_modifier: f32,
    new_modifier: f32,
    partial: f32,
    submerged: bool,
    death_ticks: u32,
    effect_scale: f32,
) -> f32 {
    let mut fov = base_fov * (old_modifier + (new_modifier - old_modifier) * partial);
    if death_ticks > 0 {
        let duration = (death_ticks as f32 + partial).min(20.0);
        fov /= (1.0 - 500.0 / (duration + 500.0)) * 2.0 + 1.0;
    }
    if submerged {
        fov *= 1.0 + (0.85714287 - 1.0) * effect_scale;
    }
    fov
}

/// `GameRenderer.bobHurt` (vanilla): the hurt-tilt roll in degrees. The render state's hurt
/// time is `hurtTime - partialTicks`; the roll is `-sin(h^4 * PI) * 14 * strength` with
/// `h = hurtTime / hurtDuration` (10), and 0 once the hurt time runs out.
pub fn hurt_tilt_degrees(hurt_ticks: u32, partial: f32, strength: f32) -> f32 {
    let hurt_time = hurt_ticks as f32 - partial;
    if hurt_time < 0.0 {
        return 0.0;
    }
    let h = hurt_time / crate::net::HURT_TILT_TICKS as f32;
    -(h * h * h * h * std::f32::consts::PI).sin() * 14.0 * strength
}

/// `GameRenderer.bobHurt` (vanilla): the death tilt in degrees, `40 - 8000/(deathTime + 200)`
/// with `deathTime` interpolated and clamped at 20.
pub fn death_roll_degrees(death_ticks: u32, partial: f32) -> f32 {
    let duration = (death_ticks as f32 + partial).min(20.0);
    40.0 - 8000.0 / (duration + 200.0)
}

/// `GameRenderer.bobView` (vanilla): the view-bob offsets for this frame — the translation
/// in the camera plane (x right, y up) and the roll/pitch wobble in degrees — from the
/// backwards-interpolated walk distance and the interpolated bob amount.
pub fn view_bob(
    walk_dist: f32,
    walk_dist_o: f32,
    bob: f32,
    bob_o: f32,
    partial: f32,
) -> (f32, f32, f32, f32) {
    // ClientAvatarState.getBackwardsInterpolatedWalkDistance / getInterpolatedBob.
    let walk = -(walk_dist + (walk_dist - walk_dist_o) * partial);
    let bob = bob_o + (bob - bob_o) * partial;
    let s = (walk * std::f32::consts::PI).sin() * bob;
    let c = (walk * std::f32::consts::PI).cos() * bob;
    (
        s * 0.5,
        -c.abs(),
        s * 3.0,
        ((walk * std::f32::consts::PI - 0.2).cos() * bob).abs() * 5.0,
    )
}

/// Builds this frame's camera and sky from the shared view at `now`. `yaw`/`pitch` are the
/// input layer's look; the player's server rotation offset is added here so mouse look is
/// smooth at frame rate while the player itself turns once per tick. In the third-person
/// perspectives the camera is moved off the eye (`game::third_person_camera`). The FOV is the
/// saved option with vanilla's per-tick FOV modifier interpolated, then the submerged and
/// death adjustments; the hurt/death tilts and the view bobbing come from the view's per-tick
/// state (vanilla's `Camera.calculateFov` and `GameRenderer.bobHurt`/`bobView`).
fn frame_view(
    shared: &Shared,
    perspective: game::Perspective,
    yaw: f32,
    pitch: f32,
    options: &Options,
    now: Instant,
) -> (Camera, SkyState, f32, i32, u64) {
    let view = *shared.view.lock();
    let partial = game::partial_tick(view.tick_at, now);
    let eye = game::interpolate(view.prev_eye, view.eye, partial);
    let look = (
        yaw + view.look_offset.0,
        (pitch + view.look_offset.1).clamp(-90.0, 90.0),
    );
    let (pos, yaw, pitch) = if perspective.draws_player() {
        // The world is read here (the outline shapes the crosshair raycast uses too), held
        // only for the ray.
        let (world, shapes) = (shared.world.read(), &shared.data.shapes);
        game::third_person_camera(perspective, eye, look.0, look.1, &*world, shapes)
    } else {
        (eye, look.0, look.1)
    };
    let fov_deg = frame_fov(
        options.fov as f32,
        view.fov_modifier_old,
        view.fov_modifier,
        partial,
        view.eye_in_water || view.eye_in_lava,
        view.death_ticks,
        options.fov_effect_scale,
    );
    let hurt_tilt = hurt_tilt_degrees(view.hurt_ticks, partial, options.damage_tilt_strength);
    let death_roll = if view.death_ticks > 0 {
        death_roll_degrees(view.death_ticks, partial)
    } else {
        0.0
    };
    let (bob_x, bob_y, bob_roll, bob_pitch) = if options.bob_view {
        view_bob(
            view.walk_dist,
            view.walk_dist_o,
            view.bob,
            view.bob_o,
            partial,
        )
    } else {
        (0.0, 0.0, 0.0, 0.0)
    };
    let camera = Camera {
        pos,
        yaw,
        pitch,
        fov_deg,
        roll_deg: 0.0,
        bob: haste_render::ViewBob {
            hurt_tilt_deg: hurt_tilt,
            hurt_dir_deg: view.hurt_dir,
            death_roll_deg: death_roll,
            x: bob_x,
            y: bob_y,
            roll_deg: bob_roll,
            pitch_deg: bob_pitch,
        },
    };
    let fog_end = (view.view_distance.max(2) * 16) as f32;
    let sky = SkyState {
        time_of_day: view.time_of_day,
        rain: 0.0,
        dimension_has_sky: view.has_sky,
        fog_start: fog_end * 0.75,
        fog_end,
    };
    (
        camera,
        sky,
        partial,
        view.view_distance,
        view.animation_tick,
    )
}

/// The SDL3 scancode (USB HID usage id, `InputConstants.KEY_*` in 26.3) of a winit `KeyCode`.
/// `haste_jvm::keys` documents the numbering; letters, digits and punctuation take the
/// consecutive HID ranges, everything else is one entry per key.
fn sdl_scancode(code: KeyCode) -> Option<i32> {
    use haste_jvm::keys as sdl;
    use KeyCode as K;
    Some(match code {
        K::KeyA => sdl::letter(b'a'),
        K::KeyB => sdl::letter(b'b'),
        K::KeyC => sdl::letter(b'c'),
        K::KeyD => sdl::letter(b'd'),
        K::KeyE => sdl::letter(b'e'),
        K::KeyF => sdl::letter(b'f'),
        K::KeyG => sdl::letter(b'g'),
        K::KeyH => sdl::letter(b'h'),
        K::KeyI => sdl::letter(b'i'),
        K::KeyJ => sdl::letter(b'j'),
        K::KeyK => sdl::letter(b'k'),
        K::KeyL => sdl::letter(b'l'),
        K::KeyM => sdl::letter(b'm'),
        K::KeyN => sdl::letter(b'n'),
        K::KeyO => sdl::letter(b'o'),
        K::KeyP => sdl::letter(b'p'),
        K::KeyQ => sdl::letter(b'q'),
        K::KeyR => sdl::letter(b'r'),
        K::KeyS => sdl::letter(b's'),
        K::KeyT => sdl::letter(b't'),
        K::KeyU => sdl::letter(b'u'),
        K::KeyV => sdl::letter(b'v'),
        K::KeyW => sdl::letter(b'w'),
        K::KeyX => sdl::letter(b'x'),
        K::KeyY => sdl::letter(b'y'),
        K::KeyZ => sdl::letter(b'z'),
        K::Digit1 => sdl::NUM_1,
        K::Digit2 => sdl::NUM_1 + 1,
        K::Digit3 => sdl::NUM_1 + 2,
        K::Digit4 => sdl::NUM_1 + 3,
        K::Digit5 => sdl::NUM_1 + 4,
        K::Digit6 => sdl::NUM_1 + 5,
        K::Digit7 => sdl::NUM_1 + 6,
        K::Digit8 => sdl::NUM_1 + 7,
        K::Digit9 => sdl::NUM_1 + 8,
        K::Digit0 => sdl::NUM_0,
        K::Enter | K::NumpadEnter => sdl::RETURN,
        K::Escape => sdl::ESCAPE,
        K::Backspace => sdl::BACKSPACE,
        K::Tab => sdl::TAB,
        K::Space => sdl::SPACE,
        K::Minus => 45,
        K::Equal => 46,
        K::BracketLeft => 47,
        K::BracketRight => 48,
        K::Backslash => 49,
        K::Semicolon => 51,
        K::Quote => 52,
        K::Backquote => 53,
        K::Comma => 54,
        K::Period => 55,
        K::Slash => sdl::SLASH,
        K::Delete => sdl::DELETE,
        K::Home => sdl::HOME,
        K::End => sdl::END,
        K::PageUp => 75,
        K::PageDown => 78,
        K::ArrowRight => sdl::RIGHT,
        K::ArrowLeft => sdl::LEFT,
        K::ArrowDown => sdl::DOWN,
        K::ArrowUp => sdl::UP,
        K::F1 => sdl::F1,
        K::F2 => 59,
        K::F3 => sdl::F3,
        K::F4 => 61,
        K::F5 => 62,
        K::F6 => 63,
        K::F7 => 64,
        K::F8 => 65,
        K::F9 => 66,
        K::F10 => 67,
        K::F11 => 68,
        K::F12 => 69,
        K::ControlLeft => sdl::LCTRL,
        K::ShiftLeft => sdl::LSHIFT,
        K::ControlRight => 228,
        K::ShiftRight => 229,
        K::AltLeft => 226,
        K::AltRight => 230,
        _ => return None,
    })
}

/// The SDL keymod bits of a key press/release, tracked by the App as keys go down and up
/// (winit gives no modifier-changed event). The three masks are the plain, left and right
/// variants of each modifier, as SDL3 defines them.
fn sdl_keymod(ctrl: bool, shift: bool, alt: bool) -> i32 {
    use haste_jvm::keys as sdl;
    (if ctrl { sdl::MOD_CONTROL } else { 0 })
        | (if shift { sdl::MOD_SHIFT } else { 0 })
        | (if alt { sdl::MOD_ALT } else { 0 })
}

/// Whether a winit key event carries text (winit's `KeyEvent::text`, given for printable keys
/// including via dead-key composition, not for control keys), which the chat and sign screens
/// need as `char_typed`.
fn typed_char_of(text: Option<&str>) -> Option<char> {
    text.and_then(|s| s.chars().next())
}

/// A winit mouse button in vanilla's numbering (1 left, 2 middle, 3 right, the same as SDL's).
fn vanilla_button(button: MouseButton) -> i32 {
    match button {
        MouseButton::Left => 1,
        MouseButton::Middle => 2,
        MouseButton::Right => 3,
        MouseButton::Back => 4,
        MouseButton::Forward => 5,
        _ => 0,
    }
}

/// The key routing decision (pure, so the routing is testable without a JVM): whether the
/// event goes to the UI at all. Repeats (a held key) only reach an open screen (held
/// Backspace in the chat box); with no screen the first press and the release go, so Tab's
/// player list shows while held and hides on release.
fn key_goes_to_ui(repeat: bool, screen_open: bool) -> bool {
    screen_open || !repeat
}

/// The other half of the routing decision: whether the game may use the key after the UI
/// saw it. It may not when the UI used it or any screen is open (no movement, no look, no
/// attack while chat or the inventory is up).
fn game_uses_key(ui_used: bool, screen_open: bool) -> bool {
    !ui_used && !screen_open
}

/// A script key in winit's numbering (the windowed client's events and the script runner's
/// actions go through the same `KeyCode`-keyed routing).
fn winit_key(key: crate::script::Key) -> KeyCode {
    use crate::script::Key as K;
    match key {
        K::A => KeyCode::KeyA,
        K::B => KeyCode::KeyB,
        K::C => KeyCode::KeyC,
        K::D => KeyCode::KeyD,
        K::E => KeyCode::KeyE,
        K::F => KeyCode::KeyF,
        K::G => KeyCode::KeyG,
        K::H => KeyCode::KeyH,
        K::I => KeyCode::KeyI,
        K::J => KeyCode::KeyJ,
        K::K => KeyCode::KeyK,
        K::L => KeyCode::KeyL,
        K::M => KeyCode::KeyM,
        K::N => KeyCode::KeyN,
        K::O => KeyCode::KeyO,
        K::P => KeyCode::KeyP,
        K::Q => KeyCode::KeyQ,
        K::R => KeyCode::KeyR,
        K::S => KeyCode::KeyS,
        K::T => KeyCode::KeyT,
        K::U => KeyCode::KeyU,
        K::V => KeyCode::KeyV,
        K::W => KeyCode::KeyW,
        K::X => KeyCode::KeyX,
        K::Y => KeyCode::KeyY,
        K::Z => KeyCode::KeyZ,
        K::Num0 => KeyCode::Digit0,
        K::Num1 => KeyCode::Digit1,
        K::Num2 => KeyCode::Digit2,
        K::Num3 => KeyCode::Digit3,
        K::Num4 => KeyCode::Digit4,
        K::Num5 => KeyCode::Digit5,
        K::Num6 => KeyCode::Digit6,
        K::Num7 => KeyCode::Digit7,
        K::Num8 => KeyCode::Digit8,
        K::Num9 => KeyCode::Digit9,
        K::F1 => KeyCode::F1,
        K::F2 => KeyCode::F2,
        K::F3 => KeyCode::F3,
        K::F4 => KeyCode::F4,
        K::F5 => KeyCode::F5,
        K::F6 => KeyCode::F6,
        K::F7 => KeyCode::F7,
        K::F8 => KeyCode::F8,
        K::F9 => KeyCode::F9,
        K::F10 => KeyCode::F10,
        K::F11 => KeyCode::F11,
        K::F12 => KeyCode::F12,
        K::Escape => KeyCode::Escape,
        K::Enter => KeyCode::Enter,
        K::Tab => KeyCode::Tab,
        K::Space => KeyCode::Space,
        K::Backspace => KeyCode::Backspace,
        K::Up => KeyCode::ArrowUp,
        K::Down => KeyCode::ArrowDown,
        K::Left => KeyCode::ArrowLeft,
        K::Right => KeyCode::ArrowRight,
        K::LeftShift => KeyCode::ShiftLeft,
        K::LeftCtrl => KeyCode::ControlLeft,
        K::LeftAlt => KeyCode::AltLeft,
    }
}

/// Number keys 1-9 select hotbar slots 0-8.
fn hotbar_key(code: KeyCode) -> Option<u8> {
    Some(match code {
        KeyCode::Digit1 => 0,
        KeyCode::Digit2 => 1,
        KeyCode::Digit3 => 2,
        KeyCode::Digit4 => 3,
        KeyCode::Digit5 => 4,
        KeyCode::Digit6 => 5,
        KeyCode::Digit7 => 6,
        KeyCode::Digit8 => 7,
        KeyCode::Digit9 => 8,
        _ => return None,
    })
}

fn camera_section(c: &Camera) -> SectionPos {
    let p = c.pos.floor().as_ivec3();
    SectionPos::new(p.x >> 4, p.y >> 4, p.z >> 4)
}

/// The input half of the client: everything between winit (or a script) and the game. The
/// windowed client's `App` and the offscreen script runner both drive one `InputRouter`, so
/// a scripted key is indistinguishable from a real one: the UI (open screen or HUD) sees it
/// first, and only a `false` return reaches the game's own handling (movement keys, hotbar,
/// attack/use, jump double-tap, drops, swap).
struct InputRouter {
    /// Keys currently held (movement reads them each tick).
    keys: HashSet<KeyCode>,
    /// The look angles in degrees (mouse motion and `look` actions write them).
    yaw: f32,
    pitch: f32,
    /// SDL keymod bits of the currently held Ctrl/Shift/Alt keys, for the UI's key events.
    mods: (bool, bool, bool),
    /// The cursor in GUI-scaled pixels (`frame`'s and every mouse event's coordinates).
    mouse: (f64, f64),
    /// Whether the window is focused and the cursor grabbed (the game input gates).
    focused: bool,
    grabbed: bool,
    /// Jump double-tap window (`last_jump_press`); a second press toggles flight.
    last_jump_press: Option<Instant>,
    toggle_flight: bool,
    /// Left / right mouse button held (attack / use) and the press-only actions since the
    /// last sample.
    attack: bool,
    use_item: bool,
    /// Left / right clicks queued since the last sample (vanilla's `KeyMapping.clickCount`:
    /// one per press). Drained into the shared input by `sample`.
    attack_clicks: u8,
    use_clicks: u8,
    pick_block: bool,
    drop_item: bool,
    drop_stack: bool,
    swap_offhand: bool,
    /// Hotbar slot chosen since the last sample.
    hotbar: Option<u8>,
    /// Scroll wheel steps not yet turned into a slot change.
    scroll: f64,
}

impl Default for InputRouter {
    fn default() -> Self {
        InputRouter {
            keys: HashSet::new(),
            yaw: 0.0,
            pitch: 0.0,
            mods: (false, false, false),
            mouse: (0.0, 0.0),
            focused: true,
            grabbed: false,
            last_jump_press: None,
            toggle_flight: false,
            attack: false,
            use_item: false,
            attack_clicks: 0,
            use_clicks: 0,
            pick_block: false,
            drop_item: false,
            drop_stack: false,
            swap_offhand: false,
            hotbar: None,
            scroll: 0.0,
        }
    }
}

impl InputRouter {
    /// A key press or release. `ui` is the HUD/screens when they are up, `text` the typed
    /// character winit decoded (if any). Returns `true` when the UI used the key.
    fn key(
        &mut self,
        code: KeyCode,
        pressed: bool,
        repeat: bool,
        text: Option<&str>,
        ui: Option<&crate::ui::Ui>,
    ) -> bool {
        // Track the keymod bits first: they go into the same event's mods.
        match code {
            KeyCode::ControlLeft | KeyCode::ControlRight => self.mods.0 = pressed,
            KeyCode::ShiftLeft | KeyCode::ShiftRight => self.mods.1 = pressed,
            KeyCode::AltLeft | KeyCode::AltRight => self.mods.2 = pressed,
            _ => {}
        }
        let mods = sdl_keymod(self.mods.0, self.mods.1, self.mods.2);
        // The UI goes first: an open screen takes every key and typed character, including
        // repeats (held Backspace in the chat box); with no screen it takes the HUD's keys,
        // first press only. Only a false return falls through to the game handling below.
        let screen_open = ui.is_some_and(|ui| ui.screen_open());
        if let (Some(ui), Some(sc)) = (ui, sdl_scancode(code)) {
            let forward = key_goes_to_ui(repeat, screen_open);
            let used = forward && ui.key(sc, keys::keycode(sc), mods, pressed);
            if let (Some(c), true) = (typed_char_of(text), pressed) {
                ui.char_typed(c, mods);
            }
            if !game_uses_key(used, screen_open) {
                // The UI took it: whatever the game thought was held must go.
                self.keys.remove(&code);
                return true;
            }
        } else if screen_open {
            self.keys.remove(&code);
            return true;
        }
        if pressed && !repeat {
            match code {
                KeyCode::Escape => {
                    // Not consumed by the UI (no screen): vanilla's Escape has no game
                    // meaning beyond opening the pause menu, so this only frees the cursor
                    // for the pre-UI behaviour.
                    self.release();
                }
                KeyCode::Space if self.grabbed && self.focused => self.jump_pressed(Instant::now()),
                _ if self.grabbed && self.focused && hotbar_key(code).is_some() => {
                    self.hotbar = hotbar_key(code);
                }
                KeyCode::KeyQ if self.grabbed && self.focused => {
                    // Ctrl+Q drops the whole stack, plain Q one item.
                    if self.keys.contains(&KeyCode::ControlLeft) {
                        self.drop_stack = true;
                    } else {
                        self.drop_item = true;
                    }
                }
                KeyCode::KeyF if self.grabbed && self.focused => {
                    self.swap_offhand = true;
                }
                _ => {}
            }
        }
        if self.focused && self.grabbed && pressed {
            self.keys.insert(code);
        } else if !pressed {
            self.keys.remove(&code);
        }
        false
    }

    /// The mouse moved to GUI-scaled `(x, y)`.
    fn mouse_moved(&mut self, x: f64, y: f64, ui: Option<&crate::ui::Ui>) {
        self.mouse = (x, y);
        if let Some(ui) = ui {
            ui.mouse_moved(x, y);
        }
    }

    /// A mouse button press or release (vanilla numbering: 1 left, 2 middle, 3 right) at
    /// GUI-scaled `(x, y)`. Returns `true` when the UI used it.
    fn mouse_button(
        &mut self,
        button: i32,
        pressed: bool,
        x: f64,
        y: f64,
        ui: Option<&crate::ui::Ui>,
    ) -> bool {
        self.mouse = (x, y);
        if let Some(ui) = ui {
            if ui.mouse_button(x, y, button, pressed) {
                return true;
            }
        }
        if !self.grabbed {
            return false;
        }
        match button {
            1 => {
                self.attack = pressed;
                if pressed {
                    self.attack_clicks = self.attack_clicks.saturating_add(1);
                }
            }
            3 => {
                self.use_item = pressed;
                if pressed {
                    self.use_clicks = self.use_clicks.saturating_add(1);
                }
            }
            2 if pressed => self.pick_block = true,
            _ => {}
        }
        false
    }

    /// The wheel moved by `(dx, dy)` notches at GUI-scaled `(x, y)`. Returns `true` when the
    /// UI used it.
    fn wheel(
        &mut self,
        dx: f64,
        dy: f64,
        x: f64,
        y: f64,
        ui: Option<&crate::ui::Ui>,
        shared: &Shared,
    ) -> bool {
        self.mouse = (x, y);
        if let Some(ui) = ui {
            if ui.scroll(x, y, dx, dy) {
                return true;
            }
        }
        if !self.grabbed {
            return false;
        }
        self.scroll += dy;
        let steps = self.scroll.trunc() as i32;
        if steps != 0 {
            self.scroll -= steps as f64;
            // Scrolling down (negative) moves to the next slot, wrapping 0..=8.
            let current = self
                .hotbar
                .map_or(shared.view.lock().held_slot, |s| s as i32);
            self.hotbar = Some((current - steps).rem_euclid(9) as u8);
        }
        true
    }

    /// A look change (mouse motion deltas while grabbed, or a script's `look`).
    fn look(&mut self, yaw: f32, pitch: f32) {
        self.yaw = yaw;
        self.pitch = pitch;
    }

    /// Jump pressed: a second press within vanilla's 7-tick window toggles flight.
    fn jump_pressed(&mut self, now: Instant) {
        match self.last_jump_press {
            Some(t) if now.duration_since(t) < DOUBLE_TAP => {
                self.toggle_flight = true;
                self.last_jump_press = None;
            }
            _ => self.last_jump_press = Some(now),
        }
    }

    /// Nothing is held anymore (focus lost, a screen opened, the script ended).
    fn release(&mut self) {
        self.keys.clear();
        self.attack = false;
        self.use_item = false;
        self.last_jump_press = None;
    }

    /// Copy the sampled state into the shared input the game thread reads each tick, and
    /// consume the press-only actions. `perspective` is the camera's own: the game thread's
    /// camera effects that are gated on it (the spyglass FOV) only apply in first person.
    fn sample(&mut self, shared: &Shared, perspective: game::Perspective) {
        let k = |c: KeyCode| self.keys.contains(&c);
        let state = InputState {
            forward: k(KeyCode::KeyW),
            back: k(KeyCode::KeyS),
            left: k(KeyCode::KeyA),
            right: k(KeyCode::KeyD),
            jump: k(KeyCode::Space),
            sneak: k(KeyCode::ShiftLeft),
            sprint: k(KeyCode::ControlLeft),
            yaw: self.yaw,
            pitch: self.pitch,
        };
        let mut shared_input = shared.input.lock();
        shared_input.state = state;
        shared_input.first_person = matches!(perspective, game::Perspective::FirstPerson);
        shared_input.toggle_flight |= std::mem::take(&mut self.toggle_flight);
        let i = &mut shared_input.interact;
        i.attack = self.attack;
        i.use_item = self.use_item;
        // The click counts accumulate across samples (the render thread may sample several
        // times per tick); the game thread drains them once per tick.
        i.attack_clicks = i
            .attack_clicks
            .saturating_add(std::mem::take(&mut self.attack_clicks));
        i.use_clicks = i
            .use_clicks
            .saturating_add(std::mem::take(&mut self.use_clicks));
        i.pick_block |= std::mem::take(&mut self.pick_block);
        i.drop_item |= std::mem::take(&mut self.drop_item);
        i.drop_stack |= std::mem::take(&mut self.drop_stack);
        i.swap_offhand |= std::mem::take(&mut self.swap_offhand);
        if let Some(slot) = self.hotbar.take() {
            i.hotbar = Some(slot);
        }
    }
}

fn open_assets() -> anyhow::Result<haste_assets::Assets> {
    let jar = haste_protocol::haste_core::paths::client_jar();
    haste_assets::Assets::open(
        &jar,
        &haste_protocol::haste_core::paths::assets_dir(),
        "26.3",
    )
    .with_context(|| format!("opening assets from {}", jar.display()))
}

/// The sound output for a joined game: `sounds.json` parsed from the real assets, the
/// sound-event name table (the server's synced registry or the static one), played
/// through the default device with the saved volumes. Without an audio device it stays
/// silent; when the assets or `sounds.json` cannot be read the game runs without sound
/// (logged) rather than not at all.
fn open_sounds(
    regs: &haste_protocol::SyncedRegistries,
    data: &haste_data::GameData,
    volumes: &haste_setup::options::SoundVolumes,
) -> Option<crate::sound::Sounds> {
    let events = (|| -> anyhow::Result<haste_audio::sounds::SoundEvents> {
        let mut assets = open_assets()?;
        let bytes = assets.read_asset("assets/minecraft/sounds.json")?;
        haste_audio::sounds::SoundEvents::parse(&bytes)
    })();
    match events {
        Ok(events) => Some(crate::sound::Sounds::new(
            events,
            open_assets().ok()?,
            crate::sound::sound_event_names(regs, data),
            volumes,
        )),
        Err(e) => {
            tracing::warn!("sound stays off: {e:#}");
            None
        }
    }
}

/// `--screenshot <file>`: join, wait until the world is meshed, render one frame offscreen,
/// save it as PNG and leave. `--seconds` bounds the wait.
pub fn screenshot(args: &Args, file: &Path) -> anyhow::Result<()> {
    let session = Session::join(args, false)?;
    let shared = session.shared.clone();
    // The shot is first person: the game thread's camera effects take that perspective.
    shared.input.lock().first_person = true;
    let mut assets = open_assets()?;
    let mut renderer =
        Renderer::new_offscreen(args.width, args.height, &mut assets, &shared.data.blocks)?;
    renderer.set_item_registry(&shared.data.registries);
    renderer.set_biomes(&session.biomes);
    renderer.set_particle_types(&session.particle_types);
    let mut gui = GuiFrame::default();
    let mut ui: Option<crate::ui::Ui> = None;
    let mut feed = WorldFeed::default();
    let deadline = Instant::now() + Duration::from_secs(args.seconds);
    // With `--pos`, the shot is only taken once the server has teleported the player to these
    // feet coordinates (the compare script teleports over RCON while we join).
    let want_pos = args
        .pos
        .as_ref()
        .filter(|v| v.len() == 3)
        .map(|v| DVec3::new(v[0], v[1], v[2]));
    // The HUD's JVM (started by the game thread) gets this long to be ready; the shot then
    // waits for it past the world settling, so the HUD is in the picture whenever it can be.
    let jvm_deadline = Instant::now() + Duration::from_secs(args.seconds.min(30));
    let mut wait_for_hud = true;
    let mut frames = 0u32;
    let mut stats;
    let mut last_camera;
    let perspective = game::Perspective::parse(&args.perspective)?;
    loop {
        // The look the shot is taken with: `--yaw`/`--pitch`, else the player's own (the
        // input layer's zero look plus the server's rotation offset). It goes into
        // `frame_view` so the third-person camera offset follows it. The player is made to
        // look the same way, so its crosshair target (and the outline drawn for it) matches
        // the picture.
        let offset = shared.view.lock().look_offset;
        let look = (args.yaw.unwrap_or(offset.0), args.pitch.unwrap_or(offset.1));
        {
            let mut input = shared.input.lock();
            input.state.yaw = look.0 - offset.0;
            input.state.pitch = (look.1 - offset.1).clamp(-90.0, 90.0);
        }
        let (mut camera, sky, partial, vd, animation_tick) = frame_view(
            &shared,
            perspective,
            look.0 - offset.0,
            look.1 - offset.1,
            &session.options,
            Instant::now(),
        );
        if let Some(pos) = want_pos {
            let view = *shared.view.lock();
            let eye = pos + DVec3::new(0.0, view.eye_height, 0.0);
            camera.pos = if perspective.draws_player() {
                let (world, shapes) = (shared.world.read(), &shared.data.shapes);
                game::third_person_camera(perspective, eye, look.0, look.1, &*world, shapes).0
            } else {
                eye
            };
        }
        last_camera = camera;
        feed.feed(&shared, &mut renderer, camera_section(&camera), vd);
        feed.set_first_person(&shared, &mut renderer, perspective);
        let entities = feed.entities(&shared, &mut renderer, session.local, partial);
        let block_target = shared.view.lock().block_target;
        let breaking = shared.breaking.lock().clone();
        // The HUD, like the window: waiting for the JVM, then one `Ui` for this render thread.
        if ui.is_none() {
            ui = shared.ui.lock().clone();
            if let Some(u) = &ui {
                u.set_text_metrics(renderer.text_metrics());
            }
        }
        if let Some(u) = &mut ui {
            let scale = resolve_gui_scale(
                u32::from(session.options.gui_scale),
                args.width,
                args.height,
            );
            let (gw, gh) = gui_scaled_size(args.width, args.height, scale);
            gui = u.frame(gw, gh, scale, (0.0, 0.0), partial).clone();
        }
        let frame = FrameInput {
            camera,
            entities,
            gui: &gui,
            sky,
            partial_tick: partial,
            block_target,
            breaking: &breaking,
            camera_entity: (!perspective.draws_player()).then_some(session.local.0),
        };
        renderer.sample_world(&*shared.world.read(), &shared.data.shapes, &frame);
        renderer.set_animation_tick(animation_tick);
        {
            let world = shared.world.read();
            let mut particles = shared.particles.lock();
            renderer.prepare_particles(&mut particles, &*world, camera, partial);
        }
        stats = renderer.render(&frame)?;
        frames += 1;
        let placed = shared.view.lock().has_position;
        if shared.is_closed() {
            anyhow::bail!(
                "disconnected: {}",
                shared.closed.lock().clone().unwrap_or_default()
            );
        }
        if let Some(pos) = want_pos {
            let view = *shared.view.lock();
            let reached = placed && (view.feet - pos).length() <= 0.5;
            if !reached {
                if Instant::now() > deadline {
                    let c = last_camera;
                    anyhow::bail!(
                        "--seconds elapsed before reaching --pos; player at ({:.2}, {:.2}, {:.2}), camera would have been at ({:.2}, {:.2}, {:.2})",
                        view.feet.x,
                        view.feet.y,
                        view.feet.z,
                        c.pos.x,
                        c.pos.y,
                        c.pos.z
                    );
                }
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
        }
        if placed && feed.settled(&stats, Duration::from_millis(1500)) && !wait_for_hud {
            break;
        }
        wait_for_hud = wait_for_hud
            && ui.is_none()
            && !shared.hud_failed.load(Ordering::Relaxed)
            && Instant::now() < jvm_deadline;
        if wait_for_hud && Instant::now() > jvm_deadline {
            tracing::warn!("the HUD's JVM did not start in time; the shot has no HUD");
        }
        if Instant::now() > deadline {
            tracing::warn!("--seconds elapsed before the world settled; saving what is there");
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let image = renderer.read_back()?;
    image
        .save(file)
        .with_context(|| format!("saving {}", file.display()))?;
    let c = last_camera;
    let view = *shared.view.lock();
    let entities = feed.instances.len();
    println!(
        "screenshot {} ({}x{}) after {frames} frames: camera ({:.2}, {:.2}, {:.2}) yaw {:.1} pitch {:.1}, time of day {}, sections {} drawn of {}, {} draw calls, {} entity instances (perspective {}, local player {}), {} particles drawn, target {:?}",
        file.display(),
        image.width(),
        image.height(),
        c.pos.x,
        c.pos.y,
        c.pos.z,
        c.yaw,
        c.pitch,
        view.time_of_day % 24000,
        stats.chunk_sections_drawn,
        stats.chunk_sections_total,
        stats.draw_calls,
        entities,
        args.perspective,
        perspective.draws_player(),
        renderer.particles_drawn(),
        view.block_target
    );
    drop(session);
    Ok(())
}

/// A script mouse button in vanilla's numbering (1 left, 2 middle, 3 right).
fn mouse_button_id(button: script::MouseButton) -> i32 {
    match button {
        script::MouseButton::Left => 1,
        script::MouseButton::Middle => 2,
        script::MouseButton::Right => 3,
    }
}

/// Prints the text and HUD-sprite facts of a script screenshot's frame. The check scripts grep
/// these lines as evidence instead of opening the PNGs.
fn log_script_frame(frame: &GuiFrame, name: &str) {
    let mut hotbar = false;
    for c in &frame.commands {
        match c {
            GuiCommand::Text { text, .. } | GuiCommand::StyledText { text, .. } => {
                println!("script text[{name}]: {text}");
            }
            GuiCommand::Sprite { sprite, .. } if sprite.path() == "hud/hotbar" => hotbar = true,
            _ => {}
        }
    }
    println!(
        "script frame[{name}]: hotbar={}",
        if hotbar { "yes" } else { "no" }
    );
}

/// `--script FILE`: join like `--screenshot`, then drive the parsed script's actions every
/// game tick (20 Hz) through the same input routing the window uses (`InputRouter`: the UI
/// first, then the game), rendering every frame offscreen. `screenshot NAME` actions save
/// the frame (HUD and open screens included) as `target/shots/script-NAME.png`. `Error`
/// actions (a `wait_until` timeout) and `Finished` end the run; an error exits non-zero.
pub fn script_run(args: &Args, file: &Path) -> anyhow::Result<()> {
    let Some(_server) = args.server.as_ref() else {
        anyhow::bail!("--script needs --server <host[:port]>");
    };
    let text = std::fs::read_to_string(file)
        .with_context(|| format!("reading script {}", file.display()))?;
    let steps = script::parse(&text).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut runner = script::Runner::new(steps);
    let session = Session::join(args, false)?;
    let shared = session.shared.clone();
    let mut assets = open_assets()?;
    let mut renderer =
        Renderer::new_offscreen(args.width, args.height, &mut assets, &shared.data.blocks)?;
    renderer.set_item_registry(&shared.data.registries);
    renderer.set_biomes(&session.biomes);
    let mut gui = GuiFrame::default();
    let mut ui: Option<crate::ui::Ui> = None;
    let mut feed = WorldFeed::default();
    // The script's client is "grabbed" (game input goes through, like an active window).
    let mut input = InputRouter {
        grabbed: true,
        ..InputRouter::default()
    };
    let deadline = Instant::now() + Duration::from_secs(args.seconds);
    let tick = Duration::from_secs_f64(haste_protocol::haste_core::game::TICK_SECONDS);
    let mut next_tick = Instant::now();
    let mut frames = 0u32;
    let mut last_stats = FrameStats::default();
    let mut fps = FpsCounter::new();
    let mut last_err: Option<String> = None;
    let mut finished = false;
    // Screenshot actions of this tick, saved after the frame is rendered (the readback must
    // include the frame the actions just produced: an open screen, the typed chat).
    let mut shots: Vec<String> = Vec::new();
    // An F2 press this tick: save the frame to the config screenshots dir after it is rendered.
    let mut f2_pending = false;
    'run: loop {
        // The look the player is rendered with: the input layer's (the script's `look`
        // actions write it) plus the server's rotation offset, like the window.
        let mut view = *shared.view.lock();
        let look = (input.yaw, input.pitch);
        // One game tick's actions whenever it is time (`wait_until settled` uses the same
        // quiet test as the screenshot mode, plus the HUD's JVM being up or known-failed: a
        // script's keys go to the UI, so pressing them before it exists would lose them).
        if Instant::now() >= next_tick {
            next_tick += tick;
            let hud_ready = ui.is_some() || shared.hud_failed.load(Ordering::Relaxed);
            let settled = view.has_position
                && hud_ready
                && feed.settled(&last_stats, Duration::from_millis(1500));
            if !settled && frames.is_multiple_of(40) {
                tracing::info!(
                    "waiting to settle: queue {} pending {} arrival {:?}",
                    feed.queue.len(),
                    last_stats.meshes_pending,
                    feed.last_arrival.map(|t| t.elapsed())
                );
            }
            let state = script::ScriptState {
                joined: view.has_position,
                settled,
                screen_open: ui.as_ref().is_some_and(|u| u.screen_open()),
            };
            for action in runner.tick(&state) {
                match action {
                    script::Action::KeyDown(k) => {
                        // F2 is the window-only screenshot key (the UI never takes it): capture
                        // the next frame, like the windowed event loop does.
                        if k == script::Key::F2 {
                            f2_pending = true;
                        }
                        input.key(winit_key(k), true, false, None, ui.as_ref());
                    }
                    script::Action::KeyUp(k) => {
                        input.key(winit_key(k), false, false, None, ui.as_ref());
                    }
                    script::Action::Text(t) => {
                        if let Some(ui) = &ui {
                            for c in t.chars() {
                                ui.char_typed(c, 0);
                            }
                        }
                    }
                    script::Action::MouseMove(x, y) => {
                        input.mouse_moved(f64::from(x), f64::from(y), ui.as_ref());
                    }
                    script::Action::Click(button, x, y) => {
                        let b = mouse_button_id(button);
                        let (x, y) = (f64::from(x), f64::from(y));
                        // A click is a press and a release on the same tick.
                        input.mouse_button(b, true, x, y, ui.as_ref());
                        input.mouse_button(b, false, x, y, ui.as_ref());
                    }
                    script::Action::MouseDown(button) => {
                        let (x, y) = input.mouse;
                        input.mouse_button(mouse_button_id(button), true, x, y, ui.as_ref());
                    }
                    script::Action::MouseUp(button) => {
                        let (x, y) = input.mouse;
                        input.mouse_button(mouse_button_id(button), false, x, y, ui.as_ref());
                    }
                    script::Action::Scroll(n) => {
                        let (x, y) = input.mouse;
                        input.wheel(0.0, f64::from(n), x, y, ui.as_ref(), &shared);
                    }
                    script::Action::Look(yaw, pitch) => input.look(yaw, pitch),
                    script::Action::Screenshot(name) => shots.push(name),
                    script::Action::Error(e) => {
                        last_err = Some(e);
                        break 'run;
                    }
                    script::Action::Finished => {
                        finished = true;
                        break 'run;
                    }
                }
            }
            // The actions may have changed what the game thread sees this tick.
            input.sample(&shared, game::Perspective::FirstPerson);
            view = *shared.view.lock();
        }
        if shared.is_closed() {
            anyhow::bail!(
                "disconnected: {}",
                shared.closed.lock().clone().unwrap_or_default()
            );
        }
        // Render the frame like the screenshot mode does (the HUD and any open screen via
        // `Ui::frame`), so `screenshot` actions capture exactly what the window would show.
        let (camera, sky, partial, vd, animation_tick) = frame_view(
            &shared,
            game::Perspective::FirstPerson,
            look.0,
            look.1,
            &session.options,
            Instant::now(),
        );
        feed.feed(&shared, &mut renderer, camera_section(&camera), vd);
        let entities = feed.entities(&shared, &mut renderer, session.local, partial);
        let block_target = view.block_target;
        let breaking = shared.breaking.lock().clone();
        if ui.is_none() {
            ui = shared.ui.lock().clone();
            if let Some(u) = &ui {
                u.set_text_metrics(renderer.text_metrics());
            }
        }
        if let Some(u) = &ui {
            // F3's numbers every frame, like the window's `draw` (cheap: short mutexes; the
            // overlay's strings are only built while it is on).
            u.set_debug(debug_info(
                &session.debug_env,
                &shared,
                &renderer,
                look,
                fps.fps as i32,
                &last_stats,
                entities.len() as i32,
                resolve_gui_scale(
                    u32::from(session.options.gui_scale),
                    args.width,
                    args.height,
                ),
                &gpu_info(&renderer),
            ));
        }
        if let Some(u) = &mut ui {
            let scale = resolve_gui_scale(
                u32::from(session.options.gui_scale),
                args.width,
                args.height,
            );
            let (gw, gh) = gui_scaled_size(args.width, args.height, scale);
            gui = u.frame(gw, gh, scale, input.mouse, partial).clone();
        }
        let frame = FrameInput {
            camera,
            entities,
            gui: &gui,
            sky,
            partial_tick: partial,
            block_target,
            breaking: &breaking,
            camera_entity: Some(session.local.0),
        };
        renderer.sample_world(&*shared.world.read(), &shared.data.shapes, &frame);
        renderer.set_animation_tick(animation_tick);
        if let Ok(stats) = renderer.render(&frame) {
            last_stats = stats;
            fps.frame(&stats);
        }
        frames += 1;
        // This tick's screenshot actions: the frame they caused is now on the GPU.
        if !shots.is_empty() || f2_pending {
            let image = renderer
                .read_back()
                .context("reading the frame back for the script screenshots")?;
            // Log what the frame drew (the check scripts grep this as evidence; images are not
            // opened). The hotbar sprite is the visible proof a HUD element is hidden or shown.
            for name in shots.drain(..) {
                log_script_frame(&gui, &name);
                let path = std::path::Path::new("target/shots").join(format!("script-{name}.png"));
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir)
                        .with_context(|| format!("creating {}", dir.display()))?;
                }
                image
                    .save(&path)
                    .with_context(|| format!("saving {}", path.display()))?;
                println!("script screenshot saved to {}", path.display());
                // The frame's text, next to the PNG: the checks' lines to read without OCR.
                let mut txt = path.clone().into_os_string();
                txt.push(".txt");
                std::fs::write(
                    std::path::PathBuf::from(txt),
                    gui.commands
                        .iter()
                        .filter_map(|c| match c {
                            haste_protocol::haste_core::gui::GuiCommand::Text { text, .. }
                            | haste_protocol::haste_core::gui::GuiCommand::StyledText {
                                text,
                                ..
                            } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
                .with_context(|| "writing the screenshot's text")?;
            }
            if std::mem::take(&mut f2_pending) {
                log_script_frame(&gui, "f2");
                // Vanilla's F2: the frame goes to the config dir's `screenshots/` folder.
                match crate::screenshots::save(image.clone()) {
                    Ok(name) => {
                        let dir = crate::screenshots::dir()
                            .map(|d| d.display().to_string())
                            .unwrap_or_default();
                        println!("script F2 screenshot saved to {dir}/{name}");
                    }
                    Err(e) => tracing::warn!("F2 screenshot failed: {e:#}"),
                }
            }
        }
        if Instant::now() > deadline {
            tracing::warn!("--seconds elapsed before the script finished; stopping early");
            break;
        }
        // Never busy-spin: sleep to the next tick, catching up if we fell behind.
        let now = Instant::now();
        if now < next_tick {
            std::thread::sleep((next_tick - now).min(Duration::from_millis(5)));
        } else {
            next_tick = now;
        }
    }
    let _ = frames;
    drop(session);
    if let Some(e) = last_err {
        anyhow::bail!("script failed: {e}");
    }
    if !finished {
        anyhow::bail!("script did not finish before --seconds elapsed");
    }
    Ok(())
}

/// The bench camera at normalised time `t` in 0..=1 (t is elapsed/total). The first half is a
/// slow 360-degree turn in place around the spawn eye; the second half flies straight along +X
/// at 10 blocks/s at eye height + 20. `base` is the settle-frame camera; only the camera moves,
/// never the player. Deterministic: same t, same camera.
pub fn bench_camera(t: f32, base: &Camera, total_seconds: f32) -> Camera {
    let half = 0.5_f32;
    let mut camera = *base;
    if t < half {
        // One full turn over the first half.
        camera.yaw = base.yaw + (t / half) * 360.0;
    } else {
        camera.yaw = base.yaw + 360.0;
        // 10 blocks/s for the second half (which is total/2 seconds long).
        let flight_seconds = total_seconds * (t - half);
        camera.pos.x = base.pos.x + 10.0 * flight_seconds as f64;
        camera.pos.y = base.pos.y + 20.0;
    }
    camera
}

/// One recorded bench frame.
#[derive(Clone, Copy, Debug)]
struct BenchSample {
    frame_ms: f32,
    sections_drawn: u32,
    sections_total: u32,
    draw_calls: u32,
    timings: haste_render::FrameTimings,
    /// WorldFeed::feed time (dirty collection, submit, mesh upload).
    feed_ms: f32,
    /// Sections that were not drawn by the previous sample's frame (meshes became visible).
    sections_new: u32,
    /// Section meshes uploaded this frame and their vertex+index bytes.
    uploads: u32,
    upload_bytes: u64,
    /// Frame index since the measure phase started.
    index: u32,
}

/// Summary of a bench run: aggregate frame times plus the renderer's phase and pass averages.
#[derive(Debug)]
struct BenchSummary {
    frames: u32,
    avg_ms: f32,
    /// 1% low: the 99th percentile frame time.
    p99_ms: f32,
    worst_ms: f32,
    sections_drawn: f32,
    sections_total: f32,
    draw_calls: f32,
    prepare_ms: f32,
    acquire_ms: f32,
    cull_ms: f32,
    encode_ms: f32,
    submit_ms: f32,
    present_ms: f32,
    /// GPU time of the main, hand and GUI passes.
    gpu_ms: [f32; 3],
}

impl BenchSummary {
    fn fps(&self) -> f32 {
        if self.avg_ms > 0.0 {
            1000.0 / self.avg_ms
        } else {
            0.0
        }
    }

    /// One block of text, printed and also written to `--bench-out` as JSON.
    fn text(&self, meta: &BenchMeta) -> String {
        format!(
            "bench: {} frames in {:.0} s\n\
             frame time: avg {:.2} ms ({:.0} FPS), 1% low {:.2} ms, worst {:.2} ms\n\
             breakdown ms: prepare {:.2} acquire {:.2} cull {:.2} encode {:.2} submit {:.2} present {:.2} | gpu main {:.2} hand {:.2} gui {:.2}\n\
             sections drawn {}/{} avg, draw calls {:.0} avg\n\
             render distance {} (requested {}), {}x{}, adapter {}",
            self.frames,
            meta.seconds,
            self.avg_ms,
            self.fps(),
            self.p99_ms,
            self.worst_ms,
            self.prepare_ms,
            self.acquire_ms,
            self.cull_ms,
            self.encode_ms,
            self.submit_ms,
            self.present_ms,
            self.gpu_ms[0],
            self.gpu_ms[1],
            self.gpu_ms[2],
            self.sections_drawn,
            self.sections_total,
            self.draw_calls,
            meta.render_distance,
            meta.requested_render_distance,
            meta.width,
            meta.height,
            meta.adapter,
        )
    }
}

/// The `count` slowest sample indices, slowest first (earliest frame breaks ties). The
/// allocation is at the end of the bench, after all frames are recorded.
fn slowest_indices(samples: &[BenchSample], count: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..samples.len()).collect();
    order.sort_by(|&a, &b| {
        samples[b]
            .frame_ms
            .total_cmp(&samples[a].frame_ms)
            .then(samples[a].index.cmp(&samples[b].index))
    });
    order.truncate(count);
    order
}

/// The `count` slowest bench frames with their full breakdown: frame index and time, the
/// renderer phases (prepare/cull/encode/submit, GPU main pass), the feed phase, mesh uploads
/// (count and bytes) and how many sections the frame drew that the previous one did not.
/// GPU pass times arrive a frame or two late (timestamp readback), so they are the latest
/// available at that frame, not necessarily that exact frame's passes.
fn print_slowest_frames(samples: &[BenchSample]) {
    const COUNT: usize = 20;
    let order = slowest_indices(samples, COUNT);
    if order.is_empty() {
        return;
    }
    println!("slowest {} frames (of {}):", order.len(), samples.len());
    println!("  frame   ms | feed prep cull enc sub gpuM | up #/KB sect+ drawn/total calls");
    for &i in &order {
        let s = &samples[i];
        println!(
            "  {:5} {:5.2} | {:5.2} {:5.2} {:5.2} {:4.2} {:5.2} {:5.2} | {:3} {:5.0} {:4} {}/{} {}",
            s.index,
            s.frame_ms,
            s.feed_ms,
            s.timings.prepare_ms,
            s.timings.cull_ms,
            s.timings.encode_ms,
            s.timings.submit_ms,
            s.timings.gpu_ms[0],
            s.uploads,
            s.upload_bytes as f32 / 1024.0,
            s.sections_new,
            s.sections_drawn,
            s.sections_total,
            s.draw_calls,
        );
    }
}

/// Run metadata printed with the summary.
struct BenchMeta {
    seconds: f32,
    /// The effective render distance: the requested one capped by the server's view distance.
    render_distance: i32,
    requested_render_distance: u8,
    width: u32,
    height: u32,
    adapter: String,
}

/// Percentile of sorted ascending `values` in 0..=1 (linear interpolation between samples,
/// nearest at the ends). Matches `haste_testkit::Bench::percentile_frame_ms`'s definition.
fn percentile(sorted: &[f32], p: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let p = p.clamp(0.0, 1.0);
    let idx = (sorted.len() as f32 - 1.0) * p;
    let lo = idx.floor() as usize;
    let hi = idx.ceil() as usize;
    let frac = idx - lo as f32;
    sorted[lo] * (1.0 - frac) + sorted[hi.min(sorted.len() - 1)] * frac
}

/// Aggregates the samples into a summary (avg, 99th percentile, phase averages).
fn bench_summary(samples: &[BenchSample]) -> BenchSummary {
    let n = samples.len().max(1) as f32;
    let mut times: Vec<f32> = samples.iter().map(|s| s.frame_ms).collect();
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    fn sum(samples: &[BenchSample], n: f32, f: impl Fn(&BenchSample) -> f32) -> f32 {
        samples.iter().map(f).sum::<f32>() / n
    }
    let t = |f: fn(&haste_render::FrameTimings) -> f32| sum(samples, n, |s| f(&s.timings));
    let gpu = |i: usize| sum(samples, n, |s| s.timings.gpu_ms[i]);
    BenchSummary {
        frames: samples.len() as u32,
        avg_ms: sum(samples, n, |s| s.frame_ms),
        p99_ms: percentile(&times, 0.99),
        worst_ms: times.last().copied().unwrap_or(0.0),
        sections_drawn: sum(samples, n, |s| s.sections_drawn as f32),
        sections_total: sum(samples, n, |s| s.sections_total as f32),
        draw_calls: sum(samples, n, |s| s.draw_calls as f32),
        prepare_ms: t(|x| x.prepare_ms),
        acquire_ms: t(|x| x.acquire_ms),
        cull_ms: t(|x| x.cull_ms),
        encode_ms: t(|x| x.encode_ms),
        submit_ms: t(|x| x.submit_ms),
        present_ms: t(|x| x.present_ms),
        gpu_ms: [gpu(0), gpu(1), gpu(2)],
    }
}

/// `--bench SECONDS`: join, wait until the world settles (screenshot's logic), then render
/// offscreen at `--width`x`--height` along the fixed `bench_camera` path for SECONDS while
/// recording per-frame `FrameStats` and `FrameTimings`. Prints one summary block, writes the
/// same as JSON to `--bench-out` when given, and disconnects.
pub fn bench(args: &Args, seconds: u64) -> anyhow::Result<()> {
    let Some(_server) = args.server.as_ref() else {
        anyhow::bail!("--bench needs --server <host[:port]>");
    };
    let session = Session::join(args, false)?;
    let shared = session.shared.clone();
    // The bench camera is first person: the game thread's camera effects take that perspective.
    shared.input.lock().first_person = true;
    let mut assets = open_assets()?;
    let mut renderer =
        Renderer::new_offscreen(args.width, args.height, &mut assets, &shared.data.blocks)?;
    renderer.set_item_registry(&shared.data.registries);
    renderer.set_biomes(&session.biomes);
    renderer.set_particle_types(&session.particle_types);
    let gui = GuiFrame::default();
    let mut feed = WorldFeed::default();
    let deadline = Instant::now() + Duration::from_secs(args.seconds);
    let mut stats;
    let mut base;
    let mut vd;
    // Settle phase: identical to the screenshot path (wait for position, meshing done, quiet).
    loop {
        let (mut camera, sky, partial, v, animation_tick) = frame_view(
            &shared,
            game::Perspective::FirstPerson,
            0.0,
            0.0,
            &session.options,
            Instant::now(),
        );
        camera.yaw = args.yaw.unwrap_or(camera.yaw);
        camera.pitch = args.pitch.unwrap_or(camera.pitch);
        base = camera;
        vd = v;
        feed.feed(&shared, &mut renderer, camera_section(&camera), vd);
        feed.set_first_person(&shared, &mut renderer, game::Perspective::FirstPerson);
        let entities = feed.entities(&shared, &mut renderer, session.local, partial);
        let block_target = shared.view.lock().block_target;
        let breaking = shared.breaking.lock().clone();
        let frame = FrameInput {
            camera,
            entities,
            gui: &gui,
            sky,
            partial_tick: partial,
            block_target,
            breaking: &breaking,
            camera_entity: Some(session.local.0),
        };
        renderer.sample_world(&*shared.world.read(), &shared.data.shapes, &frame);
        renderer.set_animation_tick(animation_tick);
        {
            let world = shared.world.read();
            let mut particles = shared.particles.lock();
            renderer.prepare_particles(&mut particles, &*world, camera, partial);
        }
        stats = renderer.render(&frame)?;
        if shared.is_closed() {
            anyhow::bail!(
                "disconnected: {}",
                shared.closed.lock().clone().unwrap_or_default()
            );
        }
        let placed = shared.view.lock().has_position;
        if placed && feed.settled(&stats, Duration::from_millis(1500)) {
            break;
        }
        if Instant::now() > deadline {
            tracing::warn!("--seconds elapsed before the world settled; benching what is there");
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // Measure phase. The buffer is sized up front: at most 1000 fps for the whole run, and it
    // grows once if the machine is faster than that (no allocation during steady-state frames).
    let mut samples: Vec<BenchSample> = Vec::with_capacity(seconds as usize * 1000);
    let total = seconds as f32;
    let adapter = {
        let info = renderer.adapter_info();
        format!("{} ({})", info.name, info.backend)
    };
    let meta = BenchMeta {
        seconds: total,
        render_distance: vd,
        requested_render_distance: session.options.render_distance,
        width: args.width,
        height: args.height,
        adapter,
    };
    let started = Instant::now();
    loop {
        let elapsed = started.elapsed().as_secs_f32();
        if elapsed >= total {
            break;
        }
        // The bench camera follows its fixed path; the sky and entities come from the live view
        // (time of day moves, the player stays where the server put it).
        let (_, mut sky, partial_tick, _, animation_tick) = frame_view(
            &shared,
            game::Perspective::FirstPerson,
            0.0,
            0.0,
            &session.options,
            Instant::now(),
        );
        let camera = bench_camera(elapsed / total, &base, total);
        let fog_end = (vd.max(2) * 16) as f32;
        sky.fog_start = fog_end * 0.75;
        sky.fog_end = fog_end;
        let feed_start = Instant::now();
        feed.feed(&shared, &mut renderer, camera_section(&camera), vd);
        let feed_ms = feed_start.elapsed().as_secs_f32() * 1000.0;
        let entities = feed.entities(&shared, &mut renderer, session.local, partial_tick);
        let block_target = shared.view.lock().block_target;
        let breaking = shared.breaking.lock().clone();
        let frame = FrameInput {
            camera,
            entities,
            gui: &gui,
            sky,
            partial_tick,
            block_target,
            breaking: &breaking,
            camera_entity: Some(session.local.0),
        };
        renderer.sample_world(&*shared.world.read(), &shared.data.shapes, &frame);
        renderer.set_animation_tick(animation_tick);
        {
            let world = shared.world.read();
            let mut particles = shared.particles.lock();
            renderer.prepare_particles(&mut particles, &*world, camera, partial_tick);
        }
        let stats = renderer.render(&frame)?;
        let prev_drawn = samples.last().map_or(0, |s| s.sections_drawn);
        samples.push(BenchSample {
            frame_ms: stats.frame_ms,
            sections_drawn: stats.chunk_sections_drawn,
            sections_total: stats.chunk_sections_total,
            draw_calls: stats.draw_calls,
            timings: renderer.timings(),
            feed_ms,
            sections_new: stats.chunk_sections_drawn.saturating_sub(prev_drawn),
            uploads: stats.uploads,
            upload_bytes: stats.upload_bytes,
            index: samples.len() as u32,
        });
        if shared.is_closed() {
            anyhow::bail!(
                "disconnected during the bench: {}",
                shared.closed.lock().clone().unwrap_or_default()
            );
        }
    }
    let summary = bench_summary(&samples);
    println!("{}", summary.text(&meta));
    print_slowest_frames(&samples);
    if let Some(out) = &args.bench_out {
        let json = serde_json::json!({
            "frames": summary.frames,
            "seconds": total,
            "avg_ms": summary.avg_ms,
            "fps": summary.fps(),
            "p99_ms": summary.p99_ms,
            "worst_ms": summary.worst_ms,
            "sections_drawn": summary.sections_drawn,
            "sections_total": summary.sections_total,
            "draw_calls": summary.draw_calls,
            "prepare_ms": summary.prepare_ms,
            "acquire_ms": summary.acquire_ms,
            "cull_ms": summary.cull_ms,
            "encode_ms": summary.encode_ms,
            "submit_ms": summary.submit_ms,
            "present_ms": summary.present_ms,
            "gpu_main_ms": summary.gpu_ms[0],
            "gpu_hand_ms": summary.gpu_ms[1],
            "gpu_gui_ms": summary.gpu_ms[2],
            "render_distance": meta.render_distance,
            "requested_render_distance": meta.requested_render_distance,
            "width": meta.width,
            "height": meta.height,
            "adapter": meta.adapter,
        });
        std::fs::write(out, serde_json::to_string_pretty(&json)?)?;
        println!("bench summary written to {}", out.display());
    }
    drop(session);
    Ok(())
}

/// Windowed mode.
pub fn run(args: &Args) -> anyhow::Result<()> {
    let session = Session::join(args, true)?;
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        shared: session.shared.clone(),
        biomes: session.biomes.clone(),
        debug_env: session.debug_env.clone(),
        particle_types: session.particle_types.clone(),
        // The flag wins over the saved option; without it, the saved option decides.
        vsync: args.vsync.unwrap_or(session.options.vsync),
        options: session.options.clone(),
        local: session.local,
        perspective: game::Perspective::default(),
        window: None,
        renderer: None,
        feed: WorldFeed::default(),
        ui: None,
        gui: GuiFrame::default(),
        input: InputRouter::default(),
        grabbed: false,
        screen_was_open: false,
        focused: true,
        breaking: Vec::new(),
        frame_stats: FrameStats::default(),
        fps: FpsCounter::new(),
        error: None,
        seconds: args.window_seconds,
        screenshot: false,
        started: Instant::now(),
    };
    event_loop.run_app(&mut app)?;
    let reason = session.shared.closed.lock().clone();
    drop(session);
    if let Some(e) = app.error {
        return Err(e);
    }
    if let Some(r) = reason {
        println!("connection ended: {r}");
    }
    Ok(())
}

struct App {
    shared: Arc<Shared>,
    biomes: Vec<haste_render::BiomeClimate>,
    debug_env: DebugEnv,
    /// The particle atlas types (uploaded to the renderer once it exists).
    particle_types: std::sync::Arc<haste_render::ParticleTypes>,
    vsync: bool,
    /// Saved options (FOV for rendering, sensitivity for mouse look).
    options: Options,
    /// The local player's entity id and UUID, for its own model in third person.
    local: (EntityId, u128),
    /// Current perspective (F5 cycles it).
    perspective: game::Perspective,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    feed: WorldFeed,
    /// The HUD's `Ui`, cloned from the game thread's once it is ready (the JVM takes seconds
    /// to start); `gui` stays the empty default until then.
    ui: Option<crate::ui::Ui>,
    gui: GuiFrame,
    /// The input half: keys, mouse, look and the routing to the UI and the game. `mouse`
    /// for the GUI frame comes from it (`input.mouse`).
    input: InputRouter,
    grabbed: bool,
    /// A screen was open in the last `sync_grab` (its closing re-grabs the cursor).
    screen_was_open: bool,
    focused: bool,
    breaking: Vec<(haste_protocol::haste_core::BlockPos, u8)>,
    /// The previous frame's statistics (F3 reads them between frames).
    frame_stats: FrameStats,
    fps: FpsCounter,
    error: Option<anyhow::Error>,
    /// Close the window after this long (`--window-seconds`, for unattended runs).
    seconds: Option<u64>,
    /// F2 pressed: the next frame is captured for a screenshot; once its async readback
    /// lands (a few frames later), `take_capture` hands it to a save thread.
    screenshot: bool,
    started: Instant,
}

impl App {
    fn set_grab(&mut self, grab: bool) {
        let Some(w) = &self.window else { return };
        if grab {
            let ok = w
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| w.set_cursor_grab(CursorGrabMode::Confined))
                .is_ok();
            w.set_cursor_visible(!ok);
            self.grabbed = ok;
            self.input.grabbed = ok;
        } else {
            let _ = w.set_cursor_grab(CursorGrabMode::None);
            w.set_cursor_visible(true);
            self.grabbed = false;
            self.input.grabbed = false;
        }
    }

    /// The UI's view of the grab, applied after a screen opens or closes and after vanilla code
    /// asks for a grab ("Back to Game"): while any screen is open the cursor is free (vanilla
    /// never grabs with a screen up, except the ones that lock it themselves like F8's pie
    /// menu, none of which exist here yet). When a screen closes with no grab request (chat
    /// Escape, "Back to Game"), vanilla re-grabs (`MouseHandler` grabs on `setScreen(null)`
    /// unless a menu asks otherwise); the frame after, we do the same.
    fn sync_grab(&mut self) {
        let (open, wanted) = match &self.ui {
            Some(ui) => (ui.screen_open(), ui.wants_mouse_grabbed()),
            None => return,
        };
        if open {
            self.screen_was_open = true;
            if self.grabbed {
                if let Some(ui) = &self.ui {
                    ui.set_mouse_grabbed(false);
                }
                self.set_grab(false);
                self.release_keys();
            }
        } else {
            let regrab = wanted || self.screen_was_open;
            self.screen_was_open = false;
            if !self.grabbed && regrab && self.focused {
                self.set_grab(true);
                if let Some(ui) = &self.ui {
                    ui.set_mouse_grabbed(true);
                }
            }
        }
    }

    /// Whether a vanilla screen is open (the UI owns all input then).
    fn screen_open(&self) -> bool {
        self.ui.as_ref().is_some_and(|ui| ui.screen_open())
    }

    /// Once per frame (in `draw`): whether the pause screen's "Disconnect" (or the death
    /// screen's "Title Screen") was clicked since the last frame, in which case the client
    /// leaves the server and the window closes.
    fn take_disconnect(&mut self, event_loop: &ActiveEventLoop) {
        if self
            .ui
            .as_ref()
            .is_some_and(|ui| ui.take_disconnect_request())
        {
            tracing::info!("disconnect requested by the UI");
            self.shared.stop.store(true, Ordering::Relaxed);
            event_loop.exit();
        }
    }

    fn release_keys(&mut self) {
        self.input.release();
        self.input.sample(&self.shared, self.perspective);
    }

    /// Mouse look speed from the saved sensitivity.
    fn look_deg_per_pixel(&self) -> f32 {
        look_deg_per_pixel(self.options.mouse_sensitivity)
    }

    fn draw(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_none() {
            return;
        }
        self.input.sample(&self.shared, self.perspective);
        // The screen that opened or closed since the last frame takes or frees the cursor,
        // and "Disconnect" ends the run.
        self.sync_grab();
        self.take_disconnect(event_loop);
        let renderer = self.renderer.as_mut().expect("checked above");
        let t0 = Instant::now();
        let (camera, sky, partial, vd, animation_tick) = frame_view(
            &self.shared,
            self.perspective,
            self.input.yaw,
            self.input.pitch,
            &self.options,
            Instant::now(),
        );
        self.feed
            .feed(&self.shared, renderer, camera_section(&camera), vd);
        let t1 = Instant::now();
        self.feed
            .set_first_person(&self.shared, renderer, self.perspective);
        let entities = self
            .feed
            .entities(&self.shared, renderer, self.local, partial);
        let block_target = self.shared.view.lock().block_target;
        self.breaking.clear();
        self.breaking
            .extend_from_slice(&self.shared.breaking.lock());
        let t2 = Instant::now();
        // The HUD: once the game thread's `Ui` is ready, clone it here (cheap, one per thread)
        // and render the frame through it; until then (JVM still starting, or failed: no JDK
        // or cache) the empty default draws no HUD.
        if self.ui.is_none() {
            self.ui = self.shared.ui.lock().clone();
            if let Some(ui) = &self.ui {
                ui.set_text_metrics(renderer.text_metrics());
            }
        }
        let (w, h) = renderer.size();
        if let Some(ui) = &self.ui {
            // F3's numbers and the water overlay's flag, every frame (cheap: short mutexes,
            // no allocation while the overlay is off). The section counts are the previous
            // frame's.
            let stats = self.frame_stats;
            let gpu = gpu_info(renderer);
            ui.set_debug(debug_info(
                &self.debug_env,
                &self.shared,
                renderer,
                (self.input.yaw, self.input.pitch),
                self.fps.fps as i32,
                &stats,
                entities.len() as i32,
                resolve_gui_scale(u32::from(self.options.gui_scale), w, h),
                &gpu,
            ));
            let view = *self.shared.view.lock();
            ui.set_eye_in_water(view.eye_in_water);
        }
        if let Some(ui) = &mut self.ui {
            let mouse = self.input.mouse;
            let scale = resolve_gui_scale(u32::from(self.options.gui_scale), w, h);
            let (gw, gh) = gui_scaled_size(w, h, scale);
            self.gui = ui.frame(gw, gh, scale, mouse, partial).clone();
        }
        let frame = FrameInput {
            camera,
            entities,
            gui: &self.gui,
            sky,
            partial_tick: partial,
            block_target,
            breaking: &self.breaking,
            camera_entity: (!self.perspective.draws_player()).then_some(self.local.0),
        };
        // Outline boxes of the target and the states of breaking blocks (one short read lock).
        renderer.sample_world(&*self.shared.world.read(), &self.shared.data.shapes, &frame);
        {
            let world = self.shared.world.read();
            let mut particles = self.shared.particles.lock();
            renderer.prepare_particles(&mut particles, &*world, camera, partial);
        }
        renderer.set_animation_tick(animation_tick);
        if self.screenshot {
            self.screenshot = false;
            renderer.request_capture();
        }
        let result = renderer.render(&frame);
        if let Ok(stats) = &result {
            self.frame_stats = *stats;
        }
        if let Some(result) = renderer.take_capture() {
            // Encode and write off the render thread: PNG compression of a full frame
            // would stall the frame loop (vanilla's chat line becomes a log line here).
            match result {
                Ok(image) => {
                    std::thread::spawn(move || match crate::screenshots::save(image) {
                        Ok(name) => tracing::info!("Saved screenshot as {name}"),
                        Err(e) => tracing::warn!("screenshot failed: {e:#}"),
                    });
                }
                Err(e) => tracing::warn!("screenshot readback failed: {e:#}"),
            }
        }
        let t3 = Instant::now();
        self.fps.phases(
            (t1 - t0).as_secs_f32() * 1e3,
            (t2 - t1).as_secs_f32() * 1e3,
            (t3 - t2).as_secs_f32() * 1e3,
            renderer.timings(),
        );
        match result {
            Ok(stats) => {
                if let Some(line) = self.fps.frame(&stats) {
                    if let Some(b) = self.fps.breakdown(renderer) {
                        tracing::info!("{b}");
                    }
                    if let Some(w) = &self.window {
                        w.set_title(&format!("Haste - {line}"));
                    }
                    tracing::info!("{line}");
                }
            }
            Err(e) => {
                self.error = Some(e.context("render"));
                event_loop.exit();
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Haste")
            .with_inner_size(winit::dpi::LogicalSize::new(854.0, 480.0));
        let result = (|| -> anyhow::Result<(Arc<Window>, Renderer)> {
            let window = Arc::new(event_loop.create_window(attrs)?);
            let mut assets = open_assets()?;
            let mut renderer =
                Renderer::new(window.clone(), &mut assets, &self.shared.data.blocks)?;
            renderer.set_item_registry(&self.shared.data.registries);
            renderer.set_biomes(&self.biomes);
            renderer.set_particle_types(&self.particle_types);
            Ok((window, renderer))
        })();
        match result {
            Ok((w, mut r)) => {
                r.set_vsync(self.vsync);
                self.window = Some(w);
                self.renderer = Some(r);
                self.set_grab(true);
            }
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::CursorMoved { position, .. } => {
                // GUI-scaled pixels: physical size divided by the GUI scale. The scale is
                // recomputed from the current framebuffer (the renderer knows the size;
                // vanilla updates it the same way on every resize: `Minecraft.resizeGui`).
                if let Some(r) = &self.renderer {
                    let s = f64::from(resolve_gui_scale(
                        u32::from(self.options.gui_scale),
                        r.size().0,
                        r.size().1,
                    ));
                    self.input
                        .mouse_moved(position.x / s, position.y / s, self.ui.as_ref());
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(size.width.max(1), size.height.max(1));
                }
            }
            WindowEvent::Focused(f) => {
                self.focused = f;
                self.input.focused = f;
                if !f {
                    self.release_keys();
                    self.set_grab(false);
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } if !self.grabbed && !self.screen_open() => self.set_grab(true),
            WindowEvent::MouseInput { state, button, .. } if self.focused => {
                let pressed = state == ElementState::Pressed;
                let (x, y) = self.input.mouse;
                self.input
                    .mouse_button(vanilla_button(button), pressed, x, y, self.ui.as_ref());
            }
            WindowEvent::MouseWheel { delta, .. } if self.focused => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x as f64, y as f64),
                    MouseScrollDelta::PixelDelta(p) => (p.x / 40.0, p.y / 40.0),
                };
                let (x, y) = self.input.mouse;
                self.input
                    .wheel(dx, dy, x, y, self.ui.as_ref(), &self.shared);
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(code),
                        state,
                        repeat,
                        text,
                        ..
                    },
                ..
            } => {
                let pressed = state == ElementState::Pressed;
                // The shared routing (UI first, then the game). Its Escape branch frees the
                // cursor; the window-only keys (F5/F11/F2) stay here.
                self.input
                    .key(code, pressed, repeat, text.as_deref(), self.ui.as_ref());
                if pressed && !repeat && self.focused {
                    match code {
                        KeyCode::F11 => {
                            if let Some(w) = &self.window {
                                let full = w.fullscreen().is_some();
                                w.set_fullscreen((!full).then_some(Fullscreen::Borderless(None)));
                            }
                        }
                        // F5 cycles the perspective (vanilla's three). In third person there
                        // is no first-person hand.
                        KeyCode::F5 => {
                            self.perspective = self.perspective.next();
                            if let Some(r) = &mut self.renderer {
                                r.set_first_person(None);
                            }
                        }
                        KeyCode::F2 => {
                            // Vanilla's screenshot key: capture on the next frame (`draw`),
                            // where the render (and its offscreen target) happens.
                            self.screenshot = true;
                        }
                        _ => {}
                    }
                }
            }
            WindowEvent::RedrawRequested => self.draw(event_loop),
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if self.grabbed && self.focused {
                let (yaw, pitch) = apply_mouse(
                    self.input.yaw,
                    self.input.pitch,
                    delta,
                    self.look_deg_per_pixel(),
                );
                self.input.look(yaw, pitch);
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.shared.is_closed() {
            event_loop.exit();
            return;
        }
        if self
            .seconds
            .is_some_and(|s| self.started.elapsed() >= Duration::from_secs(s))
        {
            event_loop.exit();
            return;
        }
        // Continuous rendering: draw here rather than waiting for RedrawRequested (Wayland
        // frame callbacks stall an unviewed window, NOTES.md).
        self.draw(event_loop);
    }
}

/// Mouse look: yaw wraps freely, pitch is clamped to straight up/down. `deg_per_pixel` is
/// [`look_deg_per_pixel`] of the saved sensitivity.
pub fn apply_mouse(yaw: f32, pitch: f32, delta: (f64, f64), deg_per_pixel: f32) -> (f32, f32) {
    let yaw = yaw + delta.0 as f32 * deg_per_pixel;
    let pitch = (pitch + delta.1 as f32 * deg_per_pixel).clamp(-90.0, 90.0);
    (yaw, pitch)
}

/// Frame-time statistics for the title bar, reported twice a second.
struct FpsCounter {
    since: Instant,
    frames: u32,
    worst_ms: f32,
    last: Instant,
    /// Smoothed frames per second (the F3 overlay reads it every frame).
    fps: f32,
    /// Per-phase sums since the last breakdown: feed, entities, render, and the renderer's
    /// own phases; logged every 2 s.
    phase_sum: [f32; 3],
    render_sum: haste_render::FrameTimings,
    phase_frames: u32,
    phase_since: Instant,
}

impl FpsCounter {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            since: now,
            frames: 0,
            worst_ms: 0.0,
            last: now,
            fps: 0.0,
            phase_sum: [0.0; 3],
            render_sum: Default::default(),
            phase_frames: 0,
            phase_since: now,
        }
    }

    fn phases(&mut self, feed: f32, entities: f32, render: f32, t: haste_render::FrameTimings) {
        self.phase_sum[0] += feed;
        self.phase_sum[1] += entities;
        self.phase_sum[2] += render;
        let s = &mut self.render_sum;
        s.prepare_ms += t.prepare_ms;
        s.acquire_ms += t.acquire_ms;
        s.cull_ms += t.cull_ms;
        s.encode_ms += t.encode_ms;
        s.submit_ms += t.submit_ms;
        s.present_ms += t.present_ms;
        for i in 0..3 {
            s.gpu_ms[i] += t.gpu_ms[i];
        }
        self.phase_frames += 1;
    }

    /// Average per-phase milliseconds over the last 2 s.
    fn breakdown(&mut self, renderer: &Renderer) -> Option<String> {
        if self.phase_since.elapsed() < Duration::from_secs(2) || self.phase_frames == 0 {
            return None;
        }
        let n = self.phase_frames as f32;
        let s = self.render_sum;
        let size = renderer.size();
        let line = format!(
            "frame ms: feed {:.2} entities {:.2} render {:.2} (prepare {:.2} acquire {:.2} cull {:.2} encode {:.2} submit {:.2} present {:.2}) gpu main {:.2} hand {:.2} gui {:.2}; {}x{} {:?}",
            self.phase_sum[0] / n,
            self.phase_sum[1] / n,
            self.phase_sum[2] / n,
            s.prepare_ms / n,
            s.acquire_ms / n,
            s.cull_ms / n,
            s.encode_ms / n,
            s.submit_ms / n,
            s.present_ms / n,
            s.gpu_ms[0] / n,
            s.gpu_ms[1] / n,
            s.gpu_ms[2] / n,
            size.0,
            size.1,
            renderer.present_mode(),
        );
        self.phase_sum = [0.0; 3];
        self.render_sum = Default::default();
        self.phase_frames = 0;
        self.phase_since = Instant::now();
        Some(line)
    }

    fn frame(&mut self, stats: &FrameStats) -> Option<String> {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32() * 1000.0;
        self.last = now;
        self.frames += 1;
        self.worst_ms = self.worst_ms.max(dt);
        // Exponential moving average: smooth enough for F3's every-frame read, settles within
        // a second of a real change.
        let instant = 1000.0 / dt.max(0.001);
        self.fps = if self.fps == 0.0 {
            instant
        } else {
            self.fps * 0.95 + instant * 0.05
        };
        let span = now.duration_since(self.since).as_secs_f32();
        if span < 0.5 {
            return None;
        }
        let line = format!(
            "{:.0} FPS, {:.2} ms avg, {:.2} ms worst, sections {}/{}, pending {}",
            self.frames as f32 / span,
            span * 1000.0 / self.frames as f32,
            self.worst_ms,
            stats.chunk_sections_drawn,
            stats.chunk_sections_total,
            stats.meshes_pending
        );
        self.since = now;
        self.frames = 0;
        self.worst_ms = 0.0;
        Some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DPP: f32 = 0.15; // vanilla's default sensitivity (50%) value

    #[test]
    fn mouse_look_clamps_pitch() {
        let (yaw, pitch) = apply_mouse(0.0, 0.0, (100.0, 0.0), DPP);
        assert!((yaw - 15.0).abs() < 1e-4 && pitch == 0.0, "{yaw} {pitch}");
        assert_eq!(apply_mouse(0.0, 80.0, (0.0, 200.0), DPP), (0.0, 90.0));
        assert_eq!(apply_mouse(10.0, -80.0, (0.0, -200.0), DPP), (10.0, -90.0));
    }

    #[test]
    fn sensitivity_formula_matches_vanilla_curve() {
        // Default sensitivity 50%: (0.5 * 0.6 + 0.2)^3 * 8 * 0.15 = 0.4^3 * 8 * 0.15.
        assert_eq!(look_deg_per_pixel(0.5), 0.15);
        // Off: the curve's minimum.
        assert_eq!(look_deg_per_pixel(0.0), 0.0096);
        // Max sensitivity: 0.8^3 * 8 * 0.15.
        assert!((look_deg_per_pixel(1.0) - 0.6144).abs() < 1e-6);
    }

    #[test]
    fn auto_gui_scale_matches_vanilla_resolutions() {
        // `Window.calculateScale(0, ...)` at vanilla's default window and common
        // resolutions: the scale grows while the framebuffer keeps >= 320x240 GUI
        // pixels. Computed by hand from the vanilla method's conditions.
        assert_eq!(gui_scale_for(0, 854, 480), 2);
        assert_eq!(gui_scale_for(0, 1280, 720), 3);
        assert_eq!(gui_scale_for(0, 1920, 1080), 4);
        assert_eq!(gui_scale_for(0, 2560, 1440), 6);
        assert_eq!(gui_scale_for(0, 3840, 2160), 9);
    }

    #[test]
    fn fixed_gui_scale_is_honoured_and_clamped() {
        // A fixed option stops there (vanilla: `calculateScale(maxScale, ...)`); a value
        // too large for the framebuffer is still bounded by the 320x240 rule.
        assert_eq!(gui_scale_for(1, 1920, 1080), 1);
        assert_eq!(gui_scale_for(3, 1920, 1080), 3);
        assert_eq!(gui_scale_for(4, 854, 480), 2);
        // The option is clamped to 0..=4 like `Options.sanitized`.
        assert_eq!(resolve_gui_scale(9, 1920, 1080), 4);
        assert_eq!(resolve_gui_scale(0, 1920, 1080), 4);
        // Tiny framebuffers still get scale 1 (the loop stops immediately).
        assert_eq!(gui_scale_for(0, 320, 240), 1);
        assert_eq!(gui_scale_for(0, 100, 100), 1);
    }

    #[test]
    fn gui_scaled_size_uses_ceiling_division() {
        // `Window.setGuiScale` ceils, so the GUI covers the whole framebuffer.
        assert_eq!(gui_scaled_size(854, 480, 2), (427, 240));
        assert_eq!(gui_scaled_size(853, 479, 2), (427, 240));
        assert_eq!(gui_scaled_size(1920, 1080, 4), (480, 270));
    }

    #[test]
    fn sdl_scancodes_match_the_hid_usage_table() {
        use haste_jvm::keys as sdl;
        // Letters, digits and punctuation: the consecutive HID ranges.
        assert_eq!(sdl_scancode(KeyCode::KeyA), Some(4));
        assert_eq!(sdl_scancode(KeyCode::KeyZ), Some(29));
        assert_eq!(sdl_scancode(KeyCode::Digit1), Some(sdl::NUM_1));
        assert_eq!(sdl_scancode(KeyCode::Digit0), Some(sdl::NUM_0));
        assert_eq!(sdl_scancode(KeyCode::Slash), Some(56));
        // The control keys and F1-F12, one entry each.
        assert_eq!(sdl_scancode(KeyCode::Escape), Some(41));
        assert_eq!(sdl_scancode(KeyCode::Enter), Some(40));
        assert_eq!(sdl_scancode(KeyCode::Backspace), Some(42));
        assert_eq!(sdl_scancode(KeyCode::Tab), Some(43));
        assert_eq!(sdl_scancode(KeyCode::Space), Some(44));
        assert_eq!(sdl_scancode(KeyCode::Delete), Some(76));
        assert_eq!(sdl_scancode(KeyCode::Home), Some(74));
        assert_eq!(sdl_scancode(KeyCode::End), Some(77));
        assert_eq!(sdl_scancode(KeyCode::PageUp), Some(75));
        assert_eq!(sdl_scancode(KeyCode::PageDown), Some(78));
        assert_eq!(sdl_scancode(KeyCode::ArrowUp), Some(82));
        assert_eq!(sdl_scancode(KeyCode::ArrowDown), Some(81));
        assert_eq!(sdl_scancode(KeyCode::ArrowLeft), Some(80));
        assert_eq!(sdl_scancode(KeyCode::ArrowRight), Some(79));
        for (i, code) in [
            KeyCode::F1,
            KeyCode::F2,
            KeyCode::F3,
            KeyCode::F4,
            KeyCode::F5,
            KeyCode::F6,
            KeyCode::F7,
            KeyCode::F8,
            KeyCode::F9,
            KeyCode::F10,
            KeyCode::F11,
            KeyCode::F12,
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(sdl_scancode(code), Some(58 + i as i32), "{code:?}");
        }
        // Modifiers: the shared left codes, the right ones one up.
        assert_eq!(sdl_scancode(KeyCode::ControlLeft), Some(224));
        assert_eq!(sdl_scancode(KeyCode::ShiftLeft), Some(225));
        assert_eq!(sdl_scancode(KeyCode::ControlRight), Some(228));
        assert_eq!(sdl_scancode(KeyCode::ShiftRight), Some(229));
        // The keycodes the screens match on (the US-layout mapping of the scancode).
        assert_eq!(keys::keycode(sdl_scancode(KeyCode::Escape).unwrap()), 27);
        assert_eq!(keys::keycode(sdl_scancode(KeyCode::Enter).unwrap()), 13);
        assert_eq!(keys::keycode(sdl_scancode(KeyCode::Backspace).unwrap()), 8);
    }

    #[test]
    fn sdl_keymod_combines_the_modifier_masks() {
        use haste_jvm::keys as sdl;
        assert_eq!(sdl_keymod(false, false, false), 0);
        assert_eq!(sdl_keymod(true, false, false), sdl::MOD_CONTROL);
        assert_eq!(sdl_keymod(false, true, false), sdl::MOD_SHIFT);
        assert_eq!(sdl_keymod(false, false, true), sdl::MOD_ALT);
        let all = sdl_keymod(true, true, true);
        assert_eq!(all & sdl::MOD_CONTROL, sdl::MOD_CONTROL);
        assert_eq!(all & sdl::MOD_SHIFT, sdl::MOD_SHIFT);
        assert_eq!(all & sdl::MOD_ALT, sdl::MOD_ALT);
    }

    #[test]
    fn typed_char_takes_the_first_character() {
        assert_eq!(typed_char_of(None), None);
        assert_eq!(typed_char_of(Some("a")), Some('a'));
        assert_eq!(typed_char_of(Some("é")), Some('é'));
    }

    #[test]
    fn key_routing_goes_to_the_ui_first() {
        // With no screen: the first press and the release reach the UI, repeats do not
        // (holding a HUD key must not re-trigger).
        assert!(key_goes_to_ui(false, false));
        assert!(key_goes_to_ui(false, true));
        assert!(!key_goes_to_ui(true, false));
        // With a screen open: everything reaches it, repeats included (held Backspace in
        // the chat box).
        assert!(key_goes_to_ui(true, true));
    }

    #[test]
    fn key_routing_blocks_game_input_when_the_ui_takes_it() {
        // The UI used it: no game input, screen or not.
        assert!(!game_uses_key(true, false));
        assert!(!game_uses_key(true, true));
        // The UI did not use it and no screen is open: the game may use it.
        assert!(game_uses_key(false, false));
        // A screen is open: no game input even for keys the screen ignored (movement stops
        // while chat or the inventory is up).
        assert!(!game_uses_key(false, true));
    }

    #[test]
    fn script_keys_map_to_their_winit_codes() {
        // The script's key names and winit's KeyCodes must be one-to-one, so a scripted key
        // and a windowed key press route identically.
        use crate::script::Key as K;
        assert_eq!(winit_key(K::T), KeyCode::KeyT);
        assert_eq!(winit_key(K::E), KeyCode::KeyE);
        assert_eq!(winit_key(K::Escape), KeyCode::Escape);
        assert_eq!(winit_key(K::Enter), KeyCode::Enter);
        assert_eq!(winit_key(K::LeftShift), KeyCode::ShiftLeft);
        assert_eq!(winit_key(K::Num1), KeyCode::Digit1);
        // Every script key must reach the SDL numbering (the scancode lookup must not miss).
        let all = [
            K::A,
            K::B,
            K::C,
            K::D,
            K::E,
            K::F,
            K::G,
            K::H,
            K::I,
            K::J,
            K::K,
            K::L,
            K::M,
            K::N,
            K::O,
            K::P,
            K::Q,
            K::R,
            K::S,
            K::T,
            K::U,
            K::V,
            K::W,
            K::X,
            K::Y,
            K::Z,
            K::Num0,
            K::Num1,
            K::Num2,
            K::Num3,
            K::Num4,
            K::Num5,
            K::Num6,
            K::Num7,
            K::Num8,
            K::Num9,
            K::F1,
            K::F2,
            K::F3,
            K::F4,
            K::F5,
            K::F6,
            K::F7,
            K::F8,
            K::F9,
            K::F10,
            K::F11,
            K::F12,
            K::Escape,
            K::Enter,
            K::Tab,
            K::Space,
            K::Backspace,
            K::Up,
            K::Down,
            K::Left,
            K::Right,
            K::LeftShift,
            K::LeftCtrl,
            K::LeftAlt,
        ];
        for k in all {
            assert!(sdl_scancode(winit_key(k)).is_some(), "{k:?}");
        }
    }

    #[test]
    fn script_actions_route_like_window_events() {
        // The scripted walk key and a held windowed walk key both end up in the movement
        // input (the InputRouter is the same code path); a grab is what a script starts with.
        let mut r = InputRouter {
            grabbed: true,
            ..InputRouter::default()
        };
        r.key(KeyCode::KeyW, true, false, None, None);
        r.key(KeyCode::Space, true, false, None, None);
        let k = |c: KeyCode| r.keys.contains(&c);
        assert!(k(KeyCode::KeyW) && k(KeyCode::Space));
        // Release clears it.
        r.key(KeyCode::KeyW, false, false, None, None);
        assert!(!r.keys.contains(&KeyCode::KeyW));
        // Without the grab (a screen is open in the windowed client), the game key never
        // lands: that is what keeps a scripted movement key from moving while chat is open.
        let mut r2 = InputRouter::default();
        r2.key(KeyCode::KeyW, true, false, None, None);
        assert!(!r2.keys.contains(&KeyCode::KeyW));
        // The hotbar digits and the wheel go through the interact path.
        let mut r3 = InputRouter {
            grabbed: true,
            ..InputRouter::default()
        };
        r3.key(KeyCode::Digit5, true, false, None, None);
        assert_eq!(r3.hotbar, Some(4));
    }

    fn sample(frame_ms: f32) -> BenchSample {
        BenchSample {
            frame_ms,
            sections_drawn: 900,
            sections_total: 4000,
            draw_calls: 200,
            timings: haste_render::FrameTimings {
                prepare_ms: 1.0,
                cull_ms: 2.0,
                encode_ms: 3.0,
                submit_ms: 4.0,
                ..Default::default()
            },
            feed_ms: 0.5,
            sections_new: 0,
            uploads: 0,
            upload_bytes: 0,
            index: 0,
        }
    }

    #[test]
    fn percentile_interpolates_and_clamps() {
        let v = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&v, 0.0), 1.0);
        assert_eq!(percentile(&v, 1.0), 4.0);
        // Halfway between 3 and 4: index (4-1)*0.75 = 2.25.
        assert!((percentile(&v, 0.75) - 3.25).abs() < 1e-6);
        assert!(percentile(&v, 2.0) <= 4.0); // out-of-range p clamps
        assert_eq!(percentile(&[], 0.5), 0.0);
    }

    #[test]
    fn bench_summary_averages_and_percentile() {
        // 200 samples: 199 at 10 ms, one at 100 ms -> the 1% low (p99) is still a 10 ms frame,
        // the outlier sits above the 99th percentile.
        let samples: Vec<BenchSample> = (0..200)
            .map(|i| sample(if i == 0 { 100.0 } else { 10.0 }))
            .collect();
        let s = bench_summary(&samples);
        assert_eq!(s.frames, 200);
        // f32: (199*10 + 100)/200 accumulates to 10.45.
        assert!((s.avg_ms - 10.45).abs() < 1e-2, "{}", s.avg_ms);
        assert!((s.p99_ms - 10.0).abs() < 1e-2, "{}", s.p99_ms);
        assert_eq!(s.worst_ms, 100.0);
        assert!((s.fps() - 1000.0 / 10.45).abs() < 1e-2, "{}", s.fps());
        assert!((s.prepare_ms - 1.0).abs() < 1e-6);
        assert!((s.cull_ms - 2.0).abs() < 1e-6);
        assert!((s.encode_ms - 3.0).abs() < 1e-6);
        assert!((s.submit_ms - 4.0).abs() < 1e-6);
        assert!((s.sections_drawn - 900.0).abs() < 1e-6);
        assert!((s.draw_calls - 200.0).abs() < 1e-6);
    }

    #[test]
    fn slowest_indices_slowest_first_with_ties_by_frame() {
        let mut slow = sample(50.0);
        slow.index = 10;
        let mut samples: Vec<BenchSample> = (0..49).map(|_| sample(5.0)).collect();
        samples.push(slow);
        let order = slowest_indices(&samples, 20);
        assert_eq!(order.len(), 20);
        assert_eq!(order[0], 49, "the 50 ms frame leads");
        // All 5 ms frames tie; the earliest frames come first among them.
        assert_eq!(order[1], 0);
        assert_eq!(order[2], 1);
        // Asking for more than there are returns all, and an empty run returns none.
        assert_eq!(slowest_indices(&samples, 100).len(), 50);
        assert!(slowest_indices(&[], 20).is_empty());
        print_slowest_frames(&samples);
    }

    #[test]
    fn bench_camera_path_turns_then_flies() {
        let base = Camera {
            pos: DVec3::new(100.0, 64.0, -200.0),
            yaw: 30.0,
            pitch: 10.0,
            fov_deg: 70.0,
            roll_deg: 0.0,
            bob: haste_render::ViewBob::default(),
        };
        // t = 0: at the base, unturned.
        let c = bench_camera(0.0, &base, 60.0);
        assert_eq!(c.pos, base.pos);
        assert_eq!(c.yaw, 30.0);
        // t = half: a full 360 turned; flight begins here (y = eye + 20, no distance yet).
        let c = bench_camera(0.5, &base, 60.0);
        assert!((c.pos.y - 84.0).abs() < 1e-4, "{}", c.pos.y);
        assert!((c.pos.x - 100.0).abs() < 1e-4, "{}", c.pos.x);
        assert!((c.yaw - 390.0).abs() < 1e-4, "{}", c.yaw);
        // t = 1: 30 s of flight at 10 b/s along +X, 20 blocks up.
        let c = bench_camera(1.0, &base, 60.0);
        assert!((c.pos.x - 400.0).abs() < 1e-4, "{}", c.pos.x);
        assert!((c.pos.y - 84.0).abs() < 1e-4, "{}", c.pos.y);
        assert!((c.yaw - 390.0).abs() < 1e-4);
        assert_eq!(c.pos.z, base.pos.z);
        // Quarter point: 180 degrees around, no flight yet.
        let c = bench_camera(0.25, &base, 60.0);
        assert!((c.yaw - 210.0).abs() < 1e-4, "{}", c.yaw);
        assert_eq!(c.pos, base.pos);
        // Just after half: climbing already done, flight starting.
        let c = bench_camera(0.51, &base, 60.0);
        assert!((c.pos.y - 84.0).abs() < 1e-4, "{}", c.pos.y);
        // Three-quarter point: 15 s flown = 150 blocks along +X.
        let c = bench_camera(0.75, &base, 60.0);
        assert!((c.pos.x - 250.0).abs() < 1e-4, "{}", c.pos.x);
        assert!((c.pos.y - 84.0).abs() < 1e-4, "{}", c.pos.y);
    }

    #[test]
    fn frame_fov_matches_vanilla_calculate_fov() {
        // `Camera.calculateFov`: base FOV times the interpolated modifier.
        assert_eq!(frame_fov(70.0, 1.0, 1.0, 0.5, false, 0, 1.0), 70.0);
        assert_eq!(frame_fov(70.0, 1.0, 1.5, 0.5, false, 0, 1.0), 70.0 * 1.25);
        // Under water/lava: `lerp(effectScale, 1.0, 0.85714287)`.
        assert!((frame_fov(70.0, 1.0, 1.0, 0.0, true, 0, 1.0) - 60.0).abs() < 1e-3);
        assert!((frame_fov(70.0, 1.0, 1.0, 0.0, true, 0, 0.5) - 65.0).abs() < 1e-4);
        // The effect scale off leaves it alone.
        assert_eq!(frame_fov(70.0, 1.0, 1.0, 0.0, true, 0, 0.0), 70.0);
        // Death: `fov /= (1 - 500/(duration+500))*2 + 1`, deathTime clamped at 20.
        assert!((frame_fov(70.0, 1.0, 1.0, 0.0, false, 20, 1.0) - 65.0).abs() < 1e-4);
        assert!((frame_fov(70.0, 1.0, 1.0, 0.0, false, 200, 1.0) - 65.0).abs() < 1e-4);
    }

    #[test]
    fn hurt_tilt_matches_bob_hurt_quartic_curve() {
        // `GameRenderer.bobHurt`: `-sin(h^4 * PI) * 14 * strength`, h = hurtTime / 10.
        // hurtTime = 5, partial 0 => h = 0.5, h^4 = 0.0625.
        assert!((hurt_tilt_degrees(5, 0.0, 1.0) + 2.73127).abs() < 1e-4);
        // hurtTime = 9 (10 ticks, partial 1) => h = 0.9, h^4 = 0.6561.
        assert!((hurt_tilt_degrees(10, 1.0, 1.0) + 12.3500).abs() < 1e-3);
        // The peak is 14 degrees where h^4 = 0.5, not at the start (the curve is quartic).
        assert!(hurt_tilt_degrees(10, 0.0, 1.0).abs() < 1e-4);
        // Strength scales it; the hurt time running out (hurtTime - partial < 0) is 0.
        assert!((hurt_tilt_degrees(5, 0.0, 0.5) + 1.36564).abs() < 1e-4);
        assert_eq!(hurt_tilt_degrees(0, 0.5, 1.0), 0.0);
    }

    #[test]
    fn death_roll_matches_bob_hurt_death_curve() {
        // `GameRenderer.bobHurt`: `40 - 8000/(deathTime + 200)`, deathTime clamped at 20.
        assert_eq!(death_roll_degrees(0, 0.0), 0.0);
        assert!((death_roll_degrees(20, 0.0) - 3.6363636).abs() < 1e-4);
        assert!((death_roll_degrees(200, 0.0) - 3.6363636).abs() < 1e-4);
        assert!((death_roll_degrees(0, 10.0) - 1.9047619).abs() < 1e-4);
    }

    #[test]
    fn view_bob_matches_bob_view() {
        // `GameRenderer.bobView` with walk*PI = 3*PI/2 (walk = -1.5) and bob interpolated to
        // 0.5: translation (0.25, 0), roll 1.5 degrees, pitch 0.49667 degrees.
        let (x, y, roll, pitch) = view_bob(1.0, 0.0, 1.0, 0.0, 0.5);
        assert!((x - 0.25).abs() < 1e-4, "{x}");
        assert!(y.abs() < 1e-4, "{y}");
        assert!((roll - 1.5).abs() < 1e-4, "{roll}");
        assert!((pitch - 0.49667).abs() < 1e-4, "{pitch}");
        // No walk distance and no bob: nothing moves.
        assert_eq!(view_bob(0.0, 0.0, 0.0, 0.0, 0.5), (0.0, 0.0, 0.0, 0.0));
    }
}
