// The Renderer (docs/INTERFACES.md): wgpu device, atlas upload, section
// submission, camera/frustum, per-frame render with sky colour and fog.
// Offscreen construction + readback for oracles's screenshot tests.
// Shading design from Pomme (chunk shaders); see THIRD_PARTY.md.
//
// Split by responsibility like vanilla's LevelRenderer/GameRenderer (the
// per-frame hot path in `render_world.rs`, construction in `init.rs`, the
// surface/window plumbing in `surface.rs`, camera math in `camera.rs`, the
// world feed in `terrain_feed.rs`, per-frame state in `frame_setup.rs`,
// readback in `readback.rs`, queries in `query.rs`, the GUI pass in `gui_pass.rs`).

use std::sync::Arc;

use haste_core::{ChunkPos, SectionPos};
use parking_lot::Mutex;

use crate::arena::MeshArenas;
use crate::mesher::{Mesher, SectionMesh};
use crate::pipeline::Gpu;

pub(crate) use readback::Capture;
pub(crate) use surface::Offscreen;

mod camera;
mod draw_list;
mod frame_setup;
mod gui_pass;
mod init;
mod mesh_gate;
mod premesh;
mod quad_draw;
mod quad_pipelines;
mod query;
mod readback;
mod render_world;
mod surface;
mod terrain_feed;
mod work_budget;

pub use premesh::PremeshMode;
pub use terrain_feed::SectionLoadState;
pub use work_budget::WorkBudgetMode;
#[cfg(test)]
mod tests;
mod textures;
mod visible_sections;

/// Camera for one frame (docs/INTERFACES.md FrameInput).
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub pos: glam::DVec3,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_deg: f32,
    /// The first-person hand's own FOV (`Camera.calculateHudFov`: the fixed 70 base with
    /// only the death and fluid modifiers, never the FOV option or its modifier), in degrees.
    pub hand_fov_deg: f32,
    /// Camera roll in degrees (a plain horizon roll around the view forward axis).
    pub roll_deg: f32,
    /// The hurt/death tilts and the view bobbing (vanilla's `GameRenderer.bobHurt`/`bobView`),
    /// applied in view space between the projection and the view.
    pub bob: ViewBob,
}

/// Per-frame view-space camera effects: the pose stack vanilla's `GameRenderer` multiplies
/// onto the projection matrix (hurt tilt, death roll, then the view-bob translation and
/// wobble), plus the camera's plain [`Camera::roll_deg`] roll.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewBob {
    /// `bobHurt`: the hurt roll in degrees, applied around the view z-axis rotated
    /// `hurt_dir_deg` toward the damage direction (vanilla's `hurtDir`).
    pub hurt_tilt_deg: f32,
    /// `bobHurt`: the damage direction in degrees.
    pub hurt_dir_deg: f32,
    /// `bobHurt`: the death roll in degrees, a plain z-axis rotation.
    pub death_roll_deg: f32,
    /// `bobView`: the translation in the camera plane (x right, y up).
    pub x: f32,
    pub y: f32,
    /// `bobView`: the roll and pitch wobble in degrees.
    pub roll_deg: f32,
    pub pitch_deg: f32,
}

/// Sky/fog state for one frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct SkyState {
    pub time_of_day: i64,
    /// World age in ticks (vanilla `Level.getGameTime`): the cloud drift advances with it
    /// even when the time of day is frozen.
    pub game_time: i64,
    pub rain: f32,
    pub thunder: f32,
    pub dimension_has_sky: bool,
    pub fog_start: f32,
    pub fog_end: f32,
    /// The dimension's `visual/cloud_height` attribute (blocks; 192.33 in the overworld).
    pub cloud_height: f32,
    /// Whether the dark disc goes under the horizon this frame (vanilla
    /// `SkyRenderer.shouldRenderDarkDisc`, see `sky::dark_disc`).
    pub dark_disc: bool,
    /// The fog environment inputs (`FogRenderer.setupFog`/`computeFogColor`): what the camera
    /// is in, the player's effects, the dimension/biome attributes and the boss-overlay values.
    pub fog: crate::fog::FogInputs,
}

