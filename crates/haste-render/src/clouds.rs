// The clouds (vanilla `CloudRenderer`): 12x12x4 boxes built from the `clouds.png`
// cells around the camera (`mesh.rs`), drawn as one batched mesh with per-face shades
// (0.7 bottom, 1.0 top, 0.8 north/south, 0.9 west/east). The mesh is
// camera-cell-relative and is rebuilt only when the camera crosses a cell boundary,
// moves between above, inside and below the cloud layer, or the cloud mode changes;
// every other frame only the small uniform (the offset inside the cell) is uploaded.
// The mode is vanilla's `Options.cloudStatus` (options.txt `renderClouds`): OFF draws
// no pass at all, FAST the flat cells of `flat.rs`, FANCY (vanilla's default) these
// extruded boxes.

use std::sync::Arc;

use crate::fog::FogData;
use crate::{Camera, SkyState};

mod flat;
mod mesh;
#[cfg(test)]
mod tests;

use mesh::build_mesh;
pub use mesh::{CloudTexture, RelativeCameraPos};

/// Vanilla `net.minecraft.client.CloudStatus` (options.txt `renderClouds`): OFF skips the
/// pass, FAST draws the flat cells, FANCY the extruded boxes. Default FANCY, like
/// `Options.cloudStatus`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CloudStatus {
    Off,
    Fast,
    /// Vanilla's default (`Options.cloudStatus`).
    #[default]
    Fancy,
}

/// One cloud cell is 12x12 blocks wide and 4 tall (vanilla `CELL_SIZE_IN_BLOCKS` and the
/// extruded cell's height).
pub const CELL_SIZE: f32 = 12.0;
/// The height of an extruded cloud cell in blocks.
pub const CELL_HEIGHT: f32 = 4.0;
/// Vanilla's cloud grid is shifted this many blocks in z (`cloudZ = camera.z + 3.96`).
const CLOUD_Z_SHIFT: f64 = 3.96;
/// Cloud height in 26.3 (blocks): the overworld `visual/cloud_height` attribute's default.
pub const CLOUD_HEIGHT: f32 = 192.33;
/// The overworld `visual/cloud_color` attribute (`DimensionTypes` overworld: white at
/// alpha 0.8); the day timeline and the weather layers modify it.
pub const OVERWORLD_CLOUD_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 0.8];

/// The clouds' eastward drift this frame (blocks): vanilla `CloudRenderer.prepare` takes
/// `(gameTime % (width * 400)) + partialTicks` and moves the grid by `0.03` blocks per
/// tick of it — game time, so a frozen time of day still moves the clouds.
pub fn cloud_drift(game_time: i64, partial_tick: f32) -> f32 {
    (game_time.rem_euclid(256 * 400) as f32 + partial_tick) * 0.030000001
}

/// The frame's cloud colour (sRGB, alpha included): the overworld `visual/cloud_color`
/// attribute (white, alpha 0.8) under the day timeline's night multiplier
/// (`cloud_color` MULTIPLY_ARGB, the bluish (0.1, 0.1, 0.15) at night), then the weather
/// layers (`WeatherAttributes`: rain blends toward the greyscale times 0.24 by 0.5,
/// thunder toward the greyscale times 0.095 by 0.94, alpha untouched —
/// `ColorModifier.BLEND_TO_GRAY_ARGB`). Rain level is the frame's rain level, thunder
/// the thunder level scaled by it (a dry thunderstorm cannot darken).
pub fn cloud_color(time_of_day: i64, rain_level: f32, thunder_level: f32) -> [f32; 4] {
    let night = crate::fog::cloud_night_multiplier(time_of_day);
    let mut c = [
        OVERWORLD_CLOUD_COLOR[0] * night[0],
        OVERWORLD_CLOUD_COLOR[1] * night[1],
        OVERWORLD_CLOUD_COLOR[2] * night[2],
        OVERWORLD_CLOUD_COLOR[3],
    ];
    let blend_to_gray = |c: [f32; 4], brightness: f32, factor: f32| {
        let grey = c[0] * 0.3 + c[1] * 0.59 + c[2] * 0.11;
        [
            c[0] + (grey * brightness - c[0]) * factor,
            c[1] + (grey * brightness - c[1]) * factor,
            c[2] + (grey * brightness - c[2]) * factor,
            c[3],
        ]
    };
    let rain_only = (rain_level - thunder_level).clamp(0.0, 1.0);
    if rain_only > 0.0 {
        let target = blend_to_gray(c, 0.24, 0.5);
        c = [
            c[0] + (target[0] - c[0]) * rain_only,
            c[1] + (target[1] - c[1]) * rain_only,
            c[2] + (target[2] - c[2]) * rain_only,
            c[3],
        ];
    }
    if thunder_level > 0.0 {
        let target = blend_to_gray(c, 0.095, 0.94);
        c = [
            c[0] + (target[0] - c[0]) * thunder_level,
            c[1] + (target[1] - c[1]) * thunder_level,
            c[2] + (target[2] - c[2]) * thunder_level,
            c[3],
        ];
    }
    c
}

