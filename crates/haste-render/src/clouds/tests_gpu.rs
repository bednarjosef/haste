// GPU tests of the clouds pass: the option gating, the cloud range option's effect on
// the mesh, and the pixel equality of the instance-record path with the CPU-expanded
// mesh the pass used to upload (the "same drawn result" guarantee of the record
// change). Skipped without a GPU adapter, like the other render tests.

use super::tests::{decode_faces, synthetic_assets, DecodedVertex};
use super::*;

const SIZE: u32 = 256;

fn test_camera() -> Camera {
    Camera {
        pos: glam::DVec3::new(0.5, 64.0, 0.5),
        yaw: 0.0,
        pitch: 0.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: Default::default(),
    }
}

fn test_sky() -> SkyState {
    SkyState {
        time_of_day: 6000,
        game_time: 1000,
        rain: 0.0,
        thunder: 0.0,
        dimension_has_sky: true,
        fog_start: 0.0,
        fog_end: 192.0,
        cloud_height: 192.33,
        ..Default::default()
    }
}

fn test_fog() -> crate::fog::FogData {
    crate::fog::FogData {
        environmental_start: 0.0,
        environmental_end: 0.0,
        render_distance_start: 0.0,
        render_distance_end: 192.0,
        sky_end: 192.0,
        cloud_end: 96.0,
        environment: crate::fog::FogEnvironment::Atmospheric,
    }
}

/// The cloud option (vanilla `Options.cloudStatus`) gates the whole pass (vanilla
/// `LevelRenderer`: `cloudStatus != OFF` is the render condition): OFF builds no mesh
/// and emits no draw, FAST and FANCY draw once each and switching between them rebuilds
/// the records once (vanilla's `prevCloudStatus` in the rebuild condition). Skips when
/// no GPU adapter is available.
#[test]
fn cloud_status_gates_the_pass() {
    let Ok(gpu) = crate::pipeline::Gpu::new(None) else {
        eprintln!("skipped: no GPU adapter");
        return;
    };
    let mut assets = synthetic_assets();
    let mut renderer = CloudRenderer::new(
        gpu.device.clone(),
        gpu.queue.clone(),
        wgpu::TextureFormat::Bgra8UnormSrgb,
        &mut assets,
    );
    assert_eq!(renderer.status, CloudStatus::Fancy, "vanilla default");
    let camera = test_camera();
    let sky = test_sky();
    let fog = test_fog();
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("clouds-test-color"),
        size: wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let depth = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("clouds-test-depth"),
        size: wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth24Plus,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let mut draws = |renderer: &CloudRenderer| -> u32 {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clouds-test-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.create_view(&wgpu::TextureViewDescriptor::default()),
                resolve_target: None,
                ops: wgpu::Operations::default(),
                depth_slice: None,
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth.create_view(&wgpu::TextureViewDescriptor::default()),
                depth_ops: Some(wgpu::Operations::default()),
                stencil_ops: Some(wgpu::Operations::default()),
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        renderer.draw(&mut pass)
    };
    // OFF: no mesh at all, no draw call.
    renderer.set_status(CloudStatus::Off);
    renderer.prepare(
        &camera,
        &sky,
        crate::sky::SkyKind::Overworld,
        glam::Mat4::IDENTITY,
        &fog,
        0.0,
    );
    assert_eq!(renderer.face_count(), 0);
    assert_eq!(draws(&renderer), 0);
    // FAST: the flat cells, one draw call.
    renderer.set_status(CloudStatus::Fast);
    renderer.prepare(
        &camera,
        &sky,
        crate::sky::SkyKind::Overworld,
        glam::Mat4::IDENTITY,
        &fog,
        0.0,
    );
    let fast = renderer.face_count();
    assert!(fast > 0);
    assert_eq!(draws(&renderer), 1);
    // FANCY: the extruded boxes, more faces than the flat cells, one draw call.
    renderer.set_status(CloudStatus::Fancy);
    renderer.prepare(
        &camera,
        &sky,
        crate::sky::SkyKind::Overworld,
        glam::Mat4::IDENTITY,
        &fog,
        0.0,
    );
    assert!(renderer.face_count() > fast);
    assert_eq!(draws(&renderer), 1);
    // Back to OFF: the mesh is dropped again.
    renderer.set_status(CloudStatus::Off);
    renderer.prepare(
        &camera,
        &sky,
        crate::sky::SkyKind::Overworld,
        glam::Mat4::IDENTITY,
        &fog,
        0.0,
    );
    assert_eq!(renderer.face_count(), 0);
    assert_eq!(draws(&renderer), 0);
}

