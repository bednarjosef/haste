// The quad-pulling terrain pipelines (`--terrain-format quad`), split out of
// `init.rs` by the file-size rule: the same fragment stages as the indexed
// terrain pipelines, but the vertex stage (`vs_quad` in chunk.wgsl) reads one
// record per quad from the arena storage buffers (group 3, bound per arena)
// and the only vertex buffer is the instance-rate draw data. Also owns the
// static index buffers the draws index through (see `quad_format.rs`).

use wgpu::util::DeviceExt;

use crate::quad_format::{QuadInstance, QUADS_PER_SPAN, QUAD_INDEX_PATTERN, TRIS_PER_SPAN};

pub(super) struct QuadPipelines {
    /// Group 3: the arena's quad records (binding 0) and pulled vertex
    /// records (binding 1), bound once per arena by the draw loop.
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub solid: wgpu::RenderPipeline,
    pub cutout: wgpu::RenderPipeline,
    pub translucent: wgpu::RenderPipeline,
    /// The per-quad corner index pattern, repeated to the span chunk size.
    pub pattern_index: wgpu::Buffer,
    /// The sequential pattern triangle soups draw through.
    pub seq_index: wgpu::Buffer,
}

pub(super) fn make(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    surface_format: wgpu::TextureFormat,
    camera_layout: &wgpu::BindGroupLayout,
    atlas_layout: &wgpu::BindGroupLayout,
    lightmap_layout: &wgpu::BindGroupLayout,
) -> QuadPipelines {
    let storage = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("chunk-quad-arena-layout"),
        entries: &[storage(0), storage(1)],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("chunk-quad-layout"),
        bind_group_layouts: &[
            Some(camera_layout),
            Some(atlas_layout),
            Some(lightmap_layout),
            Some(&bind_group_layout),
        ],
        immediate_size: 0,
    });
    let make_pipeline = |label: &str, fragment: &str, blend: Option<wgpu::BlendState>| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_quad"),
                compilation_options: Default::default(),
                buffers: &[Some(QuadInstance::LAYOUT)],
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
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
                // The same rule as the indexed terrain pipelines: vanilla's
                // DEFAULT (GREATER_THAN_OR_EQUAL) against Haste's 1-clear is
                // LessEqual, so coplanar model elements both draw.
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        })
    };
    let solid = make_pipeline("chunk-quad-solid", "fs_main", None);
    let cutout = make_pipeline("chunk-quad-cutout", "fs_cutout", None);
    // Vanilla BlendFunction.TRANSLUCENT: (SRC_ALPHA, ONE_MINUS_SRC_ALPHA)
    // both equations, the same blend the indexed translucent pipeline uses.
    let translucent = make_pipeline(
        "chunk-quad-translucent",
        "fs_translucent",
        Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        }),
    );
    let pattern: Vec<u32> = QUAD_INDEX_PATTERN
        .iter()
        .cycle()
        .take(6 * QUADS_PER_SPAN as usize)
        .map(|&v| v as u32)
        .collect();
    let pattern_index = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("quad-index-pattern"),
        contents: bytemuck::cast_slice(&pattern),
        usage: wgpu::BufferUsages::INDEX,
    });
    let seq: Vec<u32> = (0..TRIS_PER_SPAN * 3).collect();
    let seq_index = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("quad-index-sequential"),
        contents: bytemuck::cast_slice(&seq),
        usage: wgpu::BufferUsages::INDEX,
    });
    QuadPipelines {
        bind_group_layout,
        solid,
        cutout,
        translucent,
        pattern_index,
        seq_index,
    }
}