/// Vanilla's nausea/portal screen warp (`GameRenderer.renderLevel`): while the player spins
/// (nausea or a portal), the level's projection matrix is rotated about (0, √2/2, √2/2) by the
/// spinning angle and squeezed in x by `1 / skew`, which is what makes the world wobble.
/// The hand and the screen effects keep the plain projection, as in vanilla (`render3dHud`
/// sets the HUD projection before the screen effects are submitted).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpinWarp {
    /// `LocalPlayer.getSpinningEffectAngle`, in degrees.
    pub angle_deg: f32,
    /// `max(portalEffectIntensity, nauseaEffectIntensity) * screenEffectScale²`.
    pub intensity: f32,
}

impl SpinWarp {
    /// Vanilla's `5.0 / (I² + 5.0) - 0.04 * I`, squared: the squeeze factor.
    pub fn skew(&self) -> f32 {
        let skew = 5.0 / (self.intensity * self.intensity + 5.0) - self.intensity * 0.04;
        skew * skew
    }
}

/// The lightmap inputs that change every frame and are not part of the sky or the dimension:
/// the block-light flicker (`LightmapRenderStateExtractor.blockLightFlicker`), the brightness
/// option, the night-vision intensity, the darkness-effect scale and the boss-overlay world
/// darkening. `lightning_flash` forces the sky factor to 1.0 (`ClientLevel`'s
/// `SKY_LIGHT_FACTOR` layer while `skyFlashTime > 0`). The renderer folds these into the
/// dimension's [`crate::lightmap::LightmapInputs`] once per frame; the client sets them with
/// [`Renderer::set_lightmap_frame`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LightmapFrame {
    pub block_light_flicker: f32,
    pub gamma: f32,
    pub night_vision: f32,
    pub darkness: f32,
    pub boss_overlay_darkening: f32,
    pub lightning_flash: bool,
}

/// CPU-side memory the renderer keeps beside its GPU buffers (bench memory line).
#[derive(Clone, Copy, Debug, Default)]
pub struct RendererCpuMemory {
    /// Block atlas mip chain + animated sprite frames (CPU copies, byte-for-byte
    /// what was uploaded to the GPU, kept for animated region updates).
    pub atlas_bytes: u64,
    /// Baked block models (per-state quads).
    pub baked_bytes: u64,
    /// Mesher queue length (sections waiting to be meshed).
    pub mesh_pending: u64,
}

/// Per-frame render statistics (docs/INTERFACES.md FrameStats).
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    pub frame_ms: f32,
    pub chunk_sections_drawn: u32,
    pub chunk_sections_total: u32,
    pub meshes_pending: u32,
    /// Sections this frame's gate wants that have no mesh yet (pop-in; 0 unless the
    /// bench turned tracking on with `set_popin_tracking`).
    pub visible_unmeshed: u32,
    pub draw_calls: u32,
    /// Block entities drawn this frame (chests, shulker boxes; one batched draw with entities).
    pub block_entities_drawn: u32,
    /// Section meshes uploaded this frame (one per finished mesh written to the arenas).
    pub uploads: u32,
    /// Vertex+index bytes written by this frame's uploads.
    pub upload_bytes: u64,
    /// The upload budget this frame's feed worked to, in ms (`--work-budget`).
    pub upload_budget_ms: f32,
    /// Terrain indices drawn this frame (six per quad): the vertex-fetch load.
    pub indices_drawn: u32,
}

