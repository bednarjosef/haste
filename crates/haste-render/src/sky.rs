// Sky and weather: the sky dome colour by time of day with the sunrise glow, sun, moon phases,
// stars, flat clouds from `clouds.png`, rain columns, and the Nether/End backgrounds; plus the
// fog colour and daylight factor the terrain uses.
//
// The time-of-day curves follow minecraft.wiki "Daylight cycle": the celestial angle is
// (2d + 0.5 - cos(pi d) / 2) / 3 with d = frac(time / 24000 - 0.25); sky brightness is
// clamp(2 cos(2 pi angle) + 0.5); moon phase = day mod 8 (full moon first); default overworld
// sky 0x78A7FF and fog 0xC0D8FF; Nether fog 0x330808; End sky tinted 0x282828. Sunrise colour,
// star brightness and the horizon blend are approximations of the observed look.

use std::sync::Arc;

use glam::{Mat4, Vec3};
use haste_assets::Assets;
use haste_core::game::DimensionInfo;
use haste_core::Identifier;

use crate::fog::FogData;
use crate::{Camera, SkyState};

const ATLAS_SIZE: u32 = 512;
/// Cloud height in 26.3 (blocks).
pub const CLOUD_HEIGHT: f32 = 192.33;

const MOON_PHASES: [&str; 8] = [
    "full_moon",
    "waning_gibbous",
    "third_quarter",
    "waning_crescent",
    "new_moon",
    "waxing_crescent",
    "first_quarter",
    "waxing_gibbous",
];

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SkyUniforms {
    view_proj: [[f32; 4]; 4],
    rot_view_proj: [[f32; 4]; 4],
    inv_rot_view_proj: [[f32; 4]; 4],
    celestial: [[f32; 4]; 4],
    sky_color: [f32; 4],
    fog_color: [f32; 4],
    sunrise_color: [f32; 4],
    sunrise_dir: [f32; 4],
    cam_pos: [f32; 4],
    params: [f32; 4],
    clouds: [f32; 4],
    /// Vanilla fog bands for the sky dome (0..sky_end on both distances) and the weather
    /// columns (the terrain bands): env start/end, rd start/end, sky end, cloud end.
    fog_bands: [f32; 4],
    fog_bands2: [f32; 4],
    /// The sunrise/sunset fan's rotation (camera rotation frame).
    sunrise_matrix: [[f32; 4]; 4],
    end_rect: [u32; 4],
    cloud_rect: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct SkyVertex {
    pos: [f32; 3],
    uv: [f32; 2],
    color: [f32; 4],
    rect: [u16; 4],
}

impl SkyVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        0 => Float32x3, 1 => Float32x2, 2 => Float32x4, 3 => Uint16x4
    ];
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<SkyVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &Self::ATTRIBUTES,
    };
}

/// Which background a dimension draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkyKind {
    Overworld,
    /// Fog colour only (the Nether, and skyless custom dimensions).
    None,
    End,
}

impl SkyKind {
    pub fn of(dim: &DimensionInfo) -> SkyKind {
        match dim.dimension_type.path() {
            "the_nether" => SkyKind::None,
            "the_end" => SkyKind::End,
            "overworld" | "overworld_caves" => SkyKind::Overworld,
            _ if dim.has_skylight => SkyKind::Overworld,
            _ => SkyKind::None,
        }
    }
}

/// Colours and factors of one frame (sRGB colours, before conversion).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyColors {
    pub celestial_angle: f32,
    /// 0 at night .. 1 at day.
    pub brightness: f32,
    pub sky: [f32; 3],
    pub fog: [f32; 3],
    /// Sunrise/sunset glow colour and strength (0 = none).
    pub sunrise: [f32; 4],
    /// +1 east / -1 west: where the glow sits.
    pub sunrise_side: f32,
    pub star_brightness: f32,
    pub moon_phase: usize,
}

