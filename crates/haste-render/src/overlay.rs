// World overlays: the selection outline of the targeted block (its outline shape boxes' 12
// edges each, drawn as a screen-space quad `max(2.5, width / 1920 * 2.5)` pixels wide like
// vanilla's `rendertype_lines` lines, black at 40% alpha) and break cracks
// (`textures/block/destroy_stage_0..9` projected onto the block's model faces, multiplied
// 2 * src * dst into the terrain).
//
// The renderer holds no world: the caller samples the target's outline boxes and the states of
// breaking blocks with `Renderer::sample_world`.

use std::sync::Arc;

use glam::{DVec3, Vec3, Vec4};
use haste_assets::{Assets, BakedBlockModels};
use haste_core::math::Aabb;
use haste_core::{BlockPos, BlockStateId, Identifier};

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct OverlayVertex {
    pub pos: [f32; 3],
    pub data: [f32; 4],
}

impl OverlayVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<OverlayVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &Self::ATTRIBUTES,
    };
}

/// A line edge vertex: the endpoint relative to `pos` (the expansion direction) and the quad
/// corner (-1 or 1: which side of the edge). The vertex shader expands the edge into a quad.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LineVertex {
    pos: [f32; 3],
    to: [f32; 4],
    color: [f32; 4],
}

impl LineVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4, 2 => Float32x4];
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<LineVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &Self::ATTRIBUTES,
    };
}

/// What `sample_world` found for this frame's overlays.
#[derive(Default, Clone, Debug)]
pub struct OverlayWorld {
    /// The targeted block and its outline boxes (block-local).
    pub target: Option<(BlockPos, Vec<Aabb>)>,
    /// Breaking blocks with their states.
    pub breaking: Vec<(BlockPos, BlockStateId, u8)>,
}

pub struct OverlayRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    lines: wgpu::RenderPipeline,
    cracks: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    line_buffer: wgpu::Buffer,
    crack_buffer: wgpu::Buffer,
    line_capacity: usize,
    crack_capacity: usize,
    line_vertices: Vec<LineVertex>,
    crack_vertices: Vec<OverlayVertex>,
    pub world: OverlayWorld,
}