/// One frame's draw input.
pub struct FrameInput<'a> {
    pub camera: Camera,
    pub entities: &'a [haste_core::model::EntityInstance],
    /// Block entities to draw (chests, shulker boxes; see `blockentity`).
    pub block_entities: &'a [crate::blockentity::BlockEntityInstance],
    pub gui: &'a haste_core::gui::GuiFrame,
    pub sky: SkyState,
    pub partial_tick: f32,
    /// Block under the crosshair and the face hit: drawn as the outline of its outline shape
    /// (sampled with `Renderer::sample_world`).
    pub block_target: Option<(haste_core::BlockPos, haste_core::Direction)>,
    /// Crack overlays: our own break progress plus other players' stages (0..=9).
    pub breaking: &'a [(haste_core::BlockPos, u8)],
    /// The entity the camera is, when the world pass must not draw it (vanilla
    /// `LevelExtractor.extractVisibleEntities` skips the camera entity unless the camera is
    /// detached). `None` in third person, where the player model is drawn; the GUI pass still
    /// finds the instance in `entities` (the inventory's player preview).
    pub camera_entity: Option<haste_core::EntityId>,
    /// The nausea/portal screen warp for this frame (`None`: no warp).
    pub spin: Option<SpinWarp>,
    /// The extracted weather columns (`WeatherEffectRenderer.extractRenderState`), `None`
    /// when it does not rain (the pass is skipped entirely). Borrowed for one frame.
    pub weather: Option<&'a crate::weather::WeatherFrame>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    cam_pos: [f32; 4],
    view_proj: [[f32; 4]; 4],
    fog_color: [f32; 4],
    /// Fog bands (vanilla FogData): environmental (spherical) and render distance
    /// (cylindrical) start/end, sky fog end, cloud fog end.
    fog_env_start: f32,
    fog_env_end: f32,
    fog_rd_start: f32,
    fog_rd_end: f32,
    fog_sky_end: f32,
    fog_cloud_end: f32,
    /// Sky light multiplier by time of day (`chunk.wgsl`).
    daylight: f32,
    /// Selection outline stroke, framebuffer pixels divided by the target height, and the
    /// target's aspect: the line vertex shader's NDC offset per pixel (overlay.wgsl).
    line_width: f32,
    aspect: f32,
    /// Vanilla `Globals.GameTime`: `(gameTime % 24000 + partial) / 24000`, the end portal's
    /// layer scroll (0 when the frame carries no game time, e.g. the menu).
    game_time: f32,
    /// WGSL rounds a uniform struct's size up to its alignment (16): the Rust struct must
    /// be 144 bytes (36 floats), one pad float more than the WGSL fields spell out, or the
    /// buffer binds 4 bytes short of what the shader expects. The assertion below keeps
    /// Rust and WGSL agreeing.
    _pad: [f32; 2],
}

const _: () = assert!(
    std::mem::size_of::<CameraUniform>() == 144,
    "CameraUniform must match the WGSL Camera structs (144 bytes; see overlay.wgsl)"
);

/// One section layer to draw this frame.
#[derive(Clone, Copy)]
struct SectionDraw {
    /// Sort key: layer, then arena, then distance (negated for translucent: far first).
    layer: haste_assets::RenderLayer,
    arena: u32,
    key: f32,
    /// Vertex format: the index range and base vertex. Quad format: the
    /// span's record base and count (`kind` says which pool).
    first_index: u32,
    index_count: u32,
    base_vertex: u32,
    /// Quad format: the span kind (None-sized marker in the vertex format,
    /// where the index buffer carries the geometry).
    kind: crate::quad_format::SpanKind,
    origin: [i32; 3],
}