pub fn celestial_angle(time_of_day: i64) -> f32 {
    let d = ((time_of_day.rem_euclid(24000)) as f64 / 24000.0 - 0.25).rem_euclid(1.0);
    ((d * 2.0 + (0.5 - (d * std::f64::consts::PI).cos() / 2.0)) / 3.0) as f32
}

fn rgb(c: u32) -> [f32; 3] {
    [
        ((c >> 16) & 0xFF) as f32 / 255.0,
        ((c >> 8) & 0xFF) as f32 / 255.0,
        (c & 0xFF) as f32 / 255.0,
    ]
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn linear(c: [f32; 3]) -> [f32; 3] {
    c.map(srgb_to_linear)
}

/// The frame's sky and fog colours (sRGB) for a sky kind; `view_distance` in chunks.
pub fn sky_colors(sky: &SkyState, kind: SkyKind, view_distance: f32) -> SkyColors {
    let angle = celestial_angle(sky.time_of_day);
    let c = (angle * std::f32::consts::TAU).cos();
    let brightness = (c * 2.0 + 0.5).clamp(0.0, 1.0);
    let rain = sky.rain.clamp(0.0, 1.0);
    let moon_phase = (sky.time_of_day.div_euclid(24000)).rem_euclid(8) as usize;
    match kind {
        SkyKind::None => {
            let fog = if sky.dimension_has_sky {
                rgb(0x000000)
            } else {
                rgb(0x330808)
            };
            SkyColors {
                celestial_angle: angle,
                brightness: 1.0,
                sky: fog,
                fog,
                sunrise: [0.0; 4],
                sunrise_side: 1.0,
                star_brightness: 0.0,
                moon_phase,
            }
        }
        SkyKind::End => SkyColors {
            celestial_angle: angle,
            brightness: 1.0,
            sky: rgb(0x282828),
            fog: rgb(0x0A0A0A),
            sunrise: [0.0; 4],
            sunrise_side: 1.0,
            star_brightness: 0.0,
            moon_phase,
        },
        SkyKind::Overworld => {
            let base_sky = rgb(0x78A7FF);
            let base_fog = rgb(0xC0D8FF);
            let mut sky_c = base_sky.map(|v| v * brightness);
            let mut fog = [
                base_fog[0] * (brightness * 0.94 + 0.06),
                base_fog[1] * (brightness * 0.94 + 0.06),
                base_fog[2] * (brightness * 0.91 + 0.09),
            ];
            // The farther the view distance, the more the horizon fog takes the sky colour.
            let k = 1.0 - (0.25 + 0.75 * (view_distance / 32.0).clamp(0.0, 1.0)).powf(0.25);
            for i in 0..3 {
                fog[i] += (sky_c[i] - fog[i]) * k;
            }
            // The weather attribute layers (vanilla `WeatherAttributes.RAIN`/`THUNDER` over
            // SKY_COLOR): rain blends the colour toward its greyscale times 0.6 by a factor
            // of 0.75 (`ColorModifier.BLEND_TO_GRAY_RGB`), thunder toward the greyscale
            // times 0.24 by 0.94. The rain layer's weight is the rain level minus the
            // thunder level (which is itself scaled by the rain level), the thunder layer's
            // the thunder level.
            let thunder = (sky.thunder * rain).clamp(0.0, 1.0);
            let rain_only = (rain - thunder).clamp(0.0, 1.0);
            let blend_to_gray = |col: [f32; 3], brightness: f32, factor: f32| {
                let grey = col[0] * 0.3 + col[1] * 0.59 + col[2] * 0.11;
                [
                    col[0] + (grey * brightness - col[0]) * factor,
                    col[1] + (grey * brightness - col[1]) * factor,
                    col[2] + (grey * brightness - col[2]) * factor,
                ]
            };
            if rain_only > 0.0 {
                let target = blend_to_gray(sky_c, 0.6, 0.75);
                sky_c = [
                    sky_c[0] + (target[0] - sky_c[0]) * rain_only,
                    sky_c[1] + (target[1] - sky_c[1]) * rain_only,
                    sky_c[2] + (target[2] - sky_c[2]) * rain_only,
                ];
            }
            if thunder > 0.0 {
                let target = blend_to_gray(sky_c, 0.24, 0.94);
                sky_c = [
                    sky_c[0] + (target[0] - sky_c[0]) * thunder,
                    sky_c[1] + (target[1] - sky_c[1]) * thunder,
                    sky_c[2] + (target[2] - sky_c[2]) * thunder,
                ];
            }
            // The sunrise/sunset colour's alpha takes the weather layers' RGB multiply
            // (rain 0.6, thunder 0.3 on the alpha channel of the multipliers
            // (0.5, 0.5, 0.6)/(0.25, 0.25, 0.3)).
            let weather_a = 1.0 - rain_only * 0.4 - thunder * 0.7;
            let sunrise = if (-0.4..=0.4).contains(&c) {
                let f = c / 0.4 * 0.5 + 0.5;
                let a = (1.0 - (1.0 - (f * std::f32::consts::PI).sin()) * 0.99).powi(2);
                [f * 0.3 + 0.7, f * f * 0.7 + 0.2, 0.2, a * weather_a]
            } else {
                [0.0; 4]
            };
            let s = (angle * std::f32::consts::TAU).sin();
            // The weather layers also set the star brightness to 0 (`WeatherAttributes`
            // `.set(STAR_BRIGHTNESS, 0)` applied by the levels).
            let stars = (1.0 - (c * 2.0 + 0.25)).clamp(0.0, 1.0)
                * 0.5
                * (1.0 - rain_only)
                * (1.0 - thunder);
            SkyColors {
                celestial_angle: angle,
                brightness,
                sky: sky_c,
                fog,
                sunrise,
                sunrise_side: if s < 0.0 { 1.0 } else { -1.0 },
                star_brightness: stars,
                moon_phase,
            }
        }
    }
}

/// Celestial rotation: sun overhead at noon, rising in the east (+x), setting in the west.
pub fn celestial_matrix(angle: f32) -> Mat4 {
    Mat4::from_rotation_y(-std::f32::consts::FRAC_PI_2)
        * Mat4::from_rotation_x(angle * std::f32::consts::TAU)
}

pub struct SkyRenderer {
    queue: Arc<wgpu::Queue>,
    device: Arc<wgpu::Device>,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// The environment atlas's view (sun, moons, clouds, rain, snow, end sky), lent to the
    /// weather pass at startup.
    atlas_view: wgpu::TextureView,
    background: wgpu::RenderPipeline,
    celestial: wgpu::RenderPipeline,
    clouds: wgpu::RenderPipeline,
    /// The sky disc fan (vanilla `SkyRenderer.renderSkyDisc`).
    disc: wgpu::RenderPipeline,
    /// The sunrise/sunset fan (vanilla `renderSunriseAndSunset`).
    sunrise: wgpu::RenderPipeline,
    stars: wgpu::Buffer,
    star_vertices: u32,
    disc_buffer: wgpu::Buffer,
    disc_vertices: u32,
    sunrise_fan: wgpu::Buffer,
    sunrise_fan_vertices: u32,
    dynamic: wgpu::Buffer,
    dynamic_capacity: usize,
    vertices: Vec<SkyVertex>,
    /// Sun + moon vertex count at the start of `vertices`.
    celestial_count: u32,
    rects: AtlasRects,
    pub kind: SkyKind,
    pub colors: SkyColors,
}

#[derive(Clone, Copy)]
struct AtlasRects {
    sun: [u32; 4],
    moon: [[u32; 4]; 8],
    clouds: [u32; 4],
    rain: [u32; 4],
    snow: [u32; 4],
    end_sky: [u32; 4],
}

impl SkyRenderer {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        target_format: wgpu::TextureFormat,
        assets: &mut Assets,
    ) -> SkyRenderer {
        // Environment atlas: sun and moons in the top rows, rain/snow/end sky to the right,
        // clouds below.
        let mut atlas = image::RgbaImage::new(ATLAS_SIZE, ATLAS_SIZE);
        let mut put = |id: &str, x: u32, y: u32| -> [u32; 4] {
            let img = assets
                .texture(&Identifier::minecraft(id))
                .unwrap_or_else(|e| {
                    tracing::warn!("sky texture {id}: {e:#}");
                    image::RgbaImage::from_pixel(16, 16, image::Rgba([255, 0, 255, 255]))
                });
            let (w, h) = img.dimensions();
            image::imageops::replace(&mut atlas, &img, x as i64, y as i64);
            [x, y, w, h]
        };
        let sun = put("environment/celestial/sun", 0, 0);
        let moon: [[u32; 4]; 8] = std::array::from_fn(|i| {
            put(
                &format!("environment/celestial/moon/{}", MOON_PHASES[i]),
                32 + 32 * i as u32,
                0,
            )
        });
        let rain = put("environment/rain", 288, 0);
        let snow = put("environment/snow", 352, 0);
        // Past the weather tiles (rain 288..352, snow 352..416): the End sky must not
        // overlap them, or the weather pass samples its purple where snow flakes should be.
        let end_sky = put("environment/end_sky", 448, 0);
        let clouds = put("environment/clouds", 0, 256);
        let rects = AtlasRects {
            sun,
            moon,
            clouds,
            rain,
            snow,
            end_sky,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sky-atlas"),
            size: wgpu::Extent3d {
                width: ATLAS_SIZE,
                height: ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            atlas.as_raw(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * ATLAS_SIZE),
                rows_per_image: Some(ATLAS_SIZE),
            },
            wgpu::Extent3d {
                width: ATLAS_SIZE,
                height: ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
        );
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sky-uniforms"),
            size: std::mem::size_of::<SkyUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let atlas_view = texture.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky-bind"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sky.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |label: &str,
                    vs: &str,
                    fs: &str,
                    buffers: &[Option<wgpu::VertexBufferLayout>],
                    blend: wgpu::BlendState,
                    depth_test: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth24Plus,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(if depth_test {
                        wgpu::CompareFunction::Less
                    } else {
                        wgpu::CompareFunction::Always
                    }),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        let background = make(
            "sky-background",
            "vs_background",
            "fs_background",
            &[],
            wgpu::BlendState::REPLACE,
            false,
        );
        let celestial = make(
            "sky-celestial",
            "vs_celestial",
            "fs_celestial",
            &[Some(SkyVertex::LAYOUT)],
            additive,
            false,
        );
        let clouds = make(
            "sky-clouds",
            "vs_clouds",
            "fs_clouds",
            &[],
            wgpu::BlendState::ALPHA_BLENDING,
            true,
        );
        let disc = make(
            "sky-disc",
            "vs_disc",
            "fs_disc",
            &[Some(SkyVertex::LAYOUT)],
            wgpu::BlendState::REPLACE,
            false,
        );
        let sunrise = make(
            "sky-sunrise",
            "vs_sunrise",
            "fs_sunrise",
            &[Some(SkyVertex::LAYOUT)],
            wgpu::BlendState::ALPHA_BLENDING,
            false,
        );
        let star_list = star_vertices();
        let stars = wgpu::util::DeviceExt::create_buffer_init(
            device.as_ref(),
            &wgpu::util::BufferInitDescriptor {
                label: Some("stars"),
                contents: bytemuck::cast_slice(&star_list),
                usage: wgpu::BufferUsages::VERTEX,
            },
        );
        let disc_list = disc_vertices();
        let disc_buffer = wgpu::util::DeviceExt::create_buffer_init(
            device.as_ref(),
            &wgpu::util::BufferInitDescriptor {
                label: Some("sky-disc"),
                contents: bytemuck::cast_slice(&disc_list),
                usage: wgpu::BufferUsages::VERTEX,
            },
        );
        let sunrise_list = sunrise_fan_vertices();
        let sunrise_fan = wgpu::util::DeviceExt::create_buffer_init(
            device.as_ref(),
            &wgpu::util::BufferInitDescriptor {
                label: Some("sunrise-fan"),
                contents: bytemuck::cast_slice(&sunrise_list),
                usage: wgpu::BufferUsages::VERTEX,
            },
        );
        let dynamic_capacity = 4096;
        let dynamic = make_buffer(&device, dynamic_capacity);
        SkyRenderer {
            queue,
            device,
            uniforms,
            bind_group,
            atlas_view,
            background,
            celestial,
            clouds,
            disc,
            sunrise,
            stars,
            star_vertices: star_list.len() as u32,
            disc_buffer,
            disc_vertices: disc_list.len() as u32,
            sunrise_fan,
            sunrise_fan_vertices: sunrise_list.len() as u32,
            dynamic,
            dynamic_capacity,
            vertices: Vec::new(),
            celestial_count: 0,
            rects,
            kind: SkyKind::Overworld,
            colors: sky_colors(
                &SkyState {
                    time_of_day: 6000,
                    rain: 0.0,
                    dimension_has_sky: true,
                    fog_start: 0.0,
                    fog_end: 1.0,
                    ..Default::default()
                },
                SkyKind::Overworld,
                12.0,
            ),
        }
    }

    /// The environment atlas's texture view (shared with the weather pass).
    pub fn atlas_view(&self) -> &wgpu::TextureView {
        &self.atlas_view
    }

    /// The rain and snow rectangles in the environment atlas (x, y, w, h), for the weather
    /// pass.
    pub fn weather_rects(&self) -> ([u32; 4], [u32; 4]) {
        (self.rects.rain, self.rects.snow)
    }

    /// Computes this frame's colours and geometry and uploads the uniforms. `fog` is the
    /// frame's vanilla fog data (renderer.rs `frame_fog`), `fog_color` its sRGB colour.
    pub fn prepare(
        &mut self,
        camera: &Camera,
        sky: &SkyState,
        view_proj: Mat4,
        rot_view_proj: Mat4,
        fog: &FogData,
        fog_color: [f32; 3],
    ) {
        let kind = if sky.dimension_has_sky || self.kind != SkyKind::Overworld {
            self.kind
        } else {
            SkyKind::None
        };
        let colors = sky_colors(sky, kind, sky.fog_end / 16.0);
        // Overworld sky/fog colours take the vanilla timeline values (the fog colour is
        // the exact environment colour; the dome colours keep our sunrise approximation
        // fed with the exact tint and strength).
        let mut colors = colors;
        // The horizon colour is the exact environment fog colour (all dimensions), so the dome
        // and the terrain fog agree.
        colors.fog = fog_color;
        if kind == SkyKind::Overworld {
            let glow = crate::fog::sunrise_sunset_color(sky.time_of_day);
            colors.sunrise = [glow[0], glow[1], glow[2], glow[3]];
            colors.sunrise_side = crate::fog::sun_glow_side(sky.time_of_day);
            colors.sky = crate::fog::overworld_sky_color(
                sky.time_of_day,
                sky.rain.clamp(0.0, 1.0),
                (sky.thunder * sky.rain.clamp(0.0, 1.0)).clamp(0.0, 1.0),
            );
        }
        self.colors = colors;
        let rain = sky.rain.clamp(0.0, 1.0);
        let cam = camera.pos.as_vec3();

        self.vertices.clear();
        if kind == SkyKind::Overworld {
            // Sun (size 30 at distance 100) and moon (20, opposite), fading with rain.
            let alpha = 1.0 - rain;
            push_quad(
                &mut self.vertices,
                100.0,
                30.0,
                self.rects.sun,
                false,
                [1.0, 1.0, 1.0, alpha],
            );
            // Vanilla's moon quad (`SkyRenderer.buildMoonPhases`) has both U and V reversed
            // and is turned 180 degrees about X by the moon angle; drawn here at y -100 with
            // the sun's rotation that leaves only U mirrored.
            push_quad(
                &mut self.vertices,
                -100.0,
                20.0,
                self.rects.moon[colors.moon_phase],
                true,
                [1.0, 1.0, 1.0, alpha],
            );
        }
        self.celestial_count = self.vertices.len() as u32;
        if self.vertices.len() > self.dynamic_capacity {
            self.dynamic_capacity = self.vertices.len().next_power_of_two();
            self.dynamic = make_buffer(&self.device, self.dynamic_capacity);
        }
        if !self.vertices.is_empty() {
            self.queue
                .write_buffer(&self.dynamic, 0, bytemuck::cast_slice(&self.vertices));
        }

        let lin = |c: [f32; 3]| {
            let l = linear(c);
            [l[0], l[1], l[2], 1.0]
        };
        let sunrise = linear([colors.sunrise[0], colors.sunrise[1], colors.sunrise[2]]);
        // The clouds take their own weather layers (`WeatherAttributes`: blend to the
        // greyscale times 0.6 by a factor of 0.5 in rain, times 0.095 by 0.94 in thunder).
        // For the scalar brightness (a grey value) that is v * (1 - factor + factor *
        // brightness): rain keeps 0.7 of it, thunder 0.2856.
        let thunder = (sky.thunder * rain).clamp(0.0, 1.0);
        let rain_only = (rain - thunder).clamp(0.0, 1.0);
        let cloud_value = colors.brightness * 0.9 + 0.1;
        let cloud_value = cloud_value * (1.0 - rain_only * 0.3) * (1.0 - thunder * 0.7144);
        let cloud_brightness = srgb_to_linear(cloud_value);
        let u = SkyUniforms {
            view_proj: view_proj.to_cols_array_2d(),
            rot_view_proj: rot_view_proj.to_cols_array_2d(),
            inv_rot_view_proj: rot_view_proj.inverse().to_cols_array_2d(),
            celestial: celestial_matrix(colors.celestial_angle).to_cols_array_2d(),
            sky_color: lin(colors.sky),
            fog_color: lin(colors.fog),
            sunrise_color: [sunrise[0], sunrise[1], sunrise[2], colors.sunrise[3]],
            sunrise_dir: [colors.sunrise_side, 0.0, 0.0, colors.star_brightness],
            cam_pos: [cam.x, cam.y, cam.z, 0.0],
            params: [
                // Clouds drift east at 0.03 blocks per tick (approximate: world time stands in
                // for game time).
                (sky.time_of_day as f64 * 0.03).rem_euclid(256.0 * 12.0) as f32,
                sky.fog_end,
                CLOUD_HEIGHT,
                match kind {
                    SkyKind::Overworld => 0.0,
                    SkyKind::None => 1.0,
                    SkyKind::End => 2.0,
                },
            ],
            clouds: [cloud_brightness, 0.8, 0.0, 0.0],
            fog_bands: [
                fog.environmental_start,
                fog.environmental_end,
                fog.render_distance_start,
                fog.render_distance_end,
            ],
            fog_bands2: [fog.sky_end, fog.cloud_end, 0.0, 0.0],
            sunrise_matrix: sunrise_matrix(colors.sunrise_side, colors.sunrise[3])
                .to_cols_array_2d(),
            end_rect: self.rects.end_sky,
            cloud_rect: self.rects.clouds,
        };
        self.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));
    }

    /// The fog colour terrain should use (linear).
    pub fn fog_linear(&self) -> [f32; 3] {
        linear(self.colors.fog)
    }

    /// Sky light multiplier for the terrain lightmap (night dims sky light, not block light).
    pub fn daylight(&self) -> f32 {
        match self.kind {
            SkyKind::Overworld => self.colors.brightness,
            _ => 1.0,
        }
    }

    /// Background, stars, sun and moon: first in the main pass, before terrain.
    pub fn draw_background<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) -> u32 {
        pass.set_bind_group(0, &self.bind_group, &[]);
        let mut calls = 0;
        if self.kind == SkyKind::Overworld {
            // The sky disc fan over the fog-coloured clear; then sun, moon, stars and the
            // sunrise/sunset fan (vanilla order: disc, glow, celestial bodies).
            pass.set_pipeline(&self.disc);
            pass.set_vertex_buffer(0, self.disc_buffer.slice(..));
            pass.draw(0..self.disc_vertices, 0..1);
            calls += 1;
            pass.set_pipeline(&self.celestial);
            if self.colors.star_brightness > 0.0 {
                pass.set_vertex_buffer(0, self.stars.slice(..));
                pass.draw(0..self.star_vertices, 0..1);
                calls += 1;
            }
            if self.celestial_count > 0 {
                pass.set_vertex_buffer(0, self.dynamic.slice(..));
                pass.draw(0..self.celestial_count, 0..1);
                calls += 1;
            }
            if self.colors.sunrise[3] > 0.001 {
                pass.set_pipeline(&self.sunrise);
                pass.set_vertex_buffer(0, self.sunrise_fan.slice(..));
                pass.draw(0..self.sunrise_fan_vertices, 0..1);
                calls += 1;
            }
        } else {
            pass.set_pipeline(&self.background);
            pass.draw(0..3, 0..1);
            calls += 1;
        }
        calls
    }

    /// Clouds: last in the main pass, after translucent terrain (the weather columns follow
    /// through the weather pass's own draw).
    pub fn draw_overlays<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) -> u32 {
        let mut calls = 0;
        if self.kind != SkyKind::Overworld {
            return 0;
        }
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_pipeline(&self.clouds);
        pass.draw(0..6, 0..1);
        calls += 1;
        calls
    }
}

