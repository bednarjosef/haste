// wgpu device/surface management and the chunk render pipeline (WGSL port of
// Pomme's chunk.vert/chunk.frag design, rewritten for wgpu; see THIRD_PARTY.md
// for the Pomme attribution of the shading approach).
//
// Backend-neutral: requests all backends, prefers Vulkan, logs the choice.

use std::sync::Arc;

use anyhow::{Context, Result};
use wgpu::util::DeviceExt;

/// The renderer's GPU context: adapter/device/queue plus the atlas texture.
/// Shared between the windowed and offscreen paths.
pub struct Gpu {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub adapter: Arc<wgpu::Adapter>,
    /// Terrain is drawn with one multi-draw-indirect call per layer and arena.
    pub multi_draw: bool,
    /// `DownlevelFlags::VIEW_FORMATS` and `SURFACE_VIEW_FORMATS`.
    pub view_formats: bool,
    /// Kept alive for surface creation (windowed mode).
    #[allow(dead_code)]
    pub instance: Arc<wgpu::Instance>,
}

/// A display handle wgpu can hold: `HasDisplayHandle + Debug + Send + Sync`
/// (wgpu's `WgpuHasDisplayHandle` blanket-implements for this).
pub trait ShareableDisplayHandle:
    winit::raw_window_handle::HasDisplayHandle + core::fmt::Debug + Send + Sync + 'static
{
}

impl<T: winit::raw_window_handle::HasDisplayHandle + core::fmt::Debug + Send + Sync + 'static>
    ShareableDisplayHandle for T
{
}

impl Gpu {
    /// Enumerate adapters (all backends), prefer Vulkan, log the choice.
    /// `display`: the window's display handle when a surface will be created
    /// (wgpu 30 requires it in the instance for `create_surface`).
    pub fn new(display: Option<Box<dyn ShareableDisplayHandle>>) -> Result<Gpu> {
        // wgpu's WgpuHasDisplayHandle blanket-implements for any
        // HasDisplayHandle + Debug + Send + Sync; our wrapper satisfies it.
        let mut descriptor = match display {
            Some(display) => {
                // Erase to wgpu's own trait object via the blanket impl.
                struct WgpuHandle(Box<dyn ShareableDisplayHandle>);
                impl core::fmt::Debug for WgpuHandle {
                    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        self.0.fmt(f)
                    }
                }
                impl winit::raw_window_handle::HasDisplayHandle for WgpuHandle {
                    fn display_handle(
                        &self,
                    ) -> Result<
                        winit::raw_window_handle::DisplayHandle<'_>,
                        winit::raw_window_handle::HandleError,
                    > {
                        self.0.display_handle()
                    }
                }
                wgpu::InstanceDescriptor::new_with_display_handle(Box::new(WgpuHandle(display)))
            }
            None => wgpu::InstanceDescriptor::new_without_display_handle(),
        };
        // `WGPU_BACKEND=gl|vulkan|...` picks a backend (tests the GL fallback on a Vulkan box).
        descriptor.backends = wgpu::Backends::from_env().unwrap_or(wgpu::Backends::all());
        let instance = wgpu::Instance::new(descriptor);
        let instance = std::sync::Arc::new(instance);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .context("requesting a wgpu adapter")?;
        let info = adapter.get_info();
        tracing::info!(
            "wgpu adapter: {} ({}) backend {:?}",
            info.name,
            info.vendor,
            info.backend
        );
        if info.backend != wgpu::Backend::Vulkan {
            tracing::warn!(
                "Vulkan driver not found; install vulkan-radeon (Arch) or set HASTE_VULKAN_ICD. Falling back to {:?}",
                info.backend
            );
        }
        // Multi-draw-indirect terrain needs a non-zero first instance per draw (the section
        // origin); without it the renderer issues direct draws.
        let optional = wgpu::Features::INDIRECT_FIRST_INSTANCE | wgpu::Features::TIMESTAMP_QUERY;
        let required_features = adapter.features() & optional;
        let multi_draw = required_features.contains(wgpu::Features::INDIRECT_FIRST_INSTANCE)
            && adapter
                .get_downlevel_capabilities()
                .flags
                .contains(wgpu::DownlevelFlags::INDIRECT_EXECUTION);
        tracing::info!("terrain multi-draw-indirect: {multi_draw}");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("haste-render"),
            required_features,
            required_limits: wgpu::Limits::default(),
            ..Default::default()
        }))
        .context("requesting a wgpu device")?;
        // A non-sRGB view of the sRGB target lets the GUI blend in sRGB space like vanilla; GL
        // lacks it (then the GUI writes linear colour into the sRGB target directly).
        let flags = adapter.get_downlevel_capabilities().flags;
        let view_formats = flags.contains(wgpu::DownlevelFlags::VIEW_FORMATS)
            && flags.contains(wgpu::DownlevelFlags::SURFACE_VIEW_FORMATS);
        if !view_formats {
            tracing::warn!("no view formats on this backend: GUI blending happens in linear space");
        }
        Ok(Gpu {
            view_formats,
            multi_draw,
            device: Arc::new(device),
            queue: Arc::new(queue),
            adapter: Arc::new(adapter),
            instance,
        })
    }
}

