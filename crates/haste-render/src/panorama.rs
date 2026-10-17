// The title screen's rotating panorama (vanilla `CubeMap`, which `GuiRenderer.render` draws
// first thing in the GUI pass whenever a screen extracted `Panorama` this frame).
//
// Vanilla: `CubeMapTexture` stacks the six `textures/gui/title/background/panorama_N.png`
// faces into one cube texture (layers +X, -X, +Y, -Y, +Z, -Z take faces 1, 3, 5, 4, 0, 2,
// each copied upside down), sampled linearly with no mips. `CubeMap.render` draws a unit
// cube around the camera, each fragment sampling the cube texture in the direction of its
// own vertex position (`core/panorama` shaders), through a perspective of 85 degrees
// vertical FOV, near 0.05, far 10, at the window's aspect, and a model-view of a half turn
// about X, then the 10 degree pitch about X, then the spin about Y (`PanoramaRenderState`,
// the negated `Panorama.spin`). No depth test, back faces culled, no blending.
//
// Haste: the faces are read from the assets once, the first frame that asks for the
// panorama; their decoding runs on a worker thread and the cube is uploaded once when it
// lands (the frames until then draw no panorama). The cube's vertices and indices are
// uploaded at creation; a frame writes one 64-byte matrix and draws 36 indices.

use std::sync::mpsc;

use glam::Mat4;
use haste_assets::Assets;

/// Vanilla `CubeMap.PROJECTION_FOV` (degrees, vertical).
pub const PROJECTION_FOV: f32 = 85.0;
/// Vanilla `CubeMap.PROJECTION_Z_NEAR`.
pub const PROJECTION_Z_NEAR: f32 = 0.05;
/// Vanilla `CubeMap.PROJECTION_Z_FAR`.
pub const PROJECTION_Z_FAR: f32 = 10.0;
/// The pitch `GuiRenderer.render` passes to `CubeMap.render` (degrees about X).
pub const PITCH: f32 = 10.0;
/// Vanilla `CubeMapTexture.SUFFIXES`: the face file of each cube layer, in the layer order
/// +X, -X, +Y, -Y, +Z, -Z.
pub const LAYER_FACES: [usize; 6] = [1, 3, 5, 4, 0, 2];

/// The cube's 24 corners, four per face (vanilla `CubeMap.initializeVertices`).
const CUBE: [[f32; 3]; 24] = [
    [-1.0, -1.0, 1.0],
    [-1.0, 1.0, 1.0],
    [1.0, 1.0, 1.0],
    [1.0, -1.0, 1.0],
    [1.0, -1.0, 1.0],
    [1.0, 1.0, 1.0],
    [1.0, 1.0, -1.0],
    [1.0, -1.0, -1.0],
    [1.0, -1.0, -1.0],
    [1.0, 1.0, -1.0],
    [-1.0, 1.0, -1.0],
    [-1.0, -1.0, -1.0],
    [-1.0, -1.0, -1.0],
    [-1.0, 1.0, -1.0],
    [-1.0, 1.0, 1.0],
    [-1.0, -1.0, 1.0],
    [-1.0, -1.0, -1.0],
    [-1.0, -1.0, 1.0],
    [1.0, -1.0, 1.0],
    [1.0, -1.0, -1.0],
    [-1.0, 1.0, 1.0],
    [-1.0, 1.0, -1.0],
    [1.0, 1.0, -1.0],
    [1.0, 1.0, 1.0],
];

/// The quads as triangles: vanilla's sequential quad index buffer (0 1 2, 2 3 0 per quad).
fn cube_indices() -> [u16; 36] {
    let mut out = [0u16; 36];
    for q in 0..6u16 {
        let b = q * 4;
        let i = q as usize * 6;
        out[i..i + 6].copy_from_slice(&[b, b + 1, b + 2, b + 2, b + 3, b]);
    }
    out
}

/// Vanilla `CubeMap.render`'s projection times model-view for a spin (degrees, the
/// `PanoramaRenderState`'s) on a `width` x `height` target.
pub fn panorama_matrix(spin_deg: f32, width: u32, height: u32) -> Mat4 {
    let aspect = width.max(1) as f32 / height.max(1) as f32;
    // wgpu's 0..1 depth range; nothing is depth-tested, the clip volume is the same.
    let projection = Mat4::perspective_rh(
        PROJECTION_FOV.to_radians(),
        aspect,
        PROJECTION_Z_NEAR,
        PROJECTION_Z_FAR,
    );
    let model_view = Mat4::from_rotation_x(std::f32::consts::PI)
        * Mat4::from_rotation_x(PITCH.to_radians())
        * Mat4::from_rotation_y(spin_deg.to_radians());
    projection * model_view
}

/// The six faces (in file order `panorama_0` .. `panorama_5`) stacked into the cube's
/// layer order, each flipped upside down (vanilla `CubeMapTexture.loadContents`): RGBA8
/// rows, `width` x `height` per layer. Errors when the faces' sizes differ.
pub fn stack_faces(faces: &[image::RgbaImage; 6]) -> anyhow::Result<(u32, u32, Vec<u8>)> {
    let (w, h) = faces[LAYER_FACES[0]].dimensions();
    let row = w as usize * 4;
    let mut out = Vec::with_capacity(row * h as usize * 6);
    for (layer, &face) in LAYER_FACES.iter().enumerate() {
        let img = &faces[face];
        anyhow::ensure!(
            img.dimensions() == (w, h),
            "panorama faces differ in size: layer 0 is {w}x{h}, layer {layer} (panorama_{face}) is {}x{}",
            img.width(),
            img.height()
        );
        for y in (0..h as usize).rev() {
            out.extend_from_slice(&img.as_raw()[y * row..(y + 1) * row]);
        }
    }
    Ok((w, h, out))
}