fn make_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sky-vertices"),
        size: (capacity * std::mem::size_of::<SkyVertex>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// A horizontal celestial quad at height `y` (celestial space) of half-size `half`; the
/// texture's U runs along +x, or along -x with `mirror_u`.
fn push_quad(
    out: &mut Vec<SkyVertex>,
    y: f32,
    half: f32,
    rect: [u32; 4],
    mirror_u: bool,
    color: [f32; 4],
) {
    let r = rect.map(|v| v as f32);
    let (u_neg, u_pos) = if mirror_u {
        (r[0] + r[2], r[0])
    } else {
        (r[0], r[0] + r[2])
    };
    let corners = [
        ([-half, y, -half], [u_neg, r[1]]),
        ([half, y, -half], [u_pos, r[1]]),
        ([half, y, half], [u_pos, r[1] + r[3]]),
        ([-half, y, half], [u_neg, r[1] + r[3]]),
    ];
    let rect = rect.map(|v| v as u16);
    for i in [0usize, 1, 2, 0, 2, 3] {
        out.push(SkyVertex {
            pos: corners[i].0,
            uv: corners[i].1,
            color,
            rect,
        });
    }
}

/// 1500 small star quads on a sphere of radius 100 (positions from our own generator, not
/// vanilla's seed: the pattern differs).
fn star_vertices() -> Vec<SkyVertex> {
    let mut state = 10842u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f32 / (1u64 << 53) as f32
    };
    let mut out = Vec::with_capacity(1500 * 6);
    for _ in 0..1500 {
        let d = Vec3::new(next() * 2.0 - 1.0, next() * 2.0 - 1.0, next() * 2.0 - 1.0);
        let size = 0.15 + next() * 0.1;
        let len2 = d.length_squared();
        if !(0.01..1.0).contains(&len2) {
            continue;
        }
        let centre = d.normalize() * 100.0;
        let n = centre.normalize();
        let tangent = if n.y.abs() < 0.99 {
            n.cross(Vec3::Y).normalize()
        } else {
            n.cross(Vec3::X).normalize()
        };
        let spin = next() * std::f32::consts::TAU;
        let bitangent = n.cross(tangent);
        let a = (tangent * spin.cos() + bitangent * spin.sin()) * size;
        let b = n.cross(a);
        let corners = [
            centre - a - b,
            centre + a - b,
            centre + a + b,
            centre - a + b,
        ];
        for i in [0usize, 1, 2, 0, 2, 3] {
            out.push(SkyVertex {
                pos: corners[i].to_array(),
                uv: [0.0; 2],
                color: [1.0; 4],
                rect: [0; 4],
            });
        }
    }
    out
}

