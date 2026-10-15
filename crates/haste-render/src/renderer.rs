// The Renderer (docs/INTERFACES.md): wgpu device, atlas upload, section
// submission, camera/frustum, per-frame render with sky colour and fog.
// Offscreen construction + readback for oracles's screenshot tests.
// Shading design from Pomme (chunk shaders); see THIRD_PARTY.md.

use std::num::NonZeroU32;
use std::sync::Arc;

use anyhow::{Context, Result};
use haste_core::view::BlockView;
use haste_core::{ChunkPos, SectionPos};
use parking_lot::Mutex;
use wgpu::util::DeviceExt;

use crate::arena::{MeshArenas, ResidentSection};
use crate::mesher::{Mesher, SectionMesh};
use crate::pipeline::{upload_atlas, Gpu};
use crate::vertex::PackedVertex;

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
    pub rain: f32,
    pub dimension_has_sky: bool,
    pub fog_start: f32,
    pub fog_end: f32,
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

/// Per-frame render statistics (docs/INTERFACES.md FrameStats).
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    pub frame_ms: f32,
    pub chunk_sections_drawn: u32,
    pub chunk_sections_total: u32,
    pub meshes_pending: u32,
    pub draw_calls: u32,
    /// Block entities drawn this frame (chests, shulker boxes; one batched draw with entities).
    pub block_entities_drawn: u32,
    /// Section meshes uploaded this frame (one per finished mesh written to the arenas).
    pub uploads: u32,
    /// Vertex+index bytes written by this frame's uploads.
    pub upload_bytes: u64,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs the client jar, datagen reports, and a Vulkan driver (source scripts/env.sh)"]
    fn offscreen_render_flat_world() {
        let jar = haste_core::paths::client_jar();
        if !jar.exists() {
            eprintln!("ignored: needs the client jar in the cache");
            return;
        }
        let mut assets = haste_assets::Assets::open(
            &jar,
            &haste_core::paths::cache_dir().join("assets"),
            "26.3",
        )
        .unwrap();
        let states = haste_assets::BlocksJsonStates::load(
            &haste_core::paths::datagen_reports().join("blocks.json"),
        )
        .unwrap();

        let mut renderer = Renderer::new_offscreen(320, 240, &mut assets, &states).unwrap();

        // A flat stone world at y=0.
        struct Flat;
        impl haste_core::view::BlockView for Flat {
            fn block_state(&self, p: haste_core::BlockPos) -> haste_core::BlockStateId {
                if p.y == 0 {
                    haste_core::BlockStateId(1)
                } else {
                    haste_core::BlockStateId::AIR
                }
            }
            fn is_chunk_loaded(&self, _: haste_core::ChunkPos) -> bool {
                true
            }
            fn sky_light(&self, p: haste_core::BlockPos) -> u8 {
                if p.y <= 0 {
                    0
                } else {
                    15
                }
            }
            fn block_light(&self, _: haste_core::BlockPos) -> u8 {
                0
            }
            fn min_y(&self) -> i32 {
                -64
            }
            fn height(&self) -> i32 {
                384
            }
        }

        // Mesh and upload a few sections around the origin.
        let dirty: Vec<SectionPos> = (-1..=1)
            .flat_map(|x| (-1..=1).map(move |z| SectionPos::new(x, 0, z)))
            .collect();
        renderer.update_world(&Flat, &dirty, ChunkPos::new(0, 0), 4);
        // Meshing is asynchronous: keep draining until the sections are uploaded.
        for _ in 0..200 {
            renderer.update_world(&Flat, &[], ChunkPos::new(0, 0), 4);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let frame = FrameInput {
            camera: Camera {
                pos: glam::DVec3::new(8.0, 20.0, 8.0),
                yaw: 0.0,
                pitch: 60.0,
                fov_deg: 70.0,
                hand_fov_deg: 70.0,
                roll_deg: 0.0,
                bob: ViewBob::default(),
            },
            entities: &[],
            block_entities: &[],
            gui: &haste_core::gui::GuiFrame::default(),
            sky: SkyState {
                time_of_day: 6000,
                rain: 0.0,
                dimension_has_sky: true,
                fog_start: 100.0,
                fog_end: 200.0,
                ..Default::default()
            },
            partial_tick: 0.0,
            block_target: None,
            breaking: &[],
            camera_entity: None,
            spin: None,
        };
        // The section visibility walk runs off the render thread and lands a frame later
        // (vanilla draws the previous graph until the async update is swapped in), so keep
        // rendering until its mask arrives instead of asserting on the empty first frame.
        let mut stats = renderer.render(&frame).unwrap();
        for _ in 0..200 {
            if stats.chunk_sections_drawn > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            stats = renderer.render(&frame).unwrap();
        }
        eprintln!(
            "offscreen frame: {:.2} ms, {} drawn / {} sections, {} draw calls",
            stats.frame_ms,
            stats.chunk_sections_drawn,
            stats.chunk_sections_total,
            stats.draw_calls
        );
        assert!(stats.chunk_sections_drawn > 0, "no sections drawn");

        let image = renderer.read_back().unwrap();
        assert_eq!(image.dimensions(), (320, 240));
        // The frame must not be entirely the clear colour: some pixel differs
        // from the sky blue (0.47, 0.65, 1.0) => (120, 165, 255).
        let sky = image::Rgba([120, 165, 255, 255]);
        let any_different = image.pixels().any(|p| p.0 != sky.0);
        assert!(
            any_different,
            "frame is entirely clear colour; terrain not rendered"
        );
    }

    // Readback unpacking: 256-byte row padding and BGRA swap, no GPU needed.
    #[test]
    fn texel_rows_unpads_rows() {
        // 3x1 RGBA pixels, stride padded past 12 bytes.
        let row = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let stride = 16;
        let mut data = row.to_vec();
        data.resize(stride, 0);
        let out = texel_rows_to_rgba(&data, 3, 1, stride, false);
        assert_eq!(out, row.to_vec());
    }

    #[test]
    fn texel_rows_swap_bgra() {
        // Two BGRA pixels in one padded row: r and b swap per pixel.
        let row = [10u8, 20, 30, 255, 40, 50, 60, 255];
        let stride = 16;
        let mut data = row.to_vec();
        data.resize(stride, 0);
        let out = texel_rows_to_rgba(&data, 2, 1, stride, true);
        assert_eq!(out, vec![30, 20, 10, 255, 60, 50, 40, 255]);
    }

    // The async capture state machine's shared half, no GPU needed: the map callback
    // (a clone of the flag) advances Pending to Ready or Failed; take_capture polls.
    #[test]
    fn capture_flag_advances_pending_to_ready_or_failed() {
        let flag = CaptureFlag::default();
        assert_eq!(flag.get(), CapturePhase::Pending);
        flag.clone().set(CapturePhase::Ready);
        assert_eq!(flag.get(), CapturePhase::Ready);
        flag.clone().set(CapturePhase::Failed);
        assert_eq!(flag.get(), CapturePhase::Failed);
    }

    fn camera_with(bob: ViewBob, roll: f32) -> Camera {
        Camera {
            pos: glam::DVec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            fov_deg: 70.0,
            hand_fov_deg: 70.0,
            roll_deg: roll,
            bob,
        }
    }

    /// The look direction (Minecraft: yaw 0 = +z, pitch +90 = straight down) lands on view -Z
    /// at every angle, including straight up and down, where a look-at would degenerate.
    #[test]
    fn view_rotation_looks_along_minus_z_at_every_pitch() {
        for &(yaw, pitch) in &[
            (0.0f32, 0.0f32),
            (90.0, 30.0),
            (-135.0, -60.0),
            (37.0, 90.0),
            (37.0, -90.0),
            (180.0, 89.9),
        ] {
            let mut cam = camera_with(ViewBob::default(), 0.0);
            cam.yaw = yaw;
            cam.pitch = pitch;
            let (y, p) = (yaw.to_radians(), pitch.to_radians());
            let forward = glam::Vec3::new(-p.cos() * y.sin(), -p.sin(), p.cos() * y.cos());
            let v = Renderer::view_rotation(&cam).transform_vector3(forward);
            assert!(
                (v - glam::Vec3::NEG_Z).length() < 1e-5,
                "{yaw} {pitch}: {v}"
            );
            // The camera's right never flips: at yaw 0 the player's left (+x) is view -X.
            let left = glam::Vec3::new(y.cos(), 0.0, y.sin());
            let l = Renderer::view_rotation(&cam).transform_vector3(left);
            assert!(
                (l - glam::Vec3::NEG_X).length() < 1e-5,
                "{yaw} {pitch}: {l}"
            );
        }
    }

    #[test]
    fn view_transform_is_identity_without_effects() {
        let m = Renderer::view_transform(&camera_with(ViewBob::default(), 0.0));
        assert!(m.abs_diff_eq(glam::Mat4::IDENTITY, 1e-6));
    }

    #[test]
    fn view_transform_bobs_in_the_camera_plane() {
        // `bobView` translates x right / y up in view space, then rolls about Z and pitches
        // about X. With only a translation the matrix is that translation.
        let bob = ViewBob {
            x: 0.25,
            y: -0.5,
            ..ViewBob::default()
        };
        let m = Renderer::view_transform(&camera_with(bob, 0.0));
        let want = glam::Mat4::from_translation(glam::Vec3::new(0.25, -0.5, 0.0));
        assert!(m.abs_diff_eq(want, 1e-6));
    }

    #[test]
    fn view_transform_hurt_tilt_turns_the_roll_axis_by_hurt_dir() {
        // `bobHurt` rotates the z-roll axis by `-hurtDir`, rolls, then turns back. A 90
        // degree hurtDir turns the z-roll into a roll about the (rotated) axis.
        let bob = ViewBob {
            hurt_tilt_deg: 30.0,
            hurt_dir_deg: 90.0,
            ..ViewBob::default()
        };
        let m = Renderer::view_transform(&camera_with(bob, 0.0));
        let r = |axis: glam::Vec3, deg: f32| glam::Mat4::from_axis_angle(axis, deg.to_radians());
        let want = r(glam::Vec3::Y, -90.0) * r(glam::Vec3::Z, 30.0) * r(glam::Vec3::Y, 90.0);
        assert!(m.abs_diff_eq(want, 1e-6));
    }
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
    _pad: f32,
}