/// The six face files' bytes, read from the assets (packs, downloaded index, jar).
fn read_faces(assets: &mut Assets) -> anyhow::Result<[Vec<u8>; 6]> {
    let mut out: [Vec<u8>; 6] = Default::default();
    for (i, bytes) in out.iter_mut().enumerate() {
        *bytes = assets.read_asset(&format!(
            "assets/minecraft/textures/gui/title/background/panorama_{i}.png"
        ))?;
    }
    Ok(out)
}

/// Decodes and stacks the faces (on the worker thread).
fn decode_faces(files: [Vec<u8>; 6]) -> anyhow::Result<(u32, u32, Vec<u8>)> {
    let mut faces: [image::RgbaImage; 6] = Default::default();
    for (i, (face, bytes)) in faces.iter_mut().zip(files).enumerate() {
        *face = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            .map_err(|e| anyhow::anyhow!("decoding panorama_{i}.png: {e}"))?
            .to_rgba8();
    }
    stack_faces(&faces)
}

type Faces = anyhow::Result<(u32, u32, Vec<u8>)>;

/// Where the cube texture is.
enum Cube {
    /// Not asked for yet.
    Unloaded,
    /// Decoding on the worker thread.
    Loading(mpsc::Receiver<Faces>),
    /// Uploaded; the bind group samples it.
    Ready(wgpu::BindGroup),
    /// The faces could not be read (logged once); nothing is drawn.
    Failed,
}

/// The panorama pass (vanilla `CubeMap`), drawn under the GUI's commands.
pub(crate) struct PanoramaRenderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    sampler: wgpu::Sampler,
    cube: Cube,
    /// This frame's spin (set before `render`, taken by `prepare`).
    requested: Option<f32>,
    /// Whether `draw` draws this frame.
    ready: bool,
}

impl PanoramaRenderer {
    /// `format`: the GUI pass's target format (the non-sRGB view: the faces' stored sRGB
    /// bytes go through raw, like vanilla's RGBA8_UNORM main target).
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> PanoramaRenderer {
        use wgpu::util::DeviceExt;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("panorama"),
            source: wgpu::ShaderSource::Wgsl(include_str!("panorama.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("panorama-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
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
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
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
            ],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("panorama"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("panorama-pipeline-layout"),
                    bind_group_layouts: &[Some(&layout)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("panorama-uniform"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("panorama-vertices"),
            contents: bytemuck::cast_slice(&CUBE),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("panorama-indices"),
            contents: bytemuck::cast_slice(&cube_indices()),
            usage: wgpu::BufferUsages::INDEX,
        });
        // CubeMapTexture's metadata: blur (linear filtering), no clamp, one mip level.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("panorama-sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        PanoramaRenderer {
            pipeline,
            layout,
            uniform,
            vertices,
            indices,
            sampler,
            cube: Cube::Unloaded,
            requested: None,
            ready: false,
        }
    }

    /// This frame's panorama: the spin (degrees) when the open screen extracted one.
    pub(crate) fn request(&mut self, spin: Option<f32>) {
        self.requested = spin;
    }

    /// Whether this frame asked for the panorama (the GUI pass then runs even without
    /// commands).
    pub(crate) fn requested(&self) -> bool {
        self.requested.is_some()
    }

    /// Readies this frame's draw: loads the cube the first time it is asked for, and
    /// writes the frame's matrix. Consumes the request (vanilla resets the GUI render
    /// state every frame).
    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &mut Assets,
        size: (u32, u32),
    ) {
        self.ready = false;
        let Some(spin) = self.requested.take() else {
            return;
        };
        if matches!(self.cube, Cube::Unloaded) {
            self.cube = match read_faces(assets) {
                Ok(files) => {
                    let (tx, rx) = mpsc::channel();
                    let spawned = std::thread::Builder::new()
                        .name("panorama-load".into())
                        .spawn(move || {
                            let _ = tx.send(decode_faces(files));
                        });
                    match spawned {
                        Ok(_) => Cube::Loading(rx),
                        Err(e) => {
                            tracing::warn!("panorama: cannot start its loader: {e}");
                            Cube::Failed
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("panorama: {e:#}; the title background stays plain");
                    Cube::Failed
                }
            };
        }
        if let Cube::Loading(rx) = &self.cube {
            match rx.try_recv() {
                Ok(Ok((w, h, rgba))) => {
                    self.cube = Cube::Ready(self.upload(device, queue, w, h, &rgba));
                }
                Ok(Err(e)) => {
                    tracing::warn!("panorama: {e:#}; the title background stays plain");
                    self.cube = Cube::Failed;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.cube = Cube::Failed,
            }
        }
        if matches!(self.cube, Cube::Ready(_)) {
            let m = panorama_matrix(spin, size.0, size.1);
            queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&m.to_cols_array()));
            self.ready = true;
        }
    }

    /// The cube texture (six layers, viewed as a cube) and its bind group.
    fn upload(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        w: u32,
        h: u32,
        rgba: &[u8],
    ) -> wgpu::BindGroup {
        let size = wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 6,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("panorama-cube"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("panorama-cube-view"),
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("panorama-bind-group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// Draws the cube when `prepare` readied it; returns the draw calls (0 or 1).
    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) -> u32 {
        let (true, Cube::Ready(group)) = (self.ready, &self.cube) else {
            return 0;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..36, 0, 0..1);
        1
    }
}

#[cfg(test)]
mod tests;