/// One mesh vertex: the camera-cell-relative position in blocks (the per-frame offset
/// inside the cell is added in the shader) and the face shade.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CloudVertex {
    pos: [f32; 3],
    shade: f32,
}

/// Uniforms for the clouds pass. The trailing padding rounds the struct to the WGSL
/// uniform layout's size (the `vec3` members force 16-byte alignment).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CloudUniforms {
    view_proj: [[f32; 4]; 4],
    /// The offset of the mesh inside the camera's cell: (-xInCell, cloud bottom relative
    /// to the camera, -zInCell) — vanilla's `CloudOffset`.
    offset: [f32; 3],
    _pad0: f32,
    /// The cloud colour (sRGB; the shader linearises shade * colour) and alpha.
    color: [f32; 4],
    /// The cloud fog end distance.
    fog_end: f32,
    _pad1: [f32; 7],
}

/// The clouds pass (vanilla `CloudRenderer`): the cell mesh around the camera, rebuilt
/// only when the camera's cell, its position relative to the layer or the cloud mode
/// changes. FANCY draws the extruded boxes, FAST the flat cells (`flat.rs`), OFF
/// nothing.
pub struct CloudRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipeline: wgpu::RenderPipeline,
    /// The flat cells of the FAST mode (vanilla `RenderPipelines.FLAT_CLOUDS`: the
    /// clouds pipeline without back-face culling, so the plane shows from both sides).
    flat_pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    capacity: usize,
    vertices: Vec<CloudVertex>,
    vertex_count: u32,
    texture: Option<CloudTexture>,
    /// The camera cell, position relative to the layer and mode the mesh was built for.
    built_for: Option<(i32, i32, RelativeCameraPos, bool)>,
    /// The frame's cloud mode (vanilla `Options.cloudStatus`, via `set_status`).
    status: CloudStatus,
    /// Whether the one-time diagnostics line was logged.
    logged: bool,
}