/// One section layer to draw this frame.
#[derive(Clone, Copy)]
struct SectionDraw {
    /// Sort key: layer, then arena, then distance (negated for translucent: far first).
    layer: haste_assets::RenderLayer,
    arena: u32,
    key: f32,
    first_index: u32,
    index_count: u32,
    base_vertex: u32,
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
    baked: Arc<haste_assets::BakedBlockModels>,
    mesher: Mesher,
    /// Grass/foliage colormaps from the assets (biome colours are resolved against them).
    colormaps: crate::tint::Colormaps,
    sections: ahash::AHashMap<SectionPos, ResidentSection>,
    /// Shared vertex/index buffers the section meshes live in.
    arenas: MeshArenas,
    /// Cave culling: per-section sightline data (shared with the occlusion
    /// worker), the visible sections from the last completed walk, and the
    /// scheduling state. The walk runs on a worker thread; the render thread
    /// collects a finished mask, schedules a new walk, and draws the previous
    /// mask until the new one arrives (vanilla `SectionOcclusionGraph` does the
    /// same with its background full update).
    section_vis: Arc<Mutex<ahash::AHashMap<(ChunkPos, i32), crate::occlusion::VisibilitySet>>>,
    visible_mask: crate::occlusion::VisibleMask,
    /// Camera section and render distance `visible_mask` was computed for.
    visible_for: Option<(SectionPos, i32)>,
    /// Camera section and render distance the in-flight walk is computing.
    scheduled_for: Option<(SectionPos, i32)>,
    /// A walk is scheduled and not yet collected.
    walk_in_flight: bool,
    /// Newly uploaded sections next to a visible one, drawn until the new mask
    /// arrives (vanilla propagates from newly loaded sections on the render
    /// thread). Reused across frames.
    bonus: ahash::AHashSet<(ChunkPos, i32)>,
    /// The worker thread running the walks.
    walk: crate::occlusion::AsyncOcclusionWalker,
    /// Sightline data changed since the last schedule; walks for that are at
    /// most 4 per second.
    vis_dirty: bool,
    last_walk: std::time::Instant,
    /// World vertical extent in sections (`set_dimension`).
    min_section: i32,
    section_count: i32,
    view_distance: i32,
    /// Per-frame draw list and section origins (instance-rate), reused across frames.
    draw_list: Vec<SectionDraw>,
    origins: Vec<[i32; 3]>,
    origin_buffer: wgpu::Buffer,
    origin_capacity: usize,
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
    depth: Option<wgpu::TextureView>,
    size: (u32, u32),
    /// GUI batch drawing (`FrameInput::gui`) and its own handle on the assets for lazy loads.
    gui: crate::gui::GuiRenderer,
    assets: haste_assets::Assets,
    /// Entity cube models (`FrameInput::entities`).
    entity: crate::entity::EntityRenderer,
    /// Sky, celestial bodies, clouds, weather; terrain fog colour and daylight.
    sky: crate::sky::SkyRenderer,
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
    /// The fog colour (sRGB) and distances of the last frame, for logging and tests.
    last_fog: Option<([f32; 3], crate::fog::FogData)>,
}

/// An in-flight screenshot readback: the copy is submitted and `map_async` posted, the
/// callback flips `status`, and `take_capture` collects it frames later without blocking.
struct Capture {
    buffer: std::sync::Arc<wgpu::Buffer>,
    status: CaptureFlag,
    width: u32,
    height: u32,
    /// Row stride of the padded copy buffer.
    stride: usize,
    /// Red/blue swapped on BGRA surfaces.
    bgra: bool,
}

/// Phases of an in-flight screenshot readback; the `map_async` callback advances
/// `Pending` to `Ready` or `Failed`, `take_capture` polls the flag.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CapturePhase {
    /// Copy submitted, map posted, callback not yet run.
    Pending,
    /// Mapped and readable.
    Ready,
    /// The map failed.
    Failed,
}

/// The phase shared with the `map_async` callback (a `CapturePhase` in an atomic).
#[derive(Clone, Debug, Default)]
struct CaptureFlag(std::sync::Arc<std::sync::atomic::AtomicU8>);

impl CaptureFlag {
    fn get(&self) -> CapturePhase {
        match self.0.load(std::sync::atomic::Ordering::Acquire) {
            1 => CapturePhase::Ready,
            2 => CapturePhase::Failed,
            _ => CapturePhase::Pending,
        }
    }
    fn set(&self, phase: CapturePhase) {
        self.0
            .store(phase as u8, std::sync::atomic::Ordering::Release);
    }
}

#[allow(dead_code)]
struct Offscreen {
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    /// Non-sRGB view of `color` for the GUI pass.
    gui_view: wgpu::TextureView,
    depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
    width: u32,
    height: u32,
}

impl Renderer {
    /// Windowed constructor (docs/INTERFACES.md).
    pub fn new(
        window: Arc<winit::window::Window>,
        assets: &mut haste_assets::Assets,
        states: &dyn haste_core::view::BlockStateTable,
    ) -> Result<Renderer> {
        let display = DisplayHandleWrapper {
            window: window.clone(),
        };
        let display: Box<dyn crate::pipeline::ShareableDisplayHandle> = Box::new(display);
        let gpu = Gpu::new(Some(display))?;
        let surface = gpu
            .instance
            .create_surface(wgpu::SurfaceTarget::Window(Box::new(window.clone())))?;
        let cap = surface.get_capabilities(&gpu.adapter);
        let format = cap
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(cap.formats[0]);
        let size = window.inner_size();
        let mut renderer = Self::init(
            gpu,
            Some((surface, format)),
            (size.width.max(1), size.height.max(1)),
            assets,
            states,
        )?;
        renderer.configure_surface();
        Ok(renderer)
    }

    /// Offscreen constructor (oracles screenshot tests): renders into a texture.
    pub fn new_offscreen(
        w: u32,
        h: u32,
        assets: &mut haste_assets::Assets,
        states: &dyn haste_core::view::BlockStateTable,
    ) -> Result<Renderer> {
        let gpu = Gpu::new(None)?;
        Self::init(gpu, None, (w.max(1), h.max(1)), assets, states)
    }