impl OverlayRenderer {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        target_format: wgpu::TextureFormat,
        camera_layout: &wgpu::BindGroupLayout,
        assets: &mut Assets,
    ) -> OverlayRenderer {
        // The ten crack stages side by side in a 160x16 strip.
        let mut strip = image::RgbaImage::new(160, 16);
        for stage in 0..10 {
            match assets.texture(&Identifier::minecraft(&format!(
                "block/destroy_stage_{stage}"
            ))) {
                Ok(img) => {
                    let img =
                        image::imageops::resize(&img, 16, 16, image::imageops::FilterType::Nearest);
                    image::imageops::replace(&mut strip, &img, stage * 16, 0);
                }
                Err(e) => tracing::warn!("crack texture {stage}: {e:#}"),
            }
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cracks"),
            size: wgpu::Extent3d {
                width: 160,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Raw values: mid grey (128) leaves the terrain unchanged under 2 * src * dst.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            strip.as_raw(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(160 * 4),
                rows_per_image: Some(16),
            },
            wgpu::Extent3d {
                width: 160,
                height: 16,
                depth_or_array_layers: 1,
            },
        );
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("overlay-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("overlay-bind"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(
                    &texture.create_view(&Default::default()),
                ),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("overlay"),
            source: wgpu::ShaderSource::Wgsl(include_str!("overlay.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("overlay-pipeline-layout"),
            bind_group_layouts: &[Some(camera_layout), Some(&layout)],
            immediate_size: 0,
        });
        let make = |label: &str,
                    vs: &str,
                    fs: &str,
                    layout: wgpu::VertexBufferLayout<'_>,
                    topology: wgpu::PrimitiveTopology,
                    blend: wgpu::BlendState,
                    bias: wgpu::DepthBiasState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[Some(layout)],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::COLOR,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth24Plus,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias,
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let lines = make(
            "outline",
            "vs_line",
            "fs_line",
            LineVertex::LAYOUT,
            wgpu::PrimitiveTopology::TriangleList,
            wgpu::BlendState::ALPHA_BLENDING,
            // The quads' vertices are pulled toward the camera instead (`prepare`).
            Default::default(),
        );
        let cracks = make(
            "cracks",
            "vs_main",
            "fs_crack",
            OverlayVertex::LAYOUT,
            wgpu::PrimitiveTopology::TriangleList,
            wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Dst,
                    dst_factor: wgpu::BlendFactor::Src,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent::OVER,
            },
            // Pulled toward the camera so the decal wins over the face it lies on.
            wgpu::DepthBiasState {
                constant: -2,
                slope_scale: -1.0,
                clamp: 0.0,
            },
        );
        let line_capacity = 4096;
        let crack_capacity = 1024;
        let line_buffer = make_buffer(&device, line_capacity, std::mem::size_of::<LineVertex>());
        let crack_buffer = make_buffer(
            &device,
            crack_capacity,
            std::mem::size_of::<OverlayVertex>(),
        );
        OverlayRenderer {
            device,
            queue,
            lines,
            cracks,
            bind_group,
            line_buffer,
            crack_buffer,
            line_capacity,
            crack_capacity,
            line_vertices: Vec::new(),
            crack_vertices: Vec::new(),
            world: OverlayWorld::default(),
        }
    }

    /// Builds the frame's overlay geometry (camera-relative). Only blocks that match the
    /// frame's `block_target` / `breaking` lists and were sampled are drawn.
    pub fn prepare(
        &mut self,
        camera: DVec3,
        block_target: Option<BlockPos>,
        breaking: &[(BlockPos, u8)],
        baked: &BakedBlockModels,
    ) {
        self.line_vertices.clear();
        if let (Some(pos), Some((sampled, boxes))) = (block_target, &self.world.target) {
            if pos == *sampled {
                let origin = DVec3::new(pos.x as f64, pos.y as f64, pos.z as f64) - camera;
                for b in boxes {
                    push_box_lines(&mut self.line_vertices, origin, b, [0.0, 0.0, 0.0, 0.4]);
                }
            }
        }
        // Lines lie on the neighbours' faces too: move them 0.3% toward the camera (positions are
        // camera-relative) so they win the depth test without z-fighting.
        for v in &mut self.line_vertices {
            v.pos = v.pos.map(|c| c * 0.997);
        }
        if self.line_vertices.len() > self.line_capacity {
            self.line_capacity = self.line_vertices.len().next_power_of_two();
            self.line_buffer = make_buffer(
                &self.device,
                self.line_capacity,
                std::mem::size_of::<LineVertex>(),
            );
        }
        if !self.line_vertices.is_empty() {
            self.queue.write_buffer(
                &self.line_buffer,
                0,
                bytemuck::cast_slice(&self.line_vertices),
            );
        }
        self.crack_vertices.clear();
        for &(pos, stage) in breaking {
            let Some(&(_, state, _)) = self.world.breaking.iter().find(|b| b.0 == pos) else {
                continue;
            };
            let origin = DVec3::new(pos.x as f64, pos.y as f64, pos.z as f64) - camera;
            push_cracks(
                &mut self.crack_vertices,
                origin.as_vec3(),
                baked.quads(state),
                stage.min(9),
            );
        }
        if self.crack_vertices.len() > self.crack_capacity {
            self.crack_capacity = self.crack_vertices.len().next_power_of_two();
            self.crack_buffer = make_buffer(
                &self.device,
                self.crack_capacity,
                std::mem::size_of::<OverlayVertex>(),
            );
        }
        if !self.crack_vertices.is_empty() {
            self.queue.write_buffer(
                &self.crack_buffer,
                0,
                bytemuck::cast_slice(&self.crack_vertices),
            );
        }
    }

    /// Draws into the main pass after terrain (camera bind group at group 0).
    pub fn draw<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) -> u32 {
        if self.line_vertices.is_empty() && self.crack_vertices.is_empty() {
            return 0;
        }
        let mut calls = 0;
        pass.set_bind_group(1, &self.bind_group, &[]);
        if !self.crack_vertices.is_empty() {
            pass.set_pipeline(&self.cracks);
            pass.set_vertex_buffer(0, self.crack_buffer.slice(..));
            pass.draw(0..self.crack_vertices.len() as u32, 0..1);
            calls += 1;
        }
        if !self.line_vertices.is_empty() {
            pass.set_pipeline(&self.lines);
            pass.set_vertex_buffer(0, self.line_buffer.slice(..));
            pass.draw(0..self.line_vertices.len() as u32, 0..1);
            calls += 1;
        }
        calls
    }
}