/// The cloud range option (vanilla `Options.cloudRange`, wired through `set_range`)
/// sizes the mesh (vanilla `CloudRenderer.prepare`'s `radiusCells`): a larger range
/// draws more faces at the same camera, without moving it.
#[test]
fn mesh_follows_the_cloud_range_option() {
    let Ok(gpu) = crate::pipeline::Gpu::new(None) else {
        eprintln!("skipped: no GPU adapter");
        return;
    };
    let mut assets = synthetic_assets();
    let mut renderer = CloudRenderer::new(
        gpu.device.clone(),
        gpu.queue.clone(),
        wgpu::TextureFormat::Bgra8UnormSrgb,
        &mut assets,
    );
    let camera = test_camera();
    let sky = test_sky();
    let fog = test_fog();
    let mut faces = 0;
    for (range, expect_at_least) in [(2u32, 1usize), (12, 50), (128, 500)] {
        renderer.set_range(range);
        renderer.prepare(
            &camera,
            &sky,
            crate::sky::SkyKind::Overworld,
            glam::Mat4::IDENTITY,
            &fog,
            0.5,
        );
        let count = renderer.face_count() as usize;
        assert!(
            count > faces && count >= expect_at_least,
            "range {range}: {count} faces (previous {faces})"
        );
        faces = count;
    }
}

// The reference path of the equality test below: the pre-change shader, which read the
// expanded mesh's positions and shades from a vertex buffer (everything else — the
// uniforms, the fragment shader — is the same `clouds.wgsl`).
const REFERENCE_WGSL: &str = r#"
struct Clouds {
    view_proj: mat4x4<f32>,
    offset: vec3<f32>,
    _pad0: f32,
    color: vec4<f32>,
    fog_end: f32,
    _pad1: vec4<f32>,
};

@group(0) @binding(0) var<uniform> clouds: Clouds;

struct VertexIn {
    @location(0) pos: vec3<f32>,
    @location(1) shade: f32,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) pos: vec3<f32>,
    @location(1) shade: f32,
};

fn linear_fog_value(dist: f32, start: f32, end: f32) -> f32 {
    if (dist <= start) {
        return 0.0;
    }
    if (dist >= end) {
        return 1.0;
    }
    return (dist - start) / (end - start);
}

