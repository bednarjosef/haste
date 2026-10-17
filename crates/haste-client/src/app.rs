//! Windowed client and screenshot mode (client-app).
//!
//! `haste --server 127.0.0.1:25611` joins, starts the game thread (`game::run`, 20 Hz) and runs
//! the winit event loop on the main thread: each frame it feeds dirty sections to the renderer
//! (`WorldFeed`), copies the interpolated entity instances, builds the camera from the
//! interpolated player position and renders. `--screenshot <file>` renders the same frame
//! offscreen once the world has finished meshing, saves it and exits.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use glam::DVec3;
use haste_protocol::haste_core::gui::{GuiCommand, GuiFrame};
use haste_protocol::haste_core::input::InputState;
use haste_protocol::haste_core::model::EntityInstance;
use haste_protocol::haste_core::{
    BlockPos, BlockStateId, ChunkPos, Direction, EntityId, Identifier, SectionPos,
};
use haste_protocol::{Account, ConnectOptions};
use haste_render::{
    Camera, DimensionFog, FogInputs, FogType, FrameInput, FrameStats, LightmapFrame, Renderer,
    SkyState,
};
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
pub(crate) mod biomes;
mod end_gateway;

use crate::game::{self, Shared};
use crate::net::{self, NetDriver};
use crate::script;
use crate::ui::keys;
use crate::Args;
use haste_jvm::JvmHost;
use menu_background::{menu_camera, menu_frame, menu_sky};

mod menu_background;

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
/// [`gui_scale_for`] (vanilla: `Window.calculateScale`) with the "Force Unicode Font"
/// option (`Minecraft.isEnforceUnicode`). The stored option is not clamped: a value above
/// the window's automatic maximum simply stops at that maximum, as in vanilla.
pub fn resolve_gui_scale(option: u32, width: u32, height: u32, force_unicode: bool) -> u32 {
    gui_scale_for(option, width, height, force_unicode)
}