fn make_buffer(device: &wgpu::Device, capacity: usize, stride: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("overlay-vertices"),
        size: (capacity * stride) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The 12 edges of a box (block-local, inflated by 0.002 like vanilla's outline). Each edge
/// becomes two vertices per quad corner (4 vertices, 2 triangles): the vertex shader offsets
/// them perpendicular to the edge's screen-space direction, so the quad is `line_width`
/// framebuffer pixels wide at any distance and camera angle.
fn push_box_lines(out: &mut Vec<LineVertex>, origin: DVec3, b: &Aabb, color: [f32; 4]) {
    let e = 0.002;
    let lo = (origin + b.min - DVec3::splat(e)).as_vec3();
    let hi = (origin + b.max + DVec3::splat(e)).as_vec3();
    let c = |x: bool, y: bool, z: bool| {
        Vec3::new(
            if x { hi.x } else { lo.x },
            if y { hi.y } else { lo.y },
            if z { hi.z } else { lo.z },
        )
    };
    let edges = [
        (c(false, false, false), c(true, false, false)),
        (c(false, false, true), c(true, false, true)),
        (c(false, true, false), c(true, true, false)),
        (c(false, true, true), c(true, true, true)),
        (c(false, false, false), c(false, true, false)),
        (c(true, false, false), c(true, true, false)),
        (c(false, false, true), c(false, true, true)),
        (c(true, false, true), c(true, true, true)),
        (c(false, false, false), c(false, false, true)),
        (c(true, false, false), c(true, false, true)),
        (c(false, true, false), c(false, true, true)),
        (c(true, true, false), c(true, true, true)),
    ];
    let color = Vec4::from_array(color);
    for (a, b) in edges {
        let dir = b - a;
        // Two triangles per edge (TriangleList without an index buffer eats
        // consecutive triples): the quad's corners are (a, -1), (a, +1), (b, -1),
        // (b, +1), split along the (a, +1)-(b, -1) diagonal.
        let quad = [
            (-1.0f32, a),
            (1.0, a),
            (-1.0, b),
            (-1.0, b),
            (1.0, a),
            (1.0, b),
        ];
        for (corner, p) in quad {
            out.push(LineVertex {
                pos: p.to_array(),
                to: [dir.x, dir.y, dir.z, corner],
                color: color.to_array(),
            });
        }
    }
}

/// Crack decals over a block's model quads: the stage texture projected by block-local
/// position on the face's plane (16 texels per block).
fn push_cracks(
    out: &mut Vec<OverlayVertex>,
    origin: Vec3,
    quads: &[haste_assets::BakedQuad],
    stage: u8,
) {
    use haste_assets::Direction as D;
    let column = stage as f32 * 16.0;
    for q in quads {
        let uv = |p: [f32; 3]| -> [f32; 2] {
            let (u, v) = match q.direction {
                D::Up => (p[0], p[2]),
                D::Down => (p[0], 1.0 - p[2]),
                D::North => (1.0 - p[0], 1.0 - p[1]),
                D::South => (p[0], 1.0 - p[1]),
                D::West => (p[2], 1.0 - p[1]),
                D::East => (1.0 - p[2], 1.0 - p[1]),
            };
            [u * 16.0, v * 16.0]
        };
        for i in [0usize, 1, 2, 0, 2, 3] {
            let p = q.positions[i];
            let t = uv(p);
            out.push(OverlayVertex {
                pos: (origin + Vec3::from(p)).to_array(),
                data: [t[0], t[1], column, 0.0],
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One box's 12 edges each become two triangles (6 vertices): 72 line vertices,
    /// each carrying the edge direction and one of the two quad corners. The two
    /// triangles must stay within one edge: a triangle spanning two edges is the
    /// "large grey X across the face" bug (TriangleList without indices eats
    /// consecutive triples, so the vertex order is the index buffer).
    #[test]
    fn box_edges_become_quads() {
        let mut out = Vec::new();
        push_box_lines(
            &mut out,
            DVec3::ZERO,
            &Aabb::new(DVec3::new(0.0, 0.0, 0.0), DVec3::new(1.0, 1.0, 1.0)),
            [0.0, 0.0, 0.0, 0.4],
        );
        assert_eq!(out.len(), 12 * 6);
        // Vanilla's outline colour: black at 40% alpha (ARGB 0x66000000).
        assert!(out.iter().all(|v| v.color == [0.0, 0.0, 0.0, 0.4]));
        // Inflated by 0.002 like vanilla's outline: the min corner moves past the block.
        assert!(out[0].pos[0] < -0.001 && out[0].pos[1] < -0.001 && out[0].pos[2] < -0.001);
        for edge in out.chunks(6) {
            // All six carry the same edge direction; the two endpoints appear three
            // times each (both triangles span only this edge's two positions).
            let dir = [edge[0].to[0], edge[0].to[1], edge[0].to[2]];
            assert!(edge
                .iter()
                .all(|v| v.to[0] == dir[0] && v.to[1] == dir[1] && v.to[2] == dir[2]));
            let a_pos = edge[0].pos;
            let b_pos = edge[2].pos;
            for triangle in [(&edge[0..3]), (&edge[3..6])] {
                // Both endpoints appear (2 + 1 vertices): the triangle spans the
                // edge and nothing else.
                assert!(triangle.iter().any(|v| v.pos == a_pos));
                assert!(triangle.iter().any(|v| v.pos == b_pos));
                assert!(triangle.iter().all(|v| v.pos == a_pos || v.pos == b_pos));
                // Each triangle spans the quad: it touches both corner signs.
                assert!(triangle.iter().any(|v| v.to[3] < 0.0));
                assert!(triangle.iter().any(|v| v.to[3] > 0.0));
            }
        }
    }

    /// Vanilla's line width (Window.getAppropriateLineWidth):
    /// max(2.5, framebuffer width / 1920 * 2.5) pixels, at any window size.
    #[test]
    fn vanilla_line_width() {
        let width = |w: u32| (w as f32 / 1920.0 * 2.5).max(2.5);
        assert_eq!(width(1280), 2.5); // below 1920: the minimum
        assert_eq!(width(1920), 2.5);
        assert_eq!(width(3840), 5.0); // 4K: scales up
    }
}