/// The renderer. Construct with a window (windowed mode) or offscreen
/// (`new_offscreen`, oracles's screenshot tests: fixed-size texture target,
/// `read_back` returns the frame as an RgbaImage).
#[allow(dead_code)]
pub struct Renderer {
    gpu: Gpu,
    surface: Option<wgpu::Surface<'static>>,
    surface_format: wgpu::TextureFormat,
    surface_state: Option<(u32, u32, wgpu::SurfaceConfiguration)>,
    offscreen: Option<Offscreen>,
    atlas: Arc<haste_assets::AtlasBuilder>,
    /// The uploaded atlas texture; animated sprites rewrite their regions in it
    /// (`animation`) each tick, so it must outlive init.
    atlas_texture: wgpu::Texture,
    /// Animated atlas textures: per-tick region uploads (`set_animation_tick`).
    animation: crate::animation::SpriteAnimator,
    /// The game tick the client last reported (`set_animation_tick`), for weather scroll.
    animation_tick: u64,
    baked: Arc<haste_assets::BakedBlockModels>,
    mesher: Mesher,
    /// Reused buffer for meshes drained from the mesher each frame.
    drain_buf: Vec<SectionMesh>,
    /// Dirty sections held back by mesh back-pressure (see `update_world_from`):
    /// submitted first on the next call.
    mesh_carry: Vec<SectionPos>,
    /// Grass/foliage colormaps from the assets (biome colours are resolved against them).
    colormaps: crate::tint::Colormaps,
    sections: crate::section_slab::SectionSlab,
    /// Shared vertex/index buffers the section meshes live in.
    arenas: MeshArenas,
    /// Cave culling: per-section sightline data (shared with the occlusion
    /// worker), the visible sections from the last completed walk, and the
    /// scheduling state. The walk runs on a worker thread; the render thread
    /// collects a finished mask, schedules a new walk, and draws the previous
    /// mask until the new one arrives (vanilla `SectionOcclusionGraph` does the
    /// same with its background full update).
    section_vis: Arc<Mutex<ahash::AHashMap<(ChunkPos, i32), crate::occlusion::VisibilitySet>>>,
    /// Per-column empty-section bits (vanilla `ClientChunkCache`'s empty sections,
    /// `SectionOcclusionGraph.updateEmptySections`): the walk's see-through data for
    /// sections that have no mesh, so unmeshed air never blocks it.
    known_empty: Arc<Mutex<crate::occlusion::EmptyColumns>>,
    visible_mask: crate::occlusion::VisibleMask,
    /// Bumped every time a finished walk replaces `visible_mask` or the look crosses a
    /// 2-degree step, so the feed can re-check its held-back dirty sections.
    want_version: u64,
    /// The look step (`floor(yaw/2)`, `floor(pitch/2)`) `want_version` last bumped for
    /// (vanilla `LevelExtractor.extract`'s `camRotX`/`camRotY` test).
    want_look: Option<(i32, i32)>,
    /// The last frame's frustum planes and camera position, for the mesh gate
    /// (`wants_section`): written in `render`, read by the feed before the next one.
    want_frustum: Option<([glam::Vec4; 6], glam::DVec3)>,
    /// Turn prediction for the mesh gate (`--premesh`): the mode, the smoothed camera
    /// motion, the widened frustum of the current frame and the pop-in/pre-mesh metrics
    /// (see `renderer/premesh.rs`).
    premesh: premesh::Premesh,
    /// The background-work budget (`--work-budget`): the mode, the frame-cost
    /// predictions and the frame-time target (see `renderer/work_budget.rs`).
    work_budget: work_budget::WorkBudget,
    /// Finished meshes held back by the adaptive budget's visible-first ordering (a
    /// section the gate no longer wants: pre-meshed or turned away from). Reused.
    premesh_hold: std::collections::VecDeque<SectionMesh>,
    /// Pre-mesh slack gate (see `renderer/terrain_feed.rs`): when the mesh pipeline's
    /// backlog stood above `PREMESH_SATURATED` this long, pre-meshing stops until it
    /// drains — under machine contention the pre-mesh volume is what collapses the
    /// frame, and a turn burst (which drains in a few hundred ms) must not trip it.
    premesh_saturated_since: Option<std::time::Instant>,
    premesh_gated: bool,
    /// The camera entity is a spectator (`Camera.extractRenderState` reads it off the player),
    /// for the solid-block rule below.
    camera_spectator: bool,
    /// Whether this frame's sections are drawn from the occlusion walk's mask. Vanilla
    /// `Camera.extractRenderState` turns `smartCull` off for a spectator camera inside a
    /// solid-render block, so the caves around it draw instead of an empty screen.
    cull_sections: bool,
    /// Camera section and render distance `visible_mask` was computed for.
    visible_for: Option<(SectionPos, i32)>,
    /// Camera section and render distance the in-flight walk is computing.
    scheduled_for: Option<(SectionPos, i32)>,
    /// A walk is scheduled and not yet collected.
    walk_in_flight: bool,
    /// Newly uploaded sections next to a visible one, drawn until the new mask
    /// arrives (vanilla propagates from newly loaded sections on the render
    /// thread). A dense list plus a bit on each named slab record (so a
    /// re-upload at the same position keeps its bonus), cleared when a new
    /// mask arrives. Reused across frames.
    bonus_list: Vec<SectionPos>,
    bonus_dirty: bool,
    /// The worker thread running the walks.
    walk: crate::occlusion::AsyncOcclusionWalker,
    /// Sightline data changed since the last schedule; a walk for it is scheduled as
    /// soon as none is in flight (vanilla re-walks from newly compiled sections every
    /// frame, `SectionOcclusionGraph.schedulePropagationFrom`).
    vis_dirty: bool,
    /// World vertical extent in sections (`set_dimension`).
    min_section: i32,
    section_count: i32,
    view_distance: i32,
    /// Per-frame draw list and section origins (instance-rate), reused across frames.
    draw_list: Vec<SectionDraw>,
    /// Per-layer scratch lists the cull fills in walk order (Solid, Cutout,
    /// CutoutMipped, Translucent), concatenated into `draw_list` per frame.
    layer_buckets: [Vec<SectionDraw>; 4],
    origins: Vec<[i32; 3]>,
    origin_buffer: wgpu::Buffer,
    origin_capacity: usize,
    /// Quad format: per-draw instance data (origin, span kind), reused across
    /// frames.
    quad_instances: Vec<crate::quad_format::QuadInstance>,
    quad_instance_buffer: wgpu::Buffer,
    quad_instance_capacity: usize,
    /// The quad format's static index buffer: the per-quad corner pattern
    /// (see `quad_format.rs`), bound once per frame.
    quad_pattern_index: wgpu::Buffer,
    /// Indirect draw arguments (multi-draw path), reused across frames.
    /// Five u32 per draw: index count, instance count, first index, base vertex, first instance
    /// (`wgpu::util::DrawIndexedIndirectArgs` layout).
    indirect: Vec<u32>,
    indirect_buffer: wgpu::Buffer,
    indirect_capacity: usize,
    camera_uniform: wgpu::Buffer,
    camera_bind_group_layout: wgpu::BindGroupLayout,
    camera_bind_group: wgpu::BindGroup,
    /// The first-person hand's own camera uniform: the hand projection (vanilla's
    /// `hudProjection`, fixed-base FOV) over the bob transform, identity view — the hand is
    /// submitted in view space (`renderItemInHand`'s view rotation and its inverse cancel).
    hand_camera_uniform: wgpu::Buffer,
    hand_camera_bind_group: wgpu::BindGroup,
    atlas_bind_group_layout: wgpu::BindGroupLayout,
    /// The vanilla light texture (`lightmap`), bound at group 2 by terrain and entities.
    lightmap: crate::lightmap::LightmapTexture,
    /// The dimension's fixed lightmap inputs (`set_dimension`); `render` folds the frame's
    /// [`LightmapFrame`] and the sky factor into a copy of these.
    lightmap_base: crate::lightmap::LightmapInputs,
    /// This frame's dynamic lightmap inputs (`set_lightmap_frame`).
    lightmap_frame: LightmapFrame,
    lightmap_bind_group: wgpu::BindGroup,
    atlas_bind_group: wgpu::BindGroup,
    pipeline_solid: wgpu::RenderPipeline,
    pipeline_cutout: wgpu::RenderPipeline,
    pipeline_translucent: wgpu::RenderPipeline,
    /// The same three stages pulling quad records (`--terrain-format quad`).
    quad_pipeline_solid: wgpu::RenderPipeline,
    quad_pipeline_cutout: wgpu::RenderPipeline,
    quad_pipeline_translucent: wgpu::RenderPipeline,
    depth: Option<wgpu::TextureView>,
    size: (u32, u32),
    /// GUI batch drawing (`FrameInput::gui`) and its own handle on the assets for lazy loads.
    gui: crate::gui::GuiRenderer,
    /// The menu background blur (vanilla: the `blur` post chain), when a screen asks for it.
    blur: crate::blur::MenuBlur,
    /// The title panorama (vanilla `CubeMap`), under the GUI when a screen extracted it.
    panorama: crate::panorama::PanoramaRenderer,
    /// Vanilla `Options.menuBackgroundBlurriness` (the blur radius; 0 = off).
    menu_blur_radius: u32,
    /// The blur-unavailable warning (a surface without COPY_SRC) logged once.
    blur_unavailable_logged: bool,
    assets: haste_assets::Assets,
    /// Entity cube models (`FrameInput::entities`).
    entity: crate::entity::EntityRenderer,
    /// Sky, celestial bodies, clouds; terrain fog colour and daylight.
    sky: crate::sky::SkyRenderer,
    /// The clouds (`CloudRenderer`): the flat or extruded cell mesh around the camera,
    /// by the `renderClouds` option (`set_cloud_status`).
    clouds: crate::clouds::CloudRenderer,
    /// Rain and snow columns (`FrameInput::sky.weather`).
    weather: crate::weather::WeatherRenderer,
    /// Selection outline and break cracks.
    overlay: crate::overlay::OverlayRenderer,
    /// Particles (`FrameInput::particles`).
    particles: crate::particles::ParticleRenderer,
    /// Timings of the last frame and the GPU timestamp reader.
    timings: crate::timing::FrameTimings,
    timer: Option<crate::timing::GpuTimer>,
    /// Mesh uploads and their bytes during the last `update_world*` call (feed phase, before
    /// `render`): reported in `FrameStats` so the bench can attribute slow frames.
    last_uploads: (u32, u64),
    /// Surface present mode (`set_vsync`).
    present_mode: wgpu::PresentMode,
    /// First-person arm or held item (`set_first_person`).
    first_person: Option<crate::entity::FirstPerson>,
    /// Name tags projected to GUI space this frame.
    nametags: Vec<crate::gui::ScreenNametag>,
    /// Vanilla `Options.entityShadows`: draw the round entity shadows (default on).
    entity_shadows: bool,
    /// F2 screenshot requested: the next `render` arms the capture for `take_capture`.
    capture_pending: bool,
    /// In-flight screenshot readback (copy submitted, async map pending) until
    /// `take_capture` collects it.
    capture: Option<Capture>,
    /// Whether the surface was configured with COPY_SRC: F2 then copies the presented
    /// frame; without it, the capture frame renders into a temporary offscreen target.
    surface_copy: bool,
    /// Whether the F2 capture path of this surface was logged (once per renderer).
    surface_copy_logged: bool,
    /// Block entities posed into the entity stream this frame (chests, shulker boxes).
    block_entities_drawn: u32,
    /// The frame's game time in ticks (`set_game_time`; vanilla `Level.getGameTime`), for
    /// the beam animation and the end portal's layer scroll.
    frame_game_time: i64,
    /// The fog colour (sRGB) and distances of the last frame, for logging and tests.
    last_fog: Option<([f32; 3], crate::fog::FogData)>,
    /// The frame's lightning bolts, collected from the entities (reused).
    lightning_bolts: Vec<crate::weather::LightningBolt>,
}