    #[allow(clippy::too_many_arguments)]
    fn init(
        gpu: Gpu,
        surface: Option<(wgpu::Surface<'static>, wgpu::TextureFormat)>,
        size: (u32, u32),
        assets: &mut haste_assets::Assets,
        states: &dyn haste_core::view::BlockStateTable,
    ) -> Result<Renderer> {
        // Bake + atlas.
        let mut atlas = haste_assets::AtlasBuilder::build(assets, &[], false)?;
        let baked = haste_assets::BakedBlockModels::bake(assets, states, &mut atlas)?;
        let baked = Arc::new(baked);
        let atlas = Arc::new(atlas);

        let (atlas_texture, view, sampler, sprite_rects, _len) =
            upload_atlas(&gpu.device, &gpu.queue, &atlas)?;
        let animation = crate::animation::SpriteAnimator::new(
            atlas.clone(),
            gpu.queue.clone(),
            atlas_texture.clone(),
        )?;

        // Camera uniform.
        let camera_uniform = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("camera"),
                contents: &[0; std::mem::size_of::<CameraUniform>()],
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let camera_bind_group_layout =
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("camera-layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });
        let camera_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera-bind"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_uniform.as_entire_binding(),
            }],
        });
        // The hand's own uniform, same layout (drawn by the entity pipeline in its own pass).
        let hand_camera_uniform =
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("hand-camera"),
                    contents: &[0; std::mem::size_of::<CameraUniform>()],
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });
        let hand_camera_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hand-camera-bind"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: hand_camera_uniform.as_entire_binding(),
            }],
        });

        // Atlas bind group (texture, sampler, sprite rects).
        let atlas_bind_group_layout =
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("atlas-layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 3,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: true },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                    ],
                });
        let atlas_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atlas-bind"),
            layout: &atlas_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: sprite_rects.as_entire_binding(),
                },
            ],
        });

        // The vanilla light texture (LightTexture): one 16x16 texel map for all terrain and
        // entity shading, re-uploaded when its inputs change.
        let lightmap = crate::lightmap::LightmapTexture::new(
            &gpu.device,
            &gpu.queue,
            crate::lightmap::LightmapInputs::overworld_noon(),
        );
        let lightmap_bind_group_layout =
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("lightmap-layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });
        let lightmap_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lightmap-bind"),
            layout: &lightmap_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(lightmap.view()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(lightmap.sampler()),
                },
            ],
        });
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("chunk"),
                source: wgpu::ShaderSource::Wgsl(include_str!("chunk.wgsl").into()),
            });
        let surface_format = surface
            .as_ref()
            .map(|(_, f)| *f)
            .unwrap_or(wgpu::TextureFormat::Rgba8UnormSrgb);
        let make_pipeline = |label: &str, fragment: &str, blend: Option<wgpu::BlendState>| {
            gpu.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(&gpu.device.create_pipeline_layout(
                        &wgpu::PipelineLayoutDescriptor {
                            label: Some("chunk-layout"),
                            bind_group_layouts: &[
                                Some(&camera_bind_group_layout),
                                Some(&atlas_bind_group_layout),
                                Some(&lightmap_bind_group_layout),
                            ],
                            immediate_size: 0,
                        },
                    )),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_main"),
                        compilation_options: Default::default(),
                        buffers: &[
                            Some(PackedVertex::LAYOUT),
                            Some(PackedVertex::INSTANCE_LAYOUT),
                        ],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(fragment),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: surface_format,
                            blend,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleList,
                        cull_mode: Some(wgpu::Face::Back),
                        ..Default::default()
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth24Plus,
                        depth_write_enabled: Some(true),
                        // Vanilla's terrain depth test: TERRAIN_SNIPPET inherits
                        // `DepthStencilState.DEFAULT` (GREATER_THAN_OR_EQUAL), drawn
                        // against a depth cleared to 0. Haste clears to 1, so the
                        // same rule is LessEqual: a quad may share a depth (vanilla's
                        // coplanar model elements, e.g. the grass block's side
                        // overlay over its side texture, both draw).
                        depth_compare: Some(wgpu::CompareFunction::LessEqual),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                })
        };
        let pipeline_solid = make_pipeline("chunk-solid", "fs_main", None);
        let pipeline_cutout = make_pipeline("chunk-cutout", "fs_main", None);
        let pipeline_translucent = make_pipeline(
            "chunk-translucent",
            "fs_translucent",
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );

        // Offscreen targets when windowless.
        let offscreen = if surface.is_none() {
            Some(Self::make_offscreen(
                &gpu.device,
                size,
                surface_format,
                Self::gui_format_for(&gpu, surface_format),
            )?)
        } else {
            None
        };

        let mesher = Mesher::new(baked.clone(), atlas.clone());
        let colormaps = crate::tint::Colormaps::load(assets).unwrap_or_else(|e| {
            tracing::warn!("colormaps unavailable ({e:#}): uniform grass/foliage colour");
            crate::tint::Colormaps::uniform(0x91BD59)
        });
        mesher.set_tables(crate::mesher::MeshTables::new(
            Arc::new(crate::tint::TintTable::new(states)),
            Arc::new(crate::tint::BiomeColors::new(&[], &colormaps)),
            Arc::new(crate::fluid::FluidTable::new(
                states,
                &crate::fluid::Fluids::vanilla(&crate::fluid::FluidSprites::from_atlas(&*atlas)),
                &crate::fluid::FluidRules::vanilla(),
                crate::fluid::FluidSprites::from_atlas(&*atlas),
            )),
        ));
        let arenas = MeshArenas::new(gpu.device.clone(), gpu.queue.clone());
        let timer = crate::timing::GpuTimer::new(&gpu.device, &gpu.queue);
        let origin_buffer = Self::make_origin_buffer(&gpu.device, 4096);
        let indirect_buffer = Self::make_indirect_buffer(&gpu.device, 4096);

        // GUI: the font (with unifont when the assets index has it) and a renderer drawing into
        // a non-sRGB view of the target so blending happens on sRGB values like vanilla.
        let font = Arc::new(assets.font_default().context("loading the default font")?);
        let mut gui = crate::gui::GuiRenderer::new(
            gpu.device.clone(),
            gpu.queue.clone(),
            Self::gui_format_for(&gpu, surface_format),
            font.clone(),
        );
        let entity = crate::entity::EntityRenderer::new(
            gpu.device.clone(),
            gpu.queue.clone(),
            surface_format,
            &camera_bind_group_layout,
            &lightmap_bind_group_layout,
            font.clone(),
        );
        let sky = crate::sky::SkyRenderer::new(
            gpu.device.clone(),
            gpu.queue.clone(),
            surface_format,
            assets,
        );
        let overlay = crate::overlay::OverlayRenderer::new(
            gpu.device.clone(),
            gpu.queue.clone(),
            surface_format,
            &camera_bind_group_layout,
            assets,
        );
        let particles = crate::particles::ParticleRenderer::new(
            gpu.device.clone(),
            gpu.queue.clone(),
            surface_format,
            &camera_bind_group_layout,
            &lightmap_bind_group_layout,
        );
        let mut gui_assets = assets.reopen()?;
        gui.preload_sprites(&mut gui_assets, crate::gui::HUD_SPRITES);

        let section_vis = Arc::new(Mutex::new(ahash::AHashMap::new()));
        let walk = crate::occlusion::AsyncOcclusionWalker::new(section_vis.clone())?;

        Ok(Renderer {
            gpu,
            surface: surface.map(|(s, _)| s),
            surface_format,
            surface_state: None,
            offscreen,
            atlas,
            atlas_texture,
            animation,
            baked,
            mesher,
            colormaps,
            sections: ahash::AHashMap::new(),
            arenas,
            section_vis,
            visible_mask: Default::default(),
            visible_for: None,
            scheduled_for: None,
            walk_in_flight: false,
            bonus: ahash::AHashSet::new(),
            walk,
            vis_dirty: false,
            last_walk: std::time::Instant::now(),
            min_section: -4,
            section_count: 24,
            view_distance: 32,
            draw_list: Vec::new(),
            origins: Vec::new(),
            origin_buffer,
            origin_capacity: 4096,
            indirect: Vec::new(),
            indirect_buffer,
            indirect_capacity: 4096,
            camera_uniform,
            camera_bind_group_layout,
            camera_bind_group,
            hand_camera_uniform,
            hand_camera_bind_group,
            atlas_bind_group_layout,
            lightmap,
            lightmap_base: crate::lightmap::LightmapInputs::overworld_noon(),
            lightmap_frame: LightmapFrame::default(),
            lightmap_bind_group,
            atlas_bind_group,
            pipeline_solid,
            pipeline_cutout,
            pipeline_translucent,
            depth: None,
            size,
            gui,
            assets: gui_assets,
            entity,
            sky,
            overlay,
            particles,
            first_person: None,
            timings: Default::default(),
            last_uploads: (0, 0),
            timer,
            present_mode: wgpu::PresentMode::Fifo,
            nametags: Vec::new(),
            entity_shadows: true,
            capture_pending: false,
            capture: None,
            surface_copy: false,
            surface_copy_logged: false,
            block_entities_drawn: 0,
            last_fog: None,
        })
    }

    /// Vsync on (FIFO) or off (Mailbox, else Immediate, whichever the surface supports; FIFO
    /// when neither is). Reconfigures the surface; no effect offscreen.
    pub fn set_vsync(&mut self, on: bool) {
        let mode = match &self.surface {
            None => return,
            Some(surface) if !on => {
                let modes = surface.get_capabilities(&self.gpu.adapter).present_modes;
                [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Immediate]
                    .into_iter()
                    .find(|m| modes.contains(m))
                    .unwrap_or(wgpu::PresentMode::Fifo)
            }
            Some(_) => wgpu::PresentMode::Fifo,
        };
        if mode != self.present_mode {
            self.present_mode = mode;
            self.configure_surface();
        }
    }

    /// The game tick animated atlas textures (water, lava, fire, portals) show;
    /// the client calls this once per game tick (20 Hz) and the next `render`
    /// re-uploads every changed sprite region.
    pub fn set_animation_tick(&mut self, tick: u64) {
        self.animation.set_tick(tick);
        // The GUI's own animated textures (the fire and nether-portal overlays) share the tick.
        self.gui.set_animation_tick(tick);
    }

    /// The first-person view: arm (empty hand) or held item; None draws nothing (third person,
    /// spectator). Stays until changed.
    pub fn set_first_person(&mut self, fp: Option<crate::entity::FirstPerson>) {
        self.first_person = fp;
    }

    /// Samples what this frame's overlays and entities need from the world (the renderer holds
    /// no world): entity light, the targeted block's outline boxes, the states of breaking
    /// blocks. Call before `render` with the frame you are about to render.
    pub fn sample_world(
        &mut self,
        world: &dyn BlockView,
        shapes: &dyn haste_core::view::CollisionShapes,
        frame: &FrameInput,
    ) {
        self.sample_entity_lights(world, frame.entities);
        self.prepare_shadows(world, shapes, frame);
        let o = &mut self.overlay.world;
        o.target = frame.block_target.and_then(|(pos, _)| {
            let state = world.block_state(pos);
            let boxes = shapes.outline_boxes(state);
            (!boxes.is_empty()).then(|| (pos, boxes.to_vec()))
        });
        o.breaking.clear();
        for &(pos, stage) in frame.breaking {
            o.breaking.push((pos, world.block_state(pos), stage));
        }
    }

    /// The dimension being drawn: picks the sky (overworld, End, or fog colour only).
    pub fn set_dimension(&mut self, dim: &haste_core::game::DimensionInfo) {
        self.sky.kind = crate::sky::SkyKind::of(dim);
        // The dimension's fixed lightmap inputs (Nether/End ambient colours, no sky term);
        // `render` folds this frame's dynamic inputs into a copy each frame.
        self.lightmap_base = if dim.has_skylight {
            crate::lightmap::LightmapInputs::overworld_noon()
        } else if dim.name.path() == "the_end" {
            crate::lightmap::end()
        } else {
            crate::lightmap::nether()
        };
        self.min_section = dim.min_section();
        self.section_count = dim.section_count() as i32;
        self.section_vis.lock().clear();
        self.visible_for = None;
        self.scheduled_for = None;
        self.walk_in_flight = false;
        self.bonus.clear();
    }

    /// This frame's dynamic lightmap inputs (flicker, brightness, night vision, darkness, boss
    /// overlay darkening, lightning flash), set by the client before `render`.
    pub fn set_lightmap_frame(&mut self, frame: LightmapFrame) {
        self.lightmap_frame = frame;
    }

    /// Uploads the particle atlas of `types` as the particle texture (call after
    /// `ParticleTypes::build`, once).
    pub fn set_particle_types(&mut self, types: &Arc<crate::particles::ParticleTypes>) {
        self.particles.set_types(types);
    }

    /// Poses this frame's particles (billboards, interpolated at `partial`) and samples their
    /// light from `world`. Like [`Self::sample_world`]: call before `render` with the frame
    /// you are about to render (the renderer holds no world).
    pub fn prepare_particles(
        &mut self,
        particles: &mut crate::particles::Particles,
        world: &dyn BlockView,
        camera: Camera,
        partial: f32,
    ) {
        self.particles.prepare(particles, camera, partial, world);
    }

    /// Sky and block light at each entity's position, sampled from the world by the caller
    /// (the renderer holds no world reference). Call before `render`; entities without an
    /// entry are drawn at full sky light.
    pub fn set_entity_lights(
        &mut self,
        lights: impl IntoIterator<Item = (haste_core::EntityId, (u8, u8))>,
    ) {
        self.entity.set_lights(lights);
    }

    /// Convenience over `set_entity_lights`: samples `world` at each instance's block position.
    pub fn sample_entity_lights(
        &mut self,
        world: &dyn BlockView,
        entities: &[haste_core::model::EntityInstance],
    ) {
        self.entity.set_lights(entities.iter().map(|e| {
            let p = haste_core::BlockPos::new(
                e.pos.x.floor() as i32,
                (e.pos.y + 0.5).floor() as i32,
                e.pos.z.floor() as i32,
            );
            (e.id, (world.sky_light(p), world.block_light(p)))
        }));
    }

    /// Vanilla `Options.entityShadows`: draw the round entity shadows (default on).
    pub fn set_entity_shadows(&mut self, on: bool) {
        self.entity_shadows = on;
    }

    /// Number of shadow quads batched for the last frame (tests/reporting).
    pub fn shadow_quad_count(&self) -> usize {
        self.entity.shadow_quad_count()
    }

    /// Extracts and batches the entity shadows for `frame` (vanilla
    /// `EntityRenderer.extractShadow`/`finalizeRenderState`). Called by [`Self::sample_world`];
    /// call it directly when you render without `sample_world` (tests) and want shadows.
    pub fn prepare_shadows(
        &mut self,
        world: &dyn BlockView,
        shapes: &dyn haste_core::view::CollisionShapes,
        frame: &FrameInput,
    ) {
        // `Level.getSkyDarken` = (int)(15 - SKY_LIGHT_LEVEL) off the day timeline; skyless
        // dimensions never use it (`getRawBrightness` reads only the block light there).
        let sky_darken = if frame.sky.dimension_has_sky {
            (15.0 * (1.0 - crate::lightmap::overworld_sky_factor(frame.sky.time_of_day))).floor()
                as i32
        } else {
            0
        };
        // `DimensionType.ambientLight`: 0.1 in the Nether (and skyless custom dimensions), 0
        // in the overworld and End.
        let ambient = if self.sky.kind == crate::sky::SkyKind::None {
            0.1
        } else {
            0.0
        };
        self.entity.prepare_shadows(
            frame.entities,
            frame.camera_entity,
            frame.camera.pos,
            world,
            shapes,
            sky_darken,
            ambient,
            self.entity_shadows,
            &mut self.assets,
        );
    }

    /// Text measurement over the default font (vanilla `Font.width` rules), for the HUD and
    /// tier-3 `Font` natives.
    pub fn text_metrics(&self) -> Arc<dyn haste_core::gui::TextMetrics> {
        self.gui.font().clone()
    }

    /// Item names by `minecraft:item` protocol id, so `GuiCommand::Item` can find models.
    pub fn set_item_registry(&mut self, registries: &haste_data::Registries) {
        let n = registries.len("minecraft:item");
        let names = (0..n as u32)
            .map(|i| {
                registries
                    .name("minecraft:item", haste_core::RegistryIndex(i))
                    .cloned()
                    .unwrap_or_else(|| haste_core::Identifier::minecraft("air"))
            })
            .collect::<Vec<_>>();
        self.entity.set_item_names(names.clone());
        self.gui.set_item_names(names);
    }

    /// Whether GUI blending happens on sRGB values like vanilla (false on backends without
    /// view formats, e.g. GL, where it blends in linear space).
    pub fn gui_blends_in_srgb(&self) -> bool {
        self.gpu.view_formats
    }

    /// CPU phase and GPU pass timings of the last frame.
    pub fn timings(&self) -> crate::timing::FrameTimings {
        self.timings
    }

    /// Target size in pixels.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The surface present mode in use.
    pub fn present_mode(&self) -> wgpu::PresentMode {
        self.present_mode
    }

    /// The adapter this renderer runs on (name and backend, for bench reports).
    pub fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.gpu.adapter.get_info()
    }

    /// Counters of the last GUI frame.
    pub fn gui_stats(&self) -> crate::gui::GuiStats {
        self.gui.stats
    }

    /// Particles posed in the last frame (debug: also logged once a second at debug level).
    pub fn particles_drawn(&self) -> u32 {
        self.particles.drawn
    }

    /// Entity instances that posed geometry in the last frame (models, decorations and
    /// dropped items; `FrameInput::entities` instances without geometry do not count).
    pub fn entities_drawn(&self) -> u32 {
        self.entity.drawn
    }

    /// The fog colour (sRGB) and distances of the last frame (`None` before the first render).
    pub fn last_fog(&self) -> Option<([f32; 3], crate::fog::FogData)> {
        self.last_fog
    }

    fn make_origin_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("section-origins"),
            size: (capacity * 12) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn make_indirect_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("section-indirect"),
            size: (capacity * std::mem::size_of::<wgpu::util::DrawIndexedIndirectArgs>()) as u64,
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// The format the GUI pass renders in: the non-sRGB view of the target where views are
    /// supported, else the target format itself.
    fn gui_format_for(gpu: &Gpu, format: wgpu::TextureFormat) -> wgpu::TextureFormat {
        if gpu.view_formats {
            format.remove_srgb_suffix()
        } else {
            format
        }
    }

    fn make_offscreen(
        device: &wgpu::Device,
        size: (u32, u32),
        format: wgpu::TextureFormat,
        gui_format: wgpu::TextureFormat,
    ) -> Result<Offscreen> {
        let extra = [gui_format];
        let view_formats: &[wgpu::TextureFormat] = if gui_format != format { &extra } else { &[] };
        let (w, h) = size;
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen-color"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats,
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen-depth"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth24Plus,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        Ok(Offscreen {
            gui_view: color.create_view(&wgpu::TextureViewDescriptor {
                format: Some(gui_format),
                ..Default::default()
            }),
            color_view: color.create_view(&Default::default()),
            depth_view: depth.create_view(&Default::default()),
            color,
            depth,
            width: w,
            height: h,
        })
    }

    fn configure_surface(&mut self) {
        let Some(surface) = &self.surface else { return };
        let (w, h) = self.size;
        // COPY_SRC, when the surface supports it, lets F2 copy the presented frame out
        // directly; without it, the capture frame renders into an offscreen target.
        let copy_src = surface
            .get_capabilities(&self.gpu.adapter)
            .usages
            .contains(wgpu::TextureUsages::COPY_SRC);
        if !self.surface_copy_logged {
            self.surface_copy_logged = true;
            tracing::info!(
                "surface {} COPY_SRC: F2 captures {}",
                if copy_src { "supports" } else { "lacks" },
                if copy_src {
                    "the presented frame"
                } else {
                    "an offscreen frame"
                }
            );
        }
        self.surface_copy = copy_src;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | if copy_src {
                    wgpu::TextureUsages::COPY_SRC
                } else {
                    wgpu::TextureUsages::empty()
                },
            format: self.surface_format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: w,
            height: h,
            present_mode: self.present_mode,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: {
                let gui = Self::gui_format_for(&self.gpu, self.surface_format);
                if gui != self.surface_format {
                    vec![gui]
                } else {
                    vec![]
                }
            },
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&self.gpu.device, &config);
        self.surface_state = Some((w, h, config));
        self.ensure_depth();
    }

    fn ensure_depth(&mut self) {
        let depth = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width: self.size.0,
                height: self.size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth24Plus,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.depth = Some(depth.create_view(&Default::default()));
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.size = (w.max(1), h.max(1));
        if self.offscreen.is_some() {
            if let Ok(off) = Self::make_offscreen(
                &self.gpu.device,
                self.size,
                self.surface_format,
                Self::gui_format_for(&self.gpu, self.surface_format),
            ) {
                self.offscreen = Some(off);
            }
        } else {
            self.configure_surface();
        }
    }

    /// The biome table of the joined server (synced `minecraft:worldgen/biome` registry, index =
    /// biome id). Applies to sections meshed afterwards: call it before the first chunks.
    pub fn set_biomes(&mut self, biomes: &[crate::tint::BiomeClimate]) {
        let mut tables = self.mesher.tables();
        let radius = tables.colors.radius();
        tables.colors =
            Arc::new(crate::tint::BiomeColors::new(biomes, &self.colormaps).with_radius(radius));
        self.mesher.set_tables(tables);
    }

    /// Biome blend radius (vanilla "Biome Blend": 0 = off, 2 = default 5x5).
    pub fn set_biome_blend(&mut self, radius: i32) {
        let mut tables = self.mesher.tables();
        tables.colors = Arc::new((*tables.colors).clone().with_radius(radius));
        self.mesher.set_tables(tables);
    }

    /// Submit dirty sections to the mesher (snapshots taken from `world`) and
    /// upload finished meshes (per-frame budget). Without biomes every block tints as plains.
    pub fn update_world(
        &mut self,
        world: &dyn BlockView,
        dirty: &[SectionPos],
        camera_chunk: ChunkPos,
        view_distance: i32,
    ) {
        self.update_world_with_biomes(world, None, dirty, camera_chunk, view_distance);
    }

    /// `update_world` reading biomes (for grass/foliage/water tints) from `biomes`.
    pub fn update_world_with_biomes(
        &mut self,
        world: &dyn BlockView,
        biomes: Option<&dyn crate::tint::BiomeSource>,
        dirty: &[SectionPos],
        _camera_chunk: ChunkPos,
        view_distance: i32,
    ) {
        if view_distance != self.view_distance {
            self.view_distance = view_distance;
            self.visible_for = None;
        }
        for pos in dirty {
            self.mesher.submit_with_biomes(world, biomes, *pos);
        }
        self.last_uploads = self.upload_finished();
    }

    /// Like `update_world`, but the mesh workers capture the snapshots themselves from
    /// `source` (the render thread neither reads the world nor holds its lock).
    pub fn update_world_from(
        &mut self,
        source: &Arc<dyn crate::mesher::SnapshotSource>,
        dirty: &[SectionPos],
        _camera_chunk: ChunkPos,
        view_distance: i32,
    ) {
        if view_distance != self.view_distance {
            self.view_distance = view_distance;
            self.visible_for = None;
        }
        self.mesher.submit_from(source, dirty);
        self.last_uploads = self.upload_finished();
    }

    /// Uploads finished meshes for up to 2 ms; returns (sections uploaded, bytes written).
    fn upload_finished(&mut self) -> (u32, u64) {
        let started = std::time::Instant::now();
        let mut uploads = 0;
        let mut bytes = 0;
        while started.elapsed() < std::time::Duration::from_millis(2) {
            let batch = self.mesher.drain(16);
            if batch.is_empty() {
                break;
            }
            for mesh in batch {
                bytes += mesh.vertex_bytes() + mesh.index_bytes();
                self.upload_mesh(mesh);
                uploads += 1;
            }
        }
        (uploads, bytes)
    }

    fn upload_mesh(&mut self, mesh: SectionMesh) {
        let pos = mesh.pos;
        let si = pos.y - self.min_section;
        let mut vis = self.section_vis.lock();
        let prev = vis.insert((ChunkPos::new(pos.x, pos.z), si), mesh.visibility);
        drop(vis);
        if prev != Some(mesh.visibility) {
            self.vis_dirty = true;
            self.add_to_bonus(pos, si);
        }
        if let Some(old) = self.sections.remove(&pos) {
            self.arenas.free(&old);
        }
        if let Some(resident) = self.arenas.upload(&mesh) {
            self.sections.insert(pos, resident);
        }
    }

    /// The pose stack vanilla's `GameRenderer` multiplies onto the projection matrix
    /// (`renderLevel`: bobHurt, then bobView), plus the camera's plain roll. In view space
    /// (x right, y up, z back), as rotations about +Z roll the horizon one way.
    pub(crate) fn view_transform(camera: &Camera) -> glam::Mat4 {
        // The plain roll sits in front of everything else (`Camera.setRotation`).
        glam::Mat4::from_axis_angle(glam::Vec3::Z, camera.roll_deg.to_radians())
            * Self::bob_transform(camera)
    }

    /// The same stack without the plain camera roll: the death roll, the hurt tilt and the
    /// view bobbing (`GameRenderer.bobHurt` + `bobView`). Vanilla's `renderItemInHand` puts
    /// exactly this onto the hand's pose stack, after the view rotation and its inverse have
    /// cancelled, so the hand bobs with the world but never takes the camera's plain roll.
    pub(crate) fn bob_transform(camera: &Camera) -> glam::Mat4 {
        let b = camera.bob;
        let z = glam::Vec3::Z;
        let y = glam::Vec3::Y;
        // The death roll (a plain z-axis rotation), then the hurt tilt around the z-axis
        // turned by the damage direction.
        let m = glam::Mat4::from_axis_angle(z, b.death_roll_deg.to_radians())
            * glam::Mat4::from_axis_angle(y, -b.hurt_dir_deg.to_radians())
            * glam::Mat4::from_axis_angle(z, b.hurt_tilt_deg.to_radians())
            * glam::Mat4::from_axis_angle(y, b.hurt_dir_deg.to_radians());
        // The view bobbing: translate in the camera plane, then roll, then pitch.
        m * glam::Mat4::from_translation(glam::Vec3::new(b.x, b.y, 0.0))
            * glam::Mat4::from_axis_angle(z, b.roll_deg.to_radians())
            * glam::Mat4::from_axis_angle(glam::Vec3::X, b.pitch_deg.to_radians())
    }

    /// A newly uploaded section is drawn until the async walk catches up when a
    /// face-neighbour is in the current mask: vanilla seeds newly loaded
    /// sections into the propagation queue, so they appear before the next
    /// full update. The bonus is cleared when the new mask arrives.
    fn add_to_bonus(&mut self, pos: SectionPos, si: i32) {
        if !(0..32).contains(&si) {
            return;
        }
        for face in crate::occlusion::Face::ALL {
            let (dx, dy, dz) = face.offset();
            let (nx, nsy, nz) = (pos.x + dx, pos.y + dy, pos.z + dz);
            let nsi = nsy - self.min_section;
            if !(0..32).contains(&nsi) {
                continue;
            }
            let column = self.visible_mask.column(ChunkPos::new(nx, nz));
            if column & (1u32 << nsi) != 0 {
                self.bonus.insert((ChunkPos::new(pos.x, pos.z), si));
                return;
            }
        }
    }

    /// Build the view-projection matrix (right-handed, OpenGL-style depth like
    /// wgpu's default reverse... wgpu uses 0..1 z; glam's perspective_rh
    /// produces -1..1, so use perspective_infinite_rh style correction).
    fn view_projection(camera: &Camera, aspect: f32, spin: Option<SpinWarp>) -> glam::Mat4 {
        let mut proj =
            glam::Mat4::perspective_rh(camera.fov_deg.to_radians(), aspect, 0.05, 4096.0);
        if let Some(spin) = spin {
            // Vanilla's `GameRenderer.renderLevel`: rotate the projection about
            // (0, √2/2, √2/2) by the spinning angle, squeeze x by 1/skew, rotate back. The
            // multiplications are right-to-left in effect, so this is
            // P * R(axis, angle) * S(1/skew, 1, 1) * R(axis, -angle).
            let axis = glam::Vec3::new(
                0.0,
                std::f32::consts::SQRT_2 / 2.0,
                std::f32::consts::SQRT_2 / 2.0,
            );
            let angle = spin.angle_deg.to_radians();
            let skew = spin.skew();
            proj *= glam::Mat4::from_axis_angle(axis, angle);
            proj *= glam::Mat4::from_diagonal(glam::Vec4::new(1.0 / skew, 1.0, 1.0, 1.0));
            proj *= glam::Mat4::from_axis_angle(axis, -angle);
        }
        let eye = glam::Vec3::new(
            camera.pos.x as f32,
            camera.pos.y as f32,
            camera.pos.z as f32,
        );
        let view = Self::view_rotation(camera) * glam::Mat4::from_translation(-eye);
        proj * Self::view_transform(camera) * view
    }

    /// The camera's rotation as vanilla builds it (`Camera.setRotation`: a YXZ rotation from yaw
    /// and pitch, plus the roll), so looking straight up or down (pitch +-90) is exact and never
    /// flips, unlike a look-at with a fixed up vector. View space looks along -Z.
    fn view_rotation(camera: &Camera) -> glam::Mat4 {
        glam::Mat4::from_rotation_z(-camera.roll_deg.to_radians())
            * glam::Mat4::from_rotation_x(camera.pitch.to_radians())
            * glam::Mat4::from_rotation_y((camera.yaw + 180.0).to_radians())
    }

    /// Frustum culling of sections (Pomme cull.comp reference; CPU here).
    fn frustum_planes(m: &glam::Mat4) -> [glam::Vec4; 6] {
        // Gribb/Hartmann: planes are row3 +- row_i of the clip matrix (clip = M * p).
        // wgpu clip z is 0..w: the near plane is row2 alone.
        let r = |i: usize| m.row(i);
        let (r0, r1, r2, r3) = (r(0), r(1), r(2), r(3));
        [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2]
            .map(|p| p / p.truncate().length().max(1e-12))
    }

    fn aabb_visible(planes: &[glam::Vec4; 6], min: [f32; 3], max: [f32; 3]) -> bool {
        // Conservative: AABB vs plane with the positive vertex.
        for p in planes {
            let px = if p.x >= 0.0 { max[0] } else { min[0] };
            let py = if p.y >= 0.0 { max[1] } else { min[1] };
            let pz = if p.z >= 0.0 { max[2] } else { min[2] };
            if p.x * px + p.y * py + p.z * pz + p.w < 0.0 {
                return false;
            }
        }
        true
    }

    /// Render one frame. Returns stats (frame time measured around the submit).
    pub fn render(&mut self, frame: &FrameInput) -> Result<FrameStats> {
        let started = std::time::Instant::now();
        let mut mark = started;
        let lap = |m: &mut std::time::Instant| {
            let now = std::time::Instant::now();
            let d = (now - *m).as_secs_f32() * 1000.0;
            *m = now;
            d
        };
        let aspect = self.size.0 as f32 / self.size.1 as f32;
        let view_proj = Self::view_projection(&frame.camera, aspect, frame.spin);
        self.entity.prepare(
            frame.entities,
            frame.camera_entity,
            frame.camera.pos,
            &mut self.assets,
        );
        {
            // The hand is drawn in view space: vanilla's `renderItemInHand` multiplies the
            // view rotation and its inverse into the pose, so they cancel and the hand's own
            // pose stack starts with `bobHurt` and `bobView` — the same bob the world's
            // projection carries, so hand and world wobble together. The plain camera roll
            // cancels with the view rotation and must not appear here.
            self.entity.prepare_hand(
                self.first_person.as_ref(),
                Self::bob_transform(&frame.camera),
                (frame.camera.yaw, frame.camera.pitch),
                frame.partial_tick,
                &mut self.assets,
            );
        }
        self.overlay.prepare(
            frame.camera.pos,
            frame.block_target.map(|(p, _)| p),
            frame.breaking,
            &self.baked,
        );
        self.nametags.clear();
        let gui_scale = frame.gui.scale.max(1) as f32;
        for tag in &self.entity.nametags {
            if let Some(t) =
                crate::gui::project_nametag(&view_proj, &frame.camera, tag, self.size, gui_scale)
            {
                self.nametags.push(t);
            }
        }
        let rot_view_proj = Self::view_projection(
            &Camera {
                pos: glam::DVec3::ZERO,
                ..frame.camera
            },
            aspect,
            frame.spin,
        );
        // The same matrix without the warp: the hand is drawn with the HUD projection in
        // vanilla (`GameRenderer.render3dHud` sets it before `renderItemInHand`), so it keeps
        // the plain projection while the world wobbles. The warp is a pure projection
        // multiplication, so an absent one leaves the matrix exactly as it was.
        let rot_view_proj_plain = if frame.spin.is_some() {
            Self::view_projection(
                &Camera {
                    pos: glam::DVec3::ZERO,
                    ..frame.camera
                },
                aspect,
                None,
            )
        } else {
            rot_view_proj
        };
        // Vanilla fog: `FogRenderer.setupFog`/`computeFogColor` pick the environment from the
        // camera's fluid, the player's effects and the dimension/biome attributes (all in
        // `frame.sky.fog`); the same path covers the overworld, the Nether and the End.
        let camera_forward = {
            let yaw = frame.camera.yaw.to_radians();
            let pitch = frame.camera.pitch.to_radians();
            [
                -pitch.cos() * yaw.sin(),
                -pitch.sin(),
                pitch.cos() * yaw.cos(),
            ]
        };
        let world_min_y = (self.min_section << 4) as f32;
        let (fog_srgb, fog_data) = crate::fog::frame_fog(
            &frame.sky,
            frame.partial_tick,
            camera_forward,
            frame.camera.pos.y as f32,
            world_min_y,
        );
        self.last_fog = Some((fog_srgb, fog_data));
        self.sky.prepare(
            &frame.camera,
            &frame.sky,
            view_proj,
            rot_view_proj,
            &fog_data,
            fog_srgb,
        );
        let fog_color = crate::sky::linear(fog_srgb);
        let mut camera_uniform = CameraUniform {
            cam_pos: [
                frame.camera.pos.x as f32,
                frame.camera.pos.y as f32,
                frame.camera.pos.z as f32,
                0.0,
            ],
            // Camera-relative: the shaders subtract cam_pos from world positions.
            view_proj: rot_view_proj.to_cols_array_2d(),
            fog_color: [fog_color[0], fog_color[1], fog_color[2], 0.0],
            fog_env_start: fog_data.environmental_start,
            fog_env_end: fog_data.environmental_end,
            fog_rd_start: fog_data.render_distance_start,
            fog_rd_end: fog_data.render_distance_end,
            fog_sky_end: fog_data.sky_end,
            fog_cloud_end: fog_data.cloud_end,
            daylight: self.sky.daylight(),
            _pad: 0.0,
        };
        self.gpu
            .queue
            .write_buffer(&self.camera_uniform, 0, bytemuck::bytes_of(&camera_uniform));

        // The hand's own camera uniform, written only when the hand is drawn: vanilla's
        // `render3dHud` gives the hand its own projection (`hudProjection`: the fixed-base
        // hand FOV, near 0.05) with no bob, because `renderItemInHand` puts the bob onto the
        // hand's pose stack instead (the `bob_transform` prefix `prepare_hand` receives) —
        // the same bob the world's projection carries, applied exactly once. The hand is
        // submitted in view space, so there is no view matrix. The fog fields stay the
        // world's: the hand is under a metre from the eye, so like vanilla's own hand it
        // never reaches the first fog band.
        if self.entity.has_hand() {
            let hand_proj = glam::Mat4::perspective_rh(
                frame.camera.hand_fov_deg.to_radians(),
                aspect,
                0.05,
                4096.0,
            );
            let hand_uniform = CameraUniform {
                cam_pos: [0.0; 4],
                view_proj: hand_proj.to_cols_array_2d(),
                fog_color: [fog_color[0], fog_color[1], fog_color[2], 0.0],
                fog_env_start: fog_data.environmental_start,
                fog_env_end: fog_data.environmental_end,
                fog_rd_start: fog_data.render_distance_start,
                fog_rd_end: fog_data.render_distance_end,
                fog_sky_end: fog_data.sky_end,
                fog_cloud_end: fog_data.cloud_end,
                daylight: self.sky.daylight(),
                _pad: 0.0,
            };
            self.gpu.queue.write_buffer(
                &self.hand_camera_uniform,
                0,
                bytemuck::bytes_of(&hand_uniform),
            );
        }

        // The light texture's inputs for this frame (LightmapRenderStateExtractor.extract):
        // the dimension's fixed colours with the timeline's sky factor (and the rain blend,
        // WeatherAttributes.RAIN moving SKY_LIGHT_FACTOR towards 0.24 by rain level) plus the
        // frame's flicker, brightness, night vision, darkness, boss darkening and lightning.
        let mut inputs = self.lightmap_base;
        if frame.sky.dimension_has_sky {
            let mut factor = crate::lightmap::overworld_sky_factor(frame.sky.time_of_day);
            let rain = frame.sky.rain.clamp(0.0, 1.0);
            factor += (0.24 - factor) * rain * 0.3125;
            // ClientLevel's time-based SKY_LIGHT_FACTOR layer: a lightning flash is full sky.
            if self.lightmap_frame.lightning_flash {
                factor = 1.0;
            }
            inputs.sky_darken = 1.0 - factor;
        }
        inputs.block_light_flicker = self.lightmap_frame.block_light_flicker;
        inputs.gamma = self.lightmap_frame.gamma;
        inputs.night_vision = self.lightmap_frame.night_vision;
        inputs.darkness = self.lightmap_frame.darkness;
        inputs.boss_overlay_darkening = self.lightmap_frame.boss_overlay_darkening;
        self.lightmap.set_inputs(inputs);
        self.lightmap.upload(&self.gpu.queue);

        // Which target are we drawing into?
        // The acquired SurfaceTexture must outlive the pass (it presents on drop
        // after submit); hold it alongside the views.
        self.timings.prepare_ms = lap(&mut mark);
        if let Some(t) = &mut self.timer {
            t.poll(&self.gpu.device);
            self.timings.gpu_ms = t.latest;
        }
        // A pending capture (F2) copies this frame's image out for `take_capture`; with
        // COPY_SRC on the surface that is the presented frame itself, and only without it
        // does this frame render into a temporary offscreen target (and not present).
        let capture_frame = self.capture_pending && self.offscreen.is_none();
        let mut held_surface_texture: Option<wgpu::SurfaceTexture> = None;
        let mut capture_color: Option<wgpu::Texture> = None;
        let (color_view, depth_view, gui_view) = if capture_frame && !self.surface_copy {
            let cap = Self::make_offscreen(
                &self.gpu.device,
                self.size,
                self.surface_format,
                Self::gui_format_for(&self.gpu, self.surface_format),
            )?;
            let views = (
                cap.color_view.clone(),
                cap.depth_view.clone(),
                cap.gui_view.clone(),
            );
            capture_color = Some(cap.color);
            views
        } else {
            match (&self.offscreen, &self.depth) {
                (Some(off), _) => (
                    off.color_view.clone(),
                    off.depth_view.clone(),
                    off.gui_view.clone(),
                ),
                (None, Some(d)) => {
                    let surface = self.surface.as_ref().context("no surface")?;
                    let frame_tex = match surface.get_current_texture() {
                        wgpu::CurrentSurfaceTexture::Success(t)
                        | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
                        wgpu::CurrentSurfaceTexture::Timeout
                        | wgpu::CurrentSurfaceTexture::Occluded => {
                            anyhow::bail!("surface frame unavailable (timeout/occluded)")
                        }
                        wgpu::CurrentSurfaceTexture::Outdated
                        | wgpu::CurrentSurfaceTexture::Lost
                        | wgpu::CurrentSurfaceTexture::Validation => {
                            anyhow::bail!("surface lost/outdated; reconfigure needed")
                        }
                    };
                    held_surface_texture = Some(frame_tex);
                    let tex = held_surface_texture.as_ref().unwrap();
                    (
                        tex.texture.create_view(&Default::default()),
                        d.clone(),
                        tex.texture.create_view(&wgpu::TextureViewDescriptor {
                            format: Some(Self::gui_format_for(&self.gpu, self.surface_format)),
                            ..Default::default()
                        }),
                    )
                }
                _ => anyhow::bail!("no render target"),
            }
        };
        let present = held_surface_texture.is_some();
        let _ = present;

        self.timings.acquire_ms = lap(&mut mark);
        // Frustum cull in camera-relative space (f32 precision far from the origin), into the
        // reused draw list.
        let planes = Self::frustum_planes(&rot_view_proj);
        let cam = frame.camera.pos;
        // Cave culling: the walk runs on a worker thread (vanilla's
        // SectionOcclusionGraph full update runs on a background executor).
        // Collect a finished mask first, then schedule a new walk when the
        // camera moved or new sightline data arrived and none is in flight.
        // Until the new mask arrives the previous one is drawn, plus newly
        // uploaded sections next to visible ones (`bonus`).
        if let Some(mask) = self.walk.poll() {
            self.visible_mask = mask;
            self.visible_for = self.scheduled_for.take();
            self.walk_in_flight = false;
            self.bonus.clear();
        }
        let cam_section = SectionPos::new(
            (cam.x.floor() as i32) >> 4,
            (cam.y.floor() as i32) >> 4,
            (cam.z.floor() as i32) >> 4,
        );
        let need_walk = self.visible_for != Some((cam_section, self.view_distance))
            || (self.vis_dirty && self.last_walk.elapsed() > std::time::Duration::from_millis(250));
        if need_walk && !self.walk_in_flight {
            self.walk.schedule(
                cam_section,
                self.view_distance,
                self.section_count,
                self.min_section,
                cam,
            );
            self.scheduled_for = Some((cam_section, self.view_distance));
            self.walk_in_flight = true;
            self.vis_dirty = false;
            self.last_walk = std::time::Instant::now();
        }
        let mut draws = std::mem::take(&mut self.draw_list);
        draws.clear();
        for s in self.sections.values() {
            let si = s.pos.y - self.min_section;
            let column = self.visible_mask.column(ChunkPos::new(s.pos.x, s.pos.z));
            let bonus = self.bonus.contains(&(ChunkPos::new(s.pos.x, s.pos.z), si));
            if !(0..32).contains(&si) || (column & (1u32 << si) == 0 && !bonus) {
                continue;
            }
            let lo = [
                (s.aabb.0[0] as f64 - cam.x) as f32,
                (s.aabb.0[1] as f64 - cam.y) as f32,
                (s.aabb.0[2] as f64 - cam.z) as f32,
            ];
            let hi = [
                (s.aabb.1[0] as f64 - cam.x) as f32,
                (s.aabb.1[1] as f64 - cam.y) as f32,
                (s.aabb.1[2] as f64 - cam.z) as f32,
            ];
            if !Self::aabb_visible(&planes, lo, hi) {
                continue;
            }
            let d = glam::Vec3::new(
                (lo[0] + hi[0]) * 0.5,
                (lo[1] + hi[1]) * 0.5,
                (lo[2] + hi[2]) * 0.5,
            )
            .length_squared();
            for l in &s.layers {
                let translucent = l.layer == haste_assets::RenderLayer::Translucent;
                draws.push(SectionDraw {
                    layer: l.layer,
                    arena: s.arena,
                    key: if translucent { -d } else { d },
                    first_index: l.first_index,
                    index_count: l.index_count,
                    base_vertex: s.base_vertex,
                    origin: [s.pos.x << 4, s.pos.y << 4, s.pos.z << 4],
                });
            }
        }
        // Opaque front to back (early depth), translucent back to front; grouped by arena.
        draws.sort_unstable_by(|a, b| {
            a.layer
                .cmp(&b.layer)
                .then(a.arena.cmp(&b.arena))
                .then(a.key.total_cmp(&b.key))
        });
        let sections_total = self.sections.len() as u32;
        let drawn = draws.len() as u32;
        self.origins.clear();
        self.origins.extend(draws.iter().map(|d| d.origin));
        if self.origins.len() > self.origin_capacity {
            self.origin_capacity = self.origins.len().next_power_of_two();
            self.origin_buffer = Self::make_origin_buffer(&self.gpu.device, self.origin_capacity);
        }
        if !self.origins.is_empty() {
            self.gpu.queue.write_buffer(
                &self.origin_buffer,
                0,
                bytemuck::cast_slice(&self.origins),
            );
        }
        if self.gpu.multi_draw {
            self.indirect.clear();
            for (i, d) in draws.iter().enumerate() {
                self.indirect.extend_from_slice(&[
                    d.index_count,
                    1,
                    d.first_index,
                    d.base_vertex,
                    i as u32,
                ]);
            }
            if draws.len() > self.indirect_capacity {
                self.indirect_capacity = draws.len().next_power_of_two();
                self.indirect_buffer =
                    Self::make_indirect_buffer(&self.gpu.device, self.indirect_capacity);
            }
            if !self.indirect.is_empty() {
                self.gpu.queue.write_buffer(
                    &self.indirect_buffer,
                    0,
                    bytemuck::cast_slice(&self.indirect),
                );
            }
        }

        // Block entities (chests, shulker boxes) join the entity vertex stream, culled like the
        // sections: the frustum plus the visible mask (and the newly uploaded bonus sections)
        // decide; vanilla compiles block entities into their section's render lists the same way.
        if !frame.block_entities.is_empty() {
            let planes_ref = &planes;
            let bonus = &self.bonus;
            let min_section = self.min_section;
            let column = |c: ChunkPos| self.visible_mask.column(c);
            // The conduit's eye is billboarded to the camera (vanilla `ConduitRenderer` reads
            // `camera.orientation`), which `Camera.setRotation` builds from yaw and pitch.
            let orientation =
                crate::blockentity::camera_orientation(frame.camera.yaw, frame.camera.pitch);
            self.block_entities_drawn = self.entity.append_block_entities(
                frame.block_entities,
                cam,
                |inst| {
                    let pos = inst.pos;
                    let lo = [
                        pos.x as f32 - cam.x as f32,
                        pos.y as f32 - cam.y as f32,
                        pos.z as f32 - cam.z as f32,
                    ];
                    let hi = [lo[0] + 1.0, lo[1] + 1.0, lo[2] + 1.0];
                    if !Self::aabb_visible(planes_ref, lo, hi) {
                        return false;
                    }
                    let section = pos.section();
                    let si = section.y - min_section;
                    let c = ChunkPos::new(section.x, section.z);
                    column(c) & (1u32 << si) != 0 || bonus.contains(&(c, si))
                },
                &mut self.assets,
                orientation,
            );
        } else {
            self.block_entities_drawn = 0;
        }

        self.timings.cull_ms = lap(&mut mark);
        let mut ran = [false; 3];
        let mut draw_calls = 0u32;
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        {
            ran[0] = true;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: fog_color[0] as f64,
                            g: fog_color[1] as f64,
                            b: fog_color[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: self.timer.as_ref().and_then(|t| t.writes(0)),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            draw_calls += self.sky.draw_background(&mut pass);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            pass.set_bind_group(1, &self.atlas_bind_group, &[]);
            pass.set_bind_group(2, &self.lightmap_bind_group, &[]);

            // Layered draw order (RenderLayer Ord: Solid < Cutout < CutoutMipped
            // < Translucent). Section origins come from the instance-rate origin buffer, one
            // instance per draw; buffers are bound once per arena.
            pass.set_vertex_buffer(1, self.origin_buffer.slice(..));
            let mut entities_drawn = false;
            let mut bound_arena = u32::MAX;
            let mut bound_layer = None;
            let mut i = 0;
            while i < draws.len() {
                let d = &draws[i];
                let instance = i;
                // Entities after the opaque and cutout terrain, before translucent.
                if !entities_drawn && d.layer == haste_assets::RenderLayer::Translucent {
                    // Vanilla draws the entity shadows (a translucent, depth-tested pass) with
                    // the entities.
                    draw_calls += self.entity.draw_shadows(&mut pass);
                    draw_calls += self.entity.draw(&mut pass);
                    pass.set_bind_group(1, &self.atlas_bind_group, &[]);
                    entities_drawn = true;
                    bound_layer = None;
                    bound_arena = u32::MAX;
                }
                if bound_layer != Some(d.layer) {
                    let pipeline = match d.layer {
                        haste_assets::RenderLayer::Solid
                        | haste_assets::RenderLayer::CutoutMipped => &self.pipeline_solid,
                        haste_assets::RenderLayer::Cutout => &self.pipeline_cutout,
                        haste_assets::RenderLayer::Translucent => &self.pipeline_translucent,
                    };
                    pass.set_pipeline(pipeline);
                    bound_layer = Some(d.layer);
                }
                if bound_arena != d.arena {
                    let a = &self.arenas.arenas[d.arena as usize];
                    pass.set_vertex_buffer(0, a.vertex.slice(..));
                    pass.set_vertex_buffer(1, self.origin_buffer.slice(..));
                    pass.set_index_buffer(a.index.slice(..), wgpu::IndexFormat::Uint32);
                    bound_arena = d.arena;
                }
                if self.gpu.multi_draw {
                    // One call for the run of draws sharing this layer and arena.
                    let end = draws[i..]
                        .iter()
                        .position(|e| e.layer != d.layer || e.arena != d.arena)
                        .map_or(draws.len(), |n| i + n);
                    let stride = std::mem::size_of::<wgpu::util::DrawIndexedIndirectArgs>();
                    pass.multi_draw_indexed_indirect(
                        &self.indirect_buffer,
                        (i * stride) as u64,
                        (end - i) as u32,
                    );
                    draw_calls += 1;
                    i = end;
                } else {
                    pass.draw_indexed(
                        d.first_index..d.first_index + d.index_count,
                        d.base_vertex as i32,
                        instance as u32..instance as u32 + 1,
                    );
                    draw_calls += 1;
                    i += 1;
                }
            }
            if !entities_drawn {
                draw_calls += self.entity.draw_shadows(&mut pass);
                draw_calls += self.entity.draw(&mut pass);
            }
            draw_calls += self.particles.draw(&mut pass);
            draw_calls += self.overlay.draw(&mut pass);
            draw_calls += self.sky.draw_overlays(&mut pass);
        }

        // First-person hand: its own pass over the world with the depth cleared, so it never
        // sinks into walls.
        if self.entity.has_hand() {
            ran[1] = true;
            // Without the warp: the hand is drawn with the HUD projection in vanilla, which
            // the screen warp never touches.
            if rot_view_proj_plain != rot_view_proj {
                camera_uniform.view_proj = rot_view_proj_plain.to_cols_array_2d();
                self.gpu.queue.write_buffer(
                    &self.camera_uniform,
                    0,
                    bytemuck::bytes_of(&camera_uniform),
                );
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hand"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: self.timer.as_ref().and_then(|t| t.writes(1)),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.hand_camera_bind_group, &[]);
            pass.set_bind_group(2, &self.lightmap_bind_group, &[]);
            draw_calls += self.entity.draw_hand(&mut pass);
        }

        // GUI on top, in its own pass over the non-sRGB view (no depth).
        if !frame.gui.commands.is_empty() || !self.nametags.is_empty() {
            self.gui.prepare(
                frame.gui,
                frame.entities,
                &self.nametags,
                self.size,
                &mut self.assets,
            );
            ran[2] = true;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &gui_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: self.timer.as_ref().and_then(|t| t.writes(2)),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.gui.draw(&mut pass, self.size);
            draw_calls += self.gui.stats.draw_calls;
        }

        // Animated atlas textures: the animation worker writes each tick's
        // changed regions itself; this only collects what landed. Writes the
        // worker finished before this frame's submit (below) are ordered
        // before it, so this frame already sees them.
        self.animation.update();

        // Submit, then present (wgpu 30: present lives on the queue).
        let resolved = match &mut self.timer {
            Some(t) => t.resolve(&mut encoder, ran),
            None => false,
        };
        self.timings.encode_ms = lap(&mut mark);
        self.gpu.queue.submit([encoder.finish()]);
        if let Some(t) = &mut self.timer {
            t.after_submit(resolved);
        }
        self.timings.submit_ms = lap(&mut mark);
        // Submit the screenshot copy before present, so the GPU orders it ahead of the
        // present; the async map lands in `take_capture` a few frames later.
        if capture_frame {
            let src = match (&held_surface_texture, &capture_color) {
                (Some(tex), _) => tex.texture.as_image_copy(),
                (None, Some(color)) => color.as_image_copy(),
                (None, None) => anyhow::bail!("capture frame without a render target"),
            };
            self.start_capture(src);
        }
        if let Some(tex) = held_surface_texture {
            self.gpu.queue.present(tex);
        }
        self.capture_pending = false;
        self.timings.present_ms = lap(&mut mark);

        self.draw_list = draws;
        Ok(FrameStats {
            frame_ms: started.elapsed().as_secs_f32() * 1000.0,
            chunk_sections_drawn: drawn,
            chunk_sections_total: sections_total,
            meshes_pending: self.mesher.pending_len() as u32,
            uploads: self.last_uploads.0,
            upload_bytes: self.last_uploads.1,
            draw_calls,
            block_entities_drawn: self.block_entities_drawn,
        })
    }

    /// Read the last offscreen frame back as an RGBA image (screenshot tests).
    pub fn read_back(&mut self) -> Result<image::RgbaImage> {
        let off = self
            .offscreen
            .as_ref()
            .context("read_back requires an offscreen renderer")?;
        self.read_texture(off.color.as_image_copy(), off.width, off.height)
    }

    /// F2: capture the next rendered frame for `take_capture` (window renderers; the
    /// offscreen path has `read_back`). Ignored while a previous capture is still in
    /// flight: its buffer stays mapped until collected.
    pub fn request_capture(&mut self) {
        if self.capture.is_none() {
            self.capture_pending = true;
        }
    }

    /// Starts the readback of the just-rendered frame: submits the texture copy and
    /// posts `map_async` without blocking; the callback flips the shared status and
    /// `take_capture` collects the image a few frames later.
    fn start_capture(&mut self, texture: wgpu::TexelCopyTextureInfo) {
        let (w, h) = self.size;
        let bytes_per_row = NonZeroU32::new((w * 4).next_multiple_of(256)).unwrap();
        let buffer = std::sync::Arc::new(self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screenshot"),
            size: (bytes_per_row.get() * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        }));
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture,
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row.get()),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.gpu.queue.submit([encoder.finish()]);
        let status = CaptureFlag::default();
        let flag = status.clone();
        buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            flag.set(match r {
                Ok(()) => CapturePhase::Ready,
                Err(_) => CapturePhase::Failed,
            });
        });
        self.capture = Some(Capture {
            buffer,
            status,
            width: w,
            height: h,
            stride: bytes_per_row.get() as usize,
            bgra: matches!(
                self.surface_format,
                wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Bgra8Unorm
            ),
        });
    }

    /// The frame captured after `request_capture`, once its async readback has landed
    /// (a few frames later); `None` while pending or with no capture in flight.
    pub fn take_capture(&mut self) -> Option<anyhow::Result<image::RgbaImage>> {
        let cap = self.capture.as_ref()?;
        // Non-blocking pump: runs the map callback once the copy has finished.
        let _ = self.gpu.device.poll(wgpu::PollType::Poll);
        match cap.status.get() {
            CapturePhase::Pending => None,
            CapturePhase::Failed => {
                self.capture.take();
                Some(Err(anyhow::anyhow!(
                    "mapping the screenshot readback failed"
                )))
            }
            CapturePhase::Ready => {
                let cap = self.capture.take().expect("phase was Ready");
                let image = cap
                    .buffer
                    .slice(..)
                    .get_mapped_range()
                    .context("mapped screenshot range")
                    .map(|data| {
                        texel_rows_to_rgba(&data, cap.width, cap.height, cap.stride, cap.bgra)
                    })
                    .and_then(|pixels| {
                        image::RgbaImage::from_raw(cap.width, cap.height, pixels)
                            .context("screenshot size")
                    });
                cap.buffer.unmap();
                Some(image)
            }
        }
    }

    /// Copies a 4-byte-per-texel texture to a MAP_READ buffer, maps it and unwraps it
    /// into tightly packed RGBA rows.
    fn read_texture(
        &self,
        texture: wgpu::TexelCopyTextureInfo,
        w: u32,
        h: u32,
    ) -> Result<image::RgbaImage> {
        // Rows are padded to wgpu's 256-byte copy alignment.
        let bytes_per_row = NonZeroU32::new((w * 4).next_multiple_of(256)).unwrap();
        let buffer = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (bytes_per_row.get() * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        let bgra = matches!(
            self.surface_format,
            wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Bgra8Unorm
        );
        encoder.copy_texture_to_buffer(
            texture,
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row.get()),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.gpu.queue.submit([encoder.finish()]);

        let (tx, rx) = std::sync::mpsc::channel();
        let buffer = std::sync::Arc::new(buffer);
        let map = buffer.clone();
        map.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        self.gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .ok();
        rx.recv()
            .context("waiting for the readback map")?
            .context("mapping the readback buffer")?;

        let data = map.get_mapped_range(..).context("mapped range")?.to_vec();
        map.unmap();

        let pixels = texel_rows_to_rgba(&data, w, h, bytes_per_row.get() as usize, bgra);
        Ok(image::RgbaImage::from_raw(w, h, pixels).expect("readback size"))
    }
}