/// Upload the block atlas (level-0 pixels + mip chain) into a texture.
pub fn upload_atlas(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    atlas: &haste_assets::AtlasBuilder,
) -> Result<(
    wgpu::Texture,
    wgpu::TextureView,
    wgpu::Sampler,
    wgpu::Buffer,
    u32,
)> {
    let size = atlas.size();
    let mip_levels = atlas.mip_levels() + 1;
    if !size.is_power_of_two() || mip_levels > size.trailing_zeros() + 1 {
        anyhow::bail!("atlas {size}x{size} cannot hold {mip_levels} mip levels");
    }
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("block-atlas"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: mip_levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    // The builder's pixels are a concatenated mip chain (level-major).
    let mut offset = 0usize;
    for level in 0..mip_levels {
        let dim = size >> level;
        let bytes = (dim * dim * 4) as usize;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &atlas.pixels()[offset..offset + bytes],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(dim * 4),
                rows_per_image: Some(dim),
            },
            wgpu::Extent3d {
                width: dim,
                height: dim,
                depth_or_array_layers: 1,
            },
        );
        offset += bytes;
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    // Vanilla's block-atlas sampler for terrain (GlSampler: NEAREST_MIPMAP_LINEAR):
    // nearest within a level, linear blend between the two nearest mip levels.
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("atlas-sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let rects = atlas.rects();
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("sprite-rects"),
        contents: bytemuck::cast_slice(rects),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    });
    Ok((texture, view, sampler, buffer, rects.len() as u32))
}

/// Per-section GPU buffers: one vertex + index buffer per render layer pair.
pub struct SectionGpuMesh {
    pub pos: haste_core::SectionPos,
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
    /// Draw range within the layer's pipeline.
    pub layer: haste_assets::RenderLayer,
    /// World-space AABB for frustum culling (from the section origin + local).
    pub aabb: ([f32; 3], [f32; 3]),
}

/// Upload a meshed section to the GPU.
pub fn upload_section(
    device: &wgpu::Device,
    mesh: &crate::mesher::SectionMesh,
) -> Vec<SectionGpuMesh> {
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("section-vertices"),
        contents: bytemuck::cast_slice(&mesh.vertices),
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
    });
    let origin = mesh.pos.block_min();
    let world = |p: [f32; 3]| {
        [
            origin.x as f32 + p[0],
            origin.y as f32 + p[1],
            origin.z as f32 + p[2],
        ]
    };
    let (min, max) = (world(mesh.aabb.0), world(mesh.aabb.1));
    mesh.layers
        .iter()
        .map(|(layer, indices)| SectionGpuMesh {
            pos: mesh.pos,
            vertex_buffer: vertex_buffer.clone(),
            index_buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("section-indices"),
                contents: bytemuck::cast_slice(indices),
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            }),
            index_count: indices.len() as u32,
            layer: *layer,
            aabb: (min, max),
        })
        .collect()
}