/// The sky disc: a triangle fan (center (0, 16, 0), ring of radius 512 at eight compass
/// points, vanilla `SkyRenderer.buildSkyDisc`). The fragment fog is the spherical distance
/// over the sky fog end, interpolated per vertex like vanilla's varyings.
fn disc_vertices() -> Vec<SkyVertex> {
    let mut out = Vec::with_capacity(8 * 6);
    let center = [0.0f32, 16.0, 0.0];
    for i in 0..8 {
        let a0 = (i as f32 - 2.0) * std::f32::consts::FRAC_PI_4;
        let a1 = (i as f32 - 1.0) * std::f32::consts::FRAC_PI_4;
        // Vanilla's ring runs i = -180..=180 step 45 with x = +512 (top disc): the same
        // eight segments, in radians.
        let r = |a: f32| [512.0 * a.cos(), 16.0, 512.0 * a.sin()];
        let corners = [center, r(a0), r(a1)];
        for j in [0usize, 1, 2] {
            out.push(SkyVertex {
                pos: corners[j],
                uv: [0.0; 2],
                color: [1.0; 4],
                rect: [0; 4],
            });
        }
    }
    out
}

/// The sunrise/sunset fan: center (0, 100, 0), sixteen ring vertices at radius 120 flattened
/// to 40 in z (vanilla `SkyRenderer.buildSunriseFan`), white alpha ramping 1 at the center
/// to 0 at the ring; the fragment multiplies in the timeline colour.
fn sunrise_fan_vertices() -> Vec<SkyVertex> {
    let mut out = Vec::with_capacity(16 * 6);
    let center = SkyVertex {
        pos: [0.0, 100.0, 0.0],
        uv: [0.0; 2],
        color: [1.0, 1.0, 1.0, 1.0],
        rect: [0; 4],
    };
    let ring = |i: usize| {
        let a = i as f32 * std::f32::consts::TAU / 16.0;
        SkyVertex {
            pos: [a.sin() * 120.0, a.cos() * 120.0, -a.cos() * 40.0],
            uv: [0.0; 2],
            color: [1.0, 1.0, 1.0, 0.0],
            rect: [0; 4],
        }
    };
    for i in 0..16 {
        let corners = [center, ring(i), ring(i + 1)];
        for j in [0usize, 1, 2] {
            out.push(corners[j]);
        }
    }
    out
}

/// The sunrise/sunset fan's pose (vanilla `renderSunriseAndSunset`): rotate 90 degrees about
/// X, then 90 (glow west, sun descending) or 270 (glow east, sun rising) about Z. The
/// z-thickness is scaled by the glow's alpha.
fn sunrise_matrix(side: f32, alpha: f32) -> Mat4 {
    let z_angle = if side < 0.0 { 90.0f32 } else { 270.0 };
    Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2)
        * Mat4::from_rotation_z(z_angle.to_radians())
        * Mat4::from_scale(Vec3::new(1.0, 1.0, alpha.max(0.001)))
}

#[cfg(test)]
mod tests;