/// Unpacks a GPU copy buffer into tightly packed RGBA rows: drops the 256-byte row
/// padding and, for BGRA surfaces, swaps the red and blue channels.
fn texel_rows_to_rgba(data: &[u8], w: u32, h: u32, stride: usize, bgra: bool) -> Vec<u8> {
    let row = w as usize * 4;
    let mut pixels = Vec::with_capacity(row * h as usize);
    for y in 0..h as usize {
        let src = &data[y * stride..y * stride + row];
        if bgra {
            for px in src.as_chunks::<4>().0 {
                pixels.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
            }
        } else {
            pixels.extend_from_slice(src);
        }
    }
    pixels
}

/// An owned display handle: the window Arc keeps the underlying connection
/// alive; the handle is fetched from the window on demand.
struct DisplayHandleWrapper {
    #[allow(dead_code)]
    window: Arc<winit::window::Window>,
}

impl core::fmt::Debug for DisplayHandleWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DisplayHandleWrapper")
            .finish_non_exhaustive()
    }
}

impl winit::raw_window_handle::HasDisplayHandle for DisplayHandleWrapper {
    fn display_handle(
        &self,
    ) -> Result<winit::raw_window_handle::DisplayHandle<'_>, winit::raw_window_handle::HandleError>
    {
        self.window.display_handle()
    }
}