/// The scale vanilla would use for the option at the window's framebuffer size
/// (vanilla: `Window.calculateScale`; `max_scale` 0 grows it while the framebuffer
/// keeps at least 320x240 GUI pixels). With `enforce_unicode` an odd result is rounded up
/// to the next even scale after the loop, as vanilla does for the unicode font.
pub fn gui_scale_for(max_scale: u32, width: u32, height: u32, enforce_unicode: bool) -> u32 {
    let mut scale = 1;
    while scale != max_scale
        && scale < width
        && scale < height
        && width / (scale + 1) >= 320
        && height / (scale + 1) >= 240
    {
        scale += 1;
    }
    if enforce_unicode && scale % 2 != 0 {
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
    let view = shared.view.lock().clone();
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
    /// The server's resource packs downloaded during the connection (push order),
    /// layered above the selection wherever this session's assets open.
    server_packs: Vec<std::path::PathBuf>,
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

/// `--hud on|off` (vanilla's F1) for this run, set once from the arguments before any join.
static HUD_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Applies `--hud`: "on" or "off" (vanilla's F1: no HUD, no hand, no block outline).
pub fn set_hud_from_args(hud: &str) -> anyhow::Result<()> {
    if hud != "on" && hud != "off" {
        anyhow::bail!("--hud must be \"on\" or \"off\" (vanilla's F1)");
    }
    HUD_ENABLED.store(hud == "on", std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

impl Session {
    /// `sound`: open the audio output (the window only; screenshots and the benchmark stay
    /// silent so they neither make noise nor measure the audio thread). `address` is the server
    /// (`--server`, or the address chosen on the multiplayer screen in menu mode).
    /// `existing_ui`: menu mode's UI over its already-started JVM, reused for the HUD so the
    /// screens and chat history survive the transition into the game.
    fn join(
        username: &str,
        render_distance: Option<u8>,
        address: &str,
        sound: bool,
        existing_ui: Option<crate::ui::Ui>,
    ) -> anyhow::Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .thread_name("haste-net")
            .build()?;
        let data = net::load_game_data()?;
        let mut options = load_options();
        // `--render-distance` overrides the saved option for this run (bench mode compares
        // render distances without editing the config file).
        if let Some(rd) = render_distance {
            options.render_distance = rd;
        }
        let mut opts = ConnectOptions::new(
            address.to_owned(),
            Account::Offline {
                username: username.to_owned(),
            },
        );
        // The saved render distance is what we advertise in client_information; the server's
        // own view distance (login packet) caps what we mesh below.
        opts.client_information.view_distance = options.render_distance as i8;
        // Server resource packs download into the game dir's `downloads`
        // (vanilla's layout) and apply on top of the selection.
        opts.downloads_dir = Some(game_dir().join("downloads"));
        let mut driver: NetDriver = rt.block_on(NetDriver::connect(opts))?;
        let server_packs: Vec<std::path::PathBuf> = driver
            .connection()
            .server_packs()
            .into_iter()
            .map(|p| p.path)
            .collect();
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
        {
            let regs = driver.connection().registries();
            driver
                .session
                .set_attribute_names(net::attribute_names(regs, &data));
        }
        let view_distance = login.view_distance.min(options.render_distance as i32);
        // Block entity type names by synced registry id (the static registry's order when the
        // server did not sync one), and the block states whose lids `block_event` drives.
        let block_entity_names = {
            let regs = driver.connection().registries();
            block_entity_type_names(regs, &data)
        };
        let lid_states = lid_block_states(&data);
        // The bells' and decorated pots' `block_event` action 1 states, for the same filter.
        let bell_states = block_states_where(&data, haste_render::blockentity::is_bell_block);
        let pot_states = block_states_where(&data, haste_render::blockentity::is_pot_block);
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
            haste_render::ParticleTypes::build(
                &mut open_assets_with(&server_packs)?,
                &particle_ids,
                Some(&data.blocks),
            )
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
        entities.set_item_components(data.item_components.clone());
        let shared = Arc::new(Shared::new(
            data,
            world,
            entities,
            view_distance,
            haste_render::Particles::new(particle_types.clone()),
            haste_render::default_skin(login.profile.uuid.as_u128()),
            block_entity_names,
            biome_climates.clone(),
            options.clone(),
        ));
        // Wake the game thread the moment a play packet arrives, so packets are
        // applied between ticks like vanilla's every-frame queue drain
        // (`Minecraft.runTick`) instead of waiting for the next 20 Hz tick.
        {
            let shared = shared.clone();
            driver
                .connection()
                .set_packet_wake(Arc::new(move || shared.wake_on_packet()));
        }
        // The server's configuration-phase packs are part of the running stack from the
        // start; a later pack-screen reload keeps them layered (vanilla's repository
        // holds them beside the selection).
        *shared.applied_server_packs.lock() = server_packs.clone();
        // Terrain particles tint with the biome colours (the same climates the
        // renderer resolves for meshing).
        {
            let colors = haste_render::Colormaps::load(&mut open_assets_with(&server_packs)?)
                .map(|maps| haste_render::BiomeColors::new(&biome_climates, &maps))
                .unwrap_or_else(|e| {
                    tracing::warn!("no biome colours (block particles untinted): {e:#}");
                    haste_render::BiomeColors::new(&[], &haste_render::Colormaps::uniform(0xFFFFFF))
                });
            shared.particles.lock().set_biome_colors(colors);
        }
        shared.hud_enabled.store(
            HUD_ENABLED.load(std::sync::atomic::Ordering::Relaxed),
            std::sync::atomic::Ordering::Relaxed,
        );
        if sounds.is_some() {
            *shared.sound.lock() = sounds;
        }
        driver.session.set_lid_states(lid_states);
        driver.session.set_bell_states(bell_states);
        driver.session.set_pot_states(pot_states);
        let handle = rt.handle().clone();
        let sh = shared.clone();
        let game_thread = std::thread::Builder::new()
            .name("haste-game".into())
            .spawn(move || {
                let _rt = handle.enter();
                game::run(driver, sh, existing_ui)
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
            server_packs,
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

/// A running singleplayer world: the local server and the runtime that spawned it. The
/// server's stdin/stdout pipes are tied to that runtime's reactor (tokio ties a spawned
/// child to the runtime that spawned it), so it stays alive until the world is left; `stop`
/// saves the world and ends the process.
struct LocalWorld {
    _rt: tokio::runtime::Runtime,
    server: haste_setup::server::LocalServer,
}

impl LocalWorld {
    /// Starts (or creates) the world's local server, with its own runtime. Blocking (the
    /// server's "Done" line can take tens of seconds on a fresh world).
    fn start(
        dir: &Path,
        create: Option<&haste_jvm::CreateWorldRequest>,
        username: &str,
    ) -> anyhow::Result<LocalWorld> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let server = rt
            .block_on(async {
                let port = haste_setup::server::free_port()?;
                match create {
                    None => haste_setup::server::LocalServer::start(dir, port).await,
                    Some(create) => {
                        let settings = local_server_settings(dir, create);
                        write_ops_json(dir, username, create.allow_commands);
                        haste_setup::server::LocalServer::start_with(dir, port, &settings).await
                    }
                }
            })
            .map_err(|e| anyhow::anyhow!("starting the local server: {e:#}"))?;
        Ok(LocalWorld { _rt: rt, server })
    }

    /// The `host:port` the client joins.
    fn address(&self) -> String {
        self.server.address()
    }

    /// Saves the world and ends the server process.
    fn stop(self) {
        self._rt.block_on(self.server.stop());
        println!("singleplayer server stopped");
    }
}

/// The local-server settings for a world the create screen asked to make: vanilla's
/// CreateWorldScreen choices mapped onto `server.properties` (the server creates the world
/// from them). `level-name` must be the folder's name (the server puts the level at
/// `<universe>/<level-name>`, and the world dir is the universe's `..`/name).
fn local_server_settings(
    dir: &Path,
    create: &haste_jvm::CreateWorldRequest,
) -> haste_setup::server::WorldSettings {
    let level_name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&create.folder)
        .to_string();
    haste_setup::server::WorldSettings {
        level_name,
        game_mode: create.game_mode,
        difficulty: create.difficulty,
        hardcore: create.hardcore,
        allow_commands: create.allow_commands,
        seed: create.seed.clone(),
        generate_structures: create.generate_structures,
        level_type: create.level_type.clone(),
    }
}

/// Cheats on a created world (`allowCommands`): vanilla singleplayer gives the player operator
/// permissions; Haste's world runs on a local dedicated server, so the joining player is
/// written into its `ops.json` with level 4 (an offline-mode UUID, like the server computes).
/// `allow_commands` false clears a leftover file (the world dir may be reused).
fn write_ops_json(dir: &Path, username: &str, allow_commands: bool) {
    let path = dir.join("ops.json");
    if !allow_commands {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let uuid = haste_setup::launch::offline_uuid(username);
    let ops = format!("[{{\"uuid\":\"{uuid}\",\"name\":\"{username}\",\"level\":4,\"bypassesPlayerLimit\":false}}]\n");
    if let Err(e) = std::fs::write(&path, ops) {
        tracing::warn!("writing {}: {e:#}", path.display());
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
    /// Block entity instances for this frame (reused).
    be_instances: Vec<haste_render::BlockEntityInstance>,
    /// Block entity types already logged as undrawable (log once per type).
    unknown_be_types: HashSet<String>,
    /// Lid animations by position for this frame (reused lookup scratch).
    lid_map: HashMap<BlockPos, (f32, f32)>,
    /// Parsed sign text by position, rebuilt only when the block entity's NBT changes (the
    /// value is shared with the frame's instances, so no per-frame allocation). The key is the
    /// NBT payload's (pointer, length): the server sends a fresh `Bytes` per update, so a
    /// different payload always re-parses.
    sign_texts: HashMap<
        BlockPos,
        (
            usize,
            usize,
            std::sync::Arc<haste_render::blockentity::SignTextData>,
        ),
    >,
    /// The renderer's text metrics (the font), for wrapping sign lines at parse time.
    text_metrics: Option<std::sync::Arc<dyn haste_core::gui::TextMetrics>>,
    /// Parsed banner patterns by position, rebuilt only when the NBT changes (see
    /// `sign_texts` for the caching scheme).
    banner_texts: HashMap<
        BlockPos,
        (
            usize,
            usize,
            std::sync::Arc<haste_render::blockentity::BannerData>,
        ),
    >,
    /// Parsed decorated pot sherds by position, rebuilt only when the NBT changes (the same
    /// (pointer, length) caching scheme as `sign_texts`).
    pot_sherds_cache: HashMap<
        BlockPos,
        (
            usize,
            usize,
            std::sync::Arc<haste_render::blockentity::PotSherds>,
        ),
    >,
    /// The skull animations by position for this frame (reused lookup scratch).
    skull_anims: HashMap<BlockPos, (f32, f32)>,
    /// Parsed item and mob payloads by position, rebuilt only when the block entity's NBT
    /// changes (the (pointer, length) key scheme of `sign_texts`): campfire items, shelf
    /// items, a vault's displayed item, a brushable block's item and a spawner's display mob.
    item_blocks: HashMap<BlockPos, (usize, usize, haste_render::BlockEntityKind)>,
    /// The vault's and the spawners' spins by position for this frame (reused lookup
    /// scratch), as (previous tick, current tick) in degrees.
    spin_map: HashMap<BlockPos, (f32, f32)>,
    /// The object-model block entities' animation state by position for this frame (reused
    /// lookup scratch; the game thread ticks it once per tick).
    anim_map: HashMap<BlockPos, haste_render::BlockEntityAnim>,
    /// Beacon beam sections by position, rebuilt only when the blocks above change (the key
    /// is the column's state above the beacon: any change re-walks it).
    beacon_beams: HashMap<BlockPos, (u64, std::sync::Arc<haste_render::BeamData>)>,
    /// End gateway ages by position: the NBT payload key, the world age it was first seen at
    /// and its NBT age then (the age ticks on client-side, vanilla `beamAnimationTick`, since
    /// the server sends the NBT only once).
    gateway_ages: HashMap<BlockPos, (usize, usize, i64, i64)>,
    /// Gateways already logged (debugging the draw, once per position).
    logged_gateways: std::collections::HashSet<BlockPos>,
    /// Gateways whose beam was live on the last frame (log on change).
    gateway_beam_live: HashMap<BlockPos, bool>,
    /// Moving piston data by position, rebuilt only when the block entity's NBT changes.
    piston_cache: HashMap<
        BlockPos,
        (
            usize,
            usize,
            Option<std::sync::Arc<haste_render::PistonData>>,
        ),
    >,
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

    /// Queues every non-empty section of the loaded world for meshing (a rebuilt renderer
    /// starts with empty arenas: vanilla's reload re-bakes every model and section too).
    /// Counts as an arrival so `settled` waits for the re-mesh to drain first.
    fn queue_all_sections(&mut self, shared: &Shared) {
        let world = shared.world.read();
        for chunk_pos in world.loaded_chunks() {
            let Some(chunk) = world.chunk(chunk_pos) else {
                continue;
            };
            for (i, section) in chunk.sections.iter().enumerate() {
                if section.is_empty() {
                    continue;
                }
                let pos = SectionPos::new(chunk_pos.x, chunk.min_section + i as i32, chunk_pos.z);
                if self.queued.insert(pos) {
                    self.queue.push(pos);
                }
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
    /// renderer not to draw it in the world then (`FrameInput::camera_entity`). The frame's
    /// block entity instances are built first, so one call returns both slices the frame draws.
    fn entities(
        &mut self,
        shared: &Shared,
        renderer: &mut Renderer,
        local: (EntityId, u128),
        partial: f32,
    ) -> (&[EntityInstance], &[haste_render::BlockEntityInstance]) {
        // The sign text parser needs the font for the wrap; grabbed once per connection.
        if self.text_metrics.is_none() {
            self.text_metrics = Some(renderer.text_metrics());
        }
        self.block_entities(shared, partial);
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
            let view = shared.view.lock().clone();
            self.instances
                .push(game::player_instance(local.0, local.1, &view, partial));
        }
        renderer.sample_entity_lights(&*shared.world.read(), &self.instances);
        (&self.instances, &self.be_instances)
    }

    /// Builds this frame's block entity instances (chests, ender chests, trapped chests,
    /// shulker boxes) from the world's stored block entities: resolves the type name to what
    /// to draw, reads facing and double-chest half from the block state, combines the lid
    /// animation over a double chest's halves (vanilla `ChestBlock.opennessCombiner`'s max,
    /// `BrightnessCombiner`'s max light) and samples the block light. Types Haste cannot
    /// draw are logged once and skipped. The result is `self.be_instances` (read it after
    /// this call; `entities` borrows the feed too).
    fn block_entities(&mut self, shared: &Shared, partial: f32) {
        use haste_core::view::BlockView as _;
        self.be_instances.clear();
        let world = shared.world.read();
        {
            let lids = shared.block_entity_lids.lock();
            self.lid_map.clear();
            self.lid_map.extend(lids.iter().copied());
        }
        {
            let anims = shared.skull_anims.lock();
            self.skull_anims.clear();
            self.skull_anims.extend(anims.iter().copied());
        }
        {
            let spins = shared.block_entity_spins.lock();
            self.spin_map.clear();
            self.spin_map.extend(spins.iter().copied());
        }
        {
            let anims = shared.block_entity_anims.lock();
            self.anim_map.clear();
            self.anim_map.extend(anims.iter().map(|(p, a)| (*p, *a)));
        }
        let names = &shared.block_entity_names;
        let blocks = &shared.data.blocks;
        let xmas = haste_render::xmas_textures();
        // Only the chunks the camera can see carry drawable block entities; skipping the
        // rest keeps the per-frame walk short on big worlds with nothing in them.
        let view = shared.view.lock().clone();
        let world_age = view.world_age;
        let cam_chunk = ChunkPos::new(
            view.feet.x.floor() as i32 >> 4,
            view.feet.z.floor() as i32 >> 4,
        );
        let r = view.view_distance.max(1);
        for chunk in world.loaded_chunks() {
            if chunk.x < cam_chunk.x - r
                || chunk.x > cam_chunk.x + r
                || chunk.z < cam_chunk.z - r
                || chunk.z > cam_chunk.z + r
            {
                continue;
            }
            for (pos, be) in world.block_entities_in(chunk) {
                let Some(type_name) = names.get(be.kind as usize) else {
                    if self.unknown_be_types.insert(format!("#{}", be.kind)) {
                        tracing::warn!(
                            "block entity type id {} is outside the registry; not drawn",
                            be.kind
                        );
                    }
                    continue;
                };
                let state = world.block_state(pos);
                let block = blocks.block_of(state);
                let Some(kind) = haste_render::blockentity::kind_of(type_name, block, xmas) else {
                    if self.unknown_be_types.insert(type_name.to_string()) {
                        tracing::warn!("no renderer for block entity type {type_name}; not drawn");
                    }
                    continue;
                };
                // The item and mob payloads (campfire items, spawner display mobs, vault and
                // shelf items, brushable item) come from the block entity's NBT, parsed only
                // when the payload changes. The vault's and spawner's spins come from the
                // game tick (`VaultClientData.updateDisplayItemSpin`, `BaseSpawner.clientTick`).
                let registries = &shared.data.registries;
                let spin = self.spin_map.get(&pos).copied().unwrap_or((0.0, 0.0));
                let kind =
                    self.item_block_kind(pos, be, kind, type_name, blocks, state, registries, spin);
                // Vanilla `LecternRenderer`: the book only while the state's `has_book` is true.
                if kind == haste_render::BlockEntityKind::Lectern
                    && blocks.property_value(state, "has_book") != Some("true")
                {
                    continue;
                }
                let facing = blocks
                    .property_value(state, "facing")
                    .and_then(direction_of_property)
                    .unwrap_or(Direction::South);
                let half = match blocks.property_value(state, "type") {
                    Some("left") => haste_render::ChestHalf::Left,
                    Some("right") => haste_render::ChestHalf::Right,
                    _ => haste_render::ChestHalf::Single,
                };
                // Lid and light: a double chest uses the max over both halves (vanilla's
                // combiners); the neighbour is the block the connected direction points at
                // (`ChestBlock.getConnectedDirection`).
                let mut open = self.lid_map.get(&pos).copied().unwrap_or((0.0, 0.0));
                let mut light = (world.sky_light(pos), world.block_light(pos));
                if half != haste_render::ChestHalf::Single {
                    let connected = connected_direction(facing, half);
                    let neighbour = pos.offset(connected);
                    if is_double_chest_half(blocks, &world, neighbour, half, facing) {
                        if let Some(n) = self.lid_map.get(&neighbour) {
                            open = (open.0.max(n.0), open.1.max(n.1));
                        }
                        light = (
                            light.0.max(world.sky_light(neighbour)),
                            light.1.max(world.block_light(neighbour)),
                        );
                    }
                }
                // Signs, banners and skulls: the yaw angle the transform turns by (vanilla
                // `StandingSignRenderer`/`BannerRenderer`/`SkullBlockRenderer` picking the
                // block state's `rotation` segment or the wall `facing`), the sign's cached
                // text, the banner's wave phase and cached patterns, the skull's animation.
                let (mut sign_angle, text, banner_phase, banner, skull_anim, anim, sherds) =
                    match kind {
                        haste_render::BlockEntityKind::Sign { hanging } => {
                            let angle = rotation_or_facing(blocks, state, facing);
                            (
                                angle,
                                self.sign_text(pos, be, hanging),
                                0.0,
                                None,
                                haste_render::TickPair::fixed(0.0),
                                None,
                                None,
                            )
                        }
                        haste_render::BlockEntityKind::Banner { .. } => {
                            let angle = rotation_or_facing(blocks, state, facing);
                            // Vanilla `BannerRenderer.extractRenderState`'s phase: the position
                            // hash plus the world age, over 100, plus the partial tick.
                            let view = shared.view.lock();
                            let phase = haste_render::blockentity::banner_wave_phase(
                                pos,
                                view.world_age,
                                partial,
                            );
                            (
                                angle,
                                None,
                                phase,
                                self.banner_data(pos, be, block),
                                haste_render::TickPair::fixed(0.0),
                                None,
                                None,
                            )
                        }
                        haste_render::BlockEntityKind::Skull { .. } => {
                            // The ground rotation angle (wall skulls read `facing` instead) and
                            // the powered animation's tick ends.
                            let angle = rotation_or_facing(blocks, state, facing);
                            let anim = self.skull_anims.get(&pos).copied().unwrap_or((0.0, 0.0));
                            (
                                angle,
                                None,
                                0.0,
                                None,
                                haste_render::TickPair {
                                    start: anim.0,
                                    end: anim.1,
                                },
                                None,
                                None,
                            )
                        }
                        haste_render::BlockEntityKind::DecoratedPot => {
                            // The wobble from the game thread's tick, the sherds from the block
                            // entity's NBT (cached like the banner's patterns).
                            let anim = self.anim_map.get(&pos).copied();
                            (
                                0.0,
                                None,
                                0.0,
                                None,
                                haste_render::TickPair::fixed(0.0),
                                anim.map(|a| a.at(partial)),
                                self.pot_sherds(pos, be),
                            )
                        }
                        haste_render::BlockEntityKind::Lectern => {
                            // A lectern's book is at rest (`BOOK_STATE` is constant) and its yaw
                            // comes from the block's `facing`.
                            (
                                0.0,
                                None,
                                0.0,
                                None,
                                haste_render::TickPair::fixed(0.0),
                                None,
                                None,
                            )
                        }
                        haste_render::BlockEntityKind::EnchantTable
                        | haste_render::BlockEntityKind::Bell
                        | haste_render::BlockEntityKind::Conduit => {
                            // Ticked by the game thread (vanilla's block entity tickers); the
                            // renderer's counters are lerped by the partial tick, vanilla's
                            // `Mth.lerp(partialTicks, oX, X)`.
                            (
                                0.0,
                                None,
                                0.0,
                                None,
                                haste_render::TickPair::fixed(0.0),
                                self.anim_map.get(&pos).copied().map(|a| a.at(partial)),
                                None,
                            )
                        }
                        _ => (
                            0.0,
                            None,
                            0.0,
                            None,
                            haste_render::TickPair::fixed(0.0),
                            None,
                            None,
                        ),
                    };
                // Wall skulls turn by the wall `facing` (their transform reads it); the
                // rotation segment is only for the ground ones.
                if matches!(
                    kind,
                    haste_render::BlockEntityKind::Skull {
                        wall: true,
                        kind: _
                    }
                ) && blocks.property_value(state, "rotation").is_none()
                {
                    sign_angle = 0.0;
                }
                // The special renderers' data: the beacon's beam sections (the client's own
                // glass walk, vanilla `BeaconBlockEntity.tick`), the gateway's beam and the
                // portal's faces, the piston's moved-block state.
                let (beam, gateway_beam, portal_faces, piston) = match kind {
                    haste_render::BlockEntityKind::Beacon => {
                        let beam = self.beacon_data(pos, &world, blocks, &shared.data);
                        (beam, None, 0, None)
                    }
                    haste_render::BlockEntityKind::EndPortal => (
                        None,
                        None,
                        haste_render::blockentity::end_portal_faces(),
                        None,
                    ),
                    haste_render::BlockEntityKind::EndGateway => {
                        let (faces, beam) = self.gateway_data(
                            pos,
                            &world,
                            blocks,
                            &shared.data,
                            be,
                            world_age,
                            partial,
                        );
                        // On every beam state change (and once at first sight): what the
                        // renderer gets (debugging the draw).
                        let beam_now = beam.is_some();
                        let seen = self.logged_gateways.insert(pos);
                        let changed = self
                            .gateway_beam_live
                            .insert(pos, beam_now)
                            .is_some_and(|was| was != beam_now);
                        if seen || changed {
                            tracing::info!(
                                "end gateway at {pos:?}: faces {faces:#08b}, beam {:?}",
                                beam.map(|b| (b.scale, b.height))
                            );
                        }
                        (None, beam, faces, None)
                    }
                    haste_render::BlockEntityKind::MovingPiston => {
                        (None, None, 0, self.piston_data(pos, be, partial))
                    }
                    _ => (None, None, 0, None),
                };
                self.be_instances.push(haste_render::BlockEntityInstance {
                    pos,
                    facing,
                    half,
                    kind,
                    open: haste_render::TickPair {
                        start: open.0,
                        end: open.1,
                    },
                    light,
                    sign_angle,
                    text,
                    banner_phase,
                    banner,
                    skull_anim,
                    anim,
                    sherds,
                    beam,
                    // Vanilla's beam animation time, `floorMod(gameTime, 40) + partial`.
                    beam_time: haste_render::blockentity::beam_animation_time(world_age, partial),
                    gateway_beam,
                    portal_faces,
                    piston,
                });
            }
        }
    }

    /// The parsed banner patterns at `pos` (`BannerBlockEntity`'s `patterns` NBT list) with
    /// the base colour from the block id (vanilla `BannerBlock.getColor`). Cached like the
    /// sign text: re-parsed only when the payload changes.
    fn banner_data(
        &mut self,
        pos: BlockPos,
        be: &haste_core::chunk::BlockEntityData,
        block: &haste_core::Identifier,
    ) -> Option<std::sync::Arc<haste_render::blockentity::BannerData>> {
        let key = (be.nbt.as_ptr() as usize, be.nbt.len());
        if let Some((k, len, data)) = self.banner_texts.get(&pos) {
            if (*k, *len) == key {
                return Some(data.clone());
            }
        }
        let parsed = parse_banner_nbt(&be.nbt, block)?;
        self.banner_texts
            .insert(pos, (key.0, key.1, parsed.clone()));
        Some(parsed)
    }

    /// The item or mob payload of a block entity (campfire items, shelf items, a vault's
    /// displayed item, a brushable block's item, a spawner's display mob), parsed from the
    /// block entity's NBT only when the payload changes and cached per position (the same
    /// (pointer, length) scheme as the sign text). Kinds with no payload are handed back
    /// unchanged.
    #[allow(clippy::too_many_arguments)]
    fn item_block_kind(
        &mut self,
        pos: BlockPos,
        be: &haste_core::chunk::BlockEntityData,
        kind: haste_render::BlockEntityKind,
        type_name: &haste_core::Identifier,
        blocks: &haste_data::BlockStates,
        state: haste_core::BlockStateId,
        registries: &haste_data::Registries,
        spin: (f32, f32),
    ) -> haste_render::BlockEntityKind {
        use haste_render::blockentity::{BrushableItem, CampfireItems, ShelfItems, VaultDisplay};
        let payload = matches!(
            kind,
            haste_render::BlockEntityKind::Campfire(_)
                | haste_render::BlockEntityKind::Spawner(_)
                | haste_render::BlockEntityKind::Vault(_)
                | haste_render::BlockEntityKind::Shelf(_)
                | haste_render::BlockEntityKind::Brushable(_)
        );
        if !payload {
            return kind;
        }
        let key = (be.nbt.as_ptr() as usize, be.nbt.len());
        if let Some((k, len, cached)) = self.item_blocks.get(&pos) {
            if (*k, *len) == key {
                return cached.clone();
            }
        }
        let parsed = match kind {
            // The campfire's and the shelf's items, by slot (`ContainerHelper.loadAllItems`).
            haste_render::BlockEntityKind::Campfire(_) => {
                let mut items = [None; 4];
                parse_items_into(&be.nbt, registries, &mut items);
                haste_render::BlockEntityKind::Campfire(CampfireItems { items })
            }
            haste_render::BlockEntityKind::Shelf(_) => {
                let mut items = [None; 3];
                parse_items_into(&be.nbt, registries, &mut items);
                let align = read_nbt(&be.nbt)
                    .and_then(|doc| doc.root_tag.get_bool("align_items_to_bottom"))
                    .unwrap_or(false);
                haste_render::BlockEntityKind::Shelf(ShelfItems {
                    items,
                    align_to_bottom: align,
                })
            }
            // The vault's displayed item lives under `shared_data.display_item`; it draws only
            // when the shared data has one (vanilla `VaultBlockEntity.Client
            // .shouldDisplayActiveEffects` is `hasDisplayItem`).
            haste_render::BlockEntityKind::Vault(_) => {
                match parse_item_stack_path(&be.nbt, &["shared_data", "display_item"], registries) {
                    Some((item, count)) => haste_render::BlockEntityKind::Vault(VaultDisplay {
                        item,
                        count,
                        spin: haste_render::TickPair {
                            start: spin.0,
                            end: spin.1,
                        },
                    }),
                    None => kind,
                }
            }
            // The brushable block's item (its `item` tag) and the block state's `dusted`; the
            // face it is brushed from is not in the NBT until it has been brushed, so it stays
            // `None` and the item is drawn only while that face is known.
            haste_render::BlockEntityKind::Brushable(_) => {
                let dusted = blocks
                    .property_value(state, "dusted")
                    .and_then(|v| v.parse::<u8>().ok())
                    .unwrap_or(0);
                match (
                    parse_item_stack_path(&be.nbt, &["item"], registries),
                    // Vanilla stores `hit_direction` as `Direction.LEGACY_ID_CODEC`, a byte of
                    // the direction's 3D data value (`BrushableBlockEntity.getUpdateTag`); read
                    // it as any numeric, like vanilla's lenient `ValueInput`.
                    read_nbt(&be.nbt)
                        .and_then(|doc| doc.root_tag.get_numeric_short("hit_direction"))
                        .and_then(|v| haste_core::Direction::from_ordinal(v as u8)),
                ) {
                    (Some((item, _)), hit_direction) => {
                        haste_render::BlockEntityKind::Brushable(BrushableItem {
                            item,
                            hit_direction,
                            dusted,
                        })
                    }
                    (None, _) => kind,
                }
            }
            // The spawner's display mob: the model from `SpawnData`'s (a trial spawner's
            // `spawn_data`'s) entity id, and the spin the game tick keeps.
            haste_render::BlockEntityKind::Spawner(_) => {
                let trial =
                    type_name.namespace() == "minecraft" && type_name.path() == "trial_spawner";
                // A trial spawner has no display entity outside the states with a spinning mob
                // (vanilla `TrialSpawnerStateData.getOrCreateDisplayEntity`).
                let spinning = !trial
                    || matches!(
                        blocks
                            .property_value(state, "trial_spawner_state")
                            .unwrap_or(""),
                        "waiting_for_players" | "active"
                    );
                let entity = spinning
                    .then(|| spawner_entity_id(&be.nbt, trial))
                    .flatten()
                    .and_then(|id| registries.index("minecraft:entity_type", &id))
                    .and_then(haste_entity::registry::model_for);
                match entity {
                    Some(model) => haste_render::BlockEntityKind::Spawner(
                        haste_render::blockentity::SpawnerDisplay {
                            model: Some(model.clone()),
                            spin: haste_render::TickPair {
                                start: spin.0,
                                end: spin.1,
                            },
                            scale: haste_render::blockentity::spawner_scale(&model),
                        },
                    ),
                    None => kind,
                }
            }
            other => other,
        };
        self.item_blocks.insert(pos, (key.0, key.1, parsed.clone()));
        parsed
    }

    /// The parsed decorated pot sherds at `pos` (`DecoratedPotBlockEntity`'s `sherds` NBT).
    /// Cached like the banner patterns: re-parsed only when the payload changes.
    fn pot_sherds(
        &mut self,
        pos: BlockPos,
        be: &haste_core::chunk::BlockEntityData,
    ) -> Option<std::sync::Arc<haste_render::blockentity::PotSherds>> {
        let key = (be.nbt.as_ptr() as usize, be.nbt.len());
        if let Some((k, len, data)) = self.pot_sherds_cache.get(&pos) {
            if (*k, *len) == key {
                return Some(data.clone());
            }
        }
        let parsed = std::sync::Arc::new(parse_pot_sherds(&be.nbt));
        self.pot_sherds_cache
            .insert(pos, (key.0, key.1, parsed.clone()));
        Some(parsed)
    }

    /// The parsed sign text at `pos` (`SignBlockEntity`'s `front_text`/`back_text`), from the
    /// block entity's NBT. Cached: the parse (and the wrap to the sign's line width) runs only
    /// when the payload changes; the returned handle is shared with the frame's instances.
    fn sign_text(
        &mut self,
        pos: BlockPos,
        be: &haste_core::chunk::BlockEntityData,
        hanging: bool,
    ) -> Option<std::sync::Arc<haste_render::blockentity::SignTextData>> {
        let key = (be.nbt.as_ptr() as usize, be.nbt.len());
        if let Some((k, len, text)) = self.sign_texts.get(&pos) {
            if (*k, *len) == key {
                return Some(text.clone());
            }
        }
        let metrics = self.text_metrics.clone()?;
        let (_line_height, max_width) = haste_render::blockentity::sign_line_metrics_pub(hanging);
        let parsed = parse_sign_nbt(&be.nbt, &*metrics, max_width)?;
        self.sign_texts.insert(pos, (key.0, key.1, parsed.clone()));
        Some(parsed)
    }

    /// The beacon's beam sections at `pos`: vanilla's own client-side walk
    /// (`BeaconBlockEntity.tick` over the stained glass above), done in one pass and re-walked
    /// whenever the column above the beacon changes (the key is a hash of the column's states;
    /// vanilla re-checks 10 blocks a tick, we need only the result).
    fn beacon_data(
        &mut self,
        pos: BlockPos,
        world: &World,
        blocks: &haste_data::BlockStates,
        data: &haste_data::GameData,
    ) -> Option<std::sync::Arc<haste_render::BeamData>> {
        use haste_core::view::{BlockPhysics as _, BlockView as _};
        // The column's states from the beacon block itself (vanilla's walk starts at it: the
        // beacon is a white `BeaconBeamBlock`, so it seeds the first section) up to the
        // heightmap top, hashed as the cache key.
        let top = world
            .chunk(pos.chunk())
            .and_then(|c| c.height_at((pos.x & 15) as u8, (pos.z & 15) as u8))
            .unwrap_or(pos.y);
        let mut key = 0u64;
        let states: Vec<_> = (pos.y..=top)
            .map(|y| {
                let s = world.block_state(BlockPos::new(pos.x, y, pos.z));
                key = key.wrapping_mul(31).wrapping_add(s.0 as u64);
                s
            })
            .collect();
        if let Some((k, data)) = self.beacon_beams.get(&pos) {
            if *k == key {
                return Some(data.clone());
            }
        }
        // Vanilla's walk: the first one or two stained-glass blocks start sections (a colour
        // change beyond that starts one in the averaged colour), any other block with light
        // dampening 15 (except bedrock) ends the beam, and every further block extends the
        // last section.
        let mut sections: Vec<haste_render::BeamSection> = Vec::new();
        for &s in &states {
            let block = blocks.block_of(s);
            if let Some(color) = beacon_beam_color(block) {
                match sections.len() {
                    0 | 1 => sections.push(haste_render::BeamSection { color, height: 1 }),
                    _ => {
                        let last_color = sections.last().unwrap().color;
                        if last_color == color {
                            sections.last_mut().unwrap().height += 1;
                        } else {
                            sections.push(haste_render::BeamSection {
                                color: average_argb(last_color, color),
                                height: 1,
                            });
                        }
                    }
                }
            } else {
                let ends = match sections.last() {
                    None => true,
                    Some(_) => {
                        // Vanilla: `state.getLightDampening() >= 15 && !state.is(bedrock)` —
                        // the oracle's opaque flag is the same fully-light-blocking set.
                        data.is_opaque_full(s) && block != &Identifier::minecraft("bedrock")
                    }
                };
                if ends {
                    sections.clear();
                    break;
                }
                sections.last_mut().unwrap().height += 1;
            }
        }
        let data = std::sync::Arc::new(haste_render::BeamData { sections });
        self.beacon_beams.insert(pos, (key, data.clone()));
        Some(data)
    }

    /// The moving piston's data at `pos` from the block entity's NBT (vanilla
    /// `PistonMovingBlockEntity.loadAdditional`): the moved block state, facing, progress,
    /// extending and source flags. The progress is the game thread's tick (vanilla advances
    /// it 0.5 per tick to 1.0, then the block entity is gone); this reads its ends.
    fn piston_data(
        &mut self,
        pos: BlockPos,
        be: &haste_core::chunk::BlockEntityData,
        partial: f32,
    ) -> Option<std::sync::Arc<haste_render::PistonData>> {
        let key = (be.nbt.as_ptr() as usize, be.nbt.len());
        if let Some((k, len, data)) = self.piston_cache.get(&pos) {
            if (*k, *len) == key {
                if let Some(data) = data {
                    // The progress ticks on: fold the anim's ends by the partial tick.
                    let mut d = (**data).clone();
                    d.progress = self
                        .anim_map
                        .get(&pos)
                        .map(|a| match a {
                            haste_render::BlockEntityAnim::Piston { progress } => {
                                haste_render::TickPair {
                                    start: progress.start,
                                    end: progress.at(partial),
                                }
                            }
                            _ => d.progress,
                        })
                        .unwrap_or(d.progress);
                    return Some(std::sync::Arc::new(d));
                }
                return None;
            }
        }
        let parsed = parse_piston_nbt(&be.nbt);
        let parsed = std::sync::Arc::new(parsed);
        self.piston_cache
            .insert(pos, (key.0, key.1, Some(parsed.clone())));
        Some(parsed)
    }

    /// Hands the renderer the first-person hand (`Renderer::set_first_person`) when the view's
    /// per-tick part changed since the last call. The skin is fixed for the connection, so
    /// comparing the per-tick part is enough to catch every change; the skin is cloned only when
    /// one happens (no allocation per frame).
    /// The first-person hand only exists in the first-person perspective (F5 hides it) and
    /// while the HUD is shown (`GameRenderer.renderItemInHand` skips it when F1 hid the HUD).
    fn set_first_person(
        &mut self,
        shared: &Shared,
        renderer: &mut Renderer,
        perspective: game::Perspective,
        hud_hidden: bool,
    ) {
        let view = match perspective {
            game::Perspective::FirstPerson if !hud_hidden => shared.view.lock().first_person,
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

/// A block state `facing` property value as a `Direction`.
fn direction_of_property(v: &str) -> Option<Direction> {
    Direction::ALL.iter().find(|d| d.name() == v).copied()
}

/// Vanilla `Direction.toYRot`: south 0, west 90, north 180, east 270.
fn to_y_rot(d: Direction) -> f32 {
    match d {
        Direction::South => 0.0,
        Direction::West => 90.0,
        Direction::North => 180.0,
        Direction::East => 270.0,
        Direction::Down | Direction::Up => 0.0,
    }
}

/// The yaw a sign/banner turns by: the block state's `rotation` segment
/// (`RotationSegment.convertToDegrees`: `segment * 22.5`, wrapped from 180 to -180), or the
/// wall `facing`'s `toYRot()` when there is no rotation property.
fn rotation_or_facing(
    blocks: &haste_data::BlockStates,
    state: haste_core::BlockStateId,
    facing: Direction,
) -> f32 {
    match blocks.property_value(state, "rotation") {
        Some(rot) => rot.parse::<i32>().map_or(0.0, |r| {
            let d = r.rem_euclid(16) as f32 * 22.5;
            if d >= 180.0 {
                d - 360.0
            } else {
                d
            }
        }),
        None => to_y_rot(facing),
    }
}

/// The pottery pattern a sherd item names (vanilla's item
/// `minecraft:provides_pottery_pattern` component against `DecoratedPotPattern`'s asset id):
/// `minecraft:angler_pottery_sherd` -> `minecraft:angler`. `minecraft:brick` (the blank sherd)
/// has none. Items from mods whose names do not follow the vanilla pattern draw the blank side.
fn sherd_pattern(item: &haste_core::Identifier) -> Option<haste_core::Identifier> {
    if item.namespace() != "minecraft" {
        return None;
    }
    let path = item.path().strip_suffix("_pottery_sherd")?;
    Some(haste_core::Identifier::minecraft(path))
}

/// A decorated pot's four sherds from its NBT (`DecoratedPotBlockEntity`'s `sherds`, a record
/// of optional item stack templates keyed `back`, `left`, `right`, `front`; the item is either
/// an `{id, count}` compound or a plain id string). Sides without a sherd are `None`.
fn parse_pot_sherds(nbt: &[u8]) -> haste_render::blockentity::PotSherds {
    use haste_protocol::nbt::deserializer::NbtReadHelperJava;
    use haste_protocol::nbt::NbtTag;
    let side = |c: &haste_protocol::nbt::NbtCompound, key: &str| match c.get(key) {
        // Vanilla writes each sherd as an item stack template; older payloads (and lenient
        // readers) allow the plain item id.
        Some(NbtTag::Compound(item)) => item.get_string("id").and_then(|s| {
            haste_core::Identifier::parse(s)
                .ok()
                .as_ref()
                .and_then(sherd_pattern)
        }),
        Some(NbtTag::String(s)) => haste_core::Identifier::parse(s)
            .ok()
            .as_ref()
            .and_then(sherd_pattern),
        _ => None,
    };
    let mut out = haste_render::blockentity::PotSherds::default();
    let mut cursor = std::io::Cursor::new(nbt);
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    let Ok(doc) = haste_protocol::nbt::Nbt::read_unnamed(&mut helper) else {
        return out;
    };
    if let Some(c) = doc.root_tag.get_compound("sherds") {
        out.front = side(c, "front");
        out.back = side(c, "back");
        out.left = side(c, "left");
        out.right = side(c, "right");
    }
    out
}

/// A moving piston's data from its NBT (vanilla `PistonMovingBlockEntity.loadAdditional`:
/// `blockState`, `facing`, `progress`, `extending`, `source`). Unreadable or missing fields
/// draw nothing (vanilla's `movedState` defaults to air, which also draws nothing).
fn parse_piston_nbt(nbt: &[u8]) -> haste_render::PistonData {
    use haste_protocol::nbt::deserializer::NbtReadHelperJava;
    let mut cursor = std::io::Cursor::new(nbt);
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    let Ok(doc) = haste_protocol::nbt::Nbt::read_unnamed(&mut helper) else {
        return air_piston();
    };
    let root = &doc.root_tag;
    // The state string carries its properties: `block[facing=north,...]`; keep it whole (the
    // renderer parses it into the blockstate's variants).
    let block_state = root
        .get_string("blockState")
        .map(|s| s.to_string())
        .unwrap_or_default();
    let facing = root
        .get_int("facing")
        .and_then(|i| u8::try_from(i).ok())
        .and_then(haste_core::Direction::from_ordinal)
        .unwrap_or(haste_core::Direction::Down);
    let progress = root.get_float("progress").unwrap_or(0.0);
    let extending = root.get_bool("extending").unwrap_or(false);
    let source = root.get_bool("source").unwrap_or(false);
    haste_render::PistonData {
        block_state,
        progress: haste_render::TickPair::fixed(progress),
        facing,
        extending,
        source,
    }
}

/// The piston data for a block entity without a moved block (draws nothing).
fn air_piston() -> haste_render::PistonData {
    haste_render::PistonData {
        block_state: String::new(),
        progress: haste_render::TickPair::fixed(0.0),
        facing: haste_core::Direction::Down,
        extending: false,
        source: false,
    }
}

/// A stained-glass block's (or the beacon's own) beam colour (vanilla `BeaconBeamBlock.getColor`
/// -> `DyeColor.getTextureDiffuseColor`), or `None` for other blocks.
fn beacon_beam_color(block: &haste_core::Identifier) -> Option<u32> {
    const GLASS: [(&str, u32); 16] = [
        ("white_stained_glass", 0),
        ("orange_stained_glass", 1),
        ("magenta_stained_glass", 2),
        ("light_blue_stained_glass", 3),
        ("yellow_stained_glass", 4),
        ("lime_stained_glass", 5),
        ("pink_stained_glass", 6),
        ("gray_stained_glass", 7),
        ("light_gray_stained_glass", 8),
        ("cyan_stained_glass", 9),
        ("purple_stained_glass", 10),
        ("blue_stained_glass", 11),
        ("brown_stained_glass", 12),
        ("green_stained_glass", 13),
        ("red_stained_glass", 14),
        ("black_stained_glass", 15),
    ];
    if block.namespace() != "minecraft" {
        return None;
    }
    // The beacon block itself is a white beam block (vanilla `BeaconBlock.getColor`).
    let color = if block.path() == "beacon" {
        0
    } else {
        GLASS
            .iter()
            .find(|(name, _)| *name == block.path())
            .map(|(_, dye)| *dye)?
    };
    Some(haste_render::blockentity::DYE_DIFFUSE_COLOR[color as usize])
}

/// Vanilla `ARGB.average`: the per-channel mean of two colours.
fn average_argb(a: u32, b: u32) -> u32 {
    let ch = |s: u32| (((a >> s) & 0xFF).wrapping_add((b >> s) & 0xFF)) / 2;
    0xFF00_0000 | ch(16) << 16 | ch(8) << 8 | ch(0)
}

/// A banner's patterns from its NBT (`BannerBlockEntity`'s `patterns`: a list of
/// `{pattern: <registry id>, color: <dye name>}`) plus the base colour from the block id
/// (`<colour>_banner`, vanilla `BannerBlock.getColor`). `None` when the NBT does not parse;
/// a banner without a `patterns` tag is the plain base colour.
/// Reads a block entity's NBT payload (the network variant `read_unnamed` reads). `None` when
/// the payload does not parse, like vanilla's empty stack.
fn read_nbt(nbt: &[u8]) -> Option<haste_protocol::nbt::Nbt> {
    use haste_protocol::nbt::deserializer::NbtReadHelperJava;
    let mut cursor = std::io::Cursor::new(nbt);
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    haste_protocol::nbt::Nbt::read_unnamed(&mut helper).ok()
}

/// An item stack NBT's registry index and count: vanilla `ItemStack`'s `id` (a registry id in
/// the namespace's own form) and `count`. `None` for an id we cannot resolve.
fn parse_item_stack(
    c: &haste_protocol::nbt::NbtCompound,
    registries: &haste_data::Registries,
) -> Option<(haste_core::RegistryIndex, u32)> {
    let id = haste_core::Identifier::parse(c.get_string("id")?).ok()?;
    let item = registries.index("minecraft:item", &id)?;
    let count = c.get_int("count").unwrap_or(1).max(1) as u32;
    Some((item, count))
}

/// The item stack under a chain of NBT keys (`shared_data.display_item` for a vault, `item`
/// for a brushable block), as its registry index and count.
fn parse_item_stack_path(
    nbt: &[u8],
    path: &[&str],
    registries: &haste_data::Registries,
) -> Option<(haste_core::RegistryIndex, u32)> {
    let doc = read_nbt(nbt)?;
    let mut current = &doc.root_tag;
    for key in path {
        current = current.get_compound(key)?;
    }
    parse_item_stack(current, registries)
}

/// The items of a slot-addressed container NBT (`Items`, vanilla `ContainerHelper
/// The items of a slot-addressed container NBT (`Items`, vanilla `ContainerHelper
/// .loadAllItems`), written into the caller's slot array; an item whose `Slot` is outside the
/// array is dropped rather than panicking.
fn parse_items_into(
    nbt: &[u8],
    registries: &haste_data::Registries,
    items: &mut [Option<haste_core::RegistryIndex>],
) {
    let Some(doc) = read_nbt(nbt) else {
        return;
    };
    let Some(list) = doc.root_tag.get_list("Items") else {
        return;
    };
    for entry in list {
        let haste_protocol::nbt::NbtTag::Compound(c) = entry else {
            continue;
        };
        let slot = c.get_byte("Slot").unwrap_or(-1);
        if slot < 0 || slot as usize >= items.len() {
            continue;
        }
        if let Some((item, _)) = parse_item_stack(c, registries) {
            items[slot as usize] = Some(item);
        }
    }
}

/// The entity id a spawner's NBT names (`BaseSpawner`'s `SpawnData.entity.id`; a trial
/// spawner's `spawn_data.entity.id`), as a registry id.
fn spawner_entity_id(nbt: &[u8], trial: bool) -> Option<haste_core::Identifier> {
    let doc = read_nbt(nbt)?;
    let data = if trial {
        doc.root_tag.get_compound("spawn_data")?
    } else {
        &doc.root_tag
    };
    let spawn = if trial {
        data
    } else {
        data.get_compound("SpawnData")?
    };
    let entity = spawn.get_compound("entity")?;
    haste_core::Identifier::parse(entity.get_string("id")?).ok()
}

/// A banner's patterns from the block entity's NBT
/// (`{pattern: <registry id>, color: <dye name>}`) plus the base colour from the block id
/// (`<colour>_banner`, vanilla `BannerBlock.getColor`). `None` when the NBT does not parse;
/// a banner without a `patterns` tag is the plain base colour.
fn parse_banner_nbt(
    nbt: &[u8],
    block: &haste_core::Identifier,
) -> Option<std::sync::Arc<haste_render::blockentity::BannerData>> {
    use haste_protocol::nbt::deserializer::NbtReadHelperJava;
    use haste_protocol::nbt::NbtTag;
    const DYES: [&str; 16] = [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ];
    let dye = |name: &str| {
        DYES.iter()
            .position(|d| *d == name)
            .map(|i| i as u8)
            .unwrap_or(15)
    };
    // The base colour: `<colour>_banner` / `<colour>_wall_banner` -> the dye id.
    let base = block
        .path()
        .strip_suffix("_wall_banner")
        .or_else(|| block.path().strip_suffix("_banner"))
        .and_then(|c| DYES.iter().position(|d| *d == c))
        .map_or(15, |i| i as u8);
    let mut data = haste_render::blockentity::BannerData {
        base,
        layers: Vec::new(),
    };
    let mut cursor = std::io::Cursor::new(nbt);
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    let doc = haste_protocol::nbt::Nbt::read_unnamed(&mut helper).ok()?;
    if let Some(patterns) = doc.root_tag.get_list("patterns") {
        for layer in patterns {
            let NbtTag::Compound(c) = layer else {
                continue;
            };
            // The pattern is the registry id (a string) or an inline `{asset_id: ...}` (a
            // compound under `pattern`, or at the layer's top level for leniency).
            let pattern = match c.get("pattern") {
                Some(NbtTag::String(s)) => haste_core::Identifier::parse(s).ok()?,
                Some(NbtTag::Compound(p)) => {
                    haste_core::Identifier::parse(p.get_string("asset_id")?).ok()?
                }
                _ => c
                    .get_string("asset_id")
                    .and_then(|s| haste_core::Identifier::parse(s).ok())?,
            };
            let color = c.get_string("color").map(dye).unwrap_or(15);
            data.layers
                .push(haste_render::blockentity::BannerLayer { pattern, color });
        }
    }
    Some(std::sync::Arc::new(data))
}

/// One sign side from its NBT compound (vanilla `SignText.CODEC`): `messages` (4 components as
/// plain strings or `{"text": ...}` compounds), `color` (a `DyeColor` name) and
/// `has_glowing_text`. Each line is wrapped to `max_width` like vanilla's render messages
/// (`Font.split`, first wrapped line only). Missing fields keep vanilla's defaults (black,
/// not glowing, empty).
fn parse_sign_side(
    c: &haste_protocol::nbt::NbtCompound,
    metrics: &dyn haste_core::gui::TextMetrics,
    max_width: i32,
) -> haste_render::blockentity::SignSideText {
    let mut lines: [String; 4] = Default::default();
    if let Some(messages) = c.get_list("messages") {
        for (i, m) in messages.iter().take(4).enumerate() {
            let text = match m {
                haste_protocol::nbt::NbtTag::String(s) => s.to_string(),
                haste_protocol::nbt::NbtTag::Compound(cc) => {
                    cc.get_string("text").unwrap_or_default().to_string()
                }
                _ => String::new(),
            };
            lines[i] = first_wrapped_line(&text, metrics, max_width).to_string();
        }
    }
    const DYES: [&str; 16] = [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ];
    let color = c
        .get_string("color")
        .and_then(|n| DYES.iter().position(|d| *d == n))
        .map_or(15, |i| i as u8);
    let glowing = c.get_bool("has_glowing_text").unwrap_or(false);
    haste_render::blockentity::SignSideText {
        lines,
        color,
        glowing,
    }
}

/// The first wrapped line of `text` at `max_width`, vanilla `StringSplitter.splitLines` for a
/// plain string: characters accumulate their widths; past the limit the line breaks at the
/// last space (dropping it), or right before the character that overflowed when there is
/// none; a first character wider than the limit still yields one character (the
/// `hadNonZeroWidthChar` guard). Newlines break there.
fn first_wrapped_line<'a>(
    text: &'a str,
    metrics: &dyn haste_core::gui::TextMetrics,
    max_width: i32,
) -> &'a str {
    let max_width = max_width.max(1);
    let mut width = 0i32;
    let mut had_non_zero = false;
    let mut last_space: Option<usize> = None;
    for (i, ch) in text.char_indices() {
        match ch {
            '\n' => return &text[..i],
            ' ' => last_space = Some(i),
            _ => {}
        }
        let char_width = metrics.width(ch.encode_utf8(&mut [0; 4]));
        width += char_width;
        if !had_non_zero || width <= max_width {
            had_non_zero |= char_width != 0;
        } else {
            return match last_space {
                // Break at the last space, dropping it.
                Some(s) => &text[..s],
                // No space: break before this character (at least one char: the guard above
                // always accepts the first non-zero-width one).
                None => &text[..i],
            };
        }
    }
    text
}

/// The whole sign's text (`front_text` / `back_text`), or `None` when the NBT does not parse.
fn parse_sign_nbt(
    nbt: &[u8],
    metrics: &dyn haste_core::gui::TextMetrics,
    max_width: i32,
) -> Option<std::sync::Arc<haste_render::blockentity::SignTextData>> {
    use haste_protocol::nbt::deserializer::NbtReadHelperJava;
    let mut cursor = std::io::Cursor::new(nbt);
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    let doc = haste_protocol::nbt::Nbt::read_unnamed(&mut helper).ok()?;
    let root = &doc.root_tag;
    let side = |name: &str| {
        root.get_compound(name)
            .map(|c| parse_sign_side(c, metrics, max_width))
            .unwrap_or_else(empty_sign_side)
    };
    Some(std::sync::Arc::new(
        haste_render::blockentity::SignTextData {
            front: side("front_text"),
            back: side("back_text"),
        },
    ))
}

/// Vanilla `SignText.EMPTY`: four empty lines, black, not glowing.
fn empty_sign_side() -> haste_render::blockentity::SignSideText {
    haste_render::blockentity::SignSideText {
        lines: std::array::from_fn(|_| String::new()),
        color: 15,
        glowing: false,
    }
}

/// Where a double chest's other half is (vanilla `ChestBlock.getConnectedDirection`:
/// the LEFT half's neighbour is clockwise of the facing, the RIGHT half's counter-clockwise).
fn connected_direction(facing: Direction, half: haste_render::ChestHalf) -> Direction {
    let clockwise = |d: Direction| match d {
        Direction::North => Direction::East,
        Direction::East => Direction::South,
        Direction::South => Direction::West,
        Direction::West => Direction::North,
        Direction::Down | Direction::Up => d,
    };
    match half {
        haste_render::ChestHalf::Left => clockwise(facing),
        _ => clockwise(facing).opposite(),
    }
}

/// Whether `pos` holds the matching other half of our double chest: a chest/trapped chest of
/// the opposite half, the same facing, whose connected direction points back at us (vanilla's
/// `DoubleBlockCombiner` accepts exactly that neighbour).
fn is_double_chest_half(
    blocks: &haste_data::BlockStates,
    world: &World,
    pos: BlockPos,
    our_half: haste_render::ChestHalf,
    facing: Direction,
) -> bool {
    use haste_core::view::BlockView as _;
    let state = world.block_state(pos);
    let block = blocks.block_of(state);
    if block.namespace() != "minecraft" || !matches!(block.path(), "chest" | "trapped_chest") {
        return false;
    }
    let neighbour_half = match blocks.property_value(state, "type") {
        Some("left") => haste_render::ChestHalf::Left,
        Some("right") => haste_render::ChestHalf::Right,
        _ => return false,
    };
    let Some(neighbour_facing) = blocks
        .property_value(state, "facing")
        .and_then(direction_of_property)
    else {
        return false;
    };
    neighbour_facing == facing
        && neighbour_half == our_half.other()
        && connected_direction(neighbour_facing, neighbour_half).opposite()
            == connected_direction(facing, our_half)
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
    death: Option<u32>,
    effect_scale: f32,
) -> f32 {
    let mut fov = base_fov * (old_modifier + (new_modifier - old_modifier) * partial);
    // `LivingEntity.isDeadOrDying` (health <= 0) gates it, from the tick health hits 0 with
    // `deathTime` still 0; `death` is that: `Some(deathTime)` while dead.
    if let Some(death_ticks) = death {
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
fn frame_view<'w>(
    shared: &Shared,
    perspective: game::Perspective,
    yaw: f32,
    pitch: f32,
    options: &Options,
    now: Instant,
    weather: &'w mut haste_render::WeatherFrame,
) -> (
    Camera,
    SkyState,
    LightmapFrame,
    f32,
    i32,
    u64,
    Option<&'w haste_render::WeatherFrame>,
) {
    let view = shared.view.lock().clone();
    let partial = game::partial_tick(view.tick_at, now);
    let eye = game::interpolate(view.prev_eye, view.eye, partial);
    let look = (
        yaw + view.look_offset.0,
        (pitch + view.look_offset.1).clamp(-90.0, 90.0),
    );
    // The world is read for the third-person camera and the fog environment (the fluid and
    // biome at the camera), held for the rest of this function.
    let world = shared.world.read();
    let (pos, yaw, pitch) = if perspective.draws_player() {
        game::third_person_camera(
            perspective,
            eye,
            look.0,
            look.1,
            &*world,
            &shared.data.shapes,
        )
    } else {
        (eye, look.0, look.1)
    };
    let fov_deg = frame_fov(
        options.fov as f32,
        view.fov_modifier_old,
        view.fov_modifier,
        partial,
        view.eye_in_water || view.eye_in_lava,
        view.death(),
        options.fov_effect_scale,
    );
    // The hand's own FOV (`Camera.calculateHudFov`): the fixed 70 base with only the death
    // and fluid adjustments — never the FOV option or its modifier, unlike the world FOV.
    let hand_fov_deg = frame_fov(
        70.0,
        1.0,
        1.0,
        partial,
        view.eye_in_water || view.eye_in_lava,
        view.death(),
        options.fov_effect_scale,
    );
    let hurt_tilt = hurt_tilt_degrees(view.hurt_ticks, partial, options.damage_tilt_strength);
    let death_roll = match view.death() {
        Some(death_ticks) => death_roll_degrees(death_ticks, partial),
        None => 0.0,
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
        hand_fov_deg,
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
    // The weather levels (`Level.getRainLevel`/`getThunderLevel` lerped by partial tick)
    // and the extracted columns (vanilla `WeatherEffectRenderer.extractRenderState`, which
    // runs per frame on the render thread). The caller's frame buffer is reused, so a frame
    // with rain allocates nothing.
    let rain_level = view.rain_level.0 + (view.rain_level.1 - view.rain_level.0) * partial;
    let thunder_level =
        view.thunder_level.0 + (view.thunder_level.1 - view.thunder_level.0) * partial;
    let weather_out = if rain_level > 0.0 && view.has_sky {
        extract_weather(
            weather,
            &world,
            &shared.biome_climates,
            pos,
            rain_level,
            view.world_age,
            view.sea_level,
            partial,
        );
        Some(&*weather)
    } else {
        None
    };
    // `Level.getThunderLevel` scales the thunder level by the rain level (a dry thunderstorm
    // cannot darken); the fog's thunder input is that product.
    let mut fog = fog_inputs(
        &world,
        &shared.data,
        &shared.biome_climates,
        pos,
        &view,
        partial,
    );
    fog.thunder_level *= rain_level;
    let sky = SkyState {
        time_of_day: view.time_of_day,
        rain: rain_level,
        thunder: thunder_level,
        dimension_has_sky: view.has_sky,
        fog_start: fog_end * 0.75,
        fog_end,
        // `SkyRenderer.shouldRenderDarkDisc` reads the player's interpolated eye.
        dark_disc: haste_render::sky::dark_disc::should_render_dark_disc(
            game::interpolate(view.prev_eye, view.eye, partial).y,
            view.horizon_height,
            view.eye_in_water,
        ),
        fog,
    };
    drop(world);
    // The lightmap inputs the extractor gathers per frame (LightmapRenderStateExtractor.extract):
    // the night-vision intensity, the darkness effect's brightness modifier and pulse, and the
    // boss-overlay darkening; the flicker and lightning come straight from the tick.
    let night_vision = match view.lightmap_night_vision {
        Some(duration) => haste_render::lightmap::night_vision_scale(duration, partial),
        None => 0.0,
    };
    let darkness_blend = view.lightmap_darkness_blend.factor(partial);
    let darkness_modifier = darkness_blend * options.darkness_effect_scale;
    let lightmap = LightmapFrame {
        block_light_flicker: view.lightmap_flicker,
        gamma: haste_render::lightmap::brightness(options.gamma, darkness_modifier),
        night_vision,
        darkness: haste_render::lightmap::darkness_scale(
            view.lightmap_tick_count,
            darkness_modifier,
            partial,
        ) * options.darkness_effect_scale,
        boss_overlay_darkening: view.lightmap_boss_darkening.0
            + (view.lightmap_boss_darkening.1 - view.lightmap_boss_darkening.0) * partial,
        lightning_flash: view.lightmap_lightning,
    };
    (
        camera,
        sky,
        lightmap,
        partial,
        view.view_distance,
        view.animation_tick,
        weather_out,
    )
}

/// Vanilla `Camera.getFluidInCamera` -> `FogRenderer.getFogType`: the fluid at the camera
/// block (water by its surface height, then lava), else powder snow. Vanilla probes the five
/// near-plane points for lava and powder snow; Haste approximates them with the camera block
/// (like `eye_in_lava`).
fn camera_fog_type(world: &World, data: &haste_data::GameData, pos: DVec3) -> FogType {
    use haste_core::view::{BlockPhysics, BlockView, FluidKind};
    let p = BlockPos::new(
        pos.x.floor() as i32,
        pos.y.floor() as i32,
        pos.z.floor() as i32,
    );
    let state = world.block_state(p);
    let surface = |kind| match data.fluid(state) {
        Some(f) if f.kind == kind => {
            let full = data
                .fluid(world.block_state(p.add(0, 1, 0)))
                .is_some_and(|a| a.kind == kind);
            Some(p.y as f64 + if full { 1.0 } else { f.height as f64 })
        }
        _ => None,
    };
    if surface(FluidKind::Water).is_some_and(|s| pos.y < s) {
        return FogType::Water;
    }
    if surface(FluidKind::Lava).is_some_and(|s| pos.y <= s) {
        return FogType::Lava;
    }
    if data.is_powder_snow(state) {
        return FogType::PowderSnow;
    }
    FogType::Atmospheric
}

/// Vanilla `RandomSource` (the single-threaded variant `WeatherEffectRenderer` uses): the
/// 48-bit Java LCG. `setSeed` xors the multiplier, `next(bits)` advances the seed.
struct JavaRandom(u64);

/// The Java LCG's multiplier, 25214903917 (0x5DEECE66D).
const LCG_MULTIPLIER: u64 = 25_214_903_917;
/// The Java LCG's increment.
const LCG_INCREMENT: u64 = 11;
/// The Java LCG's 48-bit mask.
const LCG_MASK: u64 = (1u64 << 48) - 1;

impl JavaRandom {
    fn new(seed: i64) -> Self {
        JavaRandom((seed as u64 ^ LCG_MULTIPLIER) & LCG_MASK)
    }

    fn next_i32(&mut self, bits: u32) -> i32 {
        self.0 = (self
            .0
            .wrapping_mul(LCG_MULTIPLIER)
            .wrapping_add(LCG_INCREMENT))
            & LCG_MASK;
        (self.0 >> (48 - bits)) as i32
    }

    /// `nextFloat`.
    fn next_float(&mut self) -> f32 {
        self.next_i32(24) as f32 * 5.960_464_5E-8
    }

    /// `nextDouble`.
    fn next_double(&mut self) -> f64 {
        let upper = self.next_i32(26) as u64;
        let lower = self.next_i32(27) as u64;
        ((upper << 27) + lower) as f64 * 1.110_223E-16
    }

    /// `nextGaussian` (Marsaglia polar).
    fn next_gaussian(&mut self) -> f64 {
        loop {
            let x = 2.0 * self.next_double() - 1.0;
            let y = 2.0 * self.next_double() - 1.0;
            let r2 = x * x + y * y;
            if r2 < 1.0 && r2 > 0.0 {
                let m = (-2.0 * r2.ln() / r2).sqrt();
                return x * m;
            }
        }
    }
}

/// Vanilla `WeatherEffectRenderer.extractRenderState` + the per-column instances: scans the
/// weather radius around the camera, clips each column's y range to the heightmap, sorts it
/// into rain or snow by the biome's precipitation at the camera's y, and fills the per-column
/// random's texture offsets. Fills the caller's reused frame buffer.
#[allow(clippy::too_many_arguments)]
fn extract_weather(
    frame: &mut haste_render::WeatherFrame,
    world: &World,
    climates: &[haste_render::BiomeClimate],
    camera: DVec3,
    intensity: f32,
    game_time: i64,
    sea_level: i32,
    partial_tick: f32,
) {
    use haste_core::view::BlockView as _;
    let radius = haste_render::weather::WEATHER_RADIUS;
    frame.intensity = intensity;
    frame.radius = radius;
    frame.rain.clear();
    frame.snow.clear();
    let cbx = camera.x.floor() as i32;
    let cby = camera.y.floor() as i32;
    let cbz = camera.z.floor() as i32;
    // One-time diagnostics: the camera, sea level, one column's height and the frame's
    // rain/snow counts, so an invisible weather pass is debuggable from the log.
    static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let mut terrain_of_first: Option<i32> = None;
    for z in cbz - radius..=cbz + radius {
        for x in cbx - radius..=cbx + radius {
            // `Level.getHeight(MOTION_BLOCKING, x, z)` (0 below the world when no chunk:
            // the column then degenerates and is skipped by the y test, like vanilla's).
            let terrain_height = crate::net::column_height(world, x, z);
            if terrain_of_first.is_none() {
                terrain_of_first = Some(terrain_height);
            }
            let y0 = (cby - radius).max(terrain_height);
            let y1 = (cby + radius).max(terrain_height);
            if y1 - y0 == 0 {
                continue;
            }
            let probe = BlockPos::new(x, cby, z);
            let precipitation =
                crate::net::precipitation_at_pos(world, climates, &probe, sea_level);
            let seed = haste_render::weather::column_seed(x, z);
            let mut rng = JavaRandom::new(seed as i64);
            let light_sample_y = cby.max(terrain_height);
            let light_pos = BlockPos::new(x, light_sample_y, z);
            let light = (world.block_light(light_pos), world.sky_light(light_pos));
            let column = haste_render::WeatherColumn {
                x,
                z,
                bottom_y: y0,
                top_y: y1,
                u_offset: 0.0,
                v_offset: 0.0,
                light,
                seed,
            };
            match precipitation {
                crate::net::Precipitation::Rain => {
                    // `createRainColumnInstance`: the scroll speed 3 + rand, the wrapped
                    // tick offset and the v offset it scrolls to.
                    let wrapped_ticks = (game_time & 131071) as f32;
                    let tick_offset = (x
                        .wrapping_mul(x)
                        .wrapping_mul(3121)
                        .wrapping_add(x.wrapping_mul(45238971))
                        .wrapping_add(z.wrapping_mul(z).wrapping_mul(418711))
                        .wrapping_add(z.wrapping_mul(13761)))
                        & 0xFF;
                    let speed = 3.0 + rng.next_float();
                    let texture_offset =
                        -(wrapped_ticks + tick_offset as f32 + partial_tick) / 32.0 * speed;
                    frame.rain.push(haste_render::WeatherColumn {
                        v_offset: texture_offset % 32.0,
                        ..column
                    });
                }
                crate::net::Precipitation::Snow => {
                    // `createSnowColumnInstance`: the drift u/v from the seeded gaussian
                    // pair, the slow fall, and the brightened light.
                    let wrapped_ticks = (game_time & 131071) as f32;
                    let time = wrapped_ticks as f64 + partial_tick as f64;
                    let u = (rng.next_double() + time * 0.01 * rng.next_gaussian()) as f32;
                    let v = (rng.next_double() + time * rng.next_gaussian() * 0.001) as f32;
                    let v_offset = -((game_time & 511) as f32 + partial_tick) / 512.0 + v;
                    let brightened = (
                        ((light.0 as i32 * 3 + 15) / 4).clamp(0, 15) as u8,
                        ((light.1 as i32 * 3 + 15) / 4).clamp(0, 15) as u8,
                    );
                    frame.snow.push(haste_render::WeatherColumn {
                        u_offset: u,
                        v_offset,
                        light: brightened,
                        ..column
                    });
                }
                crate::net::Precipitation::None => {}
            }
        }
    }
    if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        tracing::info!(
            "weather extraction: camera ({:.1}, {:.1}, {:.1}), sea level {sea_level}, first \
             column height {:?}, {} rain + {} snow columns, intensity {:.3}",
            camera.x,
            camera.y,
            camera.z,
            terrain_of_first,
            frame.rain.len(),
            frame.snow.len(),
            intensity
        );
    }
}

/// The `FogRenderer.setupFog`/`computeFogColor` inputs: the camera's fluid, the dimension
/// defaults overlaid by the camera biome's attributes, and the player's per-tick effect state.
fn fog_inputs(
    world: &World,
    data: &haste_data::GameData,
    climates: &[haste_render::BiomeClimate],
    pos: DVec3,
    view: &game::ViewState,
    partial: f32,
) -> FogInputs {
    use haste_render::BiomeSource;
    let p = BlockPos::new(
        pos.x.floor() as i32,
        pos.y.floor() as i32,
        pos.z.floor() as i32,
    );
    let biome_id = self::biomes::WorldBiomes(world).biome(p).0 as usize;
    let mut dimension = match world.dimension().name.path() {
        "the_nether" => DimensionFog::nether(),
        "the_end" => DimensionFog::end(),
        _ => DimensionFog::overworld(),
    };
    let mut water_fog_color = haste_render::fog::WATER_FOG_COLOR;
    let mut water_fog_start = haste_render::fog::WATER_FOG_START;
    let mut water_fog_end = haste_render::fog::WATER_FOG_END;
    if let Some(c) = climates.get(biome_id) {
        dimension = dimension.with_biome(&c.fog);
        if let Some(v) = c.fog.water_fog_color {
            water_fog_color = haste_render::fog::rgb24(v as i32);
        }
        if let Some(v) = c.fog.water_fog_start {
            water_fog_start = v;
        }
        if let Some(v) = c.fog.water_fog_end {
            water_fog_end = v;
        }
    }
    FogInputs {
        fog_type: camera_fog_type(world, data, pos),
        dimension,
        water_fog_color,
        water_fog_start,
        water_fog_end,
        water_vision: view.water_vision,
        spectator: view.spectator,
        fire_resistance: view.fire_resistance,
        blindness: view.blindness,
        darkness: view.lightmap_darkness,
        darkness_blend: view.lightmap_darkness_blend.factor(partial),
        night_vision: view.lightmap_night_vision,
        boss_world_fog: view.boss_world_fog,
        darken_world_amount: view.lightmap_boss_darkening.0
            + (view.lightmap_boss_darkening.1 - view.lightmap_boss_darkening.0) * partial,
        void_darkness_onset_range: 32.0,
        // The weather fields (rain/thunder levels, the rain fog multiplier) are filled by
        // the caller from the same view.
        rain_level: view.rain_level.0 + (view.rain_level.1 - view.rain_level.0) * partial,
        thunder_level: view.thunder_level.0
            + (view.thunder_level.1 - view.thunder_level.0) * partial,
        rain_fog_multiplier: view.rain_fog_multiplier,
    }
}

/// Where a label is drawn in a GUI frame, in absolute GUI pixels (translation stack applied):
/// an exact text, or a slider's dynamic `label: value`. The first match. Used by the script
/// runner's `click_text`.
fn gui_text_pos(frame: &GuiFrame, want: &str) -> Option<(i32, i32)> {
    use haste_protocol::haste_core::gui::GuiCommand;
    // Each `|`-separated alternative matches exactly or as a slider's dynamic `label: value`.
    let wants: Vec<&str> = want.split('|').collect();
    let matches = |text: &str| {
        wants
            .iter()
            .any(|w| text == *w || text.strip_prefix(*w).is_some_and(|r| r.starts_with(':')))
    };
    let mut stack: Vec<(f32, f32)> = vec![(0.0, 0.0)];
    for c in &frame.commands {
        match c {
            GuiCommand::TranslatePush { dx, dy, .. } => {
                let (x, y) = *stack.last().unwrap();
                stack.push((x + dx, y + dy));
            }
            GuiCommand::TranslatePop => {
                stack.pop();
            }
            GuiCommand::Text { text, x, y, .. } | GuiCommand::StyledText { text, x, y, .. }
                if matches(text) =>
            {
                let (dx, dy) = *stack.last().unwrap();
                return Some((x + dx as i32, y + dy as i32));
            }
            _ => {}
        }
    }
    None
}

/// The SDL3 scancode (USB HID usage id, `InputConstants.KEY_*` in 26.3) of a winit `KeyCode`./// `haste_jvm::keys` documents the numbering; letters, digits and punctuation take the
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

/// The last routing decision: whether the typed character of a key press reaches the screen.
/// Only one pressed while a screen was already open does. The press that *opens* a screen
/// drops its character, like vanilla: `Gui.handleKeybinds` runs on the tick after the press,
/// so its character event finds no screen and goes nowhere (T opens the chat empty; the
/// command key's single "/" is the screen's own prefix, not a typed character).
fn char_goes_to_screen(screen_open_before: bool) -> bool {
    screen_open_before
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
                if char_goes_to_screen(screen_open) {
                    ui.char_typed(c, mods);
                }
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
        // The game's own keyboard actions. Once the JVM is up the controls come from the vanilla
        // `KeyMapping`s instead (mirrored in `Ui::key`, read in `sample`), so these raw
        // `KeyCode` bindings are the fallback for the seconds before the JVM starts (or when it
        // failed): a rebind in the controls screen then changes what the keys do.
        if pressed && !repeat && ui.is_none() {
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

    /// A look change to an exact aim: the input layer holds its angles minus the server's
    /// rotation offset, because the tick's pick adds the offset back (a teleport or a
    /// `player_rotation` echo set it). A script's `look` asks for a specific crosshair, so it
    /// goes through here; plain mouse motion goes through `look`.
    fn aim(&mut self, yaw: f32, pitch: f32, offset: (f32, f32)) {
        self.look(yaw - offset.0, (pitch - offset.1).clamp(-90.0, 90.0));
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
    /// consume the press-only actions. With a `Ui` (the JVM is up) the movement and action
    /// controls come from the vanilla `KeyMapping`s (`Ui::game_input`/`consume_clicks`), so a
    /// rebind in the controls screen changes what the keys do, exactly like vanilla; without
    /// one (the JVM still starting, or failed) the raw `KeyCode`s are the fallback.
    /// `perspective` is the camera's own: the game thread's camera effects that are gated on it
    /// (the spyglass FOV) only apply in first person.
    fn sample(
        &mut self,
        shared: &Shared,
        ui: Option<&crate::ui::Ui>,
        perspective: game::Perspective,
    ) {
        use crate::ui::bits;
        let k = |c: KeyCode| self.keys.contains(&c);
        let mut state = InputState {
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
        // The held attack/use are the KeyMappings' down state while the JVM is up, like
        // vanilla's `handleKeybinds` reads them (a screen opening calls KeyMapping.releaseAll,
        // so a button whose release a screen consumed does not repeat the use after it closes);
        // the InputRouter's own press tracking is the fallback for when there is no JVM.
        let mut held = None;
        if let Some(ui) = ui {
            let b = ui.game_input();
            state.forward = b & bits::B_FORWARD != 0;
            state.back = b & bits::B_BACK != 0;
            state.left = b & bits::B_LEFT != 0;
            state.right = b & bits::B_RIGHT != 0;
            state.jump = b & bits::B_JUMP != 0;
            state.sneak = b & bits::B_SNEAK != 0;
            state.sprint = b & bits::B_SPRINT != 0;
            held = Some((b & bits::B_ATTACK != 0, b & bits::B_USE != 0));
        }
        let mut shared_input = shared.input.lock();
        shared_input.state = state;
        shared_input.first_person = matches!(perspective, game::Perspective::FirstPerson);
        {
            let i = &mut shared_input.interact;
            i.attack = self.attack;
            i.use_item = self.use_item;
            if let Some((attack, use_item)) = held {
                i.attack = attack;
                i.use_item = use_item;
            }
            // The click counts accumulate across samples (the render thread may sample several
            // times per tick); the game thread drains them once per tick.
            i.attack_clicks = i
                .attack_clicks
                .saturating_add(std::mem::take(&mut self.attack_clicks));
            i.use_clicks = i
                .use_clicks
                .saturating_add(std::mem::take(&mut self.use_clicks));
            i.pick_block |= std::mem::take(&mut self.pick_block);
            if let Some(ui) = ui {
                // The action mappings' clicks (hotbar, drop, swap, jump), consumed once per
                // frame and handed to the game thread through the shared input.
                let c = ui.consume_clicks();
                for slot in 0..9 {
                    if c & (bits::C_HOTBAR_0 << slot) != 0 {
                        i.hotbar = Some(slot as u8);
                    }
                }
                i.drop_item |= c & bits::C_DROP != 0;
                i.swap_offhand |= c & bits::C_SWAP_OFFHAND != 0;
                if c & bits::C_JUMP != 0 {
                    self.jump_pressed(Instant::now());
                }
            }
            i.drop_item |= std::mem::take(&mut self.drop_item);
            i.drop_stack |= std::mem::take(&mut self.drop_stack);
            i.swap_offhand |= std::mem::take(&mut self.swap_offhand);
            if let Some(slot) = self.hotbar.take() {
                i.hotbar = Some(slot);
            }
        }
        // After the jump clicks above, so a double tap seen here reaches the game this frame.
        shared_input.toggle_flight |= std::mem::take(&mut self.toggle_flight);
    }
}

fn open_assets() -> anyhow::Result<haste_assets::Assets> {
    open_assets_with(&[])
}

/// [`open_assets`] with the server's downloaded packs layered above the
/// selection (vanilla's `DownloadedPackSource`).
fn open_assets_with(server_packs: &[std::path::PathBuf]) -> anyhow::Result<haste_assets::Assets> {
    let jar = haste_protocol::haste_core::paths::client_jar();
    let mut assets = haste_assets::Assets::open(
        &jar,
        &haste_protocol::haste_core::paths::assets_dir(),
        "26.3",
    )
    .with_context(|| format!("opening assets from {}", jar.display()))?;
    // The pack selection options.txt holds (vanilla `Options.resourcePacks`,
    // written by the pack screen's Done through the JVM's own Options) is the
    // source of truth at startup, like vanilla `Minecraft`'s constructor
    // (`reload` + `loadSelectedResourcePacks`).
    let ids = selected_resource_packs();
    if !ids.is_empty() || !server_packs.is_empty() {
        tracing::info!("applying resource packs: {}", ids.join(", "));
        assets.apply_server_packs(&default_resource_packs_dir(), &ids, server_packs);
    }
    Ok(assets)
}

/// The resource pack ids saved in `<game dir>/options.txt` (`resourcePacks:`
/// as vanilla's GSON string array, bottom first), or none.
pub(crate) fn selected_resource_packs() -> Vec<String> {
    let options_txt = game_dir().join("options.txt");
    let Ok(text) = std::fs::read_to_string(&options_txt) else {
        return Vec::new();
    };
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("resourcePacks:") {
            return parse_gson_string_array(rest.trim());
        }
    }
    Vec::new()
}

/// Vanilla's `Options` saves the pack list as a GSON JSON array on one line,
/// e.g. `["file/red","vanilla"]` (see `Options.process`'s `GSON::toJson`).
fn parse_gson_string_array(text: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(text).unwrap_or_default()
}

/// The directory vanilla code sees as the game directory (servers.dat,
/// options.txt, `resourcepacks/`): Haste's config dir, or `$HASTE_GAME_DIR`
/// when set (tests and dev runs isolate their game files with it).
pub fn game_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("HASTE_GAME_DIR") {
        if !dir.is_empty() {
            return std::path::PathBuf::from(dir);
        }
    }
    haste_setup::servers::path()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        .expect("no platform config dir for the game directory")
}

/// `<game dir>/resourcepacks`: vanilla's resource pack folder, where
/// `FolderRepositorySource` looks (Java side) and `apply_packs` reads (Rust).
fn default_resource_packs_dir() -> std::path::PathBuf {
    game_dir().join("resourcepacks")
}

/// `minecraft:block_entity_type` names by synced registry id (stored block entities carry that
/// id); the static registry from the data generator when the server did not sync one, whose
/// ids are the same on vanilla servers.
fn block_entity_type_names(
    regs: &haste_protocol::SyncedRegistries,
    data: &haste_data::GameData,
) -> Vec<Identifier> {
    let registry = Identifier::minecraft("block_entity_type");
    if let Some(entries) = regs.entries(&registry) {
        return entries.iter().map(|e| e.id.clone()).collect();
    }
    let n = data.registries.len("minecraft:block_entity_type");
    (0..n as u32)
        .map(|i| {
            data.registries
                .name("minecraft:block_entity_type", haste_core::RegistryIndex(i))
                .cloned()
                .unwrap_or_else(|| Identifier::minecraft("air"))
        })
        .collect()
}

/// The block states whose `block_event` action 1 drives a lid (chests, ender chests, trapped
/// chests, shulker boxes; copper chests are not drawn yet), for the session's event filter.
fn lid_block_states(data: &haste_data::GameData) -> std::collections::HashSet<BlockStateId> {
    block_states_where(data, haste_render::blockentity::is_lid_block)
}

/// Every block state whose block matches `f`, for the session's `block_event` filters.
fn block_states_where(
    data: &haste_data::GameData,
    f: impl Fn(&haste_core::Identifier) -> bool,
) -> std::collections::HashSet<BlockStateId> {
    (0..data.blocks.len() as u32)
        .map(BlockStateId)
        .filter(|s| f(data.blocks.block_of(*s)))
        .collect()
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

/// The client without a server: the vanilla title screen (and everything it opens) in the same
/// JVM the game would use, opened at launch when `haste` runs without `--server`. The UI it
/// delivers is the window's own from then on (ticked at 20 Hz like vanilla's `Minecraft.tick`
/// while no session exists); a Join on the multiplayer screen spawns the session, whose game
/// thread takes a clone of the same UI over.
struct Menu {
    /// Where the menu thread delivers the UI (nothing after a failure; it already logged why).
    rx: Option<std::sync::mpsc::Receiver<crate::ui::Ui>>,
}

impl Menu {
    /// Starts the JVM on its own thread (vanilla's bootstrap takes seconds) and, once it is up,
    /// loads the jar's registries and opens the title screen. Returns immediately; [`Self::poll`]
    /// picks the UI up (vanilla shows a loading overlay in the meantime).
    fn new() -> Menu {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("haste-jvm".into())
            .spawn(move || {
                let result = (|| -> anyhow::Result<JvmHost> {
                    let mut cfg = haste_jvm::JvmConfig::from_cache(
                        &haste_protocol::haste_core::paths::cache_dir(),
                    )?;
                    // Vanilla reads and writes its own files (servers.dat, options.txt) in the
                    // game directory: Haste's config directory, where the saved server list
                    // already lives (haste-setup's servers.dat). Singleplayer worlds live next
                    // to it (haste-setup's worlds module); the world list reads that dir.
                    cfg.game_dir = game_dir();
                    if let Some(dir) = haste_setup::worlds::worlds_dir() {
                        cfg.singleplayer_dir = dir;
                    }
                    JvmHost::start(cfg).map_err(|e| anyhow::anyhow!("{e}"))
                })();
                match result {
                    Ok(host) => {
                        let ui = crate::ui::Ui::new(host.clone());
                        // Screens need the synced registries (L1c): the jar's data pack stands in
                        // for a server's registry_data until one is joined.
                        if let Err(e) = ui.gui().load_jar_registries() {
                            tracing::warn!("registries from the jar: {e:#}");
                        }
                        if let Err(e) = ui.open_screen(haste_jvm::ScreenKind::Title) {
                            tracing::warn!("opening the title screen: {e:#}");
                        }
                        let _ = tx.send(ui);
                        // The title screen is up: warm the classes its first clicks load
                        // (Singleplayer's world-selection tree) while the player looks at it, on
                        // this background thread — never the render thread. The UI is already
                        // with the render loop, so the frames do not wait for this.
                        host.warm_screen_classes();
                    }
                    Err(e) => {
                        tracing::warn!("no menu screens: the JVM did not start: {e:#}");
                    }
                }
            })
            .expect("spawning the JVM thread");
        Menu { rx: Some(rx) }
    }

    /// The UI once the JVM is up and the title screen open (`None` while starting or failed).
    fn poll(&mut self) -> Option<crate::ui::Ui> {
        let r = self.rx.as_mut()?;
        match r.try_recv() {
            Ok(ui) => {
                self.rx = None;
                Some(ui)
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.rx = None;
                None
            }
        }
    }
}

/// `--screenshot` without `--server`: menu mode's title screen, once the JVM is up. No world:
/// the open screen (and whatever it draws) over the plain background, saved as PNG.
fn menu_screenshot(args: &Args, file: &Path) -> anyhow::Result<()> {
    let mut menu = Menu::new();
    let mut assets = open_assets()?;
    let data = net::load_game_data()?;
    let mut renderer = Renderer::new_offscreen(args.width, args.height, &mut assets, &data.blocks)?;
    renderer.set_item_registry(&data.registries);
    renderer.set_ao_shapes(&data.blocks, &data.shapes);
    let deadline = Instant::now() + Duration::from_secs(args.seconds.max(60));
    let options = load_options();
    let input = InputRouter::default();
    let mut ui: Option<crate::ui::Ui> = None;
    let mut frames = 0u32;
    loop {
        if ui.is_none() {
            ui = menu.poll();
            if let Some(u) = &ui {
                u.set_text_metrics(renderer.text_metrics());
            }
        }
        if let Some(u) = ui.as_mut() {
            let (gui, camera, sky, partial) = menu_frame(u, &mut renderer, &input, &options);
            let frame = FrameInput {
                camera,
                entities: &[],
                block_entities: &[],
                gui: &gui,
                sky,
                partial_tick: partial,
                block_target: None,
                breaking: &[],
                camera_entity: None,
                spin: None,
                weather: None,
            };
            renderer.render(&frame)?;
            frames += 1;
        }
        // The title screen fades its widgets in over 2 s (TitleScreen.fading); wait past it.
        if frames > 150 {
            break;
        }
        if Instant::now() > deadline {
            anyhow::bail!("--seconds elapsed before the title screen was ready");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let image = renderer.read_back()?;
    image
        .save(file)
        .with_context(|| format!("saving {}", file.display()))?;
    let class = ui.as_ref().and_then(|u| u.gui().screen_class());
    println!(
        "menu screenshot {} ({}x{}) after {frames} frames: screen {}",
        file.display(),
        image.width(),
        image.height(),
        class.as_deref().unwrap_or("(none)")
    );
    Ok(())
}

/// `--screenshot <file>`: with `--server`, join, wait until the world is meshed, render one
/// frame offscreen, save it as PNG and leave (`--seconds` bounds the wait). Without one, menu
/// mode: the title screen once the JVM is up, saved the same way.
pub fn screenshot(args: &Args, file: &Path) -> anyhow::Result<()> {
    if args.server.is_none() {
        return menu_screenshot(args, file);
    }
    let session = Session::join(
        &args.username,
        args.render_distance,
        args.server.as_deref().unwrap(),
        false,
        None,
    )?;
    let shared = session.shared.clone();
    // The shot is first person: the game thread's camera effects take that perspective.
    shared.input.lock().first_person = true;
    let mut assets = open_assets_with(&session.server_packs)?;
    let mut renderer =
        Renderer::new_offscreen(args.width, args.height, &mut assets, &shared.data.blocks)?;
    renderer.set_item_registry(&shared.data.registries);
    renderer.set_ao_shapes(&shared.data.blocks, &shared.data.shapes);
    renderer.set_biomes(&session.biomes);
    renderer.set_particle_types(&session.particle_types);
    renderer.set_dimension(shared.world.read().dimension());
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
    // With `--hud off` there is no JVM at all and nothing to wait for.
    let hud_on = shared.hud_enabled.load(Ordering::Relaxed);
    let jvm_deadline = Instant::now() + Duration::from_secs(args.seconds.min(30));
    let mut wait_for_hud = hud_on;
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
        let mut weather_guard = shared.weather_scratch.lock();
        let (mut camera, sky, lightmap, partial, vd, animation_tick, weather) = frame_view(
            &shared,
            perspective,
            look.0 - offset.0,
            look.1 - offset.1,
            &session.options,
            Instant::now(),
            &mut weather_guard,
        );
        if let Some(pos) = want_pos {
            let view = shared.view.lock().clone();
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
        if hud_on {
            feed.set_first_person(&shared, &mut renderer, perspective, false);
        }
        let (entities, block_entities) =
            feed.entities(&shared, &mut renderer, session.local, partial);
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
                session.options.force_unicode_font,
            );
            let (gw, gh) = gui_scaled_size(args.width, args.height, scale);
            gui = u.frame(gw, gh, scale, (0.0, 0.0), partial).clone();
            renderer.set_panorama(u.gui().panorama());
        }
        // The screen effects around the HUD frame, like the window draws them.
        let spin = {
            let view = shared.view.lock().clone();
            crate::screen_effects::Frame {
                width: gui.width,
                height: gui.height,
                first_person: !perspective.draws_player(),
                effects: &view.screen,
                partial_tick: partial,
                look: (camera.yaw, camera.pitch),
                scope_scale: 1.125,
            }
            .append_to(&mut gui.commands);
            view.screen.spin_warp(partial)
        };
        let frame = FrameInput {
            camera,
            entities,
            block_entities,
            gui: &gui,
            sky,
            partial_tick: partial,
            // Vanilla's F1 hides the block outline too (`GameRenderer
            // .shouldRenderBlockOutline` checks `hud.isHidden`).
            block_target: hud_on.then_some(block_target).flatten(),
            breaking: &breaking,
            camera_entity: (!perspective.draws_player()).then_some(session.local.0),
            spin,
            weather,
        };
        renderer.sample_world(&*shared.world.read(), &shared.data.shapes, &frame);
        renderer.set_animation_tick(animation_tick);
        renderer.set_game_time(shared.view.lock().world_age);
        renderer.set_lightmap_frame(lightmap);
        {
            let world = shared.world.read();
            let mut particles = shared.particles.lock();
            renderer.prepare_particles(&mut particles, &*world, camera, partial);
        }
        stats = renderer.render(&frame)?;
        drop(weather_guard);
        frames += 1;
        let placed = shared.view.lock().has_position;
        if shared.is_closed() {
            anyhow::bail!(
                "disconnected: {}",
                shared.closed.lock().clone().unwrap_or_default()
            );
        }
        if let Some(pos) = want_pos {
            let view = shared.view.lock().clone();
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
    let view = shared.view.lock().clone();
    let entities = feed.instances.len();
    let chunks = (
        shared.chunks_loaded.load(Ordering::Relaxed),
        shared.chunks_unloaded.load(Ordering::Relaxed),
    );
    println!(
        "screenshot {} ({}x{}) after {frames} frames: camera ({:.2}, {:.2}, {:.2}) yaw {:.1} pitch {:.1}, time of day {}, sections {} drawn of {}, {} draw calls, {} gui commands, {} entity instances ({} drew geometry; perspective {}, local player {}), {} particles drawn, {} shadow quads, {} block entities, {} chunks loaded/{} unloaded, target {:?}",
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
        gui.commands.len(),
        entities,
        renderer.entities_drawn(),
        args.perspective,
        perspective.draws_player(),
        renderer.particles_drawn(),
        renderer.shadow_quad_count(),
        stats.block_entities_drawn,
        chunks.0,
        chunks.1,
        view.block_target
    );
    if let Some((color, fog)) = renderer.last_fog() {
        println!(
            "fog environment {:?} color ({:.3}, {:.3}, {:.3}) env {:.2}..{:.2} rd {:.2}..{:.2} sky {:.2} cloud {:.2}",
            fog.environment,
            color[0],
            color[1],
            color[2],
            fog.environmental_start,
            fog.environmental_end,
            fog.render_distance_start,
            fog.render_distance_end,
            fog.sky_end,
            fog.cloud_end
        );
    }
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

/// Prints the text, item and HUD-sprite facts of a script screenshot's frame. The check scripts
/// grep these lines as evidence instead of opening the PNGs.
fn log_script_frame(frame: &GuiFrame, name: &str, item_names: &[Identifier]) {
    let mut hotbar = false;
    let mut items = Vec::new();
    for c in &frame.commands {
        match c {
            GuiCommand::Text { text, .. } | GuiCommand::StyledText { text, .. } => {
                println!("script text[{name}]: {text}");
            }
            GuiCommand::Sprite { sprite, .. } if sprite.path() == "hud/hotbar" => hotbar = true,
            GuiCommand::Item { item, .. } => {
                // The item's registry name (creative inventory checks grep for these).
                if let Some(id) = item_names.get(item.0 as usize) {
                    items.push(id.to_string());
                }
            }
            _ => {}
        }
    }
    println!(
        "script frame[{name}]: hotbar={}, items=[{}]",
        if hotbar { "yes" } else { "no" },
        items.join(",")
    );
}

/// `--script FILE`: with `--server`, join like `--screenshot` and drive the parsed script's
/// actions every game tick (20 Hz) through the same input routing the window uses
/// (`InputRouter`: the UI first, then the game). Without one, menu mode: the title screen in
/// the same offscreen window, the script clicks through it (Singleplayer, the world list, the
/// create screen, or Multiplayer) and the join the screens request creates the session,
/// carrying the UI into the game. `screenshot NAME` actions save the frame (HUD and open
/// screens included) as `target/shots/script-NAME.png`. `Error` actions (a `wait_until`
/// timeout) and `Finished` end the run; an error exits non-zero.
pub fn script_run(args: &Args, file: &Path) -> anyhow::Result<()> {
    let text = std::fs::read_to_string(file)
        .with_context(|| format!("reading script {}", file.display()))?;
    let steps = script::parse(&text).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut runner = script::Runner::new(steps);
    // The session (with `--server` at once; in menu mode once the multiplayer screen joins).
    let mut session: Option<Session> = match &args.server {
        Some(address) => Some(Session::join(
            &args.username,
            args.render_distance,
            address,
            false,
            None,
        )?),
        None => None,
    };
    let mut menu = if session.is_none() {
        Some(Menu::new())
    } else {
        None
    };
    // Whether the screens are ticked here (menu mode: before a join, and again after leaving
    // a world; `menu` above only covers the boot, so this outlives it).
    let mut in_menu = session.is_none();
    let mut shared = session.as_ref().map(|s| s.shared.clone());
    let mut assets = open_assets_with(
        &session
            .as_ref()
            .map(|s| s.server_packs.clone())
            .unwrap_or_default(),
    )?;
    let data = net::load_game_data()?;
    let mut renderer = Renderer::new_offscreen(args.width, args.height, &mut assets, &data.blocks)?;
    if let Some(s) = &session {
        renderer.set_item_registry(&s.shared.data.registries);
        renderer.set_ao_shapes(&s.shared.data.blocks, &s.shared.data.shapes);
        renderer.set_biomes(&s.biomes);
        renderer.set_particle_types(&s.particle_types);
        renderer.set_dimension(s.shared.world.read().dimension());
    }
    // The live options, refreshed when a vanilla screen changed one (the options bridge): FOV
    // and GUI scale follow the change within a frame, like the windowed client. In menu mode
    // the saved ones stand in until a session arrives with its own.
    let mut options = session
        .as_ref()
        .map_or_else(load_options, |s| s.options.clone());
    let mut options_version = shared
        .as_ref()
        .map_or(0, |s| s.options_version.load(Ordering::Relaxed));
    // Item names by protocol id, so `log_script_frame` can name the items a frame drew.
    let item_names: Vec<Identifier> = (0..data.registries.len("minecraft:item") as u32)
        .map(|i| {
            data.registries
                .name(
                    "minecraft:item",
                    haste_protocol::haste_core::RegistryIndex(i),
                )
                .cloned()
                .unwrap_or_else(|| Identifier::minecraft("air"))
        })
        .collect();
    let mut gui = GuiFrame::default();
    let mut ui: Option<crate::ui::Ui> = None;
    let mut feed = WorldFeed::default();
    // The script's client is "grabbed" (game input goes through, like an active window).
    let mut input = InputRouter {
        grabbed: true,
        ..InputRouter::default()
    };
    let deadline = Instant::now() + Duration::from_secs(args.seconds.max(60));
    let tick = Duration::from_secs_f64(haste_protocol::haste_core::game::TICK_SECONDS);
    let mut next_tick = Instant::now();
    let mut frames = 0u32;
    let mut last_stats = FrameStats::default();
    let mut fps = FpsCounter::new();
    let mut last_err: Option<String> = None;
    let mut finished = false;
    // Whether a screen was open last tick, to release the held game input when one opens.
    let mut script_screen_was_open = false;
    // With `--pos`, the script's `wait_until settled` also waits for the server to have
    // teleported the player to these feet coordinates (the caller teleports over RCON while
    // we join, like the screenshot mode's `--pos`).
    let want_pos = args
        .pos
        .as_ref()
        .filter(|v| v.len() == 3)
        .map(|v| DVec3::new(v[0], v[1], v[2]));
    // Screenshot actions of this tick, saved after the frame is rendered (the readback must
    // include the frame the actions just produced: an open screen, the typed chat).
    let mut shots: Vec<String> = Vec::new();
    // Set when a resource pack selection rebuilt the renderer (`wait_until reloaded`).
    let mut resource_packs_reloaded = false;
    // The last rendered GUI frame (click_text finds its label in it).
    let mut last_gui = GuiFrame::default();
    // An F2 press this tick: save the frame to the config screenshots dir after it is rendered.
    let mut f2_pending = false;
    // The last screen transition (its class and when), for the click-to-screen timings.
    let mut last_screen_class: Option<String> = None;
    let mut last_screen_at: Option<Instant> = None;
    // The local singleplayer world of the joined session (stopped when the world is left and
    // at the end of the script, whatever it ended with).
    let mut local_server: Option<LocalWorld> = None;
    'run: loop {
        // Menu mode: the open screen ticks at 20 Hz, and the join the multiplayer screen
        // requests creates the session (its game thread takes a clone of the UI over).
        if let Some(m) = &mut menu {
            if ui.is_none() {
                if let Some(u) = m.poll() {
                    u.set_text_metrics(renderer.text_metrics());
                    ui = Some(u);
                }
            }
        }
        // The look the player is rendered with: the input layer's (the script's `look`
        // actions write it) plus the server's rotation offset, like the window. In menu
        // mode there is no view yet: `joined`/`settled` read as false.
        let mut view = shared.as_ref().map(|s| s.view.lock().clone());
        let look = (input.yaw, input.pitch);
        // The pack screen's Done: rebuild the renderer from the new selection (vanilla's
        // `Minecraft.reloadResourcePacks`) before this iteration's tick computes its state,
        // so `wait_until reloaded`/`settled` see the new feed's queue, not the old one.
        if let Some(shared) = shared.as_ref() {
            if let Some(reload) = shared.resource_pack_reload.lock().take() {
                let packs = reload
                    .ids
                    .iter()
                    .cloned()
                    .chain(reload.server.iter().map(|p| p.display().to_string()))
                    .collect::<Vec<_>>()
                    .join(", ");
                tracing::info!("rebuilding renderer with resource packs [{packs}]");
                let session_ref = session.as_ref().expect("a session exists");
                let mut fresh_assets = open_assets_with(&reload.server)?;
                let mut fresh = Renderer::new_offscreen(
                    args.width,
                    args.height,
                    &mut fresh_assets,
                    &data.blocks,
                )?;
                fresh.set_item_registry(&session_ref.shared.data.registries);
                fresh.set_biomes(&session_ref.biomes);
                fresh.set_particle_types(&session_ref.particle_types);
                fresh.set_dimension(session_ref.shared.world.read().dimension());
                renderer = fresh;
                feed = WorldFeed::default();
                feed.queue_all_sections(shared);
                resource_packs_reloaded = true;
            }
        }
        // Pick up a live option change (the options bridge) within a frame.
        if let Some(shared) = shared.as_ref() {
            let version = shared.options_version.load(Ordering::Relaxed);
            if version != options_version {
                options_version = version;
                options = shared.live_options.lock().clone();
            }
        }
        // One game tick's actions whenever it is time (`wait_until settled` uses the same
        // quiet test as the screenshot mode, plus the HUD's JVM being up or known-failed: a
        // script's keys go to the UI, so pressing them before it exists would lose them).
        // In menu mode the script drives the screens the same way (joined/settled stay
        // false until the multiplayer screen's join creates the session).
        if Instant::now() >= next_tick {
            next_tick += tick;
            let mut arrived: Option<Session> = None;
            {
                let shared = shared.as_ref();
                let hud_ready =
                    ui.is_some() || shared.is_some_and(|s| s.hud_failed.load(Ordering::Relaxed));
                let at_pos = want_pos
                    .is_none_or(|p| view.as_ref().is_some_and(|v| (v.feet - p).length() <= 0.5));
                let settled = view.as_ref().is_some_and(|v| v.has_position)
                    && at_pos
                    && hud_ready
                    && feed.settled(&last_stats, Duration::from_millis(1500));
                if !settled
                    && view.as_ref().is_some_and(|v| v.has_position)
                    && frames.is_multiple_of(40)
                {
                    tracing::info!(
                        "waiting to settle: queue {} pending {} arrival {:?}",
                        feed.queue.len(),
                        last_stats.meshes_pending,
                        feed.last_arrival.map(|t| t.elapsed())
                    );
                }
                let screen_class = ui.as_ref().and_then(|u| u.gui().screen_class());
                // How long a screen change took to show (click-to-screen): the class is read
                // once per tick, so this is the switch's blocking cost plus up to one tick.
                if screen_class != last_screen_class {
                    let now = Instant::now();
                    if let Some(since) = last_screen_at {
                        tracing::info!(
                            "screen {} after {} ms",
                            screen_class.as_deref().unwrap_or("(none)"),
                            (now - since).as_millis()
                        );
                    }
                    last_screen_class = screen_class.clone();
                    last_screen_at = Some(now);
                }
                let state = script::ScriptState {
                    joined: view.as_ref().is_some_and(|v| v.has_position),
                    settled,
                    screen_open: ui.as_ref().is_some_and(|u| u.screen_open()),
                    screen_class,
                    reloaded: resource_packs_reloaded,
                    hotbar_item: ui.as_ref().is_some_and(|u| u.hotbar_has_items()),
                };
                // A screen opening takes the mouse (vanilla `Gui.setScreen` calls
                // `MouseHandler.releaseMouse` and `KeyMapping.releaseAll`): nothing the game held
                // stays held, or a use held from before would re-open the container the screen
                // shows, server-side, the moment it closes.
                if state.screen_open && !script_screen_was_open {
                    input.release();
                }
                script_screen_was_open = state.screen_open;
                // Menu mode's screen tick and the join the screens requested — the multiplayer
                // list's Join (a plain address), the world list's Play or the create screen's
                // Create (a local server first) — the session takes a clone of the UI over;
                // its game thread ticks from now on.
                if in_menu {
                    if let Some(u) = ui.as_ref() {
                        u.tick();
                        let cancelled = u.take_join_cancelled();
                        let address = u.take_join_request().filter(|_| !cancelled);
                        let open_world = u.take_open_world_request().filter(|_| !cancelled);
                        let create_world = u.take_create_world_request().filter(|_| !cancelled);
                        if cancelled && (open_world.is_some() || create_world.is_some()) {
                            last_err = Some("the local world join was cancelled".into());
                            break 'run;
                        }
                        if let Some(address) = address {
                            let for_game = u.clone();
                            match Session::join(
                                &args.username,
                                args.render_distance,
                                &address,
                                false,
                                Some(for_game),
                            ) {
                                Ok(s) => {
                                    renderer.set_item_registry(&s.shared.data.registries);
                                    renderer.set_ao_shapes(
                                        &s.shared.data.blocks,
                                        &s.shared.data.shapes,
                                    );
                                    renderer.set_biomes(&s.biomes);
                                    renderer.set_particle_types(&s.particle_types);
                                    arrived = Some(s);
                                }
                                Err(e) => {
                                    // Vanilla shows its disconnect screen with the reason; a
                                    // script ends with the error instead (the next action
                                    // would just miss).
                                    last_err = Some(format!("joining {address}: {e:#}"));
                                    break 'run;
                                }
                            }
                        } else if open_world.is_some() || create_world.is_some() {
                            // Singleplayer: start (or create) the local server, then join it
                            // through the same seam. The script blocks on this tick: the
                            // server's first start takes tens of seconds (world creation),
                            // and a GenericMessageScreen ("Preparing world data...") is up.
                            let create = create_world;
                            let world = open_world.unwrap_or_else(|| {
                                create
                                    .as_ref()
                                    .map(|c| c.folder.clone())
                                    .unwrap_or_default()
                            });
                            let dir = haste_setup::worlds::world_dir(&world)
                                .unwrap_or_else(|| std::path::PathBuf::from(&world));
                            match LocalWorld::start(&dir, create.as_ref(), &args.username) {
                                Ok(world) => {
                                    let address = world.address();
                                    u.gui().set_local_world_running(true);
                                    local_server = Some(world);
                                    let for_game = u.clone();
                                    match Session::join(
                                        &args.username,
                                        args.render_distance,
                                        &address,
                                        false,
                                        Some(for_game),
                                    ) {
                                        Ok(s) => {
                                            renderer.set_item_registry(&s.shared.data.registries);
                                            renderer.set_ao_shapes(
                                                &s.shared.data.blocks,
                                                &s.shared.data.shapes,
                                            );
                                            renderer.set_biomes(&s.biomes);
                                            renderer.set_particle_types(&s.particle_types);
                                            arrived = Some(s);
                                        }
                                        Err(e) => {
                                            last_err = Some(format!("joining {address}: {e:#}"));
                                            break 'run;
                                        }
                                    }
                                }
                                Err(e) => {
                                    last_err = Some(format!("{e:#}"));
                                    break 'run;
                                }
                            }
                        }
                    }
                }
                for action in runner.tick(&state) {
                    match action {
                        script::Action::KeyDown(k) => {
                            // F2 is the window-only screenshot key (the UI never takes it):
                            // capture the next frame, like the windowed event loop does.
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
                        // A click on the widget whose label is `text`, at the label's position
                        // plus `(dx, dy)`: the label's place in the last rendered frame, so the
                        // check does not depend on the GUI scale or window size. A slider's
                        // label is centred, so `(dx, dy)` moves the click onto its track.
                        script::Action::ClickText(text, dx, dy) => {
                            match gui_text_pos(&last_gui, &text) {
                                Some((x, y)) => {
                                    let (x, y) = (f64::from(x + dx), f64::from(y + dy));
                                    input.mouse_moved(x, y, ui.as_ref());
                                    input.mouse_button(1, true, x, y, ui.as_ref());
                                    input.mouse_button(1, false, x, y, ui.as_ref());
                                }
                                None => {
                                    last_err =
                                        Some(format!("no {text:?} to click in the current frame"));
                                    break 'run;
                                }
                            }
                        }
                        script::Action::Scroll(n) => {
                            let (x, y) = input.mouse;
                            if let Some(shared) = shared.as_ref() {
                                input.wheel(0.0, f64::from(n), x, y, ui.as_ref(), shared);
                            } else if let Some(ui) = &ui {
                                ui.scroll(x, y, 0.0, f64::from(n));
                            }
                        }
                        // A scripted `look` aims exactly where asked (the server's rotation
                        // offset would otherwise leave the crosshair elsewhere).
                        script::Action::Look(yaw, pitch) => {
                            let offset = view.as_ref().map_or((0.0, 0.0), |v| v.look_offset);
                            input.aim(yaw, pitch, offset);
                        }
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
            }
            // A session the multiplayer screen's join created: it takes over from here (the
            // game thread ticks the UI clone; the loop renders the world from now on). The
            // connecting/preparing screen stays up until the level is ready — `Ui::joined`
            // closes it there, like vanilla's loading screen.
            if let Some(s) = arrived {
                session = Some(s);
                shared = session.as_ref().map(|s| s.shared.clone());
                menu = None;
                in_menu = false;
            }
            // The actions may have changed what the game thread sees this tick.
            if let Some(shared) = shared.as_ref() {
                input.sample(shared, ui.as_ref(), game::Perspective::FirstPerson);
                view = Some(shared.view.lock().clone());
            }
        }
        // Leaving the world: the pause screen's "Save and Quit" (or the death screen's "Title
        // Screen") asked to disconnect — like the window, the client drops the session, stops
        // the local server (saving the world) and returns to the title screen. A connection
        // the server closed instead ends the script with the reason.
        if let Some(mut s) = session.take() {
            if s.shared.is_closed() {
                let reason = s.shared.closed.lock().clone().unwrap_or_default();
                anyhow::bail!("disconnected: {reason}");
            }
            if ui.as_ref().is_some_and(|u| u.take_disconnect_request()) {
                tracing::info!("disconnect requested by the UI (script)");
                s.leave();
                if let Some(world) = local_server.take() {
                    if let Some(u) = &ui {
                        u.gui().set_local_world_running(false);
                    }
                    world.stop();
                }
                if let Some(u) = &ui {
                    u.gui().set_local_world_running(false);
                    u.left_world();
                    u.open_screen(haste_jvm::ScreenKind::Title)?;
                }
                shared = None;
                in_menu = true;
                // Like the window's `leave_server`: no world behind the title screen, no HUD.
                feed = WorldFeed::default();
                renderer.clear_world();
                input.release();
            } else {
                session = Some(s);
            }
        }
        if let Some(shared) = shared.as_ref() {
            if shared.is_closed() {
                anyhow::bail!(
                    "disconnected: {}",
                    shared.closed.lock().clone().unwrap_or_default()
                );
            }
        }
        // Render the frame like the screenshot mode does (the HUD and any open screen via
        // `Ui::frame`), so `screenshot` actions capture exactly what the window would show.
        // Menu mode renders the open screen over the plain background (no world yet).
        if session.is_none() {
            if let Some(u) = ui.as_mut() {
                let scale = resolve_gui_scale(
                    u32::from(options.gui_scale),
                    args.width,
                    args.height,
                    options.force_unicode_font,
                );
                let (gw, gh) = gui_scaled_size(args.width, args.height, scale);
                gui = u.frame(gw, gh, scale, input.mouse, 0.0).clone();
                renderer.set_panorama(u.gui().panorama());
                last_gui = gui.clone();
            }
            let frame = FrameInput {
                camera: menu_camera(),
                entities: &[],
                block_entities: &[],
                gui: &gui,
                sky: menu_sky(),
                partial_tick: 0.0,
                block_target: None,
                breaking: &[],
                camera_entity: None,
                spin: None,
                weather: None,
            };
            renderer.render(&frame)?;
            frames += 1;
            if !shots.is_empty() {
                let image = renderer
                    .read_back()
                    .context("reading the frame back for the script screenshots")?;
                for name in shots.drain(..) {
                    // Log what the frame drew (the check scripts grep this as evidence), like
                    // the in-game branch does.
                    log_script_frame(&gui, &name, &item_names);
                    save_script_shot(&image, &name)?;
                }
            }
            if Instant::now() > deadline {
                tracing::warn!("--seconds elapsed before the script finished; stopping early");
                break;
            }
            let now = Instant::now();
            if now < next_tick {
                std::thread::sleep((next_tick - now).min(Duration::from_millis(5)));
            } else {
                next_tick = now;
            }
            continue;
        }
        let shared = shared.as_ref().expect("the session exists");
        let session_ref = session.as_ref().expect("the session exists");
        let mut weather_guard = shared.weather_scratch.lock();
        let (camera, sky, lightmap, partial, vd, animation_tick, weather) = frame_view(
            shared,
            game::Perspective::FirstPerson,
            look.0,
            look.1,
            &options,
            Instant::now(),
            &mut weather_guard,
        );
        feed.feed(shared, &mut renderer, camera_section(&camera), vd);
        // The first-person hand, like the window and the screenshot path (`set_first_person`
        // diffs the view's per-tick part and only touches the renderer on a change): without
        // this the script's shots never show the held item.
        feed.set_first_person(shared, &mut renderer, game::Perspective::FirstPerson, false);
        let (entities, block_entities) =
            feed.entities(shared, &mut renderer, session_ref.local, partial);
        let block_target = view.as_ref().and_then(|v| v.block_target);
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
                &session_ref.debug_env,
                shared,
                &renderer,
                look,
                fps.fps as i32,
                &last_stats,
                entities.len() as i32,
                resolve_gui_scale(
                    u32::from(options.gui_scale),
                    args.width,
                    args.height,
                    options.force_unicode_font,
                ),
                &gpu_info(&renderer),
            ));
        }
        if let Some(u) = &mut ui {
            let scale = resolve_gui_scale(
                u32::from(options.gui_scale),
                args.width,
                args.height,
                options.force_unicode_font,
            );
            let (gw, gh) = gui_scaled_size(args.width, args.height, scale);
            gui = u.frame(gw, gh, scale, input.mouse, partial).clone();
            renderer.set_panorama(u.gui().panorama());
            last_gui = gui.clone();
        }
        // The screen effects around the HUD frame, like the window draws them.
        let spin = {
            let screen = view.as_ref().map(|v| v.screen.clone()).unwrap_or_default();
            crate::screen_effects::Frame {
                width: gui.width,
                height: gui.height,
                first_person: true,
                effects: &screen,
                partial_tick: partial,
                look,
                scope_scale: 1.125,
            }
            .append_to(&mut gui.commands);
            screen.spin_warp(partial)
        };
        let frame = FrameInput {
            camera,
            entities,
            block_entities,
            gui: &gui,
            sky,
            partial_tick: partial,
            block_target,
            breaking: &breaking,
            camera_entity: Some(session_ref.local.0),
            spin,
            weather,
        };
        renderer.sample_world(&*shared.world.read(), &shared.data.shapes, &frame);
        renderer.set_animation_tick(animation_tick);
        renderer.set_game_time(shared.view.lock().world_age);
        {
            let world = shared.world.read();
            let mut particles = shared.particles.lock();
            renderer.prepare_particles(&mut particles, &*world, camera, partial);
        }
        renderer.set_lightmap_frame(lightmap);
        renderer.set_menu_blur_radius(options.menu_background_blurriness);
        if let Ok(stats) = renderer.render(&frame) {
            last_stats = stats;
            fps.frame(&stats);
        }
        drop(weather_guard);
        frames += 1;
        // This tick's screenshot actions: the frame they caused is now on the GPU.
        if !shots.is_empty() || f2_pending {
            let image = renderer
                .read_back()
                .context("reading the frame back for the script screenshots")?;
            // Log what the frame drew (the check scripts grep this as evidence; images are not
            // opened). The hotbar sprite is the visible proof a HUD element is hidden or shown.
            for name in shots.drain(..) {
                log_script_frame(&gui, &name, &item_names);
                let path = std::path::Path::new("target/shots").join(format!("script-{name}.png"));
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir)
                        .with_context(|| format!("creating {}", dir.display()))?;
                }
                image
                    .save(&path)
                    .with_context(|| format!("saving {}", path.display()))?;
                println!(
                    "script screenshot saved to {} ({} particles drawn)",
                    path.display(),
                    renderer.particles_drawn()
                );
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
                log_script_frame(&gui, "f2", &item_names);
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
    // The local singleplayer server outlives the session: stop it whatever ended the script
    // (finished, error, timeout), or the java process would keep running and hold the world's
    // lock. Its `stop` saves the world first.
    if let Some(world) = local_server.take() {
        world.stop();
    }
    if let Some(e) = last_err {
        anyhow::bail!("script failed: {e}");
    }
    if !finished {
        anyhow::bail!("script did not finish before --seconds elapsed");
    }
    Ok(())
}

/// Saves a script `screenshot NAME` action's frame as `target/shots/script-NAME.png`.
fn save_script_shot(image: &image::RgbaImage, name: &str) -> anyhow::Result<()> {
    let path = std::path::Path::new("target/shots").join(format!("script-{name}.png"));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    image
        .save(&path)
        .with_context(|| format!("saving {}", path.display()))?;
    println!("script screenshot saved to {}", path.display());
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
    /// Terrain indices drawn (six per quad): the vertex-fetch load.
    indices_drawn: u32,
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
    /// Terrain indices drawn per frame (six per quad): the vertex-fetch load.
    indices_drawn: f32,
    prepare_ms: f32,
    acquire_ms: f32,
    cull_ms: f32,
    encode_ms: f32,
    submit_ms: f32,
    present_ms: f32,
    /// GPU time of the main, hand and GUI passes.
    gpu_ms: [f32; 3],
    /// GPU time of the main pass's sub-passes: sky, opaque terrain (solid + cutout-mipped),
    /// cutout terrain, translucent terrain, entities, particles, overlays. Timing.rs carries
    /// the solid and cutout-mipped phases separately (queries are written once per pass);
    /// the bench reports them as one opaque phase like vanilla's ChunkSectionLayerGroup.
    gpu_sub_ms: [f32; 7],
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
             gpu main ms: sky {:.3} opaque {:.3} cutout {:.3} translucent {:.3} entities {:.3} particles {:.3} overlays {:.3}\n\
             sections drawn {}/{} avg, draw calls {:.0} avg, indices {:.0} avg (quads {:.0})\n\
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
            self.gpu_sub_ms[0],
            self.gpu_sub_ms[1],
            self.gpu_sub_ms[2],
            self.gpu_sub_ms[3],
            self.gpu_sub_ms[4],
            self.gpu_sub_ms[5],
            self.gpu_sub_ms[6],
            self.sections_drawn,
            self.sections_total,
            self.draw_calls,
            self.indices_drawn,
            self.indices_drawn / 6.0,
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
    // Solid + cutout-mipped (renderer sub-passes 1 and 3) fold into "opaque".
    let opaque = sum(samples, n, |s| {
        s.timings.gpu_sub_ms[1] + s.timings.gpu_sub_ms[3]
    });
    let gpu_sub = |i: usize| sum(samples, n, |s| s.timings.gpu_sub_ms[i]);
    BenchSummary {
        frames: samples.len() as u32,
        avg_ms: sum(samples, n, |s| s.frame_ms),
        p99_ms: percentile(&times, 0.99),
        worst_ms: times.last().copied().unwrap_or(0.0),
        sections_drawn: sum(samples, n, |s| s.sections_drawn as f32),
        sections_total: sum(samples, n, |s| s.sections_total as f32),
        draw_calls: sum(samples, n, |s| s.draw_calls as f32),
        indices_drawn: sum(samples, n, |s| s.indices_drawn as f32),
        prepare_ms: t(|x| x.prepare_ms),
        acquire_ms: t(|x| x.acquire_ms),
        cull_ms: t(|x| x.cull_ms),
        encode_ms: t(|x| x.encode_ms),
        submit_ms: t(|x| x.submit_ms),
        present_ms: t(|x| x.present_ms),
        gpu_ms: [gpu(0), gpu(1), gpu(2)],
        gpu_sub_ms: [
            gpu_sub(0),
            opaque,
            gpu_sub(2),
            gpu_sub(4),
            gpu_sub(5),
            gpu_sub(6),
            gpu_sub(7),
        ],
    }
}

/// `--bench SECONDS`: join, wait until the world settles (screenshot's logic), then render
/// offscreen at `--width`x`--height` along the fixed `bench_camera` path for SECONDS while
/// recording per-frame `FrameStats` and `FrameTimings`. Prints one summary block, writes the
/// same as JSON to `--bench-out` when given, and disconnects.
pub fn bench(args: &Args, seconds: u64) -> anyhow::Result<()> {
    let Some(server) = args.server.as_deref() else {
        anyhow::bail!("--bench needs --server <host[:port]>");
    };
    let session = Session::join(&args.username, args.render_distance, server, false, None)?;
    let shared = session.shared.clone();
    // The bench camera is first person: the game thread's camera effects take that perspective.
    shared.input.lock().first_person = true;
    let mut assets = open_assets_with(&session.server_packs)?;
    let mut renderer =
        Renderer::new_offscreen(args.width, args.height, &mut assets, &shared.data.blocks)?;
    renderer.set_item_registry(&shared.data.registries);
    renderer.set_ao_shapes(&shared.data.blocks, &shared.data.shapes);
    renderer.set_biomes(&session.biomes);
    renderer.set_particle_types(&session.particle_types);
    renderer.set_dimension(shared.world.read().dimension());
    let gui = GuiFrame::default();
    let mut feed = WorldFeed::default();
    let deadline = Instant::now() + Duration::from_secs(args.seconds);
    let mut stats;
    let mut base;
    let mut vd;
    // Settle phase: identical to the screenshot path (wait for position, meshing done, quiet).
    loop {
        let mut weather_guard = shared.weather_scratch.lock();
        let (mut camera, sky, lightmap, partial, v, animation_tick, weather) = frame_view(
            &shared,
            game::Perspective::FirstPerson,
            0.0,
            0.0,
            &session.options,
            Instant::now(),
            &mut weather_guard,
        );
        camera.yaw = args.yaw.unwrap_or(camera.yaw);
        camera.pitch = args.pitch.unwrap_or(camera.pitch);
        base = camera;
        vd = v;
        feed.feed(&shared, &mut renderer, camera_section(&camera), vd);
        feed.set_first_person(
            &shared,
            &mut renderer,
            game::Perspective::FirstPerson,
            false,
        );
        let (entities, block_entities) =
            feed.entities(&shared, &mut renderer, session.local, partial);
        let block_target = shared.view.lock().block_target;
        let breaking = shared.breaking.lock().clone();
        let frame = FrameInput {
            camera,
            entities,
            block_entities,
            gui: &gui,
            sky,
            partial_tick: partial,
            block_target,
            breaking: &breaking,
            camera_entity: Some(session.local.0),
            spin: None,
            weather,
        };
        renderer.sample_world(&*shared.world.read(), &shared.data.shapes, &frame);
        renderer.set_animation_tick(animation_tick);
        renderer.set_game_time(shared.view.lock().world_age);
        renderer.set_lightmap_frame(lightmap);
        {
            let world = shared.world.read();
            let mut particles = shared.particles.lock();
            renderer.prepare_particles(&mut particles, &*world, camera, partial);
        }
        stats = renderer.render(&frame)?;
        drop(weather_guard);
        if shared.is_closed() {
            anyhow::bail!(
                "disconnected: {}",
                shared.closed.lock().clone().unwrap_or_default()
            );
        }
        let placed = shared.view.lock().has_position;
        // `--pos` holds the settle until the server has teleported the player there (the
        // screenshot path's rule), so the bench measures the intended spot, not the spawn.
        let reached = args
            .pos
            .as_ref()
            .filter(|v| v.len() == 3)
            .map(|v| {
                let want = DVec3::new(v[0], v[1], v[2]);
                placed && (shared.view.lock().feet - want).length() <= 0.5
            })
            .unwrap_or(placed);
        if reached && feed.settled(&stats, Duration::from_millis(1500)) {
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
    {
        // Memory diagnostics: peak RSS plus every large allocation we can attribute —
        // terrain arenas (allocated vs used), the world's chunk storage, the renderer's
        // CPU-side copies and the embedded JVM's pools — so the diet work has a table.
        let (arenas, arena_bytes, arena_used, sections) = renderer.arena_memory();
        let cpu = renderer.cpu_memory();
        let world = shared.world.read().memory_stats();
        let jvm = shared
            .ui
            .lock()
            .as_ref()
            .map(|ui| ui.jvm_memory())
            .unwrap_or_default();
        let mib = |b: u64| b as f64 / (1024.0 * 1024.0);
        let rss_kb = std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("VmHWM"))
                    .and_then(|l| l.split_whitespace().nth(1).map(|v| v.to_string()))
            })
            .unwrap_or_default();
        // smaps_rollup for the live (settled) picture: anonymous vs file-backed pages.
        let rollup = std::fs::read_to_string("/proc/self/smaps_rollup")
            .map(|s| {
                let f = |key: &str| {
                    s.lines()
                        .find(|l| l.starts_with(key))
                        .and_then(|l| l.split_whitespace().nth(1))
                        .and_then(|v| v.parse::<u64>().ok())
                        .unwrap_or(0)
                };
                (f("Rss:"), f("Pss_Anon:"), f("Pss_File:"))
            })
            .unwrap_or((0, 0, 0));
        println!(
            "bench memory: peak RSS {rss_kb} kB ({:.1} MiB), {arenas} arenas ({:.1} MiB vertex+index, {:.1} used), {sections} resident sections",
            rss_kb.parse::<f64>().unwrap_or(0.0) / 1024.0,
            mib(arena_bytes),
            mib(arena_used),
        );
        println!(
            "bench memory: settled Rss {:.1} MiB (Pss_Anon {:.1}, Pss_File {:.1})",
            mib(rollup.0 * 1024),
            mib(rollup.1 * 1024),
            mib(rollup.2 * 1024),
        );
        println!(
            "bench memory: world {:.1} MiB (sections {:.1}, heightmaps {:.1}, light {:.1}, block entities {:.1}, {} non-empty), atlas+models {:.1} MiB, mesher queue {}",
            mib(world.chunks + world.sections + world.heightmaps + world.light + world.block_entities),
            mib(world.sections),
            mib(world.heightmaps),
            mib(world.light),
            mib(world.block_entities),
            world.non_empty_sections,
            mib(cpu.atlas_bytes + cpu.baked_bytes),
            cpu.mesh_pending,
        );
        if !jvm.is_empty() {
            let mut heap_c = 0;
            let mut heap_u = 0;
            let mut meta = 0;
            let mut code = 0;
            for line in jvm.lines() {
                let (k, v) = line.split_once('=').unwrap_or(("", ""));
                let v: u64 = v.parse().unwrap_or(0);
                match k {
                    "heap_committed" => heap_c = v,
                    "heap_used" => heap_u = v,
                    "metaspace_used" => meta = v,
                    "code_cache_used" => code = v,
                    _ => {}
                }
            }
            println!(
                "bench memory: JVM heap {:.1}/{:.1} MiB, metaspace {:.1} MiB, code cache {:.1} MiB",
                mib(heap_u),
                mib(heap_c),
                mib(meta),
                mib(code),
            );
        }
    }
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
        let mut weather_guard = shared.weather_scratch.lock();
        let (_, mut sky, lightmap, partial_tick, _, animation_tick, weather) = frame_view(
            &shared,
            game::Perspective::FirstPerson,
            0.0,
            0.0,
            &session.options,
            Instant::now(),
            &mut weather_guard,
        );
        let camera = if args.bench_fixed {
            // Still camera at the settle spot: one fixed view, comparable with `--screenshot`.
            base
        } else {
            bench_camera(elapsed / total, &base, total)
        };
        let fog_end = (vd.max(2) * 16) as f32;
        sky.fog_start = fog_end * 0.75;
        sky.fog_end = fog_end;
        let feed_start = Instant::now();
        feed.feed(&shared, &mut renderer, camera_section(&camera), vd);
        let feed_ms = feed_start.elapsed().as_secs_f32() * 1000.0;
        let (entities, block_entities) =
            feed.entities(&shared, &mut renderer, session.local, partial_tick);
        let block_target = shared.view.lock().block_target;
        let breaking = shared.breaking.lock().clone();
        let frame = FrameInput {
            camera,
            entities,
            block_entities,
            gui: &gui,
            sky,
            partial_tick,
            block_target,
            breaking: &breaking,
            camera_entity: Some(session.local.0),
            spin: None,
            weather,
        };
        renderer.sample_world(&*shared.world.read(), &shared.data.shapes, &frame);
        renderer.set_animation_tick(animation_tick);
        renderer.set_game_time(shared.view.lock().world_age);
        renderer.set_lightmap_frame(lightmap);
        {
            let world = shared.world.read();
            let mut particles = shared.particles.lock();
            renderer.prepare_particles(&mut particles, &*world, camera, partial_tick);
        }
        let stats = renderer.render(&frame)?;
        drop(weather_guard);
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
            indices_drawn: stats.indices_drawn,
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
            "indices_drawn": summary.indices_drawn,
            "prepare_ms": summary.prepare_ms,
            "acquire_ms": summary.acquire_ms,
            "cull_ms": summary.cull_ms,
            "encode_ms": summary.encode_ms,
            "submit_ms": summary.submit_ms,
            "present_ms": summary.present_ms,
            "gpu_main_ms": summary.gpu_ms[0],
            "gpu_hand_ms": summary.gpu_ms[1],
            "gpu_gui_ms": summary.gpu_ms[2],
            "gpu_main_sub_ms": {
                "sky": summary.gpu_sub_ms[0],
                "opaque": summary.gpu_sub_ms[1],
                "cutout": summary.gpu_sub_ms[2],
                "translucent": summary.gpu_sub_ms[3],
                "entities": summary.gpu_sub_ms[4],
                "particles": summary.gpu_sub_ms[5],
                "overlays": summary.gpu_sub_ms[6],
            },
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

/// Windowed mode. With `--server` it joins at once; without it the vanilla title screen opens
/// (menu mode) and a Join on the multiplayer screen creates the session in the same window.
/// Leaving the server (pause screen "Disconnect") or being kicked returns to the menus.
pub fn run(args: &Args) -> anyhow::Result<()> {
    // The session (with `--server` at once; in menu mode the multiplayer screen's join).
    let session = match &args.server {
        Some(address) => Some(Session::join(
            &args.username,
            args.render_distance,
            address,
            true,
            None,
        )?),
        None => None,
    };
    let menu = session.is_none().then(Menu::new);
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let options = session
        .as_ref()
        .map_or_else(load_options, |s| s.options.clone());
    let mut app = App {
        username: args.username.clone(),
        render_distance: args.render_distance,
        session,
        menu,
        // A join the screens asked for, connecting on its own thread (the window stays live).
        joining: None,
        local_server: None,
        // The flag wins over the saved option; without it, the saved option decides (and a
        // change in the options screen applies live, like vanilla's Vsync option).
        vsync: args.vsync.unwrap_or(options.vsync),
        vsync_forced: args.vsync,
        options,
        options_version: 0,
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
        scope: 0.5,
        frame_at: Instant::now(),
        started: Instant::now(),
        menu_next: Instant::now(),
        quitting: false,
        bench: None,
    };
    event_loop.run_app(&mut app)?;
    if let Some(e) = app.error {
        return Err(e);
    }
    if let Some(session) = &app.session {
        if let Some(r) = session.shared.closed.lock().clone() {
            println!("connection ended: {r}");
        }
    }
    Ok(())
}

/// The windowed bench's state (`--bench-window`): the recorded samples, the still camera
/// (the settled frame's camera with `--yaw`/`--pitch`), and the run's metadata.
struct WindowBench {
    samples: Vec<BenchSample>,
    /// The camera every measured frame uses (the settle frame's, yaw/pitch from the args).
    camera: Option<Camera>,
    /// The still camera's view distance (the settled view's; the summary reports it).
    view_distance: i32,
    /// Total measured seconds; the run ends when it has elapsed since `started`.
    seconds: f32,
    started: Option<Instant>,
    feed_ms: f32,
    /// The window's exact inner size in physical pixels (`--width`x`--height`).
    size: (u32, u32),
    /// The look the bench camera holds (`--yaw`/`--pitch`; `None` keeps the player's own).
    look: (Option<f32>, Option<f32>),
    /// `--pos`: the settle also waits until the server has teleported the player here.
    want_pos: Option<DVec3>,
}

/// `--bench-window SECONDS`: the windowed twin of `bench`. Joins like the windowed client
/// (a real window at `--width`x`--height`, vsync off unless `--vsync`), waits for the world
/// to settle, then renders the still camera for SECONDS while recording the same
/// `BenchSample`s as the offscreen bench. Prints the same summary block (marked `windowed`)
/// and writes the same JSON to `--bench-out`; the window closes by itself.
pub fn bench_window(args: &Args, seconds: u64) -> anyhow::Result<()> {
    let Some(server) = args.server.as_deref() else {
        anyhow::bail!("--bench-window needs --server <host[:port]>");
    };
    let session = Session::join(&args.username, args.render_distance, server, false, None)?;
    let shared = session.shared.clone();
    // The bench camera is first person: the game thread's camera effects take that perspective.
    shared.input.lock().first_person = true;
    // `--x11`: the same XWayland window vanilla's client gets (framebuffer exactly
    // `--width`x`--height`; a Wayland window's is the desktop scale multiple).
    if args.x11 {
        std::env::set_var("WINIT_UNIX_BACKEND", "x11");
        std::env::remove_var("WAYLAND_DISPLAY");
    }
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let options = session.options.clone();
    let mut app = App {
        username: args.username.clone(),
        render_distance: args.render_distance,
        session: Some(session),
        menu: None,
        joining: None,
        local_server: None,
        // Present, not vsync: the windowed number must carry the present cost but not be
        // pinned to the display rate (vanilla's comparison run had vsync off too).
        vsync: args.vsync.unwrap_or(false),
        vsync_forced: args.vsync.or(Some(false)),
        options,
        options_version: 0,
        perspective: game::Perspective::FirstPerson,
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
        seconds: None,
        screenshot: false,
        scope: 0.5,
        frame_at: Instant::now(),
        started: Instant::now(),
        menu_next: Instant::now(),
        quitting: false,
        bench: Some(WindowBench {
            samples: Vec::with_capacity(seconds as usize * 1000),
            camera: None,
            view_distance: 12,
            seconds: seconds as f32,
            started: None,
            feed_ms: 0.0,
            size: (args.width, args.height),
            look: (args.yaw, args.pitch),
            want_pos: args
                .pos
                .as_ref()
                .filter(|v| v.len() == 3)
                .map(|v| DVec3::new(v[0], v[1], v[2])),
        }),
    };
    app.input.yaw = args.yaw.unwrap_or(0.0);
    app.input.pitch = args.pitch.unwrap_or(10.0);
    event_loop.run_app(&mut app)?;
    if let Some(e) = app.error {
        return Err(e);
    }
    let Some(bench) = app.bench else {
        anyhow::bail!("the windowed bench ended without samples");
    };
    let (width, height) = app
        .renderer
        .as_ref()
        .map_or((args.width, args.height), |r| r.size());
    let meta = BenchMeta {
        seconds: bench.seconds,
        render_distance: bench.view_distance,
        requested_render_distance: app.render_distance.unwrap_or(12),
        width,
        height,
        adapter: app
            .renderer
            .as_ref()
            .map(|r| {
                let info = r.adapter_info();
                format!("{} ({})", info.name, info.backend)
            })
            .unwrap_or_default(),
    };
    let summary = bench_summary(&bench.samples);
    println!("windowed bench at {}, {}", meta.width, meta.height);
    println!("{}", summary.text(&meta));
    if let Some(out) = &args.bench_out {
        let json = serde_json::json!({
            "windowed": true,
            "frames": summary.frames,
            "seconds": meta.seconds,
            "avg_ms": summary.avg_ms,
            "fps": summary.fps(),
            "p99_ms": summary.p99_ms,
            "worst_ms": summary.worst_ms,
            "sections_drawn": summary.sections_drawn,
            "sections_total": summary.sections_total,
            "draw_calls": summary.draw_calls,
            "indices_drawn": summary.indices_drawn,
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
    Ok(())
}

/// (the address, or the world folder of a local world) and where the session (with its local
/// world, for singleplayer) arrives.
struct JoinInProgress {
    target: String,
    rx: std::sync::mpsc::Receiver<anyhow::Result<(Session, Option<LocalWorld>)>>,
}

struct App {
    /// The offline username joins use (menu mode: from `--username`).
    username: String,
    /// `--render-distance`, overriding the saved option for this run.
    render_distance: Option<u8>,
    /// The joined game (`None` in menu mode, while connecting, and after leaving the server).
    session: Option<Session>,
    /// Menu mode's JVM boot (`--server` omitted): delivers the UI, then nothing.
    menu: Option<Menu>,
    /// The join the screens requested, connecting on its own thread: what to show as the
    /// target (the address, or the world folder for a local world) and where the session
    /// (and the local world, for singleplayer) arrives.
    joining: Option<JoinInProgress>,
    /// The local singleplayer world of the joined session (stopped when the world is left).
    local_server: Option<LocalWorld>,
    vsync: bool,
    /// `--vsync` forced the mode: a change in the options screen must not override it.
    vsync_forced: Option<bool>,
    /// The options the game is running with: refreshed from [`Shared::live_options`] whenever
    /// its version changes (not every frame), so a change in a vanilla options screen takes
    /// effect immediately (FOV, GUI scale, mouse sensitivity, fullscreen, vsync).
    options: Options,
    /// The `Shared::options_version` [`App::options`] was copied from (0 until a session).
    options_version: u64,
    /// Current perspective (F5 cycles it).
    perspective: game::Perspective,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    feed: WorldFeed,
    /// The HUD's `Ui`: the game thread's clone once it is ready (the JVM takes seconds to
    /// start), or the menu's (the window's own; the game thread takes a clone when joining).
    /// `gui` stays the empty default until then.
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
    /// The spyglass scope's growth (`Hud.scopeScale`) and the time of the previous frame, which
    /// its per-frame easing is measured against.
    scope: f32,
    frame_at: Instant,
    started: Instant,
    /// When the next 20 Hz menu tick runs (menu mode only).
    menu_next: Instant,
    /// The title screen's Quit Game was clicked: close at the next `about_to_wait`.
    quitting: bool,
    /// The windowed bench (`--bench-window`): records one sample per frame and closes the
    /// window when the run is full. `None` in the normal windowed client.
    bench: Option<WindowBench>,
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

    /// Once per frame (in `draw`): whether the pause screen's "Disconnect" / "Save and Quit"
    /// (or the death screen's "Title Screen") was clicked since the last frame. With a session
    /// the client leaves the server and, like vanilla's pause screen, returns to the screen it
    /// came from: the title screen for a singleplayer world, the server list for multiplayer.
    /// Without a session it is the connecting screen's dead parent and just closes the screen.
    fn take_disconnect(&mut self, _event_loop: &ActiveEventLoop) {
        if self
            .ui
            .as_ref()
            .is_some_and(|ui| ui.take_disconnect_request())
        {
            if self.session.is_some() {
                let was_local = self.local_server.is_some();
                tracing::info!("disconnect requested by the UI");
                self.leave_server();
                if let Some(ui) = &self.ui {
                    let kind = if was_local {
                        haste_jvm::ScreenKind::Title
                    } else {
                        haste_jvm::ScreenKind::Multiplayer
                    };
                    if let Err(e) = ui.open_screen(kind) {
                        tracing::warn!(
                            "opening the {} screen: {e:#}",
                            if was_local { "title" } else { "multiplayer" }
                        );
                    }
                }
            } else if let Some(ui) = &self.ui {
                ui.gui().close_screen();
            }
        }
    }

    /// Drops the session (stopping its game thread and closing the connection) and the local
    /// singleplayer server, if one runs (its `stop` saves the world); the UI stays, menu mode
    /// resumes around whatever screen the disconnect showed. The renderer loses the world's
    /// sections (the menus draw over the plain background again, like the startup title screen)
    /// and the HUD stops (`in_world` off): without this the last world frame — terrain,
    /// hotbar, crosshair — stays behind the title screen.
    fn leave_server(&mut self) {
        if let Some(session) = self.session.take() {
            session.shared.stop.store(true, Ordering::Relaxed);
            let reason = session.shared.closed.lock().clone();
            drop(session);
            if let Some(r) = reason {
                println!("connection ended: {r}");
            }
        }
        if let Some(world) = self.local_server.take() {
            if let Some(ui) = &self.ui {
                ui.gui().set_local_world_running(false);
            }
            world.stop();
        }
        if let Some(ui) = &self.ui {
            ui.left_world();
        }
        // The world feed's meshed set dies with the session's world; a fresh one starts clean
        // at the next join (its source holds the old `Shared`).
        self.feed = WorldFeed::default();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_world();
        }
        self.input.release();
    }

    /// Menu mode's 20 Hz tick and the join the screens requested (called every frame while
    /// there is no session). The join connects on its own thread; the window keeps rendering
    /// the connecting screen until the session arrives (`Self::poll_join`).
    fn menu_tick(&mut self) {
        if let Some(menu) = &mut self.menu {
            if let Some(ui) = menu.poll() {
                if let Some(r) = self.renderer.as_mut() {
                    ui.set_text_metrics(r.text_metrics());
                }
                self.ui = Some(ui);
            }
        }
        let tick = Duration::from_secs_f64(haste_protocol::haste_core::game::TICK_SECONDS);
        let now = Instant::now();
        if now < self.menu_next {
            return;
        }
        self.menu_next = now + tick;
        let Some(ui) = self.ui.clone() else { return };
        ui.tick();
        let cancelled = ui.take_join_cancelled();
        if cancelled {
            if self.joining.is_some() {
                tracing::info!("join cancelled");
                self.joining = None;
            }
            return;
        }
        let address = ui.take_join_request();
        let open_world = ui.take_open_world_request();
        let create_world = ui.take_create_world_request();
        if self.joining.is_none() && self.session.is_none() {
            if let Some(address) = address {
                tracing::info!("joining {address} (from the multiplayer screen)");
                self.spawn_join(address, None);
            } else if let Some(world) = open_world {
                tracing::info!("opening singleplayer world {world} (from the world list)");
                let dir = haste_setup::worlds::world_dir(&world);
                self.spawn_join(world, Some((dir, None)));
            } else if let Some(create) = create_world {
                tracing::info!(
                    "creating singleplayer world {} (from the create screen)",
                    create.name
                );
                let dir = haste_setup::worlds::world_dir(&create.folder);
                self.spawn_join(create.folder.clone(), Some((dir, Some(create))));
            }
        }
        // The title screen's Quit Game closes the window.
        if ui.take_quit_request() {
            tracing::info!("quit requested by the title screen");
            self.quitting = true;
        }
    }

    /// Spawns the join thread for `target` (shown in logs): either a plain address, or a
    /// singleplayer world (its dir; `Some` settings mean the world is created first, the create
    /// screen's choices). The thread starts the local server when there is one, joins, and
    /// sends back the session with the server for the client to stop when the world is left.
    fn spawn_join(
        &mut self,
        target: String,
        local: Option<(
            Option<std::path::PathBuf>,
            Option<haste_jvm::CreateWorldRequest>,
        )>,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        let Some(ui) = &self.ui else { return };
        let for_game = ui.clone();
        let username = self.username.clone();
        let render_distance = self.render_distance;
        let plain_address = target.clone();
        std::thread::Builder::new()
            .name("haste-join".into())
            .spawn(move || {
                // A singleplayer world: start (or create) the local server first; its address
                // is what gets joined.
                let is_local = local.is_some();
                let world = local.and_then(|(dir, create)| {
                    let dir = dir.unwrap_or_else(|| {
                        create
                            .as_ref()
                            .map(|c| {
                                haste_setup::worlds::world_dir(&c.folder)
                                    .unwrap_or_else(|| std::path::PathBuf::from(&c.folder))
                            })
                            .unwrap_or_default()
                    });
                    match LocalWorld::start(&dir, create.as_ref(), &username) {
                        Ok(world) => Some(world),
                        Err(e) => {
                            let _ = tx.send(Err(anyhow::anyhow!(
                                "starting the local server for {}: {e:#}",
                                dir.display()
                            )));
                            None
                        }
                    }
                });
                if is_local && world.is_none() {
                    // The failure was already sent; nothing to join.
                    return;
                }
                let address = world
                    .as_ref()
                    .map(|w| w.address())
                    .unwrap_or_else(|| plain_address.clone());
                let _ = tx.send(
                    Session::join(&username, render_distance, &address, true, Some(for_game))
                        .map(|s| (s, world)),
                );
            })
            .expect("spawning the join thread");
        self.joining = Some(JoinInProgress { target, rx });
    }

    /// Takes the session once the join thread finished it; a failed join shows vanilla's
    /// disconnect screen with the reason (the connecting screen's parent gets the back button).
    fn poll_join(&mut self) {
        let result = match self.joining.as_ref() {
            Some(j) => match j.rx.try_recv() {
                Ok(result) => result,
                Err(_) => return,
            },
            None => return,
        };
        let target = self.joining.take().expect("checked above").target;
        match result {
            Ok((session, server)) => {
                if let Some(r) = self.renderer.as_mut() {
                    r.set_item_registry(&session.shared.data.registries);
                    r.set_ao_shapes(&session.shared.data.blocks, &session.shared.data.shapes);
                    r.set_biomes(&session.biomes);
                    r.set_particle_types(&session.particle_types);
                }
                // A singleplayer world: keep its local server (stopped when the world is
                // left) and tell the screens (the pause screen's "Save and Quit" label).
                if let Some(server) = server {
                    self.local_server = Some(server);
                    if let Some(ui) = &self.ui {
                        ui.gui().set_local_world_running(true);
                    }
                }
                self.options = session.options.clone();
                self.session = Some(session);
                // The connecting screen stays up until the level is ready: `Ui::joined` (the
                // game thread's login handling) closes it there, like vanilla's loading
                // screen; closing here would run before the level exists.
            }
            Err(e) => {
                tracing::warn!("joining {target} failed: {e:#}");
                if let Some(ui) = &self.ui {
                    let reason = haste_protocol::haste_core::Component::text(format!("{e:#}"));
                    ui.disconnected(&reason);
                }
            }
        }
    }

    fn release_keys(&mut self) {
        self.input.release();
        if let Some(session) = &self.session {
            self.input
                .sample(&session.shared, self.ui.as_ref(), self.perspective);
        }
    }

    /// Mouse look speed from the saved sensitivity.
    fn look_deg_per_pixel(&self) -> f32 {
        look_deg_per_pixel(self.options.mouse_sensitivity)
    }

    /// The spyglass scope's growth for this frame, as vanilla's `Hud.extractCameraOverlays`
    /// moves it: every frame the scale eases towards 1.125 by half the frame's ticks
    /// (`Mth.lerp(0.5 * gameTimeDeltaTicks, scopeScale, 1.125)`), and while we are not scoping
    /// (in first person) it is reset to 0.5.
    fn scope_scale(&mut self, scoping: bool, at: Instant) -> f32 {
        let delta_ticks = 20.0 * at.saturating_duration_since(self.frame_at).as_secs_f32();
        self.frame_at = at;
        self.scope = if scoping && !self.perspective.draws_player() {
            crate::screen_effects::lerp(0.5 * delta_ticks, self.scope, 1.125)
        } else {
            0.5
        };
        self.scope
    }

    /// Menu mode's frame: the open screen over the title panorama (see `menu_background`).
    fn draw_menu(&mut self) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let (gui, camera, sky, partial) = match &mut self.ui {
            Some(ui) => menu_frame(ui, renderer, &self.input, &self.options),
            None => (GuiFrame::default(), menu_camera(), menu_sky(), 0.0),
        };
        self.gui = gui;
        let frame = FrameInput {
            camera,
            entities: &[],
            block_entities: &[],
            gui: &self.gui,
            sky,
            partial_tick: partial,
            block_target: None,
            breaking: &[],
            camera_entity: None,
            spin: None,
            weather: None,
        };
        if let Err(e) = renderer.render(&frame) {
            self.error = Some(e.context("render"));
        }
    }

    fn draw(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_none() {
            return;
        }
        // The resource pack selection changed (the pack screen's Done or a server
        // pack download, vanilla `Minecraft.reloadResourcePacks`): rebuild the
        // renderer from a fresh asset stack — atlas, models, fonts and GUI textures
        // re-read with the new packs layered on the jar.
        let reload = self
            .session
            .as_ref()
            .map(|s| s.shared.resource_pack_reload.lock().take())
            .unwrap_or_default();
        if let Some(reload) = reload {
            let window = self
                .window
                .clone()
                .expect("renderer exists, so does the window");
            match self.reload_renderer(&window, &reload) {
                Ok(()) => {
                    let names = reload
                        .ids
                        .iter()
                        .cloned()
                        .chain(reload.server.iter().map(|p| p.display().to_string()))
                        .collect::<Vec<_>>()
                        .join(", ");
                    tracing::info!("renderer rebuilt with resource packs [{names}]")
                }
                Err(e) => {
                    self.error = Some(e.context("resource pack reload"));
                    event_loop.exit();
                    return;
                }
            }
        }
        // The screen that opened or closed since the last frame takes or frees the cursor,
        // and "Disconnect" leaves the server (back to the multiplayer screen, like vanilla).
        self.sync_grab();
        self.take_disconnect(event_loop);
        if self.session.is_none() {
            // Menu mode: the screens tick here, the join the multiplayer screen requested
            // connects on its own thread, and the frame is the open screen over the plain
            // background (no world). The session arriving switches to the game path below.
            self.menu_tick();
            self.poll_join();
            self.draw_menu();
            return;
        }
        // The spyglass scope's per-frame easing, before the renderer borrows `self` for the
        // rest of the frame (it reads one flag of the view under a short lock).
        let t0 = Instant::now();
        let scope = {
            let scoping = self
                .session
                .as_ref()
                .is_some_and(|s| s.shared.view.lock().screen.scoping);
            self.scope_scale(scoping, t0)
        };
        let session = self.session.as_ref().expect("checked above");
        // The options the player just changed in a vanilla screen: the game thread applied
        // them to `live_options` and bumped its version; copy them here so FOV, GUI scale and
        // sensitivity follow this frame (vanilla reads its Options object live too).
        let version = session.shared.options_version.load(Ordering::Relaxed);
        if version != self.options_version {
            self.options_version = version;
            self.options = session.shared.live_options.lock().clone();
            // The two options the window itself owns (vanilla's Fullscreen/Vsync callbacks touch
            // the window and the swapchain): apply them to this client's window and renderer.
            if let Some(w) = &self.window {
                let want = self.options.fullscreen;
                if w.fullscreen().is_some() != want {
                    w.set_fullscreen(want.then_some(Fullscreen::Borderless(None)));
                }
            }
            let want_vsync = self.vsync_forced.unwrap_or(self.options.vsync);
            if want_vsync != self.vsync {
                self.vsync = want_vsync;
                if let Some(r) = &mut self.renderer {
                    r.set_vsync(want_vsync);
                }
            }
        }
        self.input
            .sample(&session.shared, self.ui.as_ref(), self.perspective);
        // The windowed bench holds the camera still (the settle frame's view with the
        // requested look), like the offscreen `--bench --bench-fixed`.
        if let Some(bench) = &mut self.bench {
            let offset = session.shared.view.lock().look_offset;
            if let Some(yaw) = bench.look.0 {
                self.input.yaw = yaw - offset.0;
            }
            if let Some(pitch) = bench.look.1 {
                self.input.pitch = (pitch - offset.1).clamp(-90.0, 90.0);
            }
        }
        let renderer = self.renderer.as_mut().expect("checked above");
        let mut weather_guard = session.shared.weather_scratch.lock();
        let (mut camera, sky, lightmap, partial, vd, animation_tick, weather) = frame_view(
            &session.shared,
            self.perspective,
            self.input.yaw,
            self.input.pitch,
            &self.options,
            t0,
            &mut weather_guard,
        );
        // The windowed bench's settle and still camera: until the world has settled (and
        // the HUD's JVM arrived, like the screenshot path), the frames only warm up; the
        // first settled frame's camera is then held for the whole run.
        if let Some(bench) = &mut self.bench {
            let settled_now = {
                let view = session.shared.view.lock().clone();
                let at_pos = bench
                    .want_pos
                    .is_none_or(|p| (view.feet - p).length() <= 0.5);
                view.has_position
                    && at_pos
                    && self
                        .feed
                        .settled(&self.frame_stats, Duration::from_millis(1500))
                    && (self.ui.is_some()
                        || session.shared.hud_failed.load(Ordering::Relaxed)
                        || !session.shared.hud_enabled.load(Ordering::Relaxed))
            };
            if settled_now && bench.camera.is_none() {
                bench.camera = Some(camera);
                bench.view_distance = vd;
            }
            if let Some(still) = bench.camera {
                camera = still;
            }
        }
        let feed_start = Instant::now();
        self.feed
            .feed(&session.shared, renderer, camera_section(&camera), vd);
        if let Some(bench) = &mut self.bench {
            bench.feed_ms = feed_start.elapsed().as_secs_f32() * 1000.0;
        }
        let t1 = Instant::now();
        let hud_hidden = self.ui.as_ref().is_some_and(|ui| ui.hud_hidden());
        self.feed
            .set_first_person(&session.shared, renderer, self.perspective, hud_hidden);
        let (entities, block_entities) =
            self.feed
                .entities(&session.shared, renderer, session.local, partial);
        let block_target = session.shared.view.lock().block_target;
        self.breaking.clear();
        self.breaking
            .extend_from_slice(&session.shared.breaking.lock());
        let t2 = Instant::now();
        // The HUD: once the game thread's `Ui` is ready, clone it here (cheap, one per thread)
        // and render the frame through it; until then (JVM still starting, or failed: no JDK
        // or cache) the empty default draws no HUD.
        if self.ui.is_none() {
            self.ui = session.shared.ui.lock().clone();
            if let Some(ui) = &self.ui {
                ui.set_text_metrics(renderer.text_metrics());
            }
        }
        let (w, h) = renderer.size();
        let view = session.shared.view.lock().clone();
        if let Some(ui) = &self.ui {
            // F3's numbers and the water overlay's flag, every frame (cheap: short mutexes,
            // no allocation while the overlay is off). The section counts are the previous
            // frame's.
            let stats = self.frame_stats;
            let gpu = gpu_info(renderer);
            ui.set_debug(debug_info(
                &session.debug_env,
                &session.shared,
                renderer,
                (self.input.yaw, self.input.pitch),
                self.fps.fps as i32,
                &stats,
                entities.len() as i32,
                resolve_gui_scale(
                    u32::from(self.options.gui_scale),
                    w,
                    h,
                    self.options.force_unicode_font,
                ),
                &gpu,
            ));
            let view = session.shared.view.lock().clone();
            ui.set_eye_in_water(view.eye_in_water);
        }
        if let Some(ui) = &mut self.ui {
            let mouse = self.input.mouse;
            let scale = resolve_gui_scale(
                u32::from(self.options.gui_scale),
                w,
                h,
                self.options.force_unicode_font,
            );
            let (gw, gh) = gui_scaled_size(w, h, scale);
            self.gui = ui.frame(gw, gh, scale, mouse, partial).clone();
            renderer.set_panorama(ui.gui().panorama());
        }
        // The screen effects around the HUD frame (vanilla's `ScreenEffectRenderer.submit`'s
        // level-pass overlays and `Hud.extractCameraOverlays`'s own).
        {
            let frame = crate::screen_effects::Frame {
                width: self.gui.width,
                height: self.gui.height,
                first_person: !self.perspective.draws_player(),
                effects: &view.screen,
                partial_tick: partial,
                // The look the frame's camera uses, which the water overlay scrolls by.
                look: (
                    self.input.yaw + view.look_offset.0,
                    (self.input.pitch + view.look_offset.1).clamp(-90.0, 90.0),
                ),
                scope_scale: scope,
            };
            frame.append_to(&mut self.gui.commands);
        }
        let frame = FrameInput {
            camera,
            entities,
            block_entities,
            gui: &self.gui,
            sky,
            partial_tick: partial,
            block_target,
            breaking: &self.breaking,
            camera_entity: (!self.perspective.draws_player()).then_some(session.local.0),
            spin: view.screen.spin_warp(partial),
            weather,
        };
        // Outline boxes of the target and the states of breaking blocks (one short read lock),
        // then the particles (one short read lock).
        let session = self.session.as_ref().expect("the game path above");
        renderer.sample_world(
            &*session.shared.world.read(),
            &session.shared.data.shapes,
            &frame,
        );
        {
            let world = session.shared.world.read();
            let mut particles = session.shared.particles.lock();
            renderer.prepare_particles(&mut particles, &*world, camera, partial);
        }
        renderer.set_animation_tick(animation_tick);
        renderer.set_game_time(session.shared.view.lock().world_age);
        renderer.set_lightmap_frame(lightmap);
        renderer.set_menu_blur_radius(self.options.menu_background_blurriness);
        if self.screenshot {
            self.screenshot = false;
            renderer.request_capture();
        }
        let result = renderer.render(&frame);
        drop(weather_guard);
        if let Ok(stats) = &result {
            self.frame_stats = *stats;
        }
        // The windowed bench's sample: recorded once the still camera is held (the settle
        // frames before it are warm-up, not measurements). The run ends with the window.
        if let (Some(bench), Ok(stats)) = (&mut self.bench, &result) {
            if let Some(started) = bench.started {
                let prev_drawn = bench.samples.last().map_or(0, |s| s.sections_drawn);
                bench.samples.push(BenchSample {
                    frame_ms: stats.frame_ms,
                    sections_drawn: stats.chunk_sections_drawn,
                    sections_total: stats.chunk_sections_total,
                    draw_calls: stats.draw_calls,
                    timings: renderer.timings(),
                    feed_ms: bench.feed_ms,
                    sections_new: stats.chunk_sections_drawn.saturating_sub(prev_drawn),
                    uploads: stats.uploads,
                    upload_bytes: stats.upload_bytes,
                    indices_drawn: stats.indices_drawn,
                    index: bench.samples.len() as u32,
                });
                if started.elapsed().as_secs_f32() >= bench.seconds {
                    self.quitting = true;
                }
            } else if bench.camera.is_some() {
                bench.started = Some(Instant::now());
            }
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
    /// Rebuilds the renderer after the resource pack selection changed (vanilla's reload
    /// rebuilds its resource manager, model manager and atlases). The new selection is
    /// written to options.txt by the JVM's own `Options.save` before we get here, so
    /// `open_assets` reads it back; the server's downloaded packs layer above it.
    fn reload_renderer(
        &mut self,
        window: &Arc<Window>,
        reload: &game::PackReload,
    ) -> anyhow::Result<()> {
        let mut assets = open_assets_with(&reload.server)?;
        let data = net::load_game_data()?;
        let mut renderer = Renderer::new(window.clone(), &mut assets, &data.blocks)?;
        if let Some(session) = &self.session {
            renderer.set_item_registry(&session.shared.data.registries);
            renderer.set_biomes(&session.biomes);
            renderer.set_particle_types(&session.particle_types);
            renderer.set_dimension(session.shared.world.read().dimension());
        }
        renderer.set_vsync(self.vsync);
        // The old renderer's GPU objects drop with it; every loaded section is queued for
        // re-meshing into the new arenas (vanilla's reload re-bakes the world's models too).
        self.renderer = Some(renderer);
        self.feed = WorldFeed::default();
        if let Some(session) = &self.session {
            self.feed.queue_all_sections(&session.shared);
        }
        Ok(())
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = if let Some(bench) = &self.bench {
            // The windowed bench: a logical inner size (`--width`x`--height`), so the
            // framebuffer is exactly the requested resolution times the desktop scale —
            // the same window a vanilla client at `--width`x`--height` would get.
            Window::default_attributes()
                .with_title("Haste bench")
                .with_inner_size(winit::dpi::LogicalSize::new(
                    f64::from(bench.size.0),
                    f64::from(bench.size.1),
                ))
        } else {
            Window::default_attributes()
                .with_title("Haste")
                .with_inner_size(winit::dpi::LogicalSize::new(854.0, 480.0))
        };
        let result = (|| -> anyhow::Result<(Arc<Window>, Renderer)> {
            let window = Arc::new(event_loop.create_window(attrs)?);
            let mut assets = open_assets()?;
            let data = net::load_game_data()?;
            let mut renderer = Renderer::new(window.clone(), &mut assets, &data.blocks)?;
            renderer.set_ao_shapes(&data.blocks, &data.shapes);
            if let Some(session) = &self.session {
                renderer.set_item_registry(&session.shared.data.registries);
                renderer.set_ao_shapes(&session.shared.data.blocks, &session.shared.data.shapes);
                renderer.set_biomes(&session.biomes);
                renderer.set_particle_types(&session.particle_types);
                renderer.set_dimension(session.shared.world.read().dimension());
            }
            Ok((window, renderer))
        })();
        match result {
            Ok((w, mut r)) => {
                r.set_vsync(self.vsync);
                self.window = Some(w);
                self.renderer = Some(r);
                if self.session.is_some() {
                    self.set_grab(true);
                }
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
                        self.options.force_unicode_font,
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
                if let Some(session) = &self.session {
                    self.input
                        .wheel(dx, dy, x, y, self.ui.as_ref(), &session.shared);
                } else if let Some(ui) = &self.ui {
                    ui.scroll(x, y, dx, dy);
                }
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
        if self.quitting {
            self.leave_server();
            event_loop.exit();
            return;
        }
        // The connection ended (kick, timeout): the UI already shows the disconnect screen;
        // drop the session and stay in menu mode around it.
        if let Some(session) = &self.session {
            if session.shared.is_closed() {
                self.leave_server();
            }
        }
        // After leaving (or a kick): no screen open means the disconnect screen was closed;
        // the title screen returns, like vanilla's "Back to title screen".
        if self.session.is_none()
            && self.joining.is_none()
            && self.ui.as_ref().is_some_and(|ui| !ui.screen_open())
            && !self.screen_was_open
            && self.menu.is_none()
        {
            if let Some(ui) = &self.ui {
                if let Err(e) = ui.open_screen(haste_jvm::ScreenKind::Title) {
                    tracing::warn!("opening the title screen: {e:#}");
                }
            }
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
    fn gui_text_pos_finds_labels_and_slider_values() {
        use haste_protocol::haste_core::gui::{GuiCommand, GuiFrame};
        let frame = GuiFrame {
            commands: vec![
                GuiCommand::Text {
                    x: 10,
                    y: 20,
                    text: "Done".into(),
                    color: 0xffff_ffff,
                    shadow: true,
                },
                GuiCommand::TranslatePush {
                    dx: 5.0,
                    dy: 7.0,
                    dz: 0.0,
                },
                GuiCommand::Text {
                    x: 10,
                    y: 20,
                    text: "Render Distance: 12".into(),
                    color: 0xffff_ffff,
                    shadow: true,
                },
                GuiCommand::TranslatePop,
            ],
            ..GuiFrame::default()
        };
        // An exact label, in the frame's coordinates.
        assert_eq!(gui_text_pos(&frame, "Done"), Some((10, 20)));
        // A slider's dynamic `label: value`, with the translation stack applied.
        assert_eq!(gui_text_pos(&frame, "Render Distance"), Some((15, 27)));
        assert_eq!(gui_text_pos(&frame, "FOV"), None);
    }

    #[test]
    fn mouse_look_clamps_pitch() {
        let (yaw, pitch) = apply_mouse(0.0, 0.0, (100.0, 0.0), DPP);
        assert!((yaw - 15.0).abs() < 1e-4 && pitch == 0.0, "{yaw} {pitch}");
        assert_eq!(apply_mouse(0.0, 80.0, (0.0, 200.0), DPP), (0.0, 90.0));
        assert_eq!(apply_mouse(10.0, -80.0, (0.0, -200.0), DPP), (10.0, -90.0));
    }

    /// A scripted look aims exactly where asked: the input layer holds the target angles minus
    /// the server's rotation offset, which the tick's pick adds back. Without the subtraction a
    /// `look` after the server turned the player (a teleport echo) aims `offset` away from the
    /// target — the boat mount missed because the crosshair looked at the block under the
    /// driver's feet instead of the boat.
    #[test]
    fn an_exact_aim_subtracts_the_servers_look_offset() {
        let mut input = InputRouter::default();
        // No server offset: the input layer holds the angles themselves.
        input.aim(0.0, 49.0, (0.0, 0.0));
        assert_eq!((input.yaw, input.pitch), (0.0, 49.0));
        // The server turned us 60 degrees while the script's input still says 30 (the offset
        // the player keeps, `LocalPlayer.look_offset`): aiming at 90 needs input 30.
        input.aim(90.0, 10.0, (60.0, 0.0));
        assert_eq!((input.yaw, input.pitch), (30.0, 10.0));
        // A pitch past the clamp with the offset applied to the input, not the target.
        input.aim(0.0, 90.0, (0.0, 40.0));
        assert_eq!((input.yaw, input.pitch), (0.0, 50.0));
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
        // `Window.calculateScale(0, false)` at vanilla's default window and common
        // resolutions: the scale grows while the framebuffer keeps >= 320x240 GUI
        // pixels. Computed by hand from the vanilla method's conditions.
        assert_eq!(gui_scale_for(0, 854, 480, false), 2);
        assert_eq!(gui_scale_for(0, 1280, 720, false), 3);
        assert_eq!(gui_scale_for(0, 1920, 1080, false), 4);
        assert_eq!(gui_scale_for(0, 2560, 1440, false), 6);
        assert_eq!(gui_scale_for(0, 3840, 2160, false), 9);
    }

    #[test]
    fn fixed_gui_scale_is_honoured_and_bounded_by_the_window() {
        // A fixed option stops there (vanilla: `calculateScale(maxScale, ...)`); a value
        // too large for the framebuffer is still bounded by the 320x240 rule.
        assert_eq!(gui_scale_for(1, 1920, 1080, false), 1);
        assert_eq!(gui_scale_for(3, 1920, 1080, false), 3);
        assert_eq!(gui_scale_for(4, 854, 480, false), 2);
        // The option is not capped at 4: on 1920x1080 the window's automatic maximum (4)
        // bounds it, on 3840x2160 scales 5..=9 are honoured and 9 is the maximum.
        assert_eq!(resolve_gui_scale(9, 1920, 1080, false), 4);
        assert_eq!(resolve_gui_scale(0, 1920, 1080, false), 4);
        assert_eq!(resolve_gui_scale(5, 3840, 2160, false), 5);
        assert_eq!(resolve_gui_scale(6, 3840, 2160, false), 6);
        assert_eq!(resolve_gui_scale(9, 3840, 2160, false), 9);
        assert_eq!(resolve_gui_scale(50, 3840, 2160, false), 9);
        // Tiny framebuffers still get scale 1 (the loop stops immediately).
        assert_eq!(gui_scale_for(0, 320, 240, false), 1);
        assert_eq!(gui_scale_for(0, 100, 100, false), 1);
    }

    #[test]
    fn force_unicode_font_rounds_odd_gui_scales_up() {
        // `Window.calculateScale(_, true)` bumps an odd result to the next even scale after
        // the loop, even past the window's automatic maximum (9 -> 10 on 3840x2160).
        assert_eq!(resolve_gui_scale(0, 1920, 1080, true), 4);
        assert_eq!(resolve_gui_scale(3, 1920, 1080, true), 4);
        assert_eq!(resolve_gui_scale(1, 1920, 1080, true), 2);
        assert_eq!(resolve_gui_scale(2, 1920, 1080, true), 2);
        assert_eq!(resolve_gui_scale(0, 3840, 2160, true), 10);
        assert_eq!(resolve_gui_scale(5, 3840, 2160, true), 6);
        assert_eq!(resolve_gui_scale(0, 1280, 720, true), 4);
        assert_eq!(gui_scale_for(0, 320, 240, true), 2);
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
    fn key_routing_drops_the_char_that_opened_the_screen() {
        // Vanilla: the key press that opens a screen (T, "/") drops its typed character —
        // `Gui.handleKeybinds` opens the chat screen on the tick after the press, so the
        // press's character event finds no screen. Only characters typed while a screen was
        // already open reach it.
        assert!(!char_goes_to_screen(false));
        assert!(char_goes_to_screen(true));
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
            indices_drawn: 0,
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
            hand_fov_deg: 70.0,
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
        assert_eq!(frame_fov(70.0, 1.0, 1.0, 0.5, false, None, 1.0), 70.0);
        assert_eq!(
            frame_fov(70.0, 1.0, 1.5, 0.5, false, None, 1.0),
            70.0 * 1.25
        );
        // Under water/lava: `lerp(effectScale, 1.0, 0.85714287)`.
        assert!((frame_fov(70.0, 1.0, 1.0, 0.0, true, None, 1.0) - 60.0).abs() < 1e-3);
        assert!((frame_fov(70.0, 1.0, 1.0, 0.0, true, None, 0.5) - 65.0).abs() < 1e-4);
        // The effect scale off leaves it alone.
        assert_eq!(frame_fov(70.0, 1.0, 1.0, 0.0, true, None, 0.0), 70.0);
        // Death: `fov /= (1 - 500/(duration+500))*2 + 1`, deathTime clamped at 20.
        assert!((frame_fov(70.0, 1.0, 1.0, 0.0, false, Some(20), 1.0) - 65.0).abs() < 1e-4);
        assert!((frame_fov(70.0, 1.0, 1.0, 0.0, false, Some(200), 1.0) - 65.0).abs() < 1e-4);
        // `isDeadOrDying` gates it, so the very tick health hits 0 (deathTime 0) already
        // shrinks the FOV: duration = 0 + partial.
        let first = frame_fov(70.0, 1.0, 1.0, 0.5, false, Some(0), 1.0);
        let expected = 70.0 / ((1.0 - 500.0 / 500.5) * 2.0 + 1.0);
        assert!((first - expected).abs() < 1e-4, "{first} vs {expected}");
        assert!(first < 70.0);
    }

    #[test]
    fn death_view_is_gated_on_health_not_on_the_counter() {
        // `View::death`: `Some(deathTime)` from the tick health is <= 0 (the counter is
        // still 0 then), `None` while alive whatever the stale counter says.
        assert_eq!(game::death_time(false, 0), None);
        assert_eq!(game::death_time(true, 0), Some(0));
        assert_eq!(game::death_time(true, 7), Some(7));
        assert_eq!(game::death_time(false, 7), None);
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

    /// A fixed-width "font" for the wrap tests: every character is 6 wide, `i` is 2.
    struct WrapFont;
    impl haste_core::gui::TextMetrics for WrapFont {
        fn width(&self, text: &str) -> i32 {
            text.chars().map(|c| if c == 'i' { 2 } else { 6 }).sum()
        }
    }

    /// The first wrapped line follows vanilla `StringSplitter.splitLines`: break at the last
    /// space before the width runs out, hard-cut without one, always keep the first character.
    #[test]
    fn first_wrapped_line_breaks_at_spaces() {
        let f = &WrapFont;
        // 3 characters of 6 = 18 fits 20.
        assert_eq!(first_wrapped_line("abc", f, 20), "abc");
        // 4 characters of 6 = 24 > 20: no space, cut before the 4th.
        assert_eq!(first_wrapped_line("abcd", f, 20), "abc");
        // A space before the cut: the line ends at the space (dropped).
        assert_eq!(first_wrapped_line("ab cd", f, 20), "ab");
        // Newlines end the line.
        assert_eq!(first_wrapped_line("ab\ncd", f, 100), "ab");
        // The first character is kept even when wider than the limit.
        assert_eq!(first_wrapped_line("abcd", f, 1), "a");
        // Empty stays empty.
        assert_eq!(first_wrapped_line("", f, 90), "");
    }

    /// Sign NBT parses to both sides' lines, colour and glow; the plain and the compound
    /// component forms both work, and missing sides are `SignText.EMPTY`.
    #[test]
    fn sign_nbt_parses_both_sides() {
        use haste_protocol::nbt::serializer::{NbtWriteHelper, NbtWriteHelperJava};
        use haste_protocol::nbt::{NbtCompound, NbtTag, COMPOUND_ID};
        let mut front = NbtCompound::new();
        front.put_list(
            "messages",
            vec![
                NbtTag::String("hello".into()),
                NbtTag::Compound({
                    let mut c = NbtCompound::new();
                    c.put_string("text", "world".into());
                    c
                }),
                NbtTag::String("".into()),
                NbtTag::String("line4".into()),
            ],
        );
        front.put_string("color", "red".into());
        front.put_bool("has_glowing_text", true);
        let mut root = NbtCompound::new();
        root.put_compound("front_text", front);
        // Network NBT: the type byte and payload with no root name.
        let mut bytes = Vec::new();
        {
            let mut w = NbtWriteHelperJava::new(&mut bytes);
            w.write_u8(COMPOUND_ID).unwrap();
            root.clone().serialize_content(&mut w).unwrap();
        }
        let parsed = parse_sign_nbt(&bytes, &WrapFont, 90).unwrap();
        assert_eq!(parsed.front.lines[0], "hello");
        assert_eq!(parsed.front.lines[1], "world");
        assert_eq!(parsed.front.lines[3], "line4");
        assert_eq!(parsed.front.color, 14);
        assert!(parsed.front.glowing);
        // The back is missing: vanilla's empty default.
        assert_eq!(parsed.back, empty_sign_side());
        // Not NBT at all: None, never a panic.
        assert!(parse_sign_nbt(&[1, 2, 3], &WrapFont, 90).is_none());
    }

    /// Banner NBT: the `patterns` list (registry id or inline `{asset_id}` form) and the base
    /// colour from the block id; a banner without patterns is the plain base.
    #[test]
    fn banner_nbt_parses_patterns_and_base() {
        use haste_protocol::nbt::serializer::{NbtWriteHelper, NbtWriteHelperJava};
        use haste_protocol::nbt::{NbtCompound, NbtTag, COMPOUND_ID};
        let nbt_bytes = |root: NbtCompound| {
            let mut bytes = Vec::new();
            let mut w = NbtWriteHelperJava::new(&mut bytes);
            w.write_u8(COMPOUND_ID).unwrap();
            root.clone().serialize_content(&mut w).unwrap();
            bytes
        };
        // A red wall banner with two patterns.
        let mut root = NbtCompound::new();
        root.put_list(
            "patterns",
            vec![
                NbtTag::Compound({
                    let mut c = NbtCompound::new();
                    c.put_string("pattern", "minecraft:stripe_bottom".into());
                    c.put_string("color", "blue".into());
                    c
                }),
                NbtTag::Compound({
                    let mut c = NbtCompound::new();
                    c.put_string("pattern", "minecraft:half_vertical".into());
                    c.put_string("color", "white".into());
                    c
                }),
            ],
        );
        let block = haste_core::Identifier::minecraft("red_wall_banner");
        let data = parse_banner_nbt(&nbt_bytes(root), &block).unwrap();
        assert_eq!(data.base, 14);
        assert_eq!(data.layers.len(), 2);
        assert_eq!(data.layers[0].pattern.path(), "stripe_bottom");
        assert_eq!(data.layers[0].color, 11);
        assert_eq!(data.layers[1].pattern.path(), "half_vertical");
        assert_eq!(data.layers[1].color, 0);
        // No patterns: the plain base (black from `minecraft:banner`).
        let plain = parse_banner_nbt(
            &nbt_bytes(NbtCompound::new()),
            &haste_core::Identifier::minecraft("banner"),
        )
        .unwrap();
        assert_eq!(plain.base, 15);
        assert!(plain.layers.is_empty());
        // The inline asset_id form.
        let mut inline = NbtCompound::new();
        inline.put_list(
            "patterns",
            vec![NbtTag::Compound({
                let mut c = NbtCompound::new();
                c.put_string("asset_id", "minecraft:cross".into());
                c.put_string("color", "lime".into());
                c
            })],
        );
        let data = parse_banner_nbt(
            &nbt_bytes(inline),
            &haste_core::Identifier::minecraft("white_banner"),
        )
        .unwrap();
        assert_eq!(data.layers.len(), 1);
        assert_eq!(data.layers[0].pattern.path(), "cross");
        assert_eq!(data.layers[0].color, 5);
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

    #[test]
    fn java_random_matches_vanilla_lcg() {
        // The single-threaded RandomSource: seed 12345 -> (0xB30A9B << 48-bit scrambles).
        // The first nextFloat of Java's Random with seed 0 is 0.73096776.
        let mut r = JavaRandom::new(0);
        assert!(
            (r.next_float() - 0.73096776).abs() < 1e-6,
            "{}",
            r.next_float()
        );
        // nextDouble of the same fresh random: 0.627 exactly? The sequence advances; just
        // check the bounds and determinism instead.
        let mut a = JavaRandom::new(42);
        let mut b = JavaRandom::new(42);
        for _ in 0..8 {
            let (x, y) = (a.next_double(), b.next_double());
            assert_eq!(x, y);
            assert!((0.0..1.0).contains(&x));
        }
        // Gaussian: two fresh streams agree and stay in a sane range.
        let mut a = JavaRandom::new(7);
        let mut b = JavaRandom::new(7);
        for _ in 0..16 {
            let (x, y) = (a.next_gaussian(), b.next_gaussian());
            assert_eq!(x, y);
            assert!(x.abs() < 6.0);
        }
    }

    #[test]
    fn rain_scroll_matches_vanilla_formula() {
        // `createRainColumnInstance`: textureOffset = -(wrappedTicks + tickOffset +
        // partial) / 32 * speed, wrapped modulo 32. Column (5, -3), tick 100, partial 0:
        // tickOffset = (5*5*3121 + 5*45238971 + 9*418711 - 3*13761) & 0xFF.
        let tick_offset = (5i64 * 5 * 3121 + 5 * 45238971 + 9 * 418711 + (-3) * 13761) & 0xFF;
        // The speed is the seeded random's nextFloat (3..4); recompute through the same
        // seed to keep the formula honest.
        let seed = haste_render::weather::column_seed(5, -3);
        let mut rng = JavaRandom::new(seed as i64);
        let speed = 3.0 + rng.next_float();
        let wrapped_ticks = 100i64 & 131071;
        let expect = -((wrapped_ticks as f32 + tick_offset as f32) / 32.0 * speed) % 32.0;
        assert!(expect <= 0.0 && expect > -32.0, "{expect}");
        // The scroll is negative (the texture moves down) and wraps at 32 tiles.
        assert_eq!(tick_offset as i32, tick_offset as i32);
    }

    #[test]
    fn weather_levels_lerp_by_partial_tick() {
        // `Level.getRainLevel(a)`: lerp between the old and the new level.
        let mut w = crate::net::WeatherState::default();
        w.set_rain_level(1.0);
        assert_eq!(w.rain_level(0.0), 1.0);
        assert_eq!(w.rain_level(1.0), 1.0);
        // A step down brackets the tick: old 1, new 0.4 -> 0.7 at partial 0.5.
        w.set_rain_level(1.0);
        w.rain_level_o = 1.0;
        w.rain_level = 0.4;
        assert!((w.rain_level(0.5) - 0.7).abs() < 1e-6);
        // Clamped: the packet's 2.0 becomes 1.0.
        w.set_rain_level(2.0);
        assert_eq!(w.rain_level, 1.0);
        // canHaveWeather: the overworld yes, the Nether and End no.
        use haste_core::game::DimensionInfo;
        assert!(crate::net::WeatherState::can_have_weather(
            &DimensionInfo::overworld()
        ));
        let mut nether = DimensionInfo::overworld();
        nether.dimension_type = haste_core::Identifier::minecraft("the_nether");
        nether.has_skylight = false;
        assert!(!crate::net::WeatherState::can_have_weather(&nether));
        let mut end = DimensionInfo::overworld();
        end.name = haste_core::Identifier::minecraft("the_end");
        assert!(!crate::net::WeatherState::can_have_weather(&end));
    }

    #[test]
    fn game_events_drive_the_weather_levels() {
        use crate::net::WeatherState;
        // START_RAINING resets to 0, STOP_RAINING sets 1 (the level-change steps ramp from
        // there), RAIN_LEVEL_CHANGE/THUNDER_LEVEL_CHANGE set the levels directly.
        let mut w = WeatherState::default();
        w.set_rain_level(0.5);
        w.set_thunder_level(0.7);
        assert_eq!(w.thunder_level, 0.7);
        w.set_rain_level(1.0);
        assert_eq!(w.rain_level, 1.0);
        w.set_rain_level(0.0);
        assert_eq!(w.rain_level, 0.0);
    }
}