fn srgb_to_linear(c: f32) -> f32 {
    if (c <= 0.04045) {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    let pos = v.pos + clouds.offset;
    out.clip = clouds.view_proj * vec4<f32>(pos, 1.0);
    out.pos = pos;
    out.shade = v.shade;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let c = in.shade * clouds.color.rgb;
    let rgb = vec3<f32>(
        srgb_to_linear(c.r),
        srgb_to_linear(c.g),
        srgb_to_linear(c.b),
    );
    let fade = 1.0 - linear_fog_value(length(in.pos), 0.0, clouds.fog_end);
    return vec4<f32>(rgb, clouds.color.a * fade);
}
"#;

/// The offscreen screenshot equality of the record change: rendering the pass (one u32
/// record per face, expanded by `clouds.wgsl`) must produce exactly the pixels the
/// CPU-expanded mesh of the same faces produced — the guarantee that the change made
/// the rebuild cheap without changing what is drawn. Same camera, game time and fog for
/// both renders; skips when no GPU adapter is available.
#[test]
fn instance_path_renders_the_expanded_mesh() {
    let Ok(gpu) = crate::pipeline::Gpu::new(None) else {
        eprintln!("skipped: no GPU adapter");
        return;
    };
    let mut assets = synthetic_assets();
    let mut renderer = CloudRenderer::new(
        gpu.device.clone(),
        gpu.queue.clone(),
        wgpu::TextureFormat::Bgra8UnormSrgb,
        &mut assets,
    );
    // Camera below the layer, looking up at it: the bottoms, the sides and the interior
    // faces all in view.
    let camera = Camera {
        pos: glam::DVec3::new(0.5, 64.0, 0.5),
        yaw: 30.0,
        pitch: -35.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: Default::default(),
    };
    let sky = test_sky();
    let fog = test_fog();
    let yaw = camera.yaw.to_radians();
    let pitch = camera.pitch.to_radians();
    let forward = glam::Vec3::new(
        -pitch.cos() * yaw.sin(),
        -pitch.sin(),
        pitch.cos() * yaw.cos(),
    );
    let view = glam::Mat4::look_at_rh(
        camera.pos.as_vec3(),
        camera.pos.as_vec3() + forward,
        glam::Vec3::Y,
    );
    let view_proj =
        glam::Mat4::perspective_rh(camera.fov_deg.to_radians(), 1.0, 0.1, 1000.0) * view;
    renderer.prepare(
        &camera,
        &sky,
        crate::sky::SkyKind::Overworld,
        view_proj,
        &fog,
        0.5,
    );
    let face_count = renderer.face_count();
    assert!(face_count > 100, "the synthetic texture has cells in view");

    // The reference: the same faces expanded on the CPU (the old mesh), rendered by the
    // pre-change shader through a plain vertex buffer.
    let vertices = decode_faces(renderer.faces());
    let vertex_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("clouds-reference-vertices"),
        size: (vertices.len() * std::mem::size_of::<DecodedVertex>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    gpu.queue
        .write_buffer(&vertex_buffer, 0, bytemuck::cast_slice(&vertices));
    let shader = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("clouds-reference"),
            source: wgpu::ShaderSource::Wgsl(REFERENCE_WGSL.into()),
        });
    let layout = gpu
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("clouds-reference-layout"),
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
    let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("clouds-reference-bind"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: renderer.uniforms().as_entire_binding(),
        }],
    });
    let pipeline_layout = gpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("clouds-reference-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
    let attributes: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32];
    let vertex_layout = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<DecodedVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &attributes,
    };
    let reference = super::make_pipeline(
        &gpu.device,
        &pipeline_layout,
        &shader,
        &vertex_layout,
        wgpu::TextureFormat::Bgra8UnormSrgb,
        Some(wgpu::Face::Back),
        "clouds-reference",
    );

    // Render both into fresh targets and read the pixels back.
    let (target_a, depth_a) = make_target(&gpu.device, "a");
    let (target_b, depth_b) = make_target(&gpu.device, "b");
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = begin_pass(&mut encoder, &target_a, &depth_a);
        let draws = renderer.draw(&mut pass);
        assert_eq!(draws, 1);
    }
    {
        let mut pass = begin_pass(&mut encoder, &target_b, &depth_b);
        pass.set_pipeline(&reference);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.draw(0..vertices.len() as u32, 0..1);
    }
    gpu.queue.submit([encoder.finish()]);
    let a = read_pixels(&gpu, &target_a);
    let b = read_pixels(&gpu, &target_b);
    if a != b {
        let first = a
            .iter()
            .zip(&b)
            .position(|(x, y)| x != y)
            .unwrap_or(a.len());
        panic!(
            "the instance path and the expanded mesh differ (first byte {first}, \
             {} faces)",
            face_count
        );
    }
}

/// A color target with its depth attachment (the clouds pipeline depth-tests).
fn make_target(device: &wgpu::Device, tag: &str) -> (wgpu::Texture, wgpu::Texture) {
    let make = |format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
        let label = format!("clouds-equality-{tag}");
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&label),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    (
        make(
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        ),
        make(
            wgpu::TextureFormat::Depth24Plus,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        ),
    )
}

fn begin_pass<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    target: &wgpu::Texture,
    depth: &wgpu::Texture,
) -> wgpu::RenderPass<'e> {
    let cleared = wgpu::Operations {
        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
        store: wgpu::StoreOp::Store,
    };
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("clouds-equality-pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &target.create_view(&wgpu::TextureViewDescriptor::default()),
            resolve_target: None,
            ops: cleared,
            depth_slice: None,
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &depth.create_view(&wgpu::TextureViewDescriptor::default()),
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

/// Reads a target's pixels back, row padding stripped.
fn read_pixels(gpu: &crate::pipeline::Gpu, target: &wgpu::Texture) -> Vec<u8> {
    let bytes_per_row = (SIZE * 4).next_multiple_of(256);
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("clouds-equality-readback"),
        size: (bytes_per_row * SIZE) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer.map_async(wgpu::MapMode::Read, .., move |r| {
        let _ = tx.send(r);
    });
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("polling the readback");
    rx.recv().expect("the map callback").expect("mapping");
    let data = buffer.get_mapped_range(..).expect("mapped range").to_vec();
    buffer.unmap();
    (0..SIZE)
        .flat_map(|y| {
            data[(y * bytes_per_row) as usize..(y * bytes_per_row + SIZE * 4) as usize].to_vec()
        })
        .collect()
}
