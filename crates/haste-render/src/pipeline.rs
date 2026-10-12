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
}

impl Gpu {
    /// Enumerate adapters (all backends), prefer Vulkan, log the choice.
    pub fn new() -> Result<Gpu> {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::all();
        let instance = wgpu::Instance::new(descriptor);
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
            tracing::warn!("Vulkan unavailable; falling back to {:?}", info.backend);
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("haste-render"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            ..Default::default()
        }))
        .context("requesting a wgpu device")?;
        Ok(Gpu {
            device: Arc::new(device),
            queue: Arc::new(queue),
            adapter: Arc::new(adapter),
        })
    }
}

/// Upload the block atlas (level-0 pixels + mip chain) into a texture.
pub fn upload_atlas(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    atlas: &haste_assets::AtlasBuilder,
) -> Result<(wgpu::Texture, wgpu::TextureView, wgpu::Sampler, wgpu::Buffer, u32)> {
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
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
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
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("atlas-sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
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