impl CloudRenderer {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        target_format: wgpu::TextureFormat,
        assets: &mut haste_assets::Assets,
    ) -> CloudRenderer {
        let texture = CloudTexture::load(assets);
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("clouds-uniforms"),
            size: std::mem::size_of::<CloudUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("clouds-layout"),
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
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("clouds-bind"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("clouds"),
            source: wgpu::ShaderSource::Wgsl(include_str!("clouds.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("clouds-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let attributes: [wgpu::VertexAttribute; 2] =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32];
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<CloudVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        };
        let pipeline = make_pipeline(
            &device,
            &pipeline_layout,
            &shader,
            &vertex_layout,
            target_format,
            Some(wgpu::Face::Back),
            "clouds",
        );
        // Vanilla `RenderPipelines.FLAT_CLOUDS`: the same pipeline without back-face
        // culling, so the FAST mode's flat cells show from both sides.
        let flat_pipeline = make_pipeline(
            &device,
            &pipeline_layout,
            &shader,
            &vertex_layout,
            target_format,
            None,
            "clouds-flat",
        );
        let capacity = 4096;
        CloudRenderer {
            vertex_buffer: make_buffer(&device, capacity),
            capacity,
            vertices: Vec::with_capacity(capacity),
            vertex_count: 0,
            texture,
            built_for: None,
            status: CloudStatus::default(),
            logged: false,
            device,
            queue,
            pipeline,
            flat_pipeline,
            uniforms,
            bind_group,
        }
    }

    /// Vanilla `Options.cloudStatus` for the frames to come (the video setting; changed
    /// from a vanilla options screen or the config). A mode change rebuilds the mesh once,
    /// on the next `prepare` (vanilla's `prevCloudStatus` in the rebuild condition).
    pub fn set_status(&mut self, status: CloudStatus) {
        self.status = status;
    }

    /// The vertices of the current mesh (tests).
    #[cfg(test)]
    pub(super) fn vertex_count(&self) -> u32 {
        self.vertex_count
    }

    /// Vanilla `CloudRenderer.prepare` under `LevelRenderer.renderClouds`: the OFF mode
    /// draws no pass at all (no mesh, no uniforms), so it returns before anything else;
    /// a dimension without clouds (not the overworld) skips the pass the same way.
    /// Otherwise finds the camera's cell, rebuilds the mesh when the cell, the position
    /// relative to the layer or the mode changed, and writes the per-frame uniforms
    /// (the offset inside the cell, the colour, the fog end).
    pub fn prepare(
        &mut self,
        camera: &Camera,
        sky: &SkyState,
        kind: crate::sky::SkyKind,
        view_proj: glam::Mat4,
        fog: &FogData,
        partial_tick: f32,
    ) {
        self.vertex_count = 0;
        if self.status == CloudStatus::Off || kind != crate::sky::SkyKind::Overworld {
            self.built_for = None;
            return;
        }
        let Some(texture) = self.texture.as_ref() else {
            return;
        };
        let cloud_height = if sky.cloud_height != 0.0 {
            sky.cloud_height
        } else {
            CLOUD_HEIGHT
        };
        let relative_pos = RelativeCameraPos::of(camera.pos.y, cloud_height as f64);
        // The cloud grid: drifted by game time in x, shifted +3.96 in z, then wrapped into
        // the texture's size in blocks so the cells tile.
        let drift = cloud_drift(sky.game_time, partial_tick);
        let cloud_x = camera.pos.x + drift as f64;
        let cloud_z = camera.pos.z + CLOUD_Z_SHIFT;
        let width_blocks = texture.width as f64 * CELL_SIZE as f64;
        let height_blocks = texture.height as f64 * CELL_SIZE as f64;
        let cloud_x = cloud_x - (cloud_x / width_blocks).floor() * width_blocks;
        let cloud_z = cloud_z - (cloud_z / height_blocks).floor() * height_blocks;
        let cell_x = (cloud_x / CELL_SIZE as f64).floor() as i32;
        let cell_z = (cloud_z / CELL_SIZE as f64).floor() as i32;
        let x_in_cell = (cloud_x - cell_x as f64 * CELL_SIZE as f64) as f32;
        let z_in_cell = (cloud_z - cell_z as f64 * CELL_SIZE as f64) as f32;

        // The mesh covers the cloud fog end (vanilla draws the cloud range option's
        // distance; without the option wired, the fog end is where clouds fade out).
        let radius_cells = (fog.cloud_end / CELL_SIZE).ceil() as i32;
        let fancy = self.status == CloudStatus::Fancy;
        if self.built_for != Some((cell_x, cell_z, relative_pos, fancy)) {
            self.built_for = Some((cell_x, cell_z, relative_pos, fancy));
            self.vertices.clear();
            build_mesh(
                &mut self.vertices,
                texture,
                cell_x,
                cell_z,
                relative_pos,
                radius_cells,
                fancy,
            );
            self.vertex_count = self.vertices.len() as u32;
            if self.vertices.len() > self.capacity {
                self.capacity = self.vertices.len().next_power_of_two();
                self.vertex_buffer = make_buffer(&self.device, self.capacity);
            }
            if !self.vertices.is_empty() {
                self.queue.write_buffer(
                    &self.vertex_buffer,
                    0,
                    bytemuck::cast_slice(&self.vertices),
                );
            }
        } else {
            self.vertex_count = self.vertices.len() as u32;
        }
        if self.vertex_count == 0 {
            return;
        }
        // One-time diagnostics: what the pass actually received, so an invisible cloud
        // pass is debuggable from the log.
        if !self.logged {
            self.logged = true;
            tracing::info!(
                "clouds pass: cell ({cell_x}, {cell_z}), {radius_cells} cell radius, {} \
                 vertices, offset ({x_in_cell:.2}, {:.2}, {z_in_cell:.2})",
                self.vertex_count,
                cloud_height - camera.pos.y as f32,
            );
        }
        let color = cloud_color(sky.time_of_day, sky.rain, sky.thunder);
        let u = CloudUniforms {
            view_proj: view_proj.to_cols_array_2d(),
            offset: [-x_in_cell, (cloud_height - camera.pos.y as f32), -z_in_cell],
            _pad0: 0.0,
            color,
            fog_end: fog.cloud_end,
            _pad1: [0.0; 7],
        };
        self.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));
    }

    /// Draws the cloud mesh into the main pass, after translucent terrain (where the sky
    /// overlays run). One draw call; the mode picks the pipeline (vanilla `render`:
    /// `FLAT_CLOUDS` for the FAST cells, `CLOUDS` for the extruded boxes).
    pub fn draw<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) -> u32 {
        if self.vertex_count == 0 {
            return 0;
        }
        pass.set_pipeline(if self.status == CloudStatus::Fancy {
            &self.pipeline
        } else {
            &self.flat_pipeline
        });
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..self.vertex_count, 0..1);
        1
    }
}

fn make_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("clouds-vertices"),
        size: (capacity * std::mem::size_of::<CloudVertex>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The clouds pass's pipeline (vanilla `RenderPipelines.CLOUDS`): alpha-blended,
/// depth-tested without writes, with the given cull mode (`FLAT_CLOUDS` is the
/// unculled one).
fn make_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    vertex_layout: &wgpu::VertexBufferLayout,
    target_format: wgpu::TextureFormat,
    cull_mode: Option<wgpu::Face>,
    label: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(vertex_layout.clone())],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth24Plus,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}
